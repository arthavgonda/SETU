use crate::actions::Host;
use crate::exec::Mode;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{DuplicateTokenEx, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SecurityImpersonation, SetTokenInformation, TOKEN_ALL_ACCESS, TokenPrimary, TokenSessionId};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, CreatePipe, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW, OpenServiceW, RegisterServiceCtrlHandlerExW, SC_MANAGER_ALL_ACCESS, SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_CONTROL_STOP, SERVICE_ERROR_NORMAL, SERVICE_RUNNING, SERVICE_STATUS,
    SERVICE_STATUS_CURRENT_STATE, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW, StartServiceW,
};
use windows::Win32::System::StationsAndDesktops::{DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, OpenInputDesktop, SetThreadDesktop};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken, PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTF_USESTDHANDLES, STARTUPINFOW, WaitForSingleObject,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VK_MENU, VK_RETURN};
use windows::core::{PCWSTR, PWSTR};

const NAME: &str = "SetuSystem";
static STOP: AtomicBool = AtomicBool::new(false);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("Setu")
}

fn config() -> Option<(String, Option<PathBuf>)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(data_dir().join("system.json")).ok()?).ok()?;
    let sid = v.get("sid")?.as_str()?.to_owned();
    if !sid.starts_with("S-1-") || !sid.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S') {
        return None;
    }
    Some((sid, v.get("daemon").and_then(Value::as_str).map(PathBuf::from)))
}

struct Pipe(HANDLE);

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        unsafe { ReadFile(self.0, Some(buf), Some(&mut n), None) }.map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(n as usize)
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        unsafe { WriteFile(self.0, Some(buf), Some(&mut n), None) }.map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(n as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        unsafe { FlushFileBuffers(self.0) }.map_err(|e| std::io::Error::other(e.to_string()))
    }
}

fn client_exe(pipe: HANDLE) -> Option<PathBuf> {
    unsafe {
        let mut pid = 0u32;
        GetNamedPipeClientProcessId(pipe, &mut pid).ok()?;
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

fn create_pipe(sid: &str) -> Result<HANDLE, String> {
    let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GRGW;;;{sid})"));
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None) }.map_err(|e| e.to_string())?;
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
    let name = wide(crate::install::SOCKET);
    let h = unsafe { CreateNamedPipeW(PCWSTR(name.as_ptr()), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, Some(&sa)) };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    if h == INVALID_HANDLE_VALUE { Err("the pipe could not be created".into()) } else { Ok(h) }
}

fn serve_client(pipe: HANDLE, daemon: Option<PathBuf>, sid: &str, host: &Host, mode: &Mode) {
    if let Some(want) = &daemon
        && !client_exe(pipe).is_some_and(|got| same(&got, want))
    {
        return;
    }
    let mut r = std::io::BufReader::new(Pipe(pipe));
    let mut w = Pipe(pipe);
    let mut line = String::new();
    loop {
        line.clear();
        match std::io::BufRead::read_line(&mut r, &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.len() > 64 * 1024 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let face = |a: &str, args: &Value, _: u32| {
            if a == "setup.window" {
                return Some(crate::winsetup::request(args).and_then(|req| on_desktop(&req)));
            }
            a.starts_with("face.").then(|| crate::winface::handle(a, args, sid))
        };
        let reply = crate::proto::handle(line.trim(), 0, host, mode, &secure, &face);
        if writeln!(w, "{reply}").is_err() {
            break;
        }
    }
}

pub fn serve(mode: Mode) -> Result<(), String> {
    let (sid, daemon) = config().ok_or("The system helper isn't set up.")?;
    let host = std::sync::Arc::new(Host::detect());
    let mode = std::sync::Arc::new(mode);
    while !STOP.load(Ordering::SeqCst) {
        let pipe = create_pipe(&sid)?;
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if connected.is_err() && windows::core::Error::from_win32().code() != ERROR_PIPE_CONNECTED.to_hresult() {
            unsafe {
                let _ = CloseHandle(pipe);
            }
            continue;
        }
        let (host, mode, daemon, sid) = (host.clone(), mode.clone(), daemon.clone(), sid.clone());
        let raw = pipe.0 as usize;
        std::thread::spawn(move || {
            let pipe = HANDLE(raw as *mut _);
            serve_client(pipe, daemon, &sid, &host, &mode);
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }
        });
    }
    Ok(())
}

static mut STATUS_HANDLE: SERVICE_STATUS_HANDLE = SERVICE_STATUS_HANDLE(std::ptr::null_mut());

fn report(state: SERVICE_STATUS_CURRENT_STATE) {
    let st = SERVICE_STATUS { dwServiceType: SERVICE_WIN32_OWN_PROCESS, dwCurrentState: state, dwControlsAccepted: if state == SERVICE_RUNNING { 1 } else { 0 }, ..Default::default() };
    unsafe {
        let _ = SetServiceStatus(STATUS_HANDLE, &st);
    }
}

unsafe extern "system" fn control(code: u32, _: u32, _: *mut core::ffi::c_void, _: *mut core::ffi::c_void) -> u32 {
    if code == SERVICE_CONTROL_STOP {
        STOP.store(true, Ordering::SeqCst);
        report(SERVICE_STOPPED);
    }
    0
}

