use crate::core::Core;
use crate::router::Intent;
use crate::text::{normalize, sentence_case};
use crate::tools::media::{browser_display, browser_of};
use crate::tools::{Choice, FollowUp, Outcome, browsers, web};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const LAYER: &str = "browser";

pub static PAGE_NUMBERS: AtomicBool = AtomicBool::new(false);

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.to_owned(), args }
}

fn strip<'a>(t: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| t.strip_prefix(p)).map(str::trim).filter(|r| !r.is_empty())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ref {
    pub kind: Option<&'static str>,
    pub n: Option<i64>,
    pub from: Option<&'static str>,
    pub row: Option<i64>,
    pub text: Option<String>,
    pub this: bool,
    pub number: Option<i64>,
}

impl Ref {
    pub fn json(&self) -> Value {
        json!({ "kind": self.kind.unwrap_or("any"), "n": self.n, "from": self.from, "row": self.row, "text": self.text, "this": self.this, "number": self.number })
    }

    fn noun(&self) -> &'static str {
        match self.kind {
            Some("image") => "image",
            Some("link") => "link",
            Some("result") => "result",
            Some("video") => "video",
            Some("button") => "button",
            Some("field") => "field",
            _ => "item",
        }
    }

    pub fn describe(&self) -> String {
        let noun = self.noun();
        if self.this {
            return format!("this {noun}");
        }
        if let Some(t) = &self.text {
            return if self.kind.is_some() { format!("the {t} {noun}") } else { t.clone() };
        }
        let ord = self.n.or(self.number).map(ordinal_name).unwrap_or_default();
        let from = self.from.map(|f| format!(" from the {f}")).unwrap_or_default();
        let row = self.row.map(|r| format!(" in the {} row", ordinal_name(r))).unwrap_or_default();
        format!("the {ord} {noun}{from}{row}").replace("  ", " ").replace("the  ", "the ")
    }
}

fn ordinal_name(n: i64) -> String {
    match n {
        -1 => "last".into(),
        -2 => "second last".into(),
        1 => "first".into(),
        2 => "second".into(),
        3 => "third".into(),
        4 => "fourth".into(),
        5 => "fifth".into(),
        6 => "sixth".into(),
        7 => "seventh".into(),
        8 => "eighth".into(),
        9 => "ninth".into(),
        10 => "tenth".into(),
        n => format!("number {n}"),
    }
}

fn ordinal(w: &str) -> Option<i64> {
    const WORDS: &[&str] = &["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth", "eleventh", "twelfth"];
    if let Some(i) = WORDS.iter().position(|x| *x == w) {
        return Some(i as i64 + 1);
    }
    if w == "last" || w == "final" {
        return Some(-1);
    }
    let digits: String = w.chars().take_while(char::is_ascii_digit).collect();
    let rest = &w[digits.len()..];
    if !digits.is_empty() && matches!(rest, "st" | "nd" | "rd" | "th") {
        return digits.parse().ok();
    }
    None
}

fn cardinal(w: &str) -> Option<i64> {
    if let Ok(n) = w.parse::<i64>() {
        return Some(n);
    }
    crate::text::find_number(w).filter(|_| !matches!(w, "a" | "an" | "to" | "too" | "for"))
}

const KINDS: &[(&str, &str)] = &[
    ("search results", "result"),
    ("search result", "result"),
    ("text box", "field"),
    ("text field", "field"),
    ("search box", "field"),
    ("images", "image"),
    ("image", "image"),
    ("pictures", "image"),
    ("picture", "image"),
    ("photos", "image"),
    ("photo", "image"),
    ("pics", "image"),
    ("pic", "image"),
    ("thumbnails", "image"),
    ("thumbnail", "image"),
    ("img", "image"),
    ("links", "link"),
    ("link", "link"),
    ("hyperlink", "link"),
    ("results", "result"),
    ("result", "result"),
    ("videos", "video"),
    ("video", "video"),
    ("buttons", "button"),
    ("button", "button"),
    ("fields", "field"),
    ("field", "field"),
];

const LINKERS: &[&str] = &["that says", "which says", "saying", "that reads", "named", "called", "titled", "labelled", "labeled", "with the text", "with text", "with the name", "of", "about", "showing", "with", "for", "that is", "which is", "that", "which", "on"];

pub fn parse_ref(s: &str) -> Option<Ref> {
    let s = s.trim().trim_end_matches(" please").trim();
    let mut r = Ref::default();
    if matches!(s, "it" | "this" | "that" | "this one" | "that one" | "the one" | "here" | "this thing" | "that thing") {
        r.this = true;
        return Some(r);
    }
    let mut w: Vec<String> = s.split_whitespace().map(str::to_owned).collect();
    let mut i = 0;
    while i < w.len() {
        if w[i] == "row" {
            if i >= 1
                && let Some(n) = ordinal(&w[i - 1])
            {
                r.row = Some(n);
                let mut start = i - 1;
                while start > 0 && matches!(w[start - 1].as_str(), "the" | "in" | "on" | "of" | "from") {
                    start -= 1;
                }
                w.drain(start..=i);
                i = start;
                continue;
            }
            if i + 1 < w.len()
                && let Some(n) = cardinal(&w[i + 1])
            {
                r.row = Some(n);
                let mut start = i;
                while start > 0 && matches!(w[start - 1].as_str(), "the" | "in" | "on" | "of" | "from") {
                    start -= 1;
                }
                w.drain(start..=i + 1);
                i = start;
                continue;
            }
        }
        i += 1;
    }
    let mut i = 0;
    while i < w.len() {
        let side = match w[i].as_str() {
            "left" | "leftmost" => Some("left"),
            "right" | "rightmost" => Some("right"),
            "top" | "topmost" => Some("top"),
            "bottom" | "bottommost" => Some("bottom"),
            _ => None,
        };
        if let Some(side) = side {
            let most = w[i].ends_with("most") || i + 1 < w.len() && w[i + 1] == "most";
            let mut start = i;
            while start > 0 && matches!(w[start - 1].as_str(), "the" | "from" | "on" | "at" | "to") {
                start -= 1;
            }
            let end = if i + 1 < w.len() && w[i + 1] == "most" { i + 1 } else { i };
            let from_word = start < i && w[start..i].iter().any(|x| x == "from");
            let next_is_kind = end + 1 < w.len() && KINDS.iter().any(|(k, _)| *k == w[end + 1]);
            if from_word || most || next_is_kind || end + 1 == w.len() {
                if r.from.is_none() {
                    r.from = Some(side);
                }
                if (most || next_is_kind && !from_word) && r.n.is_none() {
                    r.n = Some(1);
                    if side == "top" || side == "bottom" {
                        r.from = Some(side);
                    }
                }
                w.drain(start..=end);
                i = start;
                continue;
            }
        }
        i += 1;
    }
    let mut kind_at = None;
    'outer: for (k, kind) in KINDS {
        let kw: Vec<&str> = k.split_whitespace().collect();
        for start in 0..w.len() {
            if start + kw.len() <= w.len() && w[start..start + kw.len()].iter().zip(&kw).all(|(a, b)| a == b) {
                if let Some(next) = w.get(start + kw.len()) {
                    let tail_ok = LINKERS.iter().any(|l| l.split_whitespace().next() == Some(next.as_str()))
                        || matches!(next.as_str(), "number" | "in" | "at" | "from" | "here" | "on" | "please" | "of")
                        || cardinal(next).is_some()
                        || ordinal(next).is_some();
                    if !tail_ok {
                        return None;
                    }
                }
                r.kind = Some(kind);
                w.drain(start..start + kw.len());
                kind_at = Some(start);
                break 'outer;
            }
        }
    }
    let mut i = 0;
    while i < w.len() {
        if i + 2 < w.len() && w[i] == "second" && (w[i + 1] == "last" || w[i + 1] == "to" && w[i + 2] == "last") {
            r.n = Some(-2);
            let end = if w[i + 1] == "to" { i + 2 } else { i + 1 };
            w.drain(i..=end);
            kind_at = kind_at.map(|k| if k > i { k.saturating_sub(end + 1 - i) } else { k });
            continue;
        }
        if i + 1 < w.len() && w[i] == "second" && w[i + 1] == "last" {
            r.n = Some(-2);
            w.drain(i..=i + 1);
            continue;
        }
        if let Some(n) = ordinal(&w[i])
            && r.n.is_none()
        {
            r.n = Some(n);
            w.remove(i);
            if kind_at.is_some_and(|k| k > i) {
                kind_at = kind_at.map(|k| k - 1);
            }
            continue;
        }
        if w[i] == "number"
            && i + 1 < w.len()
            && let Some(n) = cardinal(&w[i + 1])
        {
            r.number = Some(n);
            w.drain(i..=i + 1);
            continue;
        }
        i += 1;
    }
    if r.number.is_none()
        && let Some(k) = kind_at
        && k < w.len()
        && let Some(n) = cardinal(&w[k])
        && w.len() == k + 1
    {
        r.number = Some(n);
        w.remove(k);
    }
    if r.kind.is_none() && r.number.is_none() && w.len() == 1 && w[0].chars().all(|c| c.is_ascii_digit()) {
        r.number = w[0].parse().ok();
        w.clear();
    }
    let mut words: Vec<String> = w.into_iter().filter(|x| !matches!(x.as_str(), "the" | "a" | "an" | "one" | "please" | "on" | "screen" | "page" | "here")).collect();
    let deictic = words.first().is_some_and(|x| x == "this" || x == "that") && !words.get(1).is_some_and(|x| matches!(x.as_str(), "says" | "reads" | "is" | "shows" | "has" | "contains" | "goes" | "links" | "points" | "mentions"));
    if deictic {
        if words.len() == 1 {
            r.this = true;
        }
        words.remove(0);
    }
    let mut text = words.join(" ");
    loop {
        let before = text.clone();
        for l in LINKERS {
            if let Some(rest) = text.strip_prefix(&format!("{l} ")) {
                text = rest.to_owned();
            }
            if text == *l {
                text.clear();
            }
        }
        for a in ["a ", "an ", "the "] {
            if let Some(rest) = text.strip_prefix(a) {
                text = rest.to_owned();
            }
        }
        if text == before {
            break;
        }
    }
    let text = text.trim().to_owned();
    if !text.is_empty() {
        r.text = Some(text);
    }
    if r.kind.is_none() && r.number.is_none() && !r.this {
        return None;
    }
    if r.kind.is_some() && r.n.is_none() && r.number.is_none() && r.text.is_none() && !r.this {
        r.this = true;
    }
    Some(r)
}

fn parse_loose(s: &str) -> Ref {
    parse_ref(s).unwrap_or_else(|| Ref { text: Some(s.trim().trim_start_matches("the ").to_owned()), ..Default::default() })
}

const WHERE: &[(&str, &str)] = &[
    (" in a new incognito window", "private"),
    (" in a new private window", "private"),
    (" in an incognito window", "private"),
    (" in incognito window", "private"),
    (" in incognito mode", "private"),
    (" in incognito", "private"),
    (" incognito", "private"),
    (" in a private window", "private"),
    (" in private window", "private"),
    (" in private mode", "private"),
    (" in private browsing", "private"),
    (" in private", "private"),
    (" privately", "private"),
    (" in a new tab", "tab"),
    (" in new tab", "tab"),
    (" in another tab", "tab"),
    (" in a separate tab", "tab"),
    (" on a new tab", "tab"),
    (" on new tab", "tab"),
    (" in a new browser tab", "tab"),
    (" as a new tab", "tab"),
    (" in a tab", "tab"),
    (" in a background tab", "background"),
    (" in the background", "background"),
    (" in background", "background"),
    (" in a new window", "window"),
    (" in new window", "window"),
    (" in another window", "window"),
    (" in a separate window", "window"),
    (" in a window", "window"),
    (" in this tab", "here"),
    (" in the same tab", "here"),
    (" in the current tab", "here"),
    (" right here", "here"),
    (" here", "here"),
];

pub fn split_where(t: &str) -> (String, Option<&'static str>) {
    for (s, w) in WHERE {
        if let Some(r) = t.strip_suffix(s) {
            return (r.trim().to_owned(), Some(w));
        }
    }
    (t.to_owned(), None)
}

const NEW_WINDOW: &[&str] = &["new window", "open a new window", "open new window", "open another window", "new browser window", "open a new browser window", "open a browser window", "another window"];
const PRIVATE_WINDOW: &[&str] = &[
    "new incognito window", "open incognito", "open an incognito window", "open incognito window", "incognito mode", "go incognito", "open incognito mode", "new private window",
    "open a private window", "open private window", "private window", "open a private tab", "open incognito tab", "new incognito tab", "new private tab", "open an incognito tab",
    "open in private window", "open inprivate", "new inprivate window", "open a new incognito window", "open a new private window", "start private browsing", "private browsing",
    "incognito window", "incognito",
];
const NEW_TAB: &[&str] = &["new tab", "open a new tab", "open new tab", "open another tab", "new browser tab", "open a blank tab", "open a tab", "another tab", "add a tab", "add a new tab"];
const CLOSE_ACTIVE: &[&str] = &["close this tab", "close tab", "close the tab", "close current tab", "close the current tab", "close this page", "close the page", "close the browser tab"];
const REOPEN: &[&str] = &["reopen tab", "reopen the tab", "reopen closed tab", "reopen the closed tab", "reopen the last tab", "restore the tab", "restore closed tab", "bring back the tab", "undo close tab", "reopen the last closed tab", "open the last closed tab", "bring back the closed tab", "reopen that tab", "restore the last tab"];
const LIST_TABS: &[&str] = &["what tabs are open", "which tabs are open", "list my tabs", "list tabs", "list the tabs", "list all tabs", "list all my tabs", "what tabs do i have open", "read my tabs", "read the tabs", "read out my tabs", "what are my tabs", "show my tabs", "what's open in the browser", "what is open in the browser"];
const COUNT_TABS: &[&str] = &["how many tabs are open", "how many tabs do i have", "how many tabs do i have open", "how many tabs", "count my tabs", "count the tabs"];

