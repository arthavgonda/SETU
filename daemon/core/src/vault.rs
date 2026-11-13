use crate::core::Core;
use crate::router::Intent;
use crate::tools::{Choice, FollowUp, Outcome};
use anyhow::{Result, anyhow};
use base64::Engine;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

const LAYER: &str = "vault";
const KEY: &str = "vault.key";
pub const SYSTEM: &str = "System password";

const SITES: &[(&str, &str, &str)] = &[
    ("google", "google.com", "https://accounts.google.com/"),
    ("gmail", "google.com", "https://accounts.google.com/"),
    ("youtube", "google.com", "https://accounts.google.com/"),
    ("github", "github.com", "https://github.com/login"),
    ("gitlab", "gitlab.com", "https://gitlab.com/users/sign_in"),
    ("microsoft", "live.com", "https://login.live.com/"),
    ("outlook", "live.com", "https://login.live.com/"),
    ("hotmail", "live.com", "https://login.live.com/"),
    ("facebook", "facebook.com", "https://www.facebook.com/login"),
    ("instagram", "instagram.com", "https://www.instagram.com/accounts/login/"),
    ("twitter", "x.com", "https://x.com/login"),
    ("x", "x.com", "https://x.com/login"),
    ("linkedin", "linkedin.com", "https://www.linkedin.com/login"),
    ("amazon", "amazon.in", "https://www.amazon.in/"),
    ("flipkart", "flipkart.com", "https://www.flipkart.com/account/login"),
    ("netflix", "netflix.com", "https://www.netflix.com/login"),
    ("spotify", "spotify.com", "https://accounts.spotify.com/login"),
    ("reddit", "reddit.com", "https://www.reddit.com/login/"),
    ("discord", "discord.com", "https://discord.com/login"),
    ("apple", "apple.com", "https://account.apple.com/sign-in"),
    ("yahoo", "yahoo.com", "https://login.yahoo.com/"),
    ("zoho", "zoho.com", "https://accounts.zoho.com/signin"),
    ("paytm", "paytm.com", "https://paytm.com/"),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Entry {
    pub uid: String,
    pub title: String,
    pub account: String,
    pub site: String,
    pub kind: String,
    pub updated: i64,
}

impl Entry {
    pub fn name(&self) -> String {
        if self.account.is_empty() { self.title.clone() } else { format!("{} ({})", self.title, self.account) }
    }
}

pub fn site_of(title: &str) -> Option<(&'static str, &'static str)> {
    let t = crate::text::normalize(title);
    let first = t.split_whitespace().next().unwrap_or_default();
    SITES.iter().find(|(n, _, _)| *n == t || *n == first || t.trim_end_matches(".com") == *n).map(|(_, s, u)| (*s, *u))
}

pub fn host_of(url: &str) -> String {
    let h = crate::tools::web::host(url);
    h.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h.to_owned()).unwrap_or(h).to_lowercase()
}

pub fn site_matches(site: &str, host: &str) -> bool {
    !site.is_empty() && (host == site || host.ends_with(&format!(".{site}")))
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

pub fn open(core: &Core) -> Result<Connection> {
    let path = core.paths.data.join("vault.db");
    let c = Connection::open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    c.busy_timeout(Duration::from_secs(3))?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries(uid TEXT PRIMARY KEY, title TEXT NOT NULL, account TEXT NOT NULL DEFAULT '', site TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL DEFAULT 'web', secret BLOB NOT NULL, created INTEGER NOT NULL, updated INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS face(id INTEGER PRIMARY KEY CHECK (id = 1), data BLOB NOT NULL, updated INTEGER NOT NULL);",
    )?;
    Ok(c)
}

fn empty(c: &Connection) -> Result<bool> {
    let n: i64 = c.query_row("SELECT (SELECT COUNT(*) FROM entries) + (SELECT COUNT(*) FROM face)", [], |r| r.get(0))?;
    Ok(n == 0)
}

struct Key([u8; 32]);

impl Drop for Key {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

async fn key(core: &Core) -> Result<Key> {
    let fresh = empty(&open(core)?)?;
    match crate::secrets::read_bytes(KEY).await {
        Ok(mut v) => {
            let mut raw = base64::engine::general_purpose::STANDARD.decode(&v).map_err(|_| anyhow!("The vault's key in your keychain is damaged."))?;
            v.fill(0);
            if raw.len() != 32 {
                raw.fill(0);
                return Err(anyhow!("The vault's key in your keychain is damaged."));
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&raw);
            raw.fill(0);
            Ok(Key(k))
        }
        Err(e) if crate::secrets::missing(&e) && fresh => {
            let mut k = [0u8; 32];
            SystemRandom::new().fill(&mut k).map_err(|_| anyhow!("No secure random numbers are available."))?;
            let text = base64::engine::general_purpose::STANDARD.encode(k);
            crate::secrets::write(KEY, "Setu password vault key", text.as_bytes()).await?;
            Ok(Key(k))
        }
        Err(e) if crate::secrets::missing(&e) => Err(anyhow!("The vault's key is missing from your keychain, so your saved passwords can't be opened.")),
        Err(e) => Err(e),
    }
}

fn aad(uid: &str) -> Vec<u8> {
    format!("setu-vault-1:{uid}").into_bytes()
}

fn seal(k: &Key, uid: &str, plain: &[u8]) -> Result<Vec<u8>> {
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &k.0).map_err(|_| anyhow!("bad key"))?);
    let mut n = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut n).map_err(|_| anyhow!("No secure random numbers are available."))?;
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(n), Aad::from(aad(uid)), &mut buf).map_err(|_| anyhow!("The password couldn't be locked."))?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + buf.len());
    out.push(1);
    out.extend_from_slice(&n);
    out.extend_from_slice(&buf);
    buf.fill(0);
    Ok(out)
}

