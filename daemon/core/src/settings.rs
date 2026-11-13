use serde_json::{Map, Value, json};
use setu_common::protocol::ApiError;
use std::sync::RwLock;

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Num(f64, f64),
    Int(i64, i64),
    Choice(&'static [&'static str]),
    IntChoice(&'static [i64]),
    Name,
    Word,
    Text(usize),
    Paths,
    Object,
}

struct Def {
    key: &'static str,
    kind: Kind,
    default: fn() -> Value,
}

macro_rules! d {
    ($k:expr, $kind:expr, $v:expr) => {
        Def { key: $k, kind: $kind, default: || json!($v) }
    };
}

fn os_language() -> Value {
    let raw = std::env::var("LC_ALL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|v| !v.is_empty()))
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_default();
    let tag = raw.split(['.', '@']).next().unwrap_or("").replace('_', "-");
    let base = tag.split('-').next().unwrap_or("").to_lowercase();
    if LANGUAGES.contains(&base.as_str()) { json!(base) } else { json!("en") }
}

pub const NAME: &str = "Setu";

pub const LANGUAGES: &[&str] = &["en", "hi", "es", "fr", "de", "pt", "ar", "ja", "zh"];

static DEFS: &[Def] = &[
    d!("assistant.name", Kind::Name, "Setu"),
    d!("wake.useHey", Kind::Bool, true),
    d!("wake.sensitivity", Kind::Num(0.0, 1.0), 0.5),
    d!("wake.longWindow", Kind::Bool, false),
    d!("audio.inputDevice", Kind::Text(256), ""),
    d!("audio.outputDevice", Kind::Text(256), ""),
    d!("audio.gain", Kind::Num(0.5, 4.0), 1.0),
    d!("audio.noiseReduction", Kind::Bool, true),
    d!("audio.quietVoice", Kind::Bool, false),
    d!("audio.listeningSound", Kind::Bool, true),
    d!("audio.sounds", Kind::Bool, true),
    d!("audio.autoGain", Kind::Bool, true),
    d!("audio.wideband", Kind::Bool, true),
    d!("audio.echoCancel", Kind::Bool, true),
    d!("audio.duck", Kind::Bool, true),
    d!("media.lyrics", Kind::Bool, true),
    d!("voice.stopWord", Kind::Word, "stop"),
    d!("tts.voice", Kind::Text(64), ""),
    d!("tts.rate", Kind::Num(0.5, 2.0), 1.1),
    d!("tts.volume", Kind::Num(0.0, 1.0), 0.8),
    d!("tts.speakReplies", Kind::Bool, true),
    d!("tts.readConfirmations", Kind::Bool, true),
    d!("session.activeMinutes", Kind::Int(1, 60), 10),
    d!("session.graceSeconds", Kind::Num(1.0, 30.0), 5.0),
    d!("session.endSilence", Kind::Num(0.4, 4.0), 0.7),
    d!("speaker.verification", Kind::Bool, false),
    d!("face.unlock", Kind::Bool, true),
    d!("vault.voiceKey", Kind::Bool, false),
    d!("confirm.method", Kind::Choice(&["voice", "key", "switch", "any"]), "any"),
    d!("startup.atLogin", Kind::Bool, true),
    d!("privacy.learn", Kind::Bool, true),
    d!("privacy.crashReports", Kind::Bool, false),
    d!("privacy.screenReading", Kind::Bool, false),
    d!("history.keep", Kind::Bool, true),
    d!("history.retention", Kind::Choice(&["30d", "1y", "forever"]), "1y"),
    Def { key: "ui.language", kind: Kind::Choice(LANGUAGES), default: os_language },
    d!("ui.textScale", Kind::Num(1.0, 2.0), 1.0),
    d!("ui.theme", Kind::Choice(&["system", "light", "dark", "contrast"]), "system"),
    d!("ui.readingFont", Kind::Choice(&["default", "atkinson"]), "default"),
    d!("ui.reduceMotion", Kind::Bool, false),
    d!("ui.largeCaptions", Kind::Bool, false),
    d!("ui.timeoutScale", Kind::Choice(&["1", "2", "3", "none"]), "1"),
    d!("ui.announceDetail", Kind::Choice(&["brief", "normal", "verbose"]), "normal"),
    d!("ui.switchScanning", Kind::Bool, false),
    d!("ui.compactIndicator", Kind::Bool, true),
    d!("ui.notch", Kind::Choice(&["always", "active", "off"]), "always"),
    d!("ui.floatingIndicator", Kind::Bool, false),
    d!("ui.helperMode", Kind::Bool, false),
    d!("ui.readAloud", Kind::Bool, true),
    d!("general.switchInput", Kind::Bool, false),
    d!("models.tier", Kind::Choice(&["", "compact", "balanced", "accurate"]), ""),
    d!("models.unloadMinutes", Kind::IntChoice(&[0, 1, 5, 15, 30]), 15),
    d!("models.gpu", Kind::Bool, true),
    d!("apps.defaults", Kind::Object, {}),
    d!("permissions.files", Kind::Bool, true),
    d!("permissions.control", Kind::Bool, true),
    d!("permissions.admin", Kind::Bool, true),
    Def { key: "files.folders", kind: Kind::Paths, default: || Value::Array(Vec::new()) },
    d!("web.browser", Kind::Text(512), ""),
    d!("code.folder", Kind::Text(4096), ""),
    d!("code.ide", Kind::Text(64), ""),
    d!("code.checkOnRun", Kind::Bool, true),
    d!("code.layout", Kind::Bool, true),
    d!("plugins.developerMode", Kind::Bool, false),
    d!("update.channel", Kind::Choice(&["stable", "beta"]), "stable"),
    d!("setup.step", Kind::Int(1, 12), 1),
    d!("setup.completed", Kind::Bool, false),
];

const INTERNAL: &[&str] = &["models.slots", "wake.variants", "wake.learned", "setup.version", "setup.rerun", "migrate.skills", "migrate.retire", "migrate.speakerFit"];

pub const SETUP_VERSION: i64 = 2;

fn def(key: &str) -> Option<&'static Def> {
    DEFS.iter().find(|d| d.key == key)
}

