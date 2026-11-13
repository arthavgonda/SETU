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
