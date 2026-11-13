use anyhow::Result;
use windows::Win32::Foundation::{CloseHandle, HWND, POINT};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextW, GetWindowThreadProcessId};
use windows::core::PWSTR;

struct Com;

impl Com {
    fn init() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Com)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub fn dpi_aware() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn endpoint() -> Result<(Com, IAudioEndpointVolume)> {
    let com = Com::init()?;
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        Ok((com, volume))
    }
}

pub fn volume() -> Result<(i64, bool)> {
    let (_com, v) = endpoint()?;
    unsafe {
        let level = v.GetMasterVolumeLevelScalar()?;
        let muted = v.GetMute()?.as_bool();
        Ok(((level * 100.0).round() as i64, muted))
    }
}

pub fn set_volume(level: i64) -> Result<()> {
    let (_com, v) = endpoint()?;
    unsafe {
        v.SetMasterVolumeLevelScalar(level.clamp(0, 100) as f32 / 100.0, std::ptr::null())?;
        if level > 0 {
            v.SetMute(false, std::ptr::null())?;
        }
    }
    Ok(())
}

pub fn set_muted(muted: bool) -> Result<()> {
    let (_com, v) = endpoint()?;
    unsafe { v.SetMute(muted, std::ptr::null())? };
    Ok(())
}

pub fn scale_at(x: i32, y: i32) -> f64 {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 {
            dx as f64 / 96.0
        } else {
            1.0
        }
    }
}



fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn exe_path_of(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(h);
        if ok { String::from_utf16_lossy(&buf[..size as usize]) } else { String::new() }
    }
}

fn process_name(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

pub fn title_of(h: HWND) -> String {
    window_text(h)
}

pub fn exe_of(h: HWND) -> String {
    process_name(h)
}

pub fn friendly(exe: &str) -> String {
    match exe.to_lowercase().as_str() {
        "brave" => "Brave".into(),
        "chrome" => "Google Chrome".into(),
        "msedge" => "Microsoft Edge".into(),
        "firefox" => "Firefox".into(),
        "code" => "Visual Studio Code".into(),
        "explorer" => "File Explorer".into(),
        "windowsterminal" => "Terminal".into(),
        "cmd" => "Command Prompt".into(),
        "powershell" | "pwsh" => "PowerShell".into(),
        other => crate::text::sentence_case(other),
    }
}

pub fn authenticode(path: &std::path::Path) -> bool {
    use windows::Win32::Security::WinTrust::{WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust};
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let mut file = WINTRUST_FILE_INFO { cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32, pcwszFilePath: windows::core::PCWSTR(wide.as_ptr()), ..Default::default() };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let r = unsafe { WinVerifyTrust(HWND(-1isize as *mut _), &mut action, &mut data as *mut _ as *mut _) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(HWND(-1isize as *mut _), &mut action, &mut data as *mut _ as *mut _);
    }
    r == 0
}

fn elevated(file: &std::path::Path, params: &str) -> Result<u32, String> {
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let f: Vec<u16> = file.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let p: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
    let mut info = SHELLEXECUTEINFOW { cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32, fMask: SEE_MASK_NOCLOSEPROCESS, lpVerb: windows::core::PCWSTR(verb.as_ptr()), lpFile: windows::core::PCWSTR(f.as_ptr()), lpParameters: windows::core::PCWSTR(p.as_ptr()), nShow: 1, ..Default::default() };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| "The install was not approved.".to_owned())?;
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        Ok(code)
    }
}

fn opened(file: &std::path::Path) -> Result<u32, String> {
    use windows::Win32::System::Threading::GetProcessId;
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let f: Vec<u16> = file.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let mut info = SHELLEXECUTEINFOW { cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32, fMask: SEE_MASK_NOCLOSEPROCESS, lpVerb: windows::core::PCWSTR(verb.as_ptr()), lpFile: windows::core::PCWSTR(f.as_ptr()), nShow: 1, ..Default::default() };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|_| "The installer didn't start.".to_owned())?;
        let pid = if info.hProcess.is_invalid() { 0 } else { GetProcessId(info.hProcess) };
        if !info.hProcess.is_invalid() {
            let _ = CloseHandle(info.hProcess);
        }
        Ok(pid)
    }
}

pub fn installer_family(head: &[u8]) -> &'static str {
    let has = |n: &[u8]| head.windows(n.len()).any(|w| w == n);
    if has(b"Inno Setup") {
        "inno"
    } else if has(b"Nullsoft") || has(b"NSIS Error") {
        "nsis"
    } else if has(b"InstallShield") {
        "installshield"
    } else if has(b".wixburn") || has(b"WixBundle") {
        "wix"
    } else {
        ""
    }
}

