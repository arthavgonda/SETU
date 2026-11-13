use super::input::{self, K};
use anyhow::{Result, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Left,
    Right,
    Max,
    Min,
    Restore,
    NextScreen,
    Center,
}

impl Place {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "left" => Self::Left,
            "right" => Self::Right,
            "max" => Self::Max,
            "min" => Self::Min,
            "restore" => Self::Restore,
            "next_screen" => Self::NextScreen,
            "center" => Self::Center,
            _ => return None,
        })
    }

    pub fn done(self) -> &'static str {
        match self {
            Self::Left => "Moved the window to the left",
            Self::Right => "Moved the window to the right",
            Self::Max => "Maximised the window",
            Self::Min => "Minimised the window",
            Self::Restore => "Restored the window",
            Self::NextScreen => "Moved the window to the other screen",
            Self::Center => "Centred the window",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Corner(String),
    Nudge { dx: i32, dy: i32 },
    Scale { w: f32, h: f32 },
    Size { w: i32, h: i32 },
}

pub type Look = dyn Fn() -> Option<((i32, i32), bool)>;

pub const STEP: i32 = 10;
pub const NUDGE: i32 = 4;

impl Frame {
    pub fn done(&self) -> String {
        match self {
            Frame::Corner(c) => format!("Moved the window to the {}", c.replace('-', " ")),
            Frame::Nudge { dx, dy } => format!("Moved the window {}", if *dy < 0 { "up" } else if *dy > 0 { "down" } else if *dx < 0 { "left" } else { "right" }),
            Frame::Scale { .. } | Frame::Size { .. } => "Resized the window".into(),
        }
    }

    pub fn target(&self, now: (i32, i32)) -> Option<(i32, i32)> {
        let fit = |v: i32| v.clamp(200, 6000);
        match self {
            Frame::Scale { w, h } => Some((fit((now.0 as f32 * w).round() as i32), fit((now.1 as f32 * h).round() as i32))),
            Frame::Size { w, h } => Some((fit(*w), fit(*h))),
            _ => None,
        }
    }

