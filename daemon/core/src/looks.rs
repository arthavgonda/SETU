use crate::core::Core;
use crate::router::Intent;
use crate::text::normalize;
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

mod list {
    include!(concat!(env!("OUT_DIR"), "/looks.rs"));
}

const LAYER: &str = "browser";
pub const NEUTRAL: &str = "nature";
const PICTURE_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const MAX_PICTURE: u64 = 20 << 20;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Old {
    pub dir: String,
    pub name: String,
    pub email: String,
    pub current: bool,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: u64,
    pub browser: String,
    pub display: String,
    pub key: String,
    pub family: String,
    pub avatar: usize,
    pub background: usize,
    pub custom: Option<PathBuf>,
    pub step: &'static str,
    pub name: String,
    pub final_name: String,
    pub taken: Vec<String>,
    pub resume: Value,
    pub edit: bool,
    pub old: Vec<Old>,
}

static OPEN: Mutex<Option<Session>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn avatars() -> &'static [&'static str] {
    list::AVATARS
}

pub fn backgrounds() -> &'static [&'static str] {
    list::BACKGROUNDS
}

pub fn set_of(avatar: &str) -> Option<&str> {
    avatar.split_once('_').map(|(_, s)| s).filter(|s| !s.is_empty())
}

pub fn background_set(bg: &str) -> &str {
    match bg.rsplit_once('_') {
        Some((set, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => set,
        _ => bg,
    }
}

fn split_number(w: &str) -> (&str, &str) {
    let cut = w.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    (&w[..cut], &w[cut..])
}

fn word_case(w: &str) -> String {
    if w.len() <= 3 && w.chars().all(|c| c.is_ascii_uppercase()) {
        return w.to_owned();
    }
    let lower = w.to_lowercase();
    crate::text::sentence_case(&lower)
}

pub fn avatar_label(avatar: &str) -> String {
    let base = avatar.split_once('_').map(|(b, _)| b).unwrap_or(avatar);
    let (word, n) = split_number(base);
    let n = n.trim_start_matches('0');
    if n.is_empty() { word_case(word) } else { format!("{} {n}", word_case(word)) }
}

pub fn background_label(bg: &str) -> String {
    let set = background_set(bg);
    let n = bg[set.len()..].trim_start_matches('_').trim_start_matches('0');
    if n.is_empty() { word_case(set) } else { format!("{} {n}", word_case(set)) }
}

pub fn background_for(avatar: &str) -> String {
    let bgs = backgrounds();
    if let Some(set) = set_of(avatar)
        && let Some(b) = bgs.iter().find(|b| background_set(b).eq_ignore_ascii_case(set))
    {
        return (*b).to_owned();
    }
    if bgs.contains(&NEUTRAL) {
        return NEUTRAL.to_owned();
    }
    bgs.first().map(|b| (*b).to_owned()).unwrap_or_default()
}

const NUMBER_WORDS: &[&str] = &["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

fn spoken_forms(label: &str) -> Vec<String> {
    let n = normalize(label);
    let mut out = vec![n.clone()];
    if let Some((w, d)) = n.rsplit_once(' ')
        && let Ok(k) = d.parse::<usize>()
        && let Some(word) = NUMBER_WORDS.get(k)
    {
        out.push(format!("{w} {word}"));
        out.push(format!("{w} number {word}"));
    }
    out
}

pub fn find_avatar(q: &str) -> Option<usize> {
    let want = clean_pick(q);
    if want.is_empty() {
        return None;
    }
    let list = avatars();
    if let Some(i) = list.iter().position(|a| spoken_forms(&avatar_label(a)).contains(&want) || normalize(a) == want) {
        return Some(i);
    }
    list.iter()
        .enumerate()
        .map(|(i, a)| (crate::os::score(&want, &normalize(&avatar_label(a))), i))
        .filter(|(s, _)| *s >= 0.8)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, i)| i)
}

pub fn find_background(q: &str) -> Option<usize> {
    let want = clean_pick(q);
    let want = want.trim_end_matches(" background").trim_start_matches("background ").trim().to_owned();
    if want.is_empty() {
        return None;
    }
    let list = backgrounds();
    if let Some(i) = list.iter().position(|b| spoken_forms(&background_label(b)).contains(&want)) {
        return Some(i);
    }
    list.iter().position(|b| normalize(background_set(b)) == want)
}

fn clean_pick(q: &str) -> String {
    let mut t = normalize(q);
    for p in ["the ", "a ", "that ", "this ", "use ", "pick ", "choose ", "show me ", "show "] {
        if let Some(r) = t.strip_prefix(p) {
            t = r.to_owned();
        }
    }
    for s in [" please", " one", " picture", " avatar", " photo"] {
        if let Some(r) = t.strip_suffix(s) {
            t = r.to_owned();
        }
    }
    t.trim().to_owned()
}

pub fn final_name(typed: &str, taken: &[String]) -> String {
    let name = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return String::new();
    }
    let used = |n: &str| taken.iter().any(|t| normalize(t) == normalize(n));
    if !used(&name) {
        return name;
    }
    let first = format!("{name}_setu");
    if !used(&first) {
        return first;
    }
    (2..).map(|k| format!("{name}_setu_{k}")).find(|n| !used(n)).unwrap_or(first)
}

