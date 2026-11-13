use crate::ocr::Image;
use anyhow::{Result, anyhow};
use std::ffi::{CString, c_char, c_int, c_long, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

type Id = *mut c_void;

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(cls: Id);
    fn class_addMethod(cls: Id, name: Id, imp: *const c_void, types: *const c_char) -> bool;
    static _NSConcreteStackBlock: c_void;
}

#[link(name = "ScreenCaptureKit", kind = "framework")]
unsafe extern "C" {
    static SCStreamFrameInfoStatus: Id;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(buffer: Id) -> Id;
    fn CMSampleBufferGetSampleAttachmentsArray(buffer: Id, create: bool) -> Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(a: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
    fn CFDictionaryGetValue(d: *const c_void, key: *const c_void) -> *const c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferLockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(pb: Id, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(pb: Id) -> *mut u8;
    fn CVPixelBufferGetWidth(pb: Id) -> usize;
    fn CVPixelBufferGetHeight(pb: Id) -> usize;
    fn CVPixelBufferGetBytesPerRow(pb: Id) -> usize;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: Id) -> Id;
}

const BGRA: u32 = 0x4247_5241;
const LINGER: Duration = Duration::from_secs(10);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CmTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
struct Descriptor {
    reserved: c_ulong,
    size: c_ulong,
}

#[repr(C)]
struct ContentBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*mut ContentBlock, Id, Id),
    descriptor: *const Descriptor,
    tx: *const Sender<usize>,
}

#[repr(C)]
struct DoneBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*mut DoneBlock, Id),
    descriptor: *const Descriptor,
    tx: *const Sender<bool>,
}

static CONTENT_DESC: Descriptor = Descriptor { reserved: 0, size: std::mem::size_of::<ContentBlock>() as c_ulong };
static DONE_DESC: Descriptor = Descriptor { reserved: 0, size: std::mem::size_of::<DoneBlock>() as c_ulong };

unsafe extern "C" fn got_content(block: *mut ContentBlock, content: Id, _error: Id) {
    unsafe {
        let tx = (*block).tx;
        if !tx.is_null() {
            let kept = if content.is_null() { 0 } else { send0(content, "retain") as usize };
            let _ = (*tx).send(kept);
        }
    }
}

unsafe extern "C" fn got_done(block: *mut DoneBlock, error: Id) {
    unsafe {
        let tx = (*block).tx;
        if !tx.is_null() {
            let _ = (*tx).send(error.is_null());
        }
    }
}

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

fn int(o: Id, s: &str) -> c_long {
    let f: unsafe extern "C" fn(Id, Id) -> c_long = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

fn yes(o: Id, s: &str) -> bool {
    let f: unsafe extern "C" fn(Id, Id) -> bool = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

fn real(o: Id, s: &str) -> f64 {
    let f: unsafe extern "C" fn(Id, Id) -> f64 = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

fn set_usize(o: Id, s: &str, v: usize) {
    let f: unsafe extern "C" fn(Id, Id, usize) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), v) }
}

fn set_u32(o: Id, s: &str, v: u32) {
    let f: unsafe extern "C" fn(Id, Id, u32) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), v) }
}

fn set_bool(o: Id, s: &str, v: bool) {
    let f: unsafe extern "C" fn(Id, Id, bool) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s), v) }
}

#[cfg(target_arch = "aarch64")]
fn rect(o: Id, s: &str) -> Rect {
    let f: unsafe extern "C" fn(Id, Id) -> Rect = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(o, sel(s)) }
}

#[cfg(target_arch = "x86_64")]
fn rect(o: Id, s: &str) -> Rect {
    let f: unsafe extern "C" fn(*mut Rect, Id, Id) = unsafe { std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn()) };
    let mut r = Rect::default();
    unsafe { f(&mut r, o, sel(s)) };
    r
}

fn count(a: Id) -> usize {
    if a.is_null() { 0 } else { int(a, "count").max(0) as usize }
}

fn at(a: Id, i: usize) -> Id {
    let f: unsafe extern "C" fn(Id, Id, usize) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(a, sel("objectAtIndex:"), i) }
}

#[derive(Default)]
struct Latest {
    seq: u64,
    img: Option<std::sync::Arc<Image>>,
}

