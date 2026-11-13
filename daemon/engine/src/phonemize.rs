use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub struct Phonemizer {
    exe: PathBuf,
    voice: String,
    data: Option<String>,
    vocab: Option<HashSet<char>>,
    indian: bool,
}

fn find_exe() -> PathBuf {
    let name = if cfg!(windows) { "espeak-ng.exe" } else { "espeak-ng" };
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent() {
            for c in [dir.join(name), dir.join("../lib/setu").join(name), dir.join("../Resources").join(name)] {
                if c.is_file() {
                    return c;
                }
            }
        }
    PathBuf::from(name)
}

const BREAKS: &[char] = &[',', '.', ';', ':', '!', '?'];

const INDIAN_WORDS: &[(&str, &str)] = &[
    ("setu", "sˈeːtuː"),
    ("setu's", "sˈeːtuːz"),
    ("namaste", "nəmˈʌsteː"),
    ("namaskar", "nəmˈʌskaːɾ"),
    ("dhanyavaad", "dʰˈʌnjəʋaːd"),
    ("hindi", "hˈɪndiː"),
    ("india", "ˈɪndɪjaː"),
    ("ji", "dʒiː"),
    ("accha", "ˈʌtʃʰaː"),
    ("theek", "ʈʰiːk"),
    ("haan", "hãː"),
];

pub fn clauses(text: &str) -> Vec<(String, Option<char>)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if BREAKS.contains(&c) {
            let next_is_digit = chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            let prev_is_digit = i > 0 && chars[i - 1].is_ascii_digit();
            if (c == '.' || c == ',') && next_is_digit && prev_is_digit {
                cur.push(c);
                continue;
            }
            if !cur.trim().is_empty() {
                out.push((cur.trim().to_owned(), Some(c)));
            }
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push((cur.trim().to_owned(), None));
    }
    out
}

const JOINERS: &[&str] = &["and", "but", "so", "because", "which", "then", "or", "while", "when", "where", "if", "with", "after", "before", "until", "since", "though"];

fn split_long(piece: &str, max: usize) -> Vec<String> {
    let words: Vec<&str> = piece.split_whitespace().collect();
    if words.len() <= max {
        return vec![words.join(" ")];
    }
    let lower: Vec<String> = words.iter().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).collect();
    let cut = (4..=max.min(words.len() - 3)).rev().find(|&i| JOINERS.contains(&lower[i].as_str())).or_else(|| (words.len() > max + 6).then_some(max));
    match cut {
        Some(i) => {
            let mut out = vec![words[..i].join(" ")];
            out.extend(split_long(&words[i..].join(" "), 16));
            out
        }
        None => vec![words.join(" ")],
    }
}

pub fn chunks(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut pending = String::new();
    for (clause, punct) in clauses(text) {
        let piece = match punct {
            Some(c) => format!("{clause}{c}"),
            None => clause,
        };
        let first = out.is_empty() && pending.is_empty();
        for part in split_long(&piece, if first { 8 } else { 18 }) {
            if !pending.is_empty() {
                pending.push(' ');
            }
            pending.push_str(&part);
            let need = if out.is_empty() { 1 } else { 6 };
            if pending.split_whitespace().count() >= need {
                out.push(std::mem::take(&mut pending));
            }
        }
    }
    if !pending.is_empty() {
        let merge = pending.split_whitespace().count() < 3 && out.len() > 1;
        match out.last_mut() {
            Some(last) if merge => {
                last.push(' ');
                last.push_str(&pending);
            }
            _ => out.push(pending),
        }
    }
    out
}

pub fn read_vocab(tokens: &str) -> Option<HashSet<char>> {
    let text = std::fs::read_to_string(tokens).ok()?;
    let mut set = HashSet::new();
    for line in text.lines() {
        let Some((sym, _)) = line.rsplit_once(' ') else { continue };
        let mut it = sym.chars();
        if let (Some(c), None) = (it.next(), it.next()) {
            set.insert(c);
        } else if sym.is_empty() {
            set.insert(' ');
        }
    }
    (!set.is_empty()).then_some(set)
}

pub fn indian_accent(ipa: &str) -> String {
    let chars: Vec<char> = ipa.chars().collect();
    let mut s = String::with_capacity(ipa.len());
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        match c {
            't' if next != Some('ʃ') => s.push('ʈ'),
            'd' if next != Some('ʒ') => s.push('ɖ'),
            _ => s.push(c),
        }
    }
    for (from, to) in [("eɪ", "eː"), ("əʊ", "oː"), ("oʊ", "oː"), ("ɜː", "əɾ"), ("θ", "tʰ"), ("ð", "d"), ("w", "ʋ"), ("ɹ", "ɾ")] {
        s = s.replace(from, to);
    }
    s
}

