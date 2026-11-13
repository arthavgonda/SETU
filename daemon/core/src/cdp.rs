use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub url: String,
}

pub struct Cdp {
    pub key: String,
    out: tokio::sync::mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    next: AtomicU64,
    sessions: Mutex<HashMap<String, String>>,
    alive: std::sync::atomic::AtomicBool,
    events: tokio::sync::broadcast::Sender<Value>,
}

static LIVE: Mutex<Vec<Arc<Cdp>>> = Mutex::new(Vec::new());

pub fn connected(key: &str) -> Option<Arc<Cdp>> {
    let mut g = LIVE.lock().unwrap();
    g.retain(|c| c.alive.load(Ordering::SeqCst));
    g.iter().find(|c| c.key == key).cloned()
}

pub fn find_tab<'a>(tabs: &'a [Tab], target: &str) -> Vec<&'a Tab> {
    let t = crate::text::normalize(target.trim_end_matches(" tab").trim());
    if t.is_empty() {
        return Vec::new();
    }
    let site = crate::tools::web::site(&t).map(|s| crate::text::normalize(s.name));
    tabs.iter()
        .filter(|tab| {
            let title = crate::text::normalize(&tab.title);
            let host = crate::text::normalize(&crate::tools::web::host(&tab.url));
            title.contains(&t) || host.contains(&t.replace(' ', "")) || site.as_ref().is_some_and(|s| title.contains(s) || host.contains(&s.replace(' ', "")))
        })
        .collect()
}

pub fn any() -> Option<Arc<Cdp>> {
    let mut g = LIVE.lock().unwrap();
    g.retain(|c| c.alive.load(Ordering::SeqCst));
    g.first().cloned()
}

pub fn profile_dir(data: &Path, key: &str) -> PathBuf {
    data.join("browsers").join(key)
}

pub fn has_profile(data: &Path, key: &str) -> bool {
    profile_dir(data, key).join(".setu-profile").exists()
}

pub fn declined(data: &Path, key: &str) -> bool {
    data.join("browsers").join(format!("{key}.declined")).exists()
}

pub fn decline(data: &Path, key: &str) {
    let d = data.join("browsers");
    let _ = std::fs::create_dir_all(&d);
    let _ = std::fs::write(d.join(format!("{key}.declined")), b"");
}

pub fn create_profile(data: &Path, key: &str) -> Result<PathBuf> {
    let dir = profile_dir(data, key);
    std::fs::create_dir_all(&dir)?;
    let mark = json!({ "created": setu_common::protocol::now_ms(), "key": key });
    std::fs::write(dir.join(".setu-profile"), mark.to_string())?;
    let _ = std::fs::remove_file(data.join("browsers").join(format!("{key}.declined")));
    Ok(dir)
}

pub fn is_default(data: &Path, key: &str) -> bool {
    has_profile(data, key) && profile_dir(data, key).join(".setu-default").exists()
}

#[cfg(not(windows))]
fn override_path(app: &crate::tools::apps::App) -> Option<PathBuf> {
    let name = Path::new(&app.path).file_name()?;
    Some(dirs::data_dir()?.join("applications").join(name))
}

pub fn make_default(data: &Path, key: &str, app: &crate::tools::apps::App, me: &Path) -> Result<Option<PathBuf>> {
    std::fs::write(profile_dir(data, key).join(".setu-default"), b"")?;
    #[cfg(windows)]
    {
        let exe = exe_of(&app.exec, &app.path);
        let n = crate::tools::win::retarget_shortcuts(&exe, me, key, &data.join("browsers").join(format!("{key}.shortcuts")))?;
        log::info!("pointed {n} {key} shortcuts at the Setu profile");
        Ok(None)
    }
    #[cfg(not(windows))]
    desktop_override(key, app, me)
}