fn unseal(k: &Key, uid: &str, blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < 1 + NONCE_LEN + 16 || blob[0] != 1 {
        return Err(anyhow!("A saved password is damaged."));
    }
    let key = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &k.0).map_err(|_| anyhow!("bad key"))?);
    let mut n = [0u8; NONCE_LEN];
    n.copy_from_slice(&blob[1..1 + NONCE_LEN]);
    let mut buf = blob[1 + NONCE_LEN..].to_vec();
    let plain = key.open_in_place(Nonce::assume_unique_for_key(n), Aad::from(aad(uid)), &mut buf).map_err(|_| anyhow!("A saved password couldn't be opened. The keychain's key doesn't match."))?.to_vec();
    buf.fill(0);
    Ok(plain)
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry { uid: r.get(0)?, title: r.get(1)?, account: r.get(2)?, site: r.get(3)?, kind: r.get(4)?, updated: r.get(5)? })
}

pub fn list(core: &Core) -> Result<Vec<Entry>> {
    let c = open(core)?;
    let mut st = c.prepare("SELECT uid, title, account, site, kind, updated FROM entries ORDER BY kind = 'system' DESC, title COLLATE NOCASE, account COLLATE NOCASE")?;
    let v = st.query_map([], row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn get(core: &Core, uid: &str) -> Result<Option<Entry>> {
    let c = open(core)?;
    Ok(c.query_row("SELECT uid, title, account, site, kind, updated FROM entries WHERE uid = ?1", params![uid], row).optional()?)
}

pub fn system(core: &Core) -> Option<Entry> {
    list(core).ok()?.into_iter().find(|e| e.kind == "system")
}

fn clean_name(name: &str) -> String {
    let n = crate::text::normalize(name);
    let n = n.trim_start_matches("my ").trim_start_matches("the ");
    let n = n.trim_end_matches(" password").trim_end_matches(" account").trim_end_matches(" login");
    n.trim().to_owned()
}

pub fn find(core: &Core, name: &str) -> Result<Vec<Entry>> {
    let want = clean_name(name);
    let all = list(core)?;
    if want.is_empty() {
        return Ok(Vec::new());
    }
    if matches!(want.as_str(), "system" | "computer" | "pc" | "laptop" | "login" | "user" | "sudo" | "admin") {
        return Ok(all.into_iter().filter(|e| e.kind == "system").collect());
    }
    let site = site_of(&want).map(|(s, _)| s.to_owned());
    let mut scored: Vec<(f64, Entry)> = all
        .into_iter()
        .map(|e| {
            let t = crate::text::normalize(&e.title);
            let a = crate::text::normalize(&e.account);
            let s = if t == want || a == want {
                1.0
            } else if site.as_deref().is_some_and(|s| s == e.site) {
                0.95
            } else if t.contains(&want) || (!a.is_empty() && a.contains(&want)) || (!e.site.is_empty() && e.site.contains(&want)) {
                0.9
            } else {
                crate::os::score(&want, &t)
            };
            (s, e)
        })
        .filter(|(s, _)| *s >= 0.75)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let top = scored.first().map(|s| s.0).unwrap_or(0.0);
    Ok(scored.into_iter().filter(|(s, _)| *s >= top - 0.06).map(|(_, e)| e).collect())
}

pub async fn save(core: &Core, title: &str, account: &str, site: &str, kind: &str, password: &str) -> Result<Entry> {
    let title = title.trim();
    let account = account.trim();
    if title.is_empty() || title.chars().count() > 80 || account.chars().count() > 120 {
        return Err(anyhow!("Give it a short name."));
    }
    if password.is_empty() || password.len() > 1024 {
        return Err(anyhow!("Type the password first."));
    }
    let k = key(core).await?;
    let c = open(core)?;
    let existing: Option<String> = if kind == "system" {
        c.query_row("SELECT uid FROM entries WHERE kind = 'system'", [], |r| r.get(0)).optional()?
    } else {
        c.query_row("SELECT uid FROM entries WHERE title = ?1 COLLATE NOCASE AND account = ?2 COLLATE NOCASE", params![title, account], |r| r.get(0)).optional()?
    };
    let uid = existing.unwrap_or_else(|| format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..)));
    let site = if !site.is_empty() { site.to_lowercase() } else { site_of(title).map(|(s, _)| s.to_owned()).unwrap_or_default() };
    let blob = seal(&k, &uid, password.as_bytes())?;
    let t = now();
    c.execute(
        "INSERT INTO entries(uid, title, account, site, kind, secret, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) ON CONFLICT(uid) DO UPDATE SET title = excluded.title, account = excluded.account, site = excluded.site, kind = excluded.kind, secret = excluded.secret, updated = excluded.updated",
        params![uid, title, account, site, kind, blob, t],
    )?;
    Ok(Entry { uid, title: title.to_owned(), account: account.to_owned(), site, kind: kind.to_owned(), updated: t })
}

pub async fn secret(core: &Core, uid: &str) -> Result<String> {
    let blob: Vec<u8> = open(core)?.query_row("SELECT secret FROM entries WHERE uid = ?1", params![uid], |r| r.get(0)).optional()?.ok_or_else(|| anyhow!("That password isn't in the vault any more."))?;
    let k = key(core).await?;
    let mut plain = unseal(&k, uid, &blob)?;
    let s = String::from_utf8(plain.clone()).map_err(|_| anyhow!("A saved password is damaged."));
    plain.fill(0);
    s
}

pub fn forget(core: &Core, uid: &str) -> Result<bool> {
    Ok(open(core)?.execute("DELETE FROM entries WHERE uid = ?1", params![uid])? > 0)
}

