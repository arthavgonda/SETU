use crate::core::Core;
use crate::face::{self, Verdict};
use crate::router::Intent;
use crate::tools::{Choice, FollowUp, Outcome};
use base64::Engine;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

const LAYER: &str = "face";
static CANCEL: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);
static FAILS: AtomicU8 = AtomicU8::new(0);
pub const TRIES: u8 = 3;

pub fn installed(core: &Core) -> bool {
    face::MODELS.iter().all(|m| core.store.is_installed(m))
}

fn models(core: &Core) -> anyhow::Result<face::Models> {
    let det = core.store.file(face::MODELS[0], "model").ok_or_else(|| anyhow::anyhow!("The face models aren't downloaded yet."))?;
    let rec = core.store.file(face::MODELS[1], "model").ok_or_else(|| anyhow::anyhow!("The face models aren't downloaded yet."))?;
    face::Models::load(&det, &rec, crate::work::background_lanes().clamp(1, 2) as i32)
}

pub fn cancel() -> Value {
    CANCEL.store(true, Ordering::Relaxed);
    json!({ "cancelled": true })
}

pub async fn state(core: &Core) -> Value {
    let root = root_status().await;
    let on = |k: &str| root.as_ref().and_then(|r| r.get(k)).and_then(Value::as_bool).unwrap_or(false);
    json!({
        "enrolled": crate::vault::face_enrolled(core),
        "models": installed(core),
        "unlock": core.settings.bool("face.unlock"),
        "locked": FAILS.load(Ordering::Relaxed) >= TRIES,
        "system": crate::vault::system(core).is_some(),
        "helper": root.is_some(),
        "signin": on("signin"),
        "admin": on("admin"),
        "signinReady": on("enrolled") && on("password"),
        "seal": root.as_ref().and_then(|r| r.get("seal")).and_then(Value::as_str).unwrap_or(""),
    })
}

async fn root_status() -> Option<Value> {
    if !cfg!(target_os = "linux") || !crate::privileged::installed() {
        return None;
    }
    crate::privileged::call("face.status", json!({}), Duration::from_secs(2)).await.ok()
}

async fn root_enroll(core: &Arc<Core>) -> Result<Value, String> {
    let t = crate::vault::face_load(core).await.map_err(|e| e.to_string())?.ok_or("Face unlock isn't set up. Say set up face unlock.")?;
    let sys = crate::vault::system(core).ok_or("Your system password isn't in the vault yet.")?;
    let det = core.store.file(face::MODELS[0], "model").ok_or("The face models aren't downloaded yet.")?;
    let rec = core.store.file(face::MODELS[1], "model").ok_or("The face models aren't downloaded yet.")?;
    let mut password = crate::vault::secret(core, &sys.uid).await.map_err(|e| e.to_string())?;
    let mut args = json!({ "templates": t, "password": password, "det": det, "rec": rec });
    crate::vault::wipe(&mut password);
    let r = crate::privileged::call("face.enroll", args.clone(), Duration::from_secs(60)).await.map_err(|e| e.to_string());
    if let Some(Value::String(p)) = args.get_mut("password") {
        crate::vault::wipe(p);
    }
    r
}

pub async fn sync_root(core: &Arc<Core>) {
    let Some(st) = root_status().await else { return };
    let on = |k: &str| st.get(k).and_then(Value::as_bool).unwrap_or(false);
    if !(on("signin") || on("admin")) {
        return;
    }
    match root_enroll(core).await {
        Ok(_) => log::info!("updated the face data and password kept for the sign-in screen"),
        Err(e) => log::warn!("the sign-in screen's copy could not be updated: {e}"),
    }
}

pub fn root_confirm(tool: &str, args: &Value) -> Option<(String, Vec<String>, String)> {
    if args.get("on").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(match tool {
        "face.signin" => (
            "Sign in to this computer with your face?".into(),
            vec![
                "At the sign-in screen you pick your name and I check your face, so you don't type the password.".into(),
                "Whoever passes the face check gets into this computer. A photo is refused, but a good video of you on a screen can pass an ordinary webcam.".into(),
            ],
            "Your system password is kept sealed on this computer for the sign-in screen.".into(),
        ),
        _ => (
            "Approve admin prompts with your face?".into(),
            vec!["Admin prompts and sudo will take your face and a nod instead of the password.".into(), "The nod is there so that nothing can use your face without you meaning it.".into()],
            "Your system password is kept sealed on this computer for those prompts.".into(),
        ),
    })
}

