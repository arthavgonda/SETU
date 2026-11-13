use crate::core::Core;
use crate::engines::Expect;
use crate::router::{self, Intent, PluginTool};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use setu_common::engine::{FromEngine, ToEngine};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static PREFIX: Mutex<Option<String>> = Mutex::new(None);
static GEN: AtomicU64 = AtomicU64::new(1);

#[track_caller]
pub fn reset_prefix() {
    log::debug!("router prompt cache cleared by {}", std::panic::Location::caller());
    *PREFIX.lock().unwrap() = None;
}

fn limit(ram: u64) -> u64 {
    ram * 2 + 768 * 1024 * 1024
}

pub async fn ensure_asr(core: &Core) -> Result<()> {
    let slot = core.slot("asr");
    if slot.provider != "local" {
        return Ok(());
    }
    let cfg = core.store.asr_config(&slot, &core.lang())?;
    let key = serde_json::to_string(&cfg)?;
    core.engines.asr.set_mem_limit(limit(core.store.ram_for(&slot)));
    core.engines.asr.ensure(ToEngine::LoadAsr { config: cfg }, &key).await?;
    Ok(())
}

pub async fn ensure_llm(core: &Core) -> Result<String> {
    let slot = core.slot("llm");
    if slot.provider != "local" {
        return Ok("remote".into());
    }
    let (cfg, template) = core.store.llm_config(&slot)?;
    let key = serde_json::to_string(&cfg)?;
    core.engines.llm.set_mem_limit(limit(core.store.ram_for(&slot)));
    core.engines.llm.ensure(ToEngine::LoadLlm { config: cfg }, &key).await?;
    Ok(template)
}

pub fn usable_tts(core: &Core) -> crate::models::SlotConfig {
    let slot = core.slot("tts");
    if slot.provider != "local" || slot.path.is_some() || slot.models.iter().all(|m| core.store.is_installed(m)) {
        return slot;
    }
    let installed = core.store.manifest.models.iter().filter(|m| m.slot == "tts" && core.store.is_installed(&m.id));
    let pick = installed.min_by_key(|m| match m.kind.as_deref() {
        Some("kokoro") => 0,
        Some("svara") => 2,
        _ => 1,
    });
    match pick {
        Some(m) => crate::models::SlotConfig { provider: "local".into(), models: vec![m.id.clone()], voice: slot.voice.clone(), ..Default::default() },
        None => slot,
    }
}

pub async fn ensure_tts(core: &Core) -> Result<()> {
    let slot = usable_tts(core);
    if slot.provider != "local" {
        return Ok(());
    }
    let cfg = core.store.tts_config(&slot, &core.lang())?;
    let key = serde_json::to_string(&cfg)?;
    core.engines.tts.set_mem_limit(limit(core.store.ram_for(&slot)));
    core.engines.tts.ensure(ToEngine::LoadTts { config: cfg }, &key).await?;
    Ok(())
}

fn context(core: &Core) -> (Vec<String>, Vec<(String, String)>, Vec<PluginTool>) {
    let mut apps = core.apps.all();
    apps.retain(|a| !a.hidden);
    apps.sort_by(|a, b| b.opened.cmp(&a.opened).then(a.name.cmp(&b.name)));
    let names: Vec<String> = apps.iter().map(|a| a.name.clone()).collect();
    let defaults = core.defaults();
    let mut roles = Vec::new();
    for r in crate::tools::apps::ROLES {
        if let Some(a) = core.apps.role_default(r, &defaults) {
            roles.push((r.to_string(), a.name));
        }
    }
    (names, roles, core.plugins.tools())
}

static WARMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub async fn warm(core: &Core) {
    if core.slot("llm").provider != "local" || WARMING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (apps, defaults, plugins) = context(core);
    let name = crate::memory::user_name(core);
    let system = router::system_prompt(&router::Context { apps: &apps, defaults: &defaults, plugins: &plugins, name: name.as_deref() });
    if let Ok(template) = ensure_llm(core).await {
        let _ = prefix(core, crate::chat::system(&template, &system)).await;
    }
    WARMING.store(false, Ordering::SeqCst);
}

