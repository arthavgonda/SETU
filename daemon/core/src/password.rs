use crate::text::number_at;

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add(String),
    DeleteLast,
    StartOver,
    Change(usize, char),
    Show,
    Hide,
    ReadBack,
    Connect,
    Cancel,
}

const LETTERS: &[(&str, char)] = &[
    ("a", 'a'),
    ("ay", 'a'),
    ("alpha", 'a'),
    ("alfa", 'a'),
    ("b", 'b'),
    ("bee", 'b'),
    ("be", 'b'),
    ("bravo", 'b'),
    ("c", 'c'),
    ("see", 'c'),
    ("sea", 'c'),
    ("charlie", 'c'),
    ("d", 'd'),
    ("dee", 'd'),
    ("delta", 'd'),
    ("e", 'e'),
    ("echo", 'e'),
    ("f", 'f'),
    ("eff", 'f'),
    ("ef", 'f'),
    ("foxtrot", 'f'),
    ("g", 'g'),
    ("gee", 'g'),
    ("golf", 'g'),
    ("h", 'h'),
    ("aitch", 'h'),
    ("hotel", 'h'),
    ("i", 'i'),
    ("eye", 'i'),
    ("india", 'i'),
    ("j", 'j'),
    ("jay", 'j'),
    ("juliet", 'j'),
    ("juliett", 'j'),
    ("k", 'k'),
    ("kay", 'k'),
    ("kilo", 'k'),
    ("l", 'l'),
    ("el", 'l'),
    ("ell", 'l'),
    ("lima", 'l'),
    ("m", 'm'),
    ("em", 'm'),
    ("mike", 'm'),
    ("n", 'n'),
    ("en", 'n'),
    ("november", 'n'),
    ("o", 'o'),
    ("oscar", 'o'),
    ("p", 'p'),
    ("pee", 'p'),
    ("papa", 'p'),
    ("q", 'q'),
    ("cue", 'q'),
    ("queue", 'q'),
    ("quebec", 'q'),
    ("r", 'r'),
    ("are", 'r'),
    ("ar", 'r'),
    ("romeo", 'r'),
    ("s", 's'),
    ("es", 's'),
    ("ess", 's'),
    ("sierra", 's'),
    ("t", 't'),
    ("tee", 't'),
    ("tea", 't'),
    ("tango", 't'),
    ("u", 'u'),
    ("you", 'u'),
    ("uniform", 'u'),
    ("v", 'v'),
    ("vee", 'v'),
    ("victor", 'v'),
    ("w", 'w'),
    ("whiskey", 'w'),
    ("whisky", 'w'),
    ("x", 'x'),
    ("ex", 'x'),
    ("xray", 'x'),
    ("y", 'y'),
    ("why", 'y'),
    ("wye", 'y'),
    ("yankee", 'y'),
    ("z", 'z'),
    ("zee", 'z'),
    ("zed", 'z'),
    ("zulu", 'z'),
];

const SYMBOLS: &[(&[&str], char)] = &[
    (&["underscore"], '_'),
    (&["at", "sign"], '@'),
    (&["at"], '@'),
    (&["space"], ' '),
    (&["dash"], '-'),
    (&["hyphen"], '-'),
    (&["minus"], '-'),
    (&["dot"], '.'),
    (&["period"], '.'),
    (&["full", "stop"], '.'),
    (&["point"], '.'),
    (&["comma"], ','),
    (&["exclamation", "mark"], '!'),
    (&["exclamation", "point"], '!'),
    (&["exclamation"], '!'),
    (&["question", "mark"], '?'),
    (&["hash"], '#'),
    (&["hashtag"], '#'),
    (&["pound", "sign"], '#'),
    (&["dollar", "sign"], '$'),
    (&["dollar"], '$'),
    (&["percent", "sign"], '%'),
    (&["percent"], '%'),
    (&["ampersand"], '&'),
    (&["and", "sign"], '&'),
    (&["asterisk"], '*'),
    (&["star"], '*'),
    (&["plus", "sign"], '+'),
    (&["plus"], '+'),
    (&["equals", "sign"], '='),
    (&["equals"], '='),
    (&["equal"], '='),
    (&["forward", "slash"], '/'),
    (&["slash"], '/'),
    (&["backslash"], '\\'),
    (&["back", "slash"], '\\'),
    (&["colon"], ':'),
    (&["semicolon"], ';'),
    (&["apostrophe"], '\''),
    (&["single", "quote"], '\''),
    (&["double", "quote"], '"'),
    (&["quote"], '"'),
    (&["tilde"], '~'),
    (&["caret"], '^'),
    (&["open", "bracket"], '('),
    (&["close", "bracket"], ')'),
    (&["open", "paren"], '('),
    (&["close", "paren"], ')'),
    (&["pipe"], '|'),
    (&["less", "than"], '<'),
    (&["greater", "than"], '>'),
];

