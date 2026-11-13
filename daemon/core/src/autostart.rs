use crate::paths::Paths;
use anyhow::Result;
#[cfg(windows)]
use crate::proc::Quiet;

#[cfg(target_os = "linux")]
pub fn set(paths: &Paths, on: bool) -> Result<()> {
    use std::process::{Command, Stdio};
    let home = dirs::home_dir().unwrap_or_default();
    let config = dirs::config_dir().unwrap_or(home.join(".config"));
    let systemd = Command::new("systemctl").args(["--user", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    let shell = paths.bin.join("setu-shell");
    let daemon = paths.bin.join("setu-daemon");
    let appimage = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).filter(|p| p.is_file());
    if let Some(image) = appimage {
        let dir = config.join("autostart");
        let file = dir.join("setu-shell.desktop");
        if on {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=Setu\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n", image.display()))?;
        } else {
            let _ = std::fs::remove_file(&file);
        }
        return Ok(());
    }
    if systemd {
        let unit_dir = config.join("systemd/user");
        let packaged = ["/usr/lib/systemd/user/setu-daemon.service", "/usr/local/lib/systemd/user/setu-daemon.service"].iter().any(|p| std::path::Path::new(p).exists());
        if on && !packaged {
            std::fs::create_dir_all(&unit_dir)?;
            std::fs::write(
                unit_dir.join("setu-daemon.service"),
                format!(
                    "[Unit]\nDescription=Setu voice assistant service\nAfter=pipewire.service pipewire-pulse.service\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=1\nKillMode=process\n\n[Install]\nWantedBy=default.target\n",
                    daemon.display()
                ),
            )?;
            std::fs::write(
                unit_dir.join("setu-shell.service"),
                format!(
                    "[Unit]\nDescription=Setu tray and listening overlay\nAfter=graphical-session.target setu-daemon.service\nPartOf=graphical-session.target\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=graphical-session.target\n",
                    shell.display()
                ),
            )?;
            let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        }
        let verb = if on { "enable" } else { "disable" };
        let ok = Command::new("systemctl").args(["--user", verb, "setu-daemon.service", "setu-shell.service"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
        if ok {
            return Ok(());
        }
    }
    let dir = config.join("autostart");
    let file = dir.join("setu-shell.desktop");
    if on {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&file, format!("[Desktop Entry]\nType=Application\nName=Setu\nExec={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n", shell.display()))?;
    } else {
        let _ = std::fs::remove_file(&file);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn set(paths: &Paths, on: bool) -> Result<()> {
    let home = dirs::home_dir().unwrap_or_default();
    let plist = home.join("Library/LaunchAgents/ai.setu.daemon.plist");
    if on {
        std::fs::create_dir_all(plist.parent().unwrap())?;
        std::fs::write(
            &plist,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>ai.setu.daemon</string><key>ProgramArguments</key><array><string>{}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><true/></dict></plist>\n",
                paths.bin.join("setu-daemon").display()
            ),
        )?;
    } else {
        let _ = std::fs::remove_file(&plist);
    }
    Ok(())
}

#[cfg(windows)]
pub fn set(paths: &Paths, on: bool) -> Result<()> {
    let exe = paths.bin.join("setu-shell.exe");
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let status = if on {
        std::process::Command::new("reg").quiet().args(["add", key, "/v", "Setu", "/t", "REG_SZ", "/d", &format!("\"{}\"", exe.display()), "/f"]).status()?
    } else {
        std::process::Command::new("reg").quiet().args(["delete", key, "/v", "Setu", "/f"]).status()?
    };
    let _ = status;
    Ok(())
}
