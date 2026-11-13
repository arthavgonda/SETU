use crate::tools::Outcome;
use anyhow::{Result, anyhow};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LUID, WPARAM};
use windows::core::{PCWSTR, w};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn not_here(what: &str) -> anyhow::Error {
    anyhow!("Windows doesn't let apps change {what}. It's in Settings.")
}

pub async fn night_light(_on: bool) -> Result<()> {
    Err(not_here("night light"))
}

pub async fn dnd(_on: bool) -> Result<()> {
    Err(not_here("do not disturb"))
}

fn set_dword(path: &str, name: &str, value: u32) -> Result<()> {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, RegCloseKey, RegOpenKeyExW, RegSetValueExW};
    let p = wide(path);
    let n = wide(name);
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), Some(0), KEY_SET_VALUE, &mut key).ok().map_err(|e| anyhow!("{e}"))?;
        let r = RegSetValueExW(key, PCWSTR(n.as_ptr()), Some(0), REG_DWORD, Some(&value.to_le_bytes()));
        let _ = RegCloseKey(key);
        r.ok().map_err(|e| anyhow!("{e}"))
    }
}

pub async fn dark_mode(on: bool) -> Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
    let span = crate::layers::effect(crate::layers::Layer::System, "system.settings");
    let key = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let r = set_dword(key, "AppsUseLightTheme", if on { 0 } else { 1 }).and_then(|_| set_dword(key, "SystemUsesLightTheme", if on { 0 } else { 1 }));
    if r.is_ok() {
        let topic = wide("ImmersiveColorSet");
        unsafe {
            let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(topic.as_ptr() as isize), SMTO_ABORTIFHUNG, 2000, None);
        }
    }
    span.end(r)
}

fn radios(kind: windows::Devices::Radios::RadioKind, on: bool) -> Result<usize> {
    use windows::Devices::Radios::{Radio, RadioState};
    let _ = Radio::RequestAccessAsync().and_then(|a| a.get());
    let list = Radio::GetRadiosAsync().and_then(|a| a.get()).map_err(|e| anyhow!("{e}"))?;
    let mut n = 0;
    for r in list {
        if r.Kind().ok() == Some(kind) && r.SetStateAsync(if on { RadioState::On } else { RadioState::Off }).and_then(|a| a.get()).is_ok() {
            n += 1;
        }
    }
    if n == 0 { Err(anyhow!("No such radio could be switched.")) } else { Ok(n) }
}

pub async fn wifi(on: bool) -> Result<()> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.network");
    span.end(crate::layers::blocking(move || radios(windows::Devices::Radios::RadioKind::WiFi, on).map(|_| ())).await.map_err(|e| anyhow!("{e}")).and_then(|r| r))
}

pub async fn bluetooth(on: bool) -> Result<usize> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.bluetooth");
    span.end(crate::layers::blocking(move || radios(windows::Devices::Radios::RadioKind::Bluetooth, on)).await.map_err(|e| anyhow!("{e}")).and_then(|r| r))
}

pub async fn airplane(on: bool) -> Result<()> {
    let a = wifi(!on).await;
    let b = bluetooth(!on).await;
    if a.is_err() && b.is_err() { Err(anyhow!("Airplane mode couldn't be switched.")) } else { Ok(()) }
}

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
}

mod bt {
    use windows::Win32::Devices::Bluetooth::*;
    use windows::Win32::Foundation::HANDLE;
    use windows::core::GUID;

    pub const SERVICES: [GUID; 5] = [
        GUID::from_u128(0x0000110b_0000_1000_8000_00805f9b34fb),
        GUID::from_u128(0x0000111e_0000_1000_8000_00805f9b34fb),
        GUID::from_u128(0x00001108_0000_1000_8000_00805f9b34fb),
        GUID::from_u128(0x0000110e_0000_1000_8000_00805f9b34fb),
        GUID::from_u128(0x00001124_0000_1000_8000_00805f9b34fb),
    ];

    fn blank() -> BLUETOOTH_DEVICE_INFO {
        BLUETOOTH_DEVICE_INFO { dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() }
    }

