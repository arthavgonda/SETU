use crate::core::Core;
use crate::os::{self, Win};
use crate::router::Intent;
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LAYER: &str = "accessibility";

static SHOWN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn first_time(key: &str) -> bool {
    let mut g = SHOWN.lock().unwrap();
    g.get_or_insert_with(HashSet::new).insert(key.to_owned())
}

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.to_owned(), args }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cause {
    Permission,
    Asleep { app: String, browser: bool },
    Admin { app: String },
    Java { app: String, fixable: bool },
}

pub async fn java_cause(target: Option<&Win>) -> Option<Cause> {
    if let Some(w) = target {
        if os::java_state(w) != Some(false) {
            return None;
        }
        let app = crate::ide::of_window(w).map(|i| i.name.to_owned()).unwrap_or_else(|| w.app.clone());
        return Some(Cause::Java { app, fixable: true });
    }
    let running = crate::ide::java_ides();
    if running.is_empty() {
        return None;
    }
    let wins = os::windows().await.unwrap_or_default();
    running.into_iter().find(|(_, pid)| !wins.iter().any(|w| w.pid == *pid)).map(|(i, _)| Cause::Java { app: i.name.to_owned(), fixable: false })
}

pub async fn cause(target: Option<&Win>, browser_only: bool) -> Option<Cause> {
    if !os::trusted() {
        return Some(Cause::Permission);
    }
    if let Some(w) = target
        && os::admin_only(w.pid)
    {
        return Some(Cause::Admin { app: w.app.clone() });
    }
    let sleepers = os::asleep().await;
    let pick = sleepers.into_iter().find(|(_, _, browser, pid)| match target {
        Some(w) => w.pid != 0 && w.pid == *pid,
        None => !browser_only || *browser,
    });
    pick.map(|(app, _, browser, _)| Cause::Asleep { app, browser })
}

pub fn outcome(c: &Cause) -> Outcome {
    match c {
        Cause::Permission => {
            let line = "I need the Accessibility permission to see and use your apps.";
            if !first_time("permission") {
                return Outcome::fail(&format!("{line} Turn on Setu in System Settings, Privacy & Security, Accessibility."), LAYER).needs("accessibility");
            }
            let choices = vec![Choice::new("Open Settings", &["open settings", "open", "yes", "settings", "allow"], act("access.settings", json!({}))), Choice::new("Not now", &["not now", "no", "later", "cancel"], act("access.later", json!({})))];
            Outcome::asking("Asked for Accessibility", LAYER, FollowUp { question: line.into(), spoken: Some(format!("{line} Shall I open the setting?")), choices, cleanup: None, kind: "", focus: None })
        }
        Cause::Asleep { app, browser } => {
            let line = format!("{app} started without accessibility, so I can't see inside it.");
            if !first_time(&format!("asleep:{app}")) {
                let tail = if *browser { format!("Say restart {}, and your tabs will come back.", app.to_lowercase()) } else { format!("Close {app} and open it again, and I'll see it.") };
                return Outcome::fail(&format!("{line} {tail}"), LAYER);
            }
            let no = Choice::new("Not now", &["not now", "no", "later", "cancel"], act("access.later", json!({})));
            if *browser {
                let choices = vec![Choice::new(&format!("Restart {app}"), &["restart", "yes", "restart it", &format!("restart {}", app.to_lowercase())], act("access.restart", json!({ "app": app }))), no];
                return Outcome::asking(
                    "Asked to restart",
                    LAYER,
                    FollowUp { question: format!("{line} Restart {app}? Your tabs come back, and it will start ready for me from now on."), spoken: Some(format!("{line} Shall I restart it? Your tabs will come back.")), choices, cleanup: None, kind: "", focus: None },
                );
            }
            let choices = vec![Choice::new("Set it up", &["set it up", "yes", "okay", "do it"], act("access.ready", json!({ "app": app }))), no];
            Outcome::asking(
                "Asked to set up",
                LAYER,
                FollowUp { question: format!("{line} Shall I set {app} to start ready for me? Then close it and open it again once."), spoken: Some(format!("{line} Shall I set it up for next time?")), choices, cleanup: None, kind: "", focus: None },
            )
        }
        Cause::Java { app, fixable: true } => {
            let line = format!("{app} needs the Java Access Bridge before I can see inside it.");
            if !first_time(&format!("java:{app}")) {
                return Outcome::fail(&format!("{line} Say turn on the Java bridge, then open {app} again."), LAYER);
            }
            let choices = vec![
                Choice::new("Turn it on", &["turn it on", "yes", "okay", "do it", "turn on the java bridge"], act("access.java", json!({ "app": app }))),
                Choice::new("Not now", &["not now", "no", "later", "cancel"], act("access.later", json!({}))),
            ];
            Outcome::asking(
                "Asked to turn on the Java bridge",
                LAYER,
                FollowUp { question: format!("{line} Shall I turn it on? Then close {app} and open it again once."), spoken: Some(format!("{line} Shall I turn it on?")), choices, cleanup: None, kind: "", focus: None },
            )
        }
        Cause::Java { app, fixable: false } => {
            let line = format!("{app} is a Java app. On Linux, Java apps need the Java accessibility bridge, called java-atk-wrapper, before I can see inside them, and it isn't installed. I can still open files and lines in {app} and type for you.");
            if first_time(&format!("java:{app}")) { Outcome::ok_with(&format!("Can't see inside {app}"), LAYER, Some(app.clone())).with_reply(line) } else { Outcome::fail(&format!("I can't see inside {app}. It needs the Java accessibility bridge."), LAYER) }
        }
        Cause::Admin { app } => {
            let line = format!("{app} runs as administrator, so Windows doesn't let me use it. If Setu also runs as administrator, I can.");
            if first_time(&format!("admin:{app}")) { Outcome::ok_with("Can't use an administrator app", LAYER, Some(app.clone())).with_reply(line) } else { Outcome::fail(&line, LAYER) }
        }
    }
}

