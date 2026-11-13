use anyhow::{Result, anyhow};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub fn cores() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2)
}

pub fn blocking_threads() -> usize {
    (cores() * 2).clamp(4, 16)
}

pub fn background_lanes() -> usize {
    (cores() / 2).clamp(1, 4)
}

pub const IDLE_THREAD: Duration = Duration::from_secs(10);
#[cfg_attr(not(test), allow(dead_code))]
pub const MODEL_LINGER: Duration = Duration::from_secs(60);

static LANES: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
static RUNNING: AtomicUsize = AtomicUsize::new(0);

pub fn running_background() -> usize {
    RUNNING.load(Ordering::Relaxed)
}

pub fn state(download_worker: bool) -> serde_json::Value {
    serde_json::json!({
        "cores": cores(),
        "blockingThreads": blocking_threads(),
        "backgroundLanes": background_lanes(),
        "backgroundRunning": running_background(),
        "downloadWorker": download_worker,
        "soundWorker": crate::tools::streams::connected(),
        "threads": thread_count(),
    })
}

fn thread_count() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/self/status").ok()?;
        return s.lines().find_map(|l| l.strip_prefix("Threads:")).and_then(|v| v.trim().parse().ok());
    }
    #[allow(unreachable_code)]
    None
}

fn lower_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
}

struct Count;

impl Count {
    fn new() -> Self {
        RUNNING.fetch_add(1, Ordering::Relaxed);
        Count
    }
}

impl Drop for Count {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::Relaxed);
    }
}

pub async fn background<F, R>(name: &str, f: F) -> Result<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let lanes = LANES.get_or_init(|| tokio::sync::Semaphore::new(background_lanes()));
    let _permit = lanes.acquire().await.map_err(|_| anyhow!("Setu is shutting down."))?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let job = crate::layers::traced(f);
    std::thread::Builder::new()
        .name(format!("setu-bg-{name}"))
        .spawn(move || {
            let _count = Count::new();
            lower_priority();
            let _ = tx.send(job());
        })
        .map_err(|e| anyhow!("Couldn't start background work: {e}"))?;
    rx.await.map_err(|_| anyhow!("Background work stopped unexpectedly."))
}

#[cfg_attr(not(test), allow(dead_code))]
pub struct Linger<T> {
    slot: Mutex<Option<Arc<T>>>,
    idle: Duration,
    used: AtomicU64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl<T: Send + Sync + 'static> Linger<T> {
    pub const fn new(idle: Duration) -> Self {
        Self { slot: Mutex::new(None), idle, used: AtomicU64::new(0) }
    }

    pub fn get(&'static self, load: impl FnOnce() -> Result<T>) -> Result<Arc<T>> {
        let v = {
            let mut g = self.slot.lock().unwrap_or_else(|e| e.into_inner());
            match g.as_ref() {
                Some(v) => v.clone(),
                None => {
                    let v = Arc::new(load()?);
                    *g = Some(v.clone());
                    v
                }
            }
        };
        self.touch();
        Ok(v)
    }

    fn touch(&'static self) {
        let mine = self.used.fetch_add(1, Ordering::SeqCst) + 1;
        let idle = self.idle;
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move {
                tokio::time::sleep(idle).await;
                if self.used.load(Ordering::SeqCst) == mine {
                    self.free();
                }
            });
        }
    }

    pub fn free(&self) {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    pub fn loaded(&self) -> bool {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_cores() {
        assert!((4..=16).contains(&blocking_threads()));
        assert!((1..=4).contains(&background_lanes()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn background_work_runs_and_its_thread_ends() {
        let id = background("test", || std::thread::current().name().map(str::to_owned)).await.unwrap();
        assert_eq!(id.as_deref(), Some("setu-bg-test"));
        for _ in 0..50 {
            if running_background() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(running_background(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn background_lanes_limit_how_many_run_at_once() {
        static PEAK: AtomicUsize = AtomicUsize::new(0);
        static NOW: AtomicUsize = AtomicUsize::new(0);
        let jobs: Vec<_> = (0..12)
            .map(|_| {
                tokio::spawn(background("lane", || {
                    let n = NOW.fetch_add(1, Ordering::SeqCst) + 1;
                    PEAK.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(15));
                    NOW.fetch_sub(1, Ordering::SeqCst);
                }))
            })
            .collect();
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        assert!(PEAK.load(Ordering::SeqCst) <= background_lanes());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lingering_model_loads_once_and_is_freed_when_idle() {
        static LOADS: AtomicUsize = AtomicUsize::new(0);
        static MODEL: Linger<Vec<u8>> = Linger::new(Duration::from_millis(120));
        let load = || {
            LOADS.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1u8; 16])
        };
        MODEL.get(load).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        MODEL.get(load).unwrap();
        assert_eq!(LOADS.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(90)).await;
        assert!(MODEL.loaded(), "freed too early, before its idle time after the last use");
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(!MODEL.loaded(), "still loaded after the idle time");
        MODEL.get(load).unwrap();
        assert_eq!(LOADS.load(Ordering::SeqCst), 2);
    }
}
