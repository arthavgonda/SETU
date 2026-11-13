use crate::router::Intent;
use serde_json::{Value, json};
use std::sync::Mutex;

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.into(), args }
}

#[derive(Debug, Clone)]
struct Tok {
    raw: String,
    low: String,
}

const SEPS: &[char] = &[',', ';', '!', '?', '"', '(', ')', '[', ']', '{', '}', '“', '”', '«', '»'];

fn lex(raw: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let spaced = raw.replace([',', ';'], " , ");
    for w in spaced.split(|c: char| c.is_whitespace() || (SEPS.contains(&c) && c != ',')) {
        if w == "," {
            if out.last().is_some_and(|t| t.low != ",") {
                out.push(Tok { raw: ",".into(), low: ",".into() });
            }
            continue;
        }
        let w = w.trim_matches(|c: char| matches!(c, '\'' | '‘' | '’' | '`' | ':'));
        let w = w.trim_end_matches(['.', ':']);
        if w.is_empty() {
            continue;
        }
        out.push(Tok { raw: w.to_owned(), low: w.to_lowercase() });
    }
    let mut joined: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < out.len() {
        if i + 2 < out.len() && matches!(out[i + 1].low.as_str(), "dot" | "point") && (1..=5).contains(&out[i + 2].low.len()) && out[i + 2].low.chars().all(|c| c.is_ascii_alphanumeric() || c == '+') {
            let raw = format!("{}.{}", out[i].raw, out[i + 2].low);
            joined.push(Tok { low: raw.to_lowercase(), raw });
            i += 3;
            continue;
        }
        joined.push(out[i].clone());
        i += 1;
    }
    joined
}

const STOP: &[&str] = &[
    ",", "and", "inside", "in", "into", "within", "under", "on", "with", "where", "which", "that", "then", "also", "after", "because", "for", "so", "to", "at", "from", "please", "now", "it",
    "folder", "folders", "file", "files", "directory", "as", "but", "i", "we", "plus", "containing", "there", "here", "having", "named", "called", "too", "please", "if", "when", "or",
    "subfolder",
];

const ARTICLES: &[&str] = &["a", "an", "the", "my", "new", "another", "one", "empty", "this", "that", "some", "our", "your", "blank", "fresh", "two", "sub", "second", "first", "its"];

const CREATE: &[&str] = &["create", "make", "add", "generate", "build", "new", "creating", "making", "setup", "put", "want", "need", "give"];

const OTHER_VERBS: &[&str] = &["open", "delete", "remove", "find", "search", "rename", "move", "copy", "show", "close", "trash", "locate", "read", "edit", "write", "type", "send", "share", "upload", "download", "play", "run", "check"];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Folder,
    File,
}

#[derive(Debug, Clone, PartialEq)]
enum Parent {
    None,
    Item(usize),
    Last,
    Named(String),
    Place(String),
}

#[derive(Debug, Clone)]
struct Item {
    kind: Kind,
    at: usize,
    name: Option<String>,
    ext: Option<&'static str>,
    parent: Parent,
}

fn mention(toks: &[Tok], i: usize) -> Option<Kind> {
    match toks[i].low.as_str() {
        "folder" | "folders" | "directory" | "directories" | "subfolder" | "subfolders" => Some(Kind::Folder),
        "file" | "files" => {
            let next = toks.get(i + 1).map(|t| t.low.as_str()).unwrap_or("");
            if matches!(next, "explorer" | "manager" | "browser" | "system" | "app") { None } else { Some(Kind::File) }
        }
        _ => None,
    }
}

pub fn lang_ext(w: &str) -> Option<&'static str> {
    Some(match w {
        "python" | "py" => "py",
        "c++" | "cpp" => "cpp",
        "c" => "c",
        "java" => "java",
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "html" => "html",
        "css" => "css",
        "text" | "txt" => "txt",
        "markdown" | "md" => "md",
        "json" => "json",
        "rust" => "rs",
        "go" | "golang" => "go",
        "kotlin" => "kt",
        "swift" => "swift",
        "ruby" => "rb",
        "php" => "php",
        "shell" | "bash" => "sh",
        "csv" => "csv",
        "word" => "docx",
        "excel" => "xlsx",
        "powerpoint" => "pptx",
        "sql" => "sql",
        "yaml" => "yaml",
        "c#" => "cs",
        _ => return None,
    })
}

const TLDS: &[&str] = &["com", "org", "net", "edu", "gov", "co", "uk", "us"];

pub fn file_like(s: &str) -> bool {
    let Some((stem, ext)) = s.rsplit_once('.') else { return false };
    !stem.is_empty()
        && !stem.ends_with('.')
        && (1..=5).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '+')
        && ext.chars().any(|c| c.is_ascii_alphabetic())
        && !TLDS.contains(&ext.to_lowercase().as_str())
        && !s.to_lowercase().starts_with("www.")
}

const CODE_EXTS: &[&str] = &["c", "cc", "cpp", "cxx", "h", "hpp", "py", "js", "mjs", "ts", "jsx", "tsx", "java", "kt", "go", "rs", "rb", "php", "cs", "swift", "html", "htm", "css", "scss", "json", "sh", "sql", "yaml", "yml", "xml", "vue", "dart", "lua", "r", "m", "ipynb", "toml", "ini"];

pub fn is_code_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| CODE_EXTS.contains(&e.to_lowercase().as_str()))
}

