use crate::core::Core;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use setu_common::protocol::now_ms;
use std::collections::HashMap;
use std::sync::Mutex;

const FRESH_MS: i64 = 15 * 60_000;
const CONTEXT_MS: i64 = 5 * 60_000;

#[derive(Default)]
pub struct Threads {
    current: Mutex<Option<(i64, i64)>>,
    bound: Mutex<HashMap<u64, i64>>,
}

impl Threads {
    pub fn focus(&self, chat: Option<i64>) {
        *self.current.lock().unwrap() = chat.map(|c| (c, now_ms()));
    }

    pub fn bind(&self, inter: u64, chat: i64) {
        let mut g = self.bound.lock().unwrap();
        if g.len() > 64 {
            g.clear();
        }
        g.insert(inter, chat);
    }

    pub fn recent(&self, inter: u64, within: i64) -> Option<i64> {
        if let Some(c) = self.bound.lock().unwrap().get(&inter) {
            return Some(*c);
        }
        let g = self.current.lock().unwrap();
        g.filter(|(_, at)| now_ms() - at < within).map(|(c, _)| c)
    }

    pub fn take(&self, inter: u64) -> Option<i64> {
        self.bound.lock().unwrap().remove(&inter)
    }

    fn fresh(&self) -> Option<i64> {
        let g = self.current.lock().unwrap();
        g.filter(|(_, at)| now_ms() - at < FRESH_MS).map(|(c, _)| c)
    }

    fn touch(&self, chat: i64) {
        *self.current.lock().unwrap() = Some((chat, now_ms()));
    }

    fn forget(&self, chat: Option<i64>) {
        let mut g = self.current.lock().unwrap();
        if chat.is_none() || g.is_some_and(|(c, _)| Some(c) == chat) {
            *g = None;
        }
    }
}

pub fn title_from(text: &str) -> String {
    let clean: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = clean.split(['.', '?', '!', '\n']).map(str::trim).find(|s| !s.is_empty()).unwrap_or("").to_owned();
    let mut out = String::new();
    for w in first.split_whitespace() {
        if out.chars().count() + w.chars().count() + 1 > 48 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.is_empty() {
        out = first.chars().take(48).collect();
    }
    let mut c = out.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "New chat".into(),
    }
}

fn summary(c: &rusqlite::Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    c.query_row(
        "SELECT c.id, c.title, c.created, c.updated, (SELECT text FROM messages m WHERE m.chat = c.id ORDER BY m.id DESC LIMIT 1), (SELECT count(*) FROM messages m WHERE m.chat = c.id) FROM chats c WHERE c.id = ?1",
        params![id],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)?, "created": r.get::<_, i64>(2)?, "updated": r.get::<_, i64>(3)?,
                "preview": r.get::<_, Option<String>>(4)?.unwrap_or_default().chars().take(120).collect::<String>(), "count": r.get::<_, i64>(5)?
            }))
        },
    )
    .optional()
}

pub fn exists(core: &Core, chat: i64) -> bool {
    core.db.read(|c| Ok(c.query_row("SELECT 1 FROM chats WHERE id = ?1", params![chat], |_| Ok(())).optional()?.is_some())).unwrap_or(false)
}

pub fn create(core: &Core, title: &str) -> Result<i64> {
    let t = title.to_owned();
    let now = now_ms();
    let id = core.db.write_sync(move |c| {
        c.execute("INSERT INTO chats(title, created, updated) VALUES(?1, ?2, ?2)", params![t, now])?;
        Ok(c.last_insert_rowid())
    })?;
    core.threads.touch(id);
    Ok(id)
}

pub fn add(core: &Core, chat: i64, role: &str, text: &str, inter: Option<u64>) -> Result<Value> {
    let (r, t) = (role.to_owned(), text.trim().to_owned());
    let now = now_ms();
    let (msg, sum) = core.db.write_sync(move |c| {
        c.execute("INSERT INTO messages(chat, time, role, text, interaction) VALUES(?1, ?2, ?3, ?4, ?5)", params![chat, now, r, t, inter.map(|i| i as i64)])?;
        let id = c.last_insert_rowid();
        c.execute("UPDATE chats SET updated = ?2 WHERE id = ?1", params![chat, now])?;
        let msg = json!({ "id": id, "chat": chat, "time": now, "role": r, "text": t, "interaction": inter.map(|i| i.to_string()) });
        Ok((msg, summary(c, chat)?))
    })?;
    core.threads.touch(chat);
    core.hub.emit("chat.message", msg.clone());
    if let Some(s) = sum {
        core.hub.emit("chat.changed", json!({ "chat": s }));
    }
    Ok(msg)
}

