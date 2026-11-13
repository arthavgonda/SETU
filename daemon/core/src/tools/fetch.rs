#![cfg_attr(target_os = "linux", allow(dead_code))]

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(secs)))
        .user_agent("setu-installer")
        .build()
        .into()
}

pub fn get_text(url: &str) -> Result<String, String> {
    let mut r = agent(60).get(url).call().map_err(|e| format!("{e}"))?;
    r.body_mut().with_config().limit(64 * 1024 * 1024).read_to_string().map_err(|e| format!("{e}"))
}

pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join("setu-installs");
    let _ = std::fs::create_dir_all(&d);
    d.join(name)
}

pub fn download(url: &str, dest: &Path, sha256: Option<&str>, cancel: &AtomicBool) -> Result<(), String> {
    let mut r = agent(3600).get(url).call().map_err(|e| format!("The download failed: {e}"))?;
    let mut body = r.body_mut().with_config().limit(4 * 1024 * 1024 * 1024).reader();
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(dest);
            return Err("Cancelled.".into());
        }
        let n = body.read(&mut buf).map_err(|e| format!("The download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    f.flush().map_err(|e| e.to_string())?;
    if let Some(want) = sha256.map(str::trim).filter(|s| s.len() == 64) {
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if !got.eq_ignore_ascii_case(want) {
            let _ = std::fs::remove_file(dest);
            return Err("The download didn't match its checksum, so I didn't install it.".into());
        }
    }
    Ok(())
}

pub fn version_key(v: &str) -> Vec<(u64, String)> {
    v.split(['.', '-', '_', '+']).map(|p| {
        let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
        (digits.parse::<u64>().unwrap_or(0), p[digits.len()..].to_owned())
    }).collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Installer {
    pub arch: String,
    pub kind: String,
    pub url: String,
    pub sha256: String,
    pub scope: String,
    pub silent: String,
}

fn unquote(v: &str) -> String {
    let t = v.trim();
    let t = t.split(" #").next().unwrap_or(t).trim();
    t.trim_matches('"').trim_matches('\'').to_owned()
}

pub fn parse_installers(yaml: &str) -> Vec<Installer> {
    let mut base = Installer::default();
    let mut items: Vec<Installer> = Vec::new();
    let mut in_list = false;
    let mut item_indent = 0usize;
    let mut switches_indent: Option<usize> = None;
    let mut top_switches = false;
    for raw in yaml.lines() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let line = raw.trim();
        if indent == 0 && !line.starts_with("- ") {
            in_list = line.starts_with("Installers:");
            top_switches = line.starts_with("InstallerSwitches:");
            switches_indent = None;
            if let Some((k, v)) = line.split_once(':') {
                let v = unquote(v);
                match k {
                    "InstallerType" => base.kind = v.to_lowercase(),
                    "Scope" => base.scope = v.to_lowercase(),
                    _ => {}
                }
            }
            continue;
        }
        if top_switches && !in_list {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim() {
                    "Silent" => base.silent = unquote(v),
                    "SilentWithProgress" if base.silent.is_empty() => base.silent = unquote(v),
                    _ => {}
                }
            }
            continue;
        }
        if !in_list {
            continue;
        }
        let mut body = line;
        if let Some(rest) = line.strip_prefix("- ") {
            item_indent = indent;
            switches_indent = None;
            items.push(Installer { kind: base.kind.clone(), scope: base.scope.clone(), silent: base.silent.clone(), ..Default::default() });
            body = rest.trim();
        } else if indent <= item_indent {
            continue;
        }
        let Some(cur) = items.last_mut() else { continue };
        let Some((k, v)) = body.split_once(':') else { continue };
        let k = k.trim();
        if let Some(si) = switches_indent {
            if indent > si && !line.starts_with("- ") {
                match k {
                    "Silent" => cur.silent = unquote(v),
                    "SilentWithProgress" if cur.silent.is_empty() => cur.silent = unquote(v),
                    _ => {}
                }
                continue;
            }
            switches_indent = None;
        }
        match k {
            "Architecture" => cur.arch = unquote(v).to_lowercase(),
            "InstallerType" => cur.kind = unquote(v).to_lowercase(),
            "InstallerUrl" => cur.url = unquote(v),
            "InstallerSha256" => cur.sha256 = unquote(v),
            "Scope" => cur.scope = unquote(v).to_lowercase(),
            "InstallerSwitches" => {
                switches_indent = Some(if line.starts_with("- ") { indent + 2 } else { indent });
                cur.silent.clear();
            }
            _ => {}
        }
    }
    items.into_iter().filter(|i| !i.url.is_empty()).collect()
}