async fn root_switch(core: &Arc<Core>, tool: &str, args: &Value, cancel: Arc<AtomicBool>) -> Outcome {
    let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
    let signin = tool == "face.signin";
    let what = if signin { "Face sign-in" } else { "Face for admin prompts" };
    if !(cfg!(target_os = "linux") || (cfg!(windows) && signin)) {
        return Outcome::fail(&format!("{what} isn't available on this system yet."), LAYER);
    }
    if !crate::privileged::installed() {
        if !on {
            return Outcome::ok_with(&format!("{what} is off"), LAYER, None).with_reply(format!("{what} is off."));
        }
        let mut o = Outcome::fail("That needs admin control, because the sign-in screen runs before you are signed in. Say set up admin control first.", LAYER);
        o.fix = Some(json!({ "label": "Set up admin control", "method": "intent.run", "params": { "text": "set up admin control" } }));
        return o;
    }
    let action = if signin { "face.signin" } else { "face.admin" };
    if !on {
        return match crate::privileged::call(action, json!({ "on": false }), Duration::from_secs(30)).await {
            Ok(_) => Outcome::ok_with(&format!("{what} off"), LAYER, None).with_reply(if signin { "Face sign-in is off. The sign-in screen will want your password.".into() } else { "Admin prompts will want your password again.".into() }),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        };
    }
    if !crate::vault::face_enrolled(core) || !installed(core) {
        return Outcome::fail("Set up face unlock first. Say set up face unlock.", LAYER);
    }
    if crate::vault::system(core).is_none() {
        crate::vault::open_card(core, "system", crate::vault::SYSTEM, &crate::syspass::user().unwrap_or_default(), "");
        return Outcome::fail("I need your system password in the vault first. Type it in the card, then ask me again.", LAYER);
    }
    if let Err(e) = verify(core, if signin { "turn on face sign-in" } else { "use your face for admin prompts" }, cancel).await {
        return Outcome::fail(&e, LAYER);
    }
    if let Err(e) = root_enroll(core).await {
        return Outcome::fail(&e, LAYER);
    }
    match crate::privileged::call(action, json!({ "on": true }), Duration::from_secs(60)).await {
        Ok(st) => {
            let note = st.get("note").and_then(Value::as_str).map(|n| format!(" {n}")).unwrap_or_default();
            let seal = if st.get("seal").and_then(Value::as_str) == Some("tpm2") { " Your password is sealed with this computer's security chip." } else { "" };
            let reply = if signin {
                if st.get("noname").and_then(Value::as_bool) == Some(true) || cfg!(windows) {
                    format!("Face sign-in is on. The sign-in screen recognises you by face, with no name to pick and nothing to type. If it can't see you, it asks for your user name and password as usual.{seal}{note}")
                } else {
                    format!("Face sign-in is on. At the sign-in screen, look at the camera.{seal}{note}")
                }
            } else {
                format!("Done. When an admin prompt opens, look at the camera and nod.{seal}{note}")
            };
            Outcome::ok_with(&format!("{what} on"), LAYER, None).with_reply(reply)
        }
        Err(e) => Outcome::fail(&e.to_string(), LAYER),
    }
}

struct Busy;

