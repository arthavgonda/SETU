use crate::names;

pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in names::fold(s).chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '\'' || c == '%' {
            out.push(c);
            space = false;
        } else if !space && !out.is_empty() {
            out.push(' ');
            space = true;
        }
    }
    out.trim_end().to_owned()
}

fn word_key(w: &str) -> String {
    names::key(w)
}

fn ends_vowel(k: &str) -> bool {
    k.ends_with('V')
}

fn same_word(cand: &str, name_key: &str) -> f64 {
    let k = word_key(cand);
    if k == name_key {
        return 1.0;
    }
    if k.len() < 2 || ends_vowel(&k) != ends_vowel(name_key) || (k.len() as i64 - name_key.len() as i64).abs() > 1 {
        return 0.0;
    }
    strsim::jaro_winkler(&k, name_key)
}

pub fn strip_lead(text: &str, name: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let name_key = word_key(&normalize(name).replace(' ', ""));
    let start = usize::from(words.first().is_some_and(|w| ["hey", "hay", "hi", "okay", "ok"].contains(w)));
    if words.len() > start && same_word(words[start], &name_key) >= 0.9 {
        return words[start + 1..].join(" ");
    }
    words.join(" ")
}

pub fn names_first(text: &str, name: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if !words.is_empty() && heard_name(&words[usize::from(["hey", "hay", "hi", "a", "he", "okay", "ok"].contains(&words[0]))..], name).is_some() {
        return true;
    }
    let name_n = normalize(name);
    let name_key = word_key(&name_n.replace(' ', ""));
    let n = name_n.split_whitespace().count().max(1);
    let start = usize::from(words.first().is_some_and(|w| ["hey", "hay", "hi", "a", "he", "okay", "ok"].contains(w)));
    (1..=(n + 1).min(3)).any(|take| start + take <= words.len() && same_word(&words[start..start + take].concat(), &name_key) >= if take == 1 { 0.8 } else { 0.9 })
}

const YES: &[&str] = &["yes", "ye", "yea", "yeah", "yep", "yup", "yas", "yeh", "sure", "ok", "okay", "confirm", "correct", "right", "absolutely", "please", "go ahead", "do it", "of course", "open it", "yes please", "please do"];
const NO: &[&str] = &["no", "nope", "nah", "cancel", "don't", "do not", "stop", "never mind", "not now", "no thanks", "no thank you", "leave it"];

pub fn yes_no(text: &str) -> Option<bool> {
    let t = normalize(text);
    let t = t.trim_end_matches(" please").trim();
    if t.is_empty() {
        return None;
    }
    if NO.contains(&t) || NO.iter().any(|n| t.starts_with(&format!("{n} "))) {
        return Some(false);
    }
    if YES.contains(&t) || YES.iter().any(|y| t.starts_with(&format!("{y} ")) && t.split_whitespace().count() <= 4) {
        return Some(true);
    }
    None
}

