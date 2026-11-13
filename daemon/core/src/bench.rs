use crate::core::Core;
use serde_json::{Value, json};
use setu_common::protocol::now_ms;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
#[cfg(target_os = "linux")]
use crate::proc::Quiet;

fn progress(core: &Core, item: &str, label: &str, value: Value, unit: &str, status: &str) -> Value {
    let v = json!({ "item": item, "label": label, "value": value, "unit": unit, "status": status });
    core.hub.emit("benchmark.progress", v.clone());
    v
}

fn cpu_flags() -> Vec<&'static str> {
    let mut v = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            v.push("AVX2");
        }
        if std::arch::is_x86_feature_detected!("fma") {
            v.push("FMA");
        }
        if std::arch::is_x86_feature_detected!("f16c") {
            v.push("F16C");
        }
        if std::arch::is_x86_feature_detected!("avx512f") {
            v.push("AVX-512");
        }
        if std::arch::is_x86_feature_detected!("avxvnni") {
            v.push("AVX-VNNI");
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        if std::arch::is_aarch64_feature_detected!("neon") {
            v.push("NEON");
        }
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            v.push("DOTPROD");
        }
        if std::arch::is_aarch64_feature_detected!("i8mm") {
            v.push("I8MM");
        }
    }
    v
}

fn gpu() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(o) = std::process::Command::new("nvidia-smi").quiet().args(["--query-gpu=name,memory.total", "--format=csv,noheader"]).stderr(std::process::Stdio::null()).output()
            && o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_owned();
                if !s.is_empty() {
                    return s;
                }
            }
        if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("card") && !n.contains('-')
                    && let Ok(v) = std::fs::read_to_string(e.path().join("device/vendor")) {
                        let name = match v.trim() {
                            "0x8086" => "Intel graphics",
                            "0x1002" => "AMD graphics",
                            "0x10de" => "NVIDIA graphics",
                            _ => "Graphics adapter",
                        };
                        return name.into();
                    }
            }
        }
        "No graphics adapter found".into()
    }
    #[cfg(not(target_os = "linux"))]
    {
        "Not measured on this system".into()
    }
}

fn bandwidth() -> f64 {
    let n = 64 * 1024 * 1024 / 8;
    let src = vec![1u64; n];
    let mut dst = vec![0u64; n];
    let mut best = 0f64;
    for _ in 0..3 {
        let t = Instant::now();
        dst.copy_from_slice(&src);
        std::hint::black_box(&dst);
        let s = t.elapsed().as_secs_f64();
        best = best.max((n * 8 * 2) as f64 / s / 1e9);
    }
    best
}

pub async fn run(core: Arc<Core>) {
    if core.bench_running.swap(true, Ordering::SeqCst) {
        return;
    }
    core.bench_cancel.store(false, Ordering::SeqCst);
    let mut items = Vec::new();
    let stop = || core.bench_cancel.load(Ordering::SeqCst);
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    let ram = sys.total_memory();
    items.push(progress(&core, "ram", "Memory", json!(format!("{:.1} GB", ram as f64 / 1e9)), "", "done"));
    let cpus = sys.cpus();
    let model = cpus.first().map(|c| c.brand().trim().to_owned()).unwrap_or_default();
    let physical = sysinfo::System::physical_core_count().unwrap_or(cpus.len());
    let flags = cpu_flags();
    items.push(progress(&core, "cpu", "Processor", json!(format!("{model}, {physical} cores, {} threads", cpus.len())), "", "done"));
    items.push(progress(&core, "isa", "Instruction sets", json!(if flags.is_empty() { "baseline".to_owned() } else { flags.join(" ") }), "", "done"));
    let g = match crate::gpu::best() {
        Some(d) => format!("{}, {:.1} GB, {}", d.name, d.total as f64 / 1e9, d.backend),
        None => crate::layers::blocking(gpu).await.unwrap_or_default(),
    };
    items.push(progress(&core, "gpu", "Graphics", json!(g), "", "done"));
    let free = crate::models::free_bytes(&core.paths.models);
    items.push(progress(&core, "disk", "Free disk", json!(format!("{:.1} GB", free as f64 / 1e9)), "", "done"));
    if stop() {
        return finish(&core, items, ram, 0.0, None, None);
    }
    progress(&core, "bandwidth", "Memory speed", Value::Null, "", "running");
    let bw = crate::layers::blocking(bandwidth).await.unwrap_or(0.0);
    items.push(progress(&core, "bandwidth", "Memory speed", json!((bw * 10.0).round() / 10.0), "GB/s", "done"));
    if stop() {
        return finish(&core, items, ram, bw, None, None);
    }
    progress(&core, "asr", "Speech recognition speed", Value::Null, "", "running");
    let rtf = asr_speed(&core).await;
    match rtf {
        Some((v, measured)) => items.push(progress(&core, "asr", if measured { "Speech recognition speed" } else { "Speech recognition speed (estimated)" }, json!((v * 1000.0).round() / 1000.0), "x real time", "done")),
        None => items.push(progress(&core, "asr", "Speech recognition speed", json!("not measured"), "", "skipped")),
    }
    if stop() {
        return finish(&core, items, ram, bw, rtf.map(|r| r.0), None);
    }
    progress(&core, "llm", "Language model speed", Value::Null, "", "running");
    let tps = llm_speed(&core, bw).await;
    items.push(progress(&core, "llm", if tps.1 { "Language model speed" } else { "Language model speed (estimated for 2B)" }, json!((tps.0 * 10.0).round() / 10.0), "tokens/s", "done"));
    finish(&core, items, ram, bw, rtf.map(|r| r.0), Some(tps.0));
}

