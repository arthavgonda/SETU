use crate::core::Core;
use serde_json::{Value, json};
use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::sync::Arc;

const HOST: &str = "ai.setu.browser";
const HELPER: &str = "setu.setu-vscode";
const DONE: i64 = 1;

#[cfg_attr(windows, allow(dead_code))]
fn ours(text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return false };
    v.get("name").and_then(Value::as_str) == Some(HOST) && v.get("path").and_then(Value::as_str).is_some_and(|p| p.to_lowercase().contains("setu"))
}

#[cfg(not(windows))]
fn host_files(home: &Path) -> Vec<PathBuf> {
    let chromium: &[&str] = if cfg!(target_os = "macos") {
        &["Google/Chrome", "Google/Chrome Beta", "Google/Chrome Canary", "Chromium", "BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser-Beta", "BraveSoftware/Brave-Browser-Nightly", "Microsoft Edge", "Microsoft Edge Beta", "Vivaldi", "Arc/User Data"]
    } else {
        &["google-chrome", "google-chrome-beta", "google-chrome-unstable", "chromium", "BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser-Beta", "BraveSoftware/Brave-Browser-Nightly", "microsoft-edge", "microsoft-edge-beta", "microsoft-edge-dev", "vivaldi", "opera"]
    };
    let base = if cfg!(target_os = "macos") { home.join("Library/Application Support") } else { dirs::config_dir().unwrap_or_else(|| home.join(".config")) };
    let file = format!("{HOST}.json");
    let mut v: Vec<PathBuf> = chromium.iter().map(|n| base.join(n).join("NativeMessagingHosts").join(&file)).collect();
    if cfg!(target_os = "macos") {
        v.push(home.join("Library/Application Support/Mozilla/NativeMessagingHosts").join(&file));
    } else {
        for d in [home.join(".mozilla/native-messaging-hosts"), base.join("mozilla/native-messaging-hosts"), home.join(".librewolf/native-messaging-hosts")] {
            v.push(d.join(&file));
        }
    }
    v
}

#[cfg(not(windows))]
pub fn browser_hosts(_data: &Path) -> usize {
    let Some(home) = dirs::home_dir() else { return 0 };
    host_files(&home).into_iter().filter(|f| std::fs::read_to_string(f).is_ok_and(|t| ours(&t)) && std::fs::remove_file(f).is_ok()).count()
}

#[cfg(windows)]
pub fn browser_hosts(data: &Path) -> usize {
    use crate::proc::Quiet;
    let keys = [
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
        r"HKCU\Software\Chromium\NativeMessagingHosts",
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
        r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        r"HKCU\Software\Vivaldi\NativeMessagingHosts",
        r"HKCU\Software\Opera Software\NativeMessagingHosts",
        r"HKCU\Software\Mozilla\NativeMessagingHosts",
    ];
    let mut n = 0;
    for key in keys {
        let ok = std::process::Command::new("reg")
            .args(["delete", &format!(r"{key}\{HOST}"), "/f"])
            .quiet()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            n += 1;
        }
    }
    let dir = data.join("browser");
    if dir.join(format!("{HOST}.chromium.json")).is_file() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    n
}

pub fn vscode_helper() -> bool {
    if !crate::tools::code::installed() || !crate::tools::code::list_extensions().iter().any(|e| e == HELPER) {
        return false;
    }
    crate::tools::code::uninstall_extension(HELPER).is_ok()
}

pub fn start(core: &Arc<Core>) {
    if std::env::var_os("SETU_NO_BROWSER_HOSTS").is_some() || core.settings.get("migrate.retire").as_i64() == Some(DONE) {
        return;
    }
    let c = core.clone();
    crate::layers::blocking(move || {
        let hosts = browser_hosts(&c.paths.data);
        let helper = vscode_helper();
        log::info!("removed what older versions added: {hosts} browser connection files, {}", if helper { "the VS Code helper" } else { "no VS Code helper" });
        c.settings.set_internal("migrate.retire", json!(DONE));
        let mut save = serde_json::Map::new();
        save.insert("migrate.retire".into(), json!(DONE));
        c.db.save_settings(save);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_setu_files() {
        assert!(ours(r#"{"name":"ai.setu.browser","description":"Setu voice assistant","path":"/usr/bin/setu-daemon","type":"stdio"}"#));
        assert!(!ours(r#"{"name":"ai.setu.browser","path":"/opt/other/host","type":"stdio"}"#));
        assert!(!ours(r#"{"name":"com.other.host","path":"/usr/bin/setu-daemon"}"#));
        assert!(!ours("not json"));
        #[cfg(not(windows))]
        {
            let d = tempfile::tempdir().unwrap();
            let files = host_files(d.path());
            assert!(files.iter().all(|f| f.file_name().is_some_and(|n| n == "ai.setu.browser.json")));
            assert!(files.iter().any(|f| f.to_string_lossy().contains("BraveSoftware/Brave-Browser/NativeMessagingHosts")));
        }
    }
}
