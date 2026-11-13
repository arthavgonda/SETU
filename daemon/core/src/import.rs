use crate::profiles::Profile;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SKIP: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache", "GrShaderCache", "GraphiteDawnCache", "ShaderCache", "Media Cache", "Crashpad", "Crash Reports", "CacheStorage", "ScriptCache", "cache2", "startupCache", "thumbnails", "SingletonLock",
    "SingletonSocket", "SingletonCookie", "lock", ".parentlock", "parent.lock", "BrowserMetrics", "component_crx_cache", "optimization_guide_model_store",
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub profiles: Vec<String>,
    pub files: u64,
    pub bytes: u64,
}

pub fn running(root: &Path) -> bool {
    std::fs::symlink_metadata(root.join("SingletonLock")).is_ok() || root.join(".parentlock").exists() && cfg!(windows) || std::fs::symlink_metadata(root.join("lock")).is_ok_and(|m| m.file_type().is_symlink())
}

fn skipped(name: &str) -> bool {
    SKIP.contains(&name) || name.ends_with(".tmp")
}

pub fn size_of(p: &Path) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if m.is_file() {
        return m.len();
    }
    if !m.is_dir() {
        return 0;
    }
    std::fs::read_dir(p).map(|rd| rd.flatten().filter(|e| !skipped(&e.file_name().to_string_lossy())).map(|e| size_of(&e.path())).sum()).unwrap_or(0)
}

fn copy_tree(from: &Path, to: &Path, r: &mut Report) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        if skipped(&name.to_string_lossy()) {
            continue;
        }
        let src = e.path();
        let dst = to.join(&name);
        let m = std::fs::symlink_metadata(&src)?;
        if m.is_dir() {
            copy_tree(&src, &dst, r)?;
        } else if m.is_file() {
            std::fs::copy(&src, &dst)?;
            r.files += 1;
            r.bytes += m.len();
        }
    }
    Ok(())
}

fn free_space(p: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0 {
            return Some(st.f_bavail as u64 * st.f_frsize as u64);
        }
    }
    let _ = p;
    None
}

pub fn targets(chosen: &[Profile], main: &Profile) -> Vec<(Profile, String)> {
    let mut used: Vec<String> = vec!["Default".into()];
    let mut out = Vec::new();
    for p in chosen {
        let t = if p.dir == main.dir {
            "Default".to_owned()
        } else if used.contains(&p.dir) {
            let mut n = 100;
            while used.contains(&format!("Profile {n}")) || chosen.iter().any(|q| q.dir == format!("Profile {n}")) {
                n += 1;
            }
            format!("Profile {n}")
        } else {
            p.dir.clone()
        };
        used.push(t.clone());
        out.push((p.clone(), t));
    }
    out
}

fn local_state(src_root: &Path, chosen: &[Profile], main: &Profile) -> Value {
    let mut v: Value = std::fs::read_to_string(src_root.join("Local State")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}));
    let cache = v.pointer("/profile/info_cache").cloned().unwrap_or_else(|| json!({}));
    let mut kept = serde_json::Map::new();
    let map = targets(chosen, main);
    for (p, target) in &map {
        if let Some(info) = cache.get(&p.dir) {
            kept.insert(target.clone(), info.clone());
        }
    }
    if v.get("profile").is_none() {
        v["profile"] = json!({});
    }
    v["profile"]["info_cache"] = Value::Object(kept);
    v["profile"]["last_used"] = json!("Default");
    v["profile"]["last_active_profiles"] = json!(["Default"]);
    v["profile"]["profiles_order"] = json!(map.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
    v
}

