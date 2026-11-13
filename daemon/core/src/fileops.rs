use anyhow::{Result, anyhow};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub fn free_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let dot = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let first = dir.join(format!("{stem}{dot}"));
    if !first.exists() {
        return first;
    }
    (2..10_000).map(|n| dir.join(format!("{stem} ({n}){dot}"))).find(|p| !p.exists()).unwrap_or(first)
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    let name = new_name.trim();
    if name.is_empty() || name.contains(['/', '\\', '\0']) || name == "." || name == ".." {
        return Err(anyhow!("That isn't a name a file can have."));
    }
    let keep_ext = Path::new(name).extension().is_none() && path.is_file();
    let name = match (keep_ext, path.extension()) {
        (true, Some(e)) => format!("{name}.{}", e.to_string_lossy()),
        _ => name.to_owned(),
    };
    let dest = path.parent().ok_or_else(|| anyhow!("That file has no folder."))?.join(&name);
    if dest.exists() {
        return Err(anyhow!("There is already something called {name} there."));
    }
    std::fs::rename(path, &dest).map_err(|e| anyhow!("It couldn't be renamed: {e}"))?;
    Ok(dest)
}

fn add(zip: &mut zip::ZipWriter<std::fs::File>, base: &Path, path: &Path, n: &mut usize) -> Result<()> {
    let rel = path.strip_prefix(base).unwrap_or(path).to_string_lossy().replace('\\', "/");
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let m = std::fs::symlink_metadata(path)?;
    if m.is_dir() {
        if !rel.is_empty() {
            zip.add_directory(format!("{rel}/"), opts)?;
        }
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for e in entries {
            add(zip, base, &e, n)?;
        }
    } else if m.is_file() {
        zip.start_file(rel, opts)?;
        let mut f = std::fs::File::open(path)?;
        std::io::copy(&mut f, zip)?;
        *n += 1;
    }
    Ok(())
}

pub fn compress(path: &Path) -> Result<(PathBuf, usize)> {
    let dir = path.parent().ok_or_else(|| anyhow!("That has no folder."))?;
    let stem = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
    let stem = if path.is_file() { Path::new(&stem).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(stem) } else { stem };
    let out = free_name(dir, &stem, "zip");
    let tmp = out.with_extension("zip.part");
    let file = std::fs::File::create(&tmp)?;
    let mut zip = zip::ZipWriter::new(file);
    let mut n = 0;
    let base = if path.is_dir() { path.parent().unwrap_or(path).to_path_buf() } else { dir.to_path_buf() };
    let r = add(&mut zip, &base, path, &mut n).and_then(|_| zip.finish().map(|_| ()).map_err(Into::into));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, &out)?;
    Ok((out, n))
}