fn page(action: &str, r: &Ref, extra: Value) -> Intent {
    let mut a = json!({ "action": action, "ref": r.json(), "what": r.describe() });
    if let (Some(o), Some(e)) = (a.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    act("browser.page", a)
}

fn target_url(x: &str) -> Option<Value> {
    let x = x.trim().trim_start_matches("the ").trim_start_matches("my ");
    let clean = x.trim_end_matches(" website").trim_end_matches(" site").trim_end_matches(" page").trim();
    if let Some(u) = web::spoken_url(clean) {
        return Some(json!({ "url": u }));
    }
    web::site(clean).map(|s| json!({ "site": s.name }))
}

fn settings_page(t: &str) -> Option<(&'static str, Option<String>)> {
    let browserish = t.contains("browser") || t.contains("browsing") || ["brave", "chrome", "firefox", "edge", "vivaldi", "opera", "chromium"].iter().any(|b| t.split_whitespace().any(|w| w == *b));
    const ALWAYS: &[(&[&str], &str)] = &[
        (&["open browser settings", "browser settings", "open the browser settings", "show browser settings", "open the settings of the browser", "open browser preferences"], "settings"),
        (&["open history", "show history", "show my history", "open my history", "open browsing history", "show browsing history", "show my browsing history", "open my browsing history", "open browser history", "show browser history", "open the history", "show the history", "open the history page"], "history"),
        (&["open bookmarks", "show bookmarks", "show my bookmarks", "open my bookmarks", "open the bookmark manager", "bookmark manager", "open bookmark manager", "manage bookmarks"], "bookmarks"),
        (&["open extensions", "manage extensions", "show extensions", "open browser extensions", "show my extensions", "open the extensions page", "open add ons", "manage add ons", "open addons"], "extensions"),
        (&["open saved passwords", "show saved passwords", "open password manager", "show my passwords", "open my passwords", "show passwords", "open passwords", "open the password manager", "manage passwords", "manage my passwords"], "passwords"),
        (&["clear browsing data", "clear browser history", "clear my browsing history", "clear browsing history", "clear the cache", "clear cache", "clear cookies", "clear the cookies", "delete browsing history", "delete my browsing history", "clear my history", "clear history", "clear the browser cache", "clear browser cache", "clear browser data", "delete cookies"], "clear"),
        (&["open site settings", "site settings", "site permissions", "open site permissions", "manage site permissions"], "site"),
        (&["open cookie settings", "manage cookies", "cookie settings"], "cookies"),
        (&["open autofill settings", "autofill settings", "manage autofill"], "autofill"),
        (&["open payment methods", "manage payment methods", "saved cards", "show saved cards"], "payments"),
        (&["open saved addresses", "manage addresses", "saved addresses"], "addresses"),
        (&["open browser downloads", "show browser downloads", "open the downloads page", "show downloads in the browser", "open downloads in the browser", "browser downloads", "show download history", "open download history"], "downloads"),
        (&["brave shields", "open shields settings", "shields settings", "open brave shields"], "shields"),
        (&["open flags", "browser flags", "open chrome flags"], "flags"),
        (&["check for browser updates", "update the browser", "update my browser", "browser version", "which browser version", "about the browser", "check browser updates"], "about"),
        (&["set default browser", "make it the default browser", "make this my default browser", "make this the default browser", "set as default browser"], "default"),
        (&["reset browser settings", "reset the browser", "reset my browser"], "reset"),
        (&["turn off hardware acceleration", "turn on hardware acceleration", "hardware acceleration"], "system"),
        (&["memory saver", "turn on memory saver", "turn off memory saver", "open performance settings", "browser performance settings"], "performance"),
        (&["sync settings", "open sync settings", "turn on sync", "browser sync"], "sync"),
    ];
    for (phrases, page) in ALWAYS {
        if phrases.contains(&t) {
            return Some((page, None));
        }
    }
    for p in ["change the default search engine to ", "change the search engine to ", "change search engine to ", "set the search engine to ", "set search engine to ", "set the default search engine to ", "make the search engine ", "use ", "switch the search engine to "] {
        if let Some(rest) = t.strip_prefix(p) {
            let rest = rest.trim_end_matches(" as my search engine").trim_end_matches(" as the search engine").trim_end_matches(" as search engine").trim_end_matches(" as the default search engine");
            if p != "use " || t.contains("search engine") {
                return Some(("search", Some(sentence_case(rest))));
            }
        }
    }
    if matches!(t, "change search engine" | "change the search engine" | "change the default search engine" | "search engine settings" | "open search engine settings" | "manage search engines") {
        return Some(("search", None));
    }
    for p in ["set the homepage to ", "set homepage to ", "set my homepage to ", "change the homepage to ", "change homepage to ", "change my homepage to "] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some(("startup", Some(rest.to_owned())));
        }
    }
    if matches!(t, "change the homepage" | "change homepage" | "set the homepage" | "homepage settings" | "startup settings" | "change startup pages" | "on startup settings") {
        return Some(("startup", None));
    }
    if (t.contains("dark mode") || t.contains("light mode") || t.contains("theme") || t.contains("font size") || t.contains("appearance")) && browserish {
        let goal = if t.contains("dark mode") { Some("Dark".to_owned()) } else if t.contains("light mode") { Some("Light".to_owned()) } else { None };
        return Some(("appearance", goal));
    }
    if browserish {
        let topics: &[(&str, &str)] = &[
            ("privacy", "privacy"),
            ("security", "security"),
            ("cookie", "cookies"),
            ("search engine", "search"),
            ("password", "passwords"),
            ("history", "history"),
            ("download", "downloadsettings"),
            ("language", "languages"),
            ("accessibility", "accessibility"),
            ("startup", "startup"),
            ("homepage", "startup"),
            ("notification", "notifications"),
            ("pop up", "popups"),
            ("popup", "popups"),
            ("camera", "camera"),
            ("microphone", "microphone"),
            ("location", "location"),
            ("autofill", "autofill"),
            ("extension", "extensions"),
            ("bookmark", "bookmarks"),
            ("appearance", "appearance"),
            ("site", "site"),
            ("setting", "settings"),
            ("preference", "settings"),
        ];
        if t.starts_with("open ") || t.starts_with("show ") || t.starts_with("go to ") || t.starts_with("take me to ") || t.starts_with("bring up ") || t.ends_with(" settings") || t.ends_with(" page") {
            for (k, p) in topics {
                if t.contains(k) {
                    return Some((p, None));
                }
            }
        }
    }
    None
}

const PERMS: &[(&str, &str)] = &[
    ("pop ups", "popups"),
    ("popups", "popups"),
    ("pop-ups", "popups"),
    ("popup", "popups"),
    ("notifications", "notifications"),
    ("notification", "notifications"),
    ("camera", "camera"),
    ("webcam", "camera"),
    ("microphone", "microphone"),
    ("mic", "microphone"),
    ("location", "location"),
    ("javascript", "javascript"),
    ("java script", "javascript"),
    ("images", "images"),
    ("cookies", "cookies"),
    ("automatic downloads", "automaticDownloads"),
];

fn permission_rule(t: &str) -> Option<Intent> {
    let (setting, rest) = if let Some(r) = strip(t, &["block ", "disable ", "turn off ", "stop ", "deny ", "don't allow "]) {
        ("block", r)
    } else if let Some(r) = strip(t, &["allow ", "enable ", "turn on ", "unblock "]) {
        ("allow", r)
    } else {
        ("ask", strip(t, &["always ask for ", "ask before using ", "ask for "])?)
    };
    let mut scope = "all";
    let mut named = false;
    let mut body = rest.to_owned();
    for s in [" on this site", " for this site", " on this website", " for this website", " on this page", " for this page", " here", " on this tab", " from this site", " from this website", " for this tab"] {
        if let Some(r) = body.strip_suffix(s) {
            body = r.to_owned();
            scope = "site";
        }
    }
    for s in [" on all sites", " for all sites", " everywhere", " on every site", " for every site", " in the browser", " in my browser", " on all websites", " in brave", " in chrome", " in firefox", " in edge", " for websites", " for all websites"] {
        if let Some(r) = body.strip_suffix(s) {
            body = r.to_owned();
            scope = "all";
            named = true;
        }
    }
    let body = body.trim_start_matches("the ").trim_start_matches("all ").trim_start_matches("browser ").trim();
    let kind = PERMS.iter().find(|(w, _)| *w == body).map(|(_, k)| *k)?;
    if scope == "all" && !named && kind == "notifications" && !rest.contains("browser") && ["allow ", "block ", "deny ", "unblock ", "don't allow "].iter().any(|p| t.starts_with(p)) {
        scope = "site";
    } else if scope == "all" && !named && !matches!(kind, "popups" | "javascript" | "automaticDownloads") && !rest.contains("browser") {
        return None;
    }
    if setting == "ask" && !matches!(kind, "notifications" | "camera" | "microphone" | "location") {
        return None;
    }
    let setting = if kind == "cookies" && setting == "allow" { "allow" } else { setting };
    Some(act("browser.permission", json!({ "kind": kind, "setting": setting, "scope": scope, "what": body })))
}

fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let host = rest.split(['/', '?', '#']).next()?;
    Some(format!("{scheme}://{host}"))
}

fn tab_ref(x: &str) -> Option<Value> {
    let x = x.trim().trim_start_matches("the ");
    if matches!(x, "last tab" | "rightmost tab" | "final tab") {
        return Some(json!({ "index": -1 }));
    }
    if matches!(x, "first tab" | "leftmost tab") {
        return Some(json!({ "index": 1 }));
    }
    if let Some(n) = x.strip_suffix(" tab").and_then(ordinal) {
        return Some(json!({ "index": n }));
    }
    if let Some(n) = x.strip_prefix("tab number ").or_else(|| x.strip_prefix("tab ")).and_then(cardinal) {
        return Some(json!({ "index": n }));
    }
    let (q, all) = if let Some(q) = x.strip_suffix(" tabs") {
        (q.trim_start_matches("all ").trim_start_matches("the ").trim(), true)
    } else {
        (x.strip_suffix(" tab")?, x.starts_with("all "))
    };
    let q = q.trim_start_matches("all the ").trim_start_matches("all ").trim();
    if q.is_empty() || matches!(q, "this" | "current" | "other" | "the other" | "next" | "previous" | "new" | "a new" | "another" | "closed" | "last closed") {
        return None;
    }
    Some(json!({ "query": q, "all": all }))
}

pub fn address(raw: &str) -> Option<String> {
    for w in raw.split_whitespace() {
        let w = w.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | '(' | ')')).trim_end_matches(['.', ',', '!', '?']);
        let lower = w.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Some(w.to_owned());
        }
        let host = lower.split(['/', '?', '#']).next().unwrap_or("");
        let name = host.split(':').next().unwrap_or("");
        let port_ok = host.split_once(':').is_none_or(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let labels: Vec<&str> = name.split('.').collect();
        let local = name == "localhost" || name.ends_with(".localhost");
        let dotted = labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
        const TLDS: &[&str] = &["com", "org", "net", "io", "ai", "dev", "app", "in", "co", "uk", "edu", "gov", "me", "tv", "info", "xyz", "tech", "gg", "so", "ly", "fm", "us", "ca", "de", "fr", "jp", "au", "ru", "ch", "nl", "se", "no", "es", "it", "br", "cn", "kr", "page", "site", "online", "store", "blog", "news", "live", "cloud", "ac", "biz", "eu", "nz", "sg", "ae", "pk", "bd", "lk", "np", "id", "my", "ph", "tw", "hk", "za", "mx", "ar", "pl", "be", "at", "dk", "fi", "ie", "pt", "gr", "tr", "il", "ir", "ua", "vn", "th", "to", "cc", "ws", "wiki", "games", "media", "studio", "world", "today", "one", "pro", "fun", "lol", "run", "build", "codes", "tools", "chat", "social", "space", "website", "link", "email", "work", "shop", "art", "design"];
        let known_tld = labels.last().is_some_and(|tld| TLDS.contains(tld));
        let file_like = !known_tld && !lower.contains('/') && !host.contains(':');
        if port_ok && (local || (dotted && (known_tld || host.contains(':') || lower.contains('/')) && !file_like)) {
            let scheme = if local { "http" } else { "https" };
            return Some(format!("{scheme}://{w}"));
        }
    }
    None
}

pub fn address_rule(t: &str, raw: &str) -> Option<Intent> {
    let rest = strip(t, &["open ", "go to ", "visit ", "take me to ", "load ", "browse to ", "navigate to "])?;
    if rest.split_whitespace().count() > 10 {
        return None;
    }
    let url = address(raw)?;
    let (_, place) = split_where(t);
    Some(act("browser.open", json!({ "url": url, "where": place.unwrap_or("tab") })))
}

