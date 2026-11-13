use crate::control::ConfirmSpec;
use crate::core::Core;
use crate::os::{self, Do, Node, Scope, Win};
use crate::router::Intent;
use crate::text::sentence_case;
use crate::tools::Outcome;
use crate::tools::input::{self, K};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const LAYER: &str = "accessibility";
const SITE: &str = "web.whatsapp.com";

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.into(), args }
}

fn tidy_name(n: &str) -> String {
    let mut x = n.trim().to_owned();
    for tail in [" on whatsapp", " in whatsapp", " on whats app", " open that contact", " open the contact", " open his chat", " open her chat", " open their chat", " please", " now"] {
        if let Some(r) = x.strip_suffix(tail) {
            x = r.trim().to_owned();
        }
    }
    for lead in ["to ", "with ", "the contact ", "contact ", "my friend ", "my "] {
        if let Some(r) = x.strip_prefix(lead) {
            x = r.trim().to_owned();
        }
    }
    x.split(',').next().unwrap_or("").trim().trim_end_matches("'s").to_owned()
}

pub fn rules(t: &str) -> Option<Intent> {
    let p = crate::plan::clean(t);
    let whatsapp = p.contains("whatsapp") || p.contains("whats app");
    for lead in ["read my last message from ", "read the last message from ", "read my messages from ", "read the messages from ", "what did ", "read messages from ", "read my latest message from ", "check my messages from ", "any new message from ", "did i get a message from "] {
        if let Some(rest) = p.strip_prefix(lead) {
            let who = if lead == "what did " { rest.strip_suffix(" say").or_else(|| rest.strip_suffix(" say on whatsapp")).or_else(|| rest.strip_suffix(" send me")).unwrap_or("") } else { rest };
            let name = tidy_name(who);
            if !name.is_empty() && name.split_whitespace().count() <= 4 {
                return Some(act("messages.read", json!({ "name": name })));
            }
        }
    }
    for sep in [" saying ", " that says ", " and say ", " telling them ", " telling him ", " telling her "] {
        for lead in ["send a message to ", "send a whatsapp message to ", "send message to ", "message ", "text ", "whatsapp ", "send a text to ", "tell "] {
            if let Some(rest) = p.strip_prefix(lead)
                && let Some((who, what)) = rest.split_once(sep)
            {
                if lead == "tell " && !whatsapp {
                    continue;
                }
                let name = tidy_name(who);
                if !name.is_empty() && !what.trim().is_empty() && name.split_whitespace().count() <= 4 {
                    return Some(act("messages.send", json!({ "name": name, "text": sentence_case(what.trim()) })));
                }
            }
        }
    }
    let opens = [
        "i want to message ", "i want to chat with ", "i want to text ", "i need to message ", "message ", "open the chat with ", "open chat with ", "open the chat of ", "open my chat with ",
        "open the conversation with ", "open conversation with ", "open whatsapp chat with ", "open the whatsapp chat with ", "chat with ", "open the contact ", "open contact ",
    ];
    for lead in opens {
        if let Some(rest) = p.strip_prefix(lead) {
            if lead == "message " && !whatsapp && rest.split_whitespace().count() > 3 {
                continue;
            }
            if (lead.starts_with("open the contact") || lead.starts_with("open contact") || lead == "chat with ") && !whatsapp {
                continue;
            }
            let name = tidy_name(rest);
            if !name.is_empty() && name.split_whitespace().count() <= 4 {
                return Some(act("messages.open", json!({ "name": name })));
            }
        }
    }
    if whatsapp
        && let Some(rest) = p.strip_prefix("open ")
        && let Some(who) = rest.strip_suffix("'s chat").or_else(|| rest.strip_suffix(" chat"))
    {
        let name = tidy_name(who);
        if !name.is_empty() {
            return Some(act("messages.open", json!({ "name": name })));
        }
    }
    None
}

