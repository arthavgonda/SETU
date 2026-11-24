use crate::core::Core;
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
#[cfg(windows)]
use crate::proc::Quiet;

fn item(id: &str, name: &str, allows: &str, never: &str, status: &str, required: bool, fix: Option<&str>) -> Value {
    json!({ "id": id, "name": name, "allows": allows, "never": never, "status": status, "required": required, "fix": fix })
}

fn camera_status() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        let found = std::fs::read_dir("/dev").ok().map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("video"))).unwrap_or(false);
        if !found {
            return "unsupported";
        }
        let ok = std::fs::read_dir("/dev")
            .ok()
            .map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("video")).any(|e| std::fs::File::open(e.path()).is_ok()))
            .unwrap_or(false);
        if ok { "granted" } else { "not_granted" }
    }
    #[cfg(not(target_os = "linux"))]
    {
        "ask_each_time"
    }
}

pub async fn status(core: &Core) -> Value {
    let s = &core.settings;
    let mic = if core.audio.open.load(Ordering::Relaxed) { "granted" } else if s.bool("setup.completed") { "not_granted" } else { "ask_each_time" };
    let a11y = crate::tools::atspi::enabled().await;
    let control_on = s.bool("permissions.control");
    let basic = vec![
        item("microphone", "Microphone", "Hear the wake phrase and your requests.", "Record or store audio. Listen before the wake phrase.", mic, true, if mic == "granted" { None } else { Some("open_settings") }),
        item("launch_apps", "Open and close apps", "Start and quit apps you name.", "Open apps you didn't ask for.", "granted", false, None),
        item("system_settings", "Volume and brightness", "Change volume, brightness and Wi-Fi when you ask.", "Change other system settings.", "granted", false, None),
    ];
    let control = vec![
        item(
            "accessibility",
            "Accessibility",
            "Read buttons and fields in the active window, click them and type for you.",
            "Read windows you don't ask about. Type passwords.",
            if a11y && control_on { "granted" } else { "not_granted" },
            false,
            Some(if a11y && control_on { "revoke" } else { "allow" }),
        ),
        item(
            "screen",
            "Screen reading",
            "Read the text on screen when you ask about it.",
            "Record the screen or keep screenshots.",
            if s.bool("privacy.screenReading") { "ask_each_time" } else { "not_granted" },
            false,
            Some(if s.bool("privacy.screenReading") { "revoke" } else { "allow" }),
        ),
        item("camera", "Camera", "Read a QR code when you say scan the QR code.", "Turn the camera on without telling you, or keep pictures.", camera_status(), false, Some("open_settings")),
    ];
    let admin = vec![item("installs", "Installs and system changes", "Install apps after you confirm each one.", "Install anything without asking.", "ask_each_time", false, None)];
    json!({
        "tiers": [
            { "id": "basic", "enabled": true, "items": basic },
            { "id": "control", "enabled": control_on, "items": control },
            { "id": "admin", "enabled": s.bool("permissions.admin"), "items": admin },
        ],
        "automation": [],
    })
}

pub fn mic_blocked(error: &str) -> bool {
    #[cfg(windows)]
    {
        let e = error.to_ascii_lowercase();
        e.contains("0x80070005") || e.contains("access is denied") || windows_mic_denied()
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

#[cfg(windows)]
fn windows_mic_denied() -> bool {
    use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::PCWSTR;
    const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn denies(root: HKEY, path: &str) -> bool {
        let (p, n) = (wide(path), wide("Value"));
        let mut buf = [0u16; 16];
        let mut len = std::mem::size_of_val(&buf) as u32;
        let r = unsafe { RegGetValueW(root, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
        if r.is_err() {
            return false;
        }
        let chars = (len as usize / 2).min(buf.len());
        String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').eq_ignore_ascii_case("Deny")
    }
    let policy = {
        let (p, n) = (wide(r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy"), wide("LetAppsAccessMicrophone"));
        let mut v = 0u32;
        let mut len = 4u32;
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut len)) };
        r.is_ok() && v == 2
    };
    policy || denies(HKEY_LOCAL_MACHINE, STORE) || denies(HKEY_CURRENT_USER, STORE) || denies(HKEY_CURRENT_USER, &format!(r"{STORE}\NonPackaged"))
}

pub fn open_settings(item: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
        let (cmd, args): (&str, Vec<&str>) = if desktop.contains("kde") {
            ("systemsettings", vec![if item == "microphone" { "kcm_pulseaudio" } else { "kcm_access" }])
        } else {
            ("gnome-control-center", vec![match item {
                "microphone" => "sound",
                "camera" => "privacy",
                _ => "universal-access",
            }])
        };
        std::process::Command::new(cmd).args(args).spawn().is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        let pane = match item {
            "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
            "camera" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera",
            "screen" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            "automation" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
            _ => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        };
        std::process::Command::new("open").arg(pane).spawn().is_ok()
    }
    #[cfg(windows)]
    {
        let uri = match item {
            "microphone" => "ms-settings:privacy-microphone",
            "camera" => "ms-settings:privacy-webcam",
            _ => "ms-settings:easeofaccess",
        };
        std::process::Command::new("cmd").quiet().args(["/C", "start", "", uri]).spawn().is_ok()
    }
}