pub fn chromium(src_root: &Path, dst: &Path, chosen: &[Profile], main: &Profile) -> Result<Report> {
    if running(src_root) {
        return Err(anyhow!("Your old browser is still open. Close it, then ask me again."));
    }
    let need: u64 = chosen.iter().map(|p| size_of(&src_root.join(&p.dir))).sum();
    if let Some(free) = free_space(dst.parent().unwrap_or(dst))
        && free < need + need / 10 + (64 << 20)
    {
        return Err(anyhow!("There isn't enough free space to bring your profiles over: {} MB are needed.", need >> 20));
    }
    let tmp = dst.with_extension("import");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut r = Report::default();
    let res = (|| -> Result<()> {
        for (p, target) in targets(chosen, main) {
            copy_tree(&src_root.join(&p.dir), &tmp.join(&target), &mut r)?;
            r.profiles.push(if p.name.is_empty() { p.dir.clone() } else { p.name.clone() });
        }
        std::fs::write(tmp.join("Local State"), local_state(src_root, chosen, main).to_string())?;
        for keep in [".setu-profile", ".setu-default"] {
            if dst.join(keep).exists() {
                std::fs::copy(dst.join(keep), tmp.join(keep))?;
            }
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let old = dst.with_extension("before-import");
    let _ = std::fs::remove_dir_all(&old);
    if dst.exists() {
        std::fs::rename(dst, &old)?;
    }
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::rename(&old, dst);
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(r)
}

pub fn firefox(src_root: &Path, dst: &Path, chosen: &Profile) -> Result<Report> {
    let src = if Path::new(&chosen.dir).is_absolute() { PathBuf::from(&chosen.dir) } else { src_root.join(&chosen.dir) };
    if src.join("lock").exists() || std::fs::symlink_metadata(src.join("lock")).is_ok() || src.join(".parentlock").exists() && running(&src) {
        return Err(anyhow!("Firefox is still open on that profile. Close it, then ask me again."));
    }
    let tmp = dst.with_extension("import");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut r = Report::default();
    if let Err(e) = copy_tree(&src, &tmp, &mut r) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    for keep in [".setu-profile", ".setu-default"] {
        if dst.join(keep).exists() {
            std::fs::copy(dst.join(keep), tmp.join(keep))?;
        }
    }
    r.profiles.push(if chosen.name.is_empty() { chosen.dir.clone() } else { chosen.name.clone() });
    let old = dst.with_extension("before-import");
    let _ = std::fs::remove_dir_all(&old);
    if dst.exists() {
        std::fs::rename(dst, &old)?;
    }
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::rename(&old, dst);
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    let _ = std::fs::remove_dir_all(&old);
    crate::bidi::write_prefs(dst)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(dir: &str, name: &str) -> Profile {
        Profile { dir: dir.into(), name: name.into(), email: String::new(), current: false }
    }

    #[test]
    fn copies_chosen_profiles_without_caches_and_keeps_the_original() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("chrome");
        std::fs::create_dir_all(src.join("Default/Cache")).unwrap();
        std::fs::create_dir_all(src.join("Profile 1")).unwrap();
        std::fs::write(src.join("Default/Bookmarks"), "{\"roots\":{}}").unwrap();
        std::fs::write(src.join("Default/Cache/data_0"), vec![0u8; 4096]).unwrap();
        std::fs::write(src.join("Profile 1/Bookmarks"), "work").unwrap();
        std::fs::write(src.join("Local State"), r#"{"profile":{"info_cache":{"Default":{"name":"Anoop"},"Profile 1":{"name":"Work"}}}}"#).unwrap();
        let dst = t.path().join("setu/chrome");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join(".setu-profile"), "{}").unwrap();
        let chosen = vec![profile("Profile 1", "Work"), profile("Default", "Anoop")];
        let r = chromium(&src, &dst, &chosen, &chosen[0]).unwrap();
        assert_eq!(r.profiles, vec!["Work".to_owned(), "Anoop".to_owned()]);
        assert_eq!(std::fs::read_to_string(dst.join("Default/Bookmarks")).unwrap(), "work");
        assert_eq!(std::fs::read_to_string(dst.join("Profile 100/Bookmarks")).unwrap(), "{\"roots\":{}}");
        assert!(!dst.join("Profile 1").exists());
        assert!(!dst.join("Profile 100/Cache").exists());
        assert!(dst.join(".setu-profile").exists());
        let ls: Value = serde_json::from_str(&std::fs::read_to_string(dst.join("Local State")).unwrap()).unwrap();
        assert_eq!(ls.pointer("/profile/info_cache/Default/name").and_then(Value::as_str), Some("Work"));
        assert_eq!(ls.pointer("/profile/info_cache/Profile 100/name").and_then(Value::as_str), Some("Anoop"));
        assert!(src.join("Default/Cache/data_0").exists() && src.join("Profile 1/Bookmarks").exists());
    }

    #[test]
    fn a_running_browser_is_not_copied() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("chrome");
        std::fs::create_dir_all(src.join("Default")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("host-123", src.join("SingletonLock")).unwrap();
        let dst = t.path().join("setu");
        let p = profile("Default", "Me");
        #[cfg(unix)]
        assert!(chromium(&src, &dst, std::slice::from_ref(&p), &p).is_err());
    }
}
