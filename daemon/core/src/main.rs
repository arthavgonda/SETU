#![cfg_attr(windows, windows_subsystem = "windows")]
mod access;
mod agent;
mod attention;
mod audio;
mod autostart;
mod bench;
mod board;
mod threads;
mod memory;
mod nowplaying;
mod os;
mod plan;
mod messaging;
mod browse;
mod browserui;
mod bidi;
mod cdp;
mod chat;
mod control;
mod core;
mod coverage;
mod diagnostics;
mod dictate;
mod download;
mod engines;
mod fileops;
mod foxctl;
mod faceid;
mod feed;
mod gpu;
mod handlers;
mod ide;
mod import;
mod retire;
mod hub;
mod ipc;
mod jobs;
mod layers;
mod looks;
mod logging;
mod metrics;
mod models;
mod names;
mod ocr;
mod password;
mod paths;
mod permissions;
mod pipeline;
mod playback;
mod plugins;
mod privileged;
mod profiles;
mod proto;
mod proc;
mod provision;
mod remote;
mod router;
mod rt;
mod secrets;
mod settings;
mod signup;
mod skills;
mod sounds;
mod speaker;
mod storage;
#[cfg(target_os = "linux")]
mod screen;
#[cfg(windows)]
mod wincap;
#[cfg(target_os = "macos")]
mod maccap;
mod syscfg;
mod uidetect;
mod syspass;
#[cfg(target_os = "linux")]
mod syscfg_linux;
#[cfg(target_os = "macos")]
mod syscfg_mac;
#[cfg(windows)]
mod syscfg_win;
mod text;
mod tools;
mod understand;
mod update;
mod wakeadapt;
mod wakeword;
mod work;
mod vault;
mod touchid;
mod vision;
mod wizard;
mod voicekey;
mod writer;
#[cfg(target_os = "linux")]
mod xdg;

use crate::attention::Issue;
use setu_face as face;
use crate::control::ControlMsg;
use crate::paths::Paths;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, mpsc};

static STOP: Notify = Notify::const_new();

pub fn shutdown() {
    STOP.notify_one();
}

fn crash_hook(paths: Paths) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("Setu {} crashed at {}\n{info}\n\n{bt}\n", setu_common::VERSION, jiff::Timestamp::now());
        let _ = std::fs::write(paths.crash.join(format!("daemon-{}.txt", setu_common::protocol::now_ms())), &text);
        log::error!("panic: {info}");
        prev(info);
    }));
}

#[cfg(unix)]
fn lock(paths: &Paths) -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(paths.runtime.join("daemon.lock")).ok()?;
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

#[cfg(windows)]
fn lock(paths: &Paths) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().create(true).write(true).truncate(false).share_mode(0).open(paths.runtime.join("daemon.lock")).ok()
}

fn browser_cli(args: &[String]) -> i32 {
    let i = args.iter().position(|a| a == "--browser").unwrap_or(0);
    let key = args.get(i + 1).cloned().unwrap_or_default();
    let mut exe = None;
    let mut urls = Vec::new();
    let mut flags = Vec::new();
    let mut j = i + 2;
    while j < args.len() {
        if args[j] == "--exe" {
            exe = args.get(j + 1).cloned();
            j += 2;
            continue;
        }
        if args[j].starts_with("--") {
            flags.push(args[j].clone());
        } else {
            urls.push(args[j].clone());
        }
        j += 1;
    }
    let paths = Paths::detect();
    #[cfg(unix)]
    if ask_daemon(&paths.runtime.join("daemon.sock"), &key, &urls, &flags) {
        return 0;
    }
    let Some(exe) = exe else { return 1 };
    let mut cmd = std::process::Command::new(exe);
    if cdp::has_profile(&paths.data, &key) {
        cmd.arg(format!("--user-data-dir={}", cdp::profile_dir(&paths.data, &key).display()));
    }
    cmd.args(&flags).args(&urls);
    if cmd.spawn().is_ok() { 0 } else { 1 }
}

