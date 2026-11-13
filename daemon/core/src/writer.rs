use crate::core::Core;
use crate::os::{self, Node, Scope, Win};
use crate::text::{normalize, sentence_case};
use crate::tools::Outcome;
use crate::tools::apps::App;
use crate::tools::input::{self, K};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LAYER: &str = "accessibility";

fn aliases(app: &str) -> &'static [&'static str] {
    match app {
        "notepad" | "note pad" | "text editor" | "editor" | "a text editor" => {
            if cfg!(windows) {
                &["notepad", "notepad++"]
            } else if cfg!(target_os = "macos") {
                &["textedit"]
            } else {
                &["text editor", "gnome text editor", "gedit", "kate", "kwrite", "mousepad", "xed", "pluma", "featherpad"]
            }
        }
        "word" | "ms word" | "microsoft word" => &["microsoft word", "word", "libreoffice writer", "writer", "pages"],
        "notes" | "sticky notes" => &["sticky notes", "notes", "gnome notes", "notes"],
        _ => &[],
    }
}

fn resolve(core: &Core, name: &str) -> Option<App> {
    let defaults = core.defaults();
    let n = normalize(name);
    if let Some((a, _)) = core.apps.find(&n, None, &defaults).into_iter().next() {
        return Some(a);
    }
    for alt in aliases(&n) {
        if let Some((a, _)) = core.apps.find(alt, None, &defaults).into_iter().next() {
            return Some(a);
        }
    }
    if matches!(n.as_str(), "notepad" | "text editor" | "editor") {
        return core.apps.role_default("editor", &defaults);
    }
    None
}

fn words_for(app: &App) -> Vec<String> {
    let mut v = vec![normalize(&app.name)];
    let stem = std::path::Path::new(&app.path).file_stem().map(|s| normalize(&s.to_string_lossy())).unwrap_or_default();
    if !stem.is_empty() {
        v.push(stem);
    }
    let id = app.id.trim_end_matches(".desktop").rsplit('.').next().map(normalize).unwrap_or_default();
    if id.len() >= 3 {
        v.push(id);
    }
    for a in &app.aliases {
        v.push(normalize(a));
    }
    if let Some(first) = app.exec.split_whitespace().find(|w| !w.contains('=') && *w != "env" && !w.starts_with('-')) {
        let base = std::path::Path::new(first.trim_matches('"')).file_stem().map(|s| normalize(&s.to_string_lossy())).unwrap_or_default();
        if base.len() >= 3 && !matches!(base.as_str(), "flatpak" | "snap" | "python3" | "python" | "sh" | "bash") {
            v.push(base);
        }
    }
    v.retain(|x| !x.is_empty());
    v.dedup();
    v
}

pub fn window_for<'a>(wins: &'a [Win], app: &App) -> Option<&'a Win> {
    let words = words_for(app);
    let mut best: Option<(f64, &Win)> = None;
    for w in wins.iter().filter(|w| !os::is_setu(&w.app)) {
        let a = normalize(&w.app);
        let t = normalize(&w.title);
        let mut s = 0.0f64;
        for x in &words {
            s = s.max(os::score(x, &a));
            if t.ends_with(&format!(" {x}")) || t == *x {
                s = s.max(1.5);
            }
        }
        if w.front {
            s += 0.01;
        }
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, w));
        }
    }
    best.filter(|(s, _)| *s >= 0.8).map(|(_, w)| w)
}

fn editor_like(app: &App) -> bool {
    let n = normalize(&app.name);
    ["notepad", "text editor", "gedit", "textedit", "kate", "kwrite", "mousepad", "xed", "pluma", "word", "writer", "wordpad", "sticky", "onenote", "featherpad", "sublime"].iter().any(|w| n.contains(w))
}

pub fn looks_new(title: &str, app: &App) -> bool {
    let t = normalize(title);
    t.is_empty()
        || ["untitled", "new document", "document 1", "document1", "new tab", "new note", "new file", "blank document"].iter().any(|w| t.contains(w))
        || t == normalize(&app.name)
}

