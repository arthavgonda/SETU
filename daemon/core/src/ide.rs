use crate::core::Core;
use crate::os::{self, Do, Node, Scope, Win};

pub trait Prefs: Sync {
    fn pref(&self, key: &str) -> String;
    fn set_pref(&self, key: &str, value: &str);
}

impl Prefs for Core {
    fn pref(&self, key: &str) -> String {
        self.settings.str(key)
    }

    fn set_pref(&self, key: &str, value: &str) {
        self.settings.set_internal(key, serde_json::json!(value));
        let mut m = serde_json::Map::new();
        m.insert(key.to_owned(), serde_json::json!(value));
        self.db.save_settings(m);
    }
}

impl Prefs for std::sync::Arc<Core> {
    fn pref(&self, key: &str) -> String {
        (**self).pref(key)
    }

    fn set_pref(&self, key: &str, value: &str) {
        (**self).set_pref(key, value)
    }
}

pub struct NoPrefs;

impl Prefs for NoPrefs {
    fn pref(&self, _key: &str) -> String {
        String::new()
    }

    fn set_pref(&self, _key: &str, _value: &str) {}
}
use crate::text::normalize;
use crate::tools::input::{self, K};
use crate::tools::shell::{find_program, output, run_detached};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    VsCode,
    VisualStudio,
    JetBrains,
    Xcode,
    Eclipse,
    QtCreator,
    Sublime,
    NotepadPlus,
    Zed,
}

#[derive(Debug)]
pub struct Ide {
    pub key: &'static str,
    pub name: &'static str,
    pub family: Family,
    pub titles: &'static [&'static str],
    pub apps: &'static [&'static str],
    pub cli: &'static [&'static str],
    pub words: &'static [&'static str],
    pub bundle: &'static [&'static str],
    pub config: &'static str,
}

pub static IDES: &[Ide] = &[
    Ide { key: "vscode", name: "VS Code", family: Family::VsCode, titles: &["Visual Studio Code"], apps: &["code", "visual studio code"], cli: &["code", "code.cmd"], words: &["vs code", "vscode", "visual studio code", "v s code", "code editor"], bundle: &["Visual Studio Code"], config: "Code" },
    Ide { key: "insiders", name: "VS Code Insiders", family: Family::VsCode, titles: &["Visual Studio Code - Insiders"], apps: &["code insiders", "visual studio code insiders"], cli: &["code-insiders", "code-insiders.cmd"], words: &["vs code insiders", "code insiders"], bundle: &["Visual Studio Code - Insiders"], config: "Code - Insiders" },
    Ide { key: "vscodium", name: "VSCodium", family: Family::VsCode, titles: &["VSCodium"], apps: &["codium", "vscodium"], cli: &["codium", "codium.cmd"], words: &["vscodium", "vs codium", "codium"], bundle: &["VSCodium"], config: "VSCodium" },
    Ide { key: "codeoss", name: "Code - OSS", family: Family::VsCode, titles: &["Code - OSS"], apps: &["code oss"], cli: &["code-oss"], words: &["code oss"], bundle: &[], config: "Code - OSS" },
    Ide { key: "cursor", name: "Cursor", family: Family::VsCode, titles: &["Cursor"], apps: &["cursor"], cli: &["cursor", "cursor.cmd"], words: &["cursor", "cursor editor", "cursor ai"], bundle: &["Cursor"], config: "Cursor" },
    Ide { key: "windsurf", name: "Windsurf", family: Family::VsCode, titles: &["Windsurf"], apps: &["windsurf"], cli: &["windsurf", "windsurf.cmd"], words: &["windsurf", "wind surf"], bundle: &["Windsurf"], config: "Windsurf" },
    Ide { key: "visualstudio", name: "Visual Studio", family: Family::VisualStudio, titles: &["Microsoft Visual Studio"], apps: &["devenv", "microsoft visual studio"], cli: &["devenv.exe"], words: &["visual studio", "visual studio community", "microsoft visual studio"], bundle: &[], config: "" },
    Ide {
        key: "intellij",
        name: "IntelliJ IDEA",
        family: Family::JetBrains,
        titles: &[],
        apps: &["idea", "idea64", "intellij idea", "intellij idea community edition", "intellij idea ultimate edition"],
        cli: &["idea", "idea64.exe", "idea.sh", "intellij-idea-ultimate", "intellij-idea-community", "intellij-idea-ultimate-edition", "intellij-idea-community-edition"],
        words: &["intellij", "intellij idea", "intelli j"],
        bundle: &["IntelliJ IDEA", "IntelliJ IDEA CE", "IntelliJ IDEA Ultimate"],
        config: "IntelliJIdea|IdeaIC",
    },
    Ide { key: "pycharm", name: "PyCharm", family: Family::JetBrains, titles: &[], apps: &["pycharm", "pycharm64", "pycharm community edition", "pycharm professional edition"], cli: &["pycharm", "pycharm64.exe", "pycharm.sh", "pycharm-community", "pycharm-professional"], words: &["pycharm", "py charm", "pie charm"], bundle: &["PyCharm", "PyCharm CE"], config: "PyCharm|PyCharmCE" },
    Ide { key: "clion", name: "CLion", family: Family::JetBrains, titles: &[], apps: &["clion", "clion64"], cli: &["clion", "clion64.exe", "clion.sh"], words: &["clion", "c lion", "see lion"], bundle: &["CLion"], config: "CLion" },
    Ide { key: "webstorm", name: "WebStorm", family: Family::JetBrains, titles: &[], apps: &["webstorm", "webstorm64"], cli: &["webstorm", "webstorm64.exe", "webstorm.sh"], words: &["webstorm", "web storm"], bundle: &["WebStorm"], config: "WebStorm" },
    Ide { key: "goland", name: "GoLand", family: Family::JetBrains, titles: &[], apps: &["goland", "goland64"], cli: &["goland", "goland64.exe", "goland.sh"], words: &["goland", "go land"], bundle: &["GoLand"], config: "GoLand" },
    Ide { key: "rider", name: "Rider", family: Family::JetBrains, titles: &[], apps: &["rider", "rider64"], cli: &["rider", "rider64.exe", "rider.sh"], words: &["rider", "jetbrains rider"], bundle: &["Rider"], config: "Rider" },
    Ide { key: "phpstorm", name: "PhpStorm", family: Family::JetBrains, titles: &[], apps: &["phpstorm", "phpstorm64"], cli: &["phpstorm", "phpstorm64.exe", "phpstorm.sh"], words: &["phpstorm", "php storm"], bundle: &["PhpStorm"], config: "PhpStorm" },
    Ide { key: "rubymine", name: "RubyMine", family: Family::JetBrains, titles: &[], apps: &["rubymine", "rubymine64"], cli: &["rubymine", "rubymine64.exe", "rubymine.sh"], words: &["rubymine", "ruby mine"], bundle: &["RubyMine"], config: "RubyMine" },
    Ide { key: "rustrover", name: "RustRover", family: Family::JetBrains, titles: &[], apps: &["rustrover", "rustrover64"], cli: &["rustrover", "rustrover64.exe", "rustrover.sh"], words: &["rustrover", "rust rover"], bundle: &["RustRover"], config: "RustRover" },
    Ide { key: "androidstudio", name: "Android Studio", family: Family::JetBrains, titles: &[], apps: &["studio", "studio64", "android studio"], cli: &["studio", "studio64.exe", "studio.sh", "android-studio"], words: &["android studio"], bundle: &["Android Studio"], config: "AndroidStudio" },
    Ide { key: "xcode", name: "Xcode", family: Family::Xcode, titles: &[], apps: &["xcode"], cli: &["xed"], words: &["xcode", "x code"], bundle: &["Xcode"], config: "" },
    Ide { key: "eclipse", name: "Eclipse", family: Family::Eclipse, titles: &["Eclipse IDE", "Eclipse Platform", "Eclipse"], apps: &["eclipse"], cli: &["eclipse", "eclipse.exe"], words: &["eclipse"], bundle: &["Eclipse"], config: "" },
    Ide { key: "qtcreator", name: "Qt Creator", family: Family::QtCreator, titles: &["Qt Creator"], apps: &["qtcreator", "qt creator"], cli: &["qtcreator", "qtcreator.exe", "Qt Creator"], words: &["qt creator", "cute creator", "q t creator"], bundle: &["Qt Creator"], config: "" },
    Ide { key: "sublime", name: "Sublime Text", family: Family::Sublime, titles: &["Sublime Text"], apps: &["sublime text", "sublime text 4", "sublime text 3", "subl"], cli: &["subl", "subl.exe", "sublime_text"], words: &["sublime", "sublime text"], bundle: &["Sublime Text"], config: "" },
    Ide { key: "notepadpp", name: "Notepad++", family: Family::NotepadPlus, titles: &["Notepad++"], apps: &[], cli: &["notepad++.exe", "notepad++"], words: &["notepad plus plus", "notepad++", "notepad plus"], bundle: &[], config: "" },
    Ide { key: "zed", name: "Zed", family: Family::Zed, titles: &[], apps: &["zed", "zed editor", "dev zed zed"], cli: &["zed", "zeditor"], words: &["zed", "zed editor"], bundle: &["Zed"], config: "" },
];

pub fn by_key(key: &str) -> Option<&'static Ide> {
    IDES.iter().find(|i| i.key == key)
}

pub fn spoken(t: &str) -> Option<&'static Ide> {
    let hay = format!(" {} ", normalize(t));
    let mut best: Option<(usize, &'static Ide)> = None;
    for ide in IDES {
        for w in ide.words {
            let w = normalize(w);
            if !w.is_empty() && hay.contains(&format!(" {w} ")) && best.is_none_or(|(n, _)| w.len() > n) {
                best = Some((w.len(), ide));
            }
        }
    }
    best.map(|(_, i)| i)
}

fn title_has(title: &str, token: &str) -> bool {
    if title == token {
        return true;
    }
    for sep in [" - ", " — ", " – "] {
        let needle = format!("{sep}{token}");
        let mut from = 0;
        while let Some(i) = title[from..].find(&needle) {
            let end = from + i + needle.len();
            let rest = &title[end..];
            if rest.is_empty() || rest.starts_with(" - ") || rest.starts_with(" (") || rest.starts_with(" [") {
                return true;
            }
            from = end;
        }
    }
    false
}

pub fn of_window(w: &Win) -> Option<&'static Ide> {
    if os::is_setu(&w.app) {
        return None;
    }
    let mut best: Option<(usize, &'static Ide)> = None;
    for ide in IDES {
        for t in ide.titles {
            if title_has(&w.title, t) && best.is_none_or(|(n, _)| t.len() > n) {
                best = Some((t.len(), ide));
            }
        }
    }
    if let Some((_, i)) = best {
        return Some(i);
    }
    let app = normalize(&w.app);
    if app.is_empty() {
        return None;
    }
    IDES.iter().find(|i| i.apps.iter().any(|a| normalize(a) == app))
}

