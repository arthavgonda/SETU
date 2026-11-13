use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DET_SHA: &str = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4";
pub const REC_SHA: &str = "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79";
pub const SIGNIN: &[&str] = &["gdm-password"];
pub const ADMIN: &[&str] = &["sudo", "sudo-i", "polkit-1"];
pub const MODULE: &str = "pam_setu.so";
const POLICY: &str = "setu_face";
const TRIES: u32 = 3;
const FORGIVE: Duration = Duration::from_secs(900);
const MAX_MODEL: u64 = 96 * 1024 * 1024;
const SIDES: &[&str] = &["center", "left", "right", "up", "down"];

#[derive(Default, Clone)]
struct Test {
    passcheck: Option<PathBuf>,
    sealer: Option<PathBuf>,
    loginctl: Option<PathBuf>,
    cgroup: Option<String>,
}

pub struct Face {
    root: PathBuf,
    real: bool,
    tools: PathBuf,
    socket: Mutex<Option<PathBuf>>,
    test: Test,
    busy: Mutex<()>,
    fails: Mutex<HashMap<u32, (u32, Instant)>>,
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub mode: String,
    pub user: String,
    pub service: String,
    pub seat: String,
    pub tty: String,
    pub rhost: String,
}

fn zero(v: &mut [u8]) {
    v.iter_mut().for_each(|b| *b = 0);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha_of(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn name_of(uid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 8192];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() || pw.pw_name.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

pub fn uid_of(name: &str) -> Option<u32> {
    let c = std::ffi::CString::new(name).ok()?;
    let mut buf = vec![0 as libc::c_char; 8192];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwnam_r(c.as_ptr(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    (r == 0 && !out.is_null()).then_some(pw.pw_uid)
}

pub fn user_name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.-@$".contains(c))
}

pub fn pam_line(mode: &str) -> String {
    format!("-auth       optional     {MODULE} {mode}")
}

fn is_auth(line: &str) -> bool {
    let t = line.trim_start();
    !t.starts_with('#') && matches!(t.split_whitespace().next(), Some("auth" | "-auth"))
}

pub fn add_line(text: &str, mode: &str) -> Option<String> {
    let ours = pam_line(mode);
    let base = strip_line(text);
    let lines: Vec<&str> = base.split('\n').collect();
    let at = lines.iter().position(|l| is_auth(l))?;
    let mut out: Vec<&str> = Vec::with_capacity(lines.len() + 1);
    out.extend_from_slice(&lines[..at]);
    out.push(&ours);
    out.extend_from_slice(&lines[at..]);
    Some(out.join("\n"))
}

pub fn strip_line(text: &str) -> String {
    text.split('\n').filter(|l| !(l.contains(MODULE) && !l.trim_start().starts_with('#'))).collect::<Vec<_>>().join("\n")
}

pub fn has_line(text: &str, mode: &str) -> bool {
    text.split('\n').any(|l| is_auth(l) && l.contains(MODULE) && l.split_whitespace().any(|w| w == mode))
}

pub fn session_in(cgroup: &str) -> Option<String> {
    for part in cgroup.split(['/', '\n']) {
        if let Some(id) = part.strip_prefix("session-").and_then(|r| r.strip_suffix(".scope"))
            && !id.is_empty()
            && id.len() <= 32
            && id.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Some(id.to_owned());
        }
    }
    None
}

pub fn session_local(props: &str, user: &str) -> bool {
    let get = |k: &str| props.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).unwrap_or("").trim().to_owned();
    get("Remote") == "no" && get("Active") == "yes" && get("Seat") == "seat0" && get("Class") == "user" && get("Name") == user && get("LockedHint") != "yes"
}

pub fn valid_templates(v: &Value) -> Result<(), String> {
    let bad = || "That face data isn't something I can use.".to_owned();
    let vector = |x: &Value| -> bool {
        let Some(a) = x.as_array() else { return false };
        if a.len() != 128 {
            return false;
        }
        let mut norm = 0f64;
        for n in a {
            let Some(f) = n.as_f64() else { return false };
            if !f.is_finite() {
                return false;
            }
            norm += f * f;
        }
        (0.8..1.25).contains(&norm.sqrt())
    };
    let angles = v.get("angles").and_then(Value::as_object).ok_or_else(bad)?;
    if angles.is_empty() || angles.len() > SIDES.len() {
        return Err(bad());
    }
    let mut count = 0;
    for (k, list) in angles {
        let list = list.as_array().ok_or_else(bad)?;
        if !SIDES.contains(&k.as_str()) || list.is_empty() || list.len() > 4 || !list.iter().all(vector) {
            return Err(bad());
        }
        count += list.len();
    }
    if let Some(l) = v.get("learned") {
        let l = l.as_array().ok_or_else(bad)?;
        if l.len() > 20 || !l.iter().all(vector) {
            return Err(bad());
        }
    }
    if count < 5 {
        return Err(bad());
    }
    Ok(())
}

pub const ANYONE: u32 = u32::MAX;
const GREETER: &str = "etc/dconf/db/gdm.d/95-setu-face";
const GREETER_TEXT: &str = "[org/gnome/login-screen]\ndisable-user-list=true\n";

pub fn parse_identify(line: &str) -> Option<Request> {
    let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
    if f.len() != 6 || f[0] != "identify" || f[1] != "signin" {
        return None;
    }
    let s = |i: usize| f[i].chars().filter(|c| !c.is_control()).take(256).collect::<String>();
    Some(Request { mode: s(1), user: String::new(), service: s(2), seat: s(3), tty: s(4), rhost: s(5) })
}

pub fn parse_request(line: &str) -> Option<Request> {
    let f: Vec<&str> = line.trim_end_matches(['\n', '\r']).split('\t').collect();
    if f.len() != 7 || f[0] != "check" || !matches!(f[1], "signin" | "admin") || !user_name_ok(f[2]) {
        return None;
    }
    let s = |i: usize| f[i].chars().filter(|c| !c.is_control()).take(256).collect::<String>();
    Some(Request { mode: s(1), user: s(2), service: s(3), seat: s(4), tty: s(5), rhost: s(6) })
}

fn run_with_input(prog: &Path, args: &[&str], input: &[u8], pass_env: bool) -> Result<(bool, Vec<u8>), String> {
    let mut cmd = Command::new(prog);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if !pass_env {
        cmd.env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").env("LANG", "C.UTF-8");
    }
    let mut child = cmd.spawn().map_err(|e| format!("{} didn't start: {e}", prog.display()))?;
    if let Some(mut s) = child.stdin.take() {
        let _ = s.write_all(input);
    }
    let mut out = Vec::new();
    if let Some(s) = child.stdout.take() {
        let _ = s.take(256 * 1024).read_to_end(&mut out);
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    Ok((ok, out))
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let dir = path.parent().ok_or("no folder")?;
    let name = path.file_name().ok_or("no name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.setu-new"));
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).custom_flags(libc::O_NOFOLLOW).open(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let r = f.write_all(bytes).and_then(|_| f.sync_all()).and_then(|_| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))).and_then(|_| std::fs::rename(&tmp, path));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