fn words(text: &str) -> Vec<String> {
    crate::text::normalize(text).split_whitespace().map(str::to_owned).collect()
}

fn one_char(w: &[&str]) -> Option<(char, usize)> {
    let (mut upper, mut skip) = (None, 0);
    match w.first().copied() {
        Some("capital") | Some("uppercase") | Some("upper") | Some("big") => {
            upper = Some(true);
            skip = 1;
            if w.get(1) == Some(&"case") {
                skip = 2;
            }
        }
        Some("lowercase") | Some("lower") | Some("small") => {
            upper = Some(false);
            skip = 1;
            if w.get(1) == Some(&"case") {
                skip = 2;
            }
        }
        _ => {}
    }
    let rest = &w[skip..];
    let first = *rest.first()?;
    let mut used = skip + 1;
    let c = if first == "double" && matches!(rest.get(1).copied(), Some("you") | Some("u")) {
        used += 1;
        'w'
    } else if let Some(&(_, c)) = LETTERS.iter().find(|(n, _)| *n == first) {
        c
    } else if first.len() == 1 && first.chars().all(|c| c.is_ascii_digit()) {
        first.chars().next()?
    } else if let Some((n, u)) = number_at(rest).filter(|(n, _)| (0..=9).contains(n)) {
        if upper.is_some() {
            return None;
        }
        used = skip + u;
        char::from_digit(n as u32, 10)?
    } else {
        return None;
    };
    if c.is_ascii_alphabetic() && rest.get(used - skip) == Some(&"as") && rest.get(used - skip + 1) == Some(&"in") && rest.get(used - skip + 2).is_some() {
        used += 3;
    }
    let c = if upper == Some(true) { c.to_ascii_uppercase() } else { c };
    Some((c, used))
}

fn symbol(w: &[&str]) -> Option<(char, usize)> {
    let mut best: Option<(char, usize)> = None;
    for (pat, c) in SYMBOLS {
        if w.len() >= pat.len() && w[..pat.len()] == **pat && best.is_none_or(|(_, n)| pat.len() > n) {
            best = Some((*c, pat.len()));
        }
    }
    best.map(|(c, n)| (c, if w.get(n) == Some(&"sign") || w.get(n) == Some(&"symbol") { n + 1 } else { n }))
}

