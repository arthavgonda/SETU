use crate::frame::{FrameError, MAX_FRAME, check_len};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const KIND_JSON: u8 = 0;
pub const KIND_PCM: u8 = 1;
pub const KIND_BYTES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum EngineKind {
    Asr,
    Llm,
    Tts,
    Vision,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asr => "asr",
            Self::Llm => "llm",
            Self::Tts => "tts",
            Self::Vision => "vision",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "asr" => Some(Self::Asr),
            "llm" => Some(Self::Llm),
            "tts" => Some(Self::Tts),
            "vision" => Some(Self::Vision),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AsrConfig {
    pub streaming: Option<StreamingModel>,
    pub offline: Option<OfflineModel>,
    pub threads: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingModel {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OfflineModel {
    Transducer { encoder: String, decoder: String, joiner: String, tokens: String, nemo: bool },
    Whisper { encoder: String, decoder: String, tokens: String, language: String },
    Moonshine { preprocessor: String, encoder: String, uncached_decoder: String, cached_decoder: String, tokens: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    pub path: String,
    pub threads: i32,
    pub context: u32,
    pub gpu_layers: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisionConfig {
    pub model: String,
    pub projector: String,
    pub threads: i32,
    pub context: u32,
    pub gpu_layers: i32,
    pub min_tokens: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TtsKind {
    #[default]
    Piper,
    Kokoro,
    Svara,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TtsConfig {
    #[serde(default)]
    pub kind: TtsKind,
    pub model: String,
    pub tokens: String,
    pub voice: String,
    pub espeak_path: String,
    pub threads: i32,
    #[serde(default)]
    pub voices: String,
    #[serde(default)]
    pub speaker: i32,
    #[serde(default)]
    pub speaker_name: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub gpu_layers: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub name: String,
    pub kind: String,
    pub backend: String,
    pub total: u64,
    pub free: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToEngine {
    Ping { seq: u64 },
    LoadAsr { config: AsrConfig },
    LoadLlm { config: LlmConfig },
    LoadTts { config: TtsConfig },
    LoadVision { config: VisionConfig },
    Unload,
    Shutdown,
    AsrBegin { id: u64, chars: bool },
    AsrEnd { id: u64 },
    AsrCancel { id: u64 },
    LlmPrefix { key: String, text: String },
    LlmGenerate {
        id: u64,
        prefix_key: String,
        text: String,
        grammar: Option<String>,
        max_tokens: u32,
        temperature: f32,
        #[serde(default)]
        stream: bool,
    },
    LlmCancel { id: u64 },
    TtsSpeak {
        id: u64,
        lines: Vec<String>,
        speaker: i32,
        rate: f32,
        volume: f32,
        device: String,
        #[serde(default)]
        open: bool,
    },
    TtsMore { id: u64, lines: Vec<String> },
    TtsEnd { id: u64 },
    TtsStop,
    Bench { seconds: f32 },
    VisionAsk {
        id: u64,
        prompt: String,
        grammar: Option<String>,
        max_tokens: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum FromEngine {
    Ready { pid: u32 },
    Pong { seq: u64, rss: u64, busy_ms: u64 },
    Loaded {
        ram: u64,
        ms: u64,
        #[serde(default)]
        device: String,
    },
    Unloaded,
    Error { message: String, fatal: bool },
    AsrPartial { id: u64, text: String },
    AsrFinal {
        id: u64,
        text: String,
        #[serde(default)]
        online: String,
        language: String,
        ms: u64,
    },
    LlmPrefixed { key: String, tokens: u32, ms: u64 },
    LlmResult { id: u64, text: String, tokens: u32, ms: u64, cancelled: bool },
    LlmPartial { id: u64, text: String },
    TtsLine { id: u64, index: u32 },
    TtsLevel { id: u64, level: f32 },
    TtsDone { id: u64, stopped: bool, first_audio_ms: u64 },
    BenchResult { value: f64, unit: String, label: String },
    VisionResult { id: u64, text: String, ms: u64, encode_ms: u64, image_tokens: u32 },
}

pub enum Packet {
    Json(Vec<u8>),
    Pcm(Vec<f32>),
    Bytes(Vec<u8>),
}

pub fn write_json<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<(), FrameError> {
    let body = serde_json::to_vec(msg).map_err(|e| FrameError::BadPayload(e.to_string()))?;
    write_raw(w, KIND_JSON, &body)
}

pub fn write_pcm<W: Write>(w: &mut W, samples: &[f32]) -> Result<(), FrameError> {
    let mut body = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        body.extend_from_slice(&s.to_le_bytes());
    }
    write_raw(w, KIND_PCM, &body)
}

pub fn write_bytes<W: Write>(w: &mut W, body: &[u8]) -> Result<(), FrameError> {
    write_raw(w, KIND_BYTES, body)
}

fn write_raw<W: Write>(w: &mut W, kind: u8, body: &[u8]) -> Result<(), FrameError> {
    let len = body.len() + 1;
    check_len(len)?;
    let mut out = Vec::with_capacity(len + 4);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.push(kind);
    out.extend_from_slice(body);
    w.write_all(&out)?;
    w.flush()?;
    Ok(())
}

pub fn read_packet<R: Read>(r: &mut R) -> Result<Packet, FrameError> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).map_err(eof)?;
    let n = u32::from_be_bytes(len) as usize;
    if !(1..=MAX_FRAME).contains(&n) {
        return Err(FrameError::BadLength(n));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).map_err(eof)?;
    let kind = buf[0];
    buf.remove(0);
    match kind {
        KIND_JSON => Ok(Packet::Json(buf)),
        KIND_PCM => {
            if !buf.len().is_multiple_of(4) {
                return Err(FrameError::BadPayload("pcm length".into()));
            }
            Ok(Packet::Pcm(buf.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()))
        }
        KIND_BYTES => Ok(Packet::Bytes(buf)),
        k => Err(FrameError::BadPayload(format!("kind {k}"))),
    }
}

fn eof(e: std::io::Error) -> FrameError {
    if e.kind() == std::io::ErrorKind::UnexpectedEof { FrameError::Closed } else { e.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_pcm_round_trip() {
        let mut wire = Vec::new();
        write_json(&mut wire, &ToEngine::Ping { seq: 7 }).unwrap();
        write_pcm(&mut wire, &[0.5, -0.25]).unwrap();
        let mut cur = std::io::Cursor::new(wire);
        match read_packet(&mut cur).unwrap() {
            Packet::Json(b) => match serde_json::from_slice::<ToEngine>(&b).unwrap() {
                ToEngine::Ping { seq } => assert_eq!(seq, 7),
                _ => panic!(),
            },
            _ => panic!(),
        }
        match read_packet(&mut cur).unwrap() {
            Packet::Pcm(s) => assert_eq!(s, vec![0.5, -0.25]),
            _ => panic!(),
        }
    }

    #[test]
    fn bytes_round_trip_and_vision_kind_parses() {
        let mut wire = Vec::new();
        write_bytes(&mut wire, b"\x89PNG").unwrap();
        let mut cur = std::io::Cursor::new(wire);
        match read_packet(&mut cur).unwrap() {
            Packet::Bytes(b) => assert_eq!(b, b"\x89PNG"),
            _ => panic!(),
        }
        assert_eq!(EngineKind::parse("vision"), Some(EngineKind::Vision));
        assert_eq!(EngineKind::Vision.as_str(), "vision");
    }
}
