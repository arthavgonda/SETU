use serde_json::Value;
use std::path::{Path, PathBuf};

pub const ACTIONS: &[&str] = &[
    "ping",
    "service.start",
    "service.stop",
    "service.restart",
    "service.enable",
    "service.disable",
    "package.install",
    "package.remove",
    "package.install_file",
    "updates.apply",
    "firewall.port.open",
    "firewall.port.close",
    "firewall.state",
    "hostname.set",
    "time.ntp",
    "user.add",
    "user.remove",
    "backlight.set",
    "firefox.autoconfig.install",
    "firefox.autoconfig.remove",
    "secure.keys",
    "secure.click",
];

const PROTECTED: &[&str] = &[
    "dbus.service",
    "dbus-broker.service",
    "systemd-journald.service",
    "systemd-logind.service",
    "systemd-udevd.service",
    "polkit.service",
    "gdm.service",
    "sddm.service",
    "lightdm.service",
    "display-manager.service",
    "setu-system.service",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Run(Vec<String>),
    Write { path: PathBuf, text: String, mode: u32 },
    Remove(PathBuf),
    Stage { from: PathBuf, owner: u32, to: PathBuf },
    Secure(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pm {
    Dnf(String),
    Apt,
    Pacman,
    Zypper,
}

#[derive(Debug, Clone)]
pub struct Host {
    pub pm: Option<Pm>,
    pub firewalld: bool,
    pub backlights: Vec<(String, u64)>,
    pub stage_dir: PathBuf,
}

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

impl Host {
    pub fn detect() -> Self {
        let pm = if exists("/usr/bin/dnf5") {
            Some(Pm::Dnf("/usr/bin/dnf5".into()))
        } else if exists("/usr/bin/dnf") {
            Some(Pm::Dnf("/usr/bin/dnf".into()))
        } else if exists("/usr/bin/apt-get") {
            Some(Pm::Apt)
        } else if exists("/usr/bin/pacman") {
            Some(Pm::Pacman)
        } else if exists("/usr/bin/zypper") {
            Some(Pm::Zypper)
        } else {
            None
        };
        let mut backlights = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/sys/class/backlight") {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let max = std::fs::read_to_string(e.path().join("max_brightness")).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
                backlights.push((name, max));
            }
        }
        Host { pm, firewalld: exists("/usr/bin/firewall-cmd"), backlights, stage_dir: PathBuf::from(if cfg!(windows) { r"C:\ProgramData\Setu\stage" } else { "/var/cache/setu-system" }) }
    }
}

fn arg<'a>(args: &'a Value, k: &str) -> Result<&'a str, String> {
    args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| format!("{k} is missing"))
}

fn run(v: &[&str]) -> Step {
    Step::Run(v.iter().map(|s| (*s).to_owned()).collect())
}

fn safe_chars(s: &str, extra: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

pub fn unit_name(raw: &str) -> Result<String, String> {
    let u = raw.trim();
    let u = if [".service", ".socket", ".timer", ".target", ".mount", ".path"].iter().any(|s| u.ends_with(s)) { u.to_owned() } else { format!("{u}.service") };
    let first = u.chars().next().unwrap_or('-');
    if u.len() > 128 || !first.is_ascii_alphanumeric() || !safe_chars(&u, "@_.:-\\") {
        return Err("That isn't a service name I can use.".into());
    }
    Ok(u)
}

pub fn package_name(raw: &str) -> Result<String, String> {
    let p = raw.trim();
    let first = p.chars().next().unwrap_or('-');
    if p.is_empty() || p.len() > 128 || !first.is_ascii_alphanumeric() || !safe_chars(p, "+._:-") {
        return Err("That isn't a package name I can use.".into());
    }
    Ok(p.to_owned())
}

pub fn user_name(raw: &str) -> Result<String, String> {
    let u = raw.trim().to_lowercase();
    let first = u.chars().next().unwrap_or('-');
    if u.is_empty() || u.len() > 32 || !first.is_ascii_lowercase() || !safe_chars(&u, "_-") {
        return Err("A user name can use letters, numbers, dashes and underscores, starting with a letter.".into());
    }
    Ok(u)
}

#[cfg(unix)]
fn account(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let p = unsafe { libc::getpwnam(c.as_ptr()) };
    if p.is_null() { None } else { Some(unsafe { (*p).pw_uid }) }
}

#[cfg(not(unix))]
fn account(_name: &str) -> Option<u32> {
    None
}

pub fn host_name(raw: &str) -> Result<String, String> {
    let h = raw.trim();
    let ok = !h.is_empty() && h.len() <= 63 && safe_chars(h, "-") && !h.starts_with('-') && !h.ends_with('-');
    if ok { Ok(h.to_owned()) } else { Err("A computer name can use letters, numbers and dashes, up to 63 of them.".into()) }
}

fn port_args(args: &Value) -> Result<(u16, &'static str), String> {
    let port = args.get("port").and_then(Value::as_u64).filter(|p| (1..=65535).contains(p)).ok_or("The port must be a number from 1 to 65535.")? as u16;
    let proto = match args.get("proto").and_then(Value::as_str).unwrap_or("tcp") {
        "tcp" => "tcp",
        "udp" => "udp",
        _ => return Err("The protocol must be tcp or udp.".into()),
    };
    Ok((port, proto))
}

pub fn firefox_dir(raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw);
    if !p.is_absolute() {
        return Err("The Firefox folder must be a full path.".into());
    }
    let p = p.canonicalize().map_err(|_| "That Firefox folder doesn't exist.".to_owned())?;
    let ini = std::fs::read_to_string(p.join("application.ini")).map_err(|_| "That folder isn't a Firefox install.".to_owned())?;
    let firefox = ini.lines().any(|l| l.trim() == "Vendor=Mozilla") && ini.lines().any(|l| l.trim() == "Name=Firefox");
    if !firefox {
        return Err("That folder isn't a Firefox install.".into());
    }
    Ok(p)
}

