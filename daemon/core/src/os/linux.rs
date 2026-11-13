use super::{Do, Node, Scope, Win};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::Mutex;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const ROOT: &str = "/org/a11y/atspi/accessible/root";
const REGISTRY: &str = "org.a11y.atspi.Registry";
const ACC: &str = "org.a11y.atspi.Accessible";

const ACTIVE: u32 = 1;
const CHECKED: u32 = 4;
const EDITABLE: u32 = 7;
const ENABLED: u32 = 8;
const FOCUSED: u32 = 12;
const PRESSED: u32 = 20;
const SELECTED: u32 = 23;
const SENSITIVE: u32 = 24;
const SHOWING: u32 = 25;
const CHECKABLE: u32 = 41;
const READ_ONLY: u32 = 43;

static CONN: Mutex<Option<Connection>> = Mutex::new(None);

fn has(states: &[u32], bit: u32) -> bool {
    states.get((bit / 32) as usize).is_some_and(|w| w & (1 << (bit % 32)) != 0)
}

async fn conn() -> Result<Connection> {
    let cached = CONN.lock().unwrap().clone();
    if let Some(c) = cached {
        return Ok(c);
    }
    let addr = match std::env::var("AT_SPI_BUS_ADDRESS").ok().filter(|a| !a.trim().is_empty()) {
        Some(a) => a,
        None => {
            let session = crate::syscfg_linux::session().await?;
            let reply = tokio::time::timeout(PATIENCE, session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &()))
                .await
                .map_err(|_| anyhow!("The accessibility service isn't answering."))?
                .map_err(|_| anyhow!("The accessibility service isn't running."))?;
            reply.body().deserialize::<String>()?
        }
    };
    let c = zbus::connection::Builder::address(addr.as_str())?.build().await?;
    for event in ["window:activate", "window:deactivate", "window:create", "window:destroy", "window:minimize", "window:restore", "object:state-changed:active", "object:state-changed:focused", "object:children-changed"] {
        let empty: Vec<String> = Vec::new();
        let _ = tokio::time::timeout(PATIENCE, c.call_method(Some(REGISTRY), "/org/a11y/atspi/registry", Some("org.a11y.atspi.Registry"), "RegisterEvent", &(event, empty, ""))).await;
    }
    *CONN.lock().unwrap() = Some(c.clone());
    Ok(c)
}

fn forget() {
    *CONN.lock().unwrap() = None;
    WATCHING.store(false, std::sync::atomic::Ordering::SeqCst);
    LISTENING.store(false, std::sync::atomic::Ordering::SeqCst);
    *FOCUS.lock().unwrap() = None;
    super::feed(false);
    super::windows_changed();
}

pub fn evented() -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    LISTENING.load(SeqCst) && WATCHING.load(SeqCst)
}

static FOCUS: Mutex<Option<(String, String)>> = Mutex::new(None);

fn event_kind(m: &zbus::Message) -> Option<(String, i32)> {
    let (kind, d1, _): (String, i32, i32) = m.body().deserialize_unchecked().ok()?;
    Some((kind, d1))
}

fn note_focus(bus: &str, path: &str, on: bool) {
    let mut g = FOCUS.lock().unwrap();
    if on {
        *g = Some((bus.to_owned(), path.to_owned()));
    } else if g.as_ref().is_some_and(|(b, p)| b == bus && p == path) {
        *g = None;
    }
}