async fn prefix(core: &Core, system: String) -> Option<String> {
    let key = crate::models::hex(&Sha256::digest(system.as_bytes()))[..16].to_owned();
    let engine = &core.engines.llm;
    let _g = engine.exclusive().await;
    let cached = PREFIX.lock().unwrap().clone();
    log::debug!("router prompt key {key}, cached {cached:?}");
    if cached.as_deref() == Some(key.as_str()) {
        return Some(key);
    }
    match engine.request(&ToEngine::LlmPrefix { key: key.clone(), text: system }, Expect::Prefixed(key.clone()), Duration::from_secs(90)).await {
        Ok(FromEngine::LlmPrefixed { tokens, ms, .. }) => {
            log::info!("router prompt cached: {tokens} tokens in {ms} ms");
            *PREFIX.lock().unwrap() = Some(key.clone());
            Some(key)
        }
        Ok(_) => None,
        Err(e) => {
            log::warn!("router prompt failed: {e}");
            None
        }
    }
}

pub const STREAM_BIT: u64 = 1 << 61;
pub const ASIDE_BIT: u64 = 1 << 59;

pub struct Routed {
    pub intent: Intent,
    pub source: &'static str,
    pub notice: Option<String>,
    pub say: Option<String>,
}

fn plain(intent: Intent, source: &'static str, notice: Option<String>) -> Routed {
    Routed { intent, source, notice, say: None }
}

fn from_model(out: &str, request: &str, source: &'static str, notice: Option<String>) -> Routed {
    let intent = router::parse_model(out);
    let say = router::said(out);
    if router::grounded(&intent, request) {
        return Routed { intent, source, notice, say };
    }
    log::info!("the model's tool call didn't match what was said; answering instead");
    if matches!(intent, Intent::MuteMic { .. }) {
        let t = crate::text::normalize(request);
        if [" sound", " volume", " audio", " speaker", " music"].iter().any(|w| format!(" {t}").contains(w)) {
            return Routed { intent: Intent::Volume { action: "mute", amount: None, level: None }, source, notice, say };
        }
    }
    match say {
        Some(t) => Routed { intent: Intent::Reply { text: t }, source, notice, say: None },
        None => Routed { intent: Intent::Unknown, source, notice, say: None },
    }
}