pub fn pick(wins: &[Win], want: Option<&Ide>, last: Option<&Win>, remembered: &str) -> Option<(&'static Ide, Win)> {
    let mut cands: Vec<(&'static Ide, Win)> = wins.iter().filter_map(|w| of_window(w).map(|i| (i, w.clone()))).collect();
    if let Some(wi) = want {
        cands.retain(|(i, _)| i.key == wi.key);
    }
    cands.sort_by_key(|(i, w)| (!w.front, !last.is_some_and(|l| l.id == w.id || (l.pid == w.pid && l.pid != 0)), i.key != remembered));
    cands.into_iter().next()
}

pub async fn find(core: &dyn Prefs, want: Option<&'static Ide>) -> Option<(&'static Ide, Win)> {
    let wins = os::windows().await.ok()?;
    pick(&wins, want, os::last_other().as_ref(), &core.pref("code.ide"))
}

pub async fn wait_window(core: &dyn Prefs, want: Option<&'static Ide>, title_part: &str, secs: u64) -> Option<(&'static Ide, Win)> {
    let until = Instant::now() + Duration::from_secs(secs);
    let part = title_part.to_lowercase();
    loop {
        if let Ok(wins) = os::windows().await {
            let fit: Vec<Win> = wins.into_iter().filter(|w| part.is_empty() || w.title.to_lowercase().contains(&part)).collect();
            if let Some(f) = pick(&fit, want, os::last_other().as_ref(), &core.pref("code.ide")) {
                return Some(f);
            }
        }
        if Instant::now() >= until {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

pub fn remember(core: &dyn Prefs, ide: &Ide) {
    if core.pref("code.ide") != ide.key {
        core.set_pref("code.ide", ide.key);
    }
}

fn local_appdata() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default()
}

fn subdirs(base: &Path, tail: &[&str]) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(base) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| {
            let mut p = e.path();
            for t in tail {
                p.push(t);
            }
            p
        })
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v.reverse();
    v
}

fn vs_dirs(ide: &Ide) -> &'static [&'static str] {
    match ide.key {
        "vscode" => &["Microsoft VS Code"],
        "insiders" => &["Microsoft VS Code Insiders"],
        "vscodium" => &["VSCodium"],
        "cursor" => &["cursor", "Cursor"],
        "windsurf" => &["Windsurf"],
        _ => &[],
    }
}

fn extra_dirs(ide: &Ide) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = Vec::new();
    if ide.family == Family::JetBrains {
        v.push(if cfg!(windows) {
            local_appdata().join("JetBrains").join("Toolbox").join("scripts")
        } else if cfg!(target_os = "macos") {
            home.join("Library/Application Support/JetBrains/Toolbox/scripts")
        } else {
            home.join(".local/share/JetBrains/Toolbox/scripts")
        });
    }
    if cfg!(target_os = "linux") {
        v.push(home.join(".local/bin"));
        v.push(PathBuf::from("/snap/bin"));
        v.extend(subdirs(Path::new("/opt"), &["bin"]));
        v.extend(subdirs(&home.join(".local/share/JetBrains"), &["bin"]));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            let Some(base) = std::env::var_os(var).map(PathBuf::from) else { continue };
            match ide.family {
                Family::JetBrains => v.extend(subdirs(&base.join("JetBrains"), &["bin"])),
                Family::NotepadPlus => v.push(base.join("Notepad++")),
                Family::Sublime => {
                    v.push(base.join("Sublime Text"));
                    v.push(base.join("Sublime Text 3"));
                }
                Family::Zed => v.push(base.join("Zed")),
                Family::VsCode => {
                    for d in vs_dirs(ide) {
                        v.push(base.join(d).join("bin"));
                    }
                }
                _ => {}
            }
        }
        let local = local_appdata().join("Programs");
        match ide.family {
            Family::VsCode => {
                for d in vs_dirs(ide) {
                    v.push(local.join(d).join("bin"));
                    v.push(local.join(d).join("resources").join("app").join("bin"));
                }
            }
            Family::JetBrains => v.extend(subdirs(&local, &["bin"])),
            Family::Zed => v.push(local.join("Zed")),
            Family::Eclipse => {
                v.extend(subdirs(&home.join("eclipse"), &["eclipse"]));
                v.push(PathBuf::from("C:\\eclipse"));
            }
            Family::QtCreator => v.push(PathBuf::from("C:\\Qt\\Tools\\QtCreator\\bin")),
            _ => {}
        }
    }
    if cfg!(target_os = "macos") {
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            for b in ide.bundle {
                let app = root.join(format!("{b}.app"));
                v.push(app.join("Contents/MacOS"));
                v.push(app.join("Contents/Resources/app/bin"));
                v.push(app.join("Contents/SharedSupport/bin"));
            }
        }
    }
    v
}

fn visual_studio() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let base = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from)?;
    let vswhere = base.join("Microsoft Visual Studio").join("Installer").join("vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }
    let (ok, out, _) = output(&vswhere.to_string_lossy(), &["-latest", "-property", "productPath"], None, Duration::from_secs(8)).ok()?;
    let p = PathBuf::from(out.trim());
    (ok && p.is_file()).then_some(p)
}

pub fn cli_path(ide: &Ide) -> Option<PathBuf> {
    if ide.key == "vscode" {
        return crate::tools::code::cli();
    }
    if ide.family == Family::VisualStudio {
        return visual_studio();
    }
    find_program(ide.cli, &extra_dirs(ide))
}

pub fn installed() -> Vec<&'static Ide> {
    IDES.iter().filter(|i| cli_path(i).is_some()).collect()
}

pub fn preferred(core: &dyn Prefs) -> Option<&'static Ide> {
    let k = core.pref("code.ide");
    by_key(&k).filter(|i| cli_path(i).is_some()).or_else(|| installed().into_iter().next())
}

pub fn gui_flags(ide: &Ide) -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(target_os = "linux") && ide.family == Family::VsCode {
        v.push(crate::tools::apps::A11Y_FLAG.to_owned());
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            v.push("--ozone-platform=wayland".to_owned());
        }
    }
    v
}

async fn run_wait(cli: &Path, args: &[String]) -> Result<()> {
    let c = cli.to_string_lossy().into_owned();
    let a = args.to_vec();
    let r = crate::layers::blocking(move || {
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        output(&c, &refs, None, Duration::from_secs(25))
    })
    .await
    .map_err(|_| anyhow!("The editor's command line stopped."))??;
    if r.0 { Ok(()) } else { Err(anyhow!("{}", r.2.lines().rfind(|l| !l.to_lowercase().contains("warning")).unwrap_or("The editor reported a problem.").trim())) }
}

fn detached(cli: &Path, args: &[String]) -> Result<()> {
    run_detached(cli, args, None)
}

static OPENED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn opened(path: &Path) {
    if !path.is_file() {
        return;
    }
    let mut g = OPENED.lock().unwrap();
    g.retain(|p| p != path);
    g.insert(0, path.to_path_buf());
    g.truncate(40);
}

fn opened_named(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    OPENED.lock().unwrap().iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name)) && p.is_file()).cloned()
}

fn recent_files(ide: &Ide, name: &str) -> Option<PathBuf> {
    if ide.family != Family::QtCreator || name.is_empty() {
        return None;
    }
    let ini = dirs::config_dir()?.join("QtProject").join("QtCreator.ini");
    let text = std::fs::read_to_string(ini).ok()?;
    text.split([',', '=', '"', '\n']).map(str::trim).filter(|t| looks_full(t)).map(|t| expand(Path::new(t))).find(|p| fits(p, name) && p.is_file())
}

pub async fn open(ide: &'static Ide, path: &Path, new_window: bool) -> Result<()> {
    opened(path);
    let cli = cli_path(ide).ok_or_else(|| anyhow!("{} isn't installed.", ide.name))?;
    let p = path.to_string_lossy().into_owned();
    match ide.family {
        Family::VsCode => {
            let mut a = gui_flags(ide);
            a.push(if new_window { "-n".into() } else { "-r".into() });
            a.push(p);
            run_wait(&cli, &a).await
        }
        Family::QtCreator => detached(&cli, &["-client".into(), p]),
        Family::Eclipse if path.is_file() => detached(&cli, &["--launcher.openFile".into(), p]),
        Family::VisualStudio if path.is_file() && path.extension().is_none_or(|e| e != "sln" && e != "slnx") => detached(&cli, &["/edit".into(), p]),
        Family::Sublime if new_window => detached(&cli, &["-n".into(), p]),
        _ => detached(&cli, &[p]),
    }
}

pub async fn goto(ide: &'static Ide, win: Option<&Win>, path: &Path, line: u32, col: u32) -> Result<()> {
    opened(path);
    let (line, col) = (line.max(1), col.max(1));
    let cli = cli_path(ide).ok_or_else(|| anyhow!("{} isn't installed.", ide.name))?;
    let p = path.to_string_lossy().into_owned();
    match ide.family {
        Family::VsCode => {
            let mut a = gui_flags(ide);
            a.extend(["-r".into(), "-g".into(), format!("{p}:{line}:{col}")]);
            run_wait(&cli, &a).await
        }
        Family::JetBrains => detached(&cli, &["--line".into(), line.to_string(), "--column".into(), col.to_string(), p]),
        Family::Xcode => detached(&cli, &["-l".into(), line.to_string(), p]),
        Family::Eclipse => detached(&cli, &["--launcher.openFile".into(), format!("{p}:{line}")]),
        Family::QtCreator => detached(&cli, &["-client".into(), format!("{p}:{line}:{col}")]),
        Family::Sublime | Family::Zed => detached(&cli, &[format!("{p}:{line}:{col}")]),
        Family::NotepadPlus => detached(&cli, &[format!("-n{line}"), format!("-c{col}"), p]),
        Family::VisualStudio => {
            detached(&cli, &["/edit".into(), p])?;
            match win {
                Some(w) => {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    goto_dialog(w, line).await
                }
                None => Ok(()),
            }
        }
    }
}

async fn goto_dialog(win: &Win, line: u32) -> Result<()> {
    menu(win, &["Edit", "Go To", "Go To Line..."]).await?;
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for (w, nodes) in os::popups(win).await.unwrap_or_default() {
            if let Some(f) = nodes.iter().find(|n| n.role == "field") {
                os::type_into(&w, f, &line.to_string(), true).await?;
                if let Some(ok) = nodes.iter().find(|n| n.role == "button" && plain(&n.name) == "ok") {
                    return os::act(ok, Do::Invoke).await;
                }
                return keys_if_front(&w, &[K::Enter]).await;
            }
        }
    }
    Err(anyhow!("The Go To Line box didn't open."))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Title {
    pub file: String,
    pub project: String,
    pub path: Option<String>,
    pub dirty: bool,
}

