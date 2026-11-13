use crate::control::ConfirmSpec;
use crate::text::{normalize, similarity};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::proc::Quiet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Package {
    pub manager: String,
    pub id: String,
    pub name: String,
    pub source: String,
    pub publisher: String,
    pub scope: String,
    pub size: String,
}

fn out(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).quiet().args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    (o.status.success() || !text.trim().is_empty()).then_some(text)
}

#[allow(dead_code)]
struct Known {
    words: &'static [&'static str],
    name: &'static str,
    winget: &'static str,
    msstore: &'static str,
    flatpak: &'static str,
    native: &'static str,
    cask: &'static str,
    formula: &'static str,
}

const KNOWN: &[Known] = &[
    Known { words: &["vs code", "vscode", "visual studio code", "v s code", "code editor"], name: "Visual Studio Code", winget: "Microsoft.VisualStudioCode", msstore: "", flatpak: "com.visualstudio.code", native: "", cask: "visual-studio-code", formula: "" },
    Known { words: &["chrome", "google chrome"], name: "Google Chrome", winget: "Google.Chrome", msstore: "", flatpak: "com.google.Chrome", native: "", cask: "google-chrome", formula: "" },
    Known { words: &["brave", "brave browser"], name: "Brave", winget: "Brave.Brave", msstore: "", flatpak: "com.brave.Browser", native: "", cask: "brave-browser", formula: "" },
    Known { words: &["firefox", "mozilla firefox"], name: "Firefox", winget: "Mozilla.Firefox", msstore: "", flatpak: "org.mozilla.firefox", native: "firefox", cask: "firefox", formula: "" },
    Known { words: &["git"], name: "Git", winget: "Git.Git", msstore: "", flatpak: "", native: "git", cask: "", formula: "git" },
    Known { words: &["github cli", "gh", "github command line"], name: "GitHub CLI", winget: "GitHub.cli", msstore: "", flatpak: "", native: "gh", cask: "", formula: "gh" },
    Known { words: &["python", "python 3", "python3"], name: "Python", winget: "Python.Python.3.13", msstore: "", flatpak: "", native: "python3", cask: "", formula: "python" },
    Known { words: &["node", "nodejs", "node js", "node.js"], name: "Node.js", winget: "OpenJS.NodeJS.LTS", msstore: "", flatpak: "", native: "nodejs", cask: "", formula: "node" },
    Known { words: &["mingw", "min gw", "gcc", "g plus plus", "g++", "c++ compiler", "c plus plus compiler", "c compiler", "compiler"], name: "GCC compiler", winget: "BrechtSanders.WinLibs.POSIX.UCRT", msstore: "", flatpak: "", native: "gcc-c++", cask: "", formula: "gcc" },
    Known { words: &["java", "jdk", "java development kit"], name: "Java JDK", winget: "EclipseAdoptium.Temurin.21.JDK", msstore: "", flatpak: "", native: "java-21-openjdk-devel", cask: "temurin", formula: "" },
    Known { words: &["vlc", "vlc media player", "vlc player"], name: "VLC", winget: "VideoLAN.VLC", msstore: "", flatpak: "org.videolan.VLC", native: "vlc", cask: "vlc", formula: "" },
    Known { words: &["spotify"], name: "Spotify", winget: "", msstore: "9NCBCSZSJRSB", flatpak: "com.spotify.Client", native: "", cask: "spotify", formula: "" },
    Known { words: &["whatsapp", "whats app"], name: "WhatsApp", winget: "", msstore: "9NKSQGP7F2NH", flatpak: "", native: "", cask: "whatsapp", formula: "" },
    Known { words: &["netflix"], name: "Netflix", winget: "", msstore: "9WZDNCRFJ3TJ", flatpak: "", native: "", cask: "", formula: "" },
    Known { words: &["telegram"], name: "Telegram", winget: "Telegram.TelegramDesktop", msstore: "", flatpak: "org.telegram.desktop", native: "", cask: "telegram", formula: "" },
    Known { words: &["discord"], name: "Discord", winget: "Discord.Discord", msstore: "", flatpak: "com.discordapp.Discord", native: "", cask: "discord", formula: "" },
    Known { words: &["zoom"], name: "Zoom", winget: "Zoom.Zoom", msstore: "", flatpak: "us.zoom.Zoom", native: "", cask: "zoom", formula: "" },
    Known { words: &["obs", "obs studio"], name: "OBS Studio", winget: "OBSProject.OBSStudio", msstore: "", flatpak: "com.obsproject.Studio", native: "", cask: "obs", formula: "" },
    Known { words: &["7zip", "7 zip", "seven zip"], name: "7-Zip", winget: "7zip.7zip", msstore: "", flatpak: "", native: "p7zip", cask: "", formula: "sevenzip" },
    Known { words: &["notepad plus plus", "notepad++"], name: "Notepad++", winget: "Notepad++.Notepad++", msstore: "", flatpak: "", native: "", cask: "", formula: "" },
    Known { words: &["steam"], name: "Steam", winget: "Valve.Steam", msstore: "", flatpak: "com.valvesoftware.Steam", native: "", cask: "steam", formula: "" },
    Known { words: &["gimp"], name: "GIMP", winget: "GIMP.GIMP.3", msstore: "", flatpak: "org.gimp.GIMP", native: "gimp", cask: "gimp", formula: "" },
    Known { words: &["blender"], name: "Blender", winget: "BlenderFoundation.Blender", msstore: "", flatpak: "org.blender.Blender", native: "blender", cask: "blender", formula: "" },
    Known { words: &["libreoffice", "libre office", "office"], name: "LibreOffice", winget: "TheDocumentFoundation.LibreOffice", msstore: "", flatpak: "org.libreoffice.LibreOffice", native: "libreoffice", cask: "libreoffice", formula: "" },
    Known { words: &["docker", "docker desktop"], name: "Docker", winget: "Docker.DockerDesktop", msstore: "", flatpak: "", native: "", cask: "docker", formula: "" },
    Known { words: &["postman"], name: "Postman", winget: "Postman.Postman", msstore: "", flatpak: "com.getpostman.Postman", native: "", cask: "postman", formula: "" },
    Known { words: &["android studio"], name: "Android Studio", winget: "Google.AndroidStudio", msstore: "", flatpak: "com.google.AndroidStudio", native: "", cask: "android-studio", formula: "" },
    Known { words: &["intellij", "intellij idea"], name: "IntelliJ IDEA Community", winget: "JetBrains.IntelliJIDEA.Community", msstore: "", flatpak: "com.jetbrains.IntelliJ-IDEA-Community", native: "", cask: "intellij-idea-ce", formula: "" },
    Known { words: &["pycharm"], name: "PyCharm Community", winget: "JetBrains.PyCharm.Community", msstore: "", flatpak: "com.jetbrains.PyCharm-Community", native: "", cask: "pycharm-ce", formula: "" },
];

