use crate::core::Core;
use crate::os::{self, Do, Node, Scope, Win};
use crate::router::Intent;
use crate::text::normalize;
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LAYER: &str = "browser";

pub fn is_browser(app: &str) -> bool {
    let a = app.to_lowercase();
    ["chrome", "chromium", "brave", "edge", "firefox", "opera", "vivaldi", "safari", "librewolf", "zen"].iter().any(|w| a.contains(w))
}

fn matches_browser(app: &str, name: &str) -> bool {
    let a = normalize(app);
    match crate::tools::web::browser_key(name) {
        Some(k) => a.contains(k) || (k == "chrome" && a.contains("google chrome")) || (k == "edge" && a.contains("microsoft edge")),
        None => a.contains(&normalize(name)),
    }
}

const BRANDS: &[&str] = &["google chrome", "chromium", "brave", "microsoft edge", "edge", "mozilla firefox", "firefox", "opera", "vivaldi", "safari", "librewolf", "zen", "chrome"];

pub fn main_window(w: &Win) -> bool {
    let t = normalize(&w.title);
    let a = normalize(&w.app);
    let brand = a.trim_end_matches(" browser").trim_end_matches(" web").to_owned();
    !t.is_empty() && (t == a || t.ends_with(&format!(" {a}")) || (!brand.is_empty() && t.ends_with(&format!(" {brand}"))) || BRANDS.iter().any(|b| t.ends_with(&format!(" {b}")) || t == *b))
}

pub async fn window(name: &str) -> Option<Win> {
    let wins = os::windows().await.ok()?;
    let all: Vec<&Win> = wins.iter().filter(|w| is_browser(&w.app) && (name.trim().is_empty() || matches_browser(&w.app, name))).collect();
    let mains: Vec<&Win> = all.iter().filter(|w| main_window(w)).copied().collect();
    let cands = if mains.is_empty() { all } else { mains };
    if let Some(w) = cands.iter().find(|w| w.front) {
        return Some((*w).clone());
    }
    if let Some(p) = wins.iter().find(|w| w.front && is_browser(&w.app)).map(|w| w.pid)
        && let Some(w) = cands.iter().find(|w| w.pid == p && p != 0)
    {
        return Some((*w).clone());
    }
    if let Some(w) = cands.iter().find(|w| w.front) {
        return Some((*w).clone());
    }
    if let Ok(work) = os::working().await
        && cands.iter().any(|w| w.id == work.id)
    {
        return Some(work);
    }
    cands.first().map(|w| (*w).clone())
}

fn is_address(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("address") || n.contains("search or enter") || n.contains("search with") || n.contains("enter address") || n == "url"
}

pub fn bare(name: &str) -> String {
    let t = name.trim();
    let t = match t.rfind(" (") {
        Some(i) if t.ends_with(')') => &t[..i],
        _ => t,
    };
    normalize(t)
}

fn is_close(name: &str) -> bool {
    let n = normalize(name);
    n == "close" || n == "close tab" || n.starts_with("close tab ")
}

fn inside(n: &Node, outer: &Node) -> bool {
    let (cx, cy) = (n.x + n.w / 2, n.y + n.h / 2);
    cx >= outer.x && cx <= outer.x + outer.w && cy >= outer.y && cy <= outer.y + outer.h
}

pub fn tab_title(name: &str) -> String {
    let mut t = name.trim().to_owned();
    for tail in [" - Audio playing", " - Audio muted", " (Pinned)", " - Pinned", " - Network error", " - Crashed", " - Recording", " - Playing audio"] {
        if let Some(r) = t.strip_suffix(tail) {
            t = r.trim().to_owned();
        }
    }
    if let Some((head, _)) = t.split_once(" - Memory usage") {
        t = head.trim().to_owned();
    }
    t
}

pub struct View {
    pub win: Win,
    pub nodes: Vec<Node>,
}

pub async fn view(name: &str) -> Option<View> {
    let win = window(name).await?;
    os::wake(&win).await;
    let nodes = os::tree(&win, Scope { limit: 3000, text: false, hidden: false }).await.ok()?;
    Some(View { win, nodes })
}

async fn view_with_tabs(name: &str) -> Option<View> {
    for attempt in 0..4 {
        let v = view(name).await?;
        if !v.tabs().is_empty() || attempt == 3 {
            return Some(v);
        }
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    None
}

impl View {
    pub fn address(&self) -> Option<&Node> {
        self.nodes.iter().find(|n| n.role == "field" && !n.web && is_address(&n.name))
    }

    fn chrome_bottom(&self) -> Option<i32> {
        self.address().map(|a| a.y + a.h + 4)
    }

    pub fn tabs(&self) -> Vec<&Node> {
        let top = self.address().map(|a| a.y);
        let mut v: Vec<&Node> = self.nodes.iter().filter(|n| n.role == "tab" && !n.web && top.is_none_or(|t| n.y < t)).collect();
        v.sort_by_key(|n| (n.y / 20, n.x));
        v
    }

    pub fn active_tab(&self) -> Option<&Node> {
        let tabs = self.tabs();
        if let Some(t) = tabs.iter().find(|t| t.selected) {
            return Some(t);
        }
        let title = normalize(&self.win.title);
        tabs.into_iter().filter(|t| !t.name.is_empty() && title.starts_with(&normalize(&tab_title(&t.name)))).max_by_key(|t| t.name.len())
    }

    fn button(&self, names: &[&str]) -> Option<&Node> {
        let bottom = self.chrome_bottom().unwrap_or(i32::MAX);
        names.iter().find_map(|want| self.nodes.iter().find(|n| matches!(n.role.as_str(), "button" | "link") && !n.web && n.y < bottom && bare(&n.name) == normalize(want)))
    }

    fn close_of(&self, tab: &Node) -> Option<&Node> {
        self.nodes.iter().find(|n| n.role == "button" && is_close(&n.name) && inside(n, tab))
    }

    pub fn page(&self) -> Vec<&Node> {
        let bottom = self.chrome_bottom();
        let linux = cfg!(target_os = "linux");
        self.nodes.iter().filter(|n| if linux { n.web } else { bottom.is_some_and(|b| n.y >= b) }).collect()
    }
}

pub fn pick_tab<'a>(tabs: &[&'a Node], target: &Value) -> Vec<&'a Node> {
    if let Some(i) = target.get("index").and_then(Value::as_i64) {
        let n = tabs.len() as i64;
        let k = if i < 0 { n + i } else { i - 1 };
        return if (0..n).contains(&k) { vec![tabs[k as usize]] } else { vec![] };
    }
    if let Some(q) = target.get("query").and_then(Value::as_str) {
        let want = normalize(q);
        let all = target.get("all").and_then(Value::as_bool).unwrap_or(false);
        let mut scored: Vec<(f64, &Node)> = tabs.iter().map(|t| (os::score(&want, &normalize(&tab_title(&t.name))).max(os::score(&want, &normalize(&t.name)) * 0.98), *t)).filter(|(s, _)| *s >= 0.75).collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if all {
            return scored.into_iter().map(|(_, t)| t).collect();
        }
        return scored.into_iter().take(1).map(|(_, t)| t).collect();
    }
    Vec::new()
}

