use crate::core::Core;
use crate::router::Intent;
use crate::tools::install::Package;
use crate::tools::{Choice, FollowUp, Outcome};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const LAYER: &str = "system";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Job {
    pub id: u64,
    pub app: String,
    pub step: String,
    pub package: Option<Package>,
    pub url: Option<String>,
    pub site: Option<String>,
    pub hosts: Vec<String>,
    pub sha256: Option<String>,
    pub file: Option<PathBuf>,
    pub folder: Option<PathBuf>,
    pub target: Option<PathBuf>,
    pub install: bool,
    pub bytes: u64,
    pub total: u64,
    pub percent: Option<f64>,
    pub rate: f64,
    pub message: String,
    pub paused: bool,
    pub signed: Option<bool>,
    pub trusted: bool,
    pub started: i64,
    pub updated: i64,
    #[serde(default)]
    pub wizard: Option<Setup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Setup {
    pub pid: u32,
    pub title: String,
    pub consent: bool,
}

const SETUP: &str = "\u{1}setup:";

#[derive(Default)]
struct Flags {
    pause: AtomicBool,
    cancel: AtomicBool,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    next: u64,
    jobs: Vec<Job>,
    keep: Option<bool>,
}

struct Table {
    core: Arc<Core>,
    path: PathBuf,
    jobs: Mutex<Vec<Job>>,
    flags: Mutex<HashMap<u64, Arc<Flags>>>,
    next: AtomicU64,
    keep: Mutex<Option<bool>>,
    last_emit: Mutex<Instant>,
}

static TABLE: OnceLock<Arc<Table>> = OnceLock::new();

fn table() -> Option<&'static Arc<Table>> {
    TABLE.get()
}

fn now() -> i64 {
    setu_common::protocol::now_ms()
}

pub fn finished(step: &str) -> bool {
    matches!(step, "done" | "failed" | "cancelled")
}

impl Table {
    fn save(&self) {
        let jobs: Vec<Job> = self.jobs.lock().unwrap().iter().filter(|j| !finished(&j.step) || now() - j.updated < 6 * 3_600_000).cloned().collect();
        let s = Saved { next: self.next.load(Ordering::SeqCst), jobs, keep: *self.keep.lock().unwrap() };
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec(&s).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, &self.path);
        }
    }

    fn emit(&self, force: bool) {
        {
            let mut last = self.last_emit.lock().unwrap();
            if !force && last.elapsed() < Duration::from_millis(200) {
                return;
            }
            *last = Instant::now();
        }
        self.core.hub.emit("job.state", state());
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        let mut changed_step = false;
        {
            let mut g = self.jobs.lock().unwrap();
            if let Some(j) = g.iter_mut().find(|j| j.id == id) {
                let before = j.step.clone();
                f(j);
                j.updated = now();
                changed_step = before != j.step;
            }
        }
        if changed_step {
            self.save();
        }
        self.emit(changed_step);
    }

    fn get(&self, id: u64) -> Option<Job> {
        self.jobs.lock().unwrap().iter().find(|j| j.id == id).cloned()
    }

    fn flags(&self, id: u64) -> Arc<Flags> {
        self.flags.lock().unwrap().entry(id).or_default().clone()
    }
}

pub fn state() -> Value {
    let Some(t) = table() else { return json!({ "jobs": [] }) };
    let jobs: Vec<Value> = t
        .jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|j| !finished(&j.step) || now() - j.updated < 20_000)
        .map(|j| json!({ "id": j.id, "app": j.app, "step": j.step, "percent": j.percent, "bytes": j.bytes, "total": j.total, "paused": j.paused, "message": j.message, "icon": j.app.to_lowercase() }))
        .collect();
    json!({ "jobs": jobs })
}

pub fn start(core: &Arc<Core>) {
    let path = core.paths.data.join("jobs.json");
    let saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let t = Arc::new(Table { core: core.clone(), path, jobs: Mutex::new(saved.jobs), flags: Mutex::new(HashMap::new()), next: AtomicU64::new(saved.next.max(1)), keep: Mutex::new(saved.keep), last_emit: Mutex::new(Instant::now()) });
    if TABLE.set(t.clone()).is_err() {
        return;
    }
    let resume: Vec<u64> = t.jobs.lock().unwrap().iter().filter(|j| !finished(&j.step) && j.step != "waiting" && !j.paused).map(|j| j.id).collect();
    for id in resume {
        t.update(id, |j| j.message = "Picking up where I stopped.".into());
        spawn(id);
    }
}

fn spawn(id: u64) {
    tokio::spawn(async move {
        run(id).await;
    });
}

pub fn active() -> Vec<Job> {
    table().map(|t| t.jobs.lock().unwrap().iter().filter(|j| !finished(&j.step)).cloned().collect()).unwrap_or_default()
}

fn new_job(mut j: Job) -> Option<u64> {
    let t = table()?;
    let id = t.next.fetch_add(1, Ordering::SeqCst);
    j.id = id;
    j.started = now();
    j.updated = now();
    if j.step.is_empty() {
        j.step = "queued".into();
    }
    t.jobs.lock().unwrap().push(j);
    t.save();
    t.emit(true);
    spawn(id);
    Some(id)
}

pub fn start_package(pkg: Package) -> Option<u64> {
    if active().iter().any(|j| j.package.as_ref().is_some_and(|p| p.id == pkg.id)) {
        return None;
    }
    new_job(Job { app: pkg.name.clone(), package: Some(pkg), install: true, step: "install".into(), ..Default::default() })
}

pub fn start_setup(app: &str, pid: u32, title: &str) -> Option<u64> {
    if active().iter().any(|j| j.wizard.as_ref().is_some_and(|w| (pid != 0 && w.pid == pid) || (!title.is_empty() && w.title == title))) {
        return None;
    }
    new_job(Job { app: app.to_owned(), install: true, step: "setup".into(), wizard: Some(Setup { pid, title: title.to_owned(), consent: false }), ..Default::default() })
}

async fn ask_licence(id: u64, app: &str, text: &str) {
    let question = format!("{app} has a licence agreement. Shall I accept it? You can say read it first.");
    let yes = Intent::Act { tool: "jobs.accept".into(), args: json!({ "id": id }) };
    let read = Intent::Act { tool: "jobs.licence".into(), args: json!({ "id": id }) };
    let no = Intent::Act { tool: "jobs.cancel".into(), args: json!({ "id": id }) };
    let q = FollowUp {
        question: question.clone(),
        spoken: Some(question),
        choices: vec![
            Choice::new("Accept", &["yes", "accept", "accept it", "i accept", "agree", "i agree", "ok", "okay", "go ahead"], yes),
            Choice::new("Read it", &["read it", "read it first", "read it to me", "what does it say", "read the licence", "read the license"], read),
            Choice::new("Stop", &["no", "stop", "cancel", "don't", "decline", "not now"], no),
        ],
        cleanup: None,
        kind: "confirm",
        focus: None,
    };
    LICENCES.lock().unwrap().insert(id, text.chars().take(4000).collect());
    let Some(t) = table() else { return };
    take_ask(id);
    ASKS.lock().unwrap().push((id, q));
    let intent = Intent::Act { tool: "jobs.ask".into(), args: json!({ "id": id }) };
    let _ = crate::control::ask(&t.core, |r| crate::control::Req::RunIntent { intent, reply: r }).await;
}

static LICENCES: std::sync::LazyLock<Mutex<HashMap<u64, String>>> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn start_url(app: &str, url: &str, site: &str, hosts: Vec<String>, sha256: Option<String>, install: bool, folder: Option<PathBuf>) -> Option<u64> {
    new_job(Job { app: app.to_owned(), url: Some(url.to_owned()), site: Some(site.to_owned()), hosts, sha256, install, folder, step: "download".into(), ..Default::default() })
}

fn downloads_dir() -> PathBuf {
    crate::tools::files::known_folder("downloads").or_else(dirs::download_dir).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"))
}

