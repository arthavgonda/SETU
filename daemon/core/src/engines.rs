use anyhow::{Result, anyhow};
use crossbeam_channel::{Sender, bounded};
use serde::Serialize;
use setu_common::engine::{EngineKind, FromEngine, Packet, ToEngine, read_packet, write_bytes, write_pcm};
use std::collections::VecDeque;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use crate::proc::Quiet;

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Partial { id: u64, text: String },
    LlmPartial { id: u64, text: String },
    TtsLine { id: u64, index: u32 },
    TtsLevel { level: f32 },
    TtsDone { id: u64, first_audio_ms: u64 },
    Status { kind: EngineKind, status: &'static str, message: Option<String>, ram: u64 },
    Degraded { kind: EngineKind },
    Recovered,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    Loaded,
    Final(u64),
    Result(u64),
    Prefixed(String),
    Bench,
    Unloaded,
    Vision(u64),
}

struct Running {
    child: Child,
    tx: Sender<Packet2>,
    generation: u64,
}

enum Packet2 {
    Json(Vec<u8>),
    Pcm(Vec<f32>),
    Bytes(Vec<u8>),
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    loaded: Option<String>,
    load_msg: Option<Vec<u8>>,
    failures: VecDeque<Instant>,
    degraded: bool,
    ram: u64,
    restarting: bool,
    device: String,
}

pub struct Engine {
    pub kind: EngineKind,
    exe: PathBuf,
    allow: Vec<String>,
    state: Mutex<State>,
    pending: Mutex<Vec<(Expect, oneshot::Sender<Result<FromEngine>>)>>,
    events: mpsc::Sender<EngineEvent>,
    generation: AtomicU64,
    last_used: Mutex<Instant>,
    busy: tokio::sync::Mutex<()>,
    seq: AtomicU64,
    pong: Mutex<(u64, Instant, u64)>,
    mem_limit: AtomicU64,
    pub dropped_pcm: AtomicU64,
    gpu: std::sync::atomic::AtomicBool,
}

fn encode<T: Serialize>(m: &T) -> Vec<u8> {
    serde_json::to_vec(m).unwrap_or_default()
}

impl Engine {
    pub fn new(kind: EngineKind, exe: PathBuf, allow: Vec<String>, events: mpsc::Sender<EngineEvent>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            exe,
            allow,
            state: Mutex::new(State::default()),
            pending: Mutex::new(Vec::new()),
            events,
            generation: AtomicU64::new(0),
            last_used: Mutex::new(Instant::now()),
            busy: tokio::sync::Mutex::new(()),
            seq: AtomicU64::new(0),
            pong: Mutex::new((0, Instant::now(), 0)),
            mem_limit: AtomicU64::new(0),
            dropped_pcm: AtomicU64::new(0),
            gpu: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn set_mem_limit(&self, bytes: u64) {
        self.mem_limit.store(bytes, Ordering::Relaxed);
    }

    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }

    pub fn idle_for(&self) -> Duration {
        self.last_used.lock().unwrap().elapsed()
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().unwrap().running.is_some()
    }

    pub fn is_loaded(&self, key: &str) -> bool {
        let s = self.state.lock().unwrap();
        s.running.is_some() && s.loaded.as_deref() == Some(key)
    }

    pub fn degraded(&self) -> bool {
        self.state.lock().unwrap().degraded
    }

    pub fn ram(&self) -> u64 {
        self.state.lock().unwrap().ram
    }

    pub fn device(&self) -> String {
        let s = self.state.lock().unwrap();
        if s.loaded.is_some() { s.device.clone() } else { String::new() }
    }

    pub fn set_gpu(&self, on: bool) {
        if self.gpu.swap(on, Ordering::Relaxed) != on {
            self.stop();
        }
    }

    pub fn queue_depth(&self) -> (usize, usize) {
        self.state.lock().unwrap().running.as_ref().map(|r| (r.tx.len(), r.tx.capacity().unwrap_or(0))).unwrap_or((0, 0))
    }