pub fn validate(key: &str, v: &Value) -> Result<Value, ApiError> {
    let d = def(key).ok_or_else(|| ApiError::invalid(format!("{key} is not a setting")))?;
    let bad = || ApiError::invalid(format!("{key} has an invalid value"));
    Ok(match d.kind {
        Kind::Bool => Value::Bool(v.as_bool().ok_or_else(bad)?),
        Kind::Num(lo, hi) => {
            let n = v.as_f64().ok_or_else(bad)?;
            if !(lo..=hi).contains(&n) || !n.is_finite() {
                return Err(ApiError::invalid(format!("{key} must be between {lo} and {hi}")));
            }
            json!(n)
        }
        Kind::Int(lo, hi) => {
            let n = v.as_f64().ok_or_else(bad)?;
            if n.fract() != 0.0 || n < lo as f64 || n > hi as f64 {
                return Err(ApiError::invalid(format!("{key} must be a whole number from {lo} to {hi}")));
            }
            json!(n as i64)
        }
        Kind::IntChoice(opts) => {
            let n = v.as_i64().ok_or_else(bad)?;
            if !opts.contains(&n) {
                return Err(bad());
            }
            json!(n)
        }
        Kind::Choice(opts) => {
            let s = v.as_str().ok_or_else(bad)?;
            if !opts.contains(&s) {
                return Err(bad());
            }
            json!(s)
        }
        Kind::Name => {
            let s = v.as_str().ok_or_else(bad)?.trim();
            if !s.eq_ignore_ascii_case(NAME) {
                return Err(ApiError::invalid(format!("The assistant is always called {NAME}.")));
            }
            json!(NAME)
        }
        Kind::Word => {
            let s = v.as_str().ok_or_else(bad)?.trim().to_lowercase();
            if s.is_empty() || s.len() > 20 || !s.chars().all(|c| c.is_alphabetic()) {
                return Err(ApiError::invalid("The stop word must be one word made of letters"));
            }
            json!(s)
        }
        Kind::Text(max) => {
            let s = v.as_str().ok_or_else(bad)?;
            if s.len() > max || s.chars().any(char::is_control) {
                return Err(bad());
            }
            json!(s)
        }
        Kind::Paths => {
            let a = v.as_array().ok_or_else(bad)?;
            let mut out = Vec::new();
            for p in a {
                let s = p.as_str().ok_or_else(bad)?;
                let path = std::path::Path::new(s);
                if !path.is_absolute() || s.len() > 4096 {
                    return Err(ApiError::invalid("Folders must be absolute paths"));
                }
                if !out.iter().any(|o: &Value| o == s) {
                    out.push(json!(s));
                }
            }
            Value::Array(out)
        }
        Kind::Object => {
            let o = v.as_object().ok_or_else(bad)?;
            let mut out = Map::new();
            for (k, val) in o {
                if !crate::tools::apps::ROLES.contains(&k.as_str()) {
                    return Err(ApiError::invalid(format!("{k} is not an app role")));
                }
                if !(val.is_string() || val.is_null()) {
                    return Err(bad());
                }
                out.insert(k.clone(), val.clone());
            }
            Value::Object(out)
        }
    })
}

