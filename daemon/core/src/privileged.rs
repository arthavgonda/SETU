use crate::core::Core;
use crate::router::Intent;
use crate::tools::Outcome;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const LAYER: &str = "system";

struct Conn {
    out: tokio::sync::mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    alive: AtomicBool,
}

static CONN: Mutex<Option<Arc<Conn>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static OPENING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn socket() -> PathBuf {
    if let Some(p) = std::env::var_os("SETU_SYSTEM_SOCKET") {
        return PathBuf::from(p);
    }
    PathBuf::from(if cfg!(windows) {
        r"\\.\pipe\setu-system"
    } else if cfg!(target_os = "macos") {
        "/var/run/setu-system.sock"
    } else {
        "/run/setu-system/sock"
    })
}

pub fn installed() -> bool {
    if cfg!(windows) {
        return std::fs::metadata(socket()).is_ok() || CONN.lock().unwrap().as_ref().is_some_and(|c| c.alive.load(Ordering::SeqCst));
    }
    socket().exists()
}

fn spawn_io<R, W>(r: R, mut w: W) -> Arc<Conn>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let me = Arc::new(Conn { out: tx, pending: Mutex::new(HashMap::new()), alive: AtomicBool::new(true) });
    let writer = me.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if w.write_all(line.as_bytes()).await.is_err() || w.write_all(b"\n").await.is_err() || w.flush().await.is_err() {
                break;
            }
        }
        writer.alive.store(false, Ordering::SeqCst);
    });
    let reader = me.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(id) = v.get("id").and_then(Value::as_u64)
                && let Some(tx) = reader.pending.lock().unwrap().remove(&id)
            {
                let _ = tx.send(v);
            }
        }
        reader.alive.store(false, Ordering::SeqCst);
        reader.pending.lock().unwrap().clear();
    });
    me
}

async fn connect() -> Result<Arc<Conn>> {
    if let Some(c) = CONN.lock().unwrap().as_ref().filter(|c| c.alive.load(Ordering::SeqCst)) {
        return Ok(c.clone());
    }
    let _g = OPENING.lock().await;
    if let Some(c) = CONN.lock().unwrap().as_ref().filter(|c| c.alive.load(Ordering::SeqCst)) {
        return Ok(c.clone());
    }
    let path = socket();
    #[cfg(unix)]
    let c = {
        let s = tokio::net::UnixStream::connect(&path).await.map_err(|_| anyhow!("The system helper isn't running."))?;
        let (r, w) = s.into_split();
        spawn_io(r, w)
    };
    #[cfg(windows)]
    let c = {
        let s = tokio::net::windows::named_pipe::ClientOptions::new().open(&path).map_err(|_| anyhow!("The system helper isn't running."))?;
        let (r, w) = tokio::io::split(s);
        spawn_io(r, w)
    };
    *CONN.lock().unwrap() = Some(c.clone());
    Ok(c)
}

pub async fn call(action: &str, args: Value, wait: Duration) -> Result<Value> {
    let c = connect().await?;
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = tokio::sync::oneshot::channel();
    c.pending.lock().unwrap().insert(id, tx);
    let span = crate::layers::effect(crate::layers::Layer::Service, "service.call");
    if c.out.send(json!({ "id": id, "action": action, "args": args }).to_string()).is_err() {
        c.alive.store(false, Ordering::SeqCst);
        return span.end(Err(anyhow!("The system helper closed.")));
    }
    let r = match tokio::time::timeout(wait, rx).await {
        Ok(Ok(v)) if v.get("ok").and_then(Value::as_bool) == Some(true) => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        Ok(Ok(v)) => Err(anyhow!("{}", v.get("error").and_then(Value::as_str).unwrap_or("The system helper refused that."))),
        Ok(Err(_)) => Err(anyhow!("The system helper closed.")),
        Err(_) => {
            c.pending.lock().unwrap().remove(&id);
            Err(anyhow!("The system helper didn't answer in time."))
        }
    };
    span.end(r)
}