fn take_name(toks: &[Tok], from: usize, max: usize) -> Option<(String, usize)> {
    let mut parts: Vec<&str> = Vec::new();
    let mut j = from;
    while j < toks.len() && parts.len() < max {
        let w = toks[j].low.as_str();
        if STOP.contains(&w) || (parts.is_empty() && ARTICLES.contains(&w) && !file_like(w)) {
            if parts.is_empty() && ARTICLES.contains(&w) {
                j += 1;
                continue;
            }
            break;
        }
        parts.push(toks[j].raw.as_str());
        j += 1;
    }
    (!parts.is_empty()).then(|| (parts.join(" "), j))
}

fn skip(toks: &[Tok], j: usize) -> usize {
    let mut j = j;
    while toks.get(j).is_some_and(|t| t.low == ",") {
        j += 1;
    }
    j
}

fn place_at(toks: &[Tok], k: usize) -> Option<(String, usize)> {
    let mut k = skip(toks, k);
    while k < toks.len() && matches!(toks[k].low.as_str(), "the" | "my" | "our") {
        k += 1;
    }
    let w = toks.get(k)?.low.as_str();
    let next = toks.get(k + 1).map(|t| t.low.as_str()).unwrap_or("");
    if w.len() == 1 && w.chars().all(|c| c.is_ascii_alphabetic()) && matches!(next, "drive" | "disk") {
        return Some((format!("{w} drive"), k + 2));
    }
    if w == "drive" && next.len() == 1 && next.chars().all(|c| c.is_ascii_alphabetic()) {
        return Some((format!("{next} drive"), k + 2));
    }
    let single = ["desktop", "documents", "document", "downloads", "download", "pictures", "photos", "music", "videos", "home", "projects"];
    if single.contains(&w) {
        let end = if next == "folder" || next == "directory" { k + 2 } else { k + 1 };
        return Some((w.to_owned(), end));
    }
    if ["usb", "pendrive", "pen", "flash", "external", "sd"].contains(&w) && ["drive", "disk", "card"].iter().any(|d| toks.iter().skip(k).take(3).any(|t| t.low == *d)) {
        return Some(("usb drive".into(), (k + 2).min(toks.len())));
    }
    None
}

fn is_pronoun(w: &str) -> bool {
    matches!(w, "it" | "that" | "this" | "there" | "them" | "same")
}

struct Parsed {
    items: Vec<Item>,
    toks: Vec<Tok>,
}

fn clean_raw(raw: &str) -> (String, bool) {
    let mut s = format!(" {} ", raw.trim());
    let mut code = false;
    for p in [" in vs code", " in vscode", " in visual studio code", " using vs code", " using vscode", " on vs code", " for vs code", " with vs code", " in the code editor", " in code editor", " in the editor"] {
        let lower = s.to_ascii_lowercase();
        if let Some(i) = lower.find(p) {
            s.replace_range(i..i + p.len(), "");
            code = true;
        }
    }
    for p in [" in file explorer", " in the file explorer", " in files", " in the file manager", " in finder"] {
        let lower = s.to_ascii_lowercase();
        if let Some(i) = lower.find(p) {
            s.replace_range(i..i + p.len(), "");
        }
    }
    (s.trim().to_owned(), code)
}