pub fn rules(t: &str, raw: &str) -> Option<Intent> {
    let _ = raw;
    let t = t.replace("right-click", "right click").replace("pop-up", "pop up");
    let t = t.as_str();
    if matches!(t, "save this page as pdf" | "save this page as a pdf" | "download this page as pdf" | "download this page as a pdf" | "save the page as pdf" | "save the page as a pdf" | "print this page to pdf" | "make a pdf of this page" | "save this web page as pdf") {
        return Some(act("browser.capture", json!({ "kind": "pdf" })));
    }
    if matches!(t, "take a screenshot of this page" | "screenshot this page" | "take a picture of this page" | "capture this page" | "save a screenshot of this page" | "screenshot the page") {
        return Some(act("browser.capture", json!({ "kind": "png" })));
    }
    if matches!(t, "summarize this page" | "summarise this page" | "summarize the page" | "summarise the page" | "summarize this article" | "summarise this article" | "give me a summary of this page" | "what is this page about" | "what's this page about" | "sum up this page" | "summary of this page" | "what does this page say") {
        return Some(act("browser.summary", json!({})));
    }
    if matches!(t, "read this page" | "read the page" | "read me this page" | "read this page to me" | "read out this page" | "read this article" | "read the article") {
        return Some(act("browser.read", json!({})));
    }
    if matches!(t, "move this tab to the right" | "move the tab right" | "move this tab right" | "move tab right") {
        return Some(act("key", json!({ "keys": "ctrl shift pagedown", "label": "Moved the tab right" })));
    }
    if matches!(t, "move this tab to the left" | "move the tab left" | "move this tab left" | "move tab left") {
        return Some(act("key", json!({ "keys": "ctrl shift pageup", "label": "Moved the tab left" })));
    }
    if matches!(t, "fill in this form" | "fill this form" | "fill out this form" | "what does this form ask for" | "help me fill this form" | "fill in the form" | "what's in this form") {
        return Some(act("browser.form", json!({ "op": "list" })));
    }
    if matches!(t, "send the form" | "submit the form" | "submit this form" | "send this form" | "submit it") {
        return Some(act("browser.form", json!({ "op": "submit" })));
    }
    if let Some(r) = t.strip_prefix("fill ").or_else(|| t.strip_prefix("fill in "))
        && let Some((field, value)) = r.split_once(" with ")
        && !field.trim().is_empty()
        && !value.trim().is_empty()
    {
        let v = raw.to_lowercase().find(" with ").map(|i| raw[i + 6..].trim().to_owned()).unwrap_or_else(|| value.trim().to_owned());
        return Some(act("browser.form", json!({ "op": "fill", "field": field.trim(), "value": v })));
    }
    if matches!(t, "what errors are in the console" | "what's in the console" | "whats in the console" | "any console errors" | "are there console errors" | "check the console" | "why is this page broken" | "what is wrong with this page" | "what's wrong with this page" | "whats wrong with this page") {
        return Some(act("browser.console", json!({})));
    }
    for p in ["import my old profile", "import my profiles", "import my old profiles", "import my browser profile", "bring over my old profile", "bring over my profiles", "import everything from my old profile", "import my bookmarks"] {
        if t == p {
            return Some(act("browser.import", json!({ "which": "all" })));
        }
    }
    for p in ["import my ", "bring over my "] {
        if let Some(rest) = t.strip_prefix(p)
            && let Some(name) = rest.strip_suffix(" profile")
            && !name.trim().is_empty()
            && !matches!(name.trim(), "old" | "browser")
        {
            let (b, n) = match name.trim().split_once(' ') {
                Some((first, more)) if web::browser_key(first).is_some() => (first.to_owned(), more.to_owned()),
                _ => (String::new(), name.trim().to_owned()),
            };
            return Some(act("browser.import", json!({ "browser": b, "which": n })));
        }
    }
    for p in ["go back to my old profile", "go back to my old browser profile", "use my old profile", "use my old browser profile", "switch back to my old profile", "stop using the setu profile"] {
        if let Some(rest) = t.strip_prefix(p) {
            let b = rest.trim().trim_start_matches("in ").trim_start_matches("for ").trim_start_matches("on ").trim();
            return Some(act("browser.profile", json!({ "browser": b, "op": "old" })));
        }
    }
    if matches!(t, "set up the browser extension" | "setup the browser extension" | "install the browser extension" | "set up browser control" | "connect to the browser" | "connect the browser" | "set up the extension" | "install the setu extension" | "set up the setu extension" | "connect my browser" | "set up my browser" | "set up the browser" | "install browser extension" | "set up browser extension" | "set up the setu profile" | "set up a setu profile" | "make a setu profile" | "set up full control of the browser" | "give setu full control of the browser") {
        return Some(act("browser.setup", json!({})));
    }
    if let Some(rest) = strip(t, &["connect to ", "connect ", "set up ", "setup ", "install the extension in ", "install the setu extension in ", "set up the extension in ", "set up the extension for "]) {
        let b = rest.trim_end_matches(" extension").trim_end_matches(" browser").trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" for setu");
        if web::browser_key(b).is_some() {
            return Some(act("browser.setup", json!({ "browser": b })));
        }
    }
    if NEW_WINDOW.contains(&t) {
        return Some(act("browser.window", json!({ "private": false })));
    }
    if PRIVATE_WINDOW.contains(&t) {
        return Some(act("browser.window", json!({ "private": true })));
    }
    if NEW_TAB.contains(&t) {
        return Some(act("browser.tab.new", json!({})));
    }
    for p in ["open a new tab and go to ", "open a new tab and open ", "open a new tab with ", "new tab with ", "open new tab and go to ", "new tab and go to ", "open a new tab to ", "open a new tab for ", "open a new window with ", "open a new window and go to "] {
        if let Some(x) = t.strip_prefix(p) {
            let where_ = if p.contains("window") { "window" } else { "tab" };
            if let Some(mut v) = target_url(x) {
                v["where"] = json!(where_);
                return Some(act("browser.open", v));
            }
            return Some(act("browser.open", json!({ "query": x, "where": where_ })));
        }
    }
    for p in ["open a new tab and search for ", "open a new tab and search ", "new tab and search for ", "search in a new tab for "] {
        if let Some(x) = t.strip_prefix(p) {
            return Some(act("browser.open", json!({ "query": x, "where": "tab" })));
        }
    }
    let (body, where_) = split_where(t);
    if let Some(w) = where_ {
        for v in ["open up ", "open ", "go to ", "visit ", "load ", "launch ", "show me ", "show ", "view ", "play ", "search for ", "search ", "pull up ", "bring up ", "follow ", "watch "] {
            let Some(x) = body.strip_prefix(v) else { continue };
            if v.starts_with("search") {
                return Some(act("browser.open", json!({ "query": x, "where": w })));
            }
            if let Some(r) = parse_ref(x) {
                let full = r.kind == Some("image") && w != "here";
                return Some(page("open", &r, json!({ "where": w, "full": full })));
            }
            if let Some(mut u) = target_url(x) {
                u["where"] = json!(w);
                return Some(act("browser.open", u));
            }
            if v == "play " || v == "watch " {
                return Some(act("browser.open", json!({ "play": x, "where": w })));
            }
            if matches!(x, "a tab" | "tab" | "new tab") {
                return Some(act("browser.tab.new", json!({})));
            }
        }
        if body == "open" || body == "open it" || body == "open this" || body == "open that" {
            return Some(page("open", &Ref { this: true, ..Default::default() }, json!({ "where": w })));
        }
    }
    if CLOSE_ACTIVE.contains(&t) {
        return Some(act("browser.tab.close", json!({ "target": { "active": true } })));
    }
    if matches!(t, "close other tabs" | "close all other tabs" | "close the other tabs" | "close all the other tabs" | "close every other tab") {
        return Some(act("browser.tab.close", json!({ "target": { "others": true } })));
    }
    if matches!(t, "close tabs to the right" | "close all tabs to the right" | "close the tabs to the right") {
        return Some(act("browser.tab.close", json!({ "target": { "right": true } })));
    }
    if let Some(x) = strip(t, &["close "])
        && let Some(r) = tab_ref(x)
    {
        return Some(act("browser.tab.close", json!({ "target": r })));
    }
    if let Some(x) = strip(t, &["switch to ", "go to ", "move to ", "jump to ", "show me ", "take me to ", "open ", "bring up ", "go back to ", "switch back to ", "select "])
        && let Some(r) = tab_ref(x)
    {
        return Some(act("browser.tab.activate", json!({ "target": r })));
    }
    if REOPEN.contains(&t) {
        return Some(act("browser.tab.reopen", json!({})));
    }
    if matches!(t, "duplicate this tab" | "duplicate tab" | "duplicate the tab" | "duplicate the current tab" | "duplicate this page") {
        return Some(act("browser.tab.duplicate", json!({})));
    }
    if matches!(t, "pin this tab" | "pin tab" | "pin the tab" | "pin the current tab") {
        return Some(act("browser.tab.pin", json!({ "pinned": true })));
    }
    if matches!(t, "unpin this tab" | "unpin tab" | "unpin the tab" | "unpin the current tab") {
        return Some(act("browser.tab.pin", json!({ "pinned": false })));
    }
    if matches!(t, "move this tab to a new window" | "move tab to new window" | "move this tab to a new window please" | "open this tab in a new window" | "pop out this tab" | "detach this tab" | "move the tab to a new window" | "move this tab to another window") {
        return Some(act("browser.tab.detach", json!({})));
    }
    if LIST_TABS.contains(&t) {
        return Some(act("browser.tabs", json!({})));
    }
    if COUNT_TABS.contains(&t) {
        return Some(act("browser.tabs", json!({ "count": true })));
    }
    if matches!(t, "bookmark this page" | "bookmark this" | "bookmark this tab" | "bookmark it" | "add a bookmark" | "add this page to bookmarks" | "add to bookmarks" | "save this page as a bookmark" | "save this page to bookmarks" | "bookmark the page" | "add this to my bookmarks") {
        return Some(act("browser.bookmark", json!({})));
    }
    if matches!(t, "remove this bookmark" | "remove the bookmark" | "unbookmark this page" | "delete this bookmark" | "remove bookmark" | "remove this page from bookmarks" | "unbookmark this") {
        return Some(act("browser.bookmark", json!({ "remove": true })));
    }
    if let Some((p, goal)) = settings_page(t) {
        return Some(act("browser.settings", json!({ "page": p, "goal": goal })));
    }
    if let Some(i) = permission_rule(t) {
        return Some(i);
    }
    if matches!(t, "copy the url" | "copy this url" | "copy the page url" | "copy the link of this page" | "copy this page's link" | "copy the page link" | "copy page link" | "copy the current url" | "copy the address" | "copy the web address" | "copy this page link" | "copy the page address" | "copy the website link" | "copy the website address" | "copy url" | "copy the url of this page" | "copy this page's address") {
        return Some(act("browser.pageurl", json!({})));
    }
    if matches!(t, "show numbers on images" | "number the images" | "show image numbers" | "label the images" | "number images" | "show numbers on the images" | "show numbers on pictures" | "number the pictures") {
        return Some(act("browser.page", json!({ "action": "numbers", "kind": "image" })));
    }
    if matches!(t, "number the links" | "show link numbers" | "show numbers on links" | "show numbers on the links" | "number links" | "label the links") {
        return Some(act("browser.page", json!({ "action": "numbers", "kind": "link" })));
    }
    if matches!(t, "number the results" | "show numbers on results" | "show numbers on the results" | "number the search results") {
        return Some(act("browser.page", json!({ "action": "numbers", "kind": "result" })));
    }
    if matches!(t, "number the videos" | "show numbers on videos" | "show numbers on the videos") {
        return Some(act("browser.page", json!({ "action": "numbers", "kind": "video" })));
    }
    if matches!(t, "right click" | "show options" | "show the options" | "show me the options" | "open the context menu" | "context menu" | "show the menu" | "more options" | "show options here") {
        return Some(page("menu", &Ref { this: true, ..Default::default() }, json!({})));
    }
    if let Some(x) = strip(t, &["show options for ", "show the options for ", "show me options for ", "show me the options for ", "options for ", "right click on ", "right click ", "open the menu for ", "show the menu for ", "show menu for ", "context menu for ", "open context menu for ", "open the context menu for ", "open options for ", "more options for ", "show me the menu for ", "what can i do with "]) {
        return Some(page("menu", &parse_loose(x), json!({})));
    }
    for (verbs, what) in [(&["copy the address of ", "copy the link of ", "copy the link to ", "copy the url of "][..], "link"), (&["copy the image address of ", "copy the image link of "][..], "image")] {
        if let Some(x) = strip(t, verbs) {
            return Some(page("copy", &parse_loose(x), json!({ "what": what })));
        }
    }
    if let Some(x) = strip(t, &["copy "]) {
        let (what, rest) = if let Some(r) = x.strip_suffix(" image address").or_else(|| x.strip_suffix(" image link")).or_else(|| x.strip_suffix(" image url")) {
            ("image", format!("{r} image"))
        } else if let Some(r) = x.strip_suffix(" address").or_else(|| x.strip_suffix(" url")) {
            let r = r.trim();
            if r.ends_with("image") || r.ends_with("picture") || r.ends_with("photo") { ("image", r.to_owned()) } else { ("link", r.to_owned()) }
        } else {
            ("link", x.to_owned())
        };
        let rest = rest.trim();
        let rest = if rest == "image" || rest == "the image" || rest == "this image" { "this image".to_owned() } else if rest.is_empty() { "this link".to_owned() } else { rest.to_owned() };
        if let Some(r) = parse_ref(&rest).filter(|r| r.kind.is_some()) {
            let what = if r.kind == Some("image") && what == "link" && !x.contains("link") { "image" } else { what };
            return Some(page("copy", &r, json!({ "what": what })));
        }
    }
    if let Some(x) = strip(t, &["search ", "google ", "look up ", "reverse search ", "do a reverse image search on ", "use google lens on ", "google lens "]) {
        let x = x.trim_end_matches(" on google").trim_end_matches(" with google lens").trim_end_matches(" with google").trim_end_matches(" using google lens").trim_end_matches(" on google lens");
        if let Some(r) = parse_ref(x).filter(|r| r.kind == Some("image")) {
            return Some(page("lens", &r, json!({})));
        }
    }
    if matches!(t, "google lens" | "reverse image search" | "search this image" | "search the image" | "search this picture" | "look up this image" | "search image") {
        return Some(page("lens", &Ref { kind: Some("image"), this: true, ..Default::default() }, json!({})));
    }
    if let Some(x) = strip(t, &["save ", "download "])
        && let Some(r) = parse_ref(x).filter(|r| matches!(r.kind, Some("image") | Some("video") | Some("link")))
    {
        return Some(page("save", &r, json!({})));
    }
    if let Some(x) = strip(t, &["view ", "enlarge ", "open the full ", "show the full "])
        && let Some(r) = parse_ref(x).filter(|r| r.kind == Some("image"))
    {
        return Some(page("open", &r, json!({ "where": "tab", "full": true })));
    }
    if matches!(t, "view image" | "view the image" | "view this image" | "open image" | "open the image" | "open this image") {
        let full = t.starts_with("view");
        return Some(page("open", &Ref { kind: Some("image"), this: true, ..Default::default() }, json!({ "where": if full { "tab" } else { "here" }, "full": full })));
    }
    if let Some(x) = strip(t, &["click on ", "click ", "tap on ", "tap ", "select ", "press on ", "choose "])
        && let Some(r) = parse_ref(x).filter(|r| r.n.is_some() || r.number.is_some() || matches!(r.kind, Some("image" | "link" | "result" | "video")))
    {
        return Some(page("click", &r, json!({})));
    }
    if let Some(x) = strip(t, &["open up ", "open ", "go to ", "visit ", "follow ", "play ", "show me ", "show ", "watch "])
        && let Some(r) = parse_ref(x).filter(|r| r.kind.is_some() && !r.this || r.number.is_some() && r.kind.is_none() || r.kind.is_some() && r.this && !x.trim().is_empty() && x.split_whitespace().count() >= 2)
    {
        if t.starts_with("show") && r.kind != Some("image") {
            return None;
        }
        return Some(page("open", &r, json!({ "where": "here" })));
    }
    for p in ["find ", "search this page for ", "search the page for ", "look for ", "search for ", "highlight "] {
        if let Some(x) = t.strip_prefix(p) {
            for s in [" on this page", " on the page", " in this page", " in the page", " on the screen", " on screen"] {
                if let Some(q) = x.strip_suffix(s) {
                    return Some(act("browser.page", json!({ "action": "find", "text": q })));
                }
            }
            if p.starts_with("search th") {
                return Some(act("browser.page", json!({ "action": "find", "text": x })));
            }
        }
    }
    if matches!(t, "next image" | "next picture" | "next photo" | "show the next image" | "show next image") {
        return Some(act("key", json!({ "keys": "right", "label": "Next image" })));
    }
    if matches!(t, "previous image" | "previous picture" | "previous photo" | "show the previous image" | "last image") {
        return Some(act("key", json!({ "keys": "left", "label": "Previous image" })));
    }
    if matches!(t, "close the preview" | "close preview" | "close the image" | "close image" | "close the image preview" | "close image preview" | "close the menu" | "close menu" | "hide the menu") {
        return Some(act("browser.close_overlay", json!({})));
    }
    None
}