pub fn run_installer(file: &std::path::Path, target: Option<&std::path::Path>) -> Result<(), String> {
    let name = file.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let dir = target.map(|t| t.to_string_lossy().into_owned());
    let (exe, params) = if name.ends_with(".msi") {
        let mut p = format!("/i \"{}\" /passive /norestart", file.display());
        if let Some(d) = &dir {
            p.push_str(&format!(" INSTALLDIR=\"{d}\""));
        }
        (std::path::PathBuf::from("msiexec.exe"), p)
    } else {
        let mut head = vec![0u8; 4 << 20];
        let n = std::fs::File::open(file).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
        head.truncate(n);
        let p = match installer_family(&head) {
            "inno" => format!("/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-{}", dir.as_ref().map(|d| format!(" /DIR=\"{d}\"")).unwrap_or_default()),
            "nsis" => format!("/S{}", dir.as_ref().map(|d| format!(" /D={d}")).unwrap_or_default()),
            "installshield" => "/s /v\"/qn\"".to_owned(),
            "wix" => "/quiet /norestart".to_owned(),
            _ => {
                let pid = opened(file)?;
                return Err(format!("\u{1}setup:{pid}"));
            }
        };
        (file.to_path_buf(), p)
    };
    match elevated(&exe, &params)? {
        0 | 3010 => Ok(()),
        1602 | 1223 => Err("The install was cancelled.".into()),
        c => Err(format!("The installer stopped with code {c}.")),
    }
}

fn link_dirs() -> Vec<std::path::PathBuf> {
    let app = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    let mut v = Vec::new();
    if let Some(a) = &app {
        v.push(a.join(r"Microsoft\Windows\Start Menu\Programs"));
        v.push(a.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"));
    }
    if let Some(d) = dirs::desktop_dir() {
        v.push(d);
    }
    v
}

fn links_in(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 3 {
            links_in(&p, out, depth + 1);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            out.push(p);
        }
    }
}

pub fn retarget_shortcuts(exe: &std::path::Path, me: &std::path::Path, key: &str, backup: &std::path::Path) -> Result<usize> {
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::{IPersistFile, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, PCWSTR};
    let _com = Com::init();
    std::fs::create_dir_all(backup)?;
    let mut map: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(backup.join("map.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut links = Vec::new();
    for d in link_dirs() {
        links_in(&d, &mut links, 0);
    }
    let want = exe.to_string_lossy().to_lowercase();
    let mut n = 0;
    for l in links {
        unsafe {
            let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_ALL) else { continue };
            let Ok(file) = link.cast::<IPersistFile>() else { continue };
            if file.Load(&HSTRING::from(l.as_os_str()), STGM_READ).is_err() {
                continue;
            }
            let mut buf = [0u16; 1024];
            let mut fd = WIN32_FIND_DATAW::default();
            if link.GetPath(&mut buf, &mut fd, 0).is_err() {
                continue;
            }
            let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            let target = String::from_utf16_lossy(&buf[..len]).to_lowercase();
            if target != want {
                continue;
            }
            let name = format!("{}.lnk", map.len() + 1);
            if std::fs::copy(&l, backup.join(&name)).is_err() {
                continue;
            }
            map.insert(name, serde_json::json!(l));
            let args = format!("--browser {key} --exe \"{}\"", exe.display());
            let _ = link.SetPath(&HSTRING::from(me.as_os_str()));
            let _ = link.SetArguments(&HSTRING::from(args));
            let _ = link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0);
            if file.Save(PCWSTR::null(), true).is_ok() {
                n += 1;
            }
        }
    }
    std::fs::write(backup.join("map.json"), serde_json::Value::Object(map).to_string())?;
    Ok(n)
}

pub fn restore_shortcuts(backup: &std::path::Path) -> Result<usize> {
    let map: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(backup.join("map.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut n = 0;
    for (name, orig) in &map {
        if let Some(o) = orig.as_str()
            && std::fs::copy(backup.join(name), o).is_ok()
        {
            n += 1;
        }
    }
    let _ = std::fs::remove_dir_all(backup);
    Ok(n)
}

pub fn capture_front() -> Result<(usize, usize, Vec<u8>, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject};
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r)?;
        let (w, h) = ((r.right - r.left).max(1), (r.bottom - r.top).max(1));
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, bmp.into());
        let ok = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        let n = (w * h) as usize;
        let mut rgb = Vec::with_capacity(n * 3);
        if ok && !bits.is_null() {
            let px = std::slice::from_raw_parts(bits as *const u8, n * 4);
            for p in px.chunks_exact(4) {
                rgb.extend_from_slice(&[p[2], p[1], p[0]]);
            }
        }
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        if !ok {
            return Err(anyhow::anyhow!("The window couldn't be captured."));
        }
        Ok((w as usize, h as usize, rgb, r.left, r.top))
    }
}

