pub mod apps;
pub mod browsers;
pub mod atspi;
pub mod camera;
pub mod channels;
pub mod code;
pub mod codespeech;
pub mod fetch;
pub mod files;
pub mod git;
pub mod input;
pub mod install;
pub mod media;
pub mod pointer;
pub mod shell;
pub mod streams;
pub mod system;
pub mod web;
pub mod wifi;
pub mod wm;

pub async fn warm() {
    tokio::join!(crate::os::warm(), streams::warm(), media::warm(), channels::refill());
}
#[cfg(windows)]
pub mod win;
#[cfg(windows)]
pub use win::dpi_aware;

use crate::core::Core;
use crate::router::Intent;
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use crate::proc::Quiet;

#[derive(Debug, Clone)]
pub enum Undo {
    Close(apps::App),
    Open(apps::App),
    Volume(system::VolumeState),
    Brightness(system::Backlight),
}

#[derive(Debug, Clone)]
pub struct Choice {
    pub label: String,
    pub words: Vec<String>,
    pub then: Intent,
    pub icon: String,
}

impl Choice {
    pub fn new(label: &str, words: &[&str], then: Intent) -> Self {
        Self { label: label.to_owned(), words: words.iter().map(|w| (*w).to_owned()).collect(), then, icon: String::new() }
    }
}

#[derive(Debug, Clone)]
pub struct FollowUp {
    pub question: String,
    pub spoken: Option<String>,
    pub choices: Vec<Choice>,
    pub cleanup: Option<Intent>,
    pub kind: &'static str,
    pub focus: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub ok: bool,
    pub cancelled: bool,
    pub message: String,
    pub did: String,
    pub layer: &'static str,
    pub app: Option<String>,
    pub undo: Option<Undo>,
    pub reply: Option<String>,
    pub fix: Option<Value>,
    pub ask: Option<Box<FollowUp>>,
    pub need: Option<&'static str>,
}

impl Outcome {
    pub fn ok_with(did: &str, layer: &'static str, app: Option<String>) -> Self {
        Self { ok: true, cancelled: false, message: did.to_owned(), did: did.to_owned(), layer, app, undo: None, reply: None, fix: None, ask: None, need: None }
    }

    pub fn fail(message: &str, layer: &'static str) -> Self {
        Self { ok: false, cancelled: false, message: message.to_owned(), did: message.to_owned(), layer, app: None, undo: None, reply: None, fix: None, ask: None, need: None }
    }

    pub fn needs(mut self, what: &'static str) -> Self {
        self.need = Some(what);
        self
    }

    pub fn asking(did: &str, layer: &'static str, f: FollowUp) -> Self {
        let mut o = Self::ok_with(did, layer, None);
        o.ask = Some(Box::new(f));
        o
    }

    fn undo(mut self, u: Undo) -> Self {
        self.undo = Some(u);
        self
    }

    fn reply(mut self, r: String) -> Self {
        self.reply = Some(r);
        self
    }

    pub fn with_reply(mut self, r: String) -> Self {
        self.reply = Some(r);
        self
    }
}

fn app_name(core: &Core, id: &str) -> String {
    core.apps.get(id).map(|a| a.name).unwrap_or_else(|| crate::text::sentence_case(id))
}

pub fn describe_with(core: &Core, intent: &Intent) -> (String, String, String) {
    let (v, t, l) = describe(intent);
    match intent {
        Intent::OpenApp { target, .. } => (v, app_name(core, target), format!("Opening {}", app_name(core, target))),
        Intent::CloseApp { target } => (v, app_name(core, target), format!("Closing {}", app_name(core, target))),
        _ => (v, t, l),
    }
}

