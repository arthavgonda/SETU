use crate::text::{normalize, sentence_case, similarity};
use crate::tools::web;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Pause,
    Play,
    Next,
    Previous,
}

#[derive(Debug, Clone)]
pub struct Player {
    pub key: String,
    pub source: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    pub status: Status,
    pub browser: Option<String>,
    pub muted: bool,
    pub active: bool,
}

impl Player {
    pub fn is_tab(&self) -> bool {
        self.browser.is_some()
    }

    pub fn short_title(&self) -> String {
        let t = self.title.trim();
        let t = t.strip_suffix(" - YouTube").or_else(|| t.strip_suffix(" - YouTube Music")).or_else(|| t.strip_suffix(" - Twitch")).unwrap_or(t);
        let words: Vec<&str> = t.split_whitespace().collect();
        if words.is_empty() {
            return self.source.clone();
        }
        let mut s = words.iter().take(7).copied().collect::<Vec<_>>().join(" ");
        if words.len() > 7 {
            s.push('…');
        }
        s
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Now {
    pub key: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: String,
    pub playing: bool,
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    pub rate: f64,
    pub can_next: bool,
    pub can_prev: bool,
}

pub async fn now_playing(cache: &std::path::Path) -> Vec<Now> {
    os::detail(cache).await
}

fn layer() -> (crate::layers::Layer, &'static str, &'static str) {
    if cfg!(target_os = "macos") { (crate::layers::Layer::Command, "command.media", "command.media.list") } else { (crate::layers::Layer::Channel, "channel.media", "channel.media.list") }
}

pub async fn control_key(key: &str, action: Action) -> bool {
    if let Some(ok) = stream_control(key, action).await {
        return ok;
    }
    if !key.starts_with("os:") {
        return false;
    }
    let (l, stage, _) = layer();
    let span = crate::layers::effect(l, stage);
    span.end_bool(os::control(key, action).await)
}

pub struct Listing {
    pub players: Vec<Player>,
    pub known: bool,
}

static PAUSED: Mutex<Vec<(String, i64)>> = Mutex::new(Vec::new());

pub fn remember_paused(keys: &[String]) {
    let now = setu_common::protocol::now_ms();
    let mut g = PAUSED.lock().unwrap();
    g.retain(|(k, t)| now - *t < 2 * 3_600_000 && !keys.contains(k));
    for k in keys {
        g.push((k.clone(), now));
    }
}

pub fn recently_paused() -> Vec<String> {
    let now = setu_common::protocol::now_ms();
    let g = PAUSED.lock().unwrap();
    g.iter().filter(|(_, t)| now - *t < 2 * 3_600_000).map(|(k, _)| k.clone()).collect()
}

pub fn forget_paused(keys: &[String]) {
    PAUSED.lock().unwrap().retain(|(k, _)| !keys.contains(k));
}

const HOSTS: &[(&str, &str)] = &[
    ("music.youtube.com", "YouTube Music"),
    ("youtube.com", "YouTube"),
    ("youtu.be", "YouTube"),
    ("youtube-nocookie.com", "YouTube"),
    ("twitch.tv", "Twitch"),
    ("open.spotify.com", "Spotify"),
    ("soundcloud.com", "SoundCloud"),
    ("netflix.com", "Netflix"),
    ("primevideo.com", "Prime Video"),
    ("hotstar.com", "JioHotstar"),
    ("jiosaavn.com", "JioSaavn"),
    ("gaana.com", "Gaana"),
    ("wynk.in", "Wynk Music"),
    ("vimeo.com", "Vimeo"),
    ("dailymotion.com", "Dailymotion"),
    ("instagram.com", "Instagram"),
    ("facebook.com", "Facebook"),
    ("x.com", "X"),
    ("twitter.com", "X"),
    ("udemy.com", "Udemy"),
    ("coursera.org", "Coursera"),
    ("music.apple.com", "Apple Music"),
    ("tv.apple.com", "Apple TV"),
    ("disneyplus.com", "Disney Plus"),
    ("zee5.com", "ZEE5"),
    ("sonyliv.com", "SonyLIV"),
    ("kick.com", "Kick"),
    ("audible.in", "Audible"),
    ("audible.com", "Audible"),
    ("bandcamp.com", "Bandcamp"),
    ("deezer.com", "Deezer"),
    ("tidal.com", "Tidal"),
    ("meet.google.com", "Google Meet"),
    ("zoom.us", "Zoom"),
    ("reddit.com", "Reddit"),
    ("linkedin.com", "LinkedIn"),
    ("tiktok.com", "TikTok"),
    ("mxplayer.in", "MX Player"),
    ("jiocinema.com", "JioCinema"),
];

pub fn known_source(spoken: &str) -> bool {
    let t = normalize(spoken).replace(' ', "");
    !t.is_empty() && HOSTS.iter().any(|(_, n)| normalize(n).replace(' ', "") == t)
}

#[cfg_attr(windows, allow(dead_code))]
pub fn site_name(url: &str) -> String {
    let lower = url.to_lowercase();
    if lower.starts_with("file:") {
        return "a local file".into();
    }
    let host = web::host(&lower);
    let host = host.split(':').next().unwrap_or("").trim_end_matches('.');
    for (h, n) in HOSTS {
        if host == *h || host.ends_with(&format!(".{h}")) {
            return (*n).to_owned();
        }
    }
    let parts: Vec<&str> = host.split('.').filter(|p| !p.is_empty()).collect();
    let main = match parts.len() {
        0 => return "a web page".into(),
        1 => parts[0],
        n if n >= 3 && ["co", "com", "org", "net", "gov", "ac", "edu"].contains(&parts[n - 2]) => parts[n - 3],
        n => parts[n - 2],
    };
    sentence_case(main)
}

pub fn browser_display(key: &str) -> String {
    match key {
        "brave" => "Brave",
        "chrome" => "Chrome",
        "edge" => "Edge",
        "firefox" => "Firefox",
        "opera" => "Opera",
        "vivaldi" => "Vivaldi",
        "chromium" => "Chromium",
        "librewolf" => "LibreWolf",
        "zen" => "Zen",
        "safari" => "Safari",
        other => return sentence_case(other),
    }
    .to_owned()
}

pub fn browser_of(name: &str) -> Option<String> {
    let n = normalize(name);
    if let Some(k) = web::browser_key(&n) {
        return Some(k.to_owned());
    }
    for (k, _) in web::BROWSERS {
        if n.split_whitespace().any(|w| w == *k) || n.contains(&format!("{k}.")) {
            return Some((*k).to_owned());
        }
    }
    if n.contains("google chrome") {
        return Some("chrome".into());
    }
    if n.contains("msedge") || n.contains("microsoft edge") {
        return Some("edge".into());
    }
    None
}

#[cfg(target_os = "linux")]
fn app_display(identity: &str) -> String {
    let t = identity.trim();
    let t = t.strip_suffix(" media player").or_else(|| t.strip_suffix(" Media Player")).unwrap_or(t);
    let t = t.strip_prefix("Mozilla ").unwrap_or(t);
    if t.is_empty() { "a player".into() } else { t.to_owned() }
}

pub async fn warm() {
    #[cfg(target_os = "linux")]
    os::warm().await;
}

pub async fn watch(kick: fn()) {
    crate::tools::streams::on_change(kick);
    #[cfg(target_os = "linux")]
    os::watch(kick).await;
}

pub async fn seek(key: &str, seconds: i64) -> bool {
    let (l, stage, _) = layer();
    let span = crate::layers::effect(l, stage);
    #[cfg(any(target_os = "linux", windows))]
    let ok = os::seek(key, seconds).await;
    #[cfg(target_os = "macos")]
    let ok = {
        let _ = (key, seconds);
        false
    };
    span.end_bool(ok)
}

pub fn evented() -> bool {
    cfg!(target_os = "linux")
}

pub async fn list() -> Listing {
    let (l, _, stage) = layer();
    let span = crate::layers::read(l, stage);
    let (players, known) = tokio::time::timeout(Duration::from_millis(1500), os::players()).await.unwrap_or((Vec::new(), false));
    span.end_bool(known);
    #[cfg(not(target_os = "linux"))]
    let players = {
        let mut players = players;
        merge_streams(&mut players).await;
        players
    };
    Listing { players, known }
}

#[cfg(not(target_os = "linux"))]
async fn merge_streams(players: &mut Vec<Player>) {
    let Ok(streams) = crate::tools::streams::list().await else { return };
    for st in streams {
        if (st.corked || st.sounding == Some(false)) && !st.muted {
            continue;
        }
        let a = normalize(&st.app);
        let b = normalize(&st.binary);
        let browser = browser_of(&st.app).or_else(|| browser_of(&st.binary));
        let covered = players.iter().any(|p| {
            let pa = normalize(&p.app);
            let ps = normalize(&p.source);
            (!a.is_empty() && (pa == a || ps == a || similarity(&pa, &a) >= 0.8)) || (!b.is_empty() && (pa.replace(' ', "") == b || similarity(&pa, &b) >= 0.8)) || (browser.is_some() && p.browser == browser)
        });
        if covered {
            continue;
        }
        let name = if st.app.is_empty() { st.binary.clone() } else { st.app.clone() };
        players.push(Player { key: format!("stream:{}", st.index), source: name.clone(), app: name, title: st.title.clone(), artist: String::new(), status: if st.muted || st.corked { Status::Paused } else { Status::Playing }, browser, muted: st.muted, active: false });
    }
}

async fn stream_control(key: &str, action: Action) -> Option<bool> {
    let i = key.strip_prefix("stream:")?.parse::<u32>().ok()?;
    Some(match action {
        Action::Pause => crate::tools::streams::set_mute(i, true).await.is_ok(),
        Action::Play => crate::tools::streams::set_mute(i, false).await.is_ok(),
        _ => false,
    })
}

#[cfg_attr(windows, allow(dead_code))]
async fn futures_join<F: std::future::Future>(futs: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let handles: Vec<F> = futs.into_iter().collect();
    let mut out = Vec::with_capacity(handles.len());
    let mut pinned: Vec<std::pin::Pin<Box<F>>> = handles.into_iter().map(Box::pin).collect();
    let mut done: Vec<Option<F::Output>> = (0..pinned.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut all = true;
        for (i, f) in pinned.iter_mut().enumerate() {
            if done[i].is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => done[i] = Some(v),
                    std::task::Poll::Pending => all = false,
                }
            }
        }
        if all { std::task::Poll::Ready(()) } else { std::task::Poll::Pending }
    })
    .await;
    for v in done.into_iter().flatten() {
        out.push(v);
    }
    out
}

