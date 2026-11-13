use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData};
use windows::Win32::Security::{DACL_SECURITY_INFORMATION, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, LogonUserW, LookupAccountSidW, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE, SetFileSecurityW};
use windows::Win32::System::Registry::{HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, KEY_READ, RegSetValueExW};
use windows::core::{PCWSTR, PWSTR};

pub const CLSID: &str = "{5E7A3C21-9B4D-4F6E-8A1C-2D3B4E5F6071}";
const DET_SHA: &str = "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4";
const REC_SHA: &str = "0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79";
const MAX_MODEL: u64 = 96 * 1024 * 1024;
const FILES: &[&str] = &["setu_credprov.dll", "setu-facecheck.exe", "libunwind.dll", "libc++.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"];
const NEEDED: &[&str] = &["setu_credprov.dll", "setu-facecheck.exe", "onnxruntime.dll"];
const ONLY_SYSTEM: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn face_dir() -> PathBuf {
    crate::windows::data_dir().join("face")
}

pub fn program_dir() -> PathBuf {
    PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into())).join("Setu")
}

fn protect(path: &Path) -> Result<(), String> {
    unsafe {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let sddl = wide(ONLY_SYSTEM);
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None).map_err(|e| e.to_string())?;
        let p = wide(&path.to_string_lossy());
        let r = SetFileSecurityW(PCWSTR(p.as_ptr()), DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, sd);
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        r.ok().map_err(|e| format!("The face folder could not be locked down: {e}"))
    }
}

fn ensure_dir() -> Result<PathBuf, String> {
    let d = face_dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    protect(&d)?;
    Ok(d)
}

pub fn account(sid: &str) -> Option<(String, String)> {
    unsafe {
        let w = wide(sid);
        let mut psid = PSID::default();
        ConvertStringSidToSidW(PCWSTR(w.as_ptr()), &mut psid).ok()?;
        let mut name = [0u16; 256];
        let mut dom = [0u16; 256];
        let (mut nl, mut dl) = (name.len() as u32, dom.len() as u32);
        let mut kind = SID_NAME_USE::default();
        let r = LookupAccountSidW(PCWSTR::null(), psid, Some(PWSTR(name.as_mut_ptr())), &mut nl, Some(PWSTR(dom.as_mut_ptr())), &mut dl, &mut kind);
        let _ = LocalFree(Some(HLOCAL(psid.0)));
        r.ok()?;
        Some((String::from_utf16_lossy(&name[..nl as usize]), String::from_utf16_lossy(&dom[..dl as usize])))
    }
}

fn password_ok(user: &str, domain: &str, password: &str) -> bool {
    unsafe {
        let (u, d) = (wide(user), wide(domain));
        let mut p = wide(password);
        let mut token = HANDLE::default();
        let ok = LogonUserW(PCWSTR(u.as_ptr()), PCWSTR(d.as_ptr()), PCWSTR(p.as_ptr()), LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, &mut token).is_ok();
        for c in p.iter_mut() {
            std::ptr::write_volatile(c, 0);
        }
        if ok {
            let _ = CloseHandle(token);
        }
        ok
    }
}

fn seal(secret: &[u8]) -> Result<Vec<u8>, String> {
    let mut data = secret.to_vec();
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_mut_ptr() };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let r = CryptProtectData(&input, PCWSTR::null(), None, None, None, CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN, &mut out);
        for b in data.iter_mut() {
            std::ptr::write_volatile(b, 0);
        }
        r.map_err(|e| format!("The password could not be sealed: {e}"))?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
        Ok(v)
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn regular(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn stage_model(from: &Path, want: &str, to: &Path) -> Result<(), String> {
    if !regular(from) || std::fs::metadata(from).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_MODEL {
        return Err("The face models are missing.".into());
    }
    let bytes = std::fs::read(from).map_err(|e| e.to_string())?;
    if sha(&bytes) != want {
        return Err("A face model doesn't match the one Setu ships with.".into());
    }
    std::fs::write(to, bytes).map_err(|e| e.to_string())
}

pub fn valid_templates(v: &Value) -> Result<(), String> {
    let ok = v.is_object() && v.to_string().len() < 2 * 1024 * 1024 && v.as_object().is_some_and(|o| !o.is_empty());
    if ok { Ok(()) } else { Err("The face data is not valid.".into()) }
}

fn enrolled(sid: &str) -> bool {
    let d = face_dir();
    d.join(format!("{sid}.tpl")).is_file() && d.join(format!("{sid}.cred")).is_file()
}

fn anyone() -> bool {
    std::fs::read_dir(face_dir()).map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".cred"))).unwrap_or(false)
}

fn key_set(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    unsafe {
        let p = wide(path);
        let mut k = HKEY::default();
        RegCreateKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut k, None).ok().map_err(|e| e.to_string())?;
        let n = name.map(wide);
        let v = wide(value);
        let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
        let r = RegSetValueExW(k, n.as_ref().map(|n| PCWSTR(n.as_ptr())).unwrap_or(PCWSTR::null()), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(k);
        r.ok().map_err(|e| e.to_string())
    }
}

fn key_exists(path: &str) -> bool {
    unsafe {
        let p = wide(path);
        let mut k = HKEY::default();
        let ok = RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()), None, KEY_READ, &mut k).is_ok();
        if ok {
            let _ = RegCloseKey(k);
        }
        ok
    }
}