    pub fn corner_axes(c: &str) -> (i32, i32) {
        let x = if c.ends_with("left") { -1 } else if c.ends_with("right") { 1 } else { 0 };
        let y = if c.starts_with("top") { -1 } else if c.starts_with("bottom") { 1 } else { 0 };
        (x, y)
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_accel(a: &str) -> Option<Vec<K>> {
    let mut keys = Vec::new();
    let mut rest = a.trim();
    while let Some(r) = rest.strip_prefix('<') {
        let (m, tail) = r.split_once('>')?;
        keys.push(match m.to_lowercase().as_str() {
            "super" | "mod4" => K::Meta,
            "shift" => K::Shift,
            "alt" | "mod1" => K::Alt,
            "control" | "primary" | "ctrl" => K::Ctrl,
            _ => return None,
        });
        rest = tail;
    }
    let key = match rest {
        "Left" => K::Left,
        "Right" => K::Right,
        "Up" => K::Up,
        "Down" => K::Down,
        "Tab" => K::Tab,
        "Page_Down" | "Next" => K::PgDn,
        "Page_Up" | "Prior" => K::PgUp,
        "Home" => K::Home,
        "End" => K::End,
        "Return" => K::Enter,
        "Escape" => K::Esc,
        "space" => K::Space,
        f if f.len() >= 2 && f.starts_with('F') && f[1..].parse::<u8>().is_ok() => K::F(f[1..].parse().ok()?),
        c if c.chars().count() == 1 => K::Ch(c.chars().next()?.to_ascii_lowercase()),
        _ => return None,
    };
    keys.push(key);
    Some(keys)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::{Command, Stdio};

    fn kde() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase().contains("kde")
    }

    static KEYS: std::sync::Mutex<Vec<(String, Option<Vec<String>>)>> = std::sync::Mutex::new(Vec::new());
    static KEYS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static DCONF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    fn read_key(schema: &str, key: &str) -> Option<Vec<String>> {
        let span = crate::layers::read(crate::layers::Layer::Command, "command.gsettings");
        let out = span.end(Command::new("gsettings").args(["get", schema, key]).stdin(Stdio::null()).stderr(Stdio::null()).output()).ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Some(text.split('\'').skip(1).step_by(2).map(str::to_owned).filter(|s| !s.is_empty()).collect())
    }

    fn gsettings(schema: &str, key: &str) -> Option<Vec<String>> {
        use std::sync::atomic::Ordering::SeqCst;
        let name = format!("{schema} {key}");
        if DCONF.load(SeqCst) {
            let s = crate::layers::read(crate::layers::Layer::System, "system.keybindings");
            if let Some((_, v)) = KEYS.lock().unwrap().iter().find(|(n, _)| *n == name) {
                s.ok();
                return v.clone();
            }
            s.forget();
        }
        let before = KEYS_GEN.load(SeqCst);
        let v = read_key(schema, key);
        if DCONF.load(SeqCst) {
            let mut g = KEYS.lock().unwrap();
            if KEYS_GEN.load(SeqCst) == before && !g.iter().any(|(n, _)| *n == name) {
                g.push((name, v.clone()));
            }
        }
        v
    }

    pub async fn warm() {
        use futures_util::StreamExt;
        use std::sync::atomic::Ordering::SeqCst;
        if kde() {
            return;
        }
        let Ok(c) = crate::syscfg_linux::session().await else { return };
        let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("ca.desrt.dconf.Writer").and_then(|b| b.member("Notify")).map(|b| b.build()) else { return };
        let Ok(mut notes) = zbus::MessageStream::for_match_rule(rule, c, Some(16)).await else { return };
        DCONF.store(true, SeqCst);
        tokio::spawn(async move {
            while notes.next().await.is_some() {
                KEYS_GEN.fetch_add(1, SeqCst);
                KEYS.lock().unwrap().clear();
            }
            DCONF.store(false, SeqCst);
            KEYS_GEN.fetch_add(1, SeqCst);
            KEYS.lock().unwrap().clear();
        });
        let _ = crate::layers::blocking(|| {
            for (schema, key) in [(MUTTER, "toggle-tiled-left"), (MUTTER, "toggle-tiled-right"), (WM, "maximize"), (WM, "minimize"), (WM, "unmaximize"), (WM, "move-to-monitor-right"), (WM, "show-desktop")] {
                gsettings(schema, key);
            }
        })
        .await;
    }

    fn bound(schema: &str, key: &str) -> Option<Vec<K>> {
        gsettings(schema, key)?.iter().find_map(|a| parse_accel(a))
    }

    const WM: &str = "org.gnome.desktop.wm.keybindings";
    const MUTTER: &str = "org.gnome.mutter.keybindings";

    pub fn place(p: Place) -> Result<()> {
        let keys = if kde() {
            match p {
                Place::Left => vec![K::Meta, K::Left],
                Place::Right => vec![K::Meta, K::Right],
                Place::Max | Place::Center => vec![K::Meta, K::PgUp],
                Place::Min => vec![K::Meta, K::PgDn],
                Place::Restore => vec![K::Meta, K::PgUp],
                Place::NextScreen => vec![K::Meta, K::Shift, K::Right],
            }
        } else {
            let (schema, key, fallback) = match p {
                Place::Left => (MUTTER, "toggle-tiled-left", vec![K::Meta, K::Left]),
                Place::Right => (MUTTER, "toggle-tiled-right", vec![K::Meta, K::Right]),
                Place::Max => (WM, "maximize", vec![K::Meta, K::Up]),
                Place::Min => (WM, "minimize", vec![K::Meta, K::Ch('h')]),
                Place::Restore | Place::Center => (WM, "unmaximize", vec![K::Meta, K::Down]),
                Place::NextScreen => (WM, "move-to-monitor-right", vec![K::Meta, K::Shift, K::Right]),
            };
            bound(schema, key).unwrap_or(fallback)
        };
        input::imp_press(&keys)
    }

    pub fn desktop() -> Result<()> {
        if kde() {
            return input::imp_press(&[K::Meta, K::Ch('d')]);
        }
        if let Some(k) = bound(WM, "show-desktop") {
            return input::imp_press(&k);
        }
        let ok = Command::new("gsettings")
            .args(["set", WM, "show-desktop", "['<Super><Shift><Control>d']"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Err(anyhow!("This desktop has no shortcut to show the desktop."));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        input::imp_press(&[K::Meta, K::Shift, K::Ctrl, K::Ch('d')])
    }

    pub fn workspace(n: Option<u32>, dir: i32, carry: bool) -> Result<String> {
        if kde() {
            return match (n, carry) {
                (Some(n), false) if n <= 12 => input::imp_press(&[K::Ctrl, K::F(n as u8)]).map(|_| super::workspace_done(Some(n), dir, false)),
                (None, c) => {
                    let side = if dir > 0 { K::Right } else { K::Left };
                    let keys = if c { vec![K::Meta, K::Ctrl, K::Shift, side] } else { vec![K::Meta, K::Ctrl, side] };
                    input::imp_press(&keys).map(|_| super::workspace_done(None, dir, c))
                }
                _ => Err(anyhow!("KDE has no shortcut for that. Say move this window to the next workspace.")),
            };
        }
        let step = |right: bool| -> Vec<K> {
            let (key, fallback) = match (carry, right) {
                (false, true) => ("switch-to-workspace-right", vec![K::Meta, K::PgDn]),
                (false, false) => ("switch-to-workspace-left", vec![K::Meta, K::PgUp]),
                (true, true) => ("move-to-workspace-right", vec![K::Meta, K::Shift, K::PgDn]),
                (true, false) => ("move-to-workspace-left", vec![K::Meta, K::Shift, K::PgUp]),
            };
            bound(WM, key).unwrap_or(fallback)
        };
        match n {
            Some(n) => {
                let first = if carry { bound(WM, "move-to-workspace-1").unwrap_or(vec![K::Meta, K::Shift, K::Home]) } else { bound(WM, "switch-to-workspace-1").unwrap_or(vec![K::Meta, K::Home]) };
                input::imp_press(&first)?;
                let right = step(true);
                for _ in 1..n {
                    std::thread::sleep(std::time::Duration::from_millis(220));
                    input::imp_press(&right)?;
                }
            }
            None => input::imp_press(&step(dir > 0))?,
        }
        Ok(super::workspace_done(n, dir, carry))
    }

    fn tap(keys: &[K], times: i32) -> Result<()> {
        for _ in 0..times.max(0) {
            input::imp_press(keys)?;
            std::thread::sleep(std::time::Duration::from_millis(28));
        }
        Ok(())
    }

    fn arrow(dx: i32, dy: i32) -> K {
        if dx < 0 {
            K::Left
        } else if dx > 0 {
            K::Right
        } else if dy < 0 {
            K::Up
        } else {
            K::Down
        }
    }

    fn pause(ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn held(look: &Look, want: bool) -> bool {
        for _ in 0..12 {
            pause(50);
            if look().is_some_and(|(_, active)| active != want) {
                return true;
            }
        }
        false
    }

    fn grab(key: &str, fallback: Vec<K>, look: &Look) -> Result<()> {
        if look().is_none_or(|(_, active)| !active) {
            return Err(anyhow!("I can't follow this window, so I won't move or resize it."));
        }
        input::imp_press(&bound(WM, key).unwrap_or(fallback))?;
        if held(look, true) {
            pause(120);
            Ok(())
        } else {
            Err(anyhow!("This window can't be moved or resized. It may be full screen or a fixed size."))
        }
    }

    fn settle(look: &Look) -> Result<()> {
        pause(60);
        input::imp_press(&[K::Enter])?;
        held(look, false);
        pause(80);
        Ok(())
    }

    fn cancel(look: &Look) -> Result<()> {
        input::imp_press(&[K::Esc])?;
        held(look, false);
        pause(80);
        Ok(())
    }

    fn screens() -> usize {
        std::fs::read_dir("/sys/class/drm").map(|d| d.flatten().filter(|e| std::fs::read_to_string(e.path().join("enabled")).is_ok_and(|s| s.trim() == "enabled")).count()).unwrap_or(0).max(1)
    }

    fn reach(x: i32, y: i32, look: &Look) -> Result<(i32, i32)> {
        let before = look().map(|(s, _)| s).ok_or_else(|| anyhow!("I can't read this window's size."))?;
        grab("begin-resize", vec![K::Alt, K::F(8)], look)?;
        let mut d = (0, 0);
        let r = (|| -> Result<()> {
            if x != 0 {
                tap(&[arrow(x, 0)], 1)?;
                tap(&[K::Shift, arrow(x, 0)], 1)?;
                pause(260);
                if let Some((s, _)) = look() {
                    d.0 = (s.0 - before.0).abs();
                }
            }
            if y != 0 {
                tap(&[arrow(0, y)], 1)?;
                tap(&[K::Shift, arrow(0, y)], 1)?;
                pause(260);
                if let Some((s, _)) = look() {
                    d.1 = (s.1 - before.1).abs();
                }
            }
            Ok(())
        })();
        cancel(look)?;
        r.map(|_| d)
    }

    pub fn reframe(f: &Frame, look: &Look) -> Result<()> {
        if kde() {
            return Err(anyhow!("Moving and resizing a window to a chosen place isn't available on KDE yet. Say snap left or snap right."));
        }
        if look().is_some_and(|(_, active)| active) {
            let _ = input::imp_press(&bound(WM, "unmaximize").unwrap_or(vec![K::Meta, K::Down]));
            pause(220);
        }
        match f {
            Frame::Corner(c) => {
                let (x, y) = Frame::corner_axes(c);
                let rounds = if screens() > 1 { 1 } else { 4 };
                for _ in 0..rounds {
                    let (dx, dy) = reach(x, y, look)?;
                    if dx == 0 && dy == 0 {
                        break;
                    }
                    grab("begin-move", vec![K::Alt, K::F(7)], look)?;
                    if dx != 0 {
                        tap(&[K::Shift, arrow(x, 0)], 1)?;
                    }
                    if dy != 0 {
                        tap(&[K::Shift, arrow(0, y)], 1)?;
                    }
                    settle(look)?;
                }
                Ok(())
            }
            Frame::Nudge { dx, dy } => {
                grab("begin-move", vec![K::Alt, K::F(7)], look)?;
                tap(&[arrow(*dx, 0)], dx.abs())?;
                tap(&[arrow(0, *dy)], dy.abs())?;
                settle(look)
            }
            Frame::Scale { .. } | Frame::Size { .. } => {
                let now = look().map(|(s, _)| s).ok_or_else(|| anyhow!("I can't read this window's size, so I can't resize it."))?;
                let want = f.target(now).unwrap_or(now);
                let steps = |d: i32| (d as f32 / STEP as f32).round() as i32;
                let (dw, dh) = (steps(want.0 - now.0), steps(want.1 - now.1));
                if dw == 0 && dh == 0 {
                    return Ok(());
                }
                grab("begin-resize", vec![K::Alt, K::F(8)], look)?;
                if dw != 0 {
                    tap(&[K::Right], 1)?;
                    tap(&[arrow(dw, 0)], dw.abs().min(400))?;
                }
                if dh != 0 {
                    tap(&[K::Down], 1)?;
                    tap(&[arrow(0, dh)], dh.abs().min(400))?;
                }
                settle(look)
            }
        }
    }

    pub fn switch() -> Result<()> {
        input::imp_press(&[K::Alt, K::Tab])
    }

    pub fn close() -> Result<()> {
        let k = if kde() { None } else { bound(WM, "close") };
        input::imp_press(&k.unwrap_or_else(|| vec![K::Alt, K::F(4)]))
    }

    pub fn focus(name: &str, app: Option<&crate::tools::apps::App>) -> Result<bool> {
        if kde() {
            if let Some(a) = app {
                crate::tools::apps::launch(a)?;
                return Ok(true);
            }
            return Ok(false);
        }
        input::imp_press(&[K::Meta])?;
        std::thread::sleep(std::time::Duration::from_millis(420));
        input::imp_type(name)?;
        std::thread::sleep(std::time::Duration::from_millis(520));
        input::imp_press(&[K::Enter])?;
        Ok(true)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::process::Command;

    fn osa(script: &str) -> Result<String> {
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.osascript");
        let out = Command::new("osascript").args(["-e", script]).output().map_err(|e| anyhow!("{e}"))?;
        span.end_bool(out.status.success());
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            Err(anyhow!("macOS blocked window control. Allow Setu in Privacy & Security, Accessibility."))
        }
    }

    fn screen() -> (i64, i64, i64, i64) {
        let (x, y, w, h) = crate::os::screen_rect();
        (x as i64, y as i64, w as i64, h as i64)
    }

    fn frame(x: i64, y: i64, w: i64, h: i64) -> Result<()> {
        let span = crate::layers::effect(crate::layers::Layer::Accessibility, "accessibility.window");
        span.end(crate::os::set_window_rect(x as f64, y as f64, w as f64, h as f64))
    }

    pub fn place(p: Place) -> Result<()> {
        let (sx, sy, w, h) = screen();
        let top = sy + 25;
        let hh = h - 25;
        match p {
            Place::Left => frame(sx, top, w / 2, hh),
            Place::Right => frame(sx + w / 2, top, w - w / 2, hh),
            Place::Max => frame(sx, top, w, hh),
            Place::Center | Place::Restore => frame(sx + w / 6, top + hh / 8, w * 2 / 3, hh * 3 / 4),
            Place::Min => {
                let span = crate::layers::effect(crate::layers::Layer::Accessibility, "accessibility.window");
                span.end(crate::os::minimise_window())
            }
            Place::NextScreen => Err(anyhow!("Moving windows between screens isn't available on a Mac yet.")),
        }
    }

    pub fn reframe(f: &Frame, _look: &Look) -> Result<()> {
        let (px, py, pw, ph) = crate::os::window_rect()?;
        let (px, py, pw, ph) = (px as i64, py as i64, pw as i64, ph as i64);
        let (sx, sy, sw, sh) = screen();
        let top = sy + 25;
        let (x, y, w, h) = match f {
            Frame::Corner(c) => {
                let (ax, ay) = Frame::corner_axes(c);
                let x = if ax < 0 { sx } else if ax > 0 { sx + sw - pw } else { px };
                let y = if ay < 0 { top } else if ay > 0 { sy + sh - ph } else { py };
                (x, y, pw, ph)
            }
            Frame::Nudge { dx, dy } => (px + (*dx * STEP) as i64, (py + (*dy * STEP) as i64).max(top), pw, ph),
            _ => {
                let (w, h) = f.target((pw as i32, ph as i32)).unwrap_or((pw as i32, ph as i32));
                (px, py, (w as i64).min(sw), (h as i64).min(sh - 25))
            }
        };
        frame(x.max(sx), y.max(top), w, h)
    }

    pub fn desktop() -> Result<()> {
        input::imp_press(&[K::F(11)])
    }

    pub fn switch() -> Result<()> {
        input::imp_press(&[K::Meta, K::Tab])
    }

    pub fn workspace(n: Option<u32>, dir: i32, carry: bool) -> Result<String> {
        if carry {
            return Err(anyhow!("macOS has no shortcut to move a window to another desktop. Drag it to the top of Mission Control."));
        }
        match n {
            Some(n) => {
                for _ in 0..8 {
                    input::imp_press(&[K::Ctrl, K::Left])?;
                    std::thread::sleep(std::time::Duration::from_millis(120));
                }
                for _ in 1..n {
                    std::thread::sleep(std::time::Duration::from_millis(450));
                    input::imp_press(&[K::Ctrl, K::Right])?;
                }
            }
            None => input::imp_press(&[K::Ctrl, if dir > 0 { K::Right } else { K::Left }])?,
        }
        Ok(super::workspace_done(n, dir, false))
    }

    pub fn close() -> Result<()> {
        input::imp_press(&[K::Meta, K::Ch('w')])
    }

    pub fn focus(name: &str, _app: Option<&crate::tools::apps::App>) -> Result<bool> {
        let safe = name.replace('"', "");
        osa(&format!("tell application \"{safe}\" to activate")).map(|_| true)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, IsIconic, IsWindowVisible, PostMessageW, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
        WM_CLOSE,
    };

    fn fg() -> Result<HWND> {
        let h = unsafe { GetForegroundWindow() };
        if h.0.is_null() { Err(anyhow!("No window is active.")) } else { Ok(h) }
    }

    fn work(h: HWND) -> RECT {
        unsafe {
            let m = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(m, &mut mi);
            mi.rcWork
        }
    }

    fn frame(h: HWND, x: i32, y: i32, w: i32, hh: i32) -> Result<()> {
        unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
            SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER | SWP_SHOWWINDOW).map_err(|e| anyhow!("{e}"))
        }
    }

    pub fn place(p: Place) -> Result<()> {
        let h = fg()?;
        let r = work(h);
        let (w, hh) = (r.right - r.left, r.bottom - r.top);
        match p {
            Place::Left => frame(h, r.left, r.top, w / 2, hh),
            Place::Right => frame(h, r.left + w / 2, r.top, w - w / 2, hh),
            Place::Center => frame(h, r.left + w / 6, r.top + hh / 8, w * 2 / 3, hh * 3 / 4),
            Place::Max => {
                unsafe {
                    let _ = ShowWindow(h, SW_MAXIMIZE);
                }
                Ok(())
            }
            Place::Min => {
                unsafe {
                    let _ = ShowWindow(h, SW_MINIMIZE);
                }
                Ok(())
            }
            Place::Restore => {
                unsafe {
                    let _ = ShowWindow(h, SW_RESTORE);
                }
                Ok(())
            }
            Place::NextScreen => input::imp_press(&[K::Meta, K::Shift, K::Right]),
        }
    }

    pub fn reframe(f: &Frame, _look: &Look) -> Result<()> {
        use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let h = fg()?;
        let area = work(h);
        let mut r = RECT::default();
        unsafe { GetWindowRect(h, &mut r).map_err(|e| anyhow!("{e}"))? };
        let (pw, ph) = (r.right - r.left, r.bottom - r.top);
        let (x, y, w, hh) = match f {
            Frame::Corner(c) => {
                let (ax, ay) = Frame::corner_axes(c);
                let x = if ax < 0 { area.left } else if ax > 0 { area.right - pw } else { r.left };
                let y = if ay < 0 { area.top } else if ay > 0 { area.bottom - ph } else { r.top };
                (x, y, pw, ph)
            }
            Frame::Nudge { dx, dy } => (r.left + dx * STEP, r.top + dy * STEP, pw, ph),
            _ => {
                let (w, hh) = f.target((pw, ph)).unwrap_or((pw, ph));
                (r.left, r.top, w.min(area.right - area.left), hh.min(area.bottom - area.top))
            }
        };
        frame(h, x, y, w, hh)
    }

    pub fn desktop() -> Result<()> {
        input::imp_press(&[K::Meta, K::Ch('d')])
    }

    pub fn switch() -> Result<()> {
        input::imp_press(&[K::Alt, K::Tab])
    }

    pub fn close() -> Result<()> {
        let h = fg()?;
        unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)).map_err(|e| anyhow!("{e}")) }
    }

    struct Search {
        want: Vec<String>,
        found: Option<HWND>,
    }

    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> windows::core::BOOL {
        let s = unsafe { &mut *(l.0 as *mut Search) };
        if unsafe { !IsWindowVisible(h).as_bool() } {
            return true.into();
        }
        let title = crate::tools::win::title_of(h).to_lowercase();
        let exe = crate::tools::win::exe_of(h).to_lowercase();
        if title.is_empty() {
            return true.into();
        }
        if s.want.iter().any(|w| !w.is_empty() && (exe == *w || title.contains(w.as_str()))) {
            s.found = Some(h);
            return false.into();
        }
        true.into()
    }

    pub fn workspace(n: Option<u32>, dir: i32, carry: bool) -> Result<String> {
        if carry {
            return Err(anyhow!("Windows has no shortcut to move a window to another desktop. Open Task View with the Windows key and Tab, and drag it there."));
        }
        match n {
            Some(n) => {
                for _ in 0..8 {
                    input::imp_press(&[K::Ctrl, K::Meta, K::Left])?;
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
                for _ in 1..n {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    input::imp_press(&[K::Ctrl, K::Meta, K::Right])?;
                }
            }
            None => input::imp_press(&[K::Ctrl, K::Meta, if dir > 0 { K::Right } else { K::Left }])?,
        }
        Ok(super::workspace_done(n, dir, false))
    }

    pub fn focus(name: &str, app: Option<&crate::tools::apps::App>) -> Result<bool> {
        let mut want = vec![name.to_lowercase()];
        if let Some(a) = app {
            want.push(a.name.to_lowercase());
            if let Some(stem) = std::path::Path::new(&a.path).file_stem() {
                want.push(stem.to_string_lossy().to_lowercase());
            }
        }
        let mut s = Search { want, found: None };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut s as *mut Search as isize));
        }
        let Some(h) = s.found else { return Ok(false) };
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = input::imp_press(&[K::Alt]);
            let _ = SetForegroundWindow(h);
        }
        Ok(true)
    }
}

