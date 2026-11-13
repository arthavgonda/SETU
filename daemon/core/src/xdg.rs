use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn env_dir(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    env_dir("XDG_CONFIG_HOME", ".config")
}

pub fn data_home() -> PathBuf {
    env_dir("XDG_DATA_HOME", ".local/share")
}

fn list_var(var: &str, fallback: &str) -> Vec<PathBuf> {
    std::env::var(var).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| fallback.into()).split(':').map(PathBuf::from).filter(|p| p.is_absolute()).collect()
}

pub fn data_dirs() -> Vec<PathBuf> {
    let mut v = vec![data_home()];
    v.extend(list_var("XDG_DATA_DIRS", "/usr/local/share:/usr/share"));
    v
}

fn desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|s| !s.is_empty()).map(str::to_lowercase).collect()
}

pub fn ini(text: &str, group: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            inside = l == format!("[{group}]");
            continue;
        }
        if !inside || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            out.entry(k.trim().to_owned()).or_insert_with(|| v.trim().to_owned());
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    pub terminal: bool,
}

pub fn entry(path: &Path) -> Option<Entry> {
    let text = std::fs::read_to_string(path).ok()?;
    let m = ini(&text, "Desktop Entry");
    if m.get("Type").map(String::as_str).unwrap_or("Application") != "Application" {
        return None;
    }
    Some(Entry {
        id: path.file_name()?.to_string_lossy().into_owned(),
        path: path.to_path_buf(),
        name: m.get("Name").cloned().unwrap_or_default(),
        exec: m.get("Exec").cloned().unwrap_or_default(),
        terminal: m.get("Terminal").is_some_and(|v| v == "true"),
    })
}

pub fn find_entry(id: &str) -> Option<PathBuf> {
    let id = if id.ends_with(".desktop") { id.to_owned() } else { format!("{id}.desktop") };
    for d in data_dirs() {
        let base = d.join("applications");
        let direct = base.join(&id);
        if direct.is_file() {
            return Some(direct);
        }
        let parts: Vec<&str> = id.splitn(2, '-').collect();
        if parts.len() == 2 {
            let nested = base.join(parts[0]).join(parts[1]);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

fn unquote(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            _ => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn expand(e: &Entry, files: &[PathBuf]) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = false;
    for a in unquote(&e.exec) {
        match a.as_str() {
            "%f" | "%u" => {
                if let Some(f) = files.first() {
                    out.push(f.to_string_lossy().into_owned());
                }
                used = true;
            }
            "%F" | "%U" => {
                out.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
                used = true;
            }
            "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
            _ => {
                let mut s = String::new();
                let mut it = a.chars().peekable();
                while let Some(c) = it.next() {
                    if c == '%' {
                        match it.next() {
                            Some('%') => s.push('%'),
                            Some('f') | Some('u') => {
                                if let Some(f) = files.first() {
                                    s.push_str(&f.to_string_lossy());
                                }
                                used = true;
                            }
                            _ => {}
                        }
                    } else {
                        s.push(c);
                    }
                }
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
    }
    if !used && !files.is_empty() {
        out.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
    }
    out
}

pub fn spawn(argv: &[String]) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let (prog, args) = argv.split_first().ok_or_else(|| anyhow!("The app has no command to start it."))?;
    let mut cmd = Command::new(prog);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).current_dir(home());
    for v in ["SETU_DATA_DIR", "SETU_RUNTIME_DIR", "SETU_ASSETS", "SETU_LOG", "LD_LIBRARY_PATH"] {
        cmd.env_remove(v);
    }
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| anyhow!("{prog} could not be started: {e}"))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let mut c = child;
        let _ = c.wait();
    });
    Ok(pid)
}

pub fn launch(e: &Entry, files: &[PathBuf]) -> Result<u32> {
    if e.terminal {
        return Err(anyhow!("That app runs in a terminal."));
    }
    let span = crate::layers::effect(crate::layers::Layer::System, "system.launch");
    span.end(spawn(&expand(e, files)))
}

fn globs() -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    for d in data_dirs().iter().rev() {
        let Ok(text) = std::fs::read_to_string(d.join("mime/globs2")) else { continue };
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut p = line.splitn(3, ':');
            let (Some(w), Some(mime), Some(glob)) = (p.next(), p.next(), p.next()) else { continue };
            let glob = glob.split(':').next().unwrap_or(glob);
            out.push((w.parse().unwrap_or(50), mime.to_owned(), glob.to_owned()));
        }
    }
    out
}

