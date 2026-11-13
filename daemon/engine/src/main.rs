#![cfg_attr(windows, windows_subsystem = "windows")]
mod asr;
mod llama;
mod llm;
mod phonemize;
mod sandbox;
mod svara;
mod tts;
mod vision;

use crossbeam_channel::{Receiver, bounded};
use setu_common::engine::{EngineKind, FromEngine, Packet, ToEngine, read_packet, write_json};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub enum Msg {
    Op(ToEngine),
    Speak(ToEngine, u64),
    Pcm(Vec<f32>),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
pub struct Out {
    w: Arc<Mutex<BufWriter<std::io::Stdout>>>,
}

impl Out {
    pub fn sink() -> Self {
        Self { w: Arc::new(Mutex::new(BufWriter::new(std::io::stdout()))) }
    }

    pub fn send(&self, m: &FromEngine) {
        if let Ok(mut w) = self.w.lock()
            && write_json(&mut *w, m).is_err() {
                std::process::exit(0);
            }
    }

    pub fn error(&self, message: impl Into<String>, fatal: bool) {
        self.send(&FromEngine::Error { message: message.into(), fatal });
    }
}

#[derive(Clone, Default)]
pub struct Flags {
    pub cancel: Arc<AtomicBool>,
    pub cancel_id: Arc<AtomicU64>,
    pub stop_seq: Arc<AtomicU64>,
    pub busy_since: Arc<AtomicU64>,
    pub epoch: Arc<OnceInstant>,
    pub feed: Arc<Feed>,
}

#[derive(Default)]
struct FeedState {
    id: u64,
    lines: std::collections::VecDeque<String>,
    ended: bool,
}

#[derive(Default)]
pub struct Feed {
    state: Mutex<FeedState>,
    ready: std::sync::Condvar,
}

impl Feed {
    pub fn open(&self, id: u64) {
        let mut s = self.state.lock().unwrap();
        s.id = id;
        s.lines.clear();
        s.ended = false;
    }

    pub fn push(&self, id: u64, lines: Vec<String>) {
        let mut s = self.state.lock().unwrap();
        if s.id == id && !s.ended {
            s.lines.extend(lines.into_iter().filter(|l| !l.trim().is_empty()));
            self.ready.notify_all();
        }
    }

    pub fn end(&self, id: u64) {
        let mut s = self.state.lock().unwrap();
        if s.id == id {
            s.ended = true;
            self.ready.notify_all();
        }
    }

    pub fn close(&self) {
        let mut s = self.state.lock().unwrap();
        s.lines.clear();
        s.ended = true;
        self.ready.notify_all();
    }

    pub fn next(&self, id: u64, cancel: &AtomicBool, idle: std::time::Duration) -> Option<String> {
        let deadline = Instant::now() + idle;
        let mut s = self.state.lock().unwrap();
        loop {
            if s.id != id || cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(line) = s.lines.pop_front() {
                return Some(line);
            }
            if s.ended {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            s = self.ready.wait_timeout(s, (deadline - now).min(std::time::Duration::from_millis(50))).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

#[derive(Default)]
pub struct OnceInstant(std::sync::OnceLock<Instant>);

impl OnceInstant {
    pub fn ms(&self) -> u64 {
        self.0.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
    }
}

impl Flags {
    pub fn busy(&self) {
        self.busy_since.store(self.epoch.ms(), Ordering::Relaxed);
    }

    pub fn idle(&self) {
        self.busy_since.store(0, Ordering::Relaxed);
    }

    fn busy_ms(&self) -> u64 {
        match self.busy_since.load(Ordering::Relaxed) {
            0 => 0,
            t => self.epoch.ms().saturating_sub(t),
        }
    }
}

fn rss() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/statm")
            && let Some(p) = s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()) {
                return p * 4096;
            }
    }
    0
}

pub fn threads(requested: i32) -> i32 {
    if requested > 0 {
        return requested;
    }
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    ((n / 2).max(1) as i32).min(8)
}

fn reader(tx: crossbeam_channel::Sender<Msg>, out: Out, flags: Flags) {
    let mut stdin = std::io::stdin().lock();
    loop {
        match read_packet(&mut stdin) {
            Ok(Packet::Json(b)) => {
                let Ok(op) = serde_json::from_slice::<ToEngine>(&b) else {
                    out.error("unreadable request", false);
                    continue;
                };
                match &op {
                    ToEngine::Ping { seq } => {
                        out.send(&FromEngine::Pong { seq: *seq, rss: rss(), busy_ms: flags.busy_ms() });
                        continue;
                    }
                    ToEngine::LlmCancel { id } | ToEngine::AsrCancel { id } => {
                        flags.cancel_id.store(*id, Ordering::Relaxed);
                        flags.cancel.store(true, Ordering::Relaxed);
                    }
                    ToEngine::TtsStop => {
                        flags.stop_seq.fetch_add(1, Ordering::AcqRel);
                        flags.cancel.store(true, Ordering::Relaxed);
                        flags.feed.close();
                    }
                    ToEngine::TtsSpeak { id, open: true, .. } => flags.feed.open(*id),
                    ToEngine::TtsMore { id, lines } => {
                        flags.feed.push(*id, lines.clone());
                        continue;
                    }
                    ToEngine::TtsEnd { id } => {
                        flags.feed.end(*id);
                        continue;
                    }
                    ToEngine::Shutdown => std::process::exit(0),
                    _ => {}
                }
                let msg = if matches!(op, ToEngine::TtsSpeak { .. }) { Msg::Speak(op, flags.stop_seq.load(Ordering::Acquire)) } else { Msg::Op(op) };
                if tx.send(msg).is_err() {
                    break;
                }
            }
            Ok(Packet::Bytes(b)) => {
                if tx.send(Msg::Bytes(b)).is_err() {
                    break;
                }
            }
            Ok(Packet::Pcm(s)) => {
                if tx.try_send(Msg::Pcm(s)).is_err() {
                    out.error("audio queue full", false);
                }
            }
            Err(_) => std::process::exit(0),
        }
    }
}

fn arg(name: &str) -> Vec<String> {
    let prefix = format!("--{name}=");
    std::env::args().filter_map(|a| a.strip_prefix(&prefix).map(str::to_owned)).collect()
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("probe") {
        let devices = llama::devices();
        let mut w = std::io::stdout().lock();
        let _ = serde_json::to_writer(&mut w, &devices);
        let _ = writeln!(w);
        std::process::exit(0);
    }
    if std::env::args().nth(1).as_deref() == Some("render") {
        let a: Vec<String> = std::env::args().skip(2).collect();
        if a.len() != 3 && a.len() != 4 {
            eprintln!("usage: setu-engine render CONFIG.json TEXT OUT.wav [RATE]");
            std::process::exit(2);
        }
        let rate = a.get(3).and_then(|r| r.parse::<f32>().ok()).filter(|r| (0.5..=2.0).contains(r)).unwrap_or(1.0);
        if let Err(e) = tts::render(&a[0], &a[1], &a[2], rate) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if std::env::args().nth(1).as_deref() == Some("transcribe") {
        let a: Vec<String> = std::env::args().skip(2).collect();
        if a.len() < 2 {
            eprintln!("usage: setu-engine transcribe CONFIG.json FILE.wav...");
            std::process::exit(2);
        }
        if let Err(e) = asr::transcribe(&a[0], &a[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    let kind = std::env::args().nth(1).as_deref().and_then(EngineKind::parse);
    let Some(kind) = kind else {
        eprintln!("usage: setu-engine asr|llm|tts|vision|probe [--allow=PATH]... [--mem-limit=BYTES] [--gpu]");
        std::process::exit(2);
    };
    let gpu = std::env::args().any(|a| a == "--gpu");
    let mem = if gpu { None } else { arg("mem-limit").first().and_then(|v| v.parse::<u64>().ok()) };
    llama::set_gpu(gpu);
    if kind == EngineKind::Llm || kind == EngineKind::Vision || (kind == EngineKind::Tts && gpu) {
        llama::init();
    }
    if kind == EngineKind::Vision {
        vision::load_library();
    }
    sandbox::apply(kind, &arg("allow"), mem, gpu);
    let out = Out { w: Arc::new(Mutex::new(BufWriter::new(std::io::stdout()))) };
    let flags = Flags::default();
    let (tx, rx) = bounded::<Msg>(256);
    {
        let out = out.clone();
        let flags = flags.clone();
        std::thread::Builder::new().name("engine-reader".into()).spawn(move || reader(tx, out, flags)).expect("reader thread");
    }
    out.send(&FromEngine::Ready { pid: std::process::id() });
    let _ = std::io::stdout().flush();
    run(kind, rx, out, flags);
}

fn run(kind: EngineKind, rx: Receiver<Msg>, out: Out, flags: Flags) {
    match kind {
        EngineKind::Asr => asr::Server::new(out, flags).run(rx),
        EngineKind::Llm => llm::Server::new(out, flags).run(rx),
        EngineKind::Tts => tts::Server::new(out, flags).run(rx),
        EngineKind::Vision => vision::Server::new(out, flags).run(rx),
    }
}