pub async fn setu_tab(query: &str) -> Option<Outcome> {
    let wins = os::windows().await.ok()?;
    let mine = wins.iter().filter(|w| os::is_setu_window(w)).max_by_key(|w| w.front)?.clone();
    let nodes = os::tree(&mine, Scope { limit: 600, text: false, hidden: false }).await.ok()?;
    let tab = os::find(&nodes, query, &["tab"])?.clone();
    if tab.selected {
        return Some(Outcome::ok_with(&format!("{} is open", tab.name), LAYER, Some(mine.app.clone())));
    }
    Some(match os::act(&tab, Do::Select).await {
        Ok(()) => Outcome::ok_with(&format!("Opened {}", tab.name), LAYER, Some(mine.app.clone())),
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    })
}

pub async fn setu_in_front() -> bool {
    os::front().await.is_ok_and(|w| os::is_setu(&w.app))
}

pub async fn list(name: &str, count_only: bool) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let tabs = v.tabs();
    let n = tabs.len();
    if n == 0 {
        return None;
    }
    if count_only {
        let line = if n == 1 { "One tab is open.".to_owned() } else { format!("{n} tabs are open.") };
        return Some(Outcome::ok_with("Counted tabs", LAYER, Some(v.win.app.clone())).with_reply(line));
    }
    let names: Vec<String> = tabs.iter().take(6).map(|t| tab_title(&t.name)).collect();
    let more = if n > 6 { format!(", and {} more", n - 6) } else { String::new() };
    Some(Outcome::ok_with("Listed tabs", LAYER, Some(v.win.app.clone())).with_reply(format!("{n} tabs: {}{more}.", names.join("; "))))
}

pub async fn switch(name: &str, target: &Value) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let tabs = v.tabs();
    if tabs.is_empty() {
        return None;
    }
    let Some(tab) = pick_tab(&tabs, target).into_iter().next() else {
        return Some(Outcome::ok_with("No tab", LAYER, None).with_reply("I don't see that tab.".into()));
    };
    let title = tab_title(&tab.name);
    match os::act(tab, Do::Select).await {
        Ok(()) => Some(Outcome::ok_with(&format!("Switched to {title}"), LAYER, Some(v.win.app.clone()))),
        Err(e) => Some(Outcome::fail(&e.to_string(), LAYER)),
    }
}

async fn close_one(name: &str, title: &str, seen: Option<View>) -> Result<bool, String> {
    let v = match seen {
        Some(v) => v,
        None => view_with_tabs(name).await.ok_or_else(|| "The browser closed.".to_owned())?,
    };
    let tabs = v.tabs();
    let want = normalize(title);
    let Some(tab) = tabs.iter().find(|t| normalize(&tab_title(&t.name)) == want).copied() else { return Ok(false) };
    if let Some(b) = v.close_of(tab) {
        return os::act(b, Do::Invoke).await.map(|_| true).map_err(|e| e.to_string());
    }
    os::act(tab, Do::Select).await.map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let Some(v2) = view(name).await else { return Err("The browser closed.".into()) };
    let Some(tab2) = v2.tabs().into_iter().find(|t| normalize(&tab_title(&t.name)) == want) else { return Ok(true) };
    match v2.close_of(tab2) {
        Some(b) => os::act(b, Do::Invoke).await.map(|_| true).map_err(|e| e.to_string()),
        None => Err("That tab has no close button I can press.".into()),
    }
}

pub async fn close(name: &str, target: &Value) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let tabs = v.tabs();
    if tabs.is_empty() {
        return None;
    }
    let active = v.active_tab().map(|t| t.id.clone());
    let chosen: Vec<&Node> = if target.get("others").and_then(Value::as_bool).unwrap_or(false) {
        tabs.iter().filter(|t| Some(&t.id) != active.as_ref()).copied().collect()
    } else if target.get("right").and_then(Value::as_bool).unwrap_or(false) {
        let pos = tabs.iter().position(|t| Some(&t.id) == active.as_ref())?;
        tabs[pos + 1..].to_vec()
    } else if target.get("index").is_some() || target.get("query").is_some() {
        pick_tab(&tabs, target)
    } else {
        tabs.iter().filter(|t| Some(&t.id) == active.as_ref()).copied().collect()
    };
    if chosen.is_empty() {
        return Some(Outcome::ok_with("No tab", LAYER, None).with_reply("I don't see that tab.".into()));
    }
    let titles: Vec<String> = chosen.iter().map(|t| tab_title(&t.name)).collect();
    let app = v.win.app.clone();
    let mut seen = Some(v);
    let mut closed = 0;
    for t in &titles {
        match close_one(name, t, seen.take()).await {
            Ok(true) => closed += 1,
            Ok(false) => {}
            Err(e) if closed == 0 => return Some(Outcome::fail(&e, LAYER)),
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Some(match closed {
        0 => Outcome::ok_with("No tab", LAYER, None).with_reply("I don't see that tab.".into()),
        1 => Outcome::ok_with("Closed the tab", LAYER, Some(app)),
        n => Outcome::ok_with(&format!("Closed {n} tabs"), LAYER, Some(app)),
    })
}

pub async fn new_tab(name: &str) -> Option<Outcome> {
    let v = view(name).await?;
    let b = v.button(&["New Tab", "New tab", "Open a new tab", "New Tab Button"])?;
    match os::act(b, Do::Invoke).await {
        Ok(()) => Some(Outcome::ok_with(&format!("New tab in {}", v.win.app), LAYER, Some(v.win.app.clone()))),
        Err(e) => Some(Outcome::fail(&e.to_string(), LAYER)),
    }
}

pub async fn nav(name: &str, which: &str) -> Option<Outcome> {
    let v = view(name).await?;
    let (names, did): (&[&str], &str) = match which {
        "back" => (&["Back", "Go back"], "Went back"),
        "forward" => (&["Forward", "Go forward"], "Went forward"),
        _ => (&["Reload", "Refresh", "Reload current page"], "Refreshed"),
    };
    let b = v.button(names)?;
    if !b.enabled {
        return Some(Outcome::ok_with(did, LAYER, None).with_reply(if which == "forward" { "There's no page to go forward to.".into() } else { "There's no page to go back to.".into() }));
    }
    Some(match os::act(b, Do::Invoke).await {
        Ok(()) => Outcome::ok_with(did, LAYER, Some(v.win.app.clone())),
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    })
}

pub async fn current_url(name: &str) -> Option<(View, String)> {
    let v = view(name).await?;
    let raw = v.address()?.value.trim().to_owned();
    if raw.is_empty() {
        return None;
    }
    let url = if raw.contains("://") || raw.starts_with("about:") || raw.starts_with("chrome:") { raw } else { format!("https://{raw}") };
    Some((v, url))
}

pub async fn copy_url(name: &str) -> Option<Outcome> {
    let (v, url) = current_url(name).await?;
    let u = url.clone();
    let copied = crate::layers::blocking(move || crate::tools::shell::copy_text(&u)).await.map(|r| r.is_ok()).unwrap_or(false);
    Some(if copied {
        Outcome::ok_with("Copied the page link", LAYER, Some(v.win.app.clone()))
    } else {
        Outcome::ok_with("Read the page link", LAYER, Some(v.win.app.clone())).with_reply(format!("The page is {url}. I couldn't copy it."))
    })
}

fn kind_matches(n: &Node, kind: &str) -> bool {
    let named = !n.name.trim().is_empty();
    match kind {
        "image" => (n.role == "image" && n.w >= 24 && n.h >= 24) || (n.role == "link" && n.w >= 40 && n.h >= 40),
        "link" => n.role == "link" && named,
        "result" | "video" => n.role == "link" && named && n.name.split_whitespace().count() >= 2 && n.w >= 60,
        "button" => n.role == "button" && named,
        "field" => matches!(n.role.as_str(), "field" | "combobox"),
        _ => matches!(n.role.as_str(), "link" | "button") && named,
    }
}

pub fn candidates<'a>(page: &[&'a Node], kind: &str) -> Vec<&'a Node> {
    let mut v: Vec<&Node> = page.iter().filter(|n| kind_matches(n, kind)).copied().collect();
    if kind == "image" {
        let imgs: Vec<&Node> = v.iter().filter(|n| n.role == "image").copied().collect();
        v.retain(|n| n.role == "image" || !imgs.iter().any(|i| inside(i, n)));
    }
    v
}

