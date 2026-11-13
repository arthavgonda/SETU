use crate::core::Core;
use crate::router::Intent;
use crate::text::{normalize, sentence_case};
use crate::tools::input::{self, K};
use crate::tools::{Outcome, code, codespeech, files, git, web, wm};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.to_owned(), args }
}

fn strip<'a>(t: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| t.strip_prefix(p)).map(str::trim).filter(|r| !r.is_empty())
}

fn tail_after<'a>(t: &'a str, seps: &[&str]) -> Option<(&'a str, &'a str)> {
    let mut best: Option<(usize, usize)> = None;
    for s in seps {
        if let Some(i) = t.rfind(s)
            && best.is_none_or(|(b, _)| i > b)
        {
            best = Some((i, s.len()));
        }
    }
    best.map(|(i, n)| (t[..i].trim(), t[i + n..].trim()))
}

pub fn split_browser(t: &str) -> (String, Option<String>) {
    let mut text = t.trim().to_owned();
    let mut browser = None;
    for lead in ["inside ", "in ", "on ", "using ", "with ", "open ", "launch ", "start "] {
        if let Some(rest) = text.strip_prefix(lead) {
            let words: Vec<&str> = rest.split_whitespace().collect();
            for n in (1..=3.min(words.len())).rev() {
                let cand = words[..n].join(" ");
                if web::browser_key(&cand).is_some() {
                    let mut remain = words[n..].join(" ");
                    for joiner in ["and then ", "and ", "then ", ", "] {
                        if let Some(r) = remain.strip_prefix(joiner) {
                            remain = r.to_owned();
                        }
                    }
                    let remain = strip_polite(&remain);
                    if remain.is_empty() {
                        break;
                    }
                    browser = Some(cand);
                    text = remain;
                    break;
                }
            }
            if browser.is_some() {
                break;
            }
        }
    }
    if browser.is_none()
        && let Some((before, after)) = tail_after(&text, &[" in ", " on ", " using ", " with ", " through "])
        && web::browser_key(after).is_some()
    {
        browser = Some(after.to_owned());
        text = before.to_owned();
    }
    (text, browser)
}

fn strip_polite(t: &str) -> String {
    let mut s = t.trim().to_owned();
    loop {
        let before = s.clone();
        for p in ["can you ", "could you ", "please ", "just ", "would you ", "will you ", "for me ", "go ahead and ", "i want to ", "i want you to ", "kindly "] {
            if let Some(r) = s.strip_prefix(p) {
                s = r.trim().to_owned();
            }
        }
        if s == before {
            return s;
        }
    }
}

const FIRST: &[&str] = &["play the first video", "play the first result", "open the first result", "play the first one", "open first result", "play first video", "click the first one", "open the first video", "play the first song"];

const ANY_SONG: &[&str] = &["any song", "a song", "some song", "some songs", "songs", "song", "music", "some music", "any music", "something", "anything", "me a song", "me any song", "me some music", "me something", "me some songs", "me songs"];

fn browser_arg(b: &Option<String>) -> Value {
    b.as_ref().map(|s| json!(s)).unwrap_or(Value::Null)
}

fn site_of(spoken: &str) -> Option<&'static web::Site> {
    web::site(spoken.trim_start_matches("the ").trim_end_matches(" website").trim_end_matches(" app"))
}

fn web_rules(t: &str) -> Option<Intent> {
    if FIRST.contains(&t) {
        return Some(act("ui.first", json!({})));
    }
    let (text, browser) = split_browser(t);
    let text = strip_polite(&text);
    let tx = text.as_str();
    if browser.is_some() && tx.is_empty() {
        return Some(act("web.open", json!({ "browser": browser, "url": "" })));
    }
    for joiner in [" and then ", " and ", " then "] {
        let Some((first, second)) = tx.split_once(joiner) else { continue };
        let opened = strip(first, &["open ", "go to ", "visit ", "launch ", "open up "]).and_then(site_of);
        let Some(site) = opened else { continue };
        let second = strip_polite(second);
        if let Some(rest) = strip(&second, &["play ", "put on ", "start playing "]) {
            let q = if ANY_SONG.contains(&rest) { "" } else { rest.trim_start_matches("me ") };
            return Some(act("web.play", json!({ "query": q, "site": site.name, "browser": browser_arg(&browser) })));
        }
        if let Some(rest) = strip(&second, &["search for ", "search ", "look for ", "find "]) {
            return Some(act("web.search", json!({ "query": rest, "site": site.name, "browser": browser_arg(&browser) })));
        }
        if let Some(rest) = strip(&second, &["watch "]) {
            let tool = if site.name == "YouTube" { "web.play" } else { "web.search" };
            return Some(act(tool, json!({ "query": rest, "site": site.name, "browser": browser_arg(&browser) })));
        }
    }
    let listen = strip(tx, &["i feel like listening to ", "i'd like to listen to ", "i would like to listen to ", "i want to listen to ", "i wanna listen to ", "let me listen to ", "let's listen to ", "listen to ", "i want to hear ", "i'd like to hear ", "let me hear ", "i feel like hearing "]);
    let tx_play = listen.map(|r| format!("play {r}"));
    let tx = tx_play.as_deref().unwrap_or(tx);
    if let Some(rest) = strip(tx, &["play ", "put on ", "start playing "]) {
        let (what, site) = match tail_after(rest, &[" on ", " from "]) {
            Some((w, s)) if site_of(s).is_some() => (w.to_owned(), site_of(s).map(|x| x.name)),
            _ => (rest.to_owned(), None),
        };
        let any = ANY_SONG.contains(&what.as_str());
        let q = if any { String::new() } else { what.trim_start_matches("me ").to_owned() };
        if any || site.is_some() || browser.is_some() || !q.is_empty() && q.split_whitespace().count() <= 12 {
            return Some(act("web.play", json!({ "query": q, "site": site, "browser": browser_arg(&browser) })));
        }
    }
    if let Some(rest) = strip(tx, &["watch "])
        && let Some((w, s)) = tail_after(rest, &[" on ", " in "])
        && let Some(site) = site_of(s)
    {
        if site.name == "YouTube" {
            return Some(act("web.play", json!({ "query": w, "site": "YouTube", "browser": browser_arg(&browser) })));
        }
        return Some(act("web.search", json!({ "query": w, "site": site.name, "browser": browser_arg(&browser) })));
    }
    let search = strip(tx, &["search for ", "search ", "google for ", "google ", "look up ", "find me "]).or_else(|| strip(tx, &["find "]).filter(|r| tail_after(r, &[" on "]).is_some_and(|(_, s)| site_of(s).is_some())));
    if let Some(rest) = search {
        let (q, site) = match tail_after(rest, &[" on ", " in "]) {
            Some((w, s)) if site_of(s).is_some() => (w.to_owned(), site_of(s).map(|x| x.name)),
            _ => (rest.to_owned(), None),
        };
        if !q.is_empty() && !q.starts_with("file") && !q.starts_with("folder") && !q.starts_with("my file") {
            return Some(act("web.search", json!({ "query": q, "site": site, "browser": browser_arg(&browser) })));
        }
    }
    if let Some(rest) = strip(tx, &["log in to ", "login to ", "log into ", "sign in to ", "sign into ", "signin to ", "log me into ", "sign me into ", "log in ", "sign in "]) {
        let r = rest.trim_start_matches("my ").trim_end_matches(" account").trim();
        if matches!(r, "github" | "git hub") {
            return Some(act("git.login", json!({})));
        }
        if matches!(r, "google" | "gmail" | "google account" | "youtube" | "my google") {
            return Some(act("web.open", json!({ "site": "Google account", "browser": browser_arg(&browser) })));
        }
        if let Some(s) = site_of(r) {
            return Some(act("web.open", json!({ "site": s.name, "browser": browser_arg(&browser) })));
        }
    }
    let target = strip(tx, &["go to ", "goto ", "visit ", "navigate to ", "take me to ", "open up ", "open ", "launch ", "show me ", "bring up ", "load "]);
    if let Some(r) = target {
        let r = r.trim_start_matches("the ").trim_start_matches("my ");
        let r_clean = r.trim_end_matches(" website").trim_end_matches(" site").trim_end_matches(" page").trim();
        if matches!(r_clean, "new tab" | "a new tab" | "another tab") {
            return Some(if browser.is_some() { act("web.open", json!({ "browser": browser, "url": "" })) } else { act("key", json!({ "keys": "ctrl t", "label": "New tab" })) });
        }
        let explicit = browser.is_some() || r != r_clean || tx.starts_with("go to ") || tx.starts_with("visit ") || tx.starts_with("navigate to ") || tx.starts_with("take me to ");
        if let Some(url) = web::spoken_url(r_clean) {
            return Some(act("web.open", json!({ "url": url, "browser": browser_arg(&browser) })));
        }
        if let Some(s) = site_of(r_clean)
            && explicit
        {
            return Some(act("web.open", json!({ "site": s.name, "browser": browser_arg(&browser) })));
        }
        if let Some((what, place)) = tail_after(r_clean, &[" on ", " in "])
            && let Some(s) = site_of(place)
        {
            return Some(act("web.search", json!({ "query": what, "site": s.name, "browser": browser_arg(&browser) })));
        }
        if browser.is_some() && !r_clean.is_empty() {
            return Some(act("web.search", json!({ "query": r_clean, "browser": browser })));
        }
    }
    if browser.is_some() && !tx.is_empty() {
        return Some(act("task", json!({ "goal": tx, "app": browser })));
    }
    None
}

const KEY_PHRASES: &[(&[&str], &str, &str)] = &[
    (&["new tab", "open a new tab", "open new tab"], "ctrl t", "New tab"),
    (&["close tab", "close this tab", "close the tab"], "ctrl w", "Closed the tab"),
    (&["reopen tab", "reopen the tab", "reopen closed tab", "bring back the tab"], "ctrl shift t", "Reopened the tab"),
    (&["next tab", "switch tab", "go to next tab"], "ctrl tab", "Next tab"),
    (&["previous tab", "last tab", "go to previous tab"], "ctrl shift tab", "Previous tab"),
    (&["go back", "back", "previous page", "go to previous page"], "alt left", "Went back"),
    (&["go forward", "forward", "next page"], "alt right", "Went forward"),
    (&["refresh", "reload", "refresh the page", "reload the page", "refresh page", "reload page"], "f5", "Refreshed"),
    (&["scroll down", "page down", "scroll down a bit", "go down"], "pagedown", "Scrolled down"),
    (&["scroll up", "page up", "scroll up a bit", "go up"], "pageup", "Scrolled up"),
    (&["scroll to the top", "go to the top", "go to top", "top of the page"], "ctrl home", "At the top"),
    (&["scroll to the bottom", "go to the bottom", "go to bottom", "bottom of the page"], "ctrl end", "At the bottom"),
    (&["zoom in", "make it bigger", "bigger text"], "ctrl equals", "Zoomed in"),
    (&["zoom out", "make it smaller", "smaller text"], "ctrl minus", "Zoomed out"),
    (&["reset zoom", "normal size", "actual size"], "ctrl 0", "Zoom reset"),
    (&["full screen", "fullscreen", "go full screen", "exit full screen", "make it full screen", "make this window full screen", "make the window full screen", "make this full screen", "leave full screen"], "fullscreen", "Full screen"),
    (&["delete the last word", "delete the previous word", "delete that word", "delete last word", "remove the last word", "delete the word before"], "word delete", "Deleted the last word"),
    (&["select the last word", "select the previous word", "select last word", "select the word before"], "word select", "Selected the last word"),
    (&["go to the end of the line", "end of the line", "go to end of line", "end of line", "move to the end of the line", "jump to the end of the line"], "line end", "At the end of the line"),
    (&["go to the start of the line", "go to the beginning of the line", "start of the line", "beginning of the line", "start of line", "move to the start of the line"], "line start", "At the start of the line"),
    (&["new line", "next line", "line break", "add a new line", "add a line break", "start a new line"], "shift enter", "New line"),
    (&["bold", "make it bold", "make this bold", "make that bold", "bold that"], "ctrl b", "Bold"),
    (&["italic", "italics", "make it italic", "make this italic", "make that italic"], "ctrl i", "Italic"),
    (&["underline", "underline it", "underline this", "underline that"], "ctrl u", "Underlined"),
    (&["copy", "copy that", "copy this", "copy it"], "ctrl c", "Copied"),
    (&["paste", "paste it", "paste that", "paste here"], "ctrl v", "Pasted"),
    (&["cut", "cut that", "cut this"], "ctrl x", "Cut"),
    (&["select all", "select everything"], "ctrl a", "Selected all"),
    (&["save", "save it", "save this", "save the file", "save file", "save my work"], "ctrl s", "Saved"),
    (&["print", "print this", "print the page"], "ctrl p", "Print"),
    (&["find on page", "find on this page", "search this page"], "ctrl f", "Find"),
    (&["take a screenshot", "screenshot", "take screenshot", "capture the screen"], "screenshot", "Screenshot"),
    (&["press enter", "hit enter", "enter", "submit", "press return"], "enter", "Pressed Enter"),
    (&["press escape", "escape", "press esc", "dismiss"], "escape", "Pressed Escape"),
    (&["press tab", "next field", "tab"], "tab", "Next field"),
    (&["previous field"], "shift tab", "Previous field"),
    (&["undo typing", "undo the typing", "undo in the app"], "ctrl z", "Undone"),
    (&["redo", "redo that"], "ctrl y", "Redone"),
    (&["open the command palette", "command palette", "show command palette"], "ctrl shift p", "Command palette"),
];

fn key_rules(t: &str) -> Option<Intent> {
    for (phrases, keys, label) in KEY_PHRASES {
        if phrases.contains(&t) {
            return Some(act("key", json!({ "keys": keys, "label": label })));
        }
    }
    if let Some(rest) = strip(t, &["press ", "hit ", "push "]) {
        let rest = rest.trim_end_matches(" key").trim_end_matches(" button");
        if let Some(keys) = input::parse_combo(rest) {
            let mut n = 1;
            if let Some((k, times)) = rest.rsplit_once(" times").map(|(a, _)| a).and_then(|a| a.rsplit_once(' ')) {
                n = crate::text::find_number(times).unwrap_or(1).clamp(1, 30) as usize;
                if let Some(k2) = input::parse_combo(k) {
                    return Some(act("key", json!({ "keys": k, "times": n, "label": input::describe(&k2) })));
                }
            }
            return Some(act("key", json!({ "keys": rest, "times": n, "label": format!("Pressed {}", input::describe(&keys)) })));
        }
    }
    if t == "skip ad" || t == "skip the ad" || t == "skip ads" || t == "skip the ads" {
        return Some(act("ui.click", json!({ "name": "Skip", "roles": ["button"] })));
    }
    None
}

fn ui_rules(t: &str) -> Option<Intent> {
    let short = |x: &str| !x.is_empty() && x.split_whitespace().count() <= 5;
    for (p, on) in [("tick the box for ", true), ("untick the box for ", false), ("tick ", true), ("untick ", false), ("check the box for ", true), ("uncheck the box for ", false)] {
        if let Some(r) = t.strip_prefix(p) {
            let name = r.trim_start_matches("the ").trim_end_matches(" checkbox").trim_end_matches(" box").trim();
            return short(name).then(|| act("ui.tick", json!({ "name": name, "on": on })));
        }
    }
    for (p, on) in [("check the ", true), ("uncheck the ", false)] {
        if let Some(r) = t.strip_prefix(p)
            && let Some(name) = r.strip_suffix(" box").or_else(|| r.strip_suffix(" checkbox"))
        {
            return short(name).then(|| act("ui.tick", json!({ "name": name.trim(), "on": on })));
        }
    }
    for p in ["open the ", "show the ", "open "] {
        if let Some(r) = t.strip_prefix(p)
            && let Some(name) = r.strip_suffix(" menu")
        {
            let name = name.trim();
            if short(name) && !matches!(name, "start" | "context" | "right click" | "the start" | "windows" | "apple") && name.split_whitespace().count() <= 3 {
                return Some(act("ui.click", json!({ "name": name, "roles": ["menuitem", "other"] })));
            }
        }
    }
    for p in ["choose ", "pick ", "select ", "set "] {
        let Some(r) = t.strip_prefix(p) else { continue };
        for sep in [" from the ", " in the ", " from ", " in "] {
            if let Some((option, list)) = r.split_once(sep) {
                let list = list.trim_end_matches(" dropdown").trim_end_matches(" drop down").trim_end_matches(" list").trim_end_matches(" menu").trim_end_matches(" box").trim();
                let option = option.trim_start_matches("the ").trim();
                if (r.ends_with(" dropdown") || r.ends_with(" drop down") || r.ends_with(" list")) && short(option) && short(list) {
                    return Some(act("ui.choose", json!({ "option": option, "list": list })));
                }
            }
        }
        if p == "set "
            && let Some((list, option)) = r.split_once(" dropdown to ")
        {
            let list = list.trim_start_matches("the ").trim();
            return (short(list) && short(option)).then(|| act("ui.choose", json!({ "option": option.trim(), "list": list })));
        }
    }
    if let Some(r) = t.strip_prefix("select ")
        && !r.starts_with("the ")
        && short(r)
        && !matches!(r, "all" | "everything" | "none" | "nothing" | "it" | "this" | "that" | "text" | "the text" | "all the text" | "a word" | "word" | "line" | "this line")
        && !r.contains(" word")
        && !r.contains(" line")
        && crate::text::find_number(r).is_none()
    {
        return Some(act("ui.click", json!({ "name": r, "roles": ["row", "item", "treeitem", "tab", "radio", "menuitem", "cell"] })));
    }
    None
}

fn workspace_rules(t: &str) -> Option<Intent> {
    let n = || crate::text::find_number(t).filter(|n| (1..=16).contains(n));
    let named = t.contains("workspace") || (t.contains("desktop") && crate::text::find_number(t).is_some()) || t.contains("next desktop") || t.contains("previous desktop");
    if !named {
        return None;
    }
    let carry = ["move this window", "move the window", "send this window", "send the window", "put this window", "put the window", "move it to", "send it to"].iter().any(|p| t.starts_with(p));
    let dir = if t.contains("next") || t.contains("right") { 1 } else if t.contains("previous") || t.contains("left") || t.contains("last") { -1 } else { 0 };
    let ask = carry || ["switch to", "go to", "show", "open", "take me to", "jump to", "next", "previous", "workspace"].iter().any(|p| t.starts_with(p));
    if !ask {
        return None;
    }
    match (n(), dir) {
        (Some(n), _) => Some(act("wm.workspace", json!({ "n": n, "move": carry }))),
        (None, d) if d != 0 => Some(act("wm.workspace", json!({ "dir": d, "move": carry }))),
        _ => None,
    }
}

fn in_setu(rest: &str) -> (&str, bool) {
    for tail in [" in setu's window", " in the setu window", " in setu window", " in setu", " on setu", " of setu"] {
        if let Some(r) = rest.strip_suffix(tail) {
            return (r.trim(), true);
        }
    }
    (rest, false)
}

fn click_rules(t: &str, raw: &str) -> Option<Intent> {
    if let Some(rest) = strip(t, &["click on ", "click ", "tap on ", "tap ", "press the ", "select the ", "choose the ", "hit the "]) {
        let (rest, setu) = in_setu(rest);
        let name = rest.trim_start_matches("the ").trim_end_matches(" button").trim_end_matches(" link").trim_end_matches(" tab").trim_end_matches(" option").trim();
        let words: Vec<&str> = name.split_whitespace().collect();
        if crate::text::find_number(name).is_some() && words.len() <= 2 || name.is_empty() || words.len() > 8 {
            return None;
        }
        let first = matches!(name, "first result" | "first video" | "first one" | "first link" | "top result" | "the first result" | "first song" | "first movie" | "first item");
        if first {
            return Some(act("ui.first", json!({})));
        }
        if setu {
            return Some(act("ui.click", json!({ "name": name, "app": "setu" })));
        }
        return Some(act("ui.click", json!({ "name": name })));
    }
    for p in ["type ", "enter ", "write "] {
        if let Some(rest) = t.strip_prefix(p)
            && let Some((text, field)) = tail_after(rest, &[" in the ", " into the ", " in ", " into "])
            && field.split_whitespace().count() <= 4
            && (field.ends_with("field") || field.ends_with("box") || field.ends_with("bar") || matches!(field, "search" | "email" | "password" | "username" | "name"))
        {
            let raw_text = raw_tail_text(raw, text).unwrap_or_else(|| text.to_owned());
            let value = codespeech::spoken_email(&raw_text).unwrap_or(raw_text);
            return Some(act("ui.fill", json!({ "field": field.trim_end_matches(" field").trim_end_matches(" box"), "text": value })));
        }
    }
    if matches!(t, "type my password" | "enter my password" | "type the password" | "enter the password" | "type password" | "enter password" | "fill in my password") {
        return Some(act("ui.secret", json!({})));
    }
    if let Some(rest) = strip(t, &["type my email ", "type the email ", "enter my email ", "type email "]) {
        let v = codespeech::spoken_email(rest).unwrap_or_else(|| rest.replace(' ', ""));
        return Some(act("ui.type", json!({ "text": v })));
    }
    None
}

fn raw_tail_text(raw: &str, normalized: &str) -> Option<String> {
    let first = normalized.split_whitespace().next()?;
    let lower = raw.to_lowercase();
    let i = lower.find(first)?;
    let words = normalized.split_whitespace().count();
    let out: Vec<&str> = raw[i..].split_whitespace().take(words).collect();
    Some(out.join(" ").trim_end_matches(['.', '?', '!']).to_owned())
}