#[cfg(not(windows))]
fn desktop_override(key: &str, app: &crate::tools::apps::App, me: &Path) -> Result<Option<PathBuf>> {
    if !cfg!(target_os = "linux") || !app.path.ends_with(".desktop") {
        return Ok(None);
    }
    if app.exec.contains("flatpak ") || app.path.contains("/flatpak/") {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&app.path)?;
    if text.contains(" --browser ") {
        return Ok(override_path(app));
    }
    let exe = exe_of(&app.exec, &app.path);
    let mut out = String::new();
    for line in text.lines() {
        match line.strip_prefix("Exec=") {
            Some(cmd) => {
                let mut words = cmd.split_whitespace().peekable();
                let mut skipped = false;
                while let Some(w) = words.peek() {
                    let w = w.trim_matches('"');
                    let is_program = !skipped && (w == "env" || w.ends_with("/env") || w.contains('=') && !w.starts_with('-') || Path::new(w) == exe || crate::tools::apps::which(w).as_deref() == Some(exe.as_path()) || Path::new(w).file_name() == exe.file_name());
                    if !is_program {
                        break;
                    }
                    if !(w == "env" || w.ends_with("/env") || w.contains('=')) {
                        skipped = true;
                    }
                    words.next();
                }
                let rest: Vec<&str> = words.collect();
                out.push_str(&format!("Exec=\"{}\" --browser {key} --exe \"{}\" {}\n", me.display(), exe.display(), rest.join(" ")));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    let path = override_path(app).ok_or_else(|| anyhow!("No applications folder."))?;
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&path, out)?;
    Ok(Some(path))
}

pub fn unmake_default(data: &Path, key: &str, app: &crate::tools::apps::App) -> Result<()> {
    let _ = std::fs::remove_file(profile_dir(data, key).join(".setu-default"));
    #[cfg(windows)]
    {
        let _ = app;
        crate::tools::win::restore_shortcuts(&data.join("browsers").join(format!("{key}.shortcuts")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if let Some(p) = override_path(app)
            && std::fs::read_to_string(&p).is_ok_and(|t| t.contains(" --browser "))
        {
            std::fs::remove_file(p)?;
        }
        Ok(())
    }
}

pub fn flags() -> Vec<String> {
    let mut v = vec!["--force-renderer-accessibility".to_owned()];
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
        v.push("--ozone-platform=wayland".to_owned());
    }
    if let Ok(extra) = std::env::var("SETU_BROWSER_FLAGS") {
        v.extend(extra.split_whitespace().map(str::to_owned));
    }
    v
}

impl Cdp {
    fn handle(&self, msg: &str) {
        let Ok(v) = serde_json::from_str::<Value>(msg) else { return };
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                let _ = tx.send(v);
            }
            return;
        }
        let _ = self.events.send(v.clone());
        if v.get("method").and_then(Value::as_str) == Some("Target.detachedFromTarget")
            && let Some(sid) = v.pointer("/params/sessionId").and_then(Value::as_str)
        {
            self.sessions.lock().unwrap().retain(|_, s| s != sid);
        }
    }

    pub async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(anyhow!("The browser closed."));
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.out.send(msg.to_string()).map_err(|_| anyhow!("The browser closed."))?;
        let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.cdp");
        let r = match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(v)) => match v.get("error") {
                Some(e) => Err(anyhow!("{}", e.get("message").and_then(Value::as_str).unwrap_or("The browser refused that."))),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            },
            Ok(Err(_)) => Err(anyhow!("The browser closed.")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(anyhow!("The browser didn't answer in time."))
            }
        };
        span.end(r)
    }

    pub async fn tabs(&self) -> Result<Vec<Tab>> {
        let r = self.call("Target.getTargets", json!({}), None).await?;
        Ok(r.get("targetInfos")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
                    .map(|t| Tab {
                        id: t.get("targetId").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        title: t.get("title").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        url: t.get("url").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn session(&self, target: &str) -> Result<String> {
        if let Some(s) = self.sessions.lock().unwrap().get(target).cloned() {
            return Ok(s);
        }
        let r = self.call("Target.attachToTarget", json!({ "targetId": target, "flatten": true }), None).await?;
        let sid = r.get("sessionId").and_then(Value::as_str).ok_or_else(|| anyhow!("The browser didn't open that tab for control."))?.to_owned();
        self.sessions.lock().unwrap().insert(target.to_owned(), sid.clone());
        Ok(sid)
    }

    pub async fn activate(&self, target: &str) -> Result<()> {
        self.call("Target.activateTarget", json!({ "targetId": target }), None).await.map(|_| ())
    }

    pub async fn close(&self, target: &str) -> Result<()> {
        self.call("Target.closeTarget", json!({ "targetId": target }), None).await.map(|_| ())
    }

    pub async fn open(&self, url: &str) -> Result<String> {
        let r = self.call("Target.createTarget", json!({ "url": url }), None).await?;
        r.get("targetId").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| anyhow!("The browser didn't open a tab."))
    }

    pub async fn navigate(&self, target: &str, url: &str) -> Result<()> {
        let s = self.session(target).await?;
        let r = self.call("Page.navigate", json!({ "url": url }), Some(&s)).await?;
        match r.get("errorText").and_then(Value::as_str) {
            Some(e) if !e.is_empty() => Err(anyhow!("The page didn't load: {e}")),
            _ => Ok(()),
        }
    }

    pub async fn pause_media(&self, target: &str) -> Result<usize> {
        let s = self.session(target).await?;
        let r = self
            .call("Runtime.evaluate", json!({ "expression": "(()=>{let n=0;for(const m of document.querySelectorAll('video,audio')){if(!m.paused){m.pause();n++}}return n})()", "returnByValue": true }), Some(&s))
            .await?;
        Ok(r.pointer("/result/value").and_then(Value::as_u64).unwrap_or(0) as usize)
    }

    pub async fn page_text(&self, target: &str, max: usize) -> Result<(String, String)> {
        let s = self.session(target).await?;
        let r = self
            .call("Runtime.evaluate", json!({ "expression": "(()=>{const m=document.querySelector('main,article,[role=main]')||document.body;return [document.title, m?m.innerText:'']})()", "returnByValue": true }), Some(&s))
            .await?;
        let v = r.pointer("/result/value").cloned().unwrap_or(Value::Null);
        let title = v.get(0).and_then(Value::as_str).unwrap_or_default().to_owned();
        let body: String = v.get(1).and_then(Value::as_str).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        Ok((title, body.chars().take(max).collect()))
    }

    pub async fn capture(&self, target: &str, pdf: bool) -> Result<Vec<u8>> {
        use base64::Engine;
        let s = self.session(target).await?;
        let r = if pdf { self.call("Page.printToPDF", json!({ "printBackground": true }), Some(&s)).await? } else { self.call("Page.captureScreenshot", json!({ "format": "png", "captureBeyondViewport": false }), Some(&s)).await? };
        let data = r.get("data").and_then(Value::as_str).ok_or_else(|| anyhow!("The browser gave no picture."))?;
        base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| anyhow!("The browser's file was damaged: {e}"))
    }

    pub async fn console_errors(&self, target: &str) -> Result<Vec<String>> {
        let s = self.session(target).await?;
        let mut rx = self.events.subscribe();
        let _ = self.call("Log.enable", json!({}), Some(&s)).await;
        let _ = self.call("Runtime.enable", json!({}), Some(&s)).await;
        let mut out: Vec<String> = Vec::new();
        let until = tokio::time::Instant::now() + Duration::from_millis(700);
        while let Ok(Ok(v)) = tokio::time::timeout_at(until, rx.recv()).await {
            if v.get("sessionId").and_then(Value::as_str) != Some(s.as_str()) {
                continue;
            }
            let p = v.get("params").cloned().unwrap_or(Value::Null);
            let line = match v.get("method").and_then(Value::as_str) {
                Some("Log.entryAdded") if p.pointer("/entry/level").and_then(Value::as_str) == Some("error") => p.pointer("/entry/text").and_then(Value::as_str).map(str::to_owned),
                Some("Runtime.exceptionThrown") => p.pointer("/exceptionDetails/exception/description").and_then(Value::as_str).or_else(|| p.pointer("/exceptionDetails/text").and_then(Value::as_str)).map(|t| t.lines().next().unwrap_or("").to_owned()),
                Some("Runtime.consoleAPICalled") if p.get("type").and_then(Value::as_str) == Some("error") => p.pointer("/args/0/value").and_then(Value::as_str).or_else(|| p.pointer("/args/0/description").and_then(Value::as_str)).map(str::to_owned),
                _ => None,
            };
            if let Some(l) = line.filter(|l| !l.trim().is_empty())
                && !out.contains(&l)
            {
                out.push(l);
            }
        }
        let _ = self.call("Runtime.disable", json!({}), Some(&s)).await;
        let _ = self.call("Log.disable", json!({}), Some(&s)).await;
        Ok(out)
    }

    pub async fn version(&self) -> Result<String> {
        let r = self.call("Browser.getVersion", json!({}), None).await?;
        Ok(r.get("product").and_then(Value::as_str).unwrap_or_default().to_owned())
    }
}

pub fn exe_of(exec: &str, path: &str) -> PathBuf {
    if let Some((_, rest)) = exec.split_once(" --exe ") {
        let real = rest.trim_start().trim_start_matches('"');
        let real = match rest.trim_start().starts_with('"') {
            true => real.split('"').next().unwrap_or(""),
            false => real.split_whitespace().next().unwrap_or(""),
        };
        if !real.is_empty() {
            return PathBuf::from(real);
        }
    }
    let mut words = exec.split_whitespace().map(|w| w.trim_matches('"')).filter(|w| !w.starts_with('%'));
    let mut first = words.next().unwrap_or("");
    if first == "env" || first.ends_with("/env") {
        first = words.find(|w| !w.contains('=') && !w.starts_with('-')).unwrap_or("");
    }
    let p = PathBuf::from(first);
    if p.is_absolute() && p.exists() {
        return p;
    }
    if !first.is_empty()
        && let Some(found) = crate::tools::apps::which(first)
    {
        return found;
    }
    PathBuf::from(path)
}

#[cfg(unix)]
pub async fn launch(key: &str, exe: &Path, dir: &Path, extra: &[String], url: Option<&str>) -> Result<Arc<Cdp>> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    std::fs::create_dir_all(dir)?;
    let mut to = [0i32; 2];
    let mut from = [0i32; 2];
    unsafe {
        if libc::pipe(to.as_mut_ptr()) != 0 || libc::pipe(from.as_mut_ptr()) != 0 {
            return Err(anyhow!("Couldn't make a pipe to the browser."));
        }
    }
    let (child_read, parent_write) = unsafe { (OwnedFd::from_raw_fd(to[0]), OwnedFd::from_raw_fd(to[1])) };
    let (parent_read, child_write) = unsafe { (OwnedFd::from_raw_fd(from[0]), OwnedFd::from_raw_fd(from[1])) };
    for fd in [&parent_write, &parent_read] {
        unsafe {
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            let fl = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
        }
    }
    let (cr, cw) = (child_read.as_raw_fd(), child_write.as_raw_fd());
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--remote-debugging-pipe").arg(format!("--user-data-dir={}", dir.display())).arg("--no-first-run").arg("--no-default-browser-check");
    cmd.args(extra);
    if let Some(u) = url {
        cmd.arg(u);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(move || {
            if libc::dup2(cr, 3) < 0 || libc::dup2(cw, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::setsid();
            Ok(())
        });
    }
    let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.launch");
    let child = span.end(cmd.spawn().map_err(|e| anyhow!("The browser didn't start: {e}")))?;
    drop(child_read);
    drop(child_write);
    let reader = tokio::net::unix::pipe::Receiver::from_owned_fd(parent_read)?;
    let mut writer = tokio::net::unix::pipe::Sender::from_owned_fd(parent_write)?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let me = Arc::new(Cdp { key: key.to_owned(), out: tx, pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1), sessions: Mutex::new(HashMap::new()), alive: std::sync::atomic::AtomicBool::new(true), events: tokio::sync::broadcast::channel(512).0 });
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if writer.write_all(m.as_bytes()).await.is_err() || writer.write_all(&[0]).await.is_err() {
                break;
            }
        }
    });
    {
        let me = me.clone();
        tokio::spawn(async move {
            let mut reader = reader;
            let mut buf = Vec::with_capacity(1 << 16);
            let mut chunk = vec![0u8; 1 << 16];
            loop {
                match reader.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        while let Some(i) = buf.iter().position(|b| *b == 0) {
                            let msg: Vec<u8> = buf.drain(..=i).collect();
                            me.handle(&String::from_utf8_lossy(&msg[..msg.len() - 1]));
                        }
                    }
                }
            }
            me.alive.store(false, Ordering::SeqCst);
            for (_, tx) in me.pending.lock().unwrap().drain() {
                drop(tx);
            }
        });
    }
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    let first = tokio::time::timeout(Duration::from_secs(20), me.version()).await;
    match first {
        Ok(Ok(_)) => {
            LIVE.lock().unwrap().push(me.clone());
            Ok(me)
        }
        Ok(Err(e)) => Err(anyhow!("The browser started but its control pipe didn't answer: {e}")),
        Err(_) => Err(anyhow!("The browser started but its control pipe didn't answer.")),
    }
}

