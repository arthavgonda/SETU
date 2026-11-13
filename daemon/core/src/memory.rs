use crate::core::Core;
use crate::router::Intent;
use crate::text::{normalize, sentence_case};
use crate::tools::Outcome;
use anyhow::Result;
use rusqlite::params;
use serde_json::{Value, json};
use setu_common::protocol::now_ms;

const LAYER: &str = "memory";

const STOP: &[&str] = &[
    "the", "and", "for", "you", "your", "are", "was", "what", "whats", "what's", "who", "whom", "how", "why", "when", "where", "which", "that", "this", "with", "from", "have", "has", "had",
    "does", "did", "can", "could", "would", "should", "will", "tell", "me", "about", "do", "is", "it", "its", "of", "to", "in", "on", "at", "a", "an", "be", "my", "i", "im", "i'm",
    "know", "remember", "please", "setu", "hey", "there", "they", "them", "then", "than", "just", "any", "some",
];

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.into(), args }
}

fn strip<'a>(t: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| t.strip_prefix(p)).map(str::trim).filter(|r| !r.is_empty())
}

pub fn rules(t: &str, raw: &str) -> Option<Intent> {
    if matches!(
        t,
        "what do you know about me" | "what do you remember about me" | "what do you remember" | "what have i told you" | "what have i told you about me" | "show my memories" | "what are my memories"
    ) {
        return Some(act("memory.list", json!({})));
    }
    if matches!(t, "forget everything" | "forget everything about me" | "forget everything you know about me" | "clear your memory" | "delete my memories") {
        return Some(act("memory.clear", json!({})));
    }
    if let Some(rest) = strip(t, &["forget that ", "forget what i said about ", "forget about ", "please forget that ", "please forget ", "forget "])
        && !rest.contains("song")
        && !rest.contains("tab")
    {
        return Some(act("memory.forget", json!({ "text": rest })));
    }
    let lower_raw = raw.trim();
    let body = strip(t, &["please remember that ", "please remember ", "remember that ", "remember ", "don't forget that ", "dont forget that ", "don't forget ", "keep in mind that ", "keep in mind ", "note that "]);
    if let Some(b) = body
        && b.split_whitespace().count() >= 2
        && !b.starts_with("to ")
    {
        let text = original_tail(lower_raw, b).unwrap_or_else(|| b.to_owned());
        return Some(act("memory.add", json!({ "text": text })));
    }
    None
}

fn original_tail(raw: &str, normalized_tail: &str) -> Option<String> {
    let words = normalized_tail.split_whitespace().count();
    let raw_words: Vec<&str> = raw.split_whitespace().collect();
    (raw_words.len() >= words).then(|| raw_words[raw_words.len() - words..].join(" ").trim_end_matches(['.', '!']).to_owned())
}

pub fn personal_fact(t: &str) -> bool {
    let starts = ["my name is ", "my birthday is ", "my sister's name is ", "my brother's name is ", "my mother's name is ", "my mom's name is ", "my father's name is ", "my dad's name is ", "my wife's name is ", "my husband's name is ", "my friend's name is ", "my favourite ", "my favorite ", "i live in ", "i am from ", "i'm from ", "i work at ", "i work as ", "i study at ", "i'm allergic to ", "i am allergic to ", "call me "];
    starts.iter().any(|s| t.starts_with(s)) && !t.ends_with('?') && t.split_whitespace().count() <= 16
}

fn first_to_second(fact: &str) -> String {
    let mut out = Vec::new();
    for w in fact.split_whitespace() {
        let lw = w.to_lowercase();
        let rep = match lw.trim_end_matches([',', '.']) {
            "i" => "you",
            "i'm" => "you're",
            "i've" => "you've",
            "my" => "your",
            "me" => "you",
            "mine" => "yours",
            "am" => "are",
            "myself" => "yourself",
            _ => "",
        };
        if rep.is_empty() {
            out.push(w.to_owned());
        } else {
            let tail: String = w.chars().rev().take_while(|c| matches!(c, ',' | '.')).collect();
            out.push(format!("{rep}{tail}"));
        }
    }
    out.join(" ")
}

fn terms(text: &str) -> Vec<String> {
    let n = normalize(text);
    let mut v: Vec<String> = n
        .split_whitespace()
        .map(|w| w.trim_matches('\'').to_owned())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .collect();
    v.dedup();
    v.truncate(10);
    v
}

