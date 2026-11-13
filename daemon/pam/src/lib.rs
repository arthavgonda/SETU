#![cfg(target_os = "linux")]

use libc::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const PAM_SUCCESS: c_int = 0;
const PAM_IGNORE: c_int = 25;
const PAM_SERVICE: c_int = 1;
const PAM_USER: c_int = 2;
const PAM_TTY: c_int = 3;
const PAM_RHOST: c_int = 4;
const PAM_CONV: c_int = 5;
const PAM_AUTHTOK: c_int = 6;
const PAM_TEXT_INFO: c_int = 4;
const SOCKET: &str = "/run/setu-system/pam.sock";
const WAIT: Duration = Duration::from_secs(14);
const WATCH: Duration = Duration::from_secs(600);

#[repr(C)]
pub struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: Option<unsafe extern "C" fn(c_int, *mut *const PamMessage, *mut *mut PamResponse, *mut c_void) -> c_int>,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_get_user(pamh: *mut PamHandle, user: *mut *const c_char, prompt: *const c_char) -> c_int;
    fn pam_get_item(pamh: *const PamHandle, item_type: c_int, item: *mut *const c_void) -> c_int;
    fn pam_set_item(pamh: *mut PamHandle, item_type: c_int, item: *const c_void) -> c_int;
    fn pam_getenv(pamh: *mut PamHandle, name: *const c_char) -> *const c_char;
}

#[derive(Debug, PartialEq)]
struct Args {
    mode: &'static str,
    socket: String,
    quiet: bool,
}

fn parse_args(list: &[String]) -> Args {
    let mut a = Args { mode: "signin", socket: SOCKET.to_owned(), quiet: false };
    for x in list {
        match x.as_str() {
            "signin" => a.mode = "signin",
            "admin" => a.mode = "admin",
            "quiet" => a.quiet = true,
            other => {
                if let Some(p) = other.strip_prefix("socket=")
                    && p.starts_with('/')
                {
                    a.socket = p.to_owned();
                }
            }
        }
    }
    a
}

fn clean(field: &str) -> String {
    field.chars().filter(|c| !c.is_control()).take(256).collect()
}

fn request(mode: &str, user: &str, service: &str, seat: &str, tty: &str, rhost: &str) -> String {
    format!("check\t{}\t{}\t{}\t{}\t{}\t{}\n", clean(mode), clean(user), clean(service), clean(seat), clean(tty), clean(rhost))
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.trim().as_bytes();
    if b.is_empty() || b.len() % 2 != 0 || b.len() > 2048 {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(b.len() / 2 + 1);
    for p in b.chunks(2) {
        out.push(nib(p[0])? << 4 | nib(p[1])?);
    }
    if out.contains(&0) {
        out.iter_mut().for_each(|x| *x = 0);
        return None;
    }
    Some(out)
}

#[derive(Debug, PartialEq)]
enum Reply {
    Hint(String),
    User(String),
    Pass(Vec<u8>),
    Wait,
    No,
}

fn user_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn read_reply(line: &str) -> Reply {
    let line = line.trim_end_matches(['\n', '\r']);
    match line.split_once('\t') {
        Some(("hint", text)) => Reply::Hint(clean(text)),
        Some(("user", name)) if user_ok(name) => Reply::User(name.to_owned()),
        Some(("ok", hex)) => unhex(hex).map(Reply::Pass).unwrap_or(Reply::No),
        Some(("no", "noface" | "busy")) => Reply::Wait,
        _ => Reply::No,
    }
}

#[derive(Debug, PartialEq)]
enum Answer {
    Pass(Vec<u8>, Option<String>),
    Wait,
    No,
}

fn talk(socket: &str, req: &str, hint: &mut dyn FnMut(&str)) -> Answer {
    let end = Instant::now() + WAIT;
    let Ok(mut s) = UnixStream::connect(socket) else { return Answer::No };
    if s.set_write_timeout(Some(Duration::from_secs(2))).is_err() || s.write_all(req.as_bytes()).is_err() {
        return Answer::No;
    }
    let mut r = BufReader::new(s);
    let mut who = None;
    loop {
        let Some(left) = end.checked_duration_since(Instant::now()) else { return Answer::No };
        if r.get_ref().set_read_timeout(Some(left.max(Duration::from_millis(50)))).is_err() {
            return Answer::No;
        }
        let mut line = String::new();
        let n = r.read_line(&mut line).unwrap_or(0);
        if n == 0 || line.len() > 8192 {
            return Answer::No;
        }
        let reply = read_reply(&line);
        unsafe { line.as_bytes_mut() }.iter_mut().for_each(|b| *b = 0);
        match reply {
            Reply::Hint(t) => hint(&t),
            Reply::User(u) => who = Some(u),
            Reply::Pass(p) => return Answer::Pass(p, who),
            Reply::Wait => return Answer::Wait,
            Reply::No => return Answer::No,
        }
    }
}

fn ask(socket: &str, req: &str, hint: &mut dyn FnMut(&str)) -> Option<Vec<u8>> {
    match talk(socket, req, hint) {
        Answer::Pass(p, _) => Some(p),
        _ => None,
    }
}

fn identify_request(service: &str, seat: &str, tty: &str, rhost: &str) -> String {
    format!("identify\tsignin\t{}\t{}\t{}\t{}\n", clean(service), clean(seat), clean(tty), clean(rhost))
}

unsafe fn item(pamh: *mut PamHandle, kind: c_int) -> String {
    let mut p: *const c_void = std::ptr::null();
    if unsafe { pam_get_item(pamh, kind, &mut p) } != PAM_SUCCESS || p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p as *const c_char) }.to_string_lossy().into_owned()
}