fn base_name(s: &str) -> String {
    s.rsplit(['/', '\\']).next().unwrap_or(s).trim().to_owned()
}

fn looks_full(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/') || s.starts_with("~/") || (b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

pub fn parse_title(ide: &Ide, title: &str) -> Title {
    let mut t = title.trim().to_owned();
    let mut dirty = false;
    for mark in ['●', '•', '*'] {
        if let Some(r) = t.strip_prefix(mark) {
            t = r.trim_start().to_owned();
            dirty = true;
        }
    }
    let mut out = Title { dirty, ..Default::default() };
    let head = |t: &str| -> String {
        let token = ide.titles.iter().filter(|x| title_has(t, x)).max_by_key(|x| x.len()).copied().unwrap_or("");
        if token.is_empty() {
            return t.to_owned();
        }
        for sep in [" - ", " — ", " – "] {
            if let Some(i) = t.find(&format!("{sep}{token}")) {
                return t[..i].to_owned();
            }
        }
        String::new()
    };
    match ide.family {
        Family::VsCode => {
            let h = head(&t);
            let parts: Vec<&str> = h.split(" - ").map(str::trim).filter(|p| !p.is_empty()).collect();
            match parts.len() {
                0 => {}
                1 => {
                    if parts[0].contains('.') {
                        out.file = parts[0].to_owned();
                    } else {
                        out.project = parts[0].to_owned();
                    }
                }
                _ => {
                    out.file = parts[0].to_owned();
                    out.project = parts[1].trim_end_matches(" (Workspace)").to_owned();
                }
            }
        }
        Family::Eclipse => {
            let h = head(&t);
            let parts: Vec<&str> = h.split(" - ").map(str::trim).filter(|p| !p.is_empty()).collect();
            if let Some(last) = parts.last() {
                if last.contains('/') || last.contains('.') {
                    out.file = base_name(last);
                    out.project = last.split('/').next().filter(|_| last.contains('/')).map(str::to_owned).unwrap_or_else(|| parts.first().map(|p| p.to_string()).unwrap_or_default());
                } else {
                    out.project = (*last).to_owned();
                }
            }
        }
        Family::QtCreator => {
            let h = head(&t);
            let (f, p) = h.split_once(" @ ").map(|(a, b)| (a.trim().to_owned(), b.trim().to_owned())).unwrap_or((h.trim().to_owned(), String::new()));
            let f = f.split(" (").next().unwrap_or(&f).to_owned();
            if let Some(x) = f.strip_suffix('*') {
                out.dirty = true;
                out.file = x.trim().to_owned();
            } else {
                out.file = f;
            }
            out.project = p;
        }
        Family::Sublime => {
            let h = head(&t);
            let mut h = h.trim().trim_end_matches(" •").to_owned();
            if let Some(i) = h.rfind(" (")
                && h.ends_with(')')
            {
                out.project = h[i + 2..h.len() - 1].to_owned();
                h.truncate(i);
            }
            if looks_full(h.trim()) {
                out.path = Some(h.trim().to_owned());
            }
            out.file = base_name(&h);
        }
        Family::NotepadPlus => {
            let h = head(&t);
            let h = h.trim();
            if looks_full(h) {
                out.path = Some(h.to_owned());
            }
            out.file = base_name(h);
        }
        Family::VisualStudio => {
            let h = head(&t);
            out.project = h.split(" - ").next().unwrap_or("").trim().to_owned();
        }
        Family::JetBrains | Family::Xcode | Family::Zed => {
            let sep = [" – ", " — ", " - "].into_iter().find(|s| t.contains(s)).unwrap_or(" – ");
            let mut parts: Vec<String> = t.split(sep).map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect();
            if parts.last().is_some_and(|p| p == "Edited") {
                parts.pop();
                out.dirty = true;
            }
            if parts.len() >= 2 {
                out.project = parts[0].clone();
                let f = parts[1].split(" [").next().unwrap_or(&parts[1]).to_owned();
                if looks_full(&f) {
                    out.path = Some(f.clone());
                }
                out.file = base_name(&f);
            } else if let Some(p) = parts.first() {
                out.project = p.clone();
            }
        }
    }
    out
}

pub fn expand(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(h) = dirs::home_dir()
    {
        return h.join(rest);
    }
    p.to_path_buf()
}

fn path_like(name: &str) -> Option<String> {
    let s = name.split(" • ").next().unwrap_or(name).trim();
    (looks_full(s) && !s.contains('\n') && s.len() < 1024).then(|| s.to_owned())
}

fn fits(p: &Path, file: &str) -> bool {
    file.is_empty() || p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(file))
}

pub fn path_from_nodes(nodes: &[Node], file: &str) -> Option<PathBuf> {
    for (i, n) in nodes.iter().enumerate() {
        if n.role == "tab"
            && n.selected
            && let Some(next) = nodes.get(i + 1)
            && let Some(p) = path_like(&next.name)
        {
            let pb = expand(Path::new(&p));
            if fits(&pb, file) {
                return Some(pb);
            }
        }
    }
    nodes.iter().filter_map(|n| path_like(&n.name)).map(|p| expand(Path::new(&p))).find(|p| fits(p, file) && !file.is_empty())
}

fn decode_uri(u: &str) -> Option<PathBuf> {
    let rest = u.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    let s = if cfg!(windows) { s.trim_start_matches('/').replace('/', "\\") } else { s };
    Some(PathBuf::from(s))
}

fn read_item(db: &Path, key: &str) -> Option<String> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let _ = c.busy_timeout(Duration::from_millis(800));
    c.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |r| {
        let v = r.get_ref(0)?;
        Ok(match v {
            rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        })
    })
    .ok()
}

fn vscode_dbs() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut v = vec![home.join(".vscode-shared").join("sharedStorage").join("state.vscdb")];
    let mut bases: Vec<PathBuf> = dirs::config_dir().into_iter().collect();
    if cfg!(target_os = "linux") {
        bases.push(home.join(".var/app/com.visualstudio.code/config"));
    }
    for base in bases {
        for ide in IDES.iter().filter(|i| i.family == Family::VsCode) {
            v.push(base.join(ide.config).join("User").join("globalStorage").join("state.vscdb"));
        }
    }
    v.into_iter().filter(|p| p.is_file()).collect()
}

fn recent_folders(ide: &Ide) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match ide.family {
        Family::VsCode => {
            for db in vscode_dbs() {
                let Some(raw) = read_item(&db, "history.recentlyOpenedPathsList") else { continue };
                let v: Value = serde_json::from_str(&raw).unwrap_or_default();
                for e in v.get("entries").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(p) = e.get("folderUri").and_then(Value::as_str).and_then(decode_uri) {
                        out.push(p);
                    }
                }
            }
            if let Some(cfg) = dirs::config_dir() {
                let f = cfg.join(ide.config).join("User").join("globalStorage").join("storage.json");
                let v: Value = std::fs::read_to_string(f).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
                for e in v.pointer("/backupWorkspaces/folders").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(p) = e.get("folderUri").and_then(Value::as_str).and_then(decode_uri) {
                        out.push(p);
                    }
                }
            }
        }
        Family::JetBrains => {
            let home = dirs::home_dir().unwrap_or_default();
            let mut bases: Vec<PathBuf> = dirs::config_dir().map(|c| vec![c.join("JetBrains"), c.join("Google")]).unwrap_or_default();
            if cfg!(windows) {
                bases.extend(std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("JetBrains")));
            }
            for base in bases {
                let Ok(rd) = std::fs::read_dir(&base) else { continue };
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !ide.config.split('|').any(|c| !c.is_empty() && name.starts_with(c)) {
                        continue;
                    }
                    let Ok(xml) = std::fs::read_to_string(e.path().join("options").join("recentProjects.xml")) else { continue };
                    for piece in xml.split("<entry key=\"").skip(1) {
                        let Some(end) = piece.find('"') else { continue };
                        let p = piece[..end].replace("$USER_HOME$", &home.to_string_lossy());
                        out.push(PathBuf::from(p));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

pub fn roots(core: &dyn Prefs, ide: &Ide, project: &str) -> Vec<PathBuf> {
    let want = normalize(project);
    let home = dirs::home_dir().unwrap_or_default();
    let mut cands: Vec<PathBuf> = Vec::new();
    let saved = core.pref("code.folder");
    if !saved.is_empty() {
        cands.push(PathBuf::from(saved));
    }
    cands.extend(recent_folders(ide));
    if !project.is_empty() {
        for base in ["Projects", "", "Documents", "Desktop", "IdeaProjects", "PycharmProjects", "CLionProjects", "WebstormProjects", "GolandProjects", "eclipse-workspace", "source/repos", "workspace", "Developer", "code", "src", "dev"] {
            cands.push(home.join(base).join(project));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for c in cands {
        if !c.is_dir() || out.contains(&c) {
            continue;
        }
        let name = c.file_name().map(|n| normalize(&n.to_string_lossy())).unwrap_or_default();
        if want.is_empty() || name == want {
            out.push(c);
        }
    }
    out
}

const SKIP: &[&str] = &[".git", "node_modules", "target", "build", "dist", ".venv", "venv", "__pycache__", ".idea", ".vscode", "out", "obj", ".gradle", ".cache", "Pods", "DerivedData"];

pub fn find_file(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    for root in roots {
        let mut queue = std::collections::VecDeque::from([(root.clone(), 0usize)]);
        let mut seen = 0usize;
        while let Some((dir, depth)) = queue.pop_front() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                seen += 1;
                if seen > 20000 {
                    break;
                }
                let p = e.path();
                let fname = e.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    if depth < 6 && !SKIP.contains(&fname.as_str()) {
                        queue.push_back((p, depth + 1));
                    }
                } else if fname.eq_ignore_ascii_case(name) {
                    return Some(p);
                }
            }
        }
    }
    None
}

pub fn language_of(path: &Path) -> String {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "c" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "h" => "cpp",
        "py" | "pyw" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "java" => "java",
        "go" => "go",
        "rs" => "rust",
        "cs" => "csharp",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" => "shellscript",
        "ps1" => "powershell",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" => "markdown",
        "sql" => "sql",
        "lua" => "lua",
        "dart" => "dart",
        "xml" => "xml",
        "r" => "r",
        "pl" => "perl",
        "vue" => "vue",
        _ => "",
    }
    .to_owned()
}

pub fn parse_position(name: &str) -> Option<(u32, u32)> {
    let low = name.trim().to_lowercase();
    let nums: Vec<u32> = low.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    if nums.len() < 2 {
        return None;
    }
    let named = low.starts_with("ln ") || low.starts_with("ln:") || low.starts_with("line ") || low.starts_with("line:");
    let bare = low.chars().all(|c| c.is_ascii_digit() || c == ':' || c == ' ') && low.contains(':');
    (named || bare).then(|| (nums[0].max(1), nums[1].max(1)))
}