fn known(name: &str) -> Option<&'static Known> {
    let t = normalize(name);
    let t = t.trim_start_matches("the ").trim_end_matches(" app").trim_end_matches(" application").trim();
    KNOWN.iter().find(|k| k.words.contains(&t))
}

fn pkg(manager: &str, id: &str, name: &str, source: &str, publisher: &str, scope: &str) -> Package {
    Package { manager: manager.into(), id: id.into(), name: name.into(), source: source.into(), publisher: publisher.into(), scope: scope.into(), size: String::new() }
}

pub fn search(name: &str) -> Result<Package, String> {
    if let Some(k) = known(name)
        && let Some(p) = from_known(k)
    {
        return Ok(p);
    }
    search_stores(name)
}

#[cfg(target_os = "windows")]
fn from_known(k: &Known) -> Option<Package> {
    if !k.winget.is_empty() {
        return Some(pkg("winget", k.winget, k.name, "winget", k.winget.split('.').next().unwrap_or(""), "user"));
    }
    if !k.msstore.is_empty() {
        return Some(pkg("winget", k.msstore, k.name, "msstore", "Microsoft Store", "user"));
    }
    None
}

#[cfg(target_os = "macos")]
fn from_known(k: &Known) -> Option<Package> {
    let brew = crate::tools::apps::which("brew").is_some() || std::path::Path::new("/opt/homebrew/bin/brew").exists();
    if brew && !k.cask.is_empty() {
        return Some(pkg("brew", k.cask, k.name, "Homebrew cask", "", "user"));
    }
    if brew && !k.formula.is_empty() {
        return Some(pkg("brew-formula", k.formula, k.name, "Homebrew", "", "user"));
    }
    if !k.cask.is_empty() {
        return Some(pkg("cask", k.cask, k.name, "the publisher's website", "", "user"));
    }
    None
}