fn parse(raw: &str) -> Option<Parsed> {
    let toks = lex(raw);
    if toks.is_empty() {
        return None;
    }
    let mut items: Vec<Item> = Vec::new();
    let mut consumed = vec![false; toks.len()];
    for (i, used) in consumed.iter_mut().enumerate() {
        let Some(kind) = mention(&toks, i) else { continue };
        if kind == Kind::Folder && toks.get(i + 1).is_some_and(|t| t.low == "name" || t.low == "path") && toks.get(i + 2).is_none() {
            continue;
        }
        items.push(Item { kind, at: i, name: None, ext: None, parent: Parent::None });
        *used = true;
    }
    for it in items.iter_mut() {
        let i = it.at;
        if it.kind == Kind::File && i > 0 {
            if let Some(e) = lang_ext(&toks[i - 1].low) {
                it.ext = Some(e);
                consumed[i - 1] = true;
            } else if i > 1 && toks[i - 1].low == "plus" && toks[i - 2].low == "plus" {
                it.ext = Some("cpp");
            }
        }
        let mut j = skip(&toks, i + 1);
        if toks.get(j).is_some_and(|t| t.low == "which" || t.low == "that") && toks.get(j + 1).is_some_and(|t| t.low == "is" || t.low == "will") {
            j += 2;
            if toks.get(j).is_some_and(|t| t.low == "be") {
                j += 1;
            }
        }
        let marker = toks.get(j).map(|t| t.low.as_str()).unwrap_or("");
        let mut found: Option<(String, usize)> = None;
        if matches!(marker, "called" | "named" | "titled" | "name" | "naming") {
            let k = skip(&toks, if toks.get(j + 1).is_some_and(|t| t.low == "as") { j + 2 } else { j + 1 });
            found = take_name(&toks, k, 6);
        } else if (marker == "with" || marker == "by") && toks.get(j + 1).is_some_and(|t| t.low == "the") && toks.get(j + 2).is_some_and(|t| t.low == "name") {
            found = take_name(&toks, j + 3, 6);
        } else if !marker.is_empty() && !STOP.contains(&marker) && !ARTICLES.contains(&marker) && !CREATE.contains(&marker) && !OTHER_VERBS.contains(&marker) && mention(&toks, j).is_none() {
            let max = if file_like(marker) { 1 } else { 4 };
            found = take_name(&toks, j, max);
        }
        if let Some((n, end)) = found {
            for c in consumed.iter_mut().take(end).skip(j) {
                *c = true;
            }
            it.name = Some(n);
            continue;
        }
        let back = if it.ext.is_some() { i.checked_sub(2) } else { i.checked_sub(1) };
        if let Some(k) = back {
            let w = toks[k].low.as_str();
            if !ARTICLES.contains(&w) && !CREATE.contains(&w) && !STOP.contains(&w) && !OTHER_VERBS.contains(&w) && lang_ext(w).is_none() && !consumed[k] && w != "plus" && w.parse::<f64>().is_err() {
                it.name = Some(toks[k].raw.clone());
                consumed[k] = true;
                continue;
            }
        }
        let next_mention = toks.iter().enumerate().skip(i + 1).find(|(k, _)| mention(&toks, *k).is_some()).map(|(k, _)| k).unwrap_or(toks.len());
        let mut k = i + 1;
        while k + 1 < next_mention {
            if matches!(toks[k].low.as_str(), "call" | "name") && toks[k + 1].low == "it" {
                let start = if toks.get(k + 2).is_some_and(|t| t.low == "as") { k + 3 } else { k + 2 };
                if let Some((n, end)) = take_name(&toks, start, 6) {
                    for c in consumed.iter_mut().take(end).skip(k) {
                        *c = true;
                    }
                    it.name = Some(n);
                }
                break;
            }
            k += 1;
        }
    }
    for (i, t) in toks.iter().enumerate() {
        if !consumed[i] && file_like(&t.raw) && !items.iter().any(|it| it.name.as_deref() == Some(t.raw.as_str())) {
            let prev = i.checked_sub(1).map(|k| toks[k].low.as_str()).unwrap_or("");
            if matches!(prev, "in" | "inside" | "into" | "within" | "under") {
                continue;
            }
            items.push(Item { kind: Kind::File, at: i, name: Some(t.raw.clone()), ext: None, parent: Parent::None });
            consumed[i] = true;
        }
    }
    items.sort_by_key(|it| it.at);
    if items.is_empty() {
        return None;
    }
    let containers = ["inside", "in", "into", "within", "under", "insde", "inside of", "on"];
    let mut m = 0;
    while m < toks.len() {
        let w = toks[m].low.as_str();
        if !containers.contains(&w) {
            m += 1;
            continue;
        }
        let mut k = skip(&toks, m + 1);
        if toks.get(k).is_some_and(|t| t.low == "of") {
            k = skip(&toks, k + 1);
        }
        let Some(next) = toks.get(k) else { break };
        let before = items.iter().enumerate().filter(|(_, it)| it.at < m).map(|(n, _)| n).next_back();
        let after_first = |skip: Option<usize>| items.iter().enumerate().find(|(n, it)| it.at > m && Some(*n) != skip).map(|(n, _)| n);
        let prev_folder = items.iter().enumerate().filter(|(_, it)| it.at < m && it.kind == Kind::Folder).map(|(n, _)| n).next_back();
        let pronoun = is_pronoun(&next.low)
            && !toks.get(k + 1).is_some_and(|t| mention(&toks, k + 1).is_some() && t.low != "folder")
            || (matches!(next.low.as_str(), "the" | "that" | "this") && toks.get(k + 1).is_some_and(|t| t.low == "folder" || t.low == "directory") && !items.iter().any(|it| it.at == k + 1 && it.name.is_some()));
        if pronoun {
            let target = match prev_folder {
                Some(f) => Parent::Item(f),
                None => Parent::Last,
            };
            let who = after_first(prev_folder).filter(|n| Parent::Item(*n) != target).or(before.filter(|b| Some(*b) != prev_folder));
            if let Some(n) = who
                && items[n].parent == Parent::None
            {
                items[n].parent = target;
            }
            m = k + 1;
            continue;
        }
        if w == "on" && place_at(&toks, k).is_none() {
            m += 1;
            continue;
        }
        let mut target: Option<(Parent, Option<usize>)> = None;
        if let Some((p, _)) = place_at(&toks, k) {
            target = Some((Parent::Place(p), None));
        } else {
            let mut q = k;
            while q < toks.len() && matches!(toks[q].low.as_str(), "the" | "a" | "an" | "my" | "new" | "that" | "this") {
                q += 1;
            }
            if let Some(n) = items.iter().position(|it| it.kind == Kind::Folder && (it.at == q || it.at == q + 1 || (it.at > q && it.at <= q + 3 && it.name.is_some() && (q..it.at).all(|z| !STOP.contains(&toks[z].low.as_str()) || toks[z].low == "folder")))) {
                target = Some((Parent::Item(n), Some(n)));
            } else if let Some((n, _)) = take_name(&toks, q, 3)
                && !file_like(&n)
            {
                target = Some((Parent::Named(n), None));
            }
        }
        if let Some((parent, own)) = target {
            let who = before.filter(|b| Some(*b) != own).or_else(|| after_first(own));
            if let Some(n) = who
                && items[n].parent == Parent::None
                && Some(n) != own
            {
                items[n].parent = parent;
            }
        }
        m = k + 1;
    }
    for (m, tok) in toks.iter().enumerate() {
        if !matches!(tok.low.as_str(), "with" | "containing" | "having") {
            continue;
        }
        let Some(f) = items.iter().enumerate().filter(|(_, it)| it.at < m && it.kind == Kind::Folder).map(|(n, _)| n).next_back() else { continue };
        for (n, it) in items.iter_mut().enumerate() {
            if it.at > m && it.parent == Parent::None && n != f {
                it.parent = Parent::Item(f);
            }
        }
    }
    let folders: Vec<usize> = items.iter().enumerate().filter(|(_, it)| it.kind == Kind::Folder).map(|(n, _)| n).collect();
    if folders.len() == 1 {
        let f = folders[0];
        for n in 0..items.len() {
            if n != f && items[n].kind == Kind::File && items[n].parent == Parent::None && items[n].at > items[f].at {
                items[n].parent = Parent::Item(f);
            }
        }
    }
    Some(Parsed { items, toks })
}

