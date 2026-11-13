use std::f64::consts::PI;

const TAPS: usize = 16;
const PHASES: usize = 256;

pub struct Resampler {
    ratio: f64,
    step: f64,
    pos: f64,
    hist: Vec<f32>,
    table: Vec<f32>,
    passthrough: bool,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let ratio = to as f64 / from as f64;
        let cutoff = ratio.min(1.0) * 0.95;
        let mut table = vec![0f32; PHASES * TAPS * 2];
        for ph in 0..PHASES {
            let frac = ph as f64 / PHASES as f64;
            let mut sum = 0.0;
            let row = &mut table[ph * TAPS * 2..(ph + 1) * TAPS * 2];
            for (k, slot) in row.iter_mut().enumerate() {
                let x = k as f64 - (TAPS as f64 - 1.0) - frac;
                let sinc = if x.abs() < 1e-12 { cutoff } else { (PI * x * cutoff).sin() / (PI * x) };
                let w = 0.42 + 0.5 * (PI * (x / TAPS as f64)).cos() + 0.08 * (2.0 * PI * (x / TAPS as f64)).cos();
                let v = sinc * w;
                *slot = v as f32;
                sum += v;
            }
            if sum.abs() > 1e-12 {
                for slot in row.iter_mut() {
                    *slot = (*slot as f64 / sum) as f32;
                }
            }
        }
        Self {
            ratio,
            step: 1.0 / ratio,
            pos: 0.0,
            hist: vec![0.0; TAPS * 2],
            table,
            passthrough: from == to,
        }
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    pub fn max_output(&self, input: usize) -> usize {
        (input as f64 * self.ratio).ceil() as usize + 2
    }

    pub fn process(&mut self, input: &[f32], out: &mut [f32]) -> usize {
        if self.passthrough {
            let n = input.len().min(out.len());
            out[..n].copy_from_slice(&input[..n]);
            return n;
        }
        let mut written = 0;
        let h = TAPS * 2;
        let mut consumed = 0usize;
        while consumed < input.len() {
            while self.pos < 1.0 && written < out.len() {
                let ph = ((self.pos * PHASES as f64) as usize).min(PHASES - 1);
                let row = &self.table[ph * h..(ph + 1) * h];
                out[written] = self.hist[..h].iter().zip(row).map(|(a, b)| a * b).sum();
                written += 1;
                self.pos += self.step;
            }
            if written >= out.len() && self.pos < 1.0 {
                break;
            }
            self.hist.copy_within(1.., 0);
            self.hist[h - 1] = input[consumed];
            consumed += 1;
            self.pos -= 1.0;
        }
        written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f64, secs: f64) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize).map(|i| (2.0 * PI * hz * i as f64 / rate as f64).sin() as f32 * 0.5).collect()
    }

    fn freq(x: &[f32], rate: u32) -> f64 {
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f64 * rate as f64 / x.len() as f64
    }

    #[test]
    fn downsample_keeps_pitch_and_length() {
        let src = tone(48_000, 440.0, 1.0);
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = vec![0f32; r.max_output(src.len())];
        let mut n = 0;
        for chunk in src.chunks(480) {
            n += r.process(chunk, &mut out[n..]);
        }
        assert!((n as i64 - 16_000).abs() < 40, "{n}");
        let f = freq(&out[200..n], 16_000);
        assert!((f - 440.0).abs() < 5.0, "{f}");
    }

    #[test]
    fn upsample_keeps_pitch() {
        let src = tone(22_050, 300.0, 0.5);
        let mut r = Resampler::new(22_050, 48_000);
        let mut out = vec![0f32; r.max_output(src.len())];
        let n = r.process(&src, &mut out);
        let f = freq(&out[400..n], 48_000);
        assert!((f - 300.0).abs() < 5.0, "{f}");
    }

    #[test]
    fn rejects_aliasing_tone() {
        let src = tone(48_000, 12_000.0, 0.5);
        let mut r = Resampler::new(48_000, 16_000);
        let mut out = vec![0f32; r.max_output(src.len())];
        let n = r.process(&src, &mut out);
        let rms = (out[200..n].iter().map(|v| v * v).sum::<f32>() / (n - 200) as f32).sqrt();
        assert!(rms < 0.05, "{rms}");
    }
}
