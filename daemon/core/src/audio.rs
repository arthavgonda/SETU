use crossbeam_channel::{Receiver, Sender, unbounded};
use setu_audio::ring::{self, Consumer};
use setu_audio::{Capture, rms};
use setu_sherpa::bpe::SentencePiece;
use setu_sherpa::{Denoiser, Kws, KwsOptions, KwsStream, OnlineAsr, OnlineStream, TransducerPaths, Vad, VadOptions};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::thread::Thread;
use std::time::{Duration, Instant};

pub const SR: usize = 16_000;
const CHUNK: usize = 1_600;
const HISTORY: usize = SR * 4;
const RING: usize = SR * 5;
const NO_SPEECH: Duration = Duration::from_secs(8);
const MAX_UTTERANCE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Keyword {
    pub tag: String,
    pub phrase: String,
    pub boost: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone)]
pub enum Mode {
    Off,
    Spot(Vec<Keyword>),
    Capture { id: u64, preroll: f32 },
    Await { id: u64 },
    Enroll,
    Converse,
}

pub enum AudioCmd {
    Mode(Mode),
    Device(String),
    Gain(f32),
    AutoGain(bool),
    Denoise(bool),
    EchoCancel(bool),
    EndSilence(f32),
    Wake { sensitivity: f32, long_window: bool },
    Verify(Option<Vec<f32>>),
    Listener(Option<TransducerPaths>),
    Learned(Vec<String>),
    Learn(bool),
    DevicesChanged,
    Shutdown,
}

#[derive(Debug)]
pub enum AudioEvent {
    Keyword { tag: String, confidence: f32, loud: bool, strict: bool },
    Heard { text: String },
    Utterance { samples: Vec<f32> },
    SpeechEnd { id: u64 },
    NoSpeech { id: u64 },
    Onset { id: u64 },
    EnrollSample { samples: Vec<f32> },
    Missed { samples: Vec<f32> },
    DeviceLost(String),
    DeviceOk,
    Overrun(u64),
    Bandwidth { narrow: bool },
}

pub struct Models {
    pub kws: TransducerPaths,
    pub bpe: String,
    pub vad: String,
    pub denoise: String,
}

#[derive(Clone)]
pub struct AudioCtl {
    tx: Sender<AudioCmd>,
    thread: Thread,
    pub level: Arc<AtomicU32>,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub capture_tid: Arc<AtomicI64>,
    pub open: Arc<AtomicBool>,
    pub narrow: Arc<AtomicBool>,
}

impl AudioCtl {
    pub fn send(&self, c: AudioCmd) {
        let _ = self.tx.send(c);
        self.thread.unpark();
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn narrowband(&self) -> bool {
        self.narrow.load(Ordering::Relaxed)
    }
}

struct Agc {
    on: bool,
    env: f32,
    gain: f32,
}

impl Agc {
    const TARGET: f32 = 0.09;
    const MAX: f32 = 6.0;

    fn new() -> Self {
        Self { on: true, env: Self::TARGET, gain: 1.0 }
    }

    fn apply(&mut self, frame: &mut [f32], fixed: f32) {
        let r = rms(frame) * fixed;
        if self.on {
            let k = if r > self.env { 0.35 } else { 0.0025 };
            self.env += (r - self.env) * k;
            let want = (Self::TARGET / self.env.max(0.003)).clamp(1.0, Self::MAX);
            let rate = if want < self.gain { 0.5 } else { 0.03 };
            self.gain += (want - self.gain) * rate;
        } else {
            self.gain = 1.0;
        }
        let g = fixed * self.gain;
        for s in frame.iter_mut() {
            let v = *s * g;
            *s = if v.abs() > 0.95 { v.signum() * (0.95 + (v.abs() - 0.95).tanh() * 0.05) } else { v };
        }
    }
}

struct Band {
    st: [[f32; 4]; 4],
    avg: f32,
    seen: u32,
    narrow: bool,
}

impl Band {
    const B: [f32; 3] = [0.186_693_63, -0.373_387_25, 0.186_693_63];
    const A: [f32; 2] = [0.462_936_27, 0.209_710_77];