#[cfg(windows)]
fn spawn_piped(exe: &Path, dir: &Path, extra: &[String], url: Option<&str>) -> Result<(std::process::Child, std::fs::File, std::fs::File)> {
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation};
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::System::Pipes::CreatePipe;
    std::fs::create_dir_all(dir)?;
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
    let (mut to_r, mut to_w, mut from_r, mut from_w) = (HANDLE::default(), HANDLE::default(), HANDLE::default(), HANDLE::default());
    unsafe {
        CreatePipe(&mut to_r, &mut to_w, Some(&sa), 0).map_err(|e| anyhow!("Couldn't make a pipe to the browser: {e}"))?;
        CreatePipe(&mut from_r, &mut from_w, Some(&sa), 0).map_err(|e| anyhow!("Couldn't make a pipe to the browser: {e}"))?;
        let _ = SetHandleInformation(to_w, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
        let _ = SetHandleInformation(from_r, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0));
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--remote-debugging-pipe").arg(format!("--remote-debugging-io-pipes={},{}", to_r.0 as usize, from_w.0 as usize)).arg(format!("--user-data-dir={}", dir.display())).arg("--no-first-run").arg("--no-default-browser-check");
    cmd.args(extra);
    if let Some(u) = url {
        cmd.arg(u);
    }
    let spawned = cmd.spawn();
    unsafe {
        let _ = CloseHandle(to_r);
        let _ = CloseHandle(from_w);
    }
    let child = spawned.map_err(|e| anyhow!("The browser didn't start: {e}"))?;
    let writer = unsafe { std::fs::File::from_raw_handle(to_w.0 as RawHandle) };
    let reader = unsafe { std::fs::File::from_raw_handle(from_r.0 as RawHandle) };
    Ok((child, writer, reader))
}