fn wants_create(toks: &[Tok], first: usize) -> bool {
    let head: Vec<&str> = toks.iter().take(first).map(|t| t.low.as_str()).collect();
    if head.iter().any(|w| OTHER_VERBS.contains(w)) && !head.iter().any(|w| matches!(*w, "create" | "make" | "add" | "generate" | "build" | "new")) {
        return false;
    }
    head.iter().any(|w| CREATE.contains(w)) || head.windows(2).any(|p| p == ["set", "up"]) || toks.iter().any(|t| matches!(t.low.as_str(), "create" | "make" | "generate"))
}

fn sanitize(name: &str) -> String {
    let s: String = name.trim().chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    s.trim().trim_matches('.').trim().to_owned()
}

const DOC_EXTS: &[&str] = &["txt", "md", "pdf", "docx", "xlsx", "pptx", "csv", "odt", "rtf"];

fn file_name(name: &str, ext: Option<&str>) -> String {
    let mut n = sanitize(name);
    if !n.contains('.') && ext.is_none() {
        let words: Vec<&str> = n.split_whitespace().collect();
        if let Some((last, rest)) = words.split_last()
            && !rest.is_empty()
            && (CODE_EXTS.contains(&last.to_lowercase().as_str()) || DOC_EXTS.contains(&last.to_lowercase().as_str()))
        {
            let joiner = if CODE_EXTS.contains(&last.to_lowercase().as_str()) { "_" } else { " " };
            n = format!("{}.{}", rest.join(joiner), last.to_lowercase());
        }
    }
    let n = if n.contains('.') || ext.is_none() { n } else { format!("{n}.{}", ext.unwrap_or("")) };
    if is_code_file(&n) { n.replace(' ', "_") } else { n }
}

fn folder_name(name: &str) -> String {
    let n = sanitize(name);
    let mut c = n.chars();
    match c.next() {
        Some(f) if n.chars().all(|ch| !ch.is_uppercase()) && n.split_whitespace().count() > 1 => f.to_uppercase().collect::<String>() + c.as_str(),
        _ => n,
    }
}

pub fn create(t: &str, raw: &str) -> Option<Intent> {
    let p = crate::plan::clean(t);
    if p.contains("project") && !(p.contains("folder") || p.contains("file") || p.contains("directory")) {
        return None;
    }
    if p.contains("to do") || p.contains("todo") || p.contains("shopping list") || p.contains("reminder") || p.contains("note") && !p.contains("notes txt") && !p.contains("file") {
        return None;
    }
    let (clean, code_words) = clean_raw(raw);
    let parsed = parse(&clean)?;
    let first = parsed.items.first()?.at;
    if !wants_create(&parsed.toks, first) {
        return None;
    }
    let items = &parsed.items;
    let mut out: Vec<Value> = Vec::new();
    let mut place = String::new();
    let mut inside = String::new();
    let path_of = |n: usize| -> Option<(Vec<String>, Parent)> {
        let mut path = Vec::new();
        let mut cur = n;
        let mut root_parent = Parent::None;
        for _ in 0..8 {
            let it = &items[cur];
            let name = match (it.kind, &it.name) {
                (Kind::Folder, Some(n)) => folder_name(n),
                (Kind::Folder, None) => "New folder".to_owned(),
                (Kind::File, Some(n)) => file_name(n, it.ext),
                (Kind::File, None) => String::new(),
            };
            path.insert(0, name);
            match &it.parent {
                Parent::Item(p) if *p != cur => cur = *p,
                other => {
                    root_parent = other.clone();
                    break;
                }
            }
        }
        Some((path, root_parent))
    };
    for (n, it) in items.iter().enumerate() {
        let Some((path, root)) = path_of(n) else { continue };
        match root {
            Parent::Place(p) if place.is_empty() => place = p,
            Parent::Named(x) if inside.is_empty() => inside = x,
            Parent::Last if inside.is_empty() => inside = "it".into(),
            _ => {}
        }
        out.push(json!({ "kind": if it.kind == Kind::Folder { "folder" } else { "file" }, "path": path }));
    }
    let language = crate::skills::language_in(raw);
    let code = code_words || !language.is_empty() || items.iter().any(|it| it.kind == Kind::File && it.ext.is_some_and(|e| CODE_EXTS.contains(&e))) || out.iter().any(|o| o["kind"] == "file" && o["path"].as_array().and_then(|a| a.last()).and_then(Value::as_str).is_some_and(is_code_file));
    Some(act("files.create", json!({ "items": out, "where": place, "inside": inside, "code": code, "language": language })))
}

const WRITE_APPS: &[&str] = &[
    "notepad", "notepad++", "wordpad", "word", "microsoft word", "ms word", "gedit", "text editor", "gnome text editor", "kate", "kwrite", "mousepad", "textedit", "notes", "sticky notes", "onenote",
    "libreoffice writer", "writer", "obsidian", "notion", "pluma", "xed", "featherpad", "leafpad", "sublime text", "sublime",
];

const WRITE_VERBS: &[&str] = &["write", "type", "put", "enter", "note", "jot", "paste"];

