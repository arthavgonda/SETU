use super::{Do, Node, Scope};
use anyhow::{Result, anyhow};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetClassNameW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};
use windows::core::{PCSTR, PCWSTR};

type JObject = i64;
type JBool = i32;

const MAX_STRING: usize = 1024;
const SHORT_STRING: usize = 256;

#[repr(C)]
struct ContextInfo {
    name: [u16; MAX_STRING],
    description: [u16; MAX_STRING],
    role: [u16; SHORT_STRING],
    role_en: [u16; SHORT_STRING],
    states: [u16; SHORT_STRING],
    states_en: [u16; SHORT_STRING],
    index_in_parent: i32,
    children: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    component: JBool,
    action: JBool,
    selection: JBool,
    text: JBool,
    interfaces: JBool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ActionName {
    name: [u16; SHORT_STRING],
}

#[repr(C)]
struct Actions {
    count: i32,
    list: [ActionName; 256],
}

#[repr(C)]
struct ToDo {
    count: i32,
    list: [ActionName; 32],
}

#[repr(C)]
#[derive(Default)]
struct TextInfo {
    chars: i32,
    caret: i32,
    at_point: i32,
}

const _: () = assert!(std::mem::size_of::<ContextInfo>() == 6188);
const _: () = assert!(std::mem::size_of::<Actions>() == 4 + 256 * 512);
const _: () = assert!(std::mem::size_of::<ToDo>() == 4 + 32 * 512);

struct Api {
    run: unsafe extern "C" fn(),
    is_java: unsafe extern "C" fn(HWND) -> JBool,
    from_hwnd: unsafe extern "C" fn(HWND, *mut i32, *mut JObject) -> JBool,
    info: unsafe extern "C" fn(i32, JObject, *mut ContextInfo) -> JBool,
    child: unsafe extern "C" fn(i32, JObject, i32) -> JObject,
    release: unsafe extern "C" fn(i32, JObject),
    actions: unsafe extern "C" fn(i32, JObject, *mut Actions) -> JBool,
    do_actions: unsafe extern "C" fn(i32, JObject, *mut ToDo, *mut i32) -> JBool,
    focus: unsafe extern "C" fn(i32, JObject) -> JBool,
    set_text: unsafe extern "C" fn(i32, JObject, *const u16) -> JBool,
    text_info: unsafe extern "C" fn(i32, JObject, *mut TextInfo, i32, i32) -> JBool,
    text_range: unsafe extern "C" fn(i32, JObject, i32, i32, *mut u16, i16) -> JBool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn dll_candidates(hint: Option<HWND>) -> Vec<String> {
    let mut v = vec!["WindowsAccessBridge-64.dll".to_owned()];
    if let Some(h) = hint {
        let exe = crate::tools::win::exe_path_of(h);
        if let Some(dir) = std::path::Path::new(&exe).parent() {
            for rel in ["..\\jbr\\bin", "..\\jre\\bin", "..\\jre64\\bin", "."] {
                v.push(dir.join(rel).join("windowsaccessbridge-64.dll").to_string_lossy().into_owned());
            }
        }
    }
    if let Some(j) = std::env::var_os("JAVA_HOME") {
        v.push(std::path::PathBuf::from(j).join("bin").join("windowsaccessbridge-64.dll").to_string_lossy().into_owned());
    }
    v
}

unsafe fn load(hint: Option<HWND>) -> Option<Api> {
    for path in dll_candidates(hint) {
        let w = wide(&path);
        let Ok(lib) = (unsafe { LoadLibraryW(PCWSTR(w.as_ptr())) }) else { continue };
        let sym = |name: &str| -> Option<unsafe extern "system" fn() -> isize> {
            let c: Vec<u8> = name.bytes().chain(std::iter::once(0)).collect();
            unsafe { GetProcAddress(lib, PCSTR(c.as_ptr())) }
        };
        macro_rules! get {
            ($n:expr) => {
                unsafe { std::mem::transmute(sym($n)?) }
            };
        }
        let api = (|| {
            Some(Api {
                run: get!("Windows_run"),
                is_java: get!("isJavaWindow"),
                from_hwnd: get!("getAccessibleContextFromHWND"),
                info: get!("getAccessibleContextInfo"),
                child: get!("getAccessibleChildFromContext"),
                release: get!("releaseJavaObject"),
                actions: get!("getAccessibleActions"),
                do_actions: get!("doAccessibleActions"),
                focus: get!("requestFocus"),
                set_text: get!("setTextContents"),
                text_info: get!("getAccessibleTextInfo"),
                text_range: get!("getAccessibleTextRange"),
            })
        })();
        if api.is_some() {
            return api;
        }
    }
    None
}

type Job = Box<dyn FnOnce(&Option<Api>) + Send>;

static JOBS: OnceLock<mpsc::Sender<(Option<isize>, Job)>> = OnceLock::new();

fn pump_for(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    while Instant::now() < until {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn worker(rx: mpsc::Receiver<(Option<isize>, Job)>) {
    let mut api: Option<Api> = None;
    let mut tried = false;
    loop {
        pump_for(20);
        let next = match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(j) => j,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return,
        };
        if api.is_none() && !tried {
            tried = true;
            api = unsafe { load(next.0.map(|h| HWND(h as *mut core::ffi::c_void))) };
            if let Some(a) = &api {
                unsafe { (a.run)() };
                pump_for(700);
            }
        }
        (next.1)(&api);
    }
}

fn call<T: Send + 'static>(hint: Option<HWND>, f: impl FnOnce(&Api) -> Result<T> + Send + 'static) -> Result<T> {
    let tx = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("setu-jab".into()).spawn(move || worker(rx));
        tx
    });
    let (rtx, rrx) = mpsc::channel();
    let job: Job = Box::new(move |api: &Option<Api>| {
        let r = match api {
            Some(a) => f(a),
            None => Err(anyhow!("The Java Access Bridge isn't installed.")),
        };
        let _ = rtx.send(r);
    });
    tx.send((hint.map(|h| h.0 as isize), job)).map_err(|_| anyhow!("The Java Access Bridge stopped."))?;
    rrx.recv_timeout(Duration::from_secs(12)).map_err(|_| anyhow!("The Java app didn't answer in time."))?
}