async fn wait_window(app: &App, secs: u64) -> Option<Win> {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if let Ok(wins) = os::windows().await
            && let Some(w) = window_for(&wins, app)
        {
            return Some(w.clone());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    None
}

async fn wait_front(win: &Win, secs: u64) -> bool {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if os::front().await.is_ok_and(|f| f.id == win.id) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

async fn target_field(win: &Win) -> Option<Node> {
    if let Ok(Some(n)) = os::focused(win).await {
        return Some(n);
    }
    let nodes = os::tree(win, Scope { limit: 600, text: false, hidden: false }).await.ok()?;
    nodes.into_iter().filter(|n| n.role == "field" && n.enabled).max_by_key(|n| (n.editable as i64, n.w as i64 * n.h as i64))
}

pub async fn write(core: &Arc<Core>, app_name: &str, text: &str) -> Outcome {
    if !core.settings.bool("permissions.control") {
        let mut o = Outcome::fail("Writing in apps needs the Control permission. Turn it on in Settings, Permissions.", LAYER);
        o.fix = Some(serde_json::json!({ "label": "Open Permissions", "method": "ui.open", "params": { "page": "settings/permissions" } }));
        return o.needs("control");
    }
    if text.trim().is_empty() {
        return Outcome::fail("What should I write?", LAYER);
    }
    let Some(app) = resolve(core, app_name) else {
        let mut o = Outcome::fail(&format!("{} isn't installed. Say install {}.", sentence_case(app_name), app_name), LAYER);
        o.fix = Some(serde_json::json!({ "label": format!("Install {}", sentence_case(app_name)), "method": "intent.run", "params": { "text": format!("install {app_name}") } }));
        return o;
    };
    let shown = app.name.clone();
    let _ = os::ready().await;
    let wins = os::windows().await.unwrap_or_default();
    let existing = window_for(&wins, &app).cloned();
    let fresh = existing.is_none();
    if fresh {
        let a = app.clone();
        if !matches!(crate::layers::blocking(move || crate::tools::apps::launch(&a)).await, Ok(Ok(_))) {
            return Outcome::fail(&format!("{shown} could not be started."), LAYER);
        }
    } else if let Some(w) = &existing
        && !w.front
    {
        let _ = crate::tools::wm::focus(&shown, Some(app.clone())).await;
    }
    let Some(mut win) = wait_window(&app, if fresh { 30 } else { 4 }).await.or(existing) else {
        return Outcome::fail(&format!("{shown} opened, but I can't see its window yet. Say type, then your text."), LAYER);
    };
    os::wake(&win).await;
    let front = wait_front(&win, if fresh { 6 } else { 3 }).await;
    if fresh && editor_like(&app) && !looks_new(&win.title, &app) && front {
        let _ = input::press(&[input::primary(), K::Ch('n')]).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        if let Some(w) = wait_window(&app, 3).await {
            win = w;
        }
    } else if fresh {
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let how = match target_field(&win).await {
        Some(n) => os::type_into(&win, &n, text, false).await,
        None if front || os::front().await.is_ok_and(|f| f.id == win.id) => input::type_text(text).await.map(|_| "keyboard"),
        None => Err(anyhow::anyhow!("I couldn't find where to type in {shown}.")),
    };
    match how {
        Ok(via) => {
            let lines = text.lines().filter(|l| !l.trim().is_empty()).count().max(1);
            let did = if lines == 1 { format!("Wrote it in {shown}") } else { format!("Wrote {lines} lines in {shown}") };
            Outcome::ok_with(&did, if via == "keyboard" { "keyboard" } else { LAYER }, Some(shown.clone())).with_reply(format!("{did}."))
        }
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, path: &str) -> App {
        App { id: path.into(), name: name.into(), source: "x".into(), path: path.into(), exec: String::new(), aliases: vec![], opened: 0, hidden: false, roles: vec![], manual: false }
    }

    fn win(app: &str, title: &str, front: bool) -> Win {
        Win { id: title.into(), app: app.into(), title: title.into(), pid: 1, front }
    }

    #[test]
    fn matches_app_windows() {
        let wins = vec![win("Setu", "Open youtube - Setu", true), win("Notepad", "Untitled - Notepad", false), win("Brave", "WhatsApp - Brave", false)];
        let notepad = app("Notepad", r"C:\Windows\notepad.exe");
        assert_eq!(window_for(&wins, &notepad).map(|w| w.app.as_str()), Some("Notepad"));
        let gedit = app("Text Editor", "/usr/share/applications/org.gnome.gedit.desktop");
        let wins2 = vec![win("gedit", "Untitled Document 1 - gedit", true)];
        assert!(window_for(&wins2, &gedit).is_some());
        let mut pad = app("Scratchpad", "/home/x/.local/share/applications/scratchpad.desktop");
        pad.exec = "gedit --new-window".into();
        assert!(window_for(&wins2, &pad).is_some());
        assert!(window_for(&wins, &app("Calculator", "calc")).is_none());
        assert!(looks_new("Untitled - Notepad", &notepad));
        assert!(!looks_new("todo.txt - Notepad", &notepad));
        assert!(editor_like(&notepad));
    }
}
