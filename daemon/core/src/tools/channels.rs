use crate::router::Intent;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const EXTS: &[&str] = &["pdf", "doc", "docx", "odt", "txt", "md", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp", "png", "jpg", "jpeg", "gif", "webp", "svg", "mp3", "mp4", "mkv", "wav", "zip", "tar", "gz", "iso", "exe", "msi", "dmg", "deb", "rpm", "json", "html", "py", "rs", "js"];

fn split_ext(name: &str) -> (String, Option<String>) {
    let name = name.trim();
    if let Some((stem, last)) = name.rsplit_once(' ')
        && EXTS.contains(&last)
        && !stem.trim().is_empty()
    {
        return (stem.trim().to_owned(), Some(last.to_owned()));
    }
    (name.to_owned(), None)
}

pub fn parse_cell(c: &str) -> Option<(i32, i32, String)> {
    let c = c.trim().trim_start_matches("cell ").replace(' ', "");
    let letters: String = c.chars().take_while(|ch| ch.is_ascii_alphabetic()).collect();
    let digits: String = c.chars().skip(letters.len()).collect();
    if letters.is_empty() || letters.len() > 3 || digits.is_empty() || !digits.chars().all(|d| d.is_ascii_digit()) {
        return None;
    }
    let row: i32 = digits.parse().ok().filter(|r: &i32| *r >= 1 && *r <= 1_048_576)?;
    let col = letters.to_ascii_uppercase().bytes().fold(0i32, |a, b| a * 26 + (b - b'A' + 1) as i32);
    Some((row - 1, col - 1, format!("{}{row}", letters.to_ascii_uppercase())))
}

pub fn rules(t: &str) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    let t = t.trim().trim_end_matches(" please").trim();
    for p in ["put ", "write ", "enter ", "type "] {
        if let Some(r) = t.strip_prefix(p)
            && let Some((value, cell)) = r.rsplit_once(" in cell ").or_else(|| r.rsplit_once(" into cell ")).or_else(|| r.rsplit_once(" in ")).or_else(|| r.rsplit_once(" into "))
            && let Some((_, _, name)) = parse_cell(cell)
            && !value.trim().is_empty()
        {
            return act("doc.cell", json!({ "cell": name, "value": value.trim() }));
        }
    }
    for p in ["set cell ", "set ", "change cell ", "change "] {
        if let Some(r) = t.strip_prefix(p)
            && let Some((cell, value)) = r.split_once(" to ")
            && let Some((_, _, name)) = parse_cell(cell)
            && !value.trim().is_empty()
        {
            return act("doc.cell", json!({ "cell": name, "value": value.trim() }));
        }
    }
    for p in ["what is in cell ", "what's in cell ", "whats in cell ", "what is in ", "whats in ", "what's in ", "read cell ", "what does cell ", "what does "] {
        if let Some(r) = t.strip_prefix(p) {
            let r = r.trim_end_matches(" say").trim_end_matches(" have").trim();
            if let Some((_, _, name)) = parse_cell(r) {
                return act("doc.cell", json!({ "cell": name }));
            }
        }
    }
    if matches!(t, "save this as pdf" | "save this as a pdf" | "save it as pdf" | "save it as a pdf" | "save as pdf" | "save as a pdf" | "export as pdf" | "export as a pdf" | "export this as pdf" | "export this as a pdf" | "export to pdf" | "make a pdf of this" | "turn this into a pdf") {
        return act("doc.export", json!({ "format": "pdf" }));
    }
    if matches!(t, "lock the screen" | "lock my screen" | "lock screen" | "lock the computer" | "lock my computer" | "lock the pc" | "lock my pc" | "lock it" | "lock the laptop" | "lock my laptop") {
        return act("system.lock", json!({}));
    }
    if matches!(t, "which folder is open" | "what folder is open" | "which folder am i in" | "what folder am i in" | "what folder is this" | "which folder is this" | "where am i in files" | "what folders are open" | "which folders are open") {
        return act("files.where", json!({}));
    }
    for p in ["show me ", "show ", "reveal ", "highlight "] {
        let Some(r) = t.strip_prefix(p) else { continue };
        for tail in [" in the folder", " in its folder", " in the file manager", " in files", " in file explorer", " in explorer", " in finder", " in nautilus", " in the files app"] {
            if let Some(name) = r.strip_suffix(tail) {
                let name = name.trim().trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" file").trim();
                if name.is_empty() || name.split_whitespace().count() > 6 {
                    return None;
                }
                let (name, ext) = split_ext(name);
                return act("files.reveal", json!({ "name": name, "ext": ext }));
            }
        }
    }
    for p in ["open the folder with ", "open the folder that has ", "open the folder containing ", "show the folder with ", "show me the folder with ", "show the folder that has "] {
        if let Some(r) = t.strip_prefix(p) {
            let name = r.trim().trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" file").trim();
            if name.is_empty() || name.split_whitespace().count() > 6 {
                return None;
            }
            let (name, ext) = split_ext(name);
            return act("files.reveal", json!({ "name": name, "ext": ext }));
        }
    }
    if matches!(t, "list my extensions" | "what extensions do i have" | "which extensions do i have" | "show my extensions" | "list extensions" | "what extensions are on") {
        return act("shell.extension", json!({ "op": "list" }));
    }
    for (p, on) in [("turn off the ", false), ("turn on the ", true), ("disable the ", false), ("enable the ", true), ("switch off the ", false), ("switch on the ", true), ("turn off ", false), ("turn on ", true), ("disable ", false), ("enable ", true)] {
        if let Some(r) = t.strip_prefix(p)
            && let Some(name) = r.strip_suffix(" extension")
        {
            let name = name.trim();
            if name.is_empty() || name.split_whitespace().count() > 6 {
                return None;
            }
            return act("shell.extension", json!({ "op": if on { "on" } else { "off" }, "name": name }));
        }
    }
    None
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn uri(p: &Path) -> String {
    let mut out = String::from("file://");
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn from_uri(u: &str) -> Option<PathBuf> {
    let rest = u.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

pub async fn reveal(path: &Path) -> Result<&'static str> {
    imp::reveal(path).await
}

pub async fn open_folders() -> Result<Vec<PathBuf>> {
    imp::open_folders().await
}

pub async fn lock() -> Result<()> {
    imp::lock().await
}

pub async fn extension(op: &str, name: &str) -> Result<String> {
    imp::extension(op, name).await
}

pub async fn warm() {
    #[cfg(target_os = "linux")]
    imp::warm().await;
}

pub async fn refill() {
    #[cfg(target_os = "linux")]
    imp::refill().await;
}

pub async fn cell(cell: &str, value: Option<&str>) -> Result<String> {
    let (row, col, name) = parse_cell(cell).ok_or_else(|| anyhow!("{cell} isn't a cell name. Say a letter and a number, like B3."))?;
    imp::cell(row, col, &name, value).await
}

pub async fn export_pdf() -> Result<String> {
    imp::export_pdf().await
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use crate::layers::{self, Layer};
    use std::collections::HashMap;
    use zbus::Connection;
    use zbus::zvariant::{OwnedValue, Value as ZValue};

    const FM: &str = "org.freedesktop.FileManager1";
    const FM_PATH: &str = "/org/freedesktop/FileManager1";
    const SHELL: &str = "org.gnome.Shell";
    const EXT: &str = "org.gnome.Shell.Extensions";

    pub async fn reveal(path: &Path) -> Result<&'static str> {
        let s = layers::effect(Layer::Channel, "channel.files");
        let r = async {
            let c = crate::syscfg_linux::session().await?;
            let what = vec![super::uri(path)];
            let method = if path.is_dir() { "ShowFolders" } else { "ShowItems" };
            c.call_method(Some(FM), FM_PATH, Some(FM), method, &(what, "")).await.map_err(|e| anyhow!("The file manager didn't answer: {e}"))?;
            Ok::<_, anyhow::Error>("the file manager")
        }
        .await;
        s.end(r)
    }

    pub async fn open_folders() -> Result<Vec<PathBuf>> {
        let s = layers::effect(Layer::Channel, "channel.files.open");
        let r = async {
            let c = crate::syscfg_linux::session().await?;
            let has: bool = c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(FM,)).await?.body().deserialize()?;
            if !has {
                return Ok(Vec::new());
            }
            let v: OwnedValue = c.call_method(Some(FM), FM_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(FM, "OpenWindowsWithLocations")).await?.body().deserialize()?;
            let map = HashMap::<String, Vec<String>>::try_from(v).unwrap_or_default();
            let mut out: Vec<PathBuf> = Vec::new();
            for uris in map.values() {
                for u in uris {
                    if let Some(p) = super::from_uri(u)
                        && !out.contains(&p)
                    {
                        out.push(p);
                    }
                }
            }
            Ok::<_, anyhow::Error>(out)
        }
        .await;
        s.end(r)
    }

    pub async fn lock() -> Result<()> {
        let s = layers::effect(Layer::System, "system.lock");
        let r = async {
            let c = crate::syscfg_linux::session().await?;
            for (dest, path, iface) in [("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver"), ("org.freedesktop.ScreenSaver", "/ScreenSaver", "org.freedesktop.ScreenSaver")] {
                let has: bool = c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(dest,)).await?.body().deserialize()?;
                let activatable = !has && {
                    let names: Vec<String> = c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListActivatableNames", &()).await?.body().deserialize()?;
                    names.iter().any(|n| n == dest)
                };
                if (has || activatable) && c.call_method(Some(dest), path, Some(iface), "Lock", &()).await.is_ok() {
                    return Ok(());
                }
            }
            let sys = Connection::system().await?;
            sys.call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "Lock", &()).await.map_err(|e| anyhow!("The screen couldn't be locked: {e}"))?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        s.end(r)
    }

    fn spreadsheet(w: &crate::os::Win) -> bool {
        w.title.contains("LibreOffice Calc") || w.title.ends_with(" - Calc")
    }

    pub async fn cell(row: i32, col: i32, name: &str, value: Option<&str>) -> Result<String> {
        let w = crate::os::working().await?;
        if !spreadsheet(&w) {
            return Err(anyhow!("Open the spreadsheet in LibreOffice Calc first."));
        }
        let Some(v) = value else {
            let t = crate::os::table_cell(&w, row, col).await.ok_or_else(|| anyhow!("I couldn't read {name} in this spreadsheet."))?;
            let t = t.trim();
            return Ok(if t.is_empty() { format!("{name} is empty") } else { format!("{name} has {t}") });
        };
        use crate::tools::input::{self, K};
        if !crate::os::in_front(&w, 800).await {
            return Err(anyhow!("Bring the spreadsheet to the front first."));
        }
        input::press(&[K::Ctrl, K::Shift, K::F(5)]).await?;
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        input::press(&[K::Ctrl, K::Ch('a')]).await?;
        input::type_text(name).await?;
        input::press(&[K::Enter]).await?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        input::type_text(v).await?;
        input::press(&[K::Enter]).await?;
        Ok(format!("Put {v} in {name}"))
    }

    pub async fn export_pdf() -> Result<String> {
        super::office_convert().await
    }

    fn text(v: &ZValue) -> String {
        match v {
            ZValue::Str(s) => s.to_string(),
            ZValue::Value(inner) => text(inner),
            _ => String::new(),
        }
    }

    fn number(v: &ZValue) -> f64 {
        match v {
            ZValue::F64(n) => *n,
            ZValue::U32(n) => *n as f64,
            ZValue::I32(n) => *n as f64,
            ZValue::Value(inner) => number(inner),
            _ => 0.0,
        }
    }

    type Row = (String, String, bool);
    static EXTS: std::sync::Mutex<Option<(u64, Vec<Row>)>> = std::sync::Mutex::new(None);
    static EXT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static EXT_WATCHED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    async fn ext_rows(c: &Connection) -> Result<Vec<Row>> {
        use std::sync::atomic::Ordering::SeqCst;
        let watched = EXT_WATCHED.load(SeqCst) == 2;
        let before = EXT_EPOCH.load(SeqCst);
        if watched && let Some((e, rows)) = EXTS.lock().unwrap().as_ref() && *e == before {
            layers::read(Layer::Channel, "channel.shell.extensions.warm").ok();
            return Ok(rows.clone());
        }
        let s = layers::read(Layer::Channel, "channel.shell.extensions");
        let listed = s.end(
            c.call_method(Some(SHELL), "/org/gnome/Shell", Some(EXT), "ListExtensions", &())
                .await
                .map_err(|_| anyhow!("Extensions are a GNOME feature, and GNOME Shell isn't answering here.")),
        )?;
        let all: HashMap<String, HashMap<String, OwnedValue>> = listed.body().deserialize()?;
        let mut rows: Vec<Row> = all
            .iter()
            .map(|(uuid, info)| {
                let title = info.get("name").map(|v| text(v)).unwrap_or_else(|| uuid.clone());
                let enabled = info.get("enabled").is_some_and(|v| matches!(&**v, ZValue::Bool(true))) || info.get("state").map(|v| number(v)) == Some(1.0);
                (uuid.clone(), title, enabled)
            })
            .collect();
        rows.sort_by_key(|a| a.1.to_lowercase());
        if watched && EXT_EPOCH.load(SeqCst) == before {
            *EXTS.lock().unwrap() = Some((before, rows.clone()));
        }
        Ok(rows)
    }

    pub async fn warm() {
        use futures_util::StreamExt;
        use std::sync::atomic::Ordering::SeqCst;
        let Ok(c) = crate::syscfg_linux::session().await else { return };
        let rules = [
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(EXT).and_then(|b| b.member("ExtensionStateChanged")).map(|b| b.build()),
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg(0, SHELL)).map(|b| b.build()),
        ];
        for r in rules.into_iter().flatten() {
            if let Ok(mut st) = zbus::MessageStream::for_match_rule(r, c, Some(16)).await {
                EXT_WATCHED.fetch_add(1, SeqCst);
                tokio::spawn(async move {
                    while st.next().await.is_some() {
                        EXT_EPOCH.fetch_add(1, SeqCst);
                    }
                    EXT_WATCHED.fetch_sub(1, SeqCst);
                    EXT_EPOCH.fetch_add(1, SeqCst);
                });
            }
        }
        EXT_EPOCH.fetch_add(1, SeqCst);
        let _ = ext_rows(c).await;
    }

    pub async fn refill() {
        use std::sync::atomic::Ordering::SeqCst;
        if EXT_WATCHED.load(SeqCst) != 2 || EXTS.lock().unwrap().as_ref().is_some_and(|(e, _)| *e == EXT_EPOCH.load(SeqCst)) {
            return;
        }
        if let Ok(c) = crate::syscfg_linux::session().await {
            let _ = ext_rows(c).await;
        }
    }

    pub async fn extension(op: &str, name: &str) -> Result<String> {
        let c = crate::syscfg_linux::session().await?;
        let rows = ext_rows(c).await?;
        if op == "list" {
            if rows.is_empty() {
                return Ok("You have no GNOME extensions".into());
            }
            let on: Vec<&str> = rows.iter().filter(|r| r.2).map(|r| r.1.as_str()).collect();
            let off: Vec<&str> = rows.iter().filter(|r| !r.2).map(|r| r.1.as_str()).collect();
            let mut out = String::new();
            if !on.is_empty() {
                out.push_str(&format!("On: {}.", on.join(", ")));
            }
            if !off.is_empty() {
                out.push_str(&format!("{}Off: {}.", if out.is_empty() { "" } else { " " }, off.join(", ")));
            }
            return Ok(out);
        }
        let want = crate::text::normalize(name);
        let best = rows
            .iter()
            .map(|r| {
                let n = crate::text::normalize(&r.1);
                let u = crate::text::normalize(&r.0.split('@').next().unwrap_or("").replace('-', " "));
                let score = crate::text::similarity(&want, &n).max(crate::text::similarity(&want, &u)).max(if n.contains(&want) && want.len() >= 3 { 0.9 } else { 0.0 });
                (score, r)
            })
            .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let Some((score, (uuid, title, enabled))) = best else { return Err(anyhow!("You have no GNOME extensions.")) };
        if score < 0.6 {
            return Err(anyhow!("I couldn't find an extension called {name}."));
        }
        let on = op == "on";
        if *enabled == on {
            return Ok(format!("{title} is already {}", if on { "on" } else { "off" }));
        }
        let s = layers::effect(Layer::Channel, "channel.shell.extensions");
        let reply = s.end(c.call_method(Some(SHELL), "/org/gnome/Shell", Some(EXT), if on { "EnableExtension" } else { "DisableExtension" }, &(uuid.as_str(),)).await.map_err(|e| anyhow!("GNOME didn't change {title}: {e}")))?;
        let done: bool = reply.body().deserialize().unwrap_or(false);
        if !done {
            return Err(anyhow!("GNOME didn't change {title}."));
        }
        Ok(format!("Turned {} {title}", if on { "on" } else { "off" }))
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::layers::{self, Layer};

    pub async fn reveal(path: &Path) -> Result<&'static str> {
        let p = path.to_path_buf();
        let s = layers::effect(Layer::System, "system.reveal");
        let r = crate::layers::blocking(move || -> Result<&'static str> {
            use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
            use windows::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};
            use windows::core::HSTRING;
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let pidl = ILCreateFromPathW(&HSTRING::from(p.as_os_str()));
                if pidl.is_null() {
                    return Err(anyhow!("Windows couldn't find that file."));
                }
                let r = SHOpenFolderAndSelectItems(pidl, None, 0);
                ILFree(Some(pidl));
                r.map_err(|e| anyhow!("File Explorer didn't open: {e}"))?;
            }
            Ok("File Explorer")
        })
        .await
        .map_err(|e| anyhow!("{e}"))
        .and_then(|r| r);
        s.end(r)
    }

    pub async fn open_folders() -> Result<Vec<PathBuf>> {
        crate::layers::blocking(super::com::explorer_folders).await.map_err(|e| anyhow!("{e}"))?
    }

    pub async fn lock() -> Result<()> {
        let s = layers::effect(Layer::System, "system.lock");
        let ok = unsafe { windows::Win32::System::Shutdown::LockWorkStation() }.is_ok();
        s.end(if ok { Ok(()) } else { Err(anyhow!("Windows didn't lock the screen.")) })
    }

    pub async fn extension(_op: &str, _name: &str) -> Result<String> {
        Err(anyhow!("Desktop extensions are a GNOME feature. Windows doesn't have them."))
    }

    pub async fn cell(_row: i32, _col: i32, name: &str, value: Option<&str>) -> Result<String> {
        let name = name.to_owned();
        let value = value.map(str::to_owned);
        let s = layers::effect(Layer::Channel, "channel.office");
        let r = crate::layers::blocking(move || super::com::excel_cell(&name, value.as_deref())).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        s.end(r)
    }

    pub async fn export_pdf() -> Result<String> {
        let s = layers::effect(Layer::Channel, "channel.office");
        let r = crate::layers::blocking(super::com::office_pdf).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        s.end(r)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    pub async fn reveal(path: &Path) -> Result<&'static str> {
        let p = path.to_path_buf();
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.open");
        let ok = crate::layers::blocking(move || std::process::Command::new("open").arg("-R").arg(&p).status().map(|s| s.success()).unwrap_or(false)).await.unwrap_or(false);
        span.end_bool(ok);
        if ok { Ok("Finder") } else { Err(anyhow!("Finder couldn't show it.")) }
    }

    pub async fn open_folders() -> Result<Vec<PathBuf>> {
        Err(anyhow!("I can't read Finder's open folders yet."))
    }

    pub async fn lock() -> Result<()> {
        use crate::tools::input::K;
        crate::tools::input::press(&[K::Ctrl, K::Meta, K::Ch('q')]).await
    }

    pub async fn extension(_op: &str, _name: &str) -> Result<String> {
        Err(anyhow!("Desktop extensions are a GNOME feature. macOS doesn't have them."))
    }

    pub async fn cell(_row: i32, _col: i32, _name: &str, _value: Option<&str>) -> Result<String> {
        Err(anyhow!("Cells by name work in Excel on Windows and LibreOffice Calc on Linux for now."))
    }

    pub async fn export_pdf() -> Result<String> {
        super::office_convert().await
    }
}