fn rows(mut v: Vec<&Node>) -> Vec<Vec<&Node>> {
    v.sort_by_key(|n| (n.y, n.x));
    let mut out: Vec<Vec<&Node>> = Vec::new();
    for n in v {
        match out.last_mut() {
            Some(row) if (n.y - row[0].y).abs() <= 24 => row.push(n),
            _ => out.push(vec![n]),
        }
    }
    for r in out.iter_mut() {
        r.sort_by_key(|n| n.x);
    }
    out
}

pub fn pick_ref<'a>(page: &[&'a Node], r: &Value) -> Option<&'a Node> {
    let kind = r.get("kind").and_then(Value::as_str).unwrap_or("any");
    if let Some(text) = r.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
        let owned: Vec<Node> = if kind == "any" { page.iter().map(|n| (*n).clone()).collect() } else { candidates(page, kind).into_iter().cloned().collect() };
        let hit = os::find(&owned, text, &[])?;
        return page.iter().find(|n| n.id == hit.id).copied();
    }
    let n = r.get("n").and_then(Value::as_i64).unwrap_or(1);
    let cands: Vec<&Node> = candidates(page, kind);
    if cands.is_empty() || n == 0 {
        return None;
    }
    let from = r.get("from").and_then(Value::as_str);
    let row = r.get("row").and_then(Value::as_i64);
    if from.is_some() || row.is_some() {
        let grid = rows(cands);
        let line = grid.get((row.unwrap_or(1).max(1) - 1) as usize)?;
        let k = (n.max(1) - 1) as usize;
        return if from == Some("right") { line.iter().rev().nth(k).copied() } else { line.get(k).copied() };
    }
    let mut ordered = cands;
    ordered.sort_by_key(|x| (x.y / 8, x.x));
    if n < 0 {
        let len = ordered.len() as i64;
        return ordered.get((len + n).max(0) as usize).copied();
    }
    ordered.get((n - 1) as usize).copied()
}

static NUMBERED: Mutex<Option<(String, Vec<Node>)>> = Mutex::new(None);
static NUMBER_ROUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST: Mutex<Option<(String, Node)>> = Mutex::new(None);

fn s(args: &Value, k: &str) -> String {
    args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned()
}

fn firefox_like(app: &str) -> bool {
    let a = app.to_lowercase();
    a.contains("firefox") || a.contains("librewolf") || a.contains("zen")
}

fn launch_flags(app: &str, place: &str, url: &str) -> Vec<String> {
    let a = app.to_lowercase();
    let ff = firefox_like(app);
    match place {
        "window" => vec![if ff { "-new-window" } else { "--new-window" }.to_owned(), url.to_owned()],
        "private" => vec![if ff { "-private-window" } else if a.contains("edge") { "--inprivate" } else { "--incognito" }.to_owned(), url.to_owned()],
        _ => vec![url.to_owned()],
    }
}

async fn launch(core: &Arc<Core>, app: &str, args: Vec<String>) -> bool {
    let Some(a) = crate::tools::web::find_browser(core, Some(app)).or_else(|| crate::tools::web::find_browser(core, None)) else { return false };
    matches!(crate::layers::blocking(move || crate::tools::shell::launch_args(&a, &args)).await, Ok(Ok(_)))
}

fn where_words(place: &str) -> &'static str {
    match place {
        "window" => " in a new window",
        "private" => " in a private window",
        "here" => "",
        _ => " in a new tab",
    }
}

async fn wait_control(win: &Win, names: &[&str], roles: &[&str], ms: u64) -> Option<Node> {
    let wants: Vec<String> = names.iter().map(|n| normalize(n)).collect();
    let pick = |nodes: &[Node]| -> Option<Node> {
        wants.iter().find_map(|w| nodes.iter().find(|n| !n.web && n.enabled && (roles.is_empty() || roles.contains(&n.role.as_str())) && bare(&n.name) == *w).cloned())
    };
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        if let Some(n) = pick(&aux_nodes(win).await) {
            return Some(n);
        }
        if let Ok(t) = os::tree(win, Scope { limit: 3000, text: false, hidden: false }).await
            && let Some(n) = pick(&t)
        {
            return Some(n);
        }
        if Instant::now() >= until {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

async fn aux(win: &Win) -> Vec<(Win, Vec<Node>)> {
    os::popups(win).await.unwrap_or_default().into_iter().filter(|(w, _)| !main_window(w)).collect()
}

async fn aux_nodes(win: &Win) -> Vec<Node> {
    aux(win).await.into_iter().flat_map(|(_, n)| n).collect()
}

async fn close_bubbles(win: &Win, settle: bool) -> bool {
    let mut closed = false;
    for (_, nodes) in aux(win).await {
        if nodes.iter().any(|n| n.role == "menuitem") {
            continue;
        }
        if let Some(b) = nodes.iter().find(|n| n.role == "button" && !n.web && bare(&n.name) == "close") {
            if settle {
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
            closed |= os::act(b, Do::Invoke).await.is_ok();
        }
    }
    closed
}

async fn dismiss(win: &Win) {
    if os::front().await.is_ok_and(|f| f.pid == win.pid && f.pid != 0) {
        let _ = crate::tools::input::press(&[crate::tools::input::K::Esc]).await;
    }
}

async fn tab_menu(v: &View, tab: &Node, items: &[&str]) -> Option<Result<(), String>> {
    if os::act(tab, Do::Menu).await.is_err() {
        return None;
    }
    match wait_control(&v.win, items, &["menuitem"], 2500).await {
        Some(item) => Some(os::act(&item, Do::Invoke).await.map_err(|e| e.to_string())),
        None => {
            dismiss(&v.win).await;
            Some(Err("The tab's menu didn't have that.".into()))
        }
    }
}

fn target_tab<'a>(v: &'a View, target: &Value) -> Option<&'a Node> {
    let tabs = v.tabs();
    if target.get("index").is_some() || target.get("query").is_some() {
        return pick_tab(&tabs, target).into_iter().next();
    }
    v.active_tab()
}

pub async fn pin(name: &str, target: &Value, pinned: bool) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let tab = target_tab(&v, target)?.clone();
    let is_pinned = tab.name.contains("Pinned");
    let app = Some(v.win.app.clone());
    if pinned && is_pinned {
        return Some(Outcome::ok_with("Already pinned", LAYER, app).with_reply("This tab is already pinned.".into()));
    }
    if !pinned && !is_pinned && !firefox_like(&v.win.app) {
        return Some(Outcome::ok_with("Not pinned", LAYER, app).with_reply("This tab isn't pinned.".into()));
    }
    let items: &[&str] = if pinned { &["Pin", "Pin tab", "Pin Tab"] } else { &["Unpin", "Unpin tab", "Unpin Tab"] };
    Some(match tab_menu(&v, &tab, items).await {
        Some(Ok(())) => Outcome::ok_with(if pinned { "Pinned the tab" } else { "Unpinned the tab" }, LAYER, app),
        Some(Err(e)) => Outcome::fail(&e, LAYER),
        None => Outcome::ok_with("Can't pin", LAYER, app).with_reply(format!("I can't {} tabs in {} yet.", if pinned { "pin" } else { "unpin" }, v.win.app)),
    })
}