async fn asr_speed(core: &Arc<Core>) -> Option<(f64, bool)> {
    if crate::pipeline::ensure_asr(core).await.is_ok() {
        let wav = core.paths.assets.join("benchmark.wav");
        let (samples, sr) = setu_sherpa::read_wave(&wav.to_string_lossy())?;
        if sr != 16_000 {
            return None;
        }
        let id = (1u64 << 61) | (now_ms() as u64 & 0xffff);
        let t = Instant::now();
        core.engines.asr.send(&setu_common::engine::ToEngine::AsrBegin { id, chars: false });
        for c in samples.chunks(1600) {
            core.engines.asr.pcm(c.to_vec());
        }
        let r = core.engines.asr.request(&setu_common::engine::ToEngine::AsrEnd { id }, crate::engines::Expect::Final(id), std::time::Duration::from_secs(60)).await.ok()?;
        let _ = r;
        let secs = samples.len() as f64 / 16_000.0;
        return Some((t.elapsed().as_secs_f64() / secs, true));
    }
    let wav = core.paths.assets.join("benchmark.wav");
    let (samples, _) = setu_sherpa::read_wave(&wav.to_string_lossy())?;
    let kd = core.paths.bundled("wake-gigaspeech-3m");
    let s = move |f: &str| kd.join(f).to_string_lossy().into_owned();
    crate::work::background("benchmark", move || {
        let asr = setu_sherpa::OnlineAsr::new(
            &setu_sherpa::TransducerPaths {
                encoder: s("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
                decoder: s("decoder-epoch-12-avg-2-chunk-16-left-64.onnx"),
                joiner: s("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
                tokens: s("tokens.txt"),
            },
            2,
        )
        .ok()?;
        let st = asr.stream();
        let t = Instant::now();
        for c in samples.chunks(1600) {
            asr.accept(&st, c);
            asr.decode(&st);
        }
        asr.finish(&st);
        let secs = samples.len() as f64 / 16_000.0;
        Some((t.elapsed().as_secs_f64() / secs * 4.0, false))
    })
    .await
    .ok()
    .flatten()
}

async fn llm_speed(core: &Arc<Core>, bw: f64) -> (f64, bool) {
    if crate::pipeline::ensure_llm(core).await.is_ok() && core.slot("llm").provider == "local" {
        let _g = core.engines.llm.exclusive().await;
        if let Ok(setu_common::engine::FromEngine::BenchResult { value, .. }) =
            core.engines.llm.request(&setu_common::engine::ToEngine::Bench { seconds: 3.0 }, crate::engines::Expect::Bench, std::time::Duration::from_secs(60)).await
        {
            crate::pipeline::reset_prefix();
            return (value, true);
        }
    }
    (bw * 0.55 / 1.28, false)
}

fn finish(core: &Core, items: Vec<Value>, ram: u64, bw: f64, rtf: Option<f64>, tps: Option<f64>) {
    let est = |gb: f64| tps.map(|t| t * 1.28 / gb).unwrap_or(bw * 0.55 / gb);
    let (rec, reason) = if ram >= 15_000_000_000 && est(2.74) >= 12.0 && rtf.is_none_or(|r| r < 0.3) {
        ("accurate", format!("Chosen because: this computer has {:.0} GB of memory and runs the 4B model at about {:.0} tokens per second.", ram as f64 / 1e9, est(2.74)))
    } else if ram >= 7_500_000_000 && est(1.28) >= 8.0 {
        ("balanced", format!("Chosen because: this computer has {:.0} GB of memory and runs the 2B model at about {:.0} tokens per second.", ram as f64 / 1e9, est(1.28)))
    } else {
        ("compact", format!("Chosen because: with {:.0} GB of memory the smallest models keep the computer responsive.", ram as f64 / 1e9))
    };
    let result = json!({ "items": items, "recommended": rec, "reason": reason, "at": now_ms(), "ramBytes": ram, "bandwidth": bw, "asrRtf": rtf, "llmTokensPerS": tps });
    *core.bench.lock().unwrap() = Some(result.clone());
    let it = serde_json::to_string(&result).unwrap_or_default();
    core.db.write(move |c| {
        c.execute("INSERT INTO benchmarks(time, items) VALUES(?1, ?2)", rusqlite::params![now_ms(), it])?;
        c.execute("DELETE FROM benchmarks WHERE id NOT IN (SELECT id FROM benchmarks ORDER BY time DESC LIMIT 20)", [])?;
        Ok(())
    });
    core.hub.emit("benchmark.result", result);
    core.bench_running.store(false, Ordering::SeqCst);
}

pub fn load_last(core: &Core) {
    let last: Option<String> = core.db.read(|c| Ok(c.query_row("SELECT items FROM benchmarks ORDER BY time DESC LIMIT 1", [], |r| r.get(0)).ok())).ok().flatten();
    if let Some(v) = last.and_then(|s| serde_json::from_str(&s).ok()) {
        *core.bench.lock().unwrap() = Some(v);
    }
}

pub fn history(core: &Core) -> Value {
    core.db
        .read(|c| {
            let mut st = c.prepare("SELECT items FROM benchmarks ORDER BY time DESC LIMIT 10")?;
            let v: Vec<Value> = st.query_map([], |r| r.get::<_, String>(0))?.flatten().filter_map(|s| serde_json::from_str::<Value>(&s).ok()).map(|v| json!({ "at": v["at"], "items": v["items"] })).collect();
            Ok(Value::Array(v))
        })
        .unwrap_or(Value::Array(vec![]))
}