pub fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, c) in text.chars().enumerate() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

const NOT_EDITOR: &[&str] = &["terminal", "chat input", "search", "filter", "type the name", "go to line", "find", "replace", "commit", "message", "quick open", "ask ", "command"];

pub fn editor_node(ide: &Ide, nodes: &[Node], file: &str) -> Option<Node> {
    let fields: Vec<&Node> = nodes.iter().filter(|n| n.role == "field" && n.enabled).collect();
    if ide.family == Family::VsCode && !file.is_empty() {
        let f = file.to_lowercase();
        if let Some(n) = fields.iter().find(|n| {
            let nm = n.name.to_lowercase();
            nm == f || nm.starts_with(&format!("{f},"))
        }) {
            return Some((*n).clone());
        }
    }
    let ok: Vec<&Node> = fields
        .into_iter()
        .filter(|n| {
            let nm = n.name.to_lowercase();
            !NOT_EDITOR.iter().any(|b| nm.contains(b))
        })
        .collect();
    ok.iter().filter(|n| n.focused).max_by_key(|n| n.w as i64 * n.h as i64).or_else(|| ok.iter().max_by_key(|n| n.w as i64 * n.h as i64)).map(|n| (*n).clone())
}

#[derive(Debug, Clone, Default)]
pub struct Active {
    pub path: Option<PathBuf>,
    pub name: String,
    pub project: String,
    pub root: Option<PathBuf>,
    pub text: Option<String>,
    pub line: u32,
    pub col: u32,
    pub dirty: bool,
    pub editor: Option<Node>,
    pub language: String,
}

fn read_small(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    if m.len() > 2_000_000 {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

static LAST_ROOT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

pub fn last_root() -> Option<PathBuf> {
    LAST_ROOT.lock().unwrap().clone().filter(|p| p.is_dir())
}

pub async fn active(core: &dyn Prefs, ide: &'static Ide, win: &Win) -> Active {
    let title = parse_title(ide, &win.title);
    let nodes = os::tree(win, Scope { limit: 5000, text: true, hidden: false }).await.unwrap_or_default();
    let mut a = Active { name: title.file.clone(), project: title.project.clone(), dirty: title.dirty, ..Default::default() };
    let rts = roots(core, ide, &title.project);
    a.path = title
        .path
        .as_deref()
        .map(|p| expand(Path::new(p)))
        .filter(|p| p.is_file())
        .or_else(|| path_from_nodes(&nodes, &title.file))
        .or_else(|| opened_named(&title.file))
        .or_else(|| find_file(&rts, &title.file))
        .or_else(|| recent_files(ide, &title.file));
    if a.name.is_empty()
        && let Some(p) = &a.path
    {
        a.name = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    }
    a.root = a.path.as_ref().and_then(|p| rts.iter().find(|r| p.starts_with(r)).cloned().or_else(|| crate::tools::git::root(p))).or_else(|| rts.first().cloned());
    a.editor = editor_node(ide, &nodes, &a.name);
    let shown = match &a.editor {
        Some(e) => os::text(e, 2_000_000).await,
        None => None,
    };
    let disk = a.path.as_deref().and_then(read_small);
    a.text = if a.dirty { shown.filter(|t| !t.trim().is_empty()).or(disk) } else { disk.or(shown) };
    if let Some((l, c)) = nodes.iter().filter(|n| matches!(n.role.as_str(), "button" | "text" | "other")).find_map(|n| parse_position(&n.name)) {
        a.line = l;
        a.col = c;
    } else if let (Some(e), Some(t)) = (&a.editor, &a.text)
        && let Some(off) = os::caret(e).await
    {
        (a.line, a.col) = line_col(t, off);
    }
    a.language = a.path.as_deref().map(language_of).unwrap_or_default();
    if let Some(r) = &a.root {
        *LAST_ROOT.lock().unwrap() = Some(r.clone());
    }
    remember(core, ide);
    a
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
    pub error: bool,
}

pub fn parse_vscode_issue(name: &str) -> Option<Issue> {
    let (error, rest) = [("Error: ", true), ("Warning: ", false), ("Info: ", false), ("Hint: ", false)].iter().find_map(|(p, e)| name.strip_prefix(p).map(|r| (*e, r)))?;
    let at = rest.rfind(" at line ")?;
    let message = rest[..at].trim().trim_end_matches('.').to_owned();
    let tail = &rest[at + 9..];
    let nums: Vec<u32> = tail.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).take(2).filter_map(|s| s.parse().ok()).collect();
    let line = *nums.first()?;
    Some(Issue { file: String::new(), line, col: nums.get(1).copied().unwrap_or(1), message, error })
}

pub fn parse_resource_row(name: &str) -> Option<String> {
    let i = name.find(" in file ")?;
    let rest = &name[i + 9..];
    let end = rest.rfind(" of folder ").unwrap_or(rest.len());
    let f = rest[..end].trim();
    (!f.is_empty()).then(|| f.to_owned())
}

pub fn parse_compiler_line(s: &str) -> Option<Issue> {
    let (idx, error) = [(": error: ", true), (": fatal error: ", true), (": warning: ", false)].iter().find_map(|(m, e)| s.find(m).map(|i| (i, *e)))?;
    let head = &s[..idx];
    let msg = s[idx..].splitn(3, ':').nth(2).unwrap_or("").trim().to_owned();
    let mut parts = head.rsplitn(3, ':');
    let a = parts.next()?.trim().parse::<u32>().ok()?;
    let b = parts.next().and_then(|x| x.trim().parse::<u32>().ok());
    let file = parts.next().map(base_name).unwrap_or_default();
    let (line, col) = match b {
        Some(l) => (l, a),
        None => (a, 1),
    };
    Some(Issue { file, line, col, message: msg, error })
}

pub fn issues_in(nodes: &[Node]) -> Vec<Issue> {
    let mut out = Vec::new();
    let mut file = String::new();
    let mut last_text = String::new();
    for n in nodes {
        if let Some(f) = parse_resource_row(&n.name)
            && n.name.contains(" problem")
        {
            file = f;
            continue;
        }
        if let Some(mut i) = parse_vscode_issue(&n.name).or_else(|| parse_compiler_line(&n.name)) {
            if i.file.is_empty() {
                i.file = file.clone();
            }
            if !out.contains(&i) {
                out.push(i);
            }
            continue;
        }
        let low = n.name.trim().to_lowercase();
        if let Some(num) = low.strip_prefix("line ").and_then(|x| x.trim().parse::<u32>().ok()) {
            if !last_text.is_empty() {
                let i = Issue { file: file.clone(), line: num, col: 1, message: last_text.clone(), error: true };
                if !out.contains(&i) {
                    out.push(i);
                }
            }
            continue;
        }
        if n.name.split_whitespace().count() >= 3 && matches!(n.role.as_str(), "cell" | "treeitem" | "row" | "item" | "text") {
            last_text = n.name.clone();
        }
    }
    out
}

fn problem_status(n: &Node) -> Option<u32> {
    if n.role != "button" {
        return None;
    }
    let s = n.name.trim();
    if s == "No Problems" {
        return Some(0);
    }
    if !(s.starts_with("Errors:") || s.starts_with("Warnings:") || s.starts_with("Infos:")) {
        return None;
    }
    let mut total = 0u32;
    for part in s.split(',') {
        total += part.split(':').nth(1).and_then(|x| x.trim().parse::<u32>().ok()).unwrap_or(0);
    }
    Some(total)
}

pub async fn problems(ide: &'static Ide, win: &Win) -> Option<Vec<Issue>> {
    let scope = Scope { limit: 5000, text: true, hidden: false };
    let nodes = os::tree(win, scope).await.ok()?;
    let found = issues_in(&nodes);
    if ide.family != Family::VsCode {
        return (!found.is_empty()).then_some(found);
    }
    let status = nodes.iter().find_map(|n| problem_status(n).map(|c| (n.clone(), c)));
    match status {
        Some((_, 0)) => Some(Vec::new()),
        Some((button, _)) if found.is_empty() => {
            os::act(&button, Do::Invoke).await.ok()?;
            let until = Instant::now() + Duration::from_secs(3);
            let mut got = Vec::new();
            while Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Ok(n) = os::tree(win, scope).await {
                    got = issues_in(&n);
                    if !got.is_empty() {
                        break;
                    }
                }
            }
            let _ = os::act(&button, Do::Invoke).await;
            Some(got)
        }
        _ if found.is_empty() => None,
        _ => Some(found),
    }
}

fn chord(t: &str) -> bool {
    let l = t.to_lowercase();
    if l.len() >= 2 && l.len() <= 3 && l.starts_with('f') && l[1..].chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    if l.chars().any(|c| "⌘⌥⇧⌃".contains(c)) {
        return true;
    }
    if let Some((m, rest)) = l.split_once('+') {
        return !rest.is_empty() && matches!(m, "ctrl" | "control" | "shift" | "alt" | "cmd" | "command" | "meta" | "super" | "win" | "option" | "opt");
    }
    false
}

pub fn plain(name: &str) -> String {
    let mut words: Vec<&str> = name.split_whitespace().collect();
    while words.last().is_some_and(|w| chord(w)) {
        words.pop();
    }
    let s = words.join(" ");
    let s = s.trim_end_matches("...").trim_end_matches('…').replace('&', "");
    normalize(&s)
}

fn item_fits(n: &Node, want: &str) -> bool {
    if !matches!(n.role.as_str(), "menuitem" | "checkbox" | "radio") {
        return false;
    }
    let have = plain(&n.name);
    match want.strip_suffix('*') {
        Some(prefix) => {
            let p = plain(prefix);
            !p.is_empty() && have.starts_with(&p) && (have.len() == p.len() || p.ends_with(|c: char| !c.is_alphanumeric()) || have[p.len()..].starts_with(' '))
        }
        None => have == plain(want),
    }
}

fn find_after(nodes: &[Node], want: &str, anchor: Option<&(String, String)>) -> Option<Node> {
    let start = anchor.and_then(|(role, name)| nodes.iter().position(|n| &n.role == role && &plain(&n.name) == name)).map(|i| i + 1).unwrap_or(0);
    nodes[start.min(nodes.len())..]
        .iter()
        .find(|n| item_fits(n, want))
        .or_else(|| nodes.iter().find(|n| item_fits(n, want) && anchor.is_none_or(|(role, name)| !(&n.role == role && &plain(&n.name) == name))))
        .cloned()
}

async fn menu_nodes(win: &Win) -> Vec<Node> {
    let mut nodes = os::tree(win, Scope { limit: 5000, text: false, hidden: true }).await.unwrap_or_default();
    for (_, mut extra) in os::popups(win).await.unwrap_or_default() {
        nodes.append(&mut extra);
    }
    nodes
}