pub fn helper_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) { "setu-system.exe" } else { "setu-system" };
    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?.join(name);
    if here.exists() {
        return Some(here);
    }
    ["/usr/libexec/setu/setu-system", "/usr/lib/setu/setu-system"].iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(unix)]
fn user_id() -> String {
    unsafe { libc::getuid() }.to_string()
}

#[cfg(windows)]
fn user_id() -> String {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return String::new();
        }
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr() as *mut _), len, &mut len).is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return String::new();
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = windows::core::PWSTR::null();
        if ConvertSidToStringSidW(user.User.Sid, &mut s).is_err() {
            return String::new();
        }
        let out = s.to_string().unwrap_or_default();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        out
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

async fn elevated(args: Vec<String>) -> Result<()> {
    let bin = helper_binary().ok_or_else(|| anyhow!("The system helper isn't part of this install."))?;
    #[cfg(target_os = "linux")]
    {
        let out = tokio::process::Command::new("pkexec").arg(&bin).args(&args).output().await.map_err(|e| anyhow!("pkexec didn't start: {e}"))?;
        return match out.status.code() {
            Some(0) => Ok(()),
            Some(126) | Some(127) => Err(anyhow!("The password wasn't given, so nothing was changed.")),
            _ => Err(anyhow!("{}", String::from_utf8_lossy(&out.stderr).trim())),
        };
    }
    #[cfg(target_os = "macos")]
    {
        let line = std::iter::once(shell_quote(&bin.to_string_lossy())).chain(args.iter().map(|a| shell_quote(a))).collect::<Vec<_>>().join(" ");
        let script = format!("do shell script {} with administrator privileges", serde_json::to_string(&line).unwrap_or_default());
        let out = tokio::process::Command::new("/usr/bin/osascript").arg("-e").arg(script).output().await.map_err(|e| anyhow!("{e}"))?;
        return if out.status.success() { Ok(()) } else { Err(anyhow!("The password wasn't given, so nothing was changed.")) };
    }
    #[cfg(windows)]
    {
        let _ = shell_quote;
        let params = args.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(" ");
        let b = bin.clone();
        let code = crate::layers::blocking(move || unsafe {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
            use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
            let verb: Vec<u16> = "runas\0".encode_utf16().collect();
            let file: Vec<u16> = b.to_string_lossy().encode_utf16().chain(Some(0)).collect();
            let p: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
            let mut info = SHELLEXECUTEINFOW { cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32, fMask: SEE_MASK_NOCLOSEPROCESS, lpVerb: windows::core::PCWSTR(verb.as_ptr()), lpFile: windows::core::PCWSTR(file.as_ptr()), lpParameters: windows::core::PCWSTR(p.as_ptr()), nShow: 0, ..Default::default() };
            if ShellExecuteExW(&mut info).is_err() {
                return 1u32;
            }
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            code
        })
        .await
        .unwrap_or(1);
        return if code == 0 { Ok(()) } else { Err(anyhow!("The administrator approval wasn't given, so nothing was changed.")) };
    }
    #[allow(unreachable_code)]
    {
        let _ = (bin, args, shell_quote);
        Err(anyhow!("This system has no way to ask for administrator rights."))
    }
}

