use crate::storage::Db;
use crate::text::similarity;
use anyhow::{Result, anyhow};
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use crate::proc::Quiet;

pub const ROLES: &[&str] = &["music", "browser", "editor", "mail", "terminal", "files"];

#[derive(Debug, Clone)]
pub struct App {
    pub id: String,
    pub name: String,
    pub source: String,
    pub path: String,
    pub exec: String,
    pub aliases: Vec<String>,
    pub opened: i64,
    pub hidden: bool,
    pub roles: Vec<String>,
    pub manual: bool,
}

impl App {
    pub fn json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "source": self.source, "path": self.path,
            "aliases": self.aliases, "opened": self.opened, "hidden": self.hidden, "roles": self.roles
        })
    }
}

pub struct AppIndex {
    db: Db,
    apps: Mutex<Vec<App>>,
    pub scanning: std::sync::atomic::AtomicBool,
    lang: Mutex<String>,
}

fn app_dirs() -> Vec<(PathBuf, &'static str)> {
    let mut v = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home.join(".local/share"));
        v.push((data_home.join("applications"), "desktop"));
        v.push((data_home.join("flatpak/exports/share/applications"), "flatpak"));
        let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        for d in dirs.split(':').filter(|d| !d.is_empty()) {
            let p = PathBuf::from(d).join("applications");
            let src = if d.contains("flatpak") { "flatpak" } else if d.contains("snapd") { "snap" } else { "desktop" };
            v.push((p, src));
        }
        v.push((PathBuf::from("/var/lib/flatpak/exports/share/applications"), "flatpak"));
        v.push((PathBuf::from("/var/lib/snapd/desktop/applications"), "snap"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        v.push((PathBuf::from("/Applications"), "applications"));
        v.push((PathBuf::from("/System/Applications"), "applications"));
        v.push((home.join("Applications"), "applications"));
    }
    #[cfg(windows)]
    {
        if let Some(a) = std::env::var_os("APPDATA") {
            v.push((PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs"), "start_menu"));
        }
        if let Some(a) = std::env::var_os("ProgramData") {
            v.push((PathBuf::from(a).join("Microsoft\\Windows\\Start Menu\\Programs"), "start_menu"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|(p, _)| seen.insert(p.clone()));
    v
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn watch_dirs() -> Vec<PathBuf> {
    app_dirs().into_iter().map(|(p, _)| p).filter(|p| p.is_dir()).collect()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_list(key: &str) -> Vec<String> {
    key.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

#[cfg(target_os = "linux")]
fn parse_desktop(path: &Path, id: &str, source: &str, lang: &str) -> Option<App> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_entry = false;
    let mut kv: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_entry = l == "[Desktop Entry]";
            continue;
        }
        if !in_entry || l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            kv.entry(k.trim().to_owned()).or_insert_with(|| v.trim().to_owned());
        }
    }
    if kv.get("Type").map(String::as_str) != Some("Application") {
        return None;
    }
    if kv.get("NoDisplay").is_some_and(|v| v == "true") || kv.get("Hidden").is_some_and(|v| v == "true") {
        return None;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let mine: Vec<&str> = desktop.split(':').filter(|s| !s.is_empty()).collect();
    if let Some(only) = kv.get("OnlyShowIn")
        && !desktop_list(only).iter().any(|d| mine.contains(&d.as_str())) {
            return None;
        }
    if let Some(not) = kv.get("NotShowIn")
        && desktop_list(not).iter().any(|d| mine.contains(&d.as_str())) {
            return None;
        }
    if let Some(t) = kv.get("TryExec") {
        let p = Path::new(t);
        if p.is_absolute() && !p.exists() {
            return None;
        }
        if !p.is_absolute() && which(t).is_none() {
            return None;
        }
    }
    let name = kv.get(&format!("Name[{lang}]")).or_else(|| kv.get("Name"))?.clone();
    let exec = kv.get("Exec").cloned().unwrap_or_default();
    if exec.is_empty() && kv.get("DBusActivatable").is_none_or(|v| v != "true") {
        return None;
    }
    let cats = kv.get("Categories").map(|c| desktop_list(c)).unwrap_or_default();
    let mut roles = Vec::new();
    let has = |c: &str| cats.iter().any(|x| x == c);
    if has("WebBrowser") {
        roles.push("browser".to_owned());
    }
    if has("TextEditor") {
        roles.push("editor".into());
    }
    if has("Email") {
        roles.push("mail".into());
    }
    if has("TerminalEmulator") {
        roles.push("terminal".into());
    }
    if has("FileManager") {
        roles.push("files".into());
    }
    if has("Audio") && (has("Player") || has("Music")) {
        roles.push("music".into());
    }
    let aliases = kv.get("GenericName").map(|g| vec![g.clone()]).unwrap_or_default();
    Some(App {
        id: id.to_owned(),
        name,
        source: source.to_owned(),
        path: path.to_string_lossy().into_owned(),
        exec,
        aliases,
        opened: 0,
        hidden: false,
        roles,
        manual: false,
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn guess_roles(name: &str) -> Vec<String> {
    let n = crate::text::normalize(name);
    let any = |list: &[&str]| list.iter().any(|w| n == *w);
    let mut r = Vec::new();
    if crate::tools::browsers::key_of_app_name(name).is_some() {
        r.push("browser".to_owned());
    }
    if any(&["visual studio code", "vs code", "vscodium", "notepad", "notepad plus plus", "sublime text", "textedit", "bbedit", "zed", "cursor"]) {
        r.push("editor".to_owned());
    }
    if any(&["outlook", "microsoft outlook", "outlook new", "new outlook", "thunderbird", "mozilla thunderbird", "mail", "spark"]) {
        r.push("mail".to_owned());
    }
    if any(&["terminal", "windows terminal", "windows powershell", "powershell", "command prompt", "iterm", "iterm2", "warp", "alacritty", "kitty", "wezterm", "ghostty"]) {
        r.push("terminal".to_owned());
    }
    if any(&["file explorer", "finder"]) {
        r.push("files".to_owned());
    }
    if any(&["spotify", "music", "apple music", "itunes", "media player", "windows media player", "vlc media player", "vlc"]) {
        r.push("music".to_owned());
    }
    r
}

#[cfg_attr(windows, allow(dead_code))]
pub fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| p.is_file())
}

fn scan_dir(dir: &Path, source: &str, lang: &str, out: &mut HashMap<String, App>) {
    #[cfg(target_os = "linux")]
    {
        fn walk(base: &Path, dir: &Path, source: &str, lang: &str, out: &mut HashMap<String, App>, depth: u32) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && depth < 3 {
                    walk(base, &p, source, lang, out, depth + 1);
                } else if p.extension().is_some_and(|x| x == "desktop") {
                    let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('/', "-");
                    if out.contains_key(&rel) {
                        continue;
                    }
                    if let Some(a) = parse_desktop(&p, &rel, source, lang) {
                        out.insert(rel, a);
                    }
                }
            }
        }
        walk(dir, dir, source, lang, out, 0);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = lang;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "app") {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let id = p.to_string_lossy().into_owned();
                    let roles = guess_roles(&name);
                    out.entry(id.clone()).or_insert(App {
                        id: id.clone(),
                        name,
                        source: source.into(),
                        path: id,
                        exec: String::new(),
                        aliases: vec![],
                        opened: 0,
                        hidden: false,
                        roles,
                        manual: false,
                    });
                }
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = lang;
        fn walk(dir: &Path, source: &str, out: &mut HashMap<String, App>, depth: u32) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && depth < 3 {
                    walk(&p, source, out, depth + 1);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let lower = name.to_lowercase();
                    if lower.contains("uninstall") || lower.contains("readme") || lower.contains("help") {
                        continue;
                    }
                    let id = p.to_string_lossy().into_owned();
                    let roles = guess_roles(&name);
                    out.entry(id.clone()).or_insert(App {
                        id: id.clone(),
                        name,
                        source: source.into(),
                        path: id,
                        exec: String::new(),
                        aliases: vec![],
                        opened: 0,
                        hidden: false,
                        roles,
                        manual: false,
                    });
                }
            }
        }
        walk(dir, source, out, 0);
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<App> {
    Ok(App {
        id: r.get(0)?,
        name: r.get(1)?,
        source: r.get(2)?,
        path: r.get(3)?,
        exec: r.get(4)?,
        aliases: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        opened: r.get(6)?,
        hidden: r.get::<_, i64>(7)? != 0,
        roles: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
        manual: r.get::<_, i64>(9)? != 0,
    })
}

impl AppIndex {
    pub fn new(db: Db) -> Arc<Self> {
        let apps = db
            .read(|c| {
                let mut st = c.prepare("SELECT id, name, source, path, exec, aliases, opened, hidden, roles, manual FROM apps WHERE present = 1 ORDER BY name COLLATE NOCASE")?;
                let v = st.query_map([], row)?.flatten().collect();
                Ok(v)
            })
            .unwrap_or_default();
        Arc::new(Self { db, apps: Mutex::new(apps), scanning: Default::default(), lang: Mutex::new("en".into()) })
    }

    pub fn set_language(&self, lang: &str) {
        *self.lang.lock().unwrap() = lang.to_owned();
    }

    pub fn all(&self) -> Vec<App> {
        self.apps.lock().unwrap().clone()
    }

    pub fn count(&self) -> usize {
        self.apps.lock().unwrap().iter().filter(|a| !a.hidden).count()
    }

    pub fn get(&self, id: &str) -> Option<App> {
        self.apps.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    pub fn scan(&self) -> Result<(usize, i64)> {
        use std::sync::atomic::Ordering;
        self.scanning.store(true, Ordering::Relaxed);
        let lang = self.lang.lock().unwrap().clone();
        let mut found: HashMap<String, App> = HashMap::new();
        for (d, src) in app_dirs() {
            scan_dir(&d, src, &lang, &mut found);
        }
        let before = self.count() as i64;
        let list: Vec<App> = found.into_values().collect();
        let apps = self.db.write_sync(move |c| {
            let t = c.transaction()?;
            t.execute("UPDATE apps SET present = 0 WHERE manual = 0", [])?;
            for a in &list {
                t.execute(
                    "INSERT INTO apps(id, name, source, path, exec, aliases, roles, present) VALUES(?1,?2,?3,?4,?5,?6,?7,1)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name, source=excluded.source, path=excluded.path, exec=excluded.exec, roles=excluded.roles, present=1,
                     aliases=CASE WHEN apps.aliases = '[]' THEN excluded.aliases ELSE apps.aliases END",
                    params![a.id, a.name, a.source, a.path, a.exec, serde_json::to_string(&a.aliases)?, serde_json::to_string(&a.roles)?],
                )?;
            }
            t.commit()?;
            let mut st = c.prepare("SELECT id, name, source, path, exec, aliases, opened, hidden, roles, manual FROM apps WHERE present = 1 ORDER BY name COLLATE NOCASE")?;
            let v: Vec<App> = st.query_map([], row)?.flatten().collect();
            Ok(v)
        });
        self.scanning.store(false, Ordering::Relaxed);
        let apps = apps?;
        *self.apps.lock().unwrap() = apps;
        let after = self.count() as i64;
        Ok((after as usize, after - before))
    }

    pub fn find(&self, query: &str, role: Option<&str>, defaults: &HashMap<String, String>) -> Vec<(App, f64)> {
        let apps = self.apps.lock().unwrap();
        if let Some(r) = role {
            if let Some(id) = defaults.get(r)
                && let Some(a) = apps.iter().find(|a| &a.id == id && !a.hidden) {
                    return vec![(a.clone(), 1.0)];
                }
            let mut v: Vec<(App, f64)> = apps.iter().filter(|a| !a.hidden && a.roles.iter().any(|x| x == r)).map(|a| (a.clone(), 0.95)).collect();
            v.sort_by_key(|a| std::cmp::Reverse(a.0.opened));
            if !v.is_empty() {
                return v;
            }
        }
        let mut v: Vec<(App, f64)> = apps
            .iter()
            .filter(|a| !a.hidden)
            .map(|a| {
                let mut s = similarity(query, &a.name);
                for al in &a.aliases {
                    s = s.max(similarity(query, al) * 0.98);
                }
                let stem = a.id.trim_end_matches(".desktop").rsplit('.').next().unwrap_or("").to_owned();
                if stem.len() >= 3 {
                    s = s.max(similarity(query, &stem) * 0.9);
                }
                (a.clone(), s + (a.opened.min(50) as f64) * 0.0005)
            })
            .filter(|(_, s)| *s >= 0.78)
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        v.truncate(4);
        v
    }

    pub fn role_default(&self, role: &str, defaults: &HashMap<String, String>) -> Option<App> {
        if let Some(id) = defaults.get(role)
            && let Some(a) = self.get(id) {
                return Some(a);
            }
        let apps = self.apps.lock().unwrap();
        #[cfg(target_os = "linux")]
        {
            let mime = match role {
                "browser" => "x-scheme-handler/https",
                "mail" => "x-scheme-handler/mailto",
                "editor" => "text/plain",
                "files" => "inode/directory",
                "music" => "audio/mpeg",
                _ => "",
            };
            if !mime.is_empty()
                && let Ok(o) = Command::new("xdg-mime").args(["query", "default", mime]).stderr(Stdio::null()).output() {
                    let id = String::from_utf8_lossy(&o.stdout).trim().to_owned();
                    if let Some(a) = apps.iter().find(|a| a.id == id && !a.hidden) {
                        return Some(a.clone());
                    }
                }
        }
        #[cfg(not(target_os = "linux"))]
        if role == "browser"
            && let Some(k) = crate::tools::browsers::system_default()
            && let Some(a) = apps.iter().find(|a| !a.hidden && crate::tools::browsers::key_of(a) == Some(k))
        {
            return Some(a.clone());
        }
        let mut c: Vec<&App> = apps.iter().filter(|a| !a.hidden && a.roles.iter().any(|r| r == role)).collect();
        c.sort_by(|a, b| b.opened.cmp(&a.opened).then(a.name.cmp(&b.name)));
        c.first().map(|a| (*a).clone())
    }

    pub fn candidates(&self, role: &str) -> Vec<String> {
        self.apps.lock().unwrap().iter().filter(|a| !a.hidden && a.roles.iter().any(|r| r == role)).map(|a| a.id.clone()).collect()
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut App)) -> Option<App> {
        let mut g = self.apps.lock().unwrap();
        let a = g.iter_mut().find(|a| a.id == id)?;
        f(a);
        Some(a.clone())
    }

    pub fn set_aliases(&self, id: &str, aliases: Vec<String>) -> Result<App> {
        let clean: Vec<String> = aliases
            .into_iter()
            .map(|a| a.trim().chars().filter(|c| !c.is_control()).take(40).collect::<String>())
            .filter(|a| !a.is_empty())
            .take(12)
            .collect();
        let app = self.update(id, |a| a.aliases = clean.clone()).ok_or_else(|| anyhow!("No such app"))?;
        let idc = id.to_owned();
        self.db.write_sync(move |c| {
            c.execute("UPDATE apps SET aliases = ?2 WHERE id = ?1", params![idc, serde_json::to_string(&clean)?])?;
            Ok(())
        })?;
        Ok(app)
    }

    pub fn set_hidden(&self, id: &str, hidden: bool) -> Result<App> {
        let app = self.update(id, |a| a.hidden = hidden).ok_or_else(|| anyhow!("No such app"))?;
        let idc = id.to_owned();
        self.db.write_sync(move |c| {
            c.execute("UPDATE apps SET hidden = ?2 WHERE id = ?1", params![idc, hidden as i64])?;
            Ok(())
        })?;
        Ok(app)
    }

    pub fn bump(&self, id: &str) {
        self.update(id, |a| a.opened += 1);
        let idc = id.to_owned();
        self.db.write(move |c| {
            c.execute("UPDATE apps SET opened = opened + 1 WHERE id = ?1", params![idc])?;
            Ok(())
        });
    }

    pub fn add_manual(&self, path: &str) -> Result<App> {
        let p = Path::new(path);
        if !p.is_absolute() || !p.exists() {
            return Err(anyhow!("That file does not exist."));
        }
        let lang = self.lang.lock().unwrap().clone();
        #[cfg(target_os = "linux")]
        let parsed = if p.extension().is_some_and(|x| x == "desktop") {
            parse_desktop(p, &format!("manual-{}", p.file_name().unwrap_or_default().to_string_lossy()), "manual", &lang)
        } else {
            None
        };
        #[cfg(not(target_os = "linux"))]
        let parsed: Option<App> = {
            let _ = &lang;
            None
        };
        let app = match parsed {
            Some(mut a) => {
                a.manual = true;
                a
            }
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let meta = std::fs::metadata(p)?;
                    if !(p.extension().is_some_and(|x| x == "app") || meta.permissions().mode() & 0o111 != 0) {
                        return Err(anyhow!("Choose a program file or an application entry."));
                    }
                }
                let name = p.file_stem().map(|s| crate::text::sentence_case(&s.to_string_lossy().replace(['-', '_'], " "))).unwrap_or_default();
                App {
                    id: format!("manual-{}", &crate::models::hex(&<sha2::Sha256 as sha2::Digest>::digest(path.as_bytes()))[..12]),
                    name,
                    source: "manual".into(),
                    path: path.into(),
                    exec: format!("\"{}\"", path.replace('"', "")),
                    aliases: vec![],
                    opened: 0,
                    hidden: false,
                    roles: vec![],
                    manual: true,
                }
            }
        };
        let a = app.clone();
        self.db.write_sync(move |c| {
            c.execute(
                "INSERT INTO apps(id, name, source, path, exec, aliases, roles, manual, present) VALUES(?1,?2,?3,?4,?5,?6,?7,1,1)
                 ON CONFLICT(id) DO UPDATE SET present = 1, manual = 1",
                params![a.id, a.name, a.source, a.path, a.exec, serde_json::to_string(&a.aliases)?, serde_json::to_string(&a.roles)?],
            )?;
            Ok(())
        })?;
        let mut g = self.apps.lock().unwrap();
        g.retain(|x| x.id != app.id);
        g.push(app.clone());
        g.sort_by_key(|a| a.name.to_lowercase());
        Ok(app)
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quote = !quote,
            '\\' if quote => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if !quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .filter(|a| !(a.len() == 2 && a.starts_with('%')))
        .map(|a| a.replace("%%", "%"))
        .filter(|a| !a.contains('%') || a.len() > 2)
        .collect()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn exec_with(exec: &str, args: &[String]) -> Vec<String> {
    let cleaned = exec.replace("@@u", "").replace("@@", "");
    let mut out = Vec::new();
    let mut placed = false;
    let mut cur = String::new();
    let mut quote = false;
    let mut raw = Vec::new();
    let mut chars = cleaned.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quote = !quote,
            '\\' if quote => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if !quote => {
                if !cur.is_empty() {
                    raw.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        raw.push(cur);
    }
    for a in raw {
        match a.as_str() {
            "%u" | "%U" | "%f" | "%F" => {
                if !placed {
                    out.extend(args.iter().cloned());
                    placed = true;
                }
            }
            s if s.len() == 2 && s.starts_with('%') => {}
            s => out.push(s.replace("%%", "%")),
        }
    }
    if !placed {
        out.extend(args.iter().cloned());
    }
    out
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const CHROMIUM_LIKE: &[&str] = &[
    "brave",
    "chrome",
    "chromium",
    "msedge",
    "microsoft-edge",
    "vivaldi",
    "opera",
    "code",
    "code-insiders",
    "codium",
    "cursor",
    "windsurf",
    "slack",
    "discord",
    "signal-desktop",
    "obsidian",
    "teams-for-linux",
    "spotify",
    "com.brave.browser",
    "com.google.chrome",
    "org.chromium.chromium",
    "com.microsoft.edge",
    "com.vivaldi.vivaldi",
    "com.visualstudio.code",
    "com.vscodium.codium",
    "com.slack.slack",
    "com.discordapp.discord",
    "org.signal.signal",
    "md.obsidian.obsidian",
    "com.spotify.client",
];

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn chromium_like(app: &App) -> bool {
    let low = format!("{} {}", app.exec, Path::new(&app.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()).to_lowercase();
    if low.contains("firefox") || low.contains("thunderbird") {
        return false;
    }
    low.split(|c: char| c.is_whitespace() || c == '/' || c == '=').filter(|t| !t.is_empty()).any(|t| {
        let t = t.trim_matches('"');
        t.contains("electron") || CHROMIUM_LIKE.iter().any(|k| t == *k || t.starts_with(&format!("{k}-")) || t.ends_with(&format!(".{k}")) || t.ends_with(&format!("-{k}")) || t.contains(&format!("-{k}-")))
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const A11Y_FLAG: &str = "--force-renderer-accessibility";

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn a11y_args(app: &App) -> Vec<String> {
    if cfg!(target_os = "linux") && chromium_like(app) && !app.exec.contains(A11Y_FLAG) { vec![A11Y_FLAG.to_owned()] } else { Vec::new() }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn with_a11y_flag(exec: &str) -> String {
    if exec.contains(A11Y_FLAG) {
        return exec.to_owned();
    }
    let parts: Vec<&str> = exec.split(' ').collect();
    match parts.iter().position(|p| p.starts_with('%') || *p == "@@u" || *p == "@@") {
        Some(i) => {
            let mut v: Vec<&str> = parts[..i].to_vec();
            v.push(A11Y_FLAG);
            v.extend_from_slice(&parts[i..]);
            v.join(" ")
        }
        None => format!("{exec} {A11Y_FLAG}"),
    }
}

#[cfg(target_os = "linux")]
fn user_apps_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home.join(".local/share")).join("applications")
}

#[cfg(target_os = "linux")]
fn ready_text(system: &str) -> String {
    let mut out = String::new();
    let mut marked = false;
    for line in system.lines() {
        if let Some(rest) = line.strip_prefix("Exec=") {
            out.push_str("Exec=");
            out.push_str(&with_a11y_flag(rest));
        } else {
            out.push_str(line);
        }
        out.push('\n');
        if line.trim() == "[Desktop Entry]" && !marked {
            out.push_str("X-Setu-Accessibility=true\n");
            marked = true;
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn make_ready(app: &App) -> Result<bool> {
    if !app.path.ends_with(".desktop") || !chromium_like(app) {
        return Ok(false);
    }
    let src = Path::new(&app.path);
    let dir = user_apps_dir();
    let name = src.file_name().ok_or_else(|| anyhow!("That app has no launcher."))?;
    let dest = dir.join(name);
    if src.parent() == Some(dir.as_path()) {
        let text = std::fs::read_to_string(src)?;
        if text.contains(A11Y_FLAG) {
            return Ok(true);
        }
        if !text.contains("X-Setu-Accessibility=true") {
            return Ok(false);
        }
    }
    if dest.exists() && !std::fs::read_to_string(&dest).unwrap_or_default().contains("X-Setu-Accessibility=true") {
        return Ok(false);
    }
    let text = std::fs::read_to_string(src)?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&dest, ready_text(&text))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
pub fn refresh_ready() {
    let dir = user_apps_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let home = dirs::home_dir().unwrap_or_default();
    let system: Vec<PathBuf> = vec![PathBuf::from("/usr/share/applications"), PathBuf::from("/usr/local/share/applications"), PathBuf::from("/var/lib/flatpak/exports/share/applications"), home.join(".local/share/flatpak/exports/share/applications")];
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "desktop") {
            continue;
        }
        let Ok(mine) = std::fs::read_to_string(&p) else { continue };
        if !mine.contains("X-Setu-Accessibility=true") {
            continue;
        }
        let Some(src) = system.iter().map(|d| d.join(e.file_name())).find(|s| s.exists()) else {
            let _ = std::fs::remove_file(&p);
            continue;
        };
        if let Ok(text) = std::fs::read_to_string(&src) {
            let fresh = ready_text(&text);
            if fresh != mine {
                let _ = std::fs::write(&p, fresh);
            }
        }
    }
}

pub fn launch(app: &App) -> Result<Option<u32>> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let flag = a11y_args(app);
        if flag.is_empty()
            && app.path.ends_with(".desktop")
            && !app.manual
            && let Some(e) = crate::xdg::entry(std::path::Path::new(&app.path)).filter(|e| !e.terminal && !e.exec.is_empty())
            && let Ok(pid) = crate::xdg::launch(&e, &[])
        {
            return Ok(Some(pid));
        }
        let helper = flag.is_empty() && app.path.ends_with(".desktop") && which("gio").is_some() && !app.manual;
        let span = if helper { crate::layers::effect(crate::layers::Layer::Command, "command.launch") } else { crate::layers::effect(crate::layers::Layer::System, "system.launch") };
        let mut cmd = if !flag.is_empty() {
            let argv = exec_with(&app.exec, &flag);
            let (prog, args) = argv.split_first().ok_or_else(|| anyhow!("The app has no command to start it."))?;
            let mut c = Command::new(prog);
            c.args(args);
            c
        } else if app.path.ends_with(".desktop") && which("gio").is_some() && !app.manual {
            let mut c = Command::new("gio");
            c.arg("launch").arg(&app.path);
            c
        } else {
            let argv = split_exec(&app.exec);
            let (prog, args) = argv.split_first().ok_or_else(|| anyhow!("The app has no command to start it."))?;
            let mut c = Command::new(prog);
            c.args(args);
            c
        };
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        for v in ["SETU_DATA_DIR", "SETU_RUNTIME_DIR", "SETU_ASSETS", "SETU_LOG", "LD_LIBRARY_PATH"] {
            cmd.env_remove(v);
        }
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let child = span.end(cmd.spawn()).map_err(|e| anyhow!("{} could not be started: {e}", app.name))?;
        let pid = child.id();
        std::thread::spawn(move || {
            let mut c = child;
            let _ = c.wait();
        });
        Ok(Some(pid))
    }
    #[cfg(target_os = "macos")]
    {
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.launch");
        span.end(Command::new("open").arg("-a").arg(&app.path).status()).map_err(|e| anyhow!("{} could not be started: {e}", app.name))?;
        Ok(None)
    }
    #[cfg(windows)]
    {
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.launch");
        span.end(Command::new("cmd").quiet().args(["/C", "start", ""]).arg(&app.path).stdout(Stdio::null()).status()).map_err(|e| anyhow!("{} could not be started: {e}", app.name))?;
        Ok(None)
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const LAUNCHERS: &[&str] = &[
    "python", "python2", "python3", "sh", "bash", "dash", "zsh", "fish", "java", "node", "nodejs", "perl", "ruby", "wine", "wine64", "mono", "dotnet", "gjs", "php", "lua", "deno", "bun", "pwsh", "electron", "steam",
    "gtk-launch", "gio", "xdg-open", "setsid", "nohup", "systemd-run", "pkexec", "sudo",
];

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn launcher(base: &str) -> bool {
    LAUNCHERS.contains(&base) || base.starts_with("python3.") || base.starts_with("python2.") || base.strip_prefix("electron").is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn exec_names(app: &App) -> Vec<String> {
    let argv = split_exec(&app.exec);
    let mut names = Vec::new();
    let base_of = |p: &str| Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut at = 0;
    if argv.first().is_some_and(|f| base_of(f) == "env") {
        at = match argv.iter().skip(1).position(|a| !a.contains('=') && !a.starts_with('-')) {
            Some(i) => i + 1,
            None => return names,
        };
    }
    let Some(first) = argv.get(at) else { return names };
    let base = base_of(first);
    if base == "flatpak" {
        if let Some(id) = argv.iter().skip(at + 1).find(|a| !a.starts_with('-') && a.contains('.') && *a != "run") {
            names.push(format!("flatpak:{id}"));
        }
    } else if launcher(&base) {
        if let Some(arg) = argv.iter().skip(at + 1).find(|a| !a.starts_with('-') && !a.starts_with('%')) {
            names.push(format!("script:{base}\u{1f}{arg}"));
        }
    } else if !base.is_empty() {
        names.push(base);
    }
    names
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn running(app: &App) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let names = exec_names(app);
        let me = crate::paths::uid();
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir("/proc") else { return out };
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
            let Ok(meta) = std::fs::metadata(e.path()) else { continue };
            if std::os::unix::fs::MetadataExt::uid(&meta) != me || pid == std::process::id() {
                continue;
            }
            let Ok(cmd) = std::fs::read(e.path().join("cmdline")) else { continue };
            let args: Vec<String> = cmd.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
            let Some(first) = args.first() else { continue };
            let base = Path::new(first).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            for n in &names {
                if let Some(fid) = n.strip_prefix("flatpak:") {
                    if base == "bwrap" && args.iter().any(|a| a.contains(fid)) {
                        out.push(pid);
                    }
                } else if let Some(rest) = n.strip_prefix("script:") {
                    if let Some((interp, arg)) = rest.split_once('\u{1f}')
                        && (base == interp || base.strip_prefix(interp).is_some_and(|v| v.starts_with('.')))
                        && args.iter().skip(1).any(|a| a == arg)
                    {
                        out.push(pid);
                    }
                } else if base == *n {
                    out.push(pid);
                }
            }
        }
        out
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Vec::new()
    }
}

pub fn close(app: &App) -> Result<usize> {
    #[cfg(target_os = "linux")]
    {
        if let Some(fid) = exec_names(app).iter().find_map(|n| n.strip_prefix("flatpak:").map(str::to_owned)) {
            let span = crate::layers::effect(crate::layers::Layer::Command, "command.close");
            let ok = span.end_bool(Command::new("flatpak").args(["kill", &fid]).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false));
            return if ok { Ok(1) } else { Err(anyhow!("{} is not running.", app.name)) };
        }
        let pids = running(app);
        if pids.is_empty() {
            return Err(anyhow!("{} is not running.", app.name));
        }
        let span = crate::layers::effect(crate::layers::Layer::System, "system.close");
        for p in &pids {
            unsafe { libc::kill(*p as i32, libc::SIGTERM) };
        }
        span.ok();
        Ok(pids.len())
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"{}\" to quit", app.name.replace('"', ""));
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.osascript");
        let ok = span.end_bool(Command::new("osascript").args(["-e", &script]).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(1) } else { Err(anyhow!("{} is not running.", app.name)) }
    }
    #[cfg(windows)]
    {
        let exe = Path::new(&app.path).file_stem().map(|s| format!("{}.exe", s.to_string_lossy())).unwrap_or_default();
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.close");
        let ok = span.end_bool(Command::new("taskkill").quiet().args(["/IM", &exe]).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(1) } else { Err(anyhow!("{} is not running.", app.name)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn readies_launchers() {
        let system = "[Desktop Entry]\nName=Brave Web Browser\nExec=/usr/bin/brave-browser-stable %U\nActions=new-window;\n\n[Desktop Action new-window]\nName=New Window\nExec=/usr/bin/brave-browser-stable\n";
        let out = ready_text(system);
        assert_eq!(out.matches("--force-renderer-accessibility").count(), 2);
        assert!(out.contains("Exec=/usr/bin/brave-browser-stable --force-renderer-accessibility %U"));
        assert_eq!(out.matches("X-Setu-Accessibility=true").count(), 1);
        assert_eq!(ready_text(&out).matches("--force-renderer-accessibility").count(), 2);
    }

    #[test]
    fn flags_chromium_and_electron_apps() {
        let mk = |exec: &str, path: &str| App { id: path.into(), name: "x".into(), source: "desktop".into(), path: path.into(), exec: exec.into(), aliases: vec![], opened: 0, hidden: false, roles: vec![], manual: false };
        assert!(chromium_like(&mk("/usr/bin/brave-browser-stable %U", "/usr/share/applications/brave-browser.desktop")));
        assert!(chromium_like(&mk("/usr/share/code/code %F", "/usr/share/applications/code.desktop")));
        assert!(chromium_like(&mk("/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=brave com.brave.Browser @@u %U @@", "/var/lib/flatpak/exports/share/applications/com.brave.Browser.desktop")));
        assert!(chromium_like(&mk("/usr/bin/google-chrome-stable %U", "/usr/share/applications/google-chrome.desktop")));
        assert!(!chromium_like(&mk("/usr/lib64/firefox/firefox %u", "/usr/share/applications/org.mozilla.firefox.desktop")));
        assert!(!chromium_like(&mk("gnome-text-editor %U", "/usr/share/applications/org.gnome.TextEditor.desktop")));
        assert!(!chromium_like(&mk("barcode-scanner %U", "/usr/share/applications/barcode.desktop")));
        assert_eq!(with_a11y_flag("/usr/bin/brave-browser-stable %U"), "/usr/bin/brave-browser-stable --force-renderer-accessibility %U");
        assert_eq!(with_a11y_flag("/usr/bin/brave-browser-stable --incognito"), "/usr/bin/brave-browser-stable --incognito --force-renderer-accessibility");
        assert_eq!(with_a11y_flag("flatpak run com.brave.Browser @@u %U @@"), "flatpak run com.brave.Browser --force-renderer-accessibility @@u %U @@");
        assert_eq!(with_a11y_flag("x --force-renderer-accessibility %U"), "x --force-renderer-accessibility %U");
    }

    #[test]
    fn splits_exec_lines() {
        assert_eq!(split_exec("firefox %u"), vec!["firefox"]);
        assert_eq!(split_exec("\"/opt/My App/run\" --flag %F"), vec!["/opt/My App/run", "--flag"]);
        assert_eq!(split_exec("env FOO=1 code --new-window %F"), vec!["env", "FOO=1", "code", "--new-window"]);
        assert_eq!(split_exec("printf 100%%"), vec!["printf", "100%"]);
    }

    #[test]
    fn exec_names_handle_wrappers() {
        let mk = |exec: &str| App {
            id: "x".into(),
            name: "X".into(),
            source: "desktop".into(),
            path: String::new(),
            exec: exec.into(),
            aliases: vec![],
            opened: 0,
            hidden: false,
            roles: vec![],
            manual: false,
        };
        assert_eq!(exec_names(&mk("/usr/bin/firefox %u")), vec!["firefox"]);
        assert_eq!(exec_names(&mk("/usr/bin/flatpak run --branch=stable --arch=x86_64 com.spotify.Client")), vec!["flatpak:com.spotify.Client"]);
        assert_eq!(exec_names(&mk("env BAMF=1 /usr/share/code/code --unity-launch %F")), vec!["code"]);
        assert_eq!(exec_names(&mk("/usr/bin/python3 /opt/notes/main.py %F")), vec!["script:python3\u{1f}/opt/notes/main.py"]);
        assert_eq!(exec_names(&mk("env LANG=C java -jar /opt/tool/tool.jar")), vec!["script:java\u{1f}/opt/tool/tool.jar"]);
        assert!(exec_names(&mk("sh -c \"exec something\"")).iter().all(|n| n.starts_with("script:sh")));
        assert!(exec_names(&mk("python3")).is_empty());
        assert!(launcher("python3.14") && launcher("electron34") && !launcher("electronic-mail") && !launcher("firefox"));
    }
}
