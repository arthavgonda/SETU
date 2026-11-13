use crate::core::Core;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;

#[cfg(target_os = "linux")]
pub fn raise_capture(core: &Core) {
    let tid = core.audio.capture_tid.load(Ordering::Relaxed);
    if tid <= 0 {
        return;
    }
    tokio::spawn(async move {
        let Ok(c) = zbus::Connection::system().await else { return };
        let Ok(p) = zbus::Proxy::new(&c, "org.freedesktop.RealtimeKit1", "/org/freedesktop/RealtimeKit1", "org.freedesktop.RealtimeKit1").await else { return };
        let max: i32 = p.get_property("MaxRealtimePriority").await.unwrap_or(0);
        if max > 0 {
            let lim = libc::rlimit { rlim_cur: 200_000, rlim_max: 200_000 };
            unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &lim) };
            let r: Result<(), _> = p.call("MakeThreadRealtime", &(tid as u64, max.min(10) as u32)).await;
            match r {
                Ok(()) => log::info!("microphone thread runs at real-time priority {}", max.min(10)),
                Err(e) => log::info!("real-time priority was not granted: {e}"),
            }
        } else {
            let r: Result<(), _> = p.call("MakeThreadHighPriority", &(tid as u64, -11i32)).await;
            if r.is_ok() {
                log::info!("microphone thread runs at high priority");
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
pub fn raise_capture(_core: &Core) {}
