use crate::core::Core;
use crate::tools::media::{self, Action, Now};
use serde_json::{Value, json};
use setu_common::protocol::now_ms;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Lyrics = Arc<Vec<(u64, String)>>;

#[derive(Default)]
pub struct State {
    last: Mutex<Option<Snapshot>>,
    lyrics: Mutex<HashMap<String, Option<Lyrics>>>,
    art: Mutex<HashMap<String, String>>,
    kick: tokio::sync::Notify,
}

#[derive(Clone, PartialEq)]
struct Snapshot {
    now: Now,
    art: String,
    at: i64,
    lyrics: Option<Lyrics>,
}

impl State {
    pub fn refresh(&self) {
        self.kick.notify_one();
    }

    pub fn json(&self) -> Value {
        match self.last.lock().unwrap().as_ref() {
            Some(s) => snapshot_json(s),
            None => with_rows(json!({ "none": true })),
        }
    }
}

const LIST_FOR: Duration = Duration::from_secs(20);
static LIST_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static ROWS: std::sync::Mutex<Value> = std::sync::Mutex::new(Value::Null);

pub fn open_list() {
    *LIST_UNTIL.lock().unwrap() = Some(std::time::Instant::now() + LIST_FOR);
    CHANGED.notify_one();
}

fn list_open() -> bool {
    LIST_UNTIL.lock().unwrap().is_some_and(|t| std::time::Instant::now() < t)
}

fn with_rows(mut v: Value) -> Value {
    let open = list_open();
    v["listOpen"] = json!(open);
    v["list"] = if open { ROWS.lock().unwrap().clone() } else { json!([]) };
    v
}

fn row(p: &media::Player) -> Value {
    json!({
        "key": p.key, "app": p.source, "title": p.title, "artist": p.artist,
        "playing": p.status == media::Status::Playing, "muted": p.muted, "stream": p.key.starts_with("stream:"),
    })
}

pub async fn refresh_rows() -> bool {
    let rows = if list_open() {
        let mut ps = media::list().await.players;
        ps.sort_by_key(|p| p.status != media::Status::Playing);
        Value::Array(ps.iter().map(row).collect())
    } else {
        Value::Null
    };
    let mut g = ROWS.lock().unwrap();
    let moved = *g != rows;
    *g = rows;
    moved
}

fn track_key(n: &Now) -> String {
    format!("{}|{}|{}", n.key, n.title, n.artist)
}

fn snapshot_json(s: &Snapshot) -> Value {
    with_rows(json!({
        "key": s.now.key, "app": s.now.app, "title": s.now.title, "artist": s.now.artist, "album": s.now.album,
        "art": s.art, "playing": s.now.playing, "positionMs": s.now.position_ms, "durationMs": s.now.duration_ms,
        "rate": s.now.rate, "at": s.at, "canNext": s.now.can_next, "canPrev": s.now.can_prev,
        "lyrics": s.lyrics.as_ref().map(|l| l.iter().map(|(t, x)| json!([t, x])).collect::<Vec<_>>()),
    }))
}

fn file_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

fn cache_dir(core: &Core) -> PathBuf {
    core.paths.data.join("cache").join("art")
}

fn pick(list: Vec<Now>, previous: Option<&Snapshot>) -> Option<Now> {
    let prev_key = previous.map(|p| p.now.key.clone());
    let mut playing: Vec<Now> = list.iter().filter(|n| n.playing).cloned().collect();
    if !playing.is_empty() {
        if let Some(k) = &prev_key
            && let Some(i) = playing.iter().position(|n| &n.key == k)
        {
            return Some(playing.swap_remove(i));
        }
        return playing.into_iter().next();
    }
    let k = prev_key?;
    list.into_iter().find(|n| n.key == k)
}

fn changed(a: &Snapshot, b: &Snapshot) -> bool {
    if a.now.key != b.now.key || a.now.title != b.now.title || a.now.artist != b.now.artist || a.now.playing != b.now.playing || a.art != b.art || a.lyrics.is_some() != b.lyrics.is_some() || a.now.duration_ms != b.now.duration_ms {
        return true;
    }
    match (a.now.position_ms, b.now.position_ms) {
        (Some(pa), Some(pb)) => {
            let expected = if a.now.playing { pa as f64 + (b.at - a.at) as f64 * a.now.rate } else { pa as f64 };
            (pb as f64 - expected).abs() > 2500.0
        }
        (None, None) => false,
        _ => true,
    }
}