pub fn java_class(h: HWND) -> bool {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    let class = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    class.starts_with("SunAwt")
}

pub fn is_java(h: HWND) -> bool {
    let raw = h.0 as isize;
    call(Some(h), move |a| Ok(unsafe { (a.is_java)(HWND(raw as *mut core::ffi::c_void)) } != 0)).unwrap_or(false)
}

fn role_of(role: &str, states: &str) -> &'static str {
    let editable = states.split(',').any(|s| s.trim() == "editable");
    match role {
        "push button" => "button",
        "toggle button" => "toggle",
        "check box" | "check menu item" => "checkbox",
        "radio button" | "radio menu item" => "radio",
        "password text" => "password",
        "text" | "edit bar" | "editbar" if editable => "field",
        "combo box" => "combobox",
        "page tab" => "tab",
        "menu item" | "menu" => "menuitem",
        "hyperlink" => "link",
        "slider" => "slider",
        "spin box" => "spinner",
        "table cell" => "cell",
        "label" | "text" => "text",
        "list item" => "item",
        "row header" | "column header" => "text",
        _ => "other",
    }
}

const CLICKY: &[&str] = &["button", "toggle", "checkbox", "radio", "link", "field", "password", "combobox", "tab", "menuitem", "item", "spinner", "slider"];

unsafe fn info_of(a: &Api, vm: i32, ac: JObject) -> Option<Box<ContextInfo>> {
    let mut info: Box<ContextInfo> = Box::new(unsafe { std::mem::zeroed() });
    (unsafe { (a.info)(vm, ac, &mut *info) } != 0).then_some(info)
}