fn file_rules(t: &str) -> Option<Intent> {
    if matches!(t, "show my drives" | "show drives" | "list drives" | "what drives do i have" | "which drives do i have" | "show all drives" | "list my drives" | "open this pc" | "open my computer" | "my computer" | "this pc") {
        return Some(act("files.open", json!({ "where": "computer" })));
    }
    if t.contains("space") && (t.contains("how much") || t.contains("free") || t.contains("left") || t.contains("disk") || t.contains("storage")) && !t.contains("spacebar") {
        let place = tail_after(t, &[" on ", " in "]).map(|(_, p)| p.to_owned());
        return Some(act("files.space", json!({ "where": place })));
    }
    if matches!(t, "open my last download" | "open the last download" | "open last download" | "open the latest download" | "open latest download" | "open what i downloaded" | "open the file i just downloaded" | "open my latest download") {
        return Some(act("files.latest", json!({ "where": "downloads" })));
    }
    if let Some(rest) = strip(t, &["open ", "show ", "show me ", "go to ", "take me to ", "browse "]) {
        let r = rest.trim_start_matches("the ").trim_start_matches("my ");
        let r = r.trim_end_matches(" in file explorer").trim_end_matches(" in files").trim();
        let plain_folder = matches!(r.trim_end_matches(" folder"), "downloads" | "download" | "documents" | "desktop") || r.ends_with(" folder") && files::known_folder(r).is_some();
        let is_drive = plain_folder
            || ["flash drive", "pen drive", "pendrive", "usb", "external drive", "sd card", "memory card", "thumb drive", "hard disk"].iter().any(|w| r.contains(w))
            || (r.ends_with(" drive") || r.starts_with("drive ")) && r.split_whitespace().count() <= 3 && !r.contains("google");
        if is_drive {
            return Some(act("files.open", json!({ "where": r })));
        }
        for (suffix, kind) in [(" folder", "folder"), (" file", "file"), (" document", "file"), (" pdf", "file"), (" photo", "file"), (" picture", "file"), (" video", "file"), (" spreadsheet", "file"), (" presentation", "file")] {
            if let Some(name) = r.strip_suffix(suffix)
                && !name.is_empty()
                && !name.ends_with(" vs code")
            {
                let name = name.trim_start_matches("the ").trim_start_matches("my ");
                let (name, place) = match tail_after(name, &[" in ", " on ", " from "]) {
                    Some((n, p)) if files::location(p).is_some() => (n, Some(p)),
                    _ => (name, None),
                };
                let ext = match suffix {
                    " pdf" => Some("pdf"),
                    _ => None,
                };
                return Some(act(if kind == "folder" { "files.openfolder" } else { "files.openfile" }, json!({ "name": name, "where": place, "ext": ext })));
            }
        }
        if let Some(name) = r.strip_prefix("folder ").or_else(|| r.strip_prefix("file ")) {
            let kind = if r.starts_with("folder") { "files.openfolder" } else { "files.openfile" };
            return Some(act(kind, json!({ "name": name })));
        }
    }
    if let Some(rest) = strip(t, &["find the file ", "find file ", "find my file ", "find a file ", "search for the file ", "search for file ", "search for a file ", "look for the file ", "look for ", "find my ", "where is my ", "where's my ", "where is the file ", "locate "]) {
        let name = rest.trim_start_matches("the ").trim_end_matches(" file").trim_end_matches(" folder").trim();
        if !name.is_empty() {
            return Some(act("files.find", json!({ "name": name })));
        }
    }
    if let Some(rest) = strip(t, &["create a new folder ", "create a folder ", "create folder ", "make a new folder ", "make a folder ", "make folder ", "new folder "]) {
        let rest = rest.trim_start_matches("called ").trim_start_matches("named ");
        let (name, place) = match tail_after(rest, &[" on the ", " on my ", " in my ", " in the ", " on ", " in "]) {
            Some((n, p)) if files::location(p).is_some() || p == "desktop" => (n.trim_start_matches("called ").trim_start_matches("named ").to_owned(), Some(p.to_owned())),
            _ => (rest.to_owned(), None),
        };
        return Some(act("files.mkdir", json!({ "name": name, "where": place })));
    }
    if let Some(rest) = strip(t, &["delete the file ", "delete file ", "delete my file ", "delete the folder ", "delete folder ", "remove the file ", "remove file ", "trash ", "move to trash ", "move to recycle bin "]) {
        return Some(act("files.delete", json!({ "name": rest.trim_start_matches("the ") })));
    }
    for (verb, tool) in [("copy ", "files.copy"), ("move ", "files.move")] {
        if let Some(rest) = t.strip_prefix(verb)
            && let Some((name, dest)) = tail_after(rest, &[" to the ", " into the ", " to my ", " into my ", " to ", " into "])
            && files::location(dest).is_some()
        {
            let name = name.trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" file").trim_end_matches(" folder");
            return Some(act(tool, json!({ "name": name, "to": dest })));
        }
    }
    None
}

fn git_rules(t: &str) -> Option<Intent> {
    let t = t.trim_start_matches("git ").trim();
    let in_git = |s: &str| s.trim_end_matches(" to github").trim_end_matches(" on github").trim_end_matches(" to git").trim_end_matches(" in git").trim().to_owned();
    let t2 = in_git(t);
    let t = t2.as_str();
    if matches!(t, "status" | "what changed" | "what has changed" | "what's changed" | "show git status" | "what did i change" | "show changes" | "any changes" | "what changed in my code" | "check git status") {
        return Some(act("git.status", json!({})));
    }
    if let Some(rest) = strip(t, &["commit my changes with message ", "commit my changes with the message ", "commit changes with message ", "commit with message ", "commit with the message ", "commit all changes with message ", "commit everything with message ", "commit the changes with message ", "commit and say ", "commit message "]) {
        return Some(act("git.commit", json!({ "message": rest })));
    }
    if matches!(t, "commit" | "commit my changes" | "commit changes" | "commit everything" | "commit all changes" | "commit the changes" | "commit my code" | "commit all" | "commit it" | "commit this" | "save a commit") {
        return Some(act("git.commit", json!({ "message": "" })));
    }
    if let Some(rest) = t.strip_prefix("commit ")
        && !rest.starts_with("my ")
    {
        return Some(act("git.commit", json!({ "message": rest.trim_start_matches("saying ").trim_start_matches("as ") })));
    }
    if let Some(rest) = strip(t, &["commit and push with message ", "commit and push "]) {
        return Some(act("git.commit_push", json!({ "message": rest })));
    }
    if matches!(t, "commit and push" | "commit and push my changes" | "commit and push everything") {
        return Some(act("git.commit_push", json!({ "message": "" })));
    }
    if matches!(t, "push" | "push my changes" | "push my code" | "push the code" | "push changes" | "push my commits" | "push it" | "push the changes" | "push everything" | "upload my code") {
        return Some(act("git.push", json!({})));
    }
    if matches!(t, "pull" | "pull changes" | "pull the latest changes" | "pull latest" | "pull the latest" | "get the latest code" | "pull my code" | "sync my code" | "sync") {
        return Some(act("git.pull", json!({})));
    }
    if let Some(rest) = strip(t, &["create a new branch called ", "create a branch called ", "create branch ", "create a new branch ", "create a branch ", "new branch called ", "new branch ", "make a branch called ", "make a new branch called "]) {
        return Some(act("git.branch", json!({ "name": rest, "create": true })));
    }
    if let Some(rest) = strip(t, &["switch to branch ", "switch branch to ", "checkout branch ", "check out branch ", "checkout ", "go to branch ", "switch to the branch "]) {
        return Some(act("git.branch", json!({ "name": rest.trim_start_matches("the "), "create": false })));
    }
    if matches!(t, "what was my last commit" | "last commit" | "show last commit" | "what's my last commit" | "show the last commit") {
        return Some(act("git.last", json!({})));
    }
    if matches!(t, "init" | "initialize git" | "initialise git" | "initialize a repository" | "start a git repository" | "make this a git repository" | "start git here") {
        return Some(act("git.init", json!({})));
    }
    if matches!(t, "connect github" | "connect to github" | "github login" | "log in github" | "sign in github" | "set up github" | "setup github") {
        return Some(act("git.login", json!({})));
    }
    None
}

const WIN_OBJ: &[&str] = &["this window", "the window", "the current window", "current window", "window", "this app", "the app", "this", "it", "the active window", "my window"];
const WIN_VERBS: &[&str] = &["snap", "move", "put", "tile", "dock", "place", "push", "send", "throw", "shift"];

fn side_word(s: &str) -> Option<&'static str> {
    let s = s.trim();
    let s = s.strip_prefix("to ").or_else(|| s.strip_prefix("onto ")).or_else(|| s.strip_prefix("on ")).unwrap_or(s);
    let s = s.strip_prefix("the ").unwrap_or(s);
    let s = s.trim_end_matches(" of the screen").trim_end_matches(" of my screen").trim_end_matches(" of screen").trim_end_matches(" side").trim_end_matches(" half").trim_end_matches(" part").trim();
    Some(match s {
        "left" => "left",
        "right" => "right",
        "center" | "centre" | "middle" => "center",
        "other screen" | "next screen" | "other monitor" | "next monitor" | "other display" | "next display" | "second screen" | "second monitor" | "second display" => "next_screen",
        _ => return None,
    })
}

fn corner_word(s: &str) -> Option<&'static str> {
    let s = s.trim();
    let s = s.strip_prefix("to ").or_else(|| s.strip_prefix("into ")).or_else(|| s.strip_prefix("in ")).or_else(|| s.strip_prefix("at ")).unwrap_or(s);
    let s = s.strip_prefix("the ").unwrap_or(s);
    let s = s.trim_end_matches(" of the screen").trim_end_matches(" of my screen").trim_end_matches(" corner").trim_end_matches(" edge").trim();
    Some(match s {
        "top right" | "upper right" | "right top" => "top-right",
        "top left" | "upper left" | "left top" => "top-left",
        "bottom right" | "lower right" | "right bottom" => "bottom-right",
        "bottom left" | "lower left" | "left bottom" => "bottom-left",
        "top" => "top",
        "bottom" => "bottom",
        _ => return None,
    })
}

fn frame_rules(t: &str) -> Option<Intent> {
    let frame = |args: Value| Some(act("wm.frame", args));
    let object = |rest: &'_ str| -> Option<String> {
        let mut objs: Vec<&&str> = WIN_OBJ.iter().collect();
        objs.sort_by_key(|o| std::cmp::Reverse(o.len()));
        objs.into_iter().find_map(|o| rest.strip_prefix(*o).map(|r| r.trim().to_owned()))
    };
    for v in ["move ", "put ", "place ", "send ", "push ", "snap "] {
        let Some(rest) = t.strip_prefix(v) else { continue };
        let Some(tail) = object(rest) else { continue };
        if let Some(c) = corner_word(&tail) {
            return frame(json!({ "op": "corner", "where": c }));
        }
        let tail = tail.trim_end_matches(" a bit").trim_end_matches(" a little").trim_end_matches(" slightly").trim();
        let tail = tail.strip_prefix("a bit ").or_else(|| tail.strip_prefix("a little ")).or_else(|| tail.strip_prefix("slightly ")).unwrap_or(tail);
        let tail = tail.strip_prefix("to the ").unwrap_or(tail);
        let bit = t.contains(" a bit") || t.contains(" a little") || t.contains(" slightly");
        match tail {
            "up" | "higher" => return frame(json!({ "op": "nudge", "dir": "up" })),
            "down" | "lower" => return frame(json!({ "op": "nudge", "dir": "down" })),
            "left" if bit => return frame(json!({ "op": "nudge", "dir": "left" })),
            "right" if bit => return frame(json!({ "op": "nudge", "dir": "right" })),
            _ => {}
        }
    }
    for v in ["make ", "resize ", "set "] {
        let Some(rest) = t.strip_prefix(v) else { continue };
        let Some(tail) = object(rest) else { continue };
        let tail = tail.trim_end_matches(" a bit").trim_end_matches(" a little").trim();
        let scale = match tail {
            "half as wide" | "half the width" => Some((0.5, 1.0)),
            "half as tall" | "half as high" | "half the height" => Some((1.0, 0.5)),
            "twice as wide" | "double the width" => Some((2.0, 1.0)),
            "twice as tall" | "twice as high" | "double the height" => Some((1.0, 2.0)),
            "wider" => Some((1.25, 1.0)),
            "narrower" | "thinner" => Some((0.8, 1.0)),
            "taller" | "higher" => Some((1.0, 1.25)),
            "shorter" => Some((1.0, 0.8)),
            "half the size" | "half as big" => Some((0.7, 0.7)),
            _ => None,
        };
        if let Some((w, h)) = scale {
            return frame(json!({ "op": "scale", "w": w, "h": h }));
        }
        let dims = tail.strip_prefix("to ").unwrap_or(tail).trim_end_matches(" pixels");
        if let Some((a, b)) = dims.split_once(" by ").or_else(|| dims.split_once(" x "))
            && let (Some(w), Some(h)) = (crate::text::find_number(a), crate::text::find_number(b))
            && (200..=6000).contains(&w)
            && (200..=6000).contains(&h)
        {
            return frame(json!({ "op": "size", "w": w, "h": h }));
        }
    }
    None
}

fn app_name(s: &str) -> Option<String> {
    let s = s.trim().trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" window").trim_end_matches(" app").trim();
    (!s.is_empty() && s.split_whitespace().count() <= 4 && !WIN_OBJ.contains(&s) && !s.contains(" tab")).then(|| s.to_owned())
}

fn window_rules(t: &str) -> Option<Intent> {
    let t = t.trim_end_matches(" please").trim_end_matches(" for me").trim();
    if let Some(i) = workspace_rules(t) {
        return Some(i);
    }
    let place = |w: &str| act("wm.place", json!({ "where": w }));
    if let Some(i) = frame_rules(t) {
        return Some(i);
    }
    if matches!(t, "show desktop" | "show the desktop" | "show my desktop" | "go to desktop" | "go to the desktop" | "minimize everything" | "minimise everything" | "minimize all" | "minimise all" | "minimize all windows" | "minimise all windows" | "hide all windows" | "hide everything" | "take me to the desktop") {
        return Some(act("wm.desktop", json!({})));
    }
    if matches!(t, "what windows are open" | "which windows are open" | "list my windows" | "list the open windows" | "list open windows" | "what apps are open" | "which apps are open" | "what is open" | "what's open" | "what do i have open" | "show me what's open") {
        return Some(act("wm.list", json!({})));
    }
    if matches!(t, "switch window" | "switch windows" | "next window" | "other window" | "the other window" | "switch app" | "switch apps" | "previous window" | "last window" | "alt tab" | "switch to the other window" | "switch to the last window" | "switch to the previous window" | "go back to the last window" | "go to the other window" | "switch to the next window" | "go back to the previous window") {
        return Some(act("wm.switch", json!({})));
    }
    let groups: &[(&[&str], &str)] = &[
        (&["maximize", "maximise", "maximize this", "maximise this", "maximize it", "maximise it", "maximize the window", "maximise the window", "maximize this window", "maximise this window", "make the window bigger", "make this window bigger", "make the window full size", "make this window full size", "enlarge the window"], "max"),
        (&["minimize", "minimise", "minimize this", "minimise this", "minimize it", "minimise it", "minimize the window", "minimise the window", "minimize this window", "minimise this window", "hide this window", "hide the window"], "min"),
        (&["restore", "restore the window", "restore this window", "restore it", "restore down", "unmaximize", "unmaximise", "un maximize", "unmaximize the window", "make the window smaller", "make this window smaller"], "restore"),
        (&["center the window", "centre the window", "center this window", "centre this window", "center it", "centre it", "put it in the middle", "move it to the center", "move the window to the center", "put the window in the middle"], "center"),
    ];
    for (words, w) in groups {
        if words.contains(&t) {
            return Some(place(w));
        }
    }
    if let Some(x) = t.strip_prefix("window ").and_then(side_word) {
        return Some(place(x));
    }
    if matches!(t, "left half" | "right half") {
        return Some(place(if t.starts_with("left") { "left" } else { "right" }));
    }
    if matches!(t, "arrange my windows" | "arrange windows" | "arrange the windows" | "tile my windows" | "tile windows" | "tile the windows" | "organize my windows" | "organise my windows" | "put my windows side by side" | "put the windows side by side" | "windows side by side" | "split screen" | "split the screen" | "put the last two windows side by side") {
        return Some(act("wm.side", json!({})));
    }
    let body = strip(t, &["put ", "place ", "arrange ", "show ", "open ", "snap ", "tile ", "keep ", "set ", "move ", "show me "]).unwrap_or(t);
    if let Some(x) = body.strip_suffix(" side by side").or_else(|| body.strip_suffix(" next to each other")).or_else(|| body.strip_suffix(" in split screen"))
        && let Some((a, b)) = x.split_once(" and ")
        && let (Some(a), Some(b)) = (app_name(a), app_name(b))
    {
        return Some(act("wm.side", json!({ "left": a, "right": b })));
    }
    if let Some(x) = strip(t, &["split the screen between ", "split screen between ", "split the screen with ", "split screen with ", "split screen ", "split the screen "])
        && let Some((a, b)) = x.split_once(" and ")
        && let (Some(a), Some(b)) = (app_name(a), app_name(b))
    {
        return Some(act("wm.side", json!({ "left": a, "right": b })));
    }
    if let Some((l, r)) = body.split_once(" and ") {
        let left = l.strip_suffix(" on the left").or_else(|| l.strip_suffix(" to the left")).or_else(|| l.strip_suffix(" left"));
        let right = r.strip_suffix(" on the right").or_else(|| r.strip_suffix(" to the right")).or_else(|| r.strip_suffix(" right"));
        if let (Some(a), Some(b)) = (left.and_then(app_name), right.and_then(app_name)) {
            return Some(act("wm.side", json!({ "left": a, "right": b })));
        }
        let right2 = l.strip_suffix(" on the right").or_else(|| l.strip_suffix(" to the right"));
        let left2 = r.strip_suffix(" on the left").or_else(|| r.strip_suffix(" to the left"));
        if let (Some(b), Some(a)) = (right2.and_then(app_name), left2.and_then(app_name)) {
            return Some(act("wm.side", json!({ "left": a, "right": b })));
        }
    }
    for v in WIN_VERBS {
        let Some(rest) = t.strip_prefix(v).and_then(|r| r.strip_prefix(' ')) else { continue };
        if let Some(w) = side_word(rest) {
            return Some(place(w));
        }
        for o in WIN_OBJ {
            if let Some(w) = rest.strip_prefix(o).and_then(|r| r.strip_prefix(' ')).and_then(side_word) {
                return Some(place(w));
            }
        }
        for sep in [" to the ", " on the ", " onto the ", " to ", " on "] {
            if let Some((app, side)) = rest.split_once(sep)
                && let Some(w) = side_word(&format!("{} {side}", sep.trim()))
                && let Some(a) = app_name(app)
            {
                return Some(act("wm.place", json!({ "where": w, "app": a })));
            }
        }
    }
    for p in ["switch over to ", "switch to ", "bring up the ", "focus on ", "focus ", "go to the ", "show me the ", "jump to "] {
        let Some(x) = t.strip_prefix(p) else { continue };
        let window = x.ends_with(" window") || x.ends_with(" app");
        if !window && matches!(p, "go to the " | "show me the " | "jump to " | "bring up the ") {
            continue;
        }
        let bad = ["tab", "line ", "branch", "desktop", "sleep", "next", "previous", "mode", "full screen", "page", "top", "bottom", "settings", "setting", "folder", "drive", "file", "website", "site", "link", "song", "video", "track"];
        if bad.iter().any(|b| x.contains(b)) && !window {
            continue;
        }
        if let Some(a) = app_name(x) {
            return Some(act("wm.focus", json!({ "app": a })));
        }
    }
    if let Some(x) = strip(t, &["bring "]).and_then(|r| r.strip_suffix(" to the front").or_else(|| r.strip_suffix(" forward")).or_else(|| r.strip_suffix(" to front")))
        && let Some(a) = app_name(x)
    {
        return Some(act("wm.focus", json!({ "app": a })));
    }
    None
}

fn project_rule(t: &str) -> Option<Intent> {
    let tx = t.replace(" in vs code", "").replace(" in vscode", "").replace(" in visual studio code", "").replace(" for vs code", "").replace(" using vs code", "");
    let rest = strip(&tx, &["create a new ", "create new ", "create a ", "make a new ", "make me a new ", "make a ", "start a new ", "start new ", "open a new ", "open new ", "begin a new ", "set up a new ", "setup a new ", "set up new ", "new "])?;
    let (before, after) = rest.split_once("project").or_else(|| rest.split_once("workspace"))?;
    let lang = before.trim().trim_start_matches("a ").trim_start_matches("an ").trim();
    if lang.split_whitespace().count() > 3 || ["folder", "file", "tab", "window", "terminal"].iter().any(|w| lang.contains(w)) {
        return None;
    }
    let mut name = after.trim().trim_start_matches('s').trim().to_owned();
    for p in ["called ", "named ", "with the name ", "by the name ", "for "] {
        if let Some(r) = name.strip_prefix(p) {
            name = r.trim().to_owned();
        }
    }
    let (name, lang2) = match name.rsplit_once(" in ") {
        Some((n, l)) if code::starter(l).is_some() => (n.trim().to_owned(), l.trim().to_owned()),
        _ => (name, String::new()),
    };
    let language = if lang.is_empty() { lang2 } else { lang.to_owned() };
    Some(act("code.project", json!({ "name": name, "language": language })))
}

pub fn language_in(raw: &str) -> String {
    let r = format!(" {} ", raw.to_lowercase().replace(['.', ',', '!', '?', '(', ')'], " "));
    let table: &[(&str, &str)] = &[
        (" c++ ", "c++"), (" cpp ", "c++"), (" c plus plus ", "c++"), (" python ", "python"), (" javascript ", "javascript"), (" java script ", "javascript"), (" node ", "javascript"),
        (" typescript ", "typescript"), (" java ", "java"), (" golang ", "go"), (" go lang ", "go"), (" rust ", "rust"), (" c# ", "c#"), (" c sharp ", "c#"), (" csharp ", "c#"),
        (" html ", "html"), (" kotlin ", "kotlin"), (" swift ", "swift"), (" ruby ", "ruby"), (" php ", "php"), (" bash ", "bash"), (" c language ", "c"), (" in c ", "c"), (" with c ", "c"),
    ];
    table.iter().find(|(k, _)| r.contains(k)).map(|(_, v)| (*v).to_owned()).unwrap_or_default()
}

