use crate::{Frames, Image};
use anyhow::{Result, anyhow};
use std::ffi::{CString, c_char, c_void};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

type Id = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(cls: Id);
    fn class_addMethod(cls: Id, name: Id, imp: *const c_void, types: *const c_char) -> bool;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: Id;
    static AVCaptureSessionPreset640x480: Id;
    static AVCaptureSessionPresetInputPriority: Id;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(buffer: Id) -> Id;
    fn CMVideoFormatDescriptionGetDimensions(desc: Id) -> Dims;
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Dims {
    w: i32,
    h: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CmTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: Id;
    fn CVPixelBufferLockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: Id) -> *mut u8;
    fn CVPixelBufferGetWidth(pb: Id) -> usize;
    fn CVPixelBufferGetHeight(pb: Id) -> usize;
    fn CVPixelBufferGetBytesPerRow(pb: Id) -> usize;
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: Id) -> Id;
}

const BGRA: u32 = 0x4247_5241;

struct Latest {
    frame: Mutex<(u64, Option<Image>)>,
    ready: Condvar,
}

static LATEST: Latest = Latest { frame: Mutex::new((0, None)), ready: Condvar::new() };
static SINK: OnceLock<usize> = OnceLock::new();

fn sel(n: &str) -> Id {
    let c = CString::new(n).unwrap_or_default();
    unsafe { sel_registerName(c.as_ptr()) }
}

fn class(n: &str) -> Id {
    let c = CString::new(n).unwrap_or_default();
    unsafe { objc_getClass(c.as_ptr()) }
}

fn send0(o: Id, s: &str) -> Id {
    let f: unsafe extern "C" fn(Id, Id) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

fn send1(o: Id, s: &str, a: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Id, Id) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), a) }
}

fn send2(o: Id, s: &str, a: Id, b: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Id, Id, Id) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), a, b) }
}

fn send_bool(o: Id, s: &str, a: bool) {
    let f: unsafe extern "C" fn(Id, Id, bool) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), a) }
}

fn test(o: Id, s: &str, a: Id) -> bool {
    let f: unsafe extern "C" fn(Id, Id, Id) -> bool = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), a) }
}

fn count(a: Id) -> usize {
    let f: unsafe extern "C" fn(Id, Id) -> usize = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    if a.is_null() { 0 } else { unsafe { f(a, sel("count")) } }
}

fn at(a: Id, i: usize) -> Id {
    let f: unsafe extern "C" fn(Id, Id, usize) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(a, sel("objectAtIndex:"), i) }
}

fn number(o: Id, s: &str) -> f64 {
    let f: unsafe extern "C" fn(Id, Id) -> f64 = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

fn fastest_format(device: Id) -> Option<(Id, f64)> {
    let formats = send0(device, "formats");
    let mut best: Option<(Id, f64, i64)> = None;
    for i in 0..count(formats) {
        let fmt = at(formats, i);
        let desc = send0(fmt, "formatDescription");
        if desc.is_null() {
            continue;
        }
        let d = unsafe { CMVideoFormatDescriptionGetDimensions(desc) };
        if d.w < 320 || d.h < 240 {
            continue;
        }
        let ranges = send0(fmt, "videoSupportedFrameRateRanges");
        let fps = (0..count(ranges)).map(|k| number(at(ranges, k), "maxFrameRate")).fold(0.0, f64::max);
        let off = (d.w as i64 - 640).abs() + (d.h as i64 - 480).abs();
        if best.as_ref().is_none_or(|(_, f, o)| off < *o || (off == *o && fps > *f)) {
            best = Some((fmt, fps, off));
        }
    }
    best.filter(|(_, fps, _)| *fps > 0.0).map(|(f, fps, _)| (f, fps))
}

fn use_fastest(session: Id, device: Id) {
    let Some((fmt, fps)) = fastest_format(device) else { return };
    let mut err: Id = std::ptr::null_mut();
    if !test(device, "lockForConfiguration:", &mut err as *mut Id as Id) {
        return;
    }
    unsafe {
        if test(session, "canSetSessionPreset:", AVCaptureSessionPresetInputPriority) {
            send1(session, "setSessionPreset:", AVCaptureSessionPresetInputPriority);
        }
    }
    send1(device, "setActiveFormat:", fmt);
    let frame = CmTime { value: 1, timescale: fps.floor().max(1.0) as i32, flags: 1, epoch: 0 };
    let set: unsafe extern "C" fn(Id, Id, CmTime) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe {
        set(device, sel("setActiveVideoMinFrameDuration:"), frame);
        set(device, sel("setActiveVideoMaxFrameDuration:"), frame);
    }
    send0(device, "unlockForConfiguration");
}

unsafe extern "C" fn on_frame(_this: Id, _cmd: Id, _output: Id, buffer: Id, _connection: Id) {
    unsafe {
        let pb = CMSampleBufferGetImageBuffer(buffer);
        if pb.is_null() || CVPixelBufferLockBaseAddress(pb, 1) != 0 {
            return;
        }
        let (w, h, row) = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb), CVPixelBufferGetBytesPerRow(pb));
        let base = CVPixelBufferGetBaseAddress(pb);
        let img = (!base.is_null() && w > 0 && h > 0 && row >= w * 4).then(|| {
            let data = std::slice::from_raw_parts(base, row * h);
            let mut rgb = vec![0u8; w * h * 3];
            for y in 0..h {
                for x in 0..w {
                    let s = y * row + x * 4;
                    let d = (y * w + x) * 3;
                    rgb[d] = data[s + 2];
                    rgb[d + 1] = data[s + 1];
                    rgb[d + 2] = data[s];
                }
            }
            Image { w, h, rgb }
        });
        CVPixelBufferUnlockBaseAddress(pb, 1);
        if let Some(img) = img {
            let mut g = LATEST.frame.lock().unwrap();
            g.0 += 1;
            g.1 = Some(img);
            LATEST.ready.notify_all();
        }
    }
}