    pub fn name(d: &BLUETOOTH_DEVICE_INFO) -> String {
        let n = d.szName.iter().position(|c| *c == 0).unwrap_or(d.szName.len());
        String::from_utf16_lossy(&d.szName[..n])
    }

    pub fn list() -> Vec<BLUETOOTH_DEVICE_INFO> {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnUnknown: false.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 0,
            hRadio: HANDLE::default(),
        };
        let mut out: Vec<BLUETOOTH_DEVICE_INFO> = Vec::new();
        let mut info = blank();
        let Ok(f) = (unsafe { BluetoothFindFirstDevice(&params, &mut info) }) else { return out };
        loop {
            if !out.iter().any(|d| unsafe { d.Address.Anonymous.ullLong == info.Address.Anonymous.ullLong }) {
                out.push(info);
            }
            info = blank();
            if unsafe { BluetoothFindNextDevice(f, &mut info) }.is_err() {
                break;
            }
        }
        let _ = unsafe { BluetoothFindDeviceClose(f) };
        out
    }

    pub fn installed(d: &BLUETOOTH_DEVICE_INFO) -> Vec<GUID> {
        let mut n = 16u32;
        let mut buf = [GUID::zeroed(); 16];
        if unsafe { BluetoothEnumerateInstalledServices(None, d, &mut n, Some(buf.as_mut_ptr())) } != 0 {
            return Vec::new();
        }
        buf[..(n as usize).min(16)].to_vec()
    }

    pub fn set(d: &BLUETOOTH_DEVICE_INFO, g: &GUID, on: bool) -> bool {
        unsafe { BluetoothSetServiceState(None, d, g, if on { BLUETOOTH_SERVICE_ENABLE } else { BLUETOOTH_SERVICE_DISABLE }) == 0 }
    }
}

pub async fn bt_devices() -> Result<Vec<BtDevice>> {
    let span = crate::layers::read(crate::layers::Layer::System, "system.bluetooth.list");
    let list = crate::layers::blocking(|| bt::list().iter().filter(|d| d.fRemembered.as_bool() || d.fAuthenticated.as_bool()).map(|d| BtDevice { name: bt::name(d), connected: d.fConnected.as_bool() }).collect::<Vec<_>>()).await.map_err(|e| anyhow!(e.to_string()))?;
    span.ok();
    Ok(list)
}

fn bt_connect_now(want: &str, on: bool) -> Result<String> {
    let devs: Vec<_> = bt::list().into_iter().filter(|d| d.fRemembered.as_bool() || d.fAuthenticated.as_bool()).collect();
    let w = crate::text::normalize(want);
    let generic = matches!(w.as_str(), "headphones" | "my headphones" | "earbuds" | "my earbuds" | "speaker" | "my speaker" | "headset" | "my headset" | "bluetooth" | "it" | "");
    let named = |d: &&windows::Win32::Devices::Bluetooth::BLUETOOTH_DEVICE_INFO| {
        let n = crate::text::normalize(&bt::name(d));
        !w.is_empty() && (n == w || n.contains(&w) || (w.contains(&n) && n.len() > 2) || crate::os::score(&w, &n) >= 0.8)
    };
    let pick = devs.iter().find(named).or_else(|| if generic { devs.iter().find(|d| d.fConnected.as_bool() != on) } else { None }).copied();
    let Some(d) = pick else {
        let names: Vec<String> = devs.iter().map(bt::name).collect();
        return Err(if names.is_empty() { anyhow!("No Bluetooth devices are paired yet. Pair it once in Settings.") } else { anyhow!("I couldn't find {want}. Paired devices: {}.", names.join(", ")) });
    };
    let name = bt::name(&d);
    if d.fConnected.as_bool() == on {
        return Ok(name);
    }
    let span = crate::layers::effect(crate::layers::Layer::System, "system.bluetooth");
    let have = bt::installed(&d);
    let mut any = false;
    for g in bt::SERVICES.iter() {
        if on {
            if have.contains(g) {
                bt::set(&d, g, false);
            }
            any |= bt::set(&d, g, true);
        } else if have.contains(g) {
            any |= bt::set(&d, g, false);
        }
    }
    if !any {
        span.end_bool(false);
        return Err(anyhow!("Windows didn't {} {name}.", if on { "connect" } else { "disconnect" }));
    }
    if on {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < until {
            if bt::list().iter().any(|x| unsafe { x.Address.Anonymous.ullLong == d.Address.Anonymous.ullLong } && x.fConnected.as_bool()) {
                span.ok();
                return Ok(name);
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        span.end_bool(false);
        return Err(anyhow!("{name} didn't connect. Check it's switched on and near the PC."));
    }
    span.ok();
    Ok(name)
}

pub async fn bt_connect(want: &str, on: bool) -> Result<String> {
    let w = want.to_owned();
    crate::layers::blocking(move || bt_connect_now(&w, on)).await.map_err(|e| anyhow!(e.to_string()))?
}

fn shutdown_privilege() -> Result<()> {
    use windows::Win32::Security::{AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).map_err(|e| anyhow!("{e}"))?;
        let mut luid = LUID::default();
        LookupPrivilegeValueW(PCWSTR::null(), w!("SeShutdownPrivilege"), &mut luid).map_err(|e| anyhow!("{e}"))?;
        let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
        let r = AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None);
        let _ = CloseHandle(token);
        r.map_err(|e| anyhow!("{e}"))
    }
}

