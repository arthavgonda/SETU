use crate::resample::Resampler;
use crate::ring::{self, Consumer, Producer};
use crate::{Device, Devices, ErrorSink, RATE, Sink};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

fn find(input: bool, id: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if !id.is_empty() {
        let mut list = if input { host.input_devices().ok()? } else { host.output_devices().ok()? };
        if let Some(d) = list.find(|d| d.name().map(|n| n == id).unwrap_or(false)) {
            return Some(d);
        }
    }
    if input { host.default_input_device() } else { host.default_output_device() }
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    device: String,
}

impl Capture {
    pub fn start(device: &str, mut sink: Sink, on_error: ErrorSink) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let id = device.to_owned();
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let Some(dev) = find(true, &id) else {
                    let _ = tx.send(Err("No microphone found.".into()));
                    return;
                };
                let cfg = match dev.default_input_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let channels = cfg.channels() as usize;
                let mut rs = Resampler::new(cfg.sample_rate().0, RATE);
                let mut mono = vec![0f32; 96_000];
                let mut out = vec![0f32; rs.max_output(96_000)];
                let err_cell = Arc::new(std::sync::Mutex::new(Some(on_error)));
                let err_cb = err_cell.clone();
                let format = cfg.sample_format();
                let scfg: cpal::StreamConfig = cfg.into();
                let mut feed = move |frames: &mut dyn Iterator<Item = f32>| {
                    let mut n = 0;
                    let mut acc = 0f32;
                    let mut ch = 0;
                    for v in frames {
                        acc += v;
                        ch += 1;
                        if ch == channels {
                            if n < mono.len() {
                                mono[n] = acc / channels as f32;
                                n += 1;
                            }
                            acc = 0.0;
                            ch = 0;
                        }
                    }
                    let w = rs.process(&mono[..n], &mut out);
                    sink(&out[..w]);
                };
                let on_err = move |e: cpal::StreamError| {
                    if let Ok(mut g) = err_cb.try_lock()
                        && let Some(f) = g.as_mut()
                    {
                        f(e.to_string());
                    }
                };
                let stream = match format {
                    cpal::SampleFormat::F32 => dev.build_input_stream(&scfg, move |d: &[f32], _: &_| feed(&mut d.iter().copied()), on_err, None),
                    cpal::SampleFormat::I16 => dev.build_input_stream(
                        &scfg,
                        move |d: &[i16], _: &_| feed(&mut d.iter().map(|v| *v as f32 / 32768.0)),
                        on_err,
                        None,
                    ),
                    cpal::SampleFormat::U16 => dev.build_input_stream(
                        &scfg,
                        move |d: &[u16], _: &_| feed(&mut d.iter().map(|v| (*v as f32 - 32768.0) / 32768.0)),
                        on_err,
                        None,
                    ),
                    f => {
                        let _ = tx.send(Err(format!("unsupported sample format {f:?}")));
                        return;
                    }
                };
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                if let Err(e) = stream.play() {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
                let _ = tx.send(Ok(()));
                while !flag.load(Ordering::Relaxed) {
                    std::thread::park();
                }
                drop(stream);
                drop(err_cell);
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())??;
        Ok(Self { stop, thread: Some(thread), device: device.to_owned() })
    }

    pub fn device(&self) -> &str {
        &self.device
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

pub struct Playback {
    prod: Producer,
    flush: Arc<AtomicBool>,
    resampler: Resampler,
    scratch: Vec<f32>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

impl Playback {
    pub fn open(device: &str, rate: u32, _name: &str) -> Result<Self, String> {
        let dev = find(false, device).ok_or("No speaker found.")?;
        let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
        let out_rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let (prod, mut cons): (Producer, Consumer) = ring::ring(out_rate as usize * 2);
        let flush = Arc::new(AtomicBool::new(false));
        let fl = flush.clone();
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pend = pending.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let scfg: cpal::StreamConfig = cfg.into();
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || {
                let mut tmp: Vec<f32> = Vec::with_capacity(cons.capacity());
                let stream = dev.build_output_stream(
                    &scfg,
                    move |data: &mut [f32], _: &_| {
                        if fl.swap(false, Ordering::AcqRel) {
                            tmp.clear();
                            let cap = tmp.capacity();
                            cons.pop(&mut tmp, cap);
                        }
                        let frames = data.len() / channels;
                        tmp.clear();
                        cons.pop(&mut tmp, frames);
                        for (i, frame) in data.chunks_mut(channels).enumerate() {
                            let v = tmp.get(i).copied().unwrap_or(0.0);
                            for s in frame.iter_mut() {
                                *s = v;
                            }
                        }
                        pend.store(cons.len(), Ordering::Relaxed);
                    },
                    |_| {},
                    None,
                );
                match stream {
                    Ok(s) => {
                        if let Err(e) = s.play() {
                            let _ = tx.send(Err(e.to_string()));
                            return;
                        }
                        let _ = tx.send(Ok(()));
                        while !flag.load(Ordering::Relaxed) {
                            std::thread::park();
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())??;
        Ok(Self {
            prod,
            flush,
            resampler: Resampler::new(rate, out_rate),
            scratch: Vec::new(),
            stop,
            thread: Some(thread),
            pending,
        })
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        self.scratch.resize(self.resampler.max_output(samples.len()), 0.0);
        let n = self.resampler.process(samples, &mut self.scratch);
        let mut off = 0;
        while off < n {
            let w = self.prod.free().min(n - off);
            if w == 0 {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            self.prod.push(&self.scratch[off..off + w]);
            off += w;
        }
        self.pending.fetch_add(n, Ordering::Relaxed);
        Ok(())
    }

    pub fn flush(&mut self) {
        self.flush.store(true, Ordering::Release);
    }

    pub fn drain(&mut self) {
        for _ in 0..500 {
            if self.pending.load(Ordering::Relaxed) == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

pub fn devices() -> Devices {
    let host = cpal::default_host();
    let di = host.default_input_device().and_then(|d| d.name().ok()).unwrap_or_default();
    let doo = host.default_output_device().and_then(|d| d.name().ok()).unwrap_or_default();
    let map = |list: Option<Vec<cpal::Device>>, def: &str| -> Vec<Device> {
        list.unwrap_or_default()
            .into_iter()
            .filter_map(|d| d.name().ok())
            .map(|n| Device { is_default: n == def, id: n.clone(), name: n })
            .collect()
    };
    Devices {
        inputs: map(host.input_devices().ok().map(|i| i.collect()), &di),
        outputs: map(host.output_devices().ok().map(|i| i.collect()), &doo),
    }
}
