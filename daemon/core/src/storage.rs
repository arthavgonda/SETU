use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Sender, bounded};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE history(
        id INTEGER PRIMARY KEY,
        time INTEGER NOT NULL,
        said TEXT NOT NULL,
        did TEXT NOT NULL,
        result TEXT NOT NULL,
        layer TEXT NOT NULL,
        duration_ms INTEGER NOT NULL,
        app TEXT,
        action TEXT,
        undoable INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX history_time ON history(time);
    CREATE VIRTUAL TABLE history_fts USING fts5(said, did, app, content='history', content_rowid='id');
    CREATE TRIGGER history_ai AFTER INSERT ON history BEGIN
        INSERT INTO history_fts(rowid, said, did, app) VALUES (new.id, new.said, new.did, coalesce(new.app, ''));
    END;
    CREATE TRIGGER history_ad AFTER DELETE ON history BEGIN
        INSERT INTO history_fts(history_fts, rowid, said, did, app) VALUES ('delete', old.id, old.said, old.did, coalesce(old.app, ''));
    END;
    CREATE TABLE apps(
        rid INTEGER PRIMARY KEY,
        id TEXT NOT NULL UNIQUE,
        name TEXT NOT NULL,
        source TEXT NOT NULL,
        path TEXT NOT NULL,
        exec TEXT NOT NULL,
        aliases TEXT NOT NULL DEFAULT '[]',
        opened INTEGER NOT NULL DEFAULT 0,
        hidden INTEGER NOT NULL DEFAULT 0,
        roles TEXT NOT NULL DEFAULT '[]',
        manual INTEGER NOT NULL DEFAULT 0,
        present INTEGER NOT NULL DEFAULT 1
    );
    CREATE TABLE missing_apps(name TEXT PRIMARY KEY, count INTEGER NOT NULL, last_asked INTEGER NOT NULL, dismissed INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE preferences(key TEXT PRIMARY KEY, value TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 1, updated INTEGER NOT NULL);
    CREATE TABLE corrections(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, heard TEXT NOT NULL, meant TEXT NOT NULL, history_id INTEGER);
    CREATE TABLE models(id TEXT PRIMARY KEY, files TEXT NOT NULL, bytes INTEGER NOT NULL, verified_at INTEGER NOT NULL);
    CREATE TABLE benchmarks(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, items TEXT NOT NULL);
    CREATE TABLE plugins(id TEXT PRIMARY KEY, enabled INTEGER NOT NULL, installed INTEGER NOT NULL);
    CREATE TABLE speaker(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, embedding BLOB NOT NULL);
    "#,
    r#"
    CREATE TABLE notes(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, text TEXT NOT NULL);
    CREATE TABLE todos(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, text TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, done_at INTEGER);
    CREATE TABLE reminders(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, due INTEGER NOT NULL, text TEXT NOT NULL, fired INTEGER NOT NULL DEFAULT 0);
    CREATE INDEX reminders_due ON reminders(due);
    "#,
    r#"
    CREATE TABLE chats(id INTEGER PRIMARY KEY, title TEXT NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL);
    CREATE TABLE messages(id INTEGER PRIMARY KEY, chat INTEGER NOT NULL, time INTEGER NOT NULL, role TEXT NOT NULL, text TEXT NOT NULL, interaction INTEGER);
    CREATE INDEX chats_updated ON chats(updated);
    CREATE INDEX messages_chat ON messages(chat, id);
    "#,
    r#"
    CREATE TABLE memories(id INTEGER PRIMARY KEY, time INTEGER NOT NULL, text TEXT NOT NULL, source TEXT NOT NULL DEFAULT 'said');
    CREATE VIRTUAL TABLE memories_fts USING fts5(text, content='memories', content_rowid='id');
    CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
        INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text);
    END;
    CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, text) VALUES ('delete', old.id, old.text);
    END;
    CREATE VIRTUAL TABLE messages_fts USING fts5(text, content='messages', content_rowid='id');
    INSERT INTO messages_fts(rowid, text) SELECT id, text FROM messages;
    CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
        INSERT INTO messages_fts(rowid, text) VALUES (new.id, new.text);
    END;
    CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
        INSERT INTO messages_fts(messages_fts, rowid, text) VALUES ('delete', old.id, old.text);
    END;
    "#,
    r#"
    ALTER TABLE speaker ADD COLUMN kind TEXT NOT NULL DEFAULT 'base';
    "#,
];

