use crate::core::Core;
use crate::router::Intent;
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

#[cfg(target_os = "linux")]
use crate::syscfg_linux as plat;
#[cfg(target_os = "macos")]
use crate::syscfg_mac as plat;
#[cfg(windows)]
use crate::syscfg_win as plat;

const LAYER: &str = "system";
static DISPLAY_CHANGED: AtomicI64 = AtomicI64::new(0);

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.into(), args }
}

fn toggle(what: &str, on: bool) -> Intent {
    act("sys.toggle", json!({ "what": what, "on": on }))
}

fn on_off(t: &str, names: &[&str]) -> Option<bool> {
    for n in names {
        for (p, on) in [("turn on ", true), ("turn off ", false), ("enable ", true), ("disable ", false), ("switch on ", true), ("switch off ", false), ("start ", true), ("stop ", false)] {
            if t == format!("{p}{n}") || t == format!("{p}the {n}") {
                return Some(on);
            }
        }
        for (sfx, on) in [(" on", true), (" off", false)] {
            if t == format!("{n}{sfx}") || t == format!("turn {n}{sfx}") || t == format!("turn the {n}{sfx}") || t == format!("switch {n}{sfx}") || t == format!("switch the {n}{sfx}") {
                return Some(on);
            }
        }
    }
    None
}

fn after<'a>(t: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| t.strip_prefix(p)).map(str::trim).filter(|r| !r.is_empty())
}

fn number_words(t: &str) -> String {
    t.replace(" by ", " x ").replace(" times ", " x ").replace('×', " x ")
}

