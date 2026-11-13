use crate::core::Core;
use crate::proc::Quiet;
use setu_common::engine::Device;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

static DEVICES: RwLock<Option<Vec<Device>>> = RwLock::new(None);

fn probe(exe: &Path) -> Vec<Device> {
    let mut cmd = std::process::Command::new(exe);
    cmd.quiet();
    let Ok(mut child) = cmd.arg("probe").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() else {
        return Vec::new();
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                log::warn!("the graphics probe did not finish in time");
                return Vec::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut so, &mut out);
    }
    serde_json::from_str(out.trim()).unwrap_or_default()
}

pub fn devices() -> Vec<Device> {
    DEVICES.read().unwrap().clone().unwrap_or_default()
}

pub fn best() -> Option<Device> {
    devices().into_iter().filter(|d| d.kind == "gpu" || d.kind == "igpu").max_by_key(|d| (d.kind == "gpu", d.total))
}

pub fn apply(core: &Core) {
    let on = core.settings.bool("models.gpu") && best().is_some();
    core.engines.llm.set_gpu(on);
    core.engines.tts.set_gpu(on);
    core.engines.vision.set_gpu(on);
}

pub fn init(core: Arc<Core>) {
    std::thread::Builder::new()
        .name("gpu-probe".into())
        .spawn(move || {
            let t0 = Instant::now();
            let found = probe(&core.paths.engine_exe());
            match found.iter().filter(|d| d.kind != "cpu").max_by_key(|d| d.total) {
                Some(g) => log::info!("graphics: {} through {} with {} MB, found in {} ms", g.name, g.backend, g.total / 1_000_000, t0.elapsed().as_millis()),
                None => log::info!("graphics: no usable graphics card; models run on the processor"),
            }
            *DEVICES.write().unwrap() = Some(found);
            apply(&core);
        })
        .ok();
}

pub fn json() -> serde_json::Value {
    let g = best();
    serde_json::json!({
        "available": g.is_some(),
        "name": g.as_ref().map(|d| d.name.clone()),
        "backend": g.as_ref().map(|d| d.backend.clone()),
        "totalBytes": g.as_ref().map(|d| d.total),
        "freeBytes": g.as_ref().map(|d| d.free),
    })
}