static LATEST: Mutex<Latest> = Mutex::new(Latest { seq: 0, img: None });
static NEXT: AtomicU64 = AtomicU64::new(1);
static SINK: OnceLock<usize> = OnceLock::new();
static REAPING: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn on_screen(_this: Id, _cmd: Id, _stream: Id, buffer: Id, kind: c_long) {
    if kind != 0 || buffer.is_null() {
        return;
    }
    unsafe {
        let info = CMSampleBufferGetSampleAttachmentsArray(buffer, false);
        if !info.is_null() && CFArrayGetCount(info) > 0 {
            let first = CFArrayGetValueAtIndex(info, 0);
            let status = CFDictionaryGetValue(first, SCStreamFrameInfoStatus) as Id;
            if !status.is_null() && int(status, "integerValue") != 0 {
                return;
            }
        }
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
                let line = &data[y * row..y * row + w * 4];
                let out = &mut rgb[y * w * 3..(y + 1) * w * 3];
                for x in 0..w {
                    out[x * 3] = line[x * 4 + 2];
                    out[x * 3 + 1] = line[x * 4 + 1];
                    out[x * 3 + 2] = line[x * 4];
                }
            }
            Image { w, h, rgb }
        });
        CVPixelBufferUnlockBaseAddress(pb, 1);
        if let Some(img) = img {
            let mut g = LATEST.lock().unwrap_or_else(|e| e.into_inner());
            g.seq = NEXT.fetch_add(1, Ordering::SeqCst);
            g.img = Some(std::sync::Arc::new(img));
        }
    }
}

fn sink_class() -> Result<Id> {
    let c = *SINK.get_or_init(|| unsafe {
        let name = CString::new("SetuScreenSink").unwrap_or_default();
        let existing = objc_getClass(name.as_ptr());
        if !existing.is_null() {
            return existing as usize;
        }
        let cls = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
        if cls.is_null() {
            return 0;
        }
        let types = CString::new("v@:@@q").unwrap_or_default();
        class_addMethod(cls, sel("stream:didOutputSampleBuffer:ofType:"), on_screen as *const c_void, types.as_ptr());
        objc_registerClassPair(cls);
        cls as usize
    });
    if c == 0 { Err(anyhow!("The window reader could not be set up.")) } else { Ok(c as Id) }
}

struct Live {
    pid: c_int,
    stream: Id,
    origin: (f64, f64),
    scale: f64,
    used: Instant,
}

unsafe impl Send for Live {}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

pub struct Frame {
    pub img: Image,
    pub seq: u64,
    pub origin: (f64, f64),
    pub scale: f64,
}

fn shareable() -> Result<Id> {
    let (tx, rx) = channel::<usize>();
    let tx = Box::new(tx);
    let mut block = ContentBlock { isa: unsafe { &_NSConcreteStackBlock as *const c_void }, flags: 0, reserved: 0, invoke: got_content, descriptor: &CONTENT_DESC, tx: &*tx };
    let f: unsafe extern "C" fn(Id, Id, bool, bool, *mut ContentBlock) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { f(class("SCShareableContent"), sel("getShareableContentExcludingDesktopWindows:onScreenWindowsOnly:completionHandler:"), true, true, &mut block) };
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(p) if p != 0 => Ok(p as Id),
        Ok(_) => Err(anyhow!("macOS didn't share the windows. Turn on Screen Recording for Setu in Privacy and Security.")),
        Err(_) => {
            std::mem::forget(tx);
            Err(anyhow!("macOS didn't answer about the windows."))
        }
    }
}

fn front_pid() -> c_int {
    let ws = send0(class("NSWorkspace"), "sharedWorkspace");
    let app = send0(ws, "frontmostApplication");
    if app.is_null() { 0 } else { int(app, "processIdentifier") as c_int }
}

