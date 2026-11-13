use crate::{Flags, Msg, Out, threads};
use crossbeam_channel::Receiver;
use setu_common::engine::{AsrConfig, FromEngine, OfflineModel, ToEngine};
use setu_sherpa::{OfflineAsr, OfflineKind, OnlineAsr, OnlineStream, TransducerPaths};
use std::sync::atomic::Ordering;
use std::time::Instant;

const MAX_OFFLINE_SAMPLES: usize = 16_000 * 30;
const MAX_BUFFER: usize = 16_000 * 60;

struct Session {
    id: u64,
    stream: Option<OnlineStream>,
    buffer: Vec<f32>,
    partial: String,
}

pub fn transcribe(config: &str, wavs: &[String]) -> Result<(), String> {
    let c: AsrConfig = serde_json::from_str(&std::fs::read_to_string(config).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut s = Server::new(Out::sink(), Flags::default());
    s.quiet = true;
    s.load(c);
    let off = s.offline.as_ref().ok_or("no offline model in the configuration")?;
    for w in wavs {
        let (pcm, sr) = setu_sherpa::read_wave(w).ok_or_else(|| format!("{w} is not a WAV file"))?;
        let pcm = if sr as u32 == 16_000 {
            pcm
        } else {
            let mut r = setu_audio::resample::Resampler::new(sr as u32, 16_000);
            let mut out = vec![0f32; r.max_output(pcm.len())];
            let n = r.process(&pcm, &mut out);
            out.truncate(n);
            out
        };
        let mut padded = pcm;
        padded.extend(std::iter::repeat_n(0.0, 4000));
        println!("{w}\t{}", off.recognize(&padded).text.trim());
    }
    Ok(())
}

pub struct Server {
    quiet: bool,
    out: Out,
    flags: Flags,
    online: Option<OnlineAsr>,
    offline: Option<OfflineAsr>,
    session: Option<Session>,
}

impl Server {
    pub fn new(out: Out, flags: Flags) -> Self {
        Self { quiet: false, out, flags, online: None, offline: None, session: None }
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(m) = rx.recv() {
            match m {
                Msg::Pcm(s) => self.audio(&s),
                Msg::Op(op) => self.op(op),
                Msg::Speak(..) | Msg::Bytes(_) => {}
            }
        }
    }

    fn op(&mut self, op: ToEngine) {
        match op {
            ToEngine::LoadAsr { config } => self.load(config),
            ToEngine::Unload => {
                self.session = None;
                self.online = None;
                self.offline = None;
                self.out.send(&FromEngine::Unloaded);
            }
            ToEngine::AsrBegin { id, .. } => {
                self.flags.cancel.store(false, Ordering::Relaxed);
                let stream = self.online.as_ref().map(|o| o.stream());
                self.session = Some(Session { id, stream, buffer: Vec::with_capacity(16_000 * 8), partial: String::new() });
            }
            ToEngine::AsrEnd { id } => self.finish(id),
            ToEngine::AsrCancel { id }
                if self.session.as_ref().is_some_and(|s| s.id == id) => {
                    self.session = None;
                }
            _ => {}
        }
    }

    fn load(&mut self, c: AsrConfig) {
        let t0 = Instant::now();
        self.flags.busy();
        let n = threads(c.threads);
        let online = match &c.streaming {
            Some(m) => match OnlineAsr::new(
                &TransducerPaths { encoder: m.encoder.clone(), decoder: m.decoder.clone(), joiner: m.joiner.clone(), tokens: m.tokens.clone() },
                n.min(4),
            ) {
                Ok(o) => Some(o),
                Err(e) => {
                    self.flags.idle();
                    return self.out.error(e, true);
                }
            },
            None => None,
        };
        let offline = match &c.offline {
            Some(m) => {
                let kind = match m {
                    OfflineModel::Transducer { encoder, decoder, joiner, tokens, nemo } => OfflineKind::Transducer {
                        paths: TransducerPaths { encoder: encoder.clone(), decoder: decoder.clone(), joiner: joiner.clone(), tokens: tokens.clone() },
                        nemo: *nemo,
                    },
                    OfflineModel::Whisper { encoder, decoder, tokens, language } => OfflineKind::Whisper {
                        encoder: encoder.clone(),
                        decoder: decoder.clone(),
                        tokens: tokens.clone(),
                        language: language.clone(),
                    },
                    OfflineModel::Moonshine { preprocessor, encoder, uncached_decoder, cached_decoder, tokens } => OfflineKind::Moonshine {
                        preprocessor: preprocessor.clone(),
                        encoder: encoder.clone(),
                        uncached_decoder: uncached_decoder.clone(),
                        cached_decoder: cached_decoder.clone(),
                        tokens: tokens.clone(),
                    },
                };
                match OfflineAsr::new(&kind, n) {
                    Ok(o) => Some(o),
                    Err(e) => {
                        self.flags.idle();
                        return self.out.error(e, true);
                    }
                }
            }
            None => None,
        };
        if online.is_none() && offline.is_none() {
            self.flags.idle();
            return self.out.error("No speech recognition model was configured.", true);
        }
        self.online = online;
        self.offline = offline;
        self.flags.idle();
        if !self.quiet {
            self.out.send(&FromEngine::Loaded { ram: crate::rss(), ms: t0.elapsed().as_millis() as u64, device: "CPU".into() });
        }
    }

    fn audio(&mut self, s: &[f32]) {
        let Some(sess) = self.session.as_mut() else { return };
        if sess.buffer.len() + s.len() <= MAX_BUFFER {
            sess.buffer.extend_from_slice(s);
        }
        if let (Some(o), Some(st)) = (self.online.as_ref(), sess.stream.as_ref()) {
            self.flags.busy();
            o.accept(st, s);
            o.decode(st);
            self.flags.idle();
            let t = o.text(st);
            if !t.is_empty() && t != sess.partial {
                sess.partial = t.clone();
                self.out.send(&FromEngine::AsrPartial { id: sess.id, text: t });
            }
        }
    }

    fn finish(&mut self, id: u64) {
        let Some(sess) = self.session.take() else { return };
        if sess.id != id {
            self.session = Some(sess);
            return;
        }
        let t0 = Instant::now();
        self.flags.busy();
        let mut text = String::new();
        if let (Some(o), Some(st)) = (self.online.as_ref(), sess.stream.as_ref()) {
            o.finish(st);
            text = o.text(st);
        }
        let online = text.trim().to_owned();
        let mut language = String::from("en");
        let cancelled = || self.flags.cancel.load(Ordering::Relaxed) && self.flags.cancel_id.load(Ordering::Relaxed) == id;
        if let Some(off) = self.offline.as_ref()
            && sess.buffer.len() >= 16_000 / 4 && (sess.buffer.len() <= MAX_OFFLINE_SAMPLES || text.is_empty()) && !cancelled() {
                let mut padded = Vec::with_capacity(sess.buffer.len() + 8000);
                padded.extend_from_slice(&sess.buffer[sess.buffer.len().saturating_sub(MAX_OFFLINE_SAMPLES)..]);
                padded.extend(std::iter::repeat_n(0.0, 4000));
                let r = off.recognize(&padded);
                if !r.text.trim().is_empty() {
                    text = r.text;
                }
                if !r.lang.is_empty() {
                    language = r.lang.trim_matches(|c| c == '<' || c == '>' || c == '|').to_owned();
                }
            }
        self.flags.idle();
        if cancelled() {
            return;
        }
        self.out.send(&FromEngine::AsrFinal { id, text: text.trim().to_owned(), online, language, ms: t0.elapsed().as_millis() as u64 });
    }
}
