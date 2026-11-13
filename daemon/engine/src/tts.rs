use crate::phonemize::Phonemizer;
use crate::svara::Svara;
use crate::{Flags, Msg, Out, threads};
use crossbeam_channel::{Receiver, unbounded};
use setu_audio::Playback;
use setu_common::engine::{FromEngine, ToEngine, TtsConfig, TtsKind};
use setu_common::meter::Meter;
use setu_sherpa::Tts;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

enum Voice {
    Onnx { tts: Tts, ph: Phonemizer, sid: i32 },
    Svara(Box<Svara>),
}

impl Voice {
    fn sample_rate(&self) -> u32 {
        match self {
            Self::Onnx { tts, .. } => tts.sample_rate(),
            Self::Svara(_) => crate::svara::SAMPLE_RATE,
        }
    }

    fn line(&mut self, text: &str, speaker: i32, rate: f32, seed: u32, f: &mut dyn FnMut(&[f32]) -> bool) -> Result<(), String> {
        match self {
            Self::Onnx { tts, ph, sid } => {
                let parts = ph.parts(text)?;
                if !parts.is_empty() {
                    tts.generate(&parts, if speaker > 0 { speaker } else { *sid }, rate, f);
                }
                Ok(())
            }
            Self::Svara(s) => {
                for (k, chunk) in crate::phonemize::chunks(text).iter().enumerate() {
                    let mut go = true;
                    s.speak(chunk, seed.wrapping_add(k as u32), &mut |a| {
                        go = f(a);
                        go
                    })?;
                    if !go {
                        break;
                    }
                }
                Ok(())
            }
        }
    }
}

