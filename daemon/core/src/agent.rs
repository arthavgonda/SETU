use crate::core::Core;
use crate::tools::Outcome;
use crate::tools::atspi::{self, Element, Screen};
use crate::tools::input;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SYSTEM: &str = "You control a desktop computer for a person who cannot use the mouse or keyboard. Each turn you see the goal, what you did so far, and the controls in the active window as a numbered list. Choose exactly one next action as JSON.\n\
Actions:\n\
click(n): press control n.\n\
type(n, text): put text into field n (replaces its content). Then press enter with key if needed.\n\
key(keys): press keys, for example \"enter\", \"ctrl l\", \"tab\", \"pagedown\", \"escape\".\n\
open(url): open a web address in the browser.\n\
scroll(dir): dir is down or up.\n\
wait(): the screen is still loading.\n\
done(say): the goal is reached; say one short sentence about it.\n\
fail(say): the goal can't be reached; say why in one short sentence.\n\
Rules: never buy, pay, send money, delete, post or send messages unless the goal says exactly that. Never type passwords. Prefer clicking a visible control over guessing addresses. Finish with done as soon as the goal is reached.";

const GRAMMAR: &str = r#"root ::= "{\"do\":" ( click | type | key | open | scroll | wait | done | fail ) "}"
click ::= "\"click\",\"n\":" num
type ::= "\"type\",\"n\":" num ",\"text\":" str
key ::= "\"key\",\"keys\":" str
open ::= "\"open\",\"url\":" str
scroll ::= "\"scroll\",\"dir\":" ( "\"down\"" | "\"up\"" )
wait ::= "\"wait\""
done ::= "\"done\",\"say\":" str
fail ::= "\"fail\",\"say\":" str
str ::= "\"" chr{0,200} "\""
chr ::= [^"\\\x00-\x1f] | "\\" ["\\/bfnrt]
num ::= [0-9] [0-9]? [0-9]?
"#;

const RISKY: &[&str] = &["buy", "pay", "purchase", "place order", "order now", "checkout", "check out", "delete", "remove", "uninstall", "send", "post", "publish", "transfer", "subscribe", "sign out", "log out", "logout", "format", "erase", "confirm payment", "donate"];

fn risky(label: &str, goal: &str) -> bool {
    let l = crate::text::normalize(label);
    let g = crate::text::normalize(goal);
    RISKY.iter().any(|w| (l == *w || l.starts_with(&format!("{w} ")) || l.ends_with(&format!(" {w}"))) && !g.contains(w))
}

fn describe(screen: &Screen) -> String {
    let mut out = format!("App: {}\nWindow: {}\nControls:\n", screen.app, screen.title);
    for e in screen.elements.iter().take(90) {
        let label: String = e.label.chars().take(70).collect();
        let value = if e.value.is_empty() || e.role == "password" { String::new() } else { format!(" = \"{}\"", e.value.chars().take(50).collect::<String>()) };
        out.push_str(&format!("{} {} \"{}\"{}\n", e.n, e.role, label, value));
        if out.len() > 5200 {
            break;
        }
    }
    out
}

async fn bring_up(core: &Arc<Core>, app: &str) {
    let app = app.trim();
    if app.is_empty() || matches!(app, "browser" | "the browser" | "web") && crate::tools::web::find_browser(core, None).is_none() {
        return;
    }
    let target = if matches!(app, "browser" | "the browser" | "web") {
        crate::tools::web::find_browser(core, None)
    } else {
        crate::tools::web::find_browser(core, Some(app)).or_else(|| core.apps.find(app, None, &core.defaults()).into_iter().map(|(a, _)| a).next())
    };
    if let Some((current, _)) = atspi::active_app().await
        && target.as_ref().is_some_and(|t| crate::text::similarity(&t.name, &current) >= 0.8 || crate::text::normalize(&current).contains(&crate::text::normalize(app)))
    {
        return;
    }
    if let Some(a) = target {
        let _ = crate::layers::blocking(move || crate::tools::apps::launch(&a)).await;
        tokio::time::sleep(Duration::from_millis(2200)).await;
    }
}