pub async fn model_route(core: &Core, inter: u64, request: &str) -> Routed {
    let mut extra = crate::memory::route_context(core, inter, request);
    let screen = crate::os::context_line().await;
    if !screen.is_empty() {
        extra = if extra.is_empty() { screen } else { format!("{screen}\n{extra}") };
    }
    let routed = if extra.is_empty() { request.to_owned() } else { format!("{request}\n\n(Context: {extra})") };
    let text = routed.as_str();
    let (apps, defaults, plugins) = context(core);
    let name = crate::memory::user_name(core);
    let cx = router::Context { apps: &apps, defaults: &defaults, plugins: &plugins, name: name.as_deref() };
    let system = router::system_prompt(&cx);
    let grammar = router::grammar(&plugins);
    let slot = core.slot("llm");
    let mut notice = None;
    if slot.provider != "local" {
        match crate::remote::route(core, &slot, &system, text).await {
            Ok(out) => return from_model(&out, request, "api", None),
            Err(e) => {
                log::warn!("remote language model failed: {e}");
                let provider = crate::remote::provider_name(&slot);
                notice = Some(format!("Can't reach {provider}. Using the local model instead."));
                core.attention.raise(
                    &core.hub,
                    crate::attention::Issue::new("api.unreachable", "warning", format!("Can't reach {provider}. Using the local model instead.")).action("Open Models", "ui.open", json!({ "page": "settings/models" })),
                );
            }
        }
    }
    let local = core.store.default_slots(&core.tier(), &core.lang()).remove("llm").unwrap_or_default();
    let use_slot = if slot.provider == "local" { slot } else { local };
    if use_slot.models.is_empty() && use_slot.path.is_none() {
        return plain(Intent::Unknown, "rules", notice);
    }
    let (cfg, template) = match core.store.llm_config(&use_slot) {
        Ok(c) => c,
        Err(_) => return plain(Intent::Unknown, "rules", notice),
    };
    let ekey = serde_json::to_string(&cfg).unwrap_or_default();
    core.engines.llm.set_mem_limit(limit(core.store.ram_for(&use_slot)));
    if core.engines.llm.ensure(ToEngine::LoadLlm { config: cfg }, &ekey).await.is_err() {
        return plain(Intent::Unknown, "rules", notice);
    }
    let Some(key) = prefix(core, crate::chat::system(&template, &system)).await else { return plain(Intent::Unknown, "rules", notice) };
    let engine = &core.engines.llm;
    let _g = engine.exclusive().await;
    engine.touch();
    let id = inter | STREAM_BIT;
    let msg = ToEngine::LlmGenerate { id, prefix_key: key, text: router::user_turn(&template, text), grammar: Some(grammar), max_tokens: 168, temperature: 0.0, stream: true };
    match engine.request(&msg, Expect::Result(id), Duration::from_secs(30)).await {
        Ok(FromEngine::LlmResult { text, cancelled: false, tokens, ms, .. }) => {
            let r = from_model(&text, request, "model", notice);
            log::info!("routed by the model: {tokens} tokens in {ms} ms{}", if r.say.is_some() { ", with a spoken line" } else { "" });
            r
        }
        Ok(_) => plain(Intent::Cancel, "model", notice),
        Err(e) => {
            log::warn!("routing failed: {e}");
            reset_prefix();
            plain(Intent::Unknown, "rules", notice)
        }
    }
}

pub async fn speak(core: &Core, id: u64, lines: Vec<String>) -> Result<()> {
    speak_with(core, id, lines, false).await
}

pub fn streaming(core: &Core) -> bool {
    core.slot("tts").provider == "local"
}

pub async fn speak_open(core: &Core, id: u64, lines: Vec<String>) -> Result<()> {
    speak_with(core, id, lines, true).await
}

pub fn speak_more(core: &Core, id: u64, lines: Vec<String>) {
    core.engines.tts.send(&ToEngine::TtsMore { id, lines });
}

pub fn speak_end(core: &Core, id: u64) {
    core.engines.tts.send(&ToEngine::TtsEnd { id });
}

async fn speak_with(core: &Core, id: u64, lines: Vec<String>, open: bool) -> Result<()> {
    let slot = core.slot("tts");
    let s = &core.settings;
    if slot.provider != "local" {
        let text = lines.join(" ");
        return crate::remote::speak(core, &slot, id, &text).await;
    }
    ensure_tts(core).await?;
    core.engines.tts.touch();
    core.engines.tts.send(&ToEngine::TtsSpeak {
        id,
        lines,
        speaker: 0,
        rate: s.f64("tts.rate") as f32,
        volume: s.f64("tts.volume") as f32,
        device: s.str("audio.outputDevice"),
        open,
    });
    Ok(())
}