async fn wait_item(win: &Win, want: &str, anchor: Option<&(String, String)>, ms: u64) -> Option<Node> {
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        tokio::time::sleep(Duration::from_millis(180)).await;
        let nodes = menu_nodes(win).await;
        if let Some(n) = find_after(&nodes, want, anchor) {
            return Some(n);
        }
        if Instant::now() >= until {
            return None;
        }
    }
}

async fn close_menu(win: &Win, top: &Node) {
    if os::in_front(win, 0).await {
        let _ = input::press(&[K::Esc]).await;
    } else {
        let _ = os::act(top, Do::Expand).await;
    }
}

pub async fn menu(win: &Win, path: &[&str]) -> Result<()> {
    let (first, rest) = path.split_first().ok_or_else(|| anyhow!("Which menu?"))?;
    let nodes = menu_nodes(win).await;
    let top = match find_after(&nodes, first, None) {
        Some(n) => n,
        None => {
            let more = find_after(&nodes, "More", None).or_else(|| find_after(&nodes, "Application Menu", None)).ok_or_else(|| anyhow!("I couldn't find the {} menu.", first.trim_end_matches('*')))?;
            os::act(&more, Do::Expand).await?;
            let anchor = (more.role.clone(), plain(&more.name));
            wait_item(win, first, Some(&anchor), 1500).await.ok_or_else(|| anyhow!("I couldn't find the {} menu.", first.trim_end_matches('*')))?
        }
    };
    if rest.is_empty() {
        return os::act(&top, Do::Invoke).await;
    }
    os::act(&top, Do::Expand).await?;
    let mut anchor = (top.role.clone(), plain(&top.name));
    for (i, want) in rest.iter().enumerate() {
        let Some(item) = wait_item(win, want, Some(&anchor), 2000).await else {
            close_menu(win, &top).await;
            return Err(anyhow!("I couldn't find {} in the {} menu.", want.trim_end_matches('*'), first.trim_end_matches('*')));
        };
        if i + 1 == rest.len() {
            return os::act(&item, Do::Invoke).await;
        }
        os::act(&item, Do::Expand).await?;
        anchor = (item.role.clone(), plain(&item.name));
    }
    Ok(())
}

pub async fn keys_if_front(win: &Win, keys: &[K]) -> Result<()> {
    if os::in_front(win, 500).await {
        input::press(keys).await
    } else {
        Err(anyhow!("{} isn't in front.", if win.app.is_empty() { "That window" } else { win.app.as_str() }))
    }
}

pub async fn bring_front(ide: &'static Ide, win: &Win) -> bool {
    if os::in_front(win, 0).await {
        return true;
    }
    if ide.family == Family::VsCode
        && let Some(cli) = cli_path(ide)
    {
        let mut a = gui_flags(ide);
        a.push("-r".into());
        let _ = run_wait(&cli, &a).await;
        if os::in_front(win, 1500).await {
            return true;
        }
    }
    let _ = crate::tools::wm::focus(ide.name, None).await;
    os::in_front(win, 1500).await
}

const EDIT_PASTE: &[&str] = &["Edit", "Paste"];