pub async fn run(core: &Arc<Core>, goal: &str, app: Option<String>, cancel: Arc<AtomicBool>) -> Outcome {
    if !core.settings.bool("permissions.control") {
        let mut o = Outcome::fail("Working inside apps needs the Control permission. Turn it on in Settings, Permissions.", "agent");
        o.fix = Some(json!({ "label": "Open Permissions", "method": "ui.open", "params": { "page": "settings/permissions" } }));
        return o.needs("control");
    }
    if let Some(a) = &app {
        bring_up(core, a).await;
    }
    let mut done: Vec<String> = Vec::new();
    let mut last_screen = String::new();
    let mut stuck = 0;
    for step in 0..8 {
        if cancel.load(Ordering::Relaxed) {
            let mut o = Outcome::fail("Stopped.", "agent");
            o.cancelled = true;
            return o;
        }
        let screen = match atspi::snapshot(120).await {
            Ok(s) => s,
            Err(e) => return Outcome::fail(&format!("I can't read this window. {e}"), "agent"),
        };
        let view = describe(&screen);
        if view == last_screen {
            stuck += 1;
        } else {
            stuck = 0;
        }
        last_screen = view.clone();
        if stuck >= 3 {
            return Outcome::fail("The screen stopped changing, so I stopped. Tell me the next step.", "agent");
        }
        let history = if done.is_empty() { "nothing yet".to_owned() } else { done.iter().enumerate().map(|(i, d)| format!("{}. {d}", i + 1)).collect::<Vec<_>>().join("\n") };
        let user = format!("Goal: {goal}\nDone so far:\n{history}\n\n{view}\nStep {} of 8. Next action:", step + 1);
        let v = match crate::pipeline::ask_json(core, SYSTEM, &user, Some(GRAMMAR.to_owned()), 120, 0.0).await {
            Ok(v) => v,
            Err(e) => return Outcome::fail(&format!("I couldn't plan the next step: {e}"), "agent"),
        };
        let what = v.get("do").and_then(Value::as_str).unwrap_or("fail");
        let n = v.get("n").and_then(Value::as_u64).unwrap_or(0) as usize;
        let el: Option<&Element> = screen.elements.iter().find(|e| e.n == n);
        let say = v.get("say").and_then(Value::as_str).unwrap_or_default().to_owned();
        match what {
            "done" => {
                let mut o = Outcome::ok_with(if say.is_empty() { "Done" } else { &say }, "agent", Some(screen.app.clone()));
                o.reply = Some(if say.is_empty() { "Done.".into() } else { say });
                return o;
            }
            "fail" => return Outcome::fail(if say.is_empty() { "I couldn't finish that." } else { &say }, "agent"),
            "click" => {
                let Some(e) = el else {
                    done.push(format!("tried to click {n}, which doesn't exist"));
                    continue;
                };
                if risky(&e.label, goal) {
                    return Outcome::fail(&format!("I stopped before pressing {}. Say click {} if you want me to.", e.label, e.label), "agent");
                }
                if let Err(err) = atspi::click(e).await {
                    done.push(format!("clicking {} \"{}\" failed: {err}", e.n, e.label));
                } else {
                    done.push(format!("clicked {} \"{}\"", e.n, e.label));
                }
            }
            "type" => {
                let text = v.get("text").and_then(Value::as_str).unwrap_or_default();
                let Some(e) = el else {
                    done.push(format!("tried to type into {n}, which doesn't exist"));
                    continue;
                };
                if e.role == "password" {
                    return Outcome::fail("This needs your password. Type it yourself, then tell me to continue.", "agent");
                }
                match atspi::set_text(e, text).await {
                    Ok(()) => done.push(format!("typed \"{text}\" into {} \"{}\"", e.n, e.label)),
                    Err(err) => done.push(format!("typing into {} failed: {err}", e.n)),
                }
            }
            "key" => {
                let keys = v.get("keys").and_then(Value::as_str).unwrap_or_default();
                match input::parse_combo(keys) {
                    Some(k) => match input::press(&k).await {
                        Ok(()) => done.push(format!("pressed {keys}")),
                        Err(err) => return Outcome::fail(&err.to_string(), "agent"),
                    },
                    None => done.push(format!("{keys} is not a key")),
                }
            }
            "open" => {
                let url = v.get("url").and_then(Value::as_str).unwrap_or_default();
                let full = if url.starts_with("http") { url.to_owned() } else { format!("https://{url}") };
                let browser = crate::tools::web::find_browser(core, app.as_deref().filter(|a| crate::tools::web::browser_key(a).is_some()));
                let c = core.clone();
                let f2 = full.clone();
                match crate::layers::blocking(move || crate::tools::web::open(&c, browser.as_ref(), &f2)).await {
                    Ok(Ok(_)) => done.push(format!("opened {full}")),
                    _ => done.push(format!("opening {full} failed")),
                }
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            "scroll" => {
                let dir = v.get("dir").and_then(Value::as_str).unwrap_or("down");
                let _ = input::press(&[if dir == "up" { input::K::PgUp } else { input::K::PgDn }]).await;
                done.push(format!("scrolled {dir}"));
            }
            _ => {
                done.push("waited".into());
                tokio::time::sleep(Duration::from_millis(900)).await;
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    Outcome::fail("That took too many steps, so I stopped. Tell me the next step.", "agent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_risky_buttons() {
        assert!(risky("Buy now", "find a phone case on amazon"));
        assert!(risky("Delete", "open my files"));
        assert!(!risky("Delete", "delete the draft"));
        assert!(!risky("Search", "find cats"));
        assert!(!risky("Sender details", "open gmail"));
    }
}