fn file_name(url: &str, app: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    let clean: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')).collect();
    if clean.contains('.') && clean.len() > 2 {
        return clean;
    }
    let slug: String = app.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{}-download", if slug.is_empty() { "setu".into() } else { slug })
}

pub fn kind_of(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.ends_with(".rpm") {
        "rpm"
    } else if n.ends_with(".deb") {
        "deb"
    } else if n.ends_with(".appimage") {
        "appimage"
    } else if n.ends_with(".flatpakref") || n.ends_with(".flatpak") {
        "flatpak"
    } else if n.ends_with(".msi") {
        "msi"
    } else if n.ends_with(".exe") {
        "exe"
    } else if n.ends_with(".dmg") {
        "dmg"
    } else if n.ends_with(".pkg") {
        "pkg"
    } else {
        "file"
    }
}

pub fn fits_here(kind: &str) -> bool {
    match kind {
        "rpm" | "deb" | "appimage" | "flatpak" => cfg!(target_os = "linux"),
        "msi" | "exe" => cfg!(windows),
        "dmg" | "pkg" => cfg!(target_os = "macos"),
        _ => true,
    }
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("").to_lowercase()
}

pub fn host_allowed(host: &str, hosts: &[String]) -> bool {
    hosts.is_empty() || hosts.iter().any(|h| host == h || host.ends_with(&format!(".{h}")))
}

enum Fetched {
    Done(String),
    Paused,
    Cancelled,
    Failed(String),
}

fn fetch(t: &Table, id: u64, url: &str, dest: &Path, flags: &Flags) -> Fetched {
    use ureq::ResponseExt;
    let agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_recv_response(Some(Duration::from_secs(30))).user_agent(format!("Setu/{}", setu_common::VERSION)).build().new_agent();
    let part = dest.with_extension(format!("{}.part", dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()));
    let mut tries = 0;
    loop {
        let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let mut req = agent.get(url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(e) => {
                tries += 1;
                if tries > 8 {
                    return Fetched::Failed(format!("The download couldn't reach the server: {e}"));
                }
                t.update(id, |j| j.message = "The connection dropped. Trying again.".into());
                std::thread::sleep(Duration::from_secs((2u64 << tries.min(4)).min(30)));
                continue;
            }
        };
        let final_url = resp.get_uri().to_string();
        let status = resp.status().as_u16();
        if have > 0 && status == 200 {
            have = 0;
        } else if status != 200 && status != 206 {
            return Fetched::Failed(format!("The download server answered {status}."));
        }
        let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let total = len.map(|l| l + have).unwrap_or(0);
        t.update(id, |j| {
            j.total = total;
            j.bytes = have;
            j.percent = (total > 0).then(|| have as f64 * 100.0 / total as f64);
            j.message.clear();
        });
        let mut file = match std::fs::OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part) {
            Ok(f) => f,
            Err(e) => return Fetched::Failed(e.to_string()),
        };
        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 128 * 1024];
        let mut last = Instant::now();
        let mut window = (Instant::now(), have);
        let mut broke = None;
        loop {
            if flags.cancel.load(Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&part);
                return Fetched::Cancelled;
            }
            if flags.pause.load(Ordering::SeqCst) {
                let _ = file.flush();
                return Fetched::Paused;
            }
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    broke = Some(e.to_string());
                    break;
                }
            };
            if let Err(e) = file.write_all(&buf[..n]) {
                return Fetched::Failed(if e.raw_os_error() == Some(28) { "There isn't enough disk space.".into() } else { e.to_string() });
            }
            have += n as u64;
            if last.elapsed() >= Duration::from_millis(250) {
                last = Instant::now();
                let dt = window.0.elapsed().as_secs_f64();
                let rate = if dt > 0.0 { (have - window.1) as f64 / dt } else { 0.0 };
                if dt > 3.0 {
                    window = (Instant::now(), have);
                }
                t.update(id, |j| {
                    j.bytes = have;
                    j.rate = rate;
                    j.percent = (total > 0).then(|| have as f64 * 100.0 / total as f64);
                });
            }
        }
        let _ = file.sync_all();
        drop(file);
        if broke.is_some() || (total > 0 && have < total) {
            tries += 1;
            if tries > 8 {
                return Fetched::Failed("The download kept stopping. Try again later.".into());
            }
            t.update(id, |j| {
                j.bytes = have;
                j.message = "The connection dropped. Picking up where it stopped.".into();
            });
            std::thread::sleep(Duration::from_millis(500 * tries as u64));
            continue;
        }
        if let Err(e) = std::fs::rename(&part, dest) {
            return Fetched::Failed(e.to_string());
        }
        t.update(id, |j| {
            j.bytes = have;
            j.total = have;
            j.percent = Some(100.0);
        });
        return Fetched::Done(final_url);
    }
}

fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn signature(path: &Path, kind: &str) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        if kind == "rpm" {
            let o = std::process::Command::new("rpm").arg("-K").arg(path).output().ok()?;
            let text = String::from_utf8_lossy(&o.stdout).to_lowercase();
            return Some(o.status.success() && text.contains("signatures ok"));
        }
        let _ = path;
        Some(false)
    }
    #[cfg(target_os = "macos")]
    {
        let args: Vec<&str> = if kind == "pkg" { vec!["--assess", "--type", "install"] } else { vec!["--assess", "--type", "open", "--context", "context:primary-signature"] };
        let o = std::process::Command::new("spctl").args(&args).arg(path).output().ok()?;
        Some(o.status.success())
    }
    #[cfg(windows)]
    {
        let _ = kind;
        Some(crate::tools::win::authenticode(path))
    }
}

async fn ask(id: u64, question: String, yes: Intent, no: Intent) {
    let q = FollowUp {
        question: question.clone(),
        spoken: Some(question),
        choices: vec![Choice::new("Install", &["yes", "install it", "install", "go ahead", "ok", "okay", "sure", "install it anyway"], yes), Choice::new("Not now", &["no", "not now", "cancel", "don't", "stop"], no)],
        cleanup: None,
        kind: "confirm",
        focus: None,
    };
    let Some(t) = table() else { return };
    let intent = Intent::Act { tool: "jobs.ask".into(), args: json!({ "id": id }) };
    take_ask(id);
    ASKS.lock().unwrap().push((id, q));
    let _ = crate::control::ask(&t.core, |r| crate::control::Req::RunIntent { intent, reply: r }).await;
}

static ASKS: Mutex<Vec<(u64, FollowUp)>> = Mutex::new(Vec::new());

fn take_ask(id: u64) -> Option<FollowUp> {
    let mut g = ASKS.lock().unwrap();
    let i = g.iter().position(|(k, _)| *k == id)?;
    Some(g.remove(i).1)
}

fn notice(t: &Table, ok: bool, text: &str) {
    crate::sounds::play(&t.core, if ok { crate::sounds::Cue::Chime } else { crate::sounds::Cue::Error });
    t.core.hub.emit("notch.alert", json!({ "kind": "install", "text": text, "ok": ok, "t": now() }));
    t.core.tell(crate::control::ControlMsg::Announce(text.to_owned()));
}