pub async fn sample(core: &Core, voice: Option<String>, rate: Option<f64>, volume: Option<f64>, line: String) -> Result<Value> {
    let current = core.slot("tts");
    if voice.as_deref().is_none_or(str::is_empty) && current.provider != "local" {
        let n = line.chars().count() as u64;
        let id = GEN.fetch_add(1, Ordering::Relaxed) | (1 << 62);
        crate::remote::speak(core, &current, id, &line).await?;
        return Ok(json!({ "durationMs": n * 65 }));
    }
    if let Some(v) = voice.filter(|v| !v.is_empty()) {
        let m = core.store.manifest.model(&v).ok_or_else(|| anyhow!("No voice called {v}"))?;
        if m.slot != "tts" {
            return Err(anyhow!("{v} is not a voice"));
        }
        if !core.store.is_installed(&v) {
            return Err(anyhow!("{} is not downloaded yet.", m.name));
        }
        let mut slot = core.slot("tts");
        if slot.models.first() != Some(&v) {
            slot.provider = "local".into();
            slot.models = vec![v];
            core.set_slot("tts", Some(slot));
        }
    }
    ensure_tts(core).await?;
    core.engines.tts.send(&ToEngine::TtsStop);
    let n = line.chars().count() as u64;
    let rate = rate.unwrap_or(core.settings.f64("tts.rate")).clamp(0.5, 2.0);
    core.engines.tts.send(&ToEngine::TtsSpeak {
        id: GEN.fetch_add(1, Ordering::Relaxed) | (1 << 62),
        lines: vec![line],
        speaker: 0,
        rate: rate as f32,
        volume: volume.unwrap_or(core.settings.f64("tts.volume")).clamp(0.0, 1.0) as f32,
        device: core.settings.str("audio.outputDevice"),
        open: false,
    });
    Ok(json!({ "durationMs": ((n * 65) as f64 / rate) as u64 }))
}

static CLIP: AtomicU64 = AtomicU64::new(1 << 59);

pub async fn transcribe(core: &Core, samples: Vec<f32>) -> Option<(String, String)> {
    let slot = core.slot("asr");
    if slot.provider != "local" {
        return crate::remote::transcribe(core, &slot, samples).await.ok().map(|t| (t.clone(), t));
    }
    ensure_asr(core).await.ok()?;
    let id = CLIP.fetch_add(1, Ordering::Relaxed);
    let e = &core.engines.asr;
    let _g = e.exclusive().await;
    e.touch();
    e.send(&ToEngine::AsrBegin { id, chars: false });
    for c in samples.chunks(1600) {
        e.pcm(c.to_vec());
    }
    match e.request(&ToEngine::AsrEnd { id }, Expect::Final(id), Duration::from_secs(20)).await {
        Ok(FromEngine::AsrFinal { text, online, .. }) => Some((crate::text::repair_tail(&text, &online), online)),
        _ => None,
    }
}

pub fn string_grammar(field: &str) -> String {
    format!("root ::= \"{{\\\"{field}\\\": \\\"\" chars \"\\\"}}\"\nchars ::= char*\nchar ::= [^\"\\\\\\x00-\\x1f] | \"\\\\\" [\"\\\\/nt]\n")
}

pub fn list_grammar(field: &str) -> String {
    format!("root ::= \"{{\\\"{field}\\\": [\" item (\", \" item){{0,11}} \"]}}\"\nitem ::= \"\\\"\" char{{1,80}} \"\\\"\"\nchar ::= [^\"\\\\\\x00-\\x1f] | \"\\\\\" [\"\\\\/nt]\n")
}

pub fn choice_grammar(ids: &[String]) -> String {
    let alts: Vec<String> = ids.iter().map(|i| format!("\"\\\"{}\\\"\"", i.replace(['"', '\\'], ""))).collect();
    format!(
        "root ::= \"{{\\\"action\\\": \" action \", \\\"reply\\\": \\\"\" chars \"\\\"}}\"\naction ::= {}\nchars ::= char*\nchar ::= [^\"\\\\\\x00-\\x1f] | \"\\\\\" [\"\\\\/nt]\n",
        alts.join(" | ")
    )
}

pub const CHAT_BIT: u64 = 1 << 60;

