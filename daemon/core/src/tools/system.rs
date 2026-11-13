use anyhow::{Result, anyhow};
use std::process::{Command, Stdio};
use crate::proc::Quiet;

#[derive(Debug, Clone, Copy)]
pub struct VolumeState {
    pub level: i64,
    pub muted: bool,
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.system");
    let o = Command::new(cmd).quiet().args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().map_err(|e| anyhow!("{cmd}: {e}"))?;
    if !span.end_bool(o.status.success()) {
        return Err(anyhow!("{cmd} failed"));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

#[cfg(target_os = "linux")]
pub fn prefer_wideband_headset() -> Option<String> {
    use serde_json::Value;
    let text = run("pactl", &["-f", "json", "list", "cards"]).ok()?;
    let cards: Value = serde_json::from_str(&text).ok()?;
    for c in cards.as_array()? {
        let Some(name) = c.get("name").and_then(Value::as_str) else { continue };
        if !name.starts_with("bluez_card.") {
            continue;
        }
        let Some(profiles) = c.get("profiles").and_then(Value::as_object) else { continue };
        let desc = |k: &str| profiles.get(k).and_then(|p| p.get("description")).and_then(Value::as_str).unwrap_or_default().to_lowercase();
        let active = c.get("active_profile").and_then(Value::as_str).unwrap_or_default();
        if !(active.contains("cvsd") || desc(active).contains("cvsd")) {
            continue;
        }
        let wide = profiles.iter().find(|(k, p)| {
            let d = p.get("description").and_then(Value::as_str).unwrap_or_default().to_lowercase();
            p.get("available").and_then(Value::as_bool) != Some(false) && k.contains("head-unit") && !k.contains("cvsd") && (d.contains("msbc") || d.contains("lc3") || k.contains("msbc"))
        });
        let Some((profile, _)) = wide else { continue };
        if run("pactl", &["set-card-profile", name, profile]).is_ok() {
            let label = c.get("properties").and_then(|p| p.get("device.description")).and_then(Value::as_str).unwrap_or(name);
            return Some(label.to_owned());
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn prefer_wideband_headset() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
pub fn volume() -> Result<VolumeState> {
    let s = super::streams::master(None, None).map_err(|e| anyhow!("The volume could not be read: {e}"))?;
    Ok(VolumeState { level: (s.volume * 100.0).round() as i64, muted: s.muted })
}

#[cfg(target_os = "linux")]
pub fn set_volume(level: i64) -> Result<()> {
    let l = level.clamp(0, 100);
    super::streams::master(Some(l as f64 / 100.0), if l > 0 { Some(false) } else { None }).map(|_| ())
}

#[cfg(target_os = "linux")]
pub fn set_muted(m: bool) -> Result<()> {
    super::streams::master(None, Some(m)).map(|_| ())
}

#[cfg(target_os = "macos")]
pub fn volume() -> Result<VolumeState> {
    let v = run("osascript", &["-e", "output volume of (get volume settings)"])?;
    let m = run("osascript", &["-e", "output muted of (get volume settings)"])?;
    Ok(VolumeState { level: v.trim().parse().unwrap_or(50), muted: m.trim() == "true" })
}

#[cfg(target_os = "macos")]
pub fn set_volume(level: i64) -> Result<()> {
    run("osascript", &["-e", &format!("set volume output volume {}", level.clamp(0, 100))]).map(|_| ())
}

#[cfg(target_os = "macos")]
pub fn set_muted(m: bool) -> Result<()> {
    run("osascript", &["-e", if m { "set volume with output muted" } else { "set volume without output muted" }]).map(|_| ())
}

#[cfg(windows)]
pub fn volume() -> Result<VolumeState> {
    let span = crate::layers::read(crate::layers::Layer::System, "system.volume.read");
    let (level, muted) = span.end(super::win::volume()).map_err(|e| anyhow!("The volume could not be read: {e}"))?;
    Ok(VolumeState { level, muted })
}

#[cfg(windows)]
pub fn set_volume(level: i64) -> Result<()> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.volume");
    span.end(super::win::set_volume(level)).map_err(|e| anyhow!("The volume could not be changed: {e}"))
}

#[cfg(windows)]
pub fn set_muted(m: bool) -> Result<()> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.volume");
    span.end(super::win::set_muted(m)).map_err(|e| anyhow!("The sound could not be muted: {e}"))
}

pub fn change_volume(delta: i64) -> Result<(VolumeState, i64)> {
    let before = volume()?;
    let level = (before.level + delta).clamp(0, 100);
    set_volume(level)?;
    Ok((before, level))
}

#[derive(Debug, Clone)]
pub struct Backlight {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub name: String,
    pub max: i64,
    pub value: i64,
}

#[cfg(target_os = "linux")]
pub fn backlight() -> Result<Backlight> {
    let rd = std::fs::read_dir("/sys/class/backlight").map_err(|_| anyhow!("This screen's brightness can't be changed from here."))?;
    let mut best: Option<Backlight> = None;
    for e in rd.flatten() {
        let p = e.path();
        let read = |f: &str| std::fs::read_to_string(p.join(f)).ok().and_then(|s| s.trim().parse::<i64>().ok());
        let (Some(max), Some(value)) = (read("max_brightness"), read("brightness")) else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        let b = Backlight { name, max, value };
        if best.is_none() || kind.trim() == "firmware" || kind.trim() == "platform" {
            best = Some(b);
        }
    }
    best.ok_or_else(|| anyhow!("This screen's brightness can't be changed from here."))
}

#[cfg(target_os = "linux")]
pub async fn set_backlight(b: &Backlight, value: i64) -> Result<()> {
    let conn = zbus::Connection::system().await?;
    let v = value.clamp(b.max / 100, b.max) as u32;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.brightness");
    span.end(
        conn.call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1/session/auto",
            Some("org.freedesktop.login1.Session"),
            "SetBrightness",
            &("backlight", b.name.as_str(), v),
        )
        .await,
    )
    .map_err(|e| anyhow!("The brightness could not be changed: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn backlight() -> Result<Backlight> {
    Err(anyhow!("This screen's brightness can't be changed from here."))
}

#[cfg(target_os = "macos")]
pub async fn set_backlight(_b: &Backlight, _value: i64) -> Result<()> {
    Err(anyhow!("This screen's brightness can't be changed from here."))
}

#[cfg(windows)]
pub fn backlight() -> Result<Backlight> {
    let out = run("powershell", &["-NoProfile", "-NonInteractive", "-Command", "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop | Select-Object -First 1).CurrentBrightness"])
        .map_err(|_| anyhow!("This screen's brightness can't be changed from here. External monitors usually have their own buttons."))?;
    let value = out.trim().parse::<i64>().map_err(|_| anyhow!("This screen's brightness can't be changed from here."))?;
    Ok(Backlight { name: "wmi".into(), max: 100, value })
}

#[cfg(windows)]
pub async fn set_backlight(_b: &Backlight, value: i64) -> Result<()> {
    let script = format!("Invoke-CimMethod -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods -MethodName WmiSetBrightness -Arguments @{{Brightness=[byte]{}; Timeout=[uint32]0}} -ErrorAction Stop | Out-Null", value.clamp(0, 100));
    crate::layers::blocking(move || run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]).map(|_| ()))
        .await?
        .map_err(|e| anyhow!("The brightness could not be changed: {e}"))
}

pub fn percent(b: &Backlight) -> i64 {
    if b.max <= 0 { 0 } else { (b.value * 100 + b.max / 2) / b.max }
}

pub fn time_sentence() -> String {
    let now = jiff::Zoned::now();
    format!("It's {}.", now.strftime("%-I:%M %p"))
}

pub fn date_sentence() -> String {
    let now = jiff::Zoned::now();
    format!("Today is {}.", now.strftime("%A, %B %-d, %Y"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_are_plain() {
        let t = time_sentence();
        assert!(t.starts_with("It's ") && (t.contains("AM") || t.contains("PM")) && t.ends_with('.'));
        let d = date_sentence();
        assert!(d.starts_with("Today is ") && d.ends_with('.'));
        assert_eq!(percent(&Backlight { name: "x".into(), max: 200, value: 100 }), 50);
    }
}