async fn run(id: u64) {
    let Some(t) = table() else { return };
    let flags = t.flags(id);
    loop {
        let Some(j) = t.get(id) else { return };
        if finished(&j.step) || j.paused || j.step == "waiting" {
            return;
        }
        match j.step.as_str() {
            "queued" | "find" => t.update(id, |j| j.step = if j.package.is_some() { "install".into() } else { "download".into() }),
            "download" => {
                let url = j.url.clone().unwrap_or_default();
                let folder = j.folder.clone().unwrap_or_else(downloads_dir);
                if std::fs::create_dir_all(&folder).is_err() {
                    t.update(id, |j| {
                        j.step = "failed".into();
                        j.message = format!("I can't save into {}.", folder.display());
                    });
                    continue;
                }
                let dest = j.file.clone().unwrap_or_else(|| {
                    let n = file_name(&url, &j.app);
                    let p = Path::new(&n);
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| n.clone());
                    let ext = p.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    crate::fileops::free_name(&folder, &stem, &ext)
                });
                t.update(id, |j| j.file = Some(dest.clone()));
                let f2 = flags.clone();
                let d2 = dest.clone();
                let span = crate::layers::effect(crate::layers::Layer::System, "system.download");
                let r = crate::work::background("download", move || fetch(t, id, &url, &d2, &f2)).await.unwrap_or_else(|e| Fetched::Failed(e.to_string()));
                span.end_bool(matches!(r, Fetched::Done(_)));
                match r {
                    Fetched::Done(final_url) => {
                        let host = host_of(&final_url);
                        if !host_allowed(&host, &j.hosts) {
                            let _ = std::fs::remove_file(&dest);
                            t.update(id, |j| {
                                j.step = "failed".into();
                                j.message = format!("The file came from {host}, not from {}, so I deleted it.", j.site.clone().unwrap_or_default());
                            });
                            notice(t, false, &format!("I stopped the {} download: it came from a different site than expected.", j.app));
                            continue;
                        }
                        t.update(id, |j| {
                            j.step = "verify".into();
                            j.message.clear();
                        });
                        if !j.install {
                            notice(t, true, &format!("{} has been downloaded.", j.app));
                        }
                    }
                    Fetched::Paused => {
                        t.update(id, |j| {
                            j.paused = true;
                            j.message = "Paused.".into();
                        });
                        return;
                    }
                    Fetched::Cancelled => {
                        t.update(id, |j| {
                            j.step = "cancelled".into();
                            j.message = "Cancelled.".into();
                        });
                        return;
                    }
                    Fetched::Failed(e) => {
                        t.update(id, |j| {
                            j.step = "failed".into();
                            j.message = e.clone();
                        });
                        notice(t, false, &format!("The {} download failed. {e}", j.app));
                        return;
                    }
                }
            }
            "verify" => {
                let file = j.file.clone().unwrap_or_default();
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let kind = kind_of(&name);
                if let Some(want) = j.sha256.clone() {
                    let f = file.clone();
                    let got = crate::work::background("verify", move || sha256_file(&f)).await.ok().flatten().unwrap_or_default();
                    if !got.eq_ignore_ascii_case(&want) {
                        let _ = std::fs::remove_file(&file);
                        t.update(id, |j| {
                            j.step = "failed".into();
                            j.message = "The file didn't match what the maker published, so I deleted it.".into();
                        });
                        notice(t, false, &format!("The {} download was damaged or changed, so I deleted it.", j.app));
                        return;
                    }
                }
                if !j.install {
                    t.update(id, |j| j.step = "done".into());
                    return;
                }
                if !fits_here(kind) {
                    t.update(id, |j| {
                        j.step = "failed".into();
                        j.message = format!("{name} is made for another system, so I won't run it.");
                    });
                    notice(t, false, &format!("{name} is made for another system, so I didn't run it."));
                    return;
                }
                let (f2, k2) = (file.clone(), kind);
                let signed = crate::work::background("verify", move || signature(&f2, k2)).await.ok().flatten();
                let site = j.site.clone().unwrap_or_else(|| host_of(j.url.as_deref().unwrap_or("")));
                let question = match signed {
                    Some(true) => format!("{} is downloaded from {site} and signed by its maker. Install it now?", j.app),
                    _ => format!("{} is downloaded from {site}, but the file isn't signed in a way this computer can check. Install it anyway?", j.app),
                };
                t.update(id, |j| {
                    j.signed = signed;
                    j.step = "waiting".into();
                    j.message = question.clone();
                });
                ask(id, question, Intent::Act { tool: "jobs.trust".into(), args: json!({ "id": id }) }, Intent::Act { tool: "jobs.cancel".into(), args: json!({ "id": id }) }).await;
                return;
            }
            "install" => {
                t.update(id, |j| {
                    j.percent = None;
                    j.message = "Installing.".into();
                });
                let j2 = j.clone();
                let f2 = flags.clone();
                let span = crate::layers::effect(crate::layers::Layer::System, "system.install");
                let r = install_step(t, id, j2, f2).await;
                span.end_bool(r.is_ok());
                if let Some(pid) = r.as_ref().err().and_then(|e| e.strip_prefix(SETUP)).map(|p| p.parse::<u32>().unwrap_or(0)) {
                    t.update(id, |j| {
                        j.step = "setup".into();
                        j.wizard = Some(Setup { pid, ..Default::default() });
                        j.message = "This installer has no quiet mode, so I'm going through its windows.".into();
                    });
                    continue;
                }
                match r {
                    Ok(()) => {
                        t.update(id, |j| {
                            j.step = "done".into();
                            j.percent = Some(100.0);
                            j.message = "Installed.".into();
                        });
                        let core = t.core.clone();
                        let _ = crate::layers::blocking(move || core.apps.scan()).await;
                        t.core.hub.emit("apps.changed", json!({ "count": t.core.apps.count(), "delta": 1, "scanning": false }));
                        if let Some(file) = j.file.clone().filter(|_| *t.keep.lock().unwrap() == Some(false)) {
                            let _ = std::fs::remove_file(file);
                        }
                        notice(t, true, &format!("{} is installed.", j.app));
                    }
                    Err(e) if flags.cancel.load(Ordering::SeqCst) => {
                        t.update(id, |j| {
                            j.step = "cancelled".into();
                            j.message = e;
                        });
                    }
                    Err(e) => {
                        t.update(id, |j| {
                            j.step = "failed".into();
                            j.message = e.clone();
                        });
                        notice(t, false, &e);
                    }
                }
                return;
            }
            "setup" => {
                let w = j.wizard.clone().unwrap_or_default();
                let opts = crate::wizard::Opts { pid: w.pid, title: w.title.clone(), app: j.app.clone(), folder: j.target.as_ref().map(|p| p.to_string_lossy().into_owned()), consent: w.consent };
                t.update(id, |j| {
                    j.percent = None;
                    j.message = "Going through the setup.".into();
                });
                let cancel = Arc::new(AtomicBool::new(false));
                let (c2, f2) = (cancel.clone(), flags.clone());
                let watch = tokio::spawn(async move {
                    loop {
                        if f2.cancel.load(Ordering::SeqCst) {
                            c2.store(true, Ordering::SeqCst);
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                });
                let span = crate::layers::effect(crate::layers::Layer::Accessibility, "accessibility.setup");
                let end = crate::wizard::drive(opts, cancel, move |p| t.update(id, |j| j.percent = p)).await;
                watch.abort();
                span.end_bool(matches!(end, crate::wizard::End::Done | crate::wizard::End::NeedsConsent(_)));
                match end {
                    crate::wizard::End::Done => {
                        t.update(id, |j| {
                            j.step = "done".into();
                            j.percent = Some(100.0);
                            j.message = "Installed.".into();
                        });
                        let core = t.core.clone();
                        let _ = crate::layers::blocking(move || core.apps.scan()).await;
                        t.core.hub.emit("apps.changed", json!({ "count": t.core.apps.count(), "delta": 1, "scanning": false }));
                        notice(t, true, &format!("{} is installed.", j.app));
                    }
                    crate::wizard::End::NeedsConsent(text) => {
                        t.update(id, |j| {
                            j.step = "waiting".into();
                            j.message = "Waiting for you to accept the licence.".into();
                        });
                        ask_licence(id, &j.app, &text).await;
                    }
                    crate::wizard::End::Cancelled => t.update(id, |j| {
                        j.step = "cancelled".into();
                        j.message = "Cancelled.".into();
                    }),
                    crate::wizard::End::Failed(e) => {
                        t.update(id, |j| {
                            j.step = "failed".into();
                            j.message = e.clone();
                        });
                        notice(t, false, &e);
                    }
                }
                return;
            }
            _ => return,
        }
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn percent_in(line: &str) -> Option<f64> {
    let b = line.as_bytes();
    let mut best = None;
    for (i, c) in b.iter().enumerate() {
        if *c == b'%' {
            let mut s = i;
            while s > 0 && (b[s - 1].is_ascii_digit() || b[s - 1] == b'.') {
                s -= 1;
            }
            if s < i
                && let Ok(v) = line[s..i].parse::<f64>()
                && (0.0..=100.0).contains(&v)
            {
                best = Some(v);
            }
        }
    }
    best
}

#[cfg(unix)]
fn run_tty(argv: &[String], env: &[(&str, &str)], flags: &Flags, on: &dyn Fn(f64)) -> Result<(), String> {
    use std::os::fd::FromRawFd;
    use std::os::unix::process::CommandExt;
    let (mut master, mut slave) = (0, 0);
    let ws = libc::winsize { ws_row: 24, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &ws) } != 0 {
        return Err("No terminal was available for the installer.".into());
    }
    let (prog, args) = argv.split_first().ok_or("empty command")?;
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args).envs(env.iter().copied());
    unsafe {
        cmd.stdin(std::process::Stdio::from_raw_fd(libc::dup(slave)));
        cmd.stdout(std::process::Stdio::from_raw_fd(libc::dup(slave)));
        cmd.stderr(std::process::Stdio::from_raw_fd(libc::dup(slave)));
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn();
    unsafe {
        libc::close(slave);
    }
    let mut child = child.map_err(|e| {
        unsafe {
            libc::close(master);
        }
        format!("The installer could not start: {e}")
    })?;
    let mut out = unsafe { std::fs::File::from_raw_fd(master) };
    let mut buf = [0u8; 4096];
    let mut tail = String::new();
    loop {
        if flags.cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
        }
        match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let s = String::from_utf8_lossy(&buf[..n]);
                for part in s.split(['\r', '\n']) {
                    if let Some(p) = percent_in(part) {
                        on(p);
                    }
                    if !part.trim().is_empty() {
                        tail = part.trim().chars().take(200).collect();
                    }
                }
            }
        }
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else if st.code() == Some(126) || st.code() == Some(127) {
        Err("The install was not approved.".into())
    } else {
        Err(if tail.is_empty() { "The installer failed.".into() } else { format!("The installer failed: {tail}") })
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn slug(s: &str) -> String {
    let v: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    v.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

#[cfg(target_os = "linux")]
fn install_appimage(app: &str, file: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = dirs::home_dir().ok_or("No home folder.")?.join("Applications");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(format!("{}.AppImage", if app.is_empty() { "App".to_owned() } else { app.replace('/', "-") }));
    if std::fs::rename(file, &dest).is_err() {
        std::fs::copy(file, &dest).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(file);
    }
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let apps = crate::xdg::data_home().join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    let entry = format!("[Desktop Entry]\nType=Application\nName={app}\nExec=\"{}\" %U\nTerminal=false\nCategories=Utility;\nX-Setu-Installed=true\n", dest.display());
    std::fs::write(apps.join(format!("setu-{}.desktop", slug(app))), entry).map_err(|e| e.to_string())?;
    Ok(dest)
}

async fn install_step(t: &'static Arc<Table>, id: u64, j: Job, flags: Arc<Flags>) -> Result<(), String> {
    let progress = move |p: f64| t.update(id, |j| j.percent = Some(p));
    #[cfg(windows)]
    let _ = &progress;
    if let Some(pkg) = j.package.clone().filter(|p| p.manager == "update") {
        let _ = pkg;
        #[cfg(target_os = "linux")]
        {
            let f2 = flags.clone();
            let p2 = progress;
            let user = crate::work::background("update", move || run_tty(&["flatpak".to_owned(), "update".into(), "--user".into(), "-y".into(), "--noninteractive".into()], &[], &f2, &p2)).await.map_err(|e| e.to_string())?;
            let _ = user;
            if crate::privileged::installed() {
                return crate::privileged::call("updates.apply", json!({}), Duration::from_secs(4 * 3600)).await.map(|_| ()).map_err(|e| e.to_string());
            }
            return Ok(());
        }
        #[cfg(windows)]
        {
            return crate::work::background("update", move || {
                let ok = std::process::Command::new("winget").args(["upgrade", "--all", "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"]).status().map(|s| s.success()).unwrap_or(false);
                if ok { Ok(()) } else { Err("Some apps couldn't be updated.".to_owned()) }
            })
            .await
            .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "macos")]
        {
            let brew = if Path::new("/opt/homebrew/bin/brew").exists() { "/opt/homebrew/bin/brew" } else { "brew" };
            let argv = vec![brew.to_owned(), "upgrade".into(), "--cask".into()];
            return crate::work::background("update", move || run_tty(&argv, &[], &flags, &progress)).await.map_err(|e| e.to_string())?;
        }
    }
    if let Some(pkg) = j.package.clone() {
        #[cfg(unix)]
        {
            let tty = matches!(pkg.manager.as_str(), "flatpak" | "dnf" | "apt" | "zypper" | "pacman" | "brew");
            if tty {
                let argv: Vec<String> = match pkg.manager.as_str() {
                    "flatpak" => vec!["flatpak".into(), "install".into(), format!("--{}", pkg.scope), "-y".into(), "--noninteractive".into(), pkg.source.clone(), pkg.id.clone()],
                    "dnf" => vec!["pkexec".into(), "dnf".into(), "install".into(), "-y".into(), pkg.id.clone()],
                    "apt" => vec!["pkexec".into(), "apt-get".into(), "install".into(), "-y".into(), pkg.id.clone()],
                    "zypper" => vec!["pkexec".into(), "zypper".into(), "--non-interactive".into(), "install".into(), pkg.id.clone()],
                    "pacman" => vec!["pkexec".into(), "pacman".into(), "-S".into(), "--noconfirm".into(), "--needed".into(), pkg.id.clone()],
                    _ => vec![if Path::new("/opt/homebrew/bin/brew").exists() { "/opt/homebrew/bin/brew".into() } else { "brew".into() }, "install".into(), "--cask".into(), pkg.id.clone()],
                };
                let name = pkg.name.clone();
                return crate::work::background("install", move || run_tty(&argv, &[("DEBIAN_FRONTEND", "noninteractive")], &flags, &progress).map_err(|e| if e.starts_with("The install") { e } else { format!("{name} could not be installed. {e}") }))
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let watch = stop.clone();
        let f2 = flags.clone();
        tokio::spawn(async move {
            while !watch.load(Ordering::SeqCst) {
                if f2.cancel.load(Ordering::SeqCst) {
                    watch.store(true, Ordering::SeqCst);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
        let r = crate::layers::blocking(move || crate::tools::install::install(&pkg, &stop)).await.map_err(|e| e.to_string())?;
        return r;
    }
    let file = j.file.clone().ok_or("The downloaded file is missing.")?;
    if !file.exists() {
        return Err("The downloaded file is missing.".into());
    }
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match kind_of(&name) {
        #[cfg(target_os = "linux")]
        "rpm" | "deb" => {
            if crate::privileged::installed() {
                return crate::privileged::call("package.install_file", json!({ "path": file }), Duration::from_secs(3600)).await.map(|_| ()).map_err(|e| e.to_string());
            }
            let tool = if name.ends_with(".rpm") { vec!["pkexec".to_owned(), "dnf".into(), "install".into(), "-y".into(), file.to_string_lossy().into_owned()] } else { vec!["pkexec".to_owned(), "apt-get".into(), "install".into(), "-y".into(), file.to_string_lossy().into_owned()] };
            crate::work::background("install", move || run_tty(&tool, &[("DEBIAN_FRONTEND", "noninteractive")], &flags, &progress)).await.map_err(|e| e.to_string())?
        }
        #[cfg(target_os = "linux")]
        "flatpak" => {
            let argv = vec!["flatpak".to_owned(), "install".into(), "--user".into(), "-y".into(), "--noninteractive".into(), file.to_string_lossy().into_owned()];
            crate::work::background("install", move || run_tty(&argv, &[], &flags, &progress)).await.map_err(|e| e.to_string())?
        }
        #[cfg(target_os = "linux")]
        "appimage" => {
            let app = j.app.clone();
            crate::layers::blocking(move || install_appimage(&app, &file)).await.map_err(|e| e.to_string())?.map(|dest| {
                t.update(id, |j| j.file = Some(dest));
            })
        }
        #[cfg(windows)]
        "msi" | "exe" => crate::layers::blocking(move || crate::tools::win::run_installer(&file, j.target.as_deref())).await.map_err(|e| e.to_string())?,
        #[cfg(target_os = "macos")]
        "dmg" | "pkg" => crate::layers::blocking(move || crate::tools::fetch::mac::install_file(&file)).await.map_err(|e| e.to_string())?,
        _ => Err(format!("{name} is downloaded, but it isn't an installer I know how to run. It's in {}.", file.parent().map(|p| p.display().to_string()).unwrap_or_default())),
    }
}

fn latest(pred: impl Fn(&Job) -> bool) -> Option<Job> {
    let t = table()?;
    t.jobs.lock().unwrap().iter().rev().find(|j| pred(j)).cloned()
}

fn says(j: &Job) -> String {
    let what = match j.step.as_str() {
        "download" if j.paused => "paused while downloading".to_owned(),
        "download" => match (j.percent, j.rate > 0.0 && j.total > j.bytes) {
            (Some(p), true) => format!("{}% downloaded, about {} left", p.round(), eta(((j.total - j.bytes) as f64 / j.rate) as u64)),
            (Some(p), false) => format!("{}% downloaded", p.round()),
            _ => format!("downloading, {} so far", crate::fileops::human(j.bytes)),
        },
        "verify" => "being checked".into(),
        "waiting" => "waiting for your yes to install".into(),
        "install" => match j.percent {
            Some(p) => format!("{}% installed", p.round()),
            None => "installing".into(),
        },
        "done" => "done".into(),
        "failed" => format!("stopped: {}", j.message),
        "cancelled" => "cancelled".into(),
        _ => "starting".into(),
    };
    format!("{} is {what}", j.app)
}

fn eta(secs: u64) -> String {
    match secs {
        0..=59 => "under a minute".into(),
        60..=3599 => {
            let m = secs.div_ceil(60);
            format!("{m} minute{}", if m == 1 { "" } else { "s" })
        }
        _ => {
            let h = secs / 3600;
            format!("{h} hour{}", if h == 1 { "" } else { "s" })
        }
    }
}

pub fn rules(t: &str) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    match t {
        "what are you doing" | "what are you working on" | "what jobs are running" | "show my downloads" | "show downloads" | "what's downloading" | "any downloads" => return act("jobs.status", json!({})),
        "how much is done" | "how far along is it" | "how much is left" | "how's the download" | "how is the download going" | "how long will it take" | "is it done" | "how much is downloaded" => return act("jobs.status", json!({ "one": true })),
        "pause the download" | "pause downloading" | "pause the downloads" | "hold the download" => return act("jobs.pause", json!({})),
        "resume the download" | "continue the download" | "resume downloading" | "resume the downloads" => return act("jobs.resume", json!({})),
        "cancel the download" | "stop the download" | "cancel the install" | "stop the install" | "cancel installing" | "cancel the installation" => return act("jobs.cancel", json!({})),
        "install it anyway" | "yes install it" | "install it now" | "go ahead and install it" => return act("jobs.trust", json!({})),
        "click through the installer" | "go through the installer" | "finish the installer" | "finish the setup" | "go through the setup" | "complete the setup" | "complete the installation" | "finish the installation" | "click through the setup" | "do the setup for me" | "run through the setup" | "click next until it's done" | "click next until it is done" => return act("jobs.setup", json!({})),
        "delete installers after installing" | "don't keep installers" | "delete the installer after installing" => return act("jobs.keep", json!({ "keep": false })),
        "keep installers" | "keep the installers" | "keep installer files" => return act("jobs.keep", json!({ "keep": true })),
        _ => {}
    }
    if matches!(t, "update my apps" | "update all my apps" | "update apps" | "update all apps" | "update my programs" | "update everything") {
        return act("jobs.update", json!({}));
    }
    if matches!(t, "what folders are there" | "show me the folders" | "browse folders" | "which folders are there" | "show the folders") {
        return act("jobs.browse", json!({}));
    }
    for p in ["is ", "do i have "] {
        if let Some(r) = t.strip_prefix(p) {
            for s in [" installed", " installed on my computer", " on my computer", " installed here"] {
                if let Some(app) = r.strip_suffix(s) {
                    return act("jobs.installed", json!({ "app": app.trim() }));
                }
            }
        }
    }
    for p in ["uninstall the app ", "uninstall the ", "uninstall ", "remove the app ", "delete the app "] {
        if let Some(r) = t.strip_prefix(p)
            && !r.ends_with(" extension")
            && !r.ends_with(" plugin")
            && !r.ends_with(" extensions")
        {
            let app = r.trim_end_matches(" app").trim_end_matches(" application").trim();
            if !app.is_empty() {
                return act("jobs.uninstall", json!({ "app": app }));
            }
        }
    }
    for p in ["save downloads in ", "save the next download in ", "save new downloads in ", "put downloads in ", "save my downloads in "] {
        if let Some(r) = t.strip_prefix(p) {
            return act("jobs.folder", json!({ "where": r.trim(), "kind": "next" }));
        }
    }
    for p in ["save it in ", "save it to ", "save it on ", "put it in ", "put it on ", "download it to ", "download it into ", "save the download in "] {
        if let Some(r) = t.strip_prefix(p) {
            return act("jobs.folder", json!({ "where": r.trim(), "kind": "download" }));
        }
    }
    for p in ["install it in ", "install it into ", "install it on ", "install it to "] {
        if let Some(r) = t.strip_prefix(p) {
            return act("jobs.folder", json!({ "where": r.trim(), "kind": "install" }));
        }
    }
    for p in ["download ", "get "] {
        if let Some(r) = t.strip_prefix(p) {
            for s in [" from the website", " from its website", " from the official website", " from the web", " from their website"] {
                if let Some(app) = r.strip_suffix(s) {
                    return act("jobs.web", json!({ "app": app.trim() }));
                }
            }
        }
    }
    None
}

pub fn url_rule(raw: &str) -> Option<Intent> {
    let r = raw.trim();
    let lower = r.to_lowercase();
    for (p, install) in [("download and install ", true), ("install from ", true), ("install ", true), ("download ", false), ("get ", false)] {
        if lower.starts_with(p) {
            let url = r[p.len()..].trim();
            if (url.starts_with("https://") || url.starts_with("http://")) && !url.contains(char::is_whitespace) {
                let host = host_of(url);
                return Some(Intent::Act { tool: "jobs.url".into(), args: json!({ "url": url, "site": host, "hosts": [host], "install": install }) });
            }
        }
    }
    None
}

pub struct Site {
    pub app: &'static str,
    pub site: &'static str,
    pub hosts: &'static [&'static str],
    pub linux_rpm: &'static str,
    pub linux_deb: &'static str,
    pub windows: &'static str,
    pub macos: &'static str,
}

pub const SITES: &[Site] = &[
    Site { app: "Steam", site: "steampowered.com", hosts: &["steampowered.com", "steamstatic.com", "steamcontent.com"], linux_rpm: "", linux_deb: "https://cdn.fastly.steamstatic.com/client/installer/steam.deb", windows: "https://cdn.fastly.steamstatic.com/client/installer/SteamSetup.exe", macos: "https://cdn.fastly.steamstatic.com/client/installer/steam.dmg" },
    Site { app: "Google Chrome", site: "google.com", hosts: &["google.com", "gstatic.com"], linux_rpm: "https://dl.google.com/linux/direct/google-chrome-stable_current_x86_64.rpm", linux_deb: "https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb", windows: "https://dl.google.com/chrome/install/latest/chrome_installer.exe", macos: "https://dl.google.com/chrome/mac/universal/stable/GGRO/googlechrome.dmg" },
    Site { app: "Zoom", site: "zoom.us", hosts: &["zoom.us", "zoom.com"], linux_rpm: "https://zoom.us/client/latest/zoom_x86_64.rpm", linux_deb: "https://zoom.us/client/latest/zoom_amd64.deb", windows: "https://zoom.us/client/latest/ZoomInstallerFull.msi", macos: "https://zoom.us/client/latest/Zoom.pkg" },
    Site { app: "Discord", site: "discord.com", hosts: &["discord.com", "discordapp.net", "discordapp.com"], linux_rpm: "", linux_deb: "https://discord.com/api/download?platform=linux&format=deb", windows: "https://discord.com/api/download?platform=win", macos: "https://discord.com/api/download?platform=osx" },
    Site { app: "VS Code", site: "code.visualstudio.com", hosts: &["visualstudio.com", "microsoft.com"], linux_rpm: "https://code.visualstudio.com/sha/download?build=stable&os=linux-rpm-x64", linux_deb: "https://code.visualstudio.com/sha/download?build=stable&os=linux-deb-x64", windows: "https://code.visualstudio.com/sha/download?build=stable&os=win32-x64-user", macos: "https://code.visualstudio.com/sha/download?build=stable&os=darwin-universal" },
];

#[cfg(target_os = "linux")]
fn rpm_system() -> bool {
    crate::tools::install::native_manager().is_some_and(|m| m == "dnf" || m == "zypper")
}

#[cfg(not(target_os = "linux"))]
fn rpm_system() -> bool {
    false
}

pub fn site_for(app: &str) -> Option<(&'static Site, &'static str)> {
    let want = crate::text::normalize(app).replace(' ', "");
    let s = SITES.iter().find(|s| {
        let n = crate::text::normalize(s.app).replace(' ', "");
        n == want || n.replace("google", "") == want || (want == "vscode" && s.app == "VS Code") || (want == "visualstudiocode" && s.app == "VS Code")
    })?;
    let url = if cfg!(windows) {
        s.windows
    } else if cfg!(target_os = "macos") {
        s.macos
    } else if rpm_system() && !s.linux_rpm.is_empty() {
        s.linux_rpm
    } else {
        s.linux_deb
    };
    (!url.is_empty()).then_some((s, url))
}

fn pick(args: &Value, pred: impl Fn(&Job) -> bool) -> Option<Job> {
    match args.get("id").and_then(Value::as_u64) {
        Some(id) => table().and_then(|t| t.get(id)),
        None => latest(pred),
    }
}

pub async fn run_tool(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let ok = |did: &str, reply: String| Outcome::ok_with(did, LAYER, None).with_reply(reply);
    let Some(t) = table() else { return Outcome::fail("Background jobs aren't running.", LAYER) };
    match tool {
        "jobs.status" => {
            let list: Vec<Job> = t.jobs.lock().unwrap().iter().filter(|j| !finished(&j.step) || now() - j.updated < 120_000).cloned().collect();
            if list.is_empty() {
                return ok("Nothing running", "I'm not downloading or installing anything right now.".into());
            }
            if args.get("one").and_then(Value::as_bool) == Some(true)
                && let Some(j) = list.iter().rev().find(|j| !finished(&j.step)).or(list.last())
            {
                return ok("Read the progress", format!("{}.", says(j)));
            }
            ok(&format!("{} jobs", list.len()), format!("{}.", list.iter().map(says).collect::<Vec<_>>().join(". ")))
        }
        "jobs.pause" => {
            let Some(j) = pick(args, |j| j.step == "download" && !j.paused) else { return Outcome::fail("Nothing is downloading.", LAYER) };
            t.flags(j.id).pause.store(true, Ordering::SeqCst);
            ok("Paused the download", format!("Paused the {} download. Say resume the download to carry on.", j.app))
        }
        "jobs.resume" => {
            let Some(j) = pick(args, |j| j.paused) else { return Outcome::fail("No download is paused.", LAYER) };
            t.flags(j.id).pause.store(false, Ordering::SeqCst);
            t.update(j.id, |j| {
                j.paused = false;
                j.message.clear();
            });
            spawn(j.id);
            ok("Resumed the download", format!("Carrying on with {}.", j.app))
        }
        "jobs.cancel" => {
            let Some(j) = pick(args, |j| !finished(&j.step)) else { return Outcome::fail("Nothing is downloading or installing.", LAYER) };
            t.flags(j.id).cancel.store(true, Ordering::SeqCst);
            if j.step == "waiting" || j.paused || j.step == "verify" {
                if let Some(f) = j.file.clone() {
                    let _ = std::fs::remove_file(f);
                }
                t.update(j.id, |j| {
                    j.step = "cancelled".into();
                    j.message = "Cancelled.".into();
                });
            }
            ok("Cancelled", format!("Cancelled {}.", j.app))
        }
        "jobs.trust" => {
            let Some(j) = pick(args, |j| j.step == "waiting") else { return Outcome::fail("Nothing is waiting to be installed.", LAYER) };
            take_ask(j.id);
            t.update(j.id, |j| {
                j.trusted = true;
                j.step = "install".into();
                j.message = "Installing.".into();
            });
            spawn(j.id);
            ok(&format!("Installing {}", j.app), format!("Installing {}. I'll tell you when it's ready.", j.app))
        }
        "jobs.setup" => {
            let win = match crate::os::working().await {
                Ok(w) => w,
                Err(e) => return Outcome::fail(&e.to_string(), "accessibility"),
            };
            let app = crate::wizard::app_from_title(&win.title);
            match start_setup(&app, win.pid, &win.title) {
                Some(_) => ok(&format!("Going through the {app} setup"), format!("Going through the {app} setup. I'll ask before accepting anything and tell you when it's done.")),
                None => ok("Already going through it", format!("I'm already going through the {app} setup.")),
            }
        }
        "jobs.accept" => {
            let Some(j) = pick(args, |j| j.step == "waiting" && j.wizard.is_some()) else { return Outcome::fail("No setup is waiting for a licence.", LAYER) };
            take_ask(j.id);
            LICENCES.lock().unwrap().remove(&j.id);
            t.update(j.id, |j| {
                if let Some(w) = j.wizard.as_mut() {
                    w.consent = true;
                }
                j.step = "setup".into();
                j.message = "Accepted the licence.".into();
            });
            spawn(j.id);
            ok("Accepted the licence", format!("Accepted. Carrying on with the {} setup.", j.app))
        }
        "jobs.licence" => {
            let Some(j) = pick(args, |j| j.step == "waiting" && j.wizard.is_some()) else { return Outcome::fail("No setup is waiting for a licence.", LAYER) };
            let text = LICENCES.lock().unwrap().get(&j.id).cloned().unwrap_or_default();
            let short: String = text.chars().take(1500).collect();
            let more = if text.chars().count() > 1500 { " That's the start of it; the rest is in the setup window." } else { "" };
            let c = core.clone();
            let (id, app) = (j.id, j.app.clone());
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let _ = &c;
                ask_licence(id, &app, &text).await;
            });
            ok("Read the licence", if short.trim().is_empty() { "The setup window doesn't show the licence as text I can read. It's in the window.".into() } else { format!("{short}{more}") })
        }
        "jobs.ask" => {
            let id = args.get("id").and_then(Value::as_u64).unwrap_or(0);
            match take_ask(id) {
                Some(f) => Outcome::asking("Asked to install", LAYER, f),
                None => ok("Nothing to ask", String::new()),
            }
        }
        "jobs.keep" => {
            let keep = args.get("keep").and_then(Value::as_bool).unwrap_or(true);
            *t.keep.lock().unwrap() = Some(keep);
            t.save();
            ok("Saved the choice", if keep { "I'll keep installer files after installing.".into() } else { "I'll delete installer files after installing.".into() })
        }
        "jobs.folder" => {
            let place = args.get("where").and_then(Value::as_str).unwrap_or_default().to_owned();
            let kind = args.get("kind").and_then(Value::as_str).unwrap_or("download");
            let p = place.clone();
            let found = crate::layers::blocking(move || crate::tools::files::location(&p).or_else(|| crate::tools::files::search(&p, Some(true), None, Duration::from_millis(2500)).into_iter().next().map(|h| h.path))).await.ok().flatten();
            let Some(dir) = found else { return Outcome::fail(&format!("I couldn't find a folder called {place}."), LAYER) };
            let shown = crate::tools::files::spoken_path(&dir);
            if kind == "install" {
                if cfg!(target_os = "linux") {
                    return ok("Install folder", "On Linux, apps install into the system's own folders, so there's no folder to choose.".into());
                }
                if let Some(j) = latest(|j| !finished(&j.step) && j.step != "install") {
                    t.update(j.id, |x| x.target = Some(dir.clone()));
                    return ok("Install folder set", format!("{} will install into {shown}.", j.app));
                }
                return Outcome::fail("Nothing is waiting to be installed.", LAYER);
            }
            if kind == "next" {
                *DEFAULT_FOLDER.lock().unwrap() = Some(dir);
                return ok("Download folder set", format!("I'll save the next download in {shown}."));
            }
            if let Some(j) = latest(|j| j.step == "download" && j.bytes == 0) {
                t.update(j.id, |x| x.folder = Some(dir.clone()));
                return ok("Download folder set", format!("Saving {} in {shown}.", j.app));
            }
            if let Some(j) = latest(|j| !finished(&j.step) && j.file.is_some()).or_else(|| latest(|j| j.step == "done" && j.file.is_some() && !j.install)) {
                let from = j.file.clone().unwrap_or_default();
                let to = dir.join(from.file_name().unwrap_or_default());
                if j.step == "download" {
                    t.update(j.id, |x| x.folder = Some(dir.clone()));
                    return ok("Download folder set", format!("{} will be moved to {shown} when it finishes.", j.app));
                }
                return match std::fs::rename(&from, &to).or_else(|_| std::fs::copy(&from, &to).map(|_| ()).and_then(|_| std::fs::remove_file(&from))) {
                    Ok(()) => {
                        t.update(j.id, |x| x.file = Some(to.clone()));
                        ok("Moved the download", format!("Moved {} to {shown}.", j.app))
                    }
                    Err(e) => Outcome::fail(&e.to_string(), LAYER),
                };
            }
            *DEFAULT_FOLDER.lock().unwrap() = Some(dir);
            ok("Download folder set", format!("I'll save the next download in {shown}."))
        }
        "jobs.web" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or_default();
            let Some((s, url)) = site_for(app) else { return Outcome::fail(&format!("I don't know {app}'s official download page. Open it in the browser and say download this."), LAYER) };
            let yes = Intent::Act { tool: "jobs.url".into(), args: json!({ "app": s.app, "url": url, "site": s.site, "hosts": s.hosts, "install": true }) };
            let no = Intent::Act { tool: "jobs.none".into(), args: json!({}) };
            let q = format!("I'll download {} from {}, its maker's site, then ask before installing. Go ahead?", s.app, s.site);
            Outcome::asking("Asked to download", LAYER, FollowUp { question: q.clone(), spoken: Some(q), choices: vec![Choice::new("Download", &["yes", "go ahead", "ok", "okay", "sure", "download it"], yes), Choice::new("Not now", &["no", "not now", "cancel"], no)], cleanup: None, kind: "confirm", focus: None })
        }
        "jobs.url" => {
            let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
            let url = s("url");
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Outcome::fail("That isn't a web address I can download.", LAYER);
            }
            let hosts: Vec<String> = args.get("hosts").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            let install = args.get("install").and_then(Value::as_bool).unwrap_or(false);
            let folder = args.get("folder").and_then(Value::as_str).map(PathBuf::from).or_else(|| DEFAULT_FOLDER.lock().unwrap().take());
            let app = if s("app").is_empty() {
                let n = file_name(&url, "file");
                Path::new(&n).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or(n)
            } else {
                s("app")
            };
            let site = if s("site").is_empty() { host_of(&url) } else { s("site") };
            let sha = args.get("sha256").and_then(Value::as_str).map(str::to_owned);
            let _ = core;
            match start_url(&app, &url, &site, hosts, sha, install, folder) {
                Some(_) => ok(&format!("Downloading {app}"), format!("Downloading {app} from {site}. You can keep talking to me; I'll tell you when it's done.")),
                None => Outcome::fail("Background jobs aren't running.", LAYER),
            }
        }
        "jobs.none" => ok("Left it", "Okay, I left it.".into()),
        "jobs.installed" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or_default();
            let found = core.apps.find(app, None, &core.defaults());
            match found.into_iter().find(|(_, sc)| *sc >= 0.85) {
                Some((a, _)) => ok(&format!("{} is installed", a.name), format!("Yes, {} is installed.", a.name)),
                None => ok(&format!("{app} isn't installed"), format!("No, {} isn't installed. Say install {app} to get it.", crate::text::sentence_case(app))),
            }
        }
        "jobs.update" => {
            let pkg = Package { manager: "update".into(), id: "setu-update-all".into(), name: "your apps".into(), source: String::new(), publisher: String::new(), scope: String::new(), size: String::new() };
            match start_package(pkg) {
                Some(_) => ok("Updating your apps", "Updating your apps in the background. I'll tell you when they're done.".into()),
                None => ok("Already updating", "I'm already updating your apps.".into()),
            }
        }
        "jobs.uninstall" => uninstall(core, args.get("app").and_then(Value::as_str).unwrap_or_default()).await,
        "jobs.browse" => {
            let start = args.get("path").and_then(Value::as_str).map(PathBuf::from).or_else(|| latest(|j| !finished(&j.step)).and_then(|j| j.folder)).or_else(|| DEFAULT_FOLDER.lock().unwrap().clone()).unwrap_or_else(|| dirs::home_dir().unwrap_or_default());
            browse(&start)
        }
        "jobs.pickfolder" => {
            let p = args.get("path").and_then(Value::as_str).unwrap_or_default().to_owned();
            Box::pin(run_tool(core, "jobs.folder", &json!({ "where": p, "kind": "download" }))).await
        }
        _ => Outcome::fail("I don't know that job command.", LAYER),
    }
}

static DEFAULT_FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);