unsafe extern "system" fn service_main(_: u32, _: *mut PWSTR) {
    let name = wide(NAME);
    let Ok(h) = (unsafe { RegisterServiceCtrlHandlerExW(PCWSTR(name.as_ptr()), Some(control), None) }) else { return };
    unsafe {
        STATUS_HANDLE = h;
    }
    report(SERVICE_RUNNING);
    let _ = serve(Mode::Real);
    report(SERVICE_STOPPED);
}

pub fn run_service() -> Result<(), String> {
    let mut name = wide(NAME);
    let table = [SERVICE_TABLE_ENTRYW { lpServiceName: PWSTR(name.as_mut_ptr()), lpServiceProc: Some(service_main) }, SERVICE_TABLE_ENTRYW::default()];
    unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) }.map_err(|e| e.to_string())
}

pub fn install(sid: &str, daemon: &Path) -> Result<String, String> {
    if !sid.starts_with("S-1-") {
        return Err("That isn't a user id.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("Setu");
    let bin = dir.join("setu-system.exe");
    if !same(&exe, &bin) {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::copy(&exe, &bin).map_err(|e| e.to_string())?;
        if let Some(from) = exe.parent()
            && let Err(e) = crate::winface::copy_files(from)
        {
            eprintln!("setu-system: face sign-in files not copied: {e}");
        }
    }
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    std::fs::write(data_dir().join("system.json"), json!({ "sid": sid, "daemon": daemon }).to_string()).map_err(|e| e.to_string())?;
    let cmd = wide(&format!("\"{}\" --service", bin.display()));
    let name = wide(NAME);
    let display = wide("Setu system helper");
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS).map_err(|e| e.to_string())?;
        let svc = match OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) {
            Ok(s) => s,
            Err(_) => CreateServiceW(scm, PCWSTR(name.as_ptr()), PCWSTR(display.as_ptr()), SERVICE_ALL_ACCESS, SERVICE_WIN32_OWN_PROCESS, SERVICE_AUTO_START, SERVICE_ERROR_NORMAL, PCWSTR(cmd.as_ptr()), PCWSTR::null(), None, PCWSTR::null(), PCWSTR::null(), PCWSTR::null())
                .map_err(|e| e.to_string())?,
        };
        let _ = StartServiceW(svc, None);
        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }
    Ok(format!("installed {}", bin.display()))
}

pub fn uninstall() -> Result<String, String> {
    let name = wide(NAME);
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_ALL_ACCESS).map_err(|e| e.to_string())?;
        if let Ok(svc) = OpenServiceW(scm, PCWSTR(name.as_ptr()), SERVICE_ALL_ACCESS) {
            let mut st = SERVICE_STATUS::default();
            let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut st);
            let _ = DeleteService(svc);
            let _ = CloseServiceHandle(svc);
        }
        let _ = CloseServiceHandle(scm);
    }
    crate::winface::remove_all();
    let _ = std::fs::remove_file(data_dir().join("system.json"));
    Ok("removed".into())
}

fn secure(req: &Value) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let payload = req.to_string();
    unsafe {
        let session = WTSGetActiveConsoleSessionId();
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token).map_err(|e| e.to_string())?;
        let mut primary = HANDLE::default();
        DuplicateTokenEx(token, TOKEN_ALL_ACCESS, None, SecurityImpersonation, TokenPrimary, &mut primary).map_err(|e| e.to_string())?;
        let _ = CloseHandle(token);
        SetTokenInformation(primary, TokenSessionId, &session as *const u32 as *const _, 4).map_err(|e| e.to_string())?;
        let mut read = HANDLE::default();
        let mut write = HANDLE::default();
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
        CreatePipe(&mut read, &mut write, Some(&sa), 0).map_err(|e| e.to_string())?;
        let mut desktop = wide("winsta0\\winlogon");
        let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, lpDesktop: PWSTR(desktop.as_mut_ptr()), dwFlags: STARTF_USESTDHANDLES, hStdInput: read, ..Default::default() };
        let mut pi = PROCESS_INFORMATION::default();
        let app = wide(&exe.to_string_lossy());
        let mut cmd = wide(&format!("\"{}\" --helper", exe.display()));
        let made = CreateProcessAsUserW(Some(primary), PCWSTR(app.as_ptr()), Some(PWSTR(cmd.as_mut_ptr())), None, None, true, CREATE_NO_WINDOW, None, PCWSTR::null(), &si, &mut pi);
        let _ = CloseHandle(read);
        let _ = CloseHandle(primary);
        if let Err(e) = made {
            let _ = CloseHandle(write);
            return Err(e.to_string());
        }
        let mut p = Pipe(write);
        let _ = p.write_all(payload.as_bytes());
        let _ = CloseHandle(write);
        WaitForSingleObject(pi.hProcess, 15000);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(pi.hProcess, &mut code);
        let _ = CloseHandle(pi.hProcess);
        let _ = CloseHandle(pi.hThread);
        if code == 0 { Ok(json!({ "done": true })) } else { Err("The secure screen didn't take the input.".into()) }
    }
}