pub async fn duplicate(core: &Arc<Core>, name: &str) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let app = Some(v.win.app.clone());
    if let Some(tab) = v.active_tab().cloned()
        && let Some(r) = tab_menu(&v, &tab, &["Duplicate", "Duplicate tab", "Duplicate Tab"]).await
    {
        return Some(match r {
            Ok(()) => Outcome::ok_with("Duplicated the tab", LAYER, app),
            Err(e) => Outcome::fail(&e, LAYER),
        });
    }
    let (_, url) = current_url(name).await?;
    Some(if launch(core, &v.win.app, vec![url]).await { Outcome::ok_with("Duplicated the tab", LAYER, app) } else { Outcome::fail("The browser didn't open the copy.", LAYER) })
}

pub async fn detach(core: &Arc<Core>, name: &str) -> Option<Outcome> {
    let v = view_with_tabs(name).await?;
    let app = Some(v.win.app.clone());
    if let Some(tab) = v.active_tab().cloned()
        && let Some(r) = tab_menu(&v, &tab, &["Move tab to new window", "Move Tab to New Window", "Move to new window", "Move tab to a new window"]).await
    {
        return Some(match r {
            Ok(()) => Outcome::ok_with("Moved the tab to a new window", LAYER, app),
            Err(e) => Outcome::fail(&e, LAYER),
        });
    }
    let close_btn = v.active_tab().and_then(|t| v.close_of(t)).cloned();
    let url = v.address().map(|a| a.value.trim().to_owned()).filter(|u| !u.is_empty()).map(|u| if u.contains("://") || u.starts_with("about:") { u } else { format!("https://{u}") })?;
    if !launch(core, &v.win.app, launch_flags(&v.win.app, "window", &url)).await {
        return Some(Outcome::fail("The browser didn't open a new window.", LAYER));
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    if let Some(b) = close_btn {
        let _ = os::act(&b, Do::Invoke).await;
    }
    Some(Outcome::ok_with("Moved the tab to a new window", LAYER, app))
}

pub async fn reopen(core: &Arc<Core>, name: &str) -> Option<Outcome> {
    let win = window(name).await?;
    if !os::in_front(&win, 200).await {
        if let Some(a) = crate::tools::web::find_browser(core, Some(&win.app)) {
            let _ = crate::tools::wm::focus(&win.app, Some(a)).await;
        }
        if !os::in_front(&win, 1500).await {
            return Some(Outcome::ok_with("Not in front", LAYER, Some(win.app.clone())).with_reply(format!("I can bring back a tab once {} is in front.", win.app)));
        }
    }
    use crate::tools::input::{K, press, primary};
    Some(match press(&[primary(), K::Shift, K::Ch('t')]).await {
        Ok(()) => Outcome::ok_with("Reopened the tab", "keyboard", Some(win.app.clone())),
        Err(e) => Outcome::fail(&e.to_string(), "keyboard"),
    })
}

fn audible(t: &Node, muted: bool) -> bool {
    let n = t.name.to_lowercase();
    if muted { n.contains("muted") } else { n.contains("audio playing") || n.contains("playing audio") || n.contains("playing media") }
}

pub async fn mute(target: &str, on: bool) -> Option<Outcome> {
    let v = view_with_tabs("").await?;
    let tabs = v.tabs();
    let tab = if target.trim().is_empty() {
        tabs.iter().find(|t| audible(t, !on)).copied().or_else(|| v.active_tab())
    } else {
        pick_tab(&tabs, &serde_json::json!({ "query": target })).into_iter().next()
    };
    let Some(tab) = tab.cloned() else {
        return Some(Outcome::ok_with("No tab", LAYER, None).with_reply(if target.is_empty() { "I don't see a tab playing sound.".into() } else { format!("I don't see a {target} tab.") }));
    };
    let title = tab_title(&tab.name);
    let did = format!("{} {title}", if on { "Muted" } else { "Unmuted" });
    let buttons: &[&str] = if on { &["Mute tab", "Mute"] } else { &["Unmute tab", "Unmute"] };
    if let Some(b) = v.nodes.iter().find(|n| n.role == "button" && !n.web && inside(n, &tab) && buttons.iter().any(|w| bare(&n.name) == normalize(w))) {
        return Some(match os::act(b, Do::Invoke).await {
            Ok(()) => Outcome::ok_with(&did, LAYER, Some(v.win.app.clone())),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        });
    }
    let items: &[&str] = if on { &["Mute site", "Mute tab", "Mute Site"] } else { &["Unmute site", "Unmute tab", "Unmute Site"] };
    Some(match tab_menu(&v, &tab, items).await {
        Some(Ok(())) => Outcome::ok_with(&did, LAYER, Some(v.win.app.clone())),
        Some(Err(_)) => Outcome::ok_with("Nothing to change", LAYER, None).with_reply(if on { format!("{title} is already muted.") } else { format!("{title} isn't muted.") }),
        None => Outcome::fail(&format!("I can't mute tabs in {} yet.", v.win.app), LAYER),
    })
}

fn is_star(name: &str) -> bool {
    let b = bare(name);
    ["bookmark this", "edit bookmark", "edit this bookmark", "add this page to favorites", "edit favorite"].iter().any(|p| b.starts_with(p))
}

fn is_marked(name: &str) -> bool {
    bare(name).starts_with("edit")
}

pub async fn bookmark(name: &str, remove: bool) -> Option<Outcome> {
    let v = view(name).await?;
    let star = v.nodes.iter().find(|n| matches!(n.role.as_str(), "button" | "toggle") && !n.web && is_star(&n.name))?.clone();
    let app = Some(v.win.app.clone());
    let marked = is_marked(&star.name);
    if !remove && marked {
        return Some(Outcome::ok_with("Already bookmarked", LAYER, app).with_reply("This page is already bookmarked.".into()));
    }
    if remove && !marked {
        return Some(Outcome::ok_with("Not bookmarked", LAYER, app).with_reply("This page isn't bookmarked.".into()));
    }
    if let Err(e) = os::act(&star, Do::Invoke).await {
        return Some(Outcome::fail(&e.to_string(), LAYER));
    }
    let names: &[&str] = if remove { &["Remove", "Remove bookmark", "Remove Bookmark", "Remove favorite"] } else { &["Done", "Save"] };
    let mut found = wait_control(&v.win, names, &["button"], 1500).await;
    if found.is_none() && close_bubbles(&v.win, false).await {
        tokio::time::sleep(Duration::from_millis(300)).await;
        if os::act(&star, Do::Invoke).await.is_ok() {
            found = wait_control(&v.win, names, &["button"], 2000).await;
        }
    }
    match found {
        Some(b) => {
            tokio::time::sleep(Duration::from_millis(700)).await;
            if let Err(e) = os::act(&b, Do::Invoke).await {
                return Some(Outcome::fail(&e.to_string(), LAYER));
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            if let Some(again) = wait_control(&v.win, names, &["button"], 0).await
                && again.id == b.id
            {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let _ = os::act(&again, Do::Invoke).await;
            }
        }
        None if remove => {
            dismiss(&v.win).await;
            return Some(Outcome::fail("The browser didn't offer to remove it.", LAYER));
        }
        None => {}
    }
    if !remove {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = close_bubbles(&v.win, true).await;
    }
    Some(Outcome::ok_with(if remove { "Removed the bookmark" } else { "Bookmarked" }, LAYER, app))
}

pub async fn find(name: &str, text: &str) -> Option<Outcome> {
    let win = window(name).await?;
    let app = Some(win.app.clone());
    Some(match os::find_text(&win, text).await {
        Ok(true) => Outcome::ok_with(&format!("Found {text}"), LAYER, app),
        Ok(false) => Outcome::ok_with("Not found", LAYER, app).with_reply(format!("I couldn't find {text} on this page.")),
        Err(_) if os::in_front(&win, 300).await => {
            use crate::tools::input::{K, press, primary, type_text};
            if press(&[primary(), K::Ch('f')]).await.is_err() {
                return Some(Outcome::fail("I couldn't open find.", "keyboard"));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            match type_text(text).await {
                Ok(()) => Outcome::ok_with(&format!("Finding {text}"), "keyboard", app),
                Err(e) => Outcome::fail(&e.to_string(), "keyboard"),
            }
        }
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    })
}

pub fn forget_numbers() {
    NUMBER_ROUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    NUMBERED.lock().unwrap().take();
    crate::browse::PAGE_NUMBERS.store(false, std::sync::atomic::Ordering::Relaxed);
}

pub fn clear_numbers(core: &Core) -> bool {
    NUMBER_ROUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let had = NUMBERED.lock().unwrap().take().is_some();
    if had {
        core.hub.emit("numbers.state", serde_json::json!({ "visible": false, "elements": [] }));
    }
    crate::browse::PAGE_NUMBERS.store(false, std::sync::atomic::Ordering::Relaxed);
    had
}

async fn number(core: &Arc<Core>, name: &str, kind: &str) -> Option<Outcome> {
    let v = view_scope(name, kind == "image").await?;
    let page = v.page();
    let mut list: Vec<Node> = candidates(&page, kind).into_iter().filter(|n| n.w > 0 && n.h > 0).cloned().collect();
    os::reading_order(&mut list);
    list.truncate(99);
    let app = Some(v.win.app.clone());
    if list.is_empty() {
        clear_numbers(core);
        return Some(Outcome::ok_with("Nothing to number", LAYER, app).with_reply("I don't see any here.".into()));
    }
    let els: Vec<Value> = list
        .iter()
        .enumerate()
        .map(|(i, n)| serde_json::json!({ "n": i + 1, "x": n.x, "y": n.y, "w": n.w, "h": n.h, "label": if n.name.trim().is_empty() { "item" } else { n.name.as_str() } }))
        .collect();
    let n = list.len();
    core.hub.emit("numbers.state", serde_json::json!({ "visible": true, "elements": els }));
    *NUMBERED.lock().unwrap() = Some((v.win.id.clone(), list));
    crate::browse::PAGE_NUMBERS.store(true, std::sync::atomic::Ordering::Relaxed);
    let round = NUMBER_ROUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let c = core.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        if NUMBER_ROUND.load(std::sync::atomic::Ordering::Relaxed) == round {
            clear_numbers(&c);
        }
    });
    Some(Outcome::ok_with(&format!("Numbered {n}"), LAYER, app).with_reply(format!("Say open, click or show options, then a number from 1 to {n}.")))
}

async fn view_scope(name: &str, text: bool) -> Option<View> {
    let win = window(name).await?;
    os::wake(&win).await;
    let nodes = os::tree(&win, Scope { limit: 3000, text, hidden: false }).await.ok()?;
    Some(View { win, nodes })
}

fn this_target(v: &View, page: &[&Node], kind: &str) -> Option<Node> {
    let cands = candidates(page, kind);
    if let Some(f) = cands.iter().find(|n| n.focused) {
        return Some((*f).clone());
    }
    let last = LAST.lock().unwrap().clone();
    if let Some((id, l)) = last
        && id == v.win.id
        && let Some(n) = cands.iter().find(|n| n.id == l.id || (n.role == l.role && !n.name.is_empty() && n.name == l.name))
    {
        return Some((*n).clone());
    }
    if cands.len() == 1 {
        return Some(cands[0].clone());
    }
    if kind == "image" {
        let mut by_area: Vec<&Node> = cands.iter().filter(|n| n.role == "image").copied().collect();
        by_area.sort_by_key(|n| -(n.w as i64 * n.h as i64));
        if let Some(big) = by_area.first() {
            let a = big.w as i64 * big.h as i64;
            let b = by_area.get(1).map(|n| n.w as i64 * n.h as i64).unwrap_or(0);
            if a >= 2 * b.max(1) {
                return Some((*big).clone());
            }
        }
    }
    None
}

fn resolve(v: &View, page: &[&Node], r: &Value, kind: &str, what: &str) -> Result<Node, String> {
    if let Some(k) = r.get("number").and_then(Value::as_i64) {
        let g = NUMBERED.lock().unwrap();
        return match g.as_ref() {
            Some((id, list)) if *id == v.win.id => list.get((k.max(1) - 1) as usize).cloned().ok_or_else(|| format!("There is no number {k}.")),
            _ => Err("Numbers aren't shown on this page. Say number the images, or number the links.".into()),
        };
    }
    if r.get("this").and_then(Value::as_bool).unwrap_or(false) {
        return this_target(v, page, kind).ok_or_else(|| if kind == "any" { "Which one? Say its name, or a number.".to_owned() } else { format!("Which {kind}? Say the first {kind}, or number the {kind}s.") });
    }
    pick_ref(page, r).cloned().ok_or_else(|| format!("I couldn't find {what} on this page."))
}

fn enclosing_link<'a>(page: &[&'a Node], n: &Node) -> Option<&'a Node> {
    page.iter().filter(|l| l.role == "link" && l.id != n.id && inside(n, l)).min_by_key(|l| l.w as i64 * l.h as i64).copied()
}

