use crate::actions::Host;
use crate::exec::Mode;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Config {
    pub uid: u32,
    pub daemon: Option<PathBuf>,
}

pub struct Peer {
    pub uid: u32,
    pub pid: i32,
}

#[cfg(target_os = "linux")]
pub fn peer(s: &UnixStream) -> Option<Peer> {
    let mut c: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut c as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0).then_some(Peer { uid: c.uid, pid: c.pid })
}

#[cfg(target_os = "macos")]
pub fn peer(s: &UnixStream) -> Option<Peer> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    if unsafe { libc::getpeereid(s.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return None;
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), 0, libc::LOCAL_PEERPID, &mut pid as *mut _ as *mut libc::c_void, &mut len) };
    Some(Peer { uid, pid: if r == 0 { pid } else { -1 } })
}

#[cfg(target_os = "linux")]
pub fn exe_of(pid: i32) -> Option<PathBuf> {
    let p = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let s = p.to_string_lossy();
    Some(PathBuf::from(s.trim_end_matches(" (deleted)")))
}

#[cfg(target_os = "macos")]
pub fn exe_of(pid: i32) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
}

pub fn allowed(cfg: &Config, p: &Peer, strict: bool) -> Result<(), String> {
    if p.uid != cfg.uid {
        return Err(format!("user {} is not allowed", p.uid));
    }
    if strict && let Some(want) = &cfg.daemon {
        let got = exe_of(p.pid).ok_or("the caller's program could not be read")?;
        let canon = |x: &Path| x.canonicalize().unwrap_or_else(|_| x.to_path_buf());
        if canon(&got) != canon(want) {
            return Err(format!("{} is not Setu", got.display()));
        }
    }
    Ok(())
}

pub type Extra = Arc<dyn Fn(&str, &serde_json::Value, u32) -> Option<Result<serde_json::Value, String>> + Send + Sync>;

fn serve_one(stream: UnixStream, cfg: Arc<Config>, host: Arc<Host>, mode: Arc<Mode>, extra: Extra) {
    let strict = matches!(*mode, Mode::Real);
    let Some(p) = peer(&stream) else { return };
    if let Err(e) = allowed(&cfg, &p, strict) {
        eprintln!("setu-system: refused a connection: {e}");
        return;
    }
    let Ok(writer) = stream.try_clone() else { return };
    let writer = Arc::new(Mutex::new(writer));
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.len() > 1024 * 1024 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let (cfg, host, mode, writer, extra) = (cfg.clone(), host.clone(), mode.clone(), writer.clone(), extra.clone());
        std::thread::spawn(move || {
            let reply = crate::proto::handle(&line, cfg.uid, &host, &mode, &|_| Err("The secure desktop exists only on Windows.".into()), &*extra);
            let mut w = writer.lock().unwrap();
            let _ = writeln!(w, "{reply}");
        });
    }
}

pub fn serve(socket: &Path, cfg: Config, mode: Mode, host: Host, extra: Extra) -> Result<(), String> {
    if let Some(d) = socket.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(socket);
    let old = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    unsafe { libc::umask(old) };
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    if unsafe { libc::geteuid() } == 0 {
        let c = std::ffi::CString::new(socket.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        if unsafe { libc::chown(c.as_ptr(), cfg.uid, u32::MAX) } != 0 {
            return Err("the socket could not be handed to the user".into());
        }
    }
    let (cfg, host, mode) = (Arc::new(cfg), Arc::new(host), Arc::new(mode));
    for s in listener.incoming() {
        let Ok(s) = s else { continue };
        let (cfg, host, mode, extra) = (cfg.clone(), host.clone(), mode.clone(), extra.clone());
        std::thread::spawn(move || serve_one(s, cfg, host, mode, extra));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_configured_user_gets_in() {
        let cfg = Config { uid: 1000, daemon: None };
        assert!(allowed(&cfg, &Peer { uid: 1000, pid: 1 }, false).is_ok());
        assert!(allowed(&cfg, &Peer { uid: 1001, pid: 1 }, false).is_err());
        assert!(allowed(&cfg, &Peer { uid: 0, pid: 1 }, false).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_program_must_be_setu_when_strict() {
        let me = std::env::current_exe().unwrap();
        let uid = unsafe { libc::getuid() };
        let pid = std::process::id() as i32;
        assert!(allowed(&Config { uid, daemon: Some(me) }, &Peer { uid, pid }, true).is_ok());
        assert!(allowed(&Config { uid, daemon: Some(PathBuf::from("/usr/bin/true")) }, &Peer { uid, pid }, true).is_err());
    }

}