pub fn describe(intent: &Intent) -> (String, String, String) {
    let t = intent.target();
    match intent {
        Intent::OpenApp { .. } => ("open_app".into(), t.clone(), format!("Opening {t}")),
        Intent::CloseApp { .. } => ("close_app".into(), t.clone(), format!("Closing {t}")),
        Intent::Volume { action, level, amount } => (
            "set_volume".into(),
            t,
            match (*action, level, amount) {
                ("set", Some(l), _) => format!("Setting the volume to {l}%"),
                ("up", ..) => "Turning the volume up".into(),
                ("down", ..) => "Turning the volume down".into(),
                ("mute", ..) => "Muting the sound".into(),
                _ => "Turning the sound back on".into(),
            },
        ),
        Intent::Brightness { action, level, .. } => (
            "set_brightness".into(),
            t,
            match (*action, level) {
                ("set", Some(l)) => format!("Setting the brightness to {l}%"),
                ("up", _) => "Making the screen brighter".into(),
                _ => "Dimming the screen".into(),
            },
        ),
        Intent::Wifi { .. } => ("wifi".into(), t, "Disconnecting from Wi-Fi".into()),
        Intent::TypeText { text } => ("type_text".into(), t, format!("Typing {} characters", text.chars().count())),
        Intent::Plugin { tool, .. } => ("plugin".into(), tool.clone(), format!("Running {tool}")),
        Intent::Internal { kind: "install_package", args } => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("the app").to_owned();
            ("install_app".into(), args.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(), format!("Installing {name}"))
        }
        Intent::Internal { kind: "open_url", .. } => ("open_url".into(), t, "Opening the link".into()),
        Intent::Internal { kind: "click", .. } => ("click".into(), t.clone(), format!("Clicking {t}")),
        Intent::Act { tool, .. } => (tool.clone(), t, crate::skills::label(intent)),
        other => (other.action().into(), t, String::new()),
    }
}

pub fn cancellable(intent: &Intent) -> bool {
    match intent {
        Intent::Act { tool, .. } => crate::skills::cancellable(tool),
        other => matches!(other, Intent::Internal { kind: "install_package", .. } | Intent::Plugin { .. } | Intent::TypeText { .. }),
    }
}

