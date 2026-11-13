use crate::ocr::Image;
use anyhow::{Result, anyhow};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use zbus::zvariant::{OwnedObjectPath, Value as ZValue};

const SC: &str = "org.gnome.Mutter.ScreenCast";
const SCS: &str = "org.gnome.Mutter.ScreenCast.Session";
const LINGER: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(60);
const SETTLE: Duration = Duration::from_millis(300);

static NEXT: AtomicU64 = AtomicU64::new(1);
static REAPING: AtomicBool = AtomicBool::new(false);

struct Raw {
    w: usize,
    h: usize,
    chroma: Chroma,
    yuv: Vec<u8>,
}

#[derive(Default)]
struct Slot {
    seq: u64,
    at: Option<Instant>,
    raw: Option<Arc<Raw>>,
    done: bool,
    said: Vec<String>,
}

type Shared = Arc<(Mutex<Slot>, Condvar)>;

struct Live {
    win: String,
    conn: zbus::Connection,
    session: OwnedObjectPath,
    child: Child,
    slot: Shared,
    used: Instant,
    scale: f64,
    made: Option<(u64, Arc<Image>)>,
}

static LIVE: tokio::sync::Mutex<Option<Live>> = tokio::sync::Mutex::const_new(None);

pub struct Frame {
    pub img: Image,
    pub seq: u64,
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    Full,
    Quarter,
}

pub fn header(line: &str) -> Option<(usize, usize, Chroma)> {
    let mut parts = line.trim_end().split(' ');
    if parts.next()? != "YUV4MPEG2" {
        return None;
    }
    let (mut w, mut h, mut c) = (0usize, 0usize, String::from("420jpeg"));
    for p in parts {
        match p.as_bytes().first() {
            Some(b'W') => w = p[1..].parse().ok()?,
            Some(b'H') => h = p[1..].parse().ok()?,
            Some(b'C') => c = p[1..].to_owned(),
            _ => {}
        }
    }
    let chroma = match c.as_str() {
        "444" => Chroma::Full,
        "420jpeg" | "420mpeg2" | "420paldv" | "420" => Chroma::Quarter,
        _ => return None,
    };
    (w > 0 && h > 0 && w <= 16384 && h <= 16384).then_some((w, h, chroma))
}

fn plane(w: usize, h: usize, c: Chroma) -> (usize, usize) {
    match c {
        Chroma::Full => (w, h),
        Chroma::Quarter => (w.div_ceil(2), h.div_ceil(2)),
    }
}

pub fn frame_bytes(w: usize, h: usize, c: Chroma) -> usize {
    let (cw, ch) = plane(w, h, c);
    w * h + 2 * cw * ch
}