#[cfg(target_os = "linux")]
fn from_known(k: &Known) -> Option<Package> {
    let native = native_manager();
    if !k.native.is_empty()
        && let Some(m) = native
    {
        let id = match (m, k.native) {
            ("apt", "gcc-c++") => "g++",
            ("pacman", "gcc-c++") => "gcc",
            ("zypper", "gcc-c++") => "gcc-c++",
            ("apt", "java-21-openjdk-devel") => "openjdk-21-jdk",
            ("pacman", "java-21-openjdk-devel") => "jdk21-openjdk",
            ("pacman", "python3") => "python",
            ("pacman", "nodejs") => "nodejs",
            ("apt", "p7zip") => "p7zip-full",
            (_, n) => n,
        };
        return Some(pkg(m, id, k.name, m, "", "system"));
    }
    if !k.flatpak.is_empty() && crate::tools::apps::which("flatpak").is_some() {
        return Some(pkg("flatpak", k.flatpak, k.name, "flathub", k.flatpak.split('.').take(2).collect::<Vec<_>>().join(".").as_str(), flatpak_scope("flathub")));
    }
    None
}

#[cfg(target_os = "linux")]
fn flatpak_scope(remote: &str) -> &'static str {
    let remotes = out("flatpak", &["remotes", "--columns=name,options"]).unwrap_or_default();
    if remotes.lines().any(|l| l.starts_with(remote) && l.contains("user")) { "user" } else { "system" }
}