pub async fn explain(_core: &Arc<Core>, target: Option<&Win>) -> Option<Outcome> {
    cause(target, false).await.map(|c| outcome(&c))
}

pub async fn explain_browser(_core: &Arc<Core>) -> Option<Outcome> {
    cause(None, true).await.map(|c| outcome(&c))
}

pub fn rules(t: &str) -> Option<Intent> {
    if matches!(t, "turn on the java bridge" | "turn on java bridge" | "turn on the java access bridge" | "enable the java access bridge" | "enable java access bridge" | "turn on java access bridge") {
        return Some(act("access.java", json!({ "app": "your Java app" })));
    }
    let rest = t.strip_prefix("restart ")?.trim_start_matches("the ").trim_start_matches("my ").trim();
    let name = rest.trim_end_matches(" browser").trim();
    let key = crate::tools::web::browser_key(name)?;
    let display = crate::tools::media::browser_display(key);
    Some(act("access.restart", json!({ "app": display })))
}

fn find_app(core: &Arc<Core>, app: &str) -> Option<crate::tools::apps::App> {
    if let Some(key) = crate::tools::web::browser_key(app)
        && let Some(b) = crate::tools::web::find_browser(core, Some(key))
    {
        return Some(b);
    }
    crate::tools::web::find_browser(core, Some(app)).or_else(|| core.apps.find(app, None, &core.defaults()).into_iter().next().map(|(a, _)| a))
}

