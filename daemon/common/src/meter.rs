use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const LAG: Duration = Duration::from_millis(110);

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

pub struct Meter {
    tx: Option<Sender<(Instant, f32)>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    start: Option<Instant>,
    played: u64,
    rate: u32,
}

impl Meter {
    pub fn new(rate: u32, send: impl Fn(f32) + Send + 'static) -> Self {
        let (tx, rx) = channel::<(Instant, f32)>();
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let thread = std::thread::Builder::new().name("voice-meter".into()).spawn(move || pump(rx, s2, send)).ok();
        Self { tx: Some(tx), stop, thread, start: None, played: 0, rate: rate.max(1) }
    }

    pub fn push(&mut self, samples: &[f32]) {
        let start = *self.start.get_or_insert_with(Instant::now);
        let due = start + Duration::from_secs_f64(self.played as f64 / self.rate as f64) + LAG;
        self.played += samples.len() as u64;
        if let Some(tx) = &self.tx {
            let _ = tx.send((due, rms(samples)));
        }
    }

    pub fn cancel(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.finish();
    }

    pub fn finish(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.finish();
    }
}

fn pump(rx: Receiver<(Instant, f32)>, stop: Arc<AtomicBool>, send: impl Fn(f32)) {
    let mut last = 0.0f32;
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok((due, level)) => {
                while !stop.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    std::thread::sleep((due - now).min(Duration::from_millis(20)));
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let v = (level * 1000.0).round() / 1000.0;
                if (v - last).abs() >= 0.002 || v == 0.0 && last != 0.0 {
                    send(v);
                    last = v;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    send(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn levels_arrive_in_order_and_end_at_zero() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let mut m = Meter::new(1000, move |v| g.lock().unwrap().push(v));
        m.push(&[0.5; 40]);
        m.push(&[0.25; 40]);
        m.finish();
        let v = got.lock().unwrap().clone();
        assert_eq!(v, vec![0.5, 0.25, 0.0]);
        assert!((rms(&[0.3, -0.3]) - 0.3).abs() < 1e-6);
    }
}
