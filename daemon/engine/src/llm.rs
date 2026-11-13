use crate::llama::{Context, Model, Sampler};
use crate::{Flags, Msg, Out, threads};
use crossbeam_channel::Receiver;
use setu_common::engine::{FromEngine, LlmConfig, ToEngine};
use std::sync::atomic::Ordering;
use std::time::Instant;

struct Prefix {
    key: String,
    tokens: Vec<i32>,
    state: Option<Vec<u8>>,
}

pub struct Server {
    out: Out,
    flags: Flags,
    model: Option<Model>,
    ctx: Option<Context>,
    prefix: Option<Prefix>,
    snapshot: bool,
}

impl Server {
    pub fn new(out: Out, flags: Flags) -> Self {
        Self { out, flags, model: None, ctx: None, prefix: None, snapshot: false }
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(m) = rx.recv() {
            if let Msg::Op(op) = m {
                self.op(op);
            }
        }
    }

    fn op(&mut self, op: ToEngine) {
        match op {
            ToEngine::LoadLlm { config } => self.load(config),
            ToEngine::Unload => {
                self.prefix = None;
                self.ctx = None;
                self.model = None;
                self.out.send(&FromEngine::Unloaded);
            }
            ToEngine::LlmPrefix { key, text } => self.set_prefix(key, &text),
            ToEngine::LlmGenerate { id, prefix_key, text, grammar, max_tokens, temperature, stream } => {
                self.generate(id, &prefix_key, &text, grammar.as_deref(), max_tokens, temperature, stream)
            }
            ToEngine::Bench { seconds } => self.bench(seconds),
            _ => {}
        }
    }

    fn load(&mut self, c: LlmConfig) {
        let t0 = Instant::now();
        self.flags.busy();
        self.prefix = None;
        self.ctx = None;
        self.model = None;
        let model = match Model::load(&c.path, c.gpu_layers) {
            Ok(m) => m,
            Err(e) => {
                self.flags.idle();
                return self.out.error(e, true);
            }
        };
        let ctx = match Context::new(&model, c.context.max(1024), threads(c.threads), self.flags.cancel.clone()) {
            Ok(x) => x,
            Err(e) => {
                self.flags.idle();
                return self.out.error(e, true);
            }
        };
        self.snapshot = model.snapshot_needed();
        let device = model.device.clone();
        self.model = Some(model);
        self.ctx = Some(ctx);
        self.flags.idle();
        self.out.send(&FromEngine::Loaded { ram: crate::rss(), ms: t0.elapsed().as_millis() as u64, device });
    }

    fn set_prefix(&mut self, key: String, text: &str) {
        let (Some(model), Some(ctx)) = (self.model.as_ref(), self.ctx.as_mut()) else {
            return self.out.error("The language model is not loaded.", false);
        };
        let t0 = Instant::now();
        self.flags.busy();
        let tokens = model.tokenize(text, true);
        self.flags.cancel.store(false, Ordering::Relaxed);
        ctx.clear();
        if let Err(e) = ctx.decode(&tokens) {
            self.flags.idle();
            self.prefix = None;
            return self.out.error(e, false);
        }
        let state = if self.snapshot { Some(ctx.save()) } else { None };
        let n = tokens.len() as u32;
        self.prefix = Some(Prefix { key: key.clone(), tokens, state });
        self.flags.idle();
        self.out.send(&FromEngine::LlmPrefixed { key, tokens: n, ms: t0.elapsed().as_millis() as u64 });
    }

