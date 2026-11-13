use crate::cdp::Tab;
use anyhow::{Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

pub struct Bidi {
    pub key: String,
    out: tokio::sync::mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>,
    next: AtomicU64,
    alive: AtomicBool,
    errors: Mutex<HashMap<String, Vec<String>>>,
}

static LIVE: Mutex<Vec<Arc<Bidi>>> = Mutex::new(Vec::new());

pub fn connected(key: &str) -> Option<Arc<Bidi>> {
    let mut g = LIVE.lock().unwrap();
    g.retain(|c| c.alive.load(Ordering::SeqCst));
    g.iter().find(|c| c.key == key).cloned()
}

pub fn any() -> Option<Arc<Bidi>> {
    let mut g = LIVE.lock().unwrap();
    g.retain(|c| c.alive.load(Ordering::SeqCst));
    g.first().cloned()
}

pub const PREFS: &[(&str, &str)] = &[
    ("browser.aboutwelcome.enabled", "false"),
    ("browser.startup.homepage_override.mstone", "\"ignore\""),
    ("datareporting.policy.dataSubmissionPolicyBypassNotification", "true"),
    ("toolkit.telemetry.reportingpolicy.firstRun", "false"),
    ("browser.shell.checkDefaultBrowser", "false"),
    ("trailhead.firstrun.didSeeAboutWelcome", "true"),
];

pub fn write_prefs(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join("user.js");
    let mut text = std::fs::read_to_string(&p).unwrap_or_default();
    for (k, v) in PREFS {
        if !text.contains(&format!("\"{k}\"")) {
            text.push_str(&format!("user_pref(\"{k}\", {v});\n"));
        }
    }
    std::fs::write(p, text)?;
    Ok(())
}

impl Bidi {
    fn handle(&self, msg: &str) {
        let Ok(v) = serde_json::from_str::<Value>(msg) else { return };
        if let Some(id) = v.get("id").and_then(Value::as_u64) {
            if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
                let _ = tx.send(v);
            }
            return;
        }
        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
        let p = v.get("params").cloned().unwrap_or(Value::Null);
        if method == "log.entryAdded" && p.get("level").and_then(Value::as_str) == Some("error") {
            let ctx = p.pointer("/source/context").and_then(Value::as_str).unwrap_or("").to_owned();
            let text = p.get("text").and_then(Value::as_str).unwrap_or("").lines().next().unwrap_or("").to_owned();
            if !text.is_empty() {
                let mut g = self.errors.lock().unwrap();
                let list = g.entry(ctx).or_default();
                if !list.contains(&text) {
                    list.push(text);
                }
                if list.len() > 50 {
                    list.remove(0);
                }
            }
        }
        if method == "browsingContext.contextDestroyed"
            && let Some(ctx) = p.get("context").and_then(Value::as_str)
        {
            self.errors.lock().unwrap().remove(ctx);
        }
        if method == "browsingContext.navigationStarted"
            && let Some(ctx) = p.get("context").and_then(Value::as_str)
        {
            self.errors.lock().unwrap().remove(ctx);
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(anyhow!("The browser closed."));
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "id": id, "method": method, "params": params });
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.out.send(msg.to_string()).map_err(|_| anyhow!("The browser closed."))?;
        let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.bidi");
        let r = match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(v)) => {
                if v.get("type").and_then(Value::as_str) == Some("error") {
                    Err(anyhow!("{}", v.get("message").and_then(Value::as_str).or_else(|| v.get("error").and_then(Value::as_str)).unwrap_or("The browser refused that.")))
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                }
            }
            Ok(Err(_)) => Err(anyhow!("The browser closed.")),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(anyhow!("The browser didn't answer in time."))
            }
        };
        span.end(r)
    }

    pub async fn eval(&self, context: &str, expression: &str) -> Result<Value> {
        let r = self.call("script.evaluate", json!({ "expression": expression, "target": { "context": context }, "awaitPromise": false })).await?;
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    pub async fn tabs(&self) -> Result<Vec<Tab>> {
        let r = self.call("browsingContext.getTree", json!({ "maxDepth": 0 })).await?;
        let mut out = Vec::new();
        for c in r.get("contexts").and_then(Value::as_array).cloned().unwrap_or_default() {
            let id = c.get("context").and_then(Value::as_str).unwrap_or_default().to_owned();
            let url = c.get("url").and_then(Value::as_str).unwrap_or_default().to_owned();
            let title = self.eval(&id, "document.title").await.ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
            out.push(Tab { id, title, url });
        }
        Ok(out)
    }

    pub async fn open(&self, url: &str) -> Result<String> {
        let r = self.call("browsingContext.create", json!({ "type": "tab" })).await?;
        let id = r.get("context").and_then(Value::as_str).ok_or_else(|| anyhow!("The browser didn't open a tab."))?.to_owned();
        self.navigate(&id, url).await?;
        Ok(id)
    }

    pub async fn activate(&self, id: &str) -> Result<()> {
        self.call("browsingContext.activate", json!({ "context": id })).await.map(|_| ())
    }

    pub async fn close(&self, id: &str) -> Result<()> {
        self.call("browsingContext.close", json!({ "context": id })).await.map(|_| ())
    }

    pub async fn navigate(&self, id: &str, url: &str) -> Result<()> {
        self.call("browsingContext.navigate", json!({ "context": id, "url": url, "wait": "none" })).await.map(|_| ())
    }

    pub async fn pause_media(&self, id: &str) -> Result<usize> {
        let v = self.eval(id, "(()=>{let n=0;for(const m of document.querySelectorAll('video,audio')){if(!m.paused){m.pause();n++}}return n})()").await?;
        Ok(v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)).unwrap_or(0) as usize)
    }

    pub async fn capture(&self, id: &str, pdf: bool) -> Result<Vec<u8>> {
        use base64::Engine;
        let r = if pdf { self.call("browsingContext.print", json!({ "context": id, "background": true })).await? } else { self.call("browsingContext.captureScreenshot", json!({ "context": id })).await? };
        let data = r.get("data").and_then(Value::as_str).ok_or_else(|| anyhow!("The browser gave no picture."))?;
        base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| anyhow!("The browser's file was damaged: {e}"))
    }

    pub async fn page_text(&self, id: &str, max: usize) -> Result<(String, String)> {
        let v = self.eval(id, "JSON.stringify((()=>{const m=document.querySelector('main,article,[role=main]')||document.body;return [document.title, m?m.innerText:'']})())").await?;
        let parsed: Value = serde_json::from_str(v.as_str().unwrap_or("[]")).unwrap_or(Value::Null);
        let title = parsed.get(0).and_then(Value::as_str).unwrap_or_default().to_owned();
        let body: String = parsed.get(1).and_then(Value::as_str).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        Ok((title, body.chars().take(max).collect()))
    }

    pub async fn console_errors(&self, id: &str) -> Result<Vec<String>> {
        tokio::time::sleep(Duration::from_millis(150)).await;
        Ok(self.errors.lock().unwrap().get(id).cloned().unwrap_or_default())
    }
}