pub async fn control(players: &[Player], action: Action) -> usize {
    let mut n = 0;
    let (l, stage, _) = layer();
    let mut ordered: Vec<&Player> = players.iter().collect();
    ordered.sort_by_key(|p| !p.key.starts_with("stream:"));
    for p in ordered {
        if let Some(ok) = stream_control(&p.key, action).await {
            if ok {
                n += 1;
            }
            continue;
        }
        let span = crate::layers::effect(l, stage);
        if span.end_bool(os::control(&p.key, action).await) {
            n += 1;
        }
    }
    n
}

pub fn score(p: &Player, target: &str) -> f64 {
    let t = normalize(target);
    let t = t.trim();
    if t.is_empty() {
        return 0.0;
    }
    let source = normalize(&p.source);
    let app = normalize(&p.app);
    let mut s: f64 = 0.0;
    if source == t || source.replace(' ', "") == t.replace(' ', "") {
        s = s.max(1.0);
    }
    if app == t {
        s = s.max(0.95);
    }
    if let Some(site) = web::site(t)
        && normalize(site.name) == source
    {
        s = s.max(1.0);
    }
    if similarity(&source, t) >= 0.85 {
        s = s.max(0.85);
    }
    if similarity(&app, t) >= 0.85 {
        s = s.max(0.8);
    }
    if let (Some(b), Some(k)) = (p.browser.as_deref(), web::browser_key(t))
        && b == k
    {
        s = s.max(0.75);
    }
    let title = normalize(&p.title);
    let words: Vec<&str> = t.split_whitespace().filter(|w| w.len() > 2 && !matches!(*w, "the" | "and" | "one" | "tab" | "video" | "song")).collect();
    if !words.is_empty() && words.iter().all(|w| title.split_whitespace().any(|x| x == *w)) {
        s = s.max(if words.len() >= 2 { 0.7 } else { 0.6 });
    }
    s
}

