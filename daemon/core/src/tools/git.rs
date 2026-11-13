use super::shell::find_program;
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn git() -> Option<PathBuf> {
    let extra: Vec<PathBuf> = if cfg!(windows) {
        ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
            .iter()
            .filter_map(std::env::var_os)
            .flat_map(|b| {
                let b = PathBuf::from(b);
                [b.join("Git").join("cmd"), b.join("Programs").join("Git").join("cmd")]
            })
            .collect()
    } else {
        vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]
    };
    find_program(if cfg!(windows) { &["git.exe"] } else { &["git"] }, &extra)
}

pub fn root(dir: &Path) -> Option<PathBuf> {
    let mut d = Some(dir);
    while let Some(p) = d {
        if p.join(".git").exists() {
            return Some(p.to_path_buf());
        }
        d = p.parent();
    }
    None
}

fn run(dir: &Path, args: &[&str], secs: u64) -> Result<String> {
    let g = git().ok_or_else(|| anyhow!("Git isn't installed. Say install git and I'll set it up."))?;
    let span = crate::layers::effect(crate::layers::Layer::Channel, "channel.git");
    let (ok, out, err) = super::shell::output_quiet(&g.to_string_lossy(), args, Some(dir), Duration::from_secs(secs))?;
    span.end_bool(ok);
    if ok {
        Ok(out.trim().to_owned())
    } else {
        let e = if err.trim().is_empty() { out } else { err };
        Err(anyhow!("{}", friendly_error(e.trim())))
    }
}

pub fn friendly_error(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("authentication failed") || l.contains("could not read username") || l.contains("permission denied (publickey)") || l.contains("terminal prompts disabled") {
        return "GitHub needs you to sign in first. Say: log in to GitHub.".into();
    }
    if l.contains("nothing to commit") {
        return "There is nothing new to commit.".into();
    }
    if l.contains("no upstream branch") || l.contains("has no upstream") {
        return "This branch isn't on GitHub yet.".into();
    }
    if l.contains("rejected") && l.contains("fetch first") || l.contains("non-fast-forward") {
        return "GitHub has newer changes. Say pull first, then push again.".into();
    }
    if l.contains("not a git repository") {
        return "This folder isn't a Git project.".into();
    }
    if l.contains("could not resolve host") || l.contains("unable to access") {
        return "I couldn't reach GitHub. Check the internet connection.".into();
    }
    if l.contains("merge conflict") || l.contains("conflict") {
        return "There are merge conflicts to fix first.".into();
    }
    e.lines().rev().find(|x| !x.trim().is_empty()).unwrap_or("Git reported a problem.").trim().trim_start_matches("fatal: ").trim_start_matches("error: ").to_owned()
}

pub struct Status {
    pub branch: String,
    pub changed: Vec<String>,
    pub ahead: u32,
    pub behind: u32,
    pub upstream: bool,
}