pub fn render(config: &str, text: &str, path: &str, rate: f32) -> Result<(), String> {
    let c: TtsConfig = serde_json::from_str(&std::fs::read_to_string(config).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if c.kind == TtsKind::Svara {
        crate::llama::set_gpu(c.gpu_layers != 0);
    }
    let flags = Flags::default();
    let out = Out::sink();
    let s = Server::new(out, flags);
    let (mut v, device) = s.open(&c)?;
    let sr = v.sample_rate();
    if !matches!(v, Voice::Svara(_)) {
        v.line("Okay.", 0, 1.0, 1, &mut |_| true)?;
    }
    let mut pcm: Vec<f32> = Vec::new();
    let t0 = Instant::now();
    let mut first = 0f64;
    v.line(text, 0, rate, 7, &mut |chunk| {
        if first == 0.0 && !chunk.is_empty() {
            first = t0.elapsed().as_secs_f64();
        }
        pcm.extend_from_slice(chunk);
        true
    })?;
    let secs = pcm.len() as f64 / sr as f64;
    eprintln!("{device}: {secs:.2} s of audio in {:.2} s, first audio after {first:.2} s", t0.elapsed().as_secs_f64());
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    let data = (pcm.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&sr.to_le_bytes());
    w.extend_from_slice(&(sr * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, w).map_err(|e| e.to_string())
}

const CACHE_CHARS: usize = 90;
const CACHE_SAMPLES: usize = 24_000 * 120;

#[derive(Default)]
struct Cache {
    map: HashMap<String, Arc<Vec<f32>>>,
    order: VecDeque<String>,
    samples: usize,
}

impl Cache {
    fn key(text: &str, speaker: i32, rate: f32) -> Option<String> {
        let t = text.trim();
        (!t.is_empty() && t.chars().count() <= CACHE_CHARS).then(|| format!("{speaker}|{:.2}|{t}", rate))
    }

    fn get(&mut self, k: &str) -> Option<Arc<Vec<f32>>> {
        let v = self.map.get(k).cloned()?;
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            let key = self.order.remove(pos).unwrap_or_default();
            self.order.push_back(key);
        }
        Some(v)
    }

    fn put(&mut self, k: String, v: Vec<f32>) {
        if self.map.contains_key(&k) || v.len() > CACHE_SAMPLES / 8 {
            return;
        }
        self.samples += v.len();
        self.map.insert(k.clone(), Arc::new(v));
        self.order.push_back(k);
        while self.samples > CACHE_SAMPLES {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(a) = self.map.remove(&old) {
                self.samples -= a.len();
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.samples = 0;
    }
}

enum Play {
    Line(u32),
    Audio(Arc<Vec<f32>>),
}

struct Player<'a> {
    flags: &'a Flags,
    out: &'a Out,
    id: u64,
    sr: u32,
    vol: f32,
    t0: Instant,
    first: &'a AtomicU64,
}

impl Player<'_> {
    fn run(&self, pb: &mut Playback, rx: Receiver<Play>) -> bool {
        let step = (self.sr / 25).max(1) as usize;
        let mut buf: Vec<f32> = Vec::with_capacity(step);
        let lead = (self.sr / 50) as usize;
        let mut started = false;
        let out = self.out.clone();
        let id = self.id;
        let mut meter = Meter::new(self.sr, move |level| out.send(&FromEngine::TtsLevel { id, level }));
        for item in rx.iter() {
            if self.flags.cancel.load(Ordering::Relaxed) {
                break;
            }
            match item {
                Play::Line(i) => self.out.send(&FromEngine::TtsLine { id: self.id, index: i }),
                Play::Audio(a) => {
                    if !started {
                        started = true;
                        let silence = vec![0.0; lead];
                        meter.push(&silence);
                        let _ = pb.write(&silence);
                    }
                    if self.first.load(Ordering::Relaxed) == 0 {
                        self.first.store((self.t0.elapsed().as_millis() as u64).max(1), Ordering::Relaxed);
                    }
                    for part in a.chunks(step) {
                        if self.flags.cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        meter.push(part);
                        buf.clear();
                        buf.extend(part.iter().map(|s| s * self.vol));
                        if pb.write(&buf).is_err() {
                            meter.cancel();
                            return true;
                        }
                    }
                }
            }
        }
        if self.flags.cancel.load(Ordering::Relaxed) {
            meter.cancel();
            pb.flush();
            true
        } else {
            meter.finish();
            let _ = pb.write(&vec![0.0; (self.sr as usize) * 3 / 20]);
            pb.drain();
            false
        }
    }
}

pub struct Server {
    out: Out,
    flags: Flags,
    voice: Option<Voice>,
    playback: Option<(String, Playback)>,
    cache: Cache,
}

impl Server {
    pub fn new(out: Out, flags: Flags) -> Self {
        Self { out, flags, voice: None, playback: None, cache: Cache::default() }
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(m) = rx.recv() {
            match m {
                Msg::Op(op) => self.op(op),
                Msg::Speak(op, seq) => {
                    if seq == self.flags.stop_seq.load(Ordering::Acquire) {
                        self.op(op);
                    } else if let ToEngine::TtsSpeak { id, .. } = op {
                        self.out.send(&FromEngine::TtsDone { id, stopped: true, first_audio_ms: 0 });
                    }
                }
                Msg::Pcm(_) | Msg::Bytes(_) => {}
            }
        }
    }

    fn op(&mut self, op: ToEngine) {
        match op {
            ToEngine::LoadTts { config } => self.load(config),
            ToEngine::Unload => {
                self.playback = None;
                self.voice = None;
                self.cache.clear();
                self.out.send(&FromEngine::Unloaded);
            }
            ToEngine::TtsSpeak { id, lines, speaker, rate, volume, device, open } => self.speak(id, &lines, speaker, rate, volume, &device, open),
            ToEngine::TtsStop => self.flags.cancel.store(false, Ordering::Relaxed),
            ToEngine::Bench { .. } => self.bench(),
            _ => {}
        }
    }

    fn open(&self, c: &TtsConfig) -> Result<(Voice, String), String> {
        let wide = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4) >= 12;
        let n = if c.threads > 0 { c.threads } else { threads(0).min(if wide { 6 } else { 4 }) };
        match c.kind {
            TtsKind::Piper => {
                let tts = Tts::new(&c.model, &c.tokens, n)?;
                let ph = Phonemizer::new(&c.voice, &c.espeak_path).with_vocab(&c.tokens);
                Ok((Voice::Onnx { tts, ph, sid: c.speaker.max(0) }, "CPU".into()))
            }
            TtsKind::Kokoro => {
                let tts = Tts::kokoro(&c.model, &c.voices, &c.tokens, n)?;
                let indian = c.speaker_name.starts_with('h');
                let ph = Phonemizer::new(&c.voice, &c.espeak_path).with_vocab(&c.tokens).indian(indian);
                Ok((Voice::Onnx { tts, ph, sid: c.speaker.max(0) }, "CPU".into()))
            }
            TtsKind::Svara => {
                let s = Svara::load(&c.model, &c.codec, &c.speaker_name, c.gpu_layers, threads(c.threads), self.flags.cancel.clone())?;
                let d = s.device();
                Ok((Voice::Svara(Box::new(s)), d))
            }
        }
    }

    fn load(&mut self, c: TtsConfig) {
        let t0 = Instant::now();
        self.flags.busy();
        self.voice = None;
        self.cache.clear();
        match self.open(&c) {
            Ok((mut v, device)) => {
                if !matches!(v, Voice::Svara(_)) {
                    let _ = v.line("Okay.", 0, 1.0, 1, &mut |_| true);
                }
                self.voice = Some(v);
                self.flags.idle();
                self.out.send(&FromEngine::Loaded { ram: crate::rss(), ms: t0.elapsed().as_millis() as u64, device });
            }
            Err(e) => {
                self.flags.idle();
                self.out.error(e, true);
            }
        }
    }

    fn playback(&mut self, device: &str, rate: u32) -> Option<&mut Playback> {
        if self.playback.as_ref().is_none_or(|(d, p)| d != device || p.rate() != rate) {
            self.playback = None;
            match Playback::open(device, rate, "Assistant voice") {
                Ok(p) => self.playback = Some((device.to_owned(), p)),
                Err(e) => {
                    self.out.error(format!("No speaker is available: {e}"), false);
                    return None;
                }
            }
        }
        self.playback.as_mut().map(|(_, p)| p)
    }

    #[allow(clippy::too_many_arguments)]
    fn speak(&mut self, id: u64, lines: &[String], speaker: i32, rate: f32, volume: f32, device: &str, open: bool) {
        let t0 = Instant::now();
        self.flags.cancel.store(false, Ordering::Relaxed);
        let Some(mut voice) = self.voice.take() else {
            return self.out.error("The voice is not loaded.", false);
        };
        let sr = voice.sample_rate();
        let flags = self.flags.clone();
        let out = self.out.clone();
        let first = AtomicU64::new(0);
        let mut cache = std::mem::take(&mut self.cache);
        self.flags.busy();
        let stopped = match self.playback(device, sr) {
            Some(pb) => {
                let (tx, rx) = unbounded::<Play>();
                let player = Player { flags: &flags, out: &out, id, sr, vol: volume.clamp(0.0, 1.0), t0, first: &first };
                std::thread::scope(|s| {
                    let p = &player;
                    let handle = s.spawn(move || p.run(pb, rx));
                    let mut queued = lines.iter().cloned();
                    let mut count = 0usize;
                    loop {
                        if flags.cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let next = match queued.next() {
                            Some(l) => Some(l),
                            None if open => flags.feed.next(id, &flags.cancel, std::time::Duration::from_secs(6)),
                            None => None,
                        };
                        let Some(line) = next else { break };
                        let line = &line;
                        let i = count;
                        count += 1;
                        let _ = tx.send(Play::Line(i as u32));
                        let key = Cache::key(line, speaker, rate);
                        if let Some(a) = key.as_deref().and_then(|k| cache.get(k)) {
                            let _ = tx.send(Play::Audio(a));
                            continue;
                        }
                        let mut whole: Vec<f32> = Vec::new();
                        let keep = key.is_some();
                        let r = voice.line(line, speaker, rate, (id as u32) ^ (i as u32), &mut |chunk| {
                            if flags.cancel.load(Ordering::Relaxed) {
                                return false;
                            }
                            if chunk.is_empty() {
                                return true;
                            }
                            if keep {
                                whole.extend_from_slice(chunk);
                            }
                            tx.send(Play::Audio(Arc::new(chunk.to_vec()))).is_ok()
                        });
                        match r {
                            Ok(()) => {
                                if let Some(k) = key
                                    && !flags.cancel.load(Ordering::Relaxed)
                                    && !whole.is_empty()
                                {
                                    cache.put(k, whole);
                                }
                            }
                            Err(e) => {
                                if e != "cancelled" {
                                    out.error(e, false);
                                }
                                break;
                            }
                        }
                    }
                    drop(tx);
                    handle.join().unwrap_or(true)
                })
            }
            None => true,
        };
        self.cache = cache;
        self.voice = Some(voice);
        self.flags.cancel.store(false, Ordering::Relaxed);
        self.flags.idle();
        self.out.send(&FromEngine::TtsDone { id, stopped, first_audio_ms: first.load(Ordering::Relaxed) });
    }

    fn bench(&mut self) {
        let Some(voice) = self.voice.as_mut() else {
            return self.out.error("The voice is not loaded.", false);
        };
        self.flags.busy();
        let sr = voice.sample_rate();
        let t0 = Instant::now();
        let mut n = 0usize;
        let r = voice.line("Opening the browser. The volume is now at forty percent.", 0, 1.0, 1, &mut |c| {
            n += c.len();
            true
        });
        self.flags.idle();
        if let Err(e) = r {
            return self.out.error(e, false);
        }
        let rtf = t0.elapsed().as_secs_f64() / (n as f64 / sr as f64).max(1e-3);
        self.out.send(&FromEngine::BenchResult { value: (rtf * 1000.0).round() / 1000.0, unit: "x real time".into(), label: "Voice speed".into() });
    }
}