fn image_in<'a>(page: &[&'a Node], n: &Node) -> Option<&'a Node> {
    page.iter().filter(|i| i.role == "image" && i.id != n.id && inside(i, n)).max_by_key(|i| i.w as i64 * i.h as i64).copied()
}

async fn link_of(page: &[&Node], n: &Node) -> Option<String> {
    if let Some(u) = os::link_url(n).await {
        return Some(u);
    }
    match enclosing_link(page, n) {
        Some(l) => os::link_url(l).await,
        None => None,
    }
}

async fn src_of(page: &[&Node], n: &Node) -> Option<String> {
    if n.role == "image" {
        return os::image_url(n).await;
    }
    match image_in(page, n) {
        Some(i) => os::image_url(i).await,
        None => None,
    }
}

struct Target {
    win: Win,
    node: Node,
    link: Option<String>,
    src: Option<String>,
}

impl Target {
    fn label(&self, what: &str) -> String {
        let n = self.node.name.trim();
        if n.is_empty() { what.to_owned() } else { n.chars().take(60).collect() }
    }
}

async fn open_at(core: &Arc<Core>, t: &Target, place: &str, full: bool, what: &str) -> Outcome {
    let app = Some(t.win.app.clone());
    let label = t.label(what);
    if full
        && let Some(src) = t.src.as_ref().filter(|u| u.starts_with("http"))
    {
        let p = if place == "here" { "tab" } else { place };
        return if launch(core, &t.win.app, launch_flags(&t.win.app, p, src)).await { Outcome::ok_with(&format!("Opened the image{}", where_words(p)), LAYER, app) } else { Outcome::fail("The browser didn't open the image.", LAYER) };
    }
    if place != "here"
        && let Some(u) = &t.link
        && launch(core, &t.win.app, launch_flags(&t.win.app, place, u)).await
    {
        return Outcome::ok_with(&format!("Opened {label}{}", where_words(place)), LAYER, app);
    }
    match os::act(&t.node, Do::Invoke).await {
        Ok(()) => Outcome::ok_with(&format!("Opened {label}"), LAYER, app),
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    }
}