pub fn rgb(w: usize, h: usize, c: Chroma, yuv: &[u8]) -> Image {
    let n = w * h;
    let (cw, ch) = plane(w, h, c);
    let m = cw * ch;
    let (y, u, v) = (&yuv[..n], &yuv[n..n + m], &yuv[n + m..n + 2 * m]);
    let mut out = vec![0u8; n * 3];
    for row in 0..h {
        let crow = if c == Chroma::Full { row } else { row / 2 };
        for col in 0..w {
            let i = row * w + col;
            let j = crow * cw + if c == Chroma::Full { col } else { col / 2 };
            let cy = (y[i] as i32 - 16) * 298;
            let d = u[j] as i32 - 128;
            let e = v[j] as i32 - 128;
            out[i * 3] = ((cy + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            out[i * 3 + 1] = ((cy - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            out[i * 3 + 2] = ((cy + 516 * d + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    Image { w, h, rgb: out }
}

fn pump(out: std::process::ChildStdout, slot: Shared) {
    let mut r = BufReader::with_capacity(1 << 20, out);
    let mut line = String::new();
    let size = match r.read_line(&mut line) {
        Ok(n) if n > 0 => header(&line),
        _ => None,
    };
    if let Some((w, h, chroma)) = size {
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(n) if n > 0 && line.starts_with("FRAME") => {}
                _ => break,
            }
            let mut buf = vec![0u8; frame_bytes(w, h, chroma)];
            if r.read_exact(&mut buf).is_err() {
                break;
            }
            let mut g = slot.0.lock().unwrap();
            g.seq = NEXT.fetch_add(1, Ordering::SeqCst);
            g.at = Some(Instant::now());
            g.raw = Some(Arc::new(Raw { w, h, chroma, yuv: buf }));
            slot.1.notify_all();
        }
    }
    slot.0.lock().unwrap().done = true;
    slot.1.notify_all();
}

impl Live {
    fn dead(&mut self) -> bool {
        self.slot.0.lock().unwrap().done || !matches!(self.child.try_wait(), Ok(None))
    }
}

async fn close(mut l: Live) {
    let _ = l.child.kill();
    let _ = l.child.wait();
    let _ = tokio::time::timeout(Duration::from_secs(2), l.conn.call_method(Some(SC), l.session.as_str(), Some(SCS), "Stop", &())).await;
}

async fn open(win: &crate::os::Win) -> Result<Live> {
    use futures_util::StreamExt;
    let span = crate::layers::read(crate::layers::Layer::Vision, "vision.stream.open");
    let c = zbus::Connection::session().await?;
    let none: std::collections::HashMap<&str, ZValue> = std::collections::HashMap::new();
    let session: OwnedObjectPath = c.call_method(Some(SC), "/org/gnome/Mutter/ScreenCast", Some(SC), "CreateSession", &(none,)).await.map_err(|_| anyhow!("I can only see the screen on GNOME for now."))?.body().deserialize()?;
    let started = async {
        let mut props: std::collections::HashMap<&str, ZValue> = std::collections::HashMap::new();
        props.insert("cursor-mode", ZValue::from(0u32));
        let stream: OwnedObjectPath = c.call_method(Some(SC), session.as_str(), Some(SCS), "RecordWindow", &(props,)).await?.body().deserialize()?;
        let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.gnome.Mutter.ScreenCast.Stream")?.member("PipeWireStreamAdded")?.path(stream.as_str())?.build();
        let mut added = zbus::MessageStream::for_match_rule(rule, &c, Some(4)).await?;
        c.call_method(Some(SC), session.as_str(), Some(SCS), "Start", &()).await?;
        let msg = tokio::time::timeout(Duration::from_secs(3), added.next()).await.map_err(|_| anyhow!("GNOME didn't share the window."))?.ok_or_else(|| anyhow!("GNOME didn't share the window."))??;
        let (node,): (u32,) = msg.body().deserialize()?;
        let mut cmd = Command::new("gst-launch-1.0");
        cmd.args(["-q", "pipewiresrc", &format!("path={node}"), "always-copy=true", "!", "videoconvert", "!", "video/x-raw,format=I420", "!", "y4menc", "!", "fdsink", "fd=1", "sync=false"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|_| anyhow!("The window picture couldn't be taken. GStreamer's PipeWire plugin is needed."))?;
        let out = child.stdout.take().ok_or_else(|| anyhow!("no picture stream"))?;
        let slot: Shared = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let s2 = slot.clone();
        std::thread::Builder::new().name("setu-screen".into()).spawn(move || pump(out, s2))?;
        if let Some(err) = child.stderr.take() {
            let s4 = slot.clone();
            std::thread::Builder::new().name("setu-screen-log".into()).spawn(move || {
                for line in BufReader::new(err).lines().map_while(Result::ok) {
                    let mut g = s4.0.lock().unwrap();
                    if g.said.len() >= 6 {
                        g.said.remove(0);
                    }
                    g.said.push(line.chars().take(200).collect());
                }
            })?;
        }
        let s3 = slot.clone();
        let first = crate::layers::blocking(move || {
            let until = Instant::now() + Duration::from_secs(4);
            let mut g = s3.0.lock().unwrap();
            while g.raw.is_none() && !g.done {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                g = s3.1.wait_timeout(g, left).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
            }
            g.raw.is_some()
        })
        .await
        .unwrap_or(false);
        if !first {
            let _ = child.kill();
            let _ = child.wait();
            std::thread::sleep(Duration::from_millis(50));
            let said = slot.0.lock().unwrap().said.join(" | ");
            return Err(anyhow!("The window picture didn't arrive.{}", if said.is_empty() { String::new() } else { format!(" GStreamer said: {said}") }));
        }
        Ok::<_, anyhow::Error>((child, slot))
    }
    .await;
    match started {
        Ok((child, slot)) => {
            span.ok();
            let scale = crate::syscfg_linux::current_scale().await.unwrap_or(1.0).max(0.5);
            Ok(Live { win: win.id.clone(), conn: c, session, child, slot, used: Instant::now(), scale, made: None })
        }
        Err(e) => {
            let _ = c.call_method(Some(SC), session.as_str(), Some(SCS), "Stop", &()).await;
            Err(e)
        }
    }
}

fn reap() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut g = LIVE.lock().await;
            let idle = g.as_ref().is_none_or(|l| l.used.elapsed() >= LINGER);
            if idle {
                if let Some(l) = g.take() {
                    close(l).await;
                }
                REAPING.store(false, Ordering::SeqCst);
                return;
            }
        }
    });
}

fn settled(slot: &Shared) -> (u64, Option<Arc<Raw>>) {
    let until = Instant::now() + SETTLE;
    let mut g = slot.0.lock().unwrap();
    while g.at.is_some_and(|t| t.elapsed() < QUIET) && Instant::now() < until {
        let wait = QUIET.saturating_sub(g.at.map(|t| t.elapsed()).unwrap_or(QUIET)).max(Duration::from_millis(5));
        g = slot.1.wait_timeout(g, wait).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
    }
    (g.seq, g.raw.clone())
}

pub async fn frame(win: &crate::os::Win) -> Result<Frame> {
    let mut g = LIVE.lock().await;
    if g.as_mut().is_none_or(|l| l.win != win.id || l.dead()) {
        if let Some(old) = g.take() {
            close(old).await;
        }
        *g = Some(open(win).await?);
        reap();
    }
    let l = g.as_mut().ok_or_else(|| anyhow!("no picture stream"))?;
    l.used = Instant::now();
    let (slot, scale, made) = (l.slot.clone(), l.scale, l.made.clone());
    drop(g);
    let (seq, raw) = crate::layers::blocking(move || settled(&slot)).await.map_err(|e| anyhow!("{e}"))?;
    if let Some((s, img)) = made.filter(|(s, _)| *s == seq) {
        return Ok(Frame { img: (*img).clone(), seq: s, scale });
    }
    let raw = raw.ok_or_else(|| anyhow!("The window picture didn't arrive."))?;
    let img = Arc::new(crate::layers::blocking(move || rgb(raw.w, raw.h, raw.chroma, &raw.yuv)).await.map_err(|e| anyhow!("{e}"))?);
    if let Some(l) = LIVE.lock().await.as_mut().filter(|l| l.win == win.id) {
        l.made = Some((seq, img.clone()));
    }
    Ok(Frame { img: (*img).clone(), seq, scale })
}

pub async fn stop() {
    if let Some(l) = LIVE.lock().await.take() {
        close(l).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_stream_header_and_colours() {
        assert_eq!(header("YUV4MPEG2 W900 H560 F0:1 Ip A1:1 C444\n"), Some((900, 560, Chroma::Full)));
        assert_eq!(header("YUV4MPEG2 C420jpeg W65 H33 Ip F0:1 A1:1\n"), Some((65, 33, Chroma::Quarter)));
        assert_eq!(header("YUV4MPEG2 W900 H560 F0:1\n"), Some((900, 560, Chroma::Quarter)));
        assert_eq!(header("YUV4MPEG2 W9 H5 C422\n"), None);
        assert_eq!(header("RIFF"), None);
        assert_eq!(frame_bytes(65, 33, Chroma::Quarter), 65 * 33 + 2 * 33 * 17);
        let img = rgb(2, 1, Chroma::Full, &[235, 16, 128, 128, 128, 128]);
        assert_eq!(&img.rgb[..3], &[255, 255, 255]);
        assert_eq!(&img.rgb[3..], &[0, 0, 0]);
        let red = rgb(1, 1, Chroma::Full, &[81, 90, 240]);
        assert!(red.rgb[0] > 240 && red.rgb[1] < 20 && red.rgb[2] < 20, "{:?}", red.rgb);
        let q = rgb(3, 3, Chroma::Quarter, &[235, 235, 235, 235, 235, 235, 16, 16, 16, 128, 128, 128, 128, 128, 128, 128, 128]);
        assert_eq!(&q.rgb[..3], &[255, 255, 255]);
        assert_eq!(&q.rgb[6 * 3..6 * 3 + 3], &[0, 0, 0]);
    }
}