fn s(args: &Value, k: &str) -> String {
    args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned()
}

async fn unreachable(core: &Arc<Core>) -> Outcome {
    let blind = match crate::browserui::view("").await {
        None => true,
        Some(v) => v.nodes.is_empty(),
    };
    if blind && let Some(o) = crate::access::explain_browser(core).await {
        return o;
    }
    let name = web::find_browser(core, None).map(|b| b.name).unwrap_or_else(|| "your browser".into());
    Outcome::fail(&format!("I can't do that in {} yet.", short_name(&name)), LAYER)
}

pub fn needs_choice(core: &Core) -> bool {
    browsers::chosen(core).is_none() && browsers::installed(core).len() > 1
}

pub fn choose_first(core: &Core, tool: &str, args: &Value) -> Option<Outcome> {
    if !needs_choice(core) {
        return None;
    }
    let list = browsers::installed(core);
    let names: Vec<String> = list.iter().map(|b| browsers::display(b.key, &b.app.name)).collect();
    let choices: Vec<Choice> = list
        .iter()
        .zip(names.iter())
        .map(|(b, name)| {
            let mut words: Vec<&str> = web::BROWSERS.iter().find(|(k, _)| *k == b.key).map(|(_, w)| w.to_vec()).unwrap_or_default();
            let lower = name.to_lowercase();
            words.push(&lower);
            let mut c = Choice::new(name, &words, act("browser.choose", json!({ "id": b.app.id, "resume": { "tool": tool, "args": args } })));
            c.icon = browsers::icon_of(&b.app);
            c
        })
        .collect();
    let spoken = match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("Which browser should I use? {} or {last}?", rest.join(", ")),
        _ => "Which browser should I use?".to_owned(),
    };
    Some(Outcome::asking("Asked which browser to use", LAYER, FollowUp { question: "Which browser should I use?".into(), spoken: Some(spoken), choices, cleanup: None, kind: "browsers", focus: None }))
}

async fn set_browser(core: &Arc<Core>, id: &str) -> Result<crate::tools::apps::App, String> {
    let app = core.apps.get(id).ok_or("That browser isn't installed any more.")?;
    let mut defaults = core.settings.get("apps.defaults").as_object().cloned().unwrap_or_default();
    defaults.insert("browser".into(), json!(id));
    let patch = json!({ "patch": { "web.browser": id, "apps.defaults": defaults } });
    crate::handlers::settings_set(core, &patch).await.map_err(|e| e.message)?;
    core.hub.emit("browsers.state", browsers::list(core));
    Ok(app)
}

async fn choose(core: &Arc<Core>, args: &Value) -> Outcome {
    let id = s(args, "id");
    let app = match set_browser(core, &id).await {
        Ok(a) => a,
        Err(e) => return Outcome::fail(&e, LAYER),
    };
    let key = browsers::key_of(&app).unwrap_or("chrome");
    let name = browsers::display(key, &app.name);
    if let Some(r) = args.get("resume")
        && let Some(tool) = r.get("tool").and_then(Value::as_str)
    {
        let a = r.get("args").cloned().unwrap_or(json!({}));
        let o = Box::pin(crate::skills::run(core, tool, a, Arc::new(AtomicBool::new(false)))).await;
        return o;
    }
    Outcome::ok_with(&format!("{name} is your browser"), LAYER, None).with_reply(format!("Okay, I'll use {name}."))
}

pub async fn choose_and_connect(core: Arc<Core>, id: String) -> Result<Value, String> {
    set_browser(&core, &id).await?;
    let _ = crate::os::ready().await;
    Ok(json!({ "state": "connected" }))
}

fn short_name(n: &str) -> String {
    n.trim_end_matches(" Web Browser").trim_end_matches(" Browser").trim_end_matches(" browser").to_owned()
}

async fn keys(spoken: &str, label: &str) -> Outcome {
    let Some(mut k) = crate::tools::input::parse_combo(spoken) else { return Outcome::fail("That key isn't known.", "keyboard") };
    if cfg!(target_os = "macos") {
        for x in k.iter_mut() {
            if *x == crate::tools::input::K::Ctrl {
                *x = crate::tools::input::K::Meta;
            }
        }
    }
    match crate::tools::input::press(&k).await {
        Ok(()) => Outcome::ok_with(label, "keyboard", None),
        Err(e) => Outcome::fail(&e.to_string(), "keyboard"),
    }
}

fn private_flag(key: &str) -> &'static str {
    match key {
        "firefox" | "librewolf" | "zen" => "-private-window",
        "edge" => "--inprivate",
        _ => "--incognito",
    }
}

fn window_flag(key: &str) -> &'static str {
    match key {
        "firefox" | "librewolf" | "zen" => "-new-window",
        _ => "--new-window",
    }
}

async fn launch(core: &Arc<Core>, name: &str, flags: Vec<String>) -> Result<String, String> {
    let app = web::find_browser(core, if name.is_empty() { None } else { Some(name) }).ok_or_else(|| "No browser is installed.".to_owned())?;
    let label = short_name(&app.name);
    let a = app.clone();
    match crate::layers::blocking(move || crate::tools::shell::launch_args(&a, &flags)).await {
        Ok(Ok(())) => Ok(label),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("The browser could not be started.".into()),
    }
}

fn key_of(core: &Core, name: &str) -> String {
    web::find_browser(core, if name.is_empty() { None } else { Some(name) }).and_then(|b| browser_of(&b.name).or_else(|| browser_of(&b.id))).unwrap_or_else(|| "chrome".into())
}

async fn resolve_url(args: &Value) -> Option<(String, String)> {
    if !s(args, "url").is_empty() {
        let u = s(args, "url");
        return Some((u.clone(), web::host(&u)));
    }
    if !s(args, "site").is_empty() {
        let site = web::site(&s(args, "site")).or_else(|| web::SITES.iter().find(|x| x.name == s(args, "site")))?;
        return Some((site.home.to_owned(), site.name.to_owned()));
    }
    if !s(args, "play").is_empty() {
        let q = s(args, "play");
        let qq = q.clone();
        if let Ok(Some((id, title))) = crate::layers::blocking(move || web::youtube_video(&qq)).await {
            return Some((format!("https://www.youtube.com/watch?v={id}"), if title.is_empty() { q } else { title }));
        }
        return Some((web::search_url(web::site("youtube"), &q), q));
    }
    if !s(args, "query").is_empty() {
        let q = s(args, "query");
        return Some((web::search_url(None, &q), q));
    }
    None
}

fn where_words(w: &str) -> &'static str {
    match w {
        "window" => " in a new window",
        "private" => " in a private window",
        "background" => " in a background tab",
        "here" => "",
        _ => " in a new tab",
    }
}

async fn by_protocol(core: &Arc<Core>, app: &crate::tools::apps::App, url: &str, here: bool) -> Option<Result<(), String>> {
    let key = crate::tools::browsers::key_of(app)?;
    if !crate::proto::supported(key) || !crate::cdp::is_default(&core.paths.data, key) {
        return None;
    }
    let exe = crate::cdp::exe_of(&app.exec, &app.path);
    let r = async {
        let (c, started) = crate::proto::ensure(&core.paths.data, key, &exe, if here { None } else { Some(url) }).await?;
        if started && !here {
            return Ok::<_, anyhow::Error>(());
        }
        if here {
            let tab = crate::proto::active_tab(&c).await?;
            c.navigate(&tab.id, url).await
        } else {
            let id = c.open(url).await?;
            c.activate(&id).await
        }
    }
    .await;
    Some(r.map_err(|e| e.to_string()))
}

async fn open(core: &Arc<Core>, args: &Value) -> Outcome {
    let Some((url, what)) = resolve_url(args).await else { return Outcome::fail("Open what?", LAYER) };
    let w = s(args, "where");
    let w = if w.is_empty() { "tab".to_owned() } else { w };
    let name = s(args, "browser");
    if matches!(w.as_str(), "here" | "tab")
        && let Some(app) = web::find_browser(core, if name.is_empty() { None } else { Some(&name) })
    {
        if let Some(r) = by_protocol(core, &app, &url, w == "here").await {
            return match r {
                Ok(()) => Outcome::ok_with(&format!("Opened {what}{}", where_words(&w)), LAYER, Some(app.name.clone())),
                Err(e) => Outcome::fail(&e, LAYER),
            };
        }
        if w == "here"
            && let Some(key) = crate::tools::browsers::key_of(&app)
            && crate::proto::supported(key)
            && !crate::cdp::declined(&core.paths.data, key)
            && !crate::cdp::has_profile(&core.paths.data, key)
        {
            return offer_setup(&app, json!({ "tool": "browser.open", "args": args }));
        }
    }
    let key = key_of(core, &name);
    let flags: Vec<String> = match w.as_str() {
        "private" => vec![private_flag(&key).into(), url],
        "window" => vec![window_flag(&key).into(), url],
        _ => vec![url],
    };
    match launch(core, &name, flags).await {
        Ok(_) => Outcome::ok_with(&format!("Opened {what}{}", where_words(&w)), LAYER, None),
        Err(e) => Outcome::fail(&e, LAYER),
    }
}

