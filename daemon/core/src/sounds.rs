use crate::core::Core;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const RATE: u32 = 24_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cue {
    Wake,
    Heard,
    Done,
    Open,
    Close,
    Error,
    Cancel,
    Tap,
    Chime,
    Toggle,
}

impl Cue {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "wake" => Self::Wake,
            "heard" => Self::Heard,
            "done" => Self::Done,
            "open" => Self::Open,
            "close" => Self::Close,
            "error" => Self::Error,
            "cancel" => Self::Cancel,
            "tap" => Self::Tap,
            "chime" => Self::Chime,
            "toggle" => Self::Toggle,
            _ => return None,
        })
    }

    const ALL: [Cue; 10] = [Self::Wake, Self::Heard, Self::Done, Self::Open, Self::Close, Self::Error, Self::Cancel, Self::Tap, Self::Chime, Self::Toggle];
}

fn secs(n: usize) -> f32 {
    n as f32 / RATE as f32
}

fn len(s: f32) -> usize {
    (s * RATE as f32) as usize
}

fn mix(out: &mut Vec<f32>, at: f32, part: &[f32]) {
    let start = len(at);
    if out.len() < start + part.len() {
        out.resize(start + part.len(), 0.0);
    }
    for (o, p) in out[start..].iter_mut().zip(part) {
        *o += p;
    }
}

fn note(f0: f32, f1: f32, dur: f32, amp: f32, decay: f32, partials: &[(f32, f32)]) -> Vec<f32> {
    let n = len(dur);
    let attack = len(0.006).max(1);
    let mut phase = vec![0f32; partials.len() + 1];
    (0..n)
        .map(|i| {
            let t = secs(i);
            let k = i as f32 / n as f32;
            let f = f0 * (f1 / f0).powf(k * k * (3.0 - 2.0 * k));
            let env = (i.min(attack) as f32 / attack as f32) * (-t / decay).exp() * (1.0 - k).powf(0.35);
            phase[0] += 2.0 * PI * f / RATE as f32;
            let mut v = phase[0].sin();
            for (j, (mult, gain)) in partials.iter().enumerate() {
                phase[j + 1] += 2.0 * PI * f * mult / RATE as f32;
                v += gain * phase[j + 1].sin();
            }
            amp * env * v
        })
        .collect()
}

fn whoosh(dur: f32, from: f32, to: f32, amp: f32) -> Vec<f32> {
    let n = len(dur);
    let mut seed: u32 = 0x9E37_79B9;
    let (mut lp, mut hp_prev, mut hp) = (0f32, 0f32, 0f32);
    (0..n)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let white = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let k = i as f32 / n as f32;
            let cut = from * (to / from).powf(k);
            let a = 1.0 - (-2.0 * PI * cut / RATE as f32).exp();
            lp += a * (white - lp);
            hp = 0.97 * (hp + lp - hp_prev);
            hp_prev = lp;
            let env = (PI * k).sin().powf(1.6);
            amp * env * hp
        })
        .collect()
}

const BELL: &[(f32, f32)] = &[(2.0, 0.22), (3.01, 0.08), (4.2, 0.03)];
const SOFT: &[(f32, f32)] = &[(2.0, 0.12)];
const ROUND: &[(f32, f32)] = &[(3.0, 0.18), (5.0, 0.05)];