fn wants_to_code(t: &str) -> bool {
    let p = crate::plan::clean(t);
    [
        "i want to do coding", "i want to do some coding", "i want to code", "i wanna code", "i want to program", "i want to write code", "i want to write some code",
        "want to do coding", "want to code", "let's code", "lets code", "let's do some coding", "time to code", "open my code editor", "open the code editor", "i need to code",
    ]
    .iter()
    .any(|x| p.starts_with(x))
}

fn ext_words(name: &str) -> String {
    let exts = ["cpp", "c", "h", "hpp", "py", "js", "ts", "jsx", "tsx", "java", "kt", "go", "rs", "rb", "php", "cs", "swift", "html", "css", "json", "md", "txt", "sh", "yaml", "yml", "xml", "sql"];
    let mut n = name.trim().trim_matches(['.', ',', '"', '\'']).replace(" dot ", ".").replace(" point ", ".");
    if !n.contains('.') {
        let words: Vec<&str> = n.split_whitespace().collect();
        if words.len() >= 2 && exts.contains(&words[words.len() - 1].to_lowercase().as_str()) {
            n = format!("{}.{}", words[..words.len() - 1].join("_"), words[words.len() - 1].to_lowercase());
        }
    }
    let n = n.replace(' ', "_");
    n.chars().filter(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')).collect()
}

fn file_rule(t: &str, raw: &str) -> Option<Intent> {
    let p = crate::plan::clean(&t.replace(" in vs code", "").replace(" in vscode", "").replace(" in visual studio code", ""));
    let create = ["create a new file", "create new file", "create a file", "create file", "make a new file", "make a file", "add a new file", "add a file", "new file", "open a new file", "create another file", "make another file"];
    if !create.iter().any(|c| p.starts_with(c)) {
        return None;
    }
    let low = raw.to_lowercase();
    let after = |marks: &[&str]| marks.iter().find_map(|m| low.find(m).map(|i| (i, i + m.len())));
    let name_at = after(&[" call it as ", " call it ", " called ", " named ", " with the name ", " name it ", " name it as "]);
    let folder_at = after(&[" inside the folder ", " inside the project ", " inside ", " in the folder ", " in the project ", " in folder ", " under "]);
    let cut = |from: usize, stops: &[usize]| -> String {
        let end = stops.iter().copied().filter(|e| *e > from).min().unwrap_or(raw.len()).min(raw.len());
        raw.get(from.min(end)..end).unwrap_or("").trim().trim_matches([',', '.', '"', '\'']).trim().to_owned()
    };
    let name = match name_at {
        Some((_, start)) => cut(start, &[folder_at.map(|f| f.0).unwrap_or(usize::MAX)]),
        None => {
            let start = ["file "].iter().find_map(|m| low.find(m).map(|i| i + m.len())).unwrap_or(raw.len());
            cut(start, &[folder_at.map(|f| f.0).unwrap_or(usize::MAX)])
        }
    };
    let folder = folder_at.map(|(_, start)| cut(start, &[name_at.map(|n| n.0).unwrap_or(usize::MAX), raw.find(',').filter(|c| *c > start).unwrap_or(usize::MAX)])).unwrap_or_default();
    Some(act("code.new_file", json!({ "name": ext_words(&name), "folder": folder.trim_start_matches("the ").to_owned() })))
}

fn folder_project_rule(t: &str, raw: &str) -> Option<Intent> {
    if !(mentions_vscode(t) || !language_in(raw).is_empty()) {
        return None;
    }
    let p = crate::plan::clean(&t.replace("in vs code ", "").replace("in vscode ", "").replace("in visual studio code ", ""));
    let rest = strip(&p, &["create a new folder ", "create new folder ", "create a folder ", "make a new folder ", "make a folder ", "create a new directory ", "create a directory "])?;
    let mut name = rest.to_owned();
    for m in ["called ", "named ", "with the name "] {
        if let Some(i) = name.find(m) {
            name = name[i + m.len()..].to_owned();
        }
    }
    for stop in [" as i ", " because ", " for ", " to ", " in ", " so "] {
        if let Some(i) = name.find(stop) {
            name.truncate(i);
        }
    }
    Some(act("code.project", json!({ "name": name.trim(), "language": language_in(raw) })))
}

fn file_name_in(raw: &str, name: &str) -> String {
    let want = normalize(name);
    for tok in raw.split_whitespace() {
        let tok = tok.trim_matches(|c: char| matches!(c, ',' | '?' | '!' | '"' | '\'' | '(' | ')')).trim_end_matches('.');
        if tok.contains('.') && !tok.starts_with('.') && normalize(tok) == want {
            return tok.to_owned();
        }
    }
    if let Some((stem, ext)) = name.rsplit_once(" dot ")
        && !ext.contains(' ')
        && !stem.trim().is_empty()
    {
        return format!("{}.{ext}", stem.trim().replace(" dot ", "."));
    }
    name.to_owned()
}

fn mentions_vscode(t: &str) -> bool {
    t.contains("vs code") || t.contains("vscode") || t.contains("visual studio code") || t.contains("v s code") || t.contains("the editor") || t.contains("code editor")
}

const BUILD: &[&str] = &["build the project", "build my project", "build this project", "build the code", "build my code", "build it", "build the solution", "build my solution", "build the app", "build my app", "compile the project", "compile my project", "compile the code", "compile my code", "make a build", "do a build"];

fn ide_open_rule(t: &str) -> Option<Intent> {
    let rest = strip(t, &["open ", "open up ", "show "])?;
    let (what, place) = tail_after(rest, &[" in ", " with ", " using "])?;
    let ide = crate::ide::spoken(place)?;
    if crate::ide::spoken(what).is_some() || ide.key == "vscode" {
        return None;
    }
    let r = what.trim_start_matches("the ").trim_start_matches("my ").trim_start_matches("this ").trim();
    if r.is_empty() {
        return None;
    }
    let folder = r.ends_with(" folder") || r.ends_with(" project") || r == "project";
    let name = r.trim_end_matches(" folder").trim_end_matches(" project").trim_end_matches(" file").trim();
    Some(act("code.open", json!({ "name": if name.is_empty() { "project" } else { name }, "folder": folder, "app": ide.key })))
}

fn code_rules(t: &str, raw: &str) -> Option<Intent> {
    if codespeech::starts_dictation(t) {
        return Some(act("code.dictate", json!({})));
    }
    if BUILD.contains(&t) {
        return Some(act("code.build", json!({})));
    }
    if let Some(mut i) = ide_open_rule(t) {
        if let Intent::Act { args, .. } = &mut i
            && args["folder"] == json!(false)
            && let Some(n) = args["name"].as_str().map(str::to_owned)
        {
            args["name"] = json!(file_name_in(raw, &n));
        }
        return Some(i);
    }
    if wants_to_code(t) {
        return Some(act("code.start", json!({})));
    }
    if let Some(i) = file_rule(t, raw) {
        return Some(i);
    }
    if let Some(i) = folder_project_rule(t, raw) {
        return Some(i);
    }
    if let Some(i) = project_rule(t) {
        let lang = language_in(raw);
        return Some(match i {
            Intent::Act { tool, mut args } if !lang.is_empty() && args["language"].as_str().is_none_or(str::is_empty) => {
                args["language"] = json!(lang);
                Intent::Act { tool, args }
            }
            other => other,
        });
    }
    if codespeech::write_request(t) {
        return Some(act("code.write", json!({ "request": t })));
    }
    if let Some(c) = codespeech::comment_request(t) {
        return Some(act("code.comment", json!({ "text": c })));
    }
    if let Some(c) = codespeech::command(t) {
        match c {
            codespeech::Cmd::Check => return Some(act("code.check", json!({}))),
            codespeech::Cmd::Run if t != "run" && t != "execute" => return Some(act("code.run", json!({}))),
            codespeech::Cmd::NextError if t.contains("error") || t.contains("problem") => return Some(act("code.check", json!({ "next": true }))),
            codespeech::Cmd::GoToLine(n) if t.starts_with("go to line") || t.starts_with("jump to line") || t.starts_with("goto line") => return Some(act("code.goto", json!({ "line": n }))),
            _ => {}
        }
    }
    let tx = t.trim_end_matches(" in vs code").trim_end_matches(" in vscode").trim_end_matches(" in visual studio code").trim_end_matches(" in the editor").trim_end_matches(" for vs code").trim_end_matches(" to vs code").trim();
    if let Some(rest) = strip(tx, &["install the ", "install ", "add the ", "add ", "get the ", "download the "])
        && (rest.ends_with(" extension") || rest.ends_with(" plugin") || rest.ends_with(" extensions"))
    {
        return Some(act("code.extension", json!({ "name": rest.trim_end_matches(" extensions").trim_end_matches(" extension").trim_end_matches(" plugin") })));
    }
    if let Some(rest) = strip(tx, &["uninstall the ", "uninstall ", "remove the ", "remove ", "disable the "])
        && (rest.ends_with(" extension") || rest.ends_with(" plugin"))
    {
        return Some(act("code.uninstall", json!({ "name": rest.trim_end_matches(" extension").trim_end_matches(" plugin") })));
    }
    if mentions_vscode(t) && matches!(tx, "open settings" | "open the settings" | "settings" | "show settings" | "open preferences") {
        return Some(act("code.command", json!({ "id": "workbench.action.openSettings", "label": "Opened VS Code settings" })));
    }
    if let Some(rest) = strip(tx, &["search settings for ", "search the settings for ", "find the setting ", "find setting ", "open settings for ", "show settings for "]) {
        return Some(act("code.settings", json!({ "query": rest })));
    }
    if let Some((key, value, label)) = code::setting_for(tx) {
        let specific = !matches!(key.as_str(), "workbench.colorTheme" | "editor.fontSize");
        if mentions_vscode(t) || specific {
            return Some(act("code.setting", json!({ "key": key, "value": value, "label": label })));
        }
    }
    let editor_cmds: &[(&[&str], &str, &str)] = &[
        (&["format document", "format the document", "format my code", "format the code", "format this file"], "editor.action.formatDocument", "Formatted the document"),
        (&["toggle sidebar", "hide sidebar", "show sidebar", "hide the sidebar", "show the sidebar"], "workbench.action.toggleSidebarVisibility", "Toggled the sidebar"),
        (&["split editor", "split the editor", "split screen"], "workbench.action.splitEditor", "Split the editor"),
        (&["close editor", "close this file", "close the file", "close file"], "workbench.action.closeActiveEditor", "Closed the file"),
        (&["zen mode", "focus mode"], "workbench.action.toggleZenMode", "Zen mode"),
        (&["new terminal", "open a new terminal", "open new terminal"], "workbench.action.terminal.new", "New terminal"),
        (&["show problems", "open problems", "show the problems panel", "show errors"], "workbench.actions.view.problems", "Problems"),
        (&["show extensions", "open extensions", "extensions"], "workbench.view.extensions", "Extensions"),
        (&["open explorer", "show explorer", "show files", "file explorer in vs code"], "workbench.view.explorer", "Explorer"),
        (&["toggle comment", "comment this line", "comment line", "comment the line"], "editor.action.commentLine", "Toggled the comment"),
        (&["start debugging", "debug", "debug it"], "workbench.action.debug.start", "Debugging"),
        (&["stop debugging"], "workbench.action.debug.stop", "Stopped debugging"),
        (&["quick open"], "workbench.action.quickOpen", "Quick open"),
    ];
    for (phrases, id, label) in editor_cmds {
        if phrases.contains(&tx) && (mentions_vscode(t) || !matches!(*id, "workbench.view.explorer")) {
            return Some(act("code.command", json!({ "id": id, "label": label })));
        }
    }
    if let Some(rest) = strip(tx, &["search for ", "find "])
        && (t.contains(" in my project") || t.contains(" in the project") || t.contains(" in all files"))
    {
        let q = rest.trim_end_matches(" in my project").trim_end_matches(" in the project").trim_end_matches(" in all files");
        return Some(act("code.command", json!({ "id": "workbench.action.findInFiles", "args": { "query": q }, "label": format!("Searching for {q}") })));
    }
    if let Some(rest) = strip(tx, &["open terminal and run ", "in the terminal run ", "in terminal run ", "terminal run ", "run the command ", "run command ", "execute the command ", "execute command "]) {
        return Some(act("terminal.run", json!({ "command": spoken_command(rest, raw) })));
    }
    if let Some(rest) = t.strip_prefix("run ")
        && let Some(cmd) = rest.strip_suffix(" in the terminal").or_else(|| rest.strip_suffix(" in terminal"))
    {
        return Some(act("terminal.run", json!({ "command": spoken_command(cmd, raw) })));
    }
    if let Some(rest) = strip(t, &["set my project folder to ", "set project folder to ", "set the project folder to ", "work in ", "my project is in ", "use the folder ", "use folder "]) {
        return Some(act("code.folder", json!({ "name": rest.trim_end_matches(" folder") })));
    }
    if mentions_vscode(t)
        && let Some(rest) = strip(t, &["open ", "open up "])
    {
        let r = rest.split(" in vs code").next().unwrap_or(rest).split(" in vscode").next().unwrap_or(rest).split(" in visual studio code").next().unwrap_or(rest).trim();
        let r = r.trim_start_matches("the ").trim_start_matches("my ");
        if !r.is_empty() && !mentions_vscode(r) {
            let folder = r.ends_with(" folder") || r.ends_with(" project") || r == "project";
            let name = r.trim_end_matches(" folder").trim_end_matches(" project").trim_end_matches(" file");
            return Some(act("code.open", json!({ "name": if folder { name.to_owned() } else { file_name_in(raw, name) }, "folder": folder })));
        }
    }
    None
}

fn spoken_command(normalized: &str, raw: &str) -> String {
    let words = normalized.split_whitespace().count();
    let rendered = codespeech::render(&raw_tail_text(raw, normalized).unwrap_or_else(|| normalized.to_owned()));
    let r = rendered.split_whitespace().collect::<Vec<_>>();
    if r.len() + 2 >= words { rendered } else { codespeech::render(normalized) }
}

fn download_rules(t: &str) -> Option<Intent> {
    let rest = strip(t, &["download ", "can you download ", "please download ", "get me ", "download and install "])?;
    let r = rest.trim_start_matches("the ").trim_start_matches("me ");
    if matches!(r, "this" | "this file" | "this page" | "that" | "it" | "this video" | "this image" | "this pdf") {
        return Some(act("web.download_current", json!({})));
    }
    if let Some(url) = web::spoken_url(r) {
        return Some(act("web.download", json!({ "url": url })));
    }
    if t.starts_with("get me ") && !r.ends_with(" app") {
        return None;
    }
    let app = r.trim_end_matches(" for me").trim_end_matches(" app").trim_end_matches(" application").trim_end_matches(" from the store").trim_end_matches(" from the internet").trim();
    if app.is_empty() || app.split_whitespace().count() > 6 {
        return None;
    }
    if app.ends_with(" extension") || app.ends_with(" plugin") {
        return Some(act("code.extension", json!({ "name": app.trim_end_matches(" extension").trim_end_matches(" plugin") })));
    }
    Some(Intent::Install { app: sentence_case(app) })
}

fn with_list_text(i: Intent, raw: &str) -> Intent {
    match i {
        Intent::Act { tool, mut args } if tool == "board.todo" && args.get("op").and_then(Value::as_str) == Some("many") => {
            let keep: String = raw.to_lowercase().replace("to-do", "to do").replace("todo list", "to do list").chars().filter(|c| c.is_alphanumeric() || c.is_whitespace() || matches!(c, ',' | ';' | '\'' | ':')).collect();
            args["text"] = json!(keep.split_whitespace().collect::<Vec<_>>().join(" "));
            Intent::Act { tool, args }
        }
        other => other,
    }
}

pub fn parse(t: &str, raw: &str) -> Option<Intent> {
    if t.is_empty() {
        return None;
    }
    crate::vault::rules(t)
        .or_else(|| crate::signup::rules(t))
        .or_else(|| crate::faceid::rules(t))
        .or_else(|| crate::voicekey::rules(t))
        .or_else(|| crate::memory::rules(t, raw))
        .or_else(|| crate::messaging::rules(t))
        .or_else(|| crate::understand::rules(t, raw))
        .or_else(|| crate::access::rules(t))
        .or_else(|| crate::profiles::rules(t))
        .or_else(|| crate::vision::rules(t))
        .or_else(|| crate::foxctl::rules(t))
        .or_else(|| crate::jobs::url_rule(raw))
        .or_else(|| crate::jobs::rules(t))
        .or_else(|| crate::privileged::rules(t))
        .or_else(|| crate::syscfg::rules(t))
        .or_else(|| crate::dictate::rules(t))
        .or_else(|| code_rules(t, raw))
        .or_else(|| crate::board::rules(t).map(|i| with_list_text(i, raw)))
        .or_else(|| git_rules(t))
        .or_else(|| crate::playback::rules(t))
        .or_else(|| crate::tools::pointer::rules(t))
        .or_else(|| crate::browse::rules(t, raw))
        .or_else(|| window_rules(t))
        .or_else(|| download_rules(t))
        .or_else(|| key_rules(t))
        .or_else(|| ui_rules(t))
        .or_else(|| crate::tools::channels::rules(t))
        .or_else(|| crate::tools::channels::mail_rule(t, raw))
        .or_else(|| file_rules(t))
        .or_else(|| web_rules(t))
        .or_else(|| click_rules(t, raw))
        .or_else(|| crate::memory::personal_fact(t).then(|| Intent::Act { tool: "memory.add".into(), args: json!({ "text": raw.trim(), "source": "fact" }) }))
}

pub fn label(intent: &Intent) -> String {
    let Intent::Act { tool, args } = intent else { return String::new() };
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool.as_str() {
        "web.open" => {
            let what = if !s("site").is_empty() { s("site") } else if !s("url").is_empty() { web::host(&s("url")) } else { "the browser".into() };
            if s("browser").is_empty() { format!("Opening {what}") } else { format!("Opening {what} in {}", sentence_case(&s("browser"))) }
        }
        "web.search" => format!("Searching {} for {}", if s("site").is_empty() { "the web".into() } else { s("site") }, s("query")),
        "web.play" => if s("query").is_empty() { "Finding a song".into() } else { format!("Playing {}", s("query")) },
        "key" => s("label"),
        "ui.click" => format!("Clicking {}", s("name")),
        "pointer.click" if s("name").is_empty() => "Clicking here".into(),
        "pointer.click" => format!("Clicking {}", s("name")),
        "pointer.hover" => format!("Pointing at {}", s("name")),
        "pointer.hold" => "Holding the mouse button".into(),
        "pointer.release" => "Letting go".into(),
        "pointer.drag" => format!("Dragging {}", s("name")),
        "pointer.scroll" => "Scrolling".into(),
        "ui.first" => "Opening the first result".into(),
        t if t.starts_with("memory.") => crate::memory::label(t),
        t if t.starts_with("messages.") => crate::messaging::label(t, args),
        "ui.fill" => format!("Typing into {}", s("field")),
        "app.write" => format!("Writing in {}", sentence_case(&s("app"))),
        "files.create" => args.get("items").and_then(Value::as_array).and_then(|a| a.first()).and_then(|i| i.get("path")).and_then(Value::as_array).and_then(|p| p.last()).and_then(Value::as_str).map(|n| format!("Creating {n}")).unwrap_or_else(|| "Creating it".into()),
        "page.vertical" => format!("Opening {}", crate::understand::vertical_label(&s("kind"))),
        "ui.type" => "Typing".into(),
        "files.open" => format!("Opening {}", s("where")),
        "files.find" => format!("Looking for {}", s("name")),
        "files.openfile" | "files.openfolder" => format!("Opening {}", s("name")),
        "files.reveal" => format!("Showing {}", s("name")),
        "files.where" => "Checking the open folders".into(),
        "system.lock" => "Locking the screen".into(),
        "doc.cell" => if args.get("value").is_some() { format!("Putting {} in {}", s("value"), s("cell")) } else { format!("Reading {}", s("cell")) },
        "doc.export" => "Saving as a PDF".into(),
        "doc.mail" => if args.get("send").and_then(Value::as_bool).unwrap_or(false) { "Sending the email".into() } else { format!("Writing an email to {}", s("to")) },
        "shell.extension" => if s("op") == "list" { "Listing your extensions".into() } else { format!("Turning {} {}", s("op"), s("name")) },
        "files.mkdir" => format!("Creating the folder {}", s("name")),
        "files.delete" => format!("Moving {} to the trash", s("name")),
        "files.copy" => format!("Copying {}", s("name")),
        "files.move" => format!("Moving {}", s("name")),
        "files.space" => "Checking free space".into(),
        "files.latest" => "Opening your latest download".into(),
        "git.status" => "Checking Git".into(),
        "git.commit" => "Committing".into(),
        "git.commit_push" => "Committing and pushing".into(),
        "git.push" => "Pushing to GitHub".into(),
        "git.pull" => "Pulling".into(),
        "git.branch" => format!("Branch {}", s("name")),
        "git.login" => "Signing in to GitHub".into(),
        "code.check" => "Checking your code".into(),
        "code.run" => "Running your code".into(),
        "code.build" => "Building".into(),
        "code.failed" => "Checking why it stopped".into(),
        "code.extension" => format!("Installing the {} extension", s("name")),
        "code.uninstall" => format!("Removing the {} extension", s("name")),
        "code.setting" => s("label"),
        "code.command" => s("label"),
        "code.dictate" => "Coding mode".into(),
        "text.dictate" => "Dictation".into(),
        "code.start" => "Opening VS Code".into(),
        "code.compose" => "Writing your code".into(),
        "code.new_file" => if s("name").is_empty() { "Creating a file".into() } else { format!("Creating {}", s("name")) },
        "code.project" => if s("name").is_empty() { "Creating a new project".into() } else { format!("Creating the project {}", sentence_case(&s("name"))) },
        "code.write" => "Writing the code".into(),
        "code.comment" => "Adding a comment".into(),
        "code.insert" => "Typing".into(),
        "wm.place" => match (s("where").as_str(), s("app").is_empty()) {
            ("left", true) => "Moving the window left".into(),
            ("right", true) => "Moving the window right".into(),
            ("left", false) => format!("Moving {} to the left", sentence_case(&s("app"))),
            ("right", false) => format!("Moving {} to the right", sentence_case(&s("app"))),
            ("max", _) => "Maximising".into(),
            ("min", _) => "Minimising".into(),
            ("restore", _) => "Restoring the window".into(),
            ("center", _) => "Centring the window".into(),
            _ => "Moving the window".into(),
        },
        "wm.side" => if s("left").is_empty() { "Arranging your windows".into() } else { format!("Putting {} and {} side by side", sentence_case(&s("left")), sentence_case(&s("right"))) },
        "wm.focus" => format!("Switching to {}", sentence_case(&s("app"))),
        "wm.workspace" => if args.get("move").and_then(Value::as_bool) == Some(true) { "Moving the window".into() } else { "Switching workspace".into() },
        "ui.tick" => format!("Ticking {}", s("name")),
        "ui.choose" => format!("Choosing {}", s("option")),
        "wm.list" => "Checking the open windows".into(),
        "wm.frame" => if matches!(s("op").as_str(), "corner" | "nudge") { "Moving the window".into() } else { "Resizing the window".into() },
        "wm.desktop" => "Showing the desktop".into(),
        "wm.switch" => "Switching windows".into(),
        "wm.close" => "Closing the window".into(),
        "terminal.run" => format!("Running {}", s("command")),
        "web.download" => "Downloading".into(),
        "task" => format!("Working on it: {}", s("goal")),
        t if t.starts_with("media.") => crate::playback::label(t, args),
        t if t.starts_with("board.") => crate::board::label(t, args),
        t if t.starts_with("profiles.") => crate::profiles::label(t, args),
        t if t.starts_with("look.") => crate::looks::label(t, args),
        t if t.starts_with("admin.") => crate::privileged::label(t, args),
        t if t.starts_with("sys.") => crate::syscfg::label(t, args),
        t if t.starts_with("jobs.") => crate::jobs::label(t, args),
        t if t.starts_with("vision.") => crate::vision::label(t, args),
        "vault.signup" => crate::signup::label(args),
        t if t.starts_with("vault.") => crate::vault::label(t, args),
        t if t.starts_with("face.") => crate::faceid::label(t, args),
        t if t.starts_with("voice.") => crate::voicekey::label(t, args),
        t if t.starts_with("browser.") => crate::browse::label(t, args),
        t if t.starts_with("access.") => crate::access::label(t, args),
        _ => String::new(),
    }
}

pub fn layer(tool: &str) -> &'static str {
    match tool.split('.').next().unwrap_or("") {
        "web" => "browser",
        "key" => "keyboard",
        "ui" => "accessibility",
        "pointer" => "pointer",
        "files" => "files",
        "git" => "git",
        "code" | "terminal" => "editor",
        "task" => "agent",
        "memory" => "memory",
        "app" | "text" => "accessibility",
        "profiles" => "browser",
        "page" => "browser",
        "messages" => "browser",
        "wm" => "keyboard",
        "media" => "media",
        "browser" => "browser",
        "access" => "accessibility",
        "vault" | "face" | "voice" => "vault",
        _ => "registered",
    }
}