pub fn pick(list: &[Installer], arch: &str) -> Option<Installer> {
    let usable = ["msi", "wix", "nullsoft", "inno", "burn", "exe", "msix", "appx"];
    let rank = |i: &Installer| -> Option<(u8, u8, u8)> {
        let k = usable.iter().position(|u| *u == i.kind)? as u8;
        let a = match i.arch.as_str() {
            x if x == arch => 0,
            "neutral" | "" => 1,
            "x86" if arch == "x64" => 2,
            _ => return None,
        };
        let scope = if i.scope == "user" { 0 } else if i.scope.is_empty() { 1 } else { 2 };
        Some((a, scope, k))
    };
    list.iter().filter_map(|i| rank(i).map(|r| (r, i))).min_by_key(|(r, _)| *r).map(|(_, i)| i.clone())
}

pub fn silent_args(i: &Installer) -> Vec<String> {
    if !i.silent.is_empty() {
        return i.silent.split_whitespace().map(str::to_owned).collect();
    }
    let v: &[&str] = match i.kind.as_str() {
        "nullsoft" => &["/S"],
        "inno" => &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"],
        "burn" => &["/quiet", "/norestart"],
        _ => &[],
    };
    v.iter().map(|s| (*s).to_owned()).collect()
}

pub fn manifest_dir(id: &str) -> Option<String> {
    let first = id.chars().next()?.to_ascii_lowercase();
    Some(format!("manifests/{first}/{}", id.replace('.', "/")))
}

#[cfg(windows)]
pub mod win {
    use super::*;
    use crate::proc::Quiet;
    use std::process::{Command, Stdio};

    pub fn winget() -> Option<String> {
        let local = std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft\\WindowsApps\\winget.exe"));
        for c in ["winget".to_owned()].into_iter().chain(local.map(|p| p.to_string_lossy().into_owned())) {
            let ok = Command::new(&c).quiet().arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
            if ok {
                return Some(c);
            }
        }
        None
    }

    fn powershell(script: &str) -> bool {
        Command::new("powershell")
            .quiet()
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn ensure_winget() -> Option<String> {
        if let Some(w) = winget() {
            return Some(w);
        }
        powershell("Add-AppxPackage -RegisterByFamilyName -MainPackage Microsoft.DesktopAppInstaller_8wekyb3d8bbwe -ErrorAction SilentlyContinue");
        if let Some(w) = winget() {
            return Some(w);
        }
        let dir = scratch("winget");
        let _ = std::fs::create_dir_all(&dir);
        let d = dir.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ProgressPreference='SilentlyContinue'; $d='{d}'; \
             Invoke-WebRequest -UseBasicParsing -Uri 'https://aka.ms/Microsoft.VCLibs.x64.14.00.Desktop.appx' -OutFile \"$d\\vclibs.appx\"; \
             Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/microsoft/microsoft-ui-xaml/releases/download/v2.8.6/Microsoft.UI.Xaml.2.8.x64.appx' -OutFile \"$d\\xaml.appx\"; \
             Invoke-WebRequest -UseBasicParsing -Uri 'https://aka.ms/getwinget' -OutFile \"$d\\winget.msixbundle\"; \
             Add-AppxPackage -Path \"$d\\vclibs.appx\" -ErrorAction SilentlyContinue; \
             Add-AppxPackage -Path \"$d\\xaml.appx\" -ErrorAction SilentlyContinue; \
             Add-AppxPackage -Path \"$d\\winget.msixbundle\""
        );
        powershell(&script);
        let _ = std::fs::remove_dir_all(&dir);
        winget()
    }