pub fn tidy_answer(raw: &str, cut: bool) -> String {
    let mut seen: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for line in raw.lines() {
        let clean = crate::text::plain_reply(line);
        if clean.is_empty() {
            continue;
        }
        let mut kept = String::new();
        for sentence in clean.split_inclusive(['.', '?']) {
            let key: String = sentence.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
            if key.len() > 12 && seen.contains(&key) {
                continue;
            }
            seen.push(key);
            kept.push_str(sentence);
        }
        let kept = kept.trim().to_owned();
        if !kept.is_empty() {
            lines.push(kept);
        }
    }
    let mut out = lines.join("\n");
    if cut
        && !out.ends_with(['.', '?', ':'])
        && let Some(i) = out.rfind(['.', '?'])
    {
        out.truncate(i + 1);
    }
    out
}

pub async fn chat_answer(core: &Core, inter: u64, conversation: &str, recall: &str, action: bool) -> Result<String> {
    let (apps, defaults, plugins) = context(core);
    let name = crate::memory::user_name(core);
    let cx = router::Context { apps: &apps, defaults: &defaults, plugins: &plugins, name: name.as_deref() };
    let system = router::system_prompt(&cx);
    let limits = if action {
        "The last message asks for something Setu can't do on this computer yet. Never claim you did it. Say so in one short sentence, then offer one next step the user can ask Setu for on this computer, such as a web search for it. Never suggest using a phone or another device, and never bring up apps or websites the user did not mention."
    } else {
        "Answer it directly. Use what you know about the user when it helps."
    };
    let user = format!(
        "This is a typed chat with Setu, not a command. Reply to the last user message as a friendly, knowledgeable assistant. Write plain text without markdown or emoji. Use at most five sentences, or short numbered steps when steps help. Never repeat a sentence. {limits}\n\n{recall}{conversation}"
    );
    let max = 260;
    let slot = core.slot("llm");
    if slot.provider != "local" {
        let v = ask_json(core, &system, &user, Some(string_grammar("answer")), max, 0.3).await?;
        let text = v.get("answer").or_else(|| v.get("text")).and_then(Value::as_str).unwrap_or_default();
        return Ok(tidy_answer(text, false));
    }
    let template = ensure_llm(core).await?;
    let key = prefix(core, crate::chat::system(&template, &system)).await.ok_or_else(|| anyhow!("the language model is not ready"))?;
    let engine = &core.engines.llm;
    let _g = engine.exclusive().await;
    engine.touch();
    let id = inter | CHAT_BIT;
    let msg = ToEngine::LlmGenerate { id, prefix_key: key, text: crate::chat::user(&template, &user), grammar: Some(string_grammar("answer")), max_tokens: max, temperature: 0.3, stream: true };
    match engine.request(&msg, Expect::Result(id), Duration::from_secs(90)).await? {
        FromEngine::LlmResult { text, cancelled: false, tokens, .. } => {
            let t = crate::remote::json_object(&text);
            let v: Value = serde_json::from_str(&t).unwrap_or_else(|_| json!({ "answer": partial_answer(&text) }));
            let a = v.get("answer").and_then(Value::as_str).unwrap_or_default();
            Ok(tidy_answer(a, tokens >= max))
        }
        _ => Err(anyhow!("cancelled")),
    }
}

