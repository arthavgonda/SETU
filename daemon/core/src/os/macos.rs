use super::{Do, Node, Scope, Win};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::oneshot;

type CFTypeRef = *const c_void;
type CFIndex = isize;
type CFTypeID = usize;
type AXError = i32;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(el: CFTypeRef, attr: CFTypeRef, value: *mut CFTypeRef) -> AXError;
    fn AXUIElementCopyMultipleAttributeValues(el: CFTypeRef, attrs: CFTypeRef, options: u32, values: *mut CFTypeRef) -> AXError;
    fn AXUIElementSetAttributeValue(el: CFTypeRef, attr: CFTypeRef, value: CFTypeRef) -> AXError;
    fn AXUIElementPerformAction(el: CFTypeRef, action: CFTypeRef) -> AXError;
    fn AXUIElementCopyActionNames(el: CFTypeRef, names: *mut CFTypeRef) -> AXError;
    fn AXUIElementGetPid(el: CFTypeRef, pid: *mut i32) -> AXError;
    fn AXUIElementSetMessagingTimeout(el: CFTypeRef, seconds: f32) -> AXError;
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXUIElementGetTypeID() -> CFTypeID;
    static kAXTrustedCheckOptionPrompt: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFURLGetString(url: CFTypeRef) -> CFTypeRef;
    fn CFURLGetTypeID() -> CFTypeID;
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFBooleanGetTypeID() -> CFTypeID;
    fn CFNumberGetTypeID() -> CFTypeID;
    fn CFArrayGetTypeID() -> CFTypeID;
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: CFIndex, encoding: u32, external: u8) -> CFTypeRef;
    fn CFStringGetLength(s: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut u8, size: CFIndex, encoding: u32) -> u8;
    fn CFArrayCreate(alloc: CFTypeRef, values: *const CFTypeRef, n: CFIndex, callbacks: *const c_void) -> CFTypeRef;
    fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    fn CFNumberGetValue(n: CFTypeRef, kind: i32, out: *mut c_void) -> u8;
    fn CFDictionaryCreate(alloc: CFTypeRef, keys: *const CFTypeRef, values: *const CFTypeRef, n: CFIndex, kcb: *const c_void, vcb: *const c_void) -> CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeArrayCallBacks: c_void;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
}

const UTF8: u32 = 0x0800_0100;
const POINT: u32 = 1;
const SIZE: u32 = 2;
const DOUBLE: i32 = 13;

struct Owned(CFTypeRef);

impl Owned {
    fn get(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cfstr(s: &str) -> Owned {
    Owned(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as CFIndex, UTF8, 0) })
}

fn to_string(v: CFTypeRef) -> Option<String> {
    unsafe {
        if v.is_null() || CFGetTypeID(v) != CFStringGetTypeID() {
            return None;
        }
        let len = CFStringGetLength(v);
        let cap = CFStringGetMaximumSizeForEncoding(len, UTF8) + 1;
        let mut buf = vec![0u8; cap.max(1) as usize];
        if CFStringGetCString(v, buf.as_mut_ptr(), cap, UTF8) == 0 {
            return None;
        }
        let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
        Some(String::from_utf8_lossy(&buf[..end]).into_owned())
    }
}

fn to_bool(v: CFTypeRef) -> Option<bool> {
    unsafe {
        if v.is_null() {
            return None;
        }
        if CFGetTypeID(v) == CFBooleanGetTypeID() {
            return Some(CFBooleanGetValue(v) != 0);
        }
        if CFGetTypeID(v) == CFNumberGetTypeID() {
            let mut d = 0f64;
            if CFNumberGetValue(v, DOUBLE, &mut d as *mut f64 as *mut c_void) != 0 {
                return Some(d != 0.0);
            }
        }
        None
    }
}