fn synth(c: Cue) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::new();
    match c {
        Cue::Wake => {
            mix(&mut out, 0.0, &note(620.0, 880.0, 0.085, 0.20, 0.12, SOFT));
            mix(&mut out, 0.075, &note(930.0, 1320.0, 0.12, 0.22, 0.10, SOFT));
        }
        Cue::Heard => {
            mix(&mut out, 0.0, &note(1180.0, 1320.0, 0.05, 0.10, 0.03, SOFT));
        }
        Cue::Done => {
            mix(&mut out, 0.0, &note(1318.5, 1318.5, 0.22, 0.16, 0.07, BELL));
            mix(&mut out, 0.085, &note(1760.0, 1760.0, 0.32, 0.17, 0.10, BELL));
        }
        Cue::Open => {
            mix(&mut out, 0.0, &whoosh(0.16, 500.0, 4200.0, 0.30));
            mix(&mut out, 0.12, &note(700.0, 1180.0, 0.09, 0.22, 0.07, SOFT));
            mix(&mut out, 0.19, &note(1400.0, 1400.0, 0.20, 0.10, 0.07, BELL));
        }
        Cue::Close => {
            mix(&mut out, 0.0, &note(1180.0, 640.0, 0.11, 0.20, 0.08, SOFT));
            mix(&mut out, 0.06, &whoosh(0.15, 3200.0, 380.0, 0.22));
        }
        Cue::Error => {
            mix(&mut out, 0.0, &note(523.0, 505.0, 0.12, 0.20, 0.10, ROUND));
            mix(&mut out, 0.13, &note(392.0, 370.0, 0.18, 0.20, 0.12, ROUND));
        }
        Cue::Cancel => {
            mix(&mut out, 0.0, &note(740.0, 410.0, 0.16, 0.17, 0.12, SOFT));
        }
        Cue::Tap => {
            mix(&mut out, 0.0, &note(1900.0, 1700.0, 0.035, 0.09, 0.012, SOFT));
        }
        Cue::Toggle => {
            mix(&mut out, 0.0, &note(990.0, 1180.0, 0.05, 0.10, 0.03, SOFT));
            mix(&mut out, 0.045, &note(1480.0, 1480.0, 0.06, 0.08, 0.03, SOFT));
        }
        Cue::Chime => {
            for (k, f) in [1046.5f32, 1318.5, 1568.0, 2093.0].iter().enumerate() {
                mix(&mut out, k as f32 * 0.11, &note(*f, *f, 0.5, 0.15, 0.16, BELL));
            }
        }
    }
    let fade = len(0.004).min(out.len() / 2);
    let n = out.len();
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        out[i] *= g;
        out[n - 1 - i] *= g;
    }
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 0.6 {
        out.iter_mut().for_each(|v| *v *= 0.6 / peak);
    }
    out
}

fn bank() -> &'static HashMap<Cue, Arc<Vec<f32>>> {
    static B: OnceLock<HashMap<Cue, Arc<Vec<f32>>>> = OnceLock::new();
    B.get_or_init(|| Cue::ALL.iter().map(|c| (*c, Arc::new(synth(*c)))).collect())
}

static LAST_TAP: AtomicU64 = AtomicU64::new(0);

pub fn enabled(core: &Core, c: Cue) -> bool {
    match c {
        Cue::Wake => core.settings.bool("audio.listeningSound"),
        _ => core.settings.bool("audio.sounds"),
    }
}

pub fn play(core: &Core, c: Cue) {
    if !enabled(core, c) {
        return;
    }
    if matches!(c, Cue::Tap | Cue::Toggle) {
        let now = setu_common::protocol::now_ms() as u64;
        if now.saturating_sub(LAST_TAP.swap(now, Ordering::Relaxed)) < 60 {
            return;
        }
    }
    let device = core.settings.str("audio.outputDevice");
    let volume = (core.settings.f64("tts.volume") as f32).clamp(0.15, 1.0);
    let pcm = bank().get(&c).cloned().unwrap_or_default();
    std::thread::spawn(move || {
        let Ok(mut p) = setu_audio::Playback::open(&device, RATE, "Setu sounds") else { return };
        let scaled: Vec<f32> = pcm.iter().map(|s| s * volume).collect();
        let _ = p.write(&scaled);
        let _ = p.write(&vec![0.0; RATE as usize / 20]);
        p.drain();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_is_short_soft_and_clean() {
        for c in Cue::ALL {
            let s = synth(c);
            assert!(!s.is_empty(), "{c:?}");
            assert!(secs(s.len()) < 0.9, "{c:?} is {} s", secs(s.len()));
            let peak = s.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.03 && peak <= 0.61, "{c:?} peak {peak}");
            assert!(s[0].abs() < 0.01 && s[s.len() - 1].abs() < 0.01, "{c:?} clicks");
        }
        assert_eq!(Cue::parse("open"), Some(Cue::Open));
        assert_eq!(Cue::parse("nope"), None);
    }
}
