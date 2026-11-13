use crate::text::{normalize, similarity};
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone)]
pub struct Drive {
    pub path: PathBuf,
    pub label: String,
    pub removable: bool,
    pub total: u64,
    pub free: u64,
}

pub fn drives() -> Vec<Drive> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Drive> = Vec::new();
    for d in disks.list() {
        let mount = d.mount_point().to_path_buf();
        let m = mount.to_string_lossy().to_string();
        #[cfg(target_os = "linux")]
        {
            let keep = m == "/" || m.starts_with("/media/") || m.starts_with("/run/media/") || m.starts_with("/mnt/") || m == "/home" || d.is_removable();
            if !keep || m.starts_with("/boot") || m.starts_with("/snap") || m.contains("/flatpak") {
                continue;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if !(m == "/" || m.starts_with("/Volumes/")) || m.starts_with("/System/Volumes") {
                continue;
            }
        }
        if out.iter().any(|o| o.path == mount || !cfg!(windows) && o.total == d.total_space() && o.free.abs_diff(d.available_space()) < 64 * 1024 * 1024 && o.removable == d.is_removable()) {
            continue;
        }
        let name = d.name().to_string_lossy().to_string();
        out.push(Drive { label: if name.is_empty() { m.clone() } else { name }, path: mount, removable: d.is_removable(), total: d.total_space(), free: d.available_space() });
    }
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = (b'A' + i as u8) as char;
            let root = PathBuf::from(format!("{letter}:\\"));
            let w: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
            let kind = unsafe { GetDriveTypeW(windows::core::PCWSTR(w.as_ptr())) };
            if kind <= 1 || kind == 5 {
                continue;
            }
            match out.iter_mut().find(|o| o.path.to_string_lossy().to_uppercase().starts_with(&format!("{letter}:"))) {
                Some(o) => o.removable |= kind == 2,
                None => out.push(Drive { path: root, label: format!("{letter} drive"), removable: kind == 2, total: 0, free: 0 }),
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn drive_name(d: &Drive) -> String {
    #[cfg(windows)]
    {
        let s = d.path.to_string_lossy();
        if let Some(c) = s.chars().next().filter(|c| c.is_ascii_alphabetic()) {
            let label = if d.label.is_empty() || d.label.ends_with(" drive") { String::new() } else { format!(" ({})", d.label) };
            return format!("{} drive{label}", c.to_ascii_uppercase());
        }
    }
    if d.path == Path::new("/") {
        return "the main disk".into();
    }
    if d.removable {
        return format!("the {} drive", d.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "USB".into()));
    }
    d.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| d.label.clone())
}

pub fn size(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    if gb >= 1.0 { format!("{gb:.0} gigabytes") } else { format!("{:.0} megabytes", bytes as f64 / 1_048_576.0) }
}

pub fn letter(t: &str) -> Option<char> {
    let words: Vec<&str> = t.split_whitespace().collect();
    let map = |w: &str| -> Option<char> {
        match w {
            "see" | "sea" | "c" => Some('c'),
            "d" | "dee" | "the" => Some('d'),
            "e" | "ee" => Some('e'),
            "f" | "ef" => Some('f'),
            "g" | "gee" => Some('g'),
            "h" => Some('h'),
            _ if w.len() == 1 && w.chars().all(|c| c.is_ascii_alphabetic()) => w.chars().next(),
            _ => None,
        }
    };
    for (i, w) in words.iter().enumerate() {
        if matches!(*w, "drive" | "disk" | "disc" | "partition") {
            if i > 0
                && let Some(c) = map(words[i - 1])
            {
                return Some(c);
            }
            if let Some(n) = words.get(i + 1)
                && let Some(c) = map(n)
            {
                return Some(c);
            }
        }
    }
    None
}

fn user_dir(found: Option<PathBuf>, name: &str) -> Option<PathBuf> {
    found.or_else(|| dirs::home_dir().map(|h| h.join(name)).filter(|p| p.is_dir()))
}

pub fn known_folder(spoken: &str) -> Option<PathBuf> {
    let t = normalize(spoken);
    let t = t.trim_start_matches("the ").trim_start_matches("my ").trim_end_matches(" folder").trim();
    Some(match t {
        "downloads" | "download" | "download folder" => user_dir(dirs::download_dir(), "Downloads")?,
        "documents" | "document" | "docs" => user_dir(dirs::document_dir(), "Documents")?,
        "desktop" => user_dir(dirs::desktop_dir(), "Desktop")?,
        "pictures" | "photos" | "images" => user_dir(dirs::picture_dir(), "Pictures")?,
        "music" | "songs" => user_dir(dirs::audio_dir(), "Music")?,
        "videos" | "movies" => user_dir(dirs::video_dir(), "Videos")?,
        "home" | "home folder" | "user folder" | "profile" => dirs::home_dir()?,
        _ => return None,
    })
}