    fn latest_version(id: &str) -> Result<String, String> {
        let dir = manifest_dir(id).ok_or("That package has no name.")?;
        let text = get_text(&format!("https://api.github.com/repos/microsoft/winget-pkgs/contents/{dir}"))?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut versions: Vec<String> = v
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("dir"))
            .filter_map(|e| e.get("name").and_then(serde_json::Value::as_str).map(str::to_owned))
            .filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .collect();
        versions.sort_by_key(|a| version_key(a));
        versions.pop().ok_or_else(|| "I couldn't find that app's installer.".into())
    }

    fn run(file: &Path, args: &[String]) -> Result<(), String> {
        match Command::new(file).quiet().args(args).stdin(Stdio::null()).status() {
            Ok(s) if s.success() || s.code() == Some(3010) => Ok(()),
            Ok(s) => Err(format!("The installer stopped with code {}.", s.code().unwrap_or(-1))),
            Err(e) if e.raw_os_error() == Some(740) => {
                let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
                let list = if quoted.is_empty() { String::new() } else { format!(" -ArgumentList {}", quoted.join(",")) };
                let script = format!("$p = Start-Process -FilePath '{}'{list} -Verb RunAs -Wait -PassThru; exit $p.ExitCode", file.to_string_lossy().replace('\'', "''"));
                if powershell(&script) { Ok(()) } else { Err("The install was not approved.".into()) }
            }
            Err(e) => Err(format!("The installer could not start: {e}")),
        }
    }

    pub fn direct(id: &str, cancel: &AtomicBool) -> Result<(), String> {
        let version = latest_version(id)?;
        let dir = manifest_dir(id).ok_or("That package has no name.")?;
        let yaml = get_text(&format!("https://raw.githubusercontent.com/microsoft/winget-pkgs/master/{dir}/{version}/{id}.installer.yaml"))?;
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
        let inst = pick(&parse_installers(&yaml), arch).ok_or("This app has no installer I can run for you.")?;
        let ext = match inst.kind.as_str() {
            "msi" | "wix" => "msi",
            "msix" | "appx" => "msix",
            _ => "exe",
        };
        let file = scratch(&format!("{}-{version}.{ext}", id.replace('.', "-")));
        download(&inst.url, &file, Some(&inst.sha256), cancel)?;
        let r = match ext {
            "msi" => {
                let mut a = vec!["/i".to_owned(), file.to_string_lossy().into_owned(), "/passive".to_owned(), "/norestart".to_owned()];
                if inst.scope == "user" {
                    a.push("MSIINSTALLPERUSER=1".into());
                }
                run(Path::new("msiexec"), &a)
            }
            "msix" => {
                if powershell(&format!("Add-AppxPackage -Path '{}'", file.to_string_lossy().replace('\'', "''"))) { Ok(()) } else { Err("The app package could not be added.".into()) }
            }
            _ => run(&file, &silent_args(&inst)),
        };
        let _ = std::fs::remove_file(&file);
        r
    }
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::*;
    use serde_json::Value;
    use std::process::Command;

    #[cfg(target_os = "macos")]
    pub fn install_file(file: &std::path::Path) -> Result<(), String> {
        let name = file.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let admin = |cmd: String| -> Result<(), String> {
            let script = format!("do shell script {} with administrator privileges", serde_json::to_string(&cmd).unwrap_or_default());
            let ok = Command::new("osascript").args(["-e", &script]).status().map(|s| s.success()).unwrap_or(false);
            if ok { Ok(()) } else { Err("The install was not approved.".into()) }
        };
        let quote = |p: &std::path::Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
        if name.ends_with(".pkg") {
            return admin(format!("/usr/sbin/installer -pkg {} -target /", quote(file)));
        }
        let mount = std::env::temp_dir().join(format!("setu-dmg-{}", std::process::id()));
        std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
        let attached = Command::new("hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mount).arg(file).status().map(|s| s.success()).unwrap_or(false);
        if !attached {
            return Err("The disk image couldn't be opened.".into());
        }
        let r = (|| {
            let entries: Vec<std::path::PathBuf> = std::fs::read_dir(&mount).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
            if let Some(app) = entries.iter().find(|p| p.extension().is_some_and(|e| e == "app")) {
                let dest = std::path::Path::new("/Applications").join(app.file_name().unwrap_or_default());
                let ok = Command::new("ditto").arg(app).arg(&dest).status().map(|s| s.success()).unwrap_or(false);
                return if ok { Ok(()) } else { admin(format!("/usr/bin/ditto {} {}", quote(app), quote(&dest))) };
            }
            if let Some(pkg) = entries.iter().find(|p| p.extension().is_some_and(|e| e == "pkg")) {
                return admin(format!("/usr/sbin/installer -pkg {} -target /", quote(pkg)));
            }
            Err("The disk image has no app inside.".into())
        })();
        let _ = Command::new("hdiutil").args(["detach", "-quiet"]).arg(&mount).status();
        let _ = std::fs::remove_dir(&mount);
        r
    }

    pub fn cask(token: &str) -> Result<Value, String> {
        let text = get_text(&format!("https://formulae.brew.sh/api/cask/{token}.json"))?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn search(name: &str) -> Option<(String, String)> {
        let cache = scratch("casks.json");
        let fresh = std::fs::metadata(&cache).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(86_400));
        let text = if fresh { std::fs::read_to_string(&cache).ok()? } else {
            let t = get_text("https://formulae.brew.sh/api/cask.json").ok()?;
            let _ = std::fs::write(&cache, &t);
            t
        };
        let all: Vec<Value> = serde_json::from_str(&text).ok()?;
        let want = crate::text::normalize(name);
        let mut best: Option<(f64, String, String)> = None;
        for c in &all {
            let token = c.get("token").and_then(Value::as_str).unwrap_or_default();
            let names: Vec<&str> = c.get("name").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut score = crate::text::similarity(&want, &token.replace('-', " "));
            for n in &names {
                score = score.max(crate::text::similarity(&want, &crate::text::normalize(n)));
            }
            if score >= 0.9 && best.as_ref().is_none_or(|(b, _, _)| score > *b) {
                best = Some((score, token.to_owned(), names.first().copied().unwrap_or(token).to_owned()));
            }
        }
        best.map(|(_, t, n)| (t, n))
    }

    fn target_dir() -> PathBuf {
        let sys = PathBuf::from("/Applications");
        let probe = sys.join(".setu-write-test");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return sys;
        }
        let home = dirs::home_dir().unwrap_or_default().join("Applications");
        let _ = std::fs::create_dir_all(&home);
        home
    }

    fn find_app(root: &Path, name: &str, depth: u32) -> Option<PathBuf> {
        let direct = root.join(name);
        if direct.exists() {
            return Some(direct);
        }
        if depth == 0 {
            return None;
        }
        for e in std::fs::read_dir(root).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() && p.extension().is_none_or(|x| x != "app")
                && let Some(f) = find_app(&p, name, depth - 1)
            {
                return Some(f);
            }
        }
        None
    }

    fn ok(c: &mut Command) -> bool {
        c.status().map(|s| s.success()).unwrap_or(false)
    }

    pub fn install(token: &str, cancel: &AtomicBool) -> Result<(), String> {
        let mut c = cask(token)?;
        let intel = if cfg!(target_arch = "x86_64") {
            c.get("variations").and_then(Value::as_object).and_then(|vars| vars.iter().find(|(k, _)| !k.starts_with("arm64")).map(|(_, v)| v.clone()))
        } else {
            None
        };
        if let Some(extra) = intel
            && let Some(o) = c.as_object_mut()
        {
            for (k, v) in extra.as_object().into_iter().flatten() {
                o.insert(k.clone(), v.clone());
            }
        }
        let url = c.get("url").and_then(Value::as_str).ok_or("That app has no download.")?.to_owned();
        let sha = c.get("sha256").and_then(Value::as_str).filter(|s| *s != "no_check").map(str::to_owned);
        let mut apps: Vec<String> = Vec::new();
        let mut pkgs: Vec<String> = Vec::new();
        for a in c.get("artifacts").and_then(Value::as_array).into_iter().flatten() {
            for (kind, list) in [("app", &mut apps), ("pkg", &mut pkgs)] {
                for v in a.get(kind).and_then(Value::as_array).into_iter().flatten() {
                    if let Some(s) = v.as_str() {
                        list.push(s.to_owned());
                    }
                }
            }
        }
        let lower = url.to_lowercase();
        let ext = if lower.contains(".dmg") { "dmg" } else if lower.contains(".zip") { "zip" } else if lower.contains(".pkg") { "pkg" } else { "dmg" };
        let file = scratch(&format!("{token}.{ext}"));
        download(&url, &file, sha.as_deref(), cancel)?;
        let work = scratch(&format!("{token}-files"));
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::create_dir_all(&work);
        let root = match ext {
            "dmg" => {
                if !ok(Command::new("hdiutil").args(["attach", "-nobrowse", "-noautoopen", "-readonly", "-mountpoint"]).arg(&work).arg(&file)) {
                    return Err("The disk image could not be opened.".into());
                }
                work.clone()
            }
            "zip" => {
                if !ok(Command::new("ditto").args(["-x", "-k"]).arg(&file).arg(&work)) {
                    return Err("The archive could not be opened.".into());
                }
                work.clone()
            }
            _ => file.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        let mut result = Err("This app's installer isn't supported yet.".to_owned());
        if let Some(app) = apps.first() {
            let name = Path::new(app).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(src) = find_app(&root, &name, 3) {
                let dest = target_dir().join(&name);
                let _ = std::fs::remove_dir_all(&dest);
                result = if ok(Command::new("ditto").arg(&src).arg(&dest)) { Ok(()) } else { Err("The app could not be copied to Applications.".into()) };
            }
        } else if let Some(pkg) = pkgs.first() {
            let path = if ext == "pkg" { file.clone() } else { find_app(&root, pkg, 3).unwrap_or_else(|| root.join(pkg)) };
            let script = format!("do shell script \"installer -pkg \" & quoted form of \"{}\" & \" -target /\" with administrator privileges", path.to_string_lossy().replace('"', ""));
            result = if ok(Command::new("osascript").args(["-e", &script])) { Ok(()) } else { Err("The install was not approved.".into()) };
        }
        if ext == "dmg" {
            let _ = Command::new("hdiutil").args(["detach", "-quiet"]).arg(&work).status();
        }
        let _ = std::fs::remove_dir_all(&work);
        let _ = std::fs::remove_file(&file);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_winget_installer_manifests() {
        let y = "PackageIdentifier: VideoLAN.VLC\nInstallerType: nullsoft\nInstallerSwitches:\n  Silent: /S /L=1033\nInstallers:\n- Architecture: x86\n  InstallerUrl: https://e.x/vlc-win32.exe\n  InstallerSha256: AA\n- Architecture: x64\n  InstallerUrl: https://e.x/vlc-win64.exe\n  InstallerSha256: BB\n- Architecture: x64\n  InstallerType: wix\n  Scope: machine\n  InstallerUrl: https://e.x/vlc.msi\n  InstallerSha256: CC\n  InstallerSwitches:\n    Silent: /qn\nManifestType: installer\n";
        let list = parse_installers(y);
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].silent, "/S /L=1033");
        assert_eq!(list[2].kind, "wix");
        assert_eq!(list[2].silent, "/qn");
        let p = pick(&list, "x64").unwrap();
        assert_eq!(p.url, "https://e.x/vlc-win64.exe");
        assert_eq!(silent_args(&p), vec!["/S", "/L=1033"]);
        let indented = "Installers:\n  - Architecture: x64\n    InstallerType: inno\n    InstallerUrl: 'https://e.x/a.exe'\n    InstallerSha256: DD\n";
        let l2 = parse_installers(indented);
        assert_eq!(l2[0].url, "https://e.x/a.exe");
        assert_eq!(silent_args(&l2[0])[0], "/VERYSILENT");
        assert_eq!(manifest_dir("VideoLAN.VLC").as_deref(), Some("manifests/v/VideoLAN/VLC"));
        let mut v = ["3.0.9".to_owned(), "3.0.21".to_owned(), "3.0.10".to_owned()];
        v.sort_by_key(|a| version_key(a));
        assert_eq!(v.last().map(String::as_str), Some("3.0.21"));
    }
}