pub fn select<'a>(players: &'a [Player], target: &str) -> Vec<&'a Player> {
    let scored: Vec<(f64, &Player)> = players.iter().map(|p| (score(p, target), p)).filter(|(s, _)| *s >= 0.6).collect();
    let best = scored.iter().map(|(s, _)| *s).fold(0.0, f64::max);
    scored.into_iter().filter(|(s, _)| (*s - best).abs() < 1e-9).map(|(_, p)| p).collect()
}

pub struct Group {
    pub source: String,
    pub players: Vec<Player>,
}

impl Group {
    pub fn describe(&self) -> String {
        let n = self.players.len();
        if n <= 1 {
            return self.source.clone();
        }
        let noun = if self.players.iter().all(Player::is_tab) { "tabs" } else { "players" };
        format!("a bunch of {} {noun}", self.source)
    }
}

pub fn groups(players: &[Player]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for p in players {
        match out.iter_mut().find(|g| g.source == p.source) {
            Some(g) => g.players.push(p.clone()),
            None => out.push(Group { source: p.source.clone(), players: vec![p.clone()] }),
        }
    }
    out
}

pub fn count_word(n: usize) -> String {
    match n {
        2 => "two".into(),
        3 => "three".into(),
        4 => "four".into(),
        5 => "five".into(),
        6 => "six".into(),
        7 => "seven".into(),
        8 => "eight".into(),
        9 => "nine".into(),
        n => n.to_string(),
    }
}

pub fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

#[cfg(target_os = "linux")]
mod os {
    use super::{Action, Player, Status, app_display, browser_of, site_name};
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::sync::OnceCell;
    use zbus::zvariant::{OwnedValue, Value as Z};

    static BUS: OnceCell<zbus::Connection> = OnceCell::const_new();