fn to_number(v: CFTypeRef) -> Option<f64> {
    unsafe {
        if v.is_null() || CFGetTypeID(v) != CFNumberGetTypeID() {
            return None;
        }
        let mut d = 0f64;
        (CFNumberGetValue(v, DOUBLE, &mut d as *mut f64 as *mut c_void) != 0).then_some(d)
    }
}

fn attr(el: CFTypeRef, name: &str) -> Option<Owned> {
    let key = cfstr(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(el, key.get(), &mut out) };
    (err == 0 && !out.is_null()).then(|| Owned(out))
}

fn elements(v: &Owned) -> Vec<CFTypeRef> {
    unsafe {
        if CFGetTypeID(v.get()) != CFArrayGetTypeID() {
            return Vec::new();
        }
        (0..CFArrayGetCount(v.get())).map(|i| CFArrayGetValueAtIndex(v.get(), i)).filter(|e| !e.is_null() && CFGetTypeID(*e) == AXUIElementGetTypeID()).collect()
    }
}

fn set_attr(el: CFTypeRef, name: &str, value: CFTypeRef) -> bool {
    let key = cfstr(name);
    unsafe { AXUIElementSetAttributeValue(el, key.get(), value) == 0 }
}

fn perform(el: CFTypeRef, action: &str) -> bool {
    let a = cfstr(action);
    unsafe { AXUIElementPerformAction(el, a.get()) == 0 }
}

fn actions(el: CFTypeRef) -> Vec<String> {
    let mut out: CFTypeRef = std::ptr::null();
    if unsafe { AXUIElementCopyActionNames(el, &mut out) } != 0 || out.is_null() {
        return Vec::new();
    }
    let arr = Owned(out);
    unsafe { (0..CFArrayGetCount(arr.get())).filter_map(|i| to_string(CFArrayGetValueAtIndex(arr.get(), i))).collect() }
}

fn pid_of(el: CFTypeRef) -> i32 {
    let mut p = 0;
    unsafe {
        AXUIElementGetPid(el, &mut p);
    }
    p
}

enum Job {
    Windows(oneshot::Sender<Result<Vec<Win>>>),
    Tree(Win, Scope, oneshot::Sender<Result<Vec<Node>>>),
    Act(Node, Do, oneshot::Sender<Result<()>>),
    Focused(Win, oneshot::Sender<Result<Option<Node>>>),
    Wake(Win),
    Popups(Win, oneshot::Sender<Result<Vec<(Win, Vec<Node>)>>>),
    Text(Node, usize, oneshot::Sender<Result<Option<String>>>),
    Caret(Node, oneshot::Sender<Result<Option<usize>>>),
}

static JOBS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

fn jobs() -> &'static mpsc::Sender<Job> {
    JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new().name("setu-ax".into()).spawn(move || worker(rx)).expect("ax thread");
        tx
    })
}

async fn ask<T>(make: impl FnOnce(oneshot::Sender<Result<T>>) -> Job) -> Result<T> {
    let (tx, rx) = oneshot::channel();
    jobs().send(make(tx)).map_err(|_| anyhow!("Accessibility stopped."))?;
    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err(anyhow!("Accessibility stopped.")),
        Err(_) => Err(anyhow!("The app didn't answer in time.")),
    }
}

pub async fn ready() -> Result<()> {
    if unsafe { AXIsProcessTrusted() } != 0 {
        return Ok(());
    }
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let d = CFDictionaryCreate(std::ptr::null(), keys.as_ptr(), values.as_ptr(), 1, &kCFTypeDictionaryKeyCallBacks as *const c_void, &kCFTypeDictionaryValueCallBacks as *const c_void);
        let _ = AXIsProcessTrustedWithOptions(d);
        if !d.is_null() {
            CFRelease(d);
        }
    }
    Err(anyhow!("Allow Setu in System Settings, Privacy & Security, Accessibility."))
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