impl Phonemizer {
    pub fn new(voice: &str, data: &str) -> Self {
        Self {
            exe: find_exe(),
            voice: if voice.is_empty() { "en-us".into() } else { voice.to_owned() },
            data: (!data.is_empty() && std::path::Path::new(data).join("espeak-ng-data").is_dir()).then(|| data.to_owned()),
            vocab: None,
            indian: false,
        }
    }

    pub fn with_vocab(mut self, tokens: &str) -> Self {
        self.vocab = read_vocab(tokens);
        self
    }

    pub fn indian(mut self, on: bool) -> Self {
        self.indian = on && self.voice.starts_with("en");
        self
    }

    fn run(&self, text: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.exe);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        cmd.args(["-q", "--ipa", "--stdin", "-v", &self.voice]);
        if let Some(d) = &self.data {
            cmd.arg(format!("--path={d}"));
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("The phonemizer could not start: {e}"))?;
        if let Some(mut si) = child.stdin.take() {
            si.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
    }

    fn clause(&self, text: &str) -> Result<String, String> {
        if !self.indian {
            return self.run(text);
        }
        let mut parts: Vec<String> = Vec::new();
        let mut pending: Vec<&str> = Vec::new();
        for w in text.split_whitespace() {
            let key: String = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase();
            if let Some((_, ipa)) = INDIAN_WORDS.iter().find(|(k, _)| *k == key) {
                if !pending.is_empty() {
                    parts.push(indian_accent(&self.run(&pending.join(" "))?));
                    pending.clear();
                }
                parts.push((*ipa).to_owned());
            } else {
                pending.push(w);
            }
        }
        if !pending.is_empty() {
            parts.push(indian_accent(&self.run(&pending.join(" "))?));
        }
        Ok(parts.join(" "))
    }

    fn keep(&self, s: String) -> String {
        match &self.vocab {
            Some(v) => s.chars().filter(|c| v.contains(c)).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "),
            None => s,
        }
    }

    pub fn parts(&self, text: &str) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for c in chunks(text) {
            let p = self.sentence(&c)?;
            if !p.is_empty() {
                out.push(p);
            }
        }
        Ok(out)
    }

    pub fn sentence(&self, text: &str) -> Result<String, String> {
        let mut parts = Vec::new();
        for (clause, punct) in clauses(text) {
            let p = self.keep(self.clause(&clause)?);
            if p.is_empty() {
                continue;
            }
            match punct {
                Some(c) if self.vocab.as_ref().is_none_or(|v| v.contains(&c)) => parts.push(format!("{p}{c}")),
                _ => parts.push(p),
            }
        }
        Ok(parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_clauses_and_keeps_numbers() {
        let c = clauses("Hey Setu, it is 3.5 degrees. Open it!");
        assert_eq!(c.len(), 3);
        assert_eq!(c[0], ("Hey Setu".into(), Some(',')));
        assert_eq!(c[1], ("it is 3.5 degrees".into(), Some('.')));
        assert_eq!(c[2], ("Open it".into(), Some('!')));
    }

    #[test]
    fn first_chunk_is_short_and_long_clauses_split() {
        let c = chunks("Opening Firefox.");
        assert_eq!(c, vec!["Opening Firefox.".to_owned()]);
        let long = "Sure, I found three videos about cooking rice and the first one is from a chef in Delhi who explains every step slowly.";
        let c = chunks(long);
        assert_eq!(c[0], "Sure,");
        assert!(c.len() >= 3, "{c:?}");
        assert!(c.iter().all(|x| x.split_whitespace().count() <= 18), "{c:?}");
        assert_eq!(c.join(" "), long);
        let c = chunks("The weather today is warm and sunny with a light breeze from the sea and a few clouds in the late afternoon.");
        assert!(c[0].split_whitespace().count() <= 8, "{c:?}");
        let c = chunks("Done. Okay.");
        assert_eq!(c, vec!["Done.".to_owned(), "Okay.".to_owned()]);
    }

    #[test]
    fn indian_accent_keeps_affricates() {
        assert_eq!(indian_accent("tʃeɪndʒ"), "tʃeːndʒ");
        assert_eq!(indian_accent("wˈɜːld"), "ʋˈəɾlɖ");
        assert_eq!(indian_accent("ðə θˈɪŋ"), "də tʰˈɪŋ");
        assert_eq!(indian_accent("ɹˈuːm"), "ɾˈuːm");
    }
}