fn sink_class() -> Result<Id> {
    let c = *SINK.get_or_init(|| unsafe {
        let name = CString::new("SetuFrameSink").unwrap_or_default();
        let existing = objc_getClass(name.as_ptr());
        if !existing.is_null() {
            return existing as usize;
        }
        let cls = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
        if cls.is_null() {
            return 0;
        }
        let types = CString::new("v@:@@@").unwrap_or_default();
        class_addMethod(cls, sel("captureOutput:didOutputSampleBuffer:fromConnection:"), on_frame as *const c_void, types.as_ptr());
        objc_registerClassPair(cls);
        cls as usize
    });
    if c == 0 { Err(anyhow!("The camera reader could not be set up.")) } else { Ok(c as Id) }
}

pub struct Camera {
    session: Id,
    seen: u64,
}

unsafe impl Send for Camera {}

impl Camera {
    pub fn open() -> Result<Self> {
        unsafe {
            let device = send1(class("AVCaptureDevice"), "defaultDeviceWithMediaType:", AVMediaTypeVideo);
            if device.is_null() {
                return Err(anyhow!("No camera was found. Connect a camera, then try again."));
            }
            let mut err: Id = std::ptr::null_mut();
            let input = send2(class("AVCaptureDeviceInput"), "deviceInputWithDevice:error:", device, &mut err as *mut Id as Id);
            if input.is_null() {
                return Err(anyhow!("The camera can't be opened. Allow Setu to use the camera in System Settings, Privacy and Security, Camera."));
            }
            let session = send0(send0(class("AVCaptureSession"), "alloc"), "init");
            if test(session, "canSetSessionPreset:", AVCaptureSessionPreset640x480) {
                send1(session, "setSessionPreset:", AVCaptureSessionPreset640x480);
            }
            if !test(session, "canAddInput:", input) {
                send0(session, "release");
                return Err(anyhow!("The camera is busy in another app."));
            }
            send1(session, "addInput:", input);
            use_fastest(session, device);
            let output = send0(send0(class("AVCaptureVideoDataOutput"), "alloc"), "init");
            let fmt: unsafe extern "C" fn(Id, Id, u32) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let number = fmt(class("NSNumber"), sel("numberWithUnsignedInt:"), BGRA);
            let settings = send2(class("NSDictionary"), "dictionaryWithObject:forKey:", number, kCVPixelBufferPixelFormatTypeKey);
            send1(output, "setVideoSettings:", settings);
            send_bool(output, "setAlwaysDiscardsLateVideoFrames:", true);
            let sink = send0(send0(sink_class()?, "alloc"), "init");
            let label = CString::new("ai.setu.camera").unwrap_or_default();
            let queue = dispatch_queue_create(label.as_ptr(), std::ptr::null_mut());
            send2(output, "setSampleBufferDelegate:queue:", sink, queue);
            if !test(session, "canAddOutput:", output) {
                send0(session, "release");
                return Err(anyhow!("The camera's pictures can't be read."));
            }
            send1(session, "addOutput:", output);
            send0(output, "release");
            send0(session, "startRunning");
            let seen = LATEST.frame.lock().unwrap().0;
            Ok(Self { session, seen })
        }
    }
}

impl Frames for Camera {
    fn next(&mut self) -> Result<Option<Image>> {
        let end = Instant::now() + Duration::from_secs(3);
        let mut g = LATEST.frame.lock().unwrap();
        while g.0 == self.seen {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(anyhow!("The camera stopped sending pictures."));
            }
            g = LATEST.ready.wait_timeout(g, left).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
        self.seen = g.0;
        Ok(g.1.clone())
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        send0(self.session, "stopRunning");
        send0(self.session, "release");
        LATEST.frame.lock().unwrap().1 = None;
    }
}