fn on_object(m: &zbus::Message) {
    let h = m.header();
    let Some(sender) = h.sender() else { return };
    super::changed(sender.as_str());
    let path = h.path().map(|p| p.as_str()).unwrap_or_default();
    let member = h.member().map(|m| m.as_str()).unwrap_or_default();
    match member {
        "ChildrenChanged" if path == ROOT => super::windows_changed(),
        "BoundsChanged" => {
            if super::is_window(&format!("{sender}|{path}")) {
                BOUNDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        "StateChanged" | "PropertyChange" => {
            let Some((kind, d1)) = event_kind(m) else { return };
            match (member, kind.as_str()) {
                ("StateChanged", "focused") => note_focus(sender.as_str(), path, d1 != 0),
                ("StateChanged", "active") => super::windows_changed(),
                ("StateChanged", "showing" | "visible" | "iconified" | "defunct") | ("PropertyChange", "accessible-name") => {
                    if super::is_window(&format!("{sender}|{path}")) {
                        super::windows_changed();
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

const WATCHED: &[&str] = &["object:state-changed", "object:property-change", "object:text-changed", "object:bounds-changed", "object:visible-data-changed", "object:selection-changed"];
static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WATCH_UNTIL: Mutex<Option<std::time::Instant>> = Mutex::new(None);

async fn register(c: &Connection, method: &str, event: &str) {
    if method == "RegisterEvent" {
        let empty: Vec<String> = Vec::new();
        let _ = tokio::time::timeout(PATIENCE, c.call_method(Some(REGISTRY), "/org/a11y/atspi/registry", Some("org.a11y.atspi.Registry"), method, &(event, empty, ""))).await;
    } else {
        let _ = tokio::time::timeout(PATIENCE, c.call_method(Some(REGISTRY), "/org/a11y/atspi/registry", Some("org.a11y.atspi.Registry"), method, &(event,))).await;
    }
}

pub async fn watch(secs: u64) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    if !LISTENING.load(SeqCst) {
        return false;
    }
    let Ok(c) = conn().await else { return false };
    *WATCH_UNTIL.lock().unwrap() = Some(std::time::Instant::now() + std::time::Duration::from_secs(secs));
    if WATCHING.swap(true, SeqCst) {
        return true;
    }
    for e in WATCHED {
        register(&c, "RegisterEvent", e).await;
    }
    super::feed(true);
    super::windows_changed();
    tokio::spawn(async move {
        loop {
            let until = *WATCH_UNTIL.lock().unwrap();
            let now = std::time::Instant::now();
            match until {
                Some(u) if u > now => tokio::time::sleep(u - now).await,
                _ => break,
            }
        }
        if WATCHING.swap(false, SeqCst) {
            super::feed(false);
            super::windows_changed();
            for e in WATCHED {
                register(&c, "DeregisterEvent", e).await;
            }
        }
    });
    true
}

const PATIENCE: std::time::Duration = std::time::Duration::from_millis(2500);

static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);

async fn call<B, T>(c: &Connection, bus: &str, path: &str, iface: &str, method: &str, body: &B) -> Result<T>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    T: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    let _permit = GATE.acquire().await.map_err(|_| anyhow!("Accessibility is shutting down."))?;
    let r = match tokio::time::timeout(PATIENCE, c.call_method(Some(bus), path, Some(iface), method, body)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            log::debug!("atspi {method} on {bus}{path}: {e}");
            return Err(e.into());
        }
        Err(_) => {
            log::debug!("atspi {method} on {bus}{path}: timed out");
            return Err(anyhow!("{bus} didn't answer"));
        }
    };
    Ok(r.body().deserialize()?)
}

async fn prop(c: &Connection, bus: &str, path: &str, iface: &str, name: &str) -> Option<OwnedValue> {
    let _permit = GATE.acquire().await.ok()?;
    let r = tokio::time::timeout(PATIENCE, c.call_method(Some(bus), path, Some("org.freedesktop.DBus.Properties"), "Get", &(iface, name))).await.ok()?.ok()?;
    r.body().deserialize::<OwnedValue>().ok()
}

async fn prop_str(c: &Connection, bus: &str, path: &str, iface: &str, name: &str) -> String {
    prop(c, bus, path, iface, name).await.and_then(|v| String::try_from(v).ok()).unwrap_or_default()
}

async fn prop_i32(c: &Connection, bus: &str, path: &str, iface: &str, name: &str) -> Option<i32> {
    prop(c, bus, path, iface, name).await.and_then(|v| i32::try_from(v).ok())
}

async fn prop_f64(c: &Connection, bus: &str, path: &str, iface: &str, name: &str) -> Option<f64> {
    prop(c, bus, path, iface, name).await.and_then(|v| f64::try_from(v).ok())
}

async fn percent_of(c: &Connection, bus: &str, path: &str) -> String {
    const V: &str = "org.a11y.atspi.Value";
    let (now, lo, hi) = tokio::join!(prop_f64(c, bus, path, V, "CurrentValue"), prop_f64(c, bus, path, V, "MinimumValue"), prop_f64(c, bus, path, V, "MaximumValue"));
    match (now, lo.unwrap_or(0.0), hi) {
        (Some(v), lo, Some(hi)) if hi > lo => format!("{:.0}%", ((v - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)),
        _ => String::new(),
    }
}

pub async fn set_enabled(on: bool) -> Result<()> {
    tokio::time::timeout(PATIENCE, async {
        let p = status().await.ok_or_else(|| anyhow!("The accessibility service isn't running."))?;
        p.set_property("IsEnabled", on).await.map_err(|e| anyhow!("{e}"))?;
        ON.store(on, std::sync::atomic::Ordering::Relaxed);
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow!("The accessibility service isn't answering."))?
}

static ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static STATUS: tokio::sync::OnceCell<zbus::Proxy<'static>> = tokio::sync::OnceCell::const_new();

async fn status() -> Option<&'static zbus::Proxy<'static>> {
    STATUS
        .get_or_try_init(|| async {
            let session = crate::syscfg_linux::session().await?;
            let p = zbus::Proxy::new(session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status").await?;
            let mut changes = p.receive_property_changed::<bool>("IsEnabled").await;
            tokio::spawn(async move {
                use futures_util::StreamExt;
                while let Some(c) = changes.next().await {
                    ON.store(c.get().await.unwrap_or(false), std::sync::atomic::Ordering::Relaxed);
                }
                ON.store(false, std::sync::atomic::Ordering::Relaxed);
            });
            Ok::<_, anyhow::Error>(p)
        })
        .await
        .ok()
}

pub async fn enabled() -> bool {
    tokio::time::timeout(PATIENCE, async {
        let Some(p) = status().await else { return false };
        let on = p.get_property::<bool>("IsEnabled").await.unwrap_or(false);
        ON.store(on, std::sync::atomic::Ordering::Relaxed);
        on
    })
    .await
    .unwrap_or(false)
}

pub async fn ready() -> Result<()> {
    if !enabled().await {
        let _ = set_enabled(true).await;
    }
    conn().await.map(|_| ())
}

pub async fn wake(_win: &Win) {
    if ON.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if !enabled().await {
        let _ = set_enabled(true).await;
    }
}

fn split(id: &str) -> Result<(String, String)> {
    id.split_once('|').map(|(b, p)| (b.to_owned(), p.to_owned())).ok_or_else(|| anyhow!("That control is no longer there."))
}

pub async fn table_cell(win: &Win, row: i32, col: i32) -> Option<String> {
    let c = conn().await.ok()?;
    let (bus, path) = split(&win.id).ok()?;
    let raws = walk_menus(&c, &bus, &path, 3000, false, false).await;
    let t = raws.iter().find(|r| r.ifaces.iter().any(|i| i == "org.a11y.atspi.Table") && r.role == 55)?;
    let (cb, cp): (String, OwnedObjectPath) = call(&c, &t.bus, &t.path, "org.a11y.atspi.Table", "GetAccessibleAt", &(row, col)).await.ok()?;
    Some(text_of(&c, &cb, cp.as_str(), 4000).await)
}

static BOUNDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FRAMES: Mutex<Vec<(String, (u64, u64), (i32, i32, i32, i32))>> = Mutex::new(Vec::new());

fn frame_mark() -> (u64, u64) {
    (BOUNDS.load(std::sync::atomic::Ordering::SeqCst), super::win_epoch())
}

pub async fn frame(win: &Win) -> Option<(i32, i32, i32, i32)> {
    let mark = frame_mark();
    if evented()
        && let Some(f) = FRAMES.lock().unwrap().iter().find(|(id, m, _)| *id == win.id && *m == mark).map(|(_, _, f)| *f)
    {
        return Some(f);
    }
    let (bus, path) = split(&win.id).ok()?;
    let c = conn().await.ok()?;
    let f = extents(&c, &bus, &path, 0).await?;
    if evented() && frame_mark() == mark {
        let mut g = FRAMES.lock().unwrap();
        g.retain(|(id, m, _)| *id != win.id && *m == mark);
        if g.len() < 32 {
            g.push((win.id.clone(), mark, f));
        }
    }
    Some(f)
}

pub async fn active(win: &Win) -> Option<bool> {
    let (bus, path) = split(&win.id).ok()?;
    let c = conn().await.ok()?;
    let st = call::<_, Vec<u32>>(&c, &bus, &path, ACC, "GetState", &()).await.ok()?;
    Some(has(&st, ACTIVE))
}

pub async fn client_origin(win: &Win) -> Option<(i32, i32)> {
    let (bus, path) = split(&win.id).ok()?;
    let c = conn().await.ok()?;
    let kids: Vec<(String, OwnedObjectPath)> = call(&c, &bus, &path, ACC, "GetChildren", &()).await.unwrap_or_default();
    let mut best: Option<(i32, i32)> = None;
    for (kb, kp) in kids.iter().take(12) {
        if let Some((x, y, w, h)) = extents(&c, kb, kp.as_str(), 0).await
            && w > 0
            && h > 0
        {
            best = Some(match best {
                Some((bx, by)) => (bx.min(x), by.min(y)),
                None => (x, y),
            });
        }
    }
    best
}

pub async fn visible_size(win: &Win) -> Option<(i32, i32)> {
    let (bus, path) = split(&win.id).ok()?;
    let c = conn().await.ok()?;
    let (_, _, fw, fh) = extents(&c, &bus, &path, 0).await?;
    let kind = coord_kind(&c, &bus, &path).await;
    let kids: Vec<(String, OwnedObjectPath)> = call(&c, &bus, &path, ACC, "GetChildren", &()).await.unwrap_or_default();
    let mut b: Option<(i32, i32, i32, i32)> = None;
    for (kb, kp) in kids.iter().take(12) {
        if let Some((x, y, w, h)) = extents(&c, kb, kp.as_str(), kind).await
            && w > 0
            && h > 0
        {
            b = Some(match b {
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h)),
                None => (x, y, x + w, y + h),
            });
        }
    }
    Some(match b {
        Some((x0, y0, x1, y1)) if x1 - x0 >= fw / 2 && y1 - y0 >= fh / 2 && x1 - x0 <= fw && y1 - y0 <= fh => (x1 - x0, y1 - y0),
        _ => (fw, fh),
    })
}

async fn pid_of(c: &Connection, bus: &str) -> u32 {
    call::<_, u32>(c, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", &(bus,)).await.unwrap_or(0)
}

async fn states(c: &Connection, bus: &str, path: &str) -> Vec<u32> {
    call::<_, Vec<u32>>(c, bus, path, ACC, "GetState", &()).await.unwrap_or_default()
}

async fn app_windows(c: Connection, bus: String) -> Vec<Win> {
    let app = prop_str(&c, &bus, ROOT, ACC, "Name").await;
    let pid = pid_of(&c, &bus).await;
    let kids: Vec<(String, OwnedObjectPath)> = call(&c, &bus, ROOT, ACC, "GetChildren", &()).await.unwrap_or_default();
    let mut out = Vec::new();
    for (wb, wp) in kids {
        let st = states(&c, &wb, wp.as_str()).await;
        if !has(&st, SHOWING) && !has(&st, ACTIVE) {
            continue;
        }
        let title = prop_str(&c, &wb, wp.as_str(), ACC, "Name").await;
        out.push(Win { id: format!("{wb}|{}", wp.as_str()), app: app.clone(), title, pid, front: has(&st, ACTIVE) });
    }
    out
}

pub async fn windows() -> Result<Vec<Win>> {
    let c = conn().await?;
    let apps: Vec<(String, OwnedObjectPath)> = match call(&c, REGISTRY, ROOT, ACC, "GetChildren", &()).await {
        Ok(a) => a,
        Err(e) => {
            forget();
            return Err(anyhow!("I can't read the apps on screen: {e}"));
        }
    };
    let mut set = tokio::task::JoinSet::new();
    for (i, (bus, _)) in apps.into_iter().enumerate() {
        let c2 = c.clone();
        set.spawn(async move { (i, app_windows(c2, bus).await) });
    }
    let mut parts: Vec<(usize, Vec<Win>)> = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(p) = r {
            parts.push(p);
        }
    }
    parts.sort_by_key(|(i, _)| *i);
    Ok(parts.into_iter().flat_map(|(_, w)| w).collect())
}

pub fn track(on: fn(Win)) {
    tokio::spawn(async move {
        loop {
            if let Ok(c) = conn().await {
                if let Err(e) = listen(&c, on).await {
                    log::debug!("window tracking stopped: {e}");
                }
                forget();
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

async fn listen(c: &Connection, on: fn(Win)) -> Result<()> {
    use futures_util::StreamExt;
    let activate = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.a11y.atspi.Event.Window")?.member("Activate")?.build();
    let state = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.a11y.atspi.Event.Object")?.member("StateChanged")?.arg(0, "active")?.build();
    let a = zbus::MessageStream::for_match_rule(activate, c, Some(64)).await?;
    let b = zbus::MessageStream::for_match_rule(state, c, Some(64)).await?;
    let any = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.a11y.atspi.Event.Object")?.build();
    let mut changes = zbus::MessageStream::for_match_rule(any, c, Some(256)).await?;
    let wins = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.a11y.atspi.Event.Window")?.build();
    let mut window_events = zbus::MessageStream::for_match_rule(wins, c, Some(64)).await?;
    tokio::spawn(async move {
        while let Some(m) = changes.next().await {
            if let Ok(m) = m {
                on_object(&m);
            }
        }
    });
    tokio::spawn(async move {
        while let Some(m) = window_events.next().await {
            if m.is_ok() {
                super::windows_changed();
            }
        }
    });
    LISTENING.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut events = futures_util::stream::select(a, b);
    let mut apps: HashMap<String, (String, u32)> = HashMap::new();
    let mut last = String::new();
    while let Some(m) = events.next().await {
        let Ok(m) = m else { continue };
        let h = m.header();
        let (Some(sender), Some(path)) = (h.sender(), h.path()) else { continue };
        let (bus, path) = (sender.to_string(), path.to_string());
        let id = format!("{bus}|{path}");
        if id == last {
            continue;
        }
        let (role, st) = tokio::join!(call::<_, u32>(c, &bus, &path, ACC, "GetRole", &()), states(c, &bus, &path));
        if !matches!(role.unwrap_or(0), 2 | 16 | 23 | 69) || !has(&st, ACTIVE) {
            continue;
        }
        if !apps.contains_key(&bus) {
            let (name, pid) = tokio::join!(prop_str(c, &bus, ROOT, ACC, "Name"), pid_of(c, &bus));
            apps.insert(bus.clone(), (name, pid));
        }
        let (app, pid) = apps.get(&bus).cloned().unwrap_or_default();
        let title = prop_str(c, &bus, &path, ACC, "Name").await;
        last = id.clone();
        on(Win { id, app, title, pid, front: true });
    }
    Ok(())
}

async fn registered() -> Vec<(String, String, u32)> {
    let Ok(c) = conn().await else { return Vec::new() };
    let apps: Vec<(String, OwnedObjectPath)> = call(&c, REGISTRY, ROOT, ACC, "GetChildren", &()).await.unwrap_or_default();
    let mut set = tokio::task::JoinSet::new();
    for (bus, _) in apps {
        let c2 = c.clone();
        set.spawn(async move {
            let (pid, name) = tokio::join!(pid_of(&c2, &bus), prop_str(&c2, &bus, ROOT, ACC, "Name"));
            (bus, name, pid)
        });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(x) = r {
            out.push(x);
        }
    }
    out
}

const SLEEPY_NAMES: &[(&str, bool)] = &[("brave", true), ("chrome", true), ("chromium", true), ("edge", true), ("vivaldi", true), ("opera", true), ("code", false), ("codium", false), ("cursor", false), ("slack", false), ("discord", false), ("signal", false), ("obsidian", false), ("spotify", false)];

fn cmdline0(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline")).ok().and_then(|raw| raw.split(|b| *b == 0).next().map(|a| String::from_utf8_lossy(a).into_owned())).unwrap_or_default()
}

const SLEEPERS: &[(&str, &str, bool)] = &[
    ("brave", "Brave", true),
    ("chrome", "Google Chrome", true),
    ("chromium", "Chromium", true),
    ("chromium-browser", "Chromium", true),
    ("msedge", "Microsoft Edge", true),
    ("vivaldi-bin", "Vivaldi", true),
    ("opera", "Opera", true),
    ("code", "Visual Studio Code", false),
    ("codium", "VSCodium", false),
    ("cursor", "Cursor", false),
    ("slack", "Slack", false),
    ("discord", "Discord", false),
    ("signal-desktop", "Signal", false),
];

pub async fn asleep_pids() -> Vec<(String, String, bool, u32)> {
    let reg = registered().await;
    let pids: Vec<u32> = reg.iter().map(|(_, _, p)| *p).collect();
    let names: Vec<String> = reg.iter().map(|(_, n, _)| super::norm(n)).collect();
    let uid = unsafe { libc::getuid() };
    let mut out: Vec<(String, String, bool, u32)> = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pids.contains(&pid) {
            continue;
        }
        use std::os::unix::fs::MetadataExt;
        if e.metadata().map(|m| m.uid() != uid).unwrap_or(true) {
            continue;
        }
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let args: Vec<String> = raw.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        let Some(first) = args.first() else { continue };
        if args.iter().any(|a| a.starts_with("--type=") || a.contains(" --type=") || a == "-contentproc" || a.contains(" -contentproc")) {
            continue;
        }
        let base = std::path::Path::new(first.split_whitespace().next().unwrap_or(first)).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some((_, name, browser)) = SLEEPERS.iter().find(|(b, _, _)| *b == base) else { continue };
        let n = super::norm(name);
        let first_word = n.split_whitespace().last().unwrap_or(&n).to_owned();
        if names.iter().any(|x| x.contains(&n) || x.contains(&first_word)) || out.iter().any(|(o, _, _, _)| o == name) {
            continue;
        }
        out.push(((*name).to_owned(), first.clone(), *browser, pid));
    }
    if let Ok(c) = conn().await {
        for (bus, name, pid) in reg {
            let n = super::norm(&name);
            let Some((_, browser)) = SLEEPY_NAMES.iter().find(|(k, _)| n.split_whitespace().any(|w| w == *k)) else { continue };
            let wins = app_windows(c.clone(), bus.clone()).await;
            if wins.is_empty() {
                continue;
            }
            let mut empty = true;
            for w in &wins {
                if tree(w, Scope { limit: 8, text: false, hidden: false }).await.map(|t| !t.is_empty()).unwrap_or(false) {
                    empty = false;
                    break;
                }
            }
            if empty && !out.iter().any(|(_, _, _, p)| *p == pid) {
                out.push((name.clone(), cmdline0(pid), *browser, pid));
            }
        }
    }
    out
}

pub async fn quit(pid: u32) -> bool {
    if pid == 0 || unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return false;
    }
    let path = format!("/proc/{pid}/stat");
    for _ in 0..60 {
        let gone = match std::fs::read_to_string(&path) {
            Ok(st) => st.rsplit_once(')').is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
            Err(_) => true,
        };
        if gone {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    false
}

pub async fn front() -> Result<Win> {
    windows().await?.into_iter().find(|w| w.front).ok_or_else(|| anyhow!("I couldn't find the active window. Accessibility may be off for this app."))
}

#[derive(Debug, Clone)]
struct Raw {
    bus: String,
    path: String,
    role: u32,
    name: String,
    states: Vec<u32>,
    ifaces: Vec<String>,
    kids: Vec<(String, String)>,
}

async fn raw(c: Connection, bus: String, path: String) -> Option<Raw> {
    let (role, st, ifaces, kids, name) = tokio::join!(
        call::<_, u32>(&c, &bus, &path, ACC, "GetRole", &()),
        call::<_, Vec<u32>>(&c, &bus, &path, ACC, "GetState", &()),
        call::<_, Vec<String>>(&c, &bus, &path, ACC, "GetInterfaces", &()),
        call::<_, Vec<(String, OwnedObjectPath)>>(&c, &bus, &path, ACC, "GetChildren", &()),
        prop_str(&c, &bus, &path, ACC, "Name"),
    );
    Some(Raw {
        role: role.ok()?,
        states: st.unwrap_or_default(),
        ifaces: ifaces.unwrap_or_default(),
        kids: kids.unwrap_or_default().into_iter().map(|(b, p)| (b, p.as_str().to_owned())).collect(),
        name,
        bus,
        path,
    })
}

fn menu_role(role: u32) -> bool {
    matches!(role, 8 | 33 | 34 | 35 | 41 | 45 | 59)
}

async fn walk(c: &Connection, bus: &str, root: &str, max: usize, deep: bool) -> Vec<Raw> {
    walk_menus(c, bus, root, max, deep, false).await
}

async fn walk_menus(c: &Connection, bus: &str, root: &str, max: usize, deep: bool, menus: bool) -> Vec<Raw> {
    let mut by_key: HashMap<String, Raw> = HashMap::new();
    let mut level: Vec<(String, String)> = vec![(bus.to_owned(), root.to_owned())];
    let mut depth = 0;
    while !level.is_empty() && depth < 64 && by_key.len() < max {
        let mut set = tokio::task::JoinSet::new();
        for (i, (b, p)) in level.drain(..).enumerate() {
            let c2 = c.clone();
            set.spawn(async move { (i, raw(c2, b, p).await) });
        }
        let mut got: Vec<(usize, Raw)> = Vec::new();
        while let Some(r) = set.join_next().await {
            if let Ok((i, Some(r))) = r {
                got.push((i, r));
            }
        }
        got.sort_by_key(|(i, _)| *i);
        let mut next = Vec::new();
        for (_, r) in got {
            let shown = deep || depth == 0 || has(&r.states, SHOWING) || (menus && menu_role(r.role));
            if shown {
                for k in &r.kids {
                    next.push(k.clone());
                }
            }
            by_key.insert(format!("{}|{}", r.bus, r.path), r);
        }
        level = next;
        depth += 1;
    }
    let mut out = Vec::new();
    let mut stack = vec![format!("{bus}|{root}")];
    let mut seen = std::collections::HashSet::new();
    while let Some(k) = stack.pop() {
        if !seen.insert(k.clone()) {
            continue;
        }
        let Some(r) = by_key.remove(&k) else { continue };
        for (b, p) in r.kids.iter().rev() {
            stack.push(format!("{b}|{p}"));
        }
        out.push(r);
    }
    out
}

fn web_set(raws: &[Raw], parent: &HashMap<String, String>) -> std::collections::HashSet<String> {
    let docs: std::collections::HashSet<String> = raws.iter().filter(|r| r.role == 95).map(|r| format!("{}|{}", r.bus, r.path)).collect();
    let mut out = std::collections::HashSet::new();
    for r in raws {
        let key = format!("{}|{}", r.bus, r.path);
        let mut cur = key.clone();
        for _ in 0..80 {
            match parent.get(&cur) {
                Some(p) if docs.contains(p) || out.contains(p) => {
                    out.insert(key.clone());
                    break;
                }
                Some(p) => cur = p.clone(),
                None => break,
            }
        }
    }
    out
}

fn parents(raws: &[Raw]) -> HashMap<String, String> {
    let mut parent = HashMap::new();
    for r in raws {
        let key = format!("{}|{}", r.bus, r.path);
        for (b, p) in &r.kids {
            parent.insert(format!("{b}|{p}"), key.clone());
        }
    }
    parent
}

fn role_of(r: u32, editable: bool) -> &'static str {
    match r {
        43 | 129 => "button",
        62 => "toggle",
        7 | 8 => "checkbox",
        44 | 45 => "radio",
        88 => "link",
        40 => "password",
        79 | 76 | 77 => "field",
        61 if editable => "field",
        11 => "combobox",
        37 => "tab",
        35 | 59 => "menuitem",
        32 => "item",
        90 => "row",
        91 => "treeitem",
        56 => "cell",
        52 => "spinner",
        51 => "slider",
        95 | 82 | 94 => "document",
        27 => "image",
        83 => "heading",
        116 | 29 | 73 | 61 => "text",
        106 => "audio",
        107 => "video",
        23 | 69 => "window",
        16 | 2 => "dialog",
        31 | 98 => "list",
        42 => "progress",
        _ => "other",
    }
}

const CLICKY: &[&str] = &["button", "toggle", "checkbox", "radio", "link", "field", "password", "combobox", "tab", "menuitem", "item", "row", "treeitem", "spinner", "slider"];

fn keep(r: &Raw, text: bool, hidden: bool) -> bool {
    if !has(&r.states, SHOWING) && !(hidden && menu_role(r.role)) {
        return false;
    }
    let editable = has(&r.states, EDITABLE) && r.ifaces.iter().any(|i| i == "org.a11y.atspi.EditableText");
    let role = role_of(r.role, editable);
    let actionable = r.ifaces.iter().any(|i| i == "org.a11y.atspi.Action");
    if editable || CLICKY.contains(&role) || role == "progress" {
        return true;
    }
    let named = !r.name.trim().is_empty();
    if actionable && named && matches!(role, "cell" | "image" | "text" | "other" | "heading") {
        return true;
    }
    text && named && matches!(role, "text" | "heading" | "cell" | "document" | "image")
}

async fn extents(c: &Connection, bus: &str, path: &str, kind: u32) -> Option<(i32, i32, i32, i32)> {
    let first = call::<_, (i32, i32, i32, i32)>(c, bus, path, "org.a11y.atspi.Component", "GetExtents", &(kind,)).await.ok()?;
    if first != (-1, -1, -1, -1) {
        return Some(first);
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    call::<_, (i32, i32, i32, i32)>(c, bus, path, "org.a11y.atspi.Component", "GetExtents", &(kind,)).await.ok()
}

async fn coord_kind(c: &Connection, bus: &str, path: &str) -> u32 {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return 0;
    }
    match extents(c, bus, path, 0).await {
        Some((x, y, _, _)) if x != 0 || y != 0 => 0,
        _ => 1,
    }
}

async fn text_of(c: &Connection, bus: &str, path: &str, max: i32) -> String {
    let n = prop_i32(c, bus, path, "org.a11y.atspi.Text", "CharacterCount").await.unwrap_or(0).min(max);
    if n <= 0 {
        return String::new();
    }
    call::<_, String>(c, bus, path, "org.a11y.atspi.Text", "GetText", &(0i32, n)).await.unwrap_or_default()
}

async fn finish(c: Connection, r: Raw, kind: u32) -> Node {
    let editable = has(&r.states, EDITABLE) && r.ifaces.iter().any(|i| i == "org.a11y.atspi.EditableText");
    let role = role_of(r.role, editable);
    let (ext, value) = tokio::join!(extents(&c, &r.bus, &r.path, kind), async {
        if (editable || matches!(role, "field" | "password" | "combobox" | "spinner" | "slider")) && r.ifaces.iter().any(|i| i == "org.a11y.atspi.Text") && role != "password" {
            text_of(&c, &r.bus, &r.path, 500).await
        } else if role == "progress" && r.ifaces.iter().any(|i| i == "org.a11y.atspi.Value") {
            percent_of(&c, &r.bus, &r.path).await
        } else {
            String::new()
        }
    });
    let (x, y, w, h) = ext.unwrap_or((0, 0, 0, 0));
    let checkable = has(&r.states, CHECKABLE) || matches!(role, "checkbox" | "radio" | "toggle");
    Node {
        id: format!("{}|{}", r.bus, r.path),
        role: if editable && role != "password" { "field".into() } else { role.into() },
        name: r.name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned(),
        value,
        x,
        y,
        w,
        h,
        focused: has(&r.states, FOCUSED),
        checked: checkable.then(|| has(&r.states, CHECKED) || has(&r.states, PRESSED)),
        editable: editable && !has(&r.states, READ_ONLY),
        enabled: has(&r.states, SENSITIVE) || has(&r.states, ENABLED),
        web: false,
        selected: has(&r.states, SELECTED),
    }
}

pub async fn tree(win: &Win, scope: Scope) -> Result<Vec<Node>> {
    let c = conn().await?;
    let (bus, path) = split(&win.id)?;
    let (raws, kind) = tokio::join!(walk_menus(&c, &bus, &path, 6000, false, scope.hidden), coord_kind(&c, &bus, &path));
    if raws.is_empty() {
        return Err(anyhow!("I can't read {}. It may have closed.", if win.app.is_empty() { "that window" } else { win.app.as_str() }));
    }
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut clicky: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in &raws {
        let key = format!("{}|{}", r.bus, r.path);
        let editable = has(&r.states, EDITABLE);
        if CLICKY.contains(&role_of(r.role, editable)) {
            clicky.insert(key.clone());
        }
        for (b, p) in &r.kids {
            parent.insert(format!("{b}|{p}"), key.clone());
        }
    }
    let web = web_set(&raws, &parent);
    let in_web = |key: &str| web.contains(key);
    let inside_clicky = |key: &str| -> bool {
        let mut cur = key.to_owned();
        for _ in 0..12 {
            match parent.get(&cur) {
                Some(p) if clicky.contains(p) => return true,
                Some(p) => cur = p.clone(),
                None => return false,
            }
        }
        false
    };
    let picked: Vec<Raw> = raws
        .into_iter()
        .skip(1)
        .filter(|r| keep(r, scope.text, scope.hidden))
        .filter(|r| scope.text || CLICKY.contains(&role_of(r.role, has(&r.states, EDITABLE))) || !inside_clicky(&format!("{}|{}", r.bus, r.path)))
        .take(scope.limit.max(1))
        .collect();
    let marks: Vec<bool> = picked.iter().map(|r| in_web(&format!("{}|{}", r.bus, r.path))).collect();
    let mut set = tokio::task::JoinSet::new();
    for (i, r) in picked.into_iter().enumerate() {
        let c2 = c.clone();
        let w = marks[i];
        set.spawn(async move {
            let mut n = finish(c2, r, kind).await;
            n.web = w;
            (i, n)
        });
    }
    let mut out: Vec<(usize, Node)> = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(p) = r {
            out.push(p);
        }
    }
    out.sort_by_key(|(i, _)| *i);
    Ok(out.into_iter().map(|(_, n)| n).collect())
}

pub async fn node_attr(node: &Node, key: &str) -> Option<String> {
    let c = conn().await.ok()?;
    let (bus, path) = split(&node.id).ok()?;
    for attempt in 0..2 {
        let m: HashMap<String, String> = call(&c, &bus, &path, ACC, "GetAttributes", &()).await.ok()?;
        if let Some(v) = m.get(key).filter(|v| !v.is_empty()) {
            return Some(v.clone());
        }
        if attempt == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }
    None
}

pub async fn popups(win: &Win) -> Result<Vec<(Win, Vec<Node>)>> {
    let all = windows().await?;
    let mut out = Vec::new();
    for w in all.into_iter().filter(|w| w.id != win.id && w.pid == win.pid && w.pid != 0) {
        if let Ok(n) = tree(&w, Scope { limit: 400, text: false, hidden: false }).await {
            out.push((w, n));
        }
    }
    Ok(out)
}

fn locate(hay: &str, needle: &str) -> Option<(i32, i32)> {
    let h: Vec<char> = hay.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let n: Vec<char> = needle.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    if n.is_empty() || n.len() > h.len() {
        return None;
    }
    let at = h.windows(n.len()).position(|w| w == n.as_slice())?;
    Some((at as i32, (at + n.len()) as i32))
}

pub async fn find_text(win: &Win, text: &str) -> Result<bool> {
    let want = text.trim();
    if want.is_empty() {
        return Ok(false);
    }
    let c = conn().await?;
    let (bus, path) = split(&win.id)?;
    let raws = walk(&c, &bus, &path, 8000, true).await;
    let parent = parents(&raws);
    let web = web_set(&raws, &parent);
    let cands: Vec<(String, String)> = raws
        .iter()
        .filter(|r| r.role != 95 && web.contains(&format!("{}|{}", r.bus, r.path)) && r.ifaces.iter().any(|i| i == "org.a11y.atspi.Text"))
        .map(|r| (r.bus.clone(), r.path.clone()))
        .collect();
    let mut set = tokio::task::JoinSet::new();
    for (i, (b, p)) in cands.into_iter().enumerate() {
        let c2 = c.clone();
        set.spawn(async move {
            let t = text_of(&c2, &b, &p, 20000).await;
            (i, b, p, t)
        });
    }
    let mut got: Vec<(usize, String, String, String)> = Vec::new();
    while let Some(r) = set.join_next().await {
        if let Ok(x) = r {
            got.push(x);
        }
    }
    got.sort_by_key(|(i, _, _, _)| *i);
    for (_, b, p, t) in got {
        let Some((start, end)) = locate(&t, want) else { continue };
        let scrolled: bool = call(&c, &b, &p, "org.a11y.atspi.Text", "ScrollSubstringTo", &(start, end, 6u32)).await.unwrap_or(false);
        if !scrolled {
            let _ = call::<_, bool>(&c, &b, &p, "org.a11y.atspi.Component", "ScrollTo", &(6u32,)).await;
        }
        let n: i32 = call(&c, &b, &p, "org.a11y.atspi.Text", "GetNSelections", &()).await.unwrap_or(0);
        let _ = if n > 0 {
            call::<_, bool>(&c, &b, &p, "org.a11y.atspi.Text", "SetSelection", &(0i32, start, end)).await
        } else {
            call::<_, bool>(&c, &b, &p, "org.a11y.atspi.Text", "AddSelection", &(start, end)).await
        };
        return Ok(true);
    }
    Ok(false)
}

pub async fn node_text(node: &Node, max: usize) -> Option<String> {
    let c = conn().await.ok()?;
    let (bus, path) = split(&node.id).ok()?;
    let t = text_of(&c, &bus, &path, max.min(i32::MAX as usize) as i32).await;
    (!t.is_empty()).then_some(t)
}

pub async fn caret(node: &Node) -> Option<usize> {
    let c = conn().await.ok()?;
    let (bus, path) = split(&node.id).ok()?;
    prop_i32(&c, &bus, &path, "org.a11y.atspi.Text", "CaretOffset").await.filter(|v| *v >= 0).map(|v| v as usize)
}

pub async fn link_url(node: &Node) -> Option<String> {
    let c = conn().await.ok()?;
    let (bus, path) = split(&node.id).ok()?;
    call::<_, String>(&c, &bus, &path, "org.a11y.atspi.Hyperlink", "GetURI", &(0i32,)).await.ok().filter(|u| !u.is_empty())
}

async fn focus_of(win: &Win) -> Option<Node> {
    if !LISTENING.load(std::sync::atomic::Ordering::SeqCst) {
        return None;
    }
    let (bus, path) = FOCUS.lock().unwrap().clone()?;
    let (wbus, wpath) = split(&win.id).ok()?;
    if wbus != bus {
        return None;
    }
    let c = conn().await.ok()?;
    let (role, st, ifaces, name, count, kind) = tokio::join!(
        call::<_, u32>(&c, &bus, &path, ACC, "GetRole", &()),
        call::<_, Vec<u32>>(&c, &bus, &path, ACC, "GetState", &()),
        call::<_, Vec<String>>(&c, &bus, &path, ACC, "GetInterfaces", &()),
        prop_str(&c, &bus, &path, ACC, "Name"),
        prop_i32(&c, &bus, &path, "org.a11y.atspi.Text", "CharacterCount"),
        coord_kind(&c, &wbus, &wpath),
    );
    let r = Raw { role: role.ok()?, states: st.ok()?, ifaces: ifaces.unwrap_or_default(), kids: Vec::new(), name, bus, path };
    let editable = has(&r.states, EDITABLE) && r.ifaces.iter().any(|i| i == "org.a11y.atspi.EditableText");
    if !has(&r.states, FOCUSED) || !editable || has(&r.states, READ_ONLY) {
        return None;
    }
    let password = role_of(r.role, true) == "password";
    let n = count.unwrap_or(0).min(500);
    let (ext, value) = tokio::join!(extents(&c, &r.bus, &r.path, kind), async {
        if n > 0 && !password && r.ifaces.iter().any(|i| i == "org.a11y.atspi.Text") { call::<_, String>(&c, &r.bus, &r.path, "org.a11y.atspi.Text", "GetText", &(0i32, n)).await.unwrap_or_default() } else { String::new() }
    });
    let (x, y, w, h) = ext.unwrap_or((0, 0, 0, 0));
    Some(Node {
        id: format!("{}|{}", r.bus, r.path),
        role: if password { "password".into() } else { "field".into() },
        name: r.name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned(),
        value,
        x,
        y,
        w,
        h,
        focused: true,
        checked: None,
        editable: true,
        enabled: has(&r.states, SENSITIVE) || has(&r.states, ENABLED),
        web: false,
        selected: has(&r.states, SELECTED),
    })
}

pub async fn focused(win: &Win) -> Result<Option<Node>> {
    if let Some(n) = focus_of(win).await {
        return Ok(Some(n));
    }
    let nodes = tree(win, Scope { limit: 2000, text: false, hidden: false }).await?;
    Ok(nodes.into_iter().find(|n| n.focused && n.editable))
}

fn patient(node: &Node) -> bool {
    node.web || matches!(node.role.as_str(), "link" | "button" | "checkbox" | "radio" | "toggle" | "menuitem" | "tab" | "combobox")
}

async fn action_names(c: &Connection, bus: &str, path: &str, patient: bool) -> Vec<String> {
    let first = action_list(c, bus, path).await;
    if (first.is_empty() && !patient) || first.iter().any(|n| !n.trim().is_empty()) {
        return first;
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    action_list(c, bus, path).await
}

async fn action_list(c: &Connection, bus: &str, path: &str) -> Vec<String> {
    let listed: Vec<(String, String, String)> = call(c, bus, path, "org.a11y.atspi.Action", "GetActions", &()).await.unwrap_or_default();
    let mut names: Vec<String> = listed.into_iter().map(|(n, _, _)| n).collect();
    if names.iter().all(|n| n.trim().is_empty()) {
        let count = prop_i32(c, bus, path, "org.a11y.atspi.Action", "NActions").await.unwrap_or(names.len() as i32).clamp(0, 16);
        let mut named = Vec::new();
        for i in 0..count {
            named.push(call::<_, String>(c, bus, path, "org.a11y.atspi.Action", "GetName", &(i,)).await.unwrap_or_default());
        }
        names = named;
    }
    names
}

fn pick_action(names: &[String], want: &[&str]) -> Option<usize> {
    let low: Vec<String> = names.iter().map(|n| n.to_lowercase().replace(['_', '-'], " ")).collect();
    for w in want {
        if let Some(i) = low.iter().position(|n| n == w) {
            return Some(i);
        }
    }
    low.iter().position(|n| !n.contains("context") && !n.contains("menu") && !n.contains("focus") && !n.is_empty())
}

pub async fn act(node: &Node, what: Do) -> Result<()> {
    let c = conn().await?;
    let (bus, path) = split(&node.id)?;
    let label = if node.name.is_empty() { "That control".to_owned() } else { node.name.clone() };
    match what {
        Do::Focus => {
            let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.Component", "GrabFocus", &()).await.unwrap_or(false);
            if ok { Ok(()) } else { Err(anyhow!("{label} can't take the focus.")) }
        }
        Do::SetText(t) => {
            let _ = call::<_, bool>(&c, &bus, &path, "org.a11y.atspi.Component", "GrabFocus", &()).await;
            let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.EditableText", "SetTextContents", &(t.as_str(),)).await.map_err(|_| anyhow!("{label} doesn't accept text."))?;
            if ok { Ok(()) } else { Err(anyhow!("{label} didn't accept the text.")) }
        }
        Do::Insert(t) => {
            let _ = call::<_, bool>(&c, &bus, &path, "org.a11y.atspi.Component", "GrabFocus", &()).await;
            let caret = prop_i32(&c, &bus, &path, "org.a11y.atspi.Text", "CaretOffset").await.unwrap_or(-1);
            let caret = if caret < 0 { prop_i32(&c, &bus, &path, "org.a11y.atspi.Text", "CharacterCount").await.unwrap_or(0) } else { caret };
            let len = t.len() as i32;
            let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.EditableText", "InsertText", &(caret.max(0), t.as_str(), len)).await.map_err(|_| anyhow!("{label} doesn't accept typing."))?;
            if ok { Ok(()) } else { Err(anyhow!("{label} didn't accept the text.")) }
        }
        Do::Menu => {
            let names = action_names(&c, &bus, &path, patient(node)).await;
            let i = names
                .iter()
                .map(|n| n.to_lowercase().replace(['_', '-', ' '], ""))
                .position(|n| matches!(n.as_str(), "showcontextmenu" | "contextmenu" | "showmenu" | "menu"))
                .ok_or_else(|| anyhow!("{label} has no menu."))?;
            let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.Action", "DoAction", &(i as i32,)).await.map_err(|_| anyhow!("{label} can't be used any more."))?;
            if ok { Ok(()) } else { Err(anyhow!("{label} didn't open its menu.")) }
        }
        Do::Invoke | Do::Toggle | Do::Select | Do::Expand => {
            let names = action_names(&c, &bus, &path, patient(node)).await;
            let want: &[&str] = match what {
                Do::Toggle => &["toggle", "click", "press", "activate", "dodefault", "jump"],
                Do::Select => &["select", "click", "activate", "press", "dodefault", "jump"],
                Do::Expand => &["expand or contract", "expand", "open", "show menu", "showmenu", "click", "press", "activate"],
                _ => &["click", "press", "activate", "jump", "open", "dodefault", "default", "select", "toggle"],
            };
            if let Some(i) = pick_action(&names, want) {
                let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.Action", "DoAction", &(i as i32,)).await.map_err(|_| anyhow!("{label} can't be used any more."))?;
                return if ok { Ok(()) } else { Err(anyhow!("{label} didn't respond.")) };
            }
            if matches!(what, Do::Select) {
                let parent = prop(&c, &bus, &path, ACC, "Parent").await.and_then(|v| <(String, OwnedObjectPath)>::try_from(v).ok());
                let index: i32 = call(&c, &bus, &path, ACC, "GetIndexInParent", &()).await.unwrap_or(-1);
                if let Some((pb, pp)) = parent
                    && index >= 0
                    && call::<_, bool>(&c, &pb, pp.as_str(), "org.a11y.atspi.Selection", "SelectChild", &(index,)).await.unwrap_or(false)
                {
                    return Ok(());
                }
            }
            if !(node.editable || matches!(node.role.as_str(), "field" | "password" | "combobox" | "spinner")) {
                return Err(anyhow!("{label} can't be clicked."));
            }
            let ok: bool = call(&c, &bus, &path, "org.a11y.atspi.Component", "GrabFocus", &()).await.unwrap_or(false);
            if ok { Ok(()) } else { Err(anyhow!("{label} can't be clicked.")) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_roles_states_and_actions() {
        assert_eq!(role_of(43, false), "button");
        assert_eq!(role_of(42, false), "progress");
        assert_eq!(role_of(61, true), "field");
        assert_eq!(role_of(61, false), "text");
        assert_eq!(role_of(90, false), "row");
        assert!(has(&[1 << SHOWING], SHOWING));
        assert!(!has(&[1 << SHOWING], FOCUSED));
        assert!(has(&[0, 1 << (CHECKABLE - 32)], CHECKABLE));
        let names = vec!["showContextMenu".to_owned(), "press".to_owned()];
        assert_eq!(pick_action(&names, &["click", "press"]), Some(1));
        assert_eq!(pick_action(&["doDefault".to_owned()], &["click", "dodefault"]), Some(0));
        assert_eq!(pick_action(&[], &["click"]), None);
        assert_eq!(pick_action(&["showContextMenu".to_owned()], &["click"]), None);
        assert_eq!(pick_action(&["SetFocus".to_owned()], &["click", "press"]), None);
        assert_eq!(pick_action(&["SetFocus".to_owned(), "Press".to_owned()], &["click", "press"]), Some(1));
    }

    #[test]
    fn event_kinds_are_read_from_both_signal_layouts() {
        use std::collections::HashMap;
        use zbus::zvariant::Value;
        let new = zbus::Message::signal("/org/a11y/atspi/accessible/7", "org.a11y.atspi.Event.Object", "StateChanged").unwrap().build(&("focused", 1i32, 0i32, Value::from(0i32), HashMap::<String, Value>::new())).unwrap();
        assert_eq!(event_kind(&new), Some(("focused".to_owned(), 1)));
        let old = zbus::Message::signal("/org/a11y/atspi/accessible/7", "org.a11y.atspi.Event.Object", "PropertyChange")
            .unwrap()
            .build(&("accessible-name", 0i32, 0i32, Value::from("Inbox"), (":1.5", zbus::zvariant::ObjectPath::try_from("/org/a11y/atspi/accessible/root").unwrap())))
            .unwrap();
        assert_eq!(event_kind(&old), Some(("accessible-name".to_owned(), 0)));
    }
}