async fn page_act(core: &Arc<Core>, args: Value) -> Outcome {
    if let Some(o) = protocol_first("browser.page", &args).await {
        return o;
    }
    let name = s(&args, "browser");
    if let Some(o) = crate::browserui::page_act(core, &name, &args).await {
        return o;
    }
    let r = args.get("ref").cloned().unwrap_or(Value::Null);
    let n = r.get("n").and_then(Value::as_i64);
    let kind = r.get("kind").and_then(Value::as_str).unwrap_or("any");
    if s(&args, "action") == "open" && n == Some(1) && matches!(kind, "result" | "video" | "link") {
        return Box::pin(crate::skills::run(core, "ui.first", json!({}), Arc::new(AtomicBool::new(false)))).await;
    }
    if s(&args, "action") == "click"
        && let Some(t) = r.get("text").and_then(Value::as_str)
    {
        return Box::pin(crate::skills::run(core, "ui.click", json!({ "name": t }), Arc::new(AtomicBool::new(false)))).await;
    }
    unreachable(core).await
}

pub async fn setu_tab_first(args: &Value) -> Option<Outcome> {
    let target = args.get("target")?;
    let q = target.get("query").and_then(Value::as_str)?;
    if !crate::browserui::setu_in_front().await {
        return None;
    }
    if crate::os::typed()
        && let Some(v) = crate::browserui::view("").await
        && !crate::browserui::pick_tab(&v.tabs(), target).is_empty()
    {
        return None;
    }
    crate::browserui::setu_tab(q).await
}

async fn close_by_protocol(args: &Value) -> Option<Outcome> {
    let c = crate::proto::any()?;
    let target = args.get("target")?;
    let name = target.get("text").and_then(Value::as_str).or_else(|| target.get("name").and_then(Value::as_str)).or_else(|| target.get("query").and_then(Value::as_str))?;
    let tabs = c.tabs().await.ok()?;
    let hits: Vec<crate::cdp::Tab> = crate::cdp::find_tab(&tabs, name).into_iter().cloned().collect();
    if hits.len() != 1 {
        return None;
    }
    Some(match c.close(&hits[0].id).await {
        Ok(()) => Outcome::ok_with(&format!("Closed {}", hits[0].title), LAYER, None),
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    })
}

pub async fn launch_entry(core: &Arc<Core>, key: &str, urls: Vec<String>, flags: Vec<String>) -> Result<Value, String> {
    let data = core.paths.data.clone();
    if !crate::cdp::is_default(&data, key) {
        return Err("No Setu profile is the default for that browser.".into());
    }
    let app = browsers::installed(core).into_iter().find(|b| b.key == key).map(|b| b.app).ok_or("That browser isn't installed.")?;
    let exe = crate::cdp::exe_of(&app.exec, &app.path);
    let first = urls.first().map(String::as_str);
    let (c, started) = crate::proto::ensure(&data, key, &exe, first).await.map_err(|e| e.to_string())?;
    let rest: Vec<&String> = if started { urls.iter().skip(1).collect() } else { urls.iter().collect() };
    let mut opened = if started && first.is_some() { 1 } else { 0 };
    for u in rest {
        if let Ok(id) = c.open(u).await {
            let _ = c.activate(&id).await;
            opened += 1;
        }
    }
    if !started && urls.is_empty() {
        let _ = &flags;
        c.new_window().await.map_err(|e| e.to_string())?;
    }
    Ok(json!({ "opened": opened, "started": started }))
}

fn safe_name(title: &str) -> String {
    let t: String = title.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { ' ' }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.chars().take(60).collect();
    if t.is_empty() { "Page".into() } else { t }
}

