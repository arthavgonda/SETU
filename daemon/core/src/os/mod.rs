use crate::layers::{self, Layer};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
mod jab;
#[cfg(windows)]
mod uia;
#[cfg(windows)]
use uia as imp;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Win {
    pub id: String,
    pub app: String,
    pub title: String,
    #[serde(default)]
    pub pid: u32,
    #[serde(default)]
    pub front: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Node {
    pub id: String,
    pub role: String,
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub w: i32,
    #[serde(default)]
    pub h: i32,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub checked: Option<bool>,
    #[serde(default)]
    pub editable: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub web: bool,
    #[serde(default)]
    pub selected: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq)]
pub enum Do {
    Invoke,
    Toggle,
    Select,
    Expand,
    Focus,
    SetText(String),
    Insert(String),
    Menu,
}

impl Do {
    pub fn parse(s: &str, text: &str) -> Option<Self> {
        Some(match s {
            "invoke" | "click" | "press" => Self::Invoke,
            "toggle" => Self::Toggle,
            "select" => Self::Select,
            "expand" => Self::Expand,
            "focus" => Self::Focus,
            "set_text" | "set" => Self::SetText(text.to_owned()),
            "insert" | "type" => Self::Insert(text.to_owned()),
            "menu" | "context_menu" | "right_click" => Self::Menu,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Scope {
    pub limit: usize,
    pub text: bool,
    pub hidden: bool,
}

impl Default for Scope {
    fn default() -> Self {
        Self { limit: 400, text: false, hidden: false }
    }
}

pub async fn ready() -> Result<()> {
    imp::ready().await
}

static LAST_OTHER: std::sync::Mutex<Option<Win>> = std::sync::Mutex::new(None);

struct Warm {
    id: String,
    text: bool,
    hidden: bool,
    limit: usize,
    nodes: std::sync::Arc<Vec<Node>>,
    at: std::time::Instant,
    marks: (u64, u64),
}

const WARM_FOR: std::time::Duration = std::time::Duration::from_secs(10);
static WARM: std::sync::Mutex<Vec<Warm>> = std::sync::Mutex::new(Vec::new());
static FEED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static CHANGES: std::sync::Mutex<Vec<(String, u64)>> = std::sync::Mutex::new(Vec::new());

fn app_key(id: &str) -> &str {
    id.split('|').next().unwrap_or(id)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn feed(on: bool) {
    FEED.store(on, std::sync::atomic::Ordering::Relaxed);
    if !on {
        WARM.lock().unwrap().clear();
        CHANGES.lock().unwrap().clear();
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn changed(app: &str) {
    let mut g = CHANGES.lock().unwrap();
    match g.iter_mut().find(|(a, _)| a == app) {
        Some((_, n)) => *n += 1,
        None => {
            if g.len() >= 64 {
                g.remove(0);
            }
            g.push((app.to_owned(), 1));
        }
    }
}

pub fn disturbed() {
    EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn marks(id: &str) -> (u64, u64) {
    let app = app_key(id);
    let n = CHANGES.lock().unwrap().iter().find(|(a, _)| a == app).map(|(_, n)| *n).unwrap_or(0);
    (n, EPOCH.load(std::sync::atomic::Ordering::Relaxed))
}

fn warm_hit(id: &str, scope: Scope) -> Option<Vec<Node>> {
    let now = marks(id);
    let g = WARM.lock().unwrap();
    let w = g.iter().find(|w| w.id == id && w.text == scope.text && w.hidden == scope.hidden)?;
    if w.marks != now || w.at.elapsed() > WARM_FOR || (scope.limit > w.limit && w.nodes.len() >= w.limit) {
        return None;
    }
    Some(w.nodes[..w.nodes.len().min(scope.limit.max(1))].to_vec())
}

fn keep_warm(id: &str, scope: Scope, nodes: &[Node], marks: (u64, u64)) {
    let mut g = WARM.lock().unwrap();
    g.retain(|w| !(w.id == id && w.text == scope.text && w.hidden == scope.hidden) && w.at.elapsed() <= WARM_FOR);
    if g.len() >= 6 {
        g.remove(0);
    }
    g.push(Warm { id: id.to_owned(), text: scope.text, hidden: scope.hidden, limit: scope.limit, nodes: std::sync::Arc::new(nodes.to_vec()), at: std::time::Instant::now(), marks });
}

pub async fn warm() {
    if !imp::watch(20).await {
        return;
    }
    if listed().is_none() {
        let _ = refill().await;
    }
    let Ok(w) = working().await else { return };
    imp::wake(&w).await;
    let _ = window_box(&w).await;
    let scope = Scope { limit: 6000, text: false, hidden: false };
    for _ in 0..2 {
        let before = marks(&w.id);
        let s = layers::read(Layer::Accessibility, "accessibility.tree.prewalk");
        let Ok(nodes) = s.end(imp::tree(&w, scope).await) else { return };
        keep_warm(&w.id, scope, &nodes, before);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        if marks(&w.id) == before {
            return;
        }
    }
}
static TYPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TRACKING: std::sync::Once = std::sync::Once::new();

fn note(wins: &[Win]) {
    if let Some(w) = wins.iter().find(|w| w.front && !is_setu(&w.app)) {
        *LAST_OTHER.lock().unwrap() = Some(w.clone());
    }
}

pub fn set_typed(typed: bool) {
    TYPED.store(typed, std::sync::atomic::Ordering::Relaxed);
}

pub fn typed() -> bool {
    TYPED.load(std::sync::atomic::Ordering::Relaxed)
}

fn on_front(w: Win) {
    if w.app.trim().is_empty() {
        return;
    }
    log::debug!("front: {} \"{}\"", w.app, w.title);
    if !is_setu(&w.app) {
        *LAST_OTHER.lock().unwrap() = Some(w);
    }
}

pub fn last_other() -> Option<Win> {
    LAST_OTHER.lock().unwrap().clone()
}

pub fn start_tracking() {
    TRACKING.call_once(|| imp::track(on_front));
}

pub fn is_setu_window(w: &Win) -> bool {
    is_setu(&w.app) && !norm(&w.app).contains("shell") && !matches!(norm(&w.title).as_str(), "numbers" | "notch" | "camera")
}

pub async fn ui_targets(named_setu: bool) -> Result<Vec<Win>> {
    let wins = windows().await?;
    let mine: Option<Win> = wins.iter().filter(|w| is_setu_window(w)).max_by_key(|w| w.front).cloned();
    if named_setu {
        return mine.map(|m| vec![m]).ok_or_else(|| anyhow::anyhow!("Setu's window isn't open."));
    }
    let setu_front = wins.iter().any(|w| w.front && is_setu(&w.app));
    let other = working_in(&wins);
    let mut out = Vec::new();
    if setu_front && !typed() {
        out.extend(mine.clone());
        out.extend(other);
    } else {
        out.extend(other);
        if setu_front {
            out.extend(mine);
        }
    }
    if out.is_empty() {
        return Err(anyhow::anyhow!("No other window is open."));
    }
    Ok(out)
}

fn working_in(wins: &[Win]) -> Option<Win> {
    if let Some(w) = wins.iter().find(|w| w.front && !is_setu(&w.app)) {
        return Some(w.clone());
    }
    let last = LAST_OTHER.lock().unwrap().clone();
    if let Some(l) = last
        && let Some(w) = wins.iter().find(|w| w.id == l.id).or_else(|| wins.iter().find(|w| w.pid == l.pid && w.pid != 0))
    {
        return Some(w.clone());
    }
    wins.iter().find(|w| !is_setu(&w.app)).cloned()
}

struct WinList {
    wins: Vec<Win>,
    epoch: u64,
}

static WIN_LIST: std::sync::Mutex<Option<WinList>> = std::sync::Mutex::new(None);
static WIN_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static WIN_REFILL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn windows_changed() {
    WIN_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if !imp::evented() || WIN_REFILL.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        WIN_REFILL.store(false, std::sync::atomic::Ordering::SeqCst);
        return;
    };
    rt.spawn(async {
        let mut seen = WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            let now = WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
            if now == seen {
                break;
            }
            seen = now;
        }
        WIN_REFILL.store(false, std::sync::atomic::Ordering::SeqCst);
        if imp::evented() {
            let _ = refill().await;
        }
    });
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn win_epoch() -> u64 {
    WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn is_window(id: &str) -> bool {
    WIN_LIST.lock().unwrap().as_ref().is_some_and(|l| l.wins.iter().any(|w| w.id == id))
}

fn listed() -> Option<Vec<Win>> {
    if !imp::evented() {
        return None;
    }
    let e = WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    WIN_LIST.lock().unwrap().as_ref().filter(|l| l.epoch == e).map(|l| l.wins.clone())
}

async fn refill() -> Result<Vec<Win>> {
    let e = WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let w = imp::windows().await?;
    if imp::evented() {
        let mut g = WIN_LIST.lock().unwrap();
        if WIN_EPOCH.load(std::sync::atomic::Ordering::SeqCst) == e {
            *g = Some(WinList { wins: w.clone(), epoch: e });
        }
    }
    Ok(w)
}

pub async fn windows() -> Result<Vec<Win>> {
    if let Some(w) = listed() {
        layers::read(Layer::Accessibility, "accessibility.windows.warm").ok();
        note(&w);
        return Ok(w);
    }
    let s = layers::read(Layer::Accessibility, "accessibility.windows");
    let w = s.end(refill().await)?;
    note(&w);
    Ok(w)
}

pub async fn front() -> Result<Win> {
    if let Some(w) = listed() {
        let s = layers::read(Layer::Accessibility, "accessibility.windows.warm");
        if let Some(f) = w.into_iter().find(|w| w.front) {
            s.ok();
            note(std::slice::from_ref(&f));
            return Ok(f);
        }
        s.forget();
    }
    let s = layers::read(Layer::Accessibility, "accessibility.front");
    let w = if imp::evented() {
        match refill().await {
            Ok(list) => match list.into_iter().find(|w| w.front) {
                Some(f) => Ok(f),
                None => imp::front().await,
            },
            Err(e) => Err(e),
        }
    } else {
        imp::front().await
    };
    let w = s.end(w)?;
    note(std::slice::from_ref(&w));
    Ok(w)
}

pub async fn working() -> Result<Win> {
    let wins = windows().await?;
    working_in(&wins).ok_or_else(|| anyhow::anyhow!("No other window is open."))
}

pub async fn context_line() -> String {
    match tokio::time::timeout(std::time::Duration::from_millis(400), working()).await {
        Ok(Ok(w)) if !w.app.is_empty() => {
            let title: String = w.title.chars().filter(|c| !c.is_control()).take(90).collect();
            if title.is_empty() || norm(&title) == norm(&w.app) { format!("The user is using {}.", w.app) } else { format!("The user is using {}, window \"{}\".", w.app, title) }
        }
        _ => String::new(),
    }
}

pub async fn tree(win: &Win, scope: Scope) -> Result<Vec<Node>> {
    let fed = FEED.load(std::sync::atomic::Ordering::Relaxed);
    if fed && layers::once("tree") {
        let s = layers::read(Layer::Accessibility, "accessibility.tree.warm");
        if let Some(nodes) = warm_hit(&win.id, scope) {
            s.ok();
            return Ok(nodes);
        }
        s.forget();
    }
    let before = marks(&win.id);
    let s = layers::read(Layer::Accessibility, "accessibility.tree");
    let r = s.end(imp::tree(win, scope).await);
    if fed && let Ok(nodes) = &r {
        keep_warm(&win.id, scope, nodes, before);
    }
    r
}

pub async fn act(node: &Node, what: Do) -> Result<()> {
    let s = layers::effect(Layer::Accessibility, "accessibility.act");
    let r = s.end(imp::act(node, what).await);
    disturbed();
    r
}

pub async fn type_into(win: &Win, node: &Node, text: &str, replace: bool) -> Result<&'static str> {
    let direct = if replace { Do::SetText(text.to_owned()) } else { Do::Insert(text.to_owned()) };
    if cfg!(windows) {
        return act(node, direct).await.map(|_| "accessibility");
    }
    if node.editable && act(node, direct).await.is_ok() {
        return Ok("accessibility");
    }
    let _ = act(node, Do::Focus).await;
    if !in_front(win, 1200).await {
        return Err(anyhow::anyhow!("{} isn't in front, so I can't type there.", if win.app.is_empty() { "That window" } else { win.app.as_str() }));
    }
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    if replace {
        crate::tools::input::press(&[crate::tools::input::primary(), crate::tools::input::K::Ch('a')]).await?;
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    }
    crate::tools::input::type_text(text).await?;
    Ok("keyboard")
}

#[cfg(target_os = "linux")]
pub async fn table_cell(win: &Win, row: i32, col: i32) -> Option<String> {
    let s = layers::read(Layer::Accessibility, "accessibility.table");
    s.end_some(linux::table_cell(win, row, col).await)
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub async fn table_cell(_win: &Win, _row: i32, _col: i32) -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
pub async fn window_box(win: &Win) -> Option<(i32, i32, i32, i32)> {
    linux::frame(win).await
}

#[cfg(not(target_os = "linux"))]
pub async fn window_box(_win: &Win) -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(target_os = "macos")]
pub use macos::{minimise_window, screen_rect, set_window_rect, window_rect};

#[cfg(target_os = "linux")]
pub async fn client_origin(win: &Win) -> Option<(i32, i32)> {
    linux::client_origin(win).await
}

#[cfg(not(target_os = "linux"))]
pub async fn client_origin(_win: &Win) -> Option<(i32, i32)> {
    None
}

#[cfg(target_os = "linux")]
pub async fn window_seen(win: &Win) -> Option<((i32, i32), bool)> {
    let active = linux::active(win).await?;
    let size = linux::visible_size(win).await?;
    Some((size, active))
}

#[cfg(not(target_os = "linux"))]
pub async fn window_seen(_win: &Win) -> Option<((i32, i32), bool)> {
    None
}

pub async fn window_origin(win: &Win) -> Option<(i32, i32)> {
    if !cfg!(target_os = "linux") || std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return None;
    }
    match window_box(win).await {
        Some((x, y, _, _)) if x != 0 || y != 0 => None,
        _ => Some((0, 0)),
    }
}

pub async fn in_front(win: &Win, wait_ms: u64) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
    loop {
        if front().await.is_ok_and(|f| f.id == win.id || (f.pid == win.pid && f.pid != 0)) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

pub async fn focused(win: &Win) -> Result<Option<Node>> {
    let s = layers::read(Layer::Accessibility, "accessibility.focused");
    s.end(imp::focused(win).await)
}

pub async fn text(node: &Node, max: usize) -> Option<String> {
    let s = layers::read(Layer::Accessibility, "accessibility.text");
    s.end_some(imp::node_text(node, max).await)
}

pub async fn caret(node: &Node) -> Option<usize> {
    imp::caret(node).await
}

pub async fn wake(win: &Win) {
    imp::wake(win).await
}

pub async fn link_url(node: &Node) -> Option<String> {
    imp::link_url(node).await.filter(|u| u.starts_with("http://") || u.starts_with("https://") || u.starts_with("file://"))
}

pub async fn attr(node: &Node, key: &str) -> Option<String> {
    imp::node_attr(node, key).await.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

pub async fn image_url(node: &Node) -> Option<String> {
    let u = attr(node, "src").await?;
    (u.starts_with("http://") || u.starts_with("https://") || u.starts_with("file://") || u.starts_with("data:image/")).then_some(u)
}

pub async fn find_text(win: &Win, text: &str) -> Result<bool> {
    let s = layers::read(Layer::Accessibility, "accessibility.find_text");
    s.end(imp::find_text(win, text).await)
}

pub async fn popups(win: &Win) -> Result<Vec<(Win, Vec<Node>)>> {
    let s = layers::read(Layer::Accessibility, "accessibility.popups");
    s.end(imp::popups(win).await)
}

#[cfg(target_os = "linux")]
pub async fn asleep() -> Vec<(String, String, bool, u32)> {
    linux::asleep_pids().await
}

#[cfg(not(target_os = "linux"))]
pub async fn asleep() -> Vec<(String, String, bool, u32)> {
    Vec::new()
}

#[cfg(target_os = "linux")]
pub async fn quit_asleep(app: &str) -> bool {
    let low = app.to_lowercase();
    let mut any = false;
    for (name, _, _, pid) in linux::asleep_pids().await {
        let n = name.to_lowercase();
        if n.contains(&low) || low.contains(&n) {
            any |= linux::quit(pid).await;
        }
    }
    any
}

#[cfg(not(target_os = "linux"))]
pub async fn quit_asleep(_app: &str) -> bool {
    false
}

#[cfg(windows)]
pub fn java_state(win: &Win) -> Option<bool> {
    uia::java_state(win)
}

#[cfg(not(windows))]
pub fn java_state(_win: &Win) -> Option<bool> {
    None
}

#[cfg(windows)]
pub fn admin_only(pid: u32) -> bool {
    pid != 0 && uia::elevated(pid) && !uia::elevated(std::process::id())
}

#[cfg(not(windows))]
pub fn admin_only(_pid: u32) -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn trusted() -> bool {
    macos::trusted()
}

#[cfg(not(target_os = "macos"))]
pub fn trusted() -> bool {
    true
}

#[cfg(target_os = "linux")]
pub async fn enabled() -> bool {
    linux::enabled().await
}

#[cfg(target_os = "linux")]
pub async fn set_enabled(on: bool) -> Result<()> {
    linux::set_enabled(on).await
}

#[cfg(not(target_os = "linux"))]
pub async fn enabled() -> bool {
    imp::ready().await.is_ok()
}

#[cfg(not(target_os = "linux"))]
pub async fn set_enabled(_on: bool) -> Result<()> {
    imp::ready().await
}

pub fn norm(s: &str) -> String {
    crate::text::normalize(s)
}

pub fn score(want: &str, name: &str) -> f64 {
    if want.is_empty() || name.is_empty() {
        return 0.0;
    }
    if name == want {
        return 2.0;
    }
    let wn = want.split_whitespace().count().max(1);
    let words: Vec<&str> = name.split_whitespace().collect();
    if name.starts_with(&format!("{want} ")) || words.windows(wn).any(|w| w.join(" ") == want) {
        return 0.9 + 0.05 * (want.len() as f64 / name.len().max(1) as f64);
    }
    crate::text::similarity(want, name)
}

pub fn find<'a>(nodes: &'a [Node], name: &str, roles: &[&str]) -> Option<&'a Node> {
    let want = norm(name);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(f64, usize, &Node)> = None;
    for n in nodes {
        if !roles.is_empty() && !roles.contains(&n.role.as_str()) {
            continue;
        }
        let s = score(&want, &norm(&n.name));
        let rank = role_rank(&n.role);
        if best.as_ref().is_none_or(|(b, r, _)| s > *b || (s == *b && rank < *r)) {
            best = Some((s, rank, n));
        }
    }
    best.filter(|(s, _, _)| *s >= 0.75).map(|(_, _, n)| n)
}

fn role_rank(role: &str) -> usize {
    match role {
        "button" | "link" | "field" | "checkbox" | "radio" | "tab" | "menuitem" | "combobox" | "row" | "item" | "treeitem" => 0,
        "cell" | "toggle" => 1,
        _ => 2,
    }
}

pub fn window_named<'a>(wins: &'a [Win], name: &str) -> Option<&'a Win> {
    let want = norm(name);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(f64, &Win)> = None;
    for w in wins {
        let s = score(&want, &norm(&w.app)).max(score(&want, &norm(&w.title)) * 0.95);
        let s = if w.front { s + 0.01 } else { s };
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, w));
        }
    }
    best.filter(|(s, _)| *s >= 0.75).map(|(_, w)| w)
}

pub fn reading_order(nodes: &mut [Node]) {
    nodes.sort_by_key(|n| (n.y / 8, n.x));
}

pub fn is_setu(app: &str) -> bool {
    let a = norm(app);
    a == "setu" || a == "setu ui" || a == "setu shell" || a == "setu daemon"
}

pub async fn probe() -> i32 {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }
        let out = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(v) => run_probe(&v).await.unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() })),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        println!("{out}");
    }
    0
}

