use crate::core::Core;
use crate::ocr::{Image, Line, Ocr};
use crate::router::Intent;
use crate::tools::{Choice, FollowUp, Outcome};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use setu_common::engine::{FromEngine, ToEngine, VisionConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const LAYER: &str = "vision";
pub const MODELS: [&str; 2] = ["ocr-det-en", "ocr-rec-en"];
pub const SEE: [&str; 2] = ["llm-qwen35-0.8b", "vision-qwen35-0.8b"];
const BOX: &str = "root ::= \"{\\\"bbox_2d\\\": [\" num \", \" num \", \" num \", \" num \"]}\"\nnum ::= [0-9] | [1-9] [0-9] | [1-9] [0-9] [0-9] | \"1000\"\n";
const YES_NO: &str = "root ::= \"yes\" | \"no\"\n";
static ASKS: AtomicU64 = AtomicU64::new(1);
static LAST: AtomicU64 = AtomicU64::new(0);
static OCR: crate::work::Linger<Ocr> = crate::work::Linger::new(crate::work::MODEL_LINGER);

pub struct Shot {
    pub img: Image,
    pub win: Option<crate::os::Win>,
    pub scale: f64,
    pub origin: (f64, f64),
    pub seq: Option<u64>,
}

static MEMO: std::sync::LazyLock<std::sync::Mutex<crate::ocr::Memo>> = std::sync::LazyLock::new(Default::default);

pub fn installed(core: &Core) -> bool {
    MODELS.iter().all(|m| core.store.is_installed(m))
}

pub fn can_see(core: &Core) -> bool {
    SEE.iter().all(|m| core.store.is_installed(m))
}

fn missing(core: &Core, set: &[&str]) -> Vec<String> {
    set.iter().filter(|m| !core.store.is_installed(m)).map(|m| (*m).to_owned()).collect()
}

fn encode(img: &Image) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, img.w as u32, img.h as u32);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.set_compression(png::Compression::Fast);
        let mut w = e.write_header()?;
        w.write_image_data(&img.rgb)?;
    }
    Ok(out)
}

fn half(img: &Image) -> Image {
    let (w, h) = ((img.w / 2).max(1), (img.h / 2).max(1));
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                let at = |xx: usize, yy: usize| img.rgb[((yy.min(img.h - 1)) * img.w + xx.min(img.w - 1)) * 3 + c] as u32;
                rgb[(y * w + x) * 3 + c] = ((at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4) as u8;
            }
        }
    }
    Image { w, h, rgb }
}

pub fn picture(img: &Image) -> Result<Vec<u8>> {
    let mut small: Option<Image> = None;
    loop {
        let src = small.as_ref().unwrap_or(img);
        let b = encode(src)?;
        if b.len() <= 900_000 || src.w < 320 {
            return Ok(b);
        }
        small = Some(half(src));
    }
}

pub fn crop(img: &Image, b: (f64, f64, f64, f64)) -> Image {
    let (x1, y1, x2, y2) = b;
    let (mx, my) = (((x2 - x1) * 0.5).max(12.0), ((y2 - y1) * 0.5).max(12.0));
    let cx1 = (x1 - mx).max(0.0) as usize;
    let cy1 = (y1 - my).max(0.0) as usize;
    let cx2 = ((x2 + mx) as usize).min(img.w).max(cx1 + 1);
    let cy2 = ((y2 + my) as usize).min(img.h).max(cy1 + 1);
    let (cw, ch) = (cx2 - cx1, cy2 - cy1);
    let short = cw.min(ch).max(1);
    let k = 224usize.div_ceil(short).clamp(1, (1280 / cw.max(ch).max(1)).max(1));
    let (w, h) = (cw * k, ch * k);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let si = ((cy1 + y / k) * img.w + cx1 + x / k) * 3;
            let di = (y * w + x) * 3;
            rgb[di..di + 3].copy_from_slice(&img.rgb[si..si + 3]);
        }
    }
    Image { w, h, rgb }
}

pub fn parse_box(text: &str, w: usize, h: usize) -> Option<(f64, f64, f64, f64)> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let b = v.get("bbox_2d")?.as_array()?;
    if b.len() != 4 {
        return None;
    }
    let n: Vec<f64> = b.iter().filter_map(Value::as_f64).collect();
    if n.len() != 4 || n[2] <= n[0] || n[3] <= n[1] {
        return None;
    }
    let (x1, y1, x2, y2) = (n[0] / 1000.0 * w as f64, n[1] / 1000.0 * h as f64, n[2] / 1000.0 * w as f64, n[3] / 1000.0 * h as f64);
    ((x2 - x1) * (y2 - y1) <= 0.5 * (w * h) as f64).then_some((x1, y1, x2, y2))
}