pub async fn small_talk(core: &Core, inter: u64, said: &str, doing: &str) -> Result<String> {
    let (apps, defaults, plugins) = context(core);
    let name = crate::memory::user_name(core);
    let cx = router::Context { apps: &apps, defaults: &defaults, plugins: &plugins, name: name.as_deref() };
    let system = router::system_prompt(&cx);
    let who = name.as_deref().unwrap_or("friend");
    let heard: String = said.chars().filter(|c| !c.is_control()).take(400).collect();
    let user = format!(
        "The user said: \"{heard}\"\nSetu is already doing what they asked: {doing}. Write only what Setu says out loud now: one or two short, warm, plain sentences that answer what they told you or asked about you, address them as {who}, and end by saying what you are doing. No emoji, no questions back."
    );
    let max = 64;
    let slot = core.slot("llm");
    if slot.provider != "local" {
        let v = ask_json(core, &system, &user, Some(string_grammar("say")), max, 0.4).await?;
        return Ok(tidy_answer(v.get("say").and_then(Value::as_str).unwrap_or_default(), false));
    }
    let template = ensure_llm(core).await?;
    let key = prefix(core, crate::chat::system(&template, &system)).await.ok_or_else(|| anyhow!("the language model is not ready"))?;
    let engine = &core.engines.llm;
    let _g = engine.exclusive().await;
    engine.touch();
    let id = inter | CHAT_BIT | ASIDE_BIT;
    let msg = ToEngine::LlmGenerate { id, prefix_key: key, text: crate::chat::user(&template, &user), grammar: Some(string_grammar("say")), max_tokens: max, temperature: 0.4, stream: false };
    match engine.request(&msg, Expect::Result(id), Duration::from_secs(20)).await? {
        FromEngine::LlmResult { text, cancelled: false, .. } => {
            let v: Value = serde_json::from_str(&crate::remote::json_object(&text)).unwrap_or_else(|_| json!({ "say": partial_answer(&text) }));
            let a = tidy_answer(v.get("say").and_then(Value::as_str).unwrap_or_default(), false);
            if a.split_whitespace().count() < 2 { Err(anyhow!("no answer")) } else { Ok(a) }
        }
        _ => Err(anyhow!("cancelled")),
    }
}

pub fn partial_answer(raw: &str) -> String {
    let Some(start) = raw.find("\": \"").map(|i| i + 4).or_else(|| raw.find("\":\"").map(|i| i + 3)) else { return String::new() };
    let mut out = String::new();
    let mut esc = false;
    for c in raw[start..].chars() {
        if esc {
            out.push(match c {
                'n' => '\n',
                't' => ' ',
                other => other,
            });
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '"' {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

pub async fn ask_json(core: &Core, system: &str, user: &str, grammar: Option<String>, max_tokens: u32, temperature: f32) -> Result<Value> {
    let slot = core.slot("llm");
    if slot.provider != "local" {
        let out = crate::remote::route(core, &slot, system, user).await?;
        return serde_json::from_str(&out).map_err(|e| anyhow!("the answer was not JSON: {e}"));
    }
    let template = ensure_llm(core).await?;
    let key = prefix(core, crate::chat::system(&template, system)).await.ok_or_else(|| anyhow!("the language model is not ready"))?;
    let engine = &core.engines.llm;
    let _g = engine.exclusive().await;
    engine.touch();
    let id = GEN.fetch_add(1, Ordering::Relaxed);
    let msg = ToEngine::LlmGenerate { id, prefix_key: key, text: crate::chat::user(&template, user), grammar, max_tokens, temperature, stream: false };
    match engine.request(&msg, Expect::Result(id), Duration::from_secs(60)).await? {
        FromEngine::LlmResult { text, cancelled: false, .. } => {
            let t = crate::remote::json_object(&text);
            serde_json::from_str(&t).map_err(|e| anyhow!("the answer was not JSON: {e}"))
        }
        _ => Err(anyhow!("cancelled")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_answers_are_tidied_and_streamed() {
        let raw = "Light bends in raindrops. The colours spread out. Light bends in raindrops. The colours spread out. Red is on the outside and vi";
        assert_eq!(tidy_answer(raw, true), "Light bends in raindrops. The colours spread out. Red is on the outside and vi".rsplit_once(". ").map(|(a, _)| format!("{a}.")).unwrap());
        assert_eq!(tidy_answer("Step one.\nStep two.", false), "Step one.\nStep two.");
        assert_eq!(partial_answer(r#"{"answer": "Hello, \"friend\".\nNext"#), "Hello, \"friend\".\nNext");
        assert_eq!(partial_answer("{\"answ"), "");
    }
}