pub fn face_enrolled(core: &Core) -> bool {
    open(core).ok().and_then(|c| c.query_row("SELECT COUNT(*) FROM face", [], |r| r.get::<_, i64>(0)).ok()).unwrap_or(0) > 0
}

pub async fn face_load(core: &Core) -> Result<Option<crate::face::Templates>> {
    let blob: Option<Vec<u8>> = open(core)?.query_row("SELECT data FROM face WHERE id = 1", [], |r| r.get(0)).optional()?;
    let Some(blob) = blob else { return Ok(None) };
    let k = key(core).await?;
    let mut plain = unseal(&k, "face", &blob)?;
    let t = serde_json::from_slice(&plain).map_err(|_| anyhow!("The saved face is damaged. Say set up face unlock again."));
    plain.fill(0);
    t.map(Some)
}

pub async fn face_save(core: &Core, t: &crate::face::Templates) -> Result<()> {
    let mut plain = serde_json::to_vec(t)?;
    let k = key(core).await?;
    let blob = seal(&k, "face", &plain)?;
    plain.fill(0);
    open(core)?.execute("INSERT INTO face(id, data, updated) VALUES (1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated = excluded.updated", params![blob, now()])?;
    Ok(())
}

pub fn face_forget(core: &Core) -> Result<()> {
    open(core)?.execute("DELETE FROM face", [])?;
    Ok(())
}

pub fn wipe(s: &mut String) {
    let mut v = std::mem::take(s).into_bytes();
    v.fill(0);
}

struct Known {
    title: String,
    account: String,
    site: String,
    kind: String,
}

static SAVED: std::sync::RwLock<Vec<Known>> = std::sync::RwLock::new(Vec::new());

pub fn refresh(core: &Core) {
    if !core.paths.data.join("vault.db").exists() {
        return;
    }
    if let Ok(l) = list(core) {
        *SAVED.write().unwrap_or_else(|e| e.into_inner()) = l.into_iter().map(|e| Known { title: crate::text::normalize(&e.title), account: crate::text::normalize(&e.account), site: e.site, kind: e.kind }).collect();
    }
}

fn has_system(saved: &[Known]) -> bool {
    saved.iter().any(|k| k.kind == "system")
}

fn has_web(saved: &[Known]) -> bool {
    saved.iter().any(|k| !k.site.is_empty())
}

fn knows(saved: &[Known], name: &str) -> bool {
    let want = clean_name(name);
    if want.is_empty() || matches!(want.as_str(), "github" | "git hub") {
        return false;
    }
    let site = site_of(&want).map(|s| s.0);
    saved.iter().any(|k| k.kind != "system" && (k.title == want || k.account == want || site == Some(k.site.as_str()) || k.title.contains(&want)))
}

struct Card {
    id: String,
    mode: &'static str,
    title: String,
    account: String,
    site: String,
    done: Option<tokio::sync::oneshot::Sender<bool>>,
    tries: u8,
}

static CARDS: Mutex<Vec<Card>> = Mutex::new(Vec::new());
static REVEAL: Mutex<u64> = Mutex::new(0);

fn card_json(c: &Card, error: Option<&str>) -> Value {
    let message = match c.mode {
        "system" => "Type the password you use to sign in to this computer.",
        "confirm" => "Type your system password to continue.",
        _ => "Type the password. It stays on this computer.",
    };
    json!({ "open": true, "id": c.id, "mode": c.mode, "title": c.title, "account": c.account, "site": c.site, "message": message, "error": error })
}

pub fn open_card(core: &Core, mode: &'static str, title: &str, account: &str, site: &str) -> (String, tokio::sync::oneshot::Receiver<bool>) {
    let id = format!("card-{:x}", fastrand::u64(..));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let card = Card { id: id.clone(), mode, title: title.to_owned(), account: account.to_owned(), site: site.to_owned(), done: Some(tx), tries: 0 };
    core.hub.emit("vault.entry", card_json(&card, None));
    let mut g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
    for old in g.drain(..) {
        core.hub.emit("vault.entry", json!({ "open": false, "id": old.id }));
    }
    g.push(card);
    (id, rx)
}

fn close_card(core: &Core, id: &str) -> Option<Card> {
    let mut g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
    let i = g.iter().position(|c| c.id == id)?;
    core.hub.emit("vault.entry", json!({ "open": false, "id": id }));
    Some(g.remove(i))
}

pub fn cancel(core: &Core, id: &str) -> Value {
    if let Some(mut c) = close_card(core, id)
        && let Some(tx) = c.done.take()
    {
        let _ = tx.send(false);
    }
    json!({ "closed": true })
}

pub fn current() -> Value {
    let g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
    g.last().map(|c| card_json(c, None)).unwrap_or_else(|| json!({ "open": false }))
}