#[derive(Clone)]
pub struct Db {
    tx: Sender<Job>,
    readers: Arc<Vec<Mutex<Connection>>>,
    path: PathBuf,
}

pub struct Opened {
    pub db: Db,
    pub recovered: Option<PathBuf>,
}

fn configure(c: &Connection) -> rusqlite::Result<()> {
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    c.pragma_update(None, "cache_size", -2000)?;
    Ok(())
}

fn version(c: &Connection) -> rusqlite::Result<usize> {
    c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).map(|v| v as usize)
}

fn migrate(c: &mut Connection, path: &Path) -> Result<()> {
    let v = version(c)?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
        if v > 0 {
            let backup = path.with_extension(format!("before-v{}.db", i + 1));
            let _ = std::fs::remove_file(&backup);
            c.execute("VACUUM INTO ?1", params![backup.to_string_lossy()]).context("backup before migration")?;
        }
        let t = c.transaction()?;
        t.execute_batch(sql).with_context(|| format!("migration {}", i + 1))?;
        t.pragma_update(None, "user_version", (i + 1) as i64)?;
        t.commit()?;
    }
    Ok(())
}

fn healthy(path: &Path) -> bool {
    let Ok(c) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE) else { return false };
    c.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)).map(|s| s == "ok").unwrap_or(false)
}

impl Db {
    pub fn open(path: &Path) -> Result<Opened> {
        let mut recovered = None;
        if path.exists() && !healthy(path) {
            let aside = path.with_extension(format!("corrupt-{}.db", setu_common::protocol::now_ms()));
            std::fs::rename(path, &aside)?;
            for ext in ["db-wal", "db-shm"] {
                let _ = std::fs::remove_file(path.with_extension(ext));
            }
            log::error!("database failed its integrity check and was moved to {}", aside.display());
            recovered = Some(aside);
        }
        let mut w = Connection::open(path)?;
        w.pragma_update(None, "journal_mode", "WAL")?;
        configure(&w)?;
        migrate(&mut w, path)?;
        let mut readers = Vec::new();
        for _ in 0..2 {
            let r = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            configure(&r)?;
            readers.push(Mutex::new(r));
        }
        let (tx, rx) = bounded::<Job>(1024);
        std::thread::Builder::new()
            .name("db-writer".into())
            .spawn(move || {
                let mut c = w;
                while let Ok(job) = rx.recv() {
                    job(&mut c);
                }
                let _ = c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            })?;
        Ok(Opened { db: Db { tx, readers: Arc::new(readers), path: path.to_owned() }, recovered })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static) -> crossbeam_channel::Receiver<Result<T>> {
        let (tx, rx) = bounded(1);
        let job: Job = Box::new(move |c| {
            let _ = tx.send(f(c));
        });
        if self.tx.send(job).is_err() {
            let (etx, erx) = bounded(1);
            let _ = etx.send(Err(anyhow!("database is closed")));
            return erx;
        }
        rx
    }