    fn spawn(self: &Arc<Self>) -> Result<()> {
        let mut cmd = Command::new(&self.exe);
        cmd.quiet();
        cmd.arg(self.kind.as_str());
        for a in &self.allow {
            cmd.arg(format!("--allow={a}"));
        }
        let lim = self.mem_limit.load(Ordering::Relaxed);
        if self.gpu.load(Ordering::Relaxed) {
            cmd.arg("--gpu");
        } else if lim > 0 {
            cmd.arg(format!("--mem-limit={lim}"));
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| anyhow!("could not start {}: {e}", self.exe.display()))?;
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("engine stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("engine stdout"))?;
        let stderr = child.stderr.take();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = bounded::<Packet2>(if self.kind == EngineKind::Asr { 400 } else { 32 });
        let name = self.kind.as_str();
        std::thread::Builder::new().name(format!("{name}-writer")).spawn(move || {
            let mut w = BufWriter::new(stdin);
            while let Ok(p) = rx.recv() {
                let r = match p {
                    Packet2::Json(b) => {
                        use std::io::Write;
                        let len = (b.len() + 1) as u32;
                        w.write_all(&len.to_be_bytes()).and_then(|_| w.write_all(&[0])).and_then(|_| w.write_all(&b)).and_then(|_| w.flush()).map_err(|e| e.into())
                    }
                    Packet2::Pcm(s) => write_pcm(&mut w, &s),
                    Packet2::Bytes(b) => write_bytes(&mut w, &b),
                };
                if r.is_err() {
                    break;
                }
            }
        })?;
        let me = self.clone();
        std::thread::Builder::new().name(format!("{name}-reader")).spawn(move || {
            let mut r = std::io::BufReader::new(stdout);
            loop {
                match read_packet(&mut r) {
                    Ok(Packet::Json(b)) => match serde_json::from_slice::<FromEngine>(&b) {
                        Ok(ev) => me.dispatch(ev),
                        Err(e) => log::warn!("{name} engine sent an unreadable message: {e}"),
                    },
                    Ok(Packet::Pcm(_)) | Ok(Packet::Bytes(_)) => {}
                    Err(_) => break,
                }
            }
            me.exited(generation);
        })?;
        if let Some(se) = stderr {
            std::thread::Builder::new().name(format!("{name}-stderr")).spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(se).lines().map_while(Result::ok).take(200) {
                    if !line.trim().is_empty() && !line.chars().all(|c| c == '.') {
                        log::debug!("{name} engine: {}", line.chars().take(300).collect::<String>());
                    }
                }
            })?;
        }
        let mut s = self.state.lock().unwrap();
        s.running = Some(Running { child, tx, generation });
        s.loaded = None;
        drop(s);
        *self.pong.lock().unwrap() = (0, Instant::now(), 0);
        log::info!("{name} engine started with pid {pid}");
        self.clone().heartbeat(generation);
        Ok(())
    }

    fn heartbeat(self: Arc<Self>, generation: u64) {
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_secs(1));
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                iv.tick().await;
                if self.generation.load(Ordering::Relaxed) != generation || !self.is_running() {
                    return;
                }
                let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
                self.send(&ToEngine::Ping { seq });
                let (last_seq, at, busy) = *self.pong.lock().unwrap();
                let silent = at.elapsed() > Duration::from_secs(5);
                let wedged = busy > 120_000;
                if (silent && last_seq + 4 < seq) || wedged {
                    log::warn!("{} engine stopped answering (busy {busy} ms); restarting it", self.kind.as_str());
                    self.kill();
                    return;
                }
            }
        });
    }

    fn dispatch(&self, ev: FromEngine) {
        match &ev {
            FromEngine::Pong { seq, rss, busy_ms } => {
                *self.pong.lock().unwrap() = (*seq, Instant::now(), *busy_ms);
                self.state.lock().unwrap().ram = *rss;
                return;
            }
            FromEngine::Ready { .. } => return,
            FromEngine::AsrPartial { id, text } => {
                let _ = self.events.try_send(EngineEvent::Partial { id: *id, text: text.clone() });
                return;
            }
            FromEngine::TtsLine { id, index } => {
                let _ = self.events.try_send(EngineEvent::TtsLine { id: *id, index: *index });
                return;
            }
            FromEngine::TtsLevel { level, .. } => {
                let _ = self.events.try_send(EngineEvent::TtsLevel { level: *level });
                return;
            }
            FromEngine::LlmPartial { id, text } => {
                let _ = self.events.try_send(EngineEvent::LlmPartial { id: *id, text: text.clone() });
                return;
            }
            FromEngine::TtsDone { id, first_audio_ms, .. } => {
                let _ = self.events.try_send(EngineEvent::TtsDone { id: *id, first_audio_ms: *first_audio_ms });
                return;
            }
            _ => {}
        }
        let expect = match &ev {
            FromEngine::Loaded { .. } => Some(Expect::Loaded),
            FromEngine::Unloaded => Some(Expect::Unloaded),
            FromEngine::AsrFinal { id, .. } => Some(Expect::Final(*id)),
            FromEngine::LlmResult { id, .. } => Some(Expect::Result(*id)),
            FromEngine::LlmPrefixed { key, .. } => Some(Expect::Prefixed(key.clone())),
            FromEngine::BenchResult { .. } => Some(Expect::Bench),
            FromEngine::VisionResult { id, .. } => Some(Expect::Vision(*id)),
            _ => None,
        };
        let mut p = self.pending.lock().unwrap();
        match expect {
            Some(x) => {
                if let Some(i) = p.iter().position(|(e, _)| *e == x) {
                    let (_, tx) = p.remove(i);
                    let _ = tx.send(Ok(ev));
                }
            }
            None => {
                if let FromEngine::Error { message, fatal } = &ev {
                    log::warn!("{} engine error: {message}", self.kind.as_str());
                    if !p.is_empty() {
                        let idx = if *fatal { p.iter().position(|(e, _)| *e == Expect::Loaded).unwrap_or(0) } else { 0 };
                        let (_, tx) = p.remove(idx);
                        let _ = tx.send(Err(anyhow!(message.clone())));
                    }
                }
            }
        }
    }

    fn exited(self: &Arc<Self>, generation: u64) {
        let mut s = self.state.lock().unwrap();
        let ours = s.running.as_ref().is_some_and(|r| r.generation == generation);
        if !ours {
            return;
        }
        let mut r = s.running.take().unwrap();
        let status = r.child.try_wait().ok().flatten();
        let _ = r.child.kill();
        let _ = r.child.wait();
        s.loaded = None;
        s.ram = 0;
        let intentional = s.restarting;
        s.restarting = false;
        for (_, tx) in self.pending.lock().unwrap().drain(..) {
            let _ = tx.send(Err(anyhow!("the engine stopped")));
        }
        if intentional {
            return;
        }
        log::warn!("{} engine exited unexpectedly ({status:?})", self.kind.as_str());
        let now = Instant::now();
        s.failures.push_back(now);
        while s.failures.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(300)) {
            s.failures.pop_front();
        }
        let n = s.failures.len();
        let had_load = s.load_msg.clone();
        if n >= 5 && !s.degraded {
            s.degraded = true;
            let _ = self.events.try_send(EngineEvent::Degraded { kind: self.kind });
        }
        drop(s);
        let _ = self.events.try_send(EngineEvent::Status { kind: self.kind, status: "restarting", message: None, ram: 0 });
        if had_load.is_none() {
            return;
        }
        let delay = Duration::from_millis((500u64 << (n.saturating_sub(1)).min(6)).min(30_000));
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if me.is_running() {
                return;
            }
            if let Some(msg) = had_load
                && me.spawn().is_ok() {
                    me.raw(msg);
                    let (tx, rx) = oneshot::channel();
                    me.pending.lock().unwrap().push((Expect::Loaded, tx));
                    if let Ok(Ok(Ok(FromEngine::Loaded { ram, .. }))) = tokio::time::timeout(Duration::from_secs(90), rx).await {
                        let key = me.state.lock().unwrap().load_msg.as_ref().map(|m| String::from_utf8_lossy(m).into_owned());
                        me.state.lock().unwrap().loaded = key;
                        let _ = me.events.try_send(EngineEvent::Status { kind: me.kind, status: "loaded", message: None, ram });
                        let mut s = me.state.lock().unwrap();
                        if s.degraded && s.failures.len() < 5 {
                            s.degraded = false;
                            let _ = me.events.try_send(EngineEvent::Recovered);
                        }
                    }
                }
        });
    }

    fn raw(&self, b: Vec<u8>) {
        if let Some(r) = self.state.lock().unwrap().running.as_ref() {
            let _ = r.tx.try_send(Packet2::Json(b));
        }
    }

    pub fn send(&self, m: &ToEngine) {
        self.raw(encode(m));
    }

    pub fn bytes(&self, b: Vec<u8>) -> bool {
        self.state.lock().unwrap().running.as_ref().is_some_and(|r| r.tx.try_send(Packet2::Bytes(b)).is_ok())
    }

    pub fn gpu(&self) -> bool {
        self.gpu.load(Ordering::Relaxed)
    }

    pub fn pcm(&self, samples: Vec<f32>) {
        let s = self.state.lock().unwrap();
        if let Some(r) = s.running.as_ref()
            && r.tx.try_send(Packet2::Pcm(samples)).is_err() {
                self.dropped_pcm.fetch_add(1, Ordering::Relaxed);
            }
    }

    pub fn kill(&self) {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.running.as_mut() {
            let _ = r.child.kill();
        }
    }

    pub fn stop(&self) {
        let mut s = self.state.lock().unwrap();
        s.restarting = true;
        s.load_msg = None;
        s.loaded = None;
        if let Some(r) = s.running.as_ref() {
            let _ = r.tx.try_send(Packet2::Json(encode(&ToEngine::Shutdown)));
        }
        if let Some(mut r) = s.running.take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = r.child.kill();
                let _ = r.child.wait();
            });
        }
        s.ram = 0;
        drop(s);
        let _ = self.events.try_send(EngineEvent::Status { kind: self.kind, status: "unloaded", message: None, ram: 0 });
    }

    pub async fn request(&self, msg: &ToEngine, expect: Expect, timeout: Duration) -> Result<FromEngine> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().push((expect.clone(), tx));
        self.send(msg);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(anyhow!("the engine stopped")),
            Err(_) => {
                self.pending.lock().unwrap().retain(|(e, _)| *e != expect);
                Err(anyhow!("the engine did not answer in time"))
            }
        }
    }

    pub async fn ensure(self: &Arc<Self>, load: ToEngine, key: &str) -> Result<u64> {
        let _g = self.busy.lock().await;
        self.touch();
        if self.is_loaded(key) {
            return Ok(self.ram());
        }
        if !self.is_running() {
            self.spawn()?;
        }
        let msg = encode(&load);
        self.state.lock().unwrap().load_msg = Some(msg.clone());
        let _ = self.events.try_send(EngineEvent::Status { kind: self.kind, status: "loading", message: None, ram: 0 });
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().push((Expect::Loaded, tx));
        self.raw(msg);
        let r = tokio::time::timeout(Duration::from_secs(120), rx).await;
        match r {
            Ok(Ok(Ok(FromEngine::Loaded { ram, ms, device }))) => {
                let mut s = self.state.lock().unwrap();
                s.loaded = Some(key.to_owned());
                s.ram = ram;
                s.device = device.clone();
                drop(s);
                if device.is_empty() {
                    log::info!("{} engine loaded in {ms} ms", self.kind.as_str());
                } else {
                    log::info!("{} engine loaded in {ms} ms on {device}", self.kind.as_str());
                }
                let _ = self.events.try_send(EngineEvent::Status { kind: self.kind, status: "loaded", message: None, ram });
                Ok(ram)
            }
            other => {
                let message = match other {
                    Ok(Ok(Err(e))) => e.to_string(),
                    Err(_) => "loading took too long".to_owned(),
                    _ => "the engine stopped while loading".to_owned(),
                };
                self.state.lock().unwrap().load_msg = None;
                self.stop();
                let _ = self.events.try_send(EngineEvent::Status { kind: self.kind, status: "error", message: Some(message.clone()), ram: 0 });
                Err(anyhow!(message))
            }
        }
    }

    pub async fn exclusive(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.busy.lock().await
    }
}

pub struct Engines {
    pub asr: Arc<Engine>,
    pub llm: Arc<Engine>,
    pub tts: Arc<Engine>,
    pub vision: Arc<Engine>,
}

impl Engines {
    pub fn new(exe: PathBuf, allow: Vec<String>, events: mpsc::Sender<EngineEvent>) -> Self {
        Self {
            asr: Engine::new(EngineKind::Asr, exe.clone(), allow.clone(), events.clone()),
            llm: Engine::new(EngineKind::Llm, exe.clone(), allow.clone(), events.clone()),
            tts: Engine::new(EngineKind::Tts, exe.clone(), allow.clone(), events.clone()),
            vision: Engine::new(EngineKind::Vision, exe, allow, events),
        }
    }

    pub fn get(&self, k: EngineKind) -> &Arc<Engine> {
        match k {
            EngineKind::Asr => &self.asr,
            EngineKind::Llm => &self.llm,
            EngineKind::Tts => &self.tts,
            EngineKind::Vision => &self.vision,
        }
    }

    pub fn all(&self) -> [&Arc<Engine>; 4] {
        [&self.asr, &self.llm, &self.tts, &self.vision]
    }

    pub fn stop_all(&self) {
        for e in self.all() {
            e.stop();
        }
    }
}