fn walk(a: &Api, h: HWND, scope: Scope) -> Result<Vec<Node>> {
    let mut vm = 0i32;
    let mut root: JObject = 0;
    if unsafe { (a.from_hwnd)(h, &mut vm, &mut root) } == 0 || root == 0 {
        return Err(anyhow!("The Java Access Bridge can't see this window."));
    }
    let mut out: Vec<Node> = Vec::new();
    let mut stack: Vec<(JObject, String, usize, bool)> = vec![(root, String::new(), 0, true)];
    let mut visited = 0usize;
    while let Some((ac, path, depth, owned)) = stack.pop() {
        visited += 1;
        let info = unsafe { info_of(a, vm, ac) };
        let mut kids: Vec<(JObject, String, usize, bool)> = Vec::new();
        if let Some(info) = info {
            let states = from_wide(&info.states_en);
            let showing = states.split(',').any(|s| matches!(s.trim(), "showing" | "visible"));
            let role_en = from_wide(&info.role_en);
            let role = role_of(&role_en, &states);
            let name = from_wide(&info.name);
            let named = !name.trim().is_empty();
            let keep = depth > 0 && (showing || scope.hidden) && (CLICKY.contains(&role) || (scope.text && named && matches!(role, "text" | "cell")) || (named && role == "cell"));
            if keep && out.len() < scope.limit.max(1) {
                let st = |k: &str| states.split(',').any(|s| s.trim() == k);
                out.push(Node {
                        id: format!("jab:{}:{vm}:{path}", h.0 as isize),
                        role: role.into(),
                        name: name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned(),
                        value: String::new(),
                        x: info.x,
                        y: info.y,
                        w: info.width,
                        h: info.height,
                        focused: st("focused"),
                        checked: matches!(role, "checkbox" | "radio" | "toggle").then(|| st("checked") || st("pressed")),
                        editable: role == "field",
                        enabled: st("enabled"),
                        web: false,
                        selected: st("selected"),
                });
            }
            if (showing || scope.hidden || depth == 0) && depth < 60 && visited < 6000 && out.len() < scope.limit.max(1) {
                let n = info.children.clamp(0, 2000);
                for i in (0..n).rev() {
                    let c = unsafe { (a.child)(vm, ac, i) };
                    if c != 0 {
                        kids.push((c, if path.is_empty() { i.to_string() } else { format!("{path}.{i}") }, depth + 1, true));
                    }
                }
            }
        }
        if owned && depth > 0 {
            unsafe { (a.release)(vm, ac) };
        }
        stack.extend(kids);
    }
    unsafe { (a.release)(vm, root) };
    Ok(out)
}

pub fn tree(h: HWND, scope: Scope) -> Result<Vec<Node>> {
    let raw = h.0 as isize;
    call(Some(h), move |a| walk(a, HWND(raw as *mut core::ffi::c_void), scope))
}

fn parse_id(id: &str) -> Option<(isize, i32, Vec<i32>)> {
    let mut parts = id.strip_prefix("jab:")?.splitn(3, ':');
    let h: isize = parts.next()?.parse().ok()?;
    let vm: i32 = parts.next()?.parse().ok()?;
    let path: Vec<i32> = parts.next().unwrap_or("").split('.').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
    Some((h, vm, path))
}

fn resolve(a: &Api, h: HWND, path: &[i32]) -> Result<(i32, JObject, Vec<JObject>)> {
    let mut vm = 0i32;
    let mut cur: JObject = 0;
    if unsafe { (a.from_hwnd)(h, &mut vm, &mut cur) } == 0 || cur == 0 {
        return Err(anyhow!("That window is gone."));
    }
    let mut held = vec![cur];
    for i in path {
        let c = unsafe { (a.child)(vm, cur, *i) };
        if c == 0 {
            for x in held {
                unsafe { (a.release)(vm, x) };
            }
            return Err(anyhow!("That control is no longer there."));
        }
        held.push(c);
        cur = c;
    }
    Ok((vm, cur, held))
}

fn action_names(a: &Api, vm: i32, ac: JObject) -> Vec<String> {
    let mut acts: Box<Actions> = Box::new(unsafe { std::mem::zeroed() });
    if unsafe { (a.actions)(vm, ac, &mut *acts) } == 0 {
        return Vec::new();
    }
    acts.list.iter().take(acts.count.clamp(0, 256) as usize).map(|x| from_wide(&x.name)).collect()
}

