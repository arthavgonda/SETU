use crate::paths::Paths;
use crate::storage::Db;
use anyhow::{Result, anyhow};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use setu_common::engine::{AsrConfig, Device, LlmConfig, OfflineModel, StreamingModel, TtsConfig, TtsKind};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

static MANIFEST: &str = include_str!("../assets/manifest.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDef {
    pub id: String,
    pub slot: String,
    #[serde(default)]
    pub role: Option<String>,
    pub name: String,
    #[serde(default)]
    pub bundled: bool,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub archive: String,
    #[serde(default)]
    pub root: Option<String>,
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub context: Option<u32>,
    #[serde(default)]
    pub voice: Option<String>,
    pub ram: u64,
    pub languages: Vec<String>,
    pub license: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub params: Option<String>,
    #[serde(default)]
    pub quant: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub speakers: Vec<Speaker>,
    #[serde(default)]
    pub gpu: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Speaker {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sid: Option<i32>,
    #[serde(default)]
    pub prompt: Option<String>,
    pub gender: String,
    pub accent: String,
    #[serde(default)]
    pub espeak: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierDef {
    pub id: String,
    pub name: String,
    pub asr: Vec<String>,
    pub llm: String,
    pub tts: String,
    #[serde(default)]
    pub tts_voice: Option<String>,
    pub min_ram: u64,
    pub summary: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LangDef {
    #[serde(default)]
    pub asr: Vec<String>,
    #[serde(default)]
    pub tts: Vec<String>,
    #[serde(default)]
    pub tts_voice: Option<String>,
    #[serde(default)]
    pub espeak: Option<String>,
    #[serde(default)]
    pub whisper: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub models: Vec<ModelDef>,
    pub tiers: Vec<TierDef>,
    pub languages: HashMap<String, LangDef>,
}

impl Manifest {
    pub fn load() -> Self {
        serde_json::from_str(MANIFEST).expect("bundled manifest is valid")
    }

    pub fn model(&self, id: &str) -> Option<&ModelDef> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn tier(&self, id: &str) -> Option<&TierDef> {
        self.tiers.iter().find(|t| t.id == id)
    }

    pub fn tier_models(&self, tier: &TierDef, lang: &str) -> Vec<String> {
        let mut v: Vec<String> = tier.asr.clone();
        v.push(tier.llm.clone());
        v.push(tier.tts.clone());
        if let Some(l) = self.languages.get(lang) {
            v.extend(l.asr.iter().cloned());
            if let Some(t) = l.tts.first() {
                v.push(t.clone());
            }
        }
        v.push(crate::speaker::MODEL.to_owned());
        self.with_requires(&v)
    }

    pub fn with_requires(&self, ids: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in ids {
            if let Some(m) = self.model(id) {
                for r in &m.requires {
                    if !out.contains(r) {
                        out.push(r.clone());
                    }
                }
            }
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
        out
    }

    pub fn speaker<'a>(&'a self, m: &'a ModelDef, voice: Option<&str>, lang: &str) -> Option<&'a Speaker> {
        voice
            .and_then(|v| m.speakers.iter().find(|s| s.id == v))
            .or_else(|| m.speakers.iter().find(|s| s.languages.iter().any(|l| l == lang)))
            .or_else(|| m.speakers.first())
    }
}

pub fn template_for_file(path: &str) -> String {
    let n = Path::new(path).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
    if n.contains("llama") {
        "llama3".into()
    } else if n.contains("gemma") {
        "gemma".into()
    } else if n.contains("phi-4") || n.contains("phi4") {
        "phi4".into()
    } else if n.contains("granite") {
        "granite".into()
    } else if n.contains("qwen3") || n.contains("smollm3") {
        "chatml-nothink".into()
    } else {
        "chatml".into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub rel: String,
    pub sha256: String,
    pub size: u64,
    pub mtime: i64,
    #[serde(default)]
    pub dir: bool,
}

#[derive(Debug, Clone)]
pub struct Installed {
    pub id: String,
    pub files: BTreeMap<String, FileRecord>,
    pub bytes: u64,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlotConfig {
    pub provider: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub key_ref: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
}

pub struct ModelStore {
    pub manifest: Arc<Manifest>,
    paths: Paths,
    db: Db,
    installed: Mutex<HashMap<String, Installed>>,
    checked: Mutex<HashMap<String, i64>>,
}

pub fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn dir_size(p: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut count = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                let (b, c) = dir_size(&path);
                bytes += b;
                count += c;
            } else if let Ok(m) = e.metadata() {
                bytes += m.len();
                count += 1;
            }
        }
    }
    (bytes, count)
}

impl ModelStore {
    pub fn new(paths: Paths, db: Db, manifest: Arc<Manifest>) -> Self {
        let installed = db
            .read(|c| {
                let mut st = c.prepare("SELECT id, files, bytes, verified_at FROM models")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))?;
                let mut m = HashMap::new();
                for (id, files, bytes, at) in rows.flatten() {
                    if let Ok(f) = serde_json::from_str(&files) {
                        m.insert(id.clone(), Installed { id, files: f, bytes: bytes as u64, verified_at: at });
                    }
                }
                Ok(m)
            })
            .unwrap_or_default();
        Self { manifest, paths, db, installed: Mutex::new(installed), checked: Mutex::new(HashMap::new()) }
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        match self.manifest.model(id) {
            Some(m) if m.bundled => self.paths.bundled(id),
            _ => self.paths.models.join(id),
        }
    }

    pub fn is_installed(&self, id: &str) -> bool {
        match self.manifest.model(id) {
            Some(m) if m.bundled => m.files.values().all(|f| self.paths.bundled(id).join(f).exists()),
            Some(m) => self.installed.lock().unwrap().contains_key(id) && m.requires.iter().all(|r| self.is_installed(r)),
            None => false,
        }
    }

    pub fn missing_bytes(&self, id: &str) -> u64 {
        let Some(m) = self.manifest.model(id) else { return 0 };
        let own = if self.installed.lock().unwrap().contains_key(id) || m.bundled { 0 } else { m.size };
        own + m.requires.iter().map(|r| self.missing_bytes(r)).sum::<u64>()
    }

    pub fn file(&self, id: &str, key: &str) -> Option<PathBuf> {
        let m = self.manifest.model(id)?;
        let rel = m.files.get(key)?;
        let p = self.dir(id).join(rel);
        p.exists().then_some(p)
    }

    pub fn file_str(&self, id: &str, key: &str) -> Result<String> {
        self.file(id, key)
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| anyhow!("{} is missing {}", id, key))
    }

    pub fn record(&self, id: &str, files: BTreeMap<String, FileRecord>) -> Result<()> {
        let bytes: u64 = files.values().map(|f| f.size).sum();
        let now = setu_common::protocol::now_ms();
        let json = serde_json::to_string(&files)?;
        let idc = id.to_owned();
        self.db.write_sync(move |c| {
            c.execute(
                "INSERT INTO models(id, files, bytes, verified_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET files=excluded.files, bytes=excluded.bytes, verified_at=excluded.verified_at",
                params![idc, json, bytes as i64, now],
            )?;
            Ok(())
        })?;
        self.installed.lock().unwrap().insert(id.to_owned(), Installed { id: id.to_owned(), files, bytes, verified_at: now });
        self.checked.lock().unwrap().insert(id.to_owned(), now);
        Ok(())
    }

    pub fn verify(&self, id: &str) -> Result<()> {
        let m = self.manifest.model(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
        if m.bundled {
            return if self.is_installed(id) { Ok(()) } else { Err(anyhow!("bundled model {id} is missing")) };
        }
        let inst = self.installed.lock().unwrap().get(id).cloned().ok_or_else(|| anyhow!("{id} is not downloaded"))?;
        let dir = self.paths.models.join(id);
        let mut changed = false;
        for f in inst.files.values() {
            let p = dir.join(&f.rel);
            if f.dir {
                let (b, _) = dir_size(&p);
                if b != f.size {
                    return Err(anyhow!("{} in {id} changed on disk", f.rel));
                }
                continue;
            }
            let meta = std::fs::metadata(&p).map_err(|_| anyhow!("{} in {id} is missing", f.rel))?;
            if meta.len() != f.size {
                return Err(anyhow!("{} in {id} has the wrong size", f.rel));
            }
            if mtime(&p) != f.mtime {
                changed = true;
            }
        }
        if changed {
            for f in inst.files.values().filter(|f| !f.dir) {
                let p = dir.join(&f.rel);
                if sha256_file(&p)? != f.sha256 {
                    return Err(anyhow!("{} in {id} failed its checksum", f.rel));
                }
            }
            let mut files = inst.files.clone();
            for f in files.values_mut() {
                if !f.dir {
                    f.mtime = mtime(&dir.join(&f.rel));
                }
            }
            self.record(id, files)?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<u64> {
        let m = self.manifest.model(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
        if m.bundled {
            return Err(anyhow!("built-in models cannot be deleted"));
        }
        let dir = self.paths.models.join(id);
        let (bytes, _) = dir_size(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let idc = id.to_owned();
        self.db.write_sync(move |c| {
            c.execute("DELETE FROM models WHERE id = ?1", params![idc])?;
            Ok(())
        })?;
        self.installed.lock().unwrap().remove(id);
        Ok(bytes)
    }

    pub fn installed_list(&self) -> Vec<Installed> {
        let mut v: Vec<Installed> = self.installed.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn disk_used(&self) -> u64 {
        self.installed.lock().unwrap().values().map(|i| i.bytes).sum()
    }

    pub fn default_slots(&self, tier: &str, lang: &str) -> BTreeMap<String, SlotConfig> {
        let mut out = BTreeMap::new();
        let Some(t) = self.manifest.tier(tier).or_else(|| self.manifest.tier("balanced")) else { return out };
        let mut asr = t.asr.clone();
        if let Some(l) = self.manifest.languages.get(lang).filter(|l| !l.asr.is_empty()) {
            asr = l.asr.clone();
        }
        let l = self.manifest.languages.get(lang);
        let tts = l.and_then(|l| l.tts.first().cloned()).unwrap_or_else(|| t.tts.clone());
        let voice = l.filter(|l| !l.tts.is_empty()).and_then(|l| l.tts_voice.clone()).or_else(|| t.tts_voice.clone());
        out.insert("asr".into(), SlotConfig { provider: "local".into(), models: asr, ..Default::default() });
        out.insert("llm".into(), SlotConfig { provider: "local".into(), models: vec![t.llm.clone()], ..Default::default() });
        out.insert("tts".into(), SlotConfig { provider: "local".into(), models: vec![tts], voice, ..Default::default() });
        out
    }

    pub fn asr_config(&self, slot: &SlotConfig, lang: &str) -> Result<AsrConfig> {
        let mut cfg = AsrConfig { threads: 0, ..Default::default() };
        for id in &slot.models {
            let m = self.manifest.model(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
            self.verify(id)?;
            match m.role.as_deref() {
                Some("streaming") => {
                    cfg.streaming = Some(StreamingModel {
                        encoder: self.file_str(id, "encoder")?,
                        decoder: self.file_str(id, "decoder")?,
                        joiner: self.file_str(id, "joiner")?,
                        tokens: self.file_str(id, "tokens")?,
                    })
                }
                _ => {
                    cfg.offline = Some(match m.kind.as_deref() {
                        Some("whisper") => OfflineModel::Whisper {
                            encoder: self.file_str(id, "encoder")?,
                            decoder: self.file_str(id, "decoder")?,
                            tokens: self.file_str(id, "tokens")?,
                            language: self.manifest.languages.get(lang).and_then(|l| l.whisper.clone()).unwrap_or_default(),
                        },
                        Some("moonshine") => OfflineModel::Moonshine {
                            preprocessor: self.file_str(id, "preprocessor").unwrap_or_default(),
                            encoder: self.file_str(id, "encoder")?,
                            uncached_decoder: self.file_str(id, "uncached_decoder").unwrap_or_default(),
                            cached_decoder: self.file_str(id, "cached_decoder").unwrap_or_default(),
                            tokens: self.file_str(id, "tokens")?,
                        },
                        k => OfflineModel::Transducer {
                            encoder: self.file_str(id, "encoder")?,
                            decoder: self.file_str(id, "decoder")?,
                            joiner: self.file_str(id, "joiner")?,
                            tokens: self.file_str(id, "tokens")?,
                            nemo: k == Some("nemo_transducer"),
                        },
                    })
                }
            }
        }
        if cfg.streaming.is_none() && cfg.offline.is_none() {
            return Err(anyhow!("no speech recognition model is selected"));
        }
        Ok(cfg)
    }

    pub fn llm_config(&self, slot: &SlotConfig) -> Result<(LlmConfig, String)> {
        if let Some(p) = &slot.path {
            if !Path::new(p).is_file() {
                return Err(anyhow!("the language model file {p} is missing"));
            }
            return Ok((LlmConfig { path: p.clone(), threads: 0, context: 4096, gpu_layers: 99 }, template_for_file(p)));
        }
        let id = slot.models.first().ok_or_else(|| anyhow!("no language model is selected"))?;
        let m = self.manifest.model(id).ok_or_else(|| anyhow!("unknown model {id}"))?;
        self.verify(id)?;
        Ok((
            LlmConfig { path: self.file_str(id, "model")?, threads: 0, context: m.context.unwrap_or(4096), gpu_layers: 99 },
            m.template.clone().unwrap_or_else(|| "chatml-nothink".into()),
        ))
    }

    pub fn tts_config(&self, slot: &SlotConfig, lang: &str) -> Result<TtsConfig> {
        if let Some(p) = &slot.path {
            let model = Path::new(p);
            let dir = model.parent().ok_or_else(|| anyhow!("the voice file {p} is missing"))?;
            if !model.is_file() || !dir.join("tokens.txt").is_file() {
                return Err(anyhow!("the voice file {p} or its tokens.txt is missing"));
            }
            return Ok(TtsConfig {
                model: p.clone(),
                tokens: dir.join("tokens.txt").to_string_lossy().into_owned(),
                voice: slot.voice.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| "en-us".into()),
                espeak_path: if dir.join("espeak-ng-data").is_dir() { dir.to_string_lossy().into_owned() } else { String::new() },
                threads: 0,
                ..Default::default()
            });
        }
        let id = slot.models.first().ok_or_else(|| anyhow!("no voice is selected"))?;
        let m = self.manifest.model(id).ok_or_else(|| anyhow!("unknown voice {id}"))?;
        self.verify(id)?;
        let lang_espeak = self.manifest.languages.get(lang).and_then(|l| l.espeak.clone());
        match m.kind.as_deref() {
            Some("kokoro") => {
                let sp = self.manifest.speaker(m, slot.voice.as_deref(), lang).ok_or_else(|| anyhow!("{} has no voices", m.name))?;
                let espeak = lang_espeak.filter(|_| sp.languages.iter().any(|l| l == lang)).or_else(|| sp.espeak.clone()).unwrap_or_else(|| "en-gb".into());
                Ok(TtsConfig {
                    kind: TtsKind::Kokoro,
                    model: self.file_str(id, "model")?,
                    tokens: self.file_str(id, "tokens")?,
                    voices: self.file_str(id, "voices")?,
                    voice: espeak,
                    espeak_path: self.dir(id).to_string_lossy().into_owned(),
                    speaker: sp.sid.unwrap_or(0),
                    speaker_name: sp.id.clone(),
                    ..Default::default()
                })
            }
            Some("svara") => {
                let codec = m.requires.first().ok_or_else(|| anyhow!("{} has no audio decoder", m.name))?;
                self.verify(codec)?;
                let sp = self.manifest.speaker(m, slot.voice.as_deref(), lang).ok_or_else(|| anyhow!("{} has no voices", m.name))?;
                Ok(TtsConfig {
                    kind: TtsKind::Svara,
                    model: self.file_str(id, "model")?,
                    codec: self.file_str(codec, "model")?,
                    speaker_name: sp.prompt.clone().unwrap_or_else(|| "English (Indian) (Female)".into()),
                    gpu_layers: 99,
                    ..Default::default()
                })
            }
            _ => Ok(TtsConfig {
                model: self.file_str(id, "model")?,
                tokens: self.file_str(id, "tokens")?,
                voice: m.voice.clone().unwrap_or_else(|| "en-us".into()),
                espeak_path: self.dir(id).to_string_lossy().into_owned(),
                threads: 0,
                ..Default::default()
            }),
        }
    }

    pub fn ram_for(&self, slot: &SlotConfig) -> u64 {
        if let Some(p) = &slot.path {
            return std::fs::metadata(p).map(|m| m.len() + 400_000_000).unwrap_or(1_500_000_000);
        }
        self.manifest.with_requires(&slot.models).iter().filter_map(|id| self.manifest.model(id)).map(|m| m.ram).sum()
    }

    pub fn disk_for(&self, slot: &SlotConfig) -> u64 {
        self.manifest.with_requires(&slot.models).iter().map(|id| self.installed.lock().unwrap().get(id).map(|i| i.bytes).unwrap_or(0)).sum()
    }

    pub fn catalog_json(&self) -> Value {
        Value::Array(
            self.manifest
                .models
                .iter()
                .filter(|m| !m.bundled && ["asr", "llm", "tts"].contains(&m.slot.as_str()))
                .map(|m| {
                    let speakers: Vec<Value> = m.speakers.iter().map(|s| json!({ "id": s.id, "name": s.name, "gender": s.gender, "accent": s.accent, "languages": s.languages })).collect();
                    json!({
                        "id": m.id, "slot": m.slot, "name": m.name, "sizeBytes": m.size, "ramBytes": m.ram,
                        "totalBytes": m.size + m.requires.iter().filter_map(|r| self.manifest.model(r)).map(|r| r.size).sum::<u64>(),
                        "missingBytes": self.missing_bytes(&m.id),
                        "languages": m.languages, "required": false, "license": m.license,
                        "installed": self.is_installed(&m.id), "role": m.role, "voice": m.voice,
                        "family": m.family.clone().unwrap_or_else(|| m.name.clone()), "params": m.params, "quant": m.quant,
                        "description": m.description, "kind": m.kind, "gpu": m.gpu, "requires": m.requires, "speakers": speakers
                    })
                })
                .collect(),
        )
    }

    pub fn install_size(&self, id: &str) -> u64 {
        self.manifest.model(id).map(|m| if m.archive == "none" { m.size } else { m.size * 2 + m.size / 2 }).unwrap_or(0)
    }
}

pub fn total_ram() -> u64 {
    static TOTAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *TOTAL.get_or_init(|| {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        sys.total_memory()
    })
}

pub fn fit(manifest: &Manifest, id: &str, total: u64, gpu: Option<&Device>) -> (bool, bool) {
    let Some(m) = manifest.model(id) else { return (true, false) };
    if m.gpu.as_deref() == Some("needed") {
        return match gpu {
            Some(g) if g.total >= m.size + m.size / 4 + 500_000_000 => (true, false),
            Some(_) => (m.ram + 1_500_000_000 <= total || total == 0, true),
            None => (false, true),
        };
    }
    if total == 0 {
        return (true, false);
    }
    if let Some(g) = gpu
        && m.slot == "llm" && g.total >= m.size + m.size / 4 + 500_000_000 {
            return (true, false);
        }
    let reserve = if m.slot == "llm" { 1_500_000_000 } else { 1_000_000_000 };
    let fits = m.ram + reserve <= total;
    let min_tier = manifest.tiers.iter().filter(|t| t.llm == id || t.tts == id || t.asr.iter().any(|a| a == id)).map(|t| t.min_ram).min();
    let heavy = m.slot == "llm" && m.size > total / 4;
    (fits, fits && (min_tier.is_some_and(|r| total < r) || heavy))
}

pub fn free_bytes(p: &Path) -> u64 {
    #[cfg(unix)]
    {
        let probe = std::iter::successors(Some(p), |q| q.parent()).find(|q| q.exists()).unwrap_or(Path::new("/"));
        let Ok(c) = std::ffi::CString::new(probe.to_string_lossy().as_bytes()) else { return 0 };
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0 {
            return s.f_bavail as u64 * s.f_frsize as u64;
        }
        0
    }
    #[cfg(not(unix))]
    {
        let disks = sysinfo::Disks::new_with_refreshed_list();
        disks
            .list()
            .iter()
            .filter(|d| p.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| d.available_space())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_consistent() {
        let m = Manifest::load();
        for t in &m.tiers {
            for id in t.asr.iter().chain([&t.llm, &t.tts]) {
                assert!(m.model(id).is_some(), "{} references missing {id}", t.id);
            }
        }
        for (lang, l) in &m.languages {
            for id in l.asr.iter().chain(l.tts.iter()) {
                assert!(m.model(id).is_some(), "{lang} references missing {id}");
            }
        }
        for md in &m.models {
            assert_eq!(md.sha256.len(), 64, "{}", md.id);
            assert!(md.url.starts_with("https://"), "{}", md.id);
            assert!(md.size > 0);
            if md.archive == "tar.bz2" {
                assert!(md.root.is_some(), "{}", md.id);
            }
        }
        let bundled: Vec<_> = m.models.iter().filter(|x| x.bundled).map(|x| x.slot.as_str()).collect();
        assert_eq!(bundled, vec!["wake", "vad", "denoise"]);
        let mut ids: Vec<&str> = m.models.iter().map(|x| x.id.as_str()).collect();
        ids.sort();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "model ids are unique");
        for md in &m.models {
            for r in &md.requires {
                assert!(m.model(r).is_some(), "{} requires missing {r}", md.id);
            }
            if md.kind.as_deref() == Some("kokoro") {
                assert!(!md.speakers.is_empty() && md.speakers.iter().all(|s| s.sid.is_some()), "{}", md.id);
                for k in ["model", "voices", "tokens", "espeak"] {
                    assert!(md.files.contains_key(k), "{} lacks {k}", md.id);
                }
            }
            if md.kind.as_deref() == Some("svara") {
                assert!(md.speakers.iter().all(|s| s.prompt.is_some()), "{}", md.id);
            }
            if md.slot == "llm" {
                assert!(["chatml", "chatml-nothink", "llama3", "gemma", "phi4", "granite"].contains(&md.template.as_deref().unwrap_or("")), "{}", md.id);
            }
        }
        for t in &m.tiers {
            let tts = m.model(&t.tts).unwrap();
            if let Some(v) = &t.tts_voice {
                assert!(tts.speakers.iter().any(|s| &s.id == v), "{} voice {v}", t.id);
            }
            assert!(m.tier_models(t, "en").contains(&crate::speaker::MODEL.to_owned()));
        }
    }

    #[test]
    fn requires_come_first() {
        let m = Manifest::load();
        let v = m.with_requires(&["tts-svara".to_owned()]);
        assert_eq!(v, vec!["codec-snac-24k".to_owned(), "tts-svara".to_owned()]);
    }
}