pub async fn install() -> Result<()> {
    let me = std::env::current_exe()?;
    elevated(vec!["--install".into(), "--user".into(), user_id(), "--daemon".into(), me.to_string_lossy().into_owned()]).await?;
    for _ in 0..50 {
        if call("ping", json!({}), Duration::from_secs(2)).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(anyhow!("The system helper was installed but didn't start."))
}

pub async fn uninstall() -> Result<()> {
    elevated(vec!["--uninstall".into()]).await?;
    if let Some(c) = CONN.lock().unwrap().take() {
        c.alive.store(false, Ordering::SeqCst);
    }
    Ok(())
}

fn act(action: &str, args: Value) -> Intent {
    Intent::Act { tool: "admin.run".into(), args: json!({ "action": action, "args": args }) }
}

fn service_name(raw: &str) -> String {
    raw.trim().trim_start_matches("the ").trim_end_matches(" service").trim_end_matches(" daemon").trim().replace(' ', "-")
}

pub fn rules(t: &str) -> Option<Intent> {
    if matches!(t, "set up admin control" | "install the system helper" | "set up the system helper" | "turn on admin control" | "enable admin control" | "give yourself admin rights") {
        return Some(Intent::Act { tool: "admin.setup".into(), args: json!({}) });
    }
    if matches!(t, "remove the system helper" | "uninstall the system helper" | "turn off admin control" | "disable admin control" | "remove admin control") {
        return Some(Intent::Act { tool: "admin.remove".into(), args: json!({}) });
    }
    if matches!(t, "is admin control on" | "system helper status" | "is the system helper running" | "is admin control set up") {
        return Some(Intent::Act { tool: "admin.status".into(), args: json!({}) });
    }
    if matches!(t, "install updates" | "install system updates" | "update the system" | "update my computer" | "update my pc" | "update the computer" | "install all updates" | "run system updates") {
        return Some(act("updates.apply", json!({})));
    }
    for (verb, op) in [("restart ", "restart"), ("start ", "start"), ("stop ", "stop"), ("enable ", "enable"), ("disable ", "disable")] {
        if let Some(rest) = t.strip_prefix(verb)
            && (rest.ends_with(" service") || rest.ends_with(" daemon"))
        {
            let name = service_name(rest);
            if !name.is_empty() && name.split('-').count() <= 4 {
                return Some(act(&format!("service.{op}"), json!({ "unit": name })));
            }
        }
    }
    for (verb, op) in [("open port ", "firewall.port.open"), ("close port ", "firewall.port.close"), ("allow port ", "firewall.port.open"), ("block port ", "firewall.port.close")] {
        if let Some(rest) = t.strip_prefix(verb) {
            let mut words = rest.split_whitespace();
            if let Some(port) = words.next().and_then(|p| p.parse::<u16>().ok()) {
                let proto = if rest.contains("udp") { "udp" } else { "tcp" };
                return Some(act(op, json!({ "port": port, "proto": proto })));
            }
        }
    }
    if matches!(t, "turn on the firewall" | "enable the firewall" | "turn the firewall on") {
        return Some(act("firewall.state", json!({ "on": true })));
    }
    if matches!(t, "turn off the firewall" | "disable the firewall" | "turn the firewall off") {
        return Some(act("firewall.state", json!({ "on": false })));
    }
    for p in ["rename my computer to ", "rename the computer to ", "rename this computer to ", "set the computer name to ", "change the computer name to ", "call my computer ", "rename my pc to "] {
        if let Some(rest) = t.strip_prefix(p) {
            let name = rest.trim().replace(' ', "-");
            if !name.is_empty() {
                return Some(act("hostname.set", json!({ "name": name })));
            }
        }
    }
    for p in ["add a user called ", "add a user named ", "create a user called ", "create a user named ", "add an account called ", "create an account called "] {
        if let Some(rest) = t.strip_prefix(p) {
            let full: String = rest.split_whitespace().map(crate::text::sentence_case).collect::<Vec<_>>().join(" ");
            let name = rest.split_whitespace().collect::<Vec<_>>().join("");
            if !name.is_empty() {
                return Some(act("user.add", json!({ "name": name, "full": full })));
            }
        }
    }
    for p in ["remove the user ", "delete the user ", "remove the account ", "delete the account ", "remove user "] {
        if let Some(rest) = t.strip_prefix(p) {
            let name = rest.split_whitespace().collect::<Vec<_>>().join("");
            if !name.is_empty() {
                return Some(act("user.remove", json!({ "name": name })));
            }
        }
    }
    if matches!(t, "turn on network time" | "sync the clock" | "set the time automatically" | "turn on automatic time") {
        return Some(act("time.ntp", json!({ "on": true })));
    }
    if matches!(t, "turn off network time" | "turn off automatic time") {
        return Some(act("time.ntp", json!({ "on": false })));
    }
    None
}

pub fn describe(action: &str, args: &Value) -> (String, String) {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let unit = s("unit");
    match action {
        "service.restart" => (format!("Restart the {unit} service?"), format!("Restarted the {unit} service.")),
        "service.start" => (format!("Start the {unit} service?"), format!("Started the {unit} service.")),
        "service.stop" => (format!("Stop the {unit} service?"), format!("Stopped the {unit} service.")),
        "service.enable" => (format!("Turn on the {unit} service, now and at every start?"), format!("The {unit} service is on.")),
        "service.disable" => (format!("Turn off the {unit} service, now and at every start?"), format!("The {unit} service is off.")),
        "updates.apply" => ("Install all system updates? It can take a while, and some apps may need a restart.".into(), "System updates are installed.".into()),
        "package.install" => (format!("Install {}?", s("name")), format!("Installed {}.", s("name"))),
        "package.remove" => (format!("Remove {}?", s("name")), format!("Removed {}.", s("name"))),
        "package.install_file" => (format!("Install {}?", std::path::Path::new(&s("path")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), "Installed it.".into()),
        "firewall.port.open" | "firewall.port.close" => {
            let port = args.get("port").and_then(Value::as_u64).unwrap_or(0);
            let proto = args.get("proto").and_then(Value::as_str).unwrap_or("tcp").to_uppercase();
            if action.ends_with("open") {
                (format!("Open port {port} ({proto}) in the firewall? Other devices on the network can then reach it."), format!("Port {port} is open."))
            } else {
                (format!("Close port {port} ({proto}) in the firewall?"), format!("Port {port} is closed."))
            }
        }
        "firewall.state" if args.get("on").and_then(Value::as_bool) == Some(true) => ("Turn on the firewall?".into(), "The firewall is on.".into()),
        "firewall.state" => ("Turn off the firewall? The computer will be open to the network.".into(), "The firewall is off.".into()),
        "hostname.set" => (format!("Rename this computer to {}?", s("name")), format!("This computer is now called {}.", s("name"))),
        "time.ntp" if args.get("on").and_then(Value::as_bool) == Some(true) => ("Set the time automatically from the network?".into(), "The clock now sets itself from the network.".into()),
        "time.ntp" => ("Stop setting the time from the network?".into(), "The clock no longer sets itself.".into()),
        "backlight.set" => ("Change the screen backlight?".into(), "Changed the backlight.".into()),
        "user.add" => (format!("Add a user account called {}? It will need a password before anyone can sign in.", s("name")), format!("Added the user {}. Set a password for it in Settings, Users.", s("name"))),
        "user.remove" => (format!("Remove the user {} and all of their files? This can't be undone.", s("name")), format!("Removed the user {}.", s("name"))),
        "firefox.autoconfig.install" => ("Install Setu's script into Firefox? It works only in your Setu profile.".into(), "Firefox now has Setu's script.".into()),
        "firefox.autoconfig.remove" => ("Remove Setu's script from Firefox?".into(), "Removed Setu's script from Firefox.".into()),
        _ => (format!("Run {action} with administrator rights?"), "Done.".into()),
    }
}

pub fn danger(action: &str, args: &Value) -> bool {
    matches!(action, "service.stop" | "service.disable" | "package.remove" | "user.remove") || (action == "firewall.state" && args.get("on").and_then(Value::as_bool) == Some(false))
}

fn wait_for(action: &str) -> Duration {
    match action {
        "updates.apply" | "package.install" | "package.remove" | "package.install_file" => Duration::from_secs(4 * 3600),
        _ => Duration::from_secs(120),
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    match tool {
        "admin.status" => {
            if !installed() {
                return Outcome::ok_with("No system helper", LAYER, None).with_reply("Admin control isn't set up. Say set up admin control to add it.".into());
            }
            match call("hello", json!({}), Duration::from_secs(3)).await {
                Ok(v) => {
                    let n = v.get("actions").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
                    Outcome::ok_with("System helper is running", LAYER, None).with_reply(format!("Admin control is on. The system helper is running and can do {n} kinds of jobs."))
                }
                Err(e) => Outcome::fail(&format!("The system helper is installed but not answering: {e}"), LAYER),
            }
        }
        "admin.setup" => {
            if installed() && call("ping", json!({}), Duration::from_secs(2)).await.is_ok() {
                return Outcome::ok_with("System helper is running", LAYER, None).with_reply("Admin control is already on.".into());
            }
            match install().await {
                Ok(()) => Outcome::ok_with("Set up admin control", LAYER, None).with_reply("Done. Admin control is on. I'll still ask you before every change.".into()),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "admin.remove" => match uninstall().await {
            Ok(()) => Outcome::ok_with("Removed admin control", LAYER, None).with_reply("Admin control is off. The system helper is removed.".into()),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "admin.run" => {
            let action = args.get("action").and_then(Value::as_str).unwrap_or_default().to_owned();
            let inner = args.get("args").cloned().unwrap_or_else(|| json!({}));
            if !installed() {
                return Outcome::fail("That needs admin control. Say set up admin control first; you'll be asked for your password once.", LAYER);
            }
            let (_, done) = describe(&action, &inner);
            if action == "updates.apply" {
                let core = core.clone();
                tokio::spawn(async move {
                    let line = match call("updates.apply", json!({}), wait_for("updates.apply")).await {
                        Ok(_) => "System updates are installed.".to_owned(),
                        Err(e) => format!("The system updates didn't finish: {e}"),
                    };
                    crate::board::alert(&core, "system", &line);
                });
                return Outcome::ok_with("Installing system updates", LAYER, None).with_reply("Installing system updates now. I'll tell you when they're done.".into());
            }
            match call(&action, inner, wait_for(&action)).await {
                Ok(_) => Outcome::ok_with(done.trim_end_matches('.'), LAYER, None).with_reply(done),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        _ => Outcome::fail("I don't know that admin command.", LAYER),
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    match tool {
        "admin.setup" => "Setting up admin control".into(),
        "admin.remove" => "Removing admin control".into(),
        "admin.status" => "Checking admin control".into(),
        _ => {
            let action = args.get("action").and_then(Value::as_str).unwrap_or_default();
            let (q, _) = describe(action, &args.get("args").cloned().unwrap_or(Value::Null));
            q.trim_end_matches('?').split('?').next().unwrap_or_default().to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(t: &str) -> Option<(String, Value)> {
        match rules(t)? {
            Intent::Act { tool, args } => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn admin_phrases() {
        assert_eq!(r("set up admin control").unwrap().0, "admin.setup");
        let (tool, a) = r("restart the bluetooth service").unwrap();
        assert_eq!(tool, "admin.run");
        assert_eq!(a["action"], "service.restart");
        assert_eq!(a["args"]["unit"], "bluetooth");
        assert_eq!(r("stop the cups daemon").unwrap().1["args"]["unit"], "cups");
        assert_eq!(r("open port 8080").unwrap().1["args"]["port"], 8080);
        assert_eq!(r("open port 53 udp").unwrap().1["args"]["proto"], "udp");
        assert_eq!(r("rename my computer to dev box").unwrap().1["args"]["name"], "dev-box");
        assert_eq!(r("install updates").unwrap().1["action"], "updates.apply");
        assert_eq!(r("add a user called guest").unwrap().1["args"]["name"], "guest");
        assert_eq!(r("remove the user guest").unwrap().1["action"], "user.remove");
        assert!(r("restart the computer").is_none());
        assert!(r("stop the music").is_none());
    }

    #[test]
    fn the_router_sends_admin_phrases_here() {
        for (t, want) in [("restart the bluetooth service", "admin.run"), ("set up admin control", "admin.setup"), ("install updates", "admin.run"), ("open port 8080", "admin.run"), ("turn off the firewall", "admin.run")] {
            match crate::router::rules(t, "stop", false) {
                Some(Intent::Act { tool, .. }) => assert_eq!(tool, want, "{t}"),
                other => panic!("{t}: {other:?}"),
            }
        }
    }
}
