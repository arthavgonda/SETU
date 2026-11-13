use crate::hub::Hub;
use crate::models::{FileRecord, ModelStore, dir_size, free_bytes, hex, mtime, sha256_file};
use crate::paths::Paths;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use setu_common::protocol::{ApiError, ErrorCode};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Queued,
    Downloading,
    Paused,
    WaitingNetwork,
    Verifying,
    Extracting,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::WaitingNetwork => "waiting_network",
            Self::Verifying => "verifying",
            Self::Extracting => "extracting",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

struct Job {
    id: String,
    name: String,
    status: Status,
    bytes: u64,
    total: u64,
    rate: f64,
    message: Option<String>,
}

impl Job {
    fn json(&self) -> Value {
        let eta = if self.rate > 1.0 && self.total > self.bytes { Some(((self.total - self.bytes) as f64 / self.rate).round() as u64) } else { None };
        json!({
            "id": self.id, "name": self.name, "bytes": self.bytes, "total": self.total,
            "rate": self.rate.round() as u64, "etaS": eta, "status": self.status.as_str(), "message": self.message
        })
    }
}

type DoneHook = Box<dyn Fn(String) + Send + Sync>;

pub struct Downloads {
    jobs: Mutex<Vec<Job>>,
    cv: Condvar,
    store: Arc<ModelStore>,
    hub: Arc<Hub>,
    paths: Paths,
    pub hold: AtomicBool,
    pub power_saver: AtomicBool,
    on_done: Mutex<Option<DoneHook>>,
    agent: ureq::Agent,
    running: AtomicBool,
    me: Mutex<std::sync::Weak<Downloads>>,
}

enum Outcome {
    Done,
    Paused,
    Cancelled,
    Retry(String),
    Failed(String),
}

fn partial_dir(p: &Paths) -> PathBuf {
    p.models.join(".partial")
}

fn background_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        libc::syscall(libc::SYS_ioprio_set, 1, tid as libc::c_long, (3i64 << 13) as libc::c_long);
    }
}

impl Downloads {
    pub fn new(store: Arc<ModelStore>, hub: Arc<Hub>, paths: Paths) -> Arc<Self> {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .user_agent(format!("Setu/{}", setu_common::VERSION))
            .build()
            .new_agent();
        let me = Arc::new(Self {
            jobs: Mutex::new(Vec::new()),
            cv: Condvar::new(),
            store,
            hub,
            paths,
            hold: AtomicBool::new(false),
            power_saver: AtomicBool::new(false),
            on_done: Mutex::new(None),
            agent,
            running: AtomicBool::new(false),
            me: Mutex::new(std::sync::Weak::new()),
        });
        *me.me.lock().unwrap() = Arc::downgrade(&me);
        me
    }