    fn new() -> Self {
        Self { st: [[0.0; 4]; 4], avg: -30.0, seen: 0, narrow: false }
    }

    fn feed(&mut self, chunk: &[f32]) -> Option<bool> {
        let (mut hi, mut all) = (0f64, 0f64);
        for &x in chunk {
            let mut y = x;
            for s in self.st.iter_mut() {
                let o = Self::B[0] * y + Self::B[1] * s[0] + Self::B[2] * s[1] - Self::A[0] * s[2] - Self::A[1] * s[3];
                s[1] = s[0];
                s[0] = y;
                s[3] = s[2];
                s[2] = o;
                y = o;
            }
            hi += (y * y) as f64;
            all += (x * x) as f64;
        }
        if all / (chunk.len().max(1) as f64) < 1e-4 {
            return None;
        }
        let db = (10.0 * ((hi + 1e-12) / (all + 1e-12)).log10()) as f32;
        self.seen = self.seen.saturating_add(1);
        self.avg += (db - self.avg) * if self.seen < 10 { 0.3 } else { 0.08 };
        if self.seen < 12 {
            return None;
        }
        let narrow = if self.narrow { self.avg < -45.0 } else { self.avg < -52.0 };
        (narrow != self.narrow).then(|| {
            self.narrow = narrow;
            narrow
        })
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

pub type PcmSink = Arc<dyn Fn(Vec<f32>) + Send + Sync>;

struct Spotter {
    kws: Kws,
    stream: KwsStream,
    sp: SentencePiece,
    fed: u64,
}

struct Listener {
    asr: OnlineAsr,
    stream: Option<OnlineStream>,
    audio: Vec<f32>,
    decided: bool,
    quiet: f32,
    cooldown: f32,
}

const LISTEN_WINDOW: usize = SR * 5 / 2;
const LISTEN_PREROLL: usize = SR * 2 / 5;

struct Worker {
    models: Models,
    rx: Receiver<AudioCmd>,
    events: Sender<AudioEvent>,
    pcm: PcmSink,
    level: Arc<AtomicU32>,
    tid: Arc<AtomicI64>,
    open: Arc<AtomicBool>,
    narrow: Arc<AtomicBool>,
    agc: Agc,
    band: Band,
    mode: Mode,
    device: String,
    gain: f32,
    denoise_on: bool,
    echo_on: bool,
    #[cfg(target_os = "linux")]
    echo: Option<setu_audio::echo::Canceller>,
    end_silence: f32,
    sensitivity: f32,
    long_window: bool,
    centroid: Option<Vec<f32>>,
    capture: Option<Capture>,
    ring: Option<Consumer>,
    failed: Arc<AtomicBool>,
    retry_at: Option<Instant>,
    history: VecDeque<f32>,
    spotter: Option<Spotter>,
    vad: Option<Vad>,
    denoiser: Option<Denoiser>,
    speech_seen: bool,
    silence: f32,
    started: Instant,
    speech_run: f32,
    dropped_seen: u64,
    lost_reported: bool,
    learn: bool,
    hit_in_segment: bool,
    listener_paths: Option<TransducerPaths>,
    listener: Option<Listener>,
    learned: Vec<String>,
    listen_vad: Option<Vad>,
    talk_vad: Option<Vad>,
}

pub fn keyword_line(sp: &SentencePiece, k: &Keyword) -> Option<String> {
    let text = crate::names::spoken(&k.phrase);
    if text.is_empty() {
        return None;
    }
    let toks = sp.encode(&text);
    if toks.is_empty() {
        return None;
    }
    Some(format!("{} :{:.2} #{:.3} @{}", toks.join(" "), k.boost, k.threshold, k.tag))
}


pub fn threshold_for(sensitivity: f32) -> f32 {
    (0.25 - 0.3 * sensitivity.clamp(0.0, 1.0)).clamp(0.05, 0.25)
}

pub fn boost_for(sensitivity: f32) -> f32 {
    3.0 + sensitivity.clamp(0.0, 0.5)
}


pub fn start(models: Models, events: Sender<AudioEvent>, pcm: PcmSink) -> AudioCtl {
    let (tx, rx) = unbounded();
    let level = Arc::new(AtomicU32::new(0));
    let tid = Arc::new(AtomicI64::new(0));
    let open = Arc::new(AtomicBool::new(false));
    let narrow = Arc::new(AtomicBool::new(false));
    let (l2, t2, o2, n2) = (level.clone(), tid.clone(), open.clone(), narrow.clone());
    let handle = std::thread::Builder::new()
        .name("wake-vad".into())
        .spawn(move || {
            #[cfg(target_os = "linux")]
            unsafe {
                let me = libc::syscall(libc::SYS_gettid) as libc::id_t;
                libc::setpriority(libc::PRIO_PROCESS, me, -5);
            }
            let mut w = Worker {
                models,
                rx,
                events,
                pcm,
                level: l2,
                tid: t2,
                open: o2,
                narrow: n2,
                agc: Agc::new(),
                band: Band::new(),
                mode: Mode::Off,
                device: String::new(),
                gain: 1.0,
                denoise_on: true,
                echo_on: false,
                #[cfg(target_os = "linux")]
                echo: None,
                end_silence: 1.0,
                sensitivity: 0.5,
                long_window: false,
                centroid: None,
                capture: None,
                ring: None,
                failed: Arc::new(AtomicBool::new(false)),
                retry_at: None,
                history: VecDeque::with_capacity(HISTORY),
                spotter: None,
                vad: None,
                denoiser: None,
                speech_seen: false,
                silence: 0.0,
                started: Instant::now(),
                speech_run: 0.0,
                dropped_seen: 0,
                lost_reported: false,
                learn: false,
                hit_in_segment: false,
                listener_paths: None,
                listener: None,
                learned: Vec::new(),
                listen_vad: None,
                talk_vad: None,
            };
            w.run();
        })
        .expect("wake thread");
    AudioCtl { tx, thread: handle.thread().clone(), level, capture_tid: tid, open, narrow }
}

impl Worker {
    fn run(&mut self) {
        let mut buf = Vec::with_capacity(CHUNK * 4);
        let mut chunk: Vec<f32> = Vec::with_capacity(CHUNK);
        loop {
            while let Ok(c) = self.rx.try_recv() {
                if !self.command(c) {
                    self.close();
                    return;
                }
            }
            if matches!(self.mode, Mode::Off) {
                self.close();
                match self.rx.recv() {
                    Ok(c) => {
                        if !self.command(c) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
                continue;
            }
            if self.failed.swap(false, Ordering::Relaxed) {
                self.close();
                self.retry_at = Some(Instant::now() + Duration::from_secs(2));
                if !self.lost_reported {
                    self.lost_reported = true;
                    let _ = self.events.send(AudioEvent::DeviceLost("No microphone found. Plug one in or choose a device.".into()));
                }
            }
            if self.capture.is_none() {
                if self.retry_at.is_some_and(|t| Instant::now() < t) {
                    let wait = self.retry_at.unwrap().saturating_duration_since(Instant::now());
                    if let Ok(c) = self.rx.recv_timeout(wait)
                        && !self.command(c) {
                            return;
                        }
                    continue;
                }
                if !self.open_capture() {
                    continue;
                }
            }
            buf.clear();
            let got = self.ring.as_mut().map(|r| r.pop(&mut buf, CHUNK * 4)).unwrap_or(0);
            if got == 0 {
                std::thread::park_timeout(Duration::from_millis(500));
                continue;
            }
            if let Some(r) = self.ring.as_ref() {
                let d = r.dropped();
                if d > self.dropped_seen + SR as u64 / 10 {
                    self.dropped_seen = d;
                    let _ = self.events.send(AudioEvent::Overrun(d));
                }
            }
            let g = self.gain;
            for frame in buf.chunks_mut(160) {
                self.agc.apply(frame, g);
            }
            for &s in &buf {
                chunk.push(s);
                if chunk.len() == CHUNK {
                    let c = std::mem::replace(&mut chunk, Vec::with_capacity(CHUNK));
                    self.process(c);
                }
            }
        }
    }

    fn command(&mut self, c: AudioCmd) -> bool {
        match c {
            AudioCmd::Mode(m) => self.enter(m),
            AudioCmd::Device(d) => {
                if d != self.device {
                    self.device = d;
                    self.close();
                    self.retry_at = None;
                }
            }
            AudioCmd::Gain(g) => self.gain = g.clamp(0.25, 8.0),
            AudioCmd::AutoGain(on) => self.agc.on = on,
            AudioCmd::Denoise(on) => self.denoise_on = on,
            AudioCmd::EchoCancel(on) => {
                if on != self.echo_on {
                    self.echo_on = on;
                    if self.capture.is_some() {
                        self.close();
                        self.open_capture();
                    }
                }
            }
            AudioCmd::EndSilence(s) => self.end_silence = s,
            AudioCmd::Wake { sensitivity, long_window } => {
                let rebuild = long_window != self.long_window;
                self.sensitivity = sensitivity;
                self.long_window = long_window;
                if rebuild {
                    self.spotter = None;
                }
                if let Mode::Spot(k) = self.mode.clone() {
                    self.enter(Mode::Spot(k));
                }
            }
            AudioCmd::Learn(on) => {
                self.learn = on;
                self.hit_in_segment = false;
                if on {
                    self.ensure_vad();
                    if let Some(v) = &self.vad {
                        v.reset();
                    }
                }
            }
            AudioCmd::Verify(c) => self.centroid = c,
            AudioCmd::Listener(p) => {
                let changed = p.as_ref().map(|x| &x.encoder) != self.listener_paths.as_ref().map(|x| &x.encoder);
                self.listener_paths = p;
                if changed {
                    self.listener = None;
                }
            }
            AudioCmd::Learned(v) => self.learned = v,
            AudioCmd::DevicesChanged => {
                if self.capture.is_none() {
                    self.retry_at = None;
                }
            }
            AudioCmd::Shutdown => return false,
        }
        true
    }

    fn close(&mut self) {
        if self.capture.take().is_some() {
            self.open.store(false, Ordering::Relaxed);
        }
        self.ring = None;
        self.level.store(0, Ordering::Relaxed);
    }

    fn open_capture(&mut self) -> bool {
        let (mut prod, cons) = ring::ring(RING);
        let level = self.level.clone();
        let me = std::thread::current();
        let tid = self.tid.clone();
        let failed = self.failed.clone();
        let wake = me.clone();
        let sink = Box::new(move |s: &[f32]| {
            if tid.load(Ordering::Relaxed) == 0 {
                #[cfg(target_os = "linux")]
                tid.store(unsafe { libc::syscall(libc::SYS_gettid) } as i64, Ordering::Relaxed);
            }
            prod.push(s);
            level.store(rms(s).to_bits(), Ordering::Relaxed);
            me.unpark();
        });
        let on_err = Box::new(move |e: String| {
            log::warn!("microphone stream failed: {e}");
            failed.store(true, Ordering::Relaxed);
            wake.unpark();
        });
        let device = self.echo_source().unwrap_or_else(|| self.device.clone());
        let cancelled = device != self.device;
        let started = match Capture::start(&device, sink, on_err) {
            Err(e) if cancelled => {
                log::warn!("the echo-cancelled microphone could not be opened ({e}); using the microphone directly");
                #[cfg(target_os = "linux")]
                {
                    self.echo = None;
                }
                return self.open_capture_direct();
            }
            r => r,
        };
        self.opened(started, cancelled, cons)
    }

    #[cfg(target_os = "linux")]
    fn echo_source(&mut self) -> Option<String> {
        if !self.echo_on {
            self.echo = None;
            return None;
        }
        if self.echo.as_mut().is_some_and(|e| !e.alive() || e.mic != self.device) {
            self.echo = None;
        }
        if self.echo.is_none() {
            match setu_audio::echo::Canceller::start(&self.device) {
                Ok(e) => {
                    log::info!("speaker sound is cancelled from the microphone ({})", e.source);
                    self.echo = Some(e);
                }
                Err(e) => {
                    log::info!("no echo cancelling: {e}");
                    self.echo_on = false;
                }
            }
        }
        self.echo.as_ref().map(|e| e.source.clone())
    }

    #[cfg(not(target_os = "linux"))]
    fn echo_source(&mut self) -> Option<String> {
        let _ = self.echo_on;
        None
    }

    fn open_capture_direct(&mut self) -> bool {
        let on = self.echo_on;
        self.echo_on = false;
        let r = self.open_capture();
        self.echo_on = on;
        r
    }

    fn opened(&mut self, started: Result<Capture, String>, cancelled: bool, cons: Consumer) -> bool {
        match started {
            Ok(c) => {
                log::info!("microphone opened ({}{})", if self.device.is_empty() { "system default" } else { &self.device }, if cancelled { ", speaker sound cancelled" } else { "" });
                self.band.reset();
                self.agc = Agc { on: self.agc.on, ..Agc::new() };
                self.capture = Some(c);
                self.ring = Some(cons);
                self.dropped_seen = 0;
                self.open.store(true, Ordering::Relaxed);
                self.retry_at = None;
                if self.lost_reported {
                    self.lost_reported = false;
                }
                let _ = self.events.send(AudioEvent::DeviceOk);
                true
            }
            Err(e) => {
                log::warn!("microphone could not be opened: {e}");
                if !self.device.is_empty() {
                    self.device.clear();
                }
                self.retry_at = Some(Instant::now() + Duration::from_secs(5));
                if !self.lost_reported {
                    self.lost_reported = true;
                    let _ = self.events.send(AudioEvent::DeviceLost("No microphone found. Plug one in or choose a device.".into()));
                }
                false
            }
        }
    }

    fn ensure_spotter(&mut self, keywords: &[Keyword]) -> Option<String> {
        if self.spotter.is_none() {
            let sp = match SentencePiece::load(&self.models.bpe) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("wake word vocabulary failed to load: {e}");
                    return None;
                }
            };
            let first = keywords.iter().find_map(|k| keyword_line(&sp, k))?;
            let kws = match Kws::new(&KwsOptions {
                model: self.models.kws.clone(),
                threads: 1,
                keywords: first,
                score: boost_for(self.sensitivity),
                threshold: threshold_for(self.sensitivity),
                trailing_blanks: if self.long_window { 2 } else { 1 },
            }) {
                Ok(k) => k,
                Err(e) => {
                    log::error!("wake word detector failed to load: {e}");
                    return None;
                }
            };
            let stream = kws.stream("").ok()?;
            self.spotter = Some(Spotter { kws, stream, sp, fed: 0 });
        }
        let sp = self.spotter.as_ref()?;
        let lines: Vec<String> = keywords.iter().filter_map(|k| keyword_line(&sp.sp, k)).collect();
        Some(lines.join("/"))
    }

    fn enter(&mut self, m: Mode) {
        match &m {
            Mode::Spot(k) => {
                let Some(lines) = self.ensure_spotter(k) else {
                    self.mode = Mode::Off;
                    return;
                };
                let sp = self.spotter.as_mut().unwrap();
                match sp.kws.stream(&lines) {
                    Ok(s) => {
                        sp.stream = s;
                        sp.fed = 0;
                    }
                    Err(e) => log::error!("keywords could not be set: {e}"),
                }
            }
            Mode::Capture { id, preroll } => {
                self.ensure_vad();
                if let Some(v) = &self.vad {
                    v.reset();
                }
                if let Some(d) = self.denoiser.as_ref() {
                    d.reset();
                }
                self.speech_seen = false;
                self.silence = 0.0;
                self.speech_run = 0.0;
                self.started = Instant::now();
                let n = ((*preroll).max(0.0) * SR as f32) as usize;
                let n = n.min(self.history.len());
                if n > 0 {
                    let pre: Vec<f32> = self.history.iter().skip(self.history.len() - n).copied().collect();
                    let tail = pre.len().saturating_sub(SR / 2);
                    let seed: Vec<f32> = pre[tail..].to_vec();
                    let pre = self.clean(pre);
                    (self.pcm)(pre);
                    if let Some(v) = &self.vad {
                        v.accept(&seed);
                        let _ = v.pop();
                        if v.speaking() {
                            self.speech_seen = true;
                        }
                    }
                }
                let _ = id;
            }
            Mode::Converse => {
                if self.talk_vad.is_none() {
                    match Vad::new(&VadOptions { model: self.models.vad.clone(), threshold: 0.5, min_silence: 0.6, min_speech: 0.25, max_speech: 30.0 }) {
                        Ok(v) => self.talk_vad = Some(v),
                        Err(e) => log::error!("voice activity detector failed to load: {e}"),
                    }
                }
                if let Some(v) = &self.talk_vad {
                    v.reset();
                }
            }
            Mode::Await { .. } | Mode::Enroll => {
                self.ensure_vad();
                if let Some(v) = &self.vad {
                    v.reset();
                }
                self.speech_run = 0.0;
                self.speech_seen = false;
                self.silence = 0.0;
                self.started = Instant::now();
            }
            Mode::Off => {}
        }
        self.mode = m;
    }

    fn ensure_vad(&mut self) {
        if self.vad.is_none() {
            match Vad::new(&VadOptions { model: self.models.vad.clone(), threshold: 0.5, min_silence: 0.1, min_speech: 0.15, max_speech: 30.0 }) {
                Ok(v) => self.vad = Some(v),
                Err(e) => log::error!("voice activity detector failed to load: {e}"),
            }
        }
    }

    fn clean(&mut self, samples: Vec<f32>) -> Vec<f32> {
        if !self.denoise_on {
            return samples;
        }
        if self.denoiser.is_none() {
            match Denoiser::new(&self.models.denoise) {
                Ok(d) => self.denoiser = Some(d),
                Err(e) => {
                    log::warn!("noise reduction is unavailable: {e}");
                    self.denoise_on = false;
                    return samples;
                }
            }
        }
        let mut out = Vec::with_capacity(samples.len());
        self.denoiser.as_ref().unwrap().run(&samples, &mut out);
        out
    }

    fn remember(&mut self, c: &[f32]) {
        if self.history.len() + c.len() > HISTORY {
            let extra = self.history.len() + c.len() - HISTORY;
            self.history.drain(..extra.min(self.history.len()));
        }
        self.history.extend(c.iter().copied());
    }

    fn process(&mut self, c: Vec<f32>) {
        self.remember(&c);
        if let Some(n) = self.band.feed(&c) {
            self.narrow.store(n, Ordering::Relaxed);
            log::info!("microphone bandwidth looks {}", if n { "narrow (call quality)" } else { "wide" });
            let _ = self.events.send(AudioEvent::Bandwidth { narrow: n });
        }
        let dt = c.len() as f32 / SR as f32;
        match self.mode.clone() {
            Mode::Off => {}
            Mode::Spot(k) => {
                let wants = k.iter().any(|x| x.tag == "WAKE" || x.tag == "NAME");
                let names = k.iter().any(|x| x.tag == "NAME");
                if wants && self.listener_paths.is_some() {
                    self.listen(&c, names);
                }
                self.spot(&c);
                if self.learn
                    && let Some(v) = &self.vad {
                        v.accept(&c);
                        for seg in v.pop() {
                            let missed = !std::mem::replace(&mut self.hit_in_segment, false);
                            if missed && seg.samples.len() >= SR / 2 && seg.samples.len() <= SR * 6 {
                                let _ = self.events.send(AudioEvent::Missed { samples: seg.samples });
                            }
                        }
                    }
            }
            Mode::Capture { id, .. } => {
                let speaking = self.vad_step(&c, dt);
                let cleaned = self.clean(c);
                (self.pcm)(cleaned);
                if speaking {
                    self.speech_seen = true;
                }
                if self.speech_seen && self.silence >= self.end_silence {
                    let _ = self.events.send(AudioEvent::SpeechEnd { id });
                    self.mode = Mode::Await { id };
                    self.speech_run = 0.0;
                } else if !self.speech_seen && self.started.elapsed() >= NO_SPEECH {
                    let _ = self.events.send(AudioEvent::NoSpeech { id });
                    self.mode = Mode::Await { id };
                } else if self.started.elapsed() >= MAX_UTTERANCE {
                    let _ = self.events.send(AudioEvent::SpeechEnd { id });
                    self.mode = Mode::Await { id };
                }
            }
            Mode::Await { id } => {
                self.vad_step(&c, dt);
                if self.speech_run >= 0.25 {
                    self.speech_run = 0.0;
                    let _ = self.events.send(AudioEvent::Onset { id });
                }
            }
            Mode::Converse => {
                let cleaned = self.clean(c);
                if let Some(v) = &self.talk_vad {
                    v.accept(&cleaned);
                    for seg in v.pop() {
                        if seg.samples.len() >= SR * 7 / 20 {
                            let _ = self.events.send(AudioEvent::Utterance { samples: seg.samples });
                        }
                    }
                }
            }
            Mode::Enroll => {
                if let Some(v) = &self.vad {
                    v.accept(&c);
                    for seg in v.pop() {
                        if seg.samples.len() >= SR {
                            let _ = self.events.send(AudioEvent::EnrollSample { samples: seg.samples });
                        }
                    }
                }
            }
        }
    }

    fn vad_step(&mut self, c: &[f32], dt: f32) -> bool {
        let Some(v) = &self.vad else { return false };
        v.accept(c);
        let _ = v.pop();
        let speaking = v.speaking();
        if speaking {
            self.silence = 0.0;
            self.speech_run += dt;
        } else {
            self.silence += dt;
            self.speech_run = 0.0;
        }
        speaking
    }

    fn listen(&mut self, c: &[f32], names: bool) {
        if self.listener.is_none() {
            let Some(p) = self.listener_paths.clone() else { return };
            match OnlineAsr::new(&p, 1) {
                Ok(asr) => {
                    log::info!("wake listener ready");
                    self.listener = Some(Listener { asr, stream: None, audio: Vec::new(), decided: false, quiet: 0.0, cooldown: 0.0 });
                }
                Err(e) => {
                    log::warn!("wake listener could not start: {e}");
                    self.listener_paths = None;
                    return;
                }
            }
        }
        if self.listen_vad.is_none() {
            match Vad::new(&VadOptions { model: self.models.vad.clone(), threshold: 0.45, min_silence: 0.25, min_speech: 0.1, max_speech: 30.0 }) {
                Ok(v) => self.listen_vad = Some(v),
                Err(e) => {
                    log::warn!("wake listener has no voice detector: {e}");
                    return;
                }
            }
        }
        let dt = c.len() as f32 / SR as f32;
        let vad = self.listen_vad.as_ref().unwrap();
        vad.accept(c);
        let _ = vad.pop();
        let speaking = vad.speaking();
        let l = self.listener.as_mut().unwrap();
        l.cooldown = (l.cooldown - dt).max(0.0);
        if let Some(st) = &l.stream {
            l.asr.accept(st, c);
            l.audio.extend_from_slice(c);
        } else {
            if !speaking || l.cooldown > 0.0 {
                return;
            }
            let st = l.asr.stream();
            let n = self.history.len();
            let from = n.saturating_sub(LISTEN_PREROLL + c.len());
            let pre: Vec<f32> = self.history.iter().skip(from).copied().collect();
            l.asr.accept(&st, &pre);
            l.audio = pre;
            l.stream = Some(st);
            l.decided = false;
            l.quiet = 0.0;
        }
        l.quiet = if speaking { 0.0 } else { l.quiet + dt };
        let st = l.stream.as_ref().unwrap();
        l.asr.decode(st);
        if !l.decided {
            let text = l.asr.text(st);
            let words = crate::text::normalize(&text).split_whitespace().count();
            let ended = l.quiet >= 0.35;
            let settled = ended || l.audio.len() >= SR * 2;
            let narrow = self.narrow.load(Ordering::Relaxed);
            let mut hit = crate::wakeword::prefix_with(&text, &self.learned, narrow).filter(|(k, n)| (*k == crate::wakeword::Kind::Wake || names) && (words > *n || settled));
            let mut strict = false;
            if hit.is_none() && self.centroid.is_some() && (settled || words >= 3) && crate::wakeword::weak(&text) {
                hit = Some((crate::wakeword::Kind::Wake, 2));
                strict = true;
            }
            if let Some((kind, _)) = hit {
                l.decided = true;
                let seg = l.audio.clone();
                let tag = if kind == crate::wakeword::Kind::Wake { "WAKE" } else { "NAME" };
                log::debug!("wake listener heard {tag}");
                l.stream = None;
                l.cooldown = 1.0;
                let level = seg.chunks(320).map(rms).fold(0f32, f32::max).max(1e-6);
                let db = 20.0 * level.log10();
                self.hit_in_segment = true;
                let _ = self.events.send(AudioEvent::Keyword { tag: tag.into(), confidence: ((db + 45.0) / 35.0).clamp(0.05, 1.0), loud: db > -26.0, strict });
                return;
            }
            if words >= 4 || l.audio.len() >= LISTEN_WINDOW || ended {
                l.decided = true;
                if self.learn && !text.trim().is_empty() {
                    let _ = self.events.send(AudioEvent::Heard { text: text.clone() });
                }
            }
        }
        if l.quiet >= 0.35 || l.audio.len() >= LISTEN_WINDOW * 3 {
            l.stream = None;
            l.audio.clear();
        }
    }

    fn spot(&mut self, c: &[f32]) {
        let Some(sp) = self.spotter.as_mut() else { return };
        sp.kws.accept(&sp.stream, c);
        sp.fed += c.len() as u64;
        let Some(hit) = sp.kws.poll(&sp.stream) else { return };
        if self.listener.is_some() && (hit.keyword == "WAKE" || hit.keyword == "NAME") {
            return;
        }
        let span = ((hit.end - hit.start).clamp(0.4, 1.6) + 0.6) * SR as f32;
        let n = self.history.len();
        let a = n.saturating_sub(span as usize);
        let seg: Vec<f32> = self.history.iter().skip(a).copied().collect();
        let level = seg.chunks(320).map(rms).fold(0f32, f32::max).max(1e-6);
        let db = 20.0 * level.log10();
        let confidence = ((db + 45.0) / 35.0).clamp(0.05, 1.0);
        self.hit_in_segment = true;
        let _ = self.events.send(AudioEvent::Keyword { tag: hit.keyword, confidence, loud: db > -26.0, strict: false });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitivity_maps_monotonically() {
        assert!(threshold_for(1.0) < threshold_for(0.5));
        assert!(threshold_for(0.5) < threshold_for(0.0));
        assert!(boost_for(1.0) > boost_for(0.0));
        assert!((boost_for(0.5) - 3.5).abs() < 1e-6);
        assert!((threshold_for(0.5) - 0.10).abs() < 1e-6);
    }
}
