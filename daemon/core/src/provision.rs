use crate::control::ControlMsg;
use crate::core::Core;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static RUNNING: AtomicBool = AtomicBool::new(false);
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

pub fn init(handle: tokio::runtime::Handle) {
    let _ = RUNTIME.set(handle);
}

pub fn marker(paths: &crate::paths::Paths) -> std::path::PathBuf {
    paths.data.join("setup-complete")
}

pub fn remember(paths: &crate::paths::Paths, done: bool) {
    let path = marker(paths);
    if done {
        let body = format!("{}\n{}\n", setu_common::VERSION, setu_common::protocol::now_ms());
        let r = std::fs::File::create(&path).and_then(|mut f| {
            use std::io::Write;
            f.write_all(body.as_bytes())?;
            f.sync_all()
        });
        if let Err(e) = r {
            log::warn!("the finished setup could not be remembered: {e}");
        }
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

pub fn needed(core: &Core) -> bool {
    !core.settings.bool("setup.completed") && core.settings.str("models.tier").is_empty() && !core.settings.bool("setup.rerun")
}

fn ready(core: &Core) -> bool {
    crate::handlers::slots_json(core).iter().all(|s| s["status"] != "missing")
}

pub async fn run(core: Arc<Core>) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    core.tell(ControlMsg::Announce("Hello. I'm getting ready by downloading my speech models. This takes a few minutes, and I'll tell you when I'm ready.".into()));
    if core.bench.lock().unwrap().is_none() {
        crate::bench::run(core.clone()).await;
    }
    let tier = core.bench.lock().unwrap().as_ref().and_then(|b| b["recommended"].as_str().map(str::to_owned)).unwrap_or_else(|| "compact".into());
    log::info!("first run: setting up the {tier} tier");
    if let Err(e) = crate::handlers::settings_set(&core, &json!({ "key": "models.tier", "value": tier })).await {
        log::warn!("first run: the tier could not be set: {}", e.message);
    }
    let Some(def) = core.store.manifest.tier(&tier) else {
        RUNNING.store(false, Ordering::SeqCst);
        return;
    };
    let ids = core.store.manifest.tier_models(def, &core.lang());
    match core.downloads.enqueue(&ids) {
        Ok(_) => finish_if_ready(&core),
        Err(e) => {
            log::warn!("first run: the download could not start: {}", e.message);
            core.tell(ControlMsg::Announce(format!("I couldn't download my speech models. {}", e.message)));
            RUNNING.store(false, Ordering::SeqCst);
        }
    }
}

pub fn upgrade_voice(core: &Core) {
    if !core.settings.bool("setup.completed") {
        return;
    }
    let explicit = core.settings.get("models.slots").get("tts").is_some();
    let slot = core.slot("tts");
    if explicit || slot.provider != "local" || slot.models.iter().all(|m| core.store.is_installed(m)) {
        return;
    }
    let ids = core.store.manifest.with_requires(&slot.models);
    match core.downloads.enqueue(&ids) {
        Ok(_) => log::info!("downloading the new default voice: {}", slot.models.join(", ")),
        Err(e) => log::warn!("the new default voice could not be downloaded: {}", e.message),
    }
}

pub fn finish_if_ready(core: &Arc<Core>) {
    if core.settings.bool("setup.completed") || !ready(core) {
        return;
    }
    let Some(rt) = RUNTIME.get() else { return };
    let c = core.clone();
    rt.spawn(async move {
        if crate::handlers::settings_set(&c, &json!({ "key": "setup.completed", "value": true })).await.is_ok() {
            let phrase = c.settings.phrase();
            c.tell(ControlMsg::Announce(format!("I'm ready. Say {phrase} when you need me.")));
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}