    async fn bus() -> Option<&'static zbus::Connection> {
        BUS.get_or_try_init(|| async { zbus::Connection::session().await }).await.ok()
    }

    fn text(v: &Z) -> Option<String> {
        match v {
            Z::Str(s) => Some(s.as_str().to_owned()),
            Z::ObjectPath(p) => Some(p.as_str().to_owned()),
            Z::Value(b) => text(b),
            Z::Array(a) => {
                let parts: Vec<String> = a.iter().filter_map(text).filter(|s| !s.is_empty()).collect();
                (!parts.is_empty()).then(|| parts.join(", "))
            }
            _ => None,
        }
    }

    fn field(v: &Z, key: &str) -> Option<String> {
        match v {
            Z::Value(b) => field(b, key),
            Z::Dict(d) => d.iter().find(|(k, _)| matches!(k, Z::Str(s) if s.as_str() == key)).and_then(|(_, v)| text(v)),
            _ => None,
        }
    }

    async fn get_all(c: &zbus::Connection, name: &str, iface: &str) -> Option<HashMap<String, OwnedValue>> {
        let r = c.call_method(Some(name), "/org/mpris/MediaPlayer2", Some("org.freedesktop.DBus.Properties"), "GetAll", &(iface,)).await.ok()?;
        r.body().deserialize().ok()
    }

    async fn one(c: &zbus::Connection, name: String) -> Option<Player> {
        let (root, pl) = tokio::join!(get_all(c, &name, "org.mpris.MediaPlayer2"), get_all(c, &name, "org.mpris.MediaPlayer2.Player"));
        let pl = pl?;
        let status = match pl.get("PlaybackStatus").and_then(|v| text(v)).as_deref() {
            Some("Playing") => Status::Playing,
            Some("Paused") => Status::Paused,
            _ => Status::Stopped,
        };
        let meta = pl.get("Metadata");
        let title = meta.and_then(|m| field(m, "xesam:title")).unwrap_or_default();
        let artist = meta.and_then(|m| field(m, "xesam:artist")).unwrap_or_default();
        let url = meta.and_then(|m| field(m, "xesam:url")).unwrap_or_default();
        let short = name.trim_start_matches("org.mpris.MediaPlayer2.").to_owned();
        let identity = root.as_ref().and_then(|r| r.get("Identity")).and_then(|v| text(v)).unwrap_or_else(|| short.split('.').next().unwrap_or("").to_owned());
        let desktop = root.as_ref().and_then(|r| r.get("DesktopEntry")).and_then(|v| text(v)).unwrap_or_default();
        let browser = browser_of(&identity).or_else(|| browser_of(&short)).or_else(|| browser_of(&desktop));
        let app = match &browser {
            Some(b) => super::browser_display(b),
            None => app_display(&identity),
        };
        let source = if url.starts_with("http") { site_name(&url) } else { app.clone() };
        Some(Player { key: format!("os:{name}"), source, app, title, artist, status, browser, muted: false, active: false })
    }

    fn num(v: &Z, key: &str) -> Option<i64> {
        match v {
            Z::Value(b) => num(b, key),
            Z::Dict(d) => d.iter().find(|(k, _)| matches!(k, Z::Str(s) if s.as_str() == key)).and_then(|(_, v)| int(v)),
            _ => None,
        }
    }

    fn int(v: &Z) -> Option<i64> {
        match v {
            Z::I64(n) => Some(*n),
            Z::U64(n) => Some(*n as i64),
            Z::I32(n) => Some(*n as i64),
            Z::U32(n) => Some(*n as i64),
            Z::F64(f) => Some(*f as i64),
            Z::Value(b) => int(b),
            _ => None,
        }
    }

    fn flag(v: Option<&OwnedValue>) -> bool {
        v.and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
    }

    async fn detail_one(c: &zbus::Connection, name: String) -> Option<super::Now> {
        let (root, pl) = tokio::join!(get_all(c, &name, "org.mpris.MediaPlayer2"), get_all(c, &name, "org.mpris.MediaPlayer2.Player"));
        let pl = pl?;
        let status = pl.get("PlaybackStatus").and_then(|v| text(v)).unwrap_or_default();
        if status != "Playing" && status != "Paused" {
            return None;
        }
        let meta = pl.get("Metadata");
        let title = meta.and_then(|m| field(m, "xesam:title")).unwrap_or_default();
        if title.trim().is_empty() {
            return None;
        }
        let short = name.trim_start_matches("org.mpris.MediaPlayer2.").to_owned();
        let identity = root.as_ref().and_then(|r| r.get("Identity")).and_then(|v| text(v)).unwrap_or_else(|| short.split('.').next().unwrap_or("").to_owned());
        let browser = browser_of(&identity).or_else(|| browser_of(&short));
        let url = meta.and_then(|m| field(m, "xesam:url")).unwrap_or_default();
        let app = match &browser {
            Some(_) if url.starts_with("http") => site_name(&url),
            Some(b) => super::browser_display(b),
            None => app_display(&identity),
        };
        let position = pl.get("Position").and_then(|v| int(v)).filter(|p| *p >= 0).map(|p| (p / 1000) as u64);
        let duration = meta.and_then(|m| num(m, "mpris:length")).filter(|l| *l > 0).map(|l| (l / 1000) as u64);
        let rate = pl.get("Rate").and_then(|v| f64::try_from(v).ok()).unwrap_or(1.0);
        Some(super::Now {
            key: format!("os:{name}"),
            app,
            title,
            artist: meta.and_then(|m| field(m, "xesam:artist")).unwrap_or_default(),
            album: meta.and_then(|m| field(m, "xesam:album")).unwrap_or_default(),
            art: meta.and_then(|m| field(m, "mpris:artUrl")).unwrap_or_default(),
            playing: status == "Playing",
            position_ms: position,
            duration_ms: duration,
            rate: if rate > 0.0 { rate } else { 1.0 },
            can_next: flag(pl.get("CanGoNext")),
            can_prev: flag(pl.get("CanGoPrevious")),
        })
    }

    pub async fn detail(_cache: &std::path::Path) -> Vec<super::Now> {
        let Some(c) = bus().await else { return Vec::new() };
        let names: Vec<String> = match c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &()).await {
            Ok(r) => r.body().deserialize().unwrap_or_default(),
            Err(_) => return Vec::new(),
        };
        let wanted: Vec<String> = names.into_iter().filter(|n| n.starts_with("org.mpris.MediaPlayer2.") && !n.contains("playerctld") && !n.contains("kdeconnect")).collect();
        let futs = wanted.into_iter().map(|n| async move { tokio::time::timeout(Duration::from_millis(600), detail_one(c, n)).await.ok().flatten() });
        super::futures_join(futs).await.into_iter().flatten().collect()
    }

    pub async fn seek(key: &str, seconds: i64) -> bool {
        let Some(name) = key.strip_prefix("os:") else { return false };
        let Some(c) = bus().await else { return false };
        let us: i64 = seconds * 1_000_000;
        let ok = tokio::time::timeout(Duration::from_millis(800), c.call_method(Some(name), "/org/mpris/MediaPlayer2", Some("org.mpris.MediaPlayer2.Player"), "Seek", &(us,))).await.map(|r| r.is_ok()).unwrap_or(false);
        mpris_changed();
        ok
    }

    static MPRIS: std::sync::Mutex<Option<(u64, std::time::Instant, Vec<(Player, u32)>)>> = std::sync::Mutex::new(None);
    static MPRIS_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static MPRIS_WATCHED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn mpris_changed() {
        MPRIS_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn mpris_players(c: &'static zbus::Connection) -> Option<Vec<(Player, u32)>> {
        use std::sync::atomic::Ordering::SeqCst;
        let watched = MPRIS_WATCHED.load(SeqCst) == 3;
        let before = MPRIS_EPOCH.load(SeqCst);
        if watched && let Some((e, at, v)) = MPRIS.lock().unwrap().as_ref() && *e == before && at.elapsed() < Duration::from_secs(10) {
            return Some(v.clone());
        }
        let names: Vec<String> = c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &()).await.ok()?.body().deserialize().unwrap_or_default();
        let wanted: Vec<String> = names.into_iter().filter(|n| n.starts_with("org.mpris.MediaPlayer2.") && !n.contains("playerctld") && !n.contains("kdeconnect")).collect();
        let futs = wanted.into_iter().map(|n| async move {
            let pid = async {
                c.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "GetConnectionUnixProcessID", &(n.as_str(),)).await.ok().and_then(|r| r.body().deserialize::<u32>().ok()).unwrap_or(0)
            };
            let (p, pid) = tokio::join!(tokio::time::timeout(Duration::from_millis(600), one(c, n.clone())), pid);
            p.ok().flatten().map(|p| (p, pid))
        });
        let v: Vec<(Player, u32)> = super::futures_join(futs).await.into_iter().flatten().collect();
        if watched && MPRIS_EPOCH.load(SeqCst) == before {
            *MPRIS.lock().unwrap() = Some((before, std::time::Instant::now(), v.clone()));
        }
        Some(v)
    }

    pub async fn warm() {
        if let Some(c) = bus().await {
            let _ = mpris_players(c).await;
        }
    }

    pub async fn watch(kick: fn()) {
        use futures_util::StreamExt;
        let Some(c) = bus().await else { return };
        let rules = [
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|b| b.member("PropertiesChanged")).and_then(|b| b.path("/org/mpris/MediaPlayer2")).map(|b| b.build()),
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.mpris.MediaPlayer2.Player").and_then(|b| b.member("Seeked")).map(|b| b.build()),
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|b| b.member("NameOwnerChanged")).and_then(|b| b.arg0ns("org.mpris.MediaPlayer2")).map(|b| b.build()),
        ];
        for r in rules.into_iter().flatten() {
            if let Ok(mut st) = zbus::MessageStream::for_match_rule(r, c, Some(32)).await {
                MPRIS_WATCHED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    while st.next().await.is_some() {
                        mpris_changed();
                        kick();
                    }
                    MPRIS_WATCHED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    mpris_changed();
                });
            }
        }
        mpris_changed();
    }

    pub async fn players() -> (Vec<Player>, bool) {
        let Some(c) = bus().await else { return (Vec::new(), false) };
        let (found, streams) = tokio::join!(mpris_players(c), crate::tools::streams::list());
        let Some(found) = found else { return (Vec::new(), false) };
        let pids: Vec<u32> = found.iter().map(|(_, pid)| *pid).filter(|p| *p != 0).collect();
        let mut out: Vec<Player> = found.into_iter().map(|(p, _)| p).collect();
        if let Ok(streams) = streams {
            let browsers: Vec<String> = out.iter().filter_map(|p| p.browser.clone()).collect();
            for st in streams {
                if (st.corked || st.sounding == Some(false)) && !st.muted {
                    continue;
                }
                let browser = super::browser_of(&st.app).or_else(|| super::browser_of(&st.binary));
                if st.pid.is_some_and(|p| pids.contains(&p)) || browser.as_ref().is_some_and(|b| browsers.contains(b)) {
                    continue;
                }
                let name = if st.app.is_empty() { st.binary.clone() } else { st.app.clone() };
                out.push(Player {
                    key: format!("stream:{}", st.index),
                    source: name.clone(),
                    app: name,
                    title: st.title.clone(),
                    artist: String::new(),
                    status: if st.muted || st.corked { Status::Paused } else { Status::Playing },
                    browser,
                    muted: st.muted,
                    active: false,
                });
            }
        }
        (out, true)
    }

    pub async fn control(key: &str, action: Action) -> bool {
        if let Some(i) = key.strip_prefix("stream:").and_then(|i| i.parse::<u32>().ok()) {
            return match action {
                Action::Pause => crate::tools::streams::set_mute(i, true).await.is_ok(),
                Action::Play => crate::tools::streams::set_mute(i, false).await.is_ok(),
                _ => false,
            };
        }
        let Some(name) = key.strip_prefix("os:") else { return false };
        let method = match action {
            Action::Pause => "Pause",
            Action::Play => "Play",
            Action::Next => "Next",
            Action::Previous => "Previous",
        };
        let Some(c) = bus().await else { return false };
        mpris_changed();
        let ok = tokio::time::timeout(Duration::from_millis(800), c.call_method(Some(name), "/org/mpris/MediaPlayer2", Some("org.mpris.MediaPlayer2.Player"), method, &())).await.map(|r| r.is_ok()).unwrap_or(false);
        mpris_changed();
        ok
    }
}