pub fn location(spoken: &str) -> Option<PathBuf> {
    let t = normalize(spoken);
    if let Some(p) = known_folder(&t) {
        return Some(p);
    }
    let removable = ["flash drive", "pen drive", "pendrive", "usb", "usb drive", "external drive", "external disk", "sd card", "memory card", "thumb drive", "hard disk", "external hard disk"];
    if removable.iter().any(|w| t.contains(w)) {
        return drives().into_iter().find(|d| d.removable).map(|d| d.path);
    }
    if let Some(c) = letter(&t) {
        #[cfg(windows)]
        {
            let p = PathBuf::from(format!("{}:\\", c.to_ascii_uppercase()));
            return p.exists().then_some(p);
        }
        #[cfg(not(windows))]
        {
            let ds = drives();
            if c == 'c' {
                return ds.iter().find(|d| d.path == Path::new("/")).map(|d| d.path.clone());
            }
            let idx = (c as u8).saturating_sub(b'c') as usize;
            return ds.get(idx).map(|d| d.path.clone());
        }
    }
    let p = PathBuf::from(spoken.trim());
    if p.is_absolute() && p.exists() {
        return Some(p);
    }
    None
}

fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "AppData" | "Windows" | "$Recycle.Bin" | "System Volume Information" | "Program Files" | "Program Files (x86)" | "ProgramData" | "proc" | "sys" | "dev" | "run" | "usr" | "lib" | "lib64" | "bin" | "sbin" | "etc" | "var" | "boot" | "snap" | "Library" | "__pycache__" | "venv" | ".venv" | "build" | "dist" | "obj"
        )
}

