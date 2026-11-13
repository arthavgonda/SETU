use crate::core::Core;
use crate::storage::Db;
use rusqlite::params;
use std::time::{Duration, Instant};

pub const MODEL: &str = "speaker-campplus-en";
pub const MIN_SECONDS: f32 = 1.0;
pub const SHORT_SECONDS: f32 = 0.6;
pub const ACCEPT: f32 = 0.45;
pub const ACCEPT_STRICT: f32 = 0.55;
pub const LEARN: f32 = 0.65;
pub const NOVEL: f32 = 0.88;
pub const LEARN_SECONDS: f32 = 2.5;
pub const CARRY: Duration = Duration::from_secs(8);
pub const FRESH: Duration = Duration::from_secs(30);
const LEARNED_MAX: usize = 12;
const LEARN_EVERY: Duration = Duration::from_secs(300);
pub const STYLES: [&str; 4] = ["quiet", "loud", "fast", "short"];
pub const FIT: setu_sherpa::Fit = setu_sherpa::Fit::Mixed(11_200);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    Normal,
    Strict,
}

#[derive(Debug, Clone, Default)]
pub struct Prints {
    pub base: Vec<Vec<f32>>,
    pub learned: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub score: f32,
    pub seconds: f32,
}

pub fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut d, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    if na > 0.0 && nb > 0.0 { d / (na.sqrt() * nb.sqrt()) } else { 0.0 }
}

impl Prints {
    pub fn all(&self) -> impl Iterator<Item = &Vec<f32>> {
        self.base.iter().chain(self.learned.iter())
    }

    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub fn best(&self, v: &[f32]) -> Option<f32> {
        self.all().map(|p| cosine(p, v)).fold(None, |m: Option<f32>, s| Some(m.map_or(s, |m| m.max(s))))
    }

    pub fn worth_learning(&self, v: &[f32], seconds: f32) -> bool {
        let Some(s) = self.best(v) else { return false };
        seconds >= LEARN_SECONDS && (LEARN..NOVEL).contains(&s)
    }
}

pub fn decide(heard: Option<&Heard>, last: Option<(Duration, f32)>, level: Level) -> bool {
    let need = if level == Level::Strict { ACCEPT_STRICT } else { ACCEPT };
    match heard {
        Some(h) if h.seconds >= MIN_SECONDS => h.score >= need,
        Some(h) if h.score >= ACCEPT_STRICT => {
            let _ = h;
            true
        }
        _ if level == Level::Strict => false,
        _ => last.is_some_and(|(age, score)| age <= CARRY && score >= ACCEPT),
    }
}