fn plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.file_type().is_file()).unwrap_or(false)
}

impl Face {
    pub fn new(root: &Path, real: bool, tools: Option<PathBuf>) -> Self {
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("/usr/libexec/setu"));
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let test = if real { Test::default() } else { Test { passcheck: env("SETU_PASSCHECK"), sealer: env("SETU_SEALER"), loginctl: env("SETU_LOGINCTL"), cgroup: std::env::var("SETU_FAKE_CGROUP").ok() } };
        let tools = if real { exe_dir } else { tools.unwrap_or(exe_dir) };
        Face { root: root.to_path_buf(), real, tools, socket: Mutex::new(None), test, busy: Mutex::new(()), fails: Mutex::new(HashMap::new()) }
    }

    fn state(&self) -> PathBuf {
        self.root.join("var/lib/setu-system/face")
    }

    fn checker(&self) -> PathBuf {
        self.tools.join("setu-facecheck")
    }

    fn module_source(&self) -> PathBuf {
        self.tools.join(MODULE)
    }

    fn policy_file(&self) -> PathBuf {
        self.tools.join(format!("{POLICY}.pp"))
    }

    fn trusted(&self, p: &Path) -> bool {
        if !self.real {
            return p.is_file();
        }
        std::fs::symlink_metadata(p).map(|m| m.file_type().is_file() && m.uid() == 0 && m.mode() & 0o022 == 0).unwrap_or(false)
    }

    fn ready(&self) -> bool {
        self.trusted(&self.checker()) && self.trusted(&self.module_source())
    }

    fn ensure_state(&self) -> Result<PathBuf, String> {
        let d = self.state();
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for p in [d.clone(), d.parent().map(Path::to_path_buf).unwrap_or_else(|| d.clone())] {
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700));
        }
        Ok(d)
    }

    fn password_ok(&self, user: &str, password: &[u8]) -> Result<bool, String> {
        let prog = if self.real {
            ["/usr/sbin/unix_chkpwd", "/sbin/unix_chkpwd", "/usr/bin/unix_chkpwd"].iter().map(PathBuf::from).find(|p| p.exists()).ok_or("This system has no password checker that Setu can use.")?
        } else {
            self.test.passcheck.clone().ok_or("No password checker is set for this test.")?
        };
        let mut input = password.to_vec();
        input.push(0);
        let r = run_with_input(&prog, &[user, "nonull"], &input, !self.real);
        zero(&mut input);
        r.map(|(ok, _)| ok)
    }

    fn sealer(&self, verb: &str, uid: u32, input: &[u8]) -> Result<Vec<u8>, String> {
        let name = format!("--name=setu-face-{uid}");
        let (ok, out) = if self.real {
            let prog = Path::new("/usr/bin/systemd-creds");
            if !prog.exists() {
                return Err("This system can't seal your password for the sign-in screen: systemd-creds is missing.".into());
            }
            if verb == "encrypt" { run_with_input(prog, &["encrypt", &name, "--with-key=auto", "-", "-"], input, false)? } else { run_with_input(prog, &["decrypt", &name, "-", "-"], input, false)? }
        } else {
            let prog = self.test.sealer.clone().ok_or("No sealer is set for this test.")?;
            run_with_input(&prog, &[verb], input, true)?
        };
        if !ok || out.is_empty() {
            return Err(if verb == "encrypt" { "Your password could not be sealed on this computer.".into() } else { "The sealed password could not be opened on this computer.".into() });
        }
        Ok(out)
    }

    fn seal_kind(&self) -> &'static str {
        if !self.real {
            return "test";
        }
        let tpm = Command::new("/usr/bin/systemd-creds").arg("has-tpm2").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
        if tpm { "tpm2" } else { "host" }
    }

    fn stage_model(&self, from: &Path, owner: u32, sha: &str, to: &Path) -> Result<(), String> {
        if std::fs::read(to).map(|b| sha_of(&b) == sha).unwrap_or(false) {
            return Ok(());
        }
        if !from.is_absolute() {
            return Err("The face model path isn't one I can use.".into());
        }
        let mut f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(from).map_err(|_| "I couldn't open the face models.".to_owned())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || (m.uid() != owner && m.uid() != 0) || m.len() > MAX_MODEL {
            return Err("The face models aren't files I can use.".into());
        }
        let mut bytes = Vec::with_capacity(m.len() as usize);
        (&mut f).take(MAX_MODEL + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if sha_of(&bytes) != sha {
            return Err("The face models on disk aren't the ones Setu ships. Download them again.".into());
        }
        write_atomic(to, &bytes, 0o644)
    }

    fn enrolled(&self, uid: u32) -> bool {
        let d = self.state();
        plain_file(&d.join(format!("{uid}.tpl"))) && plain_file(&d.join("det.onnx")) && plain_file(&d.join("rec.onnx"))
    }

    fn has_password(&self, uid: u32) -> bool {
        plain_file(&self.state().join(format!("{uid}.cred")))
    }

    fn pam_file(&self, service: &str) -> PathBuf {
        self.root.join("etc/pam.d").join(service)
    }

    fn mode_on(&self, mode: &str) -> bool {
        let list = if mode == "signin" { SIGNIN } else { ADMIN };
        list.iter().any(|s| std::fs::read_to_string(self.pam_file(s)).map(|t| has_line(&t, mode)).unwrap_or(false))
    }

    pub fn status(&self, uid: u32) -> Value {
        let locked = self.fails.lock().unwrap().get(&uid).is_some_and(|(n, at)| *n >= TRIES && at.elapsed() < FORGIVE);
        json!({
            "enrolled": self.enrolled(uid),
            "password": self.has_password(uid),
            "signin": self.mode_on("signin"),
            "noname": self.root.join(GREETER).exists(),
            "admin": self.mode_on("admin"),
            "ready": self.ready(),
            "greeter": plain_file(&self.pam_file("gdm-password")),
            "seal": self.seal_kind(),
            "locked": locked,
        })
    }

    pub fn enroll(&self, owner: u32, args: &Value) -> Result<Value, String> {
        let user = name_of(owner).ok_or("I couldn't find your user account.")?;
        let t = args.get("templates").ok_or("The face data is missing.")?;
        valid_templates(t)?;
        let password = args.get("password").and_then(Value::as_str).filter(|p| !p.is_empty() && p.len() <= 512 && !p.contains('\0')).ok_or("The system password is missing.")?;
        let path = |k: &str| args.get(k).and_then(Value::as_str).map(PathBuf::from).ok_or_else(|| "The face models are missing.".to_owned());
        let (det, rec) = (path("det")?, path("rec")?);
        if !self.password_ok(&user, password.as_bytes())? {
            return Err("That isn't your system password. Save the right one in the vault first.".into());
        }
        let dir = self.ensure_state()?;
        self.stage_model(&det, owner, DET_SHA, &dir.join("det.onnx"))?;
        self.stage_model(&rec, owner, REC_SHA, &dir.join("rec.onnx"))?;
        let blob = self.sealer("encrypt", owner, password.as_bytes())?;
        write_atomic(&dir.join(format!("{owner}.tpl")), t.to_string().as_bytes(), 0o600)?;
        write_atomic(&dir.join(format!("{owner}.cred")), &blob, 0o600)?;
        self.fails.lock().unwrap().remove(&owner);
        eprintln!("setu-system: face data saved for user {owner}");
        Ok(self.status(owner))
    }

    fn module_dir(&self) -> Option<PathBuf> {
        ["usr/lib64/security", "lib64/security", "usr/lib/x86_64-linux-gnu/security", "lib/x86_64-linux-gnu/security", "usr/lib/aarch64-linux-gnu/security", "usr/lib/security", "lib/security"].iter().map(|d| self.root.join(d)).find(|d| d.join("pam_unix.so").exists())
    }

    fn install_module(&self) -> Result<(), String> {
        let src = self.module_source();
        if !self.trusted(&src) {
            return Err("Face sign-in needs Setu installed from its package.".into());
        }
        let dir = self.module_dir().ok_or("I couldn't find where this system keeps its sign-in modules.")?;
        let bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
        let to = dir.join(MODULE);
        if std::fs::read(&to).map(|b| b == bytes).unwrap_or(false) {
            return Ok(());
        }
        write_atomic(&to, &bytes, 0o755)
    }

    fn remove_module(&self) {
        if let Some(d) = self.module_dir() {
            let _ = std::fs::remove_file(d.join(MODULE));
        }
    }

    fn load_policy(&self) -> Option<String> {
        if !self.real || !Path::new("/sys/fs/selinux/enforce").exists() {
            return None;
        }
        let pp = self.policy_file();
        if !self.trusted(&pp) {
            return Some("The SELinux rules for face sign-in aren't installed, so the sign-in screen may not reach Setu.".into());
        }
        let out = Command::new("/usr/sbin/semodule").arg("-i").arg(&pp).stdin(Stdio::null()).env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").output();
        match out {
            Ok(o) if o.status.success() => {
                if let Some(s) = self.socket.lock().unwrap().clone() {
                    let _ = Command::new("/usr/bin/chcon").args(["-t", "setu_pam_sock_t"]).arg(&s).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).env_clear().status();
                }
                None
            }
            Ok(o) => Some(format!("The SELinux rules didn't load: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("unknown error"))),
            Err(e) => Some(format!("The SELinux rules didn't load: {e}")),
        }
    }

    fn backup(&self, service: &str, text: &str) -> Result<(), String> {
        let d = self.ensure_state()?.join("pam-backup");
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let to = d.join(service);
        if to.exists() {
            return Ok(());
        }
        write_atomic(&to, text.as_bytes(), 0o600)
    }

    fn edit(&self, service: &str, mode: Option<&str>) -> Result<bool, String> {
        let etc = self.pam_file(service);
        let vendor = self.root.join("usr/lib/pam.d").join(service);
        let created = self.state().join(format!("{service}.created"));
        if std::fs::symlink_metadata(&etc).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Ok(false);
        }
        match mode {
            Some(m) => {
                let (old, fresh) = if plain_file(&etc) {
                    (std::fs::read_to_string(&etc).map_err(|e| e.to_string())?, false)
                } else if plain_file(&vendor) {
                    (std::fs::read_to_string(&vendor).map_err(|e| e.to_string())?, true)
                } else {
                    return Ok(false);
                };
                let Some(new) = add_line(&old, m) else { return Ok(false) };
                if strip_line(&new) != strip_line(&old) {
                    return Err(format!("I won't change the sign-in rules for {service}: the result didn't check out."));
                }
                if new == old && !fresh {
                    return Ok(true);
                }
                if fresh {
                    self.ensure_state()?;
                    std::fs::write(&created, b"").map_err(|e| e.to_string())?;
                } else {
                    self.backup(service, &old)?;
                }
                write_atomic(&etc, new.as_bytes(), 0o644)?;
                Ok(true)
            }
            None => {
                if !plain_file(&etc) {
                    return Ok(false);
                }
                let old = std::fs::read_to_string(&etc).map_err(|e| e.to_string())?;
                if !old.contains(MODULE) {
                    return Ok(false);
                }
                let new = strip_line(&old);
                if created.exists() && std::fs::read_to_string(&vendor).map(|v| v == new).unwrap_or(false) {
                    std::fs::remove_file(&etc).map_err(|e| e.to_string())?;
                } else {
                    write_atomic(&etc, new.as_bytes(), 0o644)?;
                }
                let _ = std::fs::remove_file(&created);
                Ok(true)
            }
        }
    }

    pub fn set(&self, owner: u32, mode: &str, on: bool) -> Result<Value, String> {
        let services = match mode {
            "signin" => SIGNIN,
            "admin" => ADMIN,
            _ => return Err("That isn't a face setting I know.".into()),
        };
        let mut note = None;
        if on {
            if !self.enrolled(owner) || !self.has_password(owner) {
                return Err("Your face and system password aren't saved for the sign-in screen yet.".into());
            }
            if !self.ready() {
                return Err("Face sign-in needs Setu installed from its package.".into());
            }
            self.install_module()?;
            note = self.load_policy();
            let mut done = 0;
            for s in services {
                if self.edit(s, Some(mode))? {
                    done += 1;
                }
            }
            if mode == "signin" && done > 0 {
                self.greeter(true);
            }
            if done == 0 {
                if !self.mode_on("signin") && !self.mode_on("admin") {
                    self.remove_module();
                }
                return Err(if mode == "signin" { "This computer's sign-in screen isn't GNOME's, so face sign-in isn't available here yet.".into() } else { "I couldn't find this system's rules for admin prompts.".into() });
            }
            eprintln!("setu-system: face {mode} turned on for user {owner}");
        } else {
            for s in services {
                self.edit(s, None)?;
            }
            if mode == "signin" {
                self.greeter(false);
            }
            if !self.mode_on("signin") && !self.mode_on("admin") {
                self.remove_module();
            }
            eprintln!("setu-system: face {mode} turned off");
        }
        let mut st = self.status(owner);
        st["note"] = json!(note);
        Ok(st)
    }

    pub fn forget(&self, owner: u32) -> Result<Value, String> {
        for s in SIGNIN.iter().chain(ADMIN) {
            self.edit(s, None)?;
        }
        self.greeter(false);
        self.remove_module();
        let d = self.state();
        for f in [format!("{owner}.tpl"), format!("{owner}.cred")] {
            let _ = std::fs::remove_file(d.join(f));
        }
        self.fails.lock().unwrap().remove(&owner);
        eprintln!("setu-system: face data removed for user {owner}");
        Ok(self.status(owner))
    }

    pub fn handle(&self, action: &str, args: &Value, owner: u32) -> Result<Value, String> {
        match action {
            "face.status" => Ok(self.status(owner)),
            "face.enroll" => self.enroll(owner, args),
            "face.signin" | "face.admin" => {
                let on = args.get("on").and_then(Value::as_bool).ok_or("on is missing")?;
                self.set(owner, action.trim_start_matches("face."), on)
            }
            "face.forget" => self.forget(owner),
            _ => Err("I don't do that.".into()),
        }
    }

    fn loginctl(&self, args: &[&str]) -> Result<String, String> {
        let prog = if self.real { PathBuf::from("/usr/bin/loginctl") } else { self.test.loginctl.clone().ok_or("no session tool in this test")? };
        let (ok, out) = run_with_input(&prog, args, b"", !self.real)?;
        if !ok {
            return Err("the session could not be read".into());
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn session_ok(&self, id: &str, user: &str) -> bool {
        self.loginctl(&["show-session", id, "-p", "Remote", "-p", "Active", "-p", "Seat", "-p", "Class", "-p", "Name", "-p", "LockedHint"]).map(|p| session_local(&p, user)).unwrap_or(false)
    }

    fn local(&self, pid: i32, uid: u32, user: &str) -> Result<(), String> {
        let cgroup = match (&self.test.cgroup, self.real) {
            (Some(c), false) => c.clone(),
            _ => std::fs::read_to_string(format!("/proc/{pid}/cgroup")).map_err(|_| "the caller could not be read".to_owned())?,
        };
        if let Some(id) = session_in(&cgroup) {
            return if self.session_ok(&id, user) { Ok(()) } else { Err("the request didn't come from this computer's own screen".into()) };
        }
        let list = self.loginctl(&["show-user", &uid.to_string(), "-p", "Sessions", "--value"])?;
        if list.split_whitespace().filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric())).any(|id| self.session_ok(id, user)) { Ok(()) } else { Err("you aren't signed in at this computer's own screen".into()) }
    }

    fn failed(&self, uid: u32) -> u32 {
        let mut f = self.fails.lock().unwrap();
        let e = f.entry(uid).or_insert((0, Instant::now()));
        if e.1.elapsed() >= FORGIVE {
            e.0 = 0;
        }
        e.0 += 1;
        e.1 = Instant::now();
        e.0
    }

    pub fn passed(&self, uid: u32) {
        self.fails.lock().unwrap().remove(&uid);
    }

    fn refuse(&self, req: &Request, pid: i32) -> Result<u32, String> {
        let uid = uid_of(&req.user).ok_or("unknown user")?;
        let allowed = if req.mode == "signin" { SIGNIN } else { ADMIN };
        if !allowed.contains(&req.service.as_str()) {
            return Err(format!("{} is not a place Setu signs in", req.service));
        }
        if !self.enrolled(uid) || !self.has_password(uid) {
            return Err("not set up for this user".into());
        }
        if !std::fs::read_to_string(self.pam_file(&req.service)).map(|t| has_line(&t, &req.mode)).unwrap_or(false) {
            return Err("turned off".into());
        }
        if req.mode == "signin" {
            let vt = req.tty.strip_prefix("/dev/tty").or_else(|| req.tty.strip_prefix("tty")).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            if !req.rhost.is_empty() || !(req.seat == "seat0" || (req.seat.is_empty() && vt)) {
                return Err("the sign-in didn't come from this computer's own screen".into());
            }
        } else {
            self.local(pid, uid, &req.user)?;
        }
        if self.fails.lock().unwrap().get(&uid).is_some_and(|(n, at)| *n >= TRIES && at.elapsed() < FORGIVE) {
            return Err("too many failed face checks; the password is needed".into());
        }
        Ok(uid)
    }

    fn look(&self, uid: u32, nod: bool, hint: &mut dyn FnMut(&str)) -> Result<String, String> {
        self.look_among(&[uid], nod, hint).map(|(v, _)| v)
    }

    fn look_among(&self, uids: &[u32], nod: bool, hint: &mut dyn FnMut(&str)) -> Result<(String, Option<usize>), String> {
        let d = self.state();
        let limit = if nod { "8" } else { "6" };
        let mut cmd = Command::new(self.checker());
        for uid in uids {
            cmd.arg("--templates").arg(d.join(format!("{uid}.tpl")));
        }
        cmd.arg("--det").arg(d.join("det.onnx")).arg("--rec").arg(d.join("rec.onnx")).args(["--limit", limit]);
        if nod {
            cmd.arg("--nod");
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        if self.real {
            cmd.env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").env("LANG", "C.UTF-8");
        }
        let mut child = cmd.spawn().map_err(|e| format!("the face check didn't start: {e}"))?;
        let out = child.stdout.take().ok_or("no output")?;
        let pid = child.id() as i32;
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d2 = done.clone();
        let secs: u64 = limit.parse().unwrap_or(6) + 6;
        std::thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                if d2.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            unsafe { libc::kill(pid, libc::SIGKILL) };
        });
        let mut verdict = Err("the face check gave no answer".to_owned());
        let mut who = None;
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(h) = v.get("hint").and_then(Value::as_str) {
                hint(h);
            } else if let Some(e) = v.get("error").and_then(Value::as_str) {
                verdict = Err(e.to_owned());
            } else if let Some(x) = v.get("verdict").and_then(Value::as_str) {
                verdict = Ok(x.to_owned());
                who = v.get("who").and_then(Value::as_u64).map(|w| w as usize).filter(|w| *w < uids.len());
            }
        }
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = child.wait();
        verdict.map(|v| (v, who))
    }

    fn candidates(&self, req: &Request) -> Result<Vec<(u32, String)>, String> {
        if !SIGNIN.contains(&req.service.as_str()) {
            return Err(format!("{} is not a place Setu signs in", req.service));
        }
        if !std::fs::read_to_string(self.pam_file(&req.service)).map(|t| has_line(&t, "signin")).unwrap_or(false) {
            return Err("turned off".into());
        }
        let vt = req.tty.strip_prefix("/dev/tty").or_else(|| req.tty.strip_prefix("tty")).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if !req.rhost.is_empty() || !(req.seat == "seat0" || (req.seat.is_empty() && vt)) {
            return Err("the sign-in didn't come from this computer's own screen".into());
        }
        if self.fails.lock().unwrap().get(&ANYONE).is_some_and(|(n, at)| *n >= TRIES && at.elapsed() < FORGIVE) {
            return Err("too many failed face checks; a user name is needed".into());
        }
        let mut out: Vec<(u32, String)> = std::fs::read_dir(self.state())
            .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".tpl")?.parse::<u32>().ok()).collect::<Vec<u32>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|uid| self.enrolled(*uid) && self.has_password(*uid))
            .filter(|uid| !self.fails.lock().unwrap().get(uid).is_some_and(|(n, at)| *n >= TRIES && at.elapsed() < FORGIVE))
            .filter_map(|uid| name_of(uid).map(|n| (uid, n)))
            .collect();
        out.sort();
        if out.is_empty() {
            return Err("nobody is set up".into());
        }
        Ok(out)
    }

    pub fn identify(&self, req: &Request, out: &mut dyn FnMut(&str)) {
        let people = match self.candidates(req) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("setu-system: face sign-in without a name not run: {e}");
                out("no\tunavailable\n");
                return;
            }
        };
        let Ok(_guard) = self.busy.try_lock() else {
            out("no\tbusy\n");
            return;
        };
        out("hint\tLooking for you.\n");
        let started = Instant::now();
        let uids: Vec<u32> = people.iter().map(|p| p.0).collect();
        let r = self.look_among(&uids, false, &mut |h| out(&format!("hint\t{}\n", h.replace(['\t', '\n'], " "))));
        let ms = started.elapsed().as_millis();
        match r {
            Ok((v, Some(i))) if v == "me" => {
                let (uid, name) = people[i].clone();
                let blob = std::fs::read(self.state().join(format!("{uid}.cred"))).unwrap_or_default();
                match self.sealer("decrypt", uid, &blob) {
                    Ok(mut pw) => {
                        if self.password_ok(&name, &pw).unwrap_or(false) {
                            self.passed(uid);
                            self.fails.lock().unwrap().remove(&ANYONE);
                            out(&format!("user\t{name}\n"));
                            let mut line = format!("ok\t{}\n", hex(&pw));
                            out(&line);
                            unsafe { line.as_bytes_mut() }.iter_mut().for_each(|b| *b = 0);
                            eprintln!("setu-system: face sign-in recognised {name} in {ms} ms");
                        } else {
                            let _ = std::fs::remove_file(self.state().join(format!("{uid}.cred")));
                            eprintln!("setu-system: {name} was recognised, but the saved password is out of date; it was removed");
                            out("no\tstale\n");
                        }
                        zero(&mut pw);
                    }
                    Err(e) => {
                        eprintln!("setu-system: {e}");
                        out("no\tsealed\n");
                    }
                }
            }
            Ok((v, _)) if matches!(v.as_str(), "notme" | "still" | "unsure") => {
                let n = self.failed(ANYONE);
                eprintln!("setu-system: face sign-in refused: {v} ({n} of {TRIES}) in {ms} ms");
                out(&format!("no\t{v}\n"));
            }
            Ok((v, _)) => out(&format!("no\t{v}\n")),
            Err(e) => {
                eprintln!("setu-system: face sign-in failed: {e}");
                out("no\terror\n");
            }
        }
    }

    fn greeter(&self, on: bool) {
        let f = self.root.join(GREETER);
        let Some(dir) = f.parent() else { return };
        if !self.root.join("etc/dconf/profile/gdm").exists() && self.real {
            return;
        }
        let changed = if on {
            let _ = std::fs::create_dir_all(dir);
            std::fs::read_to_string(&f).ok().as_deref() != Some(GREETER_TEXT) && write_atomic(&f, GREETER_TEXT.as_bytes(), 0o644).is_ok()
        } else {
            std::fs::remove_file(&f).is_ok()
        };
        if changed && self.real {
            let _ = Command::new("/usr/bin/dconf").arg("update").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").status();
        }
    }

    pub fn check(&self, req: &Request, pid: i32, out: &mut dyn FnMut(&str)) {
        let uid = match self.refuse(req, pid) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("setu-system: face check for {} ({}) not run: {e}", req.user, req.service);
                out("no\tunavailable\n");
                return;
            }
        };
        let Ok(_guard) = self.busy.try_lock() else {
            out("no\tbusy\n");
            return;
        };
        let nod = req.mode == "admin";
        out(if nod { "hint\tLook at the camera and nod to approve.\n" } else { "hint\tLooking for your face.\n" });
        let started = Instant::now();
        let verdict = self.look(uid, nod, &mut |h| out(&format!("hint\t{}\n", h.replace(['\t', '\n'], " "))));
        let ms = started.elapsed().as_millis();
        match verdict.as_deref() {
            Ok("me") => {
                let blob = std::fs::read(self.state().join(format!("{uid}.cred"))).unwrap_or_default();
                match self.sealer("decrypt", uid, &blob) {
                    Ok(mut pw) => {
                        let good = self.password_ok(&req.user, &pw).unwrap_or(false);
                        if good {
                            self.passed(uid);
                            let mut line = format!("ok\t{}\n", hex(&pw));
                            out(&line);
                            unsafe { line.as_bytes_mut() }.iter_mut().for_each(|b| *b = 0);
                            eprintln!("setu-system: face check for {} ({}) passed in {ms} ms", req.user, req.service);
                        } else {
                            let _ = std::fs::remove_file(self.state().join(format!("{uid}.cred")));
                            eprintln!("setu-system: face check for {} passed, but the saved password is out of date; it was removed", req.user);
                            out("no\tstale\n");
                        }
                        zero(&mut pw);
                    }
                    Err(e) => {
                        eprintln!("setu-system: {e}");
                        out("no\tsealed\n");
                    }
                }
            }
            Ok(v @ ("notme" | "still" | "nonod")) => {
                let n = self.failed(uid);
                eprintln!("setu-system: face check for {} ({}) refused: {v} ({n} of {TRIES}) in {ms} ms", req.user, req.service);
                out(&format!("no\t{v}\n"));
            }
            Ok(v) => {
                eprintln!("setu-system: face check for {} ({}): {v} in {ms} ms", req.user, req.service);
                out(&format!("no\t{v}\n"));
            }
            Err(e) => {
                eprintln!("setu-system: face check for {} ({}) failed: {e}", req.user, req.service);
                out("no\terror\n");
            }
        }
    }

    fn serve_one(&self, mut s: UnixStream) {
        let Some(p) = crate::unix::peer(&s) else { return };
        let me = unsafe { libc::geteuid() };
        if (self.real && p.uid != 0) || (!self.real && p.uid != me) {
            return;
        }
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let Ok(reader) = s.try_clone() else { return };
        let mut line = String::new();
        if BufReader::new(reader.take(2048)).read_line(&mut line).is_err() || !line.ends_with('\n') {
            return;
        }
        if let Some(user) = line.trim_end().strip_prefix("done\t") {
            if user_name_ok(user)
                && let Some(uid) = uid_of(user)
            {
                self.passed(uid);
            }
            return;
        }
        if let Some(req) = parse_identify(&line) {
            self.identify(&req, &mut |l| {
                let _ = s.write_all(l.as_bytes());
            });
            return;
        }
        let Some(req) = parse_request(&line) else {
            let _ = s.write_all(b"no\tunreadable\n");
            return;
        };
        self.check(&req, p.pid, &mut |l| {
            let _ = s.write_all(l.as_bytes());
        });
    }

    pub fn serve(self: &Arc<Self>, socket: &Path) -> Result<(), String> {
        if let Some(d) = socket.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(socket);
        let old = unsafe { libc::umask(0o077) };
        let l = UnixListener::bind(socket).map_err(|e| format!("{}: {e}", socket.display()));
        unsafe { libc::umask(old) };
        let l = l?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        *self.socket.lock().unwrap() = Some(socket.to_path_buf());
        let me = self.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { continue };
                let me = me.clone();
                std::thread::spawn(move || me.serve_one(s));
            }
        });
        Ok(())
    }

    pub fn purge(&self) {
        for s in SIGNIN.iter().chain(ADMIN) {
            let _ = self.edit(s, None);
        }
        self.greeter(false);
        self.remove_module();
        let _ = std::fs::remove_dir_all(self.root.join("var/lib/setu-system"));
        if self.real && Path::new("/sys/fs/selinux/enforce").exists() {
            let _ = Command::new("/usr/sbin/semodule").args(["-r", POLICY]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).env_clear().status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GDM: &str = "auth     [success=done ignore=ignore default=bad] pam_selinux_permit.so\nauth        substack      password-auth\nauth        optional      pam_gnome_keyring.so\nauth        include       postlogin\n\naccount     required      pam_nologin.so\nsession     required      pam_selinux.so close\n";
    const SUDO: &str = "#%PAM-1.0\nauth       include      system-auth\naccount    include      system-auth\npassword   include      system-auth\nsession    optional     pam_keyinit.so revoke\n";

    fn templates() -> Value {
        let mut v = vec![0.0f64; 128];
        v[0] = 1.0;
        let list = |n: usize| (0..n).map(|_| json!(v)).collect::<Vec<_>>();
        json!({ "angles": { "center": list(2), "left": list(2), "right": list(2), "up": list(2), "down": list(2) }, "learned": [], "enrolled": 1 })
    }

    #[test]
    fn our_line_goes_in_front_of_the_first_auth_rule_and_comes_out_clean() {
        let with = add_line(GDM, "signin").unwrap();
        assert!(with.starts_with("-auth       optional     pam_setu.so signin\nauth     [success=done"));
        assert_eq!(strip_line(&with), GDM);
        assert_eq!(add_line(&with, "signin").unwrap(), with, "adding twice changes nothing");
        let sudo = add_line(SUDO, "admin").unwrap();
        assert!(sudo.starts_with("#%PAM-1.0\n-auth       optional     pam_setu.so admin\nauth       include      system-auth\n"));
        assert_eq!(strip_line(&sudo), SUDO);
        assert!(has_line(&sudo, "admin") && !has_line(&sudo, "signin") && !has_line(SUDO, "admin"));
        assert_eq!(add_line("# only comments\nsession required pam_unix.so\n", "admin"), None);
        assert!(!has_line("# -auth optional pam_setu.so admin\nauth include system-auth\n", "admin"));
        assert_eq!(strip_line("# pam_setu.so is mentioned here\nauth include x\n"), "# pam_setu.so is mentioned here\nauth include x\n");
    }

    #[test]
    fn requests_are_read_strictly() {
        let r = parse_request("check\tadmin\tanoop\tsudo\tseat0\t/dev/pts/1\thost\n").unwrap();
        assert_eq!(r, Request { mode: "admin".into(), user: "anoop".into(), service: "sudo".into(), seat: "seat0".into(), tty: "/dev/pts/1".into(), rhost: "host".into() });
        assert!(parse_request("check\troot-shell\tanoop\tsudo\t\t\t\n").is_none());
        assert!(parse_request("check\tadmin\t-rf\tsudo\t\t\t\n").is_none());
        assert!(parse_request("check\tadmin\tanoop\tsudo\n").is_none());
        assert!(parse_request("status\tadmin\tanoop\tsudo\t\t\t\n").is_none());
        assert!(parse_request("check\tadmin\ta b\tsudo\t\t\t\n").is_none());
    }

    #[test]
    fn only_a_local_unlocked_session_of_the_same_user_counts() {
        let p = |remote: &str, active: &str, seat: &str, name: &str, locked: &str| format!("Remote={remote}\nActive={active}\nSeat={seat}\nClass=user\nName={name}\nLockedHint={locked}\n");
        assert!(session_local(&p("no", "yes", "seat0", "anoop", "no"), "anoop"));
        assert!(!session_local(&p("yes", "yes", "", "anoop", "no"), "anoop"));
        assert!(!session_local(&p("no", "no", "seat0", "anoop", "no"), "anoop"));
        assert!(!session_local(&p("no", "yes", "seat0", "riya", "no"), "anoop"));
        assert!(!session_local(&p("no", "yes", "seat0", "anoop", "yes"), "anoop"));
        assert!(!session_local("Remote=no\nActive=yes\nSeat=seat0\nClass=greeter\nName=anoop\n", "anoop"));
        assert!(!session_local("", "anoop"));
        assert_eq!(session_in("0::/user.slice/user-1000.slice/session-3.scope\n").as_deref(), Some("3"));
        assert_eq!(session_in("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Ptyxis.scope\n"), None);
        assert_eq!(session_in("0::/user.slice/user-1000.slice/session-c2.scope"), Some("c2".into()));
        assert_eq!(session_in("0::/system.slice/session-../../x.scope"), None);
    }

    #[test]
    fn face_data_is_checked_before_it_is_kept() {
        assert!(valid_templates(&templates()).is_ok());
        let mut t = templates();
        t["angles"]["center"][0][3] = json!(9.0);
        assert!(valid_templates(&t).is_err(), "a vector that is not unit length");
        let mut t = templates();
        t["angles"]["center"][0].as_array_mut().unwrap().pop();
        assert!(valid_templates(&t).is_err(), "a vector of the wrong size");
        let mut t = templates();
        t["angles"]["sideways"] = t["angles"]["left"].clone();
        assert!(valid_templates(&t).is_err());
        assert!(valid_templates(&json!({ "angles": {} })).is_err());
        assert!(valid_templates(&json!({ "angles": { "center": [[1.0]] } })).is_err());
    }

    #[test]
    fn model_checksums_match_the_manifest() {
        let m: Value = serde_json::from_str(include_str!("../../core/assets/manifest.json")).unwrap();
        let mut found = HashMap::new();
        fn walk(v: &Value, found: &mut HashMap<String, String>) {
            match v {
                Value::Object(o) => {
                    if let (Some(id), Some(sha)) = (o.get("id").and_then(Value::as_str), o.get("sha256").and_then(Value::as_str)) {
                        found.insert(id.to_owned(), sha.to_owned());
                    }
                    o.values().for_each(|x| walk(x, found));
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, found)),
                _ => {}
            }
        }
        walk(&m, &mut found);
        assert_eq!(found.get("face-det").map(String::as_str), Some(DET_SHA));
        assert_eq!(found.get("face-rec").map(String::as_str), Some(REC_SHA));
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn scratch() -> (tempfile::TempDir, Face) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        for d in ["etc/pam.d", "usr/lib/pam.d", "usr/lib64/security"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("etc/pam.d/gdm-password"), GDM).unwrap();
        std::fs::write(root.join("etc/pam.d/sudo"), SUDO).unwrap();
        std::fs::write(root.join("usr/lib/pam.d/polkit-1"), SUDO).unwrap();
        std::fs::write(root.join("usr/lib64/security/pam_unix.so"), b"x").unwrap();
        let tools = t.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join(MODULE), b"module bytes").unwrap();
        script(&tools, "setu-facecheck", "echo '{\"hint\":\"Look at the camera.\"}'; cat \"$SETU_TEST_VERDICT\"");
        let mut f = Face::new(&root, false, Some(tools.clone()));
        f.test = Test {
            passcheck: Some(script(&tools, "passcheck", "IFS= read -r -d '' p; [ \"$p\" = 'correct horse' ]")),
            sealer: Some(script(&tools, "sealer", "if [ \"$1\" = encrypt ]; then printf 'SEALED:'; cat; else tail -c +8; fi")),
            loginctl: Some(script(&tools, "loginctl", "if [ \"$1\" = show-user ]; then echo 3; else printf 'Remote=no\\nActive=yes\\nSeat=seat0\\nClass=user\\nName=%s\\nLockedHint=no\\n' \"$(id -un)\"; fi")),
            cgroup: Some("0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope".into()),
        };
        (t, f)
    }

    fn models(t: &Path) -> (PathBuf, PathBuf) {
        (t.join("det.onnx"), t.join("rec.onnx"))
    }

    #[test]
    fn enrolling_needs_the_right_password_and_the_shipped_models() {
        let (t, f) = scratch();
        let me = unsafe { libc::getuid() };
        let (det, rec) = models(t.path());
        std::fs::write(&det, b"not the model").unwrap();
        std::fs::write(&rec, b"not the model").unwrap();
        let args = |pw: &str| json!({ "templates": templates(), "password": pw, "det": det, "rec": rec });
        assert!(f.enroll(me, &args("wrong")).unwrap_err().contains("isn't your system password"));
        assert!(f.enroll(me, &args("correct horse")).unwrap_err().contains("aren't the ones Setu ships"));
        assert!(!f.enrolled(me) && !f.has_password(me));
        assert!(f.set(me, "signin", true).is_err(), "nothing can be turned on before enrolling");
        assert_eq!(std::fs::read_to_string(f.pam_file("gdm-password")).unwrap(), GDM);
    }

    fn enrol_fake(f: &Face, uid: u32) {
        let d = f.ensure_state().unwrap();
        std::fs::write(d.join("det.onnx"), b"d").unwrap();
        std::fs::write(d.join("rec.onnx"), b"r").unwrap();
        write_atomic(&d.join(format!("{uid}.tpl")), templates().to_string().as_bytes(), 0o600).unwrap();
        write_atomic(&d.join(format!("{uid}.cred")), &f.sealer("encrypt", uid, b"correct horse").unwrap(), 0o600).unwrap();
    }

    #[test]
    fn turning_on_and_off_edits_only_our_line() {
        let (_t, f) = scratch();
        let me = unsafe { libc::getuid() };
        enrol_fake(&f, me);
        let st = f.set(me, "signin", true).unwrap();
        assert_eq!(st["signin"], true);
        assert_eq!(st["admin"], false);
        assert!(std::fs::read_to_string(f.pam_file("gdm-password")).unwrap().starts_with("-auth       optional     pam_setu.so signin\n"));
        assert_eq!(std::fs::read(f.root.join("usr/lib64/security").join(MODULE)).unwrap(), b"module bytes");
        assert_eq!(std::fs::read_to_string(f.state().join("pam-backup/gdm-password")).unwrap(), GDM);
        f.set(me, "admin", true).unwrap();
        assert!(has_line(&std::fs::read_to_string(f.pam_file("sudo")).unwrap(), "admin"));
        assert!(has_line(&std::fs::read_to_string(f.pam_file("polkit-1")).unwrap(), "admin"), "polkit's rules are copied into /etc with our line");
        assert_eq!(std::fs::read_to_string(f.root.join("usr/lib/pam.d/polkit-1")).unwrap(), SUDO, "the vendor file is never changed");
        f.set(me, "admin", false).unwrap();
        assert_eq!(std::fs::read_to_string(f.pam_file("sudo")).unwrap(), SUDO);
        assert!(!f.pam_file("polkit-1").exists(), "the copy we made is removed again");
        assert!(f.root.join("usr/lib64/security").join(MODULE).exists(), "sign-in still uses the module");
        f.set(me, "signin", false).unwrap();
        assert_eq!(std::fs::read_to_string(f.pam_file("gdm-password")).unwrap(), GDM);
        assert!(!f.root.join("usr/lib64/security").join(MODULE).exists());
    }

    #[test]
    fn a_linked_rules_file_is_left_alone() {
        let (_t, f) = scratch();
        let me = unsafe { libc::getuid() };
        enrol_fake(&f, me);
        std::fs::remove_file(f.pam_file("gdm-password")).unwrap();
        std::os::unix::fs::symlink(f.root.join("etc/pam.d/sudo"), f.pam_file("gdm-password")).unwrap();
        assert!(f.set(me, "signin", true).is_err());
        assert_eq!(std::fs::read_to_string(f.pam_file("sudo")).unwrap(), SUDO);
    }

    fn ask(f: &Face, mode: &str, service: &str, seat: &str, rhost: &str) -> Vec<String> {
        let user = name_of(unsafe { libc::getuid() }).unwrap();
        let req = Request { mode: mode.into(), user, service: service.into(), seat: seat.into(), tty: "/dev/tty2".into(), rhost: rhost.into() };
        let mut lines = Vec::new();
        f.check(&req, std::process::id() as i32, &mut |l| lines.push(l.trim_end().to_owned()));
        lines
    }

    #[test]
    fn the_password_leaves_only_after_a_match_at_this_computer() {
        let (t, f) = scratch();
        let me = unsafe { libc::getuid() };
        let verdict = t.path().join("verdict");
        unsafe { std::env::set_var("SETU_TEST_VERDICT", &verdict) };
        std::fs::write(&verdict, "{\"verdict\":\"me\",\"score\":0.7}\n").unwrap();
        assert_eq!(ask(&f, "signin", "gdm-password", "seat0", ""), vec!["no\tunavailable"], "not enrolled");
        enrol_fake(&f, me);
        assert_eq!(ask(&f, "signin", "gdm-password", "seat0", ""), vec!["no\tunavailable"], "enrolled but not turned on");
        f.set(me, "signin", true).unwrap();
        f.set(me, "admin", true).unwrap();
        let ok = format!("ok\t{}", hex(b"correct horse"));
        assert_eq!(ask(&f, "signin", "gdm-password", "seat0", ""), vec!["hint\tLooking for your face.".to_owned(), "hint\tLook at the camera.".to_owned(), ok.clone()]);
        assert_eq!(ask(&f, "signin", "gdm-password", "", "remote.example"), vec!["no\tunavailable"], "a remote sign-in never uses the camera");
        assert_eq!(ask(&f, "signin", "gdm-password", "seat1", ""), vec!["no\tunavailable"]);
        assert_eq!(ask(&f, "signin", "sshd", "seat0", ""), vec!["no\tunavailable"], "only the services Setu set up");
        assert_eq!(ask(&f, "admin", "gdm-password", "seat0", ""), vec!["no\tunavailable"]);
        assert_eq!(ask(&f, "admin", "sudo", "", "thishost").last().unwrap(), &ok);
        std::fs::write(&verdict, "{\"verdict\":\"notme\"}\n").unwrap();
        for _ in 0..3 {
            assert_eq!(ask(&f, "admin", "sudo", "", "").last().unwrap(), "no\tnotme");
        }
        std::fs::write(&verdict, "{\"verdict\":\"me\",\"score\":0.7}\n").unwrap();
        assert_eq!(ask(&f, "admin", "sudo", "", ""), vec!["no\tunavailable"], "three refusals lock the face until a password sign-in");
        f.passed(me);
        assert_eq!(ask(&f, "admin", "sudo", "", "").last().unwrap(), &ok);
        write_atomic(&f.state().join(format!("{me}.cred")), &f.sealer("encrypt", me, b"old password").unwrap(), 0o600).unwrap();
        assert_eq!(ask(&f, "admin", "sudo", "", "").last().unwrap(), "no\tstale");
        assert!(!f.has_password(me), "an out-of-date password is thrown away, never handed out");
        f.forget(me).unwrap();
        assert_eq!(std::fs::read_to_string(f.pam_file("gdm-password")).unwrap(), GDM);
        assert!(!f.enrolled(me));
    }

    #[test]
    fn an_admin_prompt_from_a_remote_session_is_refused() {
        let (t, mut f) = scratch();
        let me = unsafe { libc::getuid() };
        let verdict = t.path().join("verdict2");
        std::fs::write(&verdict, "{\"verdict\":\"me\",\"score\":0.7}\n").unwrap();
        enrol_fake(&f, me);
        f.set(me, "admin", true).unwrap();
        f.test.cgroup = Some("0::/user.slice/user-1000.slice/session-9.scope".into());
        f.test.loginctl = Some(script(&f.tools.clone(), "loginctl-remote", "printf 'Remote=yes\\nActive=yes\\nSeat=\\nClass=user\\nName=%s\\nLockedHint=no\\n' \"$(id -un)\""));
        assert_eq!(ask(&f, "admin", "sudo", "", ""), vec!["no\tunavailable"]);
    }
}
