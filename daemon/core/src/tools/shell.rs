use super::apps::App;
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(windows)]
use crate::proc::Quiet;

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_owned();
    }
    format!("\"{}\"", a.replace('"', "\\\""))
}

#[cfg(windows)]
fn shell_execute(file: &str, params: &str, dir: Option<&Path>) -> Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;
    let f = wide(file);
    let p = wide(params);
    let op = wide("open");
    let d = dir.map(|d| wide(&d.to_string_lossy()));
    let r = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(f.as_ptr()),
            if params.is_empty() { PCWSTR::null() } else { PCWSTR(p.as_ptr()) },
            d.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
            SW_SHOWNORMAL,
        )
    };
    if r.0 as isize > 32 { Ok(()) } else { Err(anyhow!("Windows could not open {file} (error {}).", r.0 as isize)) }
}

#[cfg(windows)]
pub fn shortcut_target(lnk: &Path) -> Option<(PathBuf, String)> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{Interface, PCWSTR};
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let r = (|| -> Option<(PathBuf, String)> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let file: IPersistFile = link.cast().ok()?;
            let w = wide(&lnk.to_string_lossy());
            file.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
            let mut path = [0u16; 1024];
            link.GetPath(&mut path, std::ptr::null_mut(), 0).ok()?;
            let mut args = [0u16; 2048];
            let _ = link.GetArguments(&mut args);
            let p = String::from_utf16_lossy(&path[..path.iter().position(|c| *c == 0).unwrap_or(0)]);
            let a = String::from_utf16_lossy(&args[..args.iter().position(|c| *c == 0).unwrap_or(0)]);
            (!p.is_empty()).then(|| (PathBuf::from(p), a))
        })();
        if init {
            CoUninitialize();
        }
        r
    }
}

pub fn open(target: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let span = crate::layers::effect(crate::layers::Layer::System, "system.open");
        span.end(shell_execute(target, "", None))
    }
    #[cfg(target_os = "macos")]
    {
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.open");
        let ok = span.end_bool(Command::new("open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(()) } else { Err(anyhow!("{target} could not be opened.")) }
    }
    #[cfg(target_os = "linux")]
    {
        let local = target.strip_prefix("file://").unwrap_or(target);
        if Path::new(local).is_absolute() && Path::new(local).exists() && crate::xdg::open(Path::new(local)).is_ok() {
            return Ok(());
        }
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.open");
        let mut cmd = if super::apps::which("gio").is_some() {
            let mut c = Command::new("gio");
            c.arg("open").arg(target);
            c
        } else {
            let mut c = Command::new("xdg-open");
            c.arg(target);
            c
        };
        span.end(spawn_detached(&mut cmd)).map_err(|e| anyhow!("{target} could not be opened: {e}"))
    }
}

#[cfg(target_os = "linux")]
fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    for v in ["SETU_DATA_DIR", "SETU_RUNTIME_DIR", "SETU_ASSETS", "SETU_LOG", "LD_LIBRARY_PATH"] {
        cmd.env_remove(v);
    }
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn open_with(app: &App, args: &[String]) -> Result<()> {
    #[cfg(windows)]
    {
        let p = Path::new(&app.path);
        let params: Vec<String> = args.iter().map(|a| quote(a)).collect();
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            && let Some((exe, base)) = shortcut_target(p)
            && exe.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            let all = if base.trim().is_empty() { params.join(" ") } else { format!("{} {}", base.trim(), params.join(" ")) };
            let span = crate::layers::effect(crate::layers::Layer::System, "system.launch");
            return span.end(shell_execute(&exe.to_string_lossy(), &all, exe.parent()));
        }
        let span = crate::layers::effect(crate::layers::Layer::System, "system.launch");
        span.end(shell_execute(&app.path, &params.join(" "), None))
    }
    #[cfg(target_os = "macos")]
    {
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.launch");
        let ok = span.end_bool(Command::new("open").arg("-a").arg(&app.path).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(()) } else { Err(anyhow!("{} could not be started.", app.name)) }
    }
    #[cfg(target_os = "linux")]
    {
        if app.path.ends_with(".desktop")
            && !app.manual
            && let Some(e) = crate::xdg::entry(Path::new(&app.path)).filter(|e| !e.terminal)
        {
            let files: Vec<std::path::PathBuf> = args.iter().map(std::path::PathBuf::from).collect();
            if crate::xdg::launch(&e, &files).is_ok() {
                return Ok(());
            }
        }
        let helper = app.path.ends_with(".desktop") && super::apps::which("gio").is_some() && !app.manual;
        let mut cmd = if helper {
            let mut c = Command::new("gio");
            c.arg("launch").arg(&app.path).args(args);
            c
        } else {
            let argv = super::apps::exec_with(&app.exec, args);
            let (prog, rest) = argv.split_first().ok_or_else(|| anyhow!("{} has no command to start it.", app.name))?;
            let mut c = Command::new(prog);
            c.args(rest);
            c
        };
        let span = if helper { crate::layers::effect(crate::layers::Layer::Command, "command.launch") } else { crate::layers::effect(crate::layers::Layer::System, "system.launch") };
        span.end(spawn_detached(&mut cmd)).map_err(|e| anyhow!("{} could not be started: {e}", app.name))
    }
}