pub fn list(core: &Core, limit: i64, query: &str) -> Result<Value> {
    let q = query.trim().to_lowercase();
    let items = core.db.read(|c| {
        let mut st = c.prepare("SELECT id FROM chats ORDER BY updated DESC LIMIT ?1")?;
        let ids: Vec<i64> = st.query_map(params![if q.is_empty() { limit } else { 2000 }], |r| r.get(0))?.flatten().collect();
        let mut out = Vec::new();
        for id in ids {
            if let Some(s) = summary(c, id)? {
                if !q.is_empty() {
                    let title = s["title"].as_str().unwrap_or_default().to_lowercase();
                    let hit = title.contains(&q)
                        || c.query_row("SELECT 1 FROM messages WHERE chat = ?1 AND lower(text) LIKE ?2 LIMIT 1", params![id, format!("%{q}%")], |_| Ok(())).optional()?.is_some();
                    if !hit {
                        continue;
                    }
                }
                out.push(s);
                if out.len() as i64 >= limit {
                    break;
                }
            }
        }
        Ok(out)
    })?;
    Ok(json!({ "items": items }))
}

pub fn get(core: &Core, chat: i64) -> Result<Value> {
    core.db.read(|c| {
        let Some(s) = summary(c, chat)? else { return Ok(json!({ "chat": null, "messages": [] })) };
        let mut st = c.prepare("SELECT id, time, role, text, interaction FROM messages WHERE chat = ?1 ORDER BY id")?;
        let msgs: Vec<Value> = st
            .query_map(params![chat], |r| {
                Ok(json!({ "id": r.get::<_, i64>(0)?, "chat": chat, "time": r.get::<_, i64>(1)?, "role": r.get::<_, String>(2)?, "text": r.get::<_, String>(3)?, "interaction": r.get::<_, Option<i64>>(4)?.map(|i| i.to_string()) }))
            })?
            .flatten()
            .collect();
        Ok(json!({ "chat": s, "messages": msgs }))
    })
}

pub fn rename(core: &Core, chat: i64, title: &str) -> Result<Value> {
    let t: String = title.trim().chars().take(80).collect();
    let s = core.db.write_sync(move |c| {
        c.execute("UPDATE chats SET title = ?2 WHERE id = ?1", params![chat, t])?;
        Ok(summary(c, chat)?)
    })?;
    if let Some(s) = &s {
        core.hub.emit("chat.changed", json!({ "chat": s }));
    }
    Ok(s.unwrap_or(Value::Null))
}

pub fn delete(core: &Core, chat: Option<i64>) -> Result<()> {
    core.db.write_sync(move |c| {
        match chat {
            Some(id) => {
                c.execute("DELETE FROM messages WHERE chat = ?1", params![id])?;
                c.execute("DELETE FROM chats WHERE id = ?1", params![id])?;
            }
            None => c.execute_batch("DELETE FROM messages; DELETE FROM chats;")?,
        }
        Ok(())
    })?;
    core.threads.forget(chat);
    core.hub.emit("chat.changed", json!({ "deleted": chat, "all": chat.is_none() }));
    Ok(())
}

pub fn context(core: &Core, chat: i64, turns: usize) -> String {
    let since = now_ms() - CONTEXT_MS;
    let rows: Vec<(String, String)> = core
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT role, text FROM messages WHERE chat = ?1 AND time >= ?2 ORDER BY id DESC LIMIT ?3")?;
            let v = st.query_map(params![chat, since, (turns * 2) as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
            Ok(v)
        })
        .unwrap_or_default();
    let mut out = String::new();
    for (role, text) in rows.into_iter().rev() {
        let who = if role == "user" { "User" } else { "Setu" };
        let t: String = text.chars().take(220).collect();
        out.push_str(&format!("{who}: {t}\n"));
    }
    out
}

pub fn record_turn(core: &Core, inter: u64, said: &str, answer: &str, user_logged: bool) {
    if !core.settings.bool("history.keep") {
        core.threads.take(inter);
        return;
    }
    let bound = core.threads.take(inter);
    let chat = match bound.or_else(|| core.threads.fresh()).filter(|c| exists(core, *c)) {
        Some(c) => c,
        None => {
            if said.trim().is_empty() {
                return;
            }
            match create(core, &title_from(said)) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("a chat could not be saved: {e}");
                    return;
                }
            }
        }
    };
    if !user_logged && !said.trim().is_empty() {
        let _ = add(core, chat, "user", said, Some(inter));
    }
    if !answer.trim().is_empty() {
        let _ = add(core, chat, "assistant", answer, Some(inter));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_short_sentences() {
        assert_eq!(title_from("what's the weather in delhi today? and tomorrow"), "What's the weather in delhi today");
        assert_eq!(title_from("   open firefox   "), "Open firefox");
        assert_eq!(title_from(""), "New chat");
        assert!(title_from(&"word ".repeat(40)).chars().count() <= 48);
    }
}