fn find_word(lower: &str, word: &str, from: usize) -> Option<usize> {
    let mut start = from;
    while start <= lower.len() {
        let rel = lower.get(start..)?.find(word)?;
        let i = start + rel;
        let before_ok = i == 0 || lower[..i].ends_with(|c: char| c.is_whitespace() || c == ',' || c == ':');
        let after = &lower[i + word.len()..];
        let after_ok = after.is_empty() || after.starts_with(|c: char| c.is_whitespace() || c == ':' || c == ',');
        if before_ok && after_ok {
            return Some(i);
        }
        start = i + word.len();
    }
    None
}

fn bad_target(app: &str) -> bool {
    let a = app.trim();
    let n = a.split_whitespace().count();
    n == 0
        || n > 4
        || ["box", "field", "bar", "tab", "page", "search", "input", "website", "site", "form", "chat", "message", "terminal", "file", "document", "folder"].iter().any(|w| a.ends_with(w))
        || matches!(a, "vs code" | "vscode" | "visual studio code" | "code" | "the editor" | "code editor" | "whatsapp" | "whats app" | "it" | "this" | "that" | "there" | "here")
        || crate::tools::web::site(a).is_some() && !WRITE_APPS.contains(&a)
}

fn tidy_text(t: &str) -> String {
    let mut s = t.trim_start_matches([':', ',', ' ', '\t']).trim_start().to_owned();
    for lead in ["down ", "down:", "down\n", "this:", "the following:", "following:", "this\n", "the following\n"] {
        if s.to_ascii_lowercase().starts_with(lead) {
            s = s[lead.len()..].trim_start_matches([':', ' ']).trim_start().to_owned();
        }
    }
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    s.trim().to_owned()
}

fn app_part(s: &str) -> String {
    let mut a = s.trim().trim_matches([',', ':']).trim().to_owned();
    for lead in ["the ", "a new ", "a ", "an ", "my ", "new "] {
        if let Some(r) = a.strip_prefix(lead) {
            a = r.trim().to_owned();
        }
    }
    for tail in [" app", " application", " window", " please"] {
        if let Some(r) = a.strip_suffix(tail) {
            a = r.trim().to_owned();
        }
    }
    a
}

pub fn write_in_app(raw: &str) -> Option<Intent> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let mut lead_end = 0;
    loop {
        let rest = &lower[lead_end..];
        let skip = ["please ", "can you ", "can u ", "could you ", "would you ", "now ", "i want you to ", "i want u to ", "kindly ", "ok ", "okay ", "and ", "so ", "just ", "go ahead and ", "then "]
            .iter()
            .find(|p| rest.starts_with(**p))
            .map(|p| p.len());
        match skip {
            Some(n) => lead_end += n,
            None => break,
        }
    }
    let body = &lower[lead_end..];
    for prefix in ["in ", "on ", "inside ", "into ", "using "] {
        if !body.starts_with(prefix) {
            continue;
        }
        let from = lead_end + prefix.len();
        let verb = WRITE_VERBS.iter().filter_map(|v| find_word(&lower, v, from).map(|i| (i, *v))).min_by_key(|(i, _)| *i);
        let Some((vi, v)) = verb else { continue };
        let app = app_part(&lower[from..vi]);
        if bad_target(&app) {
            return None;
        }
        let text = tidy_text(&raw[vi + v.len()..]);
        if text.is_empty() {
            return None;
        }
        return Some(act("app.write", json!({ "app": app, "text": text })));
    }
    for opener in ["open ", "launch ", "start ", "open up "] {
        if !body.starts_with(opener) {
            continue;
        }
        let from = lead_end + opener.len();
        for joiner in [" and then ", " and ", " then ", ", then ", ", "] {
            let Some(rel) = lower[from..].find(joiner) else { continue };
            let j = from + rel;
            let after = j + joiner.len();
            let verb = WRITE_VERBS.iter().find(|v| lower[after..].starts_with(**v) && lower[after + v.len()..].starts_with(|c: char| c.is_whitespace() || c == ':'));
            let Some(v) = verb else { continue };
            let app = app_part(&lower[from..j]);
            if bad_target(&app) {
                return None;
            }
            let text = tidy_text(&raw[after + v.len()..]);
            if text.is_empty() {
                return None;
            }
            return Some(act("app.write", json!({ "app": app, "text": text })));
        }
    }
    let verb = WRITE_VERBS.iter().find(|v| body.starts_with(**v) && body[v.len()..].starts_with(|c: char| c.is_whitespace() || c == ':'))?;
    let start = lead_end + verb.len();
    for marker in [" in ", " into ", " on ", " inside "] {
        let Some(rel) = lower[start..].rfind(marker) else { continue };
        let m = start + rel;
        let app = app_part(&lower[m + marker.len()..]);
        if !WRITE_APPS.contains(&app.as_str()) {
            continue;
        }
        let text = tidy_text(&raw[start..m]);
        if text.is_empty() {
            return None;
        }
        return Some(act("app.write", json!({ "app": app, "text": text })));
    }
    None
}

fn vertical_word(w: &str) -> Option<&'static str> {
    Some(match w {
        "image" | "images" | "photos" | "photo" | "pictures" | "picture" | "pics" => "images",
        "video" | "videos" => "videos",
        "news" => "news",
        "shopping" => "shopping",
        "map" | "maps" => "maps",
        _ => return None,
    })
}

