use crate::core::Core;
use crate::router::Intent;
use crate::text::{normalize, sentence_case};
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const LAYER: &str = "browser";

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub dir: String,
    pub name: String,
    pub email: String,
    pub current: bool,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

pub fn chromium_roots(key: &str) -> Vec<PathBuf> {
    let h = home();
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or(h.join(".config"));
        match key {
            "chrome" => v.extend([cfg.join("google-chrome"), h.join(".var/app/com.google.Chrome/config/google-chrome")]),
            "chromium" => v.extend([cfg.join("chromium"), h.join(".var/app/org.chromium.Chromium/config/chromium"), h.join("snap/chromium/common/chromium")]),
            "brave" => v.extend([cfg.join("BraveSoftware/Brave-Browser"), h.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser")]),
            "edge" => v.extend([cfg.join("microsoft-edge"), h.join(".var/app/com.microsoft.Edge/config/microsoft-edge")]),
            "vivaldi" => v.push(cfg.join("vivaldi")),
            "opera" => v.push(cfg.join("opera")),
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or(h.join("AppData/Local"));
        match key {
            "chrome" => v.push(local.join("Google").join("Chrome").join("User Data")),
            "chromium" => v.push(local.join("Chromium").join("User Data")),
            "brave" => v.push(local.join("BraveSoftware").join("Brave-Browser").join("User Data")),
            "edge" => v.push(local.join("Microsoft").join("Edge").join("User Data")),
            "vivaldi" => v.push(local.join("Vivaldi").join("User Data")),
            _ => {}
        }
    }
    #[cfg(target_os = "macos")]
    {
        let s = h.join("Library").join("Application Support");
        match key {
            "chrome" => v.push(s.join("Google").join("Chrome")),
            "chromium" => v.push(s.join("Chromium")),
            "brave" => v.push(s.join("BraveSoftware").join("Brave-Browser")),
            "edge" => v.push(s.join("Microsoft Edge")),
            "vivaldi" => v.push(s.join("Vivaldi")),
            _ => {}
        }
    }
    v.into_iter().filter(|p| p.join("Local State").is_file()).collect()
}

pub fn firefox_roots(key: &str) -> Vec<PathBuf> {
    let h = home();
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "linux")]
    {
        match key {
            "librewolf" => v.push(h.join(".librewolf")),
            "zen" => v.push(h.join(".zen")),
            _ => v.extend([h.join(".mozilla/firefox"), h.join(".var/app/org.mozilla.firefox/.mozilla/firefox"), h.join("snap/firefox/common/.mozilla/firefox")]),
        }
    }
    #[cfg(windows)]
    {
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(h.join("AppData/Roaming"));
        match key {
            "librewolf" => v.push(roaming.join("librewolf")),
            "zen" => v.push(roaming.join("zen")),
            _ => v.push(roaming.join("Mozilla").join("Firefox")),
        }
    }
    #[cfg(target_os = "macos")]
    {
        let s = h.join("Library").join("Application Support");
        match key {
            "librewolf" => v.push(s.join("librewolf")),
            "zen" => v.push(s.join("zen")),
            _ => v.push(s.join("Firefox")),
        }
    }
    v.into_iter().filter(|p| p.join("profiles.ini").is_file()).collect()
}

fn dir_rank(dir: &str) -> (u32, String) {
    if dir == "Default" {
        return (0, String::new());
    }
    match dir.strip_prefix("Profile ").and_then(|n| n.trim().parse::<u32>().ok()) {
        Some(n) => (n, String::new()),
        None => (u32::MAX, dir.to_owned()),
    }
}

pub fn parse_local_state(text: &str) -> Vec<Profile> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let last = v.pointer("/profile/last_used").and_then(Value::as_str).unwrap_or("Default").to_owned();
    let Some(info) = v.pointer("/profile/info_cache").and_then(Value::as_object) else { return Vec::new() };
    let mut out: Vec<Profile> = info
        .iter()
        .map(|(dir, p)| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
            let default_name = p.get("is_using_default_name").and_then(Value::as_bool).unwrap_or(false);
            let gaia = if s("gaia_given_name").is_empty() { s("gaia_name") } else { s("gaia_given_name") };
            let name = if (default_name || s("name").is_empty()) && !gaia.is_empty() { gaia } else { s("name") };
            Profile { dir: dir.clone(), name: if name.is_empty() { dir.clone() } else { name }, email: s("user_name"), current: *dir == last }
        })
        .collect();
    out.sort_by_key(|p| dir_rank(&p.dir));
    if !out.iter().any(|p| p.current)
        && let Some(f) = out.first_mut()
    {
        f.current = true;
    }
    out
}