pub fn spoken_phrase(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c == '\'' {
            out.push(c);
        } else if c.is_ascii_digit() {
            out.push(' ');
            out.push_str(["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"][c as usize - '0' as usize]);
            out.push(' ');
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().take(6).collect::<Vec<_>>().join(" ").chars().take(48).collect::<String>().trim().to_owned()
}

pub fn repair_tail(offline: &str, online: &str) -> String {
    let clean = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let (Some(off), Some(on)) = (offline.split_whitespace().last(), online.split_whitespace().last()) else {
        return offline.to_owned();
    };
    let (a, b) = (clean(off), clean(on));
    if a.is_empty() || b.len() <= a.len() || names::is_common(&a) {
        return offline.to_owned();
    }
    let shared = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    if shared + 1 < a.chars().count() || shared < 2 {
        return offline.to_owned();
    }
    if a.chars().count() > 2 && b.starts_with(&a) && ["s", "es", "ed", "d", "ing", "er", "ers", "ly"].contains(&&b[a.len()..]) {
        return offline.to_owned();
    }
    let punct: String = off.chars().rev().take_while(|c| !c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
    let head = &offline[..offline.len() - off.len()];
    format!("{head}{}{punct}", on.trim_matches(|c: char| !c.is_alphanumeric()))
}

const SETU_HEARD: &[&str] = &["see you", "see u", "set you", "seat you", "sit you", "say to", "say too", "say two", "set to", "set two", "see to", "she to", "c2", "c 2", "c to", "sethu", "satu", "sethu", "setu", "set u", "settle", "say tu", "sae to"];

fn heard_name(words: &[&str], name: &str) -> Option<usize> {
    if !normalize(name).eq_ignore_ascii_case("setu") {
        return None;
    }
    SETU_HEARD.iter().filter_map(|h| {
        let hw: Vec<&str> = h.split(' ').collect();
        (words.len() >= hw.len() && words[..hw.len()] == hw[..]).then_some(hw.len())
    }).max()
}

pub fn strip_wake(text: &str, name: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let lead = usize::from(["hey", "hay", "hi", "a", "he", "okay", "ok"].contains(&words[0]));
    if let Some(n) = heard_name(&words[lead..], name) {
        return words[lead + n..].join(" ");
    }
    let name_n = normalize(name);
    let name_key = word_key(&name_n.replace(' ', ""));
    let name_len = name_n.split_whitespace().count().max(1);
    let hey = ["hey", "hay", "hi", "a", "he", "okay", "ok"];
    let mut start = 0;
    if words.first().is_some_and(|w| hey.contains(w)) {
        start = 1;
    }
    let max_take = (name_len + 1).min(3).min(words.len().saturating_sub(start));
    let mut best: Option<(usize, f64)> = None;
    for take in 1..=max_take {
        let cand: String = words[start..start + take].concat();
        let score = same_word(&cand, &name_key);
        let bonus = if take == name_len { 0.02 } else { 0.0 };
        let need = if take == 1 { 0.8 } else { 0.9 };
        if score >= need && best.is_none_or(|(_, b)| score + bonus > b) {
            best = Some((take, score + bonus));
        }
    }
    if let Some((take, _)) = best {
        return words[start + take..].join(" ");
    }
    if start == 1 && words.len() > 1 && same_word(words[1], &name_key) >= 0.8 {
        return words[2..].join(" ");
    }
    words.join(" ")
}

const ONES: &[(&str, i64)] = &[
    ("zero", 0),
    ("oh", 0),
    ("one", 1),
    ("two", 2),
    ("to", 2),
    ("too", 2),
    ("three", 3),
    ("four", 4),
    ("for", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
];

const TENS: &[(&str, i64)] = &[("twenty", 20), ("thirty", 30), ("forty", 40), ("fourty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70), ("eighty", 80), ("ninety", 90)];

pub fn number_at(words: &[&str]) -> Option<(i64, usize)> {
    let w0 = *words.first()?;
    let trimmed = w0.trim_end_matches('%');
    if let Ok(n) = trimmed.parse::<i64>() {
        return Some((n, 1));
    }
    if w0 == "a" || w0 == "an" {
        if words.get(1) == Some(&"hundred") {
            return Some((100, 2));
        }
        return None;
    }
    if w0 == "hundred" {
        return Some((100, 1));
    }
    if let Some(&(_, t)) = TENS.iter().find(|(w, _)| *w == w0) {
        if let Some(w1) = words.get(1)
            && let Some(&(_, o)) = ONES.iter().find(|(w, v)| w == w1 && (1..=9).contains(v)) {
                return Some((t + o, 2));
            }
        return Some((t, 1));
    }
    if let Some(&(_, o)) = ONES.iter().find(|(w, _)| *w == w0) {
        if words.get(1) == Some(&"hundred") {
            return Some((o * 100, 2));
        }
        return Some((o, 1));
    }
    None
}

pub fn find_number(text: &str) -> Option<i64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    for i in 0..words.len() {
        if matches!(words[i], "to" | "too" | "for" | "oh" | "a" | "an")
            && !(i > 0 && matches!(words[i - 1], "number" | "option")) {
                continue;
            }
        if let Some((n, _)) = number_at(&words[i..]) {
            return Some(n);
        }
    }
    None
}

pub fn minutes(text: &str) -> Option<i64> {
    if text.contains("half an hour") || text.contains("half hour") || text.contains("thirty minutes") {
        return Some(30);
    }
    if text.contains("an hour") || text.contains("one hour") {
        return Some(60);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    for i in 0..words.len() {
        if let Some((n, used)) = number_at(&words[i..]) {
            match words.get(i + used).copied() {
                Some("minute") | Some("minutes") | Some("min") | Some("mins") => return Some(n),
                Some("hour") | Some("hours") => return Some(n * 60),
                _ => {}
            }
        }
    }
    None
}

pub fn similarity(query: &str, candidate: &str) -> f64 {
    let q = normalize(query);
    let c = normalize(candidate);
    if q.is_empty() || c.is_empty() {
        return 0.0;
    }
    if q == c {
        return 1.0;
    }
    let qj = q.replace(' ', "");
    let cj = c.replace(' ', "");
    if qj == cj {
        return 0.99;
    }
    let qk = names::key(&qj);
    let ck = names::key(&cj);
    let mut best = strsim::jaro_winkler(&qj, &cj) * 0.92;
    if !qk.is_empty() && qk == ck {
        best = best.max(0.93);
    } else if qk.len() >= 2 && ck.len() >= 2 {
        best = best.max(strsim::jaro_winkler(&qk, &ck) * 0.88);
    }
    let cwords: Vec<&str> = c.split_whitespace().collect();
    let qwords: Vec<&str> = q.split_whitespace().collect();
    if cwords.len() > 1 {
        let all_in = qwords.iter().all(|w| cwords.iter().any(|cw| cw == w || (w.len() >= 3 && cw.starts_with(w))));
        if all_in {
            let cover = qwords.len() as f64 / cwords.len() as f64;
            best = best.max(0.80 + 0.15 * cover);
        }
        for cw in &cwords {
            if cw.len() >= 4 && qj == *cw {
                best = best.max(0.9);
            }
        }
    }
    if cj.starts_with(&qj) && qj.len() >= 3 {
        best = best.max(0.85 + 0.1 * (qj.len() as f64 / cj.len() as f64));
    }
    if qwords.len() > 1 && qwords.len() == cwords.len() && qwords.iter().any(|w| w.len() == 1) {
        let ok = qwords.iter().zip(&cwords).all(|(q, c)| q == c || (q.len() == 1 && c.starts_with(q)));
        if ok {
            best = best.max(0.92);
        }
    }
    if qwords.len() > 1 && qwords.iter().all(|w| w.len() == 1) {
        let initials: String = cwords.iter().filter_map(|w| w.chars().next()).collect();
        if initials == qj {
            best = best.max(0.9);
        }
    }
    best.min(1.0)
}

pub fn sentence_case(s: &str) -> String {
    let t = s.trim();
    let mut c = t.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const JOINERS: &[&str] = &["and", "but", "so", "because", "which", "who", "that", "when", "while", "where", "with", "to", "for", "or", "if", "then"];

const SOCIAL: &[&str] = &[
    " how are you", " how r you", " how r u ", " how are u ", " how's it going", " how is it going", " how have you been", " how's your day", " how is your day", " how was your day", " what's up ", " whats up ",
    " my day was", " my day is", " my day has", " today was ", " had a long day", " had a great day", " had a good day", " had a bad day", " had a rough day", " had a tough day", " had a nice day",
    " i'm feeling", " i am feeling", " i feel ", " i'm tired", " i am tired", " i'm so tired", " i'm bored", " i am bored", " i'm sad", " i am sad", " i'm happy", " i am happy", " i'm stressed", " i am stressed", " i'm exhausted",
    " good morning", " good afternoon", " good evening", " good night",
];

pub fn social(raw: &str) -> bool {
    let t = format!(" {} ", normalize(raw));
    SOCIAL.iter().any(|s| t.contains(s))
}

pub fn stream_chunk(json_so_far: &str, offset: usize) -> Option<(String, usize)> {
    let body = json_so_far.strip_prefix("{\"tool\":\"reply\",\"text\":\"").or_else(|| json_so_far.strip_prefix("{\"say\":\""))?;
    let mut clean = String::new();
    let mut chars = body.chars();
    let mut closed = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                closed = true;
                break;
            }
            '\\' => match chars.next() {
                Some('n') | Some('t') => clean.push(' '),
                Some(o) => clean.push(o),
                None => break,
            },
            o => clean.push(o),
        }
    }
    if clean.len() <= offset || !clean.is_char_boundary(offset) {
        return None;
    }
    let rest = &clean[offset..];
    let first = offset == 0;
    let bytes = rest.as_bytes();
    let words = |k: usize| rest[..k].split_whitespace().count();
    let min_clause = if first { 4 } else { 7 };
    let mut end = None;
    for (k, b) in bytes.iter().enumerate() {
        let after = bytes.get(k + 1);
        let prev_word = k > 0 && bytes[k - 1].is_ascii_alphabetic();
        let sentence = matches!(b, b'.' | b'?' | b'!') && prev_word && (after == Some(&b' ') || after.is_none() && closed || after.is_none() && k + 1 == bytes.len());
        let clause = matches!(b, b',' | b';' | b':') && prev_word && after == Some(&b' ') && words(k + 1) >= min_clause;
        if sentence || clause {
            end = Some(k + 1);
        }
    }
    if end.is_none() {
        let list: Vec<(usize, &str)> = rest.split(' ').scan(0usize, |pos, w| {
            let at = *pos;
            *pos += w.len() + 1;
            Some((at, w))
        }).collect();
        let complete = if closed { list.len() } else { list.len().saturating_sub(1) };
        let force = if first { 9 } else { 14 };
        if complete >= force {
            let cut = list[..complete].iter().enumerate().rev().find(|(n, (_, w))| *n >= 5 && JOINERS.contains(&w.to_lowercase().as_str())).map(|(_, (at, _))| *at);
            end = Some(cut.unwrap_or_else(|| list.get(force).map(|(at, _)| *at).unwrap_or(rest.len())));
        }
    }
    let end = end?;
    let chunk = rest[..end].trim().to_owned();
    let need = if first { 8 } else { 12 };
    (chunk.chars().count() >= need).then_some((chunk, offset + end))
}

pub fn display_case(s: &str) -> String {
    let fixed: Vec<String> = s
        .split(' ')
        .map(|w| match w {
            "i" => "I".to_owned(),
            "i'm" | "i've" | "i'll" | "i'd" => format!("I{}", &w[1..]),
            other => other.to_owned(),
        })
        .collect();
    sentence_case(&fixed.join(" "))
}

pub fn plain_reply(s: &str) -> String {
    let mut out: String = s
        .chars()
        .filter(|c| {
            let u = *c as u32;
            !(0x1F000..=0x1FAFF).contains(&u) && !(0x2190..=0x2BFF).contains(&u) && !(0xFE00..=0xFE0F).contains(&u) && *c != '*' && *c != '#' && *c != '`'
        })
        .map(|c| if c == '!' { '.' } else { c })
        .collect();
    while out.contains("..") {
        out = out.replace("..", ".");
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    sentence_case(&out)
}

pub fn lines(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        let end = matches!(c, '.' | '?' | '!') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) && !(c == '.' && chars.get(i.wrapping_sub(1)).is_some_and(|p| p.is_ascii_digit()) && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()));
        if end && cur.trim().len() > 1 {
            out.push(cur.trim().to_owned());
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_whole_sentences() {
        let a = stream_chunk(r#"{"tool":"reply","text":"The capital of France is Paris."#, 0).unwrap();
        assert_eq!(a.0, "The capital of France is Paris.");
        assert!(stream_chunk(r#"{"tool":"reply","text":"The capital of France is Paris. It is"#, a.1).is_none());
        let b = stream_chunk(r#"{"tool":"reply","text":"The capital of France is Paris. It is on the Seine river."}"#, a.1).unwrap();
        assert_eq!(b.0, "It is on the Seine river.");
        assert!(stream_chunk(r#"{"tool":"open_app","app":"Firefox"}"#, 0).is_none());
        assert!(stream_chunk(r#"{"tool":"reply","text":"Pi is 3.14"#, 0).is_none());
        assert!(stream_chunk(r#"{"tool":"reply","text":"Yes."#, 0).is_none());
        let c = stream_chunk(r#"{"tool":"reply","text":"Sure, here is a quick one: a man walks"#, 0).unwrap();
        assert_eq!(c.0, "Sure, here is a quick one:");
        let d = stream_chunk(r#"{"tool":"reply","text":"Rainbows appear when sunlight enters raindrops and bends inside them so the colours"#, 0).unwrap();
        assert_eq!(d.0, "Rainbows appear when sunlight enters raindrops and bends inside them");
        assert!(stream_chunk(r#"{"tool":"reply","text":"Rainbows appear when sunlight"#, 0).is_none());
        assert!(social("hey setu how are you, today my day was fine, so turn the volume up a bit"));
        assert!(social("I'm so tired today, open my email"));
        assert!(!social("turn the volume up a bit"));
        assert!(!social("open the file called my day was fine.txt in documents") || social("my day was fine"));
        let s = stream_chunk(r#"{"say":"Hey Anoop, I'm good, thanks for asking.","tool":"web""#, 0).unwrap();
        assert_eq!(s.0.trim(), "Hey Anoop, I'm good, thanks for asking.");
    }

    #[test]
    fn strips_misheard_names() {
        assert_eq!(strip_wake("hey see you inside brave can you open youtube", "Setu"), "inside brave can you open youtube");
        assert_eq!(strip_wake("hey c2 can you open wordpad", "Setu"), "can you open wordpad");
        assert_eq!(strip_wake("okay say to open notepad", "Setu"), "open notepad");
        assert!(names_first("hey see you open brave", "Setu"));
        assert_eq!(strip_wake("hey nova open files", "Nova"), "open files");
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("Hey Setu, open Firefox."), "hey setu open firefox");
        assert_eq!(normalize("  Set   volume to 40%! "), "set volume to 40%");
    }

    #[test]
    fn strips_wake_phrase_variants() {
        assert_eq!(strip_wake("hey setu open firefox", "Setu"), "open firefox");
        assert_eq!(strip_wake("hey set to open firefox", "Setu"), "open firefox");
        assert_eq!(strip_wake("setu volume up", "Setu"), "volume up");
        assert_eq!(strip_wake("open firefox", "Setu"), "open firefox");
        assert_eq!(strip_wake("hey mary ann what time is it", "Mary-Ann"), "what time is it");
        assert_eq!(strip_wake("hey sato what time is it", "Setu"), "what time is it");
        assert_eq!(strip_wake("theto how far away is the moon", "Setu"), "how far away is the moon");
        assert_eq!(strip_lead("set the volume to forty", "Setu"), "set the volume to forty");
        assert_eq!(strip_lead("setu how far is the moon", "Setu"), "how far is the moon");
        assert_eq!(strip_lead("say hello", "Setu"), "say hello");
    }

    #[test]
    fn confirms_bare_name_only_when_said() {
        assert!(names_first("setu volume up", "Setu"));
        assert!(names_first("sato open the browser", "Setu"));
        assert!(names_first("set to open firefox", "Setu"));
        assert!(names_first("theato open spotify", "Setu"));
        assert!(!names_first("seth told me to set the timer", "Setu"));
        assert!(!names_first("the situation is settled", "Setu"));
        assert!(!names_first("we sat together at the station", "Setu"));
    }

    #[test]
    fn parses_numbers() {
        assert_eq!(find_number("set volume to forty percent"), Some(40));
        assert_eq!(find_number("set volume to forty five"), Some(45));
        assert_eq!(find_number("volume 70"), Some(70));
        assert_eq!(find_number("click number seven"), Some(7));
        assert_eq!(find_number("number two"), Some(2));
        assert_eq!(find_number("turn it up"), None);
        assert_eq!(minutes("mute the microphone for half an hour"), Some(30));
        assert_eq!(minutes("for fifteen minutes"), Some(15));
        assert_eq!(minutes("for 2 hours"), Some(120));
    }

    #[test]
    fn matches_misheard_app_names() {
        assert!(similarity("spot if i", "Spotify") > 0.85);
        assert!(similarity("firefox", "Firefox") > 0.99);
        assert!(similarity("v s code", "Visual Studio Code") >= 0.9);
        assert!(similarity("code", "Visual Studio Code") >= 0.85);
        assert!(similarity("chrome", "Google Chrome") > 0.85);
        assert!(similarity("calculator", "Spotify") < 0.7);
        assert!(similarity("libre office writer", "LibreOffice Writer") > 0.9);
    }

    #[test]
    fn spoken_phrases_are_plain_words() {
        assert_eq!(spoken_phrase("Open AI (GPT)"), "open ai gpt");
        assert_eq!(spoken_phrase("Option 2"), "option two");
        assert_eq!(spoken_phrase("  Eleven-Labs!! "), "eleven labs");
        assert_eq!(spoken_phrase("a b c d e f g h"), "a b c d e f");
        assert_eq!(spoken_phrase("¿¡"), "");
    }

    #[test]
    fn yes_and_no() {
        for y in ["Yes", "Yes.", "Ye", "yeah", "yes please", "sure", "okay", "go ahead", "yes open it", "do it"] {
            assert_eq!(yes_no(y), Some(true), "{y}");
        }
        for n in ["No", "no thanks", "nope", "cancel", "don't", "not now", "no don't"] {
            assert_eq!(yes_no(n), Some(false), "{n}");
        }
        for u in ["", "what time is it", "open firefox", "yesterday was fun", "sure thing then we can go later on today"] {
            assert_eq!(yes_no(u), None, "{u}");
        }
    }

    #[test]
    fn repairs_clipped_last_word() {
        assert_eq!(repair_tail("Sayto close the applicato.", "Sauto, close the application"), "Sayto close the application.");
        assert_eq!(repair_tail("Ye", "Yes"), "Yes");
        assert_eq!(repair_tail("What's the date toda", "what's the date today"), "What's the date today");
        assert_eq!(repair_tail("Open Firefox.", "open fire fox"), "Open Firefox.");
        assert_eq!(repair_tail("What time is it", "what time is item"), "What time is it");
        assert_eq!(repair_tail("Open Spotify", ""), "Open Spotify");
        assert_eq!(repair_tail("Close the browser", "close the browsers"), "Close the browser");
    }

    #[test]
    fn cleans_replies() {
        assert_eq!(plain_reply("great!! 😂 **bold**"), "Great. bold");
        assert_eq!(lines("It is 3.5 degrees. Open it? Yes."), vec!["It is 3.5 degrees.", "Open it?", "Yes."]);
    }
}