#[cfg(windows)]
mod os {
    use super::{Action, Player, Status, browser_of};
    use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as PS};

    fn name_of(aumid: &str) -> String {
        let a = aumid.to_lowercase();
        let table: &[(&str, &str)] = &[
            ("spotify", "Spotify"),
            ("chrome", "Chrome"),
            ("msedge", "Edge"),
            ("brave", "Brave"),
            ("firefox", "Firefox"),
            ("308046b0af4a39cb", "Firefox"),
            ("opera", "Opera"),
            ("vivaldi", "Vivaldi"),
            ("vlc", "VLC"),
            ("microsoft.zunemusic", "Media Player"),
            ("microsoft.zunevideo", "Films and TV"),
            ("applemusic", "Apple Music"),
            ("amazonmusic", "Amazon Music"),
            ("deezer", "Deezer"),
            ("discord", "Discord"),
            ("whatsapp", "WhatsApp"),
            ("teams", "Teams"),
            ("itunes", "iTunes"),
            ("foobar", "foobar2000"),
            ("aimp", "AIMP"),
            ("musicbee", "MusicBee"),
            ("potplayer", "PotPlayer"),
        ];
        for (k, v) in table {
            if a.contains(k) {
                return (*v).to_owned();
            }
        }
        if a.len() == 16 && a.chars().all(|c| c.is_ascii_hexdigit()) {
            return "Firefox".into();
        }
        let tail = aumid.rsplit('!').next().unwrap_or(aumid);
        let tail = tail.trim_end_matches(".exe").trim_end_matches(".EXE");
        let tail = tail.rsplit(['\\', '/']).next().unwrap_or(tail);
        if tail.is_empty() { "a player".into() } else { crate::text::sentence_case(tail) }
    }

    fn sessions() -> Option<Vec<Session>> {
        let mgr = Manager::RequestAsync().ok()?.get().ok()?;
        let list = mgr.GetSessions().ok()?;
        let n = list.Size().unwrap_or(0);
        Some((0..n).filter_map(|i| list.GetAt(i).ok()).collect())
    }

    fn read() -> (Vec<Player>, bool) {
        let Some(all) = sessions() else { return (Vec::new(), false) };
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut out = Vec::new();
        for s in all {
            let aumid = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
            let idx = seen.entry(aumid.clone()).or_insert(0);
            let key = format!("os:{aumid}#{idx}");
            *idx += 1;
            let status = match s.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()) {
                Ok(st) if st == PS::Playing => Status::Playing,
                Ok(st) if st == PS::Paused => Status::Paused,
                _ => Status::Stopped,
            };
            let (title, artist) = match s.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
                Ok(p) => (p.Title().map(|h| h.to_string()).unwrap_or_default(), p.Artist().map(|h| h.to_string()).unwrap_or_default()),
                Err(_) => (String::new(), String::new()),
            };
            let app = name_of(&aumid);
            let browser = browser_of(&app);
            out.push(Player { key, source: app.clone(), app, title, artist, status, browser, muted: false, active: false });
        }
        (out, true)
    }

    pub async fn players() -> (Vec<Player>, bool) {
        crate::layers::blocking(read).await.unwrap_or((Vec::new(), false))
    }

    fn thumbnail(props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties, dir: &std::path::Path, id: &str) -> String {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut h);
        let path = dir.join(format!("{:016x}.img", h.finish()));
        if path.is_file() {
            return path.to_string_lossy().into_owned();
        }
        let read = || -> windows::core::Result<Vec<u8>> {
            let stream = props.Thumbnail()?.OpenReadAsync()?.get()?;
            let size = stream.Size()? as u32;
            if size == 0 || size > 8 * 1024 * 1024 {
                return Ok(Vec::new());
            }
            let reader = windows::Storage::Streams::DataReader::CreateDataReader(&stream)?;
            reader.LoadAsync(size)?.get()?;
            let mut buf = vec![0u8; size as usize];
            reader.ReadBytes(&mut buf)?;
            Ok(buf)
        };
        match read() {
            Ok(bytes) if !bytes.is_empty() => {
                let _ = std::fs::create_dir_all(dir);
                if std::fs::write(&path, bytes).is_ok() { path.to_string_lossy().into_owned() } else { String::new() }
            }
            _ => String::new(),
        }
    }

    fn ticks() -> i64 {
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64 / 100).unwrap_or(0);
        unix + 116_444_736_000_000_000
    }

    fn details(cache: std::path::PathBuf) -> Vec<super::Now> {
        let Some(all) = sessions() else { return Vec::new() };
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut out = Vec::new();
        for s in all {
            let aumid = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
            let idx = seen.entry(aumid.clone()).or_insert(0);
            let key = format!("os:{aumid}#{idx}");
            *idx += 1;
            let info = s.GetPlaybackInfo().ok();
            let status = info.as_ref().and_then(|p| p.PlaybackStatus().ok());
            let playing = status == Some(PS::Playing);
            if !playing && status != Some(PS::Paused) {
                continue;
            }
            let Ok(props) = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()) else { continue };
            let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
            if title.trim().is_empty() {
                continue;
            }
            let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
            let album = props.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
            let art = thumbnail(&props, &cache, &format!("{aumid}|{title}|{artist}"));
            let rate = info.as_ref().and_then(|p| p.PlaybackRate().ok()).and_then(|r| r.Value().ok()).filter(|r| *r > 0.0).unwrap_or(1.0);
            let controls = info.as_ref().and_then(|p| p.Controls().ok());
            let (mut position, mut duration) = (None, None);
            if let Ok(t) = s.GetTimelineProperties() {
                let end = t.EndTime().map(|d| d.Duration).unwrap_or(0) - t.StartTime().map(|d| d.Duration).unwrap_or(0);
                if end > 0 {
                    duration = Some((end / 10_000) as u64);
                }
                if let Ok(p) = t.Position() {
                    let mut pos = p.Duration;
                    if playing && let Ok(updated) = t.LastUpdatedTime() {
                        let since = ticks() - updated.UniversalTime;
                        if since > 0 && since < 36_000_000_000 {
                            pos += (since as f64 * rate) as i64;
                        }
                    }
                    if pos >= 0 {
                        position = Some((pos / 10_000) as u64);
                    }
                }
            }
            let app = name_of(&aumid);
            out.push(super::Now {
                key,
                app,
                title,
                artist,
                album,
                art,
                playing,
                position_ms: position,
                duration_ms: duration,
                rate,
                can_next: controls.as_ref().and_then(|c| c.IsNextEnabled().ok()).unwrap_or(false),
                can_prev: controls.as_ref().and_then(|c| c.IsPreviousEnabled().ok()).unwrap_or(false),
            });
        }
        out
    }

    pub async fn detail(cache: &std::path::Path) -> Vec<super::Now> {
        let c = cache.to_path_buf();
        crate::layers::blocking(move || details(c)).await.unwrap_or_default()
    }

    fn session_of(key: &str) -> Option<Session> {
        let rest = key.strip_prefix("os:")?;
        let (aumid, idx) = match rest.rsplit_once('#') {
            Some((a, i)) => (a.to_owned(), i.parse::<usize>().unwrap_or(0)),
            None => (rest.to_owned(), 0),
        };
        sessions()?.into_iter().filter(|s| s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default() == aumid).nth(idx)
    }

    fn seek_now(key: String, seconds: i64) -> bool {
        let Some(s) = session_of(&key) else { return false };
        let Ok(t) = s.GetTimelineProperties() else { return false };
        let mut pos = t.Position().map(|p| p.Duration).unwrap_or(0);
        let playing = s.GetPlaybackInfo().and_then(|p| p.PlaybackStatus()).map(|st| st == PS::Playing).unwrap_or(false);
        if playing && let Ok(u) = t.LastUpdatedTime() {
            let since = ticks() - u.UniversalTime;
            if since > 0 && since < 36_000_000_000 {
                pos += since;
            }
        }
        let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
        let mut want = (pos + seconds * 10_000_000).max(0);
        if end > 0 {
            want = want.min(end);
        }
        s.TryChangePlaybackPositionAsync(want).and_then(|o| o.get()).unwrap_or(false)
    }

    pub async fn seek(key: &str, seconds: i64) -> bool {
        let k = key.to_owned();
        crate::layers::blocking(move || seek_now(k, seconds)).await.unwrap_or(false)
    }

    fn act(key: String, action: Action) -> bool {
        let Some(s) = session_of(&key) else { return false };
        let op = match action {
            Action::Pause => s.TryPauseAsync(),
            Action::Play => s.TryPlayAsync(),
            Action::Next => s.TrySkipNextAsync(),
            Action::Previous => s.TrySkipPreviousAsync(),
        };
        op.and_then(|o| o.get()).unwrap_or(false)
    }

    pub async fn control(key: &str, action: Action) -> bool {
        let k = key.to_owned();
        crate::layers::blocking(move || act(k, action)).await.unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::{Action, Player, Status};

    const APPS: &[&str] = &["Spotify", "Music"];

    fn script(s: &str) -> Option<String> {
        let out = std::process::Command::new("osascript").arg("-e").arg(s).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn read() -> (Vec<Player>, bool) {
        let mut out = Vec::new();
        for app in APPS {
            if script(&format!("application \"{app}\" is running")).as_deref() != Some("true") {
                continue;
            }
            let Some(state) = script(&format!("tell application \"{app}\" to player state as string")) else { continue };
            let title = script(&format!("tell application \"{app}\" to name of current track")).unwrap_or_default();
            let artist = script(&format!("tell application \"{app}\" to artist of current track")).unwrap_or_default();
            let status = match state.as_str() {
                "playing" => Status::Playing,
                "paused" => Status::Paused,
                _ => Status::Stopped,
            };
            let name: &str = if *app == "Music" { "Apple Music" } else { *app };
            out.push(Player { key: format!("os:{app}"), source: name.into(), app: name.into(), title, artist, status, browser: None, muted: false, active: false });
        }
        (out, true)
    }

    pub async fn players() -> (Vec<Player>, bool) {
        crate::layers::blocking(read).await.unwrap_or((Vec::new(), false))
    }

    fn running(app: &str) -> bool {
        std::process::Command::new("pgrep").args(["-xq", app]).status().map(|s| s.success()).unwrap_or(false)
    }

    fn details() -> Vec<super::Now> {
        let mut out = Vec::new();
        for app in APPS {
            if !running(app) {
                continue;
            }
            let art = if *app == "Spotify" { " & (artwork url of current track)" } else { " & \"\"" };
            let length = if *app == "Spotify" { "((duration of current track) / 1000)" } else { "(duration of current track)" };
            let sep = "(ASCII character 31)";
            let q = format!(
                "tell application \"{app}\" to (player state as string) & {sep} & (name of current track) & {sep} & (artist of current track) & {sep} & (album of current track) & {sep} & (player position as string) & {sep} & ({length} as string) & {sep}{art}"
            );
            let Some(r) = script(&q) else { continue };
            let f: Vec<&str> = r.split('\u{1f}').collect();
            if f.len() < 7 || f[1].trim().is_empty() || !(f[0] == "playing" || f[0] == "paused") {
                continue;
            }
            let secs = |x: &str| x.trim().replace(',', ".").parse::<f64>().ok().filter(|v| *v >= 0.0).map(|v| (v * 1000.0) as u64);
            let name: &str = if *app == "Music" { "Apple Music" } else { app };
            out.push(super::Now {
                key: format!("os:{app}"),
                app: name.into(),
                title: f[1].trim().into(),
                artist: f[2].trim().into(),
                album: f[3].trim().into(),
                art: f[6].trim().into(),
                playing: f[0] == "playing",
                position_ms: secs(f[4]),
                duration_ms: secs(f[5]),
                rate: 1.0,
                can_next: true,
                can_prev: true,
            });
        }
        out
    }

    pub async fn detail(_cache: &std::path::Path) -> Vec<super::Now> {
        crate::layers::blocking(details).await.unwrap_or_default()
    }

    pub async fn control(key: &str, action: Action) -> bool {
        let Some(app) = key.strip_prefix("os:").map(str::to_owned) else { return false };
        let verb = match action {
            Action::Pause => "pause",
            Action::Play => "play",
            Action::Next => "next track",
            Action::Previous => "previous track",
        };
        crate::layers::blocking(move || script(&format!("tell application \"{app}\" to {verb}")).is_some()).await.unwrap_or(false)
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod os {
    use super::{Action, Player};

    pub async fn players() -> (Vec<Player>, bool) {
        (Vec::new(), false)
    }

    pub async fn detail(_cache: &std::path::Path) -> Vec<super::Now> {
        Vec::new()
    }

    pub async fn control(_key: &str, _action: Action) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: i64, url: &str, title: &str, status: Status) -> Player {
        Player { key: format!("os:brave.instance{id}"), source: site_name(url), app: "Brave".into(), title: title.into(), artist: String::new(), status, browser: Some("brave".into()), muted: false, active: false }
    }

    fn app(name: &str, status: Status) -> Player {
        Player { key: format!("os:{name}"), source: name.into(), app: name.into(), title: "Song".into(), artist: String::new(), status, browser: None, muted: false, active: false }
    }

    #[test]
    fn names_sites() {
        assert_eq!(site_name("https://www.youtube.com/watch?v=abc"), "YouTube");
        assert_eq!(site_name("https://m.youtube.com/watch?v=abc"), "YouTube");
        assert_eq!(site_name("https://music.youtube.com/watch?v=abc"), "YouTube Music");
        assert_eq!(site_name("https://www.twitch.tv/somebody"), "Twitch");
        assert_eq!(site_name("https://example.co.in/a"), "Example");
        assert_eq!(site_name("http://localhost:8000/x"), "Localhost");
        assert_eq!(site_name("file:///home/a.mp4"), "a local file");
    }

    #[test]
    fn selects_and_groups() {
        let ps = vec![
            tab(1, "https://www.youtube.com/watch?v=1", "Lofi beats - YouTube", Status::Playing),
            tab(2, "https://www.youtube.com/watch?v=2", "Rust tutorial - YouTube", Status::Playing),
            tab(3, "https://www.twitch.tv/x", "Speedrun - Twitch", Status::Playing),
            app("Spotify", Status::Playing),
        ];
        assert_eq!(select(&ps, "youtube").len(), 2);
        assert_eq!(select(&ps, "you tube").len(), 2);
        assert_eq!(select(&ps, "spotify").len(), 1);
        assert_eq!(select(&ps, "twitch").len(), 1);
        assert_eq!(select(&ps, "rust tutorial").len(), 1);
        assert_eq!(select(&ps, "brave").len(), 3);
        assert!(select(&ps, "netflix").is_empty());
        let g = groups(&ps);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].describe(), "a bunch of YouTube tabs");
        assert_eq!(g[2].describe(), "Spotify");
        assert_eq!(ps[0].short_title(), "Lofi beats");
        assert!(ps[0].is_tab() && !ps[3].is_tab());
        assert_eq!(join_and(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