impl Busy {
    fn take() -> Option<Busy> {
        (!BUSY.swap(true, Ordering::SeqCst)).then_some(Busy)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

fn ticker(core: &Arc<Core>, mode: &'static str, stop: Arc<AtomicBool>, outer: Arc<AtomicBool>) -> impl FnMut(face::Tick) + Send + 'static {
    let hub = core.hub.clone();
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut said: Option<String> = None;
    move |t: face::Tick| {
        if CANCEL.load(Ordering::Relaxed) || outer.load(Ordering::Relaxed) {
            stop.store(true, Ordering::Relaxed);
        }
        let changed = t.message.map(str::to_owned) != said;
        if !changed && last.elapsed() < Duration::from_millis(120) {
            return;
        }
        last = Instant::now();
        said = t.message.map(str::to_owned);
        let frame = if mode == "lock" { None } else { face::preview(t.img).map(|j| base64::engine::general_purpose::STANDARD.encode(j)) };
        hub.emit("face.state", json!({ "open": true, "mode": mode, "frame": frame, "steps": t.steps, "message": t.message }));
    }
}

pub async fn check(core: &Arc<Core>, mode: &'static str, limit: Duration, outer: Arc<AtomicBool>) -> Result<Verdict, String> {
    let Some(_busy) = Busy::take() else { return Err("The camera is busy with another face check.".into()) };
    let mut t = crate::vault::face_load(core).await.map_err(|e| e.to_string())?.ok_or("Face unlock isn't set up. Say set up face unlock.")?;
    CANCEL.store(false, Ordering::Relaxed);
    let stop = Arc::new(AtomicBool::new(false));
    let mut tick = ticker(core, mode, stop.clone(), outer);
    let (c, t2) = (core.clone(), t.clone());
    core.hub.emit("face.state", json!({ "open": true, "mode": mode, "frame": null, "steps": {}, "message": "Look at the camera." }));
    let span = crate::layers::read(crate::layers::Layer::Vision, "face.check");
    let r = crate::work::background("face", move || -> Result<(Verdict, Option<Vec<f32>>), String> {
        let m = models(&c).map_err(|e| e.to_string())?;
        let mut frames = face::open_frames().map_err(|e| e.to_string())?;
        face::check(&m, frames.as_mut(), &t2, &stop, limit, &mut tick).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    core.hub.emit("face.state", json!({ "open": false, "mode": mode }));
    let (v, learn) = span.end(r)?;
    if let (Verdict::Me(_), Some(vec)) = (&v, learn)
        && t.learn(&vec)
        && let Err(e) = crate::vault::face_save(core, &t).await
    {
        log::warn!("the face template could not be updated: {e}");
    }
    Ok(v)
}

fn counts(v: &Verdict) -> bool {
    matches!(v, Verdict::NotMe | Verdict::Still)
}

#[cfg(windows)]
async fn hello(why: &str) -> Option<bool> {
    let why = format!("Setu: {why}");
    crate::layers::blocking(move || {
        use windows::Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability};
        let a = UserConsentVerifier::CheckAvailabilityAsync().ok()?.get().ok()?;
        if a != UserConsentVerifierAvailability::Available {
            return None;
        }
        let r = UserConsentVerifier::RequestVerificationAsync(&windows::core::HSTRING::from(why)).ok()?.get().ok()?;
        Some(r == UserConsentVerificationResult::Verified)
    })
    .await
    .ok()
    .flatten()
}

pub async fn verify(core: &Arc<Core>, why: &str, outer: Arc<AtomicBool>) -> Result<(), String> {
    crate::speaker::key_ok(core)?;
    let mut note = String::new();
    if crate::vault::face_enrolled(core) && installed(core) && FAILS.load(Ordering::Relaxed) < TRIES {
        match check(core, "check", Duration::from_secs(8), outer.clone()).await {
            Ok(Verdict::Me(_)) => {
                FAILS.store(0, Ordering::Relaxed);
                return Ok(());
            }
            Ok(Verdict::Cancelled) => return Err("Stopped.".into()),
            Ok(v) => {
                let n = if counts(&v) { FAILS.fetch_add(1, Ordering::Relaxed) + 1 } else { FAILS.load(Ordering::Relaxed) };
                if n < TRIES {
                    return Err(format!("{} Ask again to retry.", v.say()));
                }
                note = "That's three failed face checks. ".into();
            }
            Err(e) if e.contains("camera") || e.contains("Camera") => note = format!("{e} "),
            Err(e) => return Err(e),
        }
    }
    #[cfg(windows)]
    if let Some(ok) = hello(why).await {
        return if ok { Ok(()) } else { Err("Windows Hello didn't confirm it's you.".into()) };
    }
    #[cfg(target_os = "macos")]
    if let Some(ok) = crate::touchid::verify(why).await {
        return if ok { Ok(()) } else { Err("Touch ID didn't confirm it's you.".into()) };
    }
    let (id, mut rx) = crate::vault::open_card(core, "confirm", &crate::text::sentence_case(why), "", "");
    crate::board::alert(core, "vault", &format!("{note}Type your system password in the card to continue."));
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        tokio::select! {
            r = &mut rx => {
                return match r {
                    Ok(true) => {
                        FAILS.store(0, Ordering::Relaxed);
                        Ok(())
                    }
                    _ => Err("Stopped. Nothing was shown or typed.".into()),
                };
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if outer.load(Ordering::Relaxed) || Instant::now() >= end {
                    crate::vault::cancel(core, &id);
                    return Err("Stopped. Nothing was shown or typed.".into());
                }
            }
        }
    }
}

fn offer(then: Intent) -> Outcome {
    let then = match &then {
        Intent::Act { tool, args } => json!({ "tool": tool, "args": args }),
        _ => Value::Null,
    };
    let q = "Face unlock needs two small face models, a 39 megabyte download that runs only on this computer. Download them?";
    let yes = Intent::Act { tool: "face.models".into(), args: json!({ "then": then }) };
    let no = Intent::Act { tool: "face.none".into(), args: json!({}) };
    Outcome::asking("Asked to download the face models", LAYER, FollowUp { question: q.into(), spoken: Some(q.into()), choices: vec![Choice::new("Download", &["yes", "download it", "ok", "okay", "sure", "go ahead"], yes), Choice::new("Not now", &["no", "not now", "cancel"], no)], cleanup: None, kind: "confirm", focus: None })
}

pub fn rules(t: &str) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    if matches!(t, "set up face unlock" | "setup face unlock" | "set up face id" | "register my face" | "set up my face" | "add my face" | "enrol my face" | "enroll my face" | "scan my face" | "save my face" | "learn my face" | "set up face recognition" | "set up my face again" | "register my face again") {
        return act("face.enrol", json!({}));
    }
    if matches!(t, "check my face" | "is this me" | "do you recognise me" | "do you recognize me" | "test face unlock" | "can you recognise me" | "can you recognize me") {
        return act("face.check", json!({}));
    }
    if matches!(t, "forget my face" | "delete my face" | "remove my face" | "delete my face data" | "forget my face data") {
        return act("face.forget", json!({}));
    }
    if matches!(t, "turn on face unlock" | "turn face unlock on" | "enable face unlock" | "unlock with my face") {
        return act("face.auto", json!({ "on": true }));
    }
    if matches!(t, "turn off face unlock" | "turn face unlock off" | "disable face unlock" | "stop unlocking with my face") {
        return act("face.auto", json!({ "on": false }));
    }
    let t = t.replace("sign-in", "sign in").replace("signin", "sign in").replace("log in", "sign in").replace("login", "sign in");
    let t = t.as_str();
    if matches!(t, "use my face to sign in" | "turn on face sign in" | "turn face sign in on" | "enable face sign in" | "set up face sign in" | "sign me in with my face" | "use my face at the sign in screen" | "use my face on the sign in screen" | "unlock the sign in screen with my face" | "sign in with my face") {
        return act("face.signin", json!({ "on": true }));
    }
    if matches!(t, "turn off face sign in" | "turn face sign in off" | "disable face sign in" | "stop using my face to sign in" | "do not use my face to sign in" | "don't use my face to sign in") {
        return act("face.signin", json!({ "on": false }));
    }
    if matches!(t, "use my face for admin prompts" | "approve admin prompts with my face" | "use my face for sudo" | "use my face for administrator prompts" | "turn on face for admin prompts" | "use my face instead of my password for admin prompts" | "let me approve admin prompts with my face" | "use my face for password prompts") {
        return act("face.admin", json!({ "on": true }));
    }
    if matches!(t, "stop using my face for admin prompts" | "turn off face for admin prompts" | "do not use my face for admin prompts" | "don't use my face for admin prompts" | "stop using my face for sudo") {
        return act("face.admin", json!({ "on": false }));
    }
    None
}

pub fn label(tool: &str, args: &Value) -> String {
    match tool {
        "face.enrol" => "Setting up face unlock".into(),
        "face.check" => "Checking your face".into(),
        "face.forget" => "Forgetting your face".into(),
        "face.auto" if args.get("on").and_then(Value::as_bool) == Some(false) => "Turning off face unlock".into(),
        "face.auto" => "Turning on face unlock".into(),
        "face.models" => "Downloading the face models".into(),
        "face.signin" if args.get("on").and_then(Value::as_bool) == Some(false) => "Turning off face sign-in".into(),
        "face.signin" => "Turning on face sign-in".into(),
        "face.admin" if args.get("on").and_then(Value::as_bool) == Some(false) => "Turning off face for admin prompts".into(),
        "face.admin" => "Turning on face for admin prompts".into(),
        _ => String::new(),
    }
}

fn set_unlock(core: &Core, on: bool) {
    core.settings.set_internal("face.unlock", json!(on));
    let mut m = serde_json::Map::new();
    m.insert("face.unlock".into(), json!(on));
    core.db.save_settings(m.clone());
    core.hub.emit("settings.changed", json!({ "changes": m }));
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value, cancel: Arc<AtomicBool>) -> Outcome {
    match tool {
        "face.none" => Outcome::ok_with("Left it", LAYER, None).with_reply("Okay. Passwords will need your system password instead.".into()),
        "face.models" => {
            let ids: Vec<String> = face::MODELS.iter().map(|m| (*m).to_owned()).collect();
            if let Err(e) = core.downloads.enqueue(&ids) {
                return Outcome::fail(&e.message, LAYER);
            }
            let then = args.get("then").cloned().filter(|t| t.get("tool").is_some());
            let c = core.clone();
            tokio::spawn(async move {
                for _ in 0..1200 {
                    if installed(&c) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                if !installed(&c) {
                    crate::board::alert(&c, "install", "The face models didn't finish downloading. Ask me again later.");
                    return;
                }
                let next = then.and_then(|t| Some(Intent::Act { tool: t.get("tool")?.as_str()?.to_owned(), args: t.get("args").cloned().unwrap_or_else(|| json!({})) }));
                match next {
                    Some(intent) => {
                        if crate::control::ask(&c, |r| crate::control::Req::RunIntent { intent, reply: r }).await.is_err() {
                            crate::board::alert(&c, "install", "The face models are ready. Ask me again.");
                        }
                    }
                    None => crate::board::alert(&c, "install", "The face models are ready."),
                }
            });
            Outcome::ok_with("Downloading the face models", LAYER, None).with_reply("Downloading the face models now. I'll carry on as soon as they're ready.".into())
        }
        "face.enrol" => {
            if !installed(core) {
                return offer(Intent::Act { tool: tool.into(), args: args.clone() });
            }
            let Some(_busy) = Busy::take() else { return Outcome::fail("The camera is busy with another face check.", LAYER) };
            CANCEL.store(false, Ordering::Relaxed);
            let stop = Arc::new(AtomicBool::new(false));
            let mut tick = ticker(core, "enrol", stop.clone(), cancel);
            let c = core.clone();
            core.hub.emit("face.state", json!({ "open": true, "mode": "enrol", "frame": null, "steps": {}, "message": "Look at the camera." }));
            let span = crate::layers::read(crate::layers::Layer::Vision, "face.enrol");
            let r = crate::work::background("face", move || -> Result<face::Templates, String> {
                let m = models(&c).map_err(|e| e.to_string())?;
                let mut frames = face::open_frames().map_err(|e| e.to_string())?;
                face::enrol(&m, frames.as_mut(), &stop, Duration::from_secs(60), &mut tick)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            core.hub.emit("face.state", json!({ "open": false, "mode": "enrol" }));
            let t = match span.end(r) {
                Ok(t) => t,
                Err(m) => return Outcome::fail(&m, LAYER),
            };
            if let Err(e) = crate::vault::face_save(core, &t).await {
                return Outcome::fail(&e.to_string(), LAYER);
            }
            FAILS.store(0, Ordering::Relaxed);
            sync_root(core).await;
            if crate::vault::system(core).is_none() {
                crate::vault::open_card(core, "system", crate::vault::SYSTEM, &crate::syspass::user().unwrap_or_default(), "");
                return Outcome::ok_with("Saved your face", LAYER, None).with_reply("Got it. Only the face measurements are kept, never pictures. Now type your system password in the card, so I can use it when I've checked it's you.".into());
            }
            Outcome::ok_with("Saved your face", LAYER, None).with_reply("Got it. Face unlock is ready. Only the face measurements are kept, never pictures.".into())
        }
        "face.check" => {
            if !crate::vault::face_enrolled(core) {
                return Outcome::fail("Face unlock isn't set up. Say set up face unlock.", LAYER);
            }
            if !installed(core) {
                return offer(Intent::Act { tool: tool.into(), args: args.clone() });
            }
            match check(core, "check", Duration::from_secs(8), cancel).await {
                Ok(v @ Verdict::Me(_)) => {
                    FAILS.store(0, Ordering::Relaxed);
                    Outcome::ok_with("Recognised you", LAYER, None).with_reply(v.say().into())
                }
                Ok(v) => Outcome::fail(v.say(), LAYER),
                Err(e) => Outcome::fail(&e, LAYER),
            }
        }
        "face.signin" | "face.admin" => root_switch(core, tool, args, cancel).await,
        "face.forget" => match crate::vault::face_forget(core) {
            Ok(()) if cfg!(any(target_os = "linux", windows)) && crate::privileged::installed() && crate::privileged::call("face.forget", json!({}), Duration::from_secs(30)).await.is_err() => {
                Outcome::ok_with("Forgot your face", LAYER, None).with_reply("Forgot your face here, but the sign-in screen's copy could not be removed. Say turn off face sign-in to remove it.".into())
            }
            Ok(()) => Outcome::ok_with("Forgot your face", LAYER, None).with_reply("Forgot your face. Passwords will need your system password until you set it up again.".into()),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        "face.auto" => {
            let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
            set_unlock(core, on);
            let reply = match (on, crate::vault::face_enrolled(core), cfg!(target_os = "linux")) {
                (false, _, _) => "Face unlock is off. The lock screen will want your password.",
                (true, false, _) => "Face unlock is on, but your face isn't set up yet. Say set up face unlock.",
                (true, true, true) => "Face unlock is on. When you wake the lock screen, I'll check it's you and open it.",
                (true, true, false) => "Face unlock is on for passwords. The lock screen itself isn't supported on this system yet.",
            };
            Outcome::ok_with(if on { "Face unlock on" } else { "Face unlock off" }, LAYER, None).with_reply(reply.into())
        }
        _ => Outcome::fail("I don't know that face command.", LAYER),
    }
}

#[cfg(target_os = "linux")]
async fn unlock_session() -> anyhow::Result<()> {
    let sys = zbus::Connection::system().await?;
    sys.call_method(Some("org.freedesktop.login1"), "/org/freedesktop/login1/session/auto", Some("org.freedesktop.login1.Session"), "Unlock", &()).await?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase().contains("kde")
}

#[cfg(target_os = "linux")]
fn saver(kde: bool) -> (&'static str, &'static str, &'static str) {
    if kde { ("org.freedesktop.ScreenSaver", "/ScreenSaver", "org.freedesktop.ScreenSaver") } else { ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver") }
}

#[cfg(target_os = "linux")]
async fn still_locked(c: &zbus::Connection, kde: bool) -> bool {
    let (name, path, iface) = saver(kde);
    c.call_method(Some(name), path, Some(iface), "GetActive", &()).await.ok().and_then(|m| m.body().deserialize::<bool>().ok()).unwrap_or(false)
}

#[cfg(target_os = "linux")]
async fn idle_ms(c: &zbus::Connection) -> Option<u32> {
    let (name, path, iface) = saver(true);
    c.call_method(Some(name), path, Some(iface), "GetSessionIdleTime", &()).await.ok().and_then(|m| m.body().deserialize::<u32>().ok())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const TOUCHED_MS: u32 = 1200;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn touched(before: Option<u32>, now: Option<u32>) -> bool {
    matches!((before, now), (Some(b), Some(n)) if b >= TOUCHED_MS && n < TOUCHED_MS)
}

#[cfg(target_os = "linux")]
pub async fn watch(core: Arc<Core>) {
    use futures_util::StreamExt;
    let Ok(c) = zbus::Connection::session().await else { return };
    let kde = kde();
    let Ok(rule) = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(saver(kde).2).map(|b| b.build()) else { return };
    let Ok(mut saver) = zbus::MessageStream::for_match_rule(rule, &c, Some(32)).await else { return };
    let sleep = match zbus::Connection::system().await {
        Ok(sys) => match zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.login1.Manager").and_then(|b| b.member("PrepareForSleep")).map(|b| b.build()) {
            Ok(r) => zbus::MessageStream::for_match_rule(r, &sys, Some(8)).await.ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let mut sleep = sleep.map(|s| s.boxed()).unwrap_or_else(|| futures_util::stream::pending().boxed());
    let mut locked = still_locked(&c, kde).await;
    let mut ours = false;
    let mut quiet_until = Instant::now();
    let mut last_idle: Option<u32> = None;
    loop {
        let poll = async {
            if kde && locked {
                tokio::time::sleep(Duration::from_millis(400)).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        let wake = tokio::select! {
            _ = poll => {
                let now = idle_ms(&c).await;
                let woke = touched(last_idle, now);
                last_idle = now;
                woke
            }
            m = saver.next() => {
                let Some(Ok(m)) = m else { return };
                match m.header().member().map(|n| n.as_str().to_owned()).as_deref() {
                    Some("ActiveChanged") => {
                        let on: bool = m.body().deserialize().unwrap_or(false);
                        if !on && locked && !ours {
                            FAILS.store(0, Ordering::Relaxed);
                        }
                        if !on {
                            ours = false;
                        }
                        locked = on;
                        last_idle = None;
                        false
                    }
                    Some("WakeUpScreen") => true,
                    _ => false,
                }
            }
            m = sleep.next() => {
                let Some(Ok(m)) = m else { continue };
                let going: bool = m.body().deserialize().unwrap_or(true);
                !going
            }
        };
        if !wake || !locked || Instant::now() < quiet_until {
            continue;
        }
        if !core.settings.bool("face.unlock") || !crate::vault::face_enrolled(&core) || !installed(&core) || FAILS.load(Ordering::Relaxed) >= TRIES {
            continue;
        }
        let v = check(&core, "lock", Duration::from_secs(15), Arc::new(AtomicBool::new(false))).await;
        quiet_until = Instant::now() + Duration::from_secs(3);
        match v {
            Ok(Verdict::Me(_)) => {
                FAILS.store(0, Ordering::Relaxed);
                ours = true;
                match unlock_session().await {
                    Ok(()) => log::info!("face check passed; asked the system to unlock the session"),
                    Err(e) => {
                        ours = false;
                        log::warn!("face check passed but the session could not be unlocked: {e}");
                    }
                }
            }
            Ok(v) if counts(&v) => {
                let n = FAILS.fetch_add(1, Ordering::Relaxed) + 1;
                log::info!("face check at the lock screen failed ({n} of {TRIES})");
            }
            Ok(_) => {}
            Err(e) => log::info!("face check at the lock screen didn't run: {e}"),
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn watch(_core: Arc<Core>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_phrases() {
        let tool = |t: &str| match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        };
        assert_eq!(tool("set up face unlock").unwrap().0, "face.enrol");
        assert_eq!(tool("do you recognise me").unwrap().0, "face.check");
        assert_eq!(tool("forget my face").unwrap().0, "face.forget");
        assert_eq!(tool("turn off face unlock").unwrap().1["on"], false);
        assert!(tool("set up my phone").is_none());
        assert_eq!(tool("use my face to sign in").unwrap(), ("face.signin".to_owned(), json!({ "on": true })));
        assert_eq!(tool("turn off face sign-in").unwrap(), ("face.signin".to_owned(), json!({ "on": false })));
        assert_eq!(tool("use my face to log in").unwrap().0, "face.signin");
        assert_eq!(tool("approve admin prompts with my face").unwrap(), ("face.admin".to_owned(), json!({ "on": true })));
        assert_eq!(tool("stop using my face for sudo").unwrap().1["on"], false);
        assert!(root_confirm("face.signin", &json!({ "on": true })).is_some());
        assert!(root_confirm("face.signin", &json!({ "on": false })).is_none());
    }

    #[test]
    fn kde_wakes_on_fresh_input_after_a_quiet_spell_only() {
        assert!(touched(Some(5000), Some(100)));
        assert!(!touched(None, Some(100)), "the first sample after locking is only a baseline");
        assert!(!touched(Some(300), Some(100)), "still typing the lock shortcut is not a wake");
        assert!(!touched(Some(5000), Some(6000)));
        assert!(!touched(Some(5000), None));
    }
}
