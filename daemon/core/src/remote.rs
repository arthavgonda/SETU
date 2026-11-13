use crate::core::Core;
use crate::models::SlotConfig;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const LLM_PROVIDERS: &[&str] = &["openai", "anthropic", "gemini", "ollama", "lmstudio"];
pub const TTS_PROVIDERS: &[&str] = &["openai", "elevenlabs"];
pub const ASR_PROVIDERS: &[&str] = &["openai"];
pub const OPENAI_VOICES: &[&str] = &["alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer", "verse"];
const ANTHROPIC_VERSION: &str = "2026-10-21";
const ANTHROPIC_FALLBACK_MODELS: &[&str] = &["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"];

pub fn providers(slot: &str) -> &'static [&'static str] {
    match slot {
        "llm" => LLM_PROVIDERS,
        "tts" => TTS_PROVIDERS,
        _ => ASR_PROVIDERS,
    }
}

fn default_base(provider: &str) -> Option<&'static str> {
    match provider {
        "openai" => Some("https://api.openai.com/v1"),
        "anthropic" => Some("https://api.anthropic.com/v1"),
        "gemini" => Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        "elevenlabs" => Some("https://api.elevenlabs.io/v1"),
        "ollama" => Some("http://127.0.0.1:11434/v1"),
        "lmstudio" => Some("http://127.0.0.1:1234/v1"),
        _ => None,
    }
}

fn host(slot: &SlotConfig) -> Option<String> {
    slot.base_url.as_deref().filter(|u| !u.is_empty()).and_then(|u| u.split("://").nth(1)).and_then(|h| h.split('/').next()).map(str::to_owned)
}

fn official(slot: &SlotConfig) -> bool {
    match (host(slot), default_base(&slot.provider)) {
        (None, _) => true,
        (Some(h), Some(d)) => d.contains(&format!("://{h}/")),
        _ => false,
    }
}

pub fn provider_name(slot: &SlotConfig) -> String {
    match slot.provider.as_str() {
        "ollama" => "Ollama".into(),
        "lmstudio" => "LM Studio".into(),
        "anthropic" if official(slot) => "Anthropic".into(),
        "gemini" if official(slot) => "Google Gemini".into(),
        "elevenlabs" if official(slot) => "ElevenLabs".into(),
        "openai" if official(slot) => "OpenAI".into(),
        _ => host(slot).unwrap_or_else(|| "the API".into()),
    }
}

pub fn base(slot: &SlotConfig) -> Result<String> {
    let b = slot.base_url.clone().filter(|b| !b.is_empty()).or_else(|| default_base(&slot.provider).map(str::to_owned));
    let b = b.ok_or_else(|| anyhow!("Enter the base URL of the API."))?;
    if !(b.starts_with("https://") || b.starts_with("http://127.0.0.1") || b.starts_with("http://localhost") || b.starts_with("http://[::1]")) {
        return Err(anyhow!("Use an https address, or http only for this computer."));
    }
    Ok(b.trim_end_matches('/').to_owned())
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(timeout)))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn authorize<B>(req: ureq::RequestBuilder<B>, provider: &str, key: &Option<String>) -> ureq::RequestBuilder<B> {
    let Some(k) = key else { return req };
    match provider {
        "anthropic" => req.header("x-api-key", k).header("anthropic-version", ANTHROPIC_VERSION),
        "elevenlabs" => req.header("xi-api-key", k),
        _ => req.header("Authorization", &format!("Bearer {k}")),
    }
}

async fn key(slot: &SlotConfig) -> Option<String> {
    match slot.key_ref.as_deref() {
        Some(r) if !r.is_empty() => crate::secrets::read(r).await.ok(),
        _ => None,
    }
}