pub fn launch_args(app: &App, args: &[String]) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let flags = args.iter().any(|a| a.starts_with('-'));
        let mut c = Command::new("open");
        if flags {
            c.arg("-na").arg(&app.path).arg("--args").args(args);
        } else {
            c.arg("-a").arg(&app.path).args(args);
        }
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.launch");
        let ok = span.end_bool(c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(()) } else { Err(anyhow!("{} could not be started.", app.name)) }
    }
    #[cfg(target_os = "linux")]
    {
        let mut all = super::apps::a11y_args(app);
        all.extend(args.iter().cloned());
        let argv = super::apps::exec_with(&app.exec, &all);
        let (prog, rest) = argv.split_first().ok_or_else(|| anyhow!("{} has no command to start it.", app.name))?;
        let mut cmd = Command::new(prog);
        cmd.args(rest);
        let span = crate::layers::effect(crate::layers::Layer::System, "system.launch");
        span.end(spawn_detached(&mut cmd)).map_err(|e| anyhow!("{} could not be started: {e}", app.name))
    }
    #[cfg(windows)]
    {
        open_with(app, args)
    }
}

pub fn copy_text(text: &str) -> Result<()> {
    #[cfg(windows)]
    let span = crate::layers::effect(crate::layers::Layer::System, "system.clipboard");
    #[cfg(not(windows))]
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.clipboard");
    span.end(copy_text_now(text))
}

fn copy_text_now(text: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let tries: Vec<(&str, Vec<&str>)> = if wayland {
            vec![("wl-copy", vec![]), ("xclip", vec!["-selection", "clipboard"]), ("xsel", vec!["--clipboard", "--input"])]
        } else {
            vec![("xclip", vec!["-selection", "clipboard"]), ("xsel", vec!["--clipboard", "--input"]), ("wl-copy", vec![])]
        };
        for (prog, args) in tries {
            if super::apps::which(prog).is_none() {
                continue;
            }
            let child = Command::new(prog).args(&args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            if let Ok(mut c) = child {
                if let Some(mut input) = c.stdin.take() {
                    use std::io::Write;
                    let _ = input.write_all(text.as_bytes());
                }
                if c.wait().map(|s| s.success()).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        Err(anyhow!("No clipboard tool was found. Install wl-clipboard or xclip."))
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        let mut c = Command::new("pbcopy").stdin(Stdio::piped()).spawn()?;
        if let Some(mut input) = c.stdin.take() {
            input.write_all(text.as_bytes())?;
        }
        c.wait()?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{HANDLE, HGLOBAL};
        use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
        use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            OpenClipboard(None)?;
            let r = (|| -> Result<()> {
                EmptyClipboard()?;
                let mem: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2)?;
                let p = GlobalLock(mem) as *mut u16;
                if p.is_null() {
                    return Err(anyhow!("The clipboard could not be used."));
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(mem);
                SetClipboardData(13, Some(HANDLE(mem.0)))?;
                Ok(())
            })();
            let _ = CloseClipboard();
            r
        }
    }
}

pub fn paste_text() -> Option<String> {
    #[cfg(windows)]
    let span = crate::layers::read(crate::layers::Layer::System, "system.clipboard.read");
    #[cfg(not(windows))]
    let span = crate::layers::read(crate::layers::Layer::Command, "command.clipboard.read");
    span.end_some(paste_text_now())
}

fn paste_text_now() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let tries: Vec<(&str, Vec<&str>)> = if wayland {
            vec![("wl-paste", vec!["--no-newline", "--type", "text"]), ("xclip", vec!["-selection", "clipboard", "-o"]), ("xsel", vec!["--clipboard", "--output"])]
        } else {
            vec![("xclip", vec!["-selection", "clipboard", "-o"]), ("xsel", vec!["--clipboard", "--output"]), ("wl-paste", vec!["--no-newline", "--type", "text"])]
        };
        for (prog, args) in tries {
            if super::apps::which(prog).is_none() {
                continue;
            }
            if let Ok((true, out, _)) = output(prog, &args, None, std::time::Duration::from_secs(2)) {
                return Some(out);
            }
            return None;
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        output("pbpaste", &[], None, std::time::Duration::from_secs(2)).ok().filter(|(ok, _, _)| *ok).map(|(_, o, _)| o)
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
        use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        unsafe {
            OpenClipboard(None).ok()?;
            let out = (|| {
                let h = GetClipboardData(13).ok()?;
                let mem = HGLOBAL(h.0);
                let p = GlobalLock(mem) as *const u16;
                if p.is_null() {
                    return None;
                }
                let mut n = 0usize;
                while *p.add(n) != 0 && n < 8 * 1024 * 1024 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                let _ = GlobalUnlock(mem);
                Some(s)
            })();
            let _ = CloseClipboard();
            out
        }
    }
}