pub async fn submit(core: &Arc<Core>, p: &Value) -> Result<Value, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let id = s("id");
    let mut password = s("password");
    let (mode, title, account, site) = {
        let g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
        let c = g.iter().find(|c| c.id == id).ok_or("That card was closed.")?;
        (c.mode, if s("title").trim().is_empty() { c.title.clone() } else { s("title") }, if p.get("account").is_some() { s("account") } else { c.account.clone() }, c.site.clone())
    };
    if password.is_empty() {
        return Err("Type the password first.".into());
    }
    if mode == "system" || mode == "confirm" {
        let ok = crate::syspass::check(&password).await.map_err(|e| e.to_string())?;
        if !ok {
            wipe(&mut password);
            let mut g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
            let Some(c) = g.iter_mut().find(|c| c.id == id) else { return Err("That card was closed.".into()) };
            c.tries += 1;
            if c.tries >= 3 {
                let mut c = g.retain_take(&id);
                drop(g);
                core.hub.emit("vault.entry", json!({ "open": false, "id": id }));
                if let Some(tx) = c.as_mut().and_then(|c| c.done.take()) {
                    let _ = tx.send(false);
                }
                return Err("That isn't your system password. I closed the card after three tries.".into());
            }
            let e = "That isn't your system password. Try again.";
            core.hub.emit("vault.entry", card_json(c, Some(e)));
            return Err(e.into());
        }
    }
    if mode == "confirm" {
        wipe(&mut password);
        if let Some(mut c) = close_card(core, &id)
            && let Some(tx) = c.done.take()
        {
            let _ = tx.send(true);
        }
        return Ok(json!({ "ok": true }));
    }
    let card_title = {
        let g = CARDS.lock().unwrap_or_else(|e| e.into_inner());
        g.iter().find(|c| c.id == id).map(|c| c.title.clone()).unwrap_or_default()
    };
    let site = if crate::text::normalize(&title) != crate::text::normalize(&card_title) { site_of(&title).map(|(s, _)| s.to_owned()).unwrap_or(site) } else { site };
    let kind = if mode == "system" { "system" } else if site_of(&title).is_some() || !site.is_empty() { "web" } else { "other" };
    let title = if mode == "system" { SYSTEM.to_owned() } else { title };
    let r = save(core, &title, &account, &site, kind, &password).await;
    wipe(&mut password);
    let e = r.map_err(|e| e.to_string())?;
    refresh(core);
    if let Some(mut c) = close_card(core, &id)
        && let Some(tx) = c.done.take()
    {
        let _ = tx.send(true);
    }
    let said = if mode == "system" {
        if face_enrolled(core) { "Saved your system password. Face unlock is ready.".to_owned() } else { "Saved your system password.".to_owned() }
    } else {
        format!("Saved your {} password.", e.name())
    };
    crate::board::alert(core, "vault", &said);
    if mode == "system" {
        let c = core.clone();
        tokio::spawn(async move { crate::faceid::sync_root(&c).await });
    }
    Ok(json!({ "ok": true, "uid": e.uid }))
}

trait TakeCard {
    fn retain_take(&mut self, id: &str) -> Option<Card>;
}

impl TakeCard for Vec<Card> {
    fn retain_take(&mut self, id: &str) -> Option<Card> {
        let i = self.iter().position(|c| c.id == id)?;
        Some(self.remove(i))
    }
}

pub fn reveal(core: &Core, e: &Entry, password: &str, seconds: u64) {
    let n = {
        let mut g = REVEAL.lock().unwrap_or_else(|e| e.into_inner());
        *g += 1;
        *g
    };
    core.hub.emit("vault.reveal", json!({ "open": true, "title": e.title, "account": e.account, "password": password, "seconds": seconds }));
    let hub = core.hub.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(seconds)).await;
        if *REVEAL.lock().unwrap_or_else(|e| e.into_inner()) == n {
            hub.emit("vault.reveal", json!({ "open": false }));
        }
    });
}

pub fn hide(core: &Core) -> Value {
    *REVEAL.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    core.hub.emit("vault.reveal", json!({ "open": false }));
    json!({ "hidden": true })
}

fn between<'a>(t: &'a str, pre: &[&str], post: &[&str]) -> Option<&'a str> {
    for p in pre {
        if let Some(r) = t.strip_prefix(p) {
            for q in post {
                if let Some(x) = r.strip_suffix(q) {
                    let x = x.trim();
                    if !x.is_empty() && x.split_whitespace().count() <= 5 {
                        return Some(x);
                    }
                }
            }
        }
    }
    None
}

pub fn rules(t: &str) -> Option<Intent> {
    rules_in(t, &SAVED.read().unwrap_or_else(|e| e.into_inner()))
}