#[cfg(target_os = "linux")]
pub fn native_manager() -> Option<&'static str> {
    use crate::tools::apps::which;
    if which("dnf").is_some() || which("dnf5").is_some() {
        Some("dnf")
    } else if which("apt-get").is_some() {
        Some("apt")
    } else if which("pacman").is_some() {
        Some("pacman")
    } else if which("zypper").is_some() {
        Some("zypper")
    } else {
        None
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn best<'a>(name: &str, items: impl Iterator<Item = (&'a str, &'a str)>) -> Option<(f64, String, String)> {
    let mut top: Option<(f64, String, String)> = None;
    for (id, display) in items {
        let display_clean = display.trim_end_matches('…').trim();
        let tail = id.rsplit(['.', '/']).next().unwrap_or(id);
        let mut s = similarity(name, display_clean).max(similarity(name, tail) * 0.95).max(similarity(name, id) * 0.9);
        if normalize(display_clean) == normalize(name) || normalize(tail) == normalize(name) {
            s = s.max(1.0);
        }
        if top.as_ref().is_none_or(|(b, ..)| s > *b) {
            top = Some((s, id.to_owned(), display_clean.to_owned()));
        }
    }
    top
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_winget(text: &str) -> Vec<(String, String, String)> {
    let lines: Vec<String> = text.split('\n').map(|l| l.rsplit('\r').next().unwrap_or(l).to_owned()).collect();
    let Some(h) = lines.iter().position(|l| l.contains("Id") && l.contains("Version") && l.starts_with("Name")) else { return Vec::new() };
    let header = &lines[h];
    let id_at = header.find("Id").unwrap_or(0);
    let ver_at = header.find("Version").unwrap_or(header.len());
    let src_at = header.find("Source");
    let mut rows = Vec::new();
    for l in lines.iter().skip(h + 1) {
        if l.trim().is_empty() || l.trim_start().starts_with('-') {
            continue;
        }
        let chars: Vec<char> = l.chars().collect();
        let cut = |a: usize, b: usize| -> String { chars.iter().skip(a).take(b.saturating_sub(a)).collect::<String>().trim().to_owned() };
        let name = cut(0, id_at);
        let id = cut(id_at, chars.len()).split_whitespace().next().unwrap_or("").to_owned();
        let _ = ver_at;
        let source = src_at.map(|s| cut(s, chars.len())).unwrap_or_else(|| "winget".into());
        if !id.is_empty() && !name.is_empty() {
            rows.push((name, id, if source.is_empty() { "winget".into() } else { source }));
        }
    }
    rows
}

#[cfg(target_os = "windows")]
fn search_stores(name: &str) -> Result<Package, String> {
    let winget = crate::tools::fetch::win::ensure_winget().ok_or("I couldn't set up winget on this computer, so I can only install the apps I know.")?;
    let list = out(&winget, &["search", name, "--accept-source-agreements", "--disable-interactivity"]).ok_or("The Microsoft Store and winget aren't available on this computer.")?;
    let rows = parse_winget(&list);
    let mut top: Option<(f64, &(String, String, String))> = None;
    for r in &rows {
        let tail = r.1.rsplit('.').next().unwrap_or(&r.1);
        let mut s = similarity(name, r.0.trim_end_matches('…')).max(similarity(name, tail) * 0.95);
        if r.2 == "msstore" {
            s += 0.01;
        }
        if top.as_ref().is_none_or(|(b, _)| s > *b) {
            top = Some((s, r));
        }
    }
    match top.filter(|(s, _)| *s >= 0.8) {
        Some((_, r)) => Ok(pkg("winget", &r.1, r.0.trim_end_matches('…'), &r.2, if r.2 == "msstore" { "Microsoft Store" } else { r.1.split('.').next().unwrap_or("") }, "user")),
        None => Err(format!("I couldn't find {name} in the Microsoft Store or winget.")),
    }
}

#[cfg(target_os = "macos")]
fn search_stores(name: &str) -> Result<Package, String> {
    use crate::tools::apps::which;
    if which("mas").is_some()
        && let Some(list) = out("mas", &["search", name])
    {
        let items: Vec<(String, String)> = list
            .lines()
            .filter_map(|l| {
                let t = l.trim();
                let (id, rest) = t.split_once(char::is_whitespace)?;
                id.parse::<u64>().ok()?;
                let title = rest.rsplit_once('(').map(|(a, _)| a).unwrap_or(rest).trim().to_owned();
                Some((id.to_owned(), title))
            })
            .collect();
        if let Some((s, id, display)) = best(name, items.iter().map(|(a, b)| (a.as_str(), b.as_str())))
            && s >= 0.9
        {
            return Ok(pkg("mas", &id, &display, "App Store", "Apple App Store", "user"));
        }
    }
    let brew = if which("brew").is_some() { "brew" } else if std::path::Path::new("/opt/homebrew/bin/brew").exists() { "/opt/homebrew/bin/brew" } else { "" };
    if brew.is_empty() {
        return match crate::tools::fetch::mac::search(name) {
            Some((token, display)) => Ok(pkg("cask", &token, &display, "the publisher's website", "", "user")),
            None => Err(format!("I couldn't find {name} to download.")),
        };
    }
    for (flag, manager) in [("--cask", "brew"), ("--formula", "brew-formula")] {
        if let Some(list) = out(brew, &["search", flag, &normalize(name).replace(' ', "-")]) {
            let items: Vec<&str> = list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("==>")).collect();
            if let Some((s, id, _)) = best(name, items.iter().map(|t| (*t, *t)))
                && s >= 0.85
            {
                return Ok(pkg(manager, &id, &crate::text::sentence_case(&id.replace('-', " ")), "Homebrew", "", "user"));
            }
        }
    }
    Err(format!("I couldn't find {name} in the App Store or Homebrew."))
}

