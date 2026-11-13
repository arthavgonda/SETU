use crate::text::{find_number, minutes, normalize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    OpenApp { target: String, role: Option<&'static str> },
    CloseApp { target: String },
    Volume { action: &'static str, amount: Option<i64>, level: Option<i64> },
    Brightness { action: &'static str, amount: Option<i64>, level: Option<i64> },
    Time,
    Date,
    Wifi { action: &'static str, network: Option<String> },
    ScanQr,
    Numbers { action: &'static str, n: Option<i64> },
    TypeText { text: String },
    Install { app: String },
    MuteMic { minutes: Option<i64> },
    Sleep,
    Reply { text: String },
    Ask { question: String, options: Vec<String> },
    Cancel,
    Repeat,
    Undo,
    Correct { text: String },
    Plugin { tool: String, args: Value },
    Internal { kind: &'static str, args: Value },
    Act { tool: String, args: Value },
    Unknown,
}

impl Intent {
    pub fn action(&self) -> &'static str {
        match self {
            Self::OpenApp { .. } => "open_app",
            Self::CloseApp { .. } => "close_app",
            Self::Volume { .. } => "volume",
            Self::Brightness { .. } => "brightness",
            Self::Time => "time",
            Self::Date => "date",
            Self::Wifi { .. } => "wifi",
            Self::ScanQr => "scan_qr",
            Self::Numbers { .. } => "numbers",
            Self::TypeText { .. } => "type_text",
            Self::Install { .. } => "install_app",
            Self::MuteMic { .. } => "mute_mic",
            Self::Sleep => "sleep",
            Self::Reply { .. } => "reply",
            Self::Ask { .. } => "ask",
            Self::Cancel => "cancel",
            Self::Repeat => "repeat",
            Self::Undo => "undo",
            Self::Correct { .. } => "correct",
            Self::Plugin { .. } => "plugin",
            Self::Internal { kind, .. } => kind,
            Self::Act { .. } => "act",
            Self::Unknown => "unknown",
        }
    }

    pub fn target(&self) -> String {
        match self {
            Self::OpenApp { target, .. } | Self::CloseApp { target } => target.clone(),
            Self::Install { app } => app.clone(),
            Self::Volume { action, level, amount } | Self::Brightness { action, level, amount } => match (level, amount) {
                (Some(l), _) => format!("{action} {l}"),
                (_, Some(a)) => format!("{action} {a}"),
                _ => action.to_string(),
            },
            Self::Wifi { action, network } => network.clone().map(|n| format!("{action} {n}")).unwrap_or_else(|| action.to_string()),
            Self::Numbers { action, n } => n.map(|n| format!("{action} {n}")).unwrap_or_else(|| action.to_string()),
            Self::TypeText { .. } => "focused field".into(),
            Self::Plugin { tool, .. } => tool.clone(),
            Self::Internal { args, .. } => args.get("label").or_else(|| args.get("name")).or_else(|| args.get("url")).and_then(Value::as_str).unwrap_or_default().to_owned(),
            Self::Act { tool, args } => ["site", "query", "name", "url", "where", "keys", "command", "message", "goal", "app", "kind"]
                .iter()
                .filter_map(|k| args.get(*k).and_then(Value::as_str).filter(|v| !v.is_empty()))
                .next()
                .map(str::to_owned)
                .unwrap_or_else(|| tool.clone()),
            _ => String::new(),
        }
    }

    pub fn args(&self) -> Value {
        match self {
            Self::OpenApp { target, role } => json!({ "app": target, "role": role }),
            Self::CloseApp { target } => json!({ "app": target }),
            Self::Volume { action, amount, level } | Self::Brightness { action, amount, level } => json!({ "action": action, "amount": amount, "level": level }),
            Self::Wifi { action, network } => json!({ "action": action, "network": network }),
            Self::Numbers { action, n } => json!({ "action": action, "n": n }),
            Self::TypeText { text } => json!({ "length": text.chars().count() }),
            Self::Install { app } => json!({ "app": app }),
            Self::MuteMic { minutes } => json!({ "minutes": minutes }),
            Self::Plugin { tool, args } => json!({ "tool": tool, "args": args }),
            Self::Act { tool, args } => json!({ "tool": tool, "args": if tool == "ui.type" || tool == "code.insert" { json!({ "length": args.get("text").and_then(Value::as_str).map(|t| t.chars().count()).unwrap_or(0) }) } else if tool == "app.write" { json!({ "app": args.get("app"), "length": args.get("text").and_then(Value::as_str).map(|t| t.chars().count()).unwrap_or(0) }) } else { args.clone() } }),
            _ => json!({}),
        }
    }
}

const ROLE_WORDS: &[(&str, &str)] = &[
    ("web browser", "browser"),
    ("internet", "browser"),
    ("the internet", "browser"),
    ("browser", "browser"),
    ("music player", "music"),
    ("my music", "music"),
    ("music", "music"),
    ("text editor", "editor"),
    ("editor", "editor"),
    ("email", "mail"),
    ("e mail", "mail"),
    ("my email", "mail"),
    ("mail", "mail"),
    ("terminal", "terminal"),
    ("command line", "terminal"),
    ("file manager", "files"),
    ("my files", "files"),
    ("files", "files"),
];

fn strip_prefix<'a>(s: &'a str, prefixes: &[&str]) -> &'a str {
    let mut t = s;
    loop {
        let before = t;
        for p in prefixes {
            if let Some(r) = t.strip_prefix(p)
                && (r.is_empty() || r.starts_with(' ')) {
                    t = r.trim_start();
                }
        }
        if t == before {
            return t;
        }
    }
}

fn strip_suffix<'a>(s: &'a str, suffixes: &[&str]) -> &'a str {
    let mut t = s;
    loop {
        let before = t;
        for p in suffixes {
            if let Some(r) = t.strip_suffix(p)
                && (r.is_empty() || r.ends_with(' ')) {
                    t = r.trim_end();
                }
        }
        if t == before {
            return t;
        }
    }
}

const POLITE: &[&str] = &["please", "can you", "could you", "would you", "will you", "i want to", "i'd like to", "i would like to", "go ahead and", "just", "now"];

pub fn rules(raw: &str, stop_word: &str, numbers_visible: bool) -> Option<Intent> {
    let t = normalize(raw);
    let t = strip_suffix(strip_prefix(&t, POLITE), &["please", "for me", "now", "thanks", "thank you"]);
    if t.is_empty() {
        return None;
    }
    let is = |opts: &[&str]| opts.contains(&t);
    if let Some(i) = crate::looks::rules(t) {
        return Some(i);
    }
    if t == stop_word || is(&["stop", "cancel", "never mind", "nevermind", "forget it", "cancel that", "stop that"]) {
        return Some(Intent::Cancel);
    }
    if is(&["go to sleep", "stop listening", "sleep", "go back to sleep", "goodbye", "that's all", "that is all"]) {
        return Some(Intent::Sleep);
    }
    if is(&["repeat that", "repeat", "say that again", "say it again", "what did you say", "pardon", "come again"]) {
        return Some(Intent::Repeat);
    }
    if is(&["undo", "undo that", "undo it", "take that back", "revert that"]) {
        return Some(Intent::Undo);
    }
    for p in ["no i said ", "no i meant ", "i said ", "i meant "] {
        if let Some(r) = t.strip_prefix(p)
            && !r.is_empty() {
                return Some(Intent::Correct { text: r.to_owned() });
            }
    }
    if let Some(i) = crate::browse::address_rule(t, raw) {
        return Some(i);
    }
    let words = t.split_whitespace().count();
    if is(&["time", "the time", "current time", "tell me the time"]) || ((t.starts_with("what time") || t.starts_with("what's the time") || t.starts_with("what is the time")) && words <= 5) {
        return Some(Intent::Time);
    }
    if is(&["date", "today's date", "what is today"])
        || ((t.starts_with("what's the date") || t.starts_with("what is the date") || t.starts_with("what day") || t.starts_with("what's today") || t.starts_with("what is today's")) && words <= 6)
    {
        return Some(Intent::Date);
    }
    if (t.contains("qr") || t.contains("q r code") || t.contains("cue are code"))
        && (t.starts_with("scan") || t.starts_with("read") || t.contains("scan")) {
            return Some(Intent::ScanQr);
        }
    if is(&["show numbers", "show the numbers", "numbers", "show me numbers", "show number"]) {
        return Some(Intent::Numbers { action: "show", n: None });
    }
    if is(&["hide numbers", "hide the numbers", "hide number", "remove numbers"]) {
        return Some(Intent::Numbers { action: "hide", n: None });
    }
    if let Some(r) = t.strip_prefix("click ").or_else(|| t.strip_prefix("press ")).or_else(|| t.strip_prefix("tap "))
        && let Some(n) = find_number(&format!("number {}", r.trim_start_matches("number ").trim_start_matches("on "))) {
            return Some(Intent::Numbers { action: "click", n: Some(n) });
        }
    if numbers_visible {
        let r = t.trim_start_matches("number ");
        if let Some(n) = find_number(&format!("number {r}"))
            && r.split_whitespace().count() <= 2 {
                return Some(Intent::Numbers { action: "click", n: Some(n) });
            }
    }
    let mic = t.contains("microphone") || t.contains(" mic") || t.starts_with("mic");
    if mic && (t.starts_with("mute") || t.starts_with("turn off") || t.starts_with("switch off") || t.starts_with("disable")) {
        return Some(Intent::MuteMic { minutes: minutes(t) });
    }
    if let Some(i) = volume(t) {
        return Some(i);
    }
    if let Some(i) = brightness(t) {
        return Some(i);
    }
    if let Some(i @ Intent::Act { .. }) = crate::syscfg::rules(t)
        && matches!(&i, Intent::Act { tool, .. } if tool == "sys.toggle")
    {
        return Some(i);
    }
    if t.contains("wifi") || t.contains("wi fi") || t.contains("wireless") || t.contains("network") && t.starts_with("connect") {
        if t.starts_with("disconnect") {
            return Some(Intent::Wifi { action: "disconnect", network: None });
        }
        if t.starts_with("show") || t.starts_with("list") || t.contains("networks") && !t.starts_with("connect") {
            return Some(Intent::Wifi { action: "list", network: None });
        }
        if t.starts_with("connect") || t.starts_with("join") {
            let network = ["called ", "named ", "network ", "to "]
                .iter()
                .filter_map(|k| t.rsplit_once(k).map(|(_, r)| r.trim()))
                .find(|r| !r.is_empty() && !["wifi", "wi fi", "the wifi", "wireless", "the network", "a network", "network"].contains(r))
                .map(str::to_owned);
            return Some(Intent::Wifi { action: "connect", network });
        }
    }
    if let Some(i) = crate::skills::parse(t, raw) {
        return Some(i);
    }
    if let Some(r) = t.strip_prefix("type ") {
        let r = r.trim_start_matches("in ").trim();
        if let Some(email) = crate::tools::codespeech::spoken_email(r) {
            return Some(Intent::TypeText { text: email });
        }
        return Some(Intent::TypeText { text: raw_tail(raw, "type").unwrap_or_else(|| r.to_owned()) });
    }
    if let Some(r) = t.strip_prefix("install ") {
        let app = strip_prefix(r, &["the app", "the application", "app", "the"]);
        if !app.is_empty() {
            return Some(Intent::Install { app: crate::text::sentence_case(app) });
        }
    }
    if is(&["show yourself", "come back", "show your window", "show the dashboard", "show your dashboard"]) {
        return Some(Intent::OpenApp { target: "yourself".into(), role: None });
    }
    if is(&["hide yourself", "hide your window", "hide the dashboard", "close yourself", "close your window"]) {
        return Some(Intent::CloseApp { target: "yourself".into() });
    }
    if is(&["close the app", "close the application", "close app", "close application", "close this app", "close this application", "close the window", "close this window", "close it", "close that"]) {
        return Some(Intent::CloseApp { target: String::new() });
    }
    for verb in ["open up ", "open ", "launch ", "start up ", "start ", "run ", "bring up ", "show me "] {
        if let Some(r) = t.strip_prefix(verb) {
            if t.starts_with("start a ") || t.starts_with("start new ") || t.starts_with("run a ") {
                return None;
            }
            let target = strip_suffix(strip_prefix(r, &["the", "my", "a", "app", "application"]), &["app", "application", "program", "and then"]);
            if target.is_empty() || target.split_whitespace().count() > 5 {
                return None;
            }
            for (w, role) in ROLE_WORDS {
                if target == *w {
                    return Some(Intent::OpenApp { target: target.to_owned(), role: Some(role) });
                }
            }
            return Some(Intent::OpenApp { target: target.to_owned(), role: None });
        }
    }
    for verb in ["close ", "quit ", "exit ", "kill "] {
        if let Some(r) = t.strip_prefix(verb) {
            let target = strip_suffix(strip_prefix(r, &["all of", "all", "every", "the", "my", "app", "application"]), &["app", "application", "windows", "window", "program"]);
            if !target.is_empty() && target.split_whitespace().count() <= 5 {
                return Some(Intent::CloseApp { target: target.to_owned() });
            }
        }
    }
    None
}

const SELF_WORDS: &[&str] = &["yourself", "you", "your self", "assistant", "the assistant", "dashboard", "your dashboard", "your app", "your window", "control panel"];
const SELF_FUZZY: &[&str] = &["yourself", "dashboard"];
const SELF_SUFFIX: &[&str] = &["", "app", "application", "ui", "u i", "window", "dashboard", "interface", "home", "home page"];

pub fn self_page(target: &str, names: &[String]) -> Option<&'static str> {
    let t = normalize(target);
    let t = t.trim().trim_start_matches("the ").trim();
    if t.is_empty() {
        return None;
    }
    if SELF_WORDS.contains(&t) || SELF_FUZZY.iter().any(|w| crate::text::similarity(t, w) >= 0.8) {
        return Some("home");
    }
    if ["your settings", "your preferences", "your options"].contains(&t) {
        return Some("settings/voice");
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    for own in names.iter().map(|n| normalize(n)).chain(std::iter::once("setu".to_owned())) {
        let n = own.split_whitespace().count();
        if own.is_empty() || words.len() < n {
            continue;
        }
        for take in [n, n + 1] {
            if words.len() < take {
                continue;
            }
            let head = words[..take].join(" ");
            let rest = words[take..].join(" ");
            if head == own || crate::text::similarity(&head, &own) >= 0.85 {
                if SELF_SUFFIX.contains(&rest.as_str()) {
                    return Some("home");
                }
                if ["settings", "preferences", "options"].contains(&rest.as_str()) {
                    return Some("settings/voice");
                }
            }
        }
    }
    None
}

fn raw_tail(raw: &str, verb: &str) -> Option<String> {
    let lower = raw.to_lowercase();
    let idx = lower.find(&format!("{verb} "))?;
    let tail = raw[idx + verb.len() + 1..].trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

fn volume(t: &str) -> Option<Intent> {
    if crate::playback::app_volume(t) {
        return None;
    }
    let about = t.contains("volume") || t.contains("sound") || t.contains("audio") || t.contains("speaker");
    let it = t.starts_with("turn it ") || t.starts_with("make it ");
    if is_any(t, &["mute", "mute the sound", "mute sound", "mute audio", "mute the volume", "mute volume", "mute the audio", "silence", "be quiet"]) {
        return Some(Intent::Volume { action: "mute", amount: None, level: None });
    }
    let unmute_sound = t.strip_prefix("unmute ").is_some_and(|r| about || matches!(r, "it" | "everything" | "the computer" | "my computer" | "the laptop" | "my laptop" | "me" | "the pc" | "my pc"));
    if t == "unmute" || unmute_sound || t == "turn the sound back on" || t == "sound on" {
        return Some(Intent::Volume { action: "unmute", amount: None, level: None });
    }
    if about && (t.starts_with("set") || t.contains(" to ") || t.starts_with("volume "))
        && let Some(n) = find_number(t)
            && (0..=100).contains(&n) && !t.contains(" up ") && !t.contains(" down ") && !t.ends_with(" up") && !t.ends_with(" down") {
                return Some(Intent::Volume { action: "set", amount: None, level: Some(n) });
            }
    let up = t.contains("louder") || (about || it) && (t.contains(" up") || t.contains("increase") || t.contains("raise") || t.contains("higher"));
    let down = t.contains("quieter") || t.contains("softer") || (about || it) && (t.contains(" down") || t.contains("decrease") || t.contains("lower") || t.contains("reduce"));
    if up || down {
        let amount = find_number(t).filter(|n| (1..=100).contains(n)).or(if t.contains("a lot") { Some(30) } else if t.contains("a little") || t.contains("a bit") { Some(5) } else { None });
        return Some(Intent::Volume { action: if up { "up" } else { "down" }, amount, level: None });
    }
    None
}

fn brightness(t: &str) -> Option<Intent> {
    let about = t.contains("brightness") || t.contains("screen") || t.contains("display");
    if !(about || t.contains("brighter") || t.contains("dimmer") || t.starts_with("dim")) {
        return None;
    }
    if about && (t.starts_with("set") || t.contains(" to "))
        && let Some(n) = find_number(t).filter(|n| (0..=100).contains(n)) {
            return Some(Intent::Brightness { action: "set", amount: None, level: Some(n) });
        }
    let up = t.contains("brighter") || about && (t.contains(" up") || t.contains("increase") || t.contains("raise"));
    let down = t.contains("dimmer") || t.starts_with("dim") || about && (t.contains(" down") || t.contains("decrease") || t.contains("lower") || t.contains("reduce"));
    if up || down {
        return Some(Intent::Brightness { action: if up { "up" } else { "down" }, amount: find_number(t).filter(|n| (1..=100).contains(n)), level: None });
    }
    None
}

fn is_any(t: &str, opts: &[&str]) -> bool {
    opts.contains(&t)
}

pub const GRAMMAR: &str = r#"root ::= "{" say? ( open | close | volume | brightness | timeq | dateq | wifi | qr | numbers | typetext | install | mutemic | sleep | web | media | browser | window | key | click | files | write | todo | git | code | terminal | task | reply | ask | unknown PLUGINS ) "}"
open ::= "\"tool\":\"open_app\",\"app\":" str
media ::= "\"tool\":\"media\",\"action\":" ( "\"pause\"" | "\"resume\"" | "\"next\"" | "\"previous\"" | "\"list\"" | "\"mute\"" | "\"unmute\"" ) ",\"target\":" str
browser ::= "\"tool\":\"browser\",\"action\":" ( "\"new_tab\"" | "\"new_window\"" | "\"private_window\"" | "\"close_tab\"" | "\"switch_tab\"" | "\"open\"" | "\"click\"" | "\"options\"" | "\"save\"" | "\"copy\"" | "\"settings\"" | "\"images\"" | "\"videos\"" | "\"news\"" ) ",\"target\":" str ",\"where\":" ( "\"here\"" | "\"tab\"" | "\"window\"" | "\"private\"" )
window ::= "\"tool\":\"window\",\"action\":" ( "\"left\"" | "\"right\"" | "\"maximize\"" | "\"minimize\"" | "\"restore\"" | "\"next_screen\"" | "\"desktop\"" | "\"switch\"" | "\"close\"" | "\"side_by_side\"" | "\"focus\"" ) ",\"app\":" str ",\"app2\":" str
web ::= "\"tool\":\"web\",\"action\":" ( "\"open\"" | "\"search\"" | "\"play\"" ) ",\"site\":" str ",\"query\":" str ",\"browser\":" str
key ::= "\"tool\":\"key\",\"keys\":" str
click ::= "\"tool\":\"click\",\"name\":" str
files ::= "\"tool\":\"files\",\"action\":" ( "\"open\"" | "\"find\"" | "\"open_file\"" | "\"open_folder\"" | "\"create_folder\"" | "\"create_file\"" | "\"space\"" ) ",\"name\":" str ",\"where\":" str
write ::= "\"tool\":\"write\",\"app\":" str ",\"text\":" str
todo ::= "\"tool\":\"todo\",\"items\":[" str ( "," str ){0,9} "]"
git ::= "\"tool\":\"git\",\"action\":" ( "\"status\"" | "\"commit\"" | "\"push\"" | "\"pull\"" | "\"login\"" ) ",\"message\":" str
code ::= "\"tool\":\"code\",\"action\":" ( "\"check\"" | "\"run\"" | "\"build\"" | "\"dictate\"" | "\"install_extension\"" | "\"open_settings\"" | "\"new_project\"" | "\"new_file\"" | "\"write\"" | "\"comment\"" ) ",\"value\":" str
terminal ::= "\"tool\":\"terminal\",\"command\":" str
task ::= "\"tool\":\"task\",\"app\":" str ",\"goal\":" str
close ::= "\"tool\":\"close_app\",\"app\":" str
volume ::= "\"tool\":\"volume\",\"action\":" ( ( "\"up\"" | "\"down\"" ) ( ",\"amount\":" num )? | "\"mute\"" | "\"unmute\"" | "\"set\",\"level\":" num )
brightness ::= "\"tool\":\"brightness\",\"action\":" ( ( "\"up\"" | "\"down\"" ) ( ",\"amount\":" num )? | "\"set\",\"level\":" num )
timeq ::= "\"tool\":\"time\""
dateq ::= "\"tool\":\"date\""
wifi ::= "\"tool\":\"wifi\",\"action\":" ( "\"connect\"" ( ",\"network\":" str )? | "\"list\"" | "\"disconnect\"" )
qr ::= "\"tool\":\"scan_qr\""
numbers ::= "\"tool\":\"numbers\",\"action\":" ( "\"show\"" | "\"hide\"" | "\"click\",\"n\":" num )
typetext ::= "\"tool\":\"type_text\",\"text\":" str
install ::= "\"tool\":\"install_app\",\"app\":" str
mutemic ::= "\"tool\":\"mute_mic\"" ( ",\"minutes\":" num )?
sleep ::= "\"tool\":\"sleep\""
reply ::= "\"tool\":\"reply\",\"text\":" str
ask ::= "\"tool\":\"ask\",\"question\":" str ",\"options\":[" str ( "," str ){1,3} "]"
unknown ::= "\"tool\":\"unknown\""
PLUGINRULES
say ::= "\"say\":" str ","
str ::= "\"" chr{0,160} "\""
chr ::= [^"\\\x00-\x1f!←-⯿\U0001F000-\U0001FAFF] | "\\" ["\\/bfnrt]
num ::= [0-9] [0-9]? [0-9]?
args ::= "{" ( "\"" [a-zA-Z_]{1,32} "\":" ( str | num | "true" | "false" ) ( ",\"" [a-zA-Z_]{1,32} "\":" ( str | num | "true" | "false" ) ){0,5} )? "}"
"#;

pub struct PluginTool {
    pub name: String,
    pub description: String,
}

fn safe_tool(n: &str) -> bool {
    !n.is_empty() && n.len() <= 48 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn grammar(plugins: &[PluginTool]) -> String {
    let tools: Vec<&PluginTool> = plugins.iter().filter(|p| safe_tool(&p.name)).collect();
    if tools.is_empty() {
        return GRAMMAR.replace(" PLUGINS", "").replace("PLUGINRULES\n", "");
    }
    let names = tools.iter().map(|t| format!("\"\\\"{}\\\"\"", t.name)).collect::<Vec<_>>().join(" | ");
    GRAMMAR.replace(" PLUGINS", " | plugin").replace("PLUGINRULES", &format!("plugin ::= \"\\\"tool\\\":\\\"plugin\\\",\\\"name\\\":\" ( {names} ) \",\\\"args\\\":\" args"))
}

pub struct Context<'a> {
    pub apps: &'a [String],
    pub defaults: &'a [(String, String)],
    pub plugins: &'a [PluginTool],
    pub name: Option<&'a str>,
}

pub fn system_prompt(cx: &Context) -> String {
    let mut s = String::from(
        "You turn one spoken request into exactly one JSON tool call for a desktop computer assistant. The request comes from speech recognition, so words may be misheard; choose the most likely meaning.\n\nTools:\n\
open_app(app): start an application. Use the exact name from the installed list when one matches.\n\
close_app(app): quit an application.\n\
volume(action: up|down|mute|unmute|set, amount or level 0-100): speaker and sound volume. \"Mute the sound\" means volume mute.\n\
brightness(action: up|down|set, amount or level 0-100): screen brightness.\n\
time(): say the current time. date(): say today's date.\n\
wifi(action: connect|list|disconnect, network): Wi-Fi networks. Give network only when the user names one.\n\
scan_qr(): read a QR code with the camera.\n\
numbers(action: show|hide|click, n): show numbers on clickable things, or click number n.\n\
type_text(text): type text into the focused field.\n\
install_app(app): download and install an application from the app store.\n\
web(action: open|search|play, site, query, browser): websites. open a site, search a site (empty site means Google), or play a song or video (YouTube unless another site is named). Use play whenever the user wants to hear or watch something. browser is the browser the user named, otherwise empty.\n\
media(action: pause|resume|next|previous|list|mute|unmute, target): music and videos that are playing, in apps like Spotify or in browser tabs like YouTube. target is the app or site the user named, otherwise empty. list says what is playing.\n\
browser(action: new_tab|new_window|private_window|close_tab|switch_tab|open|click|options|save|copy|settings|images|videos|news, target, where: here|tab|window|private): the web browser. target is a website, a tab, or a thing on the page such as \"the third image from the left\" or \"the pricing link\". options shows the right-click menu for it. settings opens the browser settings page named in target. images, videos and news switch the current search to that tab.\n\
window(action: left|right|maximize|minimize|restore|next_screen|desktop|switch|close|side_by_side|focus, app, app2): application windows. left and right snap the active window, or app when named, to that half of the screen. side_by_side puts app on the left and app2 on the right. focus brings app to the front. desktop shows the desktop.\n\
key(keys): press keys in the active app, for example \"ctrl t\", \"pagedown\", \"enter\", \"alt left\".\n\
click(name): click the button or link with this name in the active window.\n\
files(action: open|find|open_file|open_folder|create_folder|create_file|space, name, where): files, folders and drives (C drive, D drive, USB drive, Downloads). create_folder makes a folder called name and create_file makes a file called name, with its extension, inside where: a place such as desktop, documents or D drive, or the name of an existing folder. Never put the whole sentence in name.\n\
write(app, text): open the app, or use it when it is open, and type text into it. text is exactly what the user wants written, with line breaks kept.\n\
todo(items): make a to-do list; items are the separate tasks the user listed, each short.\n\
git(action: status|commit|push|pull|login, message): Git in the current project; message is the commit message.\n\
code(action: check|run|build|dictate|install_extension|open_settings|new_project|new_file|write|comment, value): the user's coding app (VS Code, Visual Studio, JetBrains IDEs, Xcode, Eclipse and others). check finds mistakes in the open file, run runs it, build builds the open project, dictate starts coding by voice, install_extension installs the extension named in value, new_project creates a project folder named in value and opens it, new_file creates the file named in value in the open project, write writes the code described in value into the open file, comment adds the comment text in value.\n\
terminal(command): run a command in a terminal.\n\
task(app, goal): a task with several steps inside an app, such as replying to an email in Gmail. Use only when no other tool fits.\n\
mute_mic(minutes): turn the microphone off. Only when the user says microphone or mic.\n\
sleep(): stop listening until the wake phrase is said again.\n\
reply(text): answer a question or talk briefly, in at most two short plain sentences without emoji.\n\
ask(question, options): ask the user to choose when the request matches two to four installed apps equally well. Options must be installed apps.\n\
unknown(): the request is not something you can do.\n\
\nWhen the user also talks to you while asking for something, such as greeting you, asking how you are, or telling you about their day or how they feel, start the JSON with \"say\": one or two short, warm, plain spoken sentences that answer them and say what you are about to do. Use say only for that; never for a plain command, and never with reply.\n",
    );
    let who = cx.name.map(|n| n.chars().filter(|c| c.is_alphabetic() || *c == ' ' || *c == '\'').take(32).collect::<String>()).filter(|n| !n.trim().is_empty());
    match &who {
        Some(n) => s.push_str(&format!("The user's name is {n}. Greet them by it in say or reply when it fits.\n")),
        None => s.push_str("Greet the user as friend in say or reply when it fits.\n"),
    }
    let me = who.as_deref().unwrap_or("friend");
    for p in cx.plugins.iter().filter(|p| safe_tool(&p.name)) {
        let d: String = p.description.chars().filter(|c| !c.is_control()).take(160).collect();
        s.push_str(&format!("plugin(name: {}, args): {}\n", p.name, d));
    }
    let apps: Vec<String> = cx.apps.iter().take(60).map(|a| a.chars().filter(|c| !c.is_control()).take(48).collect()).collect();
    s.push_str(&format!("\nInstalled apps: {}.\n", if apps.is_empty() { "none found".into() } else { apps.join(", ") }));
    if !cx.defaults.is_empty() {
        let d: Vec<String> = cx.defaults.iter().map(|(r, a)| format!("{r}={a}")).collect();
        s.push_str(&format!("Default apps: {}.\n", d.join(", ")));
    }
    let browser = cx.defaults.iter().find(|(r, _)| r == "browser").map(|(_, a)| a.as_str()).unwrap_or("Firefox");
    let mail = cx.defaults.iter().find(|(r, _)| r == "mail").map(|(_, a)| a.as_str()).unwrap_or("Thunderbird");
    s.push_str(&format!(
        "\nExamples:\nopen the browser -> {{\"tool\":\"open_app\",\"app\":\"{browser}\"}}\n\
turn it up a bit -> {{\"tool\":\"volume\",\"action\":\"up\",\"amount\":10}}\n\
set volume to forty percent -> {{\"tool\":\"volume\",\"action\":\"set\",\"level\":40}}\n\
mute the sound -> {{\"tool\":\"volume\",\"action\":\"mute\"}}\n\
dim the screen -> {{\"tool\":\"brightness\",\"action\":\"down\",\"amount\":20}}\n\
connect to wifi -> {{\"tool\":\"wifi\",\"action\":\"connect\"}}\n\
write an email -> {{\"tool\":\"open_app\",\"app\":\"{mail}\"}}\n\
what's the capital of France -> {{\"tool\":\"reply\",\"text\":\"The capital of France is Paris.\"}}\n\
ugh, work was long today, i want to listen to something that eases my mood -> {{\"say\":\"Sorry the day was long, {me}. Here's something to ease your mood.\",\"tool\":\"web\",\"action\":\"play\",\"site\":\"YouTube\",\"query\":\"mood lifting songs\",\"browser\":\"\"}}\n\
hi setu, i'm so tired today, open my email -> {{\"say\":\"Sorry it's been a tiring day, {me}. Opening your email.\",\"tool\":\"open_app\",\"app\":\"{mail}\"}}\n\
good morning setu, how are you -> {{\"tool\":\"reply\",\"text\":\"Good morning, {me}. I'm doing well, thanks. What shall we do today?\"}}\n\
hey setu, how are you doing today -> {{\"tool\":\"reply\",\"text\":\"I'm doing great, {me}, thanks for asking. How about you?\"}}\n\
my day was really good today -> {{\"tool\":\"reply\",\"text\":\"That's lovely to hear, {me}. What made it good?\"}}\n\
install blender -> {{\"tool\":\"install_app\",\"app\":\"Blender\"}}\n\
i feel like hearing some old lata mangeshkar songs -> {{\"tool\":\"web\",\"action\":\"play\",\"site\":\"YouTube\",\"query\":\"old lata mangeshkar songs\",\"browser\":\"\"}}\n\
in brave open youtube and play some arijit singh -> {{\"tool\":\"web\",\"action\":\"play\",\"site\":\"YouTube\",\"query\":\"arijit singh\",\"browser\":\"Brave\"}}\n\
find cheap flights to goa -> {{\"tool\":\"web\",\"action\":\"search\",\"site\":\"\",\"query\":\"cheap flights to goa\",\"browser\":\"\"}}\n\
show me what's on my d drive -> {{\"tool\":\"files\",\"action\":\"open\",\"name\":\"\",\"where\":\"d drive\"}}\n\
save my work to git saying added the login page -> {{\"tool\":\"git\",\"action\":\"commit\",\"message\":\"added the login page\"}}\n\
is there a bug in my program -> {{\"tool\":\"code\",\"action\":\"check\",\"value\":\"\"}}\n\
in gmail open the latest email from amazon -> {{\"tool\":\"task\",\"app\":\"browser\",\"goal\":\"in gmail open the latest email from amazon\"}}\n\
hold the song on spotify -> {{\"tool\":\"media\",\"action\":\"pause\",\"target\":\"spotify\"}}\n\
open the second picture in another tab -> {{\"tool\":\"browser\",\"action\":\"open\",\"target\":\"the second image\",\"where\":\"tab\"}}\n\
what can i do with the fourth image -> {{\"tool\":\"browser\",\"action\":\"options\",\"target\":\"the fourth image\",\"where\":\"here\"}}\n\
put chrome and my code editor next to each other -> {{\"tool\":\"window\",\"action\":\"side_by_side\",\"app\":\"Google Chrome\",\"app2\":\"Visual Studio Code\"}}\n\
get this window out of the way -> {{\"tool\":\"window\",\"action\":\"minimize\",\"app\":\"\",\"app2\":\"\"}}\n\
make me a fresh project for my weather app -> {{\"tool\":\"code\",\"action\":\"new_project\",\"value\":\"weather app\"}}\n\
add a file for the login page called login.html -> {{\"tool\":\"code\",\"action\":\"new_file\",\"value\":\"login.html\"}}\n\
make a folder for my receipts in documents -> {{\"tool\":\"files\",\"action\":\"create_folder\",\"name\":\"receipts\",\"where\":\"documents\"}}\n\
i need a python file named scraper inside the tutorial folder -> {{\"tool\":\"files\",\"action\":\"create_file\",\"name\":\"scraper.py\",\"where\":\"tutorial\"}}\n\
jot down in notepad that the meeting moved to friday -> {{\"tool\":\"write\",\"app\":\"Notepad\",\"text\":\"The meeting moved to Friday\"}}\n\
my plan for today is gym at 7, then emails, then lunch with priya, make it a list -> {{\"tool\":\"todo\",\"items\":[\"Gym at 7\",\"Emails\",\"Lunch with Priya\"]}}\n\
on this search show me pictures instead -> {{\"tool\":\"browser\",\"action\":\"images\",\"target\":\"\",\"where\":\"here\"}}\n\
i need a loop that prints numbers one to ten -> {{\"tool\":\"code\",\"action\":\"write\",\"value\":\"a loop that prints numbers one to ten\"}}\n\
order me a pizza -> {{\"tool\":\"unknown\"}}\n"
    ));
    s
}

pub fn user_turn(template: &str, text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).take(400).collect();
    crate::chat::user(template, &clean)
}

fn words_of(t: &str) -> Vec<String> {
    crate::text::normalize(t).split_whitespace().filter(|w| w.chars().count() >= 3).map(|w| w.trim_end_matches('s').to_owned()).collect()
}

pub fn grounded(intent: &Intent, request: &str) -> bool {
    if matches!(intent, Intent::MuteMic { .. }) {
        let t = format!(" {} ", crate::text::normalize(request));
        return [" mic ", " mike ", " microphone ", " mics ", " microphones "].iter().any(|w| t.contains(w));
    }
    let free = match intent {
        Intent::TypeText { text } => Some(text.clone()),
        Intent::Act { tool, args } if tool.starts_with("web.") => args.get("query").and_then(Value::as_str).filter(|q| !q.trim().is_empty()).map(str::to_owned),
        _ => None,
    };
    let Some(free) = free else { return true };
    let heard = words_of(request);
    let want = words_of(&free);
    want.is_empty() || want.iter().any(|w| heard.iter().any(|h| h == w || (w.len() >= 4 && h.len() >= 4 && (h.starts_with(w.as_str()) || w.starts_with(h.as_str())))))
}

pub fn said(out: &str) -> Option<String> {
    let v: Value = serde_json::from_str(out.trim()).ok()?;
    let t = v.get("say").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty())?;
    if v.get("tool").and_then(Value::as_str).is_some_and(|t| matches!(t, "reply" | "unknown")) {
        return None;
    }
    Some(crate::text::plain_reply(t))
}