fn rules_in(t: &str, saved: &[Known]) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    if matches!(t, "what passwords do i have" | "list my passwords" | "what's in my vault" | "what is in my vault" | "show my vault" | "open my vault" | "which passwords have you saved" | "what passwords have you saved") {
        return act("vault.list", json!({}));
    }
    if matches!(t, "save my system password" | "change my system password in the vault" | "update my system password" | "update my system password in the vault" | "save my computer password" | "save my laptop password") {
        return act("vault.save", json!({ "system": true }));
    }
    if matches!(t, "save this password" | "save a password" | "save a new password" | "add a password" | "add a new password" | "add a password to the vault" | "save a password in the vault" | "save a password to the vault" | "remember a password") {
        return act("vault.save", json!({}));
    }
    if matches!(t, "save a password for this site" | "save the password for this site" | "save my password for this site" | "save a password for this page" | "remember the password for this site") {
        return act("vault.save", json!({ "here": true }));
    }
    if let Some(x) = between(t, &["save a password for ", "save my password for ", "save the password for ", "add a password for "], &[""]) {
        return act("vault.save", json!({ "title": x }));
    }
    if let Some(x) = between(t, &["save my ", "remember my ", "add my "], &[" password", " password to the vault", " password in the vault"]) {
        return act("vault.save", json!({ "title": x }));
    }
    if let Some(x) = between(t, &["change my ", "update my "], &[" password in the vault", " password in my vault", " password in vault"]) {
        return act("vault.save", json!({ "title": x }));
    }
    if let Some(x) = between(t, &["read my ", "read out my ", "read me my ", "say my ", "speak my "], &[" password aloud", " password out loud", " password out", " password to me", " password"]) {
        return act("vault.say", json!({ "name": x }));
    }
    if let Some(x) = between(t, &["show my ", "show me my ", "reveal my ", "what's my ", "what is my ", "tell me my "], &[" password aloud", " password out loud"]) {
        return act("vault.say", json!({ "name": x }));
    }
    if let Some(x) = between(t, &["show my ", "show me my ", "reveal my ", "what's my ", "what is my ", "tell me my "], &[" password"]) {
        return act("vault.show", json!({ "name": x }));
    }
    if let Some(x) = between(t, &["forget my ", "delete my ", "remove my "], &[" password", " password from the vault", " password from my vault"]) {
        return act("vault.forget", json!({ "name": x }));
    }
    if matches!(t, "type my system password" | "enter my system password" | "type my sudo password" | "enter my sudo password" | "type my password from the vault" | "use my saved password") {
        return act("vault.type", json!({ "name": "system" }));
    }
    if matches!(t, "type my password" | "enter my password" | "fill my password" | "fill in my password" | "type the password" | "enter the password") {
        return if has_system(saved) { act("vault.type", json!({ "name": "system" })) } else { None };
    }
    if let Some(x) = between(t, &["type my ", "enter my ", "fill in my ", "fill my "], &[" password"]) {
        return act("vault.type", json!({ "name": x }));
    }
    let (t, account) = match t.rsplit_once(" with my ").or_else(|| t.rsplit_once(" using my ")).or_else(|| t.rsplit_once(" as ")) {
        Some((head, who)) if head.starts_with("sign") || head.starts_with("log") => {
            let who = who.trim().trim_end_matches(" account").trim_end_matches(" address").trim_end_matches(" email").trim();
            (head.trim(), (!who.is_empty() && who.split_whitespace().count() <= 4).then(|| who.to_owned()))
        }
        _ => (t, None),
    };
    if matches!(t, "sign in" | "sign me in" | "log in" | "log me in" | "sign in here" | "log in here" | "sign in to this site" | "log in to this site" | "sign in on this page" | "sign me in here") && has_web(saved) {
        return act("vault.signin", json!({ "here": true, "account": account }));
    }
    if let Some(x) = between(t, &["sign in to ", "sign into ", "log in to ", "log into ", "login to ", "sign me in to ", "sign me into ", "log me in to ", "log me into "], &[""]) {
        let site = x.trim_start_matches("my ").trim_end_matches(" account");
        if knows(saved, site) {
            return act("vault.signin", json!({ "site": site, "account": account }));
        }
    }
    None
}

pub fn needs_confirm(tool: &str) -> bool {
    tool == "vault.forget"
}

pub fn confirm_title(core: &Core, args: &Value) -> Result<String, String> {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    let uid = args.get("uid").and_then(Value::as_str).unwrap_or_default();
    let e = if uid.is_empty() { find(core, name).map_err(|e| e.to_string())?.into_iter().next() } else { get(core, uid).map_err(|e| e.to_string())? };
    let e = e.ok_or_else(|| format!("I don't have a {} password saved.", clean_name(name)))?;
    Ok(format!("Forget your {} password?", e.name()))
}

pub fn label(tool: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool {
        "vault.save" => "Opening the password card".into(),
        "vault.show" => format!("Showing your {} password", s("name")),
        "vault.say" => format!("Reading your {} password aloud", s("name")),
        "vault.forget" => format!("Forgetting your {} password", s("name")),
        "vault.list" => "Listing your saved passwords".into(),
        "vault.type" => "Typing your password".into(),
        "vault.signin" if s("site").is_empty() => "Signing you in".into(),
        "vault.signin" => format!("Signing you in to {}", crate::text::sentence_case(&s("site"))),
        _ => String::new(),
    }
}

fn pick(question: &str, tool: &str, list: &[Entry], extra: &Value) -> Outcome {
    let choices = list
        .iter()
        .take(6)
        .map(|e| {
            let mut a = extra.clone();
            a["uid"] = json!(e.uid);
            let label = e.name();
            let word = if e.account.is_empty() { crate::text::normalize(&e.title) } else { crate::text::normalize(&e.account) };
            Choice::new(&label, &[word.as_str()], Intent::Act { tool: tool.into(), args: a })
        })
        .collect();
    Outcome::asking("Asked which account", LAYER, FollowUp { question: question.into(), spoken: Some(question.into()), choices, cleanup: None, kind: "choose", focus: None })
}

async fn chosen(core: &Arc<Core>, tool: &str, args: &Value, default: &str) -> Result<Entry, Box<Outcome>> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    if !s("uid").is_empty() {
        return get(core, &s("uid")).ok().flatten().ok_or_else(|| Box::new(Outcome::fail("That password isn't in the vault any more.", LAYER)));
    }
    let name = if s("name").is_empty() { default.to_owned() } else { s("name") };
    let found = find(core, &name).map_err(|e| Box::new(Outcome::fail(&e.to_string(), LAYER)))?;
    match found.len() {
        0 if clean_name(&name) == "system" || name == "system" => Err(Box::new(Outcome::fail("I don't have your system password yet. Say save my system password.", LAYER))),
        0 => Err(Box::new(Outcome::fail(&format!("I don't have a {} password saved. Say save my {} password.", clean_name(&name), clean_name(&name)), LAYER))),
        1 => Ok(found.into_iter().next().unwrap_or_else(|| unreachable!())),
        _ => Err(Box::new(pick("Which one?", tool, &found, args))),
    }
}

fn terminal(app: &str) -> bool {
    let a = app.to_lowercase();
    ["terminal", "konsole", "ptyxis", "console", "alacritty", "kitty", "wezterm", "xterm", "tilix", "foot", "iterm", "warp", "powershell", "cmd.exe", "guake", "terminator"].iter().any(|t| a.contains(t))
}

const PASSWORD_PROMPT: &[&str] = &["password:", "password for", "passphrase", "password: ", "pin:"];