fn glob_match(glob: &str, name: &str) -> bool {
    if let Some(ext) = glob.strip_prefix("*.")
        && !ext.contains(['*', '?', '['])
    {
        return name.len() > ext.len() + 1 && name[name.len() - ext.len() - 1..].eq_ignore_ascii_case(&format!(".{ext}"));
    }
    if !glob.contains(['*', '?', '[']) {
        return glob == name;
    }
    false
}

pub fn mime_of(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".into();
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut best: Option<(u32, usize, String)> = None;
    for (w, mime, glob) in globs() {
        if glob_match(&glob, &name) && best.as_ref().is_none_or(|(bw, bl, _)| w > *bw || (w == *bw && glob.len() > *bl)) {
            best = Some((w, glob.len(), mime));
        }
    }
    best.map(|(_, _, m)| m).unwrap_or_else(|| "application/octet-stream".into())
}

fn mimeapps_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let names: Vec<String> = desktops().iter().map(|d| format!("{d}-mimeapps.list")).chain(std::iter::once("mimeapps.list".to_owned())).collect();
    let mut config = vec![config_home()];
    config.extend(list_var("XDG_CONFIG_DIRS", "/etc/xdg"));
    for c in &config {
        for n in &names {
            v.push(c.join(n));
        }
    }
    for d in data_dirs() {
        for n in &names {
            v.push(d.join("applications").join(n));
        }
    }
    v
}

pub fn default_for(mime: &str) -> Option<Entry> {
    for f in mimeapps_files() {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        if let Some(list) = ini(&text, "Default Applications").get(mime) {
            for id in list.split(';').filter(|s| !s.is_empty()) {
                if let Some(e) = find_entry(id).and_then(|p| entry(&p)) {
                    return Some(e);
                }
            }
        }
    }
    for d in data_dirs() {
        let Ok(text) = std::fs::read_to_string(d.join("applications/mimeinfo.cache")) else { continue };
        if let Some(list) = ini(&text, "MIME Cache").get(mime) {
            for id in list.split(';').filter(|s| !s.is_empty()) {
                if let Some(e) = find_entry(id).and_then(|p| entry(&p)) {
                    return Some(e);
                }
            }
        }
    }
    None
}

pub fn open(path: &Path) -> Result<String> {
    let mime = mime_of(path);
    let e = default_for(&mime).ok_or_else(|| anyhow!("No app is set to open {mime} files."))?;
    launch(&e, &[path.to_path_buf()])?;
    Ok(e.name)
}

pub fn set_default(mimes: &[&str], id: &str) -> Result<()> {
    let p = config_home().join("mimeapps.list");
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut out = Vec::new();
    let mut inside = false;
    let mut seen_group = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if inside {
                for m in mimes {
                    out.push(format!("{m}={id};"));
                }
            }
            inside = l == "[Default Applications]";
            seen_group |= inside;
            out.push(line.to_owned());
            continue;
        }
        if inside && l.split_once('=').is_some_and(|(k, _)| mimes.contains(&k.trim())) {
            continue;
        }
        out.push(line.to_owned());
    }
    if inside {
        for m in mimes {
            out.push(format!("{m}={id};"));
        }
    }
    if !seen_group {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.push("[Default Applications]".into());
        for m in mimes {
            out.push(format!("{m}={id};"));
        }
    }
    std::fs::create_dir_all(config_home())?;
    let tmp = p.with_extension("setu-tmp");
    std::fs::write(&tmp, out.join("\n") + "\n")?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

fn url_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn trash_dir() -> PathBuf {
    data_home().join("Trash")
}

pub fn trash(path: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let path = path.canonicalize().map_err(|_| anyhow!("{} isn't there.", path.display()))?;
    let dir = trash_dir();
    let home_dev = std::fs::metadata(data_home()).or_else(|_| std::fs::metadata(home())).map(|m| m.dev()).unwrap_or(0);
    let dev = std::fs::symlink_metadata(&path)?.dev();
    if dev != home_dev {
        return Err(anyhow!("That file is on another drive."));
    }
    std::fs::create_dir_all(dir.join("files"))?;
    std::fs::create_dir_all(dir.join("info"))?;
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let now = jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S").to_string();
    for n in 0..1000 {
        let name = if n == 0 { base.clone() } else { format!("{base}.{n}") };
        let info = dir.join("info").join(format!("{name}.trashinfo"));
        let f = std::fs::OpenOptions::new().write(true).create_new(true).open(&info);
        let Ok(mut f) = f else { continue };
        use std::io::Write;
        writeln!(f, "[Trash Info]\nPath={}\nDeletionDate={now}", url_escape(&path.to_string_lossy()))?;
        let dest = dir.join("files").join(&name);
        if let Err(e) = std::fs::rename(&path, &dest) {
            let _ = std::fs::remove_file(&info);
            return Err(anyhow!("{} couldn't be moved to the trash: {e}", path.display()));
        }
        return Ok(dest);
    }
    Err(anyhow!("The trash is full of files with that name."))
}