    fn kick(&self) {
        if self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return;
        }
        let Some(w) = self.me.lock().unwrap().upgrade() else {
            self.running.store(false, Ordering::SeqCst);
            return;
        };
        let started = std::thread::Builder::new().name("downloads".into()).spawn(move || {
            background_priority();
            w.worker()
        });
        if started.is_err() {
            self.running.store(false, Ordering::SeqCst);
        }
    }

    pub fn worker_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn on_done(&self, f: DoneHook) {
        *self.on_done.lock().unwrap() = Some(f);
    }

    pub fn list(&self) -> Value {
        Value::Array(self.jobs.lock().unwrap().iter().map(Job::json).collect())
    }

    pub fn active(&self) -> bool {
        self.jobs.lock().unwrap().iter().any(|j| matches!(j.status, Status::Queued | Status::Downloading | Status::WaitingNetwork | Status::Verifying | Status::Extracting))
    }

    fn emit(&self, j: &Job) {
        self.hub.emit("download.progress", j.json());
    }

    pub fn enqueue(&self, ids: &[String]) -> Result<Value, ApiError> {
        let mut need = 0u64;
        let mut queue = Vec::new();
        for id in ids {
            let m = self.store.manifest.model(id).ok_or_else(|| ApiError::not_found(format!("No model called {id}")))?;
            if m.bundled || self.store.is_installed(id) {
                continue;
            }
            let have = std::fs::metadata(partial_dir(&self.paths).join(format!("{id}.part"))).map(|m| m.len()).unwrap_or(0);
            need += self.store.install_size(id).saturating_sub(have);
            queue.push(m.clone());
        }
        let free = free_bytes(&self.paths.models);
        if need > free {
            return Err(ApiError::new(ErrorCode::DiskFull, format!("Needs {} free. You have {} free.", gb(need), gb(free)))
                .with_details(json!({ "needBytes": need, "freeBytes": free })));
        }
        let mut g = self.jobs.lock().unwrap();
        let mut queued = Vec::new();
        for m in queue {
            if let Some(j) = g.iter_mut().find(|j| j.id == m.id) {
                if matches!(j.status, Status::Failed | Status::Cancelled | Status::Paused | Status::Done) {
                    j.status = Status::Queued;
                    j.message = None;
                }
                self.hub.emit("download.progress", j.json());
            } else {
                let j = Job { id: m.id.clone(), name: m.name.clone(), status: Status::Queued, bytes: 0, total: m.size, rate: 0.0, message: None };
                self.hub.emit("download.progress", j.json());
                g.push(j);
            }
            queued.push(m.id.clone());
        }
        drop(g);
        self.cv.notify_all();
        self.kick();
        Ok(json!({ "queued": queued, "needBytes": need, "freeBytes": free }))
    }

    pub fn pause(&self, id: &str) -> Result<(), ApiError> {
        self.set(id, Status::Paused, Some("Paused.".into()))
    }

    pub fn resume(&self, id: &str) -> Result<(), ApiError> {
        self.set(id, Status::Queued, None)
    }

    pub fn cancel(&self, id: &str) -> Result<(), ApiError> {
        self.set(id, Status::Cancelled, None)?;
        let _ = std::fs::remove_file(partial_dir(&self.paths).join(format!("{id}.part")));
        Ok(())
    }

    fn set(&self, id: &str, st: Status, msg: Option<String>) -> Result<(), ApiError> {
        let mut g = self.jobs.lock().unwrap();
        let j = g.iter_mut().find(|j| j.id == id).ok_or_else(|| ApiError::not_found("No such download"))?;
        if matches!(j.status, Status::Done) {
            return Ok(());
        }
        j.status = st;
        j.message = msg;
        j.rate = 0.0;
        self.emit(j);
        drop(g);
        self.cv.notify_all();
        self.kick();
        Ok(())
    }

    pub fn wake(&self) {
        self.cv.notify_all();
        self.kick();
    }

    fn status_of(&self, id: &str) -> Option<Status> {
        self.jobs.lock().unwrap().iter().find(|j| j.id == id).map(|j| j.status)
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        let mut g = self.jobs.lock().unwrap();
        if let Some(j) = g.iter_mut().find(|j| j.id == id) {
            f(j);
            self.emit(j);
        }
    }

    fn worker(self: Arc<Self>) {
        let mut backoff = 2u64;
        loop {
            let next = {
                let mut g = self.jobs.lock().unwrap();
                let held = self.hold.load(Ordering::Relaxed) || self.power_saver.load(Ordering::Relaxed);
                match g.iter_mut().find(|j| j.status == Status::Queued || j.status == Status::WaitingNetwork) {
                    Some(j) if !held => {
                        j.status = Status::Downloading;
                        j.id.clone()
                    }
                    _ => {
                        self.running.store(false, Ordering::SeqCst);
                        return;
                    }
                }
            };
            let Some(def) = self.store.manifest.model(&next).cloned() else { continue };
            match self.run(&def) {
                Outcome::Done => {
                    backoff = 2;
                    self.update(&next, |j| {
                        j.status = Status::Done;
                        j.bytes = j.total;
                        j.rate = 0.0;
                        j.message = None;
                    });
                    if let Some(f) = self.on_done.lock().unwrap().as_ref() {
                        f(next.clone());
                    }
                }
                Outcome::Paused | Outcome::Cancelled => {}
                Outcome::Failed(m) => {
                    log::warn!("download of {next} failed: {m}");
                    self.update(&next, |j| {
                        j.status = Status::Failed;
                        j.rate = 0.0;
                        j.message = Some(m);
                    });
                }
                Outcome::Retry(m) => {
                    let pct = self.jobs.lock().unwrap().iter().find(|j| j.id == next).and_then(|j| (j.bytes * 100).checked_div(j.total)).unwrap_or(0);
                    log::info!("download of {next} interrupted: {m}");
                    self.update(&next, |j| {
                        j.status = Status::WaitingNetwork;
                        j.rate = 0.0;
                        j.message = Some(format!("Paused at {pct}%. The connection dropped. It will resume when you're online."));
                    });
                    let g = self.jobs.lock().unwrap();
                    let _ = self.cv.wait_timeout(g, Duration::from_secs(backoff)).unwrap();
                    backoff = (backoff * 2).min(30);
                }
            }
        }
    }

    fn interrupted(&self, id: &str) -> Option<Outcome> {
        match self.status_of(id) {
            Some(Status::Paused) => Some(Outcome::Paused),
            Some(Status::Cancelled) | None => Some(Outcome::Cancelled),
            _ if self.hold.load(Ordering::Relaxed) || self.power_saver.load(Ordering::Relaxed) => {
                self.update(id, |j| {
                    j.status = Status::Queued;
                    j.rate = 0.0;
                    j.message = Some(if self.power_saver.load(Ordering::Relaxed) {
                        "Waiting while battery saver is on.".into()
                    } else {
                        "Waiting until you finish speaking.".into()
                    });
                });
                Some(Outcome::Paused)
            }
            _ => None,
        }
    }

    fn run(&self, m: &crate::models::ModelDef) -> Outcome {
        let pdir = partial_dir(&self.paths);
        if let Err(e) = std::fs::create_dir_all(&pdir) {
            return Outcome::Failed(e.to_string());
        }
        let part = pdir.join(format!("{}.part", m.id));
        let mut have = std::fs::metadata(&part).map(|x| x.len()).unwrap_or(0);
        if have > m.size {
            let _ = std::fs::remove_file(&part);
            have = 0;
        }
        let mut hasher = Sha256::new();
        if have > 0 {
            match std::fs::File::open(&part) {
                Ok(mut f) => {
                    let mut buf = vec![0u8; 1 << 20];
                    loop {
                        match f.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => hasher.update(&buf[..n]),
                            Err(e) => return Outcome::Failed(e.to_string()),
                        }
                    }
                }
                Err(e) => return Outcome::Failed(e.to_string()),
            }
        }
        self.update(&m.id, |j| {
            j.bytes = have;
            j.total = m.size;
            j.message = None;
        });
        if have < m.size {
            let mut req = self.agent.get(&m.url);
            if have > 0 {
                req = req.header("Range", &format!("bytes={have}-"));
            }
            let resp = match req.call() {
                Ok(r) => r,
                Err(e) => return Outcome::Retry(e.to_string()),
            };
            let status = resp.status().as_u16();
            if have > 0 && status == 200 {
                have = 0;
                hasher = Sha256::new();
            } else if status != 200 && status != 206 {
                return if status >= 500 || status == 429 { Outcome::Retry(format!("server answered {status}")) } else { Outcome::Failed(format!("The download server answered {status}.")) };
            }
            let mut file = match std::fs::OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part) {
                Ok(f) => f,
                Err(e) => return Outcome::Failed(e.to_string()),
            };
            let mut body = resp.into_body().into_reader();
            let mut buf = vec![0u8; 256 * 1024];
            let mut last = Instant::now();
            let mut window = (Instant::now(), have);
            loop {
                if let Some(o) = self.interrupted(&m.id) {
                    let _ = file.flush();
                    return o;
                }
                let n = match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        let _ = file.flush();
                        return Outcome::Retry(e.to_string());
                    }
                };
                if let Err(e) = file.write_all(&buf[..n]) {
                    return Outcome::Failed(if e.raw_os_error() == Some(28) { "Not enough disk space.".into() } else { e.to_string() });
                }
                hasher.update(&buf[..n]);
                have += n as u64;
                if last.elapsed() >= Duration::from_millis(250) {
                    last = Instant::now();
                    let dt = window.0.elapsed().as_secs_f64();
                    let rate = if dt > 0.0 { (have - window.1) as f64 / dt } else { 0.0 };
                    if dt > 3.0 {
                        window = (Instant::now(), have);
                    }
                    self.update(&m.id, |j| {
                        j.bytes = have;
                        j.rate = rate;
                        j.status = Status::Downloading;
                        j.message = None;
                    });
                }
            }
            let _ = file.sync_all();
            if have < m.size {
                return Outcome::Retry("the connection closed early".into());
            }
        }
        self.update(&m.id, |j| {
            j.status = Status::Verifying;
            j.bytes = have;
            j.rate = 0.0;
        });
        if hex(&hasher.finalize()) != m.sha256 {
            let _ = std::fs::remove_file(&part);
            return Outcome::Failed("The download was damaged and failed its checksum. Download it again.".into());
        }
        self.update(&m.id, |j| j.status = Status::Extracting);
        match self.install(m, &part) {
            Ok(files) => {
                let _ = std::fs::remove_file(&part);
                if let Err(e) = self.store.record(&m.id, files) {
                    return Outcome::Failed(e.to_string());
                }
                Outcome::Done
            }
            Err(e) => Outcome::Failed(format!("The model could not be unpacked: {e}")),
        }
    }

    fn install(&self, m: &crate::models::ModelDef, part: &Path) -> Result<BTreeMap<String, FileRecord>> {
        let dest = self.paths.models.join(&m.id);
        let tmp = self.paths.models.join(format!(".tmp-{}", m.id));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp)?;
        let src_root = if m.archive == "tar.bz2" {
            let f = std::fs::File::open(part)?;
            let mut ar = tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(f)));
            ar.set_preserve_permissions(false);
            for entry in ar.entries()? {
                let mut e = entry?;
                let path = e.path()?.into_owned();
                if path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))) {
                    return Err(anyhow!("archive contains an unsafe path"));
                }
                let kind = e.header().entry_type();
                if !(kind.is_file() || kind.is_dir()) {
                    continue;
                }
                e.unpack_in(&tmp)?;
            }
            tmp.join(m.root.as_deref().unwrap_or(""))
        } else {
            let name = m.files.values().next().ok_or_else(|| anyhow!("manifest entry has no file"))?;
            std::fs::rename(part, tmp.join(name))?;
            tmp.clone()
        };
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::create_dir_all(&dest)?;
        let mut out = BTreeMap::new();
        for (key, rel) in &m.files {
            let from = src_root.join(rel);
            let to = dest.join(rel);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            if !from.exists() {
                return Err(anyhow!("{rel} is missing from the download"));
            }
            std::fs::rename(&from, &to)?;
            let rec = if to.is_dir() {
                let (b, _) = dir_size(&to);
                FileRecord { rel: rel.clone(), sha256: String::new(), size: b, mtime: 0, dir: true }
            } else {
                FileRecord { rel: rel.clone(), sha256: sha256_file(&to)?, size: std::fs::metadata(&to)?.len(), mtime: mtime(&to), dir: false }
            };
            out.insert(key.clone(), rec);
        }
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(out)
    }
}

pub fn gb(b: u64) -> String {
    if b >= 1_000_000_000 { format!("{:.1} GB", b as f64 / 1e9) } else { format!("{} MB", (b as f64 / 1e6).ceil() as u64) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_sizes() {
        assert_eq!(super::gb(2_400_000_000), "2.4 GB");
        assert_eq!(super::gb(180_000_000), "180 MB");
        assert_eq!(super::gb(1), "1 MB");
    }
}