async fn copy_out(text: String, did: &str, app: Option<String>) -> Outcome {
    match crate::layers::blocking(move || crate::tools::shell::copy_text(&text)).await {
        Ok(Ok(())) => Outcome::ok_with(did, LAYER, app),
        _ => Outcome::fail("I couldn't copy it.", LAYER),
    }
}

async fn copy_target(t: &Target, what: &str) -> Outcome {
    let app = Some(t.win.app.clone());
    if what == "image" {
        return match &t.src {
            Some(u) => copy_out(u.clone(), "Copied the image address", app).await,
            None => Outcome::fail("I can't read that picture's address.", LAYER),
        };
    }
    match &t.link {
        Some(u) => copy_out(u.clone(), "Copied the link", app).await,
        None => Outcome::fail("That has no link to copy.", LAYER),
    }
}

fn pct_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn ext_for(mime: &str, url: &str) -> String {
    let m = mime.split(';').next().unwrap_or("").trim().to_lowercase();
    let by_mime = match m.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "application/pdf" => "pdf",
        "text/html" => "html",
        _ => "",
    };
    if !by_mime.is_empty() {
        return by_mime.to_owned();
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, e)) if (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()) => e.to_lowercase(),
        _ => "bin".to_owned(),
    }
}

fn stem_for(url: &str, label: &str) -> String {
    let from_url = if url.starts_with("data:") {
        String::new()
    } else {
        let path = url.split(['?', '#']).next().unwrap_or("");
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let last = last.rsplit_once('.').map(|(a, _)| a).unwrap_or(last);
        String::from_utf8_lossy(&pct_decode(last)).into_owned()
    };
    let raw = if from_url.chars().filter(|c| c.is_alphanumeric()).count() >= 3 && !url.starts_with("data:") { from_url } else { label.to_owned() };
    let clean: String = raw.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { ' ' }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.chars().take(60).collect();
    if clean.is_empty() { "image".to_owned() } else { clean }
}

pub fn save_url(url: &str, label: &str) -> Result<std::path::PathBuf, String> {
    use base64::Engine;
    let dir = dirs::download_dir().or_else(|| dirs::home_dir().map(|h| h.join("Downloads"))).ok_or("I can't find your Downloads folder.")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (bytes, mime) = if let Some(rest) = url.strip_prefix("data:") {
        let (meta, body) = rest.split_once(',').ok_or("That picture's data is broken.")?;
        let mime = meta.split(';').next().unwrap_or("").to_owned();
        let bytes = if meta.ends_with(";base64") { base64::engine::general_purpose::STANDARD.decode(body.trim()).map_err(|_| "That picture's data is broken.".to_owned())? } else { pct_decode(body) };
        (bytes, mime)
    } else if let Some(p) = url.strip_prefix("file://") {
        let p = String::from_utf8_lossy(&pct_decode(p)).into_owned();
        (std::fs::read(&p).map_err(|e| e.to_string())?, String::new())
    } else {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).timeout_global(Some(Duration::from_secs(90))).user_agent(format!("Setu/{}", setu_common::VERSION)).build().into();
        let mut r = agent.get(url).call().map_err(|e| format!("The download failed: {e}"))?;
        let mime = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        let bytes = r.body_mut().with_config().limit(512 * 1024 * 1024).read_to_vec().map_err(|e| format!("The download stopped: {e}"))?;
        (bytes, mime)
    };
    if bytes.is_empty() {
        return Err("The file was empty.".into());
    }
    let ext = ext_for(&mime, url);
    let stem = stem_for(url, label);
    let mut path = dir.join(format!("{stem}.{ext}"));
    let mut k = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({k}).{ext}"));
        k += 1;
    }
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

