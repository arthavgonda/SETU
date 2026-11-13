use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, watch};

pub type Frame = std::sync::Arc<Vec<u8>>;

const DROPPABLE: &[&str] = &["audio.level", "tts.level", "chat.delta", "camera.state", "face.state", "transcript.partial", "download.progress", "benchmark.progress"];

pub const TOPICS: &[&str] = &[
    "state.daemon",
    "state.mic",
    "session.state",
    "audio.level",
    "tts.level",
    "wake.detected",
    "wake.test",
    "transcript.partial",
    "transcript.final",
    "intent.resolved",
    "action.started",
    "action.progress",
    "action.finished",
    "prompt.ask",
    "prompt.confirm",
    "prompt.closed",
    "reply.speak",
    "reply.line",
    "reply.done",
    "notice.problem",
    "interaction.ended",
    "models.state",
    "download.progress",
    "benchmark.progress",
    "benchmark.result",
    "permissions.state",
    "apps.changed",
    "plugins.state",
    "attention.changed",
    "update.state",
    "settings.changed",
    "ui.command",
    "history.added",
    "wifi.state",
    "camera.state",
    "face.state",
    "vault.entry",
    "vault.reveal",
    "numbers.state",
    "look.state",
    "job.state",
    "wizard.command",
    "onboarding.answer",
    "onboarding.intent",
    "onboarding.heard",
    "onboarding.say",
    "onboarding.thinking",
    "models.used",
    "speaker.enroll",
    "skills.state",
    "notch.state",
    "notch.alert",
    "speaker.rejected",
    "chat.message",
    "prompt.update",
    "chat.delta",
    "chat.changed",
    "browsers.state",
    "media.now",
];

struct Conn {
    tx: mpsc::Sender<Frame>,
    topics: HashSet<String>,
    role: String,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct Hub {
    conns: Mutex<HashMap<u64, Conn>>,
    next: AtomicU64,
    level: watch::Sender<usize>,
    roles: watch::Sender<(usize, usize)>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}

impl Hub {
    pub fn new() -> Self {
        Self { conns: Mutex::new(HashMap::new()), next: AtomicU64::new(1), level: watch::channel(0).0, roles: watch::channel((0, 0)).0 }
    }

    pub fn register(&self, tx: mpsc::Sender<Frame>, kill: tokio::sync::oneshot::Sender<()>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.conns.lock().unwrap().insert(id, Conn { tx, topics: HashSet::new(), role: String::new(), kill: Some(kill) });
        id
    }

    pub fn set_role(&self, id: u64, role: &str) {
        if let Some(c) = self.conns.lock().unwrap().get_mut(&id) {
            c.role = role.to_owned();
        }
        self.recount();
    }

    pub fn unregister(&self, id: u64) {
        self.conns.lock().unwrap().remove(&id);
        self.recount();
    }

    pub fn subscribe(&self, id: u64, topics: &[String]) -> Vec<String> {
        let mut g = self.conns.lock().unwrap();
        let mut out = Vec::new();
        if let Some(c) = g.get_mut(&id) {
            for t in topics {
                if TOPICS.contains(&t.as_str()) {
                    c.topics.insert(t.clone());
                }
            }
            out = c.topics.iter().cloned().collect();
        }
        drop(g);
        self.recount();
        out.sort();
        out
    }

    pub fn unsubscribe(&self, id: u64, topics: &[String]) -> Vec<String> {
        let mut g = self.conns.lock().unwrap();
        let mut out = Vec::new();
        if let Some(c) = g.get_mut(&id) {
            for t in topics {
                c.topics.remove(t);
            }
            out = c.topics.iter().cloned().collect();
        }
        drop(g);
        self.recount();
        out.sort();
        out
    }

    fn recount(&self) {
        let g = self.conns.lock().unwrap();
        let level = g.values().filter(|c| c.topics.contains("audio.level")).count();
        let shells = g.values().filter(|c| c.role == "shell").count();
        let windows = g.values().filter(|c| c.role == "window").count();
        drop(g);
        self.level.send_if_modified(|v| std::mem::replace(v, level) != level);
        self.roles.send_if_modified(|v| std::mem::replace(v, (shells, windows)) != (shells, windows));
    }

    pub fn level_watch(&self) -> watch::Receiver<usize> {
        self.level.subscribe()
    }

    pub fn roles_watch(&self) -> watch::Receiver<(usize, usize)> {
        self.roles.subscribe()
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn has(&self, topic: &str) -> bool {
        self.conns.lock().unwrap().values().any(|c| c.topics.contains(topic))
    }

    pub fn emit(&self, topic: &str, data: Value) {
        let mut g = self.conns.lock().unwrap();
        if !g.values().any(|c| c.topics.contains(topic)) {
            return;
        }
        let msg = setu_common::protocol::event(topic, data);
        let Ok(body) = serde_json::to_vec(&msg) else { return };
        let Ok(frame) = setu_common::frame::encode(&body) else {
            log::warn!("event {topic} is larger than the frame limit and was not sent");
            return;
        };
        let frame: Frame = std::sync::Arc::new(frame);
        let droppable = DROPPABLE.contains(&topic);
        for c in g.values_mut() {
            if !c.topics.contains(topic) {
                continue;
            }
            if c.tx.try_send(frame.clone()).is_err() && !droppable
                && let Some(k) = c.kill.take() {
                    log::warn!("closing a client that is not reading its events");
                    let _ = k.send(());
                }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TOPICS;
    use std::path::{Path, PathBuf};

    fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                files(&p, ext, out);
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }

    fn quoted(text: &str) -> Vec<String> {
        text.split('"').skip(1).step_by(2).map(str::to_owned).collect()
    }

    #[test]
    fn every_used_topic_is_allowed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut rs = Vec::new();
        files(&root.join("daemon/core/src"), "rs", &mut rs);
        let mut missing = Vec::new();
        for f in rs {
            let all = std::fs::read_to_string(&f).unwrap_or_default();
            let text = all.split("#[cfg(test)]").next().unwrap_or_default();
            for part in text.split(".emit(").skip(1) {
                let t = part.trim_start();
                if let Some(rest) = t.strip_prefix('"')
                    && let Some(topic) = rest.split('"').next()
                    && !TOPICS.contains(&topic)
                {
                    missing.push(format!("{} emits {topic}", f.display()));
                }
            }
        }
        let mut qml = Vec::new();
        files(&root.join("qml"), "qml", &mut qml);
        for f in qml {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for part in text.split("topics:").skip(1) {
                let Some(list) = part.split_once('[').and_then(|(_, r)| r.split_once(']')).map(|(l, _)| l) else { continue };
                for topic in quoted(list) {
                    if !TOPICS.contains(&topic.as_str()) {
                        missing.push(format!("{} listens to {topic}", f.display()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "topics missing from TOPICS: {missing:?}");
    }
}