fn open(pid: c_int) -> Result<Live> {
    let content = shareable()?;
    let windows = send0(content, "windows");
    let mut pick: Option<(Id, Rect)> = None;
    for i in 0..count(windows) {
        let w = at(windows, i);
        let app = send0(w, "owningApplication");
        if app.is_null() || int(app, "processID") as c_int != pid || int(w, "windowLayer") != 0 || !yes(w, "isOnScreen") {
            continue;
        }
        let r = rect(w, "frame");
        if r.w < 40.0 || r.h < 40.0 {
            continue;
        }
        if pick.as_ref().is_none_or(|(_, b)| r.w * r.h > b.w * b.h) {
            pick = Some((w, r));
        }
    }
    let Some((window, frame)) = pick else {
        send0(content, "release");
        return Err(anyhow!("The window in front can't be seen."));
    };
    let screen = send0(class("NSScreen"), "mainScreen");
    let scale = if screen.is_null() { 2.0 } else { real(screen, "backingScaleFactor").max(1.0) };
    let filter = send1(send0(class("SCContentFilter"), "alloc"), "initWithDesktopIndependentWindow:", window);
    let config = send0(send0(class("SCStreamConfiguration"), "alloc"), "init");
    set_usize(config, "setWidth:", (frame.w * scale).round().max(1.0) as usize);
    set_usize(config, "setHeight:", (frame.h * scale).round().max(1.0) as usize);
    set_u32(config, "setPixelFormat:", BGRA);
    set_bool(config, "setShowsCursor:", false);
    set_usize(config, "setQueueDepth:", 3);
    let interval = CmTime { value: 1, timescale: 10, flags: 1, epoch: 0 };
    let set_time: unsafe extern "C" fn(Id, Id, CmTime) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { set_time(config, sel("setMinimumFrameInterval:"), interval) };
    let make: unsafe extern "C" fn(Id, Id, Id, Id, Id) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let stream = unsafe { make(send0(class("SCStream"), "alloc"), sel("initWithFilter:configuration:delegate:"), filter, config, std::ptr::null_mut()) };
    send0(filter, "release");
    send0(config, "release");
    send0(content, "release");
    if stream.is_null() {
        return Err(anyhow!("The window reader could not start."));
    }
    let sink = send0(send0(sink_class()?, "alloc"), "init");
    let label = CString::new("ai.setu.screen").unwrap_or_default();
    let queue = unsafe { dispatch_queue_create(label.as_ptr(), std::ptr::null_mut()) };
    let add: unsafe extern "C" fn(Id, Id, Id, c_long, Id, *mut Id) -> bool = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    let mut err: Id = std::ptr::null_mut();
    if !unsafe { add(stream, sel("addStreamOutput:type:sampleHandlerQueue:error:"), sink, 0, queue, &mut err) } {
        send0(stream, "release");
        return Err(anyhow!("The window reader could not start."));
    }
    let (tx, rx) = channel::<bool>();
    let tx = Box::new(tx);
    let mut block = DoneBlock { isa: unsafe { &_NSConcreteStackBlock as *const c_void }, flags: 0, reserved: 0, invoke: got_done, descriptor: &DONE_DESC, tx: &*tx };
    let start: unsafe extern "C" fn(Id, Id, *mut DoneBlock) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { start(stream, sel("startCaptureWithCompletionHandler:"), &mut block) };
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(true) => {}
        Ok(false) => {
            send0(stream, "release");
            return Err(anyhow!("macOS didn't allow the window picture. Turn on Screen Recording for Setu in Privacy and Security."));
        }
        Err(_) => {
            std::mem::forget(tx);
            return Err(anyhow!("The window reader didn't start."));
        }
    }
    Ok(Live { pid, stream, origin: (frame.x, frame.y), scale, used: Instant::now() })
}

fn close(l: Live) {
    let stop: unsafe extern "C" fn(Id, Id, Id) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { stop(l.stream, sel("stopCaptureWithCompletionHandler:"), std::ptr::null_mut()) };
    send0(l.stream, "release");
    LATEST.lock().unwrap_or_else(|e| e.into_inner()).img = None;
}

fn reap() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("setu-maccap".into()).spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            if g.as_ref().is_none_or(|l| l.used.elapsed() >= LINGER) {
                if let Some(l) = g.take() {
                    close(l);
                }
                REAPING.store(false, Ordering::SeqCst);
                return;
            }
        }
    });
}

pub fn frame() -> Result<Frame> {
    let pid = front_pid();
    if pid <= 0 {
        return Err(anyhow!("No app is in front."));
    }
    let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|l| l.pid != pid) {
        if let Some(old) = g.take() {
            close(old);
        }
        LATEST.lock().unwrap_or_else(|e| e.into_inner()).img = None;
        *g = Some(open(pid)?);
        reap();
    }
    let l = g.as_mut().ok_or_else(|| anyhow!("no window picture"))?;
    l.used = Instant::now();
    let (origin, scale) = (l.origin, l.scale);
    drop(g);
    let until = Instant::now() + Duration::from_millis(1500);
    loop {
        {
            let s = LATEST.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(img) = s.img.clone() {
                return Ok(Frame { img: (*img).clone(), seq: s.seq, origin, scale });
            }
        }
        if Instant::now() >= until {
            return Err(anyhow!("The window picture didn't arrive."));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