pub async fn launch(key: &str, exe: &Path, dir: &Path, url: Option<&str>) -> Result<Arc<Bidi>> {
    use tokio::io::AsyncBufReadExt;
    write_prefs(dir)?;
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--profile").arg(dir).arg("--no-remote").arg("--remote-debugging-port=0");
    if let Some(u) = url {
        cmd.arg(u);
    }
    cmd.env_remove("GDK_DEBUG").env_remove("GTK_USE_PORTAL");
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.launch");
    let mut child = span.end(cmd.spawn().map_err(|e| anyhow!("The browser didn't start: {e}")))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("The browser gave no output."))?;
    let mut lines = tokio::io::BufReader::new(stderr).lines();
    let addr = tokio::time::timeout(Duration::from_secs(30), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(i) = line.find("ws://") {
                let a: String = line[i..].chars().take_while(|c| !c.is_whitespace()).collect();
                return Some(a);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| anyhow!("The browser didn't open its control channel."))?;
    tokio::spawn(async move {
        while let Ok(Some(_)) = lines.next_line().await {}
        let _ = child.wait().await;
    });
    let base = addr.trim_end_matches('/');
    let url = if base.ends_with("/session") { base.to_owned() } else { format!("{base}/session") };
    let (ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.map_err(|e| anyhow!("The browser's control channel refused Setu: {e}"))?;
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let me = Arc::new(Bidi { key: key.to_owned(), out: tx, pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1), alive: AtomicBool::new(true), errors: Mutex::new(HashMap::new()) });
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(Message::Text(m.into())).await.is_err() {
                break;
            }
        }
    });
    {
        let me = me.clone();
        tokio::spawn(async move {
            while let Some(Ok(m)) = stream.next().await {
                if let Message::Text(t) = m {
                    me.handle(t.as_str());
                }
            }
            me.alive.store(false, Ordering::SeqCst);
            me.pending.lock().unwrap().clear();
        });
    }
    me.call("session.new", json!({ "capabilities": {} })).await?;
    me.call("session.subscribe", json!({ "events": ["log.entryAdded", "browsingContext.contextDestroyed", "browsingContext.navigationStarted"] })).await?;
    LIVE.lock().unwrap().push(me.clone());
    Ok(me)
}
