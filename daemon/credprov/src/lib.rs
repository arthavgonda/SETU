#![cfg(windows)]
#![allow(non_snake_case)]

use serde_json::Value;
use std::ffi::c_void;
use std::io::BufRead;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_NOTIMPL, E_POINTER, HANDLE, HLOCAL, HMODULE, LocalFree, NTSTATUS, S_FALSE, S_OK};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Security::Authentication::Identity::{KERB_INTERACTIVE_UNLOCK_LOGON, KerbInteractiveLogon, KerbWorkstationUnlockLogon, LSA_STRING, LsaConnectUntrusted, LsaDeregisterLogonProcess, LsaLookupAuthenticationPackage};
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData};
use windows::Win32::Security::{LookupAccountSidW, PSID, SID_NAME_USE};
use windows::Win32::System::Com::{CoTaskMemAlloc, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW};
use windows::Win32::UI::Shell::*;
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PSTR, PWSTR, Ref, Result, implement};

pub const CLSID: GUID = GUID::from_u128(0x5e7a3c21_9b4d_4f6e_8a1c_2d3b4e5f6071);
const FIELDS: u32 = 3;
const TRIES: u32 = 3;
const NO_WINDOW: u32 = 0x0800_0000;

static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LOCKS: AtomicUsize = AtomicUsize::new(0);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn dup(s: &str) -> Result<PWSTR> {
    let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let p = CoTaskMemAlloc(w.len() * 2) as *mut u16;
        if p.is_null() {
            return Err(E_POINTER.into());
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        Ok(PWSTR(p))
    }
}

fn wipe(v: &mut Vec<u16>) {
    for c in v.iter_mut() {
        unsafe { std::ptr::write_volatile(c, 0) };
    }
    v.clear();
}

fn here() -> PathBuf {
    unsafe {
        let mut m = HMODULE::default();
        let addr = here as *const () as *const u16;
        if GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(addr), &mut m).is_err() {
            return PathBuf::new();
        }
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(m), &mut buf) as usize;
        PathBuf::from(String::from_utf16_lossy(&buf[..n])).parent().map(PathBuf::from).unwrap_or_default()
    }
}

pub fn face_dir() -> PathBuf {
    PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into())).join("Setu").join("face")
}

fn enrolled() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(face_dir())
        .map(|d| d.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".cred")).map(str::to_owned)).filter(|s| s.starts_with("S-1-5-21-") && s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S')).collect())
        .unwrap_or_default();
    out.retain(|s| face_dir().join(format!("{s}.tpl")).exists());
    out.sort();
    out
}

fn account(sid: &str) -> Option<(String, String)> {
    unsafe {
        let w: Vec<u16> = sid.encode_utf16().chain(std::iter::once(0)).collect();
        let mut psid = PSID::default();
        ConvertStringSidToSidW(PCWSTR(w.as_ptr()), &mut psid).ok()?;
        let mut name = [0u16; 256];
        let mut dom = [0u16; 256];
        let (mut nl, mut dl) = (name.len() as u32, dom.len() as u32);
        let mut use_ = SID_NAME_USE::default();
        let r = LookupAccountSidW(PCWSTR::null(), psid, Some(PWSTR(name.as_mut_ptr())), &mut nl, Some(PWSTR(dom.as_mut_ptr())), &mut dl, &mut use_);
        let _ = LocalFree(Some(HLOCAL(psid.0)));
        r.ok()?;
        Some((String::from_utf16_lossy(&name[..nl as usize]), String::from_utf16_lossy(&dom[..dl as usize])))
    }
}

fn unseal(sid: &str) -> Option<Vec<u16>> {
    let mut sealed = std::fs::read(face_dir().join(format!("{sid}.cred"))).ok()?;
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: sealed.len() as u32, pbData: sealed.as_mut_ptr() };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let bytes = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let pass: Vec<u16> = std::str::from_utf8(bytes).ok().map(|s| s.encode_utf16().collect()).unwrap_or_default();
        for b in bytes.iter_mut() {
            std::ptr::write_volatile(b, 0);
        }
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut c_void)));
        (!pass.is_empty()).then_some(pass)
    }
}