pub fn roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for p in [dirs::desktop_dir(), dirs::download_dir(), dirs::document_dir(), dirs::picture_dir(), dirs::video_dir(), dirs::audio_dir(), dirs::home_dir()].into_iter().flatten() {
        if !v.contains(&p) {
            v.push(p);
        }
    }
    for d in mounted() {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

static MOUNTS: std::sync::Mutex<Option<(Instant, Vec<PathBuf>)>> = std::sync::Mutex::new(None);

fn mounted() -> Vec<PathBuf> {
    if let Some((at, v)) = MOUNTS.lock().unwrap().as_ref()
        && at.elapsed() < Duration::from_secs(10)
    {
        return v.clone();
    }
    let v: Vec<PathBuf> = drives().into_iter().map(|d| d.path).collect();
    *MOUNTS.lock().unwrap() = Some((Instant::now(), v.clone()));
    v
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub path: PathBuf,
    pub score: f64,
    pub modified: SystemTime,
}

fn stem_of(p: &Path, dir: bool) -> String {
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if dir { name } else { stem }
}

#[cfg(test)]
fn score(query: &str, p: &Path) -> f64 {
    score_as(query, p, p.is_dir())
}

fn score_as(query: &str, p: &Path, dir: bool) -> f64 {
    let q = normalize(query);
    let stem = normalize(&stem_of(p, dir).replace(['_', '-', '.'], " "));
    if stem.is_empty() {
        return 0.0;
    }
    if stem == q {
        return 1.2;
    }
    if !dir && p.extension().is_some() {
        let full = normalize(&p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        if full == q || full == q.replace(" dot ", " ") {
            return 1.2;
        }
    }
    let mut s = similarity(&q, &stem);
    if stem.contains(&q) && q.len() >= 3 {
        s = s.max(0.9 + 0.1 * q.len() as f64 / stem.len() as f64);
    }
    if q.split_whitespace().all(|w| stem.contains(w)) && q.len() >= 3 {
        s = s.max(0.88);
    }
    s
}

fn home_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for p in [dirs::desktop_dir(), dirs::download_dir(), dirs::document_dir(), dirs::picture_dir(), dirs::video_dir(), dirs::audio_dir(), dirs::home_dir()].into_iter().flatten() {
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

fn exact(hits: &[Hit]) -> bool {
    hits.iter().any(|h| h.score >= 1.0)
}

fn walk(query: &str, want_dir: Option<bool>, roots: Vec<PathBuf>, deepest: u32, until: Instant, hits: &mut Vec<Hit>) {
    let mut queue: std::collections::VecDeque<(PathBuf, u32)> = roots.into_iter().map(|r| (r, 0)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut visited = 0usize;
    let mut found: Option<u32> = hits.iter().any(|h| h.score >= 1.0).then_some(0);
    while let Some((dir, depth)) = queue.pop_front() {
        if Instant::now() > until || visited > 200_000 || found.is_some_and(|d| depth > d) {
            break;
        }
        if !seen.insert(dir.clone()) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            visited += 1;
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                if skip_dir(&name) {
                    continue;
                }
                if depth < deepest {
                    queue.push_back((p.clone(), depth + 1));
                }
            }
            if ft.is_symlink() {
                continue;
            }
            let dir = ft.is_dir();
            if want_dir.is_some_and(|w| w != dir) {
                continue;
            }
            let s = score_as(query, &p, dir);
            if s >= 0.8 && !hits.iter().any(|h| h.path == p) {
                if s >= 1.0 && found.is_none() {
                    found = Some(depth);
                }
                let modified = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                hits.push(Hit { path: p, score: s, modified });
            }
        }
    }
}

fn located(query: &str, want_dir: Option<bool>, hits: &mut Vec<Hit>) {
    #[cfg(target_os = "linux")]
    if let Some(locate) = super::shell::find_program(&["plocate", "locate"], &[]) {
        let span = crate::layers::read(crate::layers::Layer::Command, "command.locate");
        let Ok((true, out, _)) = span.end(super::shell::output(&locate.to_string_lossy(), &["-i", "-l", "400", &normalize(query).replace(' ', "*")], None, Duration::from_secs(2))) else { return };
        for line in out.lines() {
            let p = PathBuf::from(line);
            if p.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
                continue;
            }
            let dir = p.is_dir();
            if want_dir.is_some_and(|w| w != dir) {
                continue;
            }
            let s = score_as(query, &p, dir);
            if s >= 0.8 && !hits.iter().any(|h| h.path == p) {
                let modified = p.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                hits.push(Hit { path: p, score: s, modified });
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let span = crate::layers::read(crate::layers::Layer::Command, "command.locate");
        let Ok((true, out, _)) = span.end(super::shell::output("mdfind", &["-name", query.trim()], None, Duration::from_secs(2))) else { return };
        for line in out.lines().take(400) {
            let p = PathBuf::from(line);
            let dir = p.is_dir();
            if want_dir.is_some_and(|w| w != dir) {
                continue;
            }
            let s = score_as(query, &p, dir);
            if s >= 0.8 && !hits.iter().any(|h| h.path == p) {
                let modified = p.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                hits.push(Hit { path: p, score: s, modified });
            }
        }
    }
    #[cfg(windows)]
    let _ = (query, want_dir, hits);
}

pub fn search(query: &str, want_dir: Option<bool>, within: Option<&Path>, budget: Duration) -> Vec<Hit> {
    let until = Instant::now() + budget;
    let mut hits: Vec<Hit> = Vec::new();
    match within {
        Some(w) => {
            let span = crate::layers::read(crate::layers::Layer::System, "system.files.find");
            walk(query, want_dir, vec![w.to_path_buf()], 7, until, &mut hits);
            span.ok();
        }
        None => {
            let span = crate::layers::read(crate::layers::Layer::System, "system.files.find");
            walk(query, want_dir, home_roots(), 2, (Instant::now() + Duration::from_millis(40)).min(until), &mut hits);
            span.end_bool(exact(&hits));
            if !exact(&hits) {
                located(query, want_dir, &mut hits);
            }
            if !exact(&hits) {
                let span = crate::layers::read(crate::layers::Layer::System, "system.files.find");
                walk(query, want_dir, roots(), 7, until, &mut hits);
                span.ok();
            }
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(b.modified.cmp(&a.modified)));
    hits.truncate(8);
    hits
}

pub fn latest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false) && !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            !(n.ends_with(".part") || n.ends_with(".crdownload") || n.ends_with(".tmp") || n == "desktop.ini")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH))
        .map(|e| e.path())
}

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf> {
    let clean: String = name.trim().chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    if clean.trim().is_empty() {
        return Err(anyhow!("That folder name can't be used."));
    }
    let p = parent.join(clean.trim());
    if p.exists() {
        return Err(anyhow!("A folder called {} is already there.", clean.trim()));
    }
    let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
    span.end(std::fs::create_dir_all(&p)).map_err(|e| anyhow!("The folder could not be created: {e}"))?;
    Ok(p)
}