async fn save_target(t: &Target, what: &str) -> Outcome {
    let picture = t.node.role == "image" || what.contains("image") || what.contains("picture") || what.contains("photo");
    if picture && t.src.is_none() {
        return Outcome::fail("I can't read that picture's address, so I can't save it.", LAYER);
    }
    let Some(url) = t.src.clone().or_else(|| t.link.clone()) else { return Outcome::fail("I can't find a file to save there.", LAYER) };
    let label = t.label(what);
    let app = Some(t.win.app.clone());
    match crate::layers::blocking(move || save_url(&url, &label)).await {
        Ok(Ok(p)) => {
            let file = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            Outcome::ok_with(&format!("Saved {file}"), LAYER, app).with_reply(format!("Saved it to Downloads as {file}."))
        }
        Ok(Err(e)) => Outcome::fail(&e, LAYER),
        Err(_) => Outcome::fail("The download stopped.", LAYER),
    }
}

async fn lens_target(core: &Arc<Core>, t: &Target) -> Outcome {
    let Some(src) = t.src.as_ref().filter(|u| u.starts_with("http")) else { return Outcome::fail("I can only search pictures that come from the web.", LAYER) };
    let url = format!("https://lens.google.com/uploadbyurl?url={}", crate::tools::web::encode(src));
    if launch(core, &t.win.app, vec![url]).await { Outcome::ok_with("Searching the image", LAYER, Some(t.win.app.clone())) } else { Outcome::fail("The browser didn't open the search.", LAYER) }
}

fn menu_for(name: &str, t: &Target, what: &str) -> Outcome {
    let base = serde_json::json!({ "node": t.node, "win": t.win, "link": t.link, "src": t.src, "what": what });
    let mut opts: Vec<(&str, &[&str], Value)> = Vec::new();
    if t.link.is_some() {
        opts.push(("Open", &["open", "open it", "open link", "open the link"], serde_json::json!({ "do": "open", "where": "here" })));
        opts.push(("Open in a new tab", &["new tab", "open in new tab", "open in a new tab"], serde_json::json!({ "do": "open", "where": "tab" })));
        opts.push(("Open in a new window", &["new window", "open in new window", "open in a new window"], serde_json::json!({ "do": "open", "where": "window" })));
        opts.push(("Open in a private window", &["private", "private window", "incognito", "open in private"], serde_json::json!({ "do": "open", "where": "private" })));
        opts.push(("Copy link", &["copy link", "copy the link", "copy link address"], serde_json::json!({ "do": "copy", "what": "link" })));
    }
    if let Some(src) = &t.src {
        opts.push(("Open image in a new tab", &["open image", "view image", "open the image", "open image in new tab"], serde_json::json!({ "do": "open", "where": "tab", "full": true })));
        opts.push(("Save image", &["save", "save image", "save the image", "download", "download image"], serde_json::json!({ "do": "save" })));
        opts.push(("Copy image address", &["copy image", "copy image address", "copy the image address", "copy image link"], serde_json::json!({ "do": "copy", "what": "image" })));
        if src.starts_with("http") {
            opts.push(("Search with Google Lens", &["lens", "google lens", "search image", "search the image", "search with google lens"], serde_json::json!({ "do": "lens" })));
        }
    } else if t.link.is_some() {
        opts.push(("Save link", &["save", "save link", "download", "save the link"], serde_json::json!({ "do": "save" })));
    }
    if opts.is_empty() {
        opts.push(("Click", &["click", "press", "click it"], serde_json::json!({ "do": "click" })));
    }
    let choices: Vec<Choice> = opts
        .into_iter()
        .map(|(label, words, op)| {
            let mut native = base.clone();
            if let (Some(n), Some(o)) = (native.as_object_mut(), op.as_object()) {
                for (k, v) in o {
                    n.insert(k.clone(), v.clone());
                }
            }
            Choice::new(label, words, Intent::Act { tool: "browser.menu".into(), args: serde_json::json!({ "native": native, "browser": name }) })
        })
        .collect();
    let labels: Vec<String> = choices.iter().map(|c| c.label.to_lowercase()).collect();
    let spoken = if labels.len() <= 5 { format!("You can {}. Which one?", crate::tools::media::join_and(&labels)) } else { format!("Here are {} options. Say one, or its number.", labels.len()) };
    let label = t.label(what);
    Outcome::asking(
        "Showed the options",
        LAYER,
        FollowUp { question: format!("Options for {label}"), spoken: Some(spoken), choices, cleanup: Some(Intent::Act { tool: "browser.menu".into(), args: serde_json::json!({ "close": true, "native": true, "browser": name }) }), kind: "", focus: None },
    )
}