unsafe fn info(pamh: *mut PamHandle, text: &str) {
    let mut p: *const c_void = std::ptr::null();
    if unsafe { pam_get_item(pamh, PAM_CONV, &mut p) } != PAM_SUCCESS || p.is_null() {
        return;
    }
    let conv = unsafe { &*(p as *const PamConv) };
    let Some(f) = conv.conv else { return };
    let Ok(text) = CString::new(text) else { return };
    let msg = PamMessage { msg_style: PAM_TEXT_INFO, msg: text.as_ptr() };
    let mut list: *const PamMessage = &msg;
    let mut resp: *mut PamResponse = std::ptr::null_mut();
    unsafe {
        f(1, &mut list, &mut resp, conv.appdata_ptr);
        if !resp.is_null() {
            if !(*resp).resp.is_null() {
                libc::free((*resp).resp as *mut c_void);
            }
            libc::free(resp as *mut c_void);
        }
    }
}

unsafe fn seat_of(pamh: *mut PamHandle) -> String {
    let p = unsafe { pam_getenv(pamh, c"XDG_SEAT".as_ptr()) };
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

unsafe fn identify(pamh: *mut PamHandle, args: &Args) -> c_int {
    let seat = unsafe { seat_of(pamh) };
    let (service, tty, rhost) = unsafe { (item(pamh, PAM_SERVICE), item(pamh, PAM_TTY), item(pamh, PAM_RHOST)) };
    let req = identify_request(&service, &seat, &tty, &rhost);
    let quiet = args.quiet;
    let mut last = String::new();
    let mut hint = |t: &str| {
        if !quiet && !t.is_empty() && t != last {
            unsafe { info(pamh, &format!("Setu: {t}")) };
            last = t.to_owned();
        }
    };
    let end = Instant::now() + WATCH;
    while Instant::now() < end {
        match talk(&args.socket, &req, &mut hint) {
            Answer::Pass(mut pass, Some(user)) => {
                let Ok(name) = CString::new(user) else { return PAM_IGNORE };
                pass.push(0);
                let r = unsafe { pam_set_item(pamh, PAM_USER, name.as_ptr() as *const c_void) };
                let t = if r == PAM_SUCCESS { unsafe { pam_set_item(pamh, PAM_AUTHTOK, pass.as_ptr() as *const c_void) } } else { r };
                pass.iter_mut().for_each(|b| *b = 0);
                return if t == PAM_SUCCESS { PAM_SUCCESS } else { PAM_IGNORE };
            }
            Answer::Wait => std::thread::sleep(Duration::from_millis(700)),
            _ => return PAM_IGNORE,
        }
    }
    PAM_IGNORE
}

unsafe fn authenticate(pamh: *mut PamHandle, argc: c_int, argv: *const *const c_char) -> c_int {
    let args = unsafe { arguments(argc, argv) };
    let mut had: *const c_void = std::ptr::null();
    if unsafe { pam_get_item(pamh, PAM_AUTHTOK, &mut had) } == PAM_SUCCESS && !had.is_null() {
        return PAM_IGNORE;
    }
    if args.mode == "signin" && unsafe { item(pamh, PAM_USER) }.is_empty() {
        return unsafe { identify(pamh, &args) };
    }
    let mut u: *const c_char = std::ptr::null();
    if unsafe { pam_get_user(pamh, &mut u, std::ptr::null()) } != PAM_SUCCESS || u.is_null() {
        return PAM_IGNORE;
    }
    let user = unsafe { CStr::from_ptr(u) }.to_string_lossy().into_owned();
    if user.is_empty() {
        return PAM_IGNORE;
    }
    let seat = {
        let p = unsafe { pam_getenv(pamh, c"XDG_SEAT".as_ptr()) };
        if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
    };
    let (service, tty, rhost) = unsafe { (item(pamh, PAM_SERVICE), item(pamh, PAM_TTY), item(pamh, PAM_RHOST)) };
    let req = request(args.mode, &user, &service, &seat, &tty, &rhost);
    let quiet = args.quiet;
    let mut hint = |t: &str| {
        if !quiet && !t.is_empty() {
            unsafe { info(pamh, &format!("Setu: {t}")) };
        }
    };
    let Some(mut pass) = ask(&args.socket, &req, &mut hint) else { return PAM_IGNORE };
    pass.push(0);
    let r = unsafe { pam_set_item(pamh, PAM_AUTHTOK, pass.as_ptr() as *const c_void) };
    pass.iter_mut().for_each(|b| *b = 0);
    if r == PAM_SUCCESS { PAM_SUCCESS } else { PAM_IGNORE }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(pamh: *mut PamHandle, _flags: c_int, argc: c_int, argv: *const *const c_char) -> c_int {
    if pamh.is_null() {
        return PAM_IGNORE;
    }
    std::panic::catch_unwind(|| unsafe { authenticate(pamh, argc, argv) }).unwrap_or(PAM_IGNORE)
}

unsafe fn arguments(argc: c_int, argv: *const *const c_char) -> Args {
    let mut list = Vec::new();
    if !argv.is_null() {
        for i in 0..argc.max(0) as isize {
            let a = unsafe { *argv.offset(i) };
            if !a.is_null() {
                list.push(unsafe { CStr::from_ptr(a) }.to_string_lossy().into_owned());
            }
        }
    }
    parse_args(&list)
}

fn tell_done(socket: &str, user: &str) {
    let Ok(mut s) = UnixStream::connect(socket) else { return };
    let _ = s.set_write_timeout(Some(Duration::from_millis(300)));
    let _ = s.write_all(format!("done\t{}\n", clean(user)).as_bytes());
}

unsafe fn settled(pamh: *mut PamHandle, argc: c_int, argv: *const *const c_char) {
    let args = unsafe { arguments(argc, argv) };
    let mut u: *const c_char = std::ptr::null();
    if unsafe { pam_get_user(pamh, &mut u, std::ptr::null()) } != PAM_SUCCESS || u.is_null() {
        return;
    }
    let user = unsafe { CStr::from_ptr(u) }.to_string_lossy().into_owned();
    if !user.is_empty() {
        tell_done(&args.socket, &user);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_setcred(pamh: *mut PamHandle, _flags: c_int, argc: c_int, argv: *const *const c_char) -> c_int {
    if !pamh.is_null() {
        let _ = std::panic::catch_unwind(|| unsafe { settled(pamh, argc, argv) });
    }
    PAM_IGNORE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn arguments_pick_the_mode_and_socket() {
        let a = |v: &[&str]| parse_args(&v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
        assert_eq!(a(&[]), Args { mode: "signin", socket: SOCKET.into(), quiet: false });
        assert_eq!(a(&["admin", "quiet"]), Args { mode: "admin", socket: SOCKET.into(), quiet: true });
        assert_eq!(a(&["socket=/tmp/x.sock"]).socket, "/tmp/x.sock");
        assert_eq!(a(&["socket=relative"]).socket, SOCKET);
        assert_eq!(a(&["nonsense"]).mode, "signin");
    }

    #[test]
    fn requests_cannot_carry_extra_fields_or_lines() {
        let r = request("admin", "an\too\np", "sudo", "seat0", "/dev/pts/1", "");
        assert_eq!(r, "check\tadmin\tanoop\tsudo\tseat0\t/dev/pts/1\t\n");
        assert_eq!(r.matches('\n').count(), 1);
    }

    #[test]
    fn replies_are_read_strictly() {
        assert_eq!(read_reply("hint\tNod to approve.\n"), Reply::Hint("Nod to approve.".into()));
        assert_eq!(read_reply("ok\t68756e74657232\n"), Reply::Pass(b"hunter2".to_vec()));
        assert_eq!(read_reply("ok\t\n"), Reply::No);
        assert_eq!(read_reply("ok\tzz\n"), Reply::No);
        assert_eq!(read_reply("ok\t6800\n"), Reply::No);
        assert_eq!(read_reply("ok\t686\n"), Reply::No);
        assert_eq!(read_reply("no\tnot you\n"), Reply::No);
        assert_eq!(read_reply("no\tnoface\n"), Reply::Wait);
        assert_eq!(read_reply("user\tanoop\n"), Reply::User("anoop".into()));
        assert_eq!(read_reply("user\t-root\n"), Reply::No);
        assert_eq!(read_reply("user\tan oop\n"), Reply::No);
        assert_eq!(identify_request("gdm-password", "seat0", "/dev/tty1", ""), "identify\tsignin\tgdm-password\tseat0\t/dev/tty1\t\n");
        assert_eq!(read_reply("garbage"), Reply::No);
        assert_eq!(read_reply(""), Reply::No);
    }

    #[test]
    fn a_pass_comes_back_only_after_ok_and_hints_are_shown() {
        let dir = std::env::temp_dir().join(format!("setu-pam-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.sock");
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for answer in ["hint\tLook at the camera.\nok\t70617373\n", "hint\tLook at the camera.\nno\tnotme\n", ""] {
                let (mut s, _) = l.accept().unwrap();
                let mut line = String::new();
                BufReader::new(s.try_clone().unwrap()).read_line(&mut line).unwrap();
                seen.push(line);
                s.write_all(answer.as_bytes()).unwrap();
            }
            seen
        });
        let sock = path.to_string_lossy().into_owned();
        let mut hints = Vec::new();
        let req = request("signin", "anoop", "gdm-password", "seat0", "/dev/tty2", "");
        assert_eq!(ask(&sock, &req, &mut |t| hints.push(t.to_owned())), Some(b"pass".to_vec()));
        assert_eq!(ask(&sock, &req, &mut |t| hints.push(t.to_owned())), None);
        assert_eq!(ask(&sock, &req, &mut |_| {}), None);
        assert_eq!(hints, vec!["Look at the camera.", "Look at the camera."]);
        assert_eq!(server.join().unwrap()[0], req);
        assert_eq!(ask(&dir.join("missing.sock").to_string_lossy(), &req, &mut |_| {}), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