pub fn trash_count() -> usize {
    std::fs::read_dir(trash_dir().join("files")).map(|rd| rd.count()).unwrap_or(0)
}

pub fn empty_trash() -> Result<usize> {
    let dir = trash_dir();
    let mut n = 0;
    for sub in ["files", "info"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let r = if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            if r.is_ok() && sub == "files" {
                n += 1;
            }
        }
    }
    let _ = std::fs::remove_file(dir.join("directorysizes"));
    Ok(n)
}

pub fn autostart_dirs() -> Vec<PathBuf> {
    let mut v = vec![config_home().join("autostart")];
    v.extend(list_var("XDG_CONFIG_DIRS", "/etc/xdg").into_iter().map(|d| d.join("autostart")));
    v
}

#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub id: String,
    pub name: String,
    pub on: bool,
}

pub fn startup_list() -> Vec<Startup> {
    let mut seen: HashMap<String, Startup> = HashMap::new();
    for d in autostart_dirs() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".desktop") || seen.contains_key(&id) {
                continue;
            }
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            let m = ini(&text, "Desktop Entry");
            let hidden = m.get("Hidden").is_some_and(|v| v == "true") || m.get("X-GNOME-Autostart-enabled").is_some_and(|v| v == "false");
            let shown_only = m.get("OnlyShowIn").map(|v| v.to_lowercase());
            let fits = shown_only.as_ref().is_none_or(|v| desktops().iter().any(|d| v.contains(d.as_str())));
            let name = m.get("Name").cloned().unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
            seen.insert(id.clone(), Startup { id, name, on: !hidden && fits });
        }
    }
    let mut v: Vec<Startup> = seen.into_values().collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

pub fn set_startup(id: &str, source: Option<&Path>, on: bool) -> Result<()> {
    let user = config_home().join("autostart");
    std::fs::create_dir_all(&user)?;
    let dest = user.join(id);
    let base = if dest.exists() {
        std::fs::read_to_string(&dest)?
    } else if let Some(p) = autostart_dirs().iter().map(|d| d.join(id)).find(|p| p.exists()).or_else(|| source.map(Path::to_path_buf)) {
        std::fs::read_to_string(p)?
    } else {
        return Err(anyhow!("I couldn't find that app's launcher."));
    };
    let mut lines: Vec<String> = base.lines().filter(|l| !l.trim_start().starts_with("Hidden=") && !l.trim_start().starts_with("X-GNOME-Autostart-enabled=")).map(str::to_owned).collect();
    let at = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(lines.len());
    lines.insert(at, format!("Hidden={}", if on { "false" } else { "true" }));
    lines.insert(at + 1, format!("X-GNOME-Autostart-enabled={on}"));
    std::fs::write(dest, lines.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lines_expand_like_the_spec() {
        let e = Entry { exec: "gedit --new-window %U".into(), ..Default::default() };
        assert_eq!(expand(&e, &[PathBuf::from("/tmp/a b.txt")]), vec!["gedit", "--new-window", "/tmp/a b.txt"]);
        let e = Entry { exec: "\"/opt/My App/run\" %f --x=%%y".into(), ..Default::default() };
        assert_eq!(expand(&e, &[]), vec!["/opt/My App/run", "--x=%y"]);
        let e = Entry { exec: "vlc".into(), ..Default::default() };
        assert_eq!(expand(&e, &[PathBuf::from("/m.mp3")]), vec!["vlc", "/m.mp3"]);
    }

    #[test]
    fn defaults_are_written_into_mimeapps_without_losing_other_lines() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("mimeapps.list");
        std::fs::write(&p, "[Added Associations]\ntext/plain=gedit.desktop;\n\n[Default Applications]\ntext/html=old.desktop;\nimage/png=eog.desktop;\n").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(ini(&text, "Default Applications").get("image/png").map(String::as_str), Some("eog.desktop;"));
        assert!(glob_match("*.PDF", "report.pdf") && !glob_match("*.pdf", "pdf") && glob_match("Makefile", "Makefile"));
    }
}