async fn run_on(core: &Arc<Core>, name: &str, t: &Target, op: &str, args: &Value) -> Outcome {
    let what = s(args, "what");
    let place = { let p = s(args, "where"); if p.is_empty() { "here".to_owned() } else { p } };
    let full = args.get("full").and_then(Value::as_bool).unwrap_or(false);
    match op {
        "open" => open_at(core, t, &place, full, if what.is_empty() { "it" } else { &what }).await,
        "click" => match os::act(&t.node, Do::Invoke).await {
            Ok(()) => Outcome::ok_with(&format!("Clicked {}", t.label(&what)), LAYER, Some(t.win.app.clone())),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "copy" => copy_target(t, if what == "image" { "image" } else { "link" }).await,
        "save" => save_target(t, &what).await,
        "lens" => lens_target(core, t).await,
        "menu" => menu_for(name, t, &what),
        _ => Outcome::fail("I don't know that page command.", LAYER),
    }
}

pub async fn page_act(core: &Arc<Core>, name: &str, args: &Value) -> Option<Outcome> {
    let action = { let a = s(args, "action"); if a.is_empty() { "open".to_owned() } else { a } };
    let r = args.get("ref").cloned().unwrap_or(Value::Null);
    let kind = r.get("kind").and_then(Value::as_str).or_else(|| args.get("kind").and_then(Value::as_str)).unwrap_or("any").to_owned();
    match action.as_str() {
        "numbers" => return number(core, name, &kind).await,
        "numbers.clear" => {
            clear_numbers(core);
            return Some(Outcome::ok_with("Cleared the numbers", LAYER, None));
        }
        "find" => return find(name, &s(args, "text")).await,
        "open" | "click" | "copy" | "save" | "lens" | "menu" => {}
        _ => return None,
    }
    let text = kind == "image" || matches!(action.as_str(), "copy" | "save" | "lens" | "menu");
    let what = { let w = s(args, "what"); if w.is_empty() { "that".to_owned() } else { w } };
    let mut v = view_scope(name, text).await?;
    let mut found = resolve(&v, &v.page(), &r, &kind, &what);
    if found.is_err() && r.get("number").is_none() {
        tokio::time::sleep(Duration::from_millis(350)).await;
        v = view_scope(name, text).await?;
        found = resolve(&v, &v.page(), &r, &kind, &what);
    }
    let page = v.page();
    let node = match found {
        Ok(n) => n,
        Err(e) => return Some(Outcome::ok_with("Not found", LAYER, Some(v.win.app.clone())).with_reply(e)),
    };
    let place = s(args, "where");
    let full = args.get("full").and_then(Value::as_bool).unwrap_or(false);
    let copy_what = s(args, "what");
    let need_link = match action.as_str() {
        "open" => place != "here" && !place.is_empty(),
        "copy" => copy_what != "image",
        "save" | "menu" => true,
        _ => false,
    };
    let need_src = match action.as_str() {
        "open" => full,
        "copy" => copy_what == "image",
        "save" | "lens" | "menu" => true,
        _ => false,
    };
    let link = if need_link { link_of(&page, &node).await } else { None };
    let src = if need_src { src_of(&page, &node).await } else { None };
    if action == "copy" && copy_what != "image" && link.is_none() && r.get("this").and_then(Value::as_bool).unwrap_or(false) {
        return copy_url(name).await;
    }
    let t = Target { win: v.win.clone(), node: node.clone(), link, src };
    *LAST.lock().unwrap() = Some((v.win.id.clone(), node));
    let from_number = r.get("number").is_some();
    let o = run_on(core, name, &t, &action, args).await;
    if from_number && action != "menu" {
        clear_numbers(core);
    }
    Some(o)
}

pub async fn menu_choose(core: &Arc<Core>, args: &Value) -> Outcome {
    if args.get("close").and_then(Value::as_bool).unwrap_or(false) {
        clear_numbers(core);
        return Outcome::ok_with("Closed the options", LAYER, None);
    }
    let nat = args.get("native").cloned().unwrap_or(Value::Null);
    let (Ok(node), Ok(win)) = (serde_json::from_value::<Node>(nat.get("node").cloned().unwrap_or(Value::Null)), serde_json::from_value::<Win>(nat.get("win").cloned().unwrap_or(Value::Null))) else {
        return Outcome::fail("That item is no longer on the page.", LAYER);
    };
    let t = Target { win, node, link: nat.get("link").and_then(Value::as_str).map(str::to_owned), src: nat.get("src").and_then(Value::as_str).map(str::to_owned) };
    let op = s(&nat, "do");
    let name = s(args, "browser");
    let o = run_on(core, &name, &t, &op, &nat).await;
    clear_numbers(core);
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(role: &str, name: &str, x: i32, y: i32, w: i32, h: i32) -> Node {
        Node { id: format!("{name}-{x}-{y}"), role: role.into(), name: name.into(), x, y, w, h, enabled: true, web: true, ..Default::default() }
    }

    #[test]
    fn picks_tabs_and_page_items() {
        let tabs_owned = vec![n("tab", "WhatsApp", 40, 10, 200, 40), n("tab", "YouTube - Audio playing", 240, 10, 200, 40), n("tab", "Gmail", 440, 10, 200, 40)];
        let tabs: Vec<&Node> = tabs_owned.iter().collect();
        assert_eq!(pick_tab(&tabs, &json!({ "index": 2 }))[0].name, "YouTube - Audio playing");
        assert_eq!(pick_tab(&tabs, &json!({ "index": -1 }))[0].name, "Gmail");
        assert_eq!(pick_tab(&tabs, &json!({ "query": "youtube" }))[0].name, "YouTube - Audio playing");
        assert!(pick_tab(&tabs, &json!({ "query": "netflix" })).is_empty());
        assert_eq!(tab_title("YouTube - Audio playing"), "YouTube");
        let page_owned = vec![
            n("link", "Images", 100, 120, 60, 20),
            n("link", "Mr Beast official channel", 20, 200, 400, 20),
            n("link", "Mr Beast new video", 20, 260, 400, 20),
            n("link", "Thumb one", 20, 400, 120, 90),
            n("link", "Thumb two", 160, 400, 120, 90),
            n("link", "Thumb three", 300, 402, 120, 90),
            n("button", "Search", 600, 120, 40, 20),
        ];
        let page: Vec<&Node> = page_owned.iter().collect();
        assert_eq!(pick_ref(&page, &json!({ "kind": "result", "n": 2 })).unwrap().name, "Mr Beast new video");
        assert_eq!(pick_ref(&page, &json!({ "kind": "image", "n": 3, "from": "left" })).unwrap().name, "Thumb three");
        assert_eq!(pick_ref(&page, &json!({ "kind": "image", "n": 1, "from": "right" })).unwrap().name, "Thumb three");
        assert_eq!(pick_ref(&page, &json!({ "kind": "any", "text": "images" })).unwrap().name, "Images");
        assert_eq!(pick_ref(&page, &json!({ "kind": "link", "n": -1 })).unwrap().name, "Thumb three");
        let mixed_owned = vec![n("link", "Thumb one", 20, 400, 120, 93), n("image", "Thumb one", 20, 400, 120, 90), n("link", "Thumb two", 160, 400, 120, 93), n("image", "Thumb two", 160, 400, 120, 90), n("image", "icon", 600, 10, 16, 16)];
        let mixed: Vec<&Node> = mixed_owned.iter().collect();
        let imgs = candidates(&mixed, "image");
        assert_eq!(imgs.iter().map(|n| n.role.as_str()).collect::<Vec<_>>(), vec!["image", "image"]);
        assert_eq!(pick_ref(&mixed, &json!({ "kind": "image", "n": 2 })).unwrap().name, "Thumb two");
        assert_eq!(bare("Open a new tab (Ctrl+T)"), "open a new tab");
        assert_eq!(bare("Bookmark this page (Ctrl+D)"), "bookmark this page");
        assert_eq!(bare("Back"), "back");
        assert!(is_star("Bookmark this tab") && is_star("Edit this bookmark (Ctrl+D)") && !is_star("Bookmarks"));
        assert!(is_marked("Edit bookmark for this tab") && !is_marked("Bookmark this tab"));
        assert_eq!(pct_decode("%3Csvg%20x%3D'1'%3E"), b"<svg x='1'>".to_vec());
        assert_eq!(pct_decode("100%"), b"100%".to_vec());
        assert_eq!(ext_for("image/svg+xml", "data:image/svg+xml,abc"), "svg");
        assert_eq!(ext_for("", "https://x.com/a/cat.PNG?w=2"), "png");
        assert_eq!(ext_for("", "https://x.com/a/photo"), "bin");
        assert_eq!(stem_for("https://x.com/img/mr%20beast.jpg?x=1", "Thumb"), "mr beast");
        assert_eq!(stem_for("data:image/png;base64,AAAA", "Thumb two"), "Thumb two");
        assert_eq!(stem_for("https://x.com/", ""), "image");
        assert_eq!(launch_flags("Firefox", "window", "u"), vec!["-new-window".to_owned(), "u".to_owned()]);
        assert_eq!(launch_flags("Microsoft Edge", "private", "u")[0], "--inprivate");
        assert_eq!(launch_flags("Brave Browser", "tab", "u"), vec!["u".to_owned()]);
        assert!(matches_browser("Brave Browser", "brave"));
        assert!(matches_browser("Google Chrome", "chrome"));
        assert!(!matches_browser("Firefox", "brave"));
    }
}