fn negotiate() -> Option<u32> {
    unsafe {
        let mut lsa = HANDLE::default();
        if LsaConnectUntrusted(&mut lsa) != NTSTATUS(0) {
            return None;
        }
        let mut name = *b"Negotiate";
        let s = LSA_STRING { Length: name.len() as u16, MaximumLength: name.len() as u16, Buffer: PSTR(name.as_mut_ptr()) };
        let mut pkg = 0u32;
        let r = LsaLookupAuthenticationPackage(lsa, &s, &mut pkg);
        let _ = LsaDeregisterLogonProcess(lsa);
        (r == NTSTATUS(0)).then_some(pkg)
    }
}

unsafe fn pack(unlock: bool, domain: &[u16], user: &[u16], pass: &[u16]) -> Option<(*mut u8, u32)> {
    let head = std::mem::size_of::<KERB_INTERACTIVE_UNLOCK_LOGON>();
    let total = head + (domain.len() + user.len() + pass.len()) * 2;
    unsafe {
        let buf = CoTaskMemAlloc(total) as *mut u8;
        if buf.is_null() {
            return None;
        }
        std::ptr::write_bytes(buf, 0, total);
        let k = &mut *(buf as *mut KERB_INTERACTIVE_UNLOCK_LOGON);
        k.Logon.MessageType = if unlock { KerbWorkstationUnlockLogon } else { KerbInteractiveLogon };
        let mut off = head;
        let mut put = |s: &[u16]| -> (u16, PWSTR) {
            std::ptr::copy_nonoverlapping(s.as_ptr() as *const u8, buf.add(off), s.len() * 2);
            let at = off;
            off += s.len() * 2;
            ((s.len() * 2) as u16, PWSTR(at as *mut u16))
        };
        let (l, b) = put(domain);
        k.Logon.LogonDomainName.Length = l;
        k.Logon.LogonDomainName.MaximumLength = l;
        k.Logon.LogonDomainName.Buffer = b;
        let (l, b) = put(user);
        k.Logon.UserName.Length = l;
        k.Logon.UserName.MaximumLength = l;
        k.Logon.UserName.Buffer = b;
        let (l, b) = put(pass);
        k.Logon.Password.Length = l;
        k.Logon.Password.MaximumLength = l;
        k.Logon.Password.Buffer = b;
        Some((buf, total as u32))
    }
}

struct Events(Option<ICredentialProviderEvents>, usize);

unsafe impl Send for Events {}

#[derive(Default)]
struct State {
    status: String,
    password: Option<Vec<u16>>,
    running: bool,
    strikes: u32,
    stopped: bool,
}

struct Shared {
    unlock: bool,
    events: Mutex<Events>,
    stop: std::sync::atomic::AtomicBool,
    watching: std::sync::atomic::AtomicBool,
}

fn changed(shared: &Shared) {
    let ev = shared.events.lock().unwrap();
    if let Events(Some(e), ctx) = &*ev {
        let _ = unsafe { e.CredentialsChanged(*ctx) };
    }
}

fn facecheck(sids: &[String], on_hint: &mut dyn FnMut(&str)) -> (String, Option<usize>) {
    let dir = face_dir();
    let mut cmd = std::process::Command::new(here().join("setu-facecheck.exe"));
    for sid in sids {
        cmd.arg("--templates").arg(dir.join(format!("{sid}.tpl")));
    }
    let run = cmd
        .arg("--det")
        .arg(dir.join("det.onnx"))
        .arg("--rec")
        .arg(dir.join("rec.onnx"))
        .args(["--limit", "8"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(NO_WINDOW)
        .spawn();
    let (mut verdict, mut who) = (String::from("error"), None);
    if let Ok(mut child) = run {
        if let Some(out) = child.stdout.take() {
            for line in std::io::BufReader::new(out).lines().map_while(std::result::Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(h) = v.get("hint").and_then(Value::as_str) {
                    on_hint(h);
                }
                if let Some(x) = v.get("verdict").and_then(Value::as_str) {
                    verdict = x.to_owned();
                    who = v.get("who").and_then(Value::as_u64).map(|w| w as usize).filter(|w| *w < sids.len());
                }
                if v.get("error").is_some() {
                    verdict = "error".into();
                }
            }
        }
        let _ = child.wait();
    }
    (verdict, who)
}

fn watch(shared: Arc<Shared>, people: Vec<(String, Arc<Mutex<State>>)>) {
    if people.is_empty() || shared.watching.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let sids: Vec<String> = people.iter().map(|p| p.0.clone()).collect();
        let end = std::time::Instant::now() + Duration::from_secs(600);
        let mut refused = 0;
        while std::time::Instant::now() < end && !shared.stop.load(Ordering::SeqCst) {
            let busy = people.iter().any(|(_, st)| {
                let g = st.lock().unwrap();
                g.running || g.password.is_some()
            });
            if busy {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            let (verdict, who) = facecheck(&sids, &mut |h| {
                for (_, st) in &people {
                    st.lock().unwrap().status = h.to_owned();
                }
            });
            match (verdict.as_str(), who) {
                ("me", Some(i)) => {
                    if let Some(p) = unseal(&people[i].0) {
                        let mut st = people[i].1.lock().unwrap();
                        st.password = Some(p);
                        st.status = "Signing you in.".into();
                        drop(st);
                        changed(&shared);
                        break;
                    }
                    break;
                }
                ("noface", _) => std::thread::sleep(Duration::from_millis(700)),
                ("error", _) => break,
                _ => {
                    refused += 1;
                    if refused >= TRIES {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1200));
                }
            }
        }
        shared.watching.store(false, Ordering::SeqCst);
    });
}