#[cfg(unix)]
fn soffice() -> Option<PathBuf> {
    ["/usr/bin/soffice", "/usr/bin/libreoffice", "/usr/lib64/libreoffice/program/soffice", "/opt/libreoffice/program/soffice", "/Applications/LibreOffice.app/Contents/MacOS/soffice"].iter().map(PathBuf::from).find(|p| p.exists()).or_else(|| crate::tools::apps::which("soffice"))
}

#[cfg(unix)]
pub fn document_name(title: &str) -> Option<String> {
    let t = title.trim().trim_start_matches('*').trim();
    let name = t.split(" - LibreOffice").next().or_else(|| t.split(" \u{2014} LibreOffice").next())?.trim();
    (name.contains('.') && !name.starts_with("Untitled")).then(|| name.to_owned())
}

#[cfg(unix)]
async fn office_convert() -> Result<String> {
    let w = crate::os::front().await.map_err(|_| anyhow!("Bring the document to the front first."))?;
    let name = document_name(&w.title).ok_or_else(|| anyhow!("Save the document first; I export the saved file."))?;
    let n = name.clone();
    let hits = crate::layers::blocking(move || crate::tools::files::search(&n, Some(false), None, std::time::Duration::from_millis(3000))).await.unwrap_or_default();
    let file = hits.into_iter().map(|h| h.path).find(|p| p.file_name().is_some_and(|f| f.to_string_lossy() == name)).ok_or_else(|| anyhow!("I couldn't find {name} on disk. Save it first."))?;
    let office = soffice().ok_or_else(|| anyhow!("LibreOffice isn't installed."))?;
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let work = std::env::temp_dir().join(format!("setu-office-{}", std::process::id()));
    let _ = std::fs::create_dir_all(work.join("out"));
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.office");
    let (f2, w2) = (file.clone(), work.clone());
    let ok = crate::work::background("office", move || {
        std::process::Command::new(&office)
            .arg(format!("-env:UserInstallation=file://{}", w2.join("profile").display()))
            .args(["--headless", "--norestore", "--convert-to", "pdf", "--outdir"])
            .arg(w2.join("out"))
            .arg(&f2)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    let made = work.join("out").join(format!("{stem}.pdf"));
    span.end_bool(ok && made.exists());
    if !made.exists() {
        let _ = std::fs::remove_dir_all(&work);
        return Err(anyhow!("LibreOffice couldn't make the PDF."));
    }
    let dest = crate::fileops::free_name(&dir, &stem, "pdf");
    let moved = std::fs::rename(&made, &dest).or_else(|_| std::fs::copy(&made, &dest).map(|_| ()));
    let _ = std::fs::remove_dir_all(&work);
    moved?;
    Ok(format!("Saved {} from the last saved version", crate::tools::files::spoken_path(&dest)))
}

pub fn mail_rule(t: &str, raw: &str) -> Option<Intent> {
    let lower = raw.trim().to_lowercase();
    for p in ["write an email to ", "email ", "send an email to ", "draft an email to ", "compose an email to ", "write a mail to ", "mail "] {
        if !t.starts_with(p) || !lower.starts_with(p) {
            continue;
        }
        let rest = &raw.trim()[p.len()..];
        let rl = rest.to_lowercase();
        let (head, body) = match rl.find(" saying ").or_else(|| rl.find(" that says ")) {
            Some(i) => {
                let skip = if rl[i..].starts_with(" saying ") { 8 } else { 11 };
                (&rest[..i], rest[i + skip..].trim())
            }
            None => (rest, ""),
        };
        let hl = head.to_lowercase();
        let (to, subject) = match hl.find(" about ") {
            Some(i) => (head[..i].trim(), head[i + 7..].trim()),
            None => (head.trim(), ""),
        };
        if to.is_empty() || to.split_whitespace().count() > 4 {
            return None;
        }
        return Some(Intent::Act { tool: "doc.mail".into(), args: json!({ "to": to, "subject": subject, "body": body }) });
    }
    if matches!(t, "send the email" | "send this email" | "send it now" | "send the mail") {
        return Some(Intent::Act { tool: "doc.mail".into(), args: json!({ "send": true }) });
    }
    None
}

fn url_part(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub async fn mail(to: &str, subject: &str, body: &str, send: bool) -> Result<String> {
    #[cfg(windows)]
    {
        let (a, b, c) = (to.to_owned(), subject.to_owned(), body.to_owned());
        let r = crate::layers::blocking(move || com::outlook_mail(&a, &b, &c, send)).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        if r.is_ok() || send {
            return r;
        }
    }
    if send {
        return Err(anyhow!("Sending works with Outlook on Windows. In other mail apps, press Send yourself."));
    }
    let link = format!("mailto:{}?subject={}&body={}", url_part(to), url_part(subject), url_part(body));
    let span = crate::layers::effect(crate::layers::Layer::System, "system.open");
    span.end(crate::tools::shell::open(&link))?;
    Ok(format!("Opened a new email to {to} in your mail app. Check it and press Send."))
}

#[cfg(windows)]
#[allow(dead_code)]
pub mod com {
    use anyhow::{Result, anyhow};
    use std::path::PathBuf;
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, IDispatch, CLSIDFromProgID};
    use windows::Win32::System::Ole::{DISPID_PROPERTYPUT, GetActiveObject};
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VT_DISPATCH};
    use windows::core::{BSTR, GUID, HSTRING, Interface, PCWSTR};

    pub fn init() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
    }

    pub fn clsid(prog: &str) -> Result<GUID> {
        unsafe { CLSIDFromProgID(&HSTRING::from(prog)).map_err(|_| anyhow!("{prog} isn't installed.")) }
    }

    pub fn running(prog: &str) -> Result<IDispatch> {
        init();
        let id = clsid(prog)?;
        unsafe {
            let mut unk = None;
            GetActiveObject(&id, None, &mut unk).map_err(|_| anyhow!("{} isn't open.", prog.split('.').next().unwrap_or(prog)))?;
            unk.ok_or_else(|| anyhow!("{prog} isn't open."))?.cast::<IDispatch>().map_err(|e| anyhow!("{e}"))
        }
    }

    pub fn create(prog: &str) -> Result<IDispatch> {
        init();
        let id = clsid(prog)?;
        unsafe { CoCreateInstance(&id, None, CLSCTX_ALL).map_err(|e| anyhow!("{prog} didn't start: {e}")) }
    }

    pub fn attach(prog: &str) -> Result<IDispatch> {
        running(prog).or_else(|_| create(prog))
    }

    fn id_of(d: &IDispatch, name: &str) -> Result<i32> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let names = [PCWSTR(wide.as_ptr())];
        let mut id = 0i32;
        unsafe { d.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0x0400, &mut id).map_err(|_| anyhow!("The app has no {name}."))? };
        Ok(id)
    }

    fn invoke(d: &IDispatch, name: &str, flags: DISPATCH_FLAGS, args: &[VARIANT]) -> Result<VARIANT> {
        let id = id_of(d, name)?;
        let mut rev: Vec<VARIANT> = args.iter().rev().cloned().collect();
        let mut put = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: if rev.is_empty() { std::ptr::null_mut() } else { rev.as_mut_ptr() },
            rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT { &mut put } else { std::ptr::null_mut() },
            cArgs: rev.len() as u32,
            cNamedArgs: if flags == DISPATCH_PROPERTYPUT { 1 } else { 0 },
        };
        let mut out = VARIANT::default();
        unsafe { d.Invoke(id, &GUID::zeroed(), 0x0400, flags, &params, Some(&mut out), None, None).map_err(|e| anyhow!("{name} failed: {e}"))? };
        Ok(out)
    }

    pub fn get(d: &IDispatch, name: &str) -> Result<VARIANT> {
        invoke(d, name, DISPATCH_PROPERTYGET, &[])
    }

    pub fn get_with(d: &IDispatch, name: &str, args: &[VARIANT]) -> Result<VARIANT> {
        invoke(d, name, DISPATCH_PROPERTYGET | DISPATCH_METHOD, args)
    }

    pub fn put(d: &IDispatch, name: &str, v: VARIANT) -> Result<()> {
        invoke(d, name, DISPATCH_PROPERTYPUT, &[v]).map(|_| ())
    }

    pub fn call(d: &IDispatch, name: &str, args: &[VARIANT]) -> Result<VARIANT> {
        invoke(d, name, DISPATCH_METHOD, args)
    }

    pub fn object(v: &VARIANT) -> Result<IDispatch> {
        unsafe {
            if v.Anonymous.Anonymous.vt != VT_DISPATCH {
                return Err(anyhow!("The app returned no object."));
            }
            (*v.Anonymous.Anonymous.Anonymous.pdispVal).clone().ok_or_else(|| anyhow!("The app returned an empty object."))
        }
    }

    pub fn text(v: &VARIANT) -> String {
        unsafe {
            if v.Anonymous.Anonymous.vt == VT_BSTR {
                return v.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
            }
        }
        BSTR::try_from(v).map(|b| b.to_string()).unwrap_or_default()
    }

    pub fn s(v: &str) -> VARIANT {
        VARIANT::from(BSTR::from(v))
    }

    pub fn excel_cell(cell: &str, value: Option<&str>) -> Result<String> {
        let app = running("Excel.Application").map_err(|_| anyhow!("Open the spreadsheet in Excel first."))?;
        let sheet = object(&get(&app, "ActiveSheet")?)?;
        let range = object(&get_with(&sheet, "Range", &[s(cell)])?)?;
        match value {
            Some(v) => {
                put(&range, "Formula", s(v))?;
                Ok(format!("Put {v} in {cell}"))
            }
            None => {
                let t = text(&get(&range, "Text")?);
                Ok(if t.trim().is_empty() { format!("{cell} is empty") } else { format!("{cell} has {}", t.trim()) })
            }
        }
    }

    fn pdf_path(full: &str) -> PathBuf {
        let p = PathBuf::from(full);
        if p.parent().is_some_and(|d| d.as_os_str().len() > 0) {
            p.with_extension("pdf")
        } else {
            dirs::document_dir().unwrap_or_default().join(format!("{}.pdf", p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into())))
        }
    }

    pub fn office_pdf() -> Result<String> {
        if let Ok(word) = running("Word.Application")
            && let Ok(doc) = get(&word, "ActiveDocument").and_then(|v| object(&v))
        {
            let out = pdf_path(&text(&get(&doc, "FullName")?));
            call(&doc, "ExportAsFixedFormat", &[s(&out.to_string_lossy()), VARIANT::from(17i32)])?;
            return Ok(format!("Saved {} as a PDF", crate::tools::files::spoken_path(&out)));
        }
        if let Ok(excel) = running("Excel.Application")
            && let Ok(book) = get(&excel, "ActiveWorkbook").and_then(|v| object(&v))
        {
            let out = pdf_path(&text(&get(&book, "FullName")?));
            call(&book, "ExportAsFixedFormat", &[VARIANT::from(0i32), s(&out.to_string_lossy())])?;
            return Ok(format!("Saved {} as a PDF", crate::tools::files::spoken_path(&out)));
        }
        if let Ok(ppt) = running("PowerPoint.Application")
            && let Ok(deck) = get(&ppt, "ActivePresentation").and_then(|v| object(&v))
        {
            let out = pdf_path(&text(&get(&deck, "FullName")?));
            call(&deck, "SaveAs", &[s(&out.to_string_lossy()), VARIANT::from(32i32)])?;
            return Ok(format!("Saved {} as a PDF", crate::tools::files::spoken_path(&out)));
        }
        Err(anyhow!("Open the document in Word, Excel or PowerPoint first."))
    }

    pub fn outlook_mail(to: &str, subject: &str, body: &str, send: bool) -> Result<String> {
        init();
        let app = attach("Outlook.Application")?;
        if send {
            let insp = object(&get(&app, "ActiveInspector")?).map_err(|_| anyhow!("No email is open in Outlook."))?;
            let item = object(&get(&insp, "CurrentItem")?)?;
            let to_now = text(&get(&item, "To")?);
            call(&item, "Send", &[])?;
            return Ok(format!("Sent the email to {to_now}"));
        }
        let item = object(&call(&app, "CreateItem", &[VARIANT::from(0i32)])?)?;
        put(&item, "To", s(to))?;
        if !subject.is_empty() {
            put(&item, "Subject", s(subject))?;
        }
        if !body.is_empty() {
            put(&item, "Body", s(body))?;
        }
        call(&item, "Display", &[VARIANT::from(false)])?;
        Ok(format!("Wrote an email to {to} in Outlook. Say send the email when it looks right."))
    }

    pub fn explorer_folders() -> Result<Vec<PathBuf>> {
        let shell = create("Shell.Application")?;
        let wins = object(&call(&shell, "Windows", &[])?)?;
        let n = i32::try_from(&get(&wins, "Count")?).unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..n {
            let Ok(item) = call(&wins, "Item", &[VARIANT::from(i)]).and_then(|v| object(&v)) else { continue };
            let Ok(doc) = get(&item, "Document").and_then(|v| object(&v)) else { continue };
            let Ok(folder) = get(&doc, "Folder").and_then(|v| object(&v)) else { continue };
            let Ok(me) = get(&folder, "Self").and_then(|v| object(&v)) else { continue };
            let path = text(&get(&me, "Path")?);
            if !path.is_empty() && !path.starts_with("::") {
                out.push(PathBuf::from(path));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(t: &str) -> Option<(String, Value)> {
        match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn understands_channel_phrases() {
        let r = tool("show me invoice pdf in the folder").unwrap();
        assert_eq!((r.0.as_str(), r.1["name"].clone(), r.1["ext"].clone()), ("files.reveal", json!("invoice"), json!("pdf")));
        assert_eq!(tool("show the report in files").map(|x| x.1["name"].clone()), Some(json!("report")));
        assert_eq!(tool("open the folder with my resume").map(|x| x.1["name"].clone()), Some(json!("resume")));
        assert_eq!(tool("which folder is open").map(|x| x.0), Some("files.where".into()));
        assert_eq!(tool("lock the screen").map(|x| x.0), Some("system.lock".into()));
        let e = tool("turn off the dash to dock extension").unwrap();
        assert_eq!((e.0.as_str(), e.1["op"].clone(), e.1["name"].clone()), ("shell.extension", json!("off"), json!("dash to dock")));
        assert_eq!(tool("list my extensions").map(|x| x.1["op"].clone()), Some(json!("list")));
        assert_eq!(tool("put 42 in cell b3").map(|x| (x.0, x.1["cell"].clone(), x.1["value"].clone())), Some(("doc.cell".into(), json!("B3"), json!("42"))));
        assert_eq!(tool("set c10 to hello there").map(|x| (x.1["cell"].clone(), x.1["value"].clone())), Some((json!("C10"), json!("hello there"))));
        assert_eq!(tool("what is in cell aa7").map(|x| (x.1["cell"].clone(), x.1.get("value").cloned())), Some((json!("AA7"), None)));
        assert_eq!(tool("save this as a pdf").map(|x| x.0), Some("doc.export".into()));
        let m = |t: &str| match mail_rule(&crate::text::normalize(t), t) {
            Some(Intent::Act { args, .. }) => Some(args),
            _ => None,
        };
        let a = m("Email Riya about lunch saying See you at one?").unwrap();
        assert_eq!((a["to"].as_str(), a["subject"].as_str(), a["body"].as_str()), (Some("Riya"), Some("lunch"), Some("See you at one?")));
        assert_eq!(m("write an email to sam@example.com saying hi").unwrap()["to"], "sam@example.com");
        assert!(m("email").is_none());
        #[cfg(unix)]
        {
            assert_eq!(document_name("budget.ods - LibreOffice Calc").as_deref(), Some("budget.ods"));
            assert_eq!(document_name("Untitled 1 - LibreOffice Writer"), None);
        }
        assert_eq!(tool("put the kettle on"), None);
        assert_eq!(tool("set an alarm to 7"), None);
        assert_eq!(parse_cell("b3"), Some((2, 1, "B3".into())));
        assert_eq!(parse_cell("aa1"), Some((0, 26, "AA1".into())));
        assert_eq!(parse_cell("b0"), None);
        assert_eq!(tool("show me the desktop"), None);
        assert_eq!(tool("turn off the lights"), None);
    }

    #[test]
    fn file_uris_round_trip() {
        let p = Path::new("/home/a b/Résumé #1.pdf");
        let u = uri(p);
        assert!(u.starts_with("file:///home/a%20b/R%C3%A9sum%C3%A9%20%231.pdf"), "{u}");
        assert_eq!(from_uri(&u).as_deref(), Some(p));
    }
}