pub async fn power(op: &str) -> Result<()> {
    use windows::Win32::System::Power::SetSuspendState;
    use windows::Win32::System::Shutdown::{EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER};
    let span = crate::layers::effect(crate::layers::Layer::System, "system.power");
    let op = op.to_owned();
    let r = crate::layers::blocking(move || -> Result<()> {
        unsafe {
            match op.as_str() {
                "suspend" => if SetSuspendState(false, false, false) { Ok(()) } else { Err(anyhow!("The computer didn't go to sleep.")) },
                "hibernate" => if SetSuspendState(true, false, false) { Ok(()) } else { Err(anyhow!("The computer didn't hibernate.")) },
                "reboot" | "poweroff" => {
                    shutdown_privilege()?;
                    ExitWindowsEx(if op == "reboot" { EWX_REBOOT } else { EWX_POWEROFF }, SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED).map_err(|e| anyhow!("{e}"))
                }
                _ => ExitWindowsEx(EWX_LOGOFF, SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED).map_err(|e| anyhow!("{e}")),
            }
        }
    })
    .await
    .map_err(|e| anyhow!("{e}"))
    .and_then(|r| r);
    span.end(r)
}

#[derive(Debug, Clone, Default)]
pub struct Battery {
    pub present: bool,
    pub percent: f64,
    pub charging: bool,
    pub full: bool,
    pub seconds: i64,
}

pub async fn battery() -> Result<Battery> {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let span = crate::layers::read(crate::layers::Layer::System, "system.battery");
    let mut st = SYSTEM_POWER_STATUS::default();
    span.end(unsafe { GetSystemPowerStatus(&mut st) }.map_err(|e| anyhow!("{e}")))?;
    let present = st.BatteryFlag != 128 && st.BatteryFlag != 255;
    let pct = if st.BatteryLifePercent == 255 { 0.0 } else { st.BatteryLifePercent as f64 };
    let charging = st.ACLineStatus == 1 && pct < 100.0;
    Ok(Battery { present, percent: pct, charging, full: st.ACLineStatus == 1 && pct >= 100.0, seconds: if st.BatteryLifeTime == u32::MAX { 0 } else { st.BatteryLifeTime as i64 } })
}

pub async fn power_profile(want: Option<&str>) -> Result<(String, Vec<String>)> {
    use windows::Win32::System::Power::PowerSetActiveScheme;
    let Some(w) = want else { return Err(not_here("power modes")) };
    let guid = match w {
        "power-saver" => windows::core::GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a),
        "performance" => windows::core::GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c),
        _ => windows::core::GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e),
    };
    let span = crate::layers::effect(crate::layers::Layer::System, "system.power");
    span.end(unsafe { PowerSetActiveScheme(None, Some(&guid)) }.ok().map_err(|_| anyhow!("This computer has no {} plan.", w.replace('-', " "))))?;
    Ok((w.to_owned(), Vec::new()))
}

