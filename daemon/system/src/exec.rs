use crate::actions::Step;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

pub enum Mode {
    Real,
    Journal(PathBuf),
}

static JOURNAL: Mutex<()> = Mutex::new(());

pub fn describe(step: &Step) -> Value {
    match step {
        Step::Run(argv) => json!({ "run": argv }),
        Step::Write { path, text, mode } => json!({ "write": path, "bytes": text.len(), "mode": format!("{mode:o}"), "sha": short_hash(text) }),
        Step::Remove(p) => json!({ "remove": p }),
        Step::Stage { from, owner, to } => json!({ "stage": from, "owner": owner, "to": to }),
        Step::Secure(v) => json!({ "secure": v }),
    }
}

fn short_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn tail(bytes: &[u8]) -> String {
    let t = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" ").chars().take(400).collect()
}

fn stage(from: &Path, owner: u32, to: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(from).map_err(|_| "I couldn't open that file.".to_owned())?;
        let m = f.metadata().map_err(|e| e.to_string())?;
        if !m.is_file() || m.uid() != owner {
            return Err("That file doesn't belong to you.".into());
        }
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            let _ = std::fs::set_permissions(d, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        }
        let mut out = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(to).map_err(|e| e.to_string())?;
        std::io::copy(&mut (&f), &mut out).map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = owner;
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::copy(from, to).map(|_| ()).map_err(|e| e.to_string())
    }
}

fn write_file(path: &Path, text: &str, mode: u32) -> Result<(), String> {
    if let Some(d) = path.parent()
        && !path.starts_with("/sys")
    {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(!path.starts_with("/sys")).truncate(true).mode(mode).custom_flags(libc::O_NOFOLLOW).open(path).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

pub fn execute(action: &str, steps: &[Step], mode: &Mode, secure: &dyn Fn(&Value) -> Result<Value, String>) -> Result<Value, String> {
    if let Mode::Journal(path) = mode {
        let _g = JOURNAL.lock().unwrap();
        let line = json!({ "action": action, "steps": steps.iter().map(describe).collect::<Vec<_>>() }).to_string();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
        return Ok(json!({ "simulated": true }));
    }
    let mut last = Value::Null;
    for step in steps {
        match step {
            Step::Run(argv) => {
                let (prog, rest) = argv.split_first().ok_or("empty command")?;
                let out = Command::new(prog).args(rest).stdin(Stdio::null()).env_clear().env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin").env("LANG", "C.UTF-8").output().map_err(|e| format!("{prog} didn't start: {e}"))?;
                if !out.status.success() {
                    let why = tail(&out.stderr);
                    return Err(if why.is_empty() { format!("{prog} failed") } else { why });
                }
                last = json!({ "output": tail(&out.stdout) });
            }
            Step::Write { path, text, mode } => write_file(path, text, *mode)?,
            Step::Remove(p) => match std::fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            },
            Step::Stage { from, owner, to } => stage(from, *owner, to)?,
            Step::Secure(v) => last = secure(v)?,
        }
    }
    Ok(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_records_instead_of_running() {
        let t = tempfile::tempdir().unwrap();
        let j = t.path().join("journal.jsonl");
        let steps = vec![Step::Run(vec!["/bin/false".into()]), Step::Remove(t.path().join("nothing"))];
        let r = execute("service.restart", &steps, &Mode::Journal(j.clone()), &|_| Ok(Value::Null)).unwrap();
        assert_eq!(r["simulated"], true);
        let text = std::fs::read_to_string(&j).unwrap();
        assert!(text.contains("\"/bin/false\"") && text.contains("service.restart"));
    }

    #[cfg(unix)]
    #[test]
    fn staging_refuses_files_of_other_users_and_links() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.rpm");
        std::fs::write(&f, b"rpm").unwrap();
        let me = unsafe { libc::getuid() };
        assert!(stage(&f, me + 1, &t.path().join("s/x.rpm")).is_err());
        let link = t.path().join("l.rpm");
        std::os::unix::fs::symlink(&f, &link).unwrap();
        assert!(stage(&link, me, &t.path().join("s/y.rpm")).is_err());
        assert!(stage(&f, me, &t.path().join("s/z.rpm")).is_ok());
    }
}