pub struct Settings {
    values: RwLock<Map<String, Value>>,
}

impl Settings {
    pub fn new(stored: Map<String, Value>) -> Self {
        let mut values = Map::new();
        for d in DEFS {
            let v = stored.get(d.key).and_then(|v| validate(d.key, v).ok()).unwrap_or_else(d.default);
            values.insert(d.key.to_owned(), v);
        }
        for k in INTERNAL {
            if let Some(v) = stored.get(*k) {
                values.insert((*k).to_owned(), v.clone());
            }
        }
        Self { values: RwLock::new(values) }
    }

    pub fn all_public(&self) -> Map<String, Value> {
        let g = self.values.read().unwrap();
        g.iter().filter(|(k, _)| !INTERNAL.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn get(&self, key: &str) -> Value {
        self.values.read().unwrap().get(key).cloned().unwrap_or(Value::Null)
    }

    pub fn bool(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }

    pub fn f64(&self, key: &str) -> f64 {
        self.get(key).as_f64().unwrap_or(0.0)
    }

    pub fn str(&self, key: &str) -> String {
        self.get(key).as_str().unwrap_or_default().to_owned()
    }

    pub fn name(&self) -> String {
        NAME.to_owned()
    }

    pub fn phrase(&self) -> String {
        let n = self.name();
        if self.bool("wake.useHey") { format!("Hey {n}") } else { n }
    }

    pub fn timeout_scale(&self) -> Option<f64> {
        match self.str("ui.timeoutScale").as_str() {
            "none" => None,
            s => Some(s.parse().unwrap_or(1.0)),
        }
    }

    pub fn prepare(&self, patch: &Map<String, Value>) -> Result<Map<String, Value>, ApiError> {
        let mut out = Map::new();
        for (k, v) in patch {
            out.insert(k.clone(), validate(k, v)?);
        }
        Ok(out)
    }

    pub fn apply(&self, changes: &Map<String, Value>) -> Map<String, Value> {
        let mut g = self.values.write().unwrap();
        let mut changed = Map::new();
        for (k, v) in changes {
            if g.get(k) != Some(v) {
                g.insert(k.clone(), v.clone());
                changed.insert(k.clone(), v.clone());
            }
        }
        changed
    }

    pub fn set_internal(&self, key: &str, v: Value) {
        self.values.write().unwrap().insert(key.to_owned(), v);
    }

    pub fn reset_defaults(&self) -> Map<String, Value> {
        let mut m = Map::new();
        for d in DEFS {
            m.insert(d.key.to_owned(), (d.default)());
        }
        let mut g = self.values.write().unwrap();
        *g = m.clone();
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_validation() {
        let s = Settings::new(Map::new());
        assert_eq!(s.name(), "Setu");
        assert_eq!(s.phrase(), "Hey Setu");
        assert!(validate("session.activeMinutes", &json!(61)).is_err());
        assert!(validate("session.activeMinutes", &json!(1.5)).is_err());
        assert_eq!(validate("session.activeMinutes", &json!(30)).unwrap(), json!(30));
        assert!(validate("session.endSilence", &json!(0.3)).is_err());
        assert!(validate("ui.theme", &json!("neon")).is_err());
        assert!(validate("models.unloadMinutes", &json!(7)).is_err());
        assert!(validate("nope", &json!(1)).is_err());
        assert!(validate("assistant.name", &json!("A")).is_err());
        assert!(validate("assistant.name", &json!("Jarvis")).is_err());
        assert_eq!(validate("assistant.name", &json!("setu")).unwrap(), json!("Setu"));
        assert!(validate("voice.stopWord", &json!("halt now")).is_err());
        assert!(validate("files.folders", &json!(["relative"])).is_err());
        assert!(validate("apps.defaults", &json!({"browser": "firefox"})).is_ok());
        assert!(validate("apps.defaults", &json!({"weird": "x"})).is_err());
    }

    #[test]
    fn stored_invalid_values_fall_back() {
        let mut m = Map::new();
        m.insert("ui.textScale".into(), json!(9));
        m.insert("tts.rate".into(), json!(1.5));
        let s = Settings::new(m);
        assert_eq!(s.f64("ui.textScale"), 1.0);
        assert_eq!(s.f64("tts.rate"), 1.5);
    }

    #[test]
    fn apply_reports_only_changes() {
        let s = Settings::new(Map::new());
        let mut p = Map::new();
        p.insert("wake.useHey".into(), json!(true));
        p.insert("ui.theme".into(), json!("dark"));
        let c = s.apply(&s.prepare(&p).unwrap());
        assert_eq!(c.len(), 1);
        assert_eq!(s.phrase(), "Hey Setu");
    }
}