#[cfg(unix)]
fn ask_daemon(sock: &std::path::Path, key: &str, urls: &[String], flags: &[String]) -> bool {
    use std::io::{Read, Write};
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(sock) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(40)));
    let send = |s: &mut std::os::unix::net::UnixStream, v: serde_json::Value| {
        let body = v.to_string().into_bytes();
        s.write_all(&(body.len() as u32).to_be_bytes()).and_then(|_| s.write_all(&body))
    };
    if send(&mut s, serde_json::json!({ "v": 1, "id": "b1", "type": "request", "method": "hello", "params": { "client": "setu-browser", "version": setu_common::VERSION, "role": "window", "protocol": 1 } })).is_err() {
        return false;
    }
    if send(&mut s, serde_json::json!({ "v": 1, "id": "b2", "type": "request", "method": "browser.launch", "params": { "key": key, "urls": urls, "flags": flags } })).is_err() {
        return false;
    }
    loop {
        let mut len = [0u8; 4];
        if s.read_exact(&mut len).is_err() {
            return false;
        }
        let n = u32::from_be_bytes(len) as usize;
        if n > 16 << 20 {
            return false;
        }
        let mut body = vec![0u8; n];
        if s.read_exact(&mut body).is_err() {
            return false;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else { continue };
        if v.get("id").and_then(serde_json::Value::as_str) == Some("b2") && v.get("type").and_then(serde_json::Value::as_str) == Some("response") {
            return v.get("ok").and_then(serde_json::Value::as_bool).unwrap_or(v.get("error").is_none());
        }
    }
}