fn owned_by_root(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(m) = std::fs::symlink_metadata(p) else { return false };
        m.uid() == 0 && m.mode() & 0o022 == 0
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        true
    }
}

pub fn plan(action: &str, args: &Value, host: &Host, strict: bool) -> Result<Vec<Step>, String> {
    match action {
        "ping" => Ok(Vec::new()),
        "service.start" | "service.stop" | "service.restart" | "service.enable" | "service.disable" => {
            let verb = action.trim_start_matches("service.");
            if cfg!(windows) {
                let name = arg(args, "unit")?;
                if !safe_chars(name, "_.-") || name.len() > 80 {
                    return Err("That isn't a service name I can use.".into());
                }
                return Ok(match verb {
                    "start" => vec![run(&["sc.exe", "start", name])],
                    "stop" => vec![run(&["sc.exe", "stop", name])],
                    "restart" => vec![run(&["sc.exe", "stop", name]), run(&["sc.exe", "start", name])],
                    "enable" => vec![run(&["sc.exe", "config", name, "start=", "auto"])],
                    _ => vec![run(&["sc.exe", "config", name, "start=", "disabled"])],
                });
            }
            if cfg!(target_os = "macos") {
                let label = arg(args, "unit")?;
                if !safe_chars(label, "._-") || label.len() > 120 {
                    return Err("That isn't a service name I can use.".into());
                }
                let target = format!("system/{label}");
                return Ok(match verb {
                    "start" | "restart" => vec![run(&["/bin/launchctl", "kickstart", "-k", &target])],
                    "stop" => vec![run(&["/bin/launchctl", "kill", "TERM", &target])],
                    "enable" => vec![run(&["/bin/launchctl", "enable", &target])],
                    _ => vec![run(&["/bin/launchctl", "disable", &target])],
                });
            }
            let unit = unit_name(arg(args, "unit")?)?;
            if matches!(verb, "stop" | "disable") && PROTECTED.contains(&unit.as_str()) {
                return Err(format!("I won't {verb} {unit}; the system needs it to run."));
            }
            if verb == "enable" || verb == "disable" {
                return Ok(vec![run(&["/usr/bin/systemctl", verb, "--now", "--", &unit])]);
            }
            Ok(vec![run(&["/usr/bin/systemctl", verb, "--", &unit])])
        }
        "package.install" | "package.remove" => {
            let name = package_name(arg(args, "name")?)?;
            let remove = action == "package.remove";
            match host.pm.as_ref().ok_or("No package manager was found.")? {
                Pm::Dnf(bin) => Ok(vec![Step::Run(vec![bin.clone(), "-y".into(), if remove { "remove" } else { "install" }.into(), name])]),
                Pm::Apt => Ok(vec![Step::Run(vec!["/usr/bin/apt-get".into(), "-y".into(), if remove { "remove" } else { "install" }.into(), name])]),
                Pm::Pacman => Ok(vec![Step::Run(vec!["/usr/bin/pacman".into(), "--noconfirm".into(), if remove { "-R" } else { "-S" }.into(), name])]),
                Pm::Zypper => Ok(vec![Step::Run(vec!["/usr/bin/zypper".into(), "-n".into(), if remove { "remove" } else { "install" }.into(), name])]),
            }
        }
        "package.install_file" => {
            let path = PathBuf::from(arg(args, "path")?);
            let owner = args.get("owner").and_then(Value::as_u64).ok_or("owner is missing")? as u32;
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let fits = match host.pm.as_ref() {
                Some(Pm::Dnf(_)) | Some(Pm::Zypper) => ext == "rpm",
                Some(Pm::Apt) => ext == "deb",
                Some(Pm::Pacman) => path.to_string_lossy().ends_with(".pkg.tar.zst"),
                None => false,
            };
            if !path.is_absolute() || !fits {
                return Err("That file isn't a package for this system.".into());
            }
            let name: String = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')).collect();
            let staged = host.stage_dir.join(format!("{}-{name}", std::process::id()));
            let s = staged.to_string_lossy().into_owned();
            let install = match host.pm.as_ref().unwrap() {
                Pm::Dnf(bin) => Step::Run(vec![bin.clone(), "-y".into(), "install".into(), s.clone()]),
                Pm::Apt => Step::Run(vec!["/usr/bin/apt-get".into(), "-y".into(), "install".into(), s.clone()]),
                Pm::Pacman => Step::Run(vec!["/usr/bin/pacman".into(), "--noconfirm".into(), "-U".into(), s.clone()]),
                Pm::Zypper => Step::Run(vec!["/usr/bin/zypper".into(), "-n".into(), "install".into(), s.clone()]),
            };
            Ok(vec![Step::Stage { from: path, owner, to: staged.clone() }, install, Step::Remove(staged)])
        }
        "updates.apply" => {
            if cfg!(target_os = "macos") {
                return Ok(vec![run(&["/usr/sbin/softwareupdate", "--install", "--all"])]);
            }
            if cfg!(windows) {
                return Ok(vec![run(&["UsoClient.exe", "StartInstall"])]);
            }
            match host.pm.as_ref().ok_or("No package manager was found.")? {
                Pm::Dnf(bin) => Ok(vec![Step::Run(vec![bin.clone(), "-y".into(), "upgrade".into(), "--refresh".into()])]),
                Pm::Apt => Ok(vec![run(&["/usr/bin/apt-get", "update"]), run(&["/usr/bin/apt-get", "-y", "upgrade"])]),
                Pm::Pacman => Ok(vec![run(&["/usr/bin/pacman", "--noconfirm", "-Syu"])]),
                Pm::Zypper => Ok(vec![run(&["/usr/bin/zypper", "-n", "update"])]),
            }
        }
        "firewall.port.open" | "firewall.port.close" => {
            let (port, proto) = port_args(args)?;
            let open = action == "firewall.port.open";
            if cfg!(windows) {
                let name = format!("name=Setu {proto} {port}");
                return Ok(if open {
                    vec![run(&["netsh.exe", "advfirewall", "firewall", "add", "rule", &name, "dir=in", "action=allow", &format!("protocol={proto}"), &format!("localport={port}")])]
                } else {
                    vec![run(&["netsh.exe", "advfirewall", "firewall", "delete", "rule", &name])]
                });
            }
            if cfg!(target_os = "macos") {
                return Err("macOS's firewall works per app, not per port.".into());
            }
            if !host.firewalld {
                return Err("This system has no firewalld.".into());
            }
            let spec = format!("--{}-port={port}/{proto}", if open { "add" } else { "remove" });
            Ok(vec![run(&["/usr/bin/firewall-cmd", &spec]), run(&["/usr/bin/firewall-cmd", "--permanent", &spec])])
        }
        "firewall.state" => {
            let on = args.get("on").and_then(Value::as_bool).ok_or("on is missing")?;
            if cfg!(windows) {
                return Ok(vec![run(&["netsh.exe", "advfirewall", "set", "allprofiles", "state", if on { "on" } else { "off" }])]);
            }
            if cfg!(target_os = "macos") {
                return Ok(vec![run(&["/usr/libexec/ApplicationFirewall/socketfilterfw", "--setglobalstate", if on { "on" } else { "off" }])]);
            }
            if !host.firewalld {
                return Err("This system has no firewalld.".into());
            }
            Ok(vec![run(&["/usr/bin/systemctl", if on { "enable" } else { "disable" }, "--now", "--", "firewalld.service"])])
        }
        "hostname.set" => {
            let name = host_name(arg(args, "name")?)?;
            if cfg!(windows) {
                return Ok(vec![run(&["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "Rename-Computer", "-NewName", &name, "-Force"])]);
            }
            if cfg!(target_os = "macos") {
                return Ok(vec![run(&["/usr/sbin/scutil", "--set", "HostName", &name]), run(&["/usr/sbin/scutil", "--set", "LocalHostName", &name]), run(&["/usr/sbin/scutil", "--set", "ComputerName", &name])]);
            }
            Ok(vec![run(&["/usr/bin/hostnamectl", "set-hostname", "--", &name])])
        }
        "time.ntp" => {
            let on = args.get("on").and_then(Value::as_bool).ok_or("on is missing")?;
            if cfg!(windows) {
                return Ok(vec![run(&["w32tm.exe", "/resync", "/force"])]);
            }
            if cfg!(target_os = "macos") {
                return Ok(vec![run(&["/usr/sbin/systemsetup", "-setusingnetworktime", if on { "on" } else { "off" }])]);
            }
            Ok(vec![run(&["/usr/bin/timedatectl", "set-ntp", if on { "true" } else { "false" }])])
        }
        "user.add" | "user.remove" => {
            let name = user_name(arg(args, "name")?)?;
            let owner = args.get("owner").and_then(Value::as_u64).unwrap_or(u64::MAX) as u32;
            if action == "user.remove" {
                if cfg!(unix) {
                    let uid = account(&name).ok_or_else(|| format!("There is no user called {name}."))?;
                    if uid < 1000 || uid == owner {
                        return Err(format!("I won't remove {name}; that account is needed."));
                    }
                }
                if matches!(name.as_str(), "root" | "administrator" | "admin") {
                    return Err(format!("I won't remove {name}; that account is needed."));
                }
                return Ok(if cfg!(windows) {
                    vec![run(&["net.exe", "user", &name, "/delete"])]
                } else if cfg!(target_os = "macos") {
                    vec![run(&["/usr/sbin/sysadminctl", "-deleteUser", &name])]
                } else {
                    vec![run(&["/usr/sbin/userdel", "--remove", "--", &name])]
                });
            }
            if cfg!(unix) && account(&name).is_some() {
                return Err(format!("There is already a user called {name}."));
            }
            let full = args.get("full").and_then(Value::as_str).unwrap_or(&name).chars().filter(|c| c.is_alphanumeric() || *c == ' ').take(60).collect::<String>();
            Ok(if cfg!(windows) {
                vec![run(&["net.exe", "user", &name, "/add", &format!("/fullname:{full}"), "/passwordreq:yes", "/logonpasswordchg:yes"])]
            } else if cfg!(target_os = "macos") {
                vec![run(&["/usr/sbin/sysadminctl", "-addUser", &name, "-fullName", &full])]
            } else {
                vec![run(&["/usr/sbin/useradd", "--create-home", "--comment", &full, "--", &name])]
            })
        }
        "backlight.set" => {
            let dev = arg(args, "device")?;
            let (name, max) = host.backlights.iter().find(|(n, _)| n == dev).ok_or("There is no such screen backlight.")?;
            let value = args.get("value").and_then(Value::as_u64).ok_or("value is missing")?.min(*max);
            Ok(vec![Step::Write { path: PathBuf::from("/sys/class/backlight").join(name).join("brightness"), text: value.to_string(), mode: 0o644 }])
        }
        "firefox.autoconfig.install" | "firefox.autoconfig.remove" => {
            let dir = firefox_dir(arg(args, "dir")?)?;
            if strict && !owned_by_root(&dir) {
                return Err("That Firefox folder isn't a system install.".into());
            }
            let pref = dir.join("defaults").join("pref").join(crate::autoconfig::PREF_FILE);
            let cfg = dir.join(crate::autoconfig::CFG_FILE);
            if action.ends_with("remove") {
                return Ok(vec![Step::Remove(pref), Step::Remove(cfg)]);
            }
            Ok(vec![Step::Write { path: pref, text: crate::autoconfig::PREF.into(), mode: 0o644 }, Step::Write { path: cfg, text: crate::autoconfig::SCRIPT.into(), mode: 0o644 }])
        }
        "secure.keys" | "secure.click" => {
            if !cfg!(windows) {
                return Err("The secure desktop exists only on Windows.".into());
            }
            Ok(vec![Step::Secure(serde_json::json!({ "op": action, "args": args }))])
        }
        _ => Err(format!("{action} isn't something the system helper does.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn host() -> Host {
        Host { pm: Some(Pm::Dnf("/usr/bin/dnf5".into())), firewalld: true, backlights: vec![("intel_backlight".into(), 1000)], stage_dir: PathBuf::from("/var/cache/setu-system") }
    }

    #[test]
    fn names_with_shell_or_option_tricks_are_refused() {
        for bad in ["-rf", "a;b", "a b", "$(x)", "x|y", "../etc", "", "a\nb", "--now"] {
            assert!(unit_name(bad).is_err(), "{bad}");
            assert!(package_name(bad).is_err(), "{bad}");
        }
        assert_eq!(unit_name("bluetooth").unwrap(), "bluetooth.service");
        assert_eq!(unit_name("getty@tty2.service").unwrap(), "getty@tty2.service");
        assert_eq!(package_name("gimp").unwrap(), "gimp");
        assert_eq!(package_name("python3-pip").unwrap(), "python3-pip");
        assert!(host_name("my-pc").is_ok());
        assert!(host_name("-pc").is_err() && host_name("a.b").is_err());
        assert_eq!(user_name("Guest").unwrap(), "guest");
        assert!(user_name("1x").is_err() && user_name("a b").is_err() && user_name("-r").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn plans_are_fixed_commands() {
        let h = host();
        assert_eq!(plan("service.restart", &json!({ "unit": "bluetooth" }), &h, true).unwrap(), vec![Step::Run(vec!["/usr/bin/systemctl".into(), "restart".into(), "--".into(), "bluetooth.service".into()])]);
        assert!(plan("service.stop", &json!({ "unit": "dbus" }), &h, true).is_err());
        assert!(plan("service.restart", &json!({ "unit": "dbus" }), &h, true).is_ok());
        assert_eq!(plan("package.install", &json!({ "name": "gimp" }), &h, true).unwrap(), vec![Step::Run(vec!["/usr/bin/dnf5".into(), "-y".into(), "install".into(), "gimp".into()])]);
        assert_eq!(plan("firewall.port.open", &json!({ "port": 8080 }), &h, true).unwrap().len(), 2);
        assert!(plan("firewall.port.open", &json!({ "port": 0 }), &h, true).is_err());
        assert!(plan("shell.run", &json!({ "cmd": "id" }), &h, true).is_err());
        assert_eq!(plan("backlight.set", &json!({ "device": "intel_backlight", "value": 5000 }), &h, true).unwrap(), vec![Step::Write { path: PathBuf::from("/sys/class/backlight/intel_backlight/brightness"), text: "1000".into(), mode: 0o644 }]);
        assert!(plan("backlight.set", &json!({ "device": "../../etc", "value": 1 }), &h, true).is_err());
        assert!(plan("package.install_file", &json!({ "path": "/home/u/x.deb", "owner": 1000 }), &h, true).is_err());
        assert!(plan("user.remove", &json!({ "name": "root", "owner": 1000 }), &h, true).is_err());
        assert!(plan("user.add", &json!({ "name": "root", "owner": 1000 }), &h, true).is_err());
        assert_eq!(plan("user.add", &json!({ "name": "setuguest9", "owner": 1000 }), &h, true).unwrap(), vec![Step::Run(vec!["/usr/sbin/useradd".into(), "--create-home".into(), "--comment".into(), "setuguest9".into(), "--".into(), "setuguest9".into()])]);
        let staged = plan("package.install_file", &json!({ "path": "/home/u/Downloads/app 1.rpm", "owner": 1000 }), &h, true).unwrap();
        assert!(matches!(&staged[0], Step::Stage { owner: 1000, .. }));
        assert!(matches!(&staged[2], Step::Remove(p) if p.starts_with("/var/cache/setu-system")));
    }

    #[test]
    fn firefox_autoconfig_needs_a_real_firefox_folder_and_fixed_contents() {
        let t = tempfile::tempdir().unwrap();
        assert!(plan("firefox.autoconfig.install", &json!({ "dir": t.path().to_string_lossy() }), &host(), false).is_err());
        std::fs::write(t.path().join("application.ini"), "[App]\nVendor=Mozilla\nName=Firefox\n").unwrap();
        let steps = plan("firefox.autoconfig.install", &json!({ "dir": t.path().to_string_lossy(), "script": "evil()" }), &host(), false).unwrap();
        assert!(steps.iter().all(|s| !matches!(s, Step::Write { text, .. } if text.contains("evil"))));
        #[cfg(unix)]
        assert!(plan("firefox.autoconfig.install", &json!({ "dir": t.path().to_string_lossy() }), &host(), true).is_err());
    }
}