pub fn cancellable(tool: &str) -> bool {
    matches!(tool, "task" | "web.download" | "code.extension" | "git.push" | "git.pull" | "files.find" | "files.copy" | "files.move" | "face.enrol" | "face.check" | "face.signin" | "face.admin" | "voice.key" | "vault.show" | "vault.say" | "vault.type" | "vault.signin")
}

pub fn state(core: &Core) -> Value {
    json!({
        "vscode": {
            "installed": code::installed(),
        },
        "editor": crate::ide::by_key(&core.settings.str("code.ide")).map(|i| json!({ "key": i.key, "name": i.name })),
        "git": git::git().is_some(),
        "keyboard": input::status(),
        "project": core.settings.str("code.folder"),
        "browser": web::find_browser(core, None).map(|a| json!({ "id": a.id, "name": a.name })),
        "browsers": core.apps.all().into_iter().filter(|a| !a.hidden && a.roles.iter().any(|r| r == "browser")).map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>(),
        "drives": files::drives().iter().map(|d| json!({ "path": d.path, "name": files::drive_name(d), "removable": d.removable, "free": d.free, "total": d.total })).collect::<Vec<_>>(),
        "installers": installers(),
        "downloads": dirs::download_dir(),
    })
}

fn installers() -> Vec<&'static str> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        v.push("Microsoft Store");
        v.push("winget");
    }
    #[cfg(target_os = "macos")]
    {
        if crate::tools::apps::which("mas").is_some() {
            v.push("App Store");
        }
        if crate::tools::apps::which("brew").is_some() || Path::new("/opt/homebrew/bin/brew").exists() {
            v.push("Homebrew");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(m) = crate::tools::install::native_manager() {
            v.push(m);
        }
        if crate::tools::apps::which("flatpak").is_some() {
            v.push("Flatpak");
        }
        if crate::tools::apps::which("snap").is_some() {
            v.push("Snap");
        }
    }
    v
}

pub fn project_dir(core: &Core) -> Option<PathBuf> {
    if let Some(r) = crate::ide::last_root() {
        return Some(r);
    }
    let saved = core.settings.str("code.folder");
    (!saved.is_empty() && Path::new(&saved).is_dir()).then(|| PathBuf::from(saved))
}

fn repo(core: &Core) -> Result<PathBuf, String> {
    let dir = project_dir(core).ok_or("Which project? Open it in VS Code, or say: set project folder to, then the folder name.")?;
    git::root(&dir).ok_or_else(|| format!("{} isn't a Git project yet. Say initialize git.", dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
}

fn ok(did: &str, layer: &'static str) -> Outcome {
    Outcome::ok_with(did, layer, None)
}

fn fail(msg: &str, layer: &'static str) -> Outcome {
    Outcome::fail(msg, layer)
}

fn say(did: &str, layer: &'static str, reply: String) -> Outcome {
    let mut o = Outcome::ok_with(did, layer, None);
    o.reply = Some(reply);
    o
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    crate::layers::blocking(f).await.ok()
}

async fn native(core: &Core) -> Option<(&'static crate::ide::Ide, crate::os::Win)> {
    crate::ide::find(core, None).await
}

async fn native_active(core: &Core) -> Option<(&'static crate::ide::Ide, crate::os::Win, crate::ide::Active)> {
    let (ide, win) = native(core).await?;
    let a = crate::ide::active(core, ide, &win).await;
    Some((ide, win, a))
}

fn edit_command(op: &str) -> Option<&'static str> {
    Some(match op {
        "undo" => "undo",
        "redo" => "redo",
        "save" => "workbench.action.files.save",
        "format" => "editor.action.formatDocument",
        "comment" => "editor.action.commentLine",
        "indent" => "editor.action.indentLines",
        "outdent" => "editor.action.outdentLines",
        "copy_line" => "editor.action.copyLinesDownAction",
        "paste" => "editor.action.clipboardPasteAction",
        "select_all" => "editor.action.selectAll",
        "new_file" => "workbench.action.files.newUntitledFile",
        "terminal" => "workbench.action.terminal.toggleTerminal",
        "delete_line" => "editor.action.deleteLines",
        _ => return None,
    })
}

fn command_keys(id: &str) -> Option<Vec<K>> {
    let p = input::primary();
    Some(match id {
        "editor.action.formatDocument" if cfg!(target_os = "linux") => vec![K::Ctrl, K::Shift, K::Ch('i')],
        "editor.action.formatDocument" => vec![K::Shift, K::Alt, K::Ch('f')],
        "workbench.action.toggleSidebarVisibility" => vec![p, K::Ch('b')],
        "workbench.action.splitEditor" => vec![p, K::Ch('\\')],
        "workbench.action.closeActiveEditor" => vec![p, K::Ch('w')],
        "workbench.action.terminal.new" => vec![K::Ctrl, K::Shift, K::Ch('`')],
        "workbench.actions.view.problems" => vec![p, K::Shift, K::Ch('m')],
        "workbench.view.extensions" => vec![p, K::Shift, K::Ch('x')],
        "workbench.view.explorer" => vec![p, K::Shift, K::Ch('e')],
        "editor.action.commentLine" => vec![p, K::Ch('/')],
        "workbench.action.debug.start" => vec![K::F(5)],
        "workbench.action.debug.stop" => vec![K::Shift, K::F(5)],
        "workbench.action.quickOpen" => vec![p, K::Ch('p')],
        "workbench.action.openSettings" => vec![p, K::Ch(',')],
        "workbench.action.findInFiles" => vec![p, K::Shift, K::Ch('f')],
        _ => return None,
    })
}

async fn open_in(core: &Arc<Core>, dir: &Path, file: Option<&Path>, created: bool) -> Option<&'static str> {
    let ide = crate::ide::preferred(core)?;
    if created {
        crate::ide::prepare_open(ide, dir);
    }
    let ok = if ide.key == "vscode" {
        let (d, f) = (dir.to_path_buf(), file.map(Path::to_path_buf));
        matches!(blocking(move || code::open_new_window(&d, f.as_deref())).await, Some(Ok(())))
    } else {
        let mut ok = crate::ide::open(ide, dir, true).await.is_ok();
        if ok && let Some(f) = file {
            tokio::time::sleep(Duration::from_millis(if ide.family == crate::ide::Family::VsCode { 400 } else { 2500 })).await;
            ok = crate::ide::open(ide, f, false).await.is_ok() || ok;
        }
        ok
    };
    if !ok {
        return None;
    }
    let (c2, d2) = (core.clone(), dir.to_path_buf());
    tokio::spawn(async move {
        crate::ide::after_open(&c2, ide, &d2, created).await;
    });
    Some(ide.name)
}

async fn build_native(core: &Arc<Core>, layer: &'static str) -> Outcome {
    let Some((ide, win)) = native(core).await else {
        return fail("Open your project in your coding app first, then ask me to build it.", layer);
    };
    let _ = crate::ide::save(ide, &win).await;
    let list = crate::ide::build_steps(ide);
    if !list.is_empty() {
        return match crate::ide::steps(core, ide, &win, &list).await {
            Ok(()) => ok(&format!("Building in {}", ide.name), layer),
            Err(e) => fail(&e.to_string(), layer),
        };
    }
    let checked = check_code(core, layer, false, true).await;
    if !checked.ok || checked.reply.is_some() {
        return checked;
    }
    say("Checked the code", layer, format!("{} has no build command, so I checked the file instead. It looks right.", ide.name))
}

static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn watch_run(core: &Arc<Core>, ide: &'static crate::ide::Ide, win: crate::os::Win, path: PathBuf, typed: String) {
    let c = core.clone();
    let mine = RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    tokio::spawn(async move {
        let lang = code::lang(&path, "");
        let until = std::time::Instant::now() + Duration::from_secs(40);
        while std::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            if RUNS.load(std::sync::atomic::Ordering::SeqCst) != mine {
                return;
            }
            let Some((out, finished)) = crate::ide::terminal_output(&win, &typed).await else { continue };
            let Some(p) = code::run_error(lang, &out) else {
                if finished {
                    return;
                }
                continue;
            };
            let reply = if p.line == 0 {
                format!("It stopped with an error: {}.", p.message)
            } else {
                let _ = crate::ide::goto(ide, Some(&win), &path, p.line, p.col).await;
                format!("It stopped with an error. {}", sentence_case(code::explain(&p).trim_start_matches("Yes, ")))
            };
            c.tell(crate::control::ControlMsg::RunFailed { reply });
            return;
        }
    });
}