pub fn vertical(t: &str) -> Option<Intent> {
    let p = crate::plan::clean(t);
    let words: Vec<&str> = p.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        let Some(kind) = vertical_word(w) else { continue };
        let next = words.get(i + 1).copied().unwrap_or("");
        if matches!(next, "tab" | "section" | "results" | "result") && words.iter().take(i).any(|x| matches!(*x, "open" | "show" | "click" | "switch" | "go" | "see" | "view" | "take")) {
            return Some(act("page.vertical", json!({ "kind": kind })));
        }
    }
    let leads: &[(&str, bool)] = &[
        ("open up the ", true),
        ("open the ", true),
        ("show me the ", true),
        ("show the ", true),
        ("go to the ", true),
        ("switch to the ", true),
        ("click on the ", true),
        ("click the ", true),
        ("take me to the ", true),
        ("see the ", true),
        ("view the ", true),
        ("switch to ", false),
        ("click on ", false),
        ("open ", false),
        ("show me ", false),
        ("show ", false),
        ("go to ", false),
    ];
    for (l, the) in leads {
        let Some(rest) = p.strip_prefix(l) else { continue };
        let Some(kind) = vertical_word(rest) else { continue };
        let ok = match kind {
            "images" => *the || matches!(rest, "images" | "image"),
            "videos" | "news" | "shopping" => *the && !matches!(rest, "photos" | "photo" | "pictures" | "picture"),
            _ => false,
        };
        if ok && !(matches!(rest, "photos" | "photo" | "pictures" | "picture" | "pics") && !*the) {
            return Some(act("page.vertical", json!({ "kind": kind })));
        }
        return None;
    }
    None
}

static LAST_SEARCH: Mutex<String> = Mutex::new(String::new());

pub fn remember_search(q: &str) {
    if !q.trim().is_empty() {
        *LAST_SEARCH.lock().unwrap() = q.trim().to_owned();
    }
}

pub fn last_search() -> Option<String> {
    let q = LAST_SEARCH.lock().unwrap().clone();
    (!q.is_empty()).then_some(q)
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1.split('#').next().unwrap_or("");
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| decode(v))
    })
}

pub fn search_of(url: &str) -> Option<(String, String)> {
    let host = crate::tools::web::host(url).to_lowercase();
    let path = url.split("://").nth(1).unwrap_or(url).split_once('/').map(|(_, p)| p).unwrap_or("").split(['?', '#']).next().unwrap_or("").to_owned();
    let engine = if host.starts_with("google.") || host.contains(".google.") {
        "google"
    } else if host == "bing.com" || host.ends_with(".bing.com") {
        "bing"
    } else if host == "duckduckgo.com" || host.ends_with(".duckduckgo.com") {
        "duckduckgo"
    } else if host == "search.brave.com" {
        "brave"
    } else if host.ends_with("search.yahoo.com") {
        "yahoo"
    } else {
        return None;
    };
    let q = match engine {
        "yahoo" => query_param(url, "p"),
        _ => query_param(url, "q"),
    }?;
    let _ = path;
    (!q.trim().is_empty()).then(|| (engine.to_owned(), q))
}

pub fn vertical_url(engine: &str, host: &str, q: &str, kind: &str) -> String {
    let e = crate::tools::web::encode(q);
    let host = if host.is_empty() { "www.google.com".to_owned() } else { host.to_owned() };
    match (engine, kind) {
        ("bing", "images") => format!("https://www.bing.com/images/search?q={e}"),
        ("bing", "videos") => format!("https://www.bing.com/videos/search?q={e}"),
        ("bing", "news") => format!("https://www.bing.com/news/search?q={e}"),
        ("bing", "shopping") => format!("https://www.bing.com/shop?q={e}"),
        ("bing", "maps") => format!("https://www.bing.com/maps?q={e}"),
        ("duckduckgo", "images") => format!("https://duckduckgo.com/?q={e}&ia=images&iax=images"),
        ("duckduckgo", "videos") => format!("https://duckduckgo.com/?q={e}&ia=videos&iax=videos"),
        ("duckduckgo", "news") => format!("https://duckduckgo.com/?q={e}&ia=news&iar=news"),
        ("duckduckgo", _) => format!("https://duckduckgo.com/?q={e}"),
        ("brave", "images") => format!("https://search.brave.com/images?q={e}"),
        ("brave", "videos") => format!("https://search.brave.com/videos?q={e}"),
        ("brave", "news") => format!("https://search.brave.com/news?q={e}"),
        ("brave", _) => format!("https://search.brave.com/search?q={e}"),
        ("yahoo", "images") => format!("https://images.search.yahoo.com/search/images?p={e}"),
        ("yahoo", "videos") => format!("https://video.search.yahoo.com/search/video?p={e}"),
        ("yahoo", _) => format!("https://news.search.yahoo.com/search?p={e}"),
        (_, "maps") => format!("https://www.google.com/maps/search/{}", e.replace('+', "%20")),
        (_, "videos") => format!("https://{host}/search?q={e}&tbm=vid"),
        (_, "news") => format!("https://{host}/search?q={e}&tbm=nws"),
        (_, "shopping") => format!("https://{host}/search?q={e}&udm=28"),
        _ => format!("https://{host}/search?q={e}&udm=2"),
    }
}

pub fn vertical_label(kind: &str) -> &'static str {
    match kind {
        "videos" => "Videos",
        "news" => "News",
        "shopping" => "Shopping",
        "maps" => "Maps",
        _ => "Images",
    }
}

fn is_browser(app: &str) -> bool {
    let a = app.to_lowercase();
    ["chrome", "chromium", "brave", "edge", "firefox", "opera", "vivaldi", "safari", "librewolf", "zen"].iter().any(|w| a.contains(w))
}

fn is_address(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("address") || n.contains("search or enter") || n.contains("search with") || n.contains("enter address") || n == "url"
}