pub async fn paste(win: &Win, target: Option<&Node>, text: &str) -> Result<()> {
    let old = crate::layers::blocking(crate::tools::shell::paste_text).await.ok().flatten();
    let t = text.to_owned();
    crate::layers::blocking(move || crate::tools::shell::copy_text(&t)).await.map_err(|_| anyhow!("The clipboard stopped."))??;
    if let Some(n) = target {
        let _ = os::act(n, Do::Focus).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let r = match menu(win, EDIT_PASTE).await {
        Ok(()) => Ok(()),
        Err(e) => keys_if_front(win, &[input::primary(), K::Ch('v')]).await.map_err(|_| e),
    };
    tokio::time::sleep(Duration::from_millis(700)).await;
    if let Some(o) = old {
        let mine = text.to_owned();
        let _ = crate::layers::blocking(move || {
            if crate::tools::shell::paste_text().as_deref() == Some(mine.as_str()) {
                let _ = crate::tools::shell::copy_text(&o);
            }
        })
        .await;
    }
    r
}

pub async fn insert(core: &dyn Prefs, ide: &'static Ide, win: &Win, text: &str) -> Result<&'static str> {
    let a = active(core, ide, win).await;
    insert_at(ide, win, &a, text).await
}

pub async fn insert_at(ide: &'static Ide, win: &Win, a: &Active, text: &str) -> Result<&'static str> {
    if cfg!(not(windows))
        && ide.family != Family::VsCode
        && let Some(e) = a.editor.as_ref().filter(|e| e.editable)
        && os::act(e, Do::Insert(text.to_owned())).await.is_ok()
    {
        return Ok("accessibility");
    }
    match paste(win, a.editor.as_ref(), text).await {
        Ok(()) => Ok("paste"),
        Err(e) => match &a.editor {
            Some(n) => os::type_into(win, n, text, false).await,
            None => Err(e),
        },
    }
}

pub fn indent_block(code: &str, base: &str) -> String {
    let lines: Vec<String> = code.replace('\r', "").replace('\t', "    ").split('\n').map(str::to_owned).collect();
    let mut lines = lines;
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let body: Vec<String> = lines.iter().enumerate().map(|(i, l)| if i == 0 || l.is_empty() { l.clone() } else { format!("{base}{l}") }).collect();
    format!("{}\n{base}", body.join("\n"))
}

pub fn line_indent(text: &str, line: u32) -> String {
    text.lines().nth(line.saturating_sub(1) as usize).map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect()).unwrap_or_default()
}

pub async fn save(ide: &'static Ide, win: &Win) -> Result<()> {
    let path: &[&str] = match ide.family {
        Family::JetBrains | Family::VisualStudio => &["File", "Save All"],
        _ => &["File", "Save"],
    };
    match menu(win, path).await {
        Ok(()) => Ok(()),
        Err(e) => keys_if_front(win, &[input::primary(), K::Ch('s')]).await.map_err(|_| e),
    }
}

pub enum Step {
    Menu(&'static [&'static str]),
    Button(&'static str),
    Palette(&'static str),
}

pub fn vscode_steps(id: &str) -> Vec<Step> {
    use Step::*;
    match id {
        "editor.action.formatDocument" => vec![Palette("Format Document")],
        "workbench.action.toggleSidebarVisibility" => vec![Button("Toggle Primary Side Bar"), Menu(&["View", "Appearance", "Primary Side Bar"]), Palette("View: Toggle Primary Side Bar Visibility")],
        "workbench.action.splitEditor" => vec![Button("Split Editor Right"), Menu(&["View", "Editor Layout", "Split Right"]), Palette("View: Split Editor")],
        "workbench.action.closeActiveEditor" => vec![Menu(&["File", "Close Editor"])],
        "workbench.action.toggleZenMode" => vec![Menu(&["View", "Appearance", "Zen Mode"]), Palette("View: Toggle Zen Mode")],
        "workbench.action.terminal.new" => vec![Menu(&["Terminal", "New Terminal"])],
        "workbench.actions.view.problems" => vec![Menu(&["View", "Problems"])],
        "workbench.view.extensions" => vec![Menu(&["View", "Extensions"])],
        "workbench.view.explorer" => vec![Menu(&["View", "Explorer"])],
        "editor.action.commentLine" => vec![Menu(&["Edit", "Toggle Line Comment"])],
        "workbench.action.debug.start" => vec![Menu(&["Run", "Start Debugging"])],
        "workbench.action.debug.stop" => vec![Menu(&["Run", "Stop Debugging"])],
        "workbench.action.debug.run" => vec![Menu(&["Run", "Run Without Debugging"])],
        "workbench.action.quickOpen" => vec![Menu(&["Go", "Go to File"])],
        "workbench.action.openSettings" => vec![Menu(&["File", "Preferences", "Settings"]), Menu(&["Code", "Settings", "Settings"]), Palette("Preferences: Open Settings")],
        "workbench.action.findInFiles" => vec![Menu(&["Edit", "Find in Files"])],
        "undo" => vec![Menu(&["Edit", "Undo"])],
        "redo" => vec![Menu(&["Edit", "Redo"])],
        "workbench.action.files.save" => vec![Menu(&["File", "Save"])],
        "editor.action.selectAll" => vec![Menu(&["Selection", "Select All"])],
        "editor.action.clipboardPasteAction" => vec![Menu(&["Edit", "Paste"])],
        "workbench.action.files.newUntitledFile" => vec![Menu(&["File", "New Text File"])],
        "editor.action.copyLinesDownAction" => vec![Menu(&["Selection", "Copy Line Down"])],
        "editor.action.deleteLines" => vec![Palette("Delete Line")],
        "editor.action.indentLines" => vec![Palette("Indent Line")],
        "editor.action.outdentLines" => vec![Palette("Outdent Line")],
        "workbench.action.terminal.toggleTerminal" => vec![Menu(&["View", "Terminal"])],
        _ => Vec::new(),
    }
}

pub fn other_steps(ide: &Ide, id: &str) -> Vec<Step> {
    use Step::*;
    let jb = ide.family == Family::JetBrains;
    match id {
        "undo" => vec![Menu(&["Edit", "Undo*"])],
        "redo" => vec![Menu(&["Edit", "Redo*"])],
        "workbench.action.files.save" => vec![Menu(&["File", "Save All"]), Menu(&["File", "Save"])],
        "editor.action.selectAll" => vec![Menu(&["Edit", "Select All"])],
        "editor.action.clipboardPasteAction" => vec![Menu(&["Edit", "Paste"])],
        "editor.action.formatDocument" if jb => vec![Menu(&["Code", "Reformat Code"])],
        "editor.action.formatDocument" if ide.family == Family::Eclipse => vec![Menu(&["Source", "Format"])],
        "editor.action.formatDocument" if ide.family == Family::VisualStudio => vec![Menu(&["Edit", "Advanced", "Format Document"])],
        "editor.action.commentLine" if jb => vec![Menu(&["Code", "Comment with Line Comment"])],
        "editor.action.commentLine" if ide.family == Family::Eclipse => vec![Menu(&["Source", "Toggle Comment"])],
        "workbench.action.debug.start" if jb => vec![Menu(&["Run", "Debug*"])],
        "workbench.action.debug.start" if ide.family == Family::VisualStudio => vec![Menu(&["Debug", "Start Debugging"])],
        "workbench.action.debug.stop" if jb => vec![Menu(&["Run", "Stop*"])],
        "workbench.action.debug.stop" if ide.family == Family::VisualStudio => vec![Menu(&["Debug", "Stop Debugging"])],
        "workbench.action.files.newUntitledFile" => vec![Menu(&["File", "New*"])],
        _ => Vec::new(),
    }
}

pub fn run_steps(ide: &Ide) -> Vec<Step> {
    use Step::*;
    match ide.family {
        Family::VisualStudio => vec![Menu(&["Debug", "Start Without Debugging"])],
        Family::JetBrains => vec![Menu(&["Run", "Run '*"]), Menu(&["Run", "Run..."])],
        Family::Xcode => vec![Menu(&["Product", "Run"])],
        Family::Eclipse => vec![Menu(&["Run", "Run"])],
        Family::QtCreator => vec![Button("Run"), Menu(&["Build", "Run"])],
        _ => Vec::new(),
    }
}

pub fn build_steps(ide: &Ide) -> Vec<Step> {
    use Step::*;
    match ide.family {
        Family::VisualStudio => vec![Menu(&["Build", "Build Solution"])],
        Family::JetBrains => vec![Menu(&["Build", "Build Project"]), Menu(&["Build", "Build Module*"])],
        Family::Xcode => vec![Menu(&["Product", "Build"])],
        Family::Eclipse => vec![Menu(&["Project", "Build All"]), Menu(&["Project", "Build Project"])],
        Family::QtCreator => vec![Button("Build Project for All Configurations"), Menu(&["Build", "Build Project*"]), Menu(&["Build", "Build All Projects"])],
        Family::VsCode => vec![Menu(&["Terminal", "Run Build Task"])],
        _ => Vec::new(),
    }
}

async fn focused_field(win: &Win, hint: &str, secs: u64) -> Option<Node> {
    let until = Instant::now() + Duration::from_secs(secs);
    let hint = hint.to_lowercase();
    let mut seen: Option<Node> = None;
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(nodes) = os::tree(win, Scope { limit: 4000, text: false, hidden: false }).await else { continue };
        let fits: Vec<Node> = nodes.into_iter().filter(|n| n.role == "field" && (hint.is_empty() || n.name.to_lowercase().contains(&hint))).collect();
        if let Some(n) = fits.iter().find(|n| n.focused) {
            return Some(n.clone());
        }
        if let Some(n) = fits.last() {
            if seen.as_ref().is_some_and(|s| s.id == n.id) {
                return Some(n.clone());
            }
            seen = Some(n.clone());
        }
    }
    seen
}

async fn type_in(ide: &'static Ide, win: &Win, field: &Node, text: &str, enter: bool) -> Result<()> {
    if !bring_front(ide, win).await {
        return Err(anyhow!("{} isn't in front, so I can't type there.", ide.name));
    }
    os::type_into(win, field, text, false).await?;
    if enter {
        tokio::time::sleep(Duration::from_millis(350)).await;
        input::press(&[K::Enter]).await?;
    }
    Ok(())
}

async fn step(core: &dyn Prefs, ide: &'static Ide, win: &Win, s: &Step) -> Result<()> {
    match s {
        Step::Menu(p) => menu(win, p).await,
        Step::Button(prefix) => {
            let want = plain(prefix);
            let nodes = os::tree(win, Scope { limit: 5000, text: false, hidden: false }).await?;
            let b = nodes.iter().find(|n| matches!(n.role.as_str(), "button" | "toggle" | "tab") && (plain(&n.name) == want || plain(&n.name).starts_with(&format!("{want} ")))).ok_or_else(|| anyhow!("I couldn't find {prefix}."))?;
            os::act(b, Do::Invoke).await
        }
        Step::Palette(label) => {
            if let Some(e) = active(core, ide, win).await.editor {
                let _ = os::act(&e, Do::Focus).await;
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            menu(win, &["View", "Command Palette"]).await?;
            let f = focused_field(win, "command", 3).await.ok_or_else(|| anyhow!("The command palette didn't open."))?;
            type_in(ide, win, &f, label, false).await?;
            let want = plain(label);
            let until = Instant::now() + Duration::from_millis(2500);
            while Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let Ok(nodes) = os::tree(win, Scope { limit: 5000, text: false, hidden: false }).await else { continue };
                let rows: Vec<&Node> = nodes.iter().filter(|n| n.role == "item" && !n.name.is_empty()).collect();
                let fits = |n: &Node| {
                    let have = plain(&n.name);
                    have == want || have.starts_with(&format!("{want} "))
                };
                if let Some(top) = rows.iter().find(|n| n.selected || n.focused)
                    && fits(top)
                {
                    return input::press(&[K::Enter]).await;
                }
                if let Some(row) = rows.iter().find(|n| fits(n)) {
                    return os::act(row, Do::Invoke).await;
                }
            }
            let _ = input::press(&[K::Esc]).await;
            Err(anyhow!("{label} isn't in the command palette."))
        }
    }
}

pub async fn steps(core: &dyn Prefs, ide: &'static Ide, win: &Win, list: &[Step]) -> Result<()> {
    let mut last = anyhow!("{} has no way to do that.", ide.name);
    for s in list {
        match step(core, ide, win, s).await {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub async fn command(core: &dyn Prefs, ide: &'static Ide, win: &Win, id: &str) -> Result<()> {
    let list = if ide.family == Family::VsCode { vscode_steps(id) } else { other_steps(ide, id) };
    steps(core, ide, win, &list).await
}

pub async fn search_in(core: &dyn Prefs, ide: &'static Ide, win: &Win, id: &str, hint: &str, query: &str) -> Result<()> {
    command(core, ide, win, id).await?;
    let f = focused_field(win, hint, 4).await.ok_or_else(|| anyhow!("The search box didn't open."))?;
    type_in(ide, win, &f, query, false).await
}

pub fn shell_of(name: &str) -> &'static str {
    let low = name.to_lowercase();
    if low.contains("pwsh") || low.contains("powershell") {
        "powershell"
    } else if low.contains(", cmd") || low.contains("command prompt") {
        "cmd"
    } else {
        "posix"
    }
}

pub fn join_steps(steps: &[String], shell: &str, dir: Option<&Path>) -> String {
    let body = match shell {
        "powershell" => steps.iter().fold(String::new(), |acc, s| if acc.is_empty() { s.clone() } else { format!("{acc}; if ($?) {{ {s} }}") }),
        _ => steps.join(" && "),
    };
    match (dir, shell) {
        (Some(d), "powershell") => format!("Set-Location \"{}\"; {body}", d.display()),
        (Some(d), "cmd") => format!("cd /d \"{}\" && {body}", d.display()),
        (Some(d), _) => format!("cd \"{}\" && {body}", d.display()),
        (None, _) => body,
    }
}

pub async fn terminal(ide: &'static Ide, win: &Win, steps: &[String], dir: Option<&Path>) -> Result<String> {
    if ide.family != Family::VsCode {
        return Err(anyhow!("{} has no terminal I can use.", ide.name));
    }
    menu(win, &["Terminal", "New Terminal"]).await?;
    let field = focused_field(win, "terminal", 6).await.ok_or_else(|| anyhow!("The terminal didn't open."))?;
    let line = join_steps(steps, shell_of(&field.name), dir);
    if !bring_front(ide, win).await {
        return Err(anyhow!("{} isn't in front, so I can't type in its terminal.", ide.name));
    }
    os::type_into(win, &field, &format!("{line}\n"), false).await?;
    Ok(line)
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn icon_only(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| ('\u{e000}'..='\u{f8ff}').contains(&c))
}

pub fn program_output(rows: &[&str], typed: &str) -> Option<(String, bool)> {
    let marker: String = typed.chars().take(24).collect();
    if marker.trim().is_empty() {
        return None;
    }
    let start = rows.iter().rposition(|r| r.contains(&marker))?;
    let prompt = rows[start][..rows[start].find(&marker)?].trim().to_owned();
    let want = squash(typed);
    let mut seen = String::new();
    let mut i = start;
    while i < rows.len() && i < start + 12 && !squash(&seen).contains(&want) {
        seen.push_str(rows[i]);
        i += 1;
    }
    let mut out = Vec::new();
    let mut finished = false;
    for r in rows.iter().skip(i) {
        let t = r.trim();
        if !prompt.is_empty() && t.starts_with(&prompt) {
            finished = true;
            break;
        }
        if icon_only(t) || out.len() >= 300 {
            break;
        }
        out.push(*r);
    }
    Some((out.join("\n"), finished))
}

pub async fn terminal_output(win: &Win, typed: &str) -> Option<(String, bool)> {
    let nodes = os::tree(win, Scope { limit: 6000, text: true, hidden: false }).await.ok()?;
    let rows: Vec<&str> = nodes.iter().filter(|n| n.role == "text").map(|n| n.name.as_str()).collect();
    program_output(&rows, typed)
}

fn uri_of(dir: &Path) -> Value {
    let raw = dir.to_string_lossy().into_owned();
    let path = if cfg!(windows) {
        let fwd = raw.replace('\\', "/");
        let mut c = fwd.chars();
        match (c.next(), c.next()) {
            (Some(d), Some(':')) => format!("/{}:{}", d.to_ascii_lowercase(), &fwd[2..]),
            _ => format!("/{fwd}"),
        }
    } else {
        raw.clone()
    };
    let mut enc = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            enc.push(b as char);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    let fs = if cfg!(windows) {
        let mut c = raw.chars();
        match (c.next(), c.next()) {
            (Some(d), Some(':')) => format!("{}{}", d.to_ascii_lowercase(), &raw[1..]),
            _ => raw.clone(),
        }
    } else {
        raw.clone()
    };
    json!({ "$mid": 1, "fsPath": fs, "external": format!("file://{enc}"), "path": path, "scheme": "file" })
}

pub fn add_trust(raw: Option<&str>, dir: &Path) -> Option<String> {
    let mut v: Value = raw.and_then(|r| serde_json::from_str(r).ok()).unwrap_or_else(|| json!({ "uriTrustInfo": [] }));
    if !v.get("uriTrustInfo").is_some_and(Value::is_array) {
        v["uriTrustInfo"] = json!([]);
    }
    let uri = uri_of(dir);
    let list = v["uriTrustInfo"].as_array_mut()?;
    if list.iter().any(|e| e.pointer("/uri/path") == uri.get("path") && e.get("trusted").and_then(Value::as_bool) == Some(true)) {
        return None;
    }
    list.retain(|e| e.pointer("/uri/path") != uri.get("path"));
    list.push(json!({ "uri": uri, "trusted": true }));
    serde_json::to_string(&v).ok()
}

fn trust_db(db: &Path, dir: &Path) -> Result<bool> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    c.busy_timeout(Duration::from_secs(2))?;
    let cur = read_item(db, "content.trust.model.key");
    let Some(next) = add_trust(cur.as_deref(), dir) else { return Ok(false) };
    c.execute("INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)", rusqlite::params!["content.trust.model.key", next])?;
    Ok(true)
}

pub fn trust(dir: &Path) -> usize {
    vscode_dbs().iter().filter(|db| trust_db(db, dir).unwrap_or(false)).count()
}

pub fn projects_root() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Projects"))
}

pub async fn trust_window(win: &Win) -> bool {
    let Ok(nodes) = os::tree(win, Scope { limit: 5000, text: false, hidden: false }).await else { return false };
    if let Some(yes) = nodes.iter().find(|n| n.role == "button" && plain(&n.name).starts_with("yes i trust the authors")) {
        if let Some(parent) = nodes.iter().find(|n| n.role == "checkbox" && n.name.to_lowercase().contains("parent folder") && n.checked != Some(true)) {
            let _ = os::act(parent, Do::Toggle).await;
        }
        return os::act(yes, Do::Invoke).await.is_ok();
    }
    let restricted = nodes.iter().any(|n| n.name.starts_with("Restricted Mode"));
    if !restricted {
        return false;
    }
    let Some(manage) = nodes.iter().find(|n| n.role == "button" && n.name == "Manage") else { return false };
    if os::act(manage, Do::Invoke).await.is_err() {
        return false;
    }
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if let Ok(n) = os::tree(win, Scope { limit: 5000, text: false, hidden: false }).await
            && let Some(t) = n.iter().find(|x| x.role == "button" && x.name == "Trust")
        {
            let done = os::act(t, Do::Invoke).await.is_ok();
            tokio::time::sleep(Duration::from_millis(400)).await;
            if let Ok(after) = os::tree(win, Scope { limit: 5000, text: false, hidden: false }).await
                && let Some(c) = after.iter().find(|x| x.role == "button" && x.name.starts_with("Close Modal Editor"))
            {
                let _ = os::act(c, Do::Invoke).await;
            }
            return done;
        }
    }
    false
}

pub async fn after_open(core: &dyn Prefs, ide: &'static Ide, dir: &Path, created: bool) -> Option<Win> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let found = wait_window(core, Some(ide), &name, 20).await;
    if let Some((i, w)) = &found {
        remember(core, i);
        if created && i.family == Family::VsCode && projects_root().is_some_and(|r| dir.starts_with(&r) && dir != r) {
            let until = Instant::now() + Duration::from_secs(4);
            while Instant::now() < until {
                if trust_window(w).await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    }
    found.map(|(_, w)| w)
}

pub fn prepare_open(ide: &Ide, dir: &Path) {
    if ide.family == Family::VsCode
        && dir.is_dir()
        && projects_root().is_some_and(|root| dir.starts_with(&root) && dir != root)
    {
        let _ = trust(dir);
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn argv_dir(ide: &Ide) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(match ide.key {
        "vscode" => ".vscode",
        "insiders" => ".vscode-insiders",
        "vscodium" | "codeoss" => ".vscode-oss",
        "cursor" => ".cursor",
        "windsurf" => ".windsurf",
        _ => return None,
    }))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn argv_with_flag(text: &str) -> Option<String> {
    if text.contains("\"force-renderer-accessibility\"") {
        return None;
    }
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    let mut open: Option<usize> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '/' && bytes.get(i + 1) == Some(&'/') {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && bytes.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == '{' {
            open = Some(i);
            break;
        }
        i += 1;
    }
    let Some(at) = open else { return Some("{\n\t\"force-renderer-accessibility\": true\n}\n".to_owned()) };
    let after: String = bytes[at + 1..].iter().collect();
    let has_more = after.split('\n').map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with("//")).any(|l| l != "}" && !l.starts_with('}'));
    let before: String = bytes[..=at].iter().collect();
    Some(format!("{before}\n\t\"force-renderer-accessibility\": true{}{after}", if has_more { "," } else { "" }))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn make_argv_ready(ide: &Ide) -> Result<bool> {
    if ide.family != Family::VsCode || !cfg!(target_os = "linux") {
        return Ok(false);
    }
    let dir = argv_dir(ide).ok_or_else(|| anyhow!("no settings folder"))?;
    let f = dir.join("argv.json");
    let text = std::fs::read_to_string(&f).unwrap_or_default();
    let Some(next) = argv_with_flag(&text) else { return Ok(true) };
    std::fs::create_dir_all(&dir)?;
    let tmp = f.with_extension("json.setu-tmp");
    std::fs::write(&tmp, next)?;
    std::fs::rename(&tmp, &f)?;
    Ok(true)
}

#[cfg(target_os = "linux")]
pub fn java_ides() -> Vec<(&'static Ide, u32)> {
    let uid = unsafe { libc::getuid() };
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        use std::os::unix::fs::MetadataExt;
        if e.metadata().map(|m| m.uid() != uid).unwrap_or(true) {
            continue;
        }
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let line = String::from_utf8_lossy(&raw).replace('\0', " ");
        if !line.contains("com.intellij.idea.Main") {
            continue;
        }
        let prefix = line.split("-Didea.platform.prefix=").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("");
        let key = match prefix {
            "Python" | "PyCharmCore" | "PyCharmEdu" => "pycharm",
            "CLion" => "clion",
            "WebStorm" => "webstorm",
            "GoLand" => "goland",
            "Rider" => "rider",
            "PhpStorm" => "phpstorm",
            "Ruby" => "rubymine",
            "RustRover" => "rustrover",
            "AndroidStudio" => "androidstudio",
            _ if line.contains("android-studio") || line.contains("AndroidStudio") => "androidstudio",
            _ => "intellij",
        };
        if let Some(i) = by_key(key)
            && !out.iter().any(|(x, _): &(&Ide, u32)| x.key == i.key)
        {
            out.push((i, pid));
        }
    }
    out
}

#[cfg(not(target_os = "linux"))]
pub fn java_ides() -> Vec<(&'static Ide, u32)> {
    Vec::new()
}

pub fn jetbrains_screen_reader() -> usize {
    let home = dirs::home_dir().unwrap_or_default();
    let mut bases: Vec<PathBuf> = dirs::config_dir().map(|c| vec![c.join("JetBrains"), c.join("Google")]).unwrap_or_default();
    if cfg!(target_os = "macos") {
        bases.push(home.join("Library/Application Support/JetBrains"));
    }
    let mut done = 0;
    for base in bases {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !IDES.iter().filter(|i| i.family == Family::JetBrains).any(|i| i.config.split('|').any(|c| !c.is_empty() && name.starts_with(c))) {
                continue;
            }
            let dir = e.path().join("options");
            let f = dir.join("ide.general.xml");
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            let Some(next) = with_screen_reader(&text) else { continue };
            if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&f, next).is_ok() {
                done += 1;
            }
        }
    }
    done
}

pub fn with_screen_reader(xml: &str) -> Option<String> {
    let opt = "<option name=\"supportScreenReaders\" value=\"true\" />";
    if xml.contains("name=\"supportScreenReaders\" value=\"true\"") {
        return None;
    }
    if xml.contains("name=\"supportScreenReaders\"") {
        return Some(xml.replace("name=\"supportScreenReaders\" value=\"false\"", "name=\"supportScreenReaders\" value=\"true\""));
    }
    if let Some(i) = xml.find("<component name=\"GeneralSettings\">") {
        let at = i + "<component name=\"GeneralSettings\">".len();
        return Some(format!("{}\n    {opt}{}", &xml[..at], &xml[at..]));
    }
    if let Some(i) = xml.find("<application>") {
        let at = i + "<application>".len();
        return Some(format!("{}\n  <component name=\"GeneralSettings\">\n    {opt}\n  </component>{}", &xml[..at], &xml[at..]));
    }
    Some(format!("<application>\n  <component name=\"GeneralSettings\">\n    {opt}\n  </component>\n</application>\n"))
}

pub fn java_bridge_on() -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("no home folder"))?;
    let f = home.join(".accessibility.properties");
    let text = std::fs::read_to_string(&f).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.trim_start().starts_with("assistive_technologies") && !l.trim_start().starts_with("screen_magnifier_present")).map(str::to_owned).collect();
    let tech = if cfg!(windows) { "com.sun.java.accessibility.AccessBridge" } else { "org.GNOME.Accessibility.AtkWrapper" };
    lines.push(format!("assistive_technologies={tech}"));
    lines.push("screen_magnifier_present=true".into());
    std::fs::write(&f, lines.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(app: &str, title: &str, front: bool) -> Win {
        Win { id: title.into(), app: app.into(), title: title.into(), pid: 100 + title.len() as u32, front }
    }

    fn n(role: &str, name: &str) -> Node {
        Node { id: name.into(), role: role.into(), name: name.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn knows_coding_apps_by_window_and_voice() {
        assert_eq!(of_window(&w("code", "app.js - demo - Visual Studio Code - 1 problem in this file", true)).map(|i| i.key), Some("vscode"));
        assert_eq!(of_window(&w("Code - Insiders", "main.rs - x - Visual Studio Code - Insiders", true)).map(|i| i.key), Some("insiders"));
        assert_eq!(of_window(&w("QtCreator", "main.cpp - Qt Creator", true)).map(|i| i.key), Some("qtcreator"));
        assert_eq!(of_window(&w("Devenv", "ConsoleApp1 - Microsoft Visual Studio (Administrator)", true)).map(|i| i.key), Some("visualstudio"));
        assert_eq!(of_window(&w("Idea64", "demo – Main.java", true)).map(|i| i.key), Some("intellij"));
        assert_eq!(of_window(&w("Notepad++", "*C:\\notes\\a.txt - Notepad++", true)).map(|i| i.key), Some("notepadpp"));
        assert!(of_window(&w("Notepad", "a.txt - Notepad", true)).is_none());
        assert!(of_window(&w("gedit", "a.txt - gedit", true)).is_none());
        assert_eq!(spoken("open main dot cpp in visual studio code").map(|i| i.key), Some("vscode"));
        assert_eq!(spoken("open this project in visual studio").map(|i| i.key), Some("visualstudio"));
        assert_eq!(spoken("open it in pycharm").map(|i| i.key), Some("pycharm"));
        assert_eq!(spoken("switch to android studio").map(|i| i.key), Some("androidstudio"));
        let wins = vec![w("code", "a.js - demo - Visual Studio Code", false), w("gedit", "x", true), w("QtCreator", "main.cpp - Qt Creator", false)];
        assert_eq!(pick(&wins, None, None, "qtcreator").map(|(i, _)| i.key), Some("qtcreator"));
        assert_eq!(pick(&wins, None, Some(&wins[0]), "qtcreator").map(|(i, _)| i.key), Some("vscode"));
        assert_eq!(pick(&wins, by_key("vscode"), None, "").map(|(i, _)| i.key), Some("vscode"));
    }

    #[test]
    fn reads_titles() {
        let vs = by_key("vscode").unwrap();
        assert_eq!(parse_title(vs, "● app.js - demo - Visual Studio Code - 1 problem in this file"), Title { file: "app.js".into(), project: "demo".into(), path: None, dirty: true });
        assert_eq!(parse_title(vs, "demo - Visual Studio Code").project, "demo");
        assert_eq!(parse_title(vs, "Welcome - Visual Studio Code").file, "");
        let jb = by_key("intellij").unwrap();
        let t = parse_title(jb, "demo – src/main/Main.java [demo]");
        assert_eq!((t.file.as_str(), t.project.as_str()), ("Main.java", "demo"));
        let qt = by_key("qtcreator").unwrap();
        let t = parse_title(qt, "main.cpp* @ demo - Qt Creator");
        assert_eq!((t.file.as_str(), t.project.as_str(), t.dirty), ("main.cpp", "demo", true));
        let ec = by_key("eclipse").unwrap();
        let t = parse_title(ec, "eclipse-workspace - demo/src/Main.java - Eclipse IDE");
        assert_eq!((t.file.as_str(), t.project.as_str()), ("Main.java", "demo"));
        let np = by_key("notepadpp").unwrap();
        let t = parse_title(np, "*C:\\notes\\todo.txt - Notepad++");
        assert_eq!((t.file.as_str(), t.path.as_deref(), t.dirty), ("todo.txt", Some("C:\\notes\\todo.txt"), true));
        let xc = by_key("xcode").unwrap();
        let t = parse_title(xc, "Demo — ContentView.swift — Edited");
        assert_eq!((t.file.as_str(), t.project.as_str(), t.dirty), ("ContentView.swift", "Demo", true));
        let st = by_key("sublime").unwrap();
        let t = parse_title(st, "main.py (demo) - Sublime Text");
        assert_eq!((t.file.as_str(), t.project.as_str()), ("main.py", "demo"));
    }

    #[test]
    fn reads_paths_positions_and_problems() {
        let nodes = vec![n("tab", "Explorer (Ctrl+Shift+E)"), n("treeitem", "main.cpp"), n("other", "/home/u/Projects/demo/main.cpp"), Node { selected: true, ..n("tab", "app.js") }, n("other", "/home/u/Projects/demo/app.js • 1 problem in this file"), n("button", "Ln 4, Col 9")];
        assert_eq!(path_from_nodes(&nodes, "app.js"), Some(PathBuf::from("/home/u/Projects/demo/app.js")));
        assert_eq!(path_from_nodes(&nodes, "main.cpp"), Some(PathBuf::from("/home/u/Projects/demo/main.cpp")));
        assert_eq!(parse_position("Ln 4, Col 9"), Some((4, 9)));
        assert_eq!(parse_position("Line: 12, Col: 3"), Some((12, 3)));
        assert_eq!(parse_position("4:9"), Some((4, 9)));
        assert_eq!(parse_position("12 : 5 : 120"), Some((12, 5)));
        assert_eq!(parse_position("Spaces: 4"), None);
        assert_eq!(parse_position("line 5"), None);
        assert_eq!(line_col("ab\ncd\nef", 4), (2, 2));
        let i = parse_vscode_issue("Error: Expression expected. at line 4 and character 9. generated by ts").unwrap();
        assert_eq!((i.line, i.col, i.message.as_str(), i.error), (4, 9, "Expression expected", true));
        assert!(!parse_vscode_issue("Warning: 'x' is declared but its value is never read. at line 2 and character 5. generated by ts").unwrap().error);
        assert_eq!(parse_resource_row("1 problems in file app.js of folder ."), Some("app.js".into()));
        let rows = vec![n("treeitem", "2 problems in file main.cpp of folder demo"), n("treeitem", "Error: expected ';' after expression at line 4 and character 15. generated by C/C++")];
        let got = issues_in(&rows);
        assert_eq!((got[0].file.as_str(), got[0].line, got[0].col), ("main.cpp", 4, 15));
        let c = parse_compiler_line("/tmp/demo/main.cpp:4:15: error: expected ';' before 'std'").unwrap();
        assert_eq!((c.file.as_str(), c.line, c.col), ("main.cpp", 4, 15));
        let ecl = vec![n("cell", "Syntax error, insert ; to complete BlockStatements"), n("cell", "Main.java"), n("cell", "line 5")];
        assert_eq!(issues_in(&ecl).first().map(|i| i.line), Some(5));
        assert_eq!(problem_status(&n("button", "Errors: 1, Warnings: 2")), Some(3));
        assert_eq!(problem_status(&n("button", "No Problems")), Some(0));
    }

    #[test]
    fn matches_menu_items() {
        assert_eq!(plain("Paste Ctrl+V"), "paste");
        assert_eq!(plain("Save Ctrl+S"), "save");
        assert_eq!(plain("Save As... Ctrl+Shift+S"), "save as");
        assert_eq!(plain("New Terminal Ctrl+Shift+`"), "new terminal");
        assert_eq!(plain("Command Palette... Ctrl+Shift+P"), "command palette");
        assert_eq!(plain("Start Debugging F5"), "start debugging");
        assert_eq!(plain("Output Ctrl+K Ctrl+H"), "output");
        assert_eq!(plain("Go to Line/Column... Ctrl+G"), "go to line column");
        let items = vec![n("menuitem", "Run"), n("menuitem", "Start Debugging F5"), n("menuitem", "Run Without Debugging Ctrl+F5"), n("menuitem", "More"), n("menuitem", "Run 'Main' Shift+F10")];
        let anchor = ("menuitem".to_owned(), "run".to_owned());
        assert_eq!(find_after(&items, "Run Without Debugging", Some(&anchor)).map(|x| x.name), Some("Run Without Debugging Ctrl+F5".into()));
        assert_eq!(find_after(&items, "Run '*", Some(&anchor)).map(|x| x.name), Some("Run 'Main' Shift+F10".into()));
        assert!(find_after(&items, "Save", Some(&anchor)).is_none());
        let saves = vec![n("menuitem", "File"), n("menuitem", "Save Workspace As..."), n("menuitem", "Save Ctrl+S")];
        assert_eq!(find_after(&saves, "Save", Some(&("menuitem".into(), "file".into()))).map(|x| x.name), Some("Save Ctrl+S".into()));
    }

    #[test]
    fn writes_trust_argv_and_settings() {
        let one = add_trust(None, Path::new("/home/u/Projects")).unwrap();
        let v: Value = serde_json::from_str(&one).unwrap();
        assert_eq!(v["uriTrustInfo"][0]["trusted"], true);
        assert!(add_trust(Some(&one), Path::new("/home/u/Projects")).is_none());
        let two: Value = serde_json::from_str(&add_trust(Some(&one), Path::new("/home/u/Work")).unwrap()).unwrap();
        assert_eq!(two["uriTrustInfo"].as_array().map(|a| a.len()), Some(2));
        let argv = "// comment with { brace\n{\n\t// \"disable-hardware-acceleration\": true,\n\t\"enable-crash-reporter\": true\n}\n";
        let got = argv_with_flag(argv).unwrap();
        assert!(got.contains("{\n\t\"force-renderer-accessibility\": true,\n"));
        assert!(argv_with_flag(&got).is_none());
        assert_eq!(argv_with_flag("{\n}\n").unwrap(), "{\n\t\"force-renderer-accessibility\": true\n}\n");
        assert!(argv_with_flag("").unwrap().contains("force-renderer-accessibility"));
        let xml = "<application>\n  <component name=\"GeneralSettings\">\n    <option name=\"confirmExit\" value=\"false\" />\n  </component>\n</application>";
        let x = with_screen_reader(xml).unwrap();
        assert!(x.contains("supportScreenReaders\" value=\"true\""));
        assert!(with_screen_reader(&x).is_none());
        assert_eq!(join_steps(&["g++ a.cpp -o a".into(), "./a".into()], "posix", None), "g++ a.cpp -o a && ./a");
        assert_eq!(join_steps(&["a".into(), "b".into()], "powershell", None), "a; if ($?) { b }");
        assert_eq!(shell_of("Terminal 1, bash Use Shift+Alt+F1 for terminal accessibility help"), "posix");
        assert_eq!(shell_of("Terminal 1, pwsh"), "powershell");
        assert_eq!(indent_block("if (x) {\n    y();\n}\n", "    "), "if (x) {\n        y();\n    }\n    ");
        let typed = "cd \"/tmp/a/Projects/demo\" && python3 \"/tmp/a/Projects/demo/crash.py\"";
        let rows = vec!["src", "bash-5.3$ cd \"/tmp/a/Projects/demo\" && py", "thon3 \"/tmp/a/Projects/demo/crash.py\"", "Traceback (most recent call last):", "File \"/tmp/a/Projects/demo/crash.py\", l", "ine 2, in <module>", "ZeroDivisionError: division by zero", "bash-5.3$", "\u{ebfb}", "Chat"];
        let (out, done) = program_output(&rows, typed).unwrap();
        assert!(done && out.starts_with("Traceback") && out.ends_with("division by zero"));
        let running = vec!["bash-5.3$ cd \"/tmp/a/Projects/demo\" && python3 \"/tmp/a/Projects/demo/crash.py\"", "working", "\u{ebfb}"];
        assert_eq!(program_output(&running, typed), Some(("working".to_owned(), false)));
    }
}

pub async fn probe(v: &Value) -> Result<Value> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let wins = os::windows().await?;
    let want = by_key(&s("ide"));
    let (ide, win) = pick(&wins, want, None, "").ok_or_else(|| anyhow!("no coding app window"))?;
    let core = NoPrefs;
    match s("op").as_str() {
        "ide" => {
            let a = active(&core, ide, &win).await;
            let issues = if v.get("problems").and_then(Value::as_bool).unwrap_or(false) { problems(ide, &win).await } else { None };
            Ok(json!({
                "ide": ide.key, "window": win, "path": a.path, "name": a.name, "project": a.project, "root": a.root,
                "line": a.line, "col": a.col, "dirty": a.dirty, "language": a.language, "editor": a.editor.as_ref().map(|e| e.name.clone()),
                "text": a.text, "problems": issues.map(|l| l.into_iter().map(|i| json!({ "file": i.file, "line": i.line, "col": i.col, "message": i.message, "error": i.error })).collect::<Vec<_>>()),
            }))
        }
        "ide_menu" => {
            let path: Vec<String> = v.get("path").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            let refs: Vec<&str> = path.iter().map(String::as_str).collect();
            menu(&win, &refs).await?;
            Ok(json!({ "ok": true }))
        }
        "ide_insert" => Ok(json!({ "how": insert(&core, ide, &win, &s("text")).await? })),
        "ide_command" => {
            command(&core, ide, &win, &s("id")).await?;
            Ok(json!({ "ok": true }))
        }
        _ => {
            let p = PathBuf::from(s("path"));
            let line = v.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
            goto(ide, Some(&win), &p, line, v.get("col").and_then(Value::as_u64).unwrap_or(1) as u32).await?;
            Ok(json!({ "ok": true }))
        }
    }
}