pub fn confirm(args: &Value) -> Option<ConfirmSpec> {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    let text = args.get("text").and_then(Value::as_str).unwrap_or_default();
    Some(ConfirmSpec {
        title: format!("Send “{text}” to {} on WhatsApp?", sentence_case(name)),
        facts: vec![],
        consequence: String::new(),
        risk: "medium",
        action_label: "Send".into(),
        danger: false,
    })
}

fn is_browser(app: &str) -> bool {
    let a = app.to_lowercase();
    ["chrome", "chromium", "brave", "edge", "firefox", "opera", "vivaldi", "safari", "librewolf", "zen"].iter().any(|w| a.contains(w))
}

fn whatsapp_window(wins: &[Win]) -> Option<(Win, bool)> {
    if let Some(w) = wins.iter().find(|w| w.app.to_lowercase().contains("whatsapp") && !is_browser(&w.app)) {
        return Some((w.clone(), true));
    }
    let mut web: Vec<&Win> = wins.iter().filter(|w| is_browser(&w.app) && w.title.to_lowercase().contains("whatsapp")).collect();
    web.sort_by_key(|w| !w.front);
    web.first().map(|w| ((*w).clone(), false))
}

async fn wait_for(secs: u64) -> Option<(Win, bool)> {
    let until = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < until {
        if let Ok(wins) = os::windows().await
            && let Some(hit) = whatsapp_window(&wins)
        {
            return Some(hit);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    None
}

async fn locate(core: &Arc<Core>) -> Result<(Win, bool), String> {
    let _ = os::ready().await;
    let wins = os::windows().await.map_err(|e| e.to_string())?;
    if let Some(hit) = whatsapp_window(&wins) {
        return Ok(hit);
    }
    for w in wins.iter().filter(|w| is_browser(&w.app)) {
        let Ok(nodes) = os::tree(w, Scope { limit: 400, text: false, hidden: false }).await else { continue };
        if let Some(tab) = nodes.iter().find(|n| n.role == "tab" && n.name.to_lowercase().contains("whatsapp"))
            && os::act(tab, Do::Select).await.is_ok()
            && let Some(hit) = wait_for(4).await
        {
            return Ok(hit);
        }
    }
    let defaults = core.defaults();
    let app = core.apps.find("whatsapp", None, &defaults).into_iter().map(|(a, _)| a).find(|a| a.name.to_lowercase().contains("whatsapp") && !a.exec.contains("--app="));
    if let Some(a) = app {
        let _ = crate::layers::blocking(move || crate::tools::apps::launch(&a)).await;
        return wait_for(20).await.ok_or_else(|| "WhatsApp opened, but its window didn't show up.".to_owned());
    }
    let browser = crate::tools::web::find_browser(core, None);
    let c = core.clone();
    let url = format!("https://{SITE}/");
    match crate::layers::blocking(move || crate::tools::web::open(&c, browser.as_ref(), &url)).await {
        Ok(Ok(_)) => {}
        _ => return Err("I couldn't open WhatsApp Web.".into()),
    }
    wait_for(25).await.ok_or_else(|| "WhatsApp Web is still loading. Ask me again in a moment.".to_owned())
}

const SIGNED_OUT: &[&str] = &["scan to log in", "log into whatsapp", "log in with phone number", "link with phone number", "steps to log in", "use whatsapp on your computer", "scan the qr code"];
const SEARCH: &[&str] = &["search or start a new chat", "search input textbox", "search or start new chat", "search"];

async fn page(win: &Win, text: bool) -> Vec<Node> {
    os::tree(win, Scope { limit: 1200, text, hidden: false }).await.unwrap_or_default()
}

fn search_box(nodes: &[Node]) -> Option<&Node> {
    SEARCH.iter().find_map(|n| os::find(nodes, n, &["field", "combobox"]))
}

async fn ready(win: &Win) -> Result<Vec<Node>, String> {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let nodes = page(win, true).await;
        let low: Vec<String> = nodes.iter().map(|n| n.name.to_lowercase()).collect();
        if SIGNED_OUT.iter().any(|w| low.iter().any(|n| n.contains(w))) {
            return Err("WhatsApp isn't signed in on this computer. Scan the code with your phone, then ask me again.".into());
        }
        if search_box(&nodes).is_some() {
            return Ok(nodes);
        }
        if Instant::now() > until {
            return Err("WhatsApp is still loading. Ask me again in a moment.".into());
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

async fn bring_front(win: &Win, anchor: Option<&Node>) {
    if os::in_front(win, 0).await {
        return;
    }
    if let Some(n) = anchor {
        let _ = os::act(n, Do::Focus).await;
        if os::in_front(win, 1200).await {
            return;
        }
    }
    let _ = crate::tools::wm::focus(&win.app, None).await;
    let _ = os::in_front(win, 2500).await;
}

fn row_for<'a>(nodes: &'a [Node], want: &str, search: &Node) -> Option<&'a Node> {
    let w = crate::text::normalize(want);
    let mut best: Option<(f64, &Node)> = None;
    for n in nodes.iter().filter(|n| matches!(n.role.as_str(), "row" | "item" | "cell" | "treeitem" | "link" | "button") && n.id != search.id) {
        let name = crate::text::normalize(&n.name);
        if name.is_empty() || matches!(name.as_str(), "archived" | "new chat" | "menu" | "search") {
            continue;
        }
        let first = name.split_whitespace().take(w.split_whitespace().count()).collect::<Vec<_>>().join(" ");
        let s = if name == w || first == w { 2.0 } else { os::score(&w, &name).max(crate::text::similarity(&w, &first) * 0.95) };
        let s = s + if n.role == "row" || n.role == "item" { 0.01 } else { 0.0 };
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, n));
        }
    }
    best.filter(|(s, _)| *s >= 0.8).map(|(_, n)| n)
}