pub async fn vpn(on: bool, want: &str) -> Result<String> {
    let span = crate::layers::effect(crate::layers::Layer::Command, "command.vpn");
    let mut c = std::process::Command::new("rasdial");
    if on {
        if want.is_empty() {
            return span.end(Err(anyhow!("Which VPN? Say connect to the VPN, then its name.")));
        }
        c.arg(want);
    } else {
        c.arg("/disconnect");
    }
    let ok = c.output().map(|o| o.status.success()).unwrap_or(false);
    span.end_bool(ok);
    if ok { Ok(want.to_owned()) } else { Err(anyhow!("The VPN didn't {}.", if on { "connect" } else { "disconnect" })) }
}

fn hotspot_now(on: bool) -> Result<(String, Option<String>)> {
    use windows::Networking::Connectivity::NetworkInformation;
    use windows::Networking::NetworkOperators::{NetworkOperatorTetheringManager, TetheringOperationStatus, TetheringOperationalState};
    let profile = NetworkInformation::GetInternetConnectionProfile().map_err(|_| anyhow!("The hotspot needs an internet connection to share."))?;
    let m = NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile).map_err(|_| anyhow!("This PC can't share its connection as a hotspot."))?;
    let span = crate::layers::effect(crate::layers::Layer::System, "system.hotspot");
    let state = m.TetheringOperationalState().unwrap_or(TetheringOperationalState::Unknown);
    let settled = state == TetheringOperationalState::On || state == TetheringOperationalState::Off;
    if !(settled && (state == TetheringOperationalState::On) == on) {
        let verb = if on { "start" } else { "stop" };
        let op = if on { m.StartTetheringAsync() } else { m.StopTetheringAsync() };
        let r = op.and_then(|o| o.get()).map_err(|e| anyhow!("Windows didn't {verb} the hotspot: {}", e.message()))?;
        if r.Status().unwrap_or(TetheringOperationStatus(-1)) != TetheringOperationStatus::Success {
            let more = r.AdditionalErrorMessage().map(|h| h.to_string()).unwrap_or_default();
            span.end_bool(false);
            return Err(anyhow!("Windows didn't {verb} the hotspot{}.", if more.is_empty() { String::new() } else { format!(": {more}") }));
        }
    }
    span.ok();
    let cfg = m.GetCurrentAccessPointConfiguration().ok();
    let ssid = cfg.as_ref().and_then(|c| c.Ssid().ok()).map(|h| h.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "on this PC".into());
    let pass = cfg.and_then(|c| c.Passphrase().ok()).map(|h| h.to_string()).filter(|p| !p.is_empty());
    Ok((ssid, if on { pass } else { None }))
}

pub async fn hotspot(on: bool) -> Result<(String, Option<String>)> {
    crate::layers::blocking(move || hotspot_now(on)).await.map_err(|e| anyhow!(e.to_string()))?
}

fn current_mode() -> Option<(u32, u32)> {
    use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut dm) }.as_bool().then_some((dm.dmPelsWidth, dm.dmPelsHeight))
}

pub async fn current_scale() -> Result<f64> {
    Ok(unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() } as f64 / 96.0)
}

pub async fn display_info() -> Result<String> {
    let (w, h) = current_mode().ok_or_else(|| anyhow!("The screen can't be read."))?;
    Ok(format!("The screen is at {w} by {h}, scale {}%", (current_scale().await.unwrap_or(1.0) * 100.0).round()))
}