pub fn model_path(core: &Core) -> Option<String> {
    core.store.is_installed(MODEL).then(|| core.store.file_str(MODEL, "model").ok()).flatten()
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn floats(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect()
}

static EMBEDDER: std::sync::Mutex<Option<(String, setu_sherpa::SpeakerEmbedder)>> = std::sync::Mutex::new(None);
static LEARNED_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

fn embed(core: &Core, samples: &[f32]) -> Option<Vec<f32>> {
    let model = model_path(core)?;
    let mut g = EMBEDDER.lock().unwrap();
    if g.as_ref().is_none_or(|(m, _)| m != &model) {
        *g = setu_sherpa::SpeakerEmbedder::new(&model).ok().map(|e| (model.clone(), e));
    }
    let fitted = setu_sherpa::fit_speaker(samples, FIT);
    g.as_ref()?.1.embed(&fitted).map(unit)
}

pub fn enroll(core: &Core, samples: &[Vec<f32>]) -> Result<Vec<f32>, String> {
    model_path(core).ok_or("The speaker model is not downloaded.")?;
    let each: Vec<Vec<f32>> = samples.iter().filter_map(|s| embed(core, s)).collect();
    if each.is_empty() {
        return Err("Your voice could not be measured. Try again in a quieter place.".into());
    }
    let mut mean = vec![0f32; each[0].len()];
    for v in &each {
        for (m, x) in mean.iter_mut().zip(v) {
            *m += x / each.len() as f32;
        }
    }
    let centroid = unit(mean);
    let (c, rows) = (centroid.clone(), each);
    core.db
        .write_sync(move |conn| {
            let now = setu_common::protocol::now_ms();
            conn.execute("DELETE FROM speaker", [])?;
            conn.execute("INSERT INTO speaker(time, embedding, kind) VALUES(?1, ?2, 'base')", params![now, blob(&c)])?;
            for (i, v) in rows.iter().enumerate() {
                conn.execute("INSERT INTO speaker(time, embedding, kind) VALUES(?1, ?2, 'sample')", params![now + 1 + i as i64, blob(v)])?;
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(centroid)
}

pub fn load_centroid(db: &Db) -> Option<Vec<f32>> {
    db.read(|c| {
        let b: Option<Vec<u8>> = c.query_row("SELECT embedding FROM speaker WHERE kind = 'base' ORDER BY time DESC LIMIT 1", [], |r| r.get(0)).ok();
        Ok(b.map(|b| floats(&b)))
    })
    .ok()
    .flatten()
}

pub fn load_prints(db: &Db) -> Prints {
    db.read(|c| {
        let mut st = c.prepare("SELECT kind, embedding FROM speaker ORDER BY time")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
        let mut p = Prints::default();
        for (kind, b) in rows.flatten() {
            let v = unit(floats(&b));
            if kind == "learned" { p.learned.push(v) } else { p.base.push(v) }
        }
        Ok(p)
    })
    .unwrap_or_default()
}

pub fn enrolled(db: &Db) -> bool {
    load_centroid(db).is_some()
}

pub fn styles(db: &Db) -> Vec<String> {
    db.read(|c| {
        let mut st = c.prepare("SELECT DISTINCT kind FROM speaker WHERE kind NOT IN ('base', 'sample', 'learned')")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default()
}

pub fn active(core: &Core) -> bool {
    core.settings.bool("speaker.verification") && enrolled(&core.db) && model_path(core).is_some()
}

pub fn usable(core: &Core) -> bool {
    enrolled(&core.db) && model_path(core).is_some()
}

pub fn reset_old_prints(core: &Core) -> bool {
    const DONE: i64 = 1;
    if core.settings.get("migrate.speakerFit").as_i64() == Some(DONE) {
        return false;
    }
    let had = enrolled(&core.db);
    if had {
        let _ = core.db.write_sync(|conn| {
            conn.execute("DELETE FROM speaker", [])?;
            Ok(())
        });
        log::info!("removed the voice prints recorded by an older version; they were measured in a way that this version has corrected");
    }
    let was_on = core.settings.bool("speaker.verification");
    let mut save = serde_json::Map::new();
    save.insert("migrate.speakerFit".into(), serde_json::json!(DONE));
    core.settings.set_internal("migrate.speakerFit", serde_json::json!(DONE));
    if had && was_on {
        core.settings.set_internal("speaker.verification", serde_json::json!(false));
        save.insert("speaker.verification".into(), serde_json::json!(false));
    }
    core.db.save_settings(save);
    had
}

pub fn wanted(core: &Core) -> bool {
    active(core) || (core.settings.bool("vault.voiceKey") && usable(core))
}

static LAST: std::sync::Mutex<Option<(Instant, f32)>> = std::sync::Mutex::new(None);

pub fn note(h: &Heard) {
    if h.score >= ACCEPT && (h.seconds >= MIN_SECONDS || h.score >= ACCEPT_STRICT) {
        *LAST.lock().unwrap() = Some((Instant::now(), h.score));
    }
}

pub fn recent() -> Option<(Duration, f32)> {
    LAST.lock().unwrap().map(|(t, s)| (t.elapsed(), s))
}

pub fn forget_recent() {
    *LAST.lock().unwrap() = None;
}

pub fn key_ok(core: &Core) -> Result<(), String> {
    if !core.settings.bool("vault.voiceKey") {
        return Ok(());
    }
    if !usable(core) {
        return Err("Your voice isn't recorded yet, so I can't use it as a second key. Record it in Settings, Voice, or say face only for passwords.".into());
    }
    match recent() {
        Some((age, score)) if age <= FRESH && score >= ACCEPT_STRICT => Ok(()),
        _ => Err("I need your voice as well for that. Say it aloud, in your normal voice.".into()),
    }
}

fn learn(core: &Core, v: &[f32]) {
    let mut at = LEARNED_AT.lock().unwrap();
    if at.is_some_and(|t| t.elapsed() < LEARN_EVERY) {
        return;
    }
    *at = Some(Instant::now());
    let b = blob(v);
    let _ = core.db.write(move |conn| {
        conn.execute("INSERT INTO speaker(time, embedding, kind) VALUES(?1, ?2, 'learned')", params![setu_common::protocol::now_ms(), b])?;
        conn.execute("DELETE FROM speaker WHERE kind = 'learned' AND id NOT IN (SELECT id FROM speaker WHERE kind = 'learned' ORDER BY time DESC LIMIT ?1)", params![LEARNED_MAX as i64])?;
        Ok(())
    });
}

pub fn hear(core: &Core, samples: &[f32]) -> Option<Heard> {
    let seconds = samples.len() as f32 / 16_000.0;
    if seconds < SHORT_SECONDS {
        return None;
    }
    let prints = load_prints(&core.db);
    if prints.is_empty() {
        return None;
    }
    let v = embed(core, samples)?;
    let score = prints.best(&v)?;
    if core.settings.bool("privacy.learn") && prints.worth_learning(&v, seconds) {
        learn(core, &v);
    }
    Some(Heard { score, seconds })
}

pub fn add_style(core: &Core, style: &str, samples: &[f32]) -> Result<f32, String> {
    if !STYLES.contains(&style) {
        return Err("That isn't a way of speaking I can learn.".into());
    }
    let prints = load_prints(&core.db);
    if prints.is_empty() {
        return Err("Record your voice first in Settings, Voice.".into());
    }
    let seconds = samples.len() as f32 / 16_000.0;
    if seconds < 1.0 {
        return Err("That was too short. Say a whole sentence.".into());
    }
    let v = embed(core, samples).ok_or("Your voice could not be measured. Try again in a quieter place.")?;
    let score = prints.best(&v).unwrap_or(0.0);
    if score < ACCEPT {
        return Err("That didn't sound enough like the voice I know, so I didn't add it.".into());
    }
    let (b, kind) = (blob(&v), style.to_owned());
    core.db
        .write_sync(move |conn| {
            conn.execute("DELETE FROM speaker WHERE kind = ?1", params![kind])?;
            conn.execute("INSERT INTO speaker(time, embedding, kind) VALUES(?1, ?2, ?3)", params![setu_common::protocol::now_ms(), b, kind])?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: f32, b: f32, c: f32) -> Vec<f32> {
        unit(vec![a, b, c])
    }

    #[test]
    fn the_best_of_several_prints_counts() {
        let p = Prints { base: vec![v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)], learned: vec![v(0.6, 0.0, 0.8)] };
        assert!((p.best(&v(0.0, 1.0, 0.0)).unwrap() - 1.0).abs() < 1e-5);
        assert!((p.best(&v(0.0, 0.0, 1.0)).unwrap() - 0.8).abs() < 1e-5, "a learned print can be the closest");
        assert_eq!(Prints::default().best(&v(1.0, 0.0, 0.0)), None);
        assert!(Prints::default().is_empty());
    }

    #[test]
    fn only_confident_new_and_long_enough_speech_is_learned() {
        let p = Prints { base: vec![v(1.0, 0.0, 0.0)], learned: vec![] };
        let close = v(0.7, 0.714, 0.0);
        assert!(p.worth_learning(&close, 3.0));
        assert!(!p.worth_learning(&close, 1.8), "too short to learn from");
        assert!(!p.worth_learning(&v(1.0, 0.05, 0.0), 3.0), "nothing new in it");
        assert!(!p.worth_learning(&v(0.5, 0.866, 0.0), 3.0), "not confident enough: it may be someone else");
    }

    #[test]
    fn long_speech_is_judged_on_its_score_and_short_speech_needs_help() {
        let h = |score: f32, seconds: f32| Heard { score, seconds };
        let recent = Some((Duration::from_secs(3), 0.6));
        let old = Some((Duration::from_secs(20), 0.6));
        assert!(decide(Some(&h(0.5, 3.0)), None, Level::Normal));
        assert!(!decide(Some(&h(0.5, 3.0)), None, Level::Strict));
        assert!(decide(Some(&h(0.58, 3.0)), None, Level::Strict));
        assert!(!decide(Some(&h(0.3, 3.0)), recent, Level::Normal), "a clear miss is not rescued by an earlier match");
        assert!(decide(Some(&h(0.3, 0.8)), recent, Level::Normal), "a very short follow-up rides on the match just before it");
        assert!(!decide(Some(&h(0.3, 0.8)), old, Level::Normal));
        assert!(decide(Some(&h(0.5, 1.2)), None, Level::Normal), "a one-second command is judged on its own score");
        assert!(!decide(Some(&h(0.3, 1.2)), recent, Level::Normal));
        assert!(!decide(None, None, Level::Normal));
        assert!(decide(None, recent, Level::Normal));
        assert!(!decide(None, recent, Level::Strict), "sensitive commands are never carried over");
        assert!(decide(Some(&h(0.6, 0.8)), None, Level::Strict), "a very short clip that matches clearly is enough");
        assert!(!decide(Some(&h(0.5, 0.8)), Some((Duration::from_secs(3), 0.4)), Level::Normal), "the earlier sentence must itself have matched");
    }
}
