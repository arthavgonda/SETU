use crate::os::{self, Do, Node, Scope, Win};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Problem(String),
    Licence { accept: Node, text: String },
    Press(Node),
    Untick(Node),
    Choose(Node),
    Folder(Node),
    Busy(Option<f64>),
    Waiting,
}

#[derive(Debug, Clone, Default)]
pub struct Opts {
    pub pid: u32,
    pub title: String,
    pub app: String,
    pub folder: Option<String>,
    pub consent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum End {
    Done,
    NeedsConsent(String),
    Failed(String),
    Cancelled,
}

fn clean(s: &str) -> String {
    let t: String = s.chars().filter(|c| !matches!(c, '&' | '_' | '>' | '<' | '…' | '.' | ':' | '!')).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

const FORWARD: &[(&str, u8)] = &[("install", 0), ("install now", 0), ("upgrade", 1), ("update", 1), ("next", 2), ("continue", 3), ("finish", 4), ("done", 4), ("close", 5), ("ok", 6)];
const DONE_WORDS: &[&str] = &["complet", "finished", "successfully", "has been installed", "is now installed", "installation is done", "was installed", "is installed"];
const ACCEPT_BUTTONS: &[&str] = &["i agree", "i accept", "accept", "agree", "accept and install", "agree and install", "accept & install"];
const DECLINE_OFFER: &[&str] = &["decline", "skip", "no thanks", "skip all", "don't install", "do not install"];
const EXTRAS: &[&str] = &["toolbar", "home page", "homepage", "start page", "search engine", "default search", "search provider", "also install", "recommended software", "special offer", "browser extension", "mcafee", "norton", "avast", "avg ", "opera", "yandex", "bing", "ask.com", "web companion", "driver updater", "pc cleaner", "optimizer"];
const ERRORS: &[&str] = &["error", "failed", "could not", "couldn't", "cannot", "can't be installed", "not enough space", "unable to"];

fn has_any(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

fn page_text(nodes: &[Node]) -> String {
    nodes.iter().filter(|n| matches!(n.role.as_str(), "text" | "heading" | "label" | "document")).map(|n| if n.name.trim().is_empty() { n.value.trim() } else { n.name.trim() }).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

fn licence_page(text: &str) -> bool {
    has_any(text, &["license", "licence", "terms of", "terms and conditions", "eula", "agreement"])
}

fn offer_page(text: &str) -> bool {
    has_any(text, &["offer", "toolbar", "home page", "homepage", "search engine", "recommended", "partner", "sponsored"]) && !licence_page(text)
}

fn button(nodes: &[Node], words: &[&str]) -> Option<Node> {
    nodes.iter().find(|n| n.role == "button" && n.enabled && words.contains(&clean(&n.name).as_str())).cloned()
}

pub fn percent(value: &str) -> Option<f64> {
    let v = value.trim();
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f64>().ok().filter(|x| (0.0..=100.0).contains(x));
    }
    let x: f64 = v.parse().ok()?;
    if (0.0..=1.0).contains(&x) && v.contains('.') {
        Some(x * 100.0)
    } else {
        (0.0..=100.0).contains(&x).then_some(x)
    }
}

pub fn classify(nodes: &[Node], consent: bool, folder: Option<&str>) -> Step {
    let text = clean(&page_text(nodes));
    let enabled_buttons: Vec<&Node> = nodes.iter().filter(|n| n.role == "button" && n.enabled).collect();
    let done = has_any(&text, DONE_WORDS);
    let forward = enabled_buttons
        .iter()
        .filter_map(|n| {
            let w = clean(&n.name);
            let rank = FORWARD.iter().find(|(f, _)| *f == w)?.1;
            (w != "close" || done).then(|| (rank, (*n).clone()))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, n)| n);
    let only_ok = enabled_buttons.iter().all(|n| matches!(clean(&n.name).as_str(), "ok" | "retry" | "cancel" | "abort" | "ignore" | "close"));
    if has_any(&text, ERRORS) && only_ok && !enabled_buttons.is_empty() {
        return Step::Problem(page_text(nodes));
    }
    for n in nodes {
        let name = clean(&n.name);
        let accepting = (name.contains("accept") || name.contains("agree")) && !has_any(&name, &["not ", "don't", "do not", "decline", "disagree"]);
        if accepting && matches!(n.role.as_str(), "radio" | "checkbox" | "toggle") && n.checked == Some(false) && n.enabled {
            return Step::Licence { accept: n.clone(), text: page_text(nodes) };
        }
    }
    if licence_page(&text)
        && let Some(b) = button(nodes, ACCEPT_BUTTONS)
    {
        return if consent { Step::Press(b) } else { Step::Licence { accept: b, text: page_text(nodes) } };
    }
    if offer_page(&text)
        && let Some(b) = button(nodes, DECLINE_OFFER)
    {
        return Step::Press(b);
    }
    for n in nodes {
        if matches!(n.role.as_str(), "checkbox" | "toggle") && n.checked == Some(true) && n.enabled && has_any(&clean(&n.name), EXTRAS) {
            return Step::Untick(n.clone());
        }
    }
    if let Some(want) = folder.filter(|f| !f.trim().is_empty()) {
        let pathish = |v: &str| v.contains('/') || v.contains('\\') || (v.len() > 2 && v.as_bytes()[1] == b':');
        if let Some(f) = nodes.iter().find(|n| n.role == "field" && n.editable && n.enabled && pathish(&n.value) && n.value.trim() != want.trim()) {
            return Step::Folder(f.clone());
        }
    }
    if has_any(&text, &["restart", "reboot"]) {
        for n in nodes {
            let name = clean(&n.name);
            if n.role == "radio" && n.checked == Some(false) && n.enabled && has_any(&name, &["later", "manually", "not restart", "don't restart", "do not restart"]) {
                return Step::Choose(n.clone());
            }
        }
    }
    let finishing = forward.as_ref().is_some_and(|b| matches!(clean(&b.name).as_str(), "finish" | "done" | "close"));
    if finishing {
        for n in nodes {
            let name = clean(&n.name);
            let launch = ["launch", "run ", "start ", "open ", "view readme", "show readme", "view the readme"].iter().any(|w| name.starts_with(w) || name.contains(&format!(" {w}")));
            if matches!(n.role.as_str(), "checkbox" | "toggle") && n.checked == Some(true) && n.enabled && launch {
                return Step::Untick(n.clone());
            }
        }
    }
    if let Some(b) = forward {
        return Step::Press(b);
    }
    if let Some(p) = nodes.iter().find(|n| n.role == "progress") {
        return Step::Busy(percent(&p.value));
    }
    Step::Waiting
}

pub fn app_from_title(title: &str) -> String {
    let t = title.trim();
    let t = t.strip_prefix("Setup - ").or_else(|| t.strip_prefix("Install ")).unwrap_or(t);
    let lower = t.to_lowercase();
    let cut = [" setup wizard", " installation wizard", " install wizard", " setup", " installer", " installation", " install"].iter().find_map(|s| lower.strip_suffix(s).map(|r| r.len())).unwrap_or(t.len());
    let name = t[..cut].trim().trim_end_matches(['-', ':']).trim();
    if name.is_empty() { "The app".to_owned() } else { name.to_owned() }
}

fn setupish(title: &str) -> bool {
    let t = title.to_lowercase();
    ["setup", "install", "wizard"].iter().any(|w| t.contains(w))
}

async fn find(opts: &Opts) -> Option<Win> {
    let wins = os::windows().await.ok()?;
    let mine = |w: &&Win| !os::is_setu_window(w);
    if opts.pid != 0
        && let Some(w) = wins.iter().filter(mine).find(|w| w.pid == opts.pid)
    {
        return Some(w.clone());
    }
    if !opts.title.is_empty()
        && let Some(w) = wins.iter().filter(mine).find(|w| w.title == opts.title)
    {
        return Some(w.clone());
    }
    let app = opts.app.to_lowercase();
    wins.iter().filter(mine).find(|w| setupish(&w.title) && (app.is_empty() || w.title.to_lowercase().contains(&app))).cloned()
}

async fn read_nodes(win: &Win, remote: bool) -> Option<Vec<Node>> {
    if !remote {
        return os::tree(win, Scope { limit: 800, text: true, hidden: false }).await.ok();
    }
    let v = crate::privileged::call("setup.window", serde_json::json!({ "pid": win.pid, "op": "read" }), Duration::from_secs(25)).await.ok()?;
    serde_json::from_value::<Vec<Node>>(v.get("nodes")?.clone()).ok()
}

async fn press(n: &Node, how: &str, value: &str, pid: u32, remote: bool) -> bool {
    if remote {
        let Some(i) = n.id.rsplit(':').next().and_then(|i| i.parse::<u64>().ok()) else { return false };
        return crate::privileged::call("setup.window", serde_json::json!({ "pid": pid, "op": how, "index": i, "name": n.name, "value": value }), Duration::from_secs(25)).await.is_ok();
    }
    let what = match how {
        "select" => Do::Select,
        "toggle" => Do::Toggle,
        "set" => Do::SetText(value.to_owned()),
        _ => Do::Invoke,
    };
    if os::act(n, what).await.is_ok() {
        return true;
    }
    matches!(how, "select" | "toggle") && os::act(n, Do::Invoke).await.is_ok()
}

fn fingerprint(nodes: &[Node]) -> String {
    nodes.iter().filter(|n| n.role != "progress").map(|n| format!("{}:{}:{}:{:?}", n.role, n.name, n.enabled, n.checked)).collect::<Vec<_>>().join("|")
}

pub async fn drive(opts: Opts, cancel: Arc<AtomicBool>, progress: impl Fn(Option<f64>) + Send) -> End {
    let started = Instant::now();
    let mut seen = false;
    let mut gone_since: Option<Instant> = None;
    let mut last = String::new();
    let mut repeats = 0;
    let mut pressed_finish = false;
    let mut done_seen = false;
    let mut last_pct = 0.0f64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return End::Cancelled;
        }
        if started.elapsed() > Duration::from_secs(45 * 60) {
            return End::Failed("The setup has been running for 45 minutes, so I stopped watching it.".into());
        }
        let Some(win) = find(&opts).await else {
            log::debug!("setup window not found (seen before: {seen})");
            if seen {
                let since = *gone_since.get_or_insert_with(Instant::now);
                if pressed_finish {
                    return End::Done;
                }
                if since.elapsed() > Duration::from_secs(4) {
                    return if done_seen || last_pct >= 99.0 { End::Done } else { End::Failed(format!("The {} setup closed before it finished, so I can't tell whether it was installed.", if opts.app.is_empty() { "app's" } else { opts.app.as_str() })) };
                }
            } else if started.elapsed() > Duration::from_secs(40) {
                return End::Failed("The setup window didn't open.".into());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        };
        seen = true;
        gone_since = None;
        let remote = os::admin_only(win.pid);
        if remote && !crate::privileged::installed() {
            return End::Failed("This setup runs as administrator, so I need admin control to go through it. Say set up admin control, then ask me again.".into());
        }
        let Some(nodes) = read_nodes(&win, remote).await else {
            tokio::time::sleep(Duration::from_millis(400)).await;
            continue;
        };
        let step = classify(&nodes, opts.consent, opts.folder.as_deref());
        if has_any(&clean(&page_text(&nodes)), DONE_WORDS) {
            done_seen = true;
        }
        if let Step::Busy(Some(p)) = step {
            last_pct = last_pct.max(p);
        }
        let fp = format!("{}#{:?}", fingerprint(&nodes), step);
        if fp == last && !matches!(step, Step::Busy(_) | Step::Waiting) {
            repeats += 1;
            if repeats >= 5 {
                let said = page_text(&nodes);
                return End::Failed(if said.is_empty() { "The setup isn't moving on, so I stopped. It may need you to look at it.".into() } else { format!("The setup isn't moving on. It says: {}", said.chars().take(300).collect::<String>()) });
            }
        } else {
            repeats = 0;
        }
        last = fp;
        log::info!("setup step: {}", match &step {
            Step::Problem(_) => "problem".to_owned(),
            Step::Licence { accept, .. } => format!("licence ({})", accept.name),
            Step::Press(b) => format!("press {}", b.name),
            Step::Untick(n) => format!("untick {}", n.name),
            Step::Choose(n) => format!("choose {}", n.name),
            Step::Folder(_) => "folder".to_owned(),
            Step::Busy(p) => format!("busy {p:?}"),
            Step::Waiting => "waiting".to_owned(),
        });
        match step {
            Step::Problem(t) => return End::Failed(format!("The setup reported a problem: {}", t.chars().take(300).collect::<String>())),
            Step::Licence { text, .. } if !opts.consent => return End::NeedsConsent(text),
            Step::Licence { accept, .. } => {
                let how = match accept.role.as_str() {
                    "radio" => "select",
                    "checkbox" | "toggle" => "toggle",
                    _ => "invoke",
                };
                press(&accept, how, "", win.pid, remote).await;
            }
            Step::Untick(n) => {
                press(&n, "toggle", "", win.pid, remote).await;
            }
            Step::Choose(n) => {
                press(&n, "select", "", win.pid, remote).await;
            }
            Step::Folder(f) => {
                press(&f, "set", opts.folder.as_deref().unwrap_or_default(), win.pid, remote).await;
            }
            Step::Press(b) => {
                if matches!(clean(&b.name).as_str(), "finish" | "done" | "close") {
                    pressed_finish = true;
                    progress(Some(100.0));
                }
                press(&b, "invoke", "", win.pid, remote).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Step::Busy(p) => progress(p),
            Step::Waiting => {}
        }
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(role: &str, name: &str) -> Node {
        Node { role: role.into(), name: name.into(), enabled: true, ..Default::default() }
    }

    fn check(role: &str, name: &str, on: bool) -> Node {
        Node { checked: Some(on), ..n(role, name) }
    }

    #[test]
    fn welcome_page_presses_next_never_cancel() {
        let page = vec![n("heading", "Welcome to the Bench App Setup Wizard"), n("button", "< Back"), n("button", "&Next >"), n("button", "Cancel")];
        assert_eq!(classify(&page, false, None), Step::Press(n("button", "&Next >")));
    }

    #[test]
    fn a_licence_waits_for_the_user_and_is_accepted_only_after_a_yes() {
        let page = vec![n("heading", "License Agreement"), check("radio", "I accept the agreement", false), check("radio", "I do not accept the agreement", true), Node { enabled: false, ..n("button", "Next") }, n("button", "Cancel")];
        assert!(matches!(classify(&page, false, None), Step::Licence { .. }));
        assert!(matches!(classify(&page, true, None), Step::Licence { accept, .. } if accept.name == "I accept the agreement"));
        let nsis = vec![n("text", "Please review the license terms before installing Bench App."), n("button", "I Agree"), n("button", "Cancel")];
        assert!(matches!(classify(&nsis, false, None), Step::Licence { .. }));
        assert_eq!(classify(&nsis, true, None), Step::Press(n("button", "I Agree")));
    }

    #[test]
    fn bundled_extras_are_unticked_and_offers_declined() {
        let page = vec![check("checkbox", "Create a desktop shortcut", true), check("checkbox", "Also install the Bench Toolbar and make Bench Search my home page", true), n("button", "Next")];
        assert!(matches!(classify(&page, false, None), Step::Untick(x) if x.name.contains("Toolbar")));
        let offer = vec![n("text", "Recommended: install Speedy PC Optimizer, a special offer from our partner"), n("button", "Accept"), n("button", "Decline")];
        assert_eq!(classify(&offer, false, None), Step::Press(n("button", "Decline")));
    }

    #[test]
    fn the_chosen_folder_is_typed_and_launch_boxes_cleared_at_the_end() {
        let field = Node { value: "C:\\Program Files\\Bench App".into(), editable: true, ..n("field", "Destination folder") };
        let page = vec![n("text", "Select Destination Location"), field.clone(), n("button", "Browse..."), n("button", "Next >")];
        assert_eq!(classify(&page, false, Some("D:\\Games\\Bench App")), Step::Folder(field));
        assert_eq!(classify(&page, false, None), Step::Press(n("button", "Next >")));
        let end = vec![n("heading", "Completing the Bench App Setup Wizard"), check("checkbox", "Launch Bench App", true), n("button", "Finish")];
        assert!(matches!(classify(&end, false, None), Step::Untick(x) if x.name == "Launch Bench App"));
    }

    #[test]
    fn progress_and_errors_are_read_not_pressed() {
        let busy = vec![n("button", "Close"), n("text", "Installing Bench App"), Node { value: "42%".into(), ..n("progress", "") }, Node { enabled: false, ..n("button", "Next") }, n("button", "Cancel")];
        assert_eq!(classify(&busy, false, None), Step::Busy(Some(42.0)), "the window's own close button is never pressed mid-install");
        let finished = vec![n("button", "Close"), n("text", "Bench App was installed successfully.")];
        assert_eq!(classify(&finished, false, None), Step::Press(n("button", "Close")));
        let restart = vec![n("text", "Setup must restart your computer to finish."), check("radio", "Yes, restart the computer now", true), check("radio", "No, I will restart the computer later", false), n("button", "Yes"), n("button", "Finish")];
        assert!(matches!(classify(&restart, false, None), Step::Choose(x) if x.name.contains("later")));
        let ask = vec![n("text", "Restart now?"), n("button", "Yes"), n("button", "No")];
        assert_eq!(classify(&ask, false, None), Step::Waiting, "a yes that would restart the computer is never pressed");
        let err = vec![n("text", "Error: not enough space on drive C:"), n("button", "OK")];
        assert!(matches!(classify(&err, false, None), Step::Problem(_)));
        assert_eq!(percent("0.25"), Some(25.0));
        assert_eq!(percent("57"), Some(57.0));
    }

    #[test]
    fn app_names_come_from_setup_titles() {
        assert_eq!(app_from_title("Setup - Bench App"), "Bench App");
        assert_eq!(app_from_title("Bench App Setup"), "Bench App");
        assert_eq!(app_from_title("Steam Installer"), "Steam");
        assert_eq!(app_from_title("Install Zoom"), "Zoom");
    }
}