fn safe_join(root: &Path, name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_absolute() {
        return None;
    }
    let mut out = root.to_path_buf();
    for c in p.components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

pub fn extract(path: &Path) -> Result<(PathBuf, usize)> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let dir = path.parent().ok_or_else(|| anyhow!("That has no folder."))?;
    let stem = ["tar.gz", "tgz", "tar.bz2", "tbz2", "tar", "zip"].iter().find_map(|e| name.strip_suffix(&format!(".{e}"))).map(str::to_owned).ok_or_else(|| anyhow!("I can open zip and tar archives."))?;
    let original_stem = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = original_stem[..stem.len().min(original_stem.len())].to_owned();
    let out = free_name(dir, &stem, "");
    std::fs::create_dir_all(&out)?;
    let r = (|| -> Result<usize> {
        let mut n = 0;
        if name.ends_with(".zip") {
            let mut z = zip::ZipArchive::new(std::fs::File::open(path)?)?;
            for i in 0..z.len() {
                let mut entry = z.by_index(i)?;
                let Some(dest) = safe_join(&out, entry.name()) else { return Err(anyhow!("That archive tries to write outside its folder, so I stopped.")) };
                if entry.is_dir() {
                    std::fs::create_dir_all(&dest)?;
                    continue;
                }
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let mut f = std::fs::File::create(&dest)?;
                let mut buf = [0u8; 65536];
                loop {
                    let k = entry.read(&mut buf)?;
                    if k == 0 {
                        break;
                    }
                    f.write_all(&buf[..k])?;
                }
                n += 1;
            }
            return Ok(n);
        }
        let file = std::fs::File::open(path)?;
        let reader: Box<dyn Read> = if name.ends_with(".gz") || name.ends_with(".tgz") {
            Box::new(flate2::read::GzDecoder::new(file))
        } else if name.ends_with(".bz2") || name.ends_with(".tbz2") {
            Box::new(bzip2::read::BzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let mut t = tar::Archive::new(reader);
        for e in t.entries()? {
            let mut e = e?;
            let p = e.path()?.to_string_lossy().into_owned();
            let kind = e.header().entry_type();
            if !(kind.is_file() || kind.is_dir()) {
                continue;
            }
            let Some(dest) = safe_join(&out, &p) else { return Err(anyhow!("That archive tries to write outside its folder, so I stopped.")) };
            if kind.is_dir() {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            if let Some(d) = dest.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut f = std::fs::File::create(&dest)?;
            std::io::copy(&mut e, &mut f)?;
            n += 1;
        }
        Ok(n)
    })();
    match r {
        Ok(n) => Ok((out, n)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&out);
            Err(e)
        }
    }
}

pub fn size_of(p: &Path, budget: &mut usize) -> u64 {
    let Ok(m) = std::fs::symlink_metadata(p) else { return 0 };
    if m.is_file() {
        return m.len();
    }
    if !m.is_dir() || *budget == 0 {
        return 0;
    }
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    let mut total = 0;
    for e in rd.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        total += size_of(&e.path(), budget);
    }
    total
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

pub fn properties(p: &Path) -> Result<String> {
    let m = std::fs::metadata(p).map_err(|_| anyhow!("{} isn't there.", p.display()))?;
    let mut budget = 200_000;
    let size = size_of(p, &mut budget);
    let what = if m.is_dir() {
        let n = std::fs::read_dir(p).map(|r| r.count()).unwrap_or(0);
        format!("a folder with {n} {}", if n == 1 { "item" } else { "items" })
    } else {
        let ext = p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
        if ext.is_empty() { "a file".into() } else { format!("a {ext} file") }
    };
    let when = m.modified().ok().and_then(|t| jiff::Timestamp::try_from(t).ok()).map(|t| t.to_zoned(jiff::tz::TimeZone::system()).strftime("%-d %B %Y at %-I:%M %p").to_string()).unwrap_or_default();
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let place = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
    let partial = if budget == 0 { " or more" } else { "" };
    Ok(format!("{name} is {what}, {}{partial}, changed {when}, in {place}.", human(size)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_round_trip_and_names_never_clash() {
        let t = tempfile::tempdir().unwrap();
        let photos = t.path().join("photos");
        std::fs::create_dir_all(photos.join("trip")).unwrap();
        std::fs::write(photos.join("a.jpg"), b"aaa").unwrap();
        std::fs::write(photos.join("trip/b.jpg"), b"bbb").unwrap();
        let (z, n) = compress(&photos).unwrap();
        assert_eq!((z.file_name().unwrap().to_str().unwrap(), n), ("photos.zip", 2));
        let (z2, _) = compress(&photos).unwrap();
        assert_eq!(z2.file_name().unwrap(), "photos (2).zip");
        let (out, n) = extract(&z).unwrap();
        assert_eq!(n, 2);
        assert_eq!(std::fs::read(out.join("photos/trip/b.jpg")).unwrap(), b"bbb");
    }

    #[test]
    fn archives_cannot_write_outside_their_folder() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("evil.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        z.start_file("../escaped.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
        assert!(extract(&p).is_err());
        assert!(!t.path().join("escaped.txt").exists());
        assert!(!t.path().join("evil").exists());
    }

    #[test]
    fn renaming_keeps_the_extension_and_never_overwrites() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("report.pdf");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(t.path().join("final.pdf"), b"y").unwrap();
        assert!(rename(&a, "final").is_err());
        assert_eq!(rename(&a, "summary").unwrap().file_name().unwrap(), "summary.pdf");
        assert!(rename(&t.path().join("summary.pdf"), "../x").is_err());
    }
}
