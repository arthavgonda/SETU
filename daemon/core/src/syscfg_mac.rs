use crate::proc::Quiet;
use crate::tools::Outcome;
use anyhow::{Result, anyhow};
use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.system");
    let o = Command::new(cmd).quiet().args(args).output().map_err(|e| anyhow!("{cmd}: {e}"))?;
    if !span.end_bool(o.status.success()) {
        return Err(anyhow!("{}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

async fn run_async(cmd: &'static str, args: Vec<String>) -> Result<String> {
    crate::layers::blocking(move || run(cmd, &args.iter().map(String::as_str).collect::<Vec<_>>())).await.map_err(|e| anyhow!("{e}")).and_then(|r| r)
}

fn not_here(what: &str) -> anyhow::Error {
    anyhow!("macOS doesn't let apps change {what}. It's in System Settings.")
}

pub async fn night_light(_on: bool) -> Result<()> {
    Err(not_here("Night Shift"))
}

pub async fn dnd(_on: bool) -> Result<()> {
    Err(not_here("Focus"))
}

pub async fn dark_mode(on: bool) -> Result<()> {
    run_async("osascript", vec!["-e".into(), format!("tell application \"System Events\" to tell appearance preferences to set dark mode to {on}")]).await.map(|_| ())
}

fn wifi_device() -> Option<String> {
    let out = run("networksetup", &["-listallhardwareports"]).ok()?;
    let mut lines = out.lines();
    while let Some(l) = lines.next() {
        if l.contains("Wi-Fi") || l.contains("AirPort") {
            return lines.next().and_then(|d| d.strip_prefix("Device: ")).map(str::to_owned);
        }
    }
    None
}

pub async fn wifi(on: bool) -> Result<()> {
    let dev = crate::layers::blocking(wifi_device).await.ok().flatten().ok_or_else(|| anyhow!("This Mac has no Wi-Fi."))?;
    run_async("networksetup", vec!["-setairportpower".into(), dev, if on { "on" } else { "off" }.into()]).await.map(|_| ())
}

pub async fn bluetooth(_on: bool) -> Result<usize> {
    Err(not_here("Bluetooth"))
}

pub async fn airplane(on: bool) -> Result<()> {
    wifi(!on).await
}

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
}

pub async fn bt_devices() -> Result<Vec<BtDevice>> {
    Err(not_here("Bluetooth devices"))
}

pub async fn bt_connect(_want: &str, _on: bool) -> Result<String> {
    Err(not_here("which Bluetooth device is connected"))
}

pub async fn power(op: &str) -> Result<()> {
    match op {
        "suspend" => run_async("pmset", vec!["sleepnow".into()]).await.map(|_| ()),
        "hibernate" => Err(anyhow!("Macs sleep instead of hibernating. Say put the computer to sleep.")),
        other => {
            let what = match other {
                "reboot" => "restart",
                "poweroff" => "shut down",
                _ => "log out",
            };
            run_async("osascript", vec!["-e".into(), format!("tell application \"System Events\" to {what}")]).await.map(|_| ())
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Battery {
    pub present: bool,
    pub percent: f64,
    pub charging: bool,
    pub full: bool,
    pub seconds: i64,
}

pub async fn battery() -> Result<Battery> {
    let out = run_async("pmset", vec!["-g".into(), "batt".into()]).await?;
    let Some(line) = out.lines().find(|l| l.contains('%')) else { return Ok(Battery::default()) };
    let pct = line.split(|c: char| c.is_whitespace() || c == ';').find_map(|w| w.strip_suffix('%')).and_then(|p| p.parse::<f64>().ok()).unwrap_or(0.0);
    let charging = line.contains("charging") && !line.contains("discharging");
    let seconds = line.split_whitespace().find(|w| w.contains(':') && w.chars().all(|c| c.is_ascii_digit() || c == ':')).and_then(|t| t.split_once(':')).map(|(h, m)| h.parse::<i64>().unwrap_or(0) * 3600 + m.parse::<i64>().unwrap_or(0) * 60).unwrap_or(0);
    Ok(Battery { present: true, percent: pct, charging, full: line.contains("charged"), seconds })
}

pub async fn power_profile(_want: Option<&str>) -> Result<(String, Vec<String>)> {
    Err(not_here("Low Power Mode"))
}

pub async fn vpn(on: bool, want: &str) -> Result<String> {
    let list = run_async("scutil", vec!["--nc".into(), "list".into()]).await?;
    let names: Vec<String> = list.lines().filter_map(|l| l.split('"').nth(1)).map(str::to_owned).collect();
    let w = crate::text::normalize(want);
    let pick = names.iter().find(|n| !w.is_empty() && crate::text::normalize(n).contains(&w)).or_else(|| if names.len() == 1 || w.is_empty() { names.first() } else { None }).cloned().ok_or_else(|| anyhow!("No VPN is set up."))?;
    run_async("scutil", vec!["--nc".into(), if on { "start" } else { "stop" }.into(), pick.clone()]).await?;
    Ok(pick)
}

pub async fn hotspot(_on: bool) -> Result<(String, Option<String>)> {
    Err(not_here("Internet Sharing"))
}

pub async fn current_scale() -> Result<f64> {
    Ok(1.0)
}

pub async fn display_info() -> Result<String> {
    let out = run_async("system_profiler", vec!["SPDisplaysDataType".into()]).await?;
    let res: Vec<String> = out.lines().filter_map(|l| l.trim().strip_prefix("Resolution: ")).map(str::to_owned).collect();
    if res.is_empty() { Err(anyhow!("No screen was found.")) } else { Ok(format!("The screen is at {}", res.join("; "))) }
}

pub async fn display(_res: Option<(i32, i32)>, _step: i32, _scale: Option<f64>) -> Result<String> {
    Err(not_here("the screen resolution"))
}

pub async fn eject(want: &str) -> Result<String> {
    let vols: Vec<String> = std::fs::read_dir("/Volumes").map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "Macintosh HD").collect()).unwrap_or_default();
    let w = crate::text::normalize(want);
    let pick = vols.iter().find(|v| !w.is_empty() && crate::text::normalize(v).contains(&w)).or_else(|| if vols.len() == 1 { vols.first() } else { None }).cloned().ok_or_else(|| anyhow!("No removable drive is plugged in."))?;
    run_async("diskutil", vec!["eject".into(), format!("/Volumes/{pick}")]).await?;
    Ok(pick)
}

pub async fn updates() -> Result<Vec<String>> {
    let out = run_async("softwareupdate", vec!["--list".into()]).await?;
    Ok(out.lines().filter_map(|l| l.trim().strip_prefix("* Label: ")).map(str::to_owned).collect())
}

#[derive(Debug, Clone)]
pub struct Note {
    pub app: String,
    pub title: String,
    pub body: String,
}

pub fn notes() -> Vec<Note> {
    Vec::new()
}

pub fn clear_notes() {}

pub fn watch_notifications() {}

pub async fn clipboard() -> Result<String> {
    run_async("pbpaste", vec![]).await
}

pub async fn set_default(_role: &str, _app: &crate::tools::apps::App) -> Result<String> {
    Err(not_here("default apps"))
}

pub fn printers() -> Vec<String> {
    run("lpstat", &["-e"]).map(|o| o.lines().map(|l| l.trim().replace('_', " ")).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

pub fn open_printer_settings() -> Result<()> {
    run("open", &["x-apple.systempreferences:com.apple.Print-Scan-Settings.extension"]).map(|_| ())
}

pub struct Startup {
    pub name: String,
    pub on: bool,
}

pub fn startup_list() -> Vec<Startup> {
    run("osascript", &["-e", "tell application \"System Events\" to get the name of every login item"]).map(|o| o.trim().split(", ").filter(|n| !n.is_empty()).map(|n| Startup { name: n.to_owned(), on: true }).collect()).unwrap_or_default()
}

pub fn set_startup(name: &str, app: Option<&crate::tools::apps::App>, on: bool) -> Result<String> {
    if on {
        let app = app.ok_or_else(|| anyhow!("I couldn't find an app called {name}."))?;
        run("osascript", &["-e", &format!("tell application \"System Events\" to make login item at end with properties {{path:{}, hidden:false}}", serde_json::to_string(&app.path).unwrap_or_default())])?;
        return Ok(app.name.clone());
    }
    let list = startup_list();
    let w = crate::text::normalize(name);
    let hit = list.iter().find(|s| crate::text::normalize(&s.name).contains(&w)).ok_or_else(|| anyhow!("{} doesn't open at startup.", crate::text::sentence_case(name)))?;
    run("osascript", &["-e", &format!("tell application \"System Events\" to delete login item {}", serde_json::to_string(&hit.name).unwrap_or_default())])?;
    Ok(hit.name.clone())
}

pub fn pids_named(bins: &[String]) -> Vec<u32> {
    let Ok(out) = run("ps", &["-axo", "pid=,comm="]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let (pid, comm) = l.trim().split_once(' ')?;
            let name = std::path::Path::new(comm.trim()).file_name()?.to_string_lossy().to_lowercase();
            bins.iter().any(|b| !b.is_empty() && *b == name).then(|| pid.parse().ok()).flatten()
        })
        .collect()
}

pub fn kill(pids: &[u32]) -> Result<usize> {
    let mut n = 0;
    for p in pids {
        if unsafe { libc::kill(*p as i32, libc::SIGTERM) } == 0 {
            n += 1;
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(2500));
    for p in pids {
        unsafe {
            libc::kill(*p as i32, libc::SIGKILL);
        }
    }
    if n == 0 { Err(anyhow!("It couldn't be stopped.")) } else { Ok(n) }
}

pub fn addresses() -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut p = head;
        while !p.is_null() {
            let a = &*p;
            if !a.ifa_addr.is_null() && (*a.ifa_addr).sa_family as i32 == libc::AF_INET && a.ifa_flags & libc::IFF_LOOPBACK as u32 == 0 {
                let sin = &*(a.ifa_addr as *const libc::sockaddr_in);
                out.push((std::ffi::CStr::from_ptr(a.ifa_name).to_string_lossy().into_owned(), std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string()));
            }
            p = a.ifa_next;
        }
        libc::freeifaddrs(head);
    }
    out
}

pub fn trash_count() -> usize {
    dirs::home_dir().and_then(|h| std::fs::read_dir(h.join(".Trash")).ok()).map(|rd| rd.flatten().filter(|e| e.file_name() != ".DS_Store").count()).unwrap_or(0)
}

pub async fn empty_trash() -> Result<usize> {
    let n = trash_count();
    run_async("osascript", vec!["-e".into(), "tell application \"Finder\" to empty the trash".into()]).await?;
    Ok(n)
}

pub async fn outputs_fallback() -> Outcome {
    Outcome::fail("Choose the sound output in System Settings, Sound.", "system")
}