fn shown_name(row: &Node, want: &str) -> String {
    let words = want.split_whitespace().count().max(1);
    let lead: Vec<&str> = row.name.split_whitespace().take(words).collect();
    let lead = lead.join(" ");
    if crate::text::normalize(&lead) == crate::text::normalize(want) { lead } else { sentence_case(want) }
}

fn composer(nodes: &[Node]) -> Option<&Node> {
    nodes.iter().find(|n| n.role == "field" && crate::text::normalize(&n.name).starts_with("type a message")).or_else(|| os::find(nodes, "type a message", &["field"])).or_else(|| os::find(nodes, "message", &["field"]))
}

async fn open_chat(win: &Win, name: &str) -> Result<String, String> {
    let nodes = ready(win).await?;
    let search = search_box(&nodes).cloned().ok_or("WhatsApp's search box isn't on screen.")?;
    bring_front(win, Some(&search)).await;
    os::type_into(win, &search, name, true).await.map_err(|e| e.to_string())?;
    let mut row = None;
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let found = page(win, false).await;
        if let Some(r) = row_for(&found, name, &search) {
            row = Some(r.clone());
            break;
        }
    }
    let Some(row) = row else { return Err(format!("I couldn't find {} in your WhatsApp chats.", sentence_case(name))) };
    let opened = |nodes: &[Node]| composer(nodes).is_some();
    os::act(&row, Do::Invoke).await.map_err(|e| e.to_string())?;
    for attempt in 0..2 {
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            if opened(&page(win, false).await) {
                return Ok(shown_name(&row, name));
            }
        }
        if attempt == 0 {
            let _ = os::act(&row, Do::Focus).await;
            bring_front(win, Some(&row)).await;
            if os::in_front(win, 500).await {
                let _ = input::press(&[K::Enter]).await;
            }
        }
    }
    Err(format!("I found {} but the chat didn't open.", sentence_case(name)))
}