pub async fn display(res: Option<(i32, i32)>, step: i32, scale: Option<f64>) -> Result<String> {
    use windows::Win32::Graphics::Gdi::{CDS_TEST, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW};
    if scale.is_some() {
        return Err(not_here("the display scale"));
    }
    let (cw, ch) = current_mode().ok_or_else(|| anyhow!("The screen can't be read."))?;
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    let mut i = 0u32;
    loop {
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        if !unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) }.as_bool() {
            break;
        }
        sizes.push((dm.dmPelsWidth, dm.dmPelsHeight));
        i += 1;
    }
    sizes.sort_by_key(|(w, h)| w * h);
    sizes.dedup();
    let (w, h) = match res {
        Some((w, h)) if sizes.contains(&(w as u32, h as u32)) => (w as u32, h as u32),
        Some((w, h)) => return Err(anyhow!("This screen can't do {w} by {h}.")),
        None => {
            let at = sizes.iter().position(|s| *s == (cw, ch)).unwrap_or(0) as i32;
            sizes[(at + step).clamp(0, sizes.len() as i32 - 1) as usize]
        }
    };
    let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, dmPelsWidth: w, dmPelsHeight: h, dmFields: DM_PELSWIDTH | DM_PELSHEIGHT, ..Default::default() };
    let span = crate::layers::effect(crate::layers::Layer::System, "system.display");
    unsafe {
        if ChangeDisplaySettingsExW(PCWSTR::null(), Some(&dm), None, CDS_TEST, None) != DISP_CHANGE_SUCCESSFUL {
            return span.end(Err(anyhow!("The screen can't do {w} by {h}.")));
        }
        let r = ChangeDisplaySettingsExW(PCWSTR::null(), Some(&mut dm), None, CDS_TYPE(CDS_UPDATEREGISTRY.0), None);
        span.end(if r == DISP_CHANGE_SUCCESSFUL { Ok(format!("{w} by {h}")) } else { Err(anyhow!("The screen didn't accept {w} by {h}.")) })
    }
}

fn removable_drives() -> Vec<(char, String)> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
        if kind != 2 && kind != 5 {
            continue;
        }
        let mut label = [0u16; 261];
        let named = unsafe { GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut label), None, None, None, None) }.is_ok();
        let n = label.iter().position(|c| *c == 0).unwrap_or(0);
        let name = if named && n > 0 { String::from_utf16_lossy(&label[..n]) } else { format!("{letter} drive") };
        out.push((letter, name));
    }
    out
}