fn status_error(slot: &SlotConfig, code: u16) -> anyhow::Error {
    let p = provider_name(slot);
    match code {
        401 | 403 => anyhow!("{p} rejected the key."),
        404 => anyhow!("{p} doesn't know that model."),
        429 => anyhow!("{p} is busy or over its usage limit."),
        _ => anyhow!("{p} answered with an error ({code})."),
    }
}

fn effort_supported(model: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-fable-5", "claude-mythos-5", "claude-opus-4-6", "claude-opus-4-7", "claude-opus-4-8", "claude-sonnet-4-6"].iter().any(|p| model.starts_with(p))
}

pub fn anthropic_body(model: &str, system: &str, user: &str, fallback: bool) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 2048,
        "system": system,
        "messages": [ { "role": "user", "content": user } ]
    });
    if effort_supported(model) {
        body["output_config"] = json!({ "effort": "low" });
    }
    if fallback {
        body["fallbacks"] = json!("default");
    }
    body
}

pub fn anthropic_text(v: &Value) -> Result<String> {
    if v["stop_reason"] == "refusal" {
        return Err(anyhow!("Anthropic declined to answer that."));
    }
    let text: String = v["content"].as_array().map(|a| a.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect()).unwrap_or_default();
    if text.trim().is_empty() {
        return Err(anyhow!("The API returned no answer."));
    }
    Ok(text)
}

pub fn json_object(text: &str) -> String {
    match (text.find('{'), text.rfind('}')) {
        (Some(a), Some(b)) if b > a => text[a..=b].to_owned(),
        _ => text.trim().to_owned(),
    }
}

pub async fn route(_core: &Core, slot: &SlotConfig, system: &str, text: &str) -> Result<String> {
    let b = base(slot)?;
    let k = key(slot).await;
    let model = slot.model_name.clone().unwrap_or_default();
    let sys = system.replace("<|im_start|>system\n", "").replace("<|im_end|>", "").trim().to_owned() + "\nAnswer with the JSON object only.";
    let user = text.to_owned();
    let s = slot.clone();
    crate::layers::blocking(move || {
        if s.provider == "anthropic" {
            let fallback = official(&s) && ANTHROPIC_FALLBACK_MODELS.contains(&model.as_str());
            let mut req = authorize(agent(30).post(&format!("{b}/messages")).header("Content-Type", "application/json"), "anthropic", &k);
            if fallback {
                req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            let mut resp = req.send(anthropic_body(&model, &sys, &user, fallback).to_string()).map_err(|e| anyhow!("{e}"))?;
            let code = resp.status().as_u16();
            if code != 200 {
                return Err(status_error(&s, code));
            }
            let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
            return anthropic_text(&v).map(|t| json_object(&t));
        }
        let messages = json!([ { "role": "system", "content": sys }, { "role": "user", "content": user } ]);
        let full = json!({ "model": model, "temperature": 0, "max_tokens": 512, "response_format": { "type": "json_object" }, "messages": messages });
        let plain = json!({ "model": model, "max_completion_tokens": 2048, "messages": messages });
        for (n, body) in [full, plain].iter().enumerate() {
            let req = authorize(agent(30).post(&format!("{b}/chat/completions")).header("Content-Type", "application/json"), &s.provider, &k);
            let mut resp = req.send(body.to_string()).map_err(|e| anyhow!("{e}"))?;
            let code = resp.status().as_u16();
            if code == 400 && n == 0 {
                continue;
            }
            if code != 200 {
                return Err(status_error(&s, code));
            }
            let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
            return v["choices"][0]["message"]["content"].as_str().map(json_object).ok_or_else(|| anyhow!("The API returned no answer."));
        }
        Err(status_error(&s, 400))
    })
    .await?
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

pub async fn transcribe(_core: &Core, slot: &SlotConfig, audio: Vec<f32>) -> Result<String> {
    let b = base(slot)?;
    let k = key(slot).await;
    let model = slot.model_name.clone().unwrap_or_else(|| "whisper-1".into());
    let s = slot.clone();
    crate::layers::blocking(move || {
        let boundary = format!("setu{}", fastrand::u64(..));
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n").as_bytes());
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
        body.extend_from_slice(&wav(&audio));
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = authorize(agent(30).post(&format!("{b}/audio/transcriptions")).header("Content-Type", &format!("multipart/form-data; boundary={boundary}")), &s.provider, &k);
        let mut resp = req.send(&body[..]).map_err(|e| anyhow!("{e}"))?;
        let code = resp.status().as_u16();
        if code != 200 {
            return Err(status_error(&s, code));
        }
        let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?)?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    })
    .await?
}

pub fn speech_request(slot: &SlotConfig, base: &str, text: &str, speed: f64) -> Result<(String, Value)> {
    if slot.provider == "elevenlabs" {
        let voice = slot.voice.clone().filter(|v| !v.is_empty()).ok_or_else(|| anyhow!("Choose an ElevenLabs voice."))?;
        if !voice.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(anyhow!("That voice ID isn't valid."));
        }
        let model = slot.model_name.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| "eleven_flash_v2_5".into());
        return Ok((
            format!("{base}/text-to-speech/{voice}?output_format=pcm_24000"),
            json!({ "text": text, "model_id": model, "voice_settings": { "speed": speed.clamp(0.7, 1.2) } }),
        ));
    }
    let model = slot.model_name.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| "gpt-4o-mini-tts".into());
    let voice = slot.voice.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| "alloy".into());
    Ok((format!("{base}/audio/speech"), json!({ "model": model, "input": text, "voice": voice, "response_format": "pcm", "speed": speed.clamp(0.25, 4.0) })))
}