pub fn parse(text: &str) -> Vec<Op> {
    let ws = words(text);
    let w: Vec<&str> = ws.iter().map(String::as_str).collect();
    let joined = w.join(" ");
    match joined.as_str() {
        "connect" | "connect now" | "done" | "that's it" | "join" => return vec![Op::Connect],
        "delete last" | "delete the last one" | "delete" | "backspace" | "remove last" | "undo" | "delete last character" => return vec![Op::DeleteLast],
        "start over" | "clear" | "clear all" | "start again" | "reset" => return vec![Op::StartOver],
        "show password" | "show" | "show it" => return vec![Op::Show],
        "hide password" | "hide" | "hide it" => return vec![Op::Hide],
        "read it back" | "read back" | "read it" | "read the password" => return vec![Op::ReadBack],
        "cancel" | "stop" | "never mind" => return vec![Op::Cancel],
        _ => {}
    }
    if w.first() == Some(&"change") {
        let mut i = 1;
        if w.get(i) == Some(&"character") || w.get(i) == Some(&"letter") || w.get(i) == Some(&"number") {
            i += 1;
        }
        if let Some((n, used)) = number_at(&w[i.min(w.len())..]) {
            i += used;
            if w.get(i) == Some(&"to") {
                i += 1;
            }
            let rest = &w[i.min(w.len())..];
            if let Some((c, _)) = symbol(rest).or_else(|| one_char(rest))
                && n >= 1 {
                    return vec![Op::Change(n as usize, c)];
                }
        }
        return Vec::new();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < w.len() {
        let rest = &w[i..];
        if let Some((c, n)) = symbol(rest) {
            out.push(c);
            i += n;
            continue;
        }
        if let Some((c, n)) = one_char(rest) {
            out.push(c);
            i += n;
            continue;
        }
        let word = rest[0];
        if word.len() > 1 && word.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(word);
        }
        i += 1;
    }
    if out.is_empty() { Vec::new() } else { vec![Op::Add(out)] }
}

pub fn apply(buf: &mut Vec<char>, op: &Op) -> bool {
    match op {
        Op::Add(s) => {
            for c in s.chars() {
                if buf.len() < 63 {
                    buf.push(c);
                }
            }
            true
        }
        Op::DeleteLast => buf.pop().is_some(),
        Op::StartOver => {
            buf.clear();
            true
        }
        Op::Change(n, c)
            if *n >= 1 && *n <= buf.len() => {
                buf[n - 1] = *c;
                true
            }
        _ => false,
    }
}

pub fn spell(buf: &[char]) -> String {
    buf.iter()
        .map(|c| match c {
            'A'..='Z' => format!("capital {c}"),
            'a'..='z' | '0'..='9' => c.to_string(),
            c => SYMBOLS.iter().find(|(_, s)| s == c).map(|(w, _)| w.join(" ")).unwrap_or_else(|| c.to_string()),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(t: &str) -> String {
        match parse(t).first() {
            Some(Op::Add(s)) => s.clone(),
            other => panic!("{t}: {other:?}"),
        }
    }

    #[test]
    fn letters_digits_symbols() {
        assert_eq!(add("capital a b c"), "Abc");
        assert_eq!(add("capital A as in Apple seven underscore x"), "A7_x");
        assert_eq!(add("one two three"), "123");
        assert_eq!(add("at sign dot space dash"), "@. -");
        assert_eq!(add("double you why zed"), "wyz");
        assert_eq!(add("lowercase bee capital delta"), "bD");
        assert_eq!(add("2024"), "2024");
        assert_eq!(add("exclamation mark hash dollar sign"), "!#$");
    }

    #[test]
    fn edit_commands() {
        assert_eq!(parse("change character 5 to capital B"), vec![Op::Change(5, 'B')]);
        assert_eq!(parse("change character two to underscore"), vec![Op::Change(2, '_')]);
        assert_eq!(parse("delete last"), vec![Op::DeleteLast]);
        assert_eq!(parse("Start over."), vec![Op::StartOver]);
        assert_eq!(parse("connect"), vec![Op::Connect]);
        assert_eq!(parse("read it back"), vec![Op::ReadBack]);
        assert_eq!(parse("show password"), vec![Op::Show]);
        let mut b = vec!['a', 'b', 'c'];
        apply(&mut b, &Op::Change(2, 'X'));
        apply(&mut b, &Op::DeleteLast);
        assert_eq!(b, vec!['a', 'X']);
        assert!(!apply(&mut b, &Op::Change(9, 'z')));
        assert_eq!(spell(&['A', 'b', '_', '1']), "capital A, b, underscore, 1");
    }
}