pub async fn run_vertical(core: &std::sync::Arc<crate::core::Core>, kind: &str) -> crate::tools::Outcome {
    use crate::tools::Outcome;
    let layer = "browser";
    let label = vertical_label(kind);
    let wins = crate::os::windows().await.unwrap_or_default();
    let browser = wins.iter().find(|w| w.front && is_browser(&w.app)).or_else(|| wins.iter().find(|w| is_browser(&w.app))).cloned();
    if let Some(w) = &browser {
        crate::os::wake(w).await;
        if let Ok(nodes) = crate::os::tree(w, crate::os::Scope { limit: 800, text: false, hidden: false }).await {
            let addr = nodes.iter().find(|n| n.role == "field" && is_address(&n.name)).cloned();
            let raw_url = addr.as_ref().map(|n| n.value.trim().to_owned()).unwrap_or_default();
            let url = if raw_url.is_empty() || raw_url.contains("://") { raw_url } else { format!("https://{raw_url}") };
            if let Some((engine, q)) = search_of(&url) {
                remember_search(&q);
                let host = url.split("://").nth(1).unwrap_or("").split(['/', '?', '#']).next().unwrap_or("").to_owned();
                let target = vertical_url(&engine, &host, &q, kind);
                if let Some(a) = &addr
                    && crate::os::type_into(w, a, &target, true).await.is_ok()
                    && crate::tools::input::press(&[crate::tools::input::K::Enter]).await.is_ok()
                {
                    return Outcome::ok_with(&format!("Opened {label} for {q}"), layer, Some(w.app.clone()));
                }
                let b = crate::tools::web::find_browser(core, Some(&w.app)).or_else(|| crate::tools::web::find_browser(core, None));
                let c = core.clone();
                if matches!(crate::layers::blocking(move || crate::tools::web::open(&c, b.as_ref(), &target)).await, Ok(Ok(_))) {
                    return Outcome::ok_with(&format!("Opened {label} for {q}"), layer, Some(w.app.clone()));
                }
            }
            if let Some(n) = crate::os::find(&nodes, label, &["link", "tab", "button"])
                && crate::os::act(n, crate::os::Do::Invoke).await.is_ok()
            {
                return Outcome::ok_with(&format!("Opened {label}"), layer, Some(w.app.clone()));
            }
        }
    }
    if let Some(q) = last_search() {
        let target = vertical_url("google", "www.google.com", &q, kind);
        let b = crate::tools::web::find_browser(core, None);
        let c = core.clone();
        if matches!(crate::layers::blocking(move || crate::tools::web::open(&c, b.as_ref(), &target)).await, Ok(Ok(_))) {
            return Outcome::ok_with(&format!("Opened {label} for {q}"), layer, None);
        }
    }
    Outcome::fail(&format!("Search for something first, then say open the {}.", label.to_lowercase()), layer)
}

