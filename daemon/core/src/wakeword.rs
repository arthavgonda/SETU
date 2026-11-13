use crate::core::Core;
use crate::settings::Settings;
use crate::text::normalize;
use serde_json::{Map, Value, json};
use setu_sherpa::TransducerPaths;

const KEY: &str = "wake.learned";
const MAX: usize = 6;
const LISTENER: &str = "asr-kroko-en";

pub fn listener_paths(core: &Core) -> Option<TransducerPaths> {
    if !core.store.is_installed(LISTENER) {
        return None;
    }
    let f = |k: &str| core.store.file_str(LISTENER, k).ok();
    Some(TransducerPaths { encoder: f("encoder")?, decoder: f("decoder")?, joiner: f("joiner")?, tokens: f("tokens")? })
}

pub fn learned(s: &Settings) -> Vec<String> {
    s.get(KEY).as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

pub fn learn(core: &Core, text: &str) -> Option<usize> {
    let c = learnable(text)?;
    if prefix(&c, &[]).is_some_and(|(_, n)| n == c.split_whitespace().count()) {
        return None;
    }
    let mut list = learned(&core.settings);
    if list.contains(&c) {
        return None;
    }
    list.push(c);
    while list.len() > MAX {
        list.remove(0);
    }
    let v = json!(list);
    core.settings.set_internal(KEY, v.clone());
    let mut m = Map::new();
    m.insert(KEY.into(), v);
    core.db.save_settings(m);
    Some(list.len())
}

pub fn forget(core: &Core) {
    crate::wakeadapt::clear(core);
    core.settings.set_internal(KEY, json!([]));
    let mut m = Map::new();
    m.insert(KEY.into(), json!([]));
    core.db.save_settings(m);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Wake,
    Name,
}

const STRONG_LEAD: &[&str] = &["hey", "hay", "hi", "hai", "high", "okay", "ok", "okey", "okie", "oka", "occa", "eh", "hei"];
const GLUED: &[&str] = &["hey", "hay", "hi", "he", "a", "oka", "oga", "okay", "occa", "aka", "ha"];
const WEAK_LEAD: &[&str] = &["he", "a", "hey's", "hes"];
const STRONG: &[&str] = &[
    "setu", "sethu", "setoo", "settu", "setto", "seto", "situ", "saitu", "saito", "sato", "satu", "saytu", "sayto", "saytoo", "seytu", "sethoo", "setul", "situs",
];
const FIRST: &[&str] = &["say", "said", "set", "sat", "sit", "seth", "sed", "sep", "see", "sae", "sate", "sayed", "sad", "sent"];
const SECOND: &[&str] = &["to", "too", "two", "tu", "though", "do", "doo", "no", "you", "toh", "tho"];
const WEAK: &[&str] = &["set", "sat", "sep", "sant", "said", "sett", "city", "saito", "sit"];
const FRAGMENT: &[&str] = &["se", "sa", "set", "sat", "said", "say", "see", "sed"];

fn joined_ok(a: &str, b: &str) -> bool {
    FIRST.contains(&a) && SECOND.contains(&b)
}

pub fn setu_like(word: &str) -> bool {
    if word.len() < 3 || word.len() > 7 || !word.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let k = crate::names::key(word).replace('H', "").replacen("ANT", "AT", 1);
    matches!(k.as_str(), "SATV" | "XATV" | "SAATV")
}

const NARROW_ONE: &[&str] = &["etu", "eto", "edu", "itu", "ito", "atu", "ettu", "eetu", "etoo", "tu", "too", "two", "to", "ethu", "aitu", "ateu", "seb", "sab", "sef"];
const NARROW_FIRST: &[&str] = &["a", "it", "at", "eat", "eight", "ate", "et", "ed", "add", "and", "said", "say", "set", "sat", "sit", "seat"];

pub fn prefix(text: &str, learned: &[String]) -> Option<(Kind, usize)> {
    prefix_with(text, learned, false)
}

pub fn prefix_with(text: &str, learned: &[String], narrow: bool) -> Option<(Kind, usize)> {
    let t = normalize(text);
    let w: Vec<&str> = t.split_whitespace().collect();
    if w.is_empty() {
        return None;
    }
    let learned: Vec<Vec<String>> = learned.iter().map(|v| normalize(v).split_whitespace().map(str::to_owned).collect()).filter(|v: &Vec<String>| !v.is_empty()).collect();
    for l in &learned {
        if w.len() >= l.len() && w[..l.len()].iter().zip(l).all(|(a, b)| *a == b.as_str()) {
            let lead = STRONG_LEAD.contains(&l[0].as_str()) || WEAK_LEAD.contains(&l[0].as_str());
            return Some((if lead { Kind::Wake } else { Kind::Name }, l.len()));
        }
    }
    let strong_lead = STRONG_LEAD.contains(&w[0]);
    let weak_lead = WEAK_LEAD.contains(&w[0]);
    if !strong_lead && !weak_lead {
        let first = w[0];
        for k in 1..=first.len().saturating_sub(2) {
            if !first.is_char_boundary(k) || !GLUED.contains(&&first[..k]) {
                continue;
            }
            let rest = &first[k..];
            if STRONG.contains(&rest) || setu_like(rest) {
                return Some((Kind::Wake, 1));
            }
            if FRAGMENT.contains(&rest) && w.get(1).is_some_and(|b| SECOND.contains(b)) {
                return Some((Kind::Wake, 2));
            }
        }
        if w.len() >= 2 && first.len() <= 5 && STRONG.contains(&w[1]) {
            return Some((Kind::Wake, 2));
        }
    }
    let start = usize::from(strong_lead || weak_lead);
    let kind = if start == 1 { Kind::Wake } else { Kind::Name };
    let at = |i: usize| w.get(start + i).copied();
    let a = at(0)?;
    if STRONG.contains(&a) {
        return Some((kind, start + 1));
    }
    if let Some(b) = at(1)
        && joined_ok(a, b) {
            return Some((kind, start + 2));
        }
    if strong_lead && WEAK.contains(&a) {
        return Some((Kind::Wake, start + 1));
    }
    if strong_lead && (setu_like(a) || a == "setup" && at(1).is_some_and(|b| b != "the" && b != "a")) {
        return Some((Kind::Wake, start + 1));
    }
    if narrow && strong_lead {
        if NARROW_ONE.contains(&a) {
            return Some((Kind::Wake, start + 1));
        }
        if let Some(b) = at(1)
            && NARROW_FIRST.contains(&a)
            && SECOND.contains(&b)
        {
            return Some((Kind::Wake, start + 2));
        }
    }
    None
}

pub fn weak(raw: &str) -> bool {
    let lower = raw.trim().to_lowercase();
    let mut words = lower.split_whitespace();
    let (Some(a), Some(b)) = (words.next(), words.next()) else { return false };
    let lead = a.trim_end_matches(',');
    let name = b.trim_end_matches([',', '.', '!', '?']);
    let paused = b.ends_with([',', '.', '!', '?']) || words.next().is_none();
    (STRONG_LEAD.contains(&lead) || WEAK_LEAD.contains(&lead)) && (WEAK.contains(&name) || name == "city") && paused
}

pub fn strip_with(text: &str, learned: &[String], narrow: bool) -> Option<String> {
    let t = normalize(text);
    let (_, n) = prefix_with(&t, learned, narrow)?;
    Some(t.split_whitespace().skip(n).collect::<Vec<_>>().join(" "))
}

pub fn learnable(text: &str) -> Option<String> {
    let t = normalize(text);
    let w: Vec<&str> = t.split_whitespace().collect();
    if w.len() < 2 {
        return None;
    }
    let lead = STRONG_LEAD.contains(&w[0]) || WEAK_LEAD.contains(&w[0]);
    if !lead {
        return None;
    }
    let take = if w.len() >= 3 && SECOND.contains(&w[2]) { 3 } else { 2 };
    let cand: Vec<&str> = w[..take].to_vec();
    if cand[1..].iter().any(|x| x.len() > 7 || crate::names::is_common(x) && !FIRST.contains(x) && !SECOND.contains(x)) {
        return None;
    }
    Some(cand.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake(t: &str) -> bool {
        prefix(t, &[]).is_some_and(|(k, _)| k == Kind::Wake)
    }

    #[test]
    fn hears_how_people_say_it() {
        for t in [
            "Hey, Sat",
            "Hey set, open the browser",
            "Hey, Sep, what time is it",
            "Hey, Sant",
            "He said too, what time",
            "He said to, open the browser, please",
            "He said though, what time is",
            "He said no, open the browser",
            "Hey Setu",
            "hey situ what time is it",
            "okay setu",
            "hi saito",
            "Hey, say too",
            "a setu open youtube",
        ] {
            assert!(wake(t), "{t}");
        }
    }

    #[test]
    fn ignores_ordinary_speech() {
        for t in [
            "They said it was true",
            "Hey, say it again",
            "He set up the table",
            "He said, play some music",
            "Hey, it's a beautiful day",
            "Can you set the timer",
            "Let's settle this tomorrow",
            "The city is beautiful at night",
            "I went to set up last year",
            "He sell up the table",
            "Hey, sir, play some music",
        ] {
            assert!(!wake(t), "{t}");
        }
    }

    #[test]
    fn weak_forms_need_a_pause() {
        assert!(weak("He said, what time is it"));
        assert!(weak("He said"));
        assert!(weak("A city, open the browser"));
        assert!(weak("He set"));
        assert!(!weak("He said play some music"));
        assert!(!weak("He set up the table"));
        assert!(!weak("They said, it was true"));
    }

    #[test]
    fn bare_name_and_stripping() {
        assert_eq!(prefix("setu open the browser", &[]), Some((Kind::Name, 1)));
        assert_eq!(prefix("say to open the browser", &[]), Some((Kind::Name, 2)));
        assert_eq!(strip_with("He said to, open the browser, please", &[], false).as_deref(), Some("open the browser please"));
        assert_eq!(strip_with("Hey Sat what time is it", &[], false).as_deref(), Some("what time is it"));
        assert_eq!(strip_with("open the browser", &[], false), None);
    }

    #[test]
    fn hears_names_that_sound_like_setu() {
        for t in ["hey satto open firefox", "hey sadhu what time is it", "hey sandu", "okay sethu open youtube", "hi shetu", "hey setup open the browser", "hey situ", "hey saddu play music", "high set open youtube", "oka said to play some music", "ogasetu play some music", "edge setu open youtube", "occase to play some music"] {
            assert!(wake(t), "{t}");
        }
        for t in ["hey sarah how are you", "hey seth good morning", "hey sasha lets go", "hey sam open the door", "hey siri what is the weather", "hey setup the meeting", "hey settle down", "let us settle this tomorrow", "the city is beautiful at night"] {
            assert!(!wake(t), "{t}");
        }
    }

    #[test]
    fn narrowband_forms_only_when_narrow() {
        assert_eq!(prefix_with("hey etu open firefox", &[], true), Some((Kind::Wake, 2)));
        assert_eq!(prefix_with("hey it to open firefox", &[], true), Some((Kind::Wake, 3)));
        assert_eq!(prefix_with("hey etu open firefox", &[], false), None);
        assert_eq!(prefix_with("hey it is a beautiful day", &[], true), None);
        assert_eq!(strip_with("hey a two what time is it", &[], true).as_deref(), Some("what time is it"));
        assert_eq!(prefix_with("hey seb what time is it", &[], true), Some((Kind::Wake, 2)));
        assert_eq!(prefix_with("hey seb good morning", &[], false), None);
    }

    #[test]
    fn learned_spellings_win() {
        let l = vec!["hey sedu".to_owned()];
        assert_eq!(prefix("hey sedu what time is it", &l), Some((Kind::Wake, 2)));
        assert_eq!(learnable("hey sedu what time is it").as_deref(), Some("hey sedu"));
        assert_eq!(learnable("he said too open the door").as_deref(), Some("he said too"));
        assert_eq!(learnable("open the door"), None);
    }
}

#[cfg(test)]
mod eval {
    #[test]
    #[ignore]
    fn score_transcripts() {
        let Ok(dir) = std::env::var("WAKE_TSV") else { return };
        for kind in ["wb", "nb", "q", "qa"] {
            let Ok(text) = std::fs::read_to_string(format!("{dir}/asr-{kind}.tsv")) else { continue };
            for narrow in [false, true] {
                let (mut tp, mut np, mut fp, mut nn) = (0, 0, 0, 0);
                let mut misses = Vec::new();
                let mut falses = Vec::new();
                let mut weak_false = 0;
                for line in text.lines() {
                    let Some((name, said)) = line.split_once('\t') else { continue };
                    let strong = super::prefix_with(said, &[], narrow).is_some_and(|(k, _)| k == super::Kind::Wake);
                    let hit = strong || super::weak(said);
                    if !name.starts_with("pos") && hit && !strong {
                        weak_false += 1;
                    }
                    if name.starts_with("pos") {
                        np += 1;
                        if hit { tp += 1 } else { misses.push(said.to_owned()) }
                    } else {
                        nn += 1;
                        if hit { fp += 1; falses.push(said.to_owned()) }
                    }
                }
                println!("{kind} narrow={narrow}: recall {tp}/{np}, false {fp}/{nn} ({weak_false} only with the voice check)");
                println!("  misses: {misses:?}");
                println!("  false: {falses:?}");
            }
        }
    }
}