fn focused_window() -> Result<Owned> {
    if !trusted() {
        return Err(anyhow!("Allow Setu in System Settings, Privacy & Security, Accessibility."));
    }
    let sys = Owned(unsafe { AXUIElementCreateSystemWide() });
    let app = attr(sys.get(), "AXFocusedApplication").ok_or_else(|| anyhow!("No app is in front."))?;
    attr(app.get(), "AXFocusedWindow").or_else(|| attr(app.get(), "AXMainWindow")).ok_or_else(|| anyhow!("The app in front has no window."))
}

pub fn window_rect() -> Result<(f64, f64, f64, f64)> {
    let w = focused_window()?;
    let mut p = CGPoint::default();
    let mut s = CGSize::default();
    let pv = attr(w.get(), "AXPosition").ok_or_else(|| anyhow!("I can't read this window's place."))?;
    let sv = attr(w.get(), "AXSize").ok_or_else(|| anyhow!("I can't read this window's size."))?;
    let ok = unsafe { AXValueGetValue(pv.get(), POINT, &mut p as *mut CGPoint as *mut c_void) != 0 && AXValueGetValue(sv.get(), SIZE, &mut s as *mut CGSize as *mut c_void) != 0 };
    if !ok {
        return Err(anyhow!("I can't read this window's place."));
    }
    Ok((p.x, p.y, s.width, s.height))
}

pub fn set_window_rect(x: f64, y: f64, w: f64, h: f64) -> Result<()> {
    let win = focused_window()?;
    let p = CGPoint { x, y };
    let s = CGSize { width: w, height: h };
    let pv = Owned(unsafe { AXValueCreate(POINT, &p as *const CGPoint as *const c_void) });
    let sv = Owned(unsafe { AXValueCreate(SIZE, &s as *const CGSize as *const c_void) });
    if pv.get().is_null() || sv.get().is_null() {
        return Err(anyhow!("I couldn't move the window."));
    }
    let a = set_attr(win.get(), "AXSize", sv.get());
    let b = set_attr(win.get(), "AXPosition", pv.get());
    let c = set_attr(win.get(), "AXSize", sv.get());
    if a || b || c { Ok(()) } else { Err(anyhow!("This window can't be moved or resized.")) }
}

pub fn minimise_window() -> Result<()> {
    let win = focused_window()?;
    if unsafe { set_attr(win.get(), "AXMinimized", kCFBooleanTrue) } { Ok(()) } else { Err(anyhow!("This window can't be minimised.")) }
}

pub fn screen_rect() -> (f64, f64, f64, f64) {
    let r = unsafe { CGDisplayBounds(CGMainDisplayID()) };
    if r.size.width > 0.0 && r.size.height > 0.0 { (r.origin.x, r.origin.y, r.size.width, r.size.height) } else { (0.0, 0.0, 1440.0, 900.0) }
}