pub fn parse_profiles_ini(text: &str) -> Vec<Profile> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            sections.push((name.to_owned(), Vec::new()));
        } else if let Some((k, v)) = l.split_once('=')
            && let Some(last) = sections.last_mut()
        {
            last.1.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let get = |kv: &[(String, String)], k: &str| kv.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone()).unwrap_or_default();
    let install_default = sections.iter().find(|(n, _)| n.starts_with("Install")).map(|(_, kv)| get(kv, "Default")).filter(|d| !d.is_empty());
    let mut out: Vec<Profile> = sections
        .iter()
        .filter(|(n, _)| n.starts_with("Profile"))
        .map(|(_, kv)| {
            let path = get(kv, "Path");
            let current = match &install_default {
                Some(d) => *d == path,
                None => get(kv, "Default") == "1",
            };
            Profile { dir: path, name: get(kv, "Name"), email: String::new(), current }
        })
        .filter(|p| !p.dir.is_empty() && !p.name.is_empty())
        .collect();
    if !out.iter().any(|p| p.current)
        && let Some(f) = out.first_mut()
    {
        f.current = true;
    }
    out
}

pub fn family(key: &str) -> &'static str {
    match key {
        "firefox" | "librewolf" | "zen" => "firefox",
        _ => "chromium",
    }
}

pub fn read(key: &str) -> (Option<PathBuf>, Vec<Profile>) {
    if family(key) == "firefox" {
        for root in firefox_roots(key) {
            if let Ok(t) = std::fs::read_to_string(root.join("profiles.ini")) {
                let v = parse_profiles_ini(&t);
                if !v.is_empty() {
                    return (Some(root), v);
                }
            }
        }
        return (None, Vec::new());
    }
    for root in chromium_roots(key) {
        if let Ok(t) = std::fs::read_to_string(root.join("Local State")) {
            let v = parse_local_state(&t);
            if !v.is_empty() {
                return (Some(root), v);
            }
        }
    }
    (None, Vec::new())
}

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.into(), args }
}

fn split_browser(t: &str) -> (String, Option<String>) {
    let (rest, b) = crate::skills::split_browser(t);
    (rest, b)
}

pub fn rules(t: &str) -> Option<Intent> {
    let p = crate::plan::clean(t);
    let (p, browser) = split_browser(&p);
    let p = p.trim().trim_end_matches(" please").to_owned();
    let b = browser.map(Value::String).unwrap_or(Value::Null);
    let whole = [
        "switch profile", "switch profiles", "switch my profile", "switch the profile", "change profile", "change my profile", "change the profile", "switch browser profile", "change browser profile",
        "switch my browser profile", "show my profiles", "show profiles", "show the profiles", "show my browser profiles", "which profiles do i have", "what profiles do i have", "list my profiles", "open profiles",
        "open my profiles", "other profile", "switch to another profile", "switch to a different profile", "use another profile", "use a different profile",
    ];
    if whole.contains(&p.as_str()) {
        return Some(act("profiles.choose", json!({ "browser": b })));
    }
    for lead in ["create a new profile called ", "create a new profile named ", "create a profile called ", "create a profile named ", "make a new profile called ", "make a new profile named ", "make a profile called ", "add a profile called ", "add a new profile called ", "create a new browser profile called ", "create a browser profile called ", "new profile called ", "new profile named "] {
        if let Some(name) = p.strip_prefix(lead) {
            let name = name.trim().trim_end_matches(" profile").trim();
            if !name.is_empty() && name.split_whitespace().count() <= 4 {
                return Some(act("profiles.create", json!({ "name": sentence_case(name), "browser": b })));
            }
        }
    }
    if matches!(p.as_str(), "create a new profile" | "create a profile" | "make a new profile" | "add a profile" | "add a new profile" | "create a new browser profile") {
        return Some(act("profiles.create", json!({ "name": "", "browser": b })));
    }
    for lead in ["switch to the ", "switch to my ", "switch to ", "open the ", "open my ", "use the ", "use my ", "go to the ", "go to my "] {
        if let Some(rest) = p.strip_prefix(lead)
            && let Some(name) = rest.strip_suffix(" profile").or_else(|| rest.strip_suffix(" browser profile"))
        {
            let name = name.trim();
            if !name.is_empty() && name.split_whitespace().count() <= 4 && !matches!(name, "a new" | "new" | "another" | "a different" | "other" | "the other") {
                return Some(act("profiles.open", json!({ "name": name, "browser": b })));
            }
        }
    }
    None
}