pub fn the(what: &str) -> String {
    let w = what.trim();
    let l = w.to_lowercase();
    if l.starts_with("the ") || l.starts_with("a ") || l.starts_with("an ") || l.starts_with("my ") {
        w.to_owned()
    } else {
        format!("the {w}")
    }
}

async fn ask(core: &Arc<Core>, png: Vec<u8>, prompt: String, grammar: Option<&str>, max_tokens: u32) -> Result<String> {
    let model = core.store.file(SEE[0], "model").ok_or_else(|| anyhow!("The picture reader isn't downloaded yet."))?;
    let projector = core.store.file(SEE[1], "model").ok_or_else(|| anyhow!("The picture reader isn't downloaded yet."))?;
    let e = core.engines.vision.clone();
    let gpu = e.gpu();
    let key = format!("{}|{}|{gpu}", model.display(), projector.display());
    let config = VisionConfig { model: model.to_string_lossy().into_owned(), projector: projector.to_string_lossy().into_owned(), threads: 0, context: 2048, gpu_layers: if gpu { -1 } else { 0 }, min_tokens: 1024 };
    e.ensure(ToEngine::LoadVision { config }, &key).await?;
    let _g = e.exclusive().await;
    e.touch();
    let id = ASKS.fetch_add(1, Ordering::Relaxed);
    if !e.bytes(png) {
        return Err(anyhow!("The picture reader isn't running."));
    }
    let r = e.request(&ToEngine::VisionAsk { id, prompt, grammar: grammar.map(str::to_owned), max_tokens }, crate::engines::Expect::Vision(id), Duration::from_secs(90)).await;
    e.touch();
    release_later(core);
    match r? {
        FromEngine::VisionResult { text, ms, encode_ms, image_tokens, .. } => {
            log::info!("picture question answered in {ms} ms (picture {encode_ms} ms, {image_tokens} tokens)");
            Ok(text)
        }
        _ => Err(anyhow!("The picture reader gave no answer.")),
    }
}

fn release_later(core: &Arc<Core>) {
    let mine = LAST.fetch_add(1, Ordering::Relaxed) + 1;
    let e = core.engines.vision.clone();
    tokio::spawn(async move {
        tokio::time::sleep(crate::work::MODEL_LINGER).await;
        if LAST.load(Ordering::Relaxed) != mine || !e.is_running() {
            return;
        }
        let _g = e.exclusive().await;
        if LAST.load(Ordering::Relaxed) == mine && e.idle_for() >= crate::work::MODEL_LINGER {
            log::info!("releasing the picture reader after a minute unused");
            e.stop();
        }
    });
}

pub async fn find(core: &Arc<Core>, img: &Image, what: &str) -> Result<Option<(f64, f64, f64, f64)>> {
    let what = the(what);
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.ground");
    let r = async {
        let t = ask(core, picture(img)?, format!("Locate {what} in the image and output its bounding box as JSON: {{\"bbox_2d\": [x1, y1, x2, y2]}}."), Some(BOX), 40).await?;
        let Some(b) = parse_box(&t, img.w, img.h) else { return Ok(None) };
        let yes = ask(core, picture(&crop(img, b))?, format!("Is this {what}? Answer yes or no."), Some(YES_NO), 3).await?;
        Ok((yes.trim() == "yes").then_some(b))
    }
    .await;
    span.end(r)
}

fn spot(shot: &Shot, b: (f64, f64, f64, f64)) -> crate::tools::pointer::At {
    let (cx, cy) = ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0);
    match &shot.win {
        Some(w) if cfg!(target_os = "linux") => crate::tools::pointer::At { x: cx / shot.scale, y: cy / shot.scale, space: crate::tools::pointer::Space::Window(w.clone()) },
        _ => crate::tools::pointer::At { x: shot.origin.0 + cx / shot.scale, y: shot.origin.1 + cy / shot.scale, space: crate::tools::pointer::Space::Screen },
    }
}

async fn snap() -> Result<Shot> {
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.capture");
    span.end(capture().await)
}

pub async fn click_seen(core: &Arc<Core>, what: &str) -> Result<String> {
    let shot = snap().await?;
    let b = find(core, &shot.img, what).await?.ok_or_else(|| anyhow!("I couldn't see {} on the screen.", the(what)))?;
    crate::tools::pointer::click(Some(spot(&shot, b)), crate::tools::pointer::Button::Left, 1).await?;
    Ok(what.trim().trim_start_matches("the ").to_owned())
}

