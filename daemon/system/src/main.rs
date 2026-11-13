mod actions;
mod autoconfig;
mod exec;
#[cfg(target_os = "linux")]
mod face;
mod install;
mod proto;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod winface;
#[cfg(windows)]
mod winsetup;

use exec::Mode;
use std::path::PathBuf;

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn fail(msg: &str) -> ! {
    eprintln!("setu-system: {msg}");
    std::process::exit(1)
}

fn write_autoconfig(dir: &str) -> Result<(), String> {
    let d = actions::firefox_dir(dir)?;
    let steps = actions::plan("firefox.autoconfig.install", &serde_json::json!({ "dir": d }), &actions::Host::detect(), false)?;
    exec::execute("firefox.autoconfig.install", &steps, &Mode::Real, &|_| Err("no secure desktop".into())).map(|_| ())
}

#[cfg(target_os = "linux")]
fn face_api(args: &[String], root: &std::path::Path, socket: &std::path::Path, real: bool) -> unix::Extra {
    let scratch = root != std::path::Path::new("/");
    if !real && !scratch {
        return std::sync::Arc::new(|action: &str, _: &serde_json::Value, _: u32| action.starts_with("face.").then(|| Err("Face sign-in isn't available in this test mode.".to_owned())));
    }
    let f = std::sync::Arc::new(face::Face::new(root, real, value(args, "--tools").map(PathBuf::from)));
    let pam = socket.parent().map(|d| d.join("pam.sock")).unwrap_or_else(|| PathBuf::from("/run/setu-system/pam.sock"));
    if let Err(e) = f.serve(&pam) {
        eprintln!("setu-system: the sign-in socket could not be opened: {e}");
    }
    std::sync::Arc::new(move |action: &str, a: &serde_json::Value, owner: u32| action.starts_with("face.").then(|| f.handle(action, a, owner)))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn face_api(_args: &[String], _root: &std::path::Path, _socket: &std::path::Path, _real: bool) -> unix::Extra {
    std::sync::Arc::new(|_: &str, _: &serde_json::Value, _: u32| None)
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(value(&args, "--root").unwrap_or_else(|| "/".into()));
    if has(&args, "--install") {
        let uid: u32 = value(&args, "--user").and_then(|u| u.parse().ok()).unwrap_or_else(|| fail("--user is needed"));
        let daemon = PathBuf::from(value(&args, "--daemon").unwrap_or_else(|| fail("--daemon is needed")));
        match install::install(&root, uid, &daemon) {
            Ok(m) => println!("{m}"),
            Err(e) => fail(&e),
        }
        return;
    }
    if has(&args, "--uninstall") {
        #[cfg(target_os = "linux")]
        {
            let real = root == std::path::Path::new("/");
            if !real || unsafe { libc::geteuid() } == 0 {
                face::Face::new(&root, real, None).purge();
            }
        }
        match install::uninstall(&root) {
            Ok(m) => println!("{m}"),
            Err(e) => fail(&e),
        }
        return;
    }
    if let Some(dir) = value(&args, "--write-autoconfig") {
        if let Err(e) = write_autoconfig(&dir) {
            fail(&e);
        }
        return;
    }
    if has(&args, "--serve") {
        let journal = value(&args, "--journal").map(PathBuf::from);
        let root_user = unsafe { libc::geteuid() } == 0;
        let mode = match journal {
            Some(j) => Mode::Journal(j),
            None if root_user => Mode::Real,
            None => fail("run as root, or with --journal to only record actions"),
        };
        let (uid, daemon) = match value(&args, "--allow").and_then(|u| u.parse::<u32>().ok()) {
            Some(uid) => (uid, value(&args, "--daemon").map(PathBuf::from)),
            None => install::read_config(&root).unwrap_or_else(|| fail("not set up: no allowed user")),
        };
        let socket = PathBuf::from(value(&args, "--socket").unwrap_or_else(|| install::SOCKET.into()));
        let extra = face_api(&args, &root, &socket, matches!(mode, Mode::Real));
        if let Err(e) = unix::serve(&socket, unix::Config { uid, daemon }, mode, actions::Host::detect(), extra) {
            fail(&e);
        }
        return;
    }
    fail("use --serve, --install or --uninstall");
}

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if has(&args, "--helper") {
        std::process::exit(windows::helper());
    }
    if has(&args, "--service") {
        if let Err(e) = windows::run_service() {
            fail(&e);
        }
        return;
    }
    if has(&args, "--install") {
        let sid = value(&args, "--user").unwrap_or_else(|| fail("--user is needed"));
        let daemon = PathBuf::from(value(&args, "--daemon").unwrap_or_else(|| fail("--daemon is needed")));
        match windows::install(&sid, &daemon) {
            Ok(m) => println!("{m}"),
            Err(e) => fail(&e),
        }
        return;
    }
    if has(&args, "--uninstall") {
        match windows::uninstall() {
            Ok(m) => println!("{m}"),
            Err(e) => fail(&e),
        }
        return;
    }
    if let Some(dir) = value(&args, "--write-autoconfig") {
        if let Err(e) = write_autoconfig(&dir) {
            fail(&e);
        }
        return;
    }
    if has(&args, "--serve") {
        let mode = value(&args, "--journal").map(|j| Mode::Journal(PathBuf::from(j))).unwrap_or(Mode::Real);
        if let Err(e) = windows::serve(mode) {
            fail(&e);
        }
        return;
    }
    fail("use --service, --install or --uninstall");
}