pub async fn speak(core: &Core, slot: &SlotConfig, id: u64, text: &str) -> Result<()> {
    let b = base(slot)?;
    let k = key(slot).await;
    let device = core.settings.str("audio.outputDevice");
    let volume = core.settings.f64("tts.volume") as f32;
    let (url, body) = speech_request(slot, &b, text, core.settings.f64("tts.rate"))?;
    let tx = core.control.clone();
    let s = slot.clone();
    crate::layers::blocking(move || -> Result<()> {
        let t0 = Instant::now();
        let req = authorize(agent(60).post(&url).header("Content-Type", "application/json"), &s.provider, &k);
        let mut resp = req.send(body.to_string()).map_err(|e| anyhow!("{e}"))?;
        let code = resp.status().as_u16();
        if code != 200 {
            return Err(status_error(&s, code));
        }
        let bytes = resp.body_mut().with_config().limit(50 * 1024 * 1024).read_to_vec()?;
        let pcm: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0 * volume).collect();
        let mut p = setu_audio::Playback::open(&device, 24_000, "Assistant voice").map_err(|e| anyhow!(e))?;
        let _ = tx.blocking_send(crate::control::ControlMsg::Engine(crate::engines::EngineEvent::TtsLine { id, index: 0 }));
        let first = t0.elapsed().as_millis() as u64;
        let levels = tx.clone();
        let mut meter = setu_common::meter::Meter::new(24_000, move |level| {
            let _ = levels.try_send(crate::control::ControlMsg::Engine(crate::engines::EngineEvent::TtsLevel { level }));
        });
        let scale = if volume > 0.0 { 1.0 / volume } else { 0.0 };
        for part in pcm.chunks(960) {
            let raw: Vec<f32> = part.iter().map(|s| s * scale).collect();
            meter.push(&raw);
            p.write(part).map_err(|e| anyhow!(e))?;
        }
        meter.finish();
        p.write(&[0.0; 3600]).map_err(|e| anyhow!(e))?;
        p.drain();
        let _ = tx.blocking_send(crate::control::ControlMsg::Engine(crate::engines::EngineEvent::TtsDone { id, first_audio_ms: first }));
        Ok(())
    })
    .await?
}