fn browse(dir: &Path) -> Outcome {
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect()).unwrap_or_default();
    subs.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
    subs.truncate(24);
    let shown = crate::tools::files::spoken_path(dir);
    let mut choices = vec![Choice::new("Use this folder", &["here", "use this folder", "this folder", "save it here", "use it", "choose this folder"], Intent::Act { tool: "jobs.pickfolder".into(), args: json!({ "path": dir }) })];
    if let Some(parent) = dir.parent() {
        choices.push(Choice::new("Up a folder", &["back", "go back", "up a folder", "parent folder", "go up a folder"], Intent::Act { tool: "jobs.browse".into(), args: json!({ "path": parent }) }));
    }
    for s in &subs {
        let name = s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut c = Choice::new(&name, &[], Intent::Act { tool: "jobs.browse".into(), args: json!({ "path": s }) });
        c.words = vec![crate::text::normalize(&name), format!("open {}", crate::text::normalize(&name))];
        choices.push(c);
    }
    let names: Vec<String> = subs.iter().take(6).filter_map(|s| s.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
    let spoken = if names.is_empty() { format!("{shown} has no folders inside. Say here to use it, or back.") } else { format!("In {shown}: {}. Say a name to open it, here to use this folder, or back.", names.join(", ")) };
    Outcome::asking("Showed the folders", LAYER, FollowUp { question: format!("Folders in {shown}"), spoken: Some(spoken), choices, cleanup: None, kind: "folders", focus: Some(if dir.parent().is_some() { 2.min(subs.len() + 1) } else { 1 }) })
}