async fn run<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    #[cfg(windows)]
    let span = crate::layers::effect(crate::layers::Layer::System, "system.window");
    let r = match crate::layers::blocking(f).await {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    };
    #[cfg(windows)]
    let r = span.end(r);
    r
}

pub async fn place(p: Place) -> Result<()> {
    run(move || imp::place(p)).await
}

pub async fn warm() {
    #[cfg(target_os = "linux")]
    imp::warm().await;
}

pub async fn desktop() -> Result<()> {
    run(imp::desktop).await
}

pub async fn frame(f: Frame, win: Option<crate::os::Win>) -> Result<String> {
    let done = f.done();
    let rt = tokio::runtime::Handle::current();
    let look = move || win.as_ref().and_then(|w| rt.block_on(crate::os::window_seen(w)));
    run(move || imp::reframe(&f, &look)).await.map(|_| done)
}

pub async fn switch() -> Result<()> {
    run(imp::switch).await
}

pub async fn close() -> Result<()> {
    run(imp::close).await
}

fn workspace_done(n: Option<u32>, dir: i32, carry: bool) -> String {
    match (n, carry) {
        (Some(n), false) => format!("On workspace {n}"),
        (Some(n), true) => format!("Moved the window to workspace {n}"),
        (None, false) => if dir > 0 { "On the next workspace".into() } else { "On the previous workspace".into() },
        (None, true) => if dir > 0 { "Moved the window to the next workspace".into() } else { "Moved the window to the previous workspace".into() },
    }
}