async fn make_ready(app: &crate::tools::apps::App) -> bool {
    #[cfg(target_os = "linux")]
    {
        let a = app.clone();
        let editor = crate::ide::spoken(&app.name).filter(|i| i.family == crate::ide::Family::VsCode);
        matches!(
            crate::layers::blocking(move || {
                let argv = editor.map(|i| crate::ide::make_argv_ready(i).unwrap_or(false)).unwrap_or(false);
                crate::tools::apps::make_ready(&a).map(|l| l || argv)
            })
            .await,
            Ok(Ok(true))
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        false
    }
}

async fn restart(core: &Arc<Core>, app: &str) -> Outcome {
    let Some(found) = find_app(core, app) else {
        return Outcome::fail(&format!("I couldn't find {app} to restart it."), LAYER);
    };
    let key = crate::tools::web::browser_key(app).map(str::to_owned).unwrap_or_else(|| app.to_owned());
    let _ = os::ready().await;
    let ready = make_ready(&found).await;
    if !os::quit_asleep(app).await {
        return Outcome::fail(&format!("{app} didn't close, so I couldn't restart it."), LAYER);
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    let a = found.clone();
    let started = crate::layers::blocking(move || crate::tools::shell::launch_args(&a, &["--restore-last-session".to_owned()])).await;
    if !matches!(started, Ok(Ok(()))) {
        return Outcome::fail(&format!("{app} closed but didn't start again."), LAYER);
    }
    let until = Instant::now() + Duration::from_secs(25);
    let low = app.to_lowercase();
    while Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let still = os::asleep().await.iter().any(|(n, _, _, _)| n.to_lowercase().contains(&low) || low.contains(&n.to_lowercase()));
        if !still
            && let Some(v) = crate::browserui::view(&key).await
            && !v.nodes.is_empty()
        {
            let tail = if ready { " It will start ready for me from now on." } else { "" };
            return Outcome::ok_with(&format!("Restarted {app}"), LAYER, Some(app.to_owned())).with_reply(format!("{app} is back, and now I can see it.{tail}"));
        }
    }
    Outcome::fail(&format!("{app} restarted, but I still can't see it."), LAYER)
}

async fn set_up(core: &Arc<Core>, app: &str) -> Outcome {
    let Some(found) = find_app(core, app) else {
        return Outcome::fail(&format!("I couldn't find {app}."), LAYER);
    };
    if make_ready(&found).await {
        Outcome::ok_with(&format!("Set up {app}"), LAYER, Some(app.to_owned())).with_reply(format!("Done. Close {app} and open it again once, and I'll see it."))
    } else {
        Outcome::fail(&format!("I couldn't change how {app} starts. Close it and open it again from me, and I'll see it."), LAYER)
    }
}

fn open_settings() -> Outcome {
    let r = if cfg!(target_os = "macos") { crate::tools::shell::open("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility") } else { Ok(()) };
    match r {
        Ok(()) => Outcome::ok_with("Opened the Accessibility setting", LAYER, None).with_reply("Turn on Setu in the list, then ask me again.".into()),
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    match tool {
        "access.restart" => restart(core, args.get("app").and_then(Value::as_str).unwrap_or_default()).await,
        "access.settings" => crate::layers::blocking(open_settings).await.unwrap_or_else(|_| Outcome::fail("I couldn't open the setting.", LAYER)),
        "access.ready" => set_up(core, args.get("app").and_then(Value::as_str).unwrap_or_default()).await,
        "access.later" => Outcome::ok_with("Okay", LAYER, None),
        "access.java" => {
            let app = args.get("app").and_then(Value::as_str).unwrap_or("your Java app").to_owned();
            match crate::layers::blocking(|| crate::ide::java_bridge_on().map(|_| crate::ide::jetbrains_screen_reader())).await {
                Ok(Ok(_)) => Outcome::ok_with("Turned on the Java bridge", LAYER, Some(app.clone())).with_reply(format!("Done. Close {app} and open it again once, and I'll see it.")),
                _ => Outcome::fail("I couldn't turn on the Java bridge.", LAYER),
            }
        }
        _ => Outcome::fail("I don't know that command.", LAYER),
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    match tool {
        "access.restart" => format!("Restarting {}", args.get("app").and_then(Value::as_str).unwrap_or("the app")),
        "access.settings" => "Opening the Accessibility setting".into(),
        "access.java" => "Turning on the Java bridge".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_cause_pops_up_once() {
        let a = outcome(&Cause::Asleep { app: "Testbrowser".into(), browser: true });
        assert!(a.ask.is_some());
        let b = outcome(&Cause::Asleep { app: "Testbrowser".into(), browser: true });
        assert!(b.ask.is_none() && !b.ok);
        assert!(matches!(rules("restart brave"), Some(Intent::Act { tool, .. }) if tool == "access.restart"));
        assert!(rules("restart the computer").is_none());
        let c = outcome(&Cause::Asleep { app: "Testcode".into(), browser: false });
        assert!(c.ask.is_some());
        let c2 = outcome(&Cause::Asleep { app: "Testcode".into(), browser: false });
        assert!(c2.ask.is_none() && c2.message.contains("Close Testcode"));
        let d = outcome(&Cause::Admin { app: "Regedit".into() });
        assert!(d.ok && d.reply.is_some());
        let e = outcome(&Cause::Admin { app: "Regedit".into() });
        assert!(!e.ok);
    }
}