async fn uninstall(core: &Arc<Core>, name: &str) -> Outcome {
    let found = core.apps.find(name, None, &core.defaults());
    let Some((app, _)) = found.into_iter().find(|(_, sc)| *sc >= 0.8) else { return Outcome::fail(&format!("{} isn't installed.", crate::text::sentence_case(name)), LAYER) };
    let shown = app.name.clone();
    let a = app.clone();
    let r = crate::work::background("uninstall", move || remove_app(&a)).await.map_err(|e| e.to_string()).and_then(|r| r);
    let r = match r {
        Err(e) if e.starts_with("package:") => {
            let pkg = e.trim_start_matches("package:").to_owned();
            if crate::privileged::installed() {
                crate::privileged::call("package.remove", json!({ "name": pkg }), Duration::from_secs(1800)).await.map(|_| ()).map_err(|e| e.to_string())
            } else {
                let argv = ["pkexec".to_owned(), crate::tools::install::native_manager_bin(), "remove".into(), "-y".into(), pkg];
                crate::work::background("uninstall", move || {
                    let o = std::process::Command::new(&argv[0]).args(&argv[1..]).output();
                    match o {
                        Ok(o) if o.status.success() => Ok(()),
                        Ok(o) if matches!(o.status.code(), Some(126) | Some(127)) => Err("The uninstall was not approved.".to_owned()),
                        Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("The uninstall failed.").to_owned()),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
            }
        }
        other => other,
    };
    match r {
        Ok(()) => {
            let c = core.clone();
            let _ = crate::layers::blocking(move || c.apps.scan()).await;
            Outcome::ok_with(&format!("Uninstalled {shown}"), LAYER, None).with_reply(format!("{shown} is uninstalled."))
        }
        Err(e) => Outcome::fail(&e, LAYER),
    }
}

#[cfg(target_os = "linux")]
fn remove_app(app: &crate::tools::apps::App) -> Result<(), String> {
    let path = PathBuf::from(&app.path);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if text.contains("X-Setu-Installed=true") {
        let entry = crate::xdg::ini(&text, "Desktop Entry");
        let exec = entry.get("Exec").cloned().unwrap_or_default();
        let bin = exec.split('"').nth(1).map(PathBuf::from);
        if let Some(b) = bin.filter(|b| b.extension().is_some_and(|e| e.eq_ignore_ascii_case("appimage"))) {
            let _ = std::fs::remove_file(b);
        }
        return std::fs::remove_file(&path).map_err(|e| e.to_string());
    }
    let p = path.to_string_lossy();
    if p.contains("/flatpak/exports/") {
        let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let scope = if dirs::home_dir().is_some_and(|h| path.starts_with(h)) { "--user" } else { "--system" };
        let o = std::process::Command::new("flatpak").args(["uninstall", scope, "-y", "--noninteractive", &id]).output().map_err(|e| e.to_string())?;
        return if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("Flatpak couldn't remove it.").to_owned()) };
    }
    let owner = std::process::Command::new("rpm").args(["-qf", "--qf", "%{NAME}"]).arg(&path).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).or_else(|| {
        std::process::Command::new("dpkg").arg("-S").arg(&path).output().ok().filter(|o| o.status.success()).and_then(|o| String::from_utf8_lossy(&o.stdout).split(':').next().map(|s| s.trim().to_owned()))
    });
    match owner {
        Some(pkg) if !pkg.is_empty() => Err(format!("package:{pkg}")),
        _ => Err(format!("I couldn't tell how {} was installed.", app.name)),
    }
}