fn browser_app(core: &Core, name: &str) -> Option<(crate::tools::apps::App, &'static str)> {
    let app = crate::tools::web::find_browser(core, if name.trim().is_empty() { None } else { Some(name) })?;
    let key = crate::tools::browsers::key_of(&app)?;
    Some((app, key))
}

pub fn carousel_order(list: &[Profile]) -> (Vec<Profile>, usize) {
    let focus = list.iter().position(|p| p.current).unwrap_or(0);
    (list.to_vec(), focus)
}

fn locked(root: &Path, dir: &str) -> bool {
    let p = root.join(dir);
    p.join("lock").exists() || p.join(".parentlock").exists() || p.join("parent.lock").exists()
}

async fn open(core: &Arc<Core>, app: &crate::tools::apps::App, key: &str, root: Option<&Path>, p: &Profile) -> Outcome {
    let shown = crate::tools::browsers::display(key, &app.name);
    let flags: Vec<String> = if family(key) == "firefox" {
        let running_elsewhere = !crate::tools::apps::running(app).is_empty() && !root.is_some_and(|r| locked(r, &p.dir));
        let mut f = vec!["-P".to_owned(), p.name.clone()];
        if running_elsewhere {
            f.insert(0, "--new-instance".into());
        }
        f
    } else {
        vec![format!("--profile-directory={}", p.dir)]
    };
    let a = app.clone();
    match crate::layers::blocking(move || crate::tools::shell::launch_args(&a, &flags)).await {
        Ok(Ok(())) => {
            let _ = core;
            Outcome::ok_with(&format!("Opened {shown} with {}", p.name), LAYER, Some(shown.clone())).with_reply(format!("Opened {shown} with the {} profile.", p.name))
        }
        _ => Outcome::fail(&format!("{shown} could not be opened."), LAYER),
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let Some((app, key)) = browser_app(core, &s("browser")) else { return Outcome::fail("No browser is installed.", LAYER) };
    let shown = crate::tools::browsers::display(key, &app.name);
    if key == "safari" {
        return Outcome::fail("Safari keeps profiles in its own menu. Say open Safari, then use its Profiles menu.", LAYER);
    }
    let (root, list) = crate::layers::blocking(move || read(key)).await.unwrap_or((None, Vec::new()));
    match tool {
        "profiles.choose" => {
            let setu = crate::looks::summary(&core.paths.data, key);
            if list.is_empty() && setu.is_none() {
                return Outcome::ok_with("No profiles", LAYER, None).with_reply(format!("I couldn't find {shown}'s profiles. Open {shown} once, then ask again."));
            }
            if list.len() == 1 && setu.is_none() {
                return Outcome::ok_with("One profile", LAYER, None).with_reply(format!("{shown} has one profile, {}. Say create a new profile called, then a name, if you want another.", list[0].name));
            }
            let (ordered, mut focus) = carousel_order(&list);
            let mut choices: Vec<Choice> = ordered
                .iter()
                .map(|p| {
                    let mut words = vec![normalize(&p.name)];
                    if !p.email.is_empty() {
                        words.push(normalize(p.email.split('@').next().unwrap_or_default()));
                    }
                    let mut c = Choice::new(&p.name, &[], Intent::Act { tool: "profiles.open".into(), args: json!({ "dir": p.dir, "name": p.name, "browser": app.name }) });
                    c.words = words;
                    c.icon = if p.current { "current".into() } else { String::new() };
                    c
                })
                .collect();
            let mut current = list.iter().find(|p| p.current).map(|p| p.name.clone()).unwrap_or_default();
            if let Some(v) = &setu {
                let g = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
                let name = if g("name").is_empty() { "Setu profile".to_owned() } else { g("name") };
                let default = v.get("isDefault").and_then(Value::as_bool).unwrap_or(false);
                let mut c = Choice::new(&name, &[], Intent::Act { tool: "browser.setup".into(), args: json!({ "browser": app.name }) });
                c.words = vec![normalize(&name), "setu profile".into(), "the setu profile".into(), "setu".into()];
                c.icon = format!("look|{}|{}|{}", g("avatar"), g("custom"), if default { "current" } else { "" });
                if default {
                    for o in choices.iter_mut() {
                        o.icon.clear();
                    }
                    current = name.clone();
                }
                let at = (focus + 1).min(choices.len());
                choices.insert(at, c);
                if default || choices.len() == 1 {
                    focus = at;
                }
            }
            let spoken = format!("You're on {current}. Say left or right, or the name of the profile.");
            Outcome::asking("Showed the profiles", LAYER, FollowUp { question: format!("Which {shown} profile?"), spoken: Some(spoken), choices, cleanup: None, kind: "profiles", focus: Some(focus) })
        }
        "profiles.open" => {
            let dir = s("dir");
            let want = normalize(&s("name"));
            let hit = list.iter().find(|p| !dir.is_empty() && p.dir == dir).or_else(|| {
                list.iter()
                    .map(|p| (crate::os::score(&want, &normalize(&p.name)).max(crate::os::score(&want, &normalize(p.email.split('@').next().unwrap_or_default())) * 0.95), p))
                    .filter(|(sc, _)| *sc >= 0.75)
                    .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(_, p)| p)
            });
            match hit {
                Some(p) => open(core, &app, key, root.as_deref(), p).await,
                None => {
                    let names: Vec<String> = list.iter().map(|p| p.name.clone()).collect();
                    Outcome::fail(&format!("I couldn't find a profile called {}. {shown} has {}.", sentence_case(&s("name")), crate::skills::join_names_pub(&names)), LAYER)
                }
            }
        }
        "profiles.create" => create(core, &app, key, root, &list, &s("name")).await,
        other => Outcome::fail(&format!("I can't do {other} yet."), LAYER),
    }
}

