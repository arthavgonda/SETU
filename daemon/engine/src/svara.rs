use crate::llama::{Context, Model, Sampler, sys::llama_token};
use setu_sherpa::ort::SnacDecoder;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const BOS: llama_token = 128000;
const END_OF_TURN: llama_token = 128009;
const START_OF_SPEECH: llama_token = 128257;
const END_OF_SPEECH: llama_token = 128258;
const START_OF_HUMAN: llama_token = 128259;
const END_OF_HUMAN: llama_token = 128260;
const START_OF_AI: llama_token = 128261;
const AUDIO_START: llama_token = 128266;
const AUDIO: llama_token = 156939;
const WINDOW: usize = 4;
const FRAME_SAMPLES: usize = 2048;
pub const SAMPLE_RATE: u32 = 24_000;

pub struct Svara {
    model: Model,
    ctx: Context,
    snac: SnacDecoder,
    speaker: String,
    cancel: Arc<AtomicBool>,
}

fn clean(text: &str) -> String {
    let mut t = text.replace("<|", "").replace("|>", "");
    t = t.chars().filter(|c| !c.is_control()).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Svara {
    pub fn load(gguf: &str, codec: &str, speaker: &str, gpu_layers: i32, threads: i32, cancel: Arc<AtomicBool>) -> Result<Self, String> {
        let model = Model::load_with(gguf, gpu_layers, 512 * 1024 * 1024)?;
        if model.token("<custom_token_1>") != Some(START_OF_SPEECH) {
            return Err("This file is not a svara voice model.".into());
        }
        let ctx = Context::new(&model, 4096, threads, cancel.clone())?;
        let snac = SnacDecoder::new(codec, threads.clamp(1, 4))?;
        Ok(Self { model, ctx, snac, speaker: if speaker.is_empty() { "English (Indian) (Female)".into() } else { speaker.to_owned() }, cancel })
    }

    pub fn device(&self) -> String {
        self.model.device.clone()
    }

    fn prompt(&self, text: &str) -> Vec<llama_token> {
        let body = self.model.tokenize(&format!("{}: {}", self.speaker, clean(text)), false);
        let mut v = Vec::with_capacity(body.len() + 8);
        v.extend([BOS, START_OF_HUMAN, AUDIO]);
        v.extend(body);
        v.extend([END_OF_HUMAN, END_OF_TURN, START_OF_AI, START_OF_SPEECH]);
        v
    }

    fn decode_window(&self, codes: &[i64]) -> Option<Vec<f32>> {
        let f = codes.len() / 7;
        let mut c0 = Vec::with_capacity(f);
        let mut c1 = Vec::with_capacity(f * 2);
        let mut c2 = Vec::with_capacity(f * 4);
        for fr in codes.as_chunks::<7>().0 {
            c0.push(fr[0]);
            c1.extend([fr[1], fr[4]]);
            c2.extend([fr[2], fr[3], fr[5], fr[6]]);
        }
        let audio = self.snac.decode(&c0, &c1, &c2).ok()?;
        (audio.len() >= 2 * FRAME_SAMPLES).then(|| audio[FRAME_SAMPLES..2 * FRAME_SAMPLES].to_vec())
    }

    pub fn speak(&mut self, text: &str, seed: u32, sink: &mut dyn FnMut(&[f32]) -> bool) -> Result<(), String> {
        let t0 = Instant::now();
        let prompt = self.prompt(text);
        self.ctx.clear();
        self.ctx.decode(&prompt)?;
        let mut sampler = Sampler::speech(&self.model, 0.6, 0.95, 1.1, seed);
        let budget = (text.chars().count() * 9 + 300).min(4096 - prompt.len() - 8) as u32;
        let mut codes: Vec<i64> = Vec::with_capacity(budget as usize);
        let mut n = 0u32;
        let mut held: Vec<f32> = Vec::new();
        let mut streaming = false;
        let mut target = SAMPLE_RATE as f32 * 0.5;
        let expected = (text.chars().count() as f32 / 14.0).max(0.8);
        while n < budget {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let tok = sampler.sample(&self.ctx);
            n += 1;
            if tok == END_OF_SPEECH || self.model.is_eog(tok) {
                break;
            }
            if tok >= AUDIO_START {
                let code = (tok - AUDIO_START) as i64 - ((codes.len() % 7) as i64) * 4096;
                if (0..4096).contains(&code) {
                    codes.push(code);
                    if codes.len().is_multiple_of(7) && codes.len() >= WINDOW * 7
                        && let Some(pcm) = self.decode_window(&codes[codes.len() - WINDOW * 7..]) {
                            if streaming {
                                if !sink(&pcm) {
                                    return Ok(());
                                }
                            } else {
                                held.extend_from_slice(&pcm);
                                if held.len() as f32 >= target {
                                    let speed = (held.len() as f32 / SAMPLE_RATE as f32) / t0.elapsed().as_secs_f32().max(1e-3);
                                    if speed < 1.1 && target < SAMPLE_RATE as f32 * expected {
                                        target = (SAMPLE_RATE as f32 * (expected * (1.0 - speed / 1.1) + 0.5)).max(target + FRAME_SAMPLES as f32);
                                    } else {
                                        streaming = true;
                                        if !sink(&held) {
                                            return Ok(());
                                        }
                                        held.clear();
                                    }
                                }
                            }
                        }
                }
            }
            self.ctx.decode(&[tok])?;
        }
        if !held.is_empty() {
            sink(&held);
        }
        Ok(())
    }
}