    fn rewind(&mut self, key: &str) -> Result<(), String> {
        let Some(ctx) = self.ctx.as_mut() else {
            return Err("The language model is not loaded.".into());
        };
        match self.prefix.as_ref() {
            Some(p) if p.key == key => {
                let n = p.tokens.len() as i32;
                let ok = match &p.state {
                    Some(st) => ctx.restore(st, n),
                    None => ctx.truncate(n),
                };
                if !ok {
                    ctx.clear();
                    ctx.decode(&p.tokens)?;
                }
                Ok(())
            }
            _ => {
                ctx.clear();
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn generate(&mut self, id: u64, key: &str, text: &str, grammar: Option<&str>, max_tokens: u32, temperature: f32, stream: bool) {
        let t0 = Instant::now();
        let ms = || t0.elapsed().as_millis() as u64;
        if self.flags.cancel.load(Ordering::Relaxed) && self.flags.cancel_id.load(Ordering::Relaxed) == id {
            self.flags.cancel.store(false, Ordering::Relaxed);
            return self.out.send(&FromEngine::LlmResult { id, text: String::new(), tokens: 0, ms: 0, cancelled: true });
        }
        self.flags.cancel.store(false, Ordering::Relaxed);
        self.flags.busy();
        if let Err(e) = self.rewind(key) {
            self.flags.idle();
            return self.out.error(e, false);
        }
        let (Some(model), Some(ctx)) = (self.model.as_ref(), self.ctx.as_mut()) else { return };
        let add_special = self.prefix.as_ref().is_none_or(|p| p.key != key);
        let tokens = model.tokenize(text, add_special);
        let stopped = |f: &Flags| f.cancel.load(Ordering::Relaxed);
        if let Err(e) = ctx.decode(&tokens) {
            self.flags.idle();
            let c = stopped(&self.flags);
            self.flags.cancel.store(false, Ordering::Relaxed);
            self.prefix = None;
            if c {
                return self.out.send(&FromEngine::LlmResult { id, text: String::new(), tokens: 0, ms: ms(), cancelled: true });
            }
            return self.out.error(e, false);
        }
        let mut sampler = match Sampler::new(model, grammar, temperature, id as u32) {
            Ok(s) => s,
            Err(e) => {
                self.flags.idle();
                return self.out.error(e, false);
            }
        };
        let mut bytes = Vec::new();
        let mut n = 0u32;
        let mut was_cancelled = false;
        while n < max_tokens.max(1) {
            if stopped(&self.flags) {
                was_cancelled = true;
                break;
            }
            let tok = sampler.sample(ctx);
            if model.is_eog(tok) {
                break;
            }
            let before = bytes.len();
            model.piece(tok, &mut bytes);
            n += 1;
            if stream && bytes[before..].iter().any(|b| matches!(b, b'.' | b'?' | b'!' | b',')) {
                self.out.send(&FromEngine::LlmPartial { id, text: String::from_utf8_lossy(&bytes).into_owned() });
            }
            if ctx.decode(&[tok]).is_err() {
                was_cancelled = stopped(&self.flags);
                break;
            }
        }
        if was_cancelled {
            self.prefix = None;
        }
        self.flags.cancel.store(false, Ordering::Relaxed);
        self.flags.idle();
        self.out.send(&FromEngine::LlmResult { id, text: String::from_utf8_lossy(&bytes).into_owned(), tokens: n, ms: ms(), cancelled: was_cancelled });
    }

    fn bench(&mut self, seconds: f32) {
        let (Some(model), Some(ctx)) = (self.model.as_ref(), self.ctx.as_mut()) else {
            return self.out.error("The language model is not loaded.", false);
        };
        self.flags.busy();
        ctx.clear();
        let prompt = model.tokenize("The quick brown fox jumps over the lazy dog. Count from one to fifty in words:", true);
        if ctx.decode(&prompt).is_err() {
            self.flags.idle();
            return self.out.error("benchmark failed", false);
        }
        let Ok(mut sampler) = Sampler::new(model, None, 0.0, 1) else { return };
        let t0 = Instant::now();
        let mut n = 0u32;
        while t0.elapsed().as_secs_f32() < seconds.clamp(0.5, 10.0) && n < 256 {
            let tok = sampler.sample(ctx);
            if model.is_eog(tok) || ctx.decode(&[tok]).is_err() {
                break;
            }
            n += 1;
        }
        let tps = n as f64 / t0.elapsed().as_secs_f64().max(1e-3);
        ctx.clear();
        self.prefix = None;
        self.flags.idle();
        self.out.send(&FromEngine::BenchResult { value: (tps * 10.0).round() / 10.0, unit: "tokens/s".into(), label: "Language model speed".into() });
    }
}