#[cfg(target_os = "linux")]
fn search_stores(name: &str) -> Result<Package, String> {
    use crate::tools::apps::which;
    let n = name.to_owned();
    let native = native_manager().map(|m| {
        let q = n.clone();
        std::thread::spawn(move || native_search(m, &q))
    });
    let q = n.clone();
    let flat = which("flatpak").is_some().then(|| std::thread::spawn(move || flatpak_search(&q)));
    let q = n.clone();
    let snap = which("snap").is_some().then(|| std::thread::spawn(move || snap_search(&q)));
    let native = native.and_then(|h| h.join().ok()).flatten();
    let flat = flat.and_then(|h| h.join().ok()).flatten();
    let snap = snap.and_then(|h| h.join().ok()).flatten();
    if let Some((s, p)) = &native
        && *s >= 0.97
    {
        return Ok(p.clone());
    }
    if let Some((s, p)) = &flat
        && *s >= 0.8
    {
        return Ok(p.clone());
    }
    if let Some((s, p)) = &native
        && *s >= 0.85
    {
        return Ok(p.clone());
    }
    if let Some((s, p)) = &snap
        && *s >= 0.85
    {
        return Ok(p.clone());
    }
    if native.is_none() && flat.is_none() && snap.is_none() && native_manager().is_none() && which("flatpak").is_none() {
        return Err(format!("I couldn't find {name}, and no package manager is available on this system."));
    }
    Err(format!("I couldn't find {name} in the software sources."))
}

#[cfg(target_os = "linux")]
fn native_search(m: &str, name: &str) -> Option<(f64, Package)> {
    let q = normalize(name).replace(' ', "-");
    let list: Vec<(String, String)> = match m {
        "dnf" => {
            let pat = format!("*{q}*");
            let text = out("dnf", &["-C", "-q", "repoquery", "--available", "--queryformat", "%{name}\\n", &pat])
                .filter(|t| !t.trim().is_empty())
                .or_else(|| out("dnf", &["-q", "repoquery", "--available", "--queryformat", "%{name}\\n", &pat]))?;
            text.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| (l.to_owned(), l.to_owned())).collect()
        }
        "apt" => out("apt-cache", &["search", "--names-only", &q])?.lines().filter_map(|l| l.split_once(" - ").map(|(a, _)| (a.trim().to_owned(), a.trim().to_owned()))).collect(),
        "pacman" => out("pacman", &["-Ss", &q])?
            .lines()
            .filter(|l| !l.starts_with(' '))
            .filter_map(|l| l.split_whitespace().next().and_then(|x| x.split('/').nth(1)).map(|x| (x.to_owned(), x.to_owned())))
            .collect(),
        "zypper" => out("zypper", &["--non-interactive", "--no-refresh", "search", "-t", "package", &q])?
            .lines()
            .filter_map(|l| {
                let cols: Vec<&str> = l.split('|').map(str::trim).collect();
                (cols.len() >= 3 && !cols[1].is_empty() && cols[1] != "Name").then(|| (cols[1].to_owned(), cols[1].to_owned()))
            })
            .collect(),
        _ => return None,
    };
    let (s, id, _) = best(name, list.iter().map(|(a, b)| (a.as_str(), b.as_str())))?;
    Some((s, pkg(m, &id, &crate::text::sentence_case(&id.replace('-', " ")), m, "", "system")))
}

#[cfg(target_os = "linux")]
fn flatpak_search(name: &str) -> Option<(f64, Package)> {
    let list = out("flatpak", &["search", "--columns=application,name,remotes", name])?;
    let items: Vec<(String, String, String)> = list
        .lines()
        .filter_map(|l| {
            let cols: Vec<String> = l.split('\t').map(|s| s.trim().to_owned()).collect();
            (cols.len() >= 3 && cols[0].contains('.')).then(|| (cols[0].clone(), cols[1].clone(), cols[2].clone()))
        })
        .collect();
    let (s, id, display) = best(name, items.iter().map(|(a, b, _)| (a.as_str(), b.as_str())))?;
    let remote = items.iter().find(|i| i.0 == id).map(|i| i.2.split(',').next().unwrap_or("flathub").to_owned()).unwrap_or_else(|| "flathub".into());
    let scope = flatpak_scope(&remote);
    let info = out("flatpak", &["remote-info", &format!("--{scope}"), &remote, &id]).unwrap_or_default();
    let size = info.lines().find(|l| l.trim_start().starts_with("Download:")).map(|l| l.split(':').nth(1).unwrap_or("").trim().replace('\u{a0}', " ")).unwrap_or_default();
    let mut p = pkg("flatpak", &id, &display, &remote, &id.split('.').take(2).collect::<Vec<_>>().join("."), scope);
    p.size = size;
    Some((s, p))
}

