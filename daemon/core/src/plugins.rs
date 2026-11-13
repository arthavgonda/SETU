use crate::router::PluginTool;
use crate::storage::Db;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::proc::Quiet;

const TRUSTED_KEYS: &[&str] = &[];

#[derive(Debug, Clone, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub risk: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub can: Vec<String>,
    #[serde(default)]
    pub access: Vec<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sig {
    Verified,
    Unsigned,
    Invalid,
}

impl Sig {
    fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unsigned => "unsigned",
            Self::Invalid => "invalid",
        }
    }
}

struct Entry {
    dir: PathBuf,
    manifest: Manifest,
    sig: Sig,
    enabled: bool,
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
    used: Instant,
}

pub struct Plugins {
    dir: PathBuf,
    db: Db,
    entries: Mutex<HashMap<String, Entry>>,
    servers: Mutex<HashMap<String, Server>>,
    developer: AtomicBool,
}

pub fn digest(dir: &Path, m_bytes: &[u8], files: &[String]) -> Option<Vec<u8>> {
    let mut h = Sha256::new();
    h.update(m_bytes);
    for f in files {
        if f.contains("..") || Path::new(f).is_absolute() {
            return None;
        }
        let bytes = std::fs::read(dir.join(f)).ok()?;
        h.update(f.as_bytes());
        h.update(Sha256::digest(&bytes));
    }
    Some(h.finalize().to_vec())
}

fn verify(dir: &Path, m_bytes: &[u8], files: &[String]) -> Sig {
    let Ok(sig_text) = std::fs::read_to_string(dir.join("plugin.sig")) else { return Sig::Unsigned };
    use base64::Engine;
    let Ok(sig_bytes) = base64::engine::general_purpose::STANDARD.decode(sig_text.trim()) else { return Sig::Invalid };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else { return Sig::Invalid };
    let Some(d) = digest(dir, m_bytes, files) else { return Sig::Invalid };
    for k in TRUSTED_KEYS {
        let Ok(kb) = base64::engine::general_purpose::STANDARD.decode(k) else { continue };
        let Ok(arr) = <[u8; 32]>::try_from(kb.as_slice()) else { continue };
        let Ok(vk) = VerifyingKey::from_bytes(&arr) else { continue };
        if vk.verify(&d, &sig).is_ok() {
            return Sig::Verified;
        }
    }
    Sig::Invalid
}

fn tool_name(plugin: &str, tool: &str) -> String {
    let clean = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect::<String>();
    let p = clean(plugin.rsplit('.').next().unwrap_or(plugin));
    format!("{}_{}", p, clean(tool)).chars().take(48).collect()
}

impl Plugins {
    pub fn new(dir: PathBuf, db: Db, developer: bool) -> Self {
        let me = Self { dir, db, entries: Mutex::new(HashMap::new()), servers: Mutex::new(HashMap::new()), developer: AtomicBool::new(developer) };
        me.reload();
        me
    }

    pub fn set_developer(&self, on: bool) {
        self.developer.store(on, Ordering::Relaxed);
    }