async fn art_for(core: &Core, n: &Now) -> String {
    let src = n.art.trim();
    if src.is_empty() {
        return String::new();
    }
    if let Some(p) = src.strip_prefix("file://") {
        let path = percent_decode(p);
        return if Path::new(&path).is_file() { file_url(Path::new(&path)) } else { String::new() };
    }
    if Path::new(src).is_absolute() && Path::new(src).is_file() {
        return file_url(Path::new(src));
    }
    if !(src.starts_with("https://") || src.starts_with("http://")) {
        return String::new();
    }
    if let Some(hit) = core.nowplaying.art.lock().unwrap().get(src) {
        return hit.clone();
    }
    let dir = cache_dir(core);
    let url = src.to_owned();
    let got = crate::layers::blocking(move || fetch_art(&url, &dir)).await.ok().flatten().map(|p| file_url(&p)).unwrap_or_default();
    let mut g = core.nowplaying.art.lock().unwrap();
    if g.len() > 64 {
        g.clear();
    }
    g.insert(src.to_owned(), got.clone());
    got
}

fn fetch_art(url: &str, dir: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let name: String = Sha256::digest(url.as_bytes()).iter().take(10).map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("{name}.img"));
    if path.is_file() {
        return Some(path);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(4))).timeout_global(Some(Duration::from_secs(8))).build().into();
    let mut r = agent.get(url).call().ok()?;
    let bytes = r.body_mut().with_config().limit(4 * 1024 * 1024).read_to_vec().ok()?;
    std::fs::create_dir_all(dir).ok()?;
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if entries.len() > 200 {
        entries.sort();
        for (_, p) in entries.iter().take(entries.len() - 150) {
            let _ = std::fs::remove_file(p);
        }
    }
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub fn parse_lrc(text: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some((stamp, after)) = r.split_once(']') else { break };
            let Some((m, sec)) = stamp.split_once(':') else { break };
            let (Ok(m), Ok(sec)) = (m.trim().parse::<u64>(), sec.trim().replace(',', ".").parse::<f64>()) else { break };
            stamps.push(m * 60_000 + (sec * 1000.0) as u64);
            rest = after;
        }
        let words = rest.trim();
        for t in stamps {
            out.push((t, words.to_owned()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out.dedup();
    out
}

fn fetch_lyrics(title: &str, artist: &str, album: &str, duration_s: u64) -> Option<Vec<(u64, String)>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(4))).timeout_global(Some(Duration::from_secs(8))).user_agent("Setu voice assistant (https://github.com/setu)").build().into();
    let mut url = format!("https://lrclib.net/api/get?track_name={}&artist_name={}", percent_encode(title), percent_encode(artist));
    if !album.is_empty() {
        url.push_str(&format!("&album_name={}", percent_encode(album)));
    }
    if duration_s > 0 {
        url.push_str(&format!("&duration={duration_s}"));
    }
    let synced = |v: &Value| v.get("syncedLyrics").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(parse_lrc).filter(|l| !l.is_empty());
    if let Ok(mut r) = agent.get(&url).call()
        && let Ok(v) = r.body_mut().read_json::<Value>()
        && let Some(l) = synced(&v)
    {
        return Some(l);
    }
    let search = format!("https://lrclib.net/api/search?track_name={}&artist_name={}", percent_encode(title), percent_encode(artist));
    let mut r = agent.get(&search).call().ok()?;
    let v: Value = r.body_mut().read_json().ok()?;
    v.as_array()?.iter().find_map(synced)
}

fn clean_title(title: &str) -> String {
    let mut t = title.to_owned();
    for cut in [" (Official", " [Official", " (Lyric", " [Lyric", " (Audio", " [Audio", " (Video", " - Topic", " | "] {
        if let Some(i) = t.find(cut) {
            t.truncate(i);
        }
    }
    t.trim().to_owned()
}

fn lyrics_cached(core: &Core, n: &Now) -> Option<Option<Lyrics>> {
    core.nowplaying.lyrics.lock().unwrap().get(&track_key(n)).cloned()
}

fn want_lyrics(core: &Core, n: &Now) -> bool {
    core.settings.bool("media.lyrics") && !n.artist.trim().is_empty() && n.duration_ms.is_some_and(|d| d > 30_000)
}

fn emit(core: &Core, s: &Snapshot) {
    core.hub.emit("media.now", snapshot_json(s));
}

