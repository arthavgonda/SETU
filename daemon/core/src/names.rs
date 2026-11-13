use serde_json::{Value, json};
use std::sync::OnceLock;

static WORDS: &str = include_str!("../assets/common-words.txt");

struct Lexicon {
    words: Vec<(&'static str, String)>,
    pairs: Vec<(String, String)>,
}

fn lexicon() -> &'static Lexicon {
    static L: OnceLock<Lexicon> = OnceLock::new();
    L.get_or_init(|| {
        let words: Vec<(&'static str, String)> = WORDS.lines().filter(|w| !w.is_empty()).map(|w| (w, key(w))).collect();
        let short: Vec<&str> = words.iter().map(|(w, _)| *w).filter(|w| w.len() <= 4).take(160).collect();
        let mut pairs = Vec::with_capacity(short.len() * short.len());
        for a in &short {
            for b in &short {
                let p = format!("{a} {b}");
                pairs.push((key(&p), p));
            }
        }
        Lexicon { words, pairs }
    })
}

pub fn is_common(word: &str) -> bool {
    let w = word.to_lowercase();
    lexicon().words.iter().any(|(x, _)| *x == w)
}

pub fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'ē' => 'e',
            'í' | 'ì' | 'î' | 'ï' | 'ī' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ō' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'ū' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' | 'Ā' => 'A',
            'É' | 'È' | 'Ê' | 'Ë' | 'Ē' => 'E',
            'Í' | 'Ì' | 'Î' | 'Ï' | 'Ī' => 'I',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' | 'Ō' => 'O',
            'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ū' => 'U',
            'Ñ' => 'N',
            'Ç' => 'C',
            c => c,
        })
        .collect()
}

pub fn validate(name: &str) -> Result<(), String> {
    let n = name.trim();
    let count = n.chars().count();
    if !(2..=24).contains(&count) {
        return Err("The name must be 2 to 24 characters long.".into());
    }
    if !fold(n).chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-' || c == '\'') {
        return Err("Use letters A to Z, spaces or hyphens. The wake word detector understands English spelling.".into());
    }
    if !n.chars().any(|c| c.is_alphabetic()) {
        return Err("The name needs at least one letter.".into());
    }
    Ok(())
}