async fn run_probe(v: &serde_json::Value) -> Result<serde_json::Value> {
    use serde_json::json;
    let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    let pick = |wins: &[Win]| -> Option<Win> {
        let name = s("win");
        if name.is_empty() { wins.iter().find(|w| w.front).cloned() } else { wins.iter().find(|w| w.id == name).cloned().or_else(|| window_named(wins, &name).cloned()) }
    };
    let t0 = std::time::Instant::now();
    let r = match s("op").as_str() {
        "ready" => json!({ "ok": ready().await.is_ok() }),
        "windows" => json!({ "windows": windows().await? }),
        "front" => json!({ "window": front().await? }),
        "tree" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            let nodes = tree(&w, Scope { limit: 2000, text: v.get("text").and_then(serde_json::Value::as_bool).unwrap_or(false), hidden: v.get("hidden").and_then(serde_json::Value::as_bool).unwrap_or(false) }).await?;
            json!({ "window": w, "nodes": nodes })
        }
        "act" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            let nodes = tree(&w, Scope::default()).await?;
            let roles: Vec<&str> = v.get("roles").and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(serde_json::Value::as_str).collect()).unwrap_or_default();
            let n = find(&nodes, &s("name"), &roles).cloned().ok_or_else(|| anyhow::anyhow!("not found"))?;
            let what = Do::parse(&s("do"), &s("text")).ok_or_else(|| anyhow::anyhow!("bad action"))?;
            act(&n, what).await?;
            json!({ "done": n })
        }
        "type" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            let nodes = tree(&w, Scope::default()).await?;
            let roles: Vec<&str> = v.get("roles").and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(serde_json::Value::as_str).collect()).unwrap_or_default();
            let n = find(&nodes, &s("name"), &roles).cloned().ok_or_else(|| anyhow::anyhow!("not found"))?;
            let how = type_into(&w, &n, &s("text"), v.get("replace").and_then(serde_json::Value::as_bool).unwrap_or(false)).await?;
            json!({ "done": n, "how": how })
        }
        "track" => {
            start_tracking();
            json!({ "ok": true })
        }
        "working" => json!({ "window": working().await?, "last": last_other() }),
        "focused" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            json!({ "node": focused(&w).await? })
        }
        "attr" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            let nodes = tree(&w, Scope { limit: 600, text: true, hidden: false }).await?;
            let roles: Vec<&str> = v.get("roles").and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(serde_json::Value::as_str).collect()).unwrap_or_default();
            let n = find(&nodes, &s("name"), &roles).cloned().ok_or_else(|| anyhow::anyhow!("not found"))?;
            json!({ "node": n, "value": attr(&n, &s("key")).await })
        }
        "find_text" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            json!({ "found": find_text(&w, &s("text")).await? })
        }
        "popups" => {
            let wins = windows().await?;
            let w = pick(&wins).ok_or_else(|| anyhow::anyhow!("no window"))?;
            let groups = popups(&w).await?;
            json!({ "windows": groups.iter().map(|(w, _)| w.clone()).collect::<Vec<_>>(), "nodes": groups.into_iter().flat_map(|(_, n)| n).collect::<Vec<_>>() })
        }
        "pointer" => return crate::tools::pointer::probe(v).await,
        "ide" | "ide_menu" | "ide_insert" | "ide_goto" | "ide_command" => return crate::ide::probe(v).await.map(|mut r| {
            r["ms"] = json!(t0.elapsed().as_millis() as u64);
            r
        }),
        other => return Err(anyhow::anyhow!("unknown op {other}")),
    };
    let mut r = r;
    r["ms"] = json!(t0.elapsed().as_millis() as u64);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(role: &str, name: &str) -> Node {
        Node { id: name.into(), role: role.into(), name: name.into(), enabled: true, ..Default::default() }
    }

    #[test]
    fn finds_controls_by_name_and_role() {
        let nodes = vec![n("text", "Binod"), n("row", "Binod"), n("field", "Search or start a new chat"), n("button", "Send"), n("row", "Binod Kumar")];
        assert_eq!(find(&nodes, "binod", &[]).map(|x| x.role.as_str()), Some("row"));
        assert_eq!(find(&nodes, "search", &["field"]).map(|x| x.name.as_str()), Some("Search or start a new chat"));
        assert_eq!(find(&nodes, "send", &["button"]).map(|x| x.name.as_str()), Some("Send"));
        assert!(find(&nodes, "papa", &[]).is_none());
        let wins = vec![
            Win { id: "1".into(), app: "Google Chrome".into(), title: "WhatsApp - Google Chrome".into(), pid: 1, front: false },
            Win { id: "2".into(), app: "gedit".into(), title: "Untitled Document 1 - gedit".into(), pid: 2, front: true },
        ];
        assert_eq!(window_named(&wins, "chrome").map(|w| w.id.as_str()), Some("1"));
        assert_eq!(window_named(&wins, "whatsapp").map(|w| w.id.as_str()), Some("1"));
        assert_eq!(window_named(&wins, "gedit").map(|w| w.id.as_str()), Some("2"));
        assert!(is_setu("Setu") && is_setu("setu-ui") && !is_setu("Settings"));
    }

    #[test]
    fn a_warm_tree_is_served_until_something_changes() {
        feed(true);
        let nodes = vec![n("button", "Save"), n("button", "Open"), n("field", "Name")];
        let scope = Scope { limit: 6000, text: false, hidden: false };
        let id = "warmtest:1.7|/win";
        keep_warm(id, scope, &nodes, marks(id));
        assert_eq!(warm_hit(id, Scope { limit: 2, ..scope }).map(|v| v.len()), Some(2));
        assert_eq!(warm_hit(id, scope).map(|v| v.len()), Some(3));
        assert!(warm_hit(id, Scope { text: true, ..scope }).is_none());
        assert!(warm_hit("warmtest:1.8|/win", scope).is_none());
        changed("warmtest:1.7");
        assert!(warm_hit(id, scope).is_none());
        keep_warm(id, scope, &nodes, marks(id));
        assert!(warm_hit(id, scope).is_some());
        disturbed();
        assert!(warm_hit(id, scope).is_none());
        keep_warm(id, Scope { limit: 2, ..scope }, &nodes[..2], marks(id));
        assert!(warm_hit(id, scope).is_none());
        assert!(warm_hit(id, Scope { limit: 2, ..scope }).is_some());
    }
}