async fn create(core: &Arc<Core>, app: &crate::tools::apps::App, key: &str, root: Option<PathBuf>, list: &[Profile], name: &str) -> Outcome {
    let shown = crate::tools::browsers::display(key, &app.name);
    let name = name.trim();
    if name.is_empty() {
        return Outcome::fail("What should the new profile be called? Say create a new profile called, then the name.", LAYER);
    }
    if list.iter().any(|p| normalize(&p.name) == normalize(name)) {
        return Outcome::ok_with("Profile exists", LAYER, None).with_reply(format!("{shown} already has a profile called {name}. Say switch to the {name} profile."));
    }
    if family(key) == "firefox" {
        let a = app.clone();
        let n = name.to_owned();
        let made = crate::layers::blocking(move || {
            let argv = crate::tools::apps::exec_with(&a.exec, &["-CreateProfile".to_owned(), n]);
            argv.split_first().map(|(prog, rest)| std::process::Command::new(prog).args(rest).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)).unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        if !made {
            return Outcome::fail(&format!("{shown} didn't create the profile."), LAYER);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let k = key.to_owned();
        let (root2, after) = crate::layers::blocking(move || read(&k)).await.unwrap_or((None, Vec::new()));
        return match after.iter().find(|p| normalize(&p.name) == normalize(name)) {
            Some(p) => open(core, app, key, root2.as_deref().or(root.as_deref()), p).await.with_reply(format!("Created the {name} profile in {shown} and opened it.")),
            None => Outcome::ok_with(&format!("Created the {name} profile"), LAYER, None).with_reply(format!("Created the {name} profile in {shown}.")),
        };
    }
    let next = list.iter().filter_map(|p| p.dir.strip_prefix("Profile ").and_then(|n| n.trim().parse::<u32>().ok())).max().unwrap_or(0) + 1;
    let dir = format!("Profile {next}");
    let profile = Profile { dir: dir.clone(), name: name.to_owned(), email: String::new(), current: false };
    let opened = open(core, app, key, root.as_deref(), &profile).await;
    if !opened.ok {
        return opened;
    }
    let renamed = rename_chromium(app, key, &dir, name).await;
    let line = if renamed {
        format!("Created the {name} profile in {shown} and opened it.")
    } else {
        format!("Created a new profile in {shown} and opened it. Set its name to {name} on the profile page that opened.")
    };
    Outcome::ok_with(&format!("Created the {name} profile"), LAYER, Some(shown)).with_reply(line)
}

async fn rename_chromium(app: &crate::tools::apps::App, key: &str, dir: &str, name: &str) -> bool {
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let a = app.clone();
    let flags = vec![format!("--profile-directory={dir}"), "chrome://settings/manageProfile".to_owned()];
    if !matches!(crate::layers::blocking(move || crate::tools::shell::launch_args(&a, &flags)).await, Ok(Ok(()))) {
        return false;
    }
    let _ = key;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let Some(v) = crate::browserui::view("").await else { continue };
        let page = v.page();
        let field = page.iter().find(|n| n.role == "field" && ["name", "profile name", "customize profile name", "your name"].contains(&normalize(&n.name).as_str()));
        if let Some(f) = field {
            let f = (*f).clone();
            if crate::os::type_into(&v.win, &f, name, true).await.is_ok() {
                let _ = crate::tools::input::press(&[crate::tools::input::K::Tab]).await;
                return true;
            }
            return false;
        }
    }
    false
}

pub fn label(tool: &str, args: &Value) -> String {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    match tool {
        "profiles.choose" => "Showing your profiles".into(),
        "profiles.create" => format!("Creating the {name} profile"),
        _ => format!("Opening the {name} profile"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_browser_profiles() {
        let ls = r#"{"profile":{"last_used":"Profile 2","info_cache":{
            "Default":{"name":"Anoop","user_name":"anoop@example.com"},
            "Profile 2":{"name":"Work","user_name":"anoop@work.example"},
            "Profile 10":{"name":"Person 3","is_using_default_name":true,"gaia_given_name":"Mehra"},
            "Profile 3":{"name":"College"}}}}"#;
        let v = parse_local_state(ls);
        assert_eq!(v.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Anoop", "Work", "College", "Mehra"]);
        assert_eq!(v.iter().find(|p| p.current).map(|p| p.dir.as_str()), Some("Profile 2"));
        let (order, focus) = carousel_order(&v);
        assert_eq!(order[focus].name, "Work");
        let ini = "[Install4F96D1932A9F858E]\nDefault=Profiles/abc.work\nLocked=1\n\n[Profile1]\nName=default-release\nIsRelative=1\nPath=Profiles/xyz.default-release\nDefault=1\n\n[Profile0]\nName=work\nIsRelative=1\nPath=Profiles/abc.work\n\n[General]\nStartWithLastProfile=1\n";
        let f = parse_profiles_ini(ini);
        assert_eq!(f.len(), 2);
        assert_eq!(f.iter().find(|p| p.current).map(|p| p.name.as_str()), Some("work"));
        assert!(parse_local_state("not json").is_empty());
    }

    fn tool(t: &str) -> Option<(String, Value)> {
        match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn understands_profile_requests() {
        assert_eq!(tool("switch profile").unwrap().0, "profiles.choose");
        assert_eq!(tool("show my profiles in brave").unwrap().1["browser"], "brave");
        assert_eq!(tool("switch to the work profile").unwrap(), ("profiles.open".into(), json!({ "name": "work", "browser": null })));
        assert_eq!(tool("create a new profile called college").unwrap().1["name"], "College");
        assert_eq!(tool("create a new profile").unwrap().1["name"], "");
        assert!(tool("switch to a new profile").is_none());
        assert!(tool("open firefox").is_none());
    }
}