pub fn spoken(name: &str) -> String {
    fold(name)
        .chars()
        .map(|c| if c == '-' { ' ' } else { c.to_ascii_uppercase() })
        .filter(|c| c.is_ascii_uppercase() || *c == ' ' || *c == '\'')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn syllables(word: &str) -> usize {
    let w: Vec<char> = fold(word).to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if w.is_empty() {
        return 0;
    }
    let vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
    let mut n = 0;
    let mut prev = false;
    for (i, &c) in w.iter().enumerate() {
        let v = vowel(c) && !(c == 'y' && i == 0);
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    let len = w.len();
    if len > 2 && w[len - 1] == 'e' && !vowel(w[len - 2]) && !(w[len - 2] == 'l' && len > 3 && !vowel(w[len - 3])) && n > 1 {
        n -= 1;
    }
    if len > 3 && w[len - 2] == 'e' && w[len - 1] == 's' && !matches!(w[len - 3], 's' | 'z' | 'x' | 'c' | 'g' | 'h') && n > 1 {
        n -= 1;
    }
    n.max(1)
}

pub fn key(s: &str) -> String {
    let w: Vec<char> = fold(s).to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let mut out = String::new();
    let mut i = 0;
    let vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
    while i < w.len() {
        let c = w[i];
        let next = w.get(i + 1).copied();
        let (code, skip) = match c {
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' => ('A', 1),
            'p' if next == Some('h') => ('F', 2),
            'b' | 'p' => ('P', 1),
            'f' | 'v' => ('F', 1),
            'c' if next == Some('h') => ('X', 2),
            's' if next == Some('h') => ('X', 2),
            't' if next == Some('h') => ('T', 2),
            'c' if matches!(next, Some('e') | Some('i') | Some('y')) => ('S', 1),
            'c' | 'k' | 'q' => ('K', 1),
            'g' if matches!(next, Some('e') | Some('i')) => ('J', 1),
            'g' => ('K', 1),
            'j' => ('J', 1),
            'x' => ('K', 1),
            's' | 'z' => ('S', 1),
            'd' | 't' => ('T', 1),
            'l' => ('L', 1),
            'm' | 'n' => ('N', 1),
            'r' => ('R', 1),
            'w' if next.is_some_and(vowel) => ('W', 1),
            'h' if next.is_some_and(vowel) && (i == 0 || !vowel(w[i - 1])) => ('H', 1),
            _ => ('\0', 1),
        };
        if code != '\0' && !out.ends_with(code) {
            out.push(code);
        }
        i += skip;
    }
    if out.len() > 1 && out.ends_with('A') {
        out.pop();
        out.push('V');
    }
    out
}

fn similar(name: &str) -> Vec<String> {
    let k = key(name);
    let lower = name.to_lowercase();
    let lex = lexicon();
    let mut hits: Vec<(f64, String)> = Vec::new();
    let close = |a: &str, b: &str| a == b || (a.len() >= 4 && b.len() >= 4 && strsim::jaro_winkler(a, b) >= 0.93);
    for (w, wk) in &lex.words {
        if *w == lower || w.len() < 3 {
            continue;
        }
        if close(&k, wk) {
            hits.push((strsim::jaro_winkler(&lower, w), (*w).to_owned()));
        }
    }
    if !lower.contains(' ') {
        for (pk, p) in &lex.pairs {
            if k == *pk {
                hits.push((strsim::jaro_winkler(&lower, &p.replace(' ', "")) - 0.05, p.clone()));
            }
        }
    }
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<String> = Vec::new();
    for (_, w) in hits {
        if !out.contains(&w) {
            out.push(w);
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

pub fn check(name: &str, use_hey: bool) -> Value {
    let n = name.trim().to_owned();
    if let Err(e) = validate(&n) {
        return json!({ "name": n, "phrase": "", "valid": false, "error": e, "syllables": 0, "similar": [], "rating": "weak", "reason": e });
    }
    let syl: usize = n.split([' ', '-']).filter(|w| !w.is_empty()).map(syllables).sum();
    let sim = similar(&n);
    let mut score: i32 = match syl {
        0 | 1 => 0,
        2 => 1,
        _ => 2,
    };
    let mut reason = match syl {
        0 | 1 => "One-syllable names trigger by mistake often.".to_owned(),
        2 => "Two-syllable names work, but names of three syllables or more trigger by mistake less often.".to_owned(),
        _ => "Names of three syllables or more trigger by mistake less often.".to_owned(),
    };
    if let Some(w) = sim.first() {
        score -= 1;
        reason = format!("It sounds like \u{201c}{w}\u{201d}, a common word, so it may wake by mistake.");
    }
    if !use_hey {
        score -= 1;
        reason = format!("Without \u{201c}Hey\u{201d}, ordinary speech wakes it by mistake more often. {reason}");
    }
    let rating = match score {
        i32::MIN..=0 => "weak",
        1 => "fair",
        _ => "strong",
    };
    let phrase = if use_hey { format!("Hey {n}") } else { n.clone() };
    json!({ "name": n, "phrase": phrase, "valid": true, "error": null, "syllables": syl, "similar": sim, "rating": rating, "reason": reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_syllables() {
        assert_eq!(syllables("Setu"), 2);
        assert_eq!(syllables("Jarvis"), 2);
        assert_eq!(syllables("Bob"), 1);
        assert_eq!(syllables("Computer"), 3);
        assert_eq!(syllables("Maple"), 2);
        assert_eq!(syllables("Anastasia"), 4);
        assert_eq!(syllables("Jane"), 1);
    }

    #[test]
    fn validates_names() {
        assert!(validate("Setu").is_ok());
        assert!(validate("Mary-Ann").is_ok());
        assert!(validate("José").is_ok());
        assert!(validate("S").is_err());
        assert!(validate("सेतु").is_err());
        assert!(validate("R2D2").is_err());
        assert!(validate(&"a".repeat(25)).is_err());
        assert_eq!(spoken("Mary-Ann"), "MARY ANN");
        assert_eq!(spoken("José"), "JOSE");
    }

    #[test]
    fn rates_names() {
        let s = check("Setu", true);
        assert_eq!(s["valid"], true);
        assert_eq!(s["phrase"], "Hey Setu");
        assert_eq!(s["syllables"], 2);
        assert!(["fair", "weak"].contains(&s["rating"].as_str().unwrap()));
        let bob = check("Bob", true);
        assert_eq!(bob["rating"], "weak");
        let long = check("Anastasia", true);
        assert_eq!(long["rating"], "strong");
        let nohey = check("Anastasia", false);
        assert_eq!(nohey["rating"], "fair");
        assert_eq!(nohey["phrase"], "Anastasia");
    }

    #[test]
    fn finds_sound_alikes() {
        let s = similar("Setu");
        assert!(!s.is_empty(), "{s:?}");
        let c = similar("Kumpyuter");
        assert!(c.iter().any(|w| w == "computer"), "{c:?}");
    }
}
