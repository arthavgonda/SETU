use super::{Do, Node, Scope, Win};
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;
use tokio::sync::oneshot;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetMessageW, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, MSG, SW_RESTORE, SetForegroundWindow, ShowWindow,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WS_EX_TOOLWINDOW,
};
use windows::core::{BSTR, Interface};

enum Job {
    Windows(oneshot::Sender<Result<Vec<Win>>>),
    Tree(Win, Scope, oneshot::Sender<Result<Vec<Node>>>),
    Act(Node, Do, oneshot::Sender<Result<()>>),
    Focused(Win, oneshot::Sender<Result<Option<Node>>>),
    Attr(Node, String, oneshot::Sender<Result<Option<String>>>),
    FindText(Win, String, oneshot::Sender<Result<bool>>),
    Popups(Win, oneshot::Sender<Result<Vec<(Win, Vec<Node>)>>>),
    Text(Node, usize, oneshot::Sender<Result<Option<String>>>),
    Caret(Node, oneshot::Sender<Result<Option<usize>>>),
}

static JOBS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();

fn jobs() -> &'static mpsc::Sender<Job> {
    JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("setu-uia".into())
            .spawn(move || worker(rx))
            .expect("uia thread");
        tx
    })
}

async fn ask<T>(make: impl FnOnce(oneshot::Sender<Result<T>>) -> Job) -> Result<T> {
    let (tx, rx) = oneshot::channel();
    jobs().send(make(tx)).map_err(|_| anyhow!("UI Automation stopped."))?;
    match tokio::time::timeout(Duration::from_secs(15), rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => Err(anyhow!("UI Automation stopped.")),
        Err(_) => Err(anyhow!("The app didn't answer in time.")),
    }
}

pub async fn ready() -> Result<()> {
    Ok(())
}

pub fn elevated(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let mut token = HANDLE::default();
        if OpenProcessToken(p, TOKEN_QUERY, &mut token).is_err() {
            let _ = CloseHandle(p);
            return true;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, Some(&mut e as *mut TOKEN_ELEVATION as *mut core::ffi::c_void), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut len).is_ok();
        let _ = CloseHandle(token);
        let _ = CloseHandle(p);
        ok && e.TokenIsElevated != 0
    }
}

static FRONT_CB: OnceLock<fn(Win)> = OnceLock::new();

static HOOKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RT: OnceLock<tokio::runtime::Handle> = OnceLock::new();

pub fn evented() -> bool {
    HOOKED.load(std::sync::atomic::Ordering::SeqCst)
}

unsafe extern "system" fn shape(_hook: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _thread: u32, _time: u32) {
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    if hwnd.0.is_null() || object != 0 || child != 0 {
        return;
    }
    if event != EVENT_OBJECT_DESTROY && unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    let _guard = RT.get().map(|h| h.enter());
    super::windows_changed();
}