fn fts(text: &str) -> Option<String> {
    let t = terms(text);
    (!t.is_empty()).then(|| t.iter().map(|w| format!("\"{}\"*", w.replace('"', ""))).collect::<Vec<_>>().join(" OR "))
}

pub fn name_in(fact: &str) -> Option<String> {
    let t = normalize(fact);
    let rest = ["call me ", "my name is ", "you can call me ", "please call me "].iter().find_map(|p| t.strip_prefix(p))?;
    let words: Vec<&str> = rest.split_whitespace().take_while(|w| !matches!(*w, "and" | "but" | "please" | "from" | "now" | "instead")).take(3).collect();
    (!words.is_empty() && words.iter().all(|w| w.chars().all(|c| c.is_alphabetic() || c == '\''))).then(|| words.iter().map(|w| sentence_case(w)).collect::<Vec<_>>().join(" "))
}

pub fn user_name(core: &Core) -> Option<String> {
    if let Some(n) = all(core).iter().find_map(|(_, t)| name_in(t)) {
        return Some(n);
    }
    let account = crate::syspass::real_name().or_else(crate::syspass::user)?;
    let first = account.split_whitespace().next()?.trim_matches(|c: char| !c.is_alphabetic());
    (first.chars().count() >= 2 && first.chars().all(char::is_alphabetic)).then(|| sentence_case(&first.to_lowercase()))
}

pub fn add(core: &Core, text: &str, source: &str) -> Result<i64> {
    let t = text.trim().trim_end_matches(['.', '!']).to_owned();
    let key = normalize(&t);
    let src = source.to_owned();
    let now = now_ms();
    let renames = name_in(&t).is_some();
    core.db.write_sync(move |c| {
        if renames {
            let old: Vec<i64> = c
                .prepare("SELECT id, text FROM memories")?
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .flatten()
                .filter(|(_, x)| name_in(x).is_some() && normalize(x) != key)
                .map(|(id, _)| id)
                .collect();
            for id in old {
                c.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
            }
        }
        let existing: Option<i64> = c
            .prepare("SELECT id, text FROM memories")?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .flatten()
            .find(|(_, x)| normalize(x) == key)
            .map(|(id, _)| id);
        if let Some(id) = existing {
            c.execute("UPDATE memories SET time = ?2 WHERE id = ?1", params![id, now])?;
            return Ok(id);
        }
        c.execute("INSERT INTO memories(time, text, source) VALUES(?1, ?2, ?3)", params![now, t, src])?;
        Ok(c.last_insert_rowid())
    })
}

pub fn all(core: &Core) -> Vec<(i64, String)> {
    core.db
        .read(|c| {
            let mut st = c.prepare("SELECT id, text FROM memories ORDER BY time DESC LIMIT 200")?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
            Ok(v)
        })
        .unwrap_or_default()
}

