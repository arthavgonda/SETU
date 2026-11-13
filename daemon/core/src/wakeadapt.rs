use crate::core::Core;
use crate::engines::Expect;
use crate::settings::Settings;
use serde_json::{Map, Value, json};
use setu_common::engine::{FromEngine, ToEngine};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const KEY: &str = "wake.variants";
const MAX: usize = 4;
static ID: AtomicU64 = AtomicU64::new(1 << 60);

pub fn variants(s: &Settings) -> Vec<String> {
    s.get(KEY).as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

fn save(core: &Core, list: &[String]) {
    let v = json!(list);
    core.settings.set_internal(KEY, v.clone());
    let mut m = Map::new();
    m.insert(KEY.into(), v);
    core.db.save_settings(m);
}

pub fn clear(core: &Core) {
    save(core, &[]);
}

pub fn candidate(text: &str, name: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let start = usize::from(words.first().is_some_and(|w| ["hey", "hay", "hi", "a", "he", "okay", "ok"].contains(w)));
    let first = *words.get(start)?;
    let name_n = crate::text::normalize(name);
    if name_n.contains(' ') || first == name_n || first.len() < 3 || first.len() > 12 || !first.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if crate::names::is_common(first) {
        return None;
    }
    let (a, b) = (crate::names::key(first), crate::names::key(&name_n));
    let close = a == b || (a.len() >= 2 && strsim::jaro_winkler(&a, &b) >= 0.75 && a.ends_with('V') == b.ends_with('V'));
    close.then(|| first.to_uppercase())
}

pub fn learn(core: &Core, text: &str) -> Option<usize> {
    let c = candidate(text, &core.settings.name())?;
    let mut list = variants(&core.settings);
    if list.contains(&c) {
        return None;
    }
    list.push(c);
    while list.len() > MAX {
        list.remove(0);
    }
    save(core, &list);
    Some(list.len())
}

pub async fn transcribe(core: &Core, samples: Vec<f32>) -> Option<String> {
    crate::pipeline::ensure_asr(core).await.ok()?;
    if core.slot("asr").provider != "local" {
        return None;
    }
    let id = ID.fetch_add(1, Ordering::Relaxed);
    let e = &core.engines.asr;
    e.send(&ToEngine::AsrBegin { id, chars: false });
    for c in samples.chunks(1600) {
        e.pcm(c.to_vec());
    }
    match e.request(&ToEngine::AsrEnd { id }, Expect::Final(id), Duration::from_secs(20)).await {
        Ok(FromEngine::AsrFinal { text, .. }) => Some(crate::text::normalize(&text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::candidate;

    #[test]
    fn picks_close_uncommon_spellings() {
        assert_eq!(candidate("hey sato what time is it", "Setu"), Some("SATO".into()));
        assert_eq!(candidate("hey saito", "Setu"), Some("SAITO".into()));
        assert_eq!(candidate("hey setu what time is it", "Setu"), None);
        assert_eq!(candidate("hey set what time", "Setu"), None);
        assert_eq!(candidate("hey what time is it", "Setu"), None);
        assert_eq!(candidate("hey city lights", "Setu"), None);
        assert_eq!(candidate("hey sato", "Mary Ann"), None);
    }
}
