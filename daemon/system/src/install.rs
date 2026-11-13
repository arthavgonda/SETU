#[cfg(unix)]
use serde_json::json;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
pub const CONFIG: &str = "etc/setu/system.json";

#[cfg(target_os = "linux")]
pub const SOCKET: &str = "/run/setu-system/sock";
#[cfg(target_os = "macos")]
pub const SOCKET: &str = "/var/run/setu-system.sock";
#[cfg(windows)]
pub const SOCKET: &str = r"\\.\pipe\setu-system";

#[cfg(target_os = "linux")]
const UNIT: &str = "etc/systemd/system/setu-system.service";
#[cfg(target_os = "macos")]
const PLIST: &str = "Library/LaunchDaemons/ai.setu.system.plist";

#[cfg(unix)]
pub fn read_config(root: &Path) -> Option<(u32, Option<PathBuf>)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(CONFIG)).ok()?).ok()?;
    let uid = v.get("uid")?.as_u64()? as u32;
    let daemon = v.get("daemon").and_then(|d| d.as_str()).filter(|d| !d.is_empty()).map(PathBuf::from);
    Some((uid, daemon))
}

#[cfg(unix)]
fn write(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(())
}

#[cfg(unix)]
fn sh(argv: &[&str]) -> Result<(), String> {
    let out = Command::new(argv[0]).args(&argv[1..]).output().map_err(|e| format!("{}: {e}", argv[0]))?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
}

#[cfg(unix)]
fn packaged(exe: &Path) -> bool {
    ["/usr/libexec/setu/", "/usr/lib/setu/", "/opt/setu/"].iter().any(|p| exe.starts_with(p))
}

#[cfg(unix)]
pub fn binary_target(root: &Path, exe: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        return root.join("Library/Application Support/Setu/setu-system");
    }
    if packaged(exe) && root == Path::new("/") { exe.to_path_buf() } else { root.join("usr/local/libexec/setu/setu-system") }
}

#[cfg(target_os = "linux")]
pub fn unit_text(bin: &Path) -> String {
    format!(
        "[Unit]\nDescription=Setu system helper\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={} --serve\nRestart=on-failure\nRuntimeDirectory=setu-system\nRuntimeDirectoryMode=0755\nUMask=0077\nPrivateTmp=yes\nProtectHome=read-only\n\n[Install]\nWantedBy=multi-user.target\n",
        bin.display()
    )
}

#[cfg(target_os = "macos")]
pub fn plist_text(bin: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key><string>ai.setu.system</string>\n<key>ProgramArguments</key><array><string>{}</string><string>--serve</string></array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n</dict>\n</plist>\n",
        bin.display()
    )
}

#[cfg(unix)]
pub fn install(root: &Path, uid: u32, daemon: &Path) -> Result<String, String> {
    let real = root == Path::new("/");
    if real && unsafe { libc::geteuid() } != 0 {
        return Err("Installing the system helper needs administrator rights.".into());
    }
    if !daemon.is_absolute() || !daemon.exists() {
        return Err(format!("{} isn't Setu's daemon.", daemon.display()));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.canonicalize().map_err(|e| e.to_string())?;
    let bin = binary_target(root, &exe);
    if bin != exe {
        if let Some(d) = bin.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = bin.with_extension("new");
        std::fs::copy(&exe, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &bin).map_err(|e| e.to_string())?;
    }
    write(&root.join(CONFIG), &json!({ "uid": uid, "daemon": daemon }).to_string(), 0o644)?;
    let shown = if real { bin.clone() } else { Path::new("/").join(bin.strip_prefix(root).unwrap_or(&bin)) };
    #[cfg(target_os = "linux")]
    {
        write(&root.join(UNIT), &unit_text(&shown), 0o644)?;
        if real {
            sh(&["/usr/bin/systemctl", "daemon-reload"])?;
            sh(&["/usr/bin/systemctl", "enable", "--now", "setu-system.service"])?;
            sh(&["/usr/bin/systemctl", "restart", "setu-system.service"])?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let plist = root.join(PLIST);
        write(&plist, &plist_text(&shown), 0o644)?;
        if real {
            let _ = sh(&["/bin/launchctl", "bootout", "system/ai.setu.system"]);
            sh(&["/bin/launchctl", "bootstrap", "system", &plist.to_string_lossy()])?;
        }
    }
    Ok(format!("installed {}", shown.display()))
}

#[cfg(unix)]
pub fn uninstall(root: &Path) -> Result<String, String> {
    let real = root == Path::new("/");
    if real && unsafe { libc::geteuid() } != 0 {
        return Err("Removing the system helper needs administrator rights.".into());
    }
    #[cfg(target_os = "linux")]
    {
        if real {
            let _ = sh(&["/usr/bin/systemctl", "disable", "--now", "setu-system.service"]);
        }
        let _ = std::fs::remove_file(root.join(UNIT));
        if real {
            let _ = sh(&["/usr/bin/systemctl", "daemon-reload"]);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if real {
            let _ = sh(&["/bin/launchctl", "bootout", "system/ai.setu.system"]);
        }
        let _ = std::fs::remove_file(root.join(PLIST));
    }
    let _ = std::fs::remove_file(root.join(CONFIG));
    let local = if cfg!(target_os = "macos") { root.join("Library/Application Support/Setu/setu-system") } else { root.join("usr/local/libexec/setu/setu-system") };
    let _ = std::fs::remove_file(local);
    Ok("removed".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn install_into_a_scratch_root_writes_a_root_only_service() {
        let t = tempfile::tempdir().unwrap();
        let daemon = std::env::current_exe().unwrap();
        install(t.path(), 1000, &daemon).unwrap();
        let unit = std::fs::read_to_string(t.path().join(UNIT)).unwrap();
        assert!(unit.contains("ExecStart=/usr/local/libexec/setu/setu-system --serve"));
        assert!(t.path().join("usr/local/libexec/setu/setu-system").exists());
        assert_eq!(read_config(t.path()), Some((1000, Some(daemon))));
        uninstall(t.path()).unwrap();
        assert!(!t.path().join(UNIT).exists() && !t.path().join(CONFIG).exists());
    }
}