async fn send(win: &Win, text: &str) -> Result<(), String> {
    let nodes = page(win, false).await;
    let boxed = composer(&nodes).cloned().ok_or("Open a chat first.")?;
    bring_front(win, Some(&boxed)).await;
    os::type_into(win, &boxed, text, false).await.map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = page(win, false).await;
    match os::find(&after, "send", &["button"]) {
        Some(b) => os::act(b, Do::Invoke).await.map_err(|e| e.to_string())?,
        None => input::press(&[K::Enter]).await.map_err(|e| e.to_string())?,
    }
    let want = crate::text::normalize(text);
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = page(win, false).await;
        match composer(&now) {
            Some(c) if crate::text::normalize(&c.value).contains(&want) && !want.is_empty() => {}
            _ => return Ok(()),
        }
    }
    if os::in_front(win, 500).await {
        input::press(&[K::Enter]).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn read_last(win: &Win, title: &str) -> String {
    let nodes = page(win, true).await;
    let left = search_box(&nodes).map(|s| s.x + s.w).unwrap_or(0);
    let skip = crate::text::normalize(title);
    let lines: Vec<String> = nodes
        .iter()
        .filter(|n| matches!(n.role.as_str(), "text" | "row" | "cell") && n.x >= left && !n.name.trim().is_empty())
        .map(|n| n.name.trim().to_owned())
        .filter(|t| crate::text::normalize(t) != skip && !t.chars().all(|c| c.is_ascii_digit() || c == ':' || c == ' ') && !t.to_lowercase().starts_with("type a message"))
        .collect();
    let mut uniq: Vec<String> = Vec::new();
    for l in lines {
        if uniq.last() != Some(&l) {
            uniq.push(l);
        }
    }
    let last: Vec<String> = uniq.into_iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().collect();
    last.join(". ")
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let name = s("name");
    if name.is_empty() {
        return Outcome::fail("Who should I message?", LAYER);
    }
    if !core.settings.bool("permissions.control") {
        return Outcome::fail("Opening chats needs the Control permission.", LAYER).needs("control");
    }
    let (win, desktop) = match locate(core).await {
        Ok(w) => w,
        Err(e) => return Outcome::fail(&e, LAYER),
    };
    let app = if desktop { "WhatsApp".to_owned() } else { format!("WhatsApp in {}", win.app) };
    let title = match open_chat(&win, &name).await {
        Ok(t) => t,
        Err(e) => return Outcome::fail(&e, LAYER),
    };
    match tool {
        "messages.open" => Outcome::ok_with(&format!("Opened the chat with {title}"), LAYER, Some(app)).with_reply(format!("Opened your chat with {title}.")),
        "messages.read" => {
            let said = read_last(&win, &title).await;
            if said.is_empty() {
                Outcome::ok_with(&format!("Opened the chat with {title}"), LAYER, Some(app)).with_reply(format!("I opened your chat with {title}, but I can't read the messages there."))
            } else {
                Outcome::ok_with(&format!("Read the chat with {title}"), LAYER, Some(app)).with_reply(format!("The latest in your chat with {title}: {said}."))
            }
        }
        _ => match send(&win, &s("text")).await {
            Ok(()) => Outcome::ok_with(&format!("Sent a message to {title}"), LAYER, Some(app)).with_reply(format!("Sent it to {title}.")),
            Err(e) => Outcome::fail(&e, LAYER),
        },
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    let name = sentence_case(args.get("name").and_then(Value::as_str).unwrap_or("the chat"));
    match tool {
        "messages.send" => format!("Messaging {name}"),
        "messages.read" => format!("Reading the chat with {name}"),
        _ => format!("Opening the chat with {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(t: &str) -> Option<(String, Value)> {
        match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn understands_whatsapp_requests() {
        assert_eq!(tool("i want to message binod open that contact"), Some(("messages.open".into(), json!({ "name": "binod" }))));
        assert_eq!(tool("open the chat with priya on whatsapp").unwrap().1["name"], "priya");
        assert_eq!(tool("send a message to binod saying i will be late"), Some(("messages.send".into(), json!({ "name": "binod", "text": "I will be late" }))));
        assert_eq!(tool("message rahul saying call me").unwrap().0, "messages.send");
        assert_eq!(tool("read my last message from binod"), Some(("messages.read".into(), json!({ "name": "binod" }))));
        assert_eq!(tool("what did papa say").unwrap().1["name"], "papa");
        assert!(tool("open firefox").is_none());
        assert!(tool("message").is_none());
    }
}
