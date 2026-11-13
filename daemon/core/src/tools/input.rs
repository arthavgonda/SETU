use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    Ctrl,
    Shift,
    Alt,
    Meta,
    Enter,
    Tab,
    Esc,
    Back,
    Del,
    Ins,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PgUp,
    PgDn,
    Space,
    F(u8),
    Ch(char),
    PlayPause,
    NextTrack,
    PrevTrack,
    Print,
}

impl K {
    pub fn modifier(self) -> bool {
        matches!(self, K::Ctrl | K::Shift | K::Alt | K::Meta)
    }
}

const WORDS: &[(&str, K)] = &[
    ("control", K::Ctrl),
    ("ctrl", K::Ctrl),
    ("controller", K::Ctrl),
    ("shift", K::Shift),
    ("alt", K::Alt),
    ("alter", K::Alt),
    ("old", K::Alt),
    ("option", K::Alt),
    ("windows", K::Meta),
    ("window", K::Meta),
    ("win", K::Meta),
    ("super", K::Meta),
    ("command", K::Meta),
    ("cmd", K::Meta),
    ("meta", K::Meta),
    ("enter", K::Enter),
    ("return", K::Enter),
    ("tab", K::Tab),
    ("escape", K::Esc),
    ("esc", K::Esc),
    ("backspace", K::Back),
    ("delete", K::Del),
    ("del", K::Del),
    ("insert", K::Ins),
    ("up", K::Up),
    ("down", K::Down),
    ("left", K::Left),
    ("right", K::Right),
    ("home", K::Home),
    ("end", K::End),
    ("space", K::Space),
    ("spacebar", K::Space),
    ("comma", K::Ch(',')),
    ("period", K::Ch('.')),
    ("dot", K::Ch('.')),
    ("slash", K::Ch('/')),
    ("backslash", K::Ch('\\')),
    ("semicolon", K::Ch(';')),
    ("equals", K::Ch('=')),
    ("equal", K::Ch('=')),
    ("minus", K::Ch('-')),
    ("dash", K::Ch('-')),
    ("backtick", K::Ch('`')),
    ("tilde", K::Ch('`')),
    ("quote", K::Ch('\'')),
    ("bracket", K::Ch('[')),
];

const NUMBERS: &[&str] = &["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"];

fn number_word(w: &str) -> Option<u8> {
    w.parse::<u8>().ok().or_else(|| NUMBERS.iter().position(|n| *n == w).map(|i| i as u8)).or(match w {
        "for" => Some(4),
        "to" | "too" => Some(2),
        "won" => Some(1),
        _ => None,
    })
}

pub fn parse_combo(spoken: &str) -> Option<Vec<K>> {
    let t = crate::text::normalize(spoken);
    let t = t
        .replace("back space", "backspace")
        .replace("space bar", "spacebar")
        .replace("page up", "pageup")
        .replace("page down", "pagedown")
        .replace("arrow key", "")
        .replace("arrow", "")
        .replace(" key", "");
    let words: Vec<&str> = t.split_whitespace().filter(|w| !matches!(*w, "the" | "and" | "plus" | "then" | "button" | "keys" | "key" | "press" | "hit")).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w == "pageup" {
            out.push(K::PgUp);
        } else if w == "pagedown" {
            out.push(K::PgDn);
        } else if let Some(n) = w.strip_prefix('f').and_then(|r| r.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
            out.push(K::F(n));
        } else if (w == "f" || w == "ef" || w == "eff") && i + 1 < words.len() && number_word(words[i + 1]).is_some_and(|n| (1..=12).contains(&n)) {
            out.push(K::F(number_word(words[i + 1]).unwrap()));
            i += 1;
        } else if let Some((_, k)) = WORDS.iter().find(|(n, _)| *n == w) {
            out.push(*k);
        } else if w.chars().count() == 1 {
            out.push(K::Ch(w.chars().next()?));
        } else if let Some(n) = number_word(w).filter(|n| *n <= 9) {
            out.push(K::Ch((b'0' + n) as char));
        } else {
            out.push(K::Ch(letter_word(w)?));
        }
        i += 1;
    }
    if out.is_empty() || out.iter().all(|k| k.modifier()) && out.len() > 1 {
        return None;
    }
    Some(out)
}