    pub fn write_sync<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static) -> Result<T> {
        self.write(f).recv().map_err(|_| anyhow!("database is closed"))?
    }

    pub async fn write_async<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static) -> Result<T> {
        let rx = self.write(f);
        crate::layers::blocking(move || rx.recv().map_err(|_| anyhow!("database is closed"))?).await?
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        for r in self.readers.iter() {
            if let Ok(c) = r.try_lock() {
                return f(&c);
            }
        }
        let c = self.readers[0].lock().map_err(|_| anyhow!("reader poisoned"))?;
        f(&c)
    }

    pub fn load_settings(&self) -> Result<Map<String, Value>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT key, value FROM settings")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut m = Map::new();
            for row in rows.flatten() {
                if let Ok(v) = serde_json::from_str(&row.1) {
                    m.insert(row.0, v);
                }
            }
            Ok(m)
        })
    }

    pub fn save_settings(&self, changes: Map<String, Value>) {
        self.write(move |c| {
            let t = c.transaction()?;
            for (k, v) in &changes {
                t.execute("INSERT INTO settings(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![k, v.to_string()])?;
            }
            t.commit()?;
            Ok(())
        });
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.write_sync(|c| {
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
    }
}

#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub time: i64,
    pub said: String,
    pub did: String,
    pub result: String,
    pub layer: String,
    pub duration_ms: i64,
    pub app: Option<String>,
    pub action: Option<Value>,
    pub undoable: bool,
}

pub fn history_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let action: Option<String> = r.get(8)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?.to_string(),
        "time": r.get::<_, i64>(1)?,
        "said": r.get::<_, String>(2)?,
        "did": r.get::<_, String>(3)?,
        "result": r.get::<_, String>(4)?,
        "layer": r.get::<_, String>(5)?,
        "durationMs": r.get::<_, i64>(6)?,
        "app": r.get::<_, Option<String>>(7)?,
        "action": action.and_then(|a| serde_json::from_str::<Value>(&a).ok()),
        "undoable": r.get::<_, i64>(9)? != 0,
    }))
}

pub const HISTORY_COLS: &str = "id, time, said, did, result, layer, duration_ms, app, action, undoable";

pub fn insert_history(c: &Connection, h: &HistoryRow) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO history(time, said, did, result, layer, duration_ms, app, action, undoable) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![h.time, h.said, h.did, h.result, h.layer, h.duration_ms, h.app, h.action.as_ref().map(|a| a.to_string()), h.undoable as i64],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(8)
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

pub fn pref_get(c: &Connection, key: &str) -> Option<(String, i64)> {
    c.query_row("SELECT value, count FROM preferences WHERE key = ?1", params![key], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .ok()
        .flatten()
}

pub fn pref_bump(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    let now = setu_common::protocol::now_ms();
    c.execute(
        "INSERT INTO preferences(key, value, count, updated) VALUES(?1, ?2, 1, ?3)
         ON CONFLICT(key) DO UPDATE SET count = CASE WHEN value = excluded.value THEN count + 1 ELSE 1 END, value = excluded.value, updated = excluded.updated",
        params![key, value, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_writes_and_searches() {
        let dir = tempfile::tempdir().unwrap();
        let o = Db::open(&dir.path().join("t.db")).unwrap();
        assert!(o.recovered.is_none());
        let db = o.db;
        db.write_sync(|c| {
            insert_history(
                c,
                &HistoryRow {
                    time: 1,
                    said: "open firefox".into(),
                    did: "Opened Firefox".into(),
                    result: "success".into(),
                    layer: "registered".into(),
                    duration_ms: 120,
                    app: Some("Firefox".into()),
                    action: Some(json!({"action": "open_app"})),
                    undoable: true,
                },
            )?;
            Ok(())
        })
        .unwrap();
        let n: i64 = db
            .read(|c| Ok(c.query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH ?1", params![fts_query("fire").unwrap()], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 1);
        db.write_sync(|c| {
            c.execute("DELETE FROM history", [])?;
            Ok(())
        })
        .unwrap();
        let n: i64 = db.read(|c| Ok(c.query_row("SELECT count(*) FROM history_fts WHERE history_fts MATCH 'firefox'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn corrupt_database_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.db");
        std::fs::write(&p, b"this is not a database at all, just text that is long enough").unwrap();
        let o = Db::open(&p).unwrap();
        assert!(o.recovered.is_some());
        assert!(o.db.load_settings().unwrap().is_empty());
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("s.db")).unwrap().db;
        let mut m = Map::new();
        m.insert("ui.theme".into(), json!("dark"));
        db.save_settings(m);
        db.checkpoint().unwrap();
        assert_eq!(db.load_settings().unwrap()["ui.theme"], "dark");
    }

    #[test]
    fn preferences_count_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("p.db")).unwrap().db;
        db.write_sync(|c| {
            pref_bump(c, "choice:chrome", "Google Chrome")?;
            pref_bump(c, "choice:chrome", "Google Chrome")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.read(|c| Ok(pref_get(c, "choice:chrome"))).unwrap(), Some(("Google Chrome".into(), 2)));
    }
}