async fn password_focus() -> Result<(), String> {
    let w = crate::os::front().await.map_err(|_| "I can't tell which window is in front.".to_owned())?;
    if let Ok(Some(n)) = crate::os::focused(&w).await {
        if n.role == "password" {
            return Ok(());
        }
        if terminal(&w.app)
            && let Some(text) = crate::os::text(&n, 4000).await
        {
            let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().to_lowercase();
            if PASSWORD_PROMPT.iter().any(|p| last.trim_end().ends_with(p.trim_end()) || last.contains("[sudo] password")) {
                return Ok(());
            }
        }
    }
    Err("I only type passwords into a password box. Click the password box first.".into())
}

const SIGNIN_JS: &str = "JSON.stringify(((u,p)=>{const vis=e=>{const r=e.getBoundingClientRect();const s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'};const set=(e,v)=>{e.focus();Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}))};const ins=[...document.querySelectorAll('input')].filter(vis);const pw=ins.find(e=>e.type==='password');const user=ins.find(e=>e!==pw&&['email','text','tel',''].includes((e.type||'').toLowerCase())&&/user|mail|login|identifier|account|phone|name/i.test([e.autocomplete,e.name,e.id,e.type,e.getAttribute('aria-label'),e.placeholder].join(' ')));const did=[];if(user&&u&&user.value!==u){set(user,u);did.push('user')}if(pw&&p){set(pw,p);did.push('password')}const target=pw||user;if(!target)return {user:false,password:false,did,sent:false};const form=target.form;const btn=(form&&form.querySelector('button[type=submit],input[type=submit],button:not([type])'))||[...document.querySelectorAll('button,[role=button],input[type=submit]')].filter(vis).find(b=>/^(next|sign in|log in|login|continue|submit)$/i.test((b.innerText||b.value||'').trim()));if(btn)btn.click();else if(form){form.requestSubmit?form.requestSubmit():form.submit()}return {user:!!user,password:!!pw,did,sent:true}})(U,P))";
const HAS_PASSWORD_JS: &str = "JSON.stringify([...document.querySelectorAll('input[type=password]')].some(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0}))";

fn signin_js(user: &str, pass: &str) -> String {
    let u = serde_json::to_string(user).unwrap_or_else(|_| "\"\"".into());
    let p = serde_json::to_string(pass).unwrap_or_else(|_| "\"\"".into());
    SIGNIN_JS.replacen("(U,P)", &format!("({u},{p})"), 1)
}