pub fn spoken_name(raw: &str) -> String {
    let t = raw.trim();
    let lower = t.to_lowercase();
    let mut rest = t;
    for p in ["call it ", "name it ", "call the profile ", "name the profile ", "it's called ", "it is called ", "its name is ", "the name is ", "call me ", "make it ", "use "] {
        if lower.starts_with(p) {
            rest = &t[p.len()..];
            break;
        }
    }
    let rest = rest.trim().trim_end_matches(['.', '!', '?']).trim();
    if rest.chars().any(|c| c.is_uppercase()) {
        return rest.to_owned();
    }
    rest.split_whitespace().map(word_case).collect::<Vec<_>>().join(" ")
}

pub fn current() -> Option<Session> {
    OPEN.lock().unwrap().clone()
}

fn store(s: Session) {
    *OPEN.lock().unwrap() = Some(s);
}

pub fn state() -> Value {
    match current() {
        Some(s) => state_of(&s),
        None => json!({ "open": false }),
    }
}

pub fn state_of(s: &Session) -> Value {
    let pics: Vec<Value> = avatars().iter().map(|a| json!({ "id": a, "label": avatar_label(a), "set": set_of(a).unwrap_or("") })).collect();
    let bgs: Vec<Value> = backgrounds().iter().map(|b| json!({ "id": b, "label": background_label(b), "set": background_set(b) })).collect();
    let old: Vec<Value> = s.old.iter().map(|o| json!({ "dir": o.dir, "name": o.name, "email": o.email, "current": o.current, "bytes": o.bytes })).collect();
    json!({
        "open": true,
        "id": s.id,
        "browser": s.browser,
        "display": s.display,
        "family": s.family,
        "step": s.step,
        "edit": s.edit,
        "avatars": pics,
        "backgrounds": bgs,
        "avatar": avatars().get(s.avatar).copied().unwrap_or(""),
        "avatarLabel": s.custom.as_ref().map(|_| "Your picture".to_owned()).unwrap_or_else(|| avatars().get(s.avatar).map(|a| avatar_label(a)).unwrap_or_default()),
        "background": backgrounds().get(s.background).copied().unwrap_or(""),
        "custom": s.custom.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        "name": s.name,
        "finalName": s.final_name,
        "taken": s.taken,
        "profiles": old,
    })
}

fn emit(core: &Core) {
    core.hub.emit("look.state", state());
}

pub fn close(core: &Core) {
    if OPEN.lock().unwrap().take().is_some() {
        emit(core);
    }
}

fn index_of(list: &[&str], id: &str) -> Option<usize> {
    list.iter().position(|x| *x == id)
}