const SKIP_ROLES: &[&str] = &["frame", "window", "filler", "panel", "scroll pane", "scroll bar", "viewport", "separator", "menu bar", "tool bar", "application", "redundant object", "split pane", "layered pane", "root pane", "glass pane", "option pane", "image", "icon", "unknown", "invalid", "grouping", "section"];

pub fn tree_words(nodes: &[crate::os::Node], title: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = vec![crate::text::normalize(title)];
    for n in nodes {
        if SKIP_ROLES.contains(&n.role.as_str()) && n.value.trim().is_empty() {
            continue;
        }
        let raw = if !n.value.trim().is_empty() && (n.editable || matches!(n.role.as_str(), "text" | "paragraph" | "document web" | "document text" | "document frame" | "terminal" | "static" | "label" | "heading" | "entry")) { n.value.trim() } else { n.name.trim() };
        if raw.is_empty() || raw.chars().count() < 2 {
            continue;
        }
        let norm = crate::text::normalize(raw);
        if norm.is_empty() || seen.iter().any(|s| s.contains(&norm)) {
            continue;
        }
        seen.retain(|s| !norm.contains(s.as_str()) || s.is_empty());
        kept.retain(|k| !norm.contains(&crate::text::normalize(k)));
        seen.push(norm);
        kept.push(raw.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    kept.join(". ").replace("..", ".")
}

pub async fn window_words() -> Option<(String, String)> {
    let win = crate::os::working().await.ok()?;
    let nodes = crate::os::tree(&win, crate::os::Scope { limit: 3000, text: true, hidden: false }).await.ok()?;
    let words = tree_words(&nodes, &win.title);
    (words.chars().filter(|c| c.is_alphanumeric()).count() >= 30).then(|| (if win.app.is_empty() { win.title.clone() } else { win.app.clone() }, words))
}

pub async fn typed_words() -> Option<String> {
    let win = crate::os::working().await.ok()?;
    let node = crate::os::focused(&win).await.ok().flatten().filter(|n| n.editable)?;
    let text = crate::os::text(&node, 20_000).await.filter(|t| !t.trim().is_empty())?;
    let tail: Vec<&str> = text.trim().split_inclusive(['.', '!', '?', '\n']).collect();
    let mut out = String::new();
    for part in tail.iter().rev() {
        if out.chars().count() + part.chars().count() > 400 && !out.is_empty() {
            break;
        }
        out = format!("{part}{out}");
    }
    Some(out.trim().to_owned())
}

pub async fn point_seen(core: &Arc<Core>, what: &str) -> Result<()> {
    let shot = snap().await?;
    let b = find(core, &shot.img, what).await?.ok_or_else(|| anyhow!("I couldn't see {} on the screen.", the(what)))?;
    crate::tools::pointer::hover(spot(&shot, b)).await
}

pub fn place_words(b: (f64, f64, f64, f64), w: usize, h: usize) -> &'static str {
    let (cx, cy) = ((b.0 + b.2) / 2.0 / w.max(1) as f64, (b.1 + b.3) / 2.0 / h.max(1) as f64);
    let col = if cx < 1.0 / 3.0 { 0 } else if cx < 2.0 / 3.0 { 1 } else { 2 };
    let row = if cy < 1.0 / 3.0 { 0 } else if cy < 2.0 / 3.0 { 1 } else { 2 };
    [["at the top left", "at the top", "at the top right"], ["on the left", "in the middle", "on the right"], ["at the bottom left", "at the bottom", "at the bottom right"]][row][col]
}

fn ocr(core: &Core) -> Result<Arc<Ocr>> {
    let det = core.store.file(MODELS[0], "model").ok_or_else(|| anyhow!("The screen reader isn't downloaded yet."))?;
    let rec = core.store.file(MODELS[1], "model").ok_or_else(|| anyhow!("The screen reader isn't downloaded yet."))?;
    OCR.get(|| Ocr::load(&det, &rec))
}

#[cfg(target_os = "linux")]
async fn capture() -> Result<Shot> {
    let win = crate::os::front().await.ok();
    if let Some(w) = &win {
        match crate::screen::frame(w).await {
            Ok(f) => return Ok(Shot { img: f.img, win, scale: f.scale, origin: (0.0, 0.0), seq: Some(f.seq) }),
            Err(e) => log::info!("no live window picture ({e}); taking one picture"),
        }
    }
    capture_once(win).await
}

#[cfg(target_os = "linux")]
async fn capture_once(win: Option<crate::os::Win>) -> Result<Shot> {
    use futures_util::StreamExt;
    use zbus::zvariant::{OwnedObjectPath, Value as ZValue};
    const SC: &str = "org.gnome.Mutter.ScreenCast";
    const SCS: &str = "org.gnome.Mutter.ScreenCast.Session";
    let c = zbus::Connection::session().await?;
    let none: std::collections::HashMap<&str, ZValue> = std::collections::HashMap::new();
    let session: OwnedObjectPath = c.call_method(Some(SC), "/org/gnome/Mutter/ScreenCast", Some(SC), "CreateSession", &(none.clone(),)).await.map_err(|_| anyhow!("I can only see the screen on GNOME for now."))?.body().deserialize()?;
    let r = async {
        let mut props: std::collections::HashMap<&str, ZValue> = std::collections::HashMap::new();
        props.insert("cursor-mode", ZValue::from(0u32));
        let stream: OwnedObjectPath = c.call_method(Some(SC), session.as_str(), Some(SCS), "RecordWindow", &(props,)).await?.body().deserialize()?;
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.gnome.Mutter.ScreenCast.Stream")?.member("PipeWireStreamAdded")?.path(stream.as_str())?.build();
        let mut added = zbus::MessageStream::for_match_rule(rule, &c, Some(4)).await?;
        c.call_method(Some(SC), session.as_str(), Some(SCS), "Start", &()).await?;
        let msg = tokio::time::timeout(Duration::from_secs(3), added.next()).await.map_err(|_| anyhow!("GNOME didn't share the window."))?.ok_or_else(|| anyhow!("GNOME didn't share the window."))??;
        let (node,): (u32,) = msg.body().deserialize()?;
        let file = std::env::temp_dir().join(format!("setu-shot-{}-{node}.png", std::process::id()));
        let f2 = file.clone();
        let ok = tokio::time::timeout(
            Duration::from_secs(6),
            tokio::process::Command::new("gst-launch-1.0")
                .args(["-q", "pipewiresrc", &format!("path={node}"), "num-buffers=1", "always-copy=true", "!", "videoconvert", "!", "pngenc", "!", "filesink"])
                .arg(format!("location={}", f2.display()))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status(),
        )
        .await
        .map_err(|_| anyhow!("The window picture didn't arrive."))?
        .map(|s| s.success())
        .unwrap_or(false);
        let bytes = std::fs::read(&file);
        let _ = std::fs::remove_file(&file);
        if !ok {
            return Err(anyhow!("The window picture couldn't be taken. GStreamer's PipeWire plugin is needed."));
        }
        crate::ocr::decode_png(&bytes?)
    }
    .await;
    let _ = c.call_method(Some(SC), session.as_str(), Some(SCS), "Stop", &()).await;
    let img = r?;
    let scale = crate::syscfg_linux::current_scale().await.unwrap_or(1.0).max(0.5);
    Ok(Shot { img, win, scale, origin: (0.0, 0.0), seq: None })
}

#[cfg(windows)]
async fn capture() -> Result<Shot> {
    match crate::layers::blocking(crate::wincap::frame).await {
        Ok(Ok(f)) => return Ok(Shot { img: f.img, win: None, scale: 1.0, origin: f.origin, seq: Some(f.seq) }),
        Ok(Err(e)) => log::info!("no live window picture ({e}); taking one picture"),
        Err(_) => {}
    }
    let (w, h, rgb, left, top) = crate::layers::blocking(crate::tools::win::capture_front).await.map_err(|e| anyhow!("{e}"))??;
    Ok(Shot { img: Image { w, h, rgb }, win: None, scale: 1.0, origin: (left as f64, top as f64), seq: None })
}

#[cfg(target_os = "macos")]
async fn capture() -> Result<Shot> {
    match crate::layers::blocking(crate::maccap::frame).await {
        Ok(Ok(f)) => return Ok(Shot { img: f.img, win: None, scale: f.scale, origin: f.origin, seq: Some(f.seq) }),
        Ok(Err(e)) => log::info!("no live window picture ({e}); taking one picture"),
        Err(_) => {}
    }
    let file = std::env::temp_dir().join(format!("setu-shot-{}.png", std::process::id()));
    let f2 = file.clone();
    let ok = crate::layers::blocking(move || std::process::Command::new("screencapture").args(["-x", "-m", "-t", "png"]).arg(&f2).status().map(|s| s.success()).unwrap_or(false)).await.unwrap_or(false);
    let bytes = std::fs::read(&file);
    let _ = std::fs::remove_file(&file);
    if !ok {
        return Err(anyhow!("macOS didn't allow a screen picture. Turn on Screen Recording for Setu in Privacy and Security."));
    }
    let img = crate::ocr::decode_png(&bytes?)?;
    let points = crate::tools::pointer::main_display_width().unwrap_or(img.w as f64);
    Ok(Shot { scale: img.w as f64 / points.max(1.0), img, win: None, origin: (0.0, 0.0), seq: None })
}

pub async fn look(core: &Arc<Core>) -> Result<(Shot, Vec<Line>)> {
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.capture");
    let shot = span.end(capture().await)?;
    let c = core.clone();
    let img = shot.img.clone();
    let key = shot.win.as_ref().map(|w| w.id.clone()).unwrap_or_else(|| "screen".into());
    let seq = shot.seq;
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.ocr");
    let read = crate::work::background("ocr", move || {
        let o = ocr(&c)?;
        let mut memo = MEMO.lock().unwrap_or_else(|e| e.into_inner());
        o.read_changed(&img, &key, seq, &mut memo)
    })
    .await
    .and_then(|r| r);
    let (lines, pass) = span.end(read)?;
    crate::layers::record(
        match pass {
            crate::ocr::Pass::Same => "vision.ocr.same",
            crate::ocr::Pass::Bands(_) => "vision.ocr.changed",
            crate::ocr::Pass::Full => "vision.ocr.full",
        },
        0,
    );
    log::info!("screen read: {:?}, {} lines", pass, lines.len());
    Ok((shot, lines))
}

pub async fn elements(core: &Arc<Core>) -> Result<(Shot, Vec<crate::uidetect::Element>)> {
    let (shot, lines) = look(core).await?;
    let img = shot.img.clone();
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.detect");
    let els = span.end(crate::layers::blocking(move || crate::uidetect::detect(&img, &lines)).await.map_err(|e| anyhow!("{e}")))?;
    Ok((shot, els))
}

fn at_point(shot: &Shot, cx: f64, cy: f64) -> crate::tools::pointer::At {
    match &shot.win {
        Some(w) if cfg!(target_os = "linux") => crate::tools::pointer::At { x: cx / shot.scale, y: cy / shot.scale, space: crate::tools::pointer::Space::Window(w.clone()) },
        _ => crate::tools::pointer::At { x: shot.origin.0 + cx / shot.scale, y: shot.origin.1 + cy / shot.scale, space: crate::tools::pointer::Space::Screen },
    }
}

pub async fn fill_seen(core: &Arc<Core>, name: &str, text: &str) -> Result<String> {
    if !installed(core) {
        return Err(anyhow!("not installed"));
    }
    let (shot, els) = elements(core).await?;
    let f = crate::uidetect::field(&els, name).ok_or_else(|| anyhow!("I couldn't see the {name} field."))?.clone();
    let (cx, cy) = f.center();
    crate::tools::pointer::click(Some(at_point(&shot, cx, cy)), crate::tools::pointer::Button::Left, 1).await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    crate::tools::input::type_text(text).await?;
    Ok(if f.label.is_empty() { name.trim().to_owned() } else { f.label })
}

pub async fn click_text(core: &Arc<Core>, text: &str) -> Result<String> {
    if !installed(core) {
        return Err(anyhow!("not installed"));
    }
    let (shot, lines) = look(core).await?;
    let line = crate::ocr::best_match(&lines, text).ok_or_else(|| anyhow!("I couldn't see {text} on the screen."))?.clone();
    let (cx, cy) = (line.x + line.w / 2.0, line.y + line.h / 2.0);
    let at = match &shot.win {
        Some(w) if cfg!(target_os = "linux") => crate::tools::pointer::At { x: cx / shot.scale, y: cy / shot.scale, space: crate::tools::pointer::Space::Window(w.clone()) },
        _ => crate::tools::pointer::At { x: shot.origin.0 + cx / shot.scale, y: shot.origin.1 + cy / shot.scale, space: crate::tools::pointer::Space::Screen },
    };
    crate::tools::pointer::click(Some(at), crate::tools::pointer::Button::Left, 1).await?;
    Ok(line.text)
}

fn offer(question: String, then: Intent) -> Outcome {
    let yes = Intent::Act { tool: "vision.setup".into(), args: json!({ "then": serde_json::to_value(match &then {
        Intent::Act { tool, args } => json!({ "tool": tool, "args": args }),
        _ => Value::Null,
    }).unwrap_or(Value::Null) }) };
    let no = Intent::Act { tool: "vision.none".into(), args: json!({}) };
    Outcome::asking("Asked to download the screen reader", LAYER, FollowUp { question: question.clone(), spoken: Some(question), choices: vec![Choice::new("Download", &["yes", "download it", "ok", "okay", "sure", "go ahead"], yes), Choice::new("Not now", &["no", "not now", "cancel"], no)], cleanup: None, kind: "confirm", focus: None })
}

fn seen_target(t: &str) -> Option<&str> {
    let t = t.trim();
    let t = t.strip_suffix(" on the screen").or_else(|| t.strip_suffix(" on my screen")).or_else(|| t.strip_suffix(" on screen")).or_else(|| t.strip_suffix(" in this window")).or_else(|| t.strip_suffix(" in the window"))?;
    let t = t.trim();
    (!t.is_empty() && t.split_whitespace().count() <= 8).then_some(t)
}

pub fn rules(t: &str) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    if matches!(t, "describe the screen" | "describe my screen" | "describe this window" | "describe the window" | "describe this picture" | "describe this image" | "describe the picture" | "what do you see" | "what can you see" | "what's in this picture" | "what is in this picture" | "what's in this image" | "what is in this image" | "what is this picture" | "what's this picture" | "look at the screen and tell me what you see") {
        return act("vision.describe", json!({}));
    }
    if matches!(t, "set up the picture reader" | "download the picture reader" | "get the picture reader" | "learn to see the screen" | "find things by how they look") {
        return act("vision.setup", json!({ "models": "see" }));
    }
    for p in ["where is ", "where's ", "find "] {
        if let Some(x) = t.strip_prefix(p).and_then(seen_target) {
            return act("vision.where", json!({ "what": x }));
        }
    }
    if let Some(x) = t.strip_prefix("show me where ").and_then(|x| x.strip_suffix(" is")) {
        let x = seen_target(x).unwrap_or(x).trim();
        if !x.is_empty() && x.split_whitespace().count() <= 8 {
            return act("vision.point", json!({ "what": x }));
        }
    }
    if matches!(t, "what's on the screen" | "what is on the screen" | "what's on my screen" | "what is on my screen" | "read the screen" | "read my screen" | "read this window" | "read the window" | "what does the screen say" | "what does this say") {
        return act("vision.read", json!({}));
    }
    if matches!(t, "read that back" | "read back what i typed" | "read back what i wrote" | "read it back") {
        return act("vision.read", json!({ "back": true }));
    }
    if matches!(t, "read this picture" | "read this image" | "read the text in this picture" | "read the text in this image" | "what does this picture say" | "what does this image say") {
        return act("vision.read", json!({ "picture": true }));
    }
    for p in ["click the thing that says ", "click the button that says ", "click on the text ", "click the text ", "press the thing that says ", "click where it says ", "click on the thing that says "] {
        if let Some(r) = t.strip_prefix(p) {
            return act("vision.click", json!({ "text": r.trim() }));
        }
    }
    if matches!(t, "record the screen" | "start recording the screen" | "record my screen" | "start a screen recording") {
        let keys = if cfg!(windows) { "win alt r" } else if cfg!(target_os = "macos") { "cmd shift 5" } else { "ctrl alt shift r" };
        return act("key", json!({ "keys": keys, "label": "Started recording the screen" }));
    }
    None
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool {
        "vision.none" => Outcome::ok_with("Left it", LAYER, None).with_reply("Okay. I'll keep to what apps tell me about themselves.".into()),
        "vision.setup" => {
            let set: &[&str] = if s("models") == "see" { &SEE } else { &MODELS };
            let what = if s("models") == "see" { "picture reader" } else { "screen reader" };
            let ids = missing(core, set);
            if ids.is_empty() {
                return Outcome::ok_with(&format!("The {what} is ready"), LAYER, None).with_reply(format!("The {what} is already on this computer."));
            }
            if let Err(e) = core.downloads.enqueue(&ids) {
                return Outcome::fail(&e.message, LAYER);
            }
            let set: Vec<String> = set.iter().map(|m| (*m).to_owned()).collect();
            let ready = move |c: &Core| set.iter().all(|m| c.store.is_installed(m));
            let then = args.get("then").cloned().filter(|t| t.get("tool").is_some());
            let c = core.clone();
            let w2 = what;
            tokio::spawn(async move {
                for _ in 0..7200 {
                    if ready(&c) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if !ready(&c) {
                    crate::board::alert(&c, "install", &format!("The {w2} didn't finish downloading. Ask me again later."));
                    return;
                }
                let next = then.and_then(|t| Some(Intent::Act { tool: t.get("tool")?.as_str()?.to_owned(), args: t.get("args").cloned().unwrap_or_else(|| json!({})) }));
                match next {
                    Some(intent) => {
                        if crate::control::ask(&c, |r| crate::control::Req::RunIntent { intent, reply: r }).await.is_err() {
                            crate::board::alert(&c, "install", &format!("The {w2} is ready. Ask me again."));
                        }
                    }
                    None => crate::board::alert(&c, "install", &format!("The {w2} is ready.")),
                }
            });
            Outcome::ok_with(&format!("Downloading the {what}"), LAYER, None).with_reply(format!("Downloading the {what} now. I'll carry on as soon as it's ready."))
        }
        "vision.describe" | "vision.where" | "vision.point" if !can_see(core) => see_offer(core, tool, args),
        "vision.describe" => {
            let shot = match snap().await {
                Ok(s) => s,
                Err(e) => return Outcome::fail(&e.to_string(), LAYER),
            };
            let png = match picture(&shot.img) {
                Ok(p) => p,
                Err(e) => return Outcome::fail(&e.to_string(), LAYER),
            };
            match ask(core, png, "Describe what this picture shows in two short plain sentences, for someone who can't see it.".into(), None, 90).await {
                Ok(t) => {
                    let t = crate::pipeline::tidy_answer(&t, false);
                    if t.split_whitespace().count() < 3 { Outcome::fail("I couldn't make out the picture.", LAYER) } else { Outcome::ok_with("Described the screen", LAYER, None).with_reply(t) }
                }
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "vision.where" | "vision.point" => {
            let what = s("what");
            let shot = match snap().await {
                Ok(s) => s,
                Err(e) => return Outcome::fail(&e.to_string(), LAYER),
            };
            match find(core, &shot.img, &what).await {
                Ok(Some(b)) if tool == "vision.where" => {
                    let name = the(&what);
                    let mut c = name.chars();
                    let name = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                    Outcome::ok_with(&format!("Found {what}"), LAYER, None).with_reply(format!("{name} is {} of the window.", place_words(b, shot.img.w, shot.img.h)))
                }
                Ok(Some(b)) => match crate::tools::pointer::hover(spot(&shot, b)).await {
                    Ok(()) => Outcome::ok_with(&format!("Pointing at {what}"), LAYER, None),
                    Err(e) => Outcome::fail(&e.to_string(), LAYER),
                },
                Ok(None) => Outcome::fail(&format!("I couldn't see {} on the screen.", the(&what)), LAYER),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "vision.read" if args.get("back").and_then(Value::as_bool) == Some(true) && args.get("picture").is_none() => match typed_words().await {
            Some(t) => Outcome::ok_with("Read back the text", "accessibility", None).with_reply(t),
            None => Box::pin(run(core, "vision.read", &json!({}))).await,
        },
        "vision.read" if args.get("picture").is_none() && args.get("pixels").is_none() => match window_words().await {
            Some((app, words)) => {
                let short: String = words.chars().take(1200).collect();
                let more = if words.chars().count() > 1200 { " That's the start of it." } else { "" };
                Outcome::ok_with(&format!("Read {app}"), "accessibility", None).with_reply(format!("{short}{more}"))
            }
            None => Box::pin(run(core, "vision.read", &json!({ "pixels": true }))).await,
        },
        "vision.read" => {
            if !installed(core) {
                return offer("Reading the screen needs Setu's text reader, an 11 megabyte download that runs on this computer. Download it?".into(), Intent::Act { tool: tool.into(), args: args.clone() });
            }
            match look(core).await {
                Ok((_, lines)) if lines.is_empty() => Outcome::ok_with("No text", LAYER, None).with_reply("I don't see any text in this window.".into()),
                Ok((_, lines)) => {
                    let text = crate::ocr::join(&lines).replace('\n', ". ");
                    let short: String = text.chars().take(1200).collect();
                    let more = if text.chars().count() > 1200 { " That's the start of it." } else { "" };
                    Outcome::ok_with(&format!("Read {} lines", lines.len()), LAYER, None).with_reply(format!("{short}{more}"))
                }
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "vision.click" => {
            if !installed(core) {
                return offer("Finding things by what they look like needs Setu's text reader, an 11 megabyte download that runs on this computer. Download it?".into(), Intent::Act { tool: tool.into(), args: args.clone() });
            }
            match click_text(core, &s("text")).await {
                Ok(t) => Outcome::ok_with(&format!("Clicked {t}"), LAYER, None),
                Err(e) => match can_see(core) {
                    true => match click_seen(core, &format!("the thing that says {}", s("text"))).await {
                        Ok(_) => Outcome::ok_with(&format!("Clicked {}", s("text")), LAYER, None),
                        Err(_) => Outcome::fail(&e.to_string(), LAYER),
                    },
                    false => Outcome::fail(&e.to_string(), LAYER),
                },
            }
        }
        _ => Outcome::fail("I don't know that screen command.", LAYER),
    }
}

fn see_offer(core: &Core, tool: &str, args: &Value) -> Outcome {
    let llm = !core.store.is_installed(SEE[0]);
    let size = if llm { "a 740 megabyte download" } else { "a 205 megabyte download" };
    let yes = Intent::Act { tool: "vision.setup".into(), args: json!({ "models": "see", "then": { "tool": tool, "args": args } }) };
    let no = Intent::Act { tool: "vision.none".into(), args: json!({}) };
    let question = format!("Seeing things by how they look needs Setu's picture reader, {size} that runs on this computer. Download it?");
    Outcome::asking("Asked to download the picture reader", LAYER, FollowUp { question: question.clone(), spoken: Some(question), choices: vec![Choice::new("Download", &["yes", "download it", "ok", "okay", "sure", "go ahead"], yes), Choice::new("Not now", &["no", "not now", "cancel"], no)], cleanup: None, kind: "confirm", focus: None })
}

pub fn label(tool: &str, args: &Value) -> String {
    match tool {
        "vision.describe" => "Looking at the screen".into(),
        "vision.where" | "vision.point" => format!("Looking for {}", args.get("what").and_then(Value::as_str).unwrap_or_default()),
        "vision.read" => "Reading the screen".into(),
        "vision.click" => format!("Looking for {}", args.get("text").and_then(Value::as_str).unwrap_or_default()),
        "vision.setup" => "Getting the screen reader".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_phrases() {
        let tool = |t: &str| match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        };
        assert_eq!(tool("what's on the screen").unwrap().0, "vision.read");
        assert_eq!(tool("click the thing that says start game").unwrap().1["text"], "start game");
        assert_eq!(tool("record the screen").unwrap().0, "key");
        assert!(tool("click the save button").is_none());
        assert_eq!(tool("where is the save button on the screen").unwrap().1["what"], "the save button");
        assert_eq!(tool("show me where the quit button is").unwrap().0, "vision.point");
        assert_eq!(tool("describe the screen").unwrap().0, "vision.describe");
        assert!(tool("where is my invoice").is_none());
        assert!(tool("hover over the menu").is_none());
    }

    #[test]
    fn boxes_are_read_in_thousandths_and_whole_screen_answers_are_refused() {
        assert_eq!(parse_box("{\"bbox_2d\": [500, 0, 1000, 500]}", 800, 400), Some((400.0, 0.0, 800.0, 200.0)));
        assert_eq!(parse_box("{\"bbox_2d\": [0, 0, 999, 999]}", 800, 400), None);
        assert_eq!(parse_box("{\"bbox_2d\": [10, 10, 5, 20]}", 800, 400), None);
        assert_eq!(parse_box("nothing", 800, 400), None);
        assert_eq!(place_words((700.0, 0.0, 790.0, 40.0), 800, 400), "at the top right");
        assert_eq!(place_words((380.0, 180.0, 420.0, 220.0), 800, 400), "in the middle");
        assert_eq!(the("gear icon"), "the gear icon");
        assert_eq!(the("a red button"), "a red button");
    }

    #[test]
    fn window_text_is_read_once_in_order_without_the_frame() {
        let n = |role: &str, name: &str, value: &str| crate::os::Node { role: role.into(), name: name.into(), value: value.into(), ..Default::default() };
        let nodes = vec![
            n("frame", "Report - Editor", ""),
            n("panel", "", ""),
            n("heading", "Quarterly report", ""),
            n("paragraph", "", "Sales grew by ten percent."),
            n("text", "", "Sales grew by ten percent."),
            n("push button", "Save", ""),
            n("filler", "", ""),
            n("label", "Sales", ""),
        ];
        assert_eq!(tree_words(&nodes, "Report - Editor"), "Quarterly report. Sales grew by ten percent.. Save".replace("..", "."));
    }

    #[test]
    fn pictures_stay_under_the_frame_limit_and_crops_are_enlarged() {
        let noise: Vec<u8> = (0..1600 * 1000 * 3).map(|i| ((i as u64).wrapping_mul(2654435761) >> 13) as u8).collect();
        let img = Image { w: 1600, h: 1000, rgb: noise };
        assert!(picture(&img).unwrap().len() <= 900_000);
        let c = crop(&img, (100.0, 100.0, 140.0, 120.0));
        assert!(c.w.min(c.h) >= 224 && c.w * c.h * 3 == c.rgb.len());
    }
}