async fn run_native(core: &Arc<Core>, layer: &'static str, ide: &'static crate::ide::Ide, win: &crate::os::Win, a: crate::ide::Active) -> Outcome {
    let Some(path) = a.path.clone() else { return fail(&format!("I can't tell which file is open in {}. Save it first, then ask me again.", ide.name), layer) };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if a.dirty {
        let _ = crate::ide::save(ide, win).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let own = crate::ide::run_steps(ide);
    if !own.is_empty() && crate::ide::steps(core, ide, win, &own).await.is_ok() {
        return ok(&format!("Running {name} in {}", ide.name), layer);
    }
    let Some(plan) = code::run_plan(&path, &a.language) else { return fail(&format!("I don't know how to run {} files yet.", path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()), layer) };
    if ide.family == crate::ide::Family::VsCode {
        match crate::ide::terminal(ide, win, &plan.steps, Some(&plan.dir)).await {
            Ok(typed) => {
                watch_run(core, ide, win.clone(), path.clone(), typed);
                return ok(&format!("Running {name}"), layer);
            }
            Err(e) => log::info!("{} terminal: {e}", ide.name),
        }
    }
    let line = plan.steps.join(if cfg!(windows) { " ; " } else { " && " });
    let dir = plan.dir.clone();
    match blocking(move || open_terminal_in(&line, Some(&dir))).await {
        Some(Ok(())) => ok(&format!("Running {name}"), layer),
        _ => fail("A terminal could not be opened.", layer),
    }
}

async fn open_vscode(core: &Arc<Core>) -> bool {
    if crate::ide::find(core, None).await.is_some() {
        return true;
    }
    let preferred = crate::ide::preferred(core);
    let defaults = core.defaults();
    let app = match preferred {
        Some(i) if i.key != "vscode" => core.apps.find(i.name, None, &defaults).into_iter().map(|(a, _)| a).next(),
        _ => core.apps.find("visual studio code", None, &defaults).into_iter().map(|(a, _)| a).next().or_else(|| core.apps.find("vs code", None, &defaults).into_iter().map(|(a, _)| a).next()),
    };
    match (app, preferred) {
        (Some(a), _) => {
            let _ = blocking(move || crate::tools::apps::launch(&a)).await;
        }
        (None, Some(i)) if i.key != "vscode" => {
            let Some(cli) = crate::ide::cli_path(i) else { return false };
            let _ = blocking(move || crate::tools::shell::run_detached(&cli, &[], None)).await;
        }
        _ => {
            if !code::installed() {
                return false;
            }
            let _ = blocking(|| code::reuse(&[])).await;
        }
    }
    for _ in 0..60 {
        if crate::ide::find(core, None).await.is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

pub async fn run(core: &Arc<Core>, tool: &str, args: Value, cancel: Arc<AtomicBool>) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let layer = layer(tool);
    if tool.starts_with("media.") {
        return crate::playback::run(core, tool, args).await;
    }
    if tool == "browser.tab.activate"
        && let Some(o) = crate::browse::setu_tab_first(&args).await
    {
        return o;
    }
    if tool == "browser.page"
        && s("action") == "menu"
        && s("browser").is_empty()
        && let Some(t) = args.get("ref").and_then(|r| r.get("text")).and_then(Value::as_str).map(str::to_owned)
        && crate::os::working().await.is_ok_and(|w| !crate::browserui::is_browser(&w.app))
    {
        return Box::pin(run(core, "pointer.click", json!({ "name": t, "button": "right" }), cancel)).await;
    }
    if (matches!(tool, "web.open" | "web.search" | "web.play") || (tool.starts_with("browser.") && !matches!(tool, "browser.choose" | "browser.setup" | "browser.fox" | "browser.foxsetup")))
        && s("browser").is_empty()
        && let Some(o) = crate::browse::choose_first(core, tool, &args)
    {
        return o;
    }
    if tool == "browser.fox" || tool == "browser.foxsetup" {
        return crate::foxctl::run(core, tool, &args).await;
    }
    if tool.starts_with("browser.") {
        return crate::browse::run(core, tool, args).await;
    }
    if tool.starts_with("memory.") {
        crate::layers::fallback(crate::layers::Layer::Setu);
        return crate::memory::run(core, tool, &args);
    }
    if tool.starts_with("messages.") {
        return crate::messaging::run(core, tool, &args).await;
    }
    if tool.starts_with("board.") {
        crate::layers::fallback(crate::layers::Layer::Setu);
        return crate::board::run(core, tool, args).await;
    }
    if tool.starts_with("profiles.") {
        return crate::profiles::run(core, tool, &args).await;
    }
    if tool.starts_with("look.") {
        return crate::looks::run(core, tool, &args).await;
    }
    if tool.starts_with("admin.") {
        return crate::privileged::run(core, tool, &args).await;
    }
    if tool.starts_with("sys.") {
        return crate::syscfg::run(core, tool, &args).await;
    }
    if tool.starts_with("jobs.") {
        return crate::jobs::run_tool(core, tool, &args).await;
    }
    if tool.starts_with("vision.") {
        return crate::vision::run(core, tool, &args).await;
    }
    if tool.starts_with("vault.") {
        return crate::vault::run(core, tool, &args, cancel).await;
    }
    if tool.starts_with("face.") {
        return crate::faceid::run(core, tool, &args, cancel).await;
    }
    if tool.starts_with("voice.") {
        return crate::voicekey::run(core, tool, &args, cancel).await;
    }
    if tool.starts_with("access.") {
        return crate::access::run(core, tool, &args).await;
    }
    if tool.starts_with("files.") {
        crate::layers::fallback(crate::layers::Layer::System);
    } else if tool == "task" || tool == "code.write" || tool == "code.compose" || tool == "code.comment" {
        crate::layers::fallback(crate::layers::Layer::Setu);
    }
    match tool {
        "web.open" | "web.search" | "web.play" => {
            let browser_name = s("browser");
            let browser = if browser_name.is_empty() { web::find_browser(core, None) } else { web::find_browser(core, Some(&browser_name)) };
            if !browser_name.is_empty() && browser.is_none() {
                let mut o = fail(&format!("{} isn't installed. Say install {}.", sentence_case(&browser_name), browser_name), layer);
                o.fix = Some(json!({ "label": format!("Install {}", sentence_case(&browser_name)), "method": "intent.run", "params": { "text": format!("install {browser_name}") } }));
                return o;
            }
            let site = web::site(&s("site")).or_else(|| web::SITES.iter().find(|x| x.name == s("site")));
            let (url, did) = match tool {
                "web.open" => {
                    let url = if !s("url").is_empty() {
                        s("url")
                    } else if let Some(si) = site {
                        si.home.to_owned()
                    } else {
                        String::new()
                    };
                    if url.is_empty() {
                        let Some(b) = browser.clone() else { return fail("No browser is installed.", layer) };
                        return match blocking(move || crate::tools::apps::launch(&b)).await {
                            Some(Ok(_)) => ok(&format!("Opened {}", browser.map(|b| b.name).unwrap_or_default()), layer),
                            _ => fail("The browser could not be started.", layer),
                        };
                    }
                    let what = site.map(|x| x.name.to_owned()).unwrap_or_else(|| web::host(&url));
                    (url, format!("Opened {what}"))
                }
                "web.search" => {
                    let q = s("query");
                    if site.is_none_or(|x| x.name == "Google") {
                        crate::understand::remember_search(&q);
                    }
                    (web::search_url(site, &q), format!("Searched {} for {q}", site.map(|x| x.name).unwrap_or("Google")))
                }
                _ => {
                    let q = s("query");
                    let site_name = site.map(|x| x.name).unwrap_or("YouTube");
                    if site_name == "YouTube" || site_name == "YouTube Music" {
                        let query = if q.is_empty() { "top songs this week".to_owned() } else { q.clone() };
                        let qq = query.clone();
                        match blocking(move || web::youtube_video(&qq)).await.flatten() {
                            Some((id, title)) => (format!("https://www.youtube.com/watch?v={id}"), if title.is_empty() { format!("Playing {query} on YouTube") } else { format!("Playing {title}") }),
                            None => (web::search_url(web::site("youtube"), &query), format!("Showing {query} on YouTube")),
                        }
                    } else {
                        (web::search_url(site, if q.is_empty() { "popular" } else { &q }), format!("Opened {q} on {site_name}"))
                    }
                }
            };
            if s("where") == "here"
                && let Some(app) = browser.as_ref()
                && let Some(key) = crate::tools::browsers::key_of(app)
                && crate::proto::supported(key)
                && !crate::cdp::has_profile(&core.paths.data, key)
                && !crate::cdp::declined(&core.paths.data, key)
            {
                return crate::browse::offer_setup(app, json!({ "tool": tool, "args": args }));
            }
            if let Some(app) = browser.as_ref()
                && let Some(key) = crate::tools::browsers::key_of(app)
                && crate::cdp::is_default(&core.paths.data, key)
            {
                let exe = crate::cdp::exe_of(&app.exec, &app.path);
                let here = s("where") == "here";
                let r = async {
                    let (c, started) = crate::proto::ensure(&core.paths.data, key, &exe, if here { None } else { Some(url.as_str()) }).await?;
                    if started && !here {
                        return Ok::<_, anyhow::Error>(());
                    }
                    if here {
                        let tab = crate::proto::active_tab(&c).await?;
                        c.navigate(&tab.id, &url).await
                    } else {
                        let id = c.open(&url).await?;
                        c.activate(&id).await
                    }
                }
                .await;
                return match r {
                    Ok(()) => {
                        let did = if browser_name.is_empty() { did } else { format!("{did} in {}", app.name) };
                        Outcome::ok_with(&did, layer, Some(app.name.clone()))
                    }
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            let b = browser.clone();
            let c = core.clone();
            match blocking(move || web::open(&c, b.as_ref(), &url)).await {
                Some(Ok(name)) => {
                    let did = if browser_name.is_empty() { did } else { format!("{did} in {name}") };
                    let mut o = Outcome::ok_with(&did, layer, browser.map(|b| b.name));
                    o.message = did.clone();
                    o
                }
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The browser could not be started.", layer),
            }
        }
        "key" => {
            let spoken = s("keys");
            let which = match s("label").as_str() {
                "Went back" => Some("back"),
                "Went forward" => Some("forward"),
                "Refreshed" => Some("reload"),
                _ => None,
            };
            if let Some(w) = which
                && let Some(o) = crate::browse::protocol_first("browser.nav", &json!({ "op": w })).await
            {
                return o;
            }
            if let Some(w) = which
                && crate::os::working().await.is_ok_and(|win| crate::browserui::is_browser(&win.app))
                && let Some(o) = crate::browserui::nav("", w).await
            {
                return o;
            }
            let keys = match spoken.as_str() {
                "playpause" => vec![K::PlayPause],
                "nexttrack" => vec![K::NextTrack],
                "prevtrack" => vec![K::PrevTrack],
                "win d" => vec![K::Meta, K::Ch('d')],
                "win up" => vec![K::Meta, K::Up],
                "win down" => vec![K::Meta, K::Down],
                "screenshot" if cfg!(windows) => vec![K::Meta, K::Shift, K::Ch('s')],
                "screenshot" if cfg!(target_os = "macos") => vec![K::Meta, K::Shift, K::Ch('4')],
                "screenshot" => vec![K::Print],
                "ctrl equals" => vec![input::primary(), K::Ch('=')],
                "ctrl minus" => vec![input::primary(), K::Ch('-')],
                "fullscreen" if cfg!(target_os = "macos") => vec![K::Ctrl, K::Meta, K::Ch('f')],
                "fullscreen" => vec![K::F(11)],
                "word delete" if cfg!(target_os = "macos") => vec![K::Alt, K::Back],
                "word delete" => vec![K::Ctrl, K::Back],
                "word select" if cfg!(target_os = "macos") => vec![K::Alt, K::Shift, K::Left],
                "word select" => vec![K::Ctrl, K::Shift, K::Left],
                "line end" if cfg!(target_os = "macos") => vec![K::Meta, K::Right],
                "line end" => vec![K::End],
                "line start" if cfg!(target_os = "macos") => vec![K::Meta, K::Left],
                "line start" => vec![K::Home],
                other => match input::parse_combo(other) {
                    Some(mut k) => {
                        if cfg!(target_os = "macos") {
                            for x in k.iter_mut() {
                                if *x == K::Ctrl {
                                    *x = K::Meta;
                                }
                            }
                        }
                        k
                    }
                    None => return fail(&format!("I don't know the key {other}."), layer),
                },
            };
            let times = args.get("times").and_then(Value::as_u64).unwrap_or(1) as usize;
            match input::press_times(&keys, times).await {
                Ok(()) => ok(&if s("label").is_empty() { format!("Pressed {}", input::describe(&keys)) } else { s("label") }, layer),
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        t if t.starts_with("pointer.") => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            match crate::tools::pointer::tool(t, &args).await {
                Ok(m) => ok(&m, layer),
                Err(e) if matches!(t, "pointer.hover" | "pointer.click") && !s("name").is_empty() && crate::vision::can_see(core) => {
                    let name = s("name");
                    let seen = if t == "pointer.hover" { crate::vision::point_seen(core, &name).await } else { crate::vision::click_seen(core, &name).await.map(|_| ()) };
                    match seen {
                        Ok(()) => Outcome::ok_with(&if t == "pointer.hover" { format!("Pointing at {name}") } else { format!("Clicked {name}") }, "vision", None),
                        Err(_) => fail(&e.to_string(), layer),
                    }
                }
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        "ui.tick" | "ui.choose" => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            match if tool == "ui.tick" { tick(&s("name"), args.get("on").and_then(Value::as_bool).unwrap_or(true)).await } else { choose(&s("option"), &s("list")).await } {
                Ok((m, app)) => Outcome::ok_with(&m, layer, Some(app)),
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        "ui.click" | "ui.first" => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            let named = s("app") == "setu";
            let wins = match crate::os::ui_targets(named).await {
                Ok(w) => w,
                Err(e) => return fail(&e.to_string(), layer),
            };
            let roles: Vec<&str> = args.get("roles").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut problem = None;
            let mut blind: Option<crate::os::Win> = None;
            for win in &wins {
                let screen = match crate::tools::atspi::snapshot_of(win, 250).await {
                    Ok(s) => s,
                    Err(e) => {
                        problem.get_or_insert(e.to_string());
                        blind.get_or_insert(win.clone());
                        continue;
                    }
                };
                if screen.elements.is_empty() {
                    blind.get_or_insert(win.clone());
                }
                let target = if tool == "ui.first" { first_result(&screen) } else { crate::tools::atspi::find(&screen, &s("name"), &roles).cloned() };
                if let Some(el) = target {
                    let name = if el.label.is_empty() { "it".to_owned() } else { el.label.clone() };
                    let selectable = matches!(el.role.as_str(), "row" | "item" | "treeitem" | "cell" | "tab" | "radio");
                    return match crate::tools::atspi::click(&el).await {
                        Ok(()) => Outcome::ok_with(&format!("Clicked {name}"), layer, Some(win.app.clone())),
                        Err(_) if selectable && crate::tools::atspi::select(&el).await.is_ok() => Outcome::ok_with(&format!("Selected {name}"), layer, Some(win.app.clone())),
                        Err(e) if el.w > 0 && el.h > 0 => {
                            let n = crate::os::Node { x: el.x, y: el.y, w: el.w, h: el.h, ..Default::default() };
                            let at = crate::tools::pointer::place(win, &n).await;
                            match crate::tools::pointer::click(Some(at), crate::tools::pointer::Button::Left, 1).await {
                                Ok(()) => Outcome::ok_with(&format!("Clicked {name}"), layer, Some(win.app.clone())),
                                Err(_) => fail(&e.to_string(), layer),
                            }
                        }
                        Err(e) => fail(&e.to_string(), layer),
                    };
                }
            }
            if tool == "ui.click"
                && crate::vision::installed(core)
                && let Ok(t) = crate::vision::click_text(core, &s("name")).await
            {
                return Outcome::ok_with(&format!("Clicked {t}"), "vision", None);
            }
            if tool == "ui.click"
                && crate::vision::can_see(core)
                && let Ok(t) = crate::vision::click_seen(core, &s("name")).await
            {
                return Outcome::ok_with(&format!("Clicked {t}"), "vision", None);
            }
            if let Some(w) = &blind
                && let Some(o) = crate::access::explain(core, Some(w)).await
            {
                return o;
            }
            match problem {
                Some(p) if wins.len() == 1 => fail(&p, layer),
                _ if tool == "ui.first" => fail("I couldn't find a result to open on this screen.", layer),
                _ => fail(&format!("I couldn't find {} on the screen. Say show numbers to pick it by number.", s("name")), layer),
            }
        }
        "ui.fill" => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            let field = s("field");
            let wins = match crate::os::ui_targets(s("app") == "setu").await {
                Ok(w) => w,
                Err(e) => return fail(&e.to_string(), layer),
            };
            for win in &wins {
                let Ok(screen) = crate::tools::atspi::snapshot_of(win, 250).await else { continue };
                let el = crate::tools::atspi::find(&screen, &field, &["field", "password", "combobox"]).cloned().or_else(|| screen.elements.iter().find(|e| e.role == "field").cloned());
                if let Some(e) = el {
                    return match crate::tools::atspi::set_text_in(win, &e, &s("text")).await {
                        Ok(()) => ok(&format!("Typed into {}", if e.label.is_empty() { field } else { e.label }), layer),
                        Err(err) => fail(&err.to_string(), layer),
                    };
                }
            }
            if crate::vision::installed(core)
                && let Ok(label) = crate::vision::fill_seen(core, &field, &s("text")).await
            {
                return Outcome::ok_with(&format!("Typed into {label}"), "vision", None);
            }
            fail(&format!("I couldn't find the {field} field."), layer)
        }
        "ui.type" => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            let text = s("text");
            match crate::tools::atspi::type_text(&text).await {
                Ok(()) => ok(&format!("Typed {} characters", text.chars().count()), layer),
                Err(_) => match input::type_text(&text).await {
                    Ok(()) => ok(&format!("Typed {} characters", text.chars().count()), "keyboard"),
                    Err(e) => fail(&e.to_string(), layer),
                },
            }
        }
        "files.open" => {
            let place = s("where");
            if place == "computer" {
                let r = if cfg!(windows) {
                    blocking(|| crate::tools::shell::open("shell:MyComputerFolder")).await
                } else {
                    let first = files::drives().into_iter().find(|d| d.removable).map(|d| d.path).or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("/"));
                    blocking(move || crate::tools::shell::open(&first.to_string_lossy())).await
                };
                let list = files::drives().iter().map(files::drive_name).collect::<Vec<_>>();
                return match r {
                    Some(Ok(())) => say("Showed your drives", layer, format!("You have {}.", join_names(&list))),
                    _ => say("Listed your drives", layer, format!("You have {}.", join_names(&list))),
                };
            }
            let Some(p) = files::location(&place) else {
                if ["flash", "pen drive", "usb", "external", "sd card", "memory card", "thumb"].iter().any(|w| place.contains(w)) {
                    return fail("I don't see a USB or flash drive. Plug it in and try again.", layer);
                }
                return fail(&format!("I couldn't find {place}."), layer);
            };
            let shown = files::drives().into_iter().find(|d| d.path == p).map(|d| files::drive_name(&d)).unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()));
            let pp = p.clone();
            match blocking(move || crate::tools::shell::open(&pp.to_string_lossy())).await {
                Some(Ok(())) => ok(&format!("Opened {shown}"), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The folder could not be opened.", layer),
            }
        }
        "files.space" => {
            let place = s("where");
            let ds = files::drives();
            let pick: Vec<&files::Drive> = if place.is_empty() {
                ds.iter().filter(|d| d.total > 0).collect()
            } else {
                match files::location(&place) {
                    Some(p) => ds.iter().filter(|d| d.path == p || p.starts_with(&d.path)).take(1).collect(),
                    None => ds.iter().filter(|d| d.total > 0).collect(),
                }
            };
            if pick.is_empty() {
                return fail("I couldn't read the drive sizes.", layer);
            }
            let parts: Vec<String> = pick.iter().take(4).map(|d| format!("{} has {} free of {}", files::drive_name(d), files::size(d.free), files::size(d.total))).collect();
            say("Checked free space", layer, format!("{}.", sentence_case(&parts.join(", "))))
        }
        "files.latest" => {
            let dir = files::location(&s("where")).or_else(dirs::download_dir);
            let Some(dir) = dir else { return fail("The Downloads folder was not found.", layer) };
            let Some(f) = files::latest(&dir) else { return fail("Your Downloads folder is empty.", layer) };
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match blocking(move || crate::tools::shell::open(&f.to_string_lossy())).await {
                Some(Ok(())) => ok(&format!("Opened {name}"), layer),
                _ => fail(&format!("{name} could not be opened."), layer),
            }
        }
        "files.find" | "files.openfile" | "files.openfolder" | "files.reveal" => {
            let name = s("name");
            let within = files::location(&s("where"));
            let want = match tool {
                "files.openfolder" => Some(true),
                "files.openfile" => Some(false),
                _ => None,
            };
            let q = name.clone();
            let w = within.clone();
            let hits = blocking(move || files::search(&q, want, w.as_deref(), Duration::from_millis(3500))).await.unwrap_or_default();
            let ext = s("ext");
            let hits: Vec<files::Hit> = hits.into_iter().filter(|h| ext.is_empty() || h.path.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(&ext))).collect();
            let Some(top) = hits.first().cloned() else { return fail(&format!("I couldn't find {name}."), layer) };
            *core.last_paths.lock().unwrap() = hits.iter().map(|h| h.path.clone()).collect();
            if tool == "files.reveal" {
                return match crate::tools::channels::reveal(&top.path).await {
                    Ok(app) => ok(&format!("Showing {} in {app}", files::spoken_path(&top.path)), layer),
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            if tool == "files.find" {
                let more = if hits.len() > 1 { format!(" I also found {} more.", hits.len() - 1) } else { String::new() };
                return say(&format!("Found {}", files::spoken_path(&top.path)), layer, format!("I found {}.{more} Say open it to open it.", files::spoken_path(&top.path)));
            }
            let p = top.path.clone();
            let codeish = tool == "files.openfile" && code::lang(&p, "") != code::Lang::Other;
            if codeish
                && let Some((ide, _)) = native(core).await
                && crate::ide::open(ide, &p, false).await.is_ok()
            {
                return ok(&format!("Opened {} in {}", files::spoken_path(&top.path), ide.name), layer);
            }
            match blocking(move || crate::tools::shell::open(&p.to_string_lossy())).await {
                Some(Ok(())) => ok(&format!("Opened {}", files::spoken_path(&top.path)), layer),
                _ => fail("It could not be opened.", layer),
            }
        }
        "files.where" => match crate::tools::channels::open_folders().await {
            Ok(list) if list.is_empty() => say("No folder is open", layer, "No folder is open in the file manager.".into()),
            Ok(list) => {
                *core.last_paths.lock().unwrap() = list.clone();
                let names: Vec<String> = list.iter().map(|p| files::spoken_path(p)).collect();
                let line = if names.len() == 1 { format!("{} is open.", names[0]) } else { format!("These folders are open: {}.", names.join(", ")) };
                say(&line, layer, line.clone())
            }
            Err(e) => fail(&e.to_string(), layer),
        },
        "doc.cell" | "doc.export" => {
            if !core.settings.bool("permissions.control") {
                return control_needed(layer);
            }
            let r = if tool == "doc.cell" { crate::tools::channels::cell(&s("cell"), args.get("value").and_then(Value::as_str)).await } else { crate::tools::channels::export_pdf().await };
            match r {
                Ok(m) if tool == "doc.cell" && args.get("value").is_none() => say(&m, layer, format!("{m}.")),
                Ok(m) => ok(&m, layer),
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        "doc.mail" => {
            let send = args.get("send").and_then(Value::as_bool).unwrap_or(false);
            match crate::tools::channels::mail(&s("to"), &s("subject"), &s("body"), send).await {
                Ok(m) => {
                    let did = m.trim_end_matches('.').to_owned();
                    say(&did, layer, m)
                }
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        "system.lock" => match crate::tools::channels::lock().await {
            Ok(()) => ok("Locked the screen", layer),
            Err(e) => fail(&e.to_string(), layer),
        },
        "shell.extension" => match crate::tools::channels::extension(&s("op"), &s("name")).await {
            Ok(m) => say(&m, layer, format!("{m}.")),
            Err(e) => fail(&e.to_string(), layer),
        },
        "files.open_last" => {
            let paths = core.last_paths.lock().unwrap().clone();
            let k = args.get("n").and_then(Value::as_u64).unwrap_or(1).max(1) as usize - 1;
            let Some(p) = paths.get(k).cloned() else { return fail("I haven't found anything to open yet.", layer) };
            let shown = files::spoken_path(&p);
            match blocking(move || crate::tools::shell::open(&p.to_string_lossy())).await {
                Some(Ok(())) => ok(&format!("Opened {shown}"), layer),
                _ => fail("It could not be opened.", layer),
            }
        }
        "files.mkdir" => {
            let place = s("where");
            let parent = if place.is_empty() { project_dir(core).or_else(dirs::desktop_dir) } else { files::location(&place).or_else(|| (place == "desktop").then(dirs::desktop_dir).flatten()) };
            let Some(parent) = parent else { return fail(&format!("I couldn't find {place}."), layer) };
            let name = sentence_case(&s("name"));
            match files::create_folder(&parent, &name) {
                Ok(p) => ok(&format!("Created {}", files::spoken_path(&p)), layer),
                Err(e) => fail(&e.to_string(), layer),
            }
        }
        "files.delete" => {
            let p = PathBuf::from(s("path"));
            if p.as_os_str().is_empty() {
                return fail("Say which file to delete.", layer);
            }
            let shown = files::spoken_path(&p);
            match blocking(move || crate::tools::shell::trash(&p)).await {
                Some(Ok(())) => ok(&format!("Moved {shown} to the trash"), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("It could not be deleted.", layer),
            }
        }
        "files.copy" | "files.move" => {
            let src = PathBuf::from(s("path"));
            let Some(dest) = files::location(&s("to")) else { return fail(&format!("I couldn't find {}.", s("to")), layer) };
            if src.as_os_str().is_empty() || !src.exists() {
                return fail(&format!("I couldn't find {}.", s("name")), layer);
            }
            let mv = tool == "files.move";
            let d2 = dest.clone();
            match blocking(move || files::copy_into(&src, &d2, mv)).await {
                Some(Ok(p)) => ok(&format!("{} {}", if mv { "Moved" } else { "Copied" }, files::spoken_path(&p)), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The copy failed.", layer),
            }
        }
        "git.status" | "git.commit" | "git.push" | "git.pull" | "git.branch" | "git.last" | "git.commit_push" => {
            let dir = match repo(core) {
                Ok(d) => d,
                Err(e) => return fail(&e, layer),
            };
            let msg = s("message");
            let name = s("name");
            let create = args.get("create").and_then(Value::as_bool).unwrap_or(false);
            let t = tool.to_owned();
            let r = blocking(move || -> anyhow::Result<String> {
                match t.as_str() {
                    "git.status" => Ok(git::describe(&git::status(&dir)?)),
                    "git.commit" => git::commit(&dir, &msg),
                    "git.commit_push" => {
                        let a = git::commit(&dir, &msg)?;
                        let b = git::push(&dir)?;
                        Ok(format!("{a} {b}"))
                    }
                    "git.push" => git::push(&dir),
                    "git.pull" => git::pull(&dir),
                    "git.branch" => git::branch(&dir, &name, create),
                    _ => git::last_commit(&dir),
                }
            })
            .await;
            match r {
                Some(Ok(text)) => {
                    let did = text.trim_end_matches('.').to_owned();
                    if tool == "git.status" || tool == "git.last" { say(&did, layer, text) } else { ok(&did, layer) }
                }
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("Git stopped unexpectedly.", layer),
            }
        }
        "git.init" => {
            let Some(dir) = project_dir(core) else { return fail("Which folder? Open it in VS Code, or say: set project folder to, then the folder name.", layer) };
            match blocking(move || git::init(&dir)).await {
                Some(Ok(t)) => ok(t.trim_end_matches('.'), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("Git stopped unexpectedly.", layer),
            }
        }
        "git.login" => {
            if let Some(gh) = git::gh() {
                let r = blocking(move || open_terminal(&format!("\"{}\" auth login --web --git-protocol https", gh.display()))).await;
                if matches!(r, Some(Ok(()))) {
                    return say("Started GitHub sign in", layer, "A terminal opened for GitHub sign in. Press Enter there, then approve it in the browser.".into());
                }
            }
            let c = core.clone();
            match blocking(move || web::open(&c, None, "https://github.com/login")).await {
                Some(Ok(_)) => say("Opened GitHub sign in", layer, "I opened the GitHub sign in page. When you push, Git will ask to connect your account.".into()),
                _ => fail("The browser could not be opened.", layer),
            }
        }
        "text.dictate" => {
            if !core.settings.bool("permissions.control") {
                return control_needed("accessibility");
            }
            if crate::os::working().await.is_ok_and(|w| crate::ide::of_window(&w).is_some()) {
                return say("Coding mode on", "editor", "Coding mode is on. Speak your code. Say stop coding when you're done.".into());
            }
            say("Dictation on", "accessibility", "Dictation is on. I'll type what you say into the app in front. Say stop dictating when you're done.".into())
        }
        "code.dictate" => {
            if !open_vscode(core).await && crate::ide::preferred(core).is_none() {
                return fail("I can't find a coding app. Say install VS Code.", layer);
            }
            say("Coding mode on", layer, "Coding mode is on. Speak your code. Say stop coding when you're done.".into())
        }
        "code.check" => check_code(core, layer, args.get("next").and_then(Value::as_bool).unwrap_or(false), false).await,
        "code.failed" => say("Explained why it stopped", layer, s("reply")),
        "code.build" => build_native(core, layer).await,
        "code.run" => {
            let checked = check_code(core, layer, false, true).await;
            if !checked.ok || checked.reply.is_some() {
                return checked;
            }
            match native_active(core).await {
                Some((ide, win, a)) => run_native(core, layer, ide, &win, a).await,
                None => fail("Open the file you want to run in your editor first.", layer),
            }
        }
        "code.extension" => {
            let id = s("id");
            let label = s("label");
            if id.is_empty() {
                return fail("Say which extension to install.", layer);
            }
            let i2 = id.clone();
            match blocking(move || code::install_extension(&i2)).await {
                Some(Ok(())) => ok(&format!("Installed {label} in VS Code"), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The extension could not be installed.", layer),
            }
        }
        "code.uninstall" => {
            let name = s("name");
            let id = match code::known_extension(&name) {
                Some((id, _)) => id,
                None => match blocking({
                    let n = name.clone();
                    move || code::marketplace(&n)
                })
                .await
                .flatten()
                {
                    Some((id, _, _)) => id,
                    None => return fail(&format!("I couldn't find the {name} extension."), layer),
                },
            };
            match blocking(move || code::uninstall_extension(&id)).await {
                Some(Ok(())) => ok(&format!("Removed the {name} extension"), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The extension could not be removed.", layer),
            }
        }
        "code.setting" => {
            let key = s("key");
            let value = args.get("value").cloned().unwrap_or(Value::Null);
            let label = if s("label").is_empty() { format!("Set {key}") } else { s("label") };
            match blocking(move || code::write_setting(&key, &value)).await {
                Some(Ok(())) => ok(&label, layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The setting could not be changed.", layer),
            }
        }
        "code.settings" => {
            let q = s("query");
            if let Some((ide, win)) = native(core).await
                && crate::ide::search_in(core, ide, &win, "workbench.action.openSettings", "settings", &q).await.is_ok()
            {
                return ok(&format!("Showing settings for {q}"), layer);
            }
            if input::press(&[input::primary(), K::Ch(',')]).await.is_ok() {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let _ = input::type_text(&q).await;
                return ok(&format!("Showing settings for {q}"), layer);
            }
            fail("Your editor isn't open. Say open VS Code first.", layer)
        }
        "code.command" => {
            let id = s("id");
            let label = s("label");
            if !open_vscode(core).await {
                return fail("Your editor isn't open. Say open VS Code first.", layer);
            }
            let Some((ide, win)) = native(core).await else { return fail("Your editor isn't open. Say open VS Code first.", layer) };
            let query = args.get("args").and_then(|a| a.get("query")).and_then(Value::as_str).map(str::to_owned);
            let r = match &query {
                Some(q) => crate::ide::search_in(core, ide, &win, &id, "search", q).await,
                None => crate::ide::command(core, ide, &win, &id).await,
            };
            let done = if label.is_empty() { id.clone() } else { label.clone() };
            match r {
                Ok(()) => ok(&done, layer),
                Err(e) => match command_keys(&id) {
                    Some(keys) if crate::ide::bring_front(ide, &win).await => match input::press(&keys).await {
                        Ok(()) => ok(&done, "keyboard"),
                        Err(_) => fail(&e.to_string(), layer),
                    },
                    _ => fail(&e.to_string(), layer),
                },
            }
        }
        "code.goto" => {
            let line = args.get("line").and_then(Value::as_u64).unwrap_or(1);
            if let Some((ide, win, a)) = native_active(core).await
                && let Some(p) = a.path.clone()
            {
                return match crate::ide::goto(ide, Some(&win), &p, line as u32, 1).await {
                    Ok(()) => ok(&format!("Line {line}"), layer),
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            let _ = input::press(&[input::primary(), K::Ch('g')]).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
            let _ = input::type_text(&line.to_string()).await;
            let _ = input::press(&[K::Enter]).await;
            ok(&format!("Line {line}"), "keyboard")
        }
        "code.open" => {
            let name = s("name");
            let folder = args.get("folder").and_then(Value::as_bool).unwrap_or(false);
            let want = crate::ide::by_key(&s("app"));
            let target = if matches!(name.as_str(), "project" | "this project" | "my project" | "") {
                project_dir(core)
            } else {
                let n = name.clone();
                let here = project_dir(core);
                blocking(move || {
                    let pick = |hits: Vec<files::Hit>| -> Option<PathBuf> {
                        if folder || !n.contains('.') {
                            return hits.into_iter().next().map(|h| h.path);
                        }
                        hits.into_iter().find(|h| h.path.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&n))).map(|h| h.path)
                    };
                    here.as_deref().and_then(|h| pick(files::search(&n, Some(folder), Some(h), Duration::from_millis(1500)))).or_else(|| pick(files::search(&n, Some(folder), None, Duration::from_millis(3500))))
                })
                .await
                .flatten()
            };
            let Some(p) = target else { return fail(&format!("I couldn't find {name}."), layer) };
            if p.is_dir() {
                core.settings.set_internal("code.folder", json!(p.to_string_lossy()));
            }
            let p2 = p.clone();
            let chosen = match want {
                Some(i) => Some(i),
                None => native(core).await.map(|(i, _)| i).or_else(|| crate::ide::preferred(core)),
            };
            if let Some(i) = chosen.filter(|i| i.key != "vscode") {
                return match crate::ide::open(i, &p, p.is_dir()).await {
                    Ok(()) => {
                        crate::ide::remember(core, i);
                        ok(&format!("Opened {} in {}", files::spoken_path(&p2), i.name), layer)
                    }
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            match blocking(move || code::open_path(&p)).await {
                Some(Ok(())) => ok(&format!("Opened {} in VS Code", files::spoken_path(&p2)), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("VS Code could not be started.", layer),
            }
        }
        "code.folder" => {
            let name = s("name");
            let direct = files::location(&name);
            let n = name.clone();
            let p = match direct {
                Some(p) => Some(p),
                None => blocking(move || files::search(&n, Some(true), None, Duration::from_millis(3500)).into_iter().next().map(|h| h.path)).await.flatten(),
            };
            let Some(p) = p else { return fail(&format!("I couldn't find a folder called {name}."), layer) };
            core.settings.set_internal("code.folder", json!(p.to_string_lossy()));
            let mut save = serde_json::Map::new();
            save.insert("code.folder".into(), json!(p.to_string_lossy()));
            core.db.save_settings(save);
            core.hub.emit("skills.state", state(core));
            ok(&format!("Project folder is {}", files::spoken_path(&p)), layer)
        }
        "wm.place" | "wm.side" | "wm.focus" | "wm.desktop" | "wm.switch" | "wm.close" | "wm.workspace" | "wm.list" | "wm.frame" => window(core, tool, &args).await,
        "code.project" => new_project(core, &s("name"), &s("language")).await,
        "code.start" => open_editor(core).await,
        "code.compose" => compose_code(core, &s("code")).await,
        "code.new_file" => new_file(core, &s("name"), &s("folder")).await,
        "code.write" => write_code(core, &s("request")).await,
        "code.comment" => {
            if let Some((ide, win, a)) = native_active(core).await {
                let text = codespeech::comment(&s("text"), &a.language);
                return match crate::ide::insert_at(ide, &win, &a, &text).await {
                    Ok(how) => ok(&format!("Added the comment {}", text.trim()), if how == "keyboard" { "keyboard" } else { layer }),
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            let text = codespeech::comment(&s("text"), "");
            match input::type_text(&text).await {
                Ok(()) => ok(&format!("Added the comment {}", text.trim()), "keyboard"),
                Err(e) => fail(&e.to_string(), "keyboard"),
            }
        }
        "code.insert" => {
            let mut text = s("text");
            if let Some((ide, win, a)) = native_active(core).await {
                if args.get("layout").and_then(Value::as_bool).unwrap_or(false) && core.settings.bool("code.layout") {
                    text = codespeech::layout(&text, &a.language);
                }
                return match crate::ide::insert_at(ide, &win, &a, &text).await {
                    Ok(how) => ok(&format!("Typed {}", text.replace('\n', " ")), if how == "keyboard" { "keyboard" } else { layer }),
                    Err(e) => fail(&e.to_string(), layer),
                };
            }
            if args.get("layout").and_then(Value::as_bool).unwrap_or(false) && core.settings.bool("code.layout") {
                text = codespeech::layout(&text, "");
            }
            match input::type_text(&text).await {
                Ok(()) => ok(&format!("Typed {}", text.replace('\n', " ")), "keyboard"),
                Err(e) => fail(&e.to_string(), "keyboard"),
            }
        }
        "code.edit" => {
            let op = s("op");
            let n = args.get("n").and_then(Value::as_u64).unwrap_or(1);
            if let Some((ide, win)) = native(core).await {
                if let Some(id) = edit_command(&op)
                    && n <= 1
                    && crate::ide::command(core, ide, &win, id).await.is_ok()
                {
                    return ok(&sentence_case(&op.replace('_', " ")), layer);
                }
                if op == "select_line"
                    && let Some(line) = args.get("n").and_then(Value::as_u64)
                    && let Some(p) = crate::ide::active(core, ide, &win).await.path
                {
                    let _ = crate::ide::goto(ide, Some(&win), &p, line as u32, 1).await;
                }
                let _ = crate::ide::bring_front(ide, &win).await;
            }
            let p = input::primary();
            let keys: Vec<K> = match op.as_str() {
                "undo" => vec![p, K::Ch('z')],
                "redo" => vec![p, K::Shift, K::Ch('z')],
                "delete_line" => vec![p, K::Shift, K::Ch('k')],
                "delete_word" => vec![p, K::Back],
                "backspace" => vec![K::Back],
                "save" => vec![p, K::Ch('s')],
                "format" => vec![K::Shift, K::Alt, K::Ch('f')],
                "comment" => vec![p, K::Ch('/')],
                "indent" => vec![K::Tab],
                "outdent" => vec![K::Shift, K::Tab],
                "line_end" => vec![K::End],
                "line_start" => vec![K::Home],
                "copy_line" => vec![K::Shift, K::Alt, K::Down],
                "paste" => vec![p, K::Ch('v')],
                "select_all" => vec![p, K::Ch('a')],
                "new_file" => vec![p, K::Ch('n')],
                "terminal" => vec![p, K::Ch('`')],
                "escape" => vec![K::Esc],
                "enter" => vec![K::Enter],
                "select_line" => vec![p, K::Ch('l')],
                "move" => vec![match s("dir").as_str() {
                    "up" => K::Up,
                    "down" => K::Down,
                    "left" => K::Left,
                    _ => K::Right,
                }],
                _ => return fail("I can't do that edit without the VS Code helper.", layer),
            };
            let times = if matches!(op.as_str(), "backspace" | "move") { n as usize } else { 1 };
            match input::press_times(&keys, times).await {
                Ok(()) => ok(&sentence_case(&op.replace('_', " ")), "keyboard"),
                Err(e) => fail(&e.to_string(), "keyboard"),
            }
        }
        "terminal.run" => {
            let cmd = s("command");
            if cmd.is_empty() {
                return fail("Say the command to run.", layer);
            }
            if let Some((ide, win)) = native(core).await
                && ide.family == crate::ide::Family::VsCode
                && (win.front || crate::os::last_other().is_some_and(|l| l.id == win.id || (l.pid == win.pid && l.pid != 0)))
            {
                match crate::ide::terminal(ide, &win, std::slice::from_ref(&cmd), project_dir(core).as_deref()).await {
                    Ok(_) => return ok(&format!("Ran {cmd}"), layer),
                    Err(e) => log::info!("{} terminal: {e}", ide.name),
                }
            }
            let dir = project_dir(core);
            let c2 = cmd.clone();
            match blocking(move || open_terminal_in(&c2, dir.as_deref())).await {
                Some(Ok(())) => ok(&format!("Ran {cmd}"), layer),
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("A terminal could not be opened.", layer),
            }
        }
        "web.download" => {
            let url = s("url");
            let c = cancel.clone();
            match blocking(move || download(&url, &c)).await {
                Some(Ok(p)) => {
                    let shown = files::spoken_path(&p);
                    ok(&format!("Downloaded {shown}"), layer)
                }
                Some(Err(e)) => fail(&e.to_string(), layer),
                None => fail("The download stopped.", layer),
            }
        }
        "web.download_current" => {
            let screen = crate::tools::atspi::snapshot(250).await.ok();
            let url = screen.as_ref().and_then(|sc| sc.elements.iter().find(|e| e.role == "field" && (e.value.starts_with("http") || e.value.contains('.') && !e.value.contains(' '))).map(|e| e.value.clone()));
            match url {
                Some(u) if looks_like_file(&u) => {
                    let full = if u.starts_with("http") { u } else { format!("https://{u}") };
                    let c = cancel.clone();
                    match blocking(move || download(&full, &c)).await {
                        Some(Ok(p)) => ok(&format!("Downloaded {}", files::spoken_path(&p)), layer),
                        Some(Err(e)) => fail(&e.to_string(), layer),
                        None => fail("The download stopped.", layer),
                    }
                }
                _ => match input::press(&[input::primary(), K::Ch('s')]).await {
                    Ok(()) => say("Opened Save", "keyboard", "I opened the save window. Say press enter to save it to Downloads.".into()),
                    Err(e) => fail(&e.to_string(), "keyboard"),
                },
            }
        }
        "files.create" => files_create(core, &args).await,
        "app.write" => crate::writer::write(core, &s("app"), args.get("text").and_then(Value::as_str).unwrap_or_default()).await,
        "page.vertical" => crate::understand::run_vertical(core, &s("kind")).await,
        "task" => crate::agent::run(core, &s("goal"), args.get("app").and_then(Value::as_str).map(str::to_owned), cancel).await,
        other => fail(&format!("I can't do {other} yet."), layer),
    }
}

async fn window(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let layer = "keyboard";
    if !core.settings.bool("permissions.control") {
        return control_needed(layer);
    }
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let find = |name: &str| {
        let defaults = core.defaults();
        core.apps.find(name, None, &defaults).into_iter().next().map(|(a, _)| a)
    };
    let focus = |name: String| {
        let app = find(&name);
        let label = app.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| sentence_case(&name));
        async move {
            let r = wm::focus(&label, app.clone()).await;
            match r {
                Ok(true) => Ok(label),
                Ok(false) => match app {
                    Some(a) => {
                        let a2 = a.clone();
                        match blocking(move || crate::tools::apps::launch(&a2)).await {
                            Some(Ok(_)) => {
                                tokio::time::sleep(Duration::from_millis(1400)).await;
                                Ok(label)
                            }
                            _ => Err(format!("I couldn't open {label}.")),
                        }
                    }
                    None => Err(format!("I couldn't find a {label} window.")),
                },
                Err(e) => Err(e.to_string()),
            }
        }
    };
    let res: Result<String, String> = match tool {
        "wm.list" => {
            let wins: Vec<crate::os::Win> = match crate::os::windows().await {
                Ok(w) => w.into_iter().filter(|w| !crate::os::is_setu_window(w) && !(w.app.trim().is_empty() && w.title.trim().is_empty())).collect(),
                Err(e) => return fail(&e.to_string(), "accessibility"),
            };
            return say("Listed the open windows", "accessibility", open_windows_line(&wins));
        }
        "wm.frame" => {
            let req = match s("op").as_str() {
                "corner" => wm::Frame::Corner(s("where")),
                "nudge" => {
                    let n = wm::NUDGE;
                    match s("dir").as_str() {
                        "up" => wm::Frame::Nudge { dx: 0, dy: -n },
                        "down" => wm::Frame::Nudge { dx: 0, dy: n },
                        "left" => wm::Frame::Nudge { dx: -n, dy: 0 },
                        _ => wm::Frame::Nudge { dx: n, dy: 0 },
                    }
                }
                "size" => wm::Frame::Size { w: args.get("w").and_then(Value::as_i64).unwrap_or(800) as i32, h: args.get("h").and_then(Value::as_i64).unwrap_or(600) as i32 },
                _ => wm::Frame::Scale { w: args.get("w").and_then(Value::as_f64).unwrap_or(1.0) as f32, h: args.get("h").and_then(Value::as_f64).unwrap_or(1.0) as f32 },
            };
            wm::frame(req, crate::os::working().await.ok()).await.map_err(|e| e.to_string())
        }
        "wm.desktop" => wm::desktop().await.map(|_| "Showing the desktop".to_owned()).map_err(|e| e.to_string()),
        "wm.switch" => wm::switch().await.map(|_| "Switched windows".to_owned()).map_err(|e| e.to_string()),
        "wm.close" => wm::close().await.map(|_| "Closed the window".to_owned()).map_err(|e| e.to_string()),
        "wm.focus" => focus(s("app")).await.map(|l| format!("Switched to {l}")),
        "wm.workspace" => {
            let n = args.get("n").and_then(Value::as_u64).map(|n| n as u32);
            let dir = args.get("dir").and_then(Value::as_i64).unwrap_or(1) as i32;
            wm::workspace(n, dir, args.get("move").and_then(Value::as_bool).unwrap_or(false)).await.map_err(|e| e.to_string())
        }
        "wm.place" => {
            let p = wm::Place::parse(&s("where")).unwrap_or(wm::Place::Max);
            let app = s("app");
            let pre = if app.is_empty() { Ok(String::new()) } else { focus(app).await };
            match pre {
                Err(e) => Err(e),
                Ok(label) => {
                    if !label.is_empty() {
                        tokio::time::sleep(Duration::from_millis(450)).await;
                    }
                    wm::place(p).await.map(|_| if label.is_empty() { p.done().to_owned() } else { p.done().replace("the window", &label) }).map_err(|e| e.to_string())
                }
            }
        }
        "wm.side" => {
            let (a, b) = (s("left"), s("right"));
            if a.is_empty() || b.is_empty() {
                let r = async {
                    wm::place(wm::Place::Left).await?;
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    wm::switch().await?;
                    tokio::time::sleep(Duration::from_millis(450)).await;
                    wm::place(wm::Place::Right).await
                }
                .await;
                r.map(|_| "Put your last two windows side by side".to_owned()).map_err(|e| e.to_string())
            } else {
                async {
                    let la = focus(a).await?;
                    tokio::time::sleep(Duration::from_millis(450)).await;
                    wm::place(wm::Place::Left).await.map_err(|e| e.to_string())?;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let lb = focus(b).await?;
                    tokio::time::sleep(Duration::from_millis(450)).await;
                    wm::place(wm::Place::Right).await.map_err(|e| e.to_string())?;
                    Ok::<String, String>(format!("Put {la} and {lb} side by side"))
                }
                .await
            }
        }
        _ => Err("I can't do that with windows yet.".to_owned()),
    };
    match res {
        Ok(did) => ok(&did, layer),
        Err(e) => fail(&e, layer),
    }
}

pub fn open_windows_line(wins: &[crate::os::Win]) -> String {
    if wins.is_empty() {
        return "No windows are open.".into();
    }
    let mut apps: Vec<(String, usize, String)> = Vec::new();
    for w in wins {
        let name = if w.app.trim().is_empty() { w.title.trim().to_owned() } else { sentence_case(w.app.trim()) };
        match apps.iter_mut().find(|(n, _, _)| n.eq_ignore_ascii_case(&name)) {
            Some(a) => a.1 += 1,
            None => apps.push((name, 1, w.title.trim().to_owned())),
        }
    }
    let parts: Vec<String> = apps
        .iter()
        .take(8)
        .map(|(n, c, title)| {
            let short: String = title.chars().take(40).collect();
            if *c > 1 {
                format!("{c} in {n}")
            } else if !short.is_empty() && !short.to_lowercase().contains(&n.to_lowercase()) && !n.to_lowercase().contains(&short.to_lowercase()) {
                format!("{n} with {short}")
            } else {
                n.clone()
            }
        })
        .collect();
    let more = if apps.len() > 8 { format!(", and {} more apps", apps.len() - 8) } else { String::new() };
    let n = wins.len();
    format!("{n} {} open: {}{more}.", if n == 1 { "window is" } else { "windows are" }, join_names(&parts))
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(48).collect()
}

async fn open_editor(core: &Arc<Core>) -> Outcome {
    let layer = "editor";
    if let Some((i, w)) = native(core).await {
        let _ = crate::ide::bring_front(i, &w).await;
        return say(&format!("{} is open", i.name), layer, format!("{} is open. Say create a project, create a file, or start coding.", i.name));
    }
    let Some(ide) = crate::ide::preferred(core) else { return fail("I can't find a coding app. Say install VS Code.", layer) };
    let folder = core.settings.str("code.folder");
    let dir = PathBuf::from(&folder);
    let opened = if !folder.is_empty() && dir.is_dir() { open_in(core, &dir, None, false).await.is_some() } else { open_vscode(core).await };
    if !opened {
        return fail(&format!("{} could not be started.", ide.name), layer);
    }
    let where_ = dir.file_name().filter(|_| !folder.is_empty()).map(|n| format!(" with {}", n.to_string_lossy())).unwrap_or_default();
    say(&format!("Opened {}{where_}", ide.name), layer, format!("{} is open{where_}. Say create a project, create a file, or start coding.", ide.name))
}

async fn workspace(core: &Arc<Core>) -> Vec<std::path::PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for w in crate::os::windows().await.unwrap_or_default() {
        let Some(ide) = crate::ide::of_window(&w) else { continue };
        let t = crate::ide::parse_title(ide, &w.title);
        if t.project.is_empty() {
            continue;
        }
        if let Some(r) = crate::ide::roots(core, ide, &t.project).into_iter().next()
            && !out.contains(&r)
        {
            out.push(r);
        }
    }
    out
}

async fn resolve_folder(core: &Arc<Core>, wanted: &str) -> Option<std::path::PathBuf> {
    let open = workspace(core).await;
    let last = Some(core.settings.str("code.folder")).filter(|f| !f.is_empty()).map(std::path::PathBuf::from).filter(|p| p.is_dir());
    if wanted.trim().is_empty() {
        return open.into_iter().next().or(last);
    }
    let want = slug(wanted);
    let named = |p: &std::path::Path| p.file_name().map(|n| slug(&n.to_string_lossy()) == want).unwrap_or(false);
    if let Some(p) = open.iter().find(|p| named(p)) {
        return Some(p.clone());
    }
    if let Some(p) = last.as_ref().filter(|p| named(p)) {
        return Some(p.clone());
    }
    for base in open.iter().chain(last.iter()) {
        let child = base.join(wanted.trim());
        if child.is_dir() {
            return Some(child);
        }
    }
    let projects = dirs::home_dir()?.join("Projects").join(&want);
    if projects.is_dir() {
        return Some(projects);
    }
    None
}

async fn new_file(core: &Arc<Core>, name: &str, folder: &str) -> Outcome {
    let layer = "editor";
    let Some(ide) = crate::ide::preferred(core) else { return fail("I can't find a coding app. Say install VS Code.", layer) };
    let name = name.trim().to_owned();
    if name.is_empty() || name == "." {
        return fail("What should I call the file? Say, for example, create a file called main dot py.", layer);
    }
    let mut created_dir = false;
    let base = match resolve_folder(core, folder).await {
        Some(b) => b,
        None if !folder.trim().is_empty() => {
            let Some(home) = dirs::home_dir() else { return fail("I couldn't find your home folder.", layer) };
            let d = home.join("Projects").join(slug(folder));
            created_dir = !d.exists();
            if let Err(e) = std::fs::create_dir_all(&d) {
                return fail(&format!("I couldn't create the folder. {e}"), layer);
            }
            d
        }
        None => return fail("Open or create a project first, then ask again.", layer),
    };
    let path = base.join(&name);
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return fail(&format!("I couldn't create the folder for the file. {e}"), layer);
    }
    let existed = path.exists();
    if !existed && let Err(e) = std::fs::write(&path, "") {
        return fail(&format!("I couldn't create {name}. {e}"), layer);
    }
    core.settings.set_internal("code.folder", json!(base.to_string_lossy()));
    let mut save = serde_json::Map::new();
    save.insert("code.folder".into(), json!(base.to_string_lossy()));
    core.db.save_settings(save);
    let in_workspace = workspace(core).await.iter().any(|w| path.starts_with(w));
    let opened = if in_workspace && let Some((i, _)) = native(core).await {
        crate::ide::open(i, &path, false).await.is_ok()
    } else if ide.key == "vscode" {
        let created = !existed && created_dir;
        if created {
            crate::ide::prepare_open(ide, &base);
        }
        let (b, f) = (base.clone(), path.clone());
        let ok = matches!(blocking(move || code::reuse(&[&b.to_string_lossy(), &f.to_string_lossy()])).await, Some(Ok(_)));
        if ok {
            let (c2, d2) = (core.clone(), base.clone());
            tokio::spawn(async move {
                crate::ide::after_open(&c2, ide, &d2, created).await;
            });
        }
        ok
    } else {
        open_in(core, &base, Some(&path), !existed && created_dir).await.is_some()
    };
    let shown = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !opened {
        return fail(&format!("I created {name} in {shown}, but {} didn't open it.", ide.name), layer);
    }
    let what = if existed { format!("{name} already existed in {shown}, so I opened it.") } else { format!("Created {name} in {shown} and opened it.") };
    say(&format!("Created {name}"), layer, format!("{what} Say start coding when you're ready."))
}

async fn new_project(core: &Arc<Core>, name: &str, language: &str) -> Outcome {
    let layer = "editor";
    if crate::ide::preferred(core).is_none() {
        return fail("I can't find a coding app. Say install VS Code.", layer);
    }
    let Some(home) = dirs::home_dir() else { return fail("I couldn't find your home folder.", layer) };
    let root = home.join("Projects");
    let base = if name.trim().is_empty() { "new-project".to_owned() } else { slug(name) };
    let base = if base.is_empty() { "new-project".to_owned() } else { base };
    let mut dir = root.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = root.join(format!("{base}-{n}"));
        n += 1;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(&format!("I couldn't create the project folder. {e}"), layer);
    }
    let file = code::starter(language).map(|(f, body)| {
        let p = dir.join(f);
        let _ = std::fs::write(&p, body);
        p
    });
    core.settings.set_internal("code.folder", json!(dir.to_string_lossy()));
    let mut save = serde_json::Map::new();
    save.insert("code.folder".into(), json!(dir.to_string_lossy()));
    core.db.save_settings(save);
    core.hub.emit("skills.state", state(core));
    match open_in(core, &dir, file.as_deref(), true).await {
        Some(app) => {
            let shown = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(base);
            let what = file.as_ref().and_then(|f| f.file_name()).map(|f| format!(" with {}", f.to_string_lossy())).unwrap_or_default();
            say(&format!("Created the project {shown}"), layer, format!("Created the project {shown}{what} and opened it in {app}. Say start coding when you're ready."))
        }
        None => fail("Your coding app could not be started.", layer),
    }
}

fn folder_candidates(core: &Arc<Core>, open: &[PathBuf]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = open.to_vec();
    let saved = core.settings.str("code.folder");
    if !saved.is_empty() {
        v.push(PathBuf::from(saved));
    }
    if let Some(h) = dirs::home_dir() {
        v.push(h.join("Projects"));
    }
    v.extend([dirs::desktop_dir(), dirs::document_dir(), dirs::home_dir(), dirs::download_dir()].into_iter().flatten());
    v
}

async fn find_folder(core: &Arc<Core>, name: &str) -> Option<PathBuf> {
    let want = slug(name);
    if want.is_empty() {
        return None;
    }
    let open = workspace(core).await;
    let named = |p: &Path| p.file_name().is_some_and(|n| slug(&n.to_string_lossy()) == want);
    for base in folder_candidates(core, &open) {
        if base.is_dir() && named(&base) {
            return Some(base);
        }
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && named(&p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn projects_root() -> Option<PathBuf> {
    let r = dirs::home_dir()?.join("Projects");
    std::fs::create_dir_all(&r).ok()?;
    Some(r)
}

fn clean_part(name: &str) -> String {
    name.trim().chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect::<String>().trim().trim_matches('.').trim().to_owned()
}

async fn files_create(core: &Arc<Core>, args: &Value) -> Outcome {
    let layer = "files";
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let items: Vec<(bool, Vec<String>)> = args
        .get("items")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|i| (i.get("kind").and_then(Value::as_str) == Some("folder"), i.get("path").and_then(Value::as_array).map(|p| p.iter().filter_map(Value::as_str).map(clean_part).collect()).unwrap_or_default()))
                .filter(|(_, p): &(bool, Vec<String>)| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if items.is_empty() {
        return fail("What should I create? Say, for example, create a folder called tutorial.", layer);
    }
    if items.iter().any(|(folder, p)| !folder && p.last().is_none_or(|n| n.is_empty())) || items.iter().any(|(_, p)| p.iter().any(String::is_empty)) {
        return fail("What should I call it? Say, for example, create a file called notes dot txt.", layer);
    }
    let code = args.get("code").and_then(Value::as_bool).unwrap_or(false) || items.iter().any(|(f, p)| !f && p.last().is_some_and(|n| crate::understand::is_code_file(n)));
    let (place, inside, language) = (s("where"), s("inside"), s("language"));
    let first_folder = items[0].0;
    let root_name = items[0].1[0].clone();
    let last_dir = core.last_paths.lock().unwrap().iter().find(|p| p.is_dir()).cloned();
    let desktop = || dirs::desktop_dir().or_else(|| dirs::home_dir().map(|h| h.join("Desktop"))).filter(|d| std::fs::create_dir_all(d).is_ok());
    let default_parent = |code: bool| if code { projects_root() } else { desktop().or_else(dirs::home_dir) };
    let mut prefix: Vec<String> = Vec::new();
    let base: PathBuf = if inside == "it" {
        match last_dir.or_else(|| project_dir(core)) {
            Some(d) => d,
            None => return fail("Which folder should it go in? Say the folder's name.", layer),
        }
    } else if !inside.is_empty() {
        match find_folder(core, &inside).await {
            Some(d) => d,
            None => {
                prefix.push(clean_part(&inside));
                match default_parent(code) {
                    Some(d) => d,
                    None => return fail("I couldn't find your home folder.", layer),
                }
            }
        }
    } else if !place.is_empty() {
        let p = files::location(&place).or_else(|| match place.as_str() {
            "desktop" => desktop(),
            "projects" => projects_root(),
            "home" => dirs::home_dir(),
            _ => None,
        });
        match p {
            Some(p) => p,
            None => return fail(&format!("I couldn't find {place}."), layer),
        }
    } else if first_folder {
        match find_folder(core, &root_name).await.and_then(|d| d.parent().map(Path::to_path_buf)) {
            Some(parent) => parent,
            None => match default_parent(code) {
                Some(d) => d,
                None => return fail("I couldn't find your home folder.", layer),
            },
        }
    } else if code {
        match project_dir(core).or_else(projects_root) {
            Some(d) => d,
            None => return fail("I couldn't find your home folder.", layer),
        }
    } else {
        match default_parent(false) {
            Some(d) => d,
            None => return fail("I couldn't find your home folder.", layer),
        }
    };
    let mut made_files: Vec<PathBuf> = Vec::new();
    let mut existed: Vec<String> = Vec::new();
    let mut deepest: Option<PathBuf> = None;
    for (folder, parts) in &items {
        let mut p = base.clone();
        for x in prefix.iter().chain(parts.iter()) {
            p.push(x);
        }
        if *folder {
            if p.is_dir() {
                existed.push(parts.last().cloned().unwrap_or_default());
            } else if let Err(e) = std::fs::create_dir_all(&p) {
                return fail(&format!("I couldn't create {}. {e}", parts.last().cloned().unwrap_or_default()), layer);
            }
            if deepest.as_ref().is_none_or(|d| p.components().count() > d.components().count()) {
                deepest = Some(p.clone());
            }
        } else {
            if let Some(parent) = p.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                return fail(&format!("I couldn't create the folder for {}. {e}", parts.last().cloned().unwrap_or_default()), layer);
            }
            if p.exists() {
                existed.push(parts.last().cloned().unwrap_or_default());
            } else if let Err(e) = std::fs::write(&p, "") {
                return fail(&format!("I couldn't create {}. {e}", parts.last().cloned().unwrap_or_default()), layer);
            }
            made_files.push(p);
        }
    }
    if code
        && made_files.is_empty()
        && let Some(d) = deepest.clone()
        && let Some((f, body)) = code::starter(&language)
    {
        let p = d.join(f);
        if !p.exists() {
            let _ = std::fs::write(&p, body);
        }
        made_files.push(p);
    }
    let top = if first_folder || !prefix.is_empty() { base.join(prefix.first().cloned().unwrap_or_else(|| root_name.clone())) } else { base.clone() };
    let mut remember = vec![deepest.clone().unwrap_or_else(|| top.clone())];
    remember.extend(made_files.iter().cloned());
    *core.last_paths.lock().unwrap() = remember;
    let what = {
        let folders: Vec<String> = items.iter().filter(|(f, _)| *f).map(|(_, p)| p.last().cloned().unwrap_or_default()).collect();
        let names: Vec<String> = made_files.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        match (folders.is_empty(), names.is_empty()) {
            (false, false) => format!("the folder {} with {}", join_names(&folders), join_names(&names)),
            (false, true) => format!("the folder {}", join_names(&folders)),
            _ => join_names(&names),
        }
    };
    let place_shown = files::spoken_path(&top).split_once(" in ").map(|(_, p)| p.to_owned()).unwrap_or_else(|| files::spoken_path(&base));
    let verb = if existed.len() == items.len() { "Opened" } else { "Created" };
    if code && crate::ide::preferred(core).is_some() {
        core.settings.set_internal("code.folder", json!(top.to_string_lossy()));
        let mut save = serde_json::Map::new();
        save.insert("code.folder".into(), json!(top.to_string_lossy()));
        core.db.save_settings(save);
        core.hub.emit("skills.state", state(core));
        let file = made_files.last().cloned();
        let in_workspace = workspace(core).await.iter().any(|w| top.starts_with(w));
        let opened = match (&file, in_workspace) {
            (Some(f), true) => match native(core).await {
                Some((i, _)) if crate::ide::open(i, f, false).await.is_ok() => Some(i.name),
                _ => open_in(core, &top, file.as_deref(), existed.len() < items.len()).await,
            },
            _ => open_in(core, &top, file.as_deref(), existed.len() < items.len()).await,
        };
        let tail = match opened {
            Some(app) => format!(", and opened it in {app}"),
            None => ". Your coding app didn't open, so say open VS Code".to_owned(),
        };
        return say(&format!("{verb} {what}"), "editor", format!("{verb} {what} in {place_shown}{tail}."));
    }
    say(&format!("{verb} {what}"), layer, format!("{verb} {what} in {place_shown}."))
}

fn strip_fences(code: &str) -> String {
    let t = code.trim();
    let t = t.strip_prefix("```").map(|r| r.split_once('\n').map(|(_, b)| b).unwrap_or(r)).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim_end().to_owned()
}

async fn editor_file(core: &Arc<Core>) -> (Option<(&'static crate::ide::Ide, crate::os::Win, crate::ide::Active)>, Option<(String, String, Option<String>)>) {
    let nat = native_active(core).await;
    let info = nat.as_ref().and_then(|(_, _, a)| a.path.as_ref().map(|p| (p.to_string_lossy().into_owned(), a.language.clone(), a.text.clone())));
    (nat, info)
}

async fn put_block(nat: &Option<(&'static crate::ide::Ide, crate::os::Win, crate::ide::Active)>, code_text: &str) -> Result<(), String> {
    let Some((ide, win, a)) = nat else { return Err("Open a file in your editor first, then ask again.".into()) };
    let base = a.text.as_deref().map(|t| crate::ide::line_indent(t, a.line)).unwrap_or_default();
    crate::ide::insert_at(ide, win, a, &crate::ide::indent_block(code_text, &base)).await.map(|_| ()).map_err(|e| e.to_string())
}

async fn write_code(core: &Arc<Core>, request: &str) -> Outcome {
    let layer = "editor";
    if !open_vscode(core).await {
        return fail("Open your editor first, then ask again.", layer);
    }
    let (nat, info) = editor_file(core).await;
    let Some((path, lang, text)) = info else { return fail("Open a file in your editor first, then ask again.", layer) };
    let language = if lang.is_empty() { code::lang_name(code::lang(Path::new(&path), "")).to_owned() } else { lang };
    let context: String = text.unwrap_or_default().chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect();
    let system = format!(
        "You write {language} code for a programmer who is speaking to their editor. Return only the code, with correct indentation and line breaks, and no explanation. Use short clear names. Add a comment only when the request asks for one. Answer as JSON: {{\"code\": \"...\"}}."
    );
    let user = format!("File so far (end):\n{context}\n\nRequest: {request}");
    let v = match crate::pipeline::ask_json(core, &system, &user, Some(crate::pipeline::string_grammar("code")), 700, 0.2).await {
        Ok(v) => v,
        Err(e) => return fail(&format!("I couldn't write that. {e}"), layer),
    };
    let code_text = strip_fences(v.get("code").and_then(Value::as_str).unwrap_or_default());
    if code_text.trim().is_empty() {
        return fail("I couldn't write that. Try saying it another way.", layer);
    }
    let lines = code_text.lines().count();
    match put_block(&nat, &code_text).await {
        Ok(()) => say(&format!("Wrote {lines} line{}", if lines == 1 { "" } else { "s" }), layer, format!("Done. I wrote {lines} line{}.", if lines == 1 { "" } else { "s" })),
        Err(e) => fail(&e, layer),
    }
}

fn looks_formatted(code: &str) -> bool {
    let lines: Vec<&str> = code.lines().filter(|l| !l.trim().is_empty()).collect();
    lines.len() >= 2 && lines.iter().filter(|l| l.trim_end().ends_with([';', '{', '}', ':', ')', ','])).count() * 2 >= lines.len()
}

async fn compose_code(core: &Arc<Core>, typed: &str) -> Outcome {
    let layer = "editor";
    if !open_vscode(core).await {
        return fail("Open your editor first, then ask again.", layer);
    }
    let (nat, info) = editor_file(core).await;
    let Some((path, lang, text)) = info else { return fail("Open a file in your editor first. Say create a file called main dot cpp.", layer) };
    let language = if lang.is_empty() { code::lang_name(code::lang(Path::new(&path), "")).to_owned() } else { lang };
    let code_text = if looks_formatted(typed) {
        typed.to_owned()
    } else {
        let context: String = text.unwrap_or_default().chars().rev().take(800).collect::<Vec<_>>().into_iter().rev().collect();
        let system = format!(
            "You turn code that a programmer typed or dictated in one line into clean {language}. Keep exactly the same statements in the same order. Put each statement on its own line, indent blocks correctly, and add only punctuation that is clearly missing, such as semicolons, parentheses, braces, colons and quotes. Never add comments, new statements or explanations. Answer as JSON: {{\"code\": \"...\"}}."
        );
        let user = format!("File so far (end):\n{context}\n\nTyped: {typed}");
        match crate::pipeline::ask_json(core, &system, &user, Some(crate::pipeline::string_grammar("code")), 500, 0.0).await {
            Ok(v) => {
                let c = strip_fences(v.get("code").and_then(Value::as_str).unwrap_or_default());
                if c.trim().is_empty() { codespeech::layout(typed, &language) } else { c }
            }
            Err(_) => codespeech::layout(typed, &language),
        }
    };
    let lines = code_text.lines().filter(|l| !l.trim().is_empty()).count();
    match put_block(&nat, &code_text).await {
        Ok(_) => say(&format!("Wrote {lines} line{}", if lines == 1 { "" } else { "s" }), layer, format!("Added {lines} line{} to {}.", if lines == 1 { "" } else { "s" }, Path::new(&path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())),
        Err(e) => fail(&e, layer),
    }
}

async fn find_in_windows(name: &str, roles: &[&str]) -> anyhow::Result<(crate::os::Win, crate::os::Node)> {
    let wins = crate::os::ui_targets(false).await?;
    for w in &wins {
        crate::os::wake(w).await;
        if let Ok(nodes) = crate::os::tree(w, crate::os::Scope { limit: 600, text: false, hidden: false }).await
            && let Some(n) = crate::os::find(&nodes, name, roles)
        {
            return Ok((w.clone(), n.clone()));
        }
    }
    Err(anyhow::anyhow!("I couldn't find {name} on the screen."))
}

async fn tick(name: &str, on: bool) -> anyhow::Result<(String, String)> {
    let (w, n) = find_in_windows(name, &["checkbox", "toggle", "radio", "menuitem"]).await?;
    let label = if n.name.is_empty() { name.to_owned() } else { n.name.clone() };
    if n.checked == Some(on) {
        return Ok((format!("{label} is already {}", if on { "ticked" } else { "unticked" }), w.app));
    }
    crate::os::act(&n, crate::os::Do::Toggle).await?;
    Ok((format!("{} {label}", if on { "Ticked" } else { "Unticked" }), w.app))
}

async fn choose(option: &str, list: &str) -> anyhow::Result<(String, String)> {
    let (w, combo) = find_in_windows(list, &["combobox"]).await?;
    let items = ["menuitem", "item", "row", "radio", "other"];
    let pick = |nodes: &[crate::os::Node]| crate::os::find(nodes, option, &items).filter(|n| n.id != combo.id).cloned();
    if crate::os::act(&combo, crate::os::Do::Expand).await.is_err() {
        crate::os::act(&combo, crate::os::Do::Invoke).await?;
    }
    for _ in 0..6 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut found = None;
        if let Ok(groups) = crate::os::popups(&w).await {
            found = groups.iter().find_map(|(_, nodes)| pick(nodes));
        }
        if found.is_none()
            && let Ok(nodes) = crate::os::tree(&w, crate::os::Scope { limit: 1500, text: false, hidden: true }).await
        {
            found = pick(&nodes);
        }
        if let Some(item) = found {
            if crate::os::act(&item, crate::os::Do::Invoke).await.is_err() {
                crate::os::act(&item, crate::os::Do::Select).await?;
            }
            let label = if item.name.is_empty() { option.to_owned() } else { item.name.clone() };
            return Ok((format!("Chose {label}"), w.app));
        }
    }
    let _ = input::press(&[K::Esc]).await;
    Err(anyhow::anyhow!("I opened {} but couldn't find {option} in it.", if combo.name.is_empty() { list } else { combo.name.as_str() }))
}

fn control_needed(layer: &'static str) -> Outcome {
    let mut o = Outcome::fail("Clicking and typing need the Control permission. Turn it on in Settings, Permissions.", layer);
    o.fix = Some(json!({ "label": "Open Permissions", "method": "ui.open", "params": { "page": "settings/permissions" } }));
    o.needs("control")
}

pub fn join_names_pub(v: &[String]) -> String {
    join_names(v)
}

fn join_names(v: &[String]) -> String {
    match v.len() {
        0 => "no drives I can read".into(),
        1 => v[0].clone(),
        n => format!("{} and {}", v[..n - 1].join(", "), v[n - 1]),
    }
}

fn first_result(screen: &crate::tools::atspi::Screen) -> Option<crate::tools::atspi::Element> {
    let top = screen.elements.iter().map(|e| e.y).min().unwrap_or(0);
    let bottom = screen.elements.iter().map(|e| e.y + e.h).max().unwrap_or(1000);
    let content_top = top + (bottom - top) / 6;
    let skip = ["sign in", "search", "home", "menu", "settings", "guide", "voice search", "notifications", "skip navigation", "filters", "back", "forward", "reload", "close", "new tab", "youtube home", "netflix", "account"];
    screen
        .elements
        .iter()
        .filter(|e| e.role == "link" && e.y >= content_top && e.w >= 60 && e.label.split_whitespace().count() >= 2)
        .filter(|e| !skip.iter().any(|s| normalize(&e.label) == *s))
        .min_by_key(|e| (e.y / 40, e.x))
        .cloned()
}

fn looks_like_file(u: &str) -> bool {
    let path = u.split(['?', '#']).next().unwrap_or(u).to_lowercase();
    [".pdf", ".zip", ".exe", ".msi", ".dmg", ".deb", ".rpm", ".mp4", ".mp3", ".png", ".jpg", ".jpeg", ".docx", ".xlsx", ".pptx", ".iso", ".7z", ".tar.gz", ".apk", ".txt", ".csv"].iter().any(|e| path.ends_with(e))
}

fn download(url: &str, cancel: &AtomicBool) -> anyhow::Result<PathBuf> {
    use std::io::{Read, Write};
    let dir = dirs::download_dir().ok_or_else(|| anyhow::anyhow!("The Downloads folder was not found."))?;
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).build().into();
    let mut resp = agent.get(url).header("User-Agent", "Mozilla/5.0 Setu").call().map_err(|e| anyhow::anyhow!("The download failed: {e}"))?;
    let from_header = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split("filename=").nth(1))
        .map(|f| f.trim_matches(['"', ';', ' ']).to_owned());
    let from_url = url.split(['?', '#']).next().unwrap_or(url).rsplit('/').next().filter(|s| !s.is_empty()).map(str::to_owned);
    let name: String = from_header.or(from_url).unwrap_or_else(|| "download".into()).chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let mut dest = dir.join(&name);
    let mut k = 2;
    while dest.exists() {
        let p = Path::new(&name);
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        dest = dir.join(format!("{stem} ({k}){ext}"));
        k += 1;
    }
    let part = dest.with_extension("setu-part");
    let mut f = std::fs::File::create(&part)?;
    let mut r = resp.body_mut().as_reader();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            drop(f);
            let _ = std::fs::remove_file(&part);
            anyhow::bail!("Download stopped.");
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
    }
    drop(f);
    std::fs::rename(&part, &dest)?;
    Ok(dest)
}

async fn check_code(core: &Arc<Core>, layer: &'static str, next: bool, before_run: bool) -> Outcome {
    let Some((ide, win, a)) = native_active(core).await else {
        if let Some(c) = crate::access::java_cause(None).await {
            return crate::access::outcome(&c);
        }
        return fail("Open your code file in your editor first, then ask me again.", layer);
    };
    if let Some(c) = crate::access::java_cause(Some(&win)).await {
        return crate::access::outcome(&c);
    }
    let Some(file) = a.path.clone() else { return fail(&format!("I can't tell which file is open in {}. Save it first, then ask me again.", ide.name), layer) };
    let (language, text, cursor) = (a.language.clone(), a.text.clone(), a.line);
    let mut problems: Vec<code::Problem> = Vec::new();
    if let Some(list) = crate::ide::problems(ide, &win).await {
        let name = a.name.to_lowercase();
        for i in list.into_iter().filter(|i| i.error && (i.file.is_empty() || i.file.to_lowercase() == name)) {
            problems.push(code::Problem { line: i.line, col: i.col, message: i.message });
        }
    }
    if problems.is_empty() {
        let f2 = file.clone();
        let lang2 = language.clone();
        let t2 = text.clone();
        match blocking(move || code::check(&f2, t2.as_deref(), &lang2)).await {
            Some(Ok(p)) => problems = p,
            Some(Err(e)) if !before_run => return fail(&e.to_string(), layer),
            _ => {}
        }
    }
    problems.sort_by_key(|p| (p.line, p.col));
    if problems.is_empty() {
        if before_run {
            return ok("No mistakes found", layer);
        }
        return say("No mistakes found", layer, "No, it looks right. I don't see any mistakes.".into());
    }
    let p = if next { problems.iter().find(|p| p.line > cursor).unwrap_or(&problems[0]).clone() } else { problems[0].clone() };
    let mut sentence = code::explain(&p);
    if problems.len() > 1 && !next {
        sentence.push_str(&format!(" There {} {} more. Say next error to hear {}.", if problems.len() == 2 { "is" } else { "are" }, problems.len() - 1, if problems.len() == 2 { "it" } else { "them" }));
    }
    if before_run {
        sentence = format!("Not yet. {}", sentence_case(sentence.trim_start_matches("Yes, ")));
    }
    let _ = crate::ide::goto(ide, Some(&win), &file, p.line, p.col).await;
    say(&format!("Found a mistake on line {}", p.line), layer, sentence)
}

fn open_terminal(command: &str) -> anyhow::Result<()> {
    open_terminal_in(command, None)
}

pub fn open_terminal_in(command: &str, dir: Option<&Path>) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let wt = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft").join("WindowsApps").join("wt.exe")).filter(|p| p.exists());
        if let Some(wt) = wt {
            let mut a = Vec::new();
            if let Some(d) = dir {
                a.push("-d".to_owned());
                a.push(d.to_string_lossy().into_owned());
            }
            a.extend(["powershell".to_owned(), "-NoExit".to_owned(), "-Command".to_owned(), command.to_owned()]);
            return crate::tools::shell::run_detached(&wt, &a, dir);
        }
        crate::tools::shell::run_detached(Path::new("powershell.exe"), &["-NoExit".to_owned(), "-Command".to_owned(), command.to_owned()], dir)
    }
    #[cfg(target_os = "macos")]
    {
        let cd = dir.map(|d| format!("cd \\\"{}\\\" && ", d.display())).unwrap_or_default();
        let script = format!("tell application \"Terminal\" to do script \"{cd}{}\"", command.replace('\\', "\\\\").replace('"', "\\\""));
        std::process::Command::new("osascript").args(["-e", &script, "-e", "tell application \"Terminal\" to activate"]).status()?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let inner = format!("{command}; echo; exec \"${{SHELL:-bash}}\"");
        let terms: &[(&str, &[&str])] = &[("ptyxis", &["--", "bash", "-c"]), ("gnome-terminal", &["--", "bash", "-c"]), ("kgx", &["-e", "bash", "-c"]), ("konsole", &["-e", "bash", "-c"]), ("xfce4-terminal", &["-x", "bash", "-c"]), ("alacritty", &["-e", "bash", "-c"]), ("kitty", &["bash", "-c"]), ("foot", &["bash", "-c"]), ("xterm", &["-e", "bash", "-c"])];
        for (t, pre) in terms {
            if let Some(p) = crate::tools::apps::which(t) {
                let mut a: Vec<String> = pre.iter().map(|s| s.to_string()).collect();
                a.push(inner.clone());
                return crate::tools::shell::run_detached(&p, &a, dir);
            }
        }
        anyhow::bail!("No terminal app was found.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str) -> Option<(String, Value)> {
        match crate::router::rules(t, "stop", false) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            Some(Intent::Install { app }) => Some(("install".into(), json!({ "app": app }))),
            _ => None,
        }
    }

    #[test]
    fn browser_requests() {
        let (t, a) = p("in brave can you just open youtube").unwrap();
        assert_eq!(t, "web.open");
        assert_eq!(a["site"], "YouTube");
        assert_eq!(a["browser"], "brave");
        let (t, a) = p("open youtube in brave and play me any song").unwrap_or_default();
        let _ = (t, a);
        let (t, a) = p("in brave open youtube and play me any song").unwrap_or_default();
        let _ = (t, a);
        let (t, a) = p("play arijit singh songs on youtube in brave").unwrap();
        assert_eq!(t, "web.play");
        assert_eq!(a["query"], "arijit singh songs");
        assert_eq!(a["site"], "YouTube");
        assert_eq!(a["browser"], "brave");
        let (t, a) = p("i feel like listening to some old kishore kumar songs").unwrap();
        assert_eq!((t.as_str(), a["query"].as_str()), ("web.play", Some("some old kishore kumar songs")));
        let (t, a) = p("play me any song").unwrap();
        assert_eq!(t, "web.play");
        assert_eq!(a["query"], "");
        let (t, a) = p("search for rust tutorials on youtube").unwrap();
        assert_eq!((t.as_str(), a["site"].as_str()), ("web.search", Some("YouTube")));
        let (t, a) = p("watch stranger things on netflix").unwrap();
        assert_eq!((t.as_str(), a["site"].as_str(), a["query"].as_str()), ("web.search", Some("Netflix"), Some("stranger things")));
        let (t, a) = p("go to github dot com").unwrap();
        assert_eq!((t.as_str(), a["url"].as_str()), ("web.open", Some("https://github.com")));
        let (t, a) = p("log in to my google account").unwrap();
        assert_eq!((t.as_str(), a["site"].as_str()), ("web.open", Some("Google account")));
        assert_eq!(p("open brave").map(|x| x.0), None);
        assert_eq!(p("open firefox"), None);
        let (t, a) = p("In Brave, can you just open YouTube?").unwrap();
        assert_eq!((t.as_str(), a["site"].as_str(), a["browser"].as_str()), ("web.open", Some("YouTube"), Some("brave")));
        let (t, a) = p("inside brave can you open youtube and play me any song").unwrap();
        assert_eq!((t.as_str(), a["browser"].as_str(), a["site"].as_str(), a["query"].as_str()), ("web.play", Some("brave"), Some("YouTube"), Some("")));
        let (t, a) = p("open netflix and search for stranger things").unwrap();
        assert_eq!((t.as_str(), a["site"].as_str(), a["query"].as_str()), ("web.search", Some("Netflix"), Some("stranger things")));
        let (t, _) = p("open brave and go to netflix").unwrap();
        assert_eq!(t, "web.open");
        assert_eq!(p("google how tall is mount everest").map(|x| x.0), Some("web.search".into()));
    }

    #[test]
    fn controls_by_name_and_text_keys() {
        let tool = |t: &str| p(t).map(|x| x.0);
        assert_eq!(tool("tick remember"), Some("ui.tick".into()));
        assert_eq!(p("untick remember").map(|x| x.1["on"].clone()), Some(json!(false)));
        assert_eq!(p("choose english from the language dropdown").map(|x| (x.1["option"].clone(), x.1["list"].clone())), Some((json!("english"), json!("language"))));
        assert_eq!(p("open the file menu").map(|x| (x.0, x.1["name"].clone())), Some(("ui.click".into(), json!("file"))));
        assert_ne!(tool("open the start menu"), Some("ui.click".into()));
        assert_eq!(tool("select papa"), Some("ui.click".into()));
        assert_eq!(p("select all").map(|x| x.1["keys"].clone()), Some(json!("ctrl a")));
        assert_eq!(p("select the last word").map(|x| x.1["keys"].clone()), Some(json!("word select")));
        assert_eq!(p("delete the last word").map(|x| x.1["keys"].clone()), Some(json!("word delete")));
        assert_eq!(p("new line").map(|x| x.1["keys"].clone()), Some(json!("shift enter")));
        assert_eq!(p("make this window full screen").map(|x| x.1["keys"].clone()), Some(json!("fullscreen")));
        assert_eq!(p("switch to workspace 3").map(|x| (x.0, x.1["n"].clone())), Some(("wm.workspace".into(), json!(3))));
        assert_eq!(p("move this window to the next workspace").map(|x| (x.1["dir"].clone(), x.1["move"].clone())), Some((json!(1), json!(true))));
        assert_eq!(tool("switch to firefox"), Some("wm.focus".into()));
        assert_eq!(tool("go to desktop"), Some("wm.desktop".into()));
        assert_eq!(tool("double click the readme"), Some("pointer.click".into()));
        assert_eq!(tool("scroll down slowly"), Some("pointer.scroll".into()));
    }

    #[test]
    fn keys_clicks_files() {
        assert_eq!(p("new tab").map(|x| x.0), Some("browser.tab.new".into()));
        assert_eq!(p("press control shift p").map(|x| x.0), Some("key".into()));
        assert_eq!(p("scroll down").map(|x| x.1["keys"].clone()), Some(json!("pagedown")));
        assert_eq!(p("pause").map(|x| x.0), Some("media.pause".into()));
        assert_eq!(p("click sign in").map(|x| (x.0, x.1["name"].clone())), Some(("ui.click".into(), json!("sign in"))));
        assert_eq!(p("click number 4"), None);
        assert_eq!(p("play the first video").map(|x| x.0), Some("browser.page".into()));
        assert_eq!(p("open d drive").map(|x| x.0), Some("files.open".into()));
        assert_eq!(p("open my downloads").map(|x| x.0), Some("files.open".into()));
        assert_eq!(p("open my pen drive").map(|x| x.0), Some("files.open".into()));
        assert_eq!(p("find the file resume").map(|x| (x.0, x.1["name"].clone())), Some(("files.find".into(), json!("resume"))));
        assert_eq!(p("open my resume pdf").map(|x| (x.0, x.1["ext"].clone())), Some(("files.openfile".into(), json!("pdf"))));
        assert_eq!(p("create a folder called projects on the desktop").map(|x| (x.0, x.1["items"][0]["path"][0].clone(), x.1["where"].clone())), Some(("files.create".into(), json!("projects"), json!("desktop"))));
        assert_eq!(p("how much space is left on c drive").map(|x| x.0), Some("files.space".into()));
        assert_eq!(p("type my email anoop dot mehra at gmail dot com").map(|x| x.1["text"].clone()), Some(json!("anoop.mehra@gmail.com")));
    }

    #[test]
    fn coding_git_downloads() {
        assert_eq!(p("start coding").map(|x| x.0), Some("code.dictate".into()));
        assert_eq!(p("am I wrong somewhere").map(|x| x.0), Some("code.check".into()));
        assert_eq!(p("run my code").map(|x| x.0), Some("code.run".into()));
        assert_eq!(p("install the python extension").map(|x| (x.0, x.1["name"].clone())), Some(("code.extension".into(), json!("python"))));
        assert_eq!(p("install python extension in vs code").map(|x| x.0), Some("code.extension".into()));
        assert_eq!(p("turn on auto save").map(|x| x.1["key"].clone()), Some(json!("files.autoSave")));
        assert_eq!(p("change the theme to dark in vs code").map(|x| x.1["value"].clone()), Some(json!("Default Dark Modern")));
        assert_eq!(p("commit my changes with message fixed the login bug").map(|x| (x.0, x.1["message"].clone())), Some(("git.commit".into(), json!("fixed the login bug"))));
        assert_eq!(p("push my code to github").map(|x| x.0), Some("git.push".into()));
        assert_eq!(p("git status").map(|x| x.0), Some("git.status".into()));
        assert_eq!(p("log in to github").map(|x| x.0), Some("git.login".into()));
        assert_eq!(p("can you please download vlc").map(|x| x.0), Some("install".into()));
        assert_eq!(p("download whatsapp").map(|x| x.1["app"].clone()), Some(json!("Whatsapp")));
        assert_eq!(p("download this").map(|x| x.0), Some("web.download_current".into()));
        assert_eq!(p("run npm install in the terminal").map(|x| (x.0, x.1["command"].clone())), Some(("terminal.run".into(), json!("npm install"))));
        assert_eq!(p("open main dot cpp in vs code").map(|x| x.0), Some("code.open".into()));
        assert_eq!(p("open main dot cpp in vs code").map(|x| x.1["name"].clone()), Some(json!("main.cpp")));
        assert_eq!(p("build the project").map(|x| x.0), Some("code.build".into()));
        assert_eq!(p("run the project").map(|x| x.0), Some("code.run".into()));
        let f = |t: &str, raw: &str| match crate::skills::parse(t, raw) {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => panic!("{t}: {other:?}"),
        };
        let (t, a) = f("open main cpp in pycharm", "open main.cpp in PyCharm");
        assert_eq!((t.as_str(), a["name"].as_str(), a["app"].as_str()), ("code.open", Some("main.cpp"), Some("pycharm")));
        let (t, a) = f("open the project in intellij", "open the project in IntelliJ");
        assert_eq!((t.as_str(), a["name"].as_str(), a["app"].as_str(), a["folder"].as_bool()), ("code.open", Some("project"), Some("intellij"), Some(true)));
        let (_, a) = f("open app js in vs code", "open app.js in vs code");
        assert_eq!(a["name"], "app.js");
        assert_eq!(file_name_in("open app dot js", "app dot js"), "app.js");
        assert_eq!(file_name_in("open notes", "notes"), "notes");
    }
}

#[cfg(test)]
mod sweep {
    use crate::router::{Intent, rules};
    use serde_json::Value;

    fn act(t: &str) -> (String, Value) {
        match rules(t, "stop", false) {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => (format!("{other:?}"), Value::Null),
        }
    }

    #[test]
    fn coding_files_and_projects_from_chat() {
        let f = |t: &str, raw: &str| match crate::skills::parse(t, raw) {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => panic!("{t}: {other:?}"),
        };
        let (t, a) = f("alright now create a file called tut1 cpp", "alright now create a file called tut1.cpp");
        assert_eq!((t.as_str(), a["items"][0]["path"][0].as_str(), a["inside"].as_str(), a["code"].as_bool()), ("files.create", Some("tut1.cpp"), Some(""), Some(true)));
        let (t, a) = f("create a file inside tutorial call it as tut1 cpp", "create a file inside tutorial, call it as tut1.cpp");
        assert_eq!((t.as_str(), a["items"][0]["path"][0].as_str(), a["inside"].as_str()), ("files.create", Some("tut1.cpp"), Some("tutorial")));
        let (_, a) = f("create a new file called main dot py", "create a new file called main dot py");
        assert_eq!(a["items"][0]["path"][0], "main.py");
        let (t, a) = f("in vs code create a new folder called tutorial as i want to start with c", "in vs code create a new folder, called tutorial, as I want to start with c++");
        assert_eq!((t.as_str(), a["items"][0]["path"][0].as_str(), a["language"].as_str(), a["code"].as_bool()), ("files.create", Some("tutorial"), Some("c++"), Some(true)));
        assert_eq!(f("now i want to do coding vs is installed", "now I want to do coding, vs is installed").0, "code.start");
        assert_eq!(f("start coding", "start coding").0, "code.dictate");
    }

    #[test]
    fn windows_projects_and_code_requests() {
        let w = |t: &str| act(t);
        assert_eq!(w("snap this window to the left").1["where"], "left");
        assert_eq!(w("move it to the right").1["where"], "right");
        assert_eq!(w("move the window to the right side of the screen").1["where"], "right");
        assert_eq!(w("maximize the window").1["where"], "max");
        assert_eq!(w("minimise").1["where"], "min");
        assert_eq!(w("restore down").1["where"], "restore");
        assert_eq!(w("move this window to the other screen").1["where"], "next_screen");
        assert_eq!(w("show desktop").0, "wm.desktop");
        assert_eq!(w("switch windows").0, "wm.switch");
        let (t, a) = w("put firefox and vs code side by side");
        assert_eq!((t.as_str(), a["left"].as_str(), a["right"].as_str()), ("wm.side", Some("firefox"), Some("vs code")));
        let (t, a) = w("firefox on the left and terminal on the right");
        assert_eq!((t.as_str(), a["left"].as_str(), a["right"].as_str()), ("wm.side", Some("firefox"), Some("terminal")));
        let (t, a) = w("move spotify to the right");
        assert_eq!((t.as_str(), a["app"].as_str(), a["where"].as_str()), ("wm.place", Some("spotify"), Some("right")));
        assert_eq!(w("arrange my windows").0, "wm.side");
        let (t, a) = w("switch to firefox");
        assert_eq!((t.as_str(), a["app"].as_str()), ("wm.focus", Some("firefox")));
        assert_eq!(w("bring chrome to the front").1["app"], "chrome");
        assert_eq!(w("switch to the youtube tab").0, "browser.tab.activate");
        assert_eq!(w("go to tab 3").0, "browser.tab.activate");
        assert_ne!(w("switch to dark mode").0, "wm.focus");
        assert_eq!(w("new tab").0, "browser.tab.new");
        assert_eq!(w("open a new window").0, "browser.window");
        let (t, a) = w("open a new project in vs code");
        assert_eq!((t.as_str(), a["name"].as_str(), a["language"].as_str()), ("code.project", Some(""), Some("")));
        let (t, a) = w("create a new python project called weather app");
        assert_eq!((t.as_str(), a["name"].as_str(), a["language"].as_str()), ("code.project", Some("weather app"), Some("python")));
        let (_, a) = w("start a new project named todo list in java");
        assert_eq!((a["name"].as_str(), a["language"].as_str()), (Some("todo list"), Some("java")));
        assert_eq!(w("write a function that reverses a string").0, "code.write");
        let (t, a) = w("add a comment saying this adds two numbers");
        assert_eq!((t.as_str(), a["text"].as_str()), ("code.comment", Some("this adds two numbers")));
        assert_eq!(w("open the downloads folder").0, "files.open");
    }

    fn kind(t: &str) -> String {
        match rules(t, "stop", false) {
            Some(Intent::Act { tool, .. }) => tool,
            Some(Intent::OpenApp { .. }) => "open_app".into(),
            Some(Intent::CloseApp { .. }) => "close_app".into(),
            Some(i) => i.action().into(),
            None => "model".into(),
        }
    }

    #[test]
    fn old_commands_keep_their_meaning() {
        let cases = [
            ("open firefox", "open_app"),
            ("open notepad", "open_app"),
            ("launch vs code", "open_app"),
            ("open the browser", "open_app"),
            ("close spotify", "close_app"),
            ("close the window", "close_app"),
            ("turn the volume up", "volume"),
            ("mute the sound", "volume"),
            ("dim the screen", "brightness"),
            ("what time is it", "time"),
            ("connect to wifi", "wifi"),
            ("show numbers", "numbers"),
            ("click number three", "numbers"),
            ("go to sleep", "sleep"),
            ("stop", "cancel"),
            ("undo", "undo"),
            ("type hello world", "type_text"),
            ("install gimp", "install_app"),
            ("show yourself", "open_app"),
            ("how far is the moon", "model"),
            ("tell me a joke", "model"),
            ("open settings", "open_app"),
            ("open photos", "open_app"),
            ("open youtube", "open_app"),
            ("open my music", "open_app"),
            ("open my pictures folder", "files.open"),
        ];
        for (t, want) in cases {
            assert_eq!(kind(t), want, "{t}");
        }
    }

    #[test]
    fn new_commands_reach_skills() {
        let cases = [
            ("in brave open youtube", "web.open"),
            ("open youtube in chrome", "web.open"),
            ("play despacito", "web.play"),
            ("search for biryani recipes", "web.search"),
            ("go to amazon", "web.open"),
            ("new tab", "browser.tab.new"),
            ("pause", "media.pause"),
            ("pause youtube", "media.pause"),
            ("what's playing", "media.list"),
            ("open the third link in a new tab", "browser.page"),
            ("show options for the 3rd image from left", "browser.page"),
            ("open youtube in a new tab", "browser.open"),
            ("open an incognito window", "browser.window"),
            ("close the youtube tab", "browser.tab.close"),
            ("open browser settings", "browser.settings"),
            ("scroll down", "key"),
            ("go back", "key"),
            ("press control s", "key"),
            ("click the subscribe button", "ui.click"),
            ("open c drive", "files.open"),
            ("open my documents folder", "files.open"),
            ("open the project folder", "files.openfolder"),
            ("find my resume", "files.find"),
            ("download vlc", "install_app"),
            ("start coding", "code.dictate"),
            ("check my code", "code.check"),
            ("run the program", "code.run"),
            ("install the prettier extension", "code.extension"),
            ("turn on word wrap", "code.setting"),
            ("format the document", "code.command"),
            ("git push", "git.push"),
            ("commit everything", "git.commit"),
            ("pull the latest changes", "git.pull"),
            ("type my password", "ui.secret"),
        ];
        for (t, want) in cases {
            assert_eq!(kind(t), want, "{t}");
        }
    }
}