    pub fn reload(&self) {
        let enabled: HashMap<String, bool> = self
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT id, enabled FROM plugins")?;
                let v = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?.flatten().collect();
                Ok(v)
            })
            .unwrap_or_default();
        let mut map = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let dir = e.path();
                let Ok(bytes) = std::fs::read(dir.join("plugin.json")) else { continue };
                let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
                    log::warn!("plugin in {} has an unreadable manifest", dir.display());
                    continue;
                };
                let sig = verify(&dir, &bytes, &manifest.files);
                let on = enabled.get(&manifest.id).copied().unwrap_or(false) && sig != Sig::Invalid;
                map.insert(manifest.id.clone(), Entry { dir, manifest, sig, enabled: on });
            }
        }
        *self.entries.lock().unwrap() = map;
    }

    fn visible(&self, e: &Entry) -> bool {
        e.sig == Sig::Verified || self.developer.load(Ordering::Relaxed)
    }

    fn json(&self, e: &Entry) -> Value {
        json!({
            "id": e.manifest.id, "name": e.manifest.name, "publisher": e.manifest.publisher, "version": e.manifest.version,
            "can": e.manifest.can, "access": e.manifest.access, "signature": e.sig.as_str(), "enabled": e.enabled,
            "tools": e.manifest.tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(), "description": e.manifest.description
        })
    }

    pub fn list(&self) -> Value {
        let g = self.entries.lock().unwrap();
        let mut v: Vec<Value> = g.values().filter(|e| self.visible(e)).map(|e| self.json(e)).collect();
        v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        json!({ "items": v, "developerMode": self.developer.load(Ordering::Relaxed) })
    }

    pub fn invalid(&self) -> Vec<String> {
        self.entries.lock().unwrap().values().filter(|e| e.sig == Sig::Invalid).map(|e| e.manifest.name.clone()).collect()
    }

    pub fn set_enabled(&self, id: &str, on: bool) -> Result<Value, String> {
        let mut g = self.entries.lock().unwrap();
        let e = g.get_mut(id).ok_or("No such plugin.")?;
        if on && e.sig == Sig::Invalid {
            return Err(format!("{} failed signature verification and can't be turned on.", e.manifest.name));
        }
        if on && e.sig == Sig::Unsigned && !self.developer.load(Ordering::Relaxed) {
            return Err("Unsigned plugins need Developer mode.".into());
        }
        e.enabled = on;
        let idc = id.to_owned();
        self.db.write(move |c| {
            c.execute("INSERT INTO plugins(id, enabled, installed) VALUES(?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET enabled = excluded.enabled", rusqlite::params![idc, on as i64, setu_common::protocol::now_ms()])?;
            Ok(())
        });
        let j = self.json(e);
        drop(g);
        if !on {
            self.stop(id);
        }
        Ok(j)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let dir = self.entries.lock().unwrap().remove(id).map(|e| e.dir).ok_or("No such plugin.")?;
        self.stop(id);
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
        let idc = id.to_owned();
        self.db.write(move |c| {
            c.execute("DELETE FROM plugins WHERE id = ?1", [idc])?;
            Ok(())
        });
        Ok(())
    }

    pub fn name(&self, id: &str) -> Option<String> {
        self.entries.lock().unwrap().get(id).map(|e| e.manifest.name.clone())
    }

    pub fn tools(&self) -> Vec<PluginTool> {
        let g = self.entries.lock().unwrap();
        let mut out = Vec::new();
        for e in g.values().filter(|e| e.enabled && self.visible(e)) {
            for t in e.manifest.tools.iter().filter(|t| t.risk != "high") {
                out.push(PluginTool { name: tool_name(&e.manifest.id, &t.name), description: format!("{} ({})", t.description, e.manifest.name) });
            }
        }
        out
    }

    fn resolve(&self, tool: &str) -> Option<(String, String, PathBuf, Vec<String>)> {
        let g = self.entries.lock().unwrap();
        for e in g.values().filter(|e| e.enabled && self.visible(e)) {
            for t in &e.manifest.tools {
                if tool_name(&e.manifest.id, &t.name) == tool && t.risk != "high" {
                    return Some((e.manifest.id.clone(), t.name.clone(), e.dir.clone(), e.manifest.command.clone()));
                }
            }
        }
        None
    }

    fn stop(&self, id: &str) {
        if let Some(mut s) = self.servers.lock().unwrap().remove(id) {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }

    pub fn stop_idle(&self, idle: Duration) {
        let mut g = self.servers.lock().unwrap();
        let old: Vec<String> = g.iter().filter(|(_, s)| s.used.elapsed() > idle).map(|(k, _)| k.clone()).collect();
        for k in old {
            if let Some(mut s) = g.remove(&k) {
                let _ = s.child.kill();
                let _ = s.child.wait();
            }
        }
    }

    fn rpc(s: &mut Server, method: &str, params: Value, notify: bool) -> Result<Value, String> {
        let id = s.next;
        s.next += 1;
        let msg = if notify { json!({ "jsonrpc": "2.0", "method": method, "params": params }) } else { json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }) };
        writeln!(s.stdin, "{msg}").map_err(|e| e.to_string())?;
        s.stdin.flush().map_err(|e| e.to_string())?;
        if notify {
            return Ok(Value::Null);
        }
        let mut line = String::new();
        for _ in 0..200 {
            line.clear();
            let n = s.stdout.read_line(&mut line).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("The plugin stopped.".into());
            }
            if line.len() > 1 << 20 {
                return Err("The plugin sent too much data.".into());
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(e) = v.get("error") {
                    return Err(e.get("message").and_then(Value::as_str).unwrap_or("The plugin reported an error.").chars().take(200).collect());
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        Err("The plugin did not answer.".into())
    }

    fn start(dir: &Path, command: &[String]) -> Result<Server, String> {
        let (prog, args) = command.split_first().ok_or("The plugin has no command.")?;
        if prog.contains("..") {
            return Err("The plugin command is not allowed.".into());
        }
        let exe = if Path::new(prog).is_absolute() { PathBuf::from(prog) } else { dir.join(prog) };
        let mut child = Command::new(exe)
            .quiet()
            .args(args)
            .current_dir(dir)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("The plugin could not start: {e}"))?;
        let stdin = child.stdin.take().ok_or("plugin stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("plugin stdout")?);
        let mut s = Server { child, stdin, stdout, next: 1, used: Instant::now() };
        Self::rpc(&mut s, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "Setu", "version": setu_common::VERSION } }), false)?;
        Self::rpc(&mut s, "notifications/initialized", json!({}), true)?;
        Ok(s)
    }

    pub async fn call(&self, tool: &str, args: Value, cancel: Arc<AtomicBool>) -> Result<String, String> {
        let (id, real, dir, command) = self.resolve(tool).ok_or("That plugin tool isn't available.")?;
        let servers = &self.servers;
        let fut = async {
            tokio::task::block_in_place(|| {
                let mut g = servers.lock().unwrap();
                if !g.contains_key(&id) {
                    let s = Self::start(&dir, &command)?;
                    g.insert(id.clone(), s);
                }
                let s = g.get_mut(&id).unwrap();
                s.used = Instant::now();
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled.".to_owned());
                }
                let r = Self::rpc(s, "tools/call", json!({ "name": real, "arguments": args }), false);
                if r.is_err()
                    && let Some(mut dead) = g.remove(&id) {
                        let _ = dead.child.kill();
                    }
                r
            })
        };
        let result = tokio::time::timeout(Duration::from_secs(30), fut).await.map_err(|_| {
            self.stop(&id);
            "That action didn't respond.".to_owned()
        })??;
        let text: String = result
            .get("content")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
        let clean: String = text.chars().filter(|c| !c.is_control() || *c == ' ').take(600).collect();
        if result.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            return Err(if clean.is_empty() { "The plugin reported an error.".into() } else { clean });
        }
        Ok(if clean.is_empty() { "Done.".into() } else { clean })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_sanitized() {
        assert_eq!(tool_name("com.example.weather", "Get Forecast"), "weather_get_forecast");
        assert!(tool_name("x", &"a".repeat(100)).len() <= 48);
    }

    #[test]
    fn unsigned_and_tampered_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::write(p.join("plugin.json"), br#"{"id":"a","name":"A","publisher":"P","version":"1","command":["run"]}"#).unwrap();
        let bytes = std::fs::read(p.join("plugin.json")).unwrap();
        assert_eq!(verify(p, &bytes, &[]), Sig::Unsigned);
        std::fs::write(p.join("plugin.sig"), "AAAA").unwrap();
        assert_eq!(verify(p, &bytes, &[]), Sig::Invalid);
        assert!(digest(p, &bytes, &["../x".into()]).is_none());
    }
}
