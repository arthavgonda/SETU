use anyhow::{Result, anyhow};

#[cfg(unix)]
pub fn user() -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() || pw.pw_name.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

#[cfg(windows)]
pub fn user() -> Option<String> {
    std::env::var("USERNAME").ok().filter(|s| !s.is_empty())
}

#[cfg(unix)]
pub fn real_name() -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if r != 0 || out.is_null() || pw.pw_gecos.is_null() {
        return None;
    }
    let g = unsafe { std::ffi::CStr::from_ptr(pw.pw_gecos) }.to_string_lossy().into_owned();
    g.split(',').next().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

#[cfg(windows)]
pub fn real_name() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn checker() -> Option<std::path::PathBuf> {
    if std::env::var_os("SETU_BENCH").is_some()
        && let Some(p) = std::env::var_os("SETU_PASSCHECK")
    {
        return Some(p.into());
    }
    ["/usr/sbin/unix_chkpwd", "/sbin/unix_chkpwd", "/usr/bin/unix_chkpwd"].iter().map(std::path::PathBuf::from).find(|p| p.exists())
}

#[cfg(target_os = "linux")]
fn check_blocking(user: &str, password: &str) -> Result<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let exe = checker().ok_or_else(|| anyhow!("This system has no password checker that Setu can use."))?;
    let mut child = Command::new(exe).args([user, "nonull"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let mut input = Vec::with_capacity(password.len() + 1);
    input.extend_from_slice(password.as_bytes());
    input.push(0);
    if let Some(mut s) = child.stdin.take() {
        let _ = s.write_all(&input);
    }
    input.iter_mut().for_each(|b| *b = 0);
    Ok(child.wait()?.success())
}

#[cfg(windows)]
fn check_blocking(user: &str, password: &str) -> Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, LogonUserW};
    use windows::core::HSTRING;
    let mut token = HANDLE::default();
    let r = unsafe { LogonUserW(&HSTRING::from(user), &HSTRING::from("."), &HSTRING::from(password), LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, &mut token) };
    if r.is_ok() {
        unsafe {
            let _ = CloseHandle(token);
        }
    }
    Ok(r.is_ok())
}

#[cfg(target_os = "macos")]
mod pam {
    use std::ffi::{CString, c_char, c_int, c_void};

    #[repr(C)]
    struct Message {
        style: c_int,
        msg: *const c_char,
    }

    #[repr(C)]
    struct Response {
        resp: *mut c_char,
        code: c_int,
    }

    #[repr(C)]
    struct Conv {
        conv: extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int,
        data: *mut c_void,
    }

    #[link(name = "pam")]
    unsafe extern "C" {
        fn pam_start(service: *const c_char, user: *const c_char, conv: *const Conv, h: *mut *mut c_void) -> c_int;
        fn pam_authenticate(h: *mut c_void, flags: c_int) -> c_int;
        fn pam_end(h: *mut c_void, status: c_int) -> c_int;
    }

    extern "C" fn answer(n: c_int, msg: *mut *const Message, resp: *mut *mut Response, data: *mut c_void) -> c_int {
        if n <= 0 || msg.is_null() || resp.is_null() {
            return 19;
        }
        unsafe {
            let out = libc::calloc(n as usize, std::mem::size_of::<Response>()) as *mut Response;
            if out.is_null() {
                return 5;
            }
            for i in 0..n as usize {
                let m = *msg.add(i);
                if !m.is_null() && (*m).style == 1 {
                    (*out.add(i)).resp = libc::strdup(data as *const c_char);
                }
            }
            *resp = out;
        }
        0
    }

    pub fn check(user: &str, password: &str) -> bool {
        let (Ok(svc), Ok(u), Ok(p)) = (CString::new("checkpw"), CString::new(user), CString::new(password)) else { return false };
        let conv = Conv { conv: answer, data: p.as_ptr() as *mut c_void };
        let mut h: *mut c_void = std::ptr::null_mut();
        unsafe {
            if pam_start(svc.as_ptr(), u.as_ptr(), &conv, &mut h) != 0 {
                return false;
            }
            let r = pam_authenticate(h, 0);
            pam_end(h, r);
            r == 0
        }
    }
}

#[cfg(target_os = "macos")]
fn check_blocking(user: &str, password: &str) -> Result<bool> {
    Ok(pam::check(user, password))
}

pub async fn check(password: &str) -> Result<bool> {
    let user = user().ok_or_else(|| anyhow!("I couldn't tell which user is signed in."))?;
    let p = password.to_owned();
    let span = crate::layers::effect(crate::layers::Layer::System, "system.password_check");
    let r = crate::layers::blocking(move || check_blocking(&user, &p)).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
    span.end(r)
}