#[cfg(target_os = "linux")]
fn snap_search(name: &str) -> Option<(f64, Package)> {
    let list = out("snap", &["find", name])?;
    let items: Vec<(String, String)> = list.lines().skip(1).filter_map(|l| l.split_whitespace().next().map(|n| (n.to_owned(), l.split_whitespace().nth(2).unwrap_or("").to_owned()))).collect();
    let (s, id, _) = best(name, items.iter().map(|(a, _)| (a.as_str(), a.as_str())))?;
    let publisher = items.iter().find(|i| i.0 == id).map(|i| i.1.clone()).unwrap_or_default();
    Some((s, pkg("snap", &id, &crate::text::sentence_case(&id.replace('-', " ")), "Snap Store", &publisher, "system")))
}

pub fn store_name(p: &Package) -> &'static str {
    match (p.manager.as_str(), p.source.as_str()) {
        ("winget", "msstore") => "the Microsoft Store",
        ("winget", _) => "winget",
        ("mas", _) => "the App Store",
        ("brew" | "brew-formula", _) => "Homebrew",
        ("cask", _) => "the publisher's website",
        ("flatpak", _) => "Flathub",
        ("snap", _) => "the Snap Store",
        ("dnf", _) => "Fedora's software sources",
        ("apt", _) => "the system software sources",
        ("pacman", _) => "the Arch repositories",
        ("zypper", _) => "the openSUSE repositories",
        _ => "the software store",
    }
}

pub fn confirm_spec(p: &Package) -> ConfirmSpec {
    let size = if p.size.is_empty() { String::new() } else { format!("  about {}", p.size) };
    let admin = matches!(p.manager.as_str(), "dnf" | "apt" | "pacman" | "zypper" | "snap") || p.manager == "flatpak" && p.scope == "system";
    ConfirmSpec {
        title: format!("Install {} from {}?", p.name, store_name(p)),
        facts: vec![format!("{}  {}{}", p.manager, p.id, size), format!("source  {}", p.source)],
        consequence: if admin { "Your computer will ask for your password to approve it.".into() } else { String::new() },
        risk: if admin { "high" } else { "medium" },
        action_label: format!("Install {}", p.name),
        danger: false,
    }
}