fn on_desktop(req: &Value) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let payload = req.to_string();
    unsafe {
        let session = WTSGetActiveConsoleSessionId();
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token).map_err(|e| e.to_string())?;
        let mut primary = HANDLE::default();
        DuplicateTokenEx(token, TOKEN_ALL_ACCESS, None, SecurityImpersonation, TokenPrimary, &mut primary).map_err(|e| e.to_string())?;
        let _ = CloseHandle(token);
        SetTokenInformation(primary, TokenSessionId, &session as *const u32 as *const _, 4).map_err(|e| e.to_string())?;
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: std::ptr::null_mut(), bInheritHandle: true.into() };
        let (mut in_r, mut in_w, mut out_r, mut out_w) = (HANDLE::default(), HANDLE::default(), HANDLE::default(), HANDLE::default());
        CreatePipe(&mut in_r, &mut in_w, Some(&sa), 0).map_err(|e| e.to_string())?;
        CreatePipe(&mut out_r, &mut out_w, Some(&sa), 0).map_err(|e| e.to_string())?;
        let mut desktop = wide("winsta0\\default");
        let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, lpDesktop: PWSTR(desktop.as_mut_ptr()), dwFlags: STARTF_USESTDHANDLES, hStdInput: in_r, hStdOutput: out_w, ..Default::default() };
        let mut pi = PROCESS_INFORMATION::default();
        let app = wide(&exe.to_string_lossy());
        let mut cmd = wide(&format!("\"{}\" --helper", exe.display()));
        let made = CreateProcessAsUserW(Some(primary), PCWSTR(app.as_ptr()), Some(PWSTR(cmd.as_mut_ptr())), None, None, true, CREATE_NO_WINDOW, None, PCWSTR::null(), &si, &mut pi);
        let _ = CloseHandle(in_r);
        let _ = CloseHandle(out_w);
        let _ = CloseHandle(primary);
        if let Err(e) = made {
            let _ = CloseHandle(in_w);
            let _ = CloseHandle(out_r);
            return Err(e.to_string());
        }
        let mut p = Pipe(in_w);
        let _ = p.write_all(payload.as_bytes());
        let _ = CloseHandle(in_w);
        let mut text = String::new();
        let _ = Pipe(out_r).take(4 * 1024 * 1024).read_to_string(&mut text);
        let _ = CloseHandle(out_r);
        WaitForSingleObject(pi.hProcess, 20000);
        let _ = CloseHandle(pi.hProcess);
        let _ = CloseHandle(pi.hThread);
        let v: Value = serde_json::from_str(text.trim()).map_err(|_| "The setup helper gave no answer.".to_owned())?;
        match v.get("error").and_then(Value::as_str) {
            Some(e) => Err(e.to_owned()),
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        }
    }
}

fn key(vk: VIRTUAL_KEY, scan: u16, up: bool, unicode: bool) -> INPUT {
    let mut flags = if unicode { KEYEVENTF_UNICODE } else { Default::default() };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

pub fn helper() -> i32 {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        return 2;
    }
    let req: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let op0 = req.get("op").and_then(Value::as_str).unwrap_or_default();
    if op0.starts_with("setup.") {
        let r = crate::winsetup::helper(op0, req.get("args").unwrap_or(&Value::Null));
        let out = match &r {
            Ok(v) => json!({ "result": v }),
            Err(e) => json!({ "error": e }),
        };
        let mut so = std::io::stdout().lock();
        let _ = writeln!(so, "{out}");
        let _ = so.flush();
        return if r.is_ok() { 0 } else { 8 };
    }
    unsafe {
        let Ok(desk) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_ACCESS_FLAGS(0x10000000)) else { return 3 };
        if SetThreadDesktop(desk).is_err() {
            return 4;
        }
    }
    let op = req.get("op").and_then(Value::as_str).unwrap_or_default();
    let args = req.get("args").cloned().unwrap_or(Value::Null);
    let mut inputs: Vec<INPUT> = Vec::new();
    match op {
        "secure.keys" => {
            let t = args.get("text").and_then(Value::as_str).unwrap_or_default();
            for u in t.encode_utf16() {
                inputs.push(key(VIRTUAL_KEY(0), u, false, true));
                inputs.push(key(VIRTUAL_KEY(0), u, true, true));
            }
            if args.get("enter").and_then(Value::as_bool).unwrap_or(false) {
                inputs.push(key(VK_RETURN, 0, false, false));
                inputs.push(key(VK_RETURN, 0, true, false));
            }
        }
        "secure.click" => {
            let which = args.get("name").and_then(Value::as_str).unwrap_or_default().to_lowercase();
            let letter = match which.as_str() {
                "yes" => b'Y',
                "no" => b'N',
                _ => return 5,
            };
            inputs.push(key(VK_MENU, 0, false, false));
            inputs.push(key(VIRTUAL_KEY(letter as u16), 0, false, false));
            inputs.push(key(VIRTUAL_KEY(letter as u16), 0, true, false));
            inputs.push(key(VK_MENU, 0, true, false));
        }
        _ => return 6,
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() { 0 } else { 7 }
}
