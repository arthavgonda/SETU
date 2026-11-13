use serde_json::json;
use setu_face as face;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

fn value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn say(v: serde_json::Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

fn fail(msg: &str) -> ! {
    say(json!({ "error": msg }));
    std::process::exit(0)
}

#[cfg(unix)]
fn root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn root() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn drop_rights() -> Result<(), String> {
    const NOBODY: u32 = 65534;
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0 || libc::setgid(NOBODY) != 0 || libc::setuid(NOBODY) != 0 {
            return Err("the face check could not give up its administrator rights".into());
        }
        if libc::geteuid() == 0 || libc::getuid() == 0 || libc::setuid(0) == 0 {
            return Err("the face check still has administrator rights".into());
        }
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn drop_rights() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn camera() -> Result<Box<dyn face::Frames>, String> {
    let end = Instant::now() + Duration::from_millis(1500);
    loop {
        match face::Camera::open() {
            Ok(c) => return Ok(Box::new(c)),
            Err(e) if Instant::now() >= end => return Err(e.to_string()),
            Err(_) => std::thread::sleep(Duration::from_millis(150)),
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn camera() -> Result<Box<dyn face::Frames>, String> {
    face::open_frames().map_err(|e| e.to_string())
}

fn frames() -> Result<Box<dyn face::Frames>, String> {
    if root() { camera() } else { face::open_frames().map_err(|e| e.to_string()) }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let need = |flag: &str| value(&args, flag).map(PathBuf::from).unwrap_or_else(|| fail(&format!("{flag} is needed")));
    let (det, rec) = (need("--det"), need("--rec"));
    let paths: Vec<PathBuf> = args.iter().enumerate().filter(|(_, a)| *a == "--templates").filter_map(|(i, _)| args.get(i + 1)).map(PathBuf::from).collect();
    if paths.is_empty() {
        fail("--templates is needed");
    }
    let nod = args.iter().any(|a| a == "--nod");
    let limit = value(&args, "--limit").and_then(|s| s.parse::<u64>().ok()).unwrap_or(6).clamp(2, 20);
    let people: Vec<face::Templates> = paths.iter().map(|p| std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| fail("the face data could not be read"))).collect();
    if people.iter().any(|t| t.all().next().is_none()) {
        fail("no face is registered");
    }
    let t = if people.len() == 1 { people[0].clone() } else { face::Templates::merged(&people) };
    let models = match face::Models::load(&det, &rec, 2) {
        Ok(m) => m,
        Err(e) => fail(&e.to_string()),
    };
    let mut source = match frames() {
        Ok(f) => f,
        Err(e) => fail(&e),
    };
    if root()
        && let Err(e) = drop_rights()
    {
        fail(&e);
    }
    let cancel = AtomicBool::new(false);
    let mut said: Option<String> = None;
    let mut tick = |t: face::Tick| {
        let now = t.message.map(str::to_owned);
        if now != said {
            if let Some(m) = &now {
                say(json!({ "hint": m }));
            }
            said = now;
        }
    };
    let opts = face::Options { nod, no_face: Some(Duration::from_millis(2500)) };
    let started = Instant::now();
    let r = face::check_with(&models, source.as_mut(), &t, &cancel, Duration::from_secs(limit), opts, &mut tick);
    let ms = started.elapsed().as_millis() as u64;
    match r {
        Ok((face::Verdict::Me(score), v)) if people.len() > 1 => match v.as_deref().and_then(|v| face::identify(&people, v)) {
            Some(who) => say(json!({ "verdict": "me", "score": score, "who": who, "ms": ms })),
            None => say(json!({ "verdict": "unsure", "ms": ms })),
        },
        Ok((face::Verdict::Me(score), _)) => say(json!({ "verdict": "me", "score": score, "who": 0, "ms": ms })),
        Ok((v, _)) => say(json!({
            "verdict": match v {
                face::Verdict::NotMe => "notme",
                face::Verdict::NoFace => "noface",
                face::Verdict::Still => "still",
                face::Verdict::Blurry => "blurry",
                face::Verdict::NoNod => "nonod",
                _ => "cancelled",
            },
            "ms": ms
        })),
        Err(e) => say(json!({ "error": e.to_string() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_read_by_name() {
        let a: Vec<String> = ["--det", "/d", "--nod", "--limit", "9"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(value(&a, "--det").as_deref(), Some("/d"));
        assert_eq!(value(&a, "--limit").as_deref(), Some("9"));
        assert_eq!(value(&a, "--rec"), None);
    }
}