fn failure(message: impl Into<String>, ms: u64) -> Value {
    json!({ "ok": false, "latencyMs": ms, "message": message.into(), "models": [], "voices": [] })
}

pub fn model_ids(provider: &str, v: &Value) -> Vec<String> {
    let mut ids: Vec<String> = match provider {
        "elevenlabs" => v.as_array().map(|a| a.iter().filter(|m| m["can_do_text_to_speech"] != false).filter_map(|m| m["model_id"].as_str().map(str::to_owned)).collect()).unwrap_or_default(),
        _ => v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(|s| s.trim_start_matches("models/").to_owned())).collect()).unwrap_or_default(),
    };
    ids.truncate(200);
    ids
}

pub async fn test(kind: &str, slot: &SlotConfig) -> Value {
    let b = match base(slot) {
        Ok(b) => b,
        Err(e) => return failure(e.to_string(), 0),
    };
    let k = key(slot).await;
    if slot.key_ref.as_deref().is_some_and(|r| !r.is_empty()) && k.is_none() {
        return failure("The key could not be read from the keychain. Unlock the keychain and try again.", 0);
    }
    if k.is_none() && ["openai", "anthropic", "gemini", "elevenlabs"].contains(&slot.provider.as_str()) && official(slot) {
        return failure(format!("Enter your {} API key.", provider_name(slot)), 0);
    }
    let s = slot.clone();
    let kind = kind.to_owned();
    crate::layers::blocking(move || {
        let t0 = Instant::now();
        let get = |path: &str| -> Result<(u16, Value), String> {
            let mut url = format!("{b}/{path}");
            if s.provider == "anthropic" {
                url.push_str("?limit=100");
            }
            let mut r = authorize(agent(15).get(&url), &s.provider, &k).call().map_err(|e| e.to_string())?;
            let code = r.status().as_u16();
            let v = r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_string().ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
            Ok((code, v))
        };
        let (code, v) = match get("models") {
            Ok(x) => x,
            Err(e) => return failure(format!("Can't reach {}. {e}", provider_name(&s)), t0.elapsed().as_millis() as u64),
        };
        if code != 200 {
            return failure(status_error(&s, code).to_string(), t0.elapsed().as_millis() as u64);
        }
        let models = model_ids(&s.provider, &v);
        let voices: Vec<Value> = if kind != "tts" {
            Vec::new()
        } else if s.provider == "elevenlabs" {
            match get("voices") {
                Ok((200, v)) => v["voices"].as_array().map(|a| a.iter().filter_map(|x| Some(json!({ "id": x["voice_id"].as_str()?, "name": x["name"].as_str().unwrap_or("") }))).take(200).collect()).unwrap_or_default(),
                _ => Vec::new(),
            }
        } else {
            OPENAI_VOICES.iter().map(|v| json!({ "id": v, "name": v[..1].to_uppercase() + &v[1..] })).collect()
        };
        let ms = t0.elapsed().as_millis() as u64;
        let want = s.model_name.clone().unwrap_or_default();
        let known = want.is_empty() || models.is_empty() || models.contains(&want);
        let msg = if known {
            format!("Connected to {} in {ms} ms. {} models are available.", provider_name(&s), models.len())
        } else {
            format!("Connected to {}, but {want} isn't in its list of models.", provider_name(&s))
        };
        json!({ "ok": known, "latencyMs": ms, "message": msg, "models": models, "voices": voices })
    })
    .await
    .unwrap_or_else(|_| failure("The test could not run.", 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(w.len(), 44 + 6);
    }

    #[test]
    fn base_urls_are_restricted() {
        let mut s = SlotConfig { provider: "custom".into(), ..Default::default() };
        assert!(base(&s).is_err());
        s.base_url = Some("http://example.org/v1".into());
        assert!(base(&s).is_err());
        s.base_url = Some("https://api.example.org/v1/".into());
        assert_eq!(base(&s).unwrap(), "https://api.example.org/v1");
        s.provider = "ollama".into();
        s.base_url = None;
        assert_eq!(base(&s).unwrap(), "http://127.0.0.1:11434/v1");
        s.provider = "anthropic".into();
        assert_eq!(base(&s).unwrap(), "https://api.anthropic.com/v1");
        s.provider = "gemini".into();
        assert_eq!(base(&s).unwrap(), "https://generativelanguage.googleapis.com/v1beta/openai");
    }

    #[test]
    fn names_follow_the_endpoint() {
        let mut s = SlotConfig { provider: "openai".into(), ..Default::default() };
        assert_eq!(provider_name(&s), "OpenAI");
        s.base_url = Some("https://openrouter.ai/api/v1".into());
        assert_eq!(provider_name(&s), "openrouter.ai");
        s.provider = "anthropic".into();
        s.base_url = Some("https://api.anthropic.com/v1".into());
        assert_eq!(provider_name(&s), "Anthropic");
        s.provider = "elevenlabs".into();
        s.base_url = None;
        assert_eq!(provider_name(&s), "ElevenLabs");
    }

    #[test]
    fn anthropic_requests_follow_the_model() {
        let b = anthropic_body("claude-opus-5-5", "sys", "open firefox", true);
        assert_eq!(b["output_config"]["effort"], "low");
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("temperature").is_none());
        assert_eq!(b["messages"][0]["content"], "open firefox");
        let old = anthropic_body("claude-haiku-4-5", "sys", "hi", false);
        assert!(old.get("output_config").is_none());
        assert!(old.get("fallbacks").is_none());
        let reply = json!({ "stop_reason": "end_turn", "content": [ { "type": "thinking", "thinking": "" }, { "type": "text", "text": "Sure: {\"intent\":\"open_app\"}" } ] });
        assert_eq!(json_object(&anthropic_text(&reply).unwrap()), "{\"intent\":\"open_app\"}");
        assert!(anthropic_text(&json!({ "stop_reason": "refusal", "content": [] })).is_err());
    }

    #[test]
    fn speech_requests_per_provider() {
        let mut s = SlotConfig { provider: "elevenlabs".into(), ..Default::default() };
        assert!(speech_request(&s, "https://api.elevenlabs.io/v1", "hi", 1.0).is_err());
        s.voice = Some("21m00Tcm4TlvDq8ikWAM".into());
        let (url, body) = speech_request(&s, "https://api.elevenlabs.io/v1", "hi", 1.0).unwrap();
        assert_eq!(url, "https://api.elevenlabs.io/v1/text-to-speech/21m00Tcm4TlvDq8ikWAM?output_format=pcm_24000");
        assert_eq!(body["model_id"], "eleven_flash_v2_5");
        s.voice = Some("../x".into());
        assert!(speech_request(&s, "https://api.elevenlabs.io/v1", "hi", 1.0).is_err());
        let o = SlotConfig { provider: "openai".into(), voice: Some("nova".into()), ..Default::default() };
        let (url, body) = speech_request(&o, "https://api.openai.com/v1", "hi", 1.0).unwrap();
        assert_eq!(url, "https://api.openai.com/v1/audio/speech");
        assert_eq!(body["voice"], "nova");
        assert_eq!(body["response_format"], "pcm");
    }

    #[test]
    fn model_lists_are_read_per_provider() {
        let g = json!({ "data": [ { "id": "models/gemini-x-flash" }, { "id": "models/gemini-x-pro" } ] });
        assert_eq!(model_ids("gemini", &g), vec!["gemini-x-flash", "gemini-x-pro"]);
        let e = json!([ { "model_id": "eleven_flash_v2_5", "can_do_text_to_speech": true }, { "model_id": "scribe", "can_do_text_to_speech": false } ]);
        assert_eq!(model_ids("elevenlabs", &e), vec!["eleven_flash_v2_5"]);
    }
}