#[cfg(windows)]
pub async fn launch(key: &str, exe: &Path, dir: &Path, extra: &[String], url: Option<&str>) -> Result<Arc<Cdp>> {
    use std::io::{Read, Write};
    let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.launch");
    let (child, mut writer, mut reader) = span.end(spawn_piped(exe, dir, extra, url))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let me = Arc::new(Cdp { key: key.to_owned(), out: tx, pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1), sessions: Mutex::new(HashMap::new()), alive: std::sync::atomic::AtomicBool::new(true), events: tokio::sync::broadcast::channel(512).0 });
    std::thread::Builder::new().name("setu-cdp-out".into()).spawn(move || {
        while let Some(m) = rx.blocking_recv() {
            if writer.write_all(m.as_bytes()).is_err() || writer.write_all(&[0]).is_err() {
                break;
            }
        }
    })?;
    {
        let me = me.clone();
        std::thread::Builder::new().name("setu-cdp-in".into()).spawn(move || {
            let mut buf = Vec::with_capacity(1 << 16);
            let mut chunk = vec![0u8; 1 << 16];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        while let Some(i) = buf.iter().position(|b| *b == 0) {
                            let msg: Vec<u8> = buf.drain(..=i).collect();
                            me.handle(&String::from_utf8_lossy(&msg[..msg.len() - 1]));
                        }
                    }
                }
            }
            me.alive.store(false, Ordering::SeqCst);
            me.pending.lock().unwrap().clear();
        })?;
    }
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    match tokio::time::timeout(Duration::from_secs(20), me.version()).await {
        Ok(Ok(_)) => {
            LIVE.lock().unwrap().push(me.clone());
            Ok(me)
        }
        Ok(Err(e)) => Err(anyhow!("The browser started but its control pipe didn't answer: {e}")),
        Err(_) => Err(anyhow!("The browser started but its control pipe didn't answer.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_real_browser_from_setus_own_launcher() {
        assert_eq!(exe_of("\"/usr/bin/setu-daemon\" --browser chrome --exe \"/opt/google/chrome/chrome\" %U", "/x"), PathBuf::from("/opt/google/chrome/chrome"));
        assert_eq!(exe_of("/usr/bin/setu-daemon --browser brave --exe /usr/bin/brave %U", "/x"), PathBuf::from("/usr/bin/brave"));
    }

    #[test]
    fn finds_the_browser_binary_in_a_launcher_line() {
        assert_eq!(exe_of("/bin/sh -c true", "/x"), PathBuf::from("/bin/sh"));
        assert_eq!(exe_of("env FOO=1 BAR=2 /bin/sh %U", "/x"), PathBuf::from("/bin/sh"));
        assert_eq!(exe_of("\"/bin/sh\" --flag %U", "/x"), PathBuf::from("/bin/sh"));
        assert_eq!(exe_of("", "/opt/x/chrome"), PathBuf::from("/opt/x/chrome"));
    }
}