static INSTALLING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub async fn run(core: &Arc<Core>, intent: Intent, cancel: Arc<AtomicBool>) -> Outcome {
    let learn = core.settings.bool("privacy.learn");
    match intent {
        Intent::OpenApp { target, .. } => {
            let Some(app) = core.apps.get(&target) else { return Outcome::fail("That app is no longer installed.", "registered") };
            let a = app.clone();
            match crate::layers::blocking(move || apps::launch(&a)).await {
                Ok(Ok(_)) => {
                    if learn {
                        core.apps.bump(&app.id);
                    }
                    Outcome::ok_with(&format!("Opened {}", app.name), "registered", Some(app.name.clone())).undo(Undo::Close(app))
                }
                Ok(Err(e)) => Outcome::fail(&e.to_string(), "registered"),
                Err(e) => Outcome::fail(&e.to_string(), "registered"),
            }
        }
        Intent::CloseApp { target } => {
            let Some(app) = core.apps.get(&target) else { return Outcome::fail("That app is no longer installed.", "registered") };
            let a = app.clone();
            match crate::layers::blocking(move || apps::close(&a)).await {
                Ok(Ok(_)) => Outcome::ok_with(&format!("Closed {}", app.name), "registered", Some(app.name.clone())).undo(Undo::Open(app)),
                Ok(Err(e)) => Outcome::fail(&e.to_string(), "registered"),
                Err(e) => Outcome::fail(&e.to_string(), "registered"),
            }
        }
        Intent::Volume { action, amount, level } => {
            let r = crate::layers::blocking(move || -> Result<(system::VolumeState, String)> {
                let before = system::volume().unwrap_or(system::VolumeState { level: 50, muted: false });
                let msg = match action {
                    "mute" => {
                        system::set_muted(true)?;
                        "Sound muted".to_owned()
                    }
                    "unmute" => {
                        system::set_muted(false)?;
                        "Sound is on".to_owned()
                    }
                    "set" => {
                        let l = level.unwrap_or(50).clamp(0, 100);
                        system::set_volume(l)?;
                        format!("Volume is at {l}%")
                    }
                    dir => {
                        let step = amount.unwrap_or(10).clamp(1, 100);
                        let (_, l) = system::change_volume(if dir == "up" { step } else { -step })?;
                        format!("Volume is at {l}%")
                    }
                };
                Ok((before, msg))
            })
            .await;
            match r {
                Ok(Ok((before, msg))) => Outcome::ok_with(&msg, "shell", None).undo(Undo::Volume(before)),
                Ok(Err(e)) => Outcome::fail(&format!("The volume could not be changed. {e}"), "shell"),
                Err(e) => Outcome::fail(&e.to_string(), "shell"),
            }
        }
        Intent::Brightness { action, amount, level } => {
            let b = match system::backlight() {
                Ok(b) => b,
                Err(e) => return Outcome::fail(&e.to_string(), "registered"),
            };
            let cur = system::percent(&b);
            let target = match action {
                "set" => level.unwrap_or(50),
                "up" => cur + amount.unwrap_or(15),
                _ => cur - amount.unwrap_or(15),
            }
            .clamp(1, 100);
            match system::set_backlight(&b, target * b.max / 100).await {
                Ok(()) => Outcome::ok_with(&format!("Brightness is at {target}%"), "registered", None).undo(Undo::Brightness(b)),
                Err(e) => Outcome::fail(&e.to_string(), "registered"),
            }
        }
        Intent::Wifi { action: "disconnect", .. } => match wifi::disconnect().await {
            Ok(()) => Outcome::ok_with("Disconnected from Wi-Fi", "registered", None),
            Err(e) => Outcome::fail(&e.to_string(), "registered"),
        },
        Intent::TypeText { text } => {
            if !core.settings.bool("permissions.control") {
                let mut o = Outcome::fail("Typing needs the Control permission. Turn it on in Settings, Permissions.", "accessibility");
                o.fix = Some(serde_json::json!({ "label": "Open Permissions", "method": "ui.open", "params": { "page": "settings/permissions" } }));
                return o.needs("control");
            }
            match atspi::type_text(&text).await {
                Ok(()) => Outcome::ok_with(&format!("Typed {} characters", text.chars().count()), "accessibility", None),
                Err(e) => match input::type_text(&text).await {
                    Ok(()) => Outcome::ok_with(&format!("Typed {} characters", text.chars().count()), "keyboard", None),
                    Err(_) => Outcome::fail(&e.to_string(), "accessibility"),
                },
            }
        }
        Intent::Internal { kind: "click", args } => {
            let el: atspi::Element = match serde_json::from_value(args) {
                Ok(e) => e,
                Err(_) => return Outcome::fail("That item is no longer available.", "accessibility"),
            };
            match atspi::click(&el).await {
                Ok(()) => Outcome::ok_with(&format!("Clicked {}", el.label), "accessibility", None),
                Err(e) => Outcome::fail(&e.to_string(), "accessibility"),
            }
        }
        Intent::Internal { kind: "open_url", args } => {
            let url = args.get("url").and_then(Value::as_str).unwrap_or_default().to_owned();
            match crate::layers::blocking(move || open_url(&url)).await {
                Ok(Ok(())) => Outcome::ok_with("Opened the link", "shell", None),
                Ok(Err(e)) => Outcome::fail(&e.to_string(), "shell"),
                Err(e) => Outcome::fail(&e.to_string(), "shell"),
            }
        }
        Intent::Internal { kind: "install_package", args } => {
            let pkg: install::Package = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(_) => return Outcome::fail("The package details were lost.", "shell"),
            };
            let name = pkg.name.clone();
            let _ = (cancel, &INSTALLING);
            crate::layers::fallback(crate::layers::Layer::Channel);
            if crate::jobs::start_package(pkg).is_none() {
                return Outcome::ok_with(&format!("Already installing {name}"), "shell", Some(name.clone())).with_reply(format!("I'm already installing {name}."));
            }
            Outcome::ok_with(&format!("Installing {name}"), "shell", Some(name.clone())).with_reply(format!("Installing {name}. I'll tell you when it's ready."))
        }
        Intent::Act { tool, args } => crate::skills::run(core, &tool, args, cancel).await,
        Intent::Plugin { tool, args } => match core.plugins.call(&tool, args, cancel).await {
            Ok(text) => Outcome::ok_with(&format!("Ran {tool}"), "registered", None).reply(text),
            Err(e) => Outcome::fail(&e, "registered"),
        },
        _ => Outcome::fail("I can't do that yet.", "registered"),
    }
}

pub fn undo(core: &Arc<Core>, u: Undo) -> Result<String> {
    match u {
        Undo::Close(app) => {
            apps::close(&app)?;
            Ok(format!("Closed {}", app.name))
        }
        Undo::Open(app) => {
            apps::launch(&app)?;
            let _ = core;
            Ok(format!("Opened {}", app.name))
        }
        Undo::Volume(v) => {
            system::set_volume(v.level)?;
            system::set_muted(v.muted)?;
            Ok(format!("Volume is back at {}%", v.level))
        }
        Undo::Brightness(b) => {
            let rt = tokio::runtime::Handle::current();
            let v = b.value;
            let b2 = b.clone();
            std::thread::spawn(move || rt.block_on(system::set_backlight(&b2, v))).join().map_err(|_| anyhow::anyhow!("brightness"))??;
            Ok(format!("Brightness is back at {}%", system::percent(&b)))
        }
    }
}

pub fn open_url(url: &str) -> Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(anyhow::anyhow!("Only web links can be opened."));
    }
    shell::open(url).map_err(|e| anyhow::anyhow!("The link could not be opened: {e}"))
}