fn letter_word(w: &str) -> Option<char> {
    let table = [
        ("ay", 'a'), ("bee", 'b'), ("be", 'b'), ("see", 'c'), ("sea", 'c'), ("dee", 'd'), ("ee", 'e'), ("gee", 'g'), ("jay", 'j'), ("kay", 'k'), ("el", 'l'), ("em", 'm'),
        ("en", 'n'), ("oh", 'o'), ("pee", 'p'), ("queue", 'q'), ("cue", 'q'), ("are", 'r'), ("es", 's'), ("tee", 't'), ("tea", 't'), ("you", 'u'), ("vee", 'v'),
        ("ex", 'x'), ("why", 'y'), ("zed", 'z'), ("zee", 'z'),
    ];
    table.iter().find(|(n, _)| *n == w).map(|(_, c)| *c)
}

pub fn describe(keys: &[K]) -> String {
    keys.iter()
        .map(|k| match k {
            K::Ctrl => "Ctrl".to_owned(),
            K::Shift => "Shift".to_owned(),
            K::Alt => "Alt".to_owned(),
            K::Meta => if cfg!(target_os = "macos") { "Command".to_owned() } else { "Windows".to_owned() },
            K::Enter => "Enter".to_owned(),
            K::Tab => "Tab".to_owned(),
            K::Esc => "Escape".to_owned(),
            K::Back => "Backspace".to_owned(),
            K::Del => "Delete".to_owned(),
            K::Ins => "Insert".to_owned(),
            K::Up => "Up".to_owned(),
            K::Down => "Down".to_owned(),
            K::Left => "Left".to_owned(),
            K::Right => "Right".to_owned(),
            K::Home => "Home".to_owned(),
            K::End => "End".to_owned(),
            K::PgUp => "Page Up".to_owned(),
            K::PgDn => "Page Down".to_owned(),
            K::Space => "Space".to_owned(),
            K::F(n) => format!("F{n}"),
            K::Ch(c) => c.to_uppercase().to_string(),
            K::PlayPause => "Play".to_owned(),
            K::NextTrack => "Next track".to_owned(),
            K::PrevTrack => "Previous track".to_owned(),
            K::Print => "Print Screen".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub fn primary() -> K {
    if cfg!(target_os = "macos") { K::Meta } else { K::Ctrl }
}

static BACKEND: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn used(backend: u8) {
    BACKEND.store(backend, std::sync::atomic::Ordering::Relaxed);
}

fn close(mut s: crate::layers::Span, r: Result<()>) -> Result<()> {
    crate::os::disturbed();
    if BACKEND.load(std::sync::atomic::Ordering::Relaxed) == 3 {
        s.set(crate::layers::Layer::Command, "command.keys");
    }
    s.end(r)
}

pub async fn press(keys: &[K]) -> Result<()> {
    let s = crate::layers::effect(crate::layers::Layer::Keyboard, "keyboard.press");
    let keys = keys.to_vec();
    let r = match crate::layers::blocking(move || imp::press(&keys)).await {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    };
    close(s, r)
}

pub async fn press_times(keys: &[K], times: usize) -> Result<()> {
    let s = crate::layers::effect(crate::layers::Layer::Keyboard, "keyboard.press_times");
    let keys = keys.to_vec();
    let r = match crate::layers::blocking(move || {
        let n = times.clamp(1, 50);
        for i in 0..n {
            imp::press(&keys)?;
            if i + 1 < n {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        }
        Ok(())
    })
    .await
    {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    };
    close(s, r)
}

pub async fn type_text(text: &str) -> Result<()> {
    let s = crate::layers::effect(crate::layers::Layer::Keyboard, "keyboard.type");
    let t = text.to_owned();
    let r = match crate::layers::blocking(move || imp::type_text(&t)).await {
        Ok(r) => r,
        Err(e) => Err(e.into()),
    };
    close(s, r)
}

pub async fn warm() {
    let _ = crate::layers::blocking(imp::warm).await;
}

pub fn status() -> &'static str {
    imp::status()
}

pub fn imp_press(keys: &[K]) -> Result<()> {
    let s = crate::layers::effect(crate::layers::Layer::Keyboard, "keyboard.press");
    close(s, imp::press(keys))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn imp_type(text: &str) -> Result<()> {
    let s = crate::layers::effect(crate::layers::Layer::Keyboard, "keyboard.type");
    close(s, imp::type_text(text))
}

#[cfg(windows)]
mod imp {
    use super::K;
    use anyhow::{Result, anyhow};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VkKeyScanW,
    };

    fn vk(k: K) -> Option<(u16, bool, bool)> {
        Some(match k {
            K::Ctrl => (0x11, false, false),
            K::Shift => (0x10, false, false),
            K::Alt => (0x12, false, false),
            K::Meta => (0x5B, true, false),
            K::Enter => (0x0D, false, false),
            K::Tab => (0x09, false, false),
            K::Esc => (0x1B, false, false),
            K::Back => (0x08, false, false),
            K::Del => (0x2E, true, false),
            K::Ins => (0x2D, true, false),
            K::Up => (0x26, true, false),
            K::Down => (0x28, true, false),
            K::Left => (0x25, true, false),
            K::Right => (0x27, true, false),
            K::Home => (0x24, true, false),
            K::End => (0x23, true, false),
            K::PgUp => (0x21, true, false),
            K::PgDn => (0x22, true, false),
            K::Space => (0x20, false, false),
            K::F(n) => (0x6F + n as u16, false, false),
            K::PlayPause => (0xB3, true, false),
            K::NextTrack => (0xB0, true, false),
            K::PrevTrack => (0xB1, true, false),
            K::Print => (0x2C, false, false),
            K::Ch(c) => {
                let c = c.to_ascii_lowercase();
                if c.is_ascii_lowercase() {
                    (c.to_ascii_uppercase() as u16, false, false)
                } else if c.is_ascii_digit() {
                    (c as u16, false, false)
                } else {
                    let r = unsafe { VkKeyScanW(c as u16) };
                    if r == -1 {
                        return None;
                    }
                    ((r as u16) & 0xFF, false, (r as u16) & 0x100 != 0)
                }
            }
        })
    }

    fn ev(code: u16, up: bool, ext: bool) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        if ext {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(code), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
    }

    fn send(inputs: &[INPUT]) -> Result<()> {
        let t0 = std::time::Instant::now();
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
        crate::layers::record("keyboard.event", t0.elapsed().as_micros() as u64 / inputs.len().max(1) as u64);
        if sent != inputs.len() {
            return Err(anyhow!("Windows blocked the key press. The window may run as administrator."));
        }
        Ok(())
    }

    pub fn warm() {}

    pub fn press(keys: &[K]) -> Result<()> {
        let mut downs = Vec::new();
        let mut shift_needed = false;
        for k in keys {
            let (code, ext, shift) = vk(*k).ok_or_else(|| anyhow!("That key isn't on this keyboard."))?;
            shift_needed |= shift;
            downs.push((code, ext));
        }
        if shift_needed && !keys.contains(&K::Shift) {
            downs.insert(0, (0x10, false));
        }
        let mut inputs: Vec<INPUT> = downs.iter().map(|(c, e)| ev(*c, false, *e)).collect();
        inputs.extend(downs.iter().rev().map(|(c, e)| ev(*c, true, *e)));
        send(&inputs)
    }

    pub fn type_text(text: &str) -> Result<()> {
        let mut inputs = Vec::with_capacity(text.len() * 2);
        for u in text.encode_utf16() {
            if u == '\n' as u16 {
                inputs.push(ev(0x0D, false, false));
                inputs.push(ev(0x0D, true, false));
                continue;
            }
            let k = |up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: u, dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE }, time: 0, dwExtraInfo: 0 } },
            };
            inputs.push(k(false));
            inputs.push(k(true));
        }
        if inputs.is_empty() {
            return Ok(());
        }
        send(&inputs)
    }

    pub fn status() -> &'static str {
        "ready"
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::K;
    use anyhow::{Result, anyhow};
    use std::fs::File;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::time::Duration;

    const EV_SYN: u16 = 0;
    const EV_KEY: u16 = 1;
    const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
    const UI_DEV_SETUP: libc::c_ulong = 0x405C_5503;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;

    static DEV: Mutex<Option<File>> = Mutex::new(None);

    #[repr(C)]
    struct Setup {
        bustype: u16,
        vendor: u16,
        product: u16,
        version: u16,
        name: [u8; 80],
        ff_effects_max: u32,
    }

    fn open() -> Result<File> {
        let f = std::fs::OpenOptions::new().write(true).custom_flags_nonblock().open("/dev/uinput")?;
        let fd = f.as_raw_fd();
        unsafe {
            if libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int) < 0 || libc::ioctl(fd, UI_SET_EVBIT, EV_SYN as libc::c_int) < 0 {
                return Err(anyhow!("uinput refused the keyboard"));
            }
            for code in 1..=248 {
                libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_int);
            }
            let mut name = [0u8; 80];
            let label = b"Setu virtual keyboard";
            name[..label.len()].copy_from_slice(label);
            let setup = Setup { bustype: 0x06, vendor: 0x5e70, product: 0x0001, version: 1, name, ff_effects_max: 0 };
            if libc::ioctl(fd, UI_DEV_SETUP, &setup as *const Setup) < 0 || libc::ioctl(fd, UI_DEV_CREATE) < 0 {
                return Err(anyhow!("uinput could not create the keyboard"));
            }
        }
        std::thread::sleep(Duration::from_millis(250));
        Ok(f)
    }

    trait NonBlock {
        fn custom_flags_nonblock(&mut self) -> &mut Self;
    }

    impl NonBlock for std::fs::OpenOptions {
        fn custom_flags_nonblock(&mut self) -> &mut Self {
            use std::os::unix::fs::OpenOptionsExt;
            self.custom_flags(libc::O_NONBLOCK)
        }
    }

    fn emit(f: &mut File, kind: u16, code: u16, value: i32) -> std::io::Result<()> {
        let mut b = [0u8; 24];
        b[16..18].copy_from_slice(&kind.to_ne_bytes());
        b[18..20].copy_from_slice(&code.to_ne_bytes());
        b[20..24].copy_from_slice(&value.to_ne_bytes());
        f.write_all(&b)
    }

    fn code(k: K) -> Option<(u16, bool)> {
        Some(match k {
            K::Ctrl => (29, false),
            K::Shift => (42, false),
            K::Alt => (56, false),
            K::Meta => (125, false),
            K::Enter => (28, false),
            K::Tab => (15, false),
            K::Esc => (1, false),
            K::Back => (14, false),
            K::Del => (111, false),
            K::Ins => (110, false),
            K::Up => (103, false),
            K::Down => (108, false),
            K::Left => (105, false),
            K::Right => (106, false),
            K::Home => (102, false),
            K::End => (107, false),
            K::PgUp => (104, false),
            K::PgDn => (109, false),
            K::Space => (57, false),
            K::F(n) if n <= 10 => (58 + n as u16, false),
            K::F(11) => (87, false),
            K::F(12) => (88, false),
            K::F(_) => return None,
            K::PlayPause => (164, false),
            K::NextTrack => (163, false),
            K::PrevTrack => (165, false),
            K::Print => (99, false),
            K::Ch(c) => ascii(c)?,
        })
    }

    fn ascii(c: char) -> Option<(u16, bool)> {
        const ROW: &[(char, u16)] = &[
            ('1', 2), ('2', 3), ('3', 4), ('4', 5), ('5', 6), ('6', 7), ('7', 8), ('8', 9), ('9', 10), ('0', 11), ('-', 12), ('=', 13), ('q', 16), ('w', 17), ('e', 18), ('r', 19),
            ('t', 20), ('y', 21), ('u', 22), ('i', 23), ('o', 24), ('p', 25), ('[', 26), (']', 27), ('a', 30), ('s', 31), ('d', 32), ('f', 33), ('g', 34), ('h', 35), ('j', 36),
            ('k', 37), ('l', 38), (';', 39), ('\'', 40), ('`', 41), ('\\', 43), ('z', 44), ('x', 45), ('c', 46), ('v', 47), ('b', 48), ('n', 49), ('m', 50), (',', 51), ('.', 52),
            ('/', 53), (' ', 57), ('\n', 28), ('\t', 15),
        ];
        const SHIFTED: &[(char, char)] = &[
            ('!', '1'), ('@', '2'), ('#', '3'), ('$', '4'), ('%', '5'), ('^', '6'), ('&', '7'), ('*', '8'), ('(', '9'), (')', '0'), ('_', '-'), ('+', '='), ('{', '['), ('}', ']'),
            (':', ';'), ('"', '\''), ('~', '`'), ('|', '\\'), ('<', ','), ('>', '.'), ('?', '/'),
        ];
        if c.is_ascii_uppercase() {
            return ROW.iter().find(|(k, _)| *k == c.to_ascii_lowercase()).map(|(_, v)| (*v, true));
        }
        if let Some((_, base)) = SHIFTED.iter().find(|(s, _)| *s == c) {
            return ROW.iter().find(|(k, _)| k == base).map(|(_, v)| (*v, true));
        }
        ROW.iter().find(|(k, _)| *k == c).map(|(_, v)| (*v, false))
    }

    fn with_dev<T>(f: impl FnOnce(&mut File) -> std::io::Result<T>) -> Result<T> {
        let mut g = DEV.lock().unwrap();
        if g.is_none() {
            *g = Some(open()?);
        }
        let dev = g.as_mut().unwrap();
        match f(dev) {
            Ok(v) => Ok(v),
            Err(e) => {
                *g = None;
                Err(e.into())
            }
        }
    }

    fn key(f: &mut File, code: u16, value: i32) -> std::io::Result<()> {
        let t0 = std::time::Instant::now();
        emit(f, EV_KEY, code, value)?;
        emit(f, EV_SYN, 0, 0)?;
        crate::layers::record("keyboard.event", t0.elapsed().as_micros() as u64);
        Ok(())
    }

    fn chord(f: &mut File, codes: &[u16]) -> std::io::Result<()> {
        for c in codes {
            key(f, *c, 1)?;
            std::thread::sleep(Duration::from_millis(4));
        }
        for c in codes.iter().rev() {
            key(f, *c, 0)?;
            std::thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    }

    pub fn warm() {
        let _ = with_dev(|_| Ok(()));
    }

    fn codes(keys: &[K]) -> Option<Vec<u16>> {
        let mut out = Vec::new();
        let mut shift = false;
        for k in keys {
            let (c, s) = code(*k)?;
            shift |= s;
            out.push(c);
        }
        if shift && !out.contains(&42) {
            out.insert(0, 42);
        }
        Some(out)
    }

    fn tool(keys: &[K]) -> Result<()> {
        let name = |k: &K| -> Option<String> {
            Some(match k {
                K::Ctrl => "ctrl".into(),
                K::Shift => "shift".into(),
                K::Alt => "alt".into(),
                K::Meta => "super".into(),
                K::Enter => "Return".into(),
                K::Tab => "Tab".into(),
                K::Esc => "Escape".into(),
                K::Back => "BackSpace".into(),
                K::Del => "Delete".into(),
                K::Ins => "Insert".into(),
                K::Up => "Up".into(),
                K::Down => "Down".into(),
                K::Left => "Left".into(),
                K::Right => "Right".into(),
                K::Home => "Home".into(),
                K::End => "End".into(),
                K::PgUp => "Prior".into(),
                K::PgDn => "Next".into(),
                K::Space => "space".into(),
                K::F(n) => format!("F{n}"),
                K::PlayPause => "XF86AudioPlay".into(),
                K::NextTrack => "XF86AudioNext".into(),
                K::PrevTrack => "XF86AudioPrev".into(),
                K::Print => "Print".into(),
                K::Ch(c) => c.to_string(),
            })
        };
        let names: Vec<String> = keys.iter().filter_map(name).collect();
        let x11 = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some();
        if x11 && crate::tools::apps::which("xdotool").is_some() {
            let ok = Command::new("xdotool").args(["key", "--clearmodifiers", &names.join("+")]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                return Ok(());
            }
        }
        if crate::tools::apps::which("wtype").is_some() {
            let mut cmd = Command::new("wtype");
            let (mods, rest): (Vec<&K>, Vec<&K>) = keys.iter().partition(|k| k.modifier());
            for m in &mods {
                cmd.args(["-M", &name(m).unwrap_or_default()]);
            }
            for k in &rest {
                match k {
                    K::Ch(c) => {
                        cmd.arg(c.to_string());
                    }
                    other => {
                        cmd.args(["-k", &name(other).unwrap_or_default()]);
                    }
                }
            }
            for m in mods.iter().rev() {
                cmd.args(["-m", &name(m).unwrap_or_default()]);
            }
            if cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false) {
                return Ok(());
            }
        }
        if crate::tools::apps::which("ydotool").is_some()
            && let Some(cs) = codes(keys)
        {
            let mut args = vec!["key".to_owned()];
            args.extend(cs.iter().map(|c| format!("{c}:1")));
            args.extend(cs.iter().rev().map(|c| format!("{c}:0")));
            if Command::new("ydotool").args(&args).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false) {
                return Ok(());
            }
        }
        Err(anyhow!("I can't press keys yet. Keyboard control needs access to /dev/uinput; reinstall Setu or add yourself to the input group."))
    }

    pub fn press(keys: &[K]) -> Result<()> {
        if let Some(cs) = codes(keys)
            && with_dev(|f| chord(f, &cs)).is_ok()
        {
            super::used(1);
            return Ok(());
        }
        if let Some(syms) = keys.iter().map(|k| keysym(*k)).collect::<Option<Vec<u32>>>()
            && remote::chord(&syms).is_ok()
        {
            super::used(2);
            return Ok(());
        }
        let r = tool(keys);
        if r.is_ok() {
            super::used(3);
        }
        r
    }

    fn keysym(k: K) -> Option<u32> {
        Some(match k {
            K::Ctrl => 0xffe3,
            K::Shift => 0xffe1,
            K::Alt => 0xffe9,
            K::Meta => 0xffeb,
            K::Enter => 0xff0d,
            K::Tab => 0xff09,
            K::Esc => 0xff1b,
            K::Back => 0xff08,
            K::Del => 0xffff,
            K::Ins => 0xff63,
            K::Up => 0xff52,
            K::Down => 0xff54,
            K::Left => 0xff51,
            K::Right => 0xff53,
            K::Home => 0xff50,
            K::End => 0xff57,
            K::PgUp => 0xff55,
            K::PgDn => 0xff56,
            K::Space => 0x20,
            K::F(n) if (1..=24).contains(&n) => 0xffbe + (n as u32 - 1),
            K::F(_) => return None,
            K::PlayPause => 0x1008_ff14,
            K::NextTrack => 0x1008_ff17,
            K::PrevTrack => 0x1008_ff16,
            K::Print => 0xff61,
            K::Ch(c) => char_sym(c),
        })
    }

    pub fn char_sym(c: char) -> u32 {
        match c {
            '\n' | '\r' => 0xff0d,
            '\t' => 0xff09,
            c if (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) => c as u32,
            c => 0x0100_0000 | c as u32,
        }
    }

    mod remote {
        use anyhow::{Result, anyhow};
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        use zbus::zvariant::OwnedObjectPath;

        const DEST: &str = "org.gnome.Mutter.RemoteDesktop";
        const SESSION: &str = "org.gnome.Mutter.RemoteDesktop.Session";

        static LIVE: Mutex<Option<(zbus::Connection, OwnedObjectPath)>> = Mutex::new(None);
        static OFF: AtomicBool = AtomicBool::new(false);
        static USED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        const LINGER: Duration = Duration::from_secs(5);

        fn linger(path: OwnedObjectPath) {
            let mine = USED.fetch_add(1, Ordering::SeqCst) + 1;
            let Ok(h) = tokio::runtime::Handle::try_current() else { return };
            h.spawn(async move {
                tokio::time::sleep(LINGER).await;
                if USED.load(Ordering::SeqCst) != mine {
                    return;
                }
                let taken = {
                    let mut g = LIVE.lock().unwrap();
                    if g.as_ref().is_some_and(|(_, p)| *p == path) { g.take() } else { None }
                };
                if let Some((c, p)) = taken {
                    let _ = tokio::time::timeout(Duration::from_secs(2), c.call_method(Some(DEST), p.as_str(), Some(SESSION), "Stop", &())).await;
                }
            });
        }

        async fn open() -> Result<(zbus::Connection, OwnedObjectPath)> {
            let c = zbus::Connection::session().await?;
            let r = c.call_method(Some(DEST), "/org/gnome/Mutter/RemoteDesktop", Some(DEST), "CreateSession", &()).await?;
            let path: OwnedObjectPath = r.body().deserialize()?;
            c.call_method(Some(DEST), path.as_str(), Some(SESSION), "Start", &()).await?;
            tokio::time::sleep(Duration::from_millis(250)).await;
            for down in [true, false] {
                let _ = c.call_method(Some(DEST), path.as_str(), Some(SESSION), "NotifyKeyboardKeysym", &(0xffe1u32, down)).await;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok((c, path))
        }

        async fn send(events: Vec<(u32, bool)>) -> Result<()> {
            let cached = LIVE.lock().unwrap().clone();
            let (c, path) = match cached {
                Some(s) => s,
                None => match open().await {
                    Ok(s) => {
                        *LIVE.lock().unwrap() = Some(s.clone());
                        s
                    }
                    Err(e) => {
                        if std::env::var_os("SETU_BENCH").is_none() {
                            OFF.store(true, Ordering::Relaxed);
                        }
                        return Err(e);
                    }
                },
            };
            for (sym, down) in events {
                let t0 = std::time::Instant::now();
                if let Err(e) = c.call_method(Some(DEST), path.as_str(), Some(SESSION), "NotifyKeyboardKeysym", &(sym, down)).await {
                    *LIVE.lock().unwrap() = None;
                    return Err(anyhow!("{e}"));
                }
                crate::layers::record("keyboard.event", t0.elapsed().as_micros() as u64);
                tokio::time::sleep(Duration::from_millis(3)).await;
            }
            linger(path);
            Ok(())
        }

        fn run(events: Vec<(u32, bool)>) -> Result<()> {
            if OFF.load(Ordering::Relaxed) {
                return Err(anyhow!("unavailable"));
            }
            match tokio::runtime::Handle::try_current() {
                Ok(h) => h.block_on(send(events)),
                Err(_) => tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(send(events)),
            }
        }

        pub fn chord(syms: &[u32]) -> Result<()> {
            let mut ev: Vec<(u32, bool)> = syms.iter().map(|s| (*s, true)).collect();
            ev.extend(syms.iter().rev().map(|s| (*s, false)));
            run(ev)
        }

        pub fn text(t: &str) -> Result<()> {
            let mut ev = Vec::new();
            for c in t.chars() {
                let s = super::char_sym(c);
                ev.push((s, true));
                ev.push((s, false));
            }
            run(ev)
        }
    }

    pub fn type_text(text: &str) -> Result<()> {
        if text.chars().all(|c| ascii(c).is_some()) {
            let r = with_dev(|f| {
                for c in text.chars() {
                    let (code, shift) = ascii(c).unwrap();
                    if shift {
                        chord(f, &[42, code])?;
                    } else {
                        chord(f, &[code])?;
                    }
                }
                Ok(())
            });
            if r.is_ok() {
                super::used(1);
                return Ok(());
            }
        }
        if remote::text(text).is_ok() {
            super::used(2);
            return Ok(());
        }
        let x11 = std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some();
        if x11 && crate::tools::apps::which("xdotool").is_some() {
            let ok = Command::new("xdotool").args(["type", "--delay", "8", "--", text]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                super::used(3);
                return Ok(());
            }
        }
        if crate::tools::apps::which("wtype").is_some() {
            let ok = Command::new("wtype").args(["--", text]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                super::used(3);
                return Ok(());
            }
        }
        if !text.is_ascii() && crate::tools::shell::copy_text(text).is_ok() {
            std::thread::sleep(Duration::from_millis(120));
            if press(&[K::Ctrl, K::Ch('v')]).is_ok() {
                return Ok(());
            }
        }
        Err(anyhow!("I can't type here yet. Keyboard control needs access to /dev/uinput."))
    }

    pub fn status() -> &'static str {
        if DEV.lock().unwrap().is_some() {
            return "ready";
        }
        match std::fs::OpenOptions::new().write(true).open("/dev/uinput") {
            Ok(_) => "ready",
            Err(_) if crate::tools::apps::which("ydotool").is_some() || crate::tools::apps::which("wtype").is_some() || crate::tools::apps::which("xdotool").is_some() => "ready",
            Err(_) => "needs_access",
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::K;
    use anyhow::{Result, anyhow};
    use std::process::Command;

    fn key_code(k: K) -> Option<u16> {
        Some(match k {
            K::Enter => 36,
            K::Tab => 48,
            K::Space => 49,
            K::Back => 51,
            K::Esc => 53,
            K::Del => 117,
            K::Home => 115,
            K::End => 119,
            K::PgUp => 116,
            K::PgDn => 121,
            K::Left => 123,
            K::Right => 124,
            K::Down => 125,
            K::Up => 126,
            K::F(n) => [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111][(n.clamp(1, 12) - 1) as usize],
            _ => return None,
        })
    }

    fn script(keys: &[K]) -> Option<String> {
        let mut mods = Vec::new();
        let mut main = None;
        for k in keys {
            match k {
                K::Ctrl => mods.push("control down"),
                K::Shift => mods.push("shift down"),
                K::Alt => mods.push("option down"),
                K::Meta => mods.push("command down"),
                other => main = Some(*other),
            }
        }
        let main = main?;
        let using = if mods.is_empty() { String::new() } else { format!(" using {{{}}}", mods.join(", ")) };
        Some(match main {
            K::Ch(c) => format!("tell application \"System Events\" to keystroke \"{}\"{using}", if c == '"' { "\\\"".to_owned() } else { c.to_string() }),
            K::PlayPause | K::NextTrack | K::PrevTrack | K::Ins | K::Print => return None,
            other => format!("tell application \"System Events\" to key code {}{using}", key_code(other)?),
        })
    }

    pub fn press(keys: &[K]) -> Result<()> {
        let s = script(keys).ok_or_else(|| anyhow!("That key isn't available on a Mac."))?;
        let ok = Command::new("osascript").args(["-e", &s]).status().map(|s| s.success()).unwrap_or(false);
        super::used(3);
        if ok { Ok(()) } else { Err(anyhow!("macOS blocked the key press. Allow Setu in Privacy & Security, Accessibility.")) }
    }

    pub fn type_text(text: &str) -> Result<()> {
        let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
        let s = format!("tell application \"System Events\" to keystroke \"{escaped}\"");
        let ok = Command::new("osascript").args(["-e", &s]).status().map(|s| s.success()).unwrap_or(false);
        super::used(3);
        if ok { Ok(()) } else { Err(anyhow!("macOS blocked the typing. Allow Setu in Privacy & Security, Accessibility.")) }
    }

    pub fn warm() {}

    pub fn status() -> &'static str {
        "ready"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spoken_combos() {
        assert_eq!(parse_combo("control t"), Some(vec![K::Ctrl, K::Ch('t')]));
        assert_eq!(parse_combo("control plus shift plus p"), Some(vec![K::Ctrl, K::Shift, K::Ch('p')]));
        assert_eq!(parse_combo("alt f4"), Some(vec![K::Alt, K::F(4)]));
        assert_eq!(parse_combo("alt f four"), Some(vec![K::Alt, K::F(4)]));
        assert_eq!(parse_combo("page down"), Some(vec![K::PgDn]));
        assert_eq!(parse_combo("the enter key"), Some(vec![K::Enter]));
        assert_eq!(parse_combo("escape"), Some(vec![K::Esc]));
        assert_eq!(parse_combo("control see"), Some(vec![K::Ctrl, K::Ch('c')]));
        assert_eq!(parse_combo("down arrow"), Some(vec![K::Down]));
        assert_eq!(parse_combo("pizza"), None);
        assert_eq!(parse_combo("control shift"), None);
        let _ = describe(&[K::Ctrl, K::Ch('t')]);
    }
}