pub fn search(core: &Core, query: &str, limit: usize) -> Vec<(i64, String)> {
    let Some(q) = fts(query) else { return Vec::new() };
    core.db
        .read(|c| {
            let mut st = c.prepare("SELECT m.id, m.text FROM memories_fts f JOIN memories m ON m.id = f.rowid WHERE memories_fts MATCH ?1 ORDER BY bm25(memories_fts) LIMIT ?2")?;
            let v = st.query_map(params![q, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
            Ok(v)
        })
        .unwrap_or_default()
}

pub fn earlier(core: &Core, query: &str, skip_chat: Option<i64>, limit: usize) -> Vec<String> {
    let Some(q) = fts(query) else { return Vec::new() };
    let skip = skip_chat.unwrap_or(-1);
    let rows: Vec<(i64, i64, String)> = core
        .db
        .read(|c| {
            let mut st = c.prepare(
                "SELECT m.id, m.chat, m.text FROM messages_fts f JOIN messages m ON m.id = f.rowid WHERE messages_fts MATCH ?1 AND m.chat != ?2 AND m.role = 'user' ORDER BY bm25(messages_fts) LIMIT ?3",
            )?;
            let v = st.query_map(params![q, skip, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.flatten().collect();
            Ok(v)
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (id, chat, asked) in rows {
        let answer: Option<String> = core
            .db
            .read(|c| Ok(c.query_row("SELECT text FROM messages WHERE chat = ?1 AND id > ?2 AND role = 'assistant' ORDER BY id LIMIT 1", params![chat, id], |r| r.get(0)).ok()))
            .unwrap_or(None);
        let a: String = answer.unwrap_or_default().chars().take(200).collect();
        let q: String = asked.chars().take(160).collect();
        out.push(if a.is_empty() { format!("You asked: {q}") } else { format!("You asked: {q} / Setu said: {a}") });
    }
    out
}

pub fn about_user(text: &str) -> bool {
    let t = format!(" {} ", normalize(text));
    [" my ", " me ", " i ", " mine ", " myself ", " i'm "].iter().any(|w| t.contains(w))
}

pub fn context(core: &Core, query: &str, skip_chat: Option<i64>) -> String {
    let mut facts: Vec<String> = search(core, query, 4).into_iter().map(|(_, t)| t).collect();
    if about_user(query) {
        for (_, t) in all(core).into_iter().take(6) {
            if !facts.contains(&t) {
                facts.push(t);
            }
        }
        facts.truncate(8);
    }
    let past = earlier(core, query, skip_chat, 2);
    let mut out = String::new();
    if !facts.is_empty() {
        out.push_str("What the user has told you before:\n");
        for f in &facts {
            out.push_str(&format!("- {}\n", first_to_second(f)));
        }
    }
    if !past.is_empty() {
        out.push_str("From earlier chats:\n");
        for p in &past {
            out.push_str(&format!("- {p}\n"));
        }
    }
    out
}

pub fn facts_line(core: &Core, query: &str) -> String {
    let mut facts: Vec<String> = search(core, query, 3).into_iter().map(|(_, t)| first_to_second(&t)).collect();
    if facts.is_empty() && about_user(query) {
        facts = all(core).into_iter().take(4).map(|(_, t)| first_to_second(&t)).collect();
    }
    facts.join("; ")
}

fn question(text: &str) -> bool {
    let n = normalize(text);
    text.trim_end().ends_with('?') || ["what", "who", "when", "where", "which", "how", "do you know", "tell me", "remind me what", "whats", "what's"].iter().any(|w| n.starts_with(w))
}

pub fn direct_answer(core: &Core, text: &str) -> Option<String> {
    if !about_user(text) || !question(text) {
        return None;
    }
    let want = terms(text);
    let hits = search(core, text, 3);
    let (_, best) = hits.first()?;
    let have = terms(best);
    let shared = want.iter().filter(|w| have.iter().any(|h| h.starts_with(w.as_str()) || w.starts_with(h.as_str()))).count();
    if shared == 0 || shared * 2 < want.len().min(4) {
        return None;
    }
    let mut line = sentence_case(&first_to_second(best));
    if !line.ends_with(['.', '!', '?']) {
        line.push('.');
    }
    Some(line)
}

pub fn keep_case(original: &str, lowered: &str) -> String {
    let o = original.to_lowercase();
    let l = lowered.to_lowercase();
    match o.find(&l) {
        Some(i) if original.is_char_boundary(i) && original.is_char_boundary(i + l.len()) => original[i..i + l.len()].to_owned(),
        _ => {
            let words: Vec<&str> = original.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')).collect();
            lowered.split_whitespace().map(|w| words.iter().find(|x| x.to_lowercase() == w.to_lowercase()).copied().unwrap_or(w)).collect::<Vec<_>>().join(" ")
        }
    }
}

pub fn follow_up(text: &str) -> bool {
    let n = normalize(text);
    let t = format!(" {n} ");
    let leads = ["and ", "what about ", "how about ", "why ", "what else", "tell me more", "more about ", "is it ", "are they ", "was it ", "who else"];
    leads.iter().any(|l| n.starts_with(l))
        || [" he ", " she ", " it ", " they ", " him ", " her ", " them ", " his ", " its ", " their ", " that one ", " there ", " those ", " these ", " the same "].iter().any(|w| t.contains(w))
}

pub fn route_context(core: &Core, inter: u64, text: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if about_user(text) {
        let facts = facts_line(core, text);
        if !facts.is_empty() {
            parts.push(format!("What you know about the user: {facts}."));
        }
    }
    if follow_up(text)
        && let Some(chat) = core.threads.recent(inter, 180_000)
    {
        let convo = crate::threads::context(core, chat, 2);
        if !convo.is_empty() {
            parts.push(format!("Earlier in this conversation:\n{}", convo.trim_end()));
        }
    }
    let joined = parts.join("\n");
    joined.chars().rev().take(360).collect::<Vec<_>>().into_iter().rev().collect()
}

pub fn run(core: &Core, tool: &str, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    match tool {
        "memory.add" => {
            let text = s("text");
            if text.is_empty() {
                return Outcome::fail("Tell me what to remember.", LAYER);
            }
            match add(core, &text, &s("source")) {
                Ok(_) if name_in(&text).is_some() => Outcome::ok_with("Remembered your name", LAYER, None).with_reply(format!("Okay, I'll call you {}.", name_in(&text).unwrap_or_default())),
                Ok(_) => Outcome::ok_with("Remembered it", LAYER, None).with_reply(format!("Okay, I'll remember that {}.", first_to_second(&text).trim_end_matches('.'))),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "memory.forget" => {
            let text = s("text");
            let hits = search(core, &text, 3);
            let Some((id, what)) = hits.into_iter().next() else { return Outcome::fail("I don't remember anything like that.", LAYER) };
            let r = core.db.write_sync(move |c| Ok(c.execute("DELETE FROM memories WHERE id = ?1", params![id])?));
            match r {
                Ok(_) => Outcome::ok_with("Forgot it", LAYER, None).with_reply(format!("Okay, I forgot that {}.", first_to_second(&what).trim_end_matches('.'))),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "memory.clear" => {
            let r = core.db.write_sync(|c| Ok(c.execute("DELETE FROM memories", [])?));
            match r {
                Ok(n) if n > 0 => Outcome::ok_with("Forgot everything", LAYER, None).with_reply(format!("Done. I forgot the {n} things you told me.")),
                Ok(_) => Outcome::ok_with("Nothing to forget", LAYER, None).with_reply("I wasn't remembering anything yet.".into()),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        _ => {
            let list = all(core);
            if list.is_empty() {
                return Outcome::ok_with("Nothing remembered yet", LAYER, None).with_reply("You haven't asked me to remember anything yet. Say remember, then what to keep in mind.".into());
            }
            let items: Vec<String> = list.iter().take(6).map(|(_, t)| first_to_second(t).trim_end_matches('.').to_owned()).collect();
            let more = if list.len() > 6 { format!(" And {} more.", list.len() - 6) } else { String::new() };
            let joined = match items.split_last() {
                Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
                _ => items.join(""),
            };
            Outcome::ok_with("Listed what I remember", LAYER, None).with_reply(format!("I remember that {}.{more}", joined.trim_end_matches('.')))
        }
    }
}

pub fn label(tool: &str) -> String {
    match tool {
        "memory.add" => "Remembering".into(),
        "memory.forget" => "Forgetting".into(),
        "memory.clear" => "Forgetting everything".into(),
        _ => "Remembering what you told me".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(i: Option<Intent>) -> (String, Value) {
        match i {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn understands_remember_and_forget() {
        let (t, a) = tool(rules("remember that my sister's name is priya", "Remember that my sister's name is Priya."));
        assert_eq!(t, "memory.add");
        assert_eq!(a["text"], "my sister's name is Priya");
        assert_eq!(tool(rules("what do you know about me", "")).0, "memory.list");
        assert_eq!(tool(rules("forget that my sister's name is priya", "")).0, "memory.forget");
        assert!(rules("remember to buy milk", "").is_none());
        assert!(rules("open firefox", "").is_none());
        assert!(personal_fact("my name is anoop"));
        assert_eq!(name_in("call me raju").as_deref(), Some("Raju"));
        assert_eq!(name_in("My name is Anoop Mehta").as_deref(), Some("Anoop Mehta"));
        assert_eq!(name_in("you can call me boss from now on").as_deref(), Some("Boss"));
        assert_eq!(name_in("my name is 42"), None);
        assert_eq!(name_in("i live in pune"), None);
        assert!(personal_fact("i live in pune"));
        assert!(!personal_fact("my volume is too low"));
        assert_eq!(first_to_second("my sister's name is Priya and I'm a coder"), "your sister's name is Priya and you're a coder");
        assert_eq!(fts("what is my sister's name").as_deref(), Some("\"sister's\"* OR \"name\"*"));
        assert!(about_user("what's my dog's name"));
        assert!(!about_user("what is the capital of france"));
        assert!(follow_up("how old is he"));
        assert_eq!(keep_case("Remember that my sister's name is Priya.", "my sister's name is priya"), "my sister's name is Priya");
        assert_eq!(keep_case("remember: my dog is called Bruno", "my dog is called bruno"), "my dog is called Bruno");
        assert!(follow_up("what about spain"));
        assert!(!follow_up("open firefox"));
    }
}