fn eject_now(want: &str) -> Result<String> {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows::Win32::System::IO::DeviceIoControl;
    use windows::Win32::System::Ioctl::{FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_STORAGE_EJECT_MEDIA, IOCTL_STORAGE_MEDIA_REMOVAL};
    let drives = removable_drives();
    let w = crate::text::normalize(want);
    let generic = matches!(w.as_str(), "" | "usb" | "the usb" | "usb drive" | "the usb drive" | "drive" | "the drive" | "usb stick" | "pen drive" | "disk" | "it" | "flash drive" | "the flash drive");
    let letter = crate::tools::files::letter(&w);
    let pick = drives
        .iter()
        .find(|d| letter.is_some_and(|l| l.eq_ignore_ascii_case(&d.0)))
        .or_else(|| drives.iter().find(|d| !w.is_empty() && { let n = crate::text::normalize(&d.1); n == w || n.contains(&w) || w.contains(&n) && n.len() > 2 }))
        .or_else(|| if generic && drives.len() == 1 { drives.first() } else { None });
    let Some((l, name)) = pick.cloned() else {
        return Err(if drives.is_empty() { anyhow!("No removable drive is plugged in.") } else { anyhow!("Which drive? {}.", drives.iter().map(|d| format!("{} ({}:)", d.1, d.0)).collect::<Vec<_>>().join(", ")) });
    };
    let path = wide(&format!("\\\\.\\{l}:"));
    let span = crate::layers::effect(crate::layers::Layer::System, "system.drives");
    let r = (|| -> Result<()> {
        let h = unsafe { CreateFileW(PCWSTR(path.as_ptr()), GENERIC_READ.0 | GENERIC_WRITE.0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.map_err(|_| anyhow!("{name} can't be opened to eject it."))?;
        let io = |code: u32, input: Option<&[u8]>| unsafe { DeviceIoControl(h, code, input.map(|b| b.as_ptr() as *const core::ffi::c_void), input.map(|b| b.len() as u32).unwrap_or(0), None, 0, Some(&mut 0u32), None) }.is_ok();
        let mut locked = false;
        for _ in 0..20 {
            if io(FSCTL_LOCK_VOLUME, None) {
                locked = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let done = locked && io(FSCTL_DISMOUNT_VOLUME, None) && io(IOCTL_STORAGE_MEDIA_REMOVAL, Some(&[0u8])) && io(IOCTL_STORAGE_EJECT_MEDIA, None);
        unsafe {
            let _ = CloseHandle(h);
        }
        if !locked {
            return Err(anyhow!("{name} is still in use. Close the files or windows that use it, then say eject again."));
        }
        if !done {
            return Err(anyhow!("{name} couldn't be ejected."));
        }
        Ok(())
    })();
    span.end(r).map(|_| name)
}

pub async fn eject(want: &str) -> Result<String> {
    let w = want.to_owned();
    crate::layers::blocking(move || eject_now(&w)).await.map_err(|e| anyhow!(e.to_string()))?
}

pub async fn updates() -> Result<Vec<String>> {
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::UpdateAgent::{IUpdateSession, UpdateSession};
    let span = crate::layers::read(crate::layers::Layer::System, "system.updates");
    let r = crate::layers::blocking(|| -> Result<Vec<String>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let session: IUpdateSession = CoCreateInstance(&UpdateSession, None, CLSCTX_ALL).map_err(|e| anyhow!("{e}"))?;
            let searcher = session.CreateUpdateSearcher().map_err(|e| anyhow!("{e}"))?;
            let result = searcher.Search(&windows::core::BSTR::from("IsInstalled=0 and IsHidden=0")).map_err(|e| anyhow!("{e}"))?;
            let list = result.Updates().map_err(|e| anyhow!("{e}"))?;
            let n = list.Count().unwrap_or(0);
            let mut out = Vec::new();
            for i in 0..n {
                if let Ok(u) = list.get_Item(i)
                    && let Ok(t) = u.Title()
                {
                    out.push(t.to_string());
                }
            }
            Ok(out)
        }
    })
    .await
    .map_err(|e| anyhow!("{e}"))
    .and_then(|r| r);
    span.end(r)
}

#[derive(Debug, Clone)]
pub struct Note {
    pub app: String,
    pub title: String,
    pub body: String,
}

pub fn notes() -> Vec<Note> {
    Vec::new()
}

pub fn clear_notes() {}

pub fn watch_notifications() {}

pub async fn clipboard() -> Result<String> {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    let span = crate::layers::read(crate::layers::Layer::System, "system.clipboard");
    let r = crate::layers::blocking(|| -> Result<String> {
        unsafe {
            OpenClipboard(Some(HWND::default())).map_err(|e| anyhow!("{e}"))?;
            let out = (|| {
                let h = GetClipboardData(13).map_err(|_| anyhow!("The clipboard holds no text."))?;
                let g = windows::Win32::Foundation::HGLOBAL(h.0);
                let p = GlobalLock(g) as *const u16;
                if p.is_null() {
                    return Err(anyhow!("The clipboard couldn't be read."));
                }
                let mut n = 0;
                while *p.add(n) != 0 && n < 32768 {
                    n += 1;
                }
                let t = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                let _ = GlobalUnlock(g);
                Ok(t)
            })();
            let _ = CloseClipboard();
            out
        }
    })
    .await
    .map_err(|e| anyhow!("{e}"))
    .and_then(|r| r);
    span.end(r)
}

pub async fn set_default(_role: &str, app: &crate::tools::apps::App) -> Result<String> {
    let _ = crate::tools::shell::open("ms-settings:defaultapps");
    Err(anyhow!("Windows asks you to choose {} as the default yourself. I opened Default apps.", app.name))
}

pub fn printers() -> Vec<String> {
    use windows::Win32::Graphics::Printing::{EnumPrintersW, PRINTER_ENUM_CONNECTIONS, PRINTER_ENUM_LOCAL, PRINTER_INFO_4W};
    let mut need = 0u32;
    let mut count = 0u32;
    unsafe {
        let _ = EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS, PCWSTR::null(), 4, None, &mut need, &mut count);
        if need == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; need as usize];
        if EnumPrintersW(PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS, PCWSTR::null(), 4, Some(&mut buf), &mut need, &mut count).is_err() {
            return Vec::new();
        }
        let items = std::slice::from_raw_parts(buf.as_ptr() as *const PRINTER_INFO_4W, count as usize);
        items.iter().filter_map(|p| p.pPrinterName.to_string().ok()).filter(|n| !n.contains("PDF") && !n.contains("XPS") && !n.contains("OneNote") && n != "Fax").collect()
    }
}