fn key_drop(path: &str) {
    unsafe {
        let p = wide(path);
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(p.as_ptr()));
    }
}

fn provider_key() -> String {
    format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\{CLSID}")
}

fn class_key() -> String {
    format!(r"SOFTWARE\Classes\CLSID\{CLSID}")
}

pub fn registered() -> bool {
    key_exists(&provider_key())
}

fn register(dll: &Path) -> Result<(), String> {
    key_set(&class_key(), None, "Setu face sign-in")?;
    key_set(&format!(r"{}\InprocServer32", class_key()), None, &dll.to_string_lossy())?;
    key_set(&format!(r"{}\InprocServer32", class_key()), Some("ThreadingModel"), "Apartment")?;
    key_set(&provider_key(), None, "Setu face sign-in")
}

pub fn unregister() {
    key_drop(&provider_key());
    key_drop(&class_key());
}

pub fn copy_files(from: &Path) -> Result<PathBuf, String> {
    let to = program_dir();
    std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
    for f in FILES {
        let src = from.join(f);
        if !regular(&src) {
            if NEEDED.contains(f) {
                return Err(format!("{f} is missing from Setu's folder. Reinstall Setu, then try again."));
            }
            continue;
        }
        let bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
        let dst = to.join(f);
        if std::fs::read(&dst).map(|b| b == bytes).unwrap_or(false) {
            continue;
        }
        let tmp = dst.with_extension("new");
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        if std::fs::rename(&tmp, &dst).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{f} is in use. Sign out and back in, then try again."));
        }
    }
    Ok(to.join("setu_credprov.dll"))
}

pub fn status(sid: &str) -> Value {
    json!({ "enrolled": enrolled(sid), "signin": enrolled(sid) && registered(), "admin": false, "seal": "dpapi", "platform": "windows" })
}

pub fn enroll(sid: &str, args: &Value) -> Result<Value, String> {
    let (user, domain) = account(sid).ok_or("I couldn't find your user account.")?;
    let t = args.get("templates").ok_or("The face data is missing.")?;
    valid_templates(t)?;
    let password = args.get("password").and_then(Value::as_str).filter(|p| !p.is_empty() && p.len() <= 512 && !p.contains('\0')).ok_or("The system password is missing.")?;
    let path = |k: &str| args.get(k).and_then(Value::as_str).map(PathBuf::from).ok_or_else(|| "The face models are missing.".to_owned());
    let (det, rec) = (path("det")?, path("rec")?);
    if !password_ok(&user, &domain, password) {
        return Err("Windows didn't accept that password. Save the right one in the vault first.".into());
    }
    let dir = ensure_dir()?;
    stage_model(&det, DET_SHA, &dir.join("det.onnx"))?;
    stage_model(&rec, REC_SHA, &dir.join("rec.onnx"))?;
    let blob = seal(password.as_bytes())?;
    std::fs::write(dir.join(format!("{sid}.tpl")), t.to_string()).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{sid}.cred")), blob).map_err(|e| e.to_string())?;
    Ok(status(sid))
}

fn installed_files() -> Option<PathBuf> {
    let d = program_dir();
    NEEDED.iter().all(|f| regular(&d.join(f))).then(|| d.join("setu_credprov.dll"))
}

pub fn set_signin(sid: &str, on: bool) -> Result<Value, String> {
    if !on {
        let _ = std::fs::remove_file(face_dir().join(format!("{sid}.cred")));
        if !anyone() {
            unregister();
        }
        return Ok(status(sid));
    }
    if !enrolled(sid) {
        return Err("Set up face sign-in from Setu first, so your face and password are saved here.".into());
    }
    let dll = installed_files().ok_or("Face sign-in needs the admin setup from this version of Setu. Say set up admin control, approve it, then try again.")?;
    register(&dll)?;
    Ok(json!({ "enrolled": true, "signin": true, "admin": false, "seal": "dpapi", "note": "At the sign-in screen, choose Setu face sign-in under Sign-in options if it isn't already chosen." }))
}

pub fn forget(sid: &str) -> Value {
    for ext in ["tpl", "cred"] {
        let _ = std::fs::remove_file(face_dir().join(format!("{sid}.{ext}")));
    }
    if !anyone() {
        unregister();
        let _ = std::fs::remove_dir_all(face_dir());
    }
    status(sid)
}

pub fn remove_all() {
    unregister();
    let _ = std::fs::remove_dir_all(face_dir());
}

pub fn handle(action: &str, args: &Value, sid: &str) -> Result<Value, String> {
    match action {
        "face.status" => Ok(status(sid)),
        "face.enroll" => enroll(sid, args),
        "face.signin" => set_signin(sid, args.get("on").and_then(Value::as_bool).unwrap_or(true)),
        "face.admin" => Err("On Windows, admin prompts go through the secure desktop helper, not the face check.".into()),
        "face.forget" => Ok(forget(sid)),
        _ => Err("That face action isn't known.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_files_are_fixed() {
        assert!(provider_key().ends_with(CLSID));
        assert!(class_key().starts_with(r"SOFTWARE\Classes\CLSID\"));
        assert!(valid_templates(&json!({ "center": [[0.1]] })).is_ok());
        assert!(valid_templates(&json!([])).is_err());
        assert!(FILES.contains(&"setu_credprov.dll") && NEEDED.iter().all(|n| FILES.contains(n)));
    }
}