#[implement(ICredentialProviderCredential2, ICredentialProviderCredential)]
struct Credential {
    sid: String,
    shared: Arc<Shared>,
    state: Arc<Mutex<State>>,
}

impl Credential {
    fn start(&self) {
        {
            let mut st = self.state.lock().unwrap();
            if st.running || st.password.is_some() || st.stopped {
                return;
            }
            st.running = true;
            st.status = "Look at the camera.".into();
        }
        let (sid, state, shared) = (self.sid.clone(), self.state.clone(), self.shared.clone());
        std::thread::spawn(move || {
            let st2 = state.clone();
            let (verdict, _) = facecheck(std::slice::from_ref(&sid), &mut |h| st2.lock().unwrap().status = h.to_owned());
            let pass = if verdict == "me" { unseal(&sid) } else { None };
            let again = {
                let mut st = state.lock().unwrap();
                st.running = false;
                match pass {
                    Some(p) => {
                        st.password = Some(p);
                        st.status = "Signing you in.".into();
                        false
                    }
                    None => {
                        if verdict != "noface" {
                            st.strikes += 1;
                        }
                        st.stopped = st.strikes >= TRIES || verdict == "error" || verdict == "me";
                        st.status = if verdict == "me" {
                            "Setu can't open its saved password. Use your password.".into()
                        } else if st.stopped {
                            "Use your password, or choose Look again.".into()
                        } else {
                            "Still looking. Look at the camera.".into()
                        };
                        !st.stopped
                    }
                }
            };
            changed(&shared);
            if again {
                std::thread::sleep(Duration::from_millis(1200));
                let mut st = state.lock().unwrap();
                if !st.running && st.password.is_none() && !st.stopped {
                    st.status = "Look at the camera.".into();
                }
            }
        });
    }
}