pub fn track(on: fn(Win)) {
    if FRONT_CB.set(on).is_err() {
        return;
    }
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        let _ = RT.set(h);
    }
    let _ = std::thread::Builder::new().name("setu-foreground".into()).spawn(|| unsafe {
        let _hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        let shapes = [
            SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, None, Some(shape), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS),
            SetWinEventHook(EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, None, Some(shape), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS),
            SetWinEventHook(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, None, Some(shape), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS),
            SetWinEventHook(EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED, None, Some(shape), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS),
        ];
        HOOKED.store(shapes.iter().all(|h| !h.is_invalid()), std::sync::atomic::Ordering::SeqCst);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

unsafe extern "system" fn foreground(_hook: HWINEVENTHOOK, _event: u32, hwnd: HWND, _object: i32, _child: i32, _thread: u32, _time: u32) {
    {
        let _guard = RT.get().map(|h| h.enter());
        super::windows_changed();
    }
    if hwnd.0.is_null() {
        return;
    }
    let title = crate::tools::win::title_of(hwnd);
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    let exe = crate::tools::win::exe_of(hwnd);
    if let Some(cb) = FRONT_CB.get() {
        cb(Win { id: format!("hwnd:{}", hwnd.0 as isize), app: crate::tools::win::friendly(&exe), title, pid, front: true });
    }
}

pub async fn wake(_win: &Win) {}

pub async fn windows() -> Result<Vec<Win>> {
    ask(Job::Windows).await
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
    let n = node.clone();
    let k = key.to_owned();
    ask(move |tx| Job::Attr(n, k, tx)).await.ok().flatten()
}

pub async fn find_text(win: &Win, text: &str) -> Result<bool> {
    let w = win.clone();
    let t = text.to_owned();
    ask(move |tx| Job::FindText(w, t, tx)).await
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

struct State {
    uia: IUIAutomation,
    cache: HashMap<isize, (u64, Vec<IUIAutomationElement>)>,
    generation: u64,
}

fn worker(rx: mpsc::Receiver<Job>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let uia: Option<IUIAutomation> = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
    let mut st = uia.map(|u| State { uia: u, cache: HashMap::new(), generation: 0 });
    while let Ok(job) = rx.recv() {
        let Some(s) = st.as_mut() else {
            match job {
                Job::Windows(tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Tree(_, _, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Act(_, _, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Focused(_, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Attr(_, _, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::FindText(_, _, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Popups(_, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Text(_, _, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
                Job::Caret(_, tx) => drop(tx.send(Err(anyhow!("UI Automation isn't available.")))),
            }
            continue;
        };
        match job {
            Job::Windows(tx) => drop(tx.send(Ok(list_windows()))),
            Job::Tree(w, scope, tx) => drop(tx.send(build_tree(s, &w, scope))),
            Job::Act(n, d, tx) => drop(tx.send(do_act(s, &n, d))),
            Job::Focused(w, tx) => drop(tx.send(find_focused(s, &w))),
            Job::Attr(n, k, tx) => drop(tx.send(Ok(attr_of(s, &n, &k)))),
            Job::FindText(w, t, tx) => drop(tx.send(find_in(s, &w, &t))),
            Job::Popups(w, tx) => drop(tx.send(popup_nodes(s, &w))),
            Job::Text(n, max, tx) => drop(tx.send(Ok(text_of(s, &n, max)))),
            Job::Caret(n, tx) => drop(tx.send(Ok(caret_of(s, &n)))),
        }
    }
}

fn hwnd_of(id: &str) -> Option<HWND> {
    let raw = id.strip_prefix("hwnd:")?.split(':').next()?;
    raw.parse::<isize>().ok().map(|v| HWND(v as *mut core::ffi::c_void))
}

fn cloaked(h: HWND) -> bool {
    let mut v: u32 = 0;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut core::ffi::c_void, std::mem::size_of::<u32>() as u32).is_ok() && v != 0 }
}

struct Gather {
    out: Vec<Win>,
    front: isize,
}

unsafe extern "system" fn each(h: HWND, l: LPARAM) -> windows::core::BOOL {
    let g = unsafe { &mut *(l.0 as *mut Gather) };
    if unsafe { !IsWindowVisible(h).as_bool() } {
        return true.into();
    }
    let ex = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 || cloaked(h) {
        return true.into();
    }
    let title = crate::tools::win::title_of(h);
    if title.trim().is_empty() {
        return true.into();
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    let exe = crate::tools::win::exe_of(h);
    g.out.push(Win { id: format!("hwnd:{}", h.0 as isize), app: crate::tools::win::friendly(&exe), title, pid, front: h.0 as isize == g.front });
    true.into()
}

fn list_windows() -> Vec<Win> {
    let front = unsafe { GetForegroundWindow() }.0 as isize;
    let mut g = Gather { out: Vec::new(), front };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut g as *mut Gather as isize));
    }
    g.out
}

const INTERACTIVE: &[UIA_CONTROLTYPE_ID] = &[
    UIA_ButtonControlTypeId,
    UIA_HyperlinkControlTypeId,
    UIA_MenuItemControlTypeId,
    UIA_CheckBoxControlTypeId,
    UIA_RadioButtonControlTypeId,
    UIA_TabItemControlTypeId,
    UIA_ListItemControlTypeId,
    UIA_EditControlTypeId,
    UIA_ComboBoxControlTypeId,
    UIA_TreeItemControlTypeId,
    UIA_SplitButtonControlTypeId,
    UIA_DataItemControlTypeId,
    UIA_SpinnerControlTypeId,
    UIA_SliderControlTypeId,
    UIA_ProgressBarControlTypeId,
];

const READABLE: &[UIA_CONTROLTYPE_ID] = &[UIA_TextControlTypeId, UIA_DocumentControlTypeId, UIA_ImageControlTypeId, UIA_HeaderItemControlTypeId];

fn role_of(t: UIA_CONTROLTYPE_ID, password: bool) -> &'static str {
    match t {
        x if x == UIA_ButtonControlTypeId || x == UIA_SplitButtonControlTypeId => "button",
        x if x == UIA_HyperlinkControlTypeId => "link",
        x if x == UIA_MenuItemControlTypeId => "menuitem",
        x if x == UIA_CheckBoxControlTypeId => "checkbox",
        x if x == UIA_RadioButtonControlTypeId => "radio",
        x if x == UIA_TabItemControlTypeId => "tab",
        x if x == UIA_ListItemControlTypeId => "item",
        x if x == UIA_DataItemControlTypeId => "row",
        x if x == UIA_EditControlTypeId && password => "password",
        x if x == UIA_EditControlTypeId => "field",
        x if x == UIA_ComboBoxControlTypeId => "combobox",
        x if x == UIA_TreeItemControlTypeId => "treeitem",
        x if x == UIA_SpinnerControlTypeId => "spinner",
        x if x == UIA_SliderControlTypeId => "slider",
        x if x == UIA_ProgressBarControlTypeId => "progress",
        x if x == UIA_DocumentControlTypeId => "document",
        x if x == UIA_ImageControlTypeId => "image",
        x if x == UIA_TextControlTypeId || x == UIA_HeaderItemControlTypeId => "text",
        _ => "other",
    }
}

fn kinds(uia: &IUIAutomation, list: &[UIA_CONTROLTYPE_ID]) -> Result<IUIAutomationCondition> {
    let mut acc: Option<IUIAutomationCondition> = None;
    unsafe {
        for t in list {
            let c = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(t.0))?;
            acc = Some(match acc {
                Some(k) => uia.CreateOrCondition(&k, &c)?,
                None => c,
            });
        }
    }
    acc.ok_or_else(|| anyhow!("no conditions"))
}

fn cached_bool(el: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<bool> {
    unsafe { el.GetCachedPropertyValue(id).ok().and_then(|v| bool::try_from(&v).ok()) }
}

fn cached_i32(el: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<i32> {
    unsafe { el.GetCachedPropertyValue(id).ok().and_then(|v| i32::try_from(&v).ok()) }
}

fn cached_f64(el: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Option<f64> {
    unsafe { el.GetCachedPropertyValue(id).ok().and_then(|v| f64::try_from(&v).ok()) }
}

fn cached_str(el: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> String {
    unsafe { el.GetCachedPropertyValue(id).ok().and_then(|v| BSTR::try_from(&v).ok()).map(|b| b.to_string()).unwrap_or_default() }
}

fn browserish(app: &str) -> bool {
    let a = app.to_lowercase();
    ["chrome", "edge", "brave", "firefox", "opera", "vivaldi", "whatsapp", "msedgewebview"].iter().any(|w| a.contains(w))
}

fn build_tree(s: &mut State, win: &Win, scope: Scope) -> Result<Vec<Node>> {
    let h = hwnd_of(&win.id).ok_or_else(|| anyhow!("That window is gone."))?;
    if super::jab::java_class(h)
        && let Ok(n) = super::jab::tree(h, scope)
        && !n.is_empty()
    {
        return Ok(n);
    }
    let tries = if browserish(&win.app) { 4 } else { 1 };
    let mut last = Vec::new();
    for attempt in 0..tries {
        let (nodes, els) = gather(s, h, scope)?;
        let enough = nodes.len() >= 12 || attempt + 1 == tries;
        if enough {
            s.generation += 1;
            let generation = s.generation;
            let nodes: Vec<Node> = nodes.into_iter().map(|mut n| {
                n.id = format!("hwnd:{}:{generation}:{}", h.0 as isize, n.id);
                n
            }).collect();
            s.cache.insert(h.0 as isize, (generation, els));
            if s.cache.len() > 12 {
                let oldest = s.cache.iter().min_by_key(|(_, (g, _))| *g).map(|(k, _)| *k);
                if let Some(k) = oldest {
                    s.cache.remove(&k);
                }
            }
            return Ok(nodes);
        }
        last = nodes;
        std::thread::sleep(Duration::from_millis(350));
    }
    Ok(last)
}

fn gather(s: &State, h: HWND, scope: Scope) -> Result<(Vec<Node>, Vec<IUIAutomationElement>)> {
    unsafe {
        let uia = &s.uia;
        let root = uia.ElementFromHandle(h)?;
        let mut types: Vec<UIA_CONTROLTYPE_ID> = INTERACTIVE.to_vec();
        if scope.text {
            types.extend_from_slice(READABLE);
        }
        let kind = kinds(uia, &types)?;
        let cond = if scope.hidden {
            kind
        } else {
            let shown = uia.CreatePropertyCondition(UIA_IsOffscreenPropertyId, &VARIANT::from(false))?;
            uia.CreateAndCondition(&kind, &shown)?
        };
        let cr = uia.CreateCacheRequest()?;
        for p in [
            UIA_NamePropertyId,
            UIA_ControlTypePropertyId,
            UIA_BoundingRectanglePropertyId,
            UIA_IsEnabledPropertyId,
            UIA_HasKeyboardFocusPropertyId,
            UIA_IsPasswordPropertyId,
            UIA_IsValuePatternAvailablePropertyId,
            UIA_ValueValuePropertyId,
            UIA_ValueIsReadOnlyPropertyId,
            UIA_IsTogglePatternAvailablePropertyId,
            UIA_ToggleToggleStatePropertyId,
            UIA_SelectionItemIsSelectedPropertyId,
            UIA_RangeValueValuePropertyId,
            UIA_RangeValueMinimumPropertyId,
            UIA_RangeValueMaximumPropertyId,
        ] {
            cr.AddProperty(p)?;
        }
        let found = root.FindAllBuildCache(TreeScope_Descendants, &cond, &cr)?;
        let mut nodes = Vec::new();
        let mut els = Vec::new();
        for i in 0..found.Length()? {
            if nodes.len() >= scope.limit.max(1) {
                break;
            }
            let Ok(el) = found.GetElement(i) else { continue };
            let Ok(r): Result<RECT, _> = el.CachedBoundingRectangle() else { continue };
            let (w, hh) = (r.right - r.left, r.bottom - r.top);
            if (w < 2 || hh < 2) && !scope.hidden {
                continue;
            }
            let name = el.CachedName().map(|b| b.to_string()).unwrap_or_default();
            let kind = el.CachedControlType().unwrap_or(UIA_CustomControlTypeId);
            let password = cached_bool(&el, UIA_IsPasswordPropertyId).unwrap_or(false);
            let role = role_of(kind, password);
            if matches!(role, "text" | "image" | "other") && name.trim().is_empty() {
                continue;
            }
            let has_value = cached_bool(&el, UIA_IsValuePatternAvailablePropertyId).unwrap_or(false);
            let read_only = cached_bool(&el, UIA_ValueIsReadOnlyPropertyId).unwrap_or(true);
            let value = if role == "progress" {
                match (cached_f64(&el, UIA_RangeValueValuePropertyId), cached_f64(&el, UIA_RangeValueMinimumPropertyId).unwrap_or(0.0), cached_f64(&el, UIA_RangeValueMaximumPropertyId)) {
                    (Some(v), lo, Some(hi)) if hi > lo => format!("{:.0}%", ((v - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)),
                    _ => String::new(),
                }
            } else if has_value && role != "password" {
                cached_str(&el, UIA_ValueValuePropertyId).chars().take(500).collect()
            } else {
                String::new()
            };
            let toggle = cached_bool(&el, UIA_IsTogglePatternAvailablePropertyId).unwrap_or(false);
            let checked = toggle.then(|| cached_i32(&el, UIA_ToggleToggleStatePropertyId).unwrap_or(0) == ToggleState_On.0);
            let (cx, cy) = (r.left + w / 2, r.top + hh / 2);
            let scale = crate::tools::win::scale_at(cx, cy);
            nodes.push(Node {
                id: els.len().to_string(),
                role: role.into(),
                name: name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned(),
                value,
                x: (r.left as f64 / scale).round() as i32,
                y: (r.top as f64 / scale).round() as i32,
                w: (w as f64 / scale).round() as i32,
                h: (hh as f64 / scale).round() as i32,
                focused: cached_bool(&el, UIA_HasKeyboardFocusPropertyId).unwrap_or(false),
                checked,
                editable: matches!(role, "field" | "password" | "combobox") && has_value && !read_only,
                enabled: cached_bool(&el, UIA_IsEnabledPropertyId).unwrap_or(true),
                web: false,
                selected: cached_bool(&el, UIA_SelectionItemIsSelectedPropertyId).unwrap_or(false),
            });
            els.push(el);
        }
        Ok((nodes, els))
    }
}

fn element(s: &mut State, node: &Node) -> Result<(HWND, IUIAutomationElement)> {
    let rest = node.id.strip_prefix("hwnd:").ok_or_else(|| anyhow!("That control is no longer there."))?;
    let mut parts = rest.split(':');
    let h: isize = parts.next().and_then(|x| x.parse().ok()).ok_or_else(|| anyhow!("That control is no longer there."))?;
    let generation: u64 = parts.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let index: usize = parts.next().and_then(|x| x.parse().ok()).unwrap_or(usize::MAX);
    let hwnd = HWND(h as *mut core::ffi::c_void);
    if let Some((g, els)) = s.cache.get(&h)
        && *g == generation
        && let Some(el) = els.get(index)
    {
        return Ok((hwnd, el.clone()));
    }
    let (nodes, els) = gather(s, hwnd, Scope { limit: 2000, text: false, hidden: false })?;
    let want = super::norm(&node.name);
    let pos = nodes.iter().position(|n| n.role == node.role && super::norm(&n.name) == want).ok_or_else(|| anyhow!("{} is no longer on screen.", if node.name.is_empty() { "That control" } else { node.name.as_str() }))?;
    Ok((hwnd, els[pos].clone()))
}

fn bring_front(h: HWND) -> bool {
    unsafe {
        if GetForegroundWindow() == h {
            return true;
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = crate::tools::input::imp_press(&[crate::tools::input::K::Alt]);
        let _ = SetForegroundWindow(h);
        for _ in 0..30 {
            if GetForegroundWindow() == h {
                return true;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        GetForegroundWindow() == h
    }
}

fn invoke(el: &IUIAutomationElement, order: &[&str]) -> bool {
    unsafe {
        for p in order {
            let ok = match *p {
                "invoke" => el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|x| x.Invoke()).is_ok(),
                "toggle" => el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).and_then(|x| x.Toggle()).is_ok(),
                "select" => el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).and_then(|x| x.Select()).is_ok(),
                "expand" => el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId).and_then(|x| x.Expand()).is_ok(),
                "legacy" => el.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId).and_then(|x| x.DoDefaultAction()).is_ok(),
                _ => false,
            };
            if ok {
                return true;
            }
        }
        false
    }
}

fn keys_into(h: HWND, el: &IUIAutomationElement, text: &str, replace: bool) -> Result<()> {
    use crate::tools::input::{K, imp_press, imp_type};
    if !bring_front(h) {
        return Err(anyhow!("Windows didn't let me bring that window to the front."));
    }
    unsafe {
        let _ = el.SetFocus();
    }
    std::thread::sleep(Duration::from_millis(120));
    if replace {
        imp_press(&[K::Ctrl, K::Ch('a')])?;
        std::thread::sleep(Duration::from_millis(40));
    }
    imp_type(text)
}

fn java_act(node: &Node, what: Do) -> Result<()> {
    match what {
        Do::Insert(t) => {
            super::jab::act(node, Do::Focus)?;
            let h = node.id.strip_prefix("jab:").and_then(|r| r.split(':').next()).and_then(|x| x.parse::<isize>().ok()).map(|v| HWND(v as *mut core::ffi::c_void)).ok_or_else(|| anyhow!("That window is gone."))?;
            if !bring_front(h) {
                return Err(anyhow!("Windows didn't let me bring that window to the front."));
            }
            std::thread::sleep(Duration::from_millis(120));
            crate::tools::input::imp_type(&t)
        }
        other => super::jab::act(node, other),
    }
}

pub fn java_state(win: &Win) -> Option<bool> {
    let h = hwnd_of(&win.id)?;
    super::jab::java_class(h).then(|| super::jab::is_java(h))
}

fn do_act(s: &mut State, node: &Node, what: Do) -> Result<()> {
    if node.id.starts_with("jab:") {
        return java_act(node, what);
    }
    let (h, el) = element(s, node)?;
    let label = if node.name.is_empty() { "That control".to_owned() } else { node.name.clone() };
    match what {
        Do::Invoke => {
            if invoke(&el, &["invoke", "toggle", "select", "expand", "legacy"]) {
                return Ok(());
            }
            keys_into(h, &el, "", false)?;
            crate::tools::input::imp_press(&[crate::tools::input::K::Enter]).map_err(|_| anyhow!("{label} didn't respond."))
        }
        Do::Toggle => {
            if invoke(&el, &["toggle", "invoke", "legacy"]) { Ok(()) } else { Err(anyhow!("{label} can't be switched.")) }
        }
        Do::Select => {
            if invoke(&el, &["select", "invoke", "legacy"]) { Ok(()) } else { Err(anyhow!("{label} can't be selected.")) }
        }
        Do::Expand => {
            if invoke(&el, &["expand", "invoke", "legacy"]) { Ok(()) } else { Err(anyhow!("{label} can't be opened.")) }
        }
        Do::Focus => unsafe { el.SetFocus().map_err(|_| anyhow!("{label} can't take the focus.")) },
        Do::SetText(t) => {
            let done = unsafe {
                el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .and_then(|v| if v.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(true) { Err(windows::core::Error::from(windows::Win32::Foundation::E_FAIL)) } else { v.SetValue(&BSTR::from(t.as_str())) })
                    .is_ok()
            };
            if done { Ok(()) } else { keys_into(h, &el, &t, true) }
        }
        Do::Insert(t) => keys_into(h, &el, &t, false),
        Do::Menu => unsafe { el.cast::<IUIAutomationElement3>().and_then(|e| e.ShowContextMenu()).map_err(|_| anyhow!("{label} has no menu.")) },
    }
}

fn attr_of(s: &mut State, node: &Node, key: &str) -> Option<String> {
    if !matches!(key, "src" | "url") {
        return None;
    }
    let (_, el) = element(s, node).ok()?;
    unsafe {
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
            && let Ok(v) = p.CurrentValue()
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
        if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && let Ok(v) = p.CurrentValue()
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
    }
    None
}

fn find_in(s: &mut State, win: &Win, text: &str) -> Result<bool> {
    let h = hwnd_of(&win.id).ok_or_else(|| anyhow!("That window is gone."))?;
    unsafe {
        let root = s.uia.ElementFromHandle(h)?;
        let cond = s.uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_DocumentControlTypeId.0))?;
        let docs = root.FindAll(TreeScope_Descendants, &cond)?;
        for i in 0..docs.Length()? {
            let Ok(doc) = docs.GetElement(i) else { continue };
            let Ok(tp) = doc.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) else { continue };
            let Ok(range) = tp.DocumentRange() else { continue };
            if let Ok(found) = range.FindText(&BSTR::from(text), false, true) {
                let _ = found.Select();
                let _ = found.ScrollIntoView(true);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

struct Same {
    pid: u32,
    skip: isize,
    out: Vec<HWND>,
}

unsafe extern "system" fn same_pid(h: HWND, l: LPARAM) -> windows::core::BOOL {
    let g = unsafe { &mut *(l.0 as *mut Same) };
    if h.0 as isize == g.skip || unsafe { !IsWindowVisible(h).as_bool() } {
        return true.into();
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    if pid == g.pid {
        g.out.push(h);
    }
    true.into()
}

fn popup_nodes(s: &mut State, win: &Win) -> Result<Vec<(Win, Vec<Node>)>> {
    let main = hwnd_of(&win.id).map(|h| h.0 as isize).unwrap_or(0);
    let mut g = Same { pid: win.pid, skip: main, out: Vec::new() };
    if win.pid != 0 {
        unsafe {
            let _ = EnumWindows(Some(same_pid), LPARAM(&mut g as *mut Same as isize));
        }
    }
    let front = unsafe { GetForegroundWindow() }.0 as isize;
    let mut out = Vec::new();
    for h in g.out.into_iter().take(8) {
        let Ok((nodes, els)) = gather(s, h, Scope { limit: 400, text: false, hidden: false }) else { continue };
        if nodes.is_empty() {
            continue;
        }
        s.generation += 1;
        let generation = s.generation;
        let nodes: Vec<Node> = nodes
            .into_iter()
            .map(|mut n| {
                n.id = format!("hwnd:{}:{generation}:{}", h.0 as isize, n.id);
                n
            })
            .collect();
        s.cache.insert(h.0 as isize, (generation, els));
        let w = Win { id: format!("hwnd:{}", h.0 as isize), app: win.app.clone(), title: crate::tools::win::title_of(h), pid: win.pid, front: h.0 as isize == front };
        out.push((w, nodes));
    }
    Ok(out)
}

fn text_of(s: &mut State, node: &Node, max: usize) -> Option<String> {
    if node.id.starts_with("jab:") {
        return super::jab::text(node, max);
    }
    let (_, el) = element(s, node).ok()?;
    unsafe {
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            && let Ok(range) = tp.DocumentRange()
            && let Ok(t) = range.GetText(max.min(i32::MAX as usize) as i32)
        {
            let t = t.to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
        if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && let Ok(t) = v.CurrentValue()
        {
            let t: String = t.to_string().chars().take(max).collect();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn caret_of(s: &mut State, node: &Node) -> Option<usize> {
    if node.id.starts_with("jab:") {
        return super::jab::caret(node);
    }
    let (_, el) = element(s, node).ok()?;
    unsafe {
        let tp = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok()?;
        let sel = tp.GetSelection().ok()?;
        if sel.Length().ok()? < 1 {
            return None;
        }
        let first = sel.GetElement(0).ok()?;
        let before = tp.DocumentRange().ok()?;
        before.MoveEndpointByRange(TextPatternRangeEndpoint_End, &first, TextPatternRangeEndpoint_Start).ok()?;
        let t = before.GetText(-1).ok()?;
        Some(t.to_string().chars().count())
    }
}

fn find_focused(s: &mut State, win: &Win) -> Result<Option<Node>> {
    let nodes = build_tree(s, win, Scope { limit: 2000, text: false, hidden: false })?;
    Ok(nodes.into_iter().find(|n| n.focused && matches!(n.role.as_str(), "field" | "password" | "combobox")))
}

pub async fn watch(_secs: u64) -> bool {
    false
}