pub fn saved(dir: &Path) -> Value {
    std::fs::read_to_string(dir.join(".setu-profile")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

pub fn open(core: &Arc<Core>, app: &crate::tools::apps::App, key: &str, resume: Value, edit: bool) -> Outcome {
    let display = crate::tools::browsers::display(key, &app.name);
    let (_, old) = crate::profiles::read(key);
    let taken: Vec<String> = old.iter().map(|p| p.name.clone()).filter(|n| !n.is_empty()).collect();
    let mut s = Session {
        id: NEXT.fetch_add(1, Ordering::SeqCst),
        browser: app.name.clone(),
        display: display.clone(),
        key: key.to_owned(),
        family: crate::tools::browsers::family(key).to_owned(),
        avatar: 0,
        background: 0,
        custom: None,
        step: "look",
        name: String::new(),
        final_name: String::new(),
        taken,
        resume,
        edit,
        old: old.iter().map(|p| Old { dir: p.dir.clone(), name: p.name.clone(), email: p.email.clone(), current: p.current, bytes: None }).collect(),
    };
    let first = avatars().first().copied().unwrap_or("");
    s.background = index_of(backgrounds(), &background_for(first)).unwrap_or(0);
    if edit {
        let dir = crate::cdp::profile_dir(&core.paths.data, key);
        let v = saved(&dir);
        let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        if let Some(i) = index_of(avatars(), &get("avatar")) {
            s.avatar = i;
        }
        if let Some(i) = index_of(backgrounds(), &get("background")) {
            s.background = i;
        }
        if !get("custom").is_empty() && dir.join(get("custom")).exists() {
            s.custom = Some(dir.join(get("custom")));
        }
        s.name = get("name");
        s.final_name = get("name");
    }
    let line = if edit {
        "Here's your Setu profile's look. Say next, previous, or a name like the cat, then this one.".to_owned()
    } else {
        format!("Pick a picture for your {display} Setu profile. Say next, previous, or a name like the cat. Say this one when you like it.")
    };
    store(s.clone());
    emit(core);
    prompt(&s, line)
}

fn act(op: &str, value: &str) -> Intent {
    Intent::Act { tool: "look.step".into(), args: json!({ "op": op, "value": value }) }
}

pub fn choices(s: &Session) -> Vec<Choice> {
    match s.step {
        "look" => {
            let mut c = vec![
                Choice::new("This one", &["this one", "this", "that one", "yes", "okay", "ok", "choose this", "pick this", "pick this one", "done", "confirm", "looks good", "perfect", "i like it", "i like this one", "keep it", "keep this"], act("confirm", "")),
                Choice::new("Next", &["next", "next one", "right", "another", "another one", "show me another", "something else", "go right"], act("next", "")),
                Choice::new("Previous", &["previous", "previous one", "back", "left", "go back", "go left", "the last one"], act("prev", "")),
                Choice::new("Next background", &["next background", "change the background", "change background", "another background", "different background", "a different background", "new background"], act("bg_next", "")),
                Choice::new("Previous background", &["previous background", "last background", "the background before"], act("bg_prev", "")),
            ];
            for a in avatars() {
                let label = avatar_label(a);
                let mut words: Vec<String> = spoken_forms(&label);
                words.extend(spoken_forms(&label).iter().map(|w| format!("the {w}")));
                let mut ch = Choice::new(&label, &[], act("avatar", a));
                ch.words = words;
                c.push(ch);
            }
            for b in backgrounds() {
                let label = background_label(b);
                let mut words = Vec::new();
                for f in spoken_forms(&label) {
                    words.push(format!("{f} background"));
                    words.push(format!("the {f} background"));
                    words.push(format!("background {f}"));
                }
                let mut ch = Choice::new(&format!("{label} background"), &[], act("bg", b));
                ch.words = words;
                c.push(ch);
            }
            c
        }
        "name" if !s.final_name.is_empty() => vec![
            Choice::new("Create it", &["yes", "create it", "create", "okay", "ok", "sure", "go ahead", "do it", "make it", "sounds good", "perfect"], act("create", "")),
            Choice::new("Change the name", &["no", "change it", "change the name", "another name", "rename", "rename it", "different name"], act("rename", "")),
            Choice::new("Change the picture", &["go back", "back", "change the picture", "another picture", "change the look"], act("back", "")),
        ],
        _ => Vec::new(),
    }
}

fn prompt(s: &Session, line: String) -> Outcome {
    match s.step {
        "look" => Outcome::asking("Showed the profile pictures", LAYER, FollowUp { question: line.clone(), spoken: Some(line), choices: choices(s), cleanup: None, kind: "look", focus: None }),
        "name" if !s.final_name.is_empty() => {
            let q = if normalize(&s.final_name) != normalize(&s.name) {
                format!("You already have a profile called {}, so this one will be called {}. Shall I create it?", s.name, s.final_name)
            } else {
                format!("It will be called {}. Shall I create it?", s.final_name)
            };
            let q = if line.is_empty() { q } else { format!("{line} {q}") };
            Outcome::asking("Asked to create the profile", LAYER, FollowUp { question: q.clone(), spoken: Some(q), choices: choices(s), cleanup: None, kind: "look-confirm", focus: None })
        }
        _ => {
            let q = if line.is_empty() { "What should the profile be called? Type it in the window, or say call it, then the name.".to_owned() } else { line };
            Outcome::asking("Asked for the profile's name", LAYER, FollowUp { question: q.clone(), spoken: Some(q), choices: Vec::new(), cleanup: None, kind: "look-name", focus: None })
        }
    }
}

fn wrap(i: usize, d: i64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((i as i64 + d).rem_euclid(n as i64)) as usize
}

fn valid_picture(p: &Path) -> Result<(), String> {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !PICTURE_TYPES.contains(&ext.as_str()) {
        return Err("That file isn't a picture I can use. Pick a PNG, JPEG, WebP, BMP or GIF.".into());
    }
    let m = std::fs::metadata(p).map_err(|_| "I couldn't open that picture.".to_owned())?;
    if !m.is_file() {
        return Err("I couldn't open that picture.".into());
    }
    if m.len() > MAX_PICTURE {
        return Err("That picture is over 20 MB. Pick a smaller one.".into());
    }
    Ok(())
}

pub fn change(core: &Core, op: &str, value: &str) -> Result<Session, String> {
    let mut g = OPEN.lock().unwrap();
    let s = g.as_mut().ok_or_else(|| "The profile window is closed.".to_owned())?;
    let (na, nb) = (avatars().len(), backgrounds().len());
    match op {
        "next" | "prev" => {
            s.avatar = wrap(s.avatar, if op == "next" { 1 } else { -1 }, na);
            s.custom = None;
            if let Some(a) = avatars().get(s.avatar) {
                s.background = index_of(backgrounds(), &background_for(a)).unwrap_or(s.background);
            }
        }
        "avatar" => {
            let i = index_of(avatars(), value).or_else(|| find_avatar(value)).ok_or_else(|| format!("I don't have a picture called {value}."))?;
            s.avatar = i;
            s.custom = None;
            s.background = index_of(backgrounds(), &background_for(avatars()[i])).unwrap_or(s.background);
        }
        "bg_next" | "bg_prev" => s.background = wrap(s.background, if op == "bg_next" { 1 } else { -1 }, nb),
        "bg" => s.background = index_of(backgrounds(), value).or_else(|| find_background(value)).ok_or_else(|| format!("I don't have a background called {value}."))?,
        "custom" => {
            let p = PathBuf::from(value.strip_prefix("file://").unwrap_or(value));
            valid_picture(&p)?;
            s.custom = Some(p);
        }
        "confirm" => {
            if s.edit {
                s.step = "save";
            } else {
                s.step = "name";
            }
        }
        "back" => {
            s.step = "look";
            s.final_name.clear();
        }
        "name" => {
            let n = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if n.chars().count() > 60 {
                return Err("That name is too long. Keep it under 60 letters.".into());
            }
            if n.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')) {
                return Err("A profile name can't have slashes in it.".into());
            }
            s.step = "name";
            s.name = n.clone();
            s.final_name = final_name(&n, &s.taken);
        }
        "rename" => {
            s.step = "name";
            s.name.clear();
            s.final_name.clear();
        }
        "import" => s.step = "import",
        "making" => s.step = "making",
        _ => return Err("I don't know that step.".into()),
    }
    let out = s.clone();
    drop(g);
    emit(core);
    Ok(out)
}

fn sizes(core: &Arc<Core>, key: &str) {
    let core = core.clone();
    let key = key.to_owned();
    tokio::spawn(async move {
        let k = key.clone();
        let found = crate::work::background("sizes", move || {
            let (root, old) = crate::profiles::read(&k);
            let root = root.unwrap_or_default();
            old.iter()
                .map(|p| {
                    let dir = if Path::new(&p.dir).is_absolute() { PathBuf::from(&p.dir) } else { root.join(&p.dir) };
                    (p.dir.clone(), crate::import::size_of(&dir))
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let mut g = OPEN.lock().unwrap();
        if let Some(s) = g.as_mut().filter(|s| s.key == key) {
            for o in s.old.iter_mut() {
                o.bytes = found.iter().find(|(d, _)| *d == o.dir).map(|(_, b)| *b);
            }
            drop(g);
            emit(&core);
        }
    });
}

pub fn chosen(s: &Session) -> Value {
    json!({
        "name": s.final_name,
        "avatar": if s.custom.is_some() { "" } else { avatars().get(s.avatar).copied().unwrap_or("") },
        "background": backgrounds().get(s.background).copied().unwrap_or(""),
        "custom": s.custom.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
    })
}

pub fn apply(dir: &Path, family: &str, look: &Value) -> anyhow::Result<()> {
    let mut mark = saved(dir);
    if !mark.is_object() {
        mark = json!({});
    }
    let get = |k: &str| look.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let name = get("name");
    if !name.is_empty() {
        mark["name"] = json!(name);
    }
    mark["avatar"] = json!(get("avatar"));
    mark["background"] = json!(get("background"));
    let custom = get("custom");
    if custom.is_empty() {
        mark["custom"] = json!("");
    } else {
        let src = PathBuf::from(&custom);
        if src.parent() != Some(dir) {
            let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            let file = format!(".setu-avatar.{ext}");
            for e in PICTURE_TYPES {
                let _ = std::fs::remove_file(dir.join(format!(".setu-avatar.{e}")));
            }
            std::fs::copy(&src, dir.join(&file))?;
            mark["custom"] = json!(file);
        }
    }
    std::fs::write(dir.join(".setu-profile"), mark.to_string())?;
    if family == "chromium" && !name.is_empty() {
        name_chromium(dir, &name)?;
    }
    Ok(())
}

pub fn name_chromium(dir: &Path, name: &str) -> anyhow::Result<()> {
    let ls = dir.join("Local State");
    let mut v: Value = std::fs::read_to_string(&ls).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    if !v.is_object() {
        v = json!({});
    }
    if !v.get("profile").is_some_and(Value::is_object) {
        v["profile"] = json!({});
    }
    if !v["profile"].get("info_cache").is_some_and(Value::is_object) {
        v["profile"]["info_cache"] = json!({});
    }
    if !v["profile"]["info_cache"].get("Default").is_some_and(Value::is_object) {
        v["profile"]["info_cache"]["Default"] = json!({});
    }
    v["profile"]["info_cache"]["Default"]["name"] = json!(name);
    v["profile"]["info_cache"]["Default"]["is_using_default_name"] = json!(false);
    std::fs::write(&ls, v.to_string())?;
    let prefs = dir.join("Default").join("Preferences");
    std::fs::create_dir_all(dir.join("Default"))?;
    let mut p: Value = std::fs::read_to_string(&prefs).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    if !p.is_object() {
        p = json!({});
    }
    if !p.get("profile").is_some_and(Value::is_object) {
        p["profile"] = json!({});
    }
    p["profile"]["name"] = json!(name);
    std::fs::write(&prefs, p.to_string())?;
    Ok(())
}

pub fn reapply(dir: &Path, family: &str) {
    let v = saved(dir);
    if family == "chromium"
        && let Some(n) = v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())
        && let Err(e) = name_chromium(dir, n)
    {
        log::warn!("the Setu profile's name was not put back after the import: {e}");
    }
}

pub fn summary(data: &Path, key: &str) -> Option<Value> {
    if !crate::cdp::has_profile(data, key) {
        return None;
    }
    let dir = crate::cdp::profile_dir(data, key);
    let v = saved(&dir);
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let custom = get("custom");
    Some(json!({
        "name": get("name"),
        "avatar": get("avatar"),
        "background": get("background"),
        "custom": if custom.is_empty() { String::new() } else { dir.join(custom).to_string_lossy().into_owned() },
        "isDefault": crate::cdp::is_default(data, key),
    }))
}

const EDIT: &[&str] = &[
    "change my profile picture",
    "change the profile picture",
    "change my setu profile",
    "change my setu profile picture",
    "change my setu profile's look",
    "change my profile look",
    "change my profile's look",
    "change my browser profile picture",
    "change my avatar",
    "change the avatar",
    "change my profile background",
];

pub fn rules(t: &str) -> Option<Intent> {
    let Some(s) = current() else {
        return EDIT.contains(&t).then(|| act("edit", ""));
    };
    let cancel = ["cancel", "stop", "never mind", "nevermind", "close", "close it", "close the window", "not now", "forget it"];
    if cancel.contains(&t) {
        return Some(act("cancel", ""));
    }
    let confirming = s.step == "name" && !s.final_name.is_empty();
    match s.step {
        "look" => exact(t, &choices(&s)),
        "name" if confirming => exact(t, &choices(&s)),
        "import" => {
            let all: Vec<String> = s.old.iter().map(|o| o.dir.clone()).collect();
            let everything = ["bring everything", "everything", "all", "all of them", "import everything", "bring them all", "yes"];
            let nothing = ["start empty", "none", "nothing", "no", "empty", "keep it empty"];
            if everything.contains(&t) {
                let dirs = if s.family == "firefox" { s.old.iter().filter(|o| o.current).chain(s.old.iter()).take(1).map(|o| o.dir.clone()).collect() } else { all };
                return Some(Intent::Act { tool: "look.step".into(), args: json!({ "op": "import", "dirs": dirs }) });
            }
            if nothing.contains(&t) {
                return Some(Intent::Act { tool: "look.step".into(), args: json!({ "op": "import", "dirs": [] }) });
            }
            let want = t.trim_start_matches("just ").trim_start_matches("only ").trim_start_matches("bring ").trim_start_matches("the ").trim_end_matches(" profile");
            s.old.iter().find(|o| normalize(&o.name) == want).map(|o| Intent::Act { tool: "look.step".into(), args: json!({ "op": "import", "dirs": [o.dir], "main": o.dir }) })
        }
        "name" => {
            if matches!(t, "back" | "go back" | "change the picture") {
                return Some(act("back", ""));
            }
            let lower = t.to_lowercase();
            ["call it ", "name it ", "call the profile ", "name the profile ", "it's called ", "its name is ", "the name is "].iter().any(|p| lower.starts_with(p)).then(|| act("name", &spoken_name(t)))
        }
        _ => None,
    }
}

fn exact(t: &str, choices: &[Choice]) -> Option<Intent> {
    choices.iter().find(|c| normalize(&c.label) == t || c.words.iter().any(|w| normalize(w) == t)).map(|c| c.then.clone())
}

pub fn spoken_for(s: &Session, op: &str) -> String {
    let bg = backgrounds().get(s.background).map(|b| background_label(b)).unwrap_or_default();
    match op {
        "bg_next" | "bg_prev" | "bg" => format!("{bg} background."),
        "custom" => "Your own picture.".into(),
        _ => {
            let a = avatars().get(s.avatar).map(|a| avatar_label(a)).unwrap_or_default();
            if s.custom.is_some() { "Your own picture.".into() } else { format!("{a}.") }
        }
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    if tool != "look.step" {
        return Outcome::fail("I don't know that profile step.", LAYER);
    }
    let op = args.get("op").and_then(Value::as_str).unwrap_or_default().to_owned();
    let value = args.get("value").and_then(Value::as_str).unwrap_or_default().to_owned();
    if op == "edit" {
        let name = args.get("browser").and_then(Value::as_str).unwrap_or_default().to_owned();
        let Some(app) = crate::tools::web::find_browser(core, if name.is_empty() { None } else { Some(&name) }) else { return Outcome::fail("No browser is installed.", LAYER) };
        let Some(key) = crate::tools::browsers::key_of(&app) else { return Outcome::fail("That browser has no Setu profile.", LAYER) };
        if !crate::cdp::has_profile(&core.paths.data, key) {
            let d = crate::tools::browsers::display(key, &app.name);
            return Outcome::fail(&format!("{d} has no Setu profile yet. Say set up {d} first."), LAYER);
        }
        return open(core, &app, key, Value::Null, true);
    }
    let Some(s) = current() else { return Outcome::fail("The profile window is closed.", LAYER) };
    match op.as_str() {
        "cancel" if s.step == "import" => Box::pin(crate::browse::run(core, "browser.import", json!({ "browser": s.browser, "which": "none" }))).await,
        "cancel" if s.step == "making" => Outcome::ok_with("Still making the profile", LAYER, None).with_reply("I'm still making the profile. One moment.".into()),
        "cancel" => {
            close(core);
            if s.edit {
                return Outcome::ok_with("Kept the look", LAYER, None).with_reply("Okay, I left your Setu profile as it was.".into());
            }
            Outcome::ok_with("Didn't make a Setu profile", LAYER, None).with_reply(format!("Okay, I didn't make a Setu profile. Say set up {} when you want one.", s.display))
        }
        "create" | "save" => create(core, s).await,
        "import" => {
            close(core);
            let dirs: Vec<Value> = args.get("dirs").and_then(Value::as_array).cloned().unwrap_or_default();
            let which = if dirs.is_empty() { "none".to_owned() } else if dirs.len() == s.old.len() { "all".to_owned() } else { String::new() };
            let main = args.get("main").and_then(Value::as_str).unwrap_or_default();
            Box::pin(crate::browse::run(core, "browser.import", json!({ "browser": s.browser, "which": which, "dirs": dirs, "main": main }))).await
        }
        _ => {
            let typed = args.get("typed").and_then(Value::as_bool).unwrap_or(false);
            let v = if op == "name" && !typed { spoken_name(&value) } else { value.clone() };
            if op == "name" && v.trim().is_empty() {
                return prompt(&s, String::new());
            }
            match change(core, &op, &v) {
                Ok(n) if n.step == "save" => create(core, n).await,
                Ok(n) => {
                    let line = match op.as_str() {
                        "confirm" | "rename" | "name" => String::new(),
                        "back" => "Okay, pick a picture.".into(),
                        _ => spoken_for(&n, &op),
                    };
                    prompt(&n, line)
                }
                Err(e) => {
                    let mut o = prompt(&s, e.clone());
                    o.message = e;
                    o
                }
            }
        }
    }
}

async fn create(core: &Arc<Core>, s: Session) -> Outcome {
    if s.edit {
        let dir = crate::cdp::profile_dir(&core.paths.data, &s.key);
        let mut look = chosen(&s);
        look["name"] = json!("");
        let fam = s.family.clone();
        let r = crate::work::background("look", move || apply(&dir, &fam, &look)).await.and_then(|r| r);
        close(core);
        return match r {
            Ok(()) => Outcome::ok_with("Changed the profile's look", LAYER, None).with_reply("Done. Your Setu profile has its new look.".into()),
            Err(e) => Outcome::fail(&format!("I couldn't save the new look: {e}"), LAYER),
        };
    }
    if s.final_name.is_empty() {
        return prompt(&Session { step: "name", ..s }, String::new());
    }
    let look = chosen(&s);
    let _ = change(core, "making", "");
    let o = Box::pin(crate::browse::run(core, "browser.profile", json!({ "browser": s.browser, "op": "create", "look": look, "resume": s.resume }))).await;
    if o.ok && o.ask.as_ref().is_some_and(|f| f.kind == "look-import") {
        let _ = change(core, "import", "");
        sizes(core, &s.key);
    } else {
        close(core);
    }
    o
}

pub fn label(tool: &str, args: &Value) -> String {
    let _ = tool;
    match args.get("op").and_then(Value::as_str).unwrap_or_default() {
        "create" | "save" => "Making the Setu profile".into(),
        "import" => "Bringing your profiles over".into(),
        "cancel" => "Closing the profile window".into(),
        "name" => "Naming the profile".into(),
        _ => "Choosing the profile's look".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_pair_by_name() {
        assert_eq!(set_of("goku_DBZ"), Some("DBZ"));
        assert_eq!(set_of("cat"), None);
        assert_eq!(background_set("POKEMON_02"), "POKEMON");
        assert_eq!(background_set("DBZ"), "DBZ");
        assert_eq!(background_set("nature"), "nature");
        if backgrounds().contains(&"DBZ") {
            assert_eq!(background_for("goku_DBZ"), "DBZ");
        }
        if backgrounds().contains(&"POKEMON_01") {
            assert_eq!(background_for("pikachu2_POKEMON"), "POKEMON_01");
        }
        if backgrounds().contains(&NEUTRAL) {
            assert_eq!(background_for("cat"), NEUTRAL);
            assert_eq!(background_for("gohan_NOSUCHSET"), NEUTRAL);
        }
    }

    #[test]
    fn labels_read_well() {
        assert_eq!(avatar_label("goku_DBZ"), "Goku");
        assert_eq!(avatar_label("pikachu2_POKEMON"), "Pikachu 2");
        assert_eq!(avatar_label("cat"), "Cat");
        assert_eq!(background_label("POKEMON_01"), "Pokemon 1");
        assert_eq!(background_label("DBZ"), "DBZ");
        assert_eq!(background_label("nature"), "Nature");
    }

    #[test]
    fn a_taken_name_gets_the_setu_suffix() {
        let taken = vec!["Anoop".to_owned(), "Work".to_owned(), "Work_setu".to_owned()];
        assert_eq!(final_name("Anoop", &taken), "Anoop_setu");
        assert_eq!(final_name("anoop", &taken), "anoop_setu");
        assert_eq!(final_name("  Home   PC ", &taken), "Home PC");
        assert_eq!(final_name("Work", &taken), "Work_setu_2");
        assert_eq!(final_name("", &taken), "");
    }

    #[test]
    fn spoken_names_and_pictures_are_understood() {
        assert_eq!(spoken_name("call it anoop's laptop"), "Anoop's Laptop");
        assert_eq!(spoken_name("Name it DevBox"), "DevBox");
        assert_eq!(spoken_name("work"), "Work");
        if avatars().contains(&"cat") {
            assert_eq!(find_avatar("the cat").map(|i| avatars()[i]), Some("cat"));
        }
        if avatars().contains(&"pikachu2_POKEMON") {
            assert_eq!(find_avatar("pikachu two").map(|i| avatars()[i]), Some("pikachu2_POKEMON"));
        }
        if backgrounds().contains(&"DBZ") {
            assert_eq!(find_background("the dbz background").map(|i| backgrounds()[i]), Some("DBZ"));
        }
    }

    #[test]
    fn voice_needs_the_whole_phrase_outside_a_prompt() {
        let s = Session {
            id: 1,
            browser: "Chrome".into(),
            display: "Chrome".into(),
            key: "chrome".into(),
            family: "chromium".into(),
            avatar: 0,
            background: 0,
            custom: None,
            step: "look",
            name: String::new(),
            final_name: String::new(),
            taken: vec![],
            resume: Value::Null,
            edit: false,
            old: vec![],
        };
        let c = choices(&s);
        let op = |t: &str| exact(t, &c).and_then(|i| if let Intent::Act { args, .. } = i { args.get("op").and_then(Value::as_str).map(str::to_owned) } else { None });
        assert_eq!(op("next").as_deref(), Some("next"));
        assert_eq!(op("next background").as_deref(), Some("bg_next"));
        assert_eq!(op("i like this one").as_deref(), Some("confirm"));
        assert_eq!(op("turn it right up"), None);
        if avatars().contains(&"goku_DBZ") {
            assert_eq!(op("the goku").as_deref(), Some("avatar"));
        }
    }

    #[test]
    fn the_look_and_name_are_written_into_the_profile() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("chrome");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".setu-profile"), r#"{"created":1,"key":"chrome"}"#).unwrap();
        let pic = t.path().join("me.jpg");
        std::fs::write(&pic, b"jpeg").unwrap();
        apply(&dir, "chromium", &json!({ "name": "Anoop_setu", "avatar": "", "background": "nature", "custom": pic.to_string_lossy() })).unwrap();
        let v = saved(&dir);
        assert_eq!(v["name"], "Anoop_setu");
        assert_eq!(v["created"], 1);
        assert_eq!(v["custom"], ".setu-avatar.jpg");
        assert!(dir.join(".setu-avatar.jpg").exists());
        let ls: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("Local State")).unwrap()).unwrap();
        assert_eq!(ls.pointer("/profile/info_cache/Default/name").and_then(Value::as_str), Some("Anoop_setu"));
        let pr: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("Default/Preferences")).unwrap()).unwrap();
        assert_eq!(pr.pointer("/profile/name").and_then(Value::as_str), Some("Anoop_setu"));
    }
}