#[cfg(windows)]
fn remove_app(app: &crate::tools::apps::App) -> Result<(), String> {
    let o = std::process::Command::new("winget").args(["uninstall", "--name", &app.name, "--silent", "--accept-source-agreements", "--disable-interactivity"]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(format!("{} couldn't be uninstalled.", app.name)) }
}

#[cfg(target_os = "macos")]
fn remove_app(app: &crate::tools::apps::App) -> Result<(), String> {
    let p = PathBuf::from(&app.path);
    if !p.extension().is_some_and(|e| e == "app") {
        return Err(format!("I couldn't tell how {} was installed.", app.name));
    }
    let script = format!("tell application \"Finder\" to delete POSIX file {}", serde_json::to_string(&app.path).unwrap_or_default());
    let ok = std::process::Command::new("osascript").args(["-e", &script]).status().map(|s| s.success()).unwrap_or(false);
    if ok { Ok(()) } else { Err(format!("{} couldn't be moved to the Trash.", app.name)) }
}

pub fn halt_all() -> usize {
    let Some(t) = table() else { return 0 };
    let ids: Vec<u64> = t.jobs.lock().unwrap().iter().filter(|j| j.step == "download" && !j.paused).map(|j| j.id).collect();
    for id in &ids {
        t.flags(*id).pause.store(true, Ordering::SeqCst);
    }
    ids.len()
}

