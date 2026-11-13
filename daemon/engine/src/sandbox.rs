use setu_common::engine::EngineKind;

#[cfg(target_os = "linux")]
pub fn apply(kind: EngineKind, allow: &[String], mem_limit: Option<u64>, gpu: bool) {
    use landlock::{
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    };
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
    }
    if let Some(bytes) = mem_limit {
        let lim = libc::rlimit { rlim_cur: bytes, rlim_max: bytes };
        unsafe { libc::setrlimit(libc::RLIMIT_DATA, &lim) };
    }
    let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    unsafe { libc::setrlimit(libc::RLIMIT_CORE, &core) };
    let abi = ABI::V5;
    let mut read: Vec<String> = vec!["/usr".into(), "/lib".into(), "/lib64".into(), "/etc".into(), "/proc".into(), "/sys".into(), "/dev".into(), "/opt".into(), "/nix".into()];
    read.extend(allow.iter().cloned());
    if let Ok(me) = std::env::current_exe()
        && let Some(d) = me.parent() {
            read.push(d.to_string_lossy().into_owned());
            if let Some(p) = d.parent() {
                read.push(p.join("lib").to_string_lossy().into_owned());
            }
        }
    if kind == EngineKind::Tts {
        if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
            read.push(rt);
        }
        if let Some(home) = std::env::var_os("HOME") {
            let h = std::path::PathBuf::from(home);
            read.push(h.join(".config/pulse").to_string_lossy().into_owned());
            read.push(h.join(".pulse-cookie").to_string_lossy().into_owned());
        }
    }
    let result = (|| -> Result<(), landlock::RulesetError> {
        let mut rs = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(abi))?
            .handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp)?
            .create()?;
        for p in &read {
            if let Ok(fd) = PathFd::new(p) {
                rs = rs.add_rule(PathBeneath::new(fd, AccessFs::from_read(abi)))?;
            }
        }
        let mut full: Vec<String> = vec!["/dev/null".into(), "/dev/shm".into()];
        if gpu {
            full.extend(gpu_nodes());
        }
        for p in &full {
            if let Ok(fd) = PathFd::new(p) {
                rs = rs.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))?;
            }
        }
        rs.restrict_self()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("sandbox not applied: {e}");
    }
}

#[cfg(target_os = "linux")]
fn gpu_nodes() -> Vec<String> {
    let mut v = vec!["/dev/dri".to_owned(), "/dev/kfd".to_owned()];
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("nvidia") {
                v.push(format!("/dev/{n}"));
            }
        }
    }
    let nvidia = v.iter().any(|n| n.starts_with("/dev/nvidia"));
    let cache = std::env::var_os("XDG_CACHE_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")));
    if let Some(c) = cache {
        for d in ["nvidia", "mesa_shader_cache", "mesa_shader_cache_db", "radv_builtin_shaders"] {
            let p = c.join(d);
            let wanted = (d == "nvidia" && nvidia) || (d == "mesa_shader_cache_db" && std::path::Path::new("/dev/dri").exists());
            if wanted && !p.exists() {
                let _ = std::fs::create_dir_all(&p);
            }
            v.push(p.to_string_lossy().into_owned());
        }
    }
    v
}

#[cfg(not(target_os = "linux"))]
pub fn apply(_kind: EngineKind, _allow: &[String], _mem_limit: Option<u64>, _gpu: bool) {}
