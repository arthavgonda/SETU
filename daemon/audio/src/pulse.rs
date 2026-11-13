use crate::{Device, Devices, ErrorSink, FRAME, RATE, Sink};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::process::Command;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

#[repr(C)]
struct PaSimple {
    _p: [u8; 0],
}

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

const FLOAT32LE: c_int = 5;
const PLAYBACK: c_int = 1;
const RECORD: c_int = 2;

unsafe extern "C" {
    fn pa_simple_new(
        server: *const c_char,
        name: *const c_char,
        dir: c_int,
        dev: *const c_char,
        stream_name: *const c_char,
        ss: *const SampleSpec,
        map: *const c_void,
        attr: *const BufferAttr,
        error: *mut c_int,
    ) -> *mut PaSimple;
    fn pa_simple_free(s: *mut PaSimple);
    fn pa_simple_read(s: *mut PaSimple, data: *mut c_void, bytes: usize, error: *mut c_int) -> c_int;
    fn pa_simple_write(s: *mut PaSimple, data: *const c_void, bytes: usize, error: *mut c_int) -> c_int;
    fn pa_simple_drain(s: *mut PaSimple, error: *mut c_int) -> c_int;
    fn pa_simple_flush(s: *mut PaSimple, error: *mut c_int) -> c_int;
    fn pa_strerror(error: c_int) -> *const c_char;
}

fn err(code: c_int) -> String {
    unsafe {
        let p = pa_strerror(code);
        if p.is_null() { format!("audio error {code}") } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

struct Handle(*mut PaSimple);

unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { pa_simple_free(self.0) }
        }
    }
}

fn open(dir: c_int, device: &str, rate: u32, name: &str, attr: BufferAttr) -> Result<Handle, String> {
    let app = CString::new("Setu").unwrap();
    let stream = CString::new(name).map_err(|e| e.to_string())?;
    let dev = if device.is_empty() { None } else { Some(CString::new(device).map_err(|e| e.to_string())?) };
    let spec = SampleSpec { format: FLOAT32LE, rate, channels: 1 };
    let mut code: c_int = 0;
    let p = unsafe {
        pa_simple_new(
            ptr::null(),
            app.as_ptr(),
            dir,
            dev.as_ref().map(|d| d.as_ptr()).unwrap_or(ptr::null()),
            stream.as_ptr(),
            &spec,
            ptr::null(),
            &attr,
            &mut code,
        )
    };
    if p.is_null() { Err(err(code)) } else { Ok(Handle(p)) }
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    device: String,
}

impl Capture {
    pub fn start(device: &str, mut sink: Sink, mut on_error: ErrorSink) -> Result<Self, String> {
        let bytes = (FRAME * 4) as u32;
        let attr = || BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: bytes };
        let mut candidates: Vec<String> = vec![device.to_owned()];
        if device.is_empty() {
            if let Some(d) = pactl(&["get-default-source"]).filter(|d| !d.is_empty()) {
                candidates.push(d);
            }
            candidates.extend(list("sources", "").into_iter().map(|d| d.id));
        }
        let mut last = String::new();
        let mut opened = None;
        for c in candidates {
            match open(RECORD, &c, RATE, "Wake word listener", attr()) {
                Ok(h) => {
                    opened = Some((h, c));
                    break;
                }
                Err(e) => last = e,
            }
        }
        let (h, used) = opened.ok_or(last)?;
        let device = if device.is_empty() { used.as_str() } else { device };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let h = h;
                let mut buf = [0f32; FRAME];
                while !flag.load(Ordering::Relaxed) {
                    let mut code: c_int = 0;
                    let r = unsafe { pa_simple_read(h.0, buf.as_mut_ptr() as *mut c_void, FRAME * 4, &mut code) };
                    if r < 0 {
                        if !flag.load(Ordering::Relaxed) {
                            on_error(err(code));
                        }
                        break;
                    }
                    sink(&buf);
                }
            })
            .map_err(|e| e.to_string())?;
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
            let _ = t.join();
        }
    }
}

pub struct Playback {
    h: Handle,
}

impl Playback {
    pub fn open(device: &str, rate: u32, name: &str) -> Result<Self, String> {
        let tlength = rate * 4 / 10;
        let h = open(
            PLAYBACK,
            device,
            rate,
            name,
            BufferAttr { maxlength: u32::MAX, tlength, prebuf: tlength / 4, minreq: u32::MAX, fragsize: u32::MAX },
        )?;
        Ok(Self { h })
    }

    pub fn write(&mut self, samples: &[f32]) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut code: c_int = 0;
        let r = unsafe { pa_simple_write(self.h.0, samples.as_ptr() as *const c_void, samples.len() * 4, &mut code) };
        if r < 0 { Err(err(code)) } else { Ok(()) }
    }

    pub fn flush(&mut self) {
        let mut code: c_int = 0;
        unsafe { pa_simple_flush(self.h.0, &mut code) };
    }

    pub fn drain(&mut self) {
        let mut code: c_int = 0;
        unsafe { pa_simple_drain(self.h.0, &mut code) };
    }
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn list(kind: &str, default: &str) -> Vec<Device> {
    let Some(text) = pactl(&["-f", "json", "list", kind]) else { return Vec::new() };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    items
        .iter()
        .filter_map(|d| {
            let id = d.get("name")?.as_str()?.to_owned();
            if kind == "sources" && (id.ends_with(".monitor") || id.starts_with(crate::echo::PREFIX) || d.get("monitor_of_sink").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty() && s != "n/a")) {
                return None;
            }
            let name = d.get("description").and_then(|v| v.as_str()).unwrap_or(&id).to_owned();
            Some(Device { is_default: id == default, id, name })
        })
        .collect()
}

pub fn devices() -> Devices {
    let ds = pactl(&["get-default-source"]).unwrap_or_default();
    let dk = pactl(&["get-default-sink"]).unwrap_or_default();
    Devices { inputs: list("sources", &ds), outputs: list("sinks", &dk) }
}