pub fn rules(t: &str, raw: &str) -> Option<Intent> {
    vertical(t).or_else(|| write_in_app(raw)).or_else(|| create(t, raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::normalize;

    fn run(raw: &str) -> Option<(String, Value)> {
        match rules(&normalize(raw), raw) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    fn paths(v: &Value) -> Vec<(String, String)> {
        v["items"].as_array().unwrap().iter().map(|i| (i["kind"].as_str().unwrap().to_owned(), i["path"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_owned()).collect::<Vec<_>>().join("/"))).collect()
    }

    fn f(k: &str, p: &str) -> (String, String) {
        (k.to_owned(), p.to_owned())
    }

    #[test]
    fn folder_with_file_inside() {
        let (tool, a) = run("create a folder called tutorial and inside it I want a file called tut1.cpp").unwrap();
        assert_eq!(tool, "files.create");
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("file", "tutorial/tut1.cpp")]);
        assert_eq!(a["code"], true);
        assert_eq!(a["where"], "");
        let (_, a) = run("Create a file called tut1.cpp inside a new folder called tutorial").unwrap();
        assert_eq!(paths(&a), vec![f("file", "tutorial/tut1.cpp"), f("folder", "tutorial")]);
        let (_, a) = run("in the tutorial folder create a file called notes.txt").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("file", "tutorial/notes.txt")]);
        assert_eq!(a["code"], false);
        let (_, a) = run("make a folder named tutorial with a file named main dot py").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("file", "tutorial/main.py")]);
        let (_, a) = run("create a file tut1.cpp inside tutorial").unwrap();
        assert_eq!(paths(&a), vec![f("file", "tut1.cpp")]);
        assert_eq!(a["inside"], "tutorial");
        let (_, a) = run("create a folder called tutorial, and inside that create a folder called day1 and inside it a file called main.cpp").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("folder", "tutorial/day1"), f("file", "tutorial/day1/main.cpp")]);
    }

    #[test]
    fn places_and_plain_folders() {
        let (_, a) = run("create a folder called Projects 2025 on my desktop").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "Projects 2025")]);
        assert_eq!(a["where"], "desktop");
        let (_, a) = run("make a folder called tutorial in d drive").unwrap();
        assert_eq!(a["where"], "d drive");
        let (_, a) = run("create a new folder").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "New folder")]);
        let (_, a) = run("create a file called tut1 cpp inside tutorial").unwrap();
        assert_eq!(paths(&a), vec![f("file", "tut1.cpp")]);
        let (_, a) = run("create a python file called hello").unwrap();
        assert_eq!(paths(&a), vec![f("file", "hello.py")]);
        assert_eq!(a["code"], true);
        let (_, a) = run("create a file called shopping ideas.txt on the desktop").unwrap();
        assert_eq!(paths(&a), vec![f("file", "shopping ideas.txt")]);
        assert_eq!(a["where"], "desktop");
        let (_, a) = run("create a file inside it called notes.md").unwrap();
        assert_eq!(a["inside"], "it");
        let (_, a) = run("in vs code create a new folder called tutorial, as I want to start with C++").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "tutorial")]);
        assert_eq!(a["code"], true);
        assert_eq!(a["language"], "c++");
        let (_, a) = run("create the tutorial folder and put a c++ file named tut1 in it").unwrap();
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("file", "tutorial/tut1.cpp")]);
    }

    #[test]
    fn leaves_other_requests_alone() {
        assert!(run("open the file called resume").is_none());
        assert!(run("delete the folder called old").is_none());
        assert!(run("create a new python project called weather app").is_none());
        assert!(run("find the file resume").is_none());
        assert!(run("open file explorer").is_none());
        assert!(run("make a to do list of these things").is_none());
        assert!(run("take a note that the file is ready").is_none());
    }

    #[test]
    fn writes_into_apps() {
        let (tool, a) = run("in notepad write\n2 hour coding\ndrinking water\nand playing outside").unwrap();
        assert_eq!(tool, "app.write");
        assert_eq!(a["app"], "notepad");
        assert_eq!(a["text"], "2 hour coding\ndrinking water\nand playing outside");
        let (_, a) = run("open notepad and write Hello World, how are you?").unwrap();
        assert_eq!(a["app"], "notepad");
        assert_eq!(a["text"], "Hello World, how are you?");
        let (_, a) = run("Write I will be late today in notepad").unwrap();
        assert_eq!(a["text"], "I will be late today");
        let (_, a) = run("can you open word and type: Dear Sir").unwrap();
        assert_eq!(a["app"], "word");
        assert_eq!(a["text"], "Dear Sir");
        assert!(run("type hello in the search box").is_none());
        assert!(run("in vs code write a function that adds two numbers").is_none());
        assert!(run("write hello").is_none());
        assert!(run("in whatsapp write hi").is_none());
    }

    #[test]
    fn switches_search_tabs() {
        assert_eq!(run("now I want u to open the images").unwrap().1["kind"], "images");
        assert_eq!(run("the web page where mr beast was searched can u open the image tab").unwrap().1["kind"], "images");
        assert_eq!(run("show me the videos").unwrap().1["kind"], "videos");
        assert_eq!(run("go to the news tab").unwrap().1["kind"], "news");
        assert!(run("open the third image").is_none());
        assert!(run("open maps").is_none());
        assert!(run("open my pictures folder").is_none());
        assert!(run("open photos").is_none());
        assert!(run("open news").is_none());
        assert!(run("show pictures").is_none());
        assert_eq!(run("open images").unwrap().1["kind"], "images");
        assert_eq!(run("show me the news tab").unwrap().1["kind"], "news");
    }

    fn routed(raw: &str) -> Option<(String, Value)> {
        match crate::router::rules(raw, "stop", false) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn sentences_from_the_windows_test_route_correctly() {
        assert_eq!(routed("now I want to message binod").unwrap().0, "messages.open");
        assert_eq!(routed("now I want u to open the images").unwrap(), ("page.vertical".into(), json!({ "kind": "images" })));
        assert_eq!(routed("the web page where mr beast was searched can u open the image tab").unwrap().0, "page.vertical");
        let (tool, a) = routed("I want to create a to-do list, I will do 2 hours of coding, in these two hours I will not open anything except tutorials").unwrap();
        assert_eq!((tool.as_str(), a["op"].as_str()), ("board.todo", Some("many")));
        let items = crate::board::split_tasks(a["text"].as_str().unwrap());
        assert_eq!(items.first().map(String::as_str), Some("2 hours of coding"), "{items:?}");
        let (tool, a) = routed("in notepad write\n2 hour coding\ndrinking water\nand playing outside").unwrap();
        assert_eq!(tool, "app.write");
        assert_eq!(a["text"], "2 hour coding\ndrinking water\nand playing outside");
        let (tool, a) = routed("create a folder called tutorial and inside it I want a file called tut1.cpp").unwrap();
        assert_eq!(tool, "files.create");
        assert_eq!(paths(&a), vec![f("folder", "tutorial"), f("file", "tutorial/tut1.cpp")]);
        assert_eq!(routed("search for mr beast").unwrap().0, "web.search");
        assert_eq!(routed("create a new python project called weather app").unwrap().0, "code.project");
        assert_eq!(routed("create a file called tut1.cpp").unwrap().0, "files.create");
        assert_eq!(routed("make a folder on the desktop called receipts").unwrap().1["where"], "desktop");
        assert_eq!(routed("take a note that the file is ready").unwrap().0, "board.note");
        assert_eq!(routed("type hello in the search box").unwrap().0, "ui.fill");
    }

    #[test]
    fn rewrites_search_urls() {
        let (e, q) = search_of("https://www.google.com/search?q=mr+beast&sca_esv=1").unwrap();
        assert_eq!((e.as_str(), q.as_str()), ("google", "mr beast"));
        assert_eq!(vertical_url(&e, "www.google.com", &q, "images"), "https://www.google.com/search?q=mr+beast&udm=2");
        assert_eq!(search_of("https://www.bing.com/search?q=cats%20and%20dogs").unwrap().1, "cats and dogs");
        assert!(search_of("https://www.youtube.com/results?search_query=x").is_none());
        assert_eq!(vertical_url("duckduckgo", "", "a b", "videos"), "https://duckduckgo.com/?q=a+b&ia=videos&iax=videos");
    }
}