impl Drop for Credential {
    fn drop(&mut self) {
        if let Some(p) = self.state.lock().unwrap().password.as_mut() {
            wipe(p);
        }
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ICredentialProviderCredential_Impl for Credential_Impl {
    fn Advise(&self, _events: Ref<'_, ICredentialProviderCredentialEvents>) -> Result<()> {
        Ok(())
    }

    fn UnAdvise(&self) -> Result<()> {
        Ok(())
    }

    fn SetSelected(&self) -> Result<BOOL> {
        self.start();
        Ok(BOOL(0))
    }

    fn SetDeselected(&self) -> Result<()> {
        Ok(())
    }

    fn GetFieldState(&self, id: u32, state: *mut CREDENTIAL_PROVIDER_FIELD_STATE, interactive: *mut CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE) -> Result<()> {
        if state.is_null() || interactive.is_null() || id >= FIELDS {
            return Err(E_INVALIDARG.into());
        }
        unsafe {
            *state = if id == 0 { CPFS_DISPLAY_IN_BOTH } else { CPFS_DISPLAY_IN_SELECTED_TILE };
            *interactive = CPFIS_NONE;
        }
        Ok(())
    }

    fn GetStringValue(&self, id: u32) -> Result<PWSTR> {
        match id {
            0 => dup("Setu face sign-in"),
            1 => dup(&self.state.lock().unwrap().status),
            2 => dup("Look again"),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetBitmapValue(&self, _id: u32) -> Result<HBITMAP> {
        Err(E_NOTIMPL.into())
    }

    fn GetCheckboxValue(&self, _id: u32, _checked: *mut BOOL, _label: *mut PWSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSubmitButtonValue(&self, _id: u32) -> Result<u32> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueCount(&self, _id: u32, _items: *mut u32, _selected: *mut u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueAt(&self, _id: u32, _item: u32) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn SetStringValue(&self, _id: u32, _value: &PCWSTR) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetCheckboxValue(&self, _id: u32, _checked: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetComboBoxSelectedValue(&self, _id: u32, _item: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn CommandLinkClicked(&self, id: u32) -> Result<()> {
        if id != 2 {
            return Err(E_INVALIDARG.into());
        }
        {
            let mut st = self.state.lock().unwrap();
            st.strikes = 0;
            st.stopped = false;
        }
        self.start();
        Ok(())
    }

    fn GetSerialization(&self, response: *mut CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE, cs: *mut CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION, _text: *mut PWSTR, _icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON) -> Result<()> {
        if response.is_null() || cs.is_null() {
            return Err(E_POINTER.into());
        }
        let taken = self.state.lock().unwrap().password.take();
        let Some(mut pass) = taken else {
            unsafe { *response = CPGSR_NO_CREDENTIAL_NOT_FINISHED };
            return Ok(());
        };
        let done = (|| {
            let (user, domain) = account(&self.sid)?;
            let pkg = negotiate()?;
            let (buf, len) = unsafe { pack(self.shared.unlock, &wide(&domain), &wide(&user), &pass) }?;
            Some((pkg, buf, len))
        })();
        wipe(&mut pass);
        match done {
            Some((pkg, buf, len)) => unsafe {
                (*cs).ulAuthenticationPackage = pkg;
                (*cs).clsidCredentialProvider = CLSID;
                (*cs).cbSerialization = len;
                (*cs).rgbSerialization = buf;
                *response = CPGSR_RETURN_CREDENTIAL_FINISHED;
            },
            None => {
                self.state.lock().unwrap().status = "Setu couldn't prepare the sign-in. Use your password.".into();
                unsafe { *response = CPGSR_NO_CREDENTIAL_NOT_FINISHED };
            }
        }
        Ok(())
    }

    fn ReportResult(&self, status: NTSTATUS, _sub: NTSTATUS, _text: *mut PWSTR, _icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON) -> Result<()> {
        if status != NTSTATUS(0) {
            let mut st = self.state.lock().unwrap();
            st.stopped = true;
            st.status = "Windows didn't accept the saved password. Sign in with your password, then turn face sign-in on again in Setu.".into();
        }
        Ok(())
    }
}

impl ICredentialProviderCredential2_Impl for Credential_Impl {
    fn GetUserSid(&self) -> Result<PWSTR> {
        dup(&self.sid)
    }
}

#[implement(ICredentialProvider, ICredentialProviderSetUserArray)]
struct Provider {
    shared: Mutex<Option<Arc<Shared>>>,
    creds: Mutex<Vec<(ICredentialProviderCredential, Arc<Mutex<State>>)>>,
    people: Mutex<Vec<(String, Arc<Mutex<State>>)>>,
}

impl Drop for Provider {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ICredentialProvider_Impl for Provider_Impl {
    fn SetUsageScenario(&self, cpus: CREDENTIAL_PROVIDER_USAGE_SCENARIO, _flags: u32) -> Result<()> {
        let unlock = match cpus {
            CPUS_LOGON => false,
            CPUS_UNLOCK_WORKSTATION => true,
            _ => return Err(E_NOTIMPL.into()),
        };
        let sids = enrolled();
        if sids.is_empty() || !here().join("setu-facecheck.exe").exists() {
            return Err(E_NOTIMPL.into());
        }
        let shared = Arc::new(Shared { unlock, events: Mutex::new(Events(None, 0)), stop: std::sync::atomic::AtomicBool::new(false), watching: std::sync::atomic::AtomicBool::new(false) });
        let mut list = Vec::new();
        let mut people = Vec::new();
        for sid in sids {
            let state = Arc::new(Mutex::new(State { status: "Look at the camera.".into(), ..Default::default() }));
            OBJECTS.fetch_add(1, Ordering::SeqCst);
            people.push((sid.clone(), state.clone()));
            let c: ICredentialProviderCredential = Credential { sid, shared: shared.clone(), state: state.clone() }.into();
            list.push((c, state));
        }
        *self.people.lock().unwrap() = people;
        *self.shared.lock().unwrap() = Some(shared);
        *self.creds.lock().unwrap() = list;
        Ok(())
    }

    fn SetSerialization(&self, _cs: *const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Advise(&self, events: Ref<'_, ICredentialProviderEvents>, context: usize) -> Result<()> {
        if let Some(s) = self.shared.lock().unwrap().as_ref() {
            *s.events.lock().unwrap() = Events(events.cloned(), context);
            s.stop.store(false, Ordering::SeqCst);
            watch(s.clone(), self.people.lock().unwrap().clone());
        }
        Ok(())
    }

    fn UnAdvise(&self) -> Result<()> {
        if let Some(s) = self.shared.lock().unwrap().as_ref() {
            s.stop.store(true, Ordering::SeqCst);
            *s.events.lock().unwrap() = Events(None, 0);
        }
        Ok(())
    }

    fn GetFieldDescriptorCount(&self) -> Result<u32> {
        Ok(FIELDS)
    }

    fn GetFieldDescriptorAt(&self, id: u32) -> Result<*mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR> {
        let (kind, label, guid) = match id {
            0 => (CPFT_LARGE_TEXT, "Setu face sign-in", CPFG_CREDENTIAL_PROVIDER_LABEL),
            1 => (CPFT_SMALL_TEXT, "Status", GUID::zeroed()),
            2 => (CPFT_COMMAND_LINK, "Look again", GUID::zeroed()),
            _ => return Err(E_INVALIDARG.into()),
        };
        unsafe {
            let d = CoTaskMemAlloc(std::mem::size_of::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>()) as *mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR;
            if d.is_null() {
                return Err(E_POINTER.into());
            }
            d.write(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR { dwFieldID: id, cpft: kind, pszLabel: dup(label)?, guidFieldType: guid });
            Ok(d)
        }
    }

    fn GetCredentialCount(&self, count: *mut u32, default: *mut u32, auto: *mut BOOL) -> Result<()> {
        if count.is_null() || default.is_null() || auto.is_null() {
            return Err(E_POINTER.into());
        }
        let creds = self.creds.lock().unwrap();
        let ready = creds.iter().position(|(_, s)| s.lock().unwrap().password.is_some());
        unsafe {
            *count = creds.len() as u32;
            *default = ready.map(|i| i as u32).unwrap_or(CREDENTIAL_PROVIDER_NO_DEFAULT);
            *auto = BOOL(ready.is_some() as i32);
        }
        Ok(())
    }

    fn GetCredentialAt(&self, index: u32) -> Result<ICredentialProviderCredential> {
        self.creds.lock().unwrap().get(index as usize).map(|(c, _)| c.clone()).ok_or_else(|| E_INVALIDARG.into())
    }
}

impl ICredentialProviderSetUserArray_Impl for Provider_Impl {
    fn SetUserArray(&self, _users: Ref<'_, ICredentialProviderUserArray>) -> Result<()> {
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, iid: *const GUID, object: *mut *mut c_void) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *object = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        OBJECTS.fetch_add(1, Ordering::SeqCst);
        let p: IUnknown = Provider { shared: Mutex::new(None), creds: Mutex::new(Vec::new()), people: Mutex::new(Vec::new()) }.into();
        unsafe { p.query(iid, object).ok() }
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::SeqCst);
        } else {
            LOCKS.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, object: *mut *mut c_void) -> HRESULT {
    if object.is_null() || clsid.is_null() || iid.is_null() {
        return E_POINTER;
    }
    unsafe {
        *object = std::ptr::null_mut();
        if *clsid != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let f: IClassFactory = Factory.into();
        f.query(iid, object)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::SeqCst) == 0 && LOCKS.load(Ordering::SeqCst) == 0 { S_OK } else { S_FALSE }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_a_logon_with_offsets_after_the_header() {
        let (d, u, p) = (wide("PC"), wide("anoop"), wide("pw"));
        let (buf, len) = unsafe { pack(false, &d, &u, &p) }.unwrap();
        let head = std::mem::size_of::<KERB_INTERACTIVE_UNLOCK_LOGON>();
        assert_eq!(len as usize, head + 18);
        let k = unsafe { &*(buf as *const KERB_INTERACTIVE_UNLOCK_LOGON) };
        assert_eq!(k.Logon.UserName.Buffer.0 as usize, head + 4);
        assert_eq!(k.Logon.Password.Length, 4);
    }
}
