use crate::core::Core;
use crate::text::normalize;
use crate::tools::apps::App;
use serde_json::{Value, json};
#[cfg(windows)]
use crate::proc::Quiet;

const APP_NAMES: &[(&str, &str)] = &[
    ("google chrome", "chrome"),
    ("chrome", "chrome"),
    ("google chrome beta", "chrome"),
    ("google chrome dev", "chrome"),
    ("google chrome canary", "chrome"),
    ("microsoft edge", "edge"),
    ("microsoft edge beta", "edge"),
    ("microsoft edge dev", "edge"),
    ("edge", "edge"),
    ("brave", "brave"),
    ("brave browser", "brave"),
    ("brave web browser", "brave"),
    ("brave beta", "brave"),
    ("brave nightly", "brave"),
    ("firefox", "firefox"),
    ("mozilla firefox", "firefox"),
    ("firefox web browser", "firefox"),
    ("firefox developer edition", "firefox"),
    ("firefox nightly", "firefox"),
    ("vivaldi", "vivaldi"),
    ("opera", "opera"),
    ("opera browser", "opera"),
    ("opera gx", "opera"),
    ("opera gx browser", "opera"),
    ("chromium", "chromium"),
    ("chromium web browser", "chromium"),
    ("ungoogled chromium", "chromium"),
    ("librewolf", "librewolf"),
    ("zen", "zen"),
    ("zen browser", "zen"),
    ("arc", "arc"),
    ("safari", "safari"),
];

const SKIP: &[&str] = &["private", "incognito", "safe mode", "remote desktop", "profile", "proxy", "update", "uninstall", "setup", "installer", "help", "manual"];

pub fn key_of_app_name(name: &str) -> Option<&'static str> {
    let n = normalize(name);
    if SKIP.iter().any(|w| n.contains(w)) {
        return None;
    }
    APP_NAMES.iter().find(|(full, _)| *full == n).map(|(_, k)| *k)
}

pub fn key_of(app: &App) -> Option<&'static str> {
    if let Some(k) = key_of_app_name(&app.name) {
        return Some(k);
    }
    if !app.roles.iter().any(|r| r == "browser") {
        return None;
    }
    let n = normalize(&app.name);
    if SKIP.iter().any(|w| n.contains(w)) {
        return None;
    }
    let id = app.id.to_lowercase();
    for (k, words) in crate::tools::web::BROWSERS {
        if words.iter().any(|w| n == *w || n.starts_with(&format!("{w} "))) || id.contains(k) {
            return Some(k);
        }
    }
    None
}

pub fn family(key: &str) -> &'static str {
    match key {
        "firefox" | "librewolf" | "zen" => "firefox",
        "safari" => "safari",
        _ => "chromium",
    }
}


pub fn display(key: &str, fallback: &str) -> String {
    match key {
        "chrome" => "Chrome".into(),
        "edge" => "Edge".into(),
        "brave" => "Brave".into(),
        "firefox" => "Firefox".into(),
        "vivaldi" => "Vivaldi".into(),
        "opera" => "Opera".into(),
        "chromium" => "Chromium".into(),
        "librewolf" => "LibreWolf".into(),
        "zen" => "Zen".into(),
        "arc" => "Arc".into(),
        "safari" => "Safari".into(),
        _ => fallback.trim_end_matches(" Web Browser").trim_end_matches(" Browser").to_owned(),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn key_of_progid(progid: &str) -> Option<&'static str> {
    let p = progid.to_lowercase();
    [("msedge", "edge"), ("brave", "brave"), ("chromium", "chromium"), ("chrome", "chrome"), ("firefox", "firefox"), ("librewolf", "librewolf"), ("vivaldi", "vivaldi"), ("opera", "opera"), ("zen", "zen")]
        .iter()
        .find(|(needle, _)| p.contains(needle))
        .map(|(_, k)| *k)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn key_of_bundle(bundle: &str) -> Option<&'static str> {
    let b = bundle.to_lowercase();
    [
        ("com.google.chrome", "chrome"),
        ("com.microsoft.edgemac", "edge"),
        ("com.brave.browser", "brave"),
        ("org.mozilla.firefox", "firefox"),
        ("org.mozilla.firefoxdeveloperedition", "firefox"),
        ("org.mozilla.nightly", "firefox"),
        ("com.vivaldi.vivaldi", "vivaldi"),
        ("com.operasoftware.opera", "opera"),
        ("org.chromium.chromium", "chromium"),
        ("company.thebrowser.browser", "arc"),
        ("com.apple.safari", "safari"),
        ("app.zen-browser.zen", "zen"),
        ("org.mozilla.librewolf", "librewolf"),
        ("io.gitlab.librewolf-community", "librewolf"),
    ]
    .iter()
    .find(|(id, _)| b == *id || b.starts_with(&format!("{id}.")))
    .map(|(_, k)| *k)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn https_handler(lshandlers: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut role: Option<String> = None;
    let mut https = false;
    for line in lshandlers.lines() {
        let t = line.trim();
        if depth == 1 {
            if let Some(rest) = t.strip_prefix("LSHandlerRoleAll = ") {
                role = Some(rest.trim_end_matches(';').trim_matches('"').to_owned());
            }
            if t == "LSHandlerURLScheme = https;" {
                https = true;
            }
        }
        let opens = t.matches('{').count();
        let closes = t.matches('}').count();
        depth = (depth + opens).saturating_sub(closes);
        if closes > 0 && depth == 0 {
            if https && role.is_some() {
                return role;
            }
            role = None;
            https = false;
        }
    }
    None
}

pub fn system_default() -> Option<&'static str> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Option<&'static str>)>> = Mutex::new(None);
    if let Some((at, v)) = *CACHE.lock().unwrap()
        && at.elapsed() < Duration::from_secs(30)
    {
        return v;
    }
    let v = query_default();
    *CACHE.lock().unwrap() = Some((Instant::now(), v));
    v
}