fn ui_binary(core: &Core) -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) { "setu-ui.exe" } else { "setu-ui" };
    if let Some(p) = std::env::var_os("SETU_UI_BIN").map(std::path::PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    [core.paths.bin.join(name), core.paths.bin.join("../../../build/qt").join(name)].into_iter().find(|p| p.is_file())
}

pub fn launch_ui(core: &Core, page: &str) -> Result<()> {
    let exe = ui_binary(core).ok_or_else(|| anyhow::anyhow!("setu-ui was not found next to the daemon"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--page", page]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn espeak_binary(core: &Core) -> Option<(std::path::PathBuf, Option<std::path::PathBuf>)> {
    let name = if cfg!(windows) { "espeak-ng.exe" } else { "espeak-ng" };
    let dirs = [core.paths.bin.clone(), core.paths.bin.join("../lib/setu"), core.paths.bin.join("../Resources")];
    for d in dirs {
        let exe = d.join(name);
        if exe.is_file() {
            let data = d.join("espeak-ng-data").is_dir().then(|| d.clone());
            return Some((exe, data));
        }
    }
    which_in_path(name).map(|p| (p, None))
}

fn which_in_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file()))
}

pub fn fallback_speak(core: &Core, text: &str) {
    #[cfg(target_os = "macos")]
    if espeak_binary(core).is_none() {
        let mut cmd = std::process::Command::new("/usr/bin/say");
        cmd.args(["-r", "175", "--", text]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if let Err(e) = cmd.status() {
            log::warn!("the system voice could not speak: {e}");
        }
        return;
    }
    let Some((exe, data)) = espeak_binary(core) else {
        log::warn!("no voice is installed yet and espeak-ng was not found; the announcement is only logged: {text}");
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.quiet();
    cmd.args(["-v", "en-us", "-s", "150", "--stdout", "--", text]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(d) = data {
        cmd.env("ESPEAK_DATA_PATH", d);
    }
    let wav = match cmd.output() {
        Ok(o) if o.status.success() => o.stdout,
        Ok(_) => {
            log::warn!("espeak-ng could not speak");
            return;
        }
        Err(e) => {
            log::warn!("espeak-ng could not start: {e}");
            return;
        }
    };
    let Some((rate, samples)) = parse_wav(&wav) else {
        log::warn!("espeak-ng returned audio that could not be read");
        return;
    };
    let device = core.settings.str("audio.outputDevice");
    let Ok(mut p) = setu_audio::Playback::open(&device, rate, "Assistant voice") else { return };
    let volume = core.settings.f64("tts.volume").clamp(0.0, 1.0) as f32;
    let scaled: Vec<f32> = samples.iter().map(|s| s * volume).collect();
    let _ = p.write(&scaled);
    let _ = p.write(&vec![0.0; rate as usize / 6]);
    p.drain();
}

fn parse_wav(b: &[u8]) -> Option<(u32, Vec<f32>)> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let (mut pos, mut rate, mut channels, mut bits) = (12usize, 0u32, 1u16, 16u16);
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let declared = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let start = pos + 8;
        let len = if declared == 0 || declared == u32::MAX as usize || start + declared > b.len() { b.len() - start } else { declared };
        if id == b"fmt " && len >= 16 {
            channels = u16::from_le_bytes(b[start + 2..start + 4].try_into().ok()?);
            rate = u32::from_le_bytes(b[start + 4..start + 8].try_into().ok()?);
            bits = u16::from_le_bytes(b[start + 14..start + 16].try_into().ok()?);
        } else if id == b"data" {
            if bits != 16 || rate == 0 || channels == 0 {
                return None;
            }
            let frame = 2 * channels as usize;
            let samples = b[start..start + len - len % frame]
                .chunks_exact(frame)
                .map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0)
                .collect();
            return Some((rate, samples));
        }
        pos = start + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_wav;

    fn wav(rate: u32, samples: &[i16], data_len: u32) -> Vec<u8> {
        let mut b = b"RIFF\xff\xff\xff\xffWAVEfmt ".to_vec();
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        b
    }

    #[test]
    fn reads_streamed_and_sized_wav() {
        let (rate, s) = parse_wav(&wav(22050, &[0, 16384, -16384], 6)).unwrap();
        assert_eq!(rate, 22050);
        assert_eq!(s, vec![0.0, 0.5, -0.5]);
        let (_, s) = parse_wav(&wav(22050, &[1, 2, 3, 4], u32::MAX)).unwrap();
        assert_eq!(s.len(), 4);
        assert!(parse_wav(b"not a wave file").is_none());
    }
}