pub fn label(tool: &str, args: &Value) -> String {
    let _ = args;
    match tool {
        "jobs.status" => "Checking the jobs".into(),
        "jobs.pause" => "Pausing the download".into(),
        "jobs.resume" => "Resuming the download".into(),
        "jobs.cancel" => "Cancelling".into(),
        "jobs.trust" => "Installing".into(),
        "jobs.folder" => "Choosing the folder".into(),
        "jobs.setup" => "Going through the setup".into(),
        "jobs.accept" => "Accepting the licence".into(),
        "jobs.licence" => "Reading the licence".into(),
        "jobs.web" | "jobs.url" => "Starting the download".into(),
        _ => "Jobs".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_types_and_systems() {
        assert_eq!(kind_of("steam.deb"), "deb");
        assert_eq!(kind_of("Some.App-1.2.AppImage"), "appimage");
        assert_eq!(kind_of("SteamSetup.exe"), "exe");
        assert_eq!(fits_here("exe"), cfg!(windows));
        assert_eq!(fits_here("appimage"), cfg!(target_os = "linux"));
        assert_eq!(file_name("https://x.example/dl/steam.deb?x=1", "Steam"), "steam.deb");
        assert_eq!(file_name("https://discord.com/api/download?platform=linux", "Discord"), "Discord-download");
    }

    #[test]
    fn downloads_must_come_from_the_named_site() {
        let hosts = vec!["steamstatic.com".to_owned()];
        assert!(host_allowed(&host_of("https://cdn.fastly.steamstatic.com/a.deb"), &hosts));
        assert!(!host_allowed(&host_of("https://evil.example/steamstatic.com/a.deb"), &hosts));
        assert!(!host_allowed(&host_of("https://steamstatic.com.evil.example/a.deb"), &hosts));
        assert!(host_allowed("anything.example", &[]));
    }

    #[test]
    fn progress_is_read_from_installer_output() {
        assert_eq!(percent_in("Installing… 45%"), Some(45.0));
        assert_eq!(percent_in("[====      ]  37.5%  1.2 MB/s"), Some(37.5));
        assert_eq!(percent_in("no number here"), None);
        assert_eq!(percent_in("rate 900%"), None);
    }

    #[test]
    fn job_phrases() {
        let tool = |t: &str| match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        };
        assert_eq!(tool("how much is done").unwrap().0, "jobs.status");
        assert_eq!(tool("pause the download").unwrap().0, "jobs.pause");
        assert_eq!(tool("save it in documents").unwrap().1["where"], "documents");
        assert_eq!(tool("install it in d games").unwrap().1["kind"], "install");
        assert_eq!(tool("download steam from the website").unwrap().1["app"], "steam");
        assert!(site_for("steam").is_some() && site_for("vs code").is_some() && site_for("no such app").is_none());
    }
}