async fn signin(core: &Arc<Core>, args: &Value, cancel: Arc<std::sync::atomic::AtomicBool>) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let Some(conn) = crate::proto::any() else {
        return Outcome::fail("Signing in needs your Setu browser. Say open the browser, go to the sign-in page, then say sign in.", LAYER);
    };
    let tab = match crate::proto::active_tab(&conn).await {
        Ok(t) => t,
        Err(e) => return Outcome::fail(&e.to_string(), LAYER),
    };
    let host = host_of(&tab.url);
    let e = if !s("uid").is_empty() {
        match get(core, &s("uid")) {
            Ok(Some(e)) => e,
            _ => return Outcome::fail("That password isn't in the vault any more.", LAYER),
        }
    } else {
        let all = list(core).unwrap_or_default();
        let found: Vec<Entry> = if s("site").is_empty() { all.into_iter().filter(|e| site_matches(&e.site, &host)).collect() } else { find(core, &s("site")).unwrap_or_default().into_iter().filter(|e| e.kind != "system").collect() };
        let who = crate::text::normalize(&s("account"));
        let narrowed: Vec<Entry> = if who.is_empty() { Vec::new() } else { found.iter().filter(|e| crate::text::normalize(&e.account).contains(&who) || crate::text::normalize(&e.title).contains(&who)).cloned().collect() };
        if !who.is_empty() && narrowed.is_empty() && !found.is_empty() {
            return pick(&format!("I don't have a {} account saved. Which one?", s("account")), "vault.signin", &found, args);
        }
        let found = if narrowed.is_empty() { found } else { narrowed };
        match found.len() {
            0 if s("site").is_empty() => return Outcome::fail(&format!("I don't have a password saved for {host}. Say save a password for this site."), LAYER),
            0 => return Outcome::fail(&format!("I don't have a {} password saved. Say save my {} password.", s("site"), s("site")), LAYER),
            1 => found.into_iter().next().unwrap_or_else(|| unreachable!()),
            _ => return pick("Which account?", "vault.signin", &found, args),
        }
    };
    if e.site.is_empty() {
        return Outcome::fail(&format!("I don't know which website {} belongs to, so I won't put it into a page.", e.title), LAYER);
    }
    let tab = if site_matches(&e.site, &host) {
        tab
    } else {
        let Some((_, url)) = site_of(&e.title).filter(|(site, _)| *site == e.site) else {
            return Outcome::fail(&format!("This page isn't {}, so I won't put your {} password here. Open its sign-in page first.", e.site, e.title), LAYER);
        };
        if let Err(err) = conn.navigate(&tab.id, url).await {
            return Outcome::fail(&err.to_string(), LAYER);
        }
        let mut ready = None;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Ok(t) = crate::proto::active_tab(&conn).await
                && site_matches(&e.site, &host_of(&t.url))
            {
                ready = Some(t);
                break;
            }
        }
        match ready {
            Some(t) => t,
            None => return Outcome::fail(&format!("The {} sign-in page didn't open.", e.title), LAYER),
        }
    };
    if let Err(m) = crate::faceid::verify(core, &format!("sign in to {}", e.title), cancel).await {
        return Outcome::fail(&m, LAYER);
    }
    let mut pass = match secret(core, &e.uid).await {
        Ok(p) => p,
        Err(err) => return Outcome::fail(&err.to_string(), LAYER),
    };
    let check = async |c: &crate::proto::Conn, id: &str| -> bool {
        match c.tabs().await {
            Ok(tabs) => tabs.iter().find(|t| t.id == id).is_some_and(|t| site_matches(&e.site, &host_of(&t.url))),
            Err(_) => false,
        }
    };
    let mut steps = 0;
    let mut sent_password = false;
    let mut last = Value::Null;
    while steps < 2 && !sent_password {
        steps += 1;
        if !check(&conn, &tab.id).await {
            break;
        }
        let mut js = signin_js(&e.account, &pass);
        let r = conn.eval_json(&tab.id, &js).await;
        wipe(&mut js);
        last = match r {
            Ok(v) => v,
            Err(err) => {
                wipe(&mut pass);
                return Outcome::fail(&err.to_string(), LAYER);
            }
        };
        sent_password = last.get("password").and_then(Value::as_bool) == Some(true);
        if !sent_password && last.get("user").and_then(Value::as_bool) == Some(true) {
            let mut seen = false;
            for _ in 0..40 {
                tokio::time::sleep(Duration::from_millis(250)).await;
                if conn.eval_json(&tab.id, HAS_PASSWORD_JS).await.ok().and_then(|v| v.as_bool()) == Some(true) {
                    seen = true;
                    break;
                }
            }
            if !seen {
                break;
            }
        } else if !sent_password {
            break;
        }
    }
    wipe(&mut pass);
    if sent_password {
        Outcome::ok_with(&format!("Signed in to {}", e.title), LAYER, None).with_reply(format!("Sent your {} sign-in. If it asks for a code, that part is yours.", e.title))
    } else if last.get("user").and_then(Value::as_bool) == Some(true) {
        Outcome::fail("I filled in your account, but no password box came up. Look at the page; it may want a code or a check.", LAYER)
    } else {
        Outcome::fail("I couldn't find a sign-in form on this page.", LAYER)
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value, cancel: Arc<std::sync::atomic::AtomicBool>) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool {
        "vault.save" => {
            let system = args.get("system").and_then(Value::as_bool) == Some(true);
            if system {
                open_card(core, "system", SYSTEM, &crate::syspass::user().unwrap_or_default(), "");
                return Outcome::ok_with("Opened the password card", LAYER, None).with_reply("Type your system password in the card. I'll check it with the system before saving.".into());
            }
            let (mut title, mut site) = (crate::text::sentence_case(&s("title")), String::new());
            if args.get("here").and_then(Value::as_bool) == Some(true) {
                let Some(conn) = crate::proto::any() else { return Outcome::fail("Open the site in your Setu browser first.", LAYER) };
                match crate::proto::active_tab(&conn).await {
                    Ok(t) => {
                        site = host_of(&t.url).trim_start_matches("www.").to_owned();
                        title = site.clone();
                    }
                    Err(e) => return Outcome::fail(&e.to_string(), LAYER),
                }
            }
            if let Some((sv, _)) = site_of(&title) {
                site = sv.to_owned();
            }
            open_card(core, "save", &title, "", &site);
            Outcome::ok_with("Opened the password card", LAYER, None).with_reply("Type it in the card. I never read passwords out.".into())
        }
        "vault.list" => match list(core) {
            Ok(v) if v.is_empty() => Outcome::ok_with("No saved passwords", LAYER, None).with_reply("The vault is empty. Say save a password to add one.".into()),
            Ok(v) => {
                let names: Vec<String> = v.iter().map(Entry::name).collect();
                Outcome::ok_with("Listed saved passwords", LAYER, None).with_reply(format!("You have {} saved: {}.", v.len(), names.join(", ")))
            }
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "vault.show" => {
            let e = match chosen(core, tool, args, "").await {
                Ok(e) => e,
                Err(o) => return *o,
            };
            if let Err(m) = crate::faceid::verify(core, &format!("show your {} password", e.title), cancel).await {
                return Outcome::fail(&m, LAYER);
            }
            match secret(core, &e.uid).await {
                Ok(mut p) => {
                    reveal(core, &e, &p, 10);
                    wipe(&mut p);
                    Outcome::ok_with(&format!("Showed the {} password", e.title), LAYER, None).with_reply("It's on screen for 10 seconds. I won't read it out.".into())
                }
                Err(err) => Outcome::fail(&err.to_string(), LAYER),
            }
        }
        "vault.signup" => crate::signup::run(core, args).await,
        "vault.say" => {
            let e = match chosen(core, tool, args, "").await {
                Ok(e) => e,
                Err(o) => return *o,
            };
            if core.slot("tts").provider != "local" {
                return Outcome::fail(&format!("My voice runs on a service outside this computer, so I won't read a password out. Say show my {} password instead.", e.title), LAYER);
            }
            if let Err(m) = crate::faceid::verify(core, &format!("read your {} password aloud", e.title), cancel.clone()).await {
                return Outcome::fail(&m, LAYER);
            }
            match secret(core, &e.uid).await {
                Ok(mut p) => {
                    let chars: Vec<char> = p.chars().collect();
                    wipe(&mut p);
                    let mut line = format!("Your {} password, one character at a time: {}.", e.title, crate::password::spell(&chars));
                    if line.chars().count() <= 96 {
                        line.push_str(" That is the whole password, read once and not kept anywhere.");
                    }
                    let spoken = crate::pipeline::sample(core, None, None, None, line.clone()).await;
                    wipe(&mut line);
                    match spoken {
                        Ok(v) => {
                            let ms = v.get("durationMs").and_then(Value::as_u64).unwrap_or(4000).min(120_000) + 600;
                            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                            while std::time::Instant::now() < end {
                                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                    core.engines.tts.send(&setu_common::engine::ToEngine::TtsStop);
                                    return Outcome::fail("Stopped.", LAYER);
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            Outcome::ok_with(&format!("Read the {} password aloud", e.title), LAYER, None).with_reply("That was it. I haven't written it anywhere.".into())
                        }
                        Err(_) => Outcome::fail("I couldn't speak just now, so nothing was read out.", LAYER),
                    }
                }
                Err(err) => Outcome::fail(&err.to_string(), LAYER),
            }
        }
        "vault.forget" => {
            let e = match chosen(core, tool, args, "").await {
                Ok(e) => e,
                Err(o) => return *o,
            };
            let r = forget(core, &e.uid);
            refresh(core);
            match r {
                Ok(_) => Outcome::ok_with(&format!("Forgot the {} password", e.title), LAYER, None).with_reply(format!("Forgot your {} password.", e.name())),
                Err(err) => Outcome::fail(&err.to_string(), LAYER),
            }
        }
        "vault.type" => {
            let e = match chosen(core, tool, args, "system").await {
                Ok(e) => e,
                Err(o) => return *o,
            };
            if let Err(m) = password_focus().await {
                return Outcome::fail(&m, LAYER);
            }
            let front = crate::os::front().await.ok();
            if let Err(m) = crate::faceid::verify(core, &format!("type your {} password", e.title), cancel).await {
                return Outcome::fail(&m, LAYER);
            }
            if let Some(w) = &front
                && !crate::os::in_front(w, 1500).await
            {
                return Outcome::fail("The window changed, so I didn't type it.", LAYER);
            }
            if let Err(m) = password_focus().await {
                return Outcome::fail(&m, LAYER);
            }
            let mut p = match secret(core, &e.uid).await {
                Ok(p) => p,
                Err(err) => return Outcome::fail(&err.to_string(), LAYER),
            };
            let r = crate::tools::input::type_text(&p).await;
            wipe(&mut p);
            match r {
                Ok(()) => Outcome::ok_with(&format!("Typed the {} password", e.title), "keyboard", None).with_reply("Typed it. Press Enter when you're ready.".into()),
                Err(err) => Outcome::fail(&err.to_string(), "keyboard"),
            }
        }
        "vault.signin" => signin(core, args, cancel).await,
        _ => Outcome::fail("I don't know that vault command.", LAYER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealing_round_trips_and_binds_to_the_row() {
        let k = Key([7u8; 32]);
        let b = seal(&k, "a", b"hunter2").unwrap();
        assert_eq!(unseal(&k, "a", &b).unwrap(), b"hunter2");
        assert!(unseal(&k, "b", &b).is_err());
        assert!(unseal(&Key([8u8; 32]), "a", &b).is_err());
        let b2 = seal(&k, "a", b"hunter2").unwrap();
        assert_ne!(b, b2);
        assert!(!String::from_utf8_lossy(&b).contains("hunter2"));
    }

    #[test]
    fn vault_phrases() {
        let mut saved = Vec::new();
        let tool = |t: &str, saved: &[Known]| match rules_in(t, saved) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        };
        assert_eq!(tool("show my google password", &saved).unwrap().1["name"], "google");
        assert_eq!(tool("what's my github password", &saved).unwrap().0, "vault.show");
        assert_eq!(tool("forget my netflix password", &saved).unwrap().0, "vault.forget");
        assert_eq!(tool("save my system password", &saved).unwrap().1["system"], true);
        assert_eq!(tool("save my google password", &saved).unwrap().1["title"], "google");
        assert!(tool("type my password", &saved).is_none());
        assert!(tool("sign in to google", &saved).is_none());
        assert!(tool("sign in", &saved).is_none());
        saved = vec![Known { title: "system password".into(), account: String::new(), site: String::new(), kind: "system".into() }, Known { title: "google".into(), account: "a@gmail.com".into(), site: "google.com".into(), kind: "web".into() }];
        assert_eq!(tool("type my password", &saved).unwrap().1["name"], "system");
        assert_eq!(tool("sign in to google", &saved).unwrap().1["site"], "google");
        assert_eq!(tool("log in to my google account", &saved).unwrap().1["site"], "google");
        let (_, a) = tool("sign in to google with my work account", &saved).unwrap();
        assert_eq!((a["site"].as_str(), a["account"].as_str()), (Some("google"), Some("work")));
        assert_eq!(tool("sign in as a@gmail.com", &saved).unwrap().1["account"], "a@gmail.com");
        assert_eq!(tool("sign in with my work account", &saved).unwrap().1["here"], true);
        assert!(tool("log in to github", &saved).is_none());
        assert_eq!(tool("sign in", &saved).unwrap().1["here"], true);
        assert_eq!(tool("list my passwords", &saved).unwrap().0, "vault.list");
        assert!(tool("show my downloads", &saved).is_none());
        assert!(tool("sign in to the vpn and then open the mail and the calendar app", &saved).is_none());
    }

    #[test]
    fn sites_and_hosts() {
        assert_eq!(site_of("Google").map(|s| s.0), Some("google.com"));
        assert_eq!(site_of("gmail").map(|s| s.0), Some("google.com"));
        assert_eq!(site_of("Bank of Nowhere"), None);
        assert_eq!(host_of("https://accounts.google.com/v3/signin"), "accounts.google.com");
        assert_eq!(host_of("http://127.0.0.1:8765/login.html"), "127.0.0.1");
        assert!(site_matches("google.com", "accounts.google.com"));
        assert!(!site_matches("google.com", "google.com.evil.net"));
        assert!(!site_matches("google.com", "notgoogle.com"));
        assert!(!site_matches("", "google.com"));
    }
}