pub fn open_printer_settings() -> Result<()> {
    crate::tools::shell::open("ms-settings:printers")
}

pub struct Startup {
    pub name: String,
    pub on: bool,
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

fn run_values() -> Vec<String> {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegEnumValueW, RegOpenKeyExW};
    let p = wide(RUN);
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), Some(0), KEY_READ, &mut key).is_err() {
            return out;
        }
        for i in 0..256u32 {
            let mut name = vec![0u16; 260];
            let mut len = name.len() as u32;
            if RegEnumValueW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        let _ = RegCloseKey(key);
    }
    out
}

fn approved(name: &str) -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW};
    let p = wide(APPROVED);
    let n = wide(name);
    let mut buf = [0u8; 12];
    let mut len = buf.len() as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    r.is_err() || buf[0] % 2 == 0
}

pub fn startup_list() -> Vec<Startup> {
    run_values().into_iter().map(|n| Startup { on: approved(&n), name: n }).collect()
}

pub fn set_startup(name: &str, app: Option<&crate::tools::apps::App>, on: bool) -> Result<String> {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW};
    let want = crate::text::normalize(name);
    let existing = run_values().into_iter().find(|n| crate::text::normalize(n) == want || crate::text::normalize(n).contains(&want));
    let span = crate::layers::effect(crate::layers::Layer::System, "system.startup");
    unsafe {
        if let Some(n) = existing {
            let p = wide(APPROVED);
            let mut key = HKEY::default();
            RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), Some(0), PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None).ok().map_err(|e| anyhow!("{e}"))?;
            let mut v = [0u8; 12];
            v[0] = if on { 2 } else { 3 };
            let wn = wide(&n);
            let r = RegSetValueExW(key, PCWSTR(wn.as_ptr()), Some(0), REG_BINARY, Some(&v));
            let _ = RegCloseKey(key);
            span.end(r.ok().map_err(|e| anyhow!("{e}")))?;
            return Ok(n);
        }
        if !on {
            return span.end(Err(anyhow!("{} doesn't open at startup.", crate::text::sentence_case(name))));
        }
        let app = app.ok_or_else(|| anyhow!("I couldn't find an app called {name}."))?;
        let p = wide(RUN);
        let mut key = HKEY::default();
        RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), Some(0), PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut key, None).ok().map_err(|e| anyhow!("{e}"))?;
        let cmd: Vec<u8> = wide(&format!("\"{}\"", app.path)).iter().flat_map(|c| c.to_le_bytes()).collect();
        let wn = wide(&app.name);
        let r = RegSetValueExW(key, PCWSTR(wn.as_ptr()), Some(0), REG_SZ, Some(&cmd));
        let _ = RegCloseKey(key);
        span.end(r.ok().map_err(|e| anyhow!("{e}")))?;
        Ok(app.name.clone())
    }
}

pub fn pids_named(bins: &[String]) -> Vec<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            let exe = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            let stem = exe.trim_end_matches(".exe").to_owned();
            if bins.iter().any(|b| !b.is_empty() && (*b == exe || *b == stem || b.trim_end_matches(".exe") == stem)) {
                out.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn kill(pids: &[u32]) -> Result<usize> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    let span = crate::layers::effect(crate::layers::Layer::System, "system.process");
    let mut n = 0;
    for p in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, *p) {
                if TerminateProcess(h, 1).is_ok() {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    span.end_bool(n > 0);
    if n == 0 { Err(anyhow!("It couldn't be stopped.")) } else { Ok(n) }
}

pub fn addresses() -> Vec<(String, String)> {
    Vec::new()
}

pub fn trash_count() -> usize {
    use windows::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    if unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.is_ok() { info.i64NumItems.max(0) as usize } else { 0 }
}

pub async fn empty_trash() -> Result<usize> {
    use windows::Win32::UI::Shell::{SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW};
    let n = trash_count();
    let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
    span.end(unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) }.map_err(|e| anyhow!("{e}")))?;
    Ok(n)
}

pub async fn outputs_fallback() -> Outcome {
    let _ = crate::tools::shell::open("ms-settings:sound");
    Outcome::fail("Windows doesn't let apps choose the sound output. I opened Sound settings.", "system")
}