fn query_default() -> Option<&'static str> {
    #[cfg(windows)]
    {
        let o = std::process::Command::new("reg")
            .quiet()
            .args(["query", r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "/v", "ProgId"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&o.stdout).into_owned();
        let line = text.lines().find(|l| l.contains("ProgId"))?;
        let progid = line.split_whitespace().last()?;
        key_of_progid(progid)
    }
    #[cfg(target_os = "macos")]
    {
        let o = std::process::Command::new("defaults").args(["read", "com.apple.LaunchServices/com.apple.launchservices.secure", "LSHandlers"]).output().ok()?;
        let bundle = https_handler(&String::from_utf8_lossy(&o.stdout))?;
        key_of_bundle(&bundle)
    }
    #[cfg(target_os = "linux")]
    {
        None
    }
}

pub fn icon_of(app: &App) -> String {
    #[cfg(target_os = "linux")]
    {
        if app.path.ends_with(".desktop")
            && let Ok(text) = std::fs::read_to_string(&app.path)
        {
            let mut entry = false;
            for l in text.lines() {
                let l = l.trim();
                if l.starts_with('[') {
                    entry = l == "[Desktop Entry]";
                    continue;
                }
                if entry && let Some(v) = l.strip_prefix("Icon=") {
                    return v.trim().to_owned();
                }
            }
        }
        String::new()
    }
    #[cfg(not(target_os = "linux"))]
    {
        app.path.clone()
    }
}


#[derive(Debug, Clone)]
pub struct Installed {
    pub key: &'static str,
    pub app: App,
}

pub fn installed(core: &Core) -> Vec<Installed> {
    let mut best: Vec<Installed> = Vec::new();
    for a in core.apps.all().into_iter().filter(|a| !a.hidden) {
        let Some(k) = key_of(&a) else { continue };
        match best.iter_mut().find(|b| b.key == k) {
            Some(b) => {
                let shorter = a.name.len() < b.app.name.len();
                if a.opened > b.app.opened || (a.opened == b.app.opened && shorter) {
                    b.app = a;
                }
            }
            None => best.push(Installed { key: k, app: a }),
        }
    }
    best.sort_by(|a, b| b.app.opened.cmp(&a.app.opened).then(a.app.name.cmp(&b.app.name)));
    best
}

pub fn chosen(core: &Core) -> Option<String> {
    let id = core.settings.str("web.browser");
    (!id.is_empty() && core.apps.get(&id).is_some()).then_some(id)
}

pub fn list(core: &Core) -> Value {
    let all = installed(core);
    let pick = chosen(core);
    let system = system_default();
    let items: Vec<Value> = all
        .iter()
        .map(|b| {
            json!({
                "id": b.app.id,
                "key": b.key,
                "name": display(b.key, &b.app.name),
                "icon": icon_of(&b.app),
                "chosen": pick.as_deref() == Some(b.app.id.as_str()),
                "system": system == Some(b.key),
                "family": family(b.key),
                "connected": pick.as_deref() == Some(b.app.id.as_str()),
                "setuProfile": crate::looks::summary(&core.paths.data, b.key),
            })
        })
        .collect();
    json!({ "items": items, "chosen": pick, "needsChoice": pick.is_none() && all.len() > 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> App {
        App { id: id.into(), name: name.into(), source: "start_menu".into(), path: id.into(), exec: String::new(), aliases: vec![], opened: 0, hidden: false, roles: vec![], manual: false }
    }

    #[test]
    fn recognises_browsers_on_every_system() {
        assert_eq!(key_of(&app("Google Chrome", r"C:\x\Google Chrome.lnk")), Some("chrome"));
        assert_eq!(key_of(&app("Microsoft Edge", r"C:\x\Microsoft Edge.lnk")), Some("edge"));
        assert_eq!(key_of(&app("Brave", r"C:\x\Brave.lnk")), Some("brave"));
        assert_eq!(key_of(&app("Brave Browser", "/Applications/Brave Browser.app")), Some("brave"));
        assert_eq!(key_of(&app("Firefox Private Browsing", r"C:\x\Firefox Private Browsing.lnk")), None);
        assert_eq!(key_of(&app("Chrome Remote Desktop", "x")), None);
        assert_eq!(key_of(&app("Opera GX Browser", "x")), Some("opera"));
        assert_eq!(key_of(&app("Notepad", "x")), None);
        let mut f = app("Firefox Web Browser", "firefox.desktop");
        f.roles = vec!["browser".into()];
        assert_eq!(key_of(&f), Some("firefox"));
        assert_eq!(key_of_progid("ChromeHTML"), Some("chrome"));
        assert_eq!(key_of_progid("MSEdgeHTM"), Some("edge"));
        assert_eq!(key_of_progid("FirefoxURL-308046B0AF4A39CB"), Some("firefox"));
        assert_eq!(key_of_progid("BraveHTML"), Some("brave"));
        assert_eq!(key_of_bundle("com.brave.Browser"), Some("brave"));
        let ls = "(\n    {\n        LSHandlerRoleAll = \"com.apple.mail\";\n        LSHandlerURLScheme = mailto;\n    },\n    {\n        LSHandlerPreferredVersions =         {\n            LSHandlerRoleAll = \"-\";\n        };\n        LSHandlerRoleAll = \"com.brave.browser\";\n        LSHandlerURLScheme = https;\n    }\n)";
        assert_eq!(https_handler(ls).as_deref(), Some("com.brave.browser"));
        assert_eq!(family("zen"), "firefox");
    }
}