pub fn track(on: fn(Win)) {
    let _ = std::thread::Builder::new().name("setu-front".into()).spawn(move || {
        let mut last = 0i32;
        loop {
            let sys = Owned(unsafe { AXUIElementCreateSystemWide() });
            if let Some(app) = attr(sys.get(), "AXFocusedApplication") {
                let pid = pid_of(app.get());
                if pid > 0 && pid != last {
                    last = pid;
                    wake_app(pid);
                    let name = attr(app.get(), "AXTitle").and_then(|t| to_string(t.get())).unwrap_or_default();
                    let title = attr(app.get(), "AXFocusedWindow").and_then(|w| attr(w.get(), "AXTitle")).and_then(|t| to_string(t.get())).unwrap_or_default();
                    on(Win { id: String::new(), app: name, title, pid: pid as u32, front: true });
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

pub fn evented() -> bool {
    false
}

pub async fn wake(win: &Win) {
    let _ = jobs().send(Job::Wake(win.clone()));
}

pub async fn windows() -> Result<Vec<Win>> {
    ready().await?;
    let mut w: Vec<Win> = ask(Job::Windows).await?;
    w.retain(|x| !x.title.is_empty() || x.front);
    Ok(w)
}

pub async fn front() -> Result<Win> {
    windows().await?.into_iter().find(|w| w.front).ok_or_else(|| anyhow!("No window is active."))
}

pub async fn tree(win: &Win, scope: Scope) -> Result<Vec<Node>> {
    let w = win.clone();
    ask(move |tx| Job::Tree(w, scope, tx)).await
}

pub async fn act(node: &Node, what: Do) -> Result<()> {
    let n = node.clone();
    ask(move |tx| Job::Act(n, what, tx)).await
}

pub async fn link_url(node: &Node) -> Option<String> {
    (node.role == "link" && !node.value.is_empty()).then(|| node.value.clone())
}

pub async fn focused(win: &Win) -> Result<Option<Node>> {
    let w = win.clone();
    ask(move |tx| Job::Focused(w, tx)).await
}

pub async fn node_attr(node: &Node, key: &str) -> Option<String> {
    (matches!(key, "src" | "url") && matches!(node.role.as_str(), "image" | "link") && !node.value.is_empty()).then(|| node.value.clone())
}

pub async fn find_text(_win: &Win, _text: &str) -> Result<bool> {
    Err(anyhow!("Find works through the keyboard on macOS."))
}

pub async fn popups(win: &Win) -> Result<Vec<(Win, Vec<Node>)>> {
    let w = win.clone();
    ask(move |tx| Job::Popups(w, tx)).await
}

pub async fn node_text(node: &Node, max: usize) -> Option<String> {
    let n = node.clone();
    ask(move |tx| Job::Text(n, max, tx)).await.ok().flatten()
}

pub async fn caret(node: &Node) -> Option<usize> {
    let n = node.clone();
    ask(move |tx| Job::Caret(n, tx)).await.ok().flatten()
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CFRange {
    location: CFIndex,
    length: CFIndex,
}

const RANGE: u32 = 4;

fn caret_at(el: CFTypeRef) -> Option<usize> {
    let v = attr(el, "AXSelectedTextRange")?;
    let mut r = CFRange::default();
    unsafe {
        if CFGetTypeID(v.get()) != AXValueGetTypeID() || AXValueGetValue(v.get(), RANGE, &mut r as *mut CFRange as *mut c_void) == 0 {
            return None;
        }
    }
    (r.location >= 0).then_some(r.location as usize)
}

fn gui_apps() -> Vec<(String, i32)> {
    let script = "JSON.stringify(Application('System Events').processes.whose({backgroundOnly: false})().map(p => [p.name(), p.unixId()]))";
    let Ok(o) = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", script]).output() else { return Vec::new() };
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap_or_default();
    v.as_array()
        .map(|a| a.iter().filter_map(|x| Some((x.get(0)?.as_str()?.to_owned(), x.get(1)?.as_i64()? as i32))).collect())
        .unwrap_or_default()
}

struct State {
    cache: HashMap<i32, (u64, Vec<Owned>)>,
    windows: HashMap<String, Owned>,
    generation: u64,
}

fn worker(rx: mpsc::Receiver<Job>) {
    let mut st = State { cache: HashMap::new(), windows: HashMap::new(), generation: 0 };
    while let Ok(job) = rx.recv() {
        match job {
            Job::Windows(tx) => drop(tx.send(Ok(list(&mut st)))),
            Job::Tree(w, scope, tx) => drop(tx.send(build(&mut st, &w, scope))),
            Job::Act(n, d, tx) => drop(tx.send(do_act(&mut st, &n, d))),
            Job::Focused(w, tx) => drop(tx.send(build(&mut st, &w, Scope { limit: 2000, text: false, hidden: false }).map(|v| v.into_iter().find(|n| n.focused && n.editable)))),
            Job::Wake(w) => wake_app(w.pid as i32),
            Job::Popups(w, tx) => drop(tx.send(popup_nodes(&mut st, &w))),
            Job::Text(n, max, tx) => drop(tx.send(Ok(element(&mut st, &n).ok().and_then(|el| attr(el, "AXValue")).and_then(|v| to_string(v.get())).map(|t| t.chars().take(max).collect::<String>()).filter(|t| !t.is_empty())))),
            Job::Caret(n, tx) => drop(tx.send(Ok(element(&mut st, &n).ok().and_then(caret_at)))),
        }
    }
}

fn wake_app(pid: i32) {
    if pid <= 0 {
        return;
    }
    let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    unsafe {
        let _ = set_attr(app.get(), "AXManualAccessibility", kCFBooleanTrue);
    }
}

fn list(st: &mut State) -> Vec<Win> {
    let sys = Owned(unsafe { AXUIElementCreateSystemWide() });
    let front_pid = attr(sys.get(), "AXFocusedApplication").map(|a| pid_of(a.get())).unwrap_or(0);
    let mut pids: Vec<(String, i32)> = gui_apps();
    if front_pid > 0 && !pids.iter().any(|(_, p)| *p == front_pid) {
        pids.push((String::new(), front_pid));
    }
    st.windows.clear();
    let mut out = Vec::new();
    for (name, pid) in pids {
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        unsafe {
            AXUIElementSetMessagingTimeout(app.get(), 1.5);
        }
        let app_name = if name.is_empty() { attr(app.get(), "AXTitle").and_then(|t| to_string(t.get())).unwrap_or_default() } else { name };
        let focused = attr(app.get(), "AXFocusedWindow");
        let Some(ws) = attr(app.get(), "AXWindows") else { continue };
        for (i, w) in elements(&ws).into_iter().enumerate() {
            let title = attr(w, "AXTitle").and_then(|t| to_string(t.get())).unwrap_or_default();
            let is_focused = focused.as_ref().is_some_and(|f| unsafe { CFEqual(f.get(), w) } != 0);
            let id = format!("ax:{pid}:w{i}");
            st.windows.insert(id.clone(), Owned(unsafe { CFRetain(w) }));
            out.push(Win { id, app: app_name.clone(), title, pid: pid as u32, front: pid == front_pid && is_focused });
        }
    }
    if !out.iter().any(|w| w.front)
        && let Some(w) = out.iter_mut().find(|w| w.pid as i32 == front_pid)
    {
        w.front = true;
    }
    out
}

const ATTRS: &[&str] = &["AXRole", "AXSubrole", "AXTitle", "AXDescription", "AXValue", "AXEnabled", "AXFocused", "AXPosition", "AXSize", "AXChildren", "AXPlaceholderValue", "AXURL", "AXSelected"];

fn role_of(role: &str, subrole: &str) -> &'static str {
    match (role, subrole) {
        (_, "AXSecureTextField") => "password",
        (_, "AXTabButton") => "tab",
        ("AXButton" | "AXMenuButton" | "AXPopUpButton" | "AXDisclosureTriangle", _) => "button",
        ("AXCheckBox", "AXSwitch") | ("AXCheckBox", "AXToggle") => "toggle",
        ("AXCheckBox", _) => "checkbox",
        ("AXRadioButton", _) => "radio",
        ("AXLink", _) => "link",
        ("AXTextField" | "AXTextArea" | "AXSearchField", _) => "field",
        ("AXComboBox", _) => "combobox",
        ("AXMenuItem" | "AXMenuBarItem", _) => "menuitem",
        ("AXRow", _) => "row",
        ("AXCell", _) => "cell",
        ("AXOutlineRow", _) => "treeitem",
        ("AXSlider", _) => "slider",
        ("AXIncrementor" | "AXStepper", _) => "spinner",
        ("AXStaticText", _) => "text",
        ("AXHeading", _) => "heading",
        ("AXImage", _) => "image",
        ("AXWebArea", _) => "document",
        ("AXListItem" | "AXListBoxOption", _) => "item",
        ("AXProgressIndicator" | "AXBusyIndicator", _) => "progress",
        _ => "other",
    }
}

struct Info {
    role: &'static str,
    name: String,
    value: String,
    enabled: bool,
    focused: bool,
    rect: (i32, i32, i32, i32),
    checked: Option<bool>,
    children: Vec<CFTypeRef>,
    keep_children: Option<Owned>,
    editable: bool,
    selected: bool,
}

fn read(el: CFTypeRef) -> Option<Info> {
    let names: Vec<Owned> = ATTRS.iter().map(|a| cfstr(a)).collect();
    let raw: Vec<CFTypeRef> = names.iter().map(|n| n.get()).collect();
    let arr = Owned(unsafe { CFArrayCreate(std::ptr::null(), raw.as_ptr(), raw.len() as CFIndex, &kCFTypeArrayCallBacks as *const c_void) });
    let mut out: CFTypeRef = std::ptr::null();
    if unsafe { AXUIElementCopyMultipleAttributeValues(el, arr.get(), 0, &mut out) } != 0 || out.is_null() {
        return None;
    }
    let vals = Owned(out);
    let at = |i: usize| -> CFTypeRef { unsafe { if (i as CFIndex) < CFArrayGetCount(vals.get()) { CFArrayGetValueAtIndex(vals.get(), i as CFIndex) } else { std::ptr::null() } } };
    let role_s = to_string(at(0)).unwrap_or_default();
    let sub = to_string(at(1)).unwrap_or_default();
    let role = role_of(&role_s, &sub);
    let title = to_string(at(2)).unwrap_or_default();
    let desc = to_string(at(3)).unwrap_or_default();
    let placeholder = to_string(at(10)).unwrap_or_default();
    let value_ref = at(4);
    let value = to_string(value_ref).unwrap_or_else(|| to_number(value_ref).map(|n| if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") }).unwrap_or_default());
    let name = if !title.trim().is_empty() {
        title
    } else if !desc.trim().is_empty() {
        desc
    } else if role == "text" || role == "heading" {
        value.clone()
    } else {
        placeholder
    };
    let mut p = CGPoint::default();
    let mut s = CGSize::default();
    unsafe {
        let pv = at(7);
        if !pv.is_null() && CFGetTypeID(pv) == AXValueGetTypeID() {
            AXValueGetValue(pv, POINT, &mut p as *mut CGPoint as *mut c_void);
        }
        let sv = at(8);
        if !sv.is_null() && CFGetTypeID(sv) == AXValueGetTypeID() {
            AXValueGetValue(sv, SIZE, &mut s as *mut CGSize as *mut c_void);
        }
    }
    let kids_ref = at(9);
    let children: Vec<CFTypeRef> = unsafe {
        if !kids_ref.is_null() && CFGetTypeID(kids_ref) == CFArrayGetTypeID() {
            (0..CFArrayGetCount(kids_ref)).map(|i| CFArrayGetValueAtIndex(kids_ref, i)).filter(|e| !e.is_null() && CFGetTypeID(*e) == AXUIElementGetTypeID()).collect()
        } else {
            Vec::new()
        }
    };
    let checked = matches!(role, "checkbox" | "radio" | "toggle").then(|| to_bool(value_ref).unwrap_or(false));
    let url_ref = at(11);
    let url = unsafe { if !url_ref.is_null() && CFGetTypeID(url_ref) == CFURLGetTypeID() { to_string(CFURLGetString(url_ref)) } else { None } };
    let selected = to_bool(at(12)).unwrap_or(false) || (role == "tab" && to_bool(value_ref).unwrap_or(false));
    Some(Info {
        role,
        name: name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned(),
        value: if matches!(role, "field" | "combobox" | "slider" | "spinner") { value.chars().take(500).collect() } else if role == "link" || role == "image" { url.unwrap_or_default() } else { String::new() },
        enabled: to_bool(at(5)).unwrap_or(true),
        focused: to_bool(at(6)).unwrap_or(false),
        rect: (p.x.round() as i32, p.y.round() as i32, s.width.round() as i32, s.height.round() as i32),
        checked,
        editable: matches!(role, "field" | "combobox"),
        children,
        keep_children: Some(vals),
        selected,
    })
}

const CLICKY: &[&str] = &["button", "toggle", "checkbox", "radio", "link", "field", "password", "combobox", "tab", "menuitem", "item", "row", "treeitem", "spinner", "slider"];

fn build(st: &mut State, win: &Win, scope: Scope) -> Result<Vec<Node>> {
    let root = match st.windows.get(&win.id) {
        Some(w) => Owned(unsafe { CFRetain(w.get()) }),
        None => {
            list(st);
            let w = st.windows.get(&win.id).ok_or_else(|| anyhow!("That window is gone."))?;
            Owned(unsafe { CFRetain(w.get()) })
        }
    };
    let pid = win.pid as i32;
    if scope.hidden {
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        unsafe {
            AXUIElementSetMessagingTimeout(app.get(), 1.5);
        }
        if let Some(bar) = attr(app.get(), "AXMenuBar") {
            return Ok(collect(st, &[root.get(), bar.get()], pid, scope));
        }
    }
    Ok(collect(st, &[root.get()], pid, scope))
}

fn collect(st: &mut State, roots: &[CFTypeRef], key: i32, scope: Scope) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut keep: Vec<Owned> = Vec::new();
    let mut stack: Vec<(CFTypeRef, usize, bool)> = roots.iter().rev().map(|r| (*r, 0, false)).collect();
    let mut visited = 0usize;
    let mut holders: Vec<Owned> = Vec::new();
    while let Some((el, depth, in_clicky)) = stack.pop() {
        visited += 1;
        if visited > 6000 || nodes.len() >= scope.limit.max(1) {
            break;
        }
        let Some(info) = read(el) else { continue };
        let clicky = CLICKY.contains(&info.role);
        let show = scope.hidden || (info.rect.2 >= 2 && info.rect.3 >= 2);
        let named = !info.name.is_empty();
        let wanted = show && (clicky || info.role == "progress" || (scope.text && named && matches!(info.role, "text" | "heading" | "image" | "cell")) || (!in_clicky && named && matches!(info.role, "cell" | "image")));
        if wanted && depth > 0 {
            keep.push(Owned(unsafe { CFRetain(el) }));
            nodes.push(Node {
                id: String::new(),
                role: info.role.into(),
                name: info.name.clone(),
                value: info.value.clone(),
                x: info.rect.0,
                y: info.rect.1,
                w: info.rect.2,
                h: info.rect.3,
                focused: info.focused,
                checked: info.checked,
                editable: info.editable,
                enabled: info.enabled,
                web: false,
                selected: info.selected,
            });
        }
        if depth < 60 {
            for k in info.children.iter().rev() {
                stack.push((*k, depth + 1, in_clicky || clicky));
            }
        }
        if let Some(h) = info.keep_children {
            holders.push(h);
        }
    }
    st.generation += 1;
    let generation = st.generation;
    for (i, n) in nodes.iter_mut().enumerate() {
        n.id = format!("ax:{key}:{generation}:{i}");
    }
    st.cache.insert(key, (generation, keep));
    drop(holders);
    nodes
}

fn popup_nodes(st: &mut State, win: &Win) -> Result<Vec<(Win, Vec<Node>)>> {
    let pid = win.pid as i32;
    if pid <= 0 {
        return Ok(Vec::new());
    }
    let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
    unsafe {
        AXUIElementSetMessagingTimeout(app.get(), 1.5);
    }
    if !st.windows.contains_key(&win.id) {
        list(st);
    }
    let main = st.windows.get(&win.id).map(|w| w.get());
    let Some(kids) = attr(app.get(), "AXChildren") else { return Ok(Vec::new()) };
    let roots: Vec<CFTypeRef> = elements(&kids).into_iter().filter(|k| main.is_none_or(|m| unsafe { CFEqual(*k, m) } == 0)).collect();
    let mut out = Vec::new();
    for (i, r) in roots.into_iter().take(8).enumerate() {
        let key = -(pid.saturating_mul(32).saturating_add(i as i32 + 1));
        let nodes = collect(st, &[r], key, Scope { limit: 400, text: false, hidden: false });
        if nodes.is_empty() {
            continue;
        }
        let title = attr(r, "AXTitle").and_then(|t| to_string(t.get())).unwrap_or_default();
        out.push((Win { id: format!("ax:{pid}:p{i}"), app: win.app.clone(), title, pid: win.pid, front: false }, nodes));
    }
    Ok(out)
}

fn element(st: &mut State, node: &Node) -> Result<CFTypeRef> {
    let mut parts = node.id.strip_prefix("ax:").ok_or_else(|| anyhow!("That control is no longer there."))?.split(':');
    let pid: i32 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let generation: u64 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let index: usize = parts.next().and_then(|x| x.parse().ok()).unwrap_or(usize::MAX);
    match st.cache.get(&pid) {
        Some((g, els)) if *g == generation => els.get(index).map(|e| e.get()).ok_or_else(|| anyhow!("That control is no longer there.")),
        _ => Err(anyhow!("{} changed. Ask me again.", if node.name.is_empty() { "The screen" } else { node.name.as_str() })),
    }
}

fn keys(el: CFTypeRef, text: &str, replace: bool) -> Result<()> {
    unsafe {
        let _ = set_attr(el, "AXFocused", kCFBooleanTrue);
    }
    std::thread::sleep(Duration::from_millis(120));
    if replace {
        crate::tools::input::imp_press(&[crate::tools::input::K::Meta, crate::tools::input::K::Ch('a')])?;
    }
    crate::tools::input::imp_type(text)
}

fn do_act(st: &mut State, node: &Node, what: Do) -> Result<()> {
    let el = element(st, node)?;
    let label = if node.name.is_empty() { "That control".to_owned() } else { node.name.clone() };
    let acts = actions(el);
    let has = |a: &str| acts.iter().any(|x| x == a);
    match what {
        Do::Focus => {
            if unsafe { set_attr(el, "AXFocused", kCFBooleanTrue) } { Ok(()) } else { Err(anyhow!("{label} can't take the focus.")) }
        }
        Do::SetText(t) => {
            let v = cfstr(&t);
            if set_attr(el, "AXValue", v.get()) {
                return Ok(());
            }
            keys(el, &t, true)
        }
        Do::Insert(t) => {
            unsafe {
                let _ = set_attr(el, "AXFocused", kCFBooleanTrue);
            }
            let v = cfstr(&t);
            if set_attr(el, "AXSelectedText", v.get()) {
                return Ok(());
            }
            keys(el, &t, false)
        }
        Do::Select if unsafe { set_attr(el, "AXSelected", kCFBooleanTrue) } => Ok(()),
        Do::Expand if has("AXShowMenu") && perform(el, "AXShowMenu") => Ok(()),
        Do::Menu => {
            if has("AXShowMenu") && perform(el, "AXShowMenu") { Ok(()) } else { Err(anyhow!("{label} has no menu.")) }
        }
        _ => {
            for a in ["AXPress", "AXPick", "AXConfirm", "AXOpen"] {
                if has(a) && perform(el, a) {
                    return Ok(());
                }
            }
            if perform(el, "AXPress") {
                return Ok(());
            }
            unsafe {
                let _ = set_attr(el, "AXFocused", kCFBooleanTrue);
            }
            std::thread::sleep(Duration::from_millis(80));
            let key = if matches!(node.role.as_str(), "checkbox" | "radio" | "toggle") { crate::tools::input::K::Space } else { crate::tools::input::K::Enter };
            crate::tools::input::imp_press(&[key]).map_err(|_| anyhow!("{label} didn't respond."))
        }
    }
}

pub async fn watch(_secs: u64) -> bool {
    false
}