const USAGE: &str = "Usage: setu-daemon [option]\n\n  (no option)            run the Setu daemon in the background\n  --foreground           run the daemon and log to the terminal\n  --version              print the version\n  --coverage [--os linux|windows|macos] [--json] [--no-probes]\n                         print what Setu can control on this system\n  --route                read commands on standard input, print how each is understood\n  --os-probe             accessibility probe for the test bench (JSON on standard input)\n  --browser KEY --exe PATH [URL]\n                         open a browser on its Setu profile\n  --help                 print this text";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--os-probe") {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
        std::process::exit(rt.block_on(os::probe()));
    }
    if args.iter().any(|a| a == "--browser") {
        std::process::exit(browser_cli(&args));
    }
    #[cfg(unix)]
    if args.iter().any(|a| a == "--route" || a == "--coverage") {
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    if args.iter().any(|a| a == "--route") {
        std::process::exit(coverage::route_lines());
    }
    if args.iter().any(|a| a == "--coverage") {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
        std::process::exit(rt.block_on(coverage::run(&args)));
    }
    if args.iter().any(|a| a == "--version") {
        println!("setu-daemon {} (protocol {}-{})", setu_common::VERSION, setu_common::PROTOCOL_MIN, setu_common::PROTOCOL_MAX);
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return;
    }
    if let Some(bad) = args.iter().skip(1).find(|a| a.as_str() != "--foreground" && !a.starts_with("-psn_")) {
        eprintln!("setu-daemon: unknown option {bad}\n\n{USAGE}");
        std::process::exit(2);
    }
    #[cfg(windows)]
    tools::dpi_aware();
    let paths = Paths::detect();
    if let Err(e) = paths.ensure() {
        eprintln!("setu-daemon: cannot create data folders: {e}");
        std::process::exit(1);
    }
    let foreground = args.iter().any(|a| a == "--foreground");
    logging::init(paths.logs.clone(), foreground);
    crash_hook(paths.clone());
    let Some(_lock) = lock(&paths) else {
        log::info!("another instance is already running");
        return;
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(work::blocking_threads())
        .thread_name("setu-rt")
        .thread_keep_alive(work::IDLE_THREAD)
        .enable_all()
        .build()
        .expect("runtime");
    let code = rt.block_on(run(paths));
    rt.shutdown_timeout(Duration::from_millis(500));
    std::process::exit(code);
}

fn bundled_models(paths: &Paths) -> audio::Models {
    let k = paths.bundled("wake-gigaspeech-3m");
    let s = |p: std::path::PathBuf| p.to_string_lossy().into_owned();
    audio::Models {
        kws: setu_sherpa::TransducerPaths {
            encoder: s(k.join("encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx")),
            decoder: s(k.join("decoder-epoch-12-avg-2-chunk-16-left-64.onnx")),
            joiner: s(k.join("joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx")),
            tokens: s(k.join("tokens.txt")),
        },
        bpe: s(k.join("bpe.model")),
        vad: s(paths.bundled("vad-silero").join("silero_vad.onnx")),
        denoise: s(paths.bundled("denoise-gtcrn").join("gtcrn_simple.onnx")),
    }
}

async fn level_pump(core: Arc<core::Core>) {
    let mut w = core.hub.level_watch();
    loop {
        while *w.borrow_and_update() == 0 {
            if w.changed().await.is_err() {
                return;
            }
        }
        let mut iv = tokio::time::interval(Duration::from_millis(33));
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = iv.tick() => {
                    let rms = core.audio.level();
                    core.hub.emit("audio.level", json!({ "rms": (rms * 10_000.0).round() / 10_000.0, "t": setu_common::protocol::now_ms() }));
                }
                r = w.changed() => {
                    if r.is_err() || *w.borrow() == 0 {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn watch_apps(core: Arc<core::Core>) {
    use notify::{RecursiveMode, Watcher};
    let (tx, mut rx) = mpsc::channel::<()>(8);
    let watcher = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
        if let Ok(e) = r {
            let change = matches!(e.kind, notify::EventKind::Create(_) | notify::EventKind::Remove(_) | notify::EventKind::Modify(notify::event::ModifyKind::Data(_) | notify::event::ModifyKind::Name(_) | notify::event::ModifyKind::Any));
            if change && e.paths.iter().any(|p| p.extension().is_some_and(|x| x == "desktop")) {
                let _ = tx.try_send(());
            }
        }
    });
    let Ok(mut watcher) = watcher else { return };
    for d in tools::apps::watch_dirs() {
        let _ = watcher.watch(&d, RecursiveMode::Recursive);
    }
    tokio::spawn(async move {
        let _keep = watcher;
        while rx.recv().await.is_some() {
            tokio::time::sleep(Duration::from_secs(3)).await;
            while rx.try_recv().is_ok() {}
            let _ = handlers::rescan(core.clone()).await;
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn watch_apps(_core: Arc<core::Core>) {}

#[cfg(target_os = "linux")]
async fn system_signals(core: Arc<core::Core>) {
    use futures_util::StreamExt;
    if let Ok(sys) = zbus::Connection::system().await {
        if let Ok(p) = zbus::Proxy::new(&sys, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await
            && let Ok(mut s) = p.receive_signal("PrepareForSleep").await {
                let c = core.clone();
                tokio::spawn(async move {
                    while let Some(m) = s.next().await {
                        let going: bool = m.body().deserialize().unwrap_or(false);
                        if !going {
                            log::info!("resumed from sleep; re-arming the microphone");
                            c.audio.send(audio::AudioCmd::DevicesChanged);
                            c.tell(ControlMsg::Settings(serde_json::Map::new()));
                        }
                    }
                });
            }
        if let Ok(p) = zbus::Proxy::new(&sys, "net.hadess.PowerProfiles", "/net/hadess/PowerProfiles", "net.hadess.PowerProfiles").await {
            let apply = |c: &core::Core, v: &str| {
                let saver = v == "power-saver";
                c.downloads.power_saver.store(saver, Ordering::Relaxed);
                if !saver {
                    c.downloads.wake();
                }
            };
            if let Ok(v) = p.get_property::<String>("ActiveProfile").await {
                apply(&core, &v);
            }
            let mut s = p.receive_property_changed::<String>("ActiveProfile").await;
            let c = core.clone();
            tokio::spawn(async move {
                while let Some(ch) = s.next().await {
                    if let Ok(v) = ch.get().await {
                        apply(&c, &v);
                    }
                }
            });
        }
    }
}

#[cfg(not(target_os = "linux"))]
async fn system_signals(_core: Arc<core::Core>) {}

async fn run(paths: Paths) -> i32 {
    let t0 = Instant::now();
    let opened = match storage::Db::open(&paths.db()) {
        Ok(o) => o,
        Err(e) => {
            log::error!("database could not be opened: {e}");
            return 1;
        }
    };
    let db = opened.db;
    let settings = settings::Settings::new(db.load_settings().unwrap_or_default());
    if settings.get("setup.version").as_i64() != Some(settings::SETUP_VERSION) {
        let mut save = serde_json::Map::new();
        settings.set_internal("setup.version", json!(settings::SETUP_VERSION));
        save.insert("setup.version".into(), json!(settings::SETUP_VERSION));
        db.save_settings(save);
    }
    let finished = provision::marker(&paths).is_file();
    if settings.bool("setup.completed") && !finished {
        provision::remember(&paths, true);
    } else if finished && !settings.bool("setup.completed") {
        log::info!("setup was finished before; it is not shown again");
        let mut done = serde_json::Map::new();
        done.insert("setup.completed".into(), json!(true));
        let save = settings.apply(&done);
        db.save_settings(save);
    }
    if settings.get("migrate.skills").as_i64() != Some(1) {
        let mut patch = serde_json::Map::new();
        patch.insert("permissions.control".into(), json!(true));
        patch.insert("permissions.files".into(), json!(true));
        if settings.f64("session.endSilence") >= 0.99 {
            patch.insert("session.endSilence".into(), json!(0.7));
        }
        if settings.get("models.unloadMinutes").as_i64() == Some(5) {
            patch.insert("models.unloadMinutes".into(), json!(15));
        }
        let mut save = settings.apply(&patch);
        settings.set_internal("migrate.skills", json!(1));
        save.insert("migrate.skills".into(), json!(1));
        db.save_settings(save);
        log::info!("enabled app control and file access for the new skills");
    }
    let hub = Arc::new(hub::Hub::new());
    let manifest = Arc::new(models::Manifest::load());
    let store = Arc::new(models::ModelStore::new(paths.clone(), db.clone(), manifest));
    let downloads = download::Downloads::new(store.clone(), hub.clone(), paths.clone());
    let (ctl_tx, ctl_rx) = mpsc::channel::<ControlMsg>(512);
    let (eng_tx, mut eng_rx) = mpsc::channel(256);
    let allow = vec![paths.models.to_string_lossy().into_owned(), paths.assets.to_string_lossy().into_owned()];
    let engines = engines::Engines::new(paths.engine_exe(), allow, eng_tx);
    let feed = feed::AsrFeed::new(engines.asr.clone());
    let (aud_tx, aud_rx) = crossbeam_channel::unbounded();
    let f2 = feed.clone();
    let audio = audio::start(bundled_models(&paths), aud_tx, Arc::new(move |c| f2.push(c)));
    let apps = tools::apps::AppIndex::new(db.clone());
    apps.set_language(&settings.str("ui.language"));
    let plugins = plugins::Plugins::new(paths.data.join("plugins"), db.clone(), settings.bool("plugins.developerMode"));
    let core = Arc::new(core::Core {
        paths: paths.clone(),
        settings,
        db,
        hub,
        attention: attention::Attention::default(),
        metrics: metrics::Metrics::default(),
        store,
        downloads,
        engines,
        audio,
        apps,
        feed,
        control: ctl_tx.clone(),
        started: Instant::now(),
        status: std::sync::Mutex::new("starting"),
        stopping: AtomicBool::new(false),
        bench: std::sync::Mutex::new(None),
        bench_running: AtomicBool::new(false),
        bench_cancel: AtomicBool::new(false),
        undoable: std::sync::Mutex::new(std::collections::HashSet::new()),
        pending_use: std::sync::Mutex::new(Vec::new()),
        last_paths: std::sync::Mutex::new(Vec::new()),
        plugins,
        board: Default::default(),
        threads: Default::default(),
        nowplaying: Default::default(),
    });
    board::restore(&core);
    bench::load_last(&core);
    gpu::init(core.clone());
    if let Some(p) = opened.recovered {
        core.attention.raise(
            &core.hub,
            Issue::new("storage.recovered", "warning", "Your history file was damaged, so Setu started a new one. The old file was kept.").action(
                "Open folder",
                "ui.openPath",
                json!({ "path": p }),
            ),
        );
    }
    for name in core.plugins.invalid() {
        core.attention.raise(&core.hub, Issue::new("plugin.invalid_signature", "danger", format!("{name} failed signature verification and was disabled.")).action("Open Plugins", "ui.open", json!({ "page": "settings/plugins" })));
    }
    if std::fs::read_dir(&paths.crash).map(|r| r.flatten().any(|e| e.file_name().to_string_lossy().starts_with("daemon-"))).unwrap_or(false) {
        core.attention.raise(&core.hub, Issue::new("daemon.restarted", "info", "Setu restarted after a problem.").action("Dismiss", "attention.dismiss", json!({ "code": "daemon.restarted" })));
        if let Ok(rd) = std::fs::read_dir(&paths.crash) {
            for e in rd.flatten() {
                let to = paths.crash.join(format!("seen-{}", e.file_name().to_string_lossy()));
                if e.file_name().to_string_lossy().starts_with("daemon-") {
                    let _ = std::fs::rename(e.path(), to);
                }
            }
        }
    }
    {
        let c = core.clone();
        core.downloads.on_done(Box::new(move |id| {
            log::info!("model {id} is ready");
            for s in handlers::slots_json(&c) {
                c.hub.emit("models.state", s);
            }
            c.tell(ControlMsg::ModelsChanged);
            handlers::finish_pending_use(&c);
            provision::finish_if_ready(&c);
        }));
    }
    let control_task = control::spawn(core.clone(), ctl_rx);
    provision::upgrade_voice(&core);
    provision::init(tokio::runtime::Handle::current());
    if provision::needed(&core) {
        let c = core.clone();
        tokio::spawn(async move {
            let mut roles = c.hub.roles_watch();
            let deadline = tokio::time::sleep(std::time::Duration::from_secs(90));
            tokio::pin!(deadline);
            loop {
                if roles.borrow_and_update().1 > 0 {
                    log::info!("first run: the setup window is open, so setup waits for it");
                    return;
                }
                tokio::select! {
                    _ = &mut deadline => break,
                    r = roles.changed() => if r.is_err() { break },
                }
            }
            if provision::needed(&c) {
                log::info!("first run: no setup window opened, so setup runs in the background");
                provision::run(c).await;
            }
        });
    }
    {
        let tx = ctl_tx.clone();
        tokio::spawn(async move {
            while let Some(e) = eng_rx.recv().await {
                if tx.send(ControlMsg::Engine(e)).await.is_err() {
                    break;
                }
            }
        });
    }
    {
        let tx = ctl_tx.clone();
        std::thread::Builder::new()
            .name("audio-events".into())
            .spawn(move || {
                while let Ok(e) = aud_rx.recv() {
                    if tx.blocking_send(ControlMsg::Audio(e)).is_err() {
                        break;
                    }
                }
            })
            .expect("audio events thread");
    }
    {
        let mut w = core.hub.roles_watch();
        let tx = ctl_tx.clone();
        tokio::spawn(async move {
            while w.changed().await.is_ok() {
                let (_, win) = *w.borrow();
                let _ = tx.send(ControlMsg::Roles(win)).await;
            }
        });
    }
    tokio::spawn(level_pump(core.clone()));
    tokio::spawn(nowplaying::run(core.clone()));
    syscfg::start(&core);
    jobs::start(&core);
    crate::os::start_tracking();
    tokio::spawn(tools::input::warm());
    tokio::spawn(tools::wm::warm());
    tokio::spawn(tools::channels::warm());
    #[cfg(target_os = "linux")]
    tokio::spawn(crate::work::background("apps", crate::tools::apps::refresh_ready));
    let watcher = {
        let c = core.clone();
        let checking = Arc::new(AtomicBool::new(false));
        setu_audio::Watcher::start(Box::new(move |line| {
            c.audio.send(audio::AudioCmd::DevicesChanged);
            if line.contains(" card ") && c.settings.bool("audio.wideband") && !checking.swap(true, Ordering::SeqCst) {
                let c2 = c.clone();
                let flag = checking.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(800));
                    if let Some(name) = tools::system::prefer_wideband_headset() {
                        log::info!("switched {name} to wideband speech");
                        c2.audio.send(audio::AudioCmd::DevicesChanged);
                    }
                    flag.store(false, Ordering::SeqCst);
                });
            }
        }))
    };
    if core.settings.bool("audio.wideband") {
        let c = core.clone();
        std::thread::spawn(move || {
            if let Some(name) = tools::system::prefer_wideband_headset() {
                log::info!("switched {name} to wideband speech");
                c.audio.send(audio::AudioCmd::DevicesChanged);
            }
        });
    }
    if core.apps.count() == 0 {
        let c = core.clone();
        tokio::spawn(async move {
            let _ = handlers::rescan(c).await;
        });
    }
    watch_apps(core.clone());
    tokio::spawn(system_signals(core.clone()));
    crate::vault::refresh(&core);
    tokio::spawn(crate::faceid::watch(core.clone()));
    retire::start(&core);
    if speaker::reset_old_prints(&core) {
        core.attention.raise(
            &core.hub,
            Issue::new("speaker.rerecord", "warning", "Voice matching was rebuilt in this version and is far more accurate. Your old voice recording could not be carried over, so record your voice again to use it.")
                .action("Record my voice", "ui.open", json!({ "page": "settings/voice" }))
                .action("Dismiss", "attention.dismiss", json!({ "code": "speaker.rerecord" })),
        );
    }
    {
        let c = core.clone();
        tokio::spawn(async move {
            loop {
                handlers::retention(&c);
                c.plugins.stop_idle(Duration::from_secs(600));
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            }
        });
    }
    let server = {
        let c = core.clone();
        tokio::spawn(async move {
            if let Err(e) = ipc::serve(c).await {
                log::error!("IPC server stopped: {e}");
                shutdown();
            }
        })
    };
    *core.status.lock().unwrap() = "running";
    core.emit_daemon();
    log::info!("Setu {} ready in {} ms", setu_common::VERSION, t0.elapsed().as_millis());

    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM");
        let mut int = signal(SignalKind::interrupt()).expect("SIGINT");
        tokio::select! {
            _ = STOP.notified() => {}
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = STOP.notified() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }

    log::info!("shutting down");
    if tools::streams::ducked() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), tools::streams::duck(false)).await;
    }
    #[cfg(target_os = "linux")]
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), screen::stop()).await;
    let t1 = Instant::now();
    *core.status.lock().unwrap() = "stopping";
    core.stopping.store(true, Ordering::Relaxed);
    core.emit_daemon();
    let _ = core.control.send(ControlMsg::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_millis(800), control_task).await;
    core.engines.stop_all();
    core.audio.send(audio::AudioCmd::Shutdown);
    drop(watcher);
    server.abort();
    let db = core.db.clone();
    let _ = tokio::time::timeout(Duration::from_millis(900), crate::layers::blocking(move || db.checkpoint())).await;
    let _ = std::fs::remove_file(core.paths.socket());
    log::info!("stopped in {} ms", t1.elapsed().as_millis());
    log::logger().flush();
    0
}