pub fn run_detached(program: &Path, args: &[String], dir: Option<&Path>) -> Result<()> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.launch");
    span.end(run_detached_now(program, args, dir))
}

fn run_detached_now(program: &Path, args: &[String], dir: Option<&Path>) -> Result<()> {
    #[cfg(windows)]
    {
        let params: Vec<String> = args.iter().map(|a| quote(a)).collect();
        shell_execute(&program.to_string_lossy(), &params.join(" "), dir)
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        #[cfg(target_os = "linux")]
        {
            spawn_detached(&mut cmd).map_err(|e| anyhow!("{} could not be started: {e}", program.display()))
        }
        #[cfg(target_os = "macos")]
        {
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ()).map_err(|e| anyhow!("{} could not be started: {e}", program.display()))
        }
    }
}

pub fn trash(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("{} no longer exists.", path.display()));
    }
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW, SHFileOperationW};
        use windows::core::PCWSTR;
        let mut from: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
        from.push(0);
        from.push(0);
        let mut op = SHFILEOPSTRUCTW { wFunc: FO_DELETE, pFrom: PCWSTR(from.as_ptr()), fFlags: (FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_SILENT.0 | FOF_NOERRORUI.0) as u16, ..Default::default() };
        let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
        let r = unsafe { SHFileOperationW(&mut op) };
        span.end_bool(r == 0);
        if r == 0 { Ok(()) } else { Err(anyhow!("Windows could not move it to the Recycle Bin (error {r}).")) }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", path.to_string_lossy().replace('"', "\\\""));
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.osascript");
        let ok = span.end_bool(Command::new("osascript").args(["-e", &script]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false));
        if ok { Ok(()) } else { Err(anyhow!("Finder could not move it to the Trash.")) }
    }
    #[cfg(target_os = "linux")]
    {
        {
            let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
            if span.end_bool(crate::xdg::trash(path).is_ok()) {
                return Ok(());
            }
        }
        if super::apps::which("gio").is_some() {
            let span = crate::layers::effect(crate::layers::Layer::Command, "command.trash");
            if span.end_bool(Command::new("gio").arg("trash").arg(path).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)) {
                return Ok(());
            }
        }
        let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
        let home = dirs::data_dir().ok_or_else(|| anyhow!("No trash folder was found."))?.join("Trash");
        std::fs::create_dir_all(home.join("files"))?;
        std::fs::create_dir_all(home.join("info"))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "item".into());
        let mut dest = home.join("files").join(&name);
        let mut k = 1;
        while dest.exists() {
            dest = home.join("files").join(format!("{name}.{k}"));
            k += 1;
        }
        let stem = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let info = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", path.display(), jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S"));
        std::fs::write(home.join("info").join(format!("{stem}.trashinfo")), info)?;
        span.end(std::fs::rename(path, &dest)).map_err(|e| anyhow!("It could not be moved to the trash: {e}"))
    }
}

pub fn output(program: &str, args: &[&str], dir: Option<&Path>, timeout: std::time::Duration) -> Result<(bool, String, String)> {
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.run");
    let r = output_now(program, args, dir, timeout);
    span.end_bool(r.as_ref().is_ok_and(|(ok, _, _)| *ok));
    r
}

pub fn output_quiet(program: &str, args: &[&str], dir: Option<&Path>, timeout: std::time::Duration) -> Result<(bool, String, String)> {
    output_now(program, args, dir, timeout)
}

fn output_now(program: &str, args: &[&str], dir: Option<&Path>, timeout: std::time::Duration) -> Result<(bool, String, String)> {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.quiet();
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C").env("LANG", "C");
    let mut child = cmd.spawn().map_err(|e| anyhow!("{program} could not start: {e}"))?;
    let out = child.stdout.take();
    let err = child.stderr.take();
    let ro = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut o) = out {
            let _ = std::io::Read::read_to_string(&mut o, &mut s);
        }
        s
    });
    let re = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = err {
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
        }
        s
    });
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        let waited = start.elapsed();
        if waited > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("{program} took too long and was stopped."));
        }
        let nap = if waited.as_millis() < 50 {
            1
        } else if waited.as_millis() < 500 {
            5
        } else {
            20
        };
        std::thread::sleep(std::time::Duration::from_millis(nap));
    };
    Ok((status.success(), ro.join().unwrap_or_default(), re.join().unwrap_or_default()))
}

pub fn find_program(names: &[&str], extra: &[PathBuf]) -> Option<PathBuf> {
    for d in extra {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let paths = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&paths) {
        for n in names {
            let p = d.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}