pub fn install(p: &Package, cancel: &AtomicBool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if p.manager == "cask" {
        return crate::tools::fetch::mac::install(&p.id, cancel);
    }
    #[cfg(windows)]
    let winget = if p.manager == "winget" {
        match crate::tools::fetch::win::ensure_winget() {
            Some(w) => w,
            None if p.source != "msstore" => return crate::tools::fetch::win::direct(&p.id, cancel),
            None => {
                let _ = Command::new("cmd").quiet().args(["/C", "start", "", &format!("ms-windows-store://pdp/?productid={}", p.id)]).spawn();
                return Err(format!("I opened {} in the Microsoft Store. Choose Get to finish.", p.name));
            }
        }
    } else {
        "winget".to_owned()
    };
    #[cfg(not(windows))]
    let winget = "winget".to_owned();
    let mut cmd = match p.manager.as_str() {
        "flatpak" => {
            let mut c = Command::new("flatpak");
            c.args(["install", &format!("--{}", p.scope), "-y", "--noninteractive", &p.source, &p.id]);
            c
        }
        "winget" => {
            let mut c = Command::new(&winget);
            c.quiet();
            c.args(["install", "--id", &p.id, "--exact", "--source", if p.source == "msstore" { "msstore" } else { "winget" }, "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"]);
            c
        }
        "brew" => {
            let mut c = Command::new(if std::path::Path::new("/opt/homebrew/bin/brew").exists() { "/opt/homebrew/bin/brew" } else { "brew" });
            c.args(["install", "--cask", &p.id]);
            c
        }
        "brew-formula" => {
            let mut c = Command::new(if std::path::Path::new("/opt/homebrew/bin/brew").exists() { "/opt/homebrew/bin/brew" } else { "brew" });
            c.args(["install", &p.id]);
            c
        }
        "mas" => {
            let mut c = Command::new("mas");
            c.args(["install", &p.id]);
            c
        }
        "dnf" => {
            let mut c = Command::new("pkexec");
            c.args(["dnf", "install", "-y", &p.id]);
            c
        }
        "apt" => {
            let mut c = Command::new("pkexec");
            c.args(["apt-get", "install", "-y", &p.id]);
            c.env("DEBIAN_FRONTEND", "noninteractive");
            c
        }
        "pacman" => {
            let mut c = Command::new("pkexec");
            c.args(["pacman", "-S", "--noconfirm", "--needed", &p.id]);
            c
        }
        "zypper" => {
            let mut c = Command::new("pkexec");
            c.args(["zypper", "--non-interactive", "install", &p.id]);
            c
        }
        "snap" => {
            let mut c = Command::new("snap");
            c.args(["install", &p.id]);
            c
        }
        other => return Err(format!("{other} is not a supported installer.")),
    };
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("The installer could not start: {e}"))?;
    let stderr = child.stderr.take();
    let err_text = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr {
            for l in BufReader::new(e).lines().map_while(Result::ok).take(80) {
                s.push_str(&l);
                s.push('\n');
            }
        }
        s
    });
    let mut last = String::new();
    if let Some(o) = child.stdout.take() {
        for l in BufReader::new(o).lines().map_while(Result::ok) {
            if !l.trim().is_empty() {
                last = l;
            }
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                break;
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_text.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else if status.code() == Some(126) || status.code() == Some(127) && p.manager != "winget" {
        Err("The install was not approved.".into())
    } else if err.contains("authoriz") || err.contains("polkit") || err.contains("Not authorized") {
        Err("This install needs administrator approval. Approve the system prompt to continue.".into())
    } else if last.contains("already installed") || err.contains("already installed") {
        Ok(())
    } else {
        let why = err.lines().rev().find(|l| !l.trim().is_empty()).or(Some(last.as_str())).unwrap_or("").trim().to_owned();
        Err(format!("{} could not be installed. {why}", p.name))
    }
}

pub fn native_manager_bin() -> String {
    for (bin, name) in [("/usr/bin/dnf5", "dnf5"), ("/usr/bin/dnf", "dnf"), ("/usr/bin/apt-get", "apt-get"), ("/usr/bin/zypper", "zypper")] {
        if std::path::Path::new(bin).exists() {
            return name.to_owned();
        }
    }
    "dnf".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_winget_tables() {
        let text = "   - \r   \\ \rName                 Id                       Version     Match       Source\n--------------------------------------------------------------------------------\nWhatsApp             9NKSQGP7F2NH             Unknown                 msstore\nVLC media player     VideoLAN.VLC             3.0.21      Tag: vlc    winget\nVisual Studio Code   Microsoft.VisualStudioCode 1.95.0                winget\n";
        let rows = parse_winget(text);
        assert_eq!(rows[0], ("WhatsApp".into(), "9NKSQGP7F2NH".into(), "msstore".into()));
        assert_eq!(rows[1].1, "VideoLAN.VLC");
        assert_eq!(rows[1].2, "winget");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].1, "Microsoft.VisualStudioCode");
    }

    #[test]
    fn knows_common_apps() {
        assert_eq!(known("vs code").map(|k| k.winget), Some("Microsoft.VisualStudioCode"));
        assert_eq!(known("the WhatsApp app").map(|k| k.msstore), Some("9NKSQGP7F2NH"));
        assert_eq!(known("mingw").map(|k| k.native), Some("gcc-c++"));
        assert!(known("some unknown tool").is_none());
        let b = best("vlc", [("vlc", "vlc"), ("vlc-core", "vlc-core"), ("vlc-plugin-gstreamer", "x")].into_iter()).unwrap();
        assert_eq!(b.1, "vlc");
    }
}
