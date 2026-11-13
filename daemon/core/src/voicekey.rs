use crate::core::Core;
use crate::router::Intent;
use crate::tools::Outcome;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const LAYER: &str = "vault";

pub fn rules(t: &str) -> Option<Intent> {
    let act = |tool: &str, args: Value| Some(Intent::Act { tool: tool.into(), args });
    if matches!(t, "use my face and voice for passwords" | "use my face and my voice for passwords" | "use face and voice together" | "use my voice as a second key" | "turn on the voice key" | "require my voice for passwords" | "ask for my voice too" | "use face and voice for passwords") {
        return act("voice.key", json!({ "on": true }));
    }
    if matches!(t, "face only for passwords" | "use only my face for passwords" | "use my face only for passwords" | "turn off the voice key" | "stop asking for my voice" | "do not ask for my voice" | "don't ask for my voice") {
        return act("voice.key", json!({ "on": false }));
    }
    for (style, phrases) in [
        ("quiet", &["learn my quiet voice", "learn how i speak quietly", "learn my soft voice", "learn my voice when i speak quietly"][..]),
        ("loud", &["learn my loud voice", "learn how i speak loudly", "learn my voice when i speak loudly"][..]),
        ("fast", &["learn my fast voice", "learn how i speak quickly", "learn how i speak fast", "learn my voice when i speak quickly"][..]),
        ("short", &["learn my short commands", "learn how i say short commands", "learn my voice for short commands"][..]),
    ] {
        if phrases.contains(&t) {
            return act("voice.learn", json!({ "style": style }));
        }
    }
    if matches!(t, "what do you know about my voice" | "which voices of mine do you know" | "voice key status") {
        return act("voice.status", json!({}));
    }
    None
}

pub fn label(tool: &str, args: &Value) -> String {
    match tool {
        "voice.key" if args.get("on").and_then(Value::as_bool) == Some(false) => "Turning off the voice key".into(),
        "voice.key" => "Turning on the voice key".into(),
        "voice.learn" => "Learning more of your voice".into(),
        "voice.status" => "Checking your voice prints".into(),
        _ => String::new(),
    }
}

pub fn prompt_for(style: &str) -> &'static str {
    match style {
        "quiet" => "Say a whole sentence quietly, now.",
        "loud" => "Say a whole sentence a little louder than usual, now.",
        "fast" => "Say a whole sentence quickly, now.",
        _ => "Say a short command the way you usually would, such as Hey Setu, volume up.",
    }
}

pub fn style_word(style: &str) -> &'static str {
    match style {
        "quiet" => "quiet voice",
        "loud" => "loud voice",
        "fast" => "fast speech",
        _ => "short commands",
    }
}

fn set(core: &Core, key: &str, on: bool) {
    core.settings.set_internal(key, json!(on));
    let mut m = serde_json::Map::new();
    m.insert(key.into(), json!(on));
    core.db.save_settings(m.clone());
    core.hub.emit("settings.changed", json!({ "changes": m }));
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value, cancel: Arc<AtomicBool>) -> Outcome {
    match tool {
        "voice.key" => {
            let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
            if on {
                if !crate::speaker::usable(core) {
                    let mut o = Outcome::fail("Your voice isn't recorded yet. Record it in Settings, Voice, then ask me again.", LAYER);
                    o.fix = Some(json!({ "label": "Open Voice settings", "method": "ui.open", "params": { "page": "settings/voice" } }));
                    return o;
                }
                set(core, "vault.voiceKey", true);
                return Outcome::ok_with("Voice key on", LAYER, None).with_reply("From now on, showing or typing a password needs your face and your voice together. Ask for it aloud, in your normal voice.".into());
            }
            if !core.settings.bool("vault.voiceKey") {
                return Outcome::ok_with("Voice key off", LAYER, None).with_reply("Passwords already need only your face.".into());
            }
            if let Err(e) = crate::faceid::verify(core, "use your face only", cancel).await {
                return Outcome::fail(&e, LAYER);
            }
            set(core, "vault.voiceKey", false);
            Outcome::ok_with("Voice key off", LAYER, None).with_reply("Passwords now need only your face. On an ordinary webcam a good video of you can pass a face check; your voice was the guard against that.".into())
        }
        "voice.learn" => {
            let style = args.get("style").and_then(Value::as_str).unwrap_or("short");
            if !crate::speaker::STYLES.contains(&style) {
                return Outcome::fail("That isn't a way of speaking I can learn.", LAYER);
            }
            if !crate::speaker::usable(core) {
                let mut o = Outcome::fail("Your voice isn't recorded yet. Record it in Settings, Voice, first.", LAYER);
                o.fix = Some(json!({ "label": "Open Voice settings", "method": "ui.open", "params": { "page": "settings/voice" } }));
                return o;
            }
            Outcome::ok_with("Listening for your voice", LAYER, None).with_reply(prompt_for(style).into())
        }
        "voice.status" => {
            if !crate::speaker::usable(core) {
                return Outcome::ok_with("No voice recorded", LAYER, None).with_reply("Your voice isn't recorded yet.".into());
            }
            let p = crate::speaker::load_prints(&core.db);
            let styles = crate::speaker::styles(&core.db);
            let extra = if styles.is_empty() { String::new() } else { format!(" I also know your {}.", crate::skills::join_names_pub(&styles.iter().map(|s| style_word(s).to_owned()).collect::<Vec<_>>())) };
            let learned = match p.learned.len() {
                0 => String::new(),
                1 => " I've added 1 print from hearing you since.".into(),
                n => format!(" I've added {n} prints from hearing you since."),
            };
            let key = if core.settings.bool("vault.voiceKey") { " Passwords need your face and your voice." } else { " Passwords need only your face." };
            Outcome::ok_with("Told you about your voice prints", LAYER, None).with_reply(format!("I know your voice from the recordings you made.{extra}{learned}{key}"))
        }
        _ => Outcome::fail("I don't know that voice command.", LAYER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_key_phrases() {
        let tool = |t: &str| match rules(t) {
            Some(Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        };
        assert_eq!(tool("use my face and voice for passwords").unwrap(), ("voice.key".to_owned(), json!({ "on": true })));
        assert_eq!(tool("face only for passwords").unwrap(), ("voice.key".to_owned(), json!({ "on": false })));
        assert_eq!(tool("learn my quiet voice").unwrap(), ("voice.learn".to_owned(), json!({ "style": "quiet" })));
        assert_eq!(tool("learn how i say short commands").unwrap().1["style"], "short");
        assert!(tool("learn my lines").is_none());
        for s in crate::speaker::STYLES {
            assert!(!prompt_for(s).is_empty() && !style_word(s).is_empty());
        }
    }
}