pub fn parse_model(out: &str) -> Intent {
    let Ok(v) = serde_json::from_str::<Value>(out.trim()) else { return Intent::Unknown };
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(|x| x.trim().to_owned()).filter(|x| !x.is_empty());
    let n = |k: &str| v.get(k).and_then(Value::as_i64);
    let act = |opts: &[&'static str]| v.get("action").and_then(Value::as_str).and_then(|a| opts.iter().find(|o| **o == a).copied());
    match v.get("tool").and_then(Value::as_str).unwrap_or("") {
        "open_app" => s("app").map(|t| Intent::OpenApp { target: t, role: None }).unwrap_or(Intent::Unknown),
        "close_app" => s("app").map(|t| Intent::CloseApp { target: t }).unwrap_or(Intent::Unknown),
        "volume" => act(&["up", "down", "mute", "unmute", "set"])
            .map(|a| Intent::Volume { action: a, amount: n("amount").map(|x| x.clamp(1, 100)), level: n("level").map(|x| x.clamp(0, 100)) })
            .unwrap_or(Intent::Unknown),
        "brightness" => act(&["up", "down", "set"])
            .map(|a| Intent::Brightness { action: a, amount: n("amount").map(|x| x.clamp(1, 100)), level: n("level").map(|x| x.clamp(0, 100)) })
            .unwrap_or(Intent::Unknown),
        "time" => Intent::Time,
        "date" => Intent::Date,
        "wifi" => act(&["connect", "list", "disconnect"]).map(|a| Intent::Wifi { action: a, network: s("network") }).unwrap_or(Intent::Unknown),
        "scan_qr" => Intent::ScanQr,
        "numbers" => act(&["show", "hide", "click"]).map(|a| Intent::Numbers { action: a, n: n("n") }).unwrap_or(Intent::Unknown),
        "type_text" => s("text").map(|t| Intent::TypeText { text: t }).unwrap_or(Intent::Unknown),
        "install_app" => s("app").map(|t| Intent::Install { app: t }).unwrap_or(Intent::Unknown),
        "mute_mic" => Intent::MuteMic { minutes: n("minutes").map(|m| m.clamp(1, 1440)) },
        "sleep" => Intent::Sleep,
        "reply" => s("text").map(|t| Intent::Reply { text: crate::text::plain_reply(&t) }).unwrap_or(Intent::Unknown),
        "ask" => {
            let options: Vec<String> = v.get("options").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).take(4).collect()).unwrap_or_default();
            match s("question") {
                Some(q) if options.len() >= 2 => Intent::Ask { question: q, options },
                _ => Intent::Unknown,
            }
        }
        "web" => {
            let action = act(&["open", "search", "play"]).unwrap_or("open");
            let site = s("site").unwrap_or_default();
            let query = s("query").unwrap_or_default();
            let browser = s("browser").filter(|b| !matches!(b.to_lowercase().as_str(), "default" | "browser" | "none"));
            let mut args = json!({ "site": site, "query": query, "browser": browser });
            if action == "open" && crate::tools::web::site(&site).is_none() {
                match crate::tools::web::spoken_url(&site).or_else(|| crate::tools::web::spoken_url(&query)) {
                    Some(u) => args["url"] = json!(u),
                    None if !query.is_empty() || !site.is_empty() => return Intent::Act { tool: "web.search".into(), args: json!({ "query": if query.is_empty() { site } else { query }, "browser": args["browser"] }) },
                    None => {}
                }
            } else if let Some(si) = crate::tools::web::site(&site) {
                args["site"] = json!(si.name);
            }
            Intent::Act { tool: format!("web.{action}"), args }
        }
        "media" => {
            let tool = match act(&["pause", "resume", "next", "previous", "list", "mute", "unmute"]) {
                Some("resume") => "media.play",
                Some(a) => return Intent::Act { tool: format!("media.{a}"), args: json!({ "target": s("target"), "verb": a }) },
                None => return Intent::Unknown,
            };
            Intent::Act { tool: tool.into(), args: json!({ "target": s("target"), "verb": "play" }) }
        }
        "browser" if matches!(v.get("action").and_then(Value::as_str), Some("images" | "videos" | "news")) => Intent::Act { tool: "page.vertical".into(), args: json!({ "kind": v.get("action").and_then(Value::as_str).unwrap_or("images") }) },
        "write" => match (s("app"), v.get("text").and_then(Value::as_str).map(str::to_owned).filter(|t| !t.trim().is_empty())) {
            (Some(app), Some(text)) => Intent::Act { tool: "app.write".into(), args: json!({ "app": app, "text": text }) },
            _ => Intent::Unknown,
        },
        "todo" => {
            let items: Vec<String> = v.get("items").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).filter(|x| !x.trim().is_empty()).collect()).unwrap_or_default();
            if items.is_empty() { Intent::Unknown } else { Intent::Act { tool: "board.todo".into(), args: json!({ "op": "many", "items": items, "text": "" }) } }
        }
        "browser" => crate::browse::from_model(act(&["new_tab", "new_window", "private_window", "close_tab", "switch_tab", "open", "click", "options", "save", "copy", "settings"]).unwrap_or(""), &s("target").unwrap_or_default(), v.get("where").and_then(Value::as_str).unwrap_or("here")),
        "window" => {
            let app = s("app").unwrap_or_default();
            let app2 = s("app2").unwrap_or_default();
            let place = |w: &str| Intent::Act { tool: "wm.place".into(), args: json!({ "where": w, "app": app }) };
            match act(&["left", "right", "maximize", "minimize", "restore", "next_screen", "desktop", "switch", "close", "side_by_side", "focus"]) {
                Some("left") => place("left"),
                Some("right") => place("right"),
                Some("maximize") => place("max"),
                Some("minimize") => place("min"),
                Some("restore") => place("restore"),
                Some("next_screen") => place("next_screen"),
                Some("desktop") => Intent::Act { tool: "wm.desktop".into(), args: json!({}) },
                Some("switch") => Intent::Act { tool: "wm.switch".into(), args: json!({}) },
                Some("close") => Intent::Act { tool: "wm.close".into(), args: json!({}) },
                Some("side_by_side") => Intent::Act { tool: "wm.side".into(), args: json!({ "left": app, "right": app2 }) },
                Some("focus") if !app.is_empty() => Intent::Act { tool: "wm.focus".into(), args: json!({ "app": app }) },
                _ => Intent::Unknown,
            }
        }
        "key" => s("keys").map(|k| Intent::Act { tool: "key".into(), args: json!({ "keys": k }) }).unwrap_or(Intent::Unknown),
        "click" => s("name").map(|n| Intent::Act { tool: "ui.click".into(), args: json!({ "name": n }) }).unwrap_or(Intent::Unknown),
        "files" => {
            if let Some(kind @ ("create_folder" | "create_file")) = act(&["create_folder", "create_file"]) {
                let name = s("name").unwrap_or_default();
                if name.is_empty() || name.split_whitespace().count() > 6 {
                    return Intent::Unknown;
                }
                let place = s("where").unwrap_or_default();
                let known = !place.is_empty() && (crate::tools::files::location(&place).is_some() || matches!(crate::text::normalize(&place).trim_start_matches("the ").trim_start_matches("my "), "desktop" | "documents" | "downloads" | "projects" | "home" | "pictures" | "music" | "videos"));
                let folder = kind == "create_folder";
                let code = !folder && crate::understand::is_code_file(&name);
                return Intent::Act {
                    tool: "files.create".into(),
                    args: json!({ "items": [{ "kind": if folder { "folder" } else { "file" }, "path": [name] }], "where": if known { place.clone() } else { String::new() }, "inside": if known { String::new() } else { place }, "code": code, "language": "" }),
                };
            }
            let tool = match act(&["open", "find", "open_file", "open_folder", "create_folder", "space"]) {
                Some("open") => "files.open",
                Some("find") => "files.find",
                Some("open_file") => "files.openfile",
                Some("open_folder") => "files.openfolder",
                Some("create_folder") => "files.mkdir",
                Some("space") => "files.space",
                _ => return Intent::Unknown,
            };
            let name = s("name").unwrap_or_default();
            let place = s("where").unwrap_or_default();
            if tool == "files.open" && place.is_empty() && name.is_empty() {
                return Intent::Unknown;
            }
            let place = if tool == "files.open" && place.is_empty() { name.clone() } else { place };
            Intent::Act { tool: tool.into(), args: json!({ "name": name, "where": if place.is_empty() { Value::Null } else { json!(place) } }) }
        }
        "git" => match act(&["status", "commit", "push", "pull", "login"]) {
            Some(a) => Intent::Act { tool: format!("git.{a}"), args: json!({ "message": s("message").unwrap_or_default() }) },
            None => Intent::Unknown,
        },
        "code" => {
            let value = s("value").unwrap_or_default();
            match act(&["check", "run", "build", "dictate", "install_extension", "open_settings", "new_project", "new_file", "write", "comment"]) {
                Some("install_extension") if !value.is_empty() => Intent::Act { tool: "code.extension".into(), args: json!({ "name": value }) },
                Some("open_settings") if !value.is_empty() => Intent::Act { tool: "code.settings".into(), args: json!({ "query": value }) },
                Some("open_settings") => Intent::Act { tool: "code.command".into(), args: json!({ "id": "workbench.action.openSettings", "label": "Opened VS Code settings" }) },
                Some(a @ ("check" | "run" | "build" | "dictate")) => Intent::Act { tool: format!("code.{a}"), args: json!({}) },
                Some("new_project") => Intent::Act { tool: "code.project".into(), args: json!({ "name": value, "language": "" }) },
                Some("new_file") => Intent::Act { tool: "code.new_file".into(), args: json!({ "name": value.replace(' ', "_"), "folder": "" }) },
                Some("write") if !value.is_empty() => Intent::Act { tool: "code.write".into(), args: json!({ "request": value }) },
                Some("comment") if !value.is_empty() => Intent::Act { tool: "code.comment".into(), args: json!({ "text": value }) },
                _ => Intent::Unknown,
            }
        }
        "terminal" => s("command").map(|c| Intent::Act { tool: "terminal.run".into(), args: json!({ "command": c }) }).unwrap_or(Intent::Unknown),
        "task" => match s("goal") {
            Some(g) => Intent::Act { tool: "task".into(), args: json!({ "goal": g, "app": s("app") }) },
            None => Intent::Unknown,
        },
        "plugin" => match s("name") {
            Some(tool) => Intent::Plugin { tool, args: v.get("args").cloned().unwrap_or(json!({})) },
            None => Intent::Unknown,
        },
        _ => Intent::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(t: &str) -> Option<Intent> {
        rules(t, "stop", false)
    }

    #[test]
    fn routes_common_commands() {
        assert_eq!(r("open firefox"), Some(Intent::OpenApp { target: "firefox".into(), role: None }));
        assert_eq!(r("can you open the browser please"), Some(Intent::OpenApp { target: "browser".into(), role: Some("browser") }));
        assert_eq!(r("launch v s code"), Some(Intent::OpenApp { target: "v s code".into(), role: None }));
        assert_eq!(r("close spotify"), Some(Intent::CloseApp { target: "spotify".into() }));
        assert_eq!(r("volume up"), Some(Intent::Volume { action: "up", amount: None, level: None }));
        assert_eq!(r("turn the volume down by twenty"), Some(Intent::Volume { action: "down", amount: Some(20), level: None }));
        assert_eq!(r("set volume to forty percent"), Some(Intent::Volume { action: "set", amount: None, level: Some(40) }));
        assert_eq!(r("make it louder"), Some(Intent::Volume { action: "up", amount: None, level: None }));
        assert_eq!(r("mute the sound"), Some(Intent::Volume { action: "mute", amount: None, level: None }));
        assert_eq!(r("mute the microphone for half an hour"), Some(Intent::MuteMic { minutes: Some(30) }));
        assert_eq!(r("dim the screen"), Some(Intent::Brightness { action: "down", amount: None, level: None }));
        assert_eq!(r("what time is it"), Some(Intent::Time));
        assert_eq!(r("what time is i"), Some(Intent::Time));
        assert_eq!(r("what day is it today"), Some(Intent::Date));
        assert_eq!(r("what time does the store close on sunday evening"), None);
        assert_eq!(r("What's the date?"), Some(Intent::Date));
        assert_eq!(r("connect to wifi"), Some(Intent::Wifi { action: "connect", network: None }));
        assert_eq!(r("connect to the network called home five g"), Some(Intent::Wifi { action: "connect", network: Some("home five g".into()) }));
        assert_eq!(r("scan the qr code"), Some(Intent::ScanQr));
        assert_eq!(r("show numbers"), Some(Intent::Numbers { action: "show", n: None }));
        assert_eq!(r("click number seven"), Some(Intent::Numbers { action: "click", n: Some(7) }));
        assert_eq!(rules("seven", "stop", true), Some(Intent::Numbers { action: "click", n: Some(7) }));
        assert_eq!(r("go to sleep"), Some(Intent::Sleep));
        assert_eq!(r("stop"), Some(Intent::Cancel));
        assert_eq!(rules("halt", "halt", false), Some(Intent::Cancel));
        assert_eq!(r("no I said open chrome"), Some(Intent::Correct { text: "open chrome".into() }));
        assert_eq!(r("install gimp"), Some(Intent::Install { app: "Gimp".into() }));
        assert_eq!(r("how far is the moon"), None);
        assert_eq!(r("start a new email"), None);
    }

    #[test]
    fn a_spoken_line_rides_along_with_the_tool() {
        let out = r#"{"say":"Hey Anoop, I'm good. Here's something calm.","tool":"web","action":"play","site":"YouTube","query":"calm songs","browser":""}"#;
        assert_eq!(said(out).as_deref(), Some("Hey Anoop, I'm good. Here's something calm."));
        assert_ne!(parse_model(out), Intent::Unknown);
        assert_eq!(said(r#"{"tool":"open_app","app":"Firefox"}"#), None);
        assert_eq!(said(r#"{"say":"Sure.","tool":"reply","text":"Paris."}"#), None);
        assert!(GRAMMAR.starts_with(r#"root ::= "{" say? ("#) && GRAMMAR.contains(r#"say ::= "\"say\":" str ",""#));
        let p = system_prompt(&Context { apps: &[], defaults: &[], plugins: &[], name: Some("Anoop") });
        assert!(p.contains("The user's name is Anoop.") && p.contains("Sorry the day was long, Anoop."));
    }

    #[test]
    fn model_arguments_must_come_from_what_was_said() {
        let copied = Intent::Act { tool: "web.play".into(), args: json!({ "query": "mood lifting songs", "site": "YouTube" }) };
        assert!(!grounded(&copied, "hey setu, how are you doing today"));
        assert!(grounded(&copied, "so what i want to do is listen to songs that ease my mood"));
        assert!(grounded(&Intent::Act { tool: "web.play".into(), args: json!({ "query": "arijit singh" }) }, "in brave open youtube and play some arijit singh"));
        assert!(grounded(&Intent::OpenApp { target: "Firefox".into(), role: None }, "open the browser"));
        assert!(!grounded(&Intent::MuteMic { minutes: Some(1) }, "good evening setu, i'm so tired today, mute the sound"));
        assert!(grounded(&Intent::MuteMic { minutes: None }, "mute my mic for a bit"));
    }

    #[test]
    fn parses_model_output() {
        assert_eq!(parse_model(r#"{"tool":"open_app","app":"Firefox"}"#), Intent::OpenApp { target: "Firefox".into(), role: None });
        assert_eq!(parse_model(r#"{"tool":"volume","action":"set","level":400}"#), Intent::Volume { action: "set", amount: None, level: Some(100) });
        assert_eq!(parse_model(r#"{"tool":"rm_rf"}"#), Intent::Unknown);
        assert_eq!(parse_model("not json"), Intent::Unknown);
        let tool = |o: &str| match parse_model(o) {
            Intent::Act { tool, args } => (tool, args),
            other => (format!("{other:?}"), Value::Null),
        };
        assert_eq!(tool(r#"{"tool":"code","action":"write","value":"a loop from one to ten"}"#).0, "code.write");
        assert_eq!(tool(r#"{"tool":"code","action":"new_project","value":"weather app"}"#).1["name"], "weather app");
        assert_eq!(tool(r#"{"tool":"code","action":"new_file","value":"login.html"}"#), ("code.new_file".into(), json!({ "name": "login.html", "folder": "" })));
        assert_eq!(tool(r#"{"tool":"code","action":"comment","value":"adds numbers"}"#).0, "code.comment");
        assert_eq!(tool(r#"{"tool":"window","action":"side_by_side","app":"Chrome","app2":"Code"}"#).1["right"], "Code");
        assert_eq!(tool(r#"{"tool":"window","action":"minimize","app":"","app2":""}"#).1["where"], "min");
        assert_eq!(parse_model(r#"{"tool":"window","action":"focus","app":"","app2":""}"#), Intent::Unknown);
        assert_eq!(parse_model(r#"{"tool":"ask","question":"Which?","options":["A"]}"#), Intent::Unknown);
    }

    #[test]
    fn self_references() {
        let names = |n: &str| vec![n.to_owned()];
        let setu = names("Setu");
        assert_eq!(self_page("yourself", &setu), Some("home"));
        assert_eq!(self_page("your soul", &setu), Some("home"));
        assert_eq!(self_page("your self", &setu), Some("home"));
        assert_eq!(self_page("setu", &setu), Some("home"));
        assert_eq!(self_page("the setu app", &setu), Some("home"));
        assert_eq!(self_page("setu ui", &setu), Some("home"));
        assert_eq!(self_page("setu dashboard", &setu), Some("home"));
        assert_eq!(self_page("dashboard", &setu), Some("home"));
        assert_eq!(self_page("setu settings", &setu), Some("settings/voice"));
        assert_eq!(self_page("your settings", &setu), Some("settings/voice"));
        assert_eq!(self_page("nova", &names("Nova")), Some("home"));
        assert_eq!(self_page("setu app", &names("Nova")), Some("home"));
        assert_eq!(self_page("say to app", &["Setu".into(), "say to".into()]), Some("home"));
        assert_eq!(self_page("settings", &setu), None);
        assert_eq!(self_page("firefox", &setu), None);
        assert_eq!(self_page("spotify", &setu), None);
        assert_eq!(self_page("calculator", &setu), None);
        assert_eq!(self_page("terminal", &setu), None);
        assert_eq!(self_page("", &setu), None);
        assert_eq!(rules("show yourself", "stop", false), Some(Intent::OpenApp { target: "yourself".into(), role: None }));
        assert_eq!(rules("close the setu app", "stop", false), Some(Intent::CloseApp { target: "setu".into() }));
        assert_eq!(rules("close the application", "stop", false), Some(Intent::CloseApp { target: String::new() }));
    }

    #[test]
    fn grammar_includes_plugins_only_when_present() {
        assert!(!grammar(&[]).contains("plugin"));
        let g = grammar(&[PluginTool { name: "weather_now".into(), description: "Current weather".into() }, PluginTool { name: "bad name".into(), description: String::new() }]);
        assert!(g.contains("weather_now"));
        assert!(!g.contains("bad name"));
    }
}