async fn deep(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let c = match crate::proto::any() {
        Some(c) => c,
        None => {
            if let Some(app) = web::find_browser(core, None)
                && let Some(key) = crate::tools::browsers::key_of(&app)
                && crate::proto::supported(key)
            {
                if crate::cdp::is_default(&core.paths.data, key) {
                    let exe = crate::cdp::exe_of(&app.exec, &app.path);
                    if crate::proto::ensure(&core.paths.data, key, &exe, None).await.is_ok() {
                        return Outcome::ok_with("Started the browser", LAYER, None).with_reply("I've started the browser on your Setu profile. Open the page and ask again.".into());
                    }
                }
                if !crate::cdp::has_profile(&core.paths.data, key) && !crate::cdp::declined(&core.paths.data, key) {
                    return offer_setup(&app, json!({ "tool": tool, "args": args }));
                }
            }
            return Outcome::fail("That needs full control of the browser. Say set up my browser.", LAYER);
        }
    };
    let tab = match crate::proto::active_tab(&c).await {
        Ok(t) => t,
        Err(e) => return Outcome::fail(&e.to_string(), LAYER),
    };
    match tool {
        "browser.capture" => {
            let pdf = s(args, "kind") == "pdf";
            let bytes = match c.capture(&tab.id, pdf).await {
                Ok(b) => b,
                Err(e) => return Outcome::fail(&format!("The browser didn't make the {}: {e}", if pdf { "PDF" } else { "picture" }), LAYER),
            };
            let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
            let downloads = dirs::download_dir().unwrap_or_else(|| home.join("Downloads"));
            let pictures = dirs::picture_dir().unwrap_or_else(|| home.join("Pictures"));
            let dir = if pdf {
                downloads
            } else if pictures.join("Screenshots").is_dir() {
                pictures.join("Screenshots")
            } else {
                pictures
            };
            let _ = std::fs::create_dir_all(&dir);
            let base = safe_name(&tab.title);
            let ext = if pdf { "pdf" } else { "png" };
            let mut path = dir.join(format!("{base}.{ext}"));
            let mut n = 2;
            while path.exists() {
                path = dir.join(format!("{base} ({n}).{ext}"));
                n += 1;
            }
            match std::fs::write(&path, &bytes) {
                Ok(()) => Outcome::ok_with(&format!("Saved {}", crate::tools::files::spoken_path(&path)), LAYER, None),
                Err(e) => Outcome::fail(&format!("I couldn't save it: {e}"), LAYER),
            }
        }
        "browser.read" => match c.page_text(&tab.id, 1500).await {
            Ok((title, body)) if !body.is_empty() => {
                let more = if body.chars().count() >= 1500 { " That's the start of the page." } else { "" };
                Outcome::ok_with(&format!("Read {title}"), LAYER, None).with_reply(format!("{body}{more}"))
            }
            Ok((title, _)) => Outcome::ok_with("Nothing to read", LAYER, None).with_reply(format!("{} has no text to read.", if title.is_empty() { "This page".to_owned() } else { title })),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "browser.summary" => match c.page_text(&tab.id, 6000).await {
            Ok((title, body)) if body.chars().count() >= 240 => {
                let system = "You summarise a web page for someone who will hear the summary spoken. Use only what the page says. Answer with JSON: {\"summary\": \"three short plain sentences\"}.";
                let user = format!("Title: {title}\n\n{body}");
                let said = crate::pipeline::ask_json(core, system, &user, Some(crate::pipeline::string_grammar("summary")), 220, 0.2).await.ok().and_then(|v| v.get("summary").and_then(Value::as_str).map(|s| crate::pipeline::tidy_answer(s, false))).filter(|s| s.split_whitespace().count() >= 5);
                match said {
                    Some(sum) => Outcome::ok_with(&format!("Summarised {title}"), LAYER, None).with_reply(sum),
                    None => {
                        let start: String = body.split_inclusive(['.', '!', '?']).take(2).collect::<String>().chars().take(320).collect();
                        Outcome::ok_with(&format!("Read the start of {title}"), LAYER, None).with_reply(format!("I couldn't make a summary just now. The page starts: {}", start.trim()))
                    }
                }
            }
            Ok((title, body)) if !body.is_empty() => Outcome::ok_with(&format!("Read {title}"), LAYER, None).with_reply(format!("It's a short page. It says: {body}")),
            Ok((title, _)) => Outcome::ok_with("Nothing to summarise", LAYER, None).with_reply(format!("{} has no text to summarise.", if title.is_empty() { "This page".to_owned() } else { title })),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "browser.form" => form(&c, &tab, args).await,
        _ => {
            let failed = c.failed_requests(&tab.id).await.unwrap_or_default();
            let net = if failed.is_empty() {
                String::new()
            } else {
                let shown: Vec<String> = failed.iter().take(4).map(|(p, st)| format!("{p} answered {st}")).collect();
                format!(" {} {} failed: {}.", failed.len(), if failed.len() == 1 { "request" } else { "requests" }, shown.join(", "))
            };
            let page = if tab.title.is_empty() { "This page".to_owned() } else { tab.title.clone() };
            match c.console_errors(&tab.id).await {
                Ok(list) if list.is_empty() && failed.is_empty() => Outcome::ok_with("No console errors", LAYER, None).with_reply(format!("{page} shows no errors in its console, and every request it made worked.")),
                Ok(list) if list.is_empty() => Outcome::ok_with("Failed requests", LAYER, None).with_reply(format!("{page} shows no errors in its console.{net}")),
                Ok(list) => {
                    let n = list.len();
                    let shown: Vec<String> = list.into_iter().take(3).map(|l| l.chars().take(160).collect()).collect();
                    Outcome::ok_with(&format!("{n} console {}", if n == 1 { "error" } else { "errors" }), LAYER, None).with_reply(format!("{page} has {n} {} in its console. {}.{net}", if n == 1 { "error" } else { "errors" }, shown.join(". ").trim_end_matches('.')))
                }
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
    }
}

fn field_score(want: &str, f: &Value) -> f64 {
    let label = crate::text::normalize(f.get("label").and_then(Value::as_str).unwrap_or_default());
    let kind = f.get("type").and_then(Value::as_str).unwrap_or_default();
    let w = crate::text::normalize(want).trim_end_matches(" field").trim_end_matches(" box").trim_start_matches("the ").trim_start_matches("my ").to_owned();
    if w.is_empty() {
        return 0.0;
    }
    if label == w || kind == w {
        return 1.0;
    }
    if label.contains(&w) || (w.contains(&label) && label.len() > 2) {
        return 0.9;
    }
    crate::os::score(&w, &label)
}

async fn form(c: &crate::proto::Conn, tab: &crate::cdp::Tab, args: &Value) -> Outcome {
    let fields = match c.eval_json(&tab.id, crate::proto::FORM_LIST_JS).await {
        Ok(Value::Array(a)) => a,
        Ok(_) => Vec::new(),
        Err(e) => return Outcome::fail(&e.to_string(), LAYER),
    };
    let label = |f: &Value| f.get("label").and_then(Value::as_str).unwrap_or_default().to_owned();
    match s(args, "op").as_str() {
        "fill" => {
            let want = s(args, "field");
            let value = args.get("value").and_then(Value::as_str).unwrap_or_default().to_owned();
            let best = fields.iter().map(|f| (field_score(&want, f), f)).filter(|(sc, _)| *sc >= 0.7).max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)).map(|(_, f)| f.clone());
            let Some(f) = best else {
                let names: Vec<String> = fields.iter().map(&label).filter(|l| !l.is_empty()).collect();
                return Outcome::fail(&format!("This form has no {want} field. It has {}.", crate::skills::join_names_pub(&names)), LAYER);
            };
            if f.get("type").and_then(Value::as_str) == Some("password") {
                return Outcome::fail("I don't type passwords said out loud, because people nearby can hear them. Type it yourself, or keep it in Setu's password vault.", LAYER);
            }
            let i = f.get("i").and_then(Value::as_u64).unwrap_or(0) as usize;
            match c.eval_json(&tab.id, &crate::proto::form_fill_js(i, &value)).await {
                Ok(r) if r.get("ok").and_then(Value::as_bool) == Some(true) => Outcome::ok_with(&format!("Filled {}", label(&f)), LAYER, None),
                Ok(r) if r.get("why").and_then(Value::as_str) == Some("choice") => Outcome::fail(&format!("{} has no choice called {value}.", label(&f)), LAYER),
                Ok(_) => Outcome::fail("That field went away. Say fill in this form again.", LAYER),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "submit" => match c.eval_json(&tab.id, crate::proto::FORM_SUBMIT_JS).await {
            Ok(r) if r.get("ok").and_then(Value::as_bool) == Some(true) => Outcome::ok_with("Sent the form", LAYER, None),
            Ok(_) => Outcome::fail("This page has no form to send.", LAYER),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        _ => {
            if fields.is_empty() {
                return Outcome::ok_with("No form", LAYER, None).with_reply("This page has no form to fill in.".into());
            }
            let empty: Vec<String> = fields.iter().filter(|f| f.get("filled").and_then(Value::as_bool) != Some(true)).map(&label).filter(|l| !l.is_empty()).collect();
            let all: Vec<String> = fields.iter().map(&label).filter(|l| !l.is_empty()).collect();
            let first = empty.first().or(all.first()).cloned().unwrap_or_default();
            Outcome::ok_with(&format!("A form with {} fields", fields.len()), LAYER, None).with_reply(format!("This form asks for {}. Say, for example, fill {} with, then what to type. Say send the form when it's done.", crate::skills::join_names_pub(&all), first.to_lowercase()))
        }
    }
}

fn import_question(app: &crate::tools::apps::App, old: &[crate::profiles::Profile], lead: &str, kind: &'static str) -> Outcome {
    let display = short_name(&app.name);
    let names: Vec<String> = old.iter().map(|p| if p.name.is_empty() { p.dir.clone() } else { p.name.clone() }).collect();
    let mut choices = vec![Choice::new("Bring everything", &["everything", "all", "bring everything", "yes", "all of them", "import everything"], act("browser.import", json!({ "browser": app.name, "which": "all" })))];
    if old.len() > 1 {
        for (p, n) in old.iter().zip(names.iter()) {
            let lower = n.to_lowercase();
            choices.push(Choice::new(n, &[lower.as_str(), &format!("just {lower}"), &format!("only {lower}")], act("browser.import", json!({ "browser": app.name, "which": p.dir }))));
        }
    }
    choices.push(Choice::new("Start empty", &["none", "start empty", "nothing", "no", "empty", "not now"], act("browser.import", json!({ "browser": app.name, "which": "none" }))));
    let question = if old.len() == 1 {
        format!("{lead} Shall I bring over your bookmarks, history, passwords, extensions and settings from your old {display} profile?")
    } else {
        format!("{lead} Shall I bring over everything from your {} {display} profiles ({}), just one of them, or start empty?", old.len(), media_names(&names))
    };
    Outcome::asking(
        "Asked what to bring over",
        LAYER,
        FollowUp { question: question.clone(), spoken: Some(question), choices, cleanup: None, kind, focus: None },
    )
}

fn media_names(n: &[String]) -> String {
    match n.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

async fn import(core: &Arc<Core>, args: &Value) -> Outcome {
    let name = s(args, "browser");
    let Some(app) = web::find_browser(core, if name.is_empty() { None } else { Some(&name) }) else { return Outcome::fail("No browser is installed.", LAYER) };
    let display = short_name(&app.name);
    let Some(key) = crate::tools::browsers::key_of(&app) else { return Outcome::fail(&format!("I can't bring profiles into {display}."), LAYER) };
    let data = core.paths.data.clone();
    if !crate::cdp::has_profile(&data, key) {
        return Outcome::fail(&format!("{display} has no Setu profile yet. Say set up {display} first."), LAYER);
    }
    crate::looks::close(core);
    let which = s(args, "which");
    if which == "none" {
        return Outcome::ok_with("Started empty", LAYER, None).with_reply(format!("Okay, the Setu profile starts empty. Say import my {display} profile any time to bring things over."));
    }
    let (root, old) = crate::profiles::read(key);
    let Some(root) = root else { return Outcome::fail(&format!("I couldn't find your old {display} profiles."), LAYER) };
    let want = crate::text::normalize(&which);
    let dirs: Vec<String> = args.get("dirs").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let chosen: Vec<crate::profiles::Profile> = if !dirs.is_empty() {
        old.iter().filter(|p| dirs.contains(&p.dir)).cloned().collect()
    } else if which.is_empty() || which == "all" {
        old.clone()
    } else {
        old.iter().filter(|p| p.dir == which || crate::text::normalize(&p.name) == want || crate::text::normalize(&p.email) == want).cloned().collect()
    };
    if chosen.is_empty() {
        return Outcome::fail(&format!("I couldn't find a {display} profile called {which}."), LAYER);
    }
    let main_dir = s(args, "main");
    let main = chosen.iter().find(|p| !main_dir.is_empty() && p.dir == main_dir).or_else(|| chosen.iter().find(|p| p.current)).cloned().unwrap_or_else(|| chosen[0].clone());
    if let Some(c) = crate::proto::connected(key) {
        c.shutdown().await;
    }
    let dst = crate::cdp::profile_dir(&data, key);
    let firefox = crate::tools::browsers::family(key) == "firefox";
    let job = {
        let (root, dst, chosen, main) = (root.clone(), dst.clone(), chosen.clone(), main.clone());
        move || if firefox { crate::import::firefox(&root, &dst, &main) } else { crate::import::chromium(&root, &dst, &chosen, &main) }
    };
    let span = crate::layers::effect(crate::layers::Layer::System, "system.import");
    let r = span.end(crate::work::background("import", job).await.and_then(|r| r));
    if r.is_ok() {
        crate::looks::reapply(&dst, crate::tools::browsers::family(key));
    }
    let exe = crate::cdp::exe_of(&app.exec, &app.path);
    let restarted = crate::proto::ensure(&data, key, &exe, None).await.is_ok();
    match r {
        Ok(rep) => {
            let mb = rep.bytes >> 20;
            let again = if restarted { "" } else { " Open it again when you're ready." };
            Outcome::ok_with(&format!("Brought over {}", media_names(&rep.profiles)), LAYER, Some(app.name.clone())).with_reply(format!("Brought over {} ({mb} MB) into your Setu profile. Your old profile is unchanged. Some sites may ask you to sign in again.{again}", media_names(&rep.profiles)))
        }
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    }
}

pub fn offer_setup(app: &crate::tools::apps::App, resume: Value) -> Outcome {
    let display = short_name(&app.name);
    let yes = act("browser.profile", json!({ "browser": app.name, "op": "create", "resume": resume }));
    let no = act("browser.profile", json!({ "browser": app.name, "op": "decline", "resume": resume }));
    let question = format!("That needs full control of {display}, so I need to set up a Setu profile. Your current profile stays as it is. Shall I?");
    Outcome::asking(
        "Asked to set up a Setu profile",
        LAYER,
        FollowUp {
            question: question.clone(),
            spoken: Some(question),
            choices: vec![Choice::new("Set it up", &["yes", "set it up", "okay", "ok", "sure", "go ahead", "do it"], yes), Choice::new("Not now", &["no", "not now", "later", "cancel", "don't"], no)],
            cleanup: None,
            kind: "confirm",
            focus: None,
        },
    )
}

const LINKS_JS: &str = "JSON.stringify(((kind)=>{const vis=e=>{const r=e.getBoundingClientRect();const s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'};let els=[];if(kind==='result'){els=[...document.querySelectorAll('a h3, a[data-testid=result-title-a], li.b_algo h2 a, a.result__a, [data-result] h2 a, .g a h3')].map(e=>e.closest('a')).filter(Boolean)}if(!els.length){els=[...document.querySelectorAll('main a[href], article a[href], [role=main] a[href]')];if(!els.length)els=[...document.querySelectorAll('a[href]')];els=els.filter(a=>(a.innerText||a.title||'').trim().length>(kind==='result'?12:0))}els=[...new Set(els)].filter(vis).filter(a=>!/^(javascript:|#$)/i.test((a.getAttribute('href')||'').trim()));window.__setuLinks=els;return els.slice(0,60).map(a=>({t:(a.innerText||a.title||'').trim().replace(/\\s+/g,' ').slice(0,120),h:a.href}))})(KIND))";

fn find_js(q: &str) -> String {
    let q = serde_json::to_string(q).unwrap_or_else(|_| "\"\"".into());
    format!("JSON.stringify((q=>{{const t=((document.body&&document.body.innerText)||'').toLowerCase();const n=q?t.split(q.toLowerCase()).length-1:0;if(n&&window.find){{const s=window.getSelection();if(s)s.removeAllRanges();window.find(q,false,false,true)}}return {{n}}}})({q}))")
}

async fn front_conn() -> Option<crate::proto::Conn> {
    crate::proto::any()?;
    let win = crate::os::working().await.ok()?;
    let key = browser_of(&win.app)?;
    crate::proto::connected(&key)
}

fn nth_word(n: usize) -> String {
    match n {
        1 => "first".into(),
        2 => "second".into(),
        3 => "third".into(),
        _ => format!("number {n}"),
    }
}

pub async fn protocol_first(tool: &str, args: &Value) -> Option<Outcome> {
    let c = front_conn().await?;
    let done = |did: &str| Outcome::ok_with(did, LAYER, None);
    let target = args.get("target").cloned().unwrap_or(Value::Null);
    match tool {
        "browser.tab.new" => {
            let key = match &c {
                crate::proto::Conn::Cdp(x) => x.key.clone(),
                crate::proto::Conn::Bidi(x) => x.key.clone(),
            };
            let url = if browsers::family(&key) == "firefox" { "about:blank" } else { "chrome://newtab/" };
            let id = c.open(url).await.ok()?;
            let _ = c.activate(&id).await;
            Some(done("New tab"))
        }
        "browser.tab.activate" => {
            let q = target.get("text").or_else(|| target.get("name")).or_else(|| target.get("query")).and_then(Value::as_str)?;
            let tabs = c.tabs().await.ok()?;
            let hits = crate::cdp::find_tab(&tabs, q);
            if hits.len() != 1 {
                return None;
            }
            c.activate(&hits[0].id).await.ok()?;
            Some(done(&format!("Switched to {}", hits[0].title)))
        }
        "browser.tab.close" if target.get("active").and_then(Value::as_bool) == Some(true) => {
            let t = crate::proto::active_tab(&c).await.ok()?;
            c.close(&t.id).await.ok()?;
            Some(done("Closed the tab"))
        }
        "browser.tab.duplicate" => {
            let t = crate::proto::active_tab(&c).await.ok()?;
            let id = c.open(&t.url).await.ok()?;
            let _ = c.activate(&id).await;
            Some(done("Duplicated the tab"))
        }
        "browser.tabs" => {
            let tabs = c.tabs().await.ok()?;
            let n = tabs.len();
            if args.get("count").and_then(Value::as_bool) == Some(true) || n > 8 {
                return Some(done("Counted the tabs").with_reply(if n == 1 { "One tab is open.".into() } else { format!("{n} tabs are open.") }));
            }
            let names: Vec<String> = tabs.iter().map(|t| if t.title.trim().is_empty() { web::host(&t.url) } else { t.title.chars().take(60).collect() }).collect();
            Some(done("Listed the tabs").with_reply(format!("{} open: {}.", if n == 1 { "One tab is".to_owned() } else { format!("{n} tabs are") }, names.join(", "))))
        }
        "browser.nav" => {
            let t = crate::proto::active_tab(&c).await.ok()?;
            let (js, did) = match s(args, "op").as_str() {
                "back" => ("JSON.stringify((history.length>1&&history.back(),true))", "Went back"),
                "forward" => ("JSON.stringify((history.forward(),true))", "Went forward"),
                _ => ("JSON.stringify((location.reload(),true))", "Refreshed"),
            };
            c.eval_json(&t.id, js).await.ok()?;
            Some(done(did))
        }
        "browser.page" => {
            let action = s(args, "action");
            let t = crate::proto::active_tab(&c).await.ok()?;
            if action == "find" {
                let q = s(args, "text");
                let v = c.eval_json(&t.id, &find_js(&q)).await.ok()?;
                let n = v.get("n").and_then(Value::as_u64).unwrap_or(0);
                return Some(done("Searched the page").with_reply(match n {
                    0 => format!("{} isn't on this page.", sentence_case(&q)),
                    1 => format!("Found one match for {q}."),
                    n => format!("Found {n} matches for {q}."),
                }));
            }
            let r = args.get("ref").cloned().unwrap_or(Value::Null);
            if !matches!(action.as_str(), "open" | "click" | "") || r.get("number").is_some_and(|v| !v.is_null()) {
                return None;
            }
            let n = r.get("n").and_then(Value::as_i64).filter(|n| *n >= 1)? as usize;
            let kind = r.get("kind").and_then(Value::as_str).unwrap_or("link");
            if !matches!(kind, "link" | "result") {
                return None;
            }
            let v = c.eval_json(&t.id, &LINKS_JS.replace("KIND", &serde_json::to_string(kind).unwrap_or_default())).await.ok()?;
            let list = v.as_array()?;
            let Some(hit) = list.get(n - 1) else {
                return Some(done("Not found").with_reply(format!("This page has {} {}{}.", list.len(), kind, if list.len() == 1 { "" } else { "s" })));
            };
            let title = hit.get("t").and_then(Value::as_str).unwrap_or_default().to_owned();
            let place = s(args, "where");
            if !place.is_empty() && place != "here" {
                let href = hit.get("h").and_then(Value::as_str)?;
                let id = c.open(href).await.ok()?;
                let _ = c.activate(&id).await;
            } else {
                c.eval_json(&t.id, &format!("JSON.stringify((window.__setuLinks&&window.__setuLinks[{}]&&window.__setuLinks[{}].click(),true))", n - 1, n - 1)).await.ok()?;
            }
            Some(done(&format!("Opened the {} {kind}", nth_word(n))).with_reply(if title.is_empty() { format!("Opened the {} {kind}.", nth_word(n)) } else { format!("Opened {}.", title) }))
        }
        _ => None,
    }
}

async fn tabs_op(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    if let Some(o) = protocol_first(tool, args).await {
        return o;
    }
    if tool == "browser.tab.close"
        && let Some(o) = close_by_protocol(args).await
    {
        return o;
    }
    let name = s(args, "browser");
    let target = args.get("target").cloned().unwrap_or(json!({ "active": true }));
    let active = target.get("active").and_then(Value::as_bool).unwrap_or(false) || matches!(tool, "browser.tab.duplicate" | "browser.tab.pin" | "browser.tab.detach");
    let native = match tool {
        "browser.tab.close" => crate::browserui::close(&name, &target).await,
        "browser.tab.activate" => crate::browserui::switch(&name, &target).await,
        "browser.tab.new" => crate::browserui::new_tab(&name).await,
        "browser.tab.pin" => crate::browserui::pin(&name, &target, args.get("pinned").and_then(Value::as_bool).unwrap_or(true)).await,
        "browser.tab.duplicate" => crate::browserui::duplicate(core, &name).await,
        "browser.tab.detach" => crate::browserui::detach(core, &name).await,
        "browser.tab.reopen" => crate::browserui::reopen(core, &name).await,
        _ => None,
    };
    if let Some(o) = native {
        return o;
    }
    if tool == "browser.tab.new" {
        let in_browser = crate::tools::atspi::active_app().await.is_some_and(|(app, _)| browser_of(&app).is_some());
        if !in_browser {
            let key = key_of(core, &name);
            let firefox = matches!(key.as_str(), "firefox" | "librewolf" | "zen");
            let flags = if firefox { vec!["-new-tab".to_owned(), "about:newtab".to_owned()] } else { vec!["chrome://newtab".to_owned()] };
            return match launch(core, &name, flags).await {
                Ok(b) => Outcome::ok_with(&format!("New tab in {b}"), LAYER, None),
                Err(e) => Outcome::fail(&e, LAYER),
            };
        }
    }
    let fallback = match tool {
        "browser.tab.close" if active => Some(("ctrl w", "Closed the tab")),
        "browser.tab.reopen" => Some(("ctrl shift t", "Reopened the tab")),
        "browser.tab.new" => Some(("ctrl t", "New tab")),
        _ => None,
    };
    match fallback {
        Some((k, label)) => keys(k, label).await,
        None => unreachable(core).await,
    }
}

async fn list_tabs(core: &Arc<Core>, args: &Value) -> Outcome {
    if let Some(o) = protocol_first("browser.tabs", args).await {
        return o;
    }
    match crate::browserui::list(&s(args, "browser"), args.get("count").and_then(Value::as_bool).unwrap_or(false)).await {
        Some(o) => o,
        None => unreachable(core).await,
    }
}

async fn window(core: &Arc<Core>, args: &Value) -> Outcome {
    let private = args.get("private").and_then(Value::as_bool).unwrap_or(false);
    let name = s(args, "browser");
    let key = key_of(core, &name);
    let flag = if private { private_flag(&key) } else { window_flag(&key) };
    match launch(core, &name, vec![flag.to_owned()]).await {
        Ok(b) => Outcome::ok_with(&format!("Opened a {} window in {b}", if private { "private" } else { "new" }), LAYER, None),
        Err(e) => Outcome::fail(&e, LAYER),
    }
}

const CHROMIUM_PAGES: &[(&str, &str)] = &[
    ("settings", "chrome://settings"),
    ("history", "chrome://history"),
    ("downloads", "chrome://downloads"),
    ("bookmarks", "chrome://bookmarks"),
    ("extensions", "chrome://extensions"),
    ("passwords", "chrome://password-manager/passwords"),
    ("clear", "chrome://settings/clearBrowserData"),
    ("search", "chrome://settings/searchEngines"),
    ("appearance", "chrome://settings/appearance"),
    ("startup", "chrome://settings/onStartup"),
];

const FIREFOX_PAGES: &[(&str, &str)] = &[
    ("settings", "about:preferences"),
    ("history", "about:preferences#privacy"),
    ("downloads", "about:downloads"),
    ("extensions", "about:addons"),
    ("passwords", "about:logins"),
    ("clear", "about:preferences#privacy"),
    ("search", "about:preferences#search"),
    ("appearance", "about:preferences#general"),
    ("startup", "about:preferences#home"),
];

fn settings_label(page: &str) -> &'static str {
    match page {
        "history" => "history",
        "downloads" => "downloads",
        "bookmarks" => "bookmarks",
        "extensions" => "extensions",
        "passwords" => "passwords",
        "clear" => "clear browsing data",
        "search" => "search engine settings",
        "appearance" => "appearance settings",
        "startup" => "startup settings",
        "privacy" => "privacy settings",
        "security" => "security settings",
        "site" | "notifications" | "popups" | "camera" | "microphone" | "location" | "cookies" => "site settings",
        "about" => "the about page",
        _ => "settings",
    }
}

async fn settings(core: &Arc<Core>, args: &Value) -> Outcome {
    let page = s(args, "page");
    let goal = s(args, "goal");
    let name = s(args, "browser");
    let label = settings_label(&page);
    let hint = match (page.as_str(), goal.is_empty()) {
        ("search", false) => Some(format!("Choose {goal} under search engine.")),
        ("startup", false) => Some(format!("Choose open a specific page, then add {goal}.")),
        ("appearance", false) => Some(format!("Choose {goal} under mode or theme.")),
        ("clear", _) => Some("Pick what to clear, then choose delete.".to_owned()),
        _ => None,
    };
    let done = |o: Outcome| match &hint {
        Some(h) => o.with_reply(h.clone()),
        None => o,
    };
    let key = key_of(core, &name);
    let firefox = matches!(key.as_str(), "firefox" | "librewolf" | "zen");
    let table = if firefox { FIREFOX_PAGES } else { CHROMIUM_PAGES };
    let url = table.iter().find(|(k, _)| *k == page).or_else(|| table.first()).map(|(_, u)| (*u).to_owned()).unwrap_or_default();
    match launch(core, &name, vec![url]).await {
        Ok(_) => done(Outcome::ok_with(&format!("Opened {label}"), LAYER, None)),
        Err(e) => Outcome::fail(&e, LAYER),
    }
}

fn permission_page(kind: &str) -> &'static str {
    match kind {
        "popups" => "popups",
        "notifications" => "notifications",
        "camera" => "camera",
        "microphone" => "microphone",
        "location" => "location",
        _ => "site",
    }
}

async fn permission(core: &Arc<Core>, args: &Value) -> Outcome {
    let b = browser_display(&key_of(core, &s(args, "browser")));
    let what = s(args, "what");
    let kind = s(args, "kind");
    if s(args, "scope") == "site"
        && let Some(name) = match kind.as_str() {
            "notifications" => Some("notifications"),
            "camera" => Some("camera"),
            "microphone" => Some("microphone"),
            "location" => Some("geolocation"),
            _ => None,
        }
        && let Some(c) = crate::proto::any()
        && let Ok(tab) = crate::proto::active_tab(&c).await
        && let Some(origin) = origin_of(&tab.url)
    {
        let state = match s(args, "setting").as_str() {
            "allow" => "granted",
            "block" => "denied",
            _ => "prompt",
        };
        let site = origin.split("://").nth(1).unwrap_or(&origin).to_owned();
        return match c.set_permission(name, state, &origin).await {
            Ok(()) => Outcome::ok_with(&format!("{} {what} on {site}", match state { "granted" => "Allowed", "denied" => "Blocked", _ => "Asking for" }), "protocol", None),
            Err(e) => Outcome::fail(&format!("The browser didn't change that: {e}"), "protocol"),
        };
    }
    settings(core, &json!({ "page": permission_page(&s(args, "kind")), "browser": s(args, "browser") })).await.with_reply(format!("{b} keeps {what} on this settings page."))
}

async fn bookmark(core: &Arc<Core>, args: &Value) -> Outcome {
    let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
    match crate::browserui::bookmark(&s(args, "browser"), remove).await {
        Some(o) => o,
        None => unreachable(core).await,
    }
}

async fn page_url(args: &Value) -> Outcome {
    match crate::browserui::copy_url(&s(args, "browser")).await {
        Some(o) => o,
        None => keys("ctrl l", "Address bar").await,
    }
}

async fn menu(core: &Arc<Core>, args: &Value) -> Outcome {
    if args.get("native").is_some_and(|n| !n.is_null()) {
        return crate::browserui::menu_choose(core, args).await;
    }
    if args.get("close").and_then(Value::as_bool).unwrap_or(false) {
        return Outcome::ok_with("Closed the options", LAYER, None);
    }
    unreachable(core).await
}

async fn setup(core: &Arc<Core>, args: &Value) -> Outcome {
    let name = s(args, "browser");
    let Some(app) = web::find_browser(core, if name.is_empty() { None } else { Some(&name) }) else { return Outcome::fail("No browser is installed.", LAYER) };
    let display = short_name(&app.name);
    let _ = crate::os::ready().await;
    let Some(key) = crate::tools::browsers::key_of(&app) else {
        return Outcome::ok_with(&format!("{display} is ready"), LAYER, None).with_reply(format!("{display} is ready. I work in it directly, so nothing needs to be installed."));
    };
    if !crate::proto::supported(key) {
        return Outcome::ok_with(&format!("{display} is ready"), LAYER, None).with_reply(format!("{display} is ready. I work in it directly through accessibility."));
    }
    let data = core.paths.data.clone();
    if crate::cdp::has_profile(&data, key) {
        let exe = crate::cdp::exe_of(&app.exec, &app.path);
        if !crate::cdp::is_default(&data, key) {
            let me = std::env::current_exe().unwrap_or_default();
            let _ = crate::cdp::make_default(&data, key, &app, &me);
        }
        return match crate::proto::ensure(&data, key, &exe, None).await {
            Ok((c, _)) => {
                let tabs = c.tabs().await.map(|t| t.len()).unwrap_or(0);
                Outcome::ok_with(&format!("{display} has full control"), LAYER, Some(app.name.clone())).with_reply(format!("{display} is running on your Setu profile with full control, {tabs} {}.", if tabs == 1 { "tab" } else { "tabs" }))
            }
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        };
    }
    let yes = act("browser.profile", json!({ "browser": app.name, "op": "create" }));
    let no = act("browser.profile", json!({ "browser": app.name, "op": "decline" }));
    let question = format!("For full control of {display} I need to set up a Setu profile. Your current profile stays as it is. Shall I?");
    Outcome::asking(
        "Asked to set up a Setu profile",
        LAYER,
        FollowUp {
            question: question.clone(),
            spoken: Some(question),
            choices: vec![Choice::new("Set it up", &["yes", "set it up", "okay", "ok", "sure", "go ahead", "do it"], yes), Choice::new("Not now", &["no", "not now", "later", "cancel", "don't"], no)],
            cleanup: None,
            kind: "confirm",
            focus: None,
        },
    )
}

async fn profile(core: &Arc<Core>, args: &Value) -> Outcome {
    let name = s(args, "browser");
    let Some(app) = web::find_browser(core, if name.is_empty() { None } else { Some(&name) }) else { return Outcome::fail("No browser is installed.", LAYER) };
    let display = short_name(&app.name);
    let Some(key) = crate::tools::browsers::key_of(&app) else { return Outcome::fail(&format!("I can't set up a Setu profile for {display}."), LAYER) };
    let data = core.paths.data.clone();
    if s(args, "op") == "old" {
        return match crate::cdp::unmake_default(&data, key, &app) {
            Ok(()) => Outcome::ok_with(&format!("{display} opens on your old profile"), LAYER, None).with_reply(format!("Done. {display} will open on your old profile again. The Setu profile is kept; say set up {display} to use it again.")),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        };
    }
    if s(args, "op") == "decline" {
        crate::cdp::decline(&data, key);
        return Outcome::ok_with("Kept the normal profile", LAYER, None).with_reply(format!("Okay. I'll keep using {display} as it is. Say set up {display} if you change your mind."));
    }
    let Some(look) = args.get("look").filter(|l| l.is_object()).cloned() else {
        if crate::cdp::has_profile(&data, key) {
            return setup(core, &json!({ "browser": app.name })).await;
        }
        return crate::looks::open(core, &app, key, args.get("resume").cloned().unwrap_or(Value::Null), false);
    };
    match crate::cdp::create_profile(&data, key) {
        Ok(dir) => {
            if let Err(e) = crate::looks::apply(&dir, crate::tools::browsers::family(key), &look) {
                log::warn!("the Setu profile's look was not saved: {e}");
            }
        }
        Err(e) => return Outcome::fail(&format!("I couldn't make the Setu profile: {e}"), LAYER),
    }
    let named = look.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(|n| format!(" called {n}")).unwrap_or_default();
    let me = std::env::current_exe().unwrap_or_default();
    if let Err(e) = crate::cdp::make_default(&data, key, &app, &me) {
        log::warn!("the Setu profile was made but not set as the default: {e}");
    }
    let exe = crate::cdp::exe_of(&app.exec, &app.path);
    match crate::proto::ensure(&data, key, &exe, None).await {
        Ok(_) => {
            if let Some(r) = args.get("resume")
                && let Some(tool) = r.get("tool").and_then(Value::as_str)
            {
                let mut o = Box::pin(crate::skills::run(core, tool, r.get("args").cloned().unwrap_or(json!({})), std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))).await;
                o.reply = Some(format!("{display} now runs on a new Setu profile{named} with full control. {}", o.reply.clone().unwrap_or_else(|| format!("{}.", o.message))));
                return o;
            }
            let (_, old) = crate::profiles::read(key);
            if old.is_empty() {
                return Outcome::ok_with(&format!("{display} has a Setu profile"), LAYER, Some(app.name.clone())).with_reply(format!("Done. {display} now runs on a new Setu profile{named} with full control. Your old profile is unchanged."));
            }
            import_question(&app, &old, &format!("Done. {display} now runs on a new Setu profile{named} with full control."), "look-import")
        }
        Err(e) => Outcome::fail(&format!("The Setu profile is made, but {display} didn't start on it: {e}"), LAYER),
    }
}

async fn close_overlay(core: &Arc<Core>) -> Outcome {
    if crate::browserui::clear_numbers(core) {
        return Outcome::ok_with("Closed", LAYER, None);
    }
    keys("escape", "Closed").await
}

pub async fn run(core: &Arc<Core>, tool: &str, args: Value) -> Outcome {
    run_once(core, tool, args).await
}

async fn run_once(core: &Arc<Core>, tool: &str, args: Value) -> Outcome {
    match tool {
        "browser.choose" => choose(core, &args).await,
        "browser.setup" => setup(core, &args).await,
        "browser.profile" => profile(core, &args).await,
        "browser.import" => import(core, &args).await,
        "browser.capture" | "browser.read" | "browser.summary" | "browser.console" | "browser.form" => deep(core, tool, &args).await,
        "browser.window" => window(core, &args).await,
        "browser.open" => open(core, &args).await,
        "browser.tab.close" | "browser.tab.new" | "browser.tab.activate" | "browser.tab.reopen" | "browser.tab.duplicate" | "browser.tab.pin" | "browser.tab.detach" => tabs_op(core, tool, &args).await,
        "browser.tabs" => list_tabs(core, &args).await,
        "browser.page" => page_act(core, args).await,
        "browser.menu" => menu(core, &args).await,
        "browser.settings" => settings(core, &args).await,
        "browser.permission" => permission(core, &args).await,
        "browser.bookmark" => bookmark(core, &args).await,
        "browser.pageurl" => page_url(&args).await,
        "browser.close_overlay" => close_overlay(core).await,
        _ => Outcome::fail("I don't know that browser command.", LAYER),
    }
}

pub fn from_model(action: &str, target: &str, where_: &str) -> Intent {
    let t = normalize(target);
    let w = if matches!(where_, "tab" | "window" | "private") { where_ } else { "here" };
    match action {
        "new_tab" => match target_url(&t) {
            Some(mut u) => {
                u["where"] = json!("tab");
                act("browser.open", u)
            }
            None => act("browser.tab.new", json!({})),
        },
        "new_window" => act("browser.window", json!({ "private": false })),
        "private_window" => act("browser.window", json!({ "private": true })),
        "close_tab" => act("browser.tab.close", json!({ "target": tab_ref(&format!("{} tab", t.trim_end_matches(" tab"))).filter(|_| !t.is_empty()).unwrap_or(json!({ "active": true })) })),
        "switch_tab" => match tab_ref(&format!("{} tab", t.trim_end_matches(" tab"))) {
            Some(r) => act("browser.tab.activate", json!({ "target": r })),
            None => Intent::Unknown,
        },
        "open" => {
            if let Some(r) = parse_ref(&t) {
                return page("open", &r, json!({ "where": w, "full": r.kind == Some("image") && w != "here" }));
            }
            match target_url(&t) {
                Some(mut u) => {
                    u["where"] = json!(if w == "here" { "tab" } else { w });
                    act("browser.open", u)
                }
                None if !t.is_empty() => act("browser.open", json!({ "query": t, "where": if w == "here" { "tab" } else { w } })),
                None => Intent::Unknown,
            }
        }
        "click" if !t.is_empty() => page("click", &parse_loose(&t), json!({})),
        "options" => page("menu", &if t.is_empty() { Ref { this: true, ..Default::default() } } else { parse_loose(&t) }, json!({})),
        "save" => page("save", &parse_ref(&t).unwrap_or(Ref { kind: Some("image"), this: true, ..Default::default() }), json!({})),
        "copy" => match parse_ref(&t) {
            Some(r) => {
                let what = if r.kind == Some("image") { "image" } else { "link" };
                page("copy", &r, json!({ "what": what }))
            }
            None => act("browser.pageurl", json!({})),
        },
        "settings" => {
            let probe = if t.is_empty() { "open browser settings".to_owned() } else if t.contains("setting") { format!("open browser {t}") } else { format!("open browser {t} settings") };
            let (p, goal) = settings_page(&probe).unwrap_or(("settings", None));
            act("browser.settings", json!({ "page": p, "goal": goal }))
        }
        _ => Intent::Unknown,
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    let what = s(args, "what");
    match tool {
        "browser.setup" => "Setting up the browser".into(),
        "browser.profile" => "Setting up the Setu profile".into(),
        "browser.import" => "Bringing your profile over".into(),
        "browser.capture" => if s(args, "kind") == "pdf" { "Saving the page as a PDF".into() } else { "Taking a picture of the page".into() },
        "browser.read" => "Reading the page".into(),
        "browser.summary" => "Summarising the page".into(),
        "browser.console" => "Checking the page".into(),
        "browser.form" => match s(args, "op").as_str() {
            "fill" => format!("Filling {}", s(args, "field")),
            "submit" => "Sending the form".into(),
            _ => "Reading the form".into(),
        },
        "browser.choose" => "Choosing the browser".into(),
        "browser.window" => if args.get("private").and_then(Value::as_bool).unwrap_or(false) { "Opening a private window".into() } else { "Opening a new window".into() },
        "browser.open" => "Opening".into(),
        "browser.tab.new" => "New tab".into(),
        "browser.tab.close" => "Closing the tab".into(),
        "browser.tab.activate" => "Switching tab".into(),
        "browser.tab.reopen" => "Reopening the tab".into(),
        "browser.tab.duplicate" => "Duplicating the tab".into(),
        "browser.tab.pin" => "Pinning the tab".into(),
        "browser.tab.detach" => "Moving the tab".into(),
        "browser.tabs" => "Checking tabs".into(),
        "browser.page" => match s(args, "action").as_str() {
            "menu" => format!("Options for {what}"),
            "click" => format!("Clicking {what}"),
            "open" => format!("Opening {what}"),
            "save" => format!("Saving {what}"),
            "copy" => format!("Copying {what}"),
            "lens" => "Searching the image".into(),
            "numbers" => "Numbering".into(),
            "find" => format!("Finding {}", s(args, "text")),
            _ => "Working in the page".into(),
        },
        "browser.menu" => "Choosing".into(),
        "browser.settings" => format!("Opening {}", settings_label(&s(args, "page"))),
        "browser.permission" => format!("Changing {}", s(args, "what")),
        "browser.bookmark" => "Bookmarking".into(),
        "browser.pageurl" => "Copying the page link".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Ref {
        parse_ref(s).unwrap_or_else(|| panic!("no ref for {s}"))
    }

    fn tool(i: Option<Intent>) -> (String, Value) {
        match i {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => panic!("not a skill: {other:?}"),
        }
    }

    #[test]
    fn positional_references() {
        let x = r("the third image from left");
        assert_eq!((x.kind, x.n, x.from), (Some("image"), Some(3), Some("left")));
        let x = r("3rd image from the left");
        assert_eq!((x.kind, x.n, x.from), (Some("image"), Some(3), Some("left")));
        let x = r("second image from the right in the third row");
        assert_eq!((x.kind, x.n, x.from, x.row), (Some("image"), Some(2), Some("right"), Some(3)));
        let x = r("the last link");
        assert_eq!((x.kind, x.n), (Some("link"), Some(-1)));
        let x = r("the second last result");
        assert_eq!((x.kind, x.n), (Some("result"), Some(-2)));
        let x = r("image 4");
        assert_eq!((x.kind, x.number), (Some("image"), Some(4)));
        let x = r("image number 4");
        assert_eq!((x.kind, x.number), (Some("image"), Some(4)));
        let x = r("the link that says pricing");
        assert_eq!((x.kind, x.text.as_deref()), (Some("link"), Some("pricing")));
        let x = r("the pricing link");
        assert_eq!((x.kind, x.text.as_deref()), (Some("link"), Some("pricing")));
        let x = r("the image of a cat");
        assert_eq!((x.kind, x.text.as_deref()), (Some("image"), Some("cat")));
        let x = r("this image");
        assert_eq!((x.kind, x.this), (Some("image"), true));
        let x = r("the first video about cooking");
        assert_eq!((x.kind, x.n, x.text.as_deref()), (Some("video"), Some(1), Some("cooking")));
        let x = r("the leftmost image");
        assert_eq!((x.kind, x.n, x.from), (Some("image"), Some(1), Some("left")));
        let x = r("the top left image");
        assert_eq!((x.kind, x.n), (Some("image"), Some(1)));
        let x = r("5");
        assert_eq!(x.number, Some(5));
        assert!(parse_ref("youtube").is_none());
        assert!(parse_ref("sign in").is_none());
        assert_eq!(r("the third image from left").describe(), "the third image from the left");
    }

    #[test]
    fn browser_phrases() {
        assert_eq!(tool(rules("new tab", "")).0, "browser.tab.new");
        assert_eq!(tool(rules("open a new window", "")).1["private"], false);
        assert_eq!(tool(rules("open an incognito window", "")).1["private"], true);
        let (t, a) = tool(rules("open youtube in a new tab", ""));
        assert_eq!((t.as_str(), a["site"].as_str(), a["where"].as_str()), ("browser.open", Some("YouTube"), Some("tab")));
        let (t, a) = tool(rules("open github dot com in a new window", ""));
        assert_eq!((t.as_str(), a["url"].as_str(), a["where"].as_str()), ("browser.open", Some("https://github.com"), Some("window")));
        let (t, a) = tool(rules("open the third link in a new tab", ""));
        assert_eq!((t.as_str(), a["action"].as_str(), a["where"].as_str(), a["ref"]["n"].as_i64()), ("browser.page", Some("open"), Some("tab"), Some(3)));
        let (_, a) = tool(rules("open the 3rd image from left in a new tab", ""));
        assert_eq!((a["full"].as_bool(), a["ref"]["from"].as_str()), (Some(true), Some("left")));
        let (_, a) = tool(rules("show options for the 3rd image from left", ""));
        assert_eq!((a["action"].as_str(), a["ref"]["kind"].as_str(), a["ref"]["n"].as_i64()), (Some("menu"), Some("image"), Some(3)));
        let (_, a) = tool(rules("right click on the pricing link", ""));
        assert_eq!((a["action"].as_str(), a["ref"]["text"].as_str()), (Some("menu"), Some("pricing")));
        let (_, a) = tool(rules("show options for sign in", ""));
        assert_eq!((a["action"].as_str(), a["ref"]["text"].as_str()), (Some("menu"), Some("sign in")));
        let (_, a) = tool(rules("click the second image", ""));
        assert_eq!((a["action"].as_str(), a["ref"]["n"].as_i64()), (Some("click"), Some(2)));
        let (_, a) = tool(rules("open the first result", ""));
        assert_eq!((a["action"].as_str(), a["ref"]["kind"].as_str()), (Some("open"), Some("result")));
        let (_, a) = tool(rules("view the third image", ""));
        assert_eq!((a["action"].as_str(), a["full"].as_bool()), (Some("open"), Some(true)));
        let (_, a) = tool(rules("save this image", ""));
        assert_eq!(a["action"].as_str(), Some("save"));
        let (_, a) = tool(rules("copy the image address", ""));
        assert_eq!((a["action"].as_str(), a["what"].as_str()), (Some("copy"), Some("image")));
        let (_, a) = tool(rules("copy the link address", ""));
        assert_eq!((a["action"].as_str(), a["what"].as_str()), (Some("copy"), Some("link")));
        assert_eq!(tool(rules("copy the url", "")).0, "browser.pageurl");
        let (t, a) = tool(rules("close the youtube tab", ""));
        assert_eq!((t.as_str(), a["target"]["query"].as_str()), ("browser.tab.close", Some("youtube")));
        let (_, a) = tool(rules("close all youtube tabs", ""));
        assert_eq!(a["target"]["all"], true);
        let (t, a) = tool(rules("switch to the gmail tab", ""));
        assert_eq!((t.as_str(), a["target"]["query"].as_str()), ("browser.tab.activate", Some("gmail")));
        assert_eq!(tool(rules("go to tab 3", "")).1["target"]["index"], 3);
        assert_eq!(tool(rules("go to the last tab", "")).1["target"]["index"], -1);
        assert_eq!(tool(rules("close this tab", "")).1["target"]["active"], true);
        assert_eq!(tool(rules("reopen closed tab", "")).0, "browser.tab.reopen");
        assert_eq!(tool(rules("open browser settings", "")).1["page"], "settings");
        assert_eq!(tool(rules("open brave settings", "")).1["page"], "settings");
        assert_eq!(tool(rules("take me to the browser privacy page", "")).1["page"], "privacy");
        assert_eq!(tool(rules("clear browsing data", "")).1["page"], "clear");
        let (_, a) = tool(rules("change the search engine to duckduckgo", ""));
        assert_eq!((a["page"].as_str(), a["goal"].as_str()), (Some("search"), Some("Duckduckgo")));
        let (t, a) = tool(rules("block pop ups on this site", ""));
        assert_eq!((t.as_str(), a["kind"].as_str(), a["scope"].as_str()), ("browser.permission", Some("popups"), Some("site")));
        assert_eq!(tool(rules("allow notifications on this site", "")).1["setting"], "allow");
        assert_eq!(tool(rules("block notifications in the browser", "")).1["scope"], "all");
        assert!(rules("turn off notifications", "").is_none());
        assert!(rules("block the camera", "").is_none());
        assert_eq!(tool(rules("block pop ups", "")).1["kind"], "popups");
        assert_eq!(tool(rules("number the images", "")).1["kind"], "image");
        assert_eq!(tool(rules("find pricing on this page", "")).1["text"], "pricing");
        assert_eq!(tool(rules("bookmark this page", "")).0, "browser.bookmark");
        assert_eq!(tool(rules("what tabs are open", "")).0, "browser.tabs");
        assert_eq!(tool(rules("set up the browser extension", "")).0, "browser.setup");
        assert_eq!(tool(rules("connect brave", "")).1["browser"], "brave");
        assert!(rules("open youtube", "").is_none());
        assert!(rules("close spotify", "").is_none());
        assert!(rules("open firefox", "").is_none());
        assert!(rules("click sign in", "").is_none());
        assert!(rules("stop the download", "").is_none());
        assert!(rules("block", "").is_none());
    }
}