async fn tick(core: &Arc<Core>) {
    let dir = cache_dir(core);
    let (list, rows_moved) = tokio::join!(media::now_playing(&dir), refresh_rows());
    let previous = core.nowplaying.last.lock().unwrap().clone();
    let Some(now) = pick(list, previous.as_ref()) else {
        if previous.is_some() || rows_moved {
            *core.nowplaying.last.lock().unwrap() = None;
            core.hub.emit("media.now", with_rows(json!({ "none": true })));
        }
        return;
    };
    let art = art_for(core, &now).await;
    let lyrics = lyrics_cached(core, &now).flatten();
    let snap = Snapshot { now: now.clone(), art, at: now_ms(), lyrics };
    let emit_it = rows_moved || previous.as_ref().is_none_or(|p| changed(p, &snap));
    if emit_it {
        *core.nowplaying.last.lock().unwrap() = Some(snap.clone());
        emit(core, &snap);
    } else if let Some(p) = core.nowplaying.last.lock().unwrap().as_mut() {
        p.now.playing = snap.now.playing;
    }
    if want_lyrics(core, &now) && lyrics_cached(core, &now).is_none() {
        core.nowplaying.lyrics.lock().unwrap().insert(track_key(&now), None);
        let c = core.clone();
        let n = now.clone();
        tokio::spawn(async move {
            let (t, a, al, d) = (clean_title(&n.title), n.artist.clone(), n.album.clone(), n.duration_ms.unwrap_or(0) / 1000);
            let found = crate::layers::blocking(move || fetch_lyrics(&t, &a, &al, d)).await.ok().flatten().map(Arc::new);
            {
                let mut g = c.nowplaying.lyrics.lock().unwrap();
                if g.len() > 40 {
                    g.clear();
                }
                g.insert(track_key(&n), found.clone());
            }
            let mut last = c.nowplaying.last.lock().unwrap();
            if let (Some(s), Some(l)) = (last.as_mut(), found)
                && track_key(&s.now) == track_key(&n)
            {
                s.lyrics = Some(l);
                emit(&c, s);
            }
        });
    }
}

static CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();

fn stirred() {
    CHANGED.notify_one();
}

pub async fn run(core: Arc<Core>) {
    tokio::spawn(crate::tools::media::watch(stirred));
    let fast = if crate::tools::media::evented() { 15_000 } else if cfg!(target_os = "macos") { 2000 } else { 1000 };
    loop {
        let watching = core.hub.has("media.now");
        let wait = if watching { fast } else { 30_000 };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            _ = core.nowplaying.kick.notified() => {}
            _ = CHANGED.notified() => {}
        }
        if !watching {
            if core.nowplaying.last.lock().unwrap().take().is_some() {
                core.nowplaying.lyrics.lock().unwrap().clear();
            }
            continue;
        }
        tick(&core).await;
    }
}

pub async fn control(core: &Arc<Core>, key: &str, action: &str) -> Result<Value, String> {
    let a = match action {
        "play" => Action::Play,
        "pause" => Action::Pause,
        "next" => Action::Next,
        "previous" => Action::Previous,
        "toggle" => {
            let playing = core.nowplaying.last.lock().unwrap().as_ref().is_some_and(|s| s.now.playing);
            if playing { Action::Pause } else { Action::Play }
        }
        _ => return Err("Unknown media action.".into()),
    };
    let ok = media::control_key(key, a).await;
    let c = core.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(350)).await;
        c.nowplaying.refresh();
    });
    if ok { Ok(json!({})) } else { Err("That player didn't respond.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lrc_and_titles() {
        let l = parse_lrc("[00:12.50] Into paper birds tonight\n[00:15.00][01:15.00] And send them through the sky\n[ar:Lumen]\n");
        assert_eq!(l[0], (12_500, "Into paper birds tonight".into()));
        assert_eq!(l[1], (15_000, "And send them through the sky".into()));
        assert_eq!(l[2].0, 75_000);
        assert_eq!(clean_title("Slow It Down (Official Video)"), "Slow It Down");
        assert_eq!(percent_encode("Tum Hi Ho & co"), "Tum%20Hi%20Ho%20%26%20co");
        assert_eq!(percent_decode("/tmp/a%20b.jpg"), "/tmp/a b.jpg");
        let a = Snapshot { now: Now { key: "k".into(), title: "t".into(), playing: true, position_ms: Some(10_000), rate: 1.0, ..Default::default() }, art: String::new(), at: 1_000, lyrics: None };
        let mut b = a.clone();
        b.at = 3_000;
        b.now.position_ms = Some(12_100);
        assert!(!changed(&a, &b));
        b.now.position_ms = Some(40_000);
        assert!(changed(&a, &b));
    }
}
