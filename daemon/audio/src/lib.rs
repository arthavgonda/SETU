pub mod resample;
pub mod ring;

#[cfg(target_os = "linux")]
mod pulse;
#[cfg(target_os = "linux")]
pub mod echo;
#[cfg(any(not(target_os = "linux"), feature = "portable"))]
mod portable;

use std::process::{Child, Command, Stdio};

pub const RATE: u32 = 16_000;
pub const FRAME: usize = 320;

pub type Sink = Box<dyn FnMut(&[f32]) + Send>;
pub type ErrorSink = Box<dyn FnMut(String) + Send>;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Devices {
    pub inputs: Vec<Device>,
    pub outputs: Vec<Device>,
}

pub struct Capture {
    #[cfg(target_os = "linux")]
    inner: pulse::Capture,
    #[cfg(not(target_os = "linux"))]
    inner: portable::Capture,
}

impl Capture {
    pub fn start(device: &str, sink: Sink, on_error: ErrorSink) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        let inner = pulse::Capture::start(device, sink, on_error)?;
        #[cfg(not(target_os = "linux"))]
        let inner = portable::Capture::start(device, sink, on_error)?;
        Ok(Self { inner })
    }

    pub fn device(&self) -> &str {
        self.inner.device()
    }
}

pub struct Playback {
    #[cfg(target_os = "linux")]
    inner: pulse::Playback,
    #[cfg(not(target_os = "linux"))]
    inner: portable::Playback,
    rate: u32,
}

impl Playback {
    pub fn open(device: &str, rate: u32, name: &str) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        let inner = pulse::Playback::open(device, rate, name)?;
        #[cfg(not(target_os = "linux"))]
        let inner = portable::Playback::open(device, rate, name)?;
        Ok(Self { inner, rate })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        self.inner.write(samples)
    }

    pub fn flush(&mut self) {
        self.inner.flush()
    }

    pub fn drain(&mut self) {
        self.inner.drain()
    }
}

pub fn devices() -> Devices {
    #[cfg(target_os = "linux")]
    return pulse::devices();
    #[cfg(not(target_os = "linux"))]
    return portable::devices();
}

pub struct Watcher {
    child: Option<Child>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watcher {
    pub fn start(mut on_change: Box<dyn FnMut(&str) + Send>) -> Option<Self> {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let mut child = Command::new("pactl")
            .arg("subscribe")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let out = child.stdout.take()?;
        let thread = std::thread::Builder::new()
            .name("audio-watch".into())
            .spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let interesting = (line.contains("'new'") || line.contains("'remove'") || line.contains("'change'"))
                        && (line.contains(" source ") || line.contains(" sink ") || line.contains(" server "));
                    if interesting && !line.contains("source-output") && !line.contains("sink-input") {
                        on_change(&line);
                    }
                }
            })
            .ok()?;
        Some(Self { child: Some(child), thread: Some(thread) })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut acc = 0f32;
    for s in samples {
        acc += s * s;
    }
    (acc / samples.len() as f32).sqrt()
}