pub fn copy_into(src: &Path, dest_dir: &Path, move_it: bool) -> Result<PathBuf> {
    let span = crate::layers::effect(crate::layers::Layer::System, "system.files");
    span.end(copy_into_now(src, dest_dir, move_it))
}

fn copy_into_now(src: &Path, dest_dir: &Path, move_it: bool) -> Result<PathBuf> {
    if !dest_dir.is_dir() {
        return Err(anyhow!("{} isn't a folder.", dest_dir.display()));
    }
    let name = src.file_name().ok_or_else(|| anyhow!("That item has no name."))?;
    let mut dest = dest_dir.join(name);
    let mut k = 2;
    while dest.exists() {
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        dest = dest_dir.join(format!("{stem} ({k}){ext}"));
        k += 1;
    }
    if move_it && std::fs::rename(src, &dest).is_ok() {
        return Ok(dest);
    }
    copy_tree(src, &dest)?;
    if move_it {
        if src.is_dir() { std::fs::remove_dir_all(src)? } else { std::fs::remove_file(src)? }
    }
    Ok(dest)
}

fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dest)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_tree(&e.path(), &dest.join(e.file_name()))?;
        }
    } else {
        std::fs::copy(src, dest)?;
    }
    Ok(())
}

pub fn spoken_path(p: &Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
    let parent = p.parent().map(|q| {
        if q == home {
            "your home folder".to_owned()
        } else if Some(q.to_path_buf()) == dirs::download_dir() {
            "Downloads".to_owned()
        } else if Some(q.to_path_buf()) == dirs::document_dir() {
            "Documents".to_owned()
        } else if Some(q.to_path_buf()) == dirs::desktop_dir() {
            "the desktop".to_owned()
        } else {
            q.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| q.display().to_string())
        }
    });
    match parent {
        Some(par) if !par.is_empty() => format!("{name} in {par}"),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_drive_letters_and_folders() {
        assert_eq!(letter("open d drive"), Some('d'));
        assert_eq!(letter("the see drive"), Some('c'));
        assert_eq!(letter("drive e"), Some('e'));
        assert_eq!(letter("my documents"), None);
        assert!(known_folder("my downloads").is_some() || dirs::download_dir().is_none());
        assert!(known_folder("the moon").is_none());
    }

    #[test]
    fn scores_names() {
        assert!(score("resume", Path::new("/x/Resume.pdf")) >= 1.0);
        assert!(score("project report", Path::new("/x/final_project_report_v2.docx")) >= 0.88);
        assert!(score("cat", Path::new("/x/dog.png")) < 0.8);
        assert!(score("report.pdf", Path::new("/x/report.pdf")) >= 1.0);
        assert!(score("report.pdf", Path::new("/x/report.docx")) < 1.0);
        assert!(score("report pdf", Path::new("/x/report.pdf")) >= 1.0);
        assert!(score("report dot pdf", Path::new("/x/report.pdf")) >= 1.0);
    }

    #[test]
    fn an_exact_name_ends_the_walk_at_its_depth() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("report.pdf"), "x").unwrap();
        std::fs::create_dir_all(d.path().join("a").join("b")).unwrap();
        std::fs::write(d.path().join("a").join("b").join("report.pdf"), "x").unwrap();
        let hits = search("report.pdf", Some(false), Some(d.path()), Duration::from_secs(1));
        assert_eq!(hits.iter().map(|h| h.path.clone()).collect::<Vec<_>>(), vec![d.path().join("report.pdf")]);
    }

    #[test]
    fn creates_and_moves() {
        let d = tempfile::tempdir().unwrap();
        let f = create_folder(d.path(), "Notes").unwrap();
        assert!(f.is_dir());
        assert!(create_folder(d.path(), "Notes").is_err());
        std::fs::write(d.path().join("a.txt"), "x").unwrap();
        let moved = copy_into(&d.path().join("a.txt"), &f, true).unwrap();
        assert!(moved.exists() && !d.path().join("a.txt").exists());
        let hits = search("a", Some(false), Some(d.path()), Duration::from_secs(1));
        assert!(hits.iter().any(|h| h.path == moved));
    }
}
