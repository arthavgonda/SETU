use crate::attention::Attention;
use crate::audio::AudioCtl;
use crate::control::ControlMsg;
use crate::download::Downloads;
use crate::engines::Engines;
use crate::feed::AsrFeed;
use crate::hub::Hub;
use crate::metrics::Metrics;
use crate::models::{ModelStore, SlotConfig};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::storage::Db;
use crate::tools::apps::AppIndex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

pub struct Core {
    pub paths: Paths,
    pub settings: Settings,
    pub db: Db,
    pub hub: Arc<Hub>,
    pub attention: Attention,
    pub metrics: Metrics,
    pub store: Arc<ModelStore>,
    pub downloads: Arc<Downloads>,
    pub engines: Engines,
    pub audio: AudioCtl,
    pub apps: Arc<AppIndex>,
    pub feed: Arc<AsrFeed>,
    pub control: mpsc::Sender<ControlMsg>,
    pub started: Instant,
    pub status: Mutex<&'static str>,
    pub stopping: AtomicBool,
    pub bench: Mutex<Option<Value>>,
    pub bench_running: AtomicBool,
    pub bench_cancel: AtomicBool,
    pub plugins: crate::plugins::Plugins,
    pub undoable: Mutex<std::collections::HashSet<String>>,
    pub pending_use: Mutex<Vec<(String, String, Option<String>)>>,
    pub last_paths: Mutex<Vec<std::path::PathBuf>>,
    pub board: crate::board::Board,
    pub threads: crate::threads::Threads,
    pub nowplaying: crate::nowplaying::State,
}

impl Core {
    pub fn tell(&self, m: ControlMsg) {
        if self.control.try_send(m).is_err() {
            log::warn!("control queue is full; a message was dropped");
        }
    }

    pub fn live_undo(&self, mut item: Value) -> Value {
        let id = item.pointer("/action/id").and_then(Value::as_str).map(str::to_owned);
        let live = id.is_some_and(|id| self.undoable.lock().unwrap().contains(&id));
        if let Some(o) = item.as_object_mut() {
            o.insert("undoable".into(), Value::Bool(live));
        }
        item
    }

    pub fn lang(&self) -> String {
        self.settings.str("ui.language")
    }

    pub fn defaults(&self) -> HashMap<String, String> {
        self.settings
            .get("apps.defaults")
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned()))).collect())
            .unwrap_or_default()
    }

    pub fn tier(&self) -> String {
        let t = self.settings.str("models.tier");
        if t.is_empty() { self.bench.lock().unwrap().as_ref().and_then(|b| b["recommended"].as_str().map(str::to_owned)).unwrap_or_else(|| "balanced".into()) } else { t }
    }

    pub fn slots(&self) -> BTreeMap<String, SlotConfig> {
        let mut base = self.store.default_slots(&self.tier(), &self.lang());
        if let Some(o) = self.settings.get("models.slots").as_object() {
            for (k, v) in o {
                if let Ok(s) = serde_json::from_value::<SlotConfig>(v.clone()) {
                    base.insert(k.clone(), s);
                }
            }
        }
        base
    }

    pub fn slot(&self, name: &str) -> SlotConfig {
        self.slots().remove(name).unwrap_or_default()
    }

    pub fn set_slot(&self, name: &str, cfg: Option<SlotConfig>) {
        let mut m = self.settings.get("models.slots").as_object().cloned().unwrap_or_default();
        match cfg {
            Some(c) => {
                m.insert(name.to_owned(), serde_json::to_value(c).unwrap_or(Value::Null));
            }
            None => {
                m.remove(name);
            }
        }
        self.settings.set_internal("models.slots", Value::Object(m.clone()));
        let mut save = Map::new();
        save.insert("models.slots".into(), Value::Object(m));
        self.db.save_settings(save);
    }

    pub fn daemon_state(&self) -> Value {
        let comps: Vec<Value> = [("audio", self.audio.open.load(std::sync::atomic::Ordering::Relaxed)), ("wake", true), ("storage", true), ("ipc", true)]
            .iter()
            .map(|(id, ok)| json!({ "id": id, "status": if *ok { "ok" } else { "degraded" }, "restarts": 0, "message": null }))
            .chain(self.engines.all().iter().map(|e| {
                let status = if e.degraded() {
                    "degraded"
                } else if e.is_running() {
                    "ok"
                } else {
                    "stopped"
                };
                json!({ "id": e.kind.as_str(), "status": status, "restarts": 0, "message": null })
            }))
            .collect();
        json!({
            "status": *self.status.lock().unwrap(),
            "uptimeS": self.started.elapsed().as_secs(),
            "version": setu_common::VERSION,
            "components": comps,
        })
    }

    pub fn emit_daemon(&self) {
        self.hub.emit("state.daemon", self.daemon_state());
    }
}