pub fn status(dir: &Path) -> Result<Status> {
    let out = run(dir, &["status", "--porcelain=v1", "--branch"], 15)?;
    let mut st = Status { branch: String::new(), changed: Vec::new(), ahead: 0, behind: 0, upstream: false };
    for line in out.lines() {
        if let Some(b) = line.strip_prefix("## ") {
            let (head, rest) = b.split_once("...").map(|(h, r)| (h, Some(r))).unwrap_or((b, None));
            st.branch = head.trim_start_matches("No commits yet on ").to_owned();
            if let Some(r) = rest {
                st.upstream = true;
                if let Some(i) = r.find("ahead ") {
                    st.ahead = r[i + 6..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                }
                if let Some(i) = r.find("behind ") {
                    st.behind = r[i + 7..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                }
            }
        } else if line.len() > 3 {
            let path = line[3..].rsplit(" -> ").next().unwrap_or("").trim_matches('"').to_owned();
            st.changed.push(path);
        }
    }
    Ok(st)
}

pub fn describe(st: &Status) -> String {
    let files = match st.changed.len() {
        0 => "No changes".to_owned(),
        1 => format!("1 changed file, {}", short(&st.changed[0])),
        n => format!("{n} changed files"),
    };
    let sync = match (st.upstream, st.ahead, st.behind) {
        (false, _, _) => " This branch isn't on GitHub yet.".to_owned(),
        (_, 0, 0) => String::new(),
        (_, a, 0) => format!(" {a} commit{} to push.", if a == 1 { "" } else { "s" }),
        (_, 0, b) => format!(" {b} commit{} to pull.", if b == 1 { "" } else { "s" }),
        (_, a, b) => format!(" {a} to push and {b} to pull."),
    };
    format!("{files} on {}.{sync}", if st.branch.is_empty() { "this branch" } else { &st.branch })
}

fn short(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_owned())
}

pub fn auto_message(st: &Status) -> String {
    let names: Vec<String> = st.changed.iter().map(|p| short(p)).collect();
    match names.len() {
        0 => "Update".into(),
        1 => format!("Update {}", names[0]),
        2 => format!("Update {} and {}", names[0], names[1]),
        n => format!("Update {}, {} and {} more", names[0], names[1], n - 2),
    }
}

pub fn commit(dir: &Path, message: &str) -> Result<String> {
    let st = status(dir)?;
    if st.changed.is_empty() {
        return Err(anyhow!("There is nothing new to commit."));
    }
    let msg = if message.trim().is_empty() { auto_message(&st) } else { crate::text::sentence_case(message.trim()) };
    run(dir, &["add", "-A"], 60)?;
    run(dir, &["commit", "-m", &msg], 60)?;
    Ok(format!("Committed {} file{}: {msg}.", st.changed.len(), if st.changed.len() == 1 { "" } else { "s" }))
}

pub fn push(dir: &Path) -> Result<String> {
    let st = status(dir)?;
    if !st.upstream {
        let remotes = run(dir, &["remote"], 10)?;
        let remote = remotes.lines().next().map(str::to_owned).ok_or_else(|| anyhow!("This project has no GitHub link yet. Add a remote first."))?;
        run(dir, &["push", "--set-upstream", &remote, &st.branch], 120)?;
        return Ok(format!("Pushed {} to {remote}.", st.branch));
    }
    run(dir, &["push"], 120)?;
    Ok(if st.ahead > 0 { format!("Pushed {} commit{}.", st.ahead, if st.ahead == 1 { "" } else { "s" }) } else { "Pushed. Everything is up to date.".into() })
}

pub fn pull(dir: &Path) -> Result<String> {
    let out = run(dir, &["pull", "--ff-only"], 120)?;
    Ok(if out.contains("Already up to date") { "Already up to date.".into() } else { "Pulled the latest changes.".into() })
}

pub fn branch(dir: &Path, name: &str, create: bool) -> Result<String> {
    let clean: String = name.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    if clean.is_empty() {
        return Err(anyhow!("Say the branch name."));
    }
    if create {
        run(dir, &["switch", "-c", &clean], 30)?;
        Ok(format!("Created and switched to {clean}."))
    } else {
        let list = run(dir, &["branch", "--format=%(refname:short)"], 10)?;
        let pick = list.lines().map(str::trim).max_by(|a, b| crate::text::similarity(&clean, a).partial_cmp(&crate::text::similarity(&clean, b)).unwrap_or(std::cmp::Ordering::Equal));
        let target = pick.filter(|p| crate::text::similarity(&clean, p) >= 0.75).unwrap_or(&clean).to_owned();
        run(dir, &["switch", &target], 30)?;
        Ok(format!("Switched to {target}."))
    }
}

pub fn last_commit(dir: &Path) -> Result<String> {
    let out = run(dir, &["log", "-1", "--format=%s, %cr"], 10)?;
    Ok(format!("The last commit was: {out}."))
}

pub fn init(dir: &Path) -> Result<String> {
    run(dir, &["init"], 20)?;
    Ok(format!("Started a Git project in {}.", short(&dir.to_string_lossy())))
}

pub fn gh() -> Option<PathBuf> {
    let extra: Vec<PathBuf> = if cfg!(windows) {
        ["ProgramFiles", "LOCALAPPDATA"].iter().filter_map(std::env::var_os).map(|b| PathBuf::from(b).join("GitHub CLI")).collect()
    } else {
        vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]
    };
    find_program(if cfg!(windows) { &["gh.exe"] } else { &["gh"] }, &extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_errors_and_messages() {
        assert!(friendly_error("fatal: Authentication failed for 'https://github.com/x'").contains("sign in"));
        assert!(friendly_error("! [rejected] main -> main (fetch first)").contains("pull"));
        let st = Status { branch: "main".into(), changed: vec!["src/main.cpp".into(), "README.md".into()], ahead: 2, behind: 0, upstream: true };
        assert_eq!(auto_message(&st), "Update main.cpp and README.md");
        assert_eq!(describe(&st), "2 changed files on main. 2 commits to push.");
    }

    #[test]
    fn commits_in_a_new_repository() {
        if git().is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        init(d.path()).unwrap();
        let _ = run(d.path(), &["config", "user.email", "test@example.com"], 10);
        let _ = run(d.path(), &["config", "user.name", "Test"], 10);
        std::fs::write(d.path().join("hello.cpp"), "int main() {}\n").unwrap();
        let st = status(d.path()).unwrap();
        assert_eq!(st.changed, vec!["hello.cpp".to_owned()]);
        let msg = commit(d.path(), "first version").unwrap();
        assert!(msg.contains("First version"));
        assert!(commit(d.path(), "again").is_err());
    }
}