fn do_action(a: &Api, vm: i32, ac: JObject, name: &str) -> bool {
    let mut todo: Box<ToDo> = Box::new(unsafe { std::mem::zeroed() });
    todo.count = 1;
    for (i, c) in name.encode_utf16().take(SHORT_STRING - 1).enumerate() {
        todo.list[0].name[i] = c;
    }
    let mut failure = 0i32;
    unsafe { (a.do_actions)(vm, ac, &mut *todo, &mut failure) != 0 }
}

fn text_of(a: &Api, vm: i32, ac: JObject, max: usize) -> Option<String> {
    let mut ti = TextInfo::default();
    if unsafe { (a.text_info)(vm, ac, &mut ti, 0, 0) } == 0 || ti.chars <= 0 {
        return None;
    }
    let total = (ti.chars as usize).min(max);
    let mut out = String::new();
    let mut start = 0usize;
    while start < total {
        let len = (total - start).min(30000);
        let mut buf = vec![0u16; len + 1];
        if unsafe { (a.text_range)(vm, ac, start as i32, (start + len - 1) as i32, buf.as_mut_ptr(), (len + 1) as i16) } == 0 {
            break;
        }
        out.push_str(&from_wide(&buf));
        start += len;
    }
    (!out.is_empty()).then_some(out)
}

pub fn act(node: &Node, what: Do) -> Result<()> {
    let (h, _, path) = parse_id(&node.id).ok_or_else(|| anyhow!("That control is no longer there."))?;
    let label = if node.name.is_empty() { "That control".to_owned() } else { node.name.clone() };
    let hw = HWND(h as *mut core::ffi::c_void);
    call(Some(hw), move |a| {
        let hwnd = HWND(h as *mut core::ffi::c_void);
        let (vm, ac, held) = resolve(a, hwnd, &path)?;
        let names = action_names(a, vm, ac);
        let pick = |want: &[&str]| names.iter().find(|n| want.iter().any(|w| n.eq_ignore_ascii_case(w))).cloned().or_else(|| names.first().cloned());
        let r = match &what {
            Do::Focus | Do::Insert(_) => {
                if unsafe { (a.focus)(vm, ac) } != 0 {
                    Ok(())
                } else {
                    Err(anyhow!("{label} can't take the focus."))
                }
            }
            Do::SetText(t) => {
                let w = wide(t);
                if unsafe { (a.set_text)(vm, ac, w.as_ptr()) } != 0 {
                    Ok(())
                } else {
                    Err(anyhow!("{label} didn't accept the text."))
                }
            }
            Do::Menu => Err(anyhow!("{label} has no menu.")),
            Do::Invoke | Do::Toggle | Do::Select | Do::Expand => match pick(&["click", "press", "toggle", "togglePopup", "toggleExpand"]) {
                Some(n) if do_action(a, vm, ac, &n) => Ok(()),
                _ => Err(anyhow!("{label} didn't respond.")),
            },
        };
        for x in held.into_iter().rev() {
            unsafe { (a.release)(vm, x) };
        }
        r
    })
}

pub fn text(node: &Node, max: usize) -> Option<String> {
    let (h, _, path) = parse_id(&node.id)?;
    call(Some(HWND(h as *mut core::ffi::c_void)), move |a| {
        let (vm, ac, held) = resolve(a, HWND(h as *mut core::ffi::c_void), &path)?;
        let t = text_of(a, vm, ac, max);
        for x in held.into_iter().rev() {
            unsafe { (a.release)(vm, x) };
        }
        Ok(t)
    })
    .ok()
    .flatten()
}

pub fn caret(node: &Node) -> Option<usize> {
    let (h, _, path) = parse_id(&node.id)?;
    call(Some(HWND(h as *mut core::ffi::c_void)), move |a| {
        let (vm, ac, held) = resolve(a, HWND(h as *mut core::ffi::c_void), &path)?;
        let mut ti = TextInfo::default();
        let ok = unsafe { (a.text_info)(vm, ac, &mut ti, 0, 0) } != 0;
        for x in held.into_iter().rev() {
            unsafe { (a.release)(vm, x) };
        }
        Ok((ok && ti.caret >= 0).then_some(ti.caret as usize))
    })
    .ok()
    .flatten()
}
