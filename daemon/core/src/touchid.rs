#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CString, c_char, c_int, c_long, c_ulong, c_void};
    use std::sync::mpsc::{Sender, channel};
    use std::time::Duration;

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        static _NSConcreteStackBlock: c_void;
    }

    #[link(name = "LocalAuthentication", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    const BIOMETRICS: c_long = 1;

    #[repr(C)]
    struct Descriptor {
        reserved: c_ulong,
        size: c_ulong,
    }

    #[repr(C)]
    struct Block {
        isa: *const c_void,
        flags: c_int,
        reserved: c_int,
        invoke: unsafe extern "C" fn(*mut Block, bool, Id),
        descriptor: *const Descriptor,
        tx: *const Sender<bool>,
    }

    static DESCRIPTOR: Descriptor = Descriptor { reserved: 0, size: std::mem::size_of::<Block>() as c_ulong };

    unsafe extern "C" fn reply(block: *mut Block, ok: bool, _error: Id) {
        unsafe {
            let tx = (*block).tx;
            if !tx.is_null() {
                let _ = (*tx).send(ok);
            }
        }
    }

    fn sel(name: &str) -> Sel {
        let c = CString::new(name).unwrap_or_default();
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn class(name: &str) -> Id {
        let c = CString::new(name).unwrap_or_default();
        unsafe { objc_getClass(c.as_ptr()) }
    }

    fn send0(obj: Id, s: &str) -> Id {
        let f: unsafe extern "C" fn(Id, Sel) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { f(obj, sel(s)) }
    }

    pub fn verify(why: &str) -> Option<bool> {
        let cls = class("LAContext");
        if cls.is_null() {
            return None;
        }
        let ctx = send0(send0(cls, "alloc"), "init");
        if ctx.is_null() {
            return None;
        }
        let can: unsafe extern "C" fn(Id, Sel, c_long, *mut Id) -> bool = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let mut err: Id = std::ptr::null_mut();
        if !unsafe { can(ctx, sel("canEvaluatePolicy:error:"), BIOMETRICS, &mut err) } {
            send0(ctx, "release");
            return None;
        }
        let text = CString::new(why.replace('\0', "")).unwrap_or_default();
        let string: unsafe extern "C" fn(Id, Sel, *const c_char) -> Id = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let reason = unsafe { string(class("NSString"), sel("stringWithUTF8String:"), text.as_ptr()) };
        let (tx, rx) = channel::<bool>();
        let tx = Box::new(tx);
        let mut block = Block { isa: unsafe { &_NSConcreteStackBlock as *const c_void }, flags: 0, reserved: 0, invoke: reply, descriptor: &DESCRIPTOR, tx: &*tx };
        let evaluate: unsafe extern "C" fn(Id, Sel, c_long, Id, *mut Block) = unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { evaluate(ctx, sel("evaluatePolicy:localizedReason:reply:"), BIOMETRICS, reason, &mut block) };
        let r = rx.recv_timeout(Duration::from_secs(60)).ok();
        let answered = r.is_some() || {
            send0(ctx, "invalidate");
            rx.recv_timeout(Duration::from_secs(2)).is_ok()
        };
        send0(ctx, "release");
        if answered {
            drop(tx);
        } else {
            std::mem::forget(tx);
        }
        r
    }
}

#[cfg(target_os = "macos")]
pub async fn verify(why: &str) -> Option<bool> {
    let why = format!("Setu: {why}");
    crate::layers::blocking(move || imp::verify(&why)).await.ok().flatten()
}