fn resolution(t: &str) -> Option<(i32, i32)> {
    let t = number_words(t);
    let nums: Vec<i32> = t.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).filter(|n| *n >= 320).collect();
    match nums.as_slice() {
        [w, h, ..] => Some((*w, *h)),
        [n] => match *n {
            720 => Some((1280, 720)),
            1080 => Some((1920, 1080)),
            1440 => Some((2560, 1440)),
            2160 => Some((3840, 2160)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const ROLES: &[(&str, &[&str])] = &[
    ("browser", &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"]),
    ("email", &["x-scheme-handler/mailto"]),
    ("music", &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/x-vorbis+ogg"]),
    ("video", &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo"]),
    ("photos", &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp"]),
    ("pdf", &["application/pdf"]),
    ("text", &["text/plain"]),
];

fn role_of(t: &str) -> Option<&'static str> {
    let t = t.trim().trim_start_matches("my ").trim_start_matches("the ").trim_end_matches(" app").trim_end_matches(" program");
    Some(match t {
        "browser" | "web browser" | "internet browser" => "browser",
        "email" | "mail" | "email client" | "mail client" | "mail app" => "email",
        "music player" | "music" | "audio player" => "music",
        "video player" | "video" | "movie player" | "media player" => "video",
        "photo viewer" | "image viewer" | "photos" | "pictures" | "photo app" => "photos",
        "pdf reader" | "pdf viewer" | "pdf" => "pdf",
        "text editor" | "editor" | "notepad" => "text",
        _ => return None,
    })
}

const FILE_TYPES: &[&str] = &[
    "txt", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "csv", "md", "json", "xml", "html", "png", "jpg", "jpeg", "gif", "webp", "svg", "mp3", "mp4", "mkv", "wav", "flac", "zip", "rs", "py", "js", "ts", "c", "cpp", "h", "java", "sh", "log", "epub",
];

pub fn rules(t: &str) -> Option<Intent> {
    if let Some(on) = on_off(t, &["night light", "night shift", "night mode", "blue light filter"]) {
        return Some(toggle("nightlight", on));
    }
    if let Some(on) = on_off(t, &["dark mode", "dark theme"]) {
        return Some(toggle("dark", on));
    }
    if matches!(t, "switch to dark mode" | "use dark mode" | "make it dark" | "go dark") {
        return Some(toggle("dark", true));
    }
    if matches!(t, "switch to light mode" | "use light mode" | "turn on light mode" | "light mode") {
        return Some(toggle("dark", false));
    }
    if let Some(on) = on_off(t, &["do not disturb", "don't disturb", "focus mode notifications"]) {
        return Some(toggle("dnd", on));
    }
    if matches!(t, "silence notifications" | "mute notifications" | "hide notifications") {
        return Some(toggle("dnd", true));
    }
    if matches!(t, "show notifications again" | "unmute notifications" | "turn notifications back on") {
        return Some(toggle("dnd", false));
    }
    if let Some(on) = on_off(t, &["wifi", "wi fi", "wireless"]) {
        return Some(toggle("wifi", on));
    }
    if let Some(on) = on_off(t, &["airplane mode", "aeroplane mode", "flight mode"]) {
        return Some(toggle("airplane", on));
    }
    if let Some(on) = on_off(t, &["bluetooth"]) {
        return Some(toggle("bluetooth", on));
    }
    if let Some(on) = on_off(t, &["hotspot", "wifi hotspot", "mobile hotspot"]) {
        return Some(toggle("hotspot", on));
    }
    if matches!(t, "share my internet" | "share my connection" | "share internet") {
        return Some(toggle("hotspot", true));
    }
    if let Some(on) = on_off(t, &["vpn"]) {
        return Some(act("sys.vpn", json!({ "on": on, "name": "" })));
    }
    if let Some(r) = after(t, &["connect to the vpn ", "connect to vpn ", "connect the vpn "]) {
        return Some(act("sys.vpn", json!({ "on": true, "name": r })));
    }
    if matches!(t, "connect to the vpn" | "connect to vpn" | "connect the vpn") {
        return Some(act("sys.vpn", json!({ "on": true, "name": "" })));
    }
    if let Some(r) = t.strip_prefix("connect to ").and_then(|r| r.strip_suffix(" vpn")) {
        return Some(act("sys.vpn", json!({ "on": true, "name": r })));
    }
    if matches!(t, "disconnect the vpn" | "disconnect from the vpn" | "disconnect vpn" | "disconnect from vpn") {
        return Some(act("sys.vpn", json!({ "on": false, "name": "" })));
    }
    if matches!(t, "what's my ip" | "what's my ip address" | "what is my ip" | "what is my ip address" | "my ip address" | "ip address" | "show my ip address") {
        return Some(act("sys.ip", json!({})));
    }
    if matches!(t, "what bluetooth devices are there" | "list bluetooth devices" | "show bluetooth devices" | "which bluetooth devices are paired") {
        return Some(act("sys.bt", json!({ "op": "list" })));
    }
    let bt_words = ["headphones", "earbuds", "earphones", "headset", "speaker", "airpods", "buds", "bluetooth"];
    for (p, op) in [("connect to ", "connect"), ("connect ", "connect"), ("disconnect from ", "disconnect"), ("disconnect ", "disconnect")] {
        if let Some(r) = t.strip_prefix(p)
            && bt_words.iter().any(|w| r.contains(w))
            && !r.contains("wifi")
        {
            let name = r.trim_end_matches(" over bluetooth").trim_end_matches(" via bluetooth").trim_start_matches("bluetooth ").trim_start_matches("the ");
            return Some(act("sys.bt", json!({ "op": op, "name": name })));
        }
    }
    if matches!(t, "put the computer to sleep" | "put my computer to sleep" | "sleep the computer" | "suspend the computer" | "suspend" | "put the pc to sleep" | "computer go to sleep") {
        return Some(act("sys.power", json!({ "op": "suspend" })));
    }
    if matches!(t, "hibernate" | "hibernate the computer") {
        return Some(act("sys.power", json!({ "op": "hibernate" })));
    }
    if matches!(t, "restart the computer" | "restart my computer" | "reboot" | "reboot the computer" | "restart the pc" | "restart my pc" | "reboot my computer") {
        return Some(act("sys.power", json!({ "op": "reboot" })));
    }
    if matches!(t, "shut down" | "shutdown" | "shut down the computer" | "shut down my computer" | "turn off the computer" | "turn off my computer" | "power off" | "power off the computer" | "shut down the pc") {
        return Some(act("sys.power", json!({ "op": "poweroff" })));
    }
    if matches!(t, "log out" | "log me out" | "sign out" | "sign me out" | "logout") {
        return Some(act("sys.power", json!({ "op": "logout" })));
    }
    if matches!(t, "how much battery is left" | "how much battery do i have" | "battery level" | "battery status" | "battery percentage" | "what's my battery" | "what's the battery" | "how's the battery" | "is it charging" | "how much charge is left" | "battery") {
        return Some(act("sys.battery", json!({})));
    }
    if let Some(on) = on_off(t, &["battery saver", "power saver", "power saving mode", "power saver mode", "low power mode", "battery saving mode"]) {
        return Some(act("sys.profile", json!({ "name": if on { "power-saver" } else { "balanced" } })));
    }
    for (p, name) in [("performance mode", "performance"), ("balanced mode", "balanced"), ("power saver mode", "power-saver")] {
        if t == format!("switch to {p}") || t == format!("use {p}") || t == format!("turn on {p}") {
            return Some(act("sys.profile", json!({ "name": name })));
        }
    }
    if matches!(t, "what power mode" | "which power mode" | "what power mode is on" | "what's the power mode") {
        return Some(act("sys.profile", json!({})));
    }
    if matches!(t, "what's on my clipboard" | "what is on my clipboard" | "read my clipboard" | "what did i copy" | "read the clipboard") {
        return Some(act("sys.clipboard", json!({})));
    }
    if matches!(t, "read my notifications" | "any notifications" | "what notifications do i have" | "read notifications" | "do i have notifications" | "what did i miss") {
        return Some(act("sys.notifications", json!({})));
    }
    if matches!(t, "clear my notifications" | "forget my notifications") {
        return Some(act("sys.notifications", json!({ "clear": true })));
    }
    for p in ["set ", "make ", "use "] {
        if let Some(r) = t.strip_prefix(p) {
            for mid in [" as my default ", " my default ", " as the default ", " the default "] {
                if let Some((app, role)) = r.split_once(mid)
                    && let Some(role) = role_of(role)
                {
                    return Some(act("sys.default", json!({ "role": role, "app": app.trim() })));
                }
            }
        }
    }
    for p in ["stop ", "don't ", "do not ", "never "] {
        if let Some(r) = t.strip_prefix(p) {
            for sfx in [" opening at startup", " starting at startup", " launching at startup", " from opening at startup", " from starting at startup", " open at startup", " start at startup", " opening when i log in", " starting automatically", " open automatically"] {
                if let Some(app) = r.strip_suffix(sfx) {
                    return Some(act("sys.startup", json!({ "app": app.trim(), "on": false })));
                }
            }
        }
    }
    for p in ["open ", "start ", "launch ", "run "] {
        if let Some(r) = t.strip_prefix(p) {
            for sfx in [" at startup", " when i log in", " automatically", " on startup"] {
                if let Some(app) = r.strip_suffix(sfx) {
                    return Some(act("sys.startup", json!({ "app": app.trim(), "on": true })));
                }
            }
        }
    }
    if matches!(t, "what opens at startup" | "list startup apps" | "show startup apps" | "what starts automatically" | "which apps open at startup") {
        return Some(act("sys.startup", json!({})));
    }
    if matches!(t, "check for updates" | "are there any updates" | "any updates" | "are there updates" | "check for system updates") {
        return Some(act("sys.updates", json!({})));
    }
    if matches!(t, "add a printer" | "set up a printer" | "set up my printer" | "what printers are there" | "list printers" | "which printers can i use" | "connect a printer") {
        return Some(act("sys.printers", json!({})));
    }
    if matches!(t, "free up space" | "free up disk space" | "clean up the disk" | "clean up space" | "make some space" | "my disk is full") {
        return Some(act("sys.space", json!({})));
    }
    if matches!(t, "change the keyboard layout" | "switch the keyboard layout" | "next keyboard layout" | "switch keyboard language" | "change keyboard language" | "switch the keyboard language") {
        let keys = if cfg!(target_os = "macos") { "ctrl space" } else { "super space" };
        return Some(act("key", json!({ "keys": keys, "label": "Switched the keyboard layout" })));
    }
    if matches!(t, "what resolution is the screen" | "what's the screen resolution" | "what is the screen resolution" | "what resolution am i using" | "screen resolution") {
        return Some(act("sys.display", json!({ "info": true })));
    }
    if matches!(t, "lower the resolution" | "decrease the resolution" | "reduce the resolution" | "change the resolution" | "make the resolution lower") {
        return Some(act("sys.display", json!({ "step": -1 })));
    }
    if matches!(t, "raise the resolution" | "increase the resolution" | "higher resolution" | "make the resolution higher") {
        return Some(act("sys.display", json!({ "step": 1 })));
    }
    if let Some(r) = after(t, &["change the resolution to ", "set the resolution to ", "set resolution to ", "change resolution to ", "make the resolution ", "switch the resolution to "])
        && let Some((w, h)) = resolution(r)
    {
        return Some(act("sys.display", json!({ "w": w, "h": h })));
    }
    if let Some(r) = after(t, &["set the scale to ", "set display scale to ", "set the display scale to ", "change the scale to ", "scale the screen to "]) {
        let n: f64 = r.trim_end_matches(" percent").trim_end_matches('%').trim().parse().unwrap_or(0.0);
        if n >= 50.0 {
            return Some(act("sys.display", json!({ "scale": n / 100.0 })));
        }
    }
    if matches!(t, "make everything bigger" | "make the screen bigger" | "make things bigger" | "make everything larger") {
        return Some(act("sys.display", json!({ "scaleStep": 1 })));
    }
    if matches!(t, "make everything smaller" | "make things smaller" | "make the screen smaller") {
        return Some(act("sys.display", json!({ "scaleStep": -1 })));
    }
    if matches!(t, "keep it" | "keep the changes" | "keep changes" | "keep this" | "keep the display")
        && setu_common::protocol::now_ms() - DISPLAY_CHANGED.load(Ordering::SeqCst) < 30_000
    {
        return Some(act("key", json!({ "keys": "enter", "label": "Kept the display settings" })));
    }
    if let Some(r) = after(t, &["force quit ", "force close ", "kill "]) {
        return Some(act("sys.kill", json!({ "app": r })));
    }
    if matches!(t, "empty the trash" | "empty trash" | "empty the recycle bin" | "empty recycle bin" | "clear the trash") {
        return Some(act("sys.trash", json!({})));
    }
    if let Some(r) = after(t, &["eject the ", "eject ", "safely remove the ", "safely remove ", "unmount the ", "unmount "]) {
        return Some(act("sys.eject", json!({ "name": r })));
    }
    if matches!(t, "eject" | "eject it") {
        return Some(act("sys.eject", json!({ "name": "" })));
    }
    if let Some(r) = after(t, &["rename the file ", "rename the folder ", "rename "])
        && let Some((from, to)) = r.split_once(" to ")
    {
        return Some(act("sys.file", json!({ "op": "rename", "name": from.trim(), "to": to.trim() })));
    }
    if let Some(r) = after(t, &["zip the folder ", "zip the file ", "zip up ", "zip ", "compress the folder ", "compress the file ", "compress "]) {
        return Some(act("sys.file", json!({ "op": "zip", "name": r })));
    }
    if let Some(r) = after(t, &["extract the archive ", "extract the zip ", "extract ", "unzip the file ", "unzip ", "unpack "]) {
        return Some(act("sys.file", json!({ "op": "unzip", "name": r })));
    }
    if let Some(r) = after(t, &["show properties of ", "show the properties of ", "properties of ", "how big is the file ", "how big is the folder ", "how big is ", "what size is ", "details of the file "]) {
        return Some(act("sys.file", json!({ "op": "properties", "name": r })));
    }
    if let Some(r) = t.strip_prefix("open ") {
        for sep in [" in ", " with ", " using "] {
            if let Some((file, app)) = r.split_once(sep)
                && (file.contains('.') || file.rsplit(' ').next().is_some_and(|e| FILE_TYPES.contains(&e)))
                && app.split_whitespace().count() <= 3
                && crate::ide::spoken(app).is_none()
                && !["new tab", "a new tab", "new window", "a new window", "this tab", "the browser", "private"].iter().any(|w| app.contains(w))
            {
                return Some(act("sys.file", json!({ "op": "openwith", "name": file.trim(), "app": app.trim() })));
            }
        }
    }
    for p in ["switch the audio output to ", "switch audio output to ", "switch the sound output to ", "switch sound to ", "switch the sound to ", "switch audio to ", "play sound through ", "play the sound through ", "play audio through ", "play sound on ", "output sound to ", "use the ", "change the audio output to ", "change audio output to ", "set the audio output to "] {
        if let Some(r) = t.strip_prefix(p) {
            let r = r.trim();
            if p == "use the " {
                if let Some(dev) = r.strip_suffix(" for sound").or_else(|| r.strip_suffix(" for audio")) {
                    return Some(act("sys.output", json!({ "name": dev })));
                }
                continue;
            }
            return Some(act("sys.output", json!({ "name": r })));
        }
    }
    if matches!(t, "what audio outputs are there" | "list the audio outputs" | "list audio outputs" | "which speakers are there" | "what sound outputs are there" | "switch the audio output" | "change the audio output") {
        return Some(act("sys.output", json!({ "name": "" })));
    }
    None
}

pub fn needs_confirm(tool: &str) -> bool {
    matches!(tool, "sys.power" | "sys.kill" | "sys.trash")
}

pub async fn confirm(core: &Arc<Core>, tool: &str, args: &Value) -> Result<(String, Vec<String>, String, String, bool), String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool {
        "sys.power" => Ok(match s("op").as_str() {
            "suspend" => ("Put the computer to sleep?".into(), vec![], String::new(), "Sleep".into(), false),
            "hibernate" => ("Hibernate the computer?".into(), vec![], String::new(), "Hibernate".into(), false),
            "reboot" => ("Restart the computer now?".into(), vec![], "Unsaved work in open apps may be lost.".into(), "Restart".into(), true),
            "poweroff" => ("Shut down the computer now?".into(), vec![], "Unsaved work in open apps may be lost.".into(), "Shut down".into(), true),
            _ => ("Log out now?".into(), vec![], "Unsaved work in open apps may be lost.".into(), "Log out".into(), true),
        }),
        "sys.kill" => {
            let app = s("app");
            let pids = pids_of(core, &app).await;
            if pids.is_empty() {
                return Err(format!("{} isn't running.", crate::text::sentence_case(&app)));
            }
            Ok((format!("Force quit {}?", crate::text::sentence_case(&app)), vec![], "Anything unsaved in it will be lost.".into(), "Force quit".into(), true))
        }
        "sys.trash" => {
            let n = trash_count();
            if n == 0 {
                return Err("The trash is already empty.".into());
            }
            Ok((format!("Empty the trash? {n} {} will be deleted for good.", if n == 1 { "item" } else { "items" }), vec![], "This can't be undone.".into(), "Empty trash".into(), true))
        }
        _ => Err("Nothing to confirm.".into()),
    }
}

fn trash_count() -> usize {
    #[cfg(target_os = "linux")]
    {
        crate::xdg::trash_count()
    }
    #[cfg(not(target_os = "linux"))]
    {
        plat::trash_count()
    }
}

async fn pids_of(core: &Arc<Core>, name: &str) -> Vec<u32> {
    let want = crate::text::normalize(name);
    let found = core.apps.find(name, None, &core.defaults());
    let bins: Vec<String> = found
        .iter()
        .filter(|(_, sc)| *sc >= 0.8)
        .filter_map(|(a, _)| crate::tools::apps::split_exec(&a.exec).first().and_then(|p| std::path::Path::new(p).file_name().map(|f| f.to_string_lossy().to_lowercase())))
        .chain(std::iter::once(want.replace(' ', "")))
        .collect();
    let wins = crate::os::windows().await.unwrap_or_default();
    let mut pids: Vec<u32> = wins.iter().filter(|w| crate::text::normalize(&w.app) == want || bins.contains(&w.app.to_lowercase())).filter_map(|w| (w.pid > 0).then_some(w.pid)).collect();
    pids.extend(plat::pids_named(&bins));
    pids.sort_unstable();
    pids.dedup();
    let me = std::process::id();
    pids.retain(|p| *p != me && *p > 1);
    pids
}

fn ok(did: &str, reply: String) -> Outcome {
    Outcome::ok_with(did, LAYER, None).with_reply(reply)
}

fn fail(e: impl std::fmt::Display) -> Outcome {
    Outcome::fail(&e.to_string(), LAYER)
}

fn duration(secs: i64) -> String {
    let m = (secs + 30) / 60;
    match (m / 60, m % 60) {
        (0, m) => format!("{m} minute{}", if m == 1 { "" } else { "s" }),
        (h, 0) => format!("{h} hour{}", if h == 1 { "" } else { "s" }),
        (h, m) => format!("{h} hour{} {m} minute{}", if h == 1 { "" } else { "s" }, if m == 1 { "" } else { "s" }),
    }
}

async fn find_file(name: &str) -> Option<PathBuf> {
    let n = name.trim().trim_start_matches("the ").to_owned();
    let p = PathBuf::from(&n);
    if p.is_absolute() && p.exists() {
        return Some(p);
    }
    let hits = crate::layers::blocking(move || crate::tools::files::search(&n, None, None, Duration::from_millis(3500))).await.unwrap_or_default();
    hits.into_iter().next().map(|h| h.path)
}

async fn file_op(core: &Arc<Core>, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let op = s("op");
    let name = s("name");
    let Some(path) = find_file(&name).await else { return fail(format!("I couldn't find {name}.")) };
    let shown = crate::tools::files::spoken_path(&path);
    let p = path.clone();
    match op.as_str() {
        "rename" => {
            let to = s("to");
            match crate::layers::blocking(move || crate::fileops::rename(&p, &to)).await {
                Ok(Ok(dest)) => ok(&format!("Renamed {shown}"), format!("Renamed it to {}.", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
                Ok(Err(e)) => fail(e),
                Err(e) => fail(e),
            }
        }
        "zip" => match crate::work::background("zip", move || crate::fileops::compress(&p)).await {
            Ok(Ok((out, n))) => ok(&format!("Zipped {shown}"), format!("Made {} with {n} {}.", out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), if n == 1 { "file" } else { "files" })),
            Ok(Err(e)) => fail(e),
            Err(e) => fail(e),
        },
        "unzip" => match crate::work::background("unzip", move || crate::fileops::extract(&p)).await {
            Ok(Ok((out, n))) => ok(&format!("Extracted {shown}"), format!("Extracted {n} {} into the folder {}.", if n == 1 { "file" } else { "files" }, out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
            Ok(Err(e)) => fail(e),
            Err(e) => fail(e),
        },
        "properties" => match crate::layers::blocking(move || crate::fileops::properties(&p)).await {
            Ok(Ok(line)) => ok("Read the properties", line),
            Ok(Err(e)) => fail(e),
            Err(e) => fail(e),
        },
        "openwith" => {
            let app_name = s("app");
            let found = core.apps.find(&app_name, None, &core.defaults());
            let Some((app, _)) = found.into_iter().find(|(_, sc)| *sc >= 0.7) else { return fail(format!("I couldn't find an app called {app_name}.")) };
            let a = app.clone();
            let arg = path.to_string_lossy().into_owned();
            match crate::layers::blocking(move || crate::tools::shell::open_with(&a, &[arg])).await {
                Ok(Ok(())) => Outcome::ok_with(&format!("Opened {shown} in {}", app.name), LAYER, Some(app.name.clone())),
                Ok(Err(e)) => fail(e),
                Err(e) => fail(e),
            }
        }
        _ => fail("I don't know that file action."),
    }
}

async fn outputs_step(name: &str) -> Outcome {
    #[cfg(target_os = "linux")]
    {
        let list = match crate::layers::blocking(crate::tools::streams::sinks).await.map_err(|e| anyhow::anyhow!("{e}")).and_then(|r| r) {
            Ok(l) => l,
            Err(e) => return fail(e),
        };
        if list.is_empty() {
            return fail("There is no sound output.");
        }
        let want = crate::text::normalize(name);
        if want.is_empty() {
            let choices: Vec<Choice> = list.iter().map(|s| Choice::new(&s.description, &[crate::text::normalize(&s.description).as_str()], act("sys.output", json!({ "name": s.name })))).collect();
            let cur = list.iter().find(|s| s.default).map(|s| s.description.clone()).unwrap_or_default();
            let q = format!("Sound plays through {cur}. Which output?");
            return Outcome::asking("Listed sound outputs", LAYER, FollowUp { question: q.clone(), spoken: Some(q), choices, cleanup: None, kind: "confirm", focus: None });
        }
        let pick = list
            .iter()
            .find(|s| s.name == name)
            .or_else(|| list.iter().find(|s| crate::text::normalize(&s.description) == want))
            .or_else(|| {
                list.iter()
                    .map(|s| (crate::os::score(&want, &crate::text::normalize(&s.description)).max(if crate::text::normalize(&s.description).contains(&want) { 0.9 } else { 0.0 }), s))
                    .filter(|(sc, _)| *sc >= 0.6)
                    .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(_, s)| s)
            })
            .cloned();
        let Some(sink) = pick else {
            return fail(format!("I couldn't find {name}. Outputs: {}.", list.iter().map(|s| s.description.clone()).collect::<Vec<_>>().join(", ")));
        };
        let n = sink.name.clone();
        match crate::layers::blocking(move || crate::tools::streams::set_output(&n)).await.map_err(|e| anyhow::anyhow!("{e}")).and_then(|r| r) {
            Ok(()) => ok(&format!("Sound plays through {}", sink.description), format!("Sound now plays through {}.", sink.description)),
            Err(e) => fail(e),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (name, act("", Value::Null), FollowUp { question: String::new(), spoken: None, choices: Vec::<Choice>::new(), cleanup: None, kind: "confirm", focus: None });
        plat::outputs_fallback().await
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
    match tool {
        "sys.toggle" => {
            let what = s("what");
            let state = if on { "on" } else { "off" };
            let r = match what.as_str() {
                "nightlight" => plat::night_light(on).await.map(|_| format!("Night light is {state}.")),
                "dark" => plat::dark_mode(on).await.map(|_| if on { "Dark mode is on.".to_owned() } else { "Light mode is on.".to_owned() }),
                "dnd" => plat::dnd(on).await.map(|_| if on { "Do not disturb is on. Notifications stay quiet.".to_owned() } else { "Notifications will show again.".to_owned() }),
                "wifi" => plat::wifi(on).await.map(|_| format!("Wi-Fi is {state}.")),
                "airplane" => plat::airplane(on).await.map(|_| format!("Airplane mode is {state}.")),
                "bluetooth" => plat::bluetooth(on).await.map(|_| format!("Bluetooth is {state}.")),
                "hotspot" => plat::hotspot(on).await.map(|(name, pw)| match (on, pw) {
                    (true, Some(p)) => format!("The hotspot {name} is on. Its password is {p}."),
                    (true, None) => format!("The hotspot {name} is on."),
                    (false, _) => "The hotspot is off.".to_owned(),
                }),
                _ => Err(anyhow::anyhow!("I don't know that setting.")),
            };
            match r {
                Ok(line) => {
                    let did = line.trim_end_matches('.').to_owned();
                    ok(&did, line)
                }
                Err(e) => fail(e),
            }
        }
        "sys.vpn" => match plat::vpn(on, &s("name")).await {
            Ok(name) => ok(&format!("VPN {}", if on { "connecting" } else { "off" }), if on { format!("Connecting to the {name} VPN.") } else { format!("Disconnected from {name}.") }),
            Err(e) => fail(e),
        },
        "sys.ip" => {
            let addrs = plat::addresses();
            let main = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|u| u.connect("192.0.2.1:9").and_then(|_| u.local_addr())).map(|a| a.ip().to_string()).ok();
            match main.or_else(|| addrs.first().map(|(_, a)| a.clone())) {
                Some(ip) => {
                    let others: Vec<String> = addrs.iter().filter(|(_, a)| *a != ip).map(|(n, a)| format!("{a} on {n}")).collect();
                    let extra = if others.is_empty() { String::new() } else { format!(" Also {}.", others.join(", ")) };
                    ok("Read the IP address", format!("Your IP address on this network is {ip}.{extra}"))
                }
                None => fail("This computer isn't connected to a network."),
            }
        }
        "sys.bt" => match s("op").as_str() {
            "list" => match plat::bt_devices().await {
                Ok(list) if list.is_empty() => ok("No Bluetooth devices", "No Bluetooth devices are paired.".into()),
                Ok(list) => ok("Listed Bluetooth devices", format!("Paired: {}.", list.iter().map(|d| if d.connected { format!("{} (connected)", d.name) } else { d.name.clone() }).collect::<Vec<_>>().join(", "))),
                Err(e) => fail(e),
            },
            op => match plat::bt_connect(&s("name"), op == "connect").await {
                Ok(name) => ok(&format!("{} {name}", if op == "connect" { "Connected" } else { "Disconnected" }), format!("{} {name}.", if op == "connect" { "Connected to" } else { "Disconnected" })),
                Err(e) => fail(e),
            },
        },
        "sys.power" => match plat::power(&s("op")).await {
            Ok(()) => ok("Done", match s("op").as_str() {
                "suspend" => "Going to sleep.".into(),
                "reboot" => "Restarting.".into(),
                "poweroff" => "Shutting down.".into(),
                "hibernate" => "Hibernating.".into(),
                _ => "Logging out.".into(),
            }),
            Err(e) => fail(e),
        },
        "sys.battery" => match plat::battery().await {
            Ok(b) if !b.present => ok("No battery", "This computer runs on mains power; it has no battery.".into()),
            Ok(b) => {
                let pct = b.percent.round() as i64;
                let line = if b.full {
                    format!("The battery is full, {pct}%.")
                } else if b.charging {
                    if b.seconds > 0 { format!("The battery is at {pct}% and charging, full in about {}.", duration(b.seconds)) } else { format!("The battery is at {pct}% and charging.") }
                } else if b.seconds > 0 {
                    format!("The battery is at {pct}%, about {} left.", duration(b.seconds))
                } else {
                    format!("The battery is at {pct}%.")
                };
                ok(&format!("Battery {pct}%"), line)
            }
            Err(e) => fail(e),
        },
        "sys.profile" => {
            let want = s("name");
            match plat::power_profile(if want.is_empty() { None } else { Some(&want) }).await {
                Ok((cur, _)) if want.is_empty() => ok("Read the power mode", format!("The power mode is {}.", cur.replace('-', " "))),
                Ok((cur, _)) => ok(&format!("{} mode", cur.replace('-', " ")), match cur.as_str() {
                    "power-saver" => "Battery saver is on.".into(),
                    "performance" => "Performance mode is on.".into(),
                    _ => "Balanced mode is on.".into(),
                }),
                Err(e) => fail(e),
            }
        }
        "sys.clipboard" => match plat::clipboard().await {
            Ok(t) if t.trim().is_empty() => ok("Clipboard is empty", "The clipboard is empty.".into()),
            Ok(t) => {
                let short: String = t.trim().chars().take(400).collect();
                let more = if t.trim().chars().count() > 400 { " and more" } else { "" };
                ok("Read the clipboard", format!("The clipboard has: {short}{more}"))
            }
            Err(e) => fail(e),
        },
        "sys.notifications" => {
            if args.get("clear").and_then(Value::as_bool) == Some(true) {
                plat::clear_notes();
                return ok("Cleared notifications", "Done. I've forgotten the notifications.".into());
            }
            let notes = plat::notes();
            if notes.is_empty() {
                return ok("No notifications", "No notifications have come in since I started.".into());
            }
            let take: Vec<String> = notes.iter().rev().take(5).map(|n| {
                let who = if n.app.is_empty() { String::new() } else { format!("{}: ", n.app) };
                let body = if n.body.is_empty() { String::new() } else { format!(", {}", n.body.chars().take(160).collect::<String>()) };
                format!("{who}{}{body}", n.title)
            }).collect();
            let more = notes.len().saturating_sub(5);
            let tail = if more > 0 { format!(" And {more} older.") } else { String::new() };
            ok(&format!("{} notifications", notes.len()), format!("{}.{tail}", take.join(". ")))
        }
        "sys.default" => {
            let role = s("role");
            let app_name = s("app");
            let found = core.apps.find(&app_name, None, &core.defaults());
            let Some((app, _)) = found.into_iter().find(|(_, sc)| *sc >= 0.7) else { return fail(format!("I couldn't find an app called {app_name}.")) };
            match plat::set_default(&role, &app).await {
                Ok(line) => ok(&format!("{} is the default", app.name), line),
                Err(e) => fail(e),
            }
        }
        "sys.startup" => {
            let app_name = s("app");
            if app_name.is_empty() {
                let list = plat::startup_list();
                let on: Vec<String> = list.iter().filter(|s| s.on).map(|s| s.name.clone()).collect();
                return ok("Listed startup apps", if on.is_empty() { "Nothing extra opens at startup.".into() } else { format!("At startup: {}.", on.join(", ")) });
            }
            let found = core.apps.find(&app_name, None, &core.defaults());
            let app = found.into_iter().find(|(_, sc)| *sc >= 0.7).map(|(a, _)| a);
            match plat::set_startup(&app_name, app.as_ref(), on) {
                Ok(name) => ok(&format!("{name} {}", if on { "opens at startup" } else { "won't open at startup" }), if on { format!("{name} will open when you log in.") } else { format!("{name} won't open at startup any more.") }),
                Err(e) => fail(e),
            }
        }
        "sys.updates" => match plat::updates().await {
            Ok(list) if list.is_empty() => ok("No updates", "Everything is up to date.".into()),
            Ok(list) => {
                let names: Vec<String> = list.iter().take(5).cloned().collect();
                let more = list.len().saturating_sub(5);
                let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
                ok(&format!("{} updates", list.len()), format!("{} {} ready: {}{tail}. Say install updates to install them.", list.len(), if list.len() == 1 { "update is" } else { "updates are" }, names.join(", ")))
            }
            Err(e) => fail(e),
        },
        "sys.space" => {
            let home = dirs::home_dir().unwrap_or_default();
            let free = crate::layers::blocking(move || {
                let disks = sysinfo::Disks::new_with_refreshed_list();
                disks.iter().filter(|d| home.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()).map(|d| (d.available_space(), d.total_space()))
            })
            .await
            .ok()
            .flatten();
            let trash = trash_count();
            let cache = dirs::cache_dir().map(|c| {
                let mut budget = 200_000;
                crate::fileops::size_of(&c, &mut budget)
            });
            let mut parts = Vec::new();
            if let Some((f, t)) = free {
                parts.push(format!("{} free of {}", crate::fileops::human(f), crate::fileops::human(t)));
            }
            if trash > 0 {
                parts.push(format!("the trash has {trash} {} you can empty", if trash == 1 { "item" } else { "items" }));
            }
            if let Some(c) = cache.filter(|c| *c > 100_000_000) {
                parts.push(format!("apps keep {} of cache", crate::fileops::human(c)));
            }
            ok("Checked the disk", format!("{}. Say empty the trash to free space; I don't delete anything without asking.", crate::text::sentence_case(&parts.join("; "))))
        }
        "sys.display" => {
            if args.get("info").and_then(Value::as_bool) == Some(true) {
                return match plat::display_info().await {
                    Ok(line) => ok("Read the display", format!("{line}.")),
                    Err(e) => fail(e),
                };
            }
            let res = match (args.get("w").and_then(Value::as_i64), args.get("h").and_then(Value::as_i64)) {
                (Some(w), Some(h)) => Some((w as i32, h as i32)),
                _ => None,
            };
            let step = args.get("step").and_then(Value::as_i64).unwrap_or(0) as i32;
            let mut scale = args.get("scale").and_then(Value::as_f64);
            if let Some(d) = args.get("scaleStep").and_then(Value::as_i64) {
                let cur = plat::current_scale().await.unwrap_or(1.0);
                scale = Some((cur + 0.25 * d as f64).clamp(1.0, 3.0));
            }
            match plat::display(res, step, scale).await {
                Ok(line) => {
                    DISPLAY_CHANGED.store(setu_common::protocol::now_ms(), Ordering::SeqCst);
                    ok("Changed the display", format!("The screen is now {line}. If it looks right, say keep it; otherwise it goes back by itself."))
                }
                Err(e) => fail(e),
            }
        }
        "sys.kill" => {
            let app = s("app");
            let pids = pids_of(core, &app).await;
            if pids.is_empty() {
                return fail(format!("{} isn't running.", crate::text::sentence_case(&app)));
            }
            match crate::layers::blocking(move || plat::kill(&pids)).await.map_err(|e| anyhow::anyhow!("{e}")).and_then(|r| r) {
                Ok(_) => ok(&format!("Force quit {app}"), format!("Closed {} by force.", crate::text::sentence_case(&app))),
                Err(e) => fail(e),
            }
        }
        "sys.trash" => {
            #[cfg(target_os = "linux")]
            let r = crate::layers::blocking(crate::xdg::empty_trash).await.map_err(|e| anyhow::anyhow!("{e}")).and_then(|r| r);
            #[cfg(not(target_os = "linux"))]
            let r = plat::empty_trash().await;
            match r {
                Ok(n) => ok("Emptied the trash", format!("Emptied the trash: {n} {} deleted.", if n == 1 { "item" } else { "items" })),
                Err(e) => fail(e),
            }
        }
        "sys.eject" => match plat::eject(&s("name")).await {
            Ok(name) => ok(&format!("Ejected {name}"), format!("{name} is safe to unplug.")),
            Err(e) => fail(e),
        },
        "sys.file" => file_op(core, args).await,
        "sys.output" => outputs_step(&s("name")).await,
        "sys.printers" => {
            let list = crate::layers::blocking(plat::printers).await.unwrap_or_default();
            if list.is_empty() {
                let _ = plat::open_printer_settings();
                return ok("Opened printer settings", "I don't see a printer on the network or plugged in. I opened the printer settings; pick it there and press Add.".into());
            }
            ok(&format!("{} printers", list.len()), format!("Ready to print: {}. Printers on your network work without adding them.", list.join(", ")))
        }
        _ => fail("I don't know that setting."),
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
    match tool {
        "sys.toggle" => {
            let what = match s("what").as_str() {
                "nightlight" => "night light",
                "dark" => "dark mode",
                "dnd" => "do not disturb",
                "wifi" => "Wi-Fi",
                "airplane" => "airplane mode",
                "bluetooth" => "Bluetooth",
                "hotspot" => "the hotspot",
                _ => "it",
            };
            format!("Turning {} {what}", if on { "on" } else { "off" })
        }
        "sys.vpn" => if on { "Connecting the VPN".into() } else { "Disconnecting the VPN".into() },
        "sys.ip" => "Reading the IP address".into(),
        "sys.bt" => format!("Bluetooth {}", s("name")).trim().to_owned(),
        "sys.power" => match s("op").as_str() {
            "suspend" => "Going to sleep".into(),
            "reboot" => "Restarting".into(),
            "poweroff" => "Shutting down".into(),
            _ => "Logging out".into(),
        },
        "sys.battery" => "Checking the battery".into(),
        "sys.profile" => "Power mode".into(),
        "sys.clipboard" => "Reading the clipboard".into(),
        "sys.notifications" => "Reading notifications".into(),
        "sys.default" => format!("Making {} the default", s("app")),
        "sys.startup" => "Startup apps".into(),
        "sys.updates" => "Checking for updates".into(),
        "sys.space" => "Checking disk space".into(),
        "sys.display" => "Changing the display".into(),
        "sys.kill" => format!("Force quitting {}", s("app")),
        "sys.trash" => "Emptying the trash".into(),
        "sys.eject" => "Ejecting the drive".into(),
        "sys.file" => match s("op").as_str() {
            "rename" => format!("Renaming {}", s("name")),
            "zip" => format!("Zipping {}", s("name")),
            "unzip" => format!("Extracting {}", s("name")),
            "openwith" => format!("Opening {} in {}", s("name"), s("app")),
            _ => format!("Reading {}", s("name")),
        },
        "sys.output" => "Choosing the sound output".into(),
        "sys.printers" => "Looking for printers".into(),
        _ => String::new(),
    }
}

pub fn start(core: &Arc<Core>) {
    let _ = core;
    plat::watch_notifications();
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
    fn settings_phrases() {
        assert_eq!(r("turn on night light").unwrap().1["what"], "nightlight");
        assert_eq!(r("turn off dark mode").unwrap().1["on"], false);
        assert_eq!(r("switch to dark mode").unwrap().1["on"], true);
        assert_eq!(r("wifi off").unwrap().1["what"], "wifi");
        assert_eq!(r("turn on airplane mode").unwrap().1["what"], "airplane");
        assert_eq!(r("connect to my headphones").unwrap().1["name"], "my headphones");
        assert_eq!(r("restart the computer").unwrap().1["op"], "reboot");
        assert_eq!(r("how much battery is left").unwrap().0, "sys.battery");
        assert_eq!(r("turn on battery saver").unwrap().1["name"], "power-saver");
        assert_eq!(r("set firefox as my default browser").unwrap().1["role"], "browser");
        assert_eq!(r("stop spotify opening at startup").unwrap().1["app"], "spotify");
        assert_eq!(r("change the resolution to 1280 by 720").unwrap().1["w"], 1280);
        assert_eq!(r("set the scale to 150 percent").unwrap().1["scale"], 1.5);
        assert_eq!(r("rename report.pdf to final").unwrap().1["to"], "final");
        assert_eq!(r("zip the folder photos").unwrap().1["name"], "photos");
        assert_eq!(r("open notes.txt in gedit").unwrap().1["app"], "gedit");
        assert_eq!(r("switch the audio output to headphones").unwrap().1["name"], "headphones");
        assert_eq!(r("eject the usb drive").unwrap().1["name"], "usb drive");
        assert!(r("open firefox").is_none());
        assert!(r("open the third link in a new tab").is_none());
        assert!(r("keep it").is_none());
    }

    #[test]
    fn the_router_sends_settings_here() {
        for (t, want) in [("turn on night light", "sys.toggle"), ("shut down the computer", "sys.power"), ("what's my ip address", "sys.ip"), ("check for updates", "sys.updates"), ("empty the trash", "sys.trash"), ("read my notifications", "sys.notifications")] {
            match crate::router::rules(t, "stop", false) {
                Some(Intent::Act { tool, .. }) => assert_eq!(tool, want, "{t}"),
                other => panic!("{t}: {other:?}"),
            }
        }
    }
}