pub async fn workspace(n: Option<u32>, dir: i32, carry: bool) -> Result<String> {
    run(move || imp::workspace(n, dir, carry)).await
}

pub async fn focus(name: &str, app: Option<crate::tools::apps::App>) -> Result<bool> {
    let n = name.to_owned();
    run(move || imp::focus(&n, app.as_ref())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gnome_accelerators() {
        assert_eq!(parse_accel("<Super>h"), Some(vec![K::Meta, K::Ch('h')]));
        assert_eq!(parse_accel("<Super><Shift>Right"), Some(vec![K::Meta, K::Shift, K::Right]));
        assert_eq!(parse_accel("<Alt>F4"), Some(vec![K::Alt, K::F(4)]));
        assert_eq!(parse_accel("<Primary><Alt>Page_Down"), Some(vec![K::Ctrl, K::Alt, K::PgDn]));
        assert_eq!(parse_accel("<Hyper>x"), None);
        assert_eq!(Place::parse("left"), Some(Place::Left));
        assert_eq!(Frame::corner_axes("top-right"), (1, -1));
        assert_eq!(Frame::corner_axes("bottom"), (0, 1));
        assert_eq!(Frame::Scale { w: 0.5, h: 1.0 }.target((700, 500)), Some((350, 500)));
        assert_eq!(Frame::Scale { w: 0.1, h: 9.0 }.target((700, 500)), Some((200, 4500)), "sizes stay within sane limits");
        assert_eq!(Frame::Size { w: 800, h: 600 }.target((1, 1)), Some((800, 600)));
        assert_eq!(Frame::Corner("top-left".into()).done(), "Moved the window to the top left");
    }
}
