#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all)]
pub mod bpe;
pub mod ort;

pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/sherpa.rs"));
}

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use sys::*;

const SR: i32 = 16_000;

#[derive(Default)]
struct Strings(Vec<CString>);

impl Strings {
    fn s(&mut self, v: &str) -> *const c_char {
        let c = CString::new(v.replace('\0', "")).unwrap_or_default();
        let p = c.as_ptr();
        self.0.push(c);
        p
    }

    fn opt(&mut self, v: &str) -> *const c_char {
        if v.is_empty() { ptr::null() } else { self.s(v) }
    }
}

unsafe fn copy_str(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn cpu() -> &'static [u8] {
    b"cpu\0"
}

pub fn version() -> String {
    unsafe { copy_str(SherpaOnnxGetVersionStr()) }
}

#[derive(Clone, Debug)]
pub struct TransducerPaths {
    pub encoder: String,
    pub decoder: String,
    pub joiner: String,
    pub tokens: String,
}

#[derive(Clone, Debug)]
pub struct KwsOptions {
    pub model: TransducerPaths,
    pub threads: i32,
    pub keywords: String,
    pub score: f32,
    pub threshold: f32,
    pub trailing_blanks: i32,
}

pub struct Kws {
    p: *const SherpaOnnxKeywordSpotter,
}

unsafe impl Send for Kws {}

pub struct KwsStream {
    p: *const SherpaOnnxOnlineStream,
}

unsafe impl Send for KwsStream {}

#[derive(Debug, Clone)]
pub struct KeywordHit {
    pub keyword: String,
    pub start: f32,
    pub end: f32,
}

impl Kws {
    pub fn new(o: &KwsOptions) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxKeywordSpotterConfig::default();
        c.feat_config.sample_rate = SR;
        c.feat_config.feature_dim = 80;
        c.model_config.transducer.encoder = s.s(&o.model.encoder);
        c.model_config.transducer.decoder = s.s(&o.model.decoder);
        c.model_config.transducer.joiner = s.s(&o.model.joiner);
        c.model_config.tokens = s.s(&o.model.tokens);
        c.model_config.num_threads = o.threads.max(1);
        c.model_config.provider = cpu().as_ptr() as *const c_char;
        c.max_active_paths = 4;
        c.num_trailing_blanks = o.trailing_blanks;
        c.keywords_score = o.score;
        c.keywords_threshold = o.threshold;
        let kw = CString::new(o.keywords.clone()).map_err(|e| e.to_string())?;
        c.keywords_buf = kw.as_ptr();
        c.keywords_buf_size = o.keywords.len() as i32;
        let p = unsafe { SherpaOnnxCreateKeywordSpotter(&c) };
        if p.is_null() { Err("keyword spotter could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn stream(&self, keywords: &str) -> Result<KwsStream, String> {
        let p = if keywords.is_empty() {
            unsafe { SherpaOnnxCreateKeywordStream(self.p) }
        } else {
            let kw = CString::new(keywords).map_err(|e| e.to_string())?;
            unsafe { SherpaOnnxCreateKeywordStreamWithKeywords(self.p, kw.as_ptr()) }
        };
        if p.is_null() { Err("keyword stream could not be created".into()) } else { Ok(KwsStream { p }) }
    }

    pub fn accept(&self, st: &KwsStream, samples: &[f32]) {
        unsafe { SherpaOnnxOnlineStreamAcceptWaveform(st.p, SR, samples.as_ptr(), samples.len() as i32) }
    }

    pub fn poll(&self, st: &KwsStream) -> Option<KeywordHit> {
        let mut hit = None;
        unsafe {
            while SherpaOnnxIsKeywordStreamReady(self.p, st.p) == 1 {
                SherpaOnnxDecodeKeywordStream(self.p, st.p);
                let r = SherpaOnnxGetKeywordResult(self.p, st.p);
                if r.is_null() {
                    continue;
                }
                let kw = copy_str((*r).keyword);
                if !kw.is_empty() && hit.is_none() {
                    let n = (*r).count.max(0) as usize;
                    let ts = if n > 0 && !(*r).timestamps.is_null() {
                        std::slice::from_raw_parts((*r).timestamps, n).to_vec()
                    } else {
                        Vec::new()
                    };
                    let start = (*r).start_time + ts.first().copied().unwrap_or(0.0);
                    let end = (*r).start_time + ts.last().copied().unwrap_or(0.0);
                    hit = Some(KeywordHit { keyword: kw, start, end });
                    SherpaOnnxDestroyKeywordResult(r);
                    SherpaOnnxResetKeywordStream(self.p, st.p);
                    continue;
                }
                SherpaOnnxDestroyKeywordResult(r);
            }
        }
        hit
    }
}

impl Drop for KwsStream {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOnlineStream(self.p) }
    }
}

impl Drop for Kws {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyKeywordSpotter(self.p) }
    }
}

#[derive(Clone, Debug)]
pub struct VadOptions {
    pub model: String,
    pub threshold: f32,
    pub min_silence: f32,
    pub min_speech: f32,
    pub max_speech: f32,
}

pub struct Vad {
    p: *const SherpaOnnxVoiceActivityDetector,
}

unsafe impl Send for Vad {}

pub struct Segment {
    pub start: i64,
    pub samples: Vec<f32>,
}

impl Vad {
    pub fn new(o: &VadOptions) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxVadModelConfig::default();
        c.silero_vad.model = s.s(&o.model);
        c.silero_vad.threshold = o.threshold;
        c.silero_vad.min_silence_duration = o.min_silence;
        c.silero_vad.min_speech_duration = o.min_speech;
        c.silero_vad.max_speech_duration = o.max_speech;
        c.silero_vad.window_size = 512;
        c.sample_rate = SR;
        c.num_threads = 1;
        c.provider = cpu().as_ptr() as *const c_char;
        let p = unsafe { SherpaOnnxCreateVoiceActivityDetector(&c, o.max_speech.max(10.0) + 5.0) };
        if p.is_null() { Err("voice activity detector could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn accept(&self, samples: &[f32]) {
        unsafe { SherpaOnnxVoiceActivityDetectorAcceptWaveform(self.p, samples.as_ptr(), samples.len() as i32) }
    }

    pub fn speaking(&self) -> bool {
        unsafe { SherpaOnnxVoiceActivityDetectorDetected(self.p) == 1 }
    }

    pub fn pop(&self) -> Vec<Segment> {
        let mut out = Vec::new();
        unsafe {
            while SherpaOnnxVoiceActivityDetectorEmpty(self.p) == 0 {
                let seg = SherpaOnnxVoiceActivityDetectorFront(self.p);
                if !seg.is_null() {
                    let n = (*seg).n.max(0) as usize;
                    let samples = if n > 0 { std::slice::from_raw_parts((*seg).samples, n).to_vec() } else { Vec::new() };
                    out.push(Segment { start: (*seg).start, samples });
                    SherpaOnnxDestroySpeechSegment(seg);
                }
                SherpaOnnxVoiceActivityDetectorPop(self.p);
            }
        }
        out
    }

    pub fn flush(&self) {
        unsafe { SherpaOnnxVoiceActivityDetectorFlush(self.p) }
    }

    pub fn reset(&self) {
        unsafe { SherpaOnnxVoiceActivityDetectorReset(self.p) }
    }
}

impl Drop for Vad {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyVoiceActivityDetector(self.p) }
    }
}

pub struct Denoiser {
    p: *const SherpaOnnxOnlineSpeechDenoiser,
}

unsafe impl Send for Denoiser {}

impl Denoiser {
    pub fn new(model: &str) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxOnlineSpeechDenoiserConfig::default();
        c.model.gtcrn.model = s.s(model);
        c.model.num_threads = 1;
        c.model.provider = cpu().as_ptr() as *const c_char;
        let p = unsafe { SherpaOnnxCreateOnlineSpeechDenoiser(&c) };
        if p.is_null() { Err("denoiser could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn run(&self, samples: &[f32], out: &mut Vec<f32>) {
        unsafe {
            let a = SherpaOnnxOnlineSpeechDenoiserRun(self.p, samples.as_ptr(), samples.len() as i32, SR);
            if !a.is_null() {
                let n = (*a).n.max(0) as usize;
                if n > 0 {
                    out.extend_from_slice(std::slice::from_raw_parts((*a).samples, n));
                }
                SherpaOnnxDestroyDenoisedAudio(a);
            }
        }
    }

    pub fn reset(&self) {
        unsafe { SherpaOnnxOnlineSpeechDenoiserReset(self.p) }
    }
}

impl Drop for Denoiser {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOnlineSpeechDenoiser(self.p) }
    }
}

pub struct OnlineAsr {
    p: *const SherpaOnnxOnlineRecognizer,
}

unsafe impl Send for OnlineAsr {}

pub struct OnlineStream {
    p: *const SherpaOnnxOnlineStream,
}

unsafe impl Send for OnlineStream {}

impl OnlineAsr {
    pub fn new(m: &TransducerPaths, threads: i32) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxOnlineRecognizerConfig::default();
        c.feat_config.sample_rate = SR;
        c.feat_config.feature_dim = 80;
        c.model_config.transducer.encoder = s.s(&m.encoder);
        c.model_config.transducer.decoder = s.s(&m.decoder);
        c.model_config.transducer.joiner = s.s(&m.joiner);
        c.model_config.tokens = s.s(&m.tokens);
        c.model_config.num_threads = threads.max(1);
        c.model_config.provider = cpu().as_ptr() as *const c_char;
        c.decoding_method = s.s("modified_beam_search");
        c.max_active_paths = 4;
        c.enable_endpoint = 0;
        let p = unsafe { SherpaOnnxCreateOnlineRecognizer(&c) };
        if p.is_null() { Err("streaming recognizer could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn stream(&self) -> OnlineStream {
        OnlineStream { p: unsafe { SherpaOnnxCreateOnlineStream(self.p) } }
    }

    pub fn accept(&self, st: &OnlineStream, samples: &[f32]) {
        unsafe { SherpaOnnxOnlineStreamAcceptWaveform(st.p, SR, samples.as_ptr(), samples.len() as i32) }
    }

    pub fn decode(&self, st: &OnlineStream) {
        unsafe {
            while SherpaOnnxIsOnlineStreamReady(self.p, st.p) == 1 {
                SherpaOnnxDecodeOnlineStream(self.p, st.p);
            }
        }
    }

    pub fn finish(&self, st: &OnlineStream) {
        let tail = vec![0f32; (SR as usize) * 3 / 10];
        self.accept(st, &tail);
        unsafe { SherpaOnnxOnlineStreamInputFinished(st.p) };
        self.decode(st);
    }

    pub fn text(&self, st: &OnlineStream) -> String {
        unsafe {
            let r = SherpaOnnxGetOnlineStreamResult(self.p, st.p);
            if r.is_null() {
                return String::new();
            }
            let t = copy_str((*r).text);
            SherpaOnnxDestroyOnlineRecognizerResult(r);
            t
        }
    }
}

impl Drop for OnlineStream {
    fn drop(&mut self) {
        if !self.p.is_null() {
            unsafe { SherpaOnnxDestroyOnlineStream(self.p) }
        }
    }
}

impl Drop for OnlineAsr {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOnlineRecognizer(self.p) }
    }
}

#[derive(Clone, Debug)]
pub enum OfflineKind {
    Transducer { paths: TransducerPaths, nemo: bool },
    Whisper { encoder: String, decoder: String, tokens: String, language: String },
    Moonshine { preprocessor: String, encoder: String, uncached_decoder: String, cached_decoder: String, tokens: String },
}

pub struct OfflineAsr {
    p: *const SherpaOnnxOfflineRecognizer,
}

unsafe impl Send for OfflineAsr {}

pub struct OfflineText {
    pub text: String,
    pub lang: String,
}

impl OfflineAsr {
    pub fn new(k: &OfflineKind, threads: i32) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxOfflineRecognizerConfig::default();
        c.feat_config.sample_rate = SR;
        c.feat_config.feature_dim = 80;
        c.model_config.num_threads = threads.max(1);
        c.model_config.provider = cpu().as_ptr() as *const c_char;
        c.decoding_method = s.s("greedy_search");
        match k {
            OfflineKind::Transducer { paths, nemo } => {
                c.model_config.transducer.encoder = s.s(&paths.encoder);
                c.model_config.transducer.decoder = s.s(&paths.decoder);
                c.model_config.transducer.joiner = s.s(&paths.joiner);
                c.model_config.tokens = s.s(&paths.tokens);
                if *nemo {
                    c.model_config.model_type = s.s("nemo_transducer");
                }
            }
            OfflineKind::Whisper { encoder, decoder, tokens, language } => {
                c.model_config.whisper.encoder = s.s(encoder);
                c.model_config.whisper.decoder = s.s(decoder);
                c.model_config.whisper.language = s.opt(language);
                c.model_config.whisper.task = s.s("transcribe");
                c.model_config.whisper.tail_paddings = -1;
                c.model_config.tokens = s.s(tokens);
            }
            OfflineKind::Moonshine { preprocessor, encoder, uncached_decoder, cached_decoder, tokens } => {
                c.model_config.moonshine.preprocessor = s.opt(preprocessor);
                c.model_config.moonshine.encoder = s.s(encoder);
                c.model_config.moonshine.uncached_decoder = s.opt(uncached_decoder);
                c.model_config.moonshine.cached_decoder = s.opt(cached_decoder);
                c.model_config.tokens = s.s(tokens);
            }
        }
        let p = unsafe { SherpaOnnxCreateOfflineRecognizer(&c) };
        if p.is_null() { Err("offline recognizer could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn recognize(&self, samples: &[f32]) -> OfflineText {
        unsafe {
            let st = SherpaOnnxCreateOfflineStream(self.p);
            if st.is_null() {
                return OfflineText { text: String::new(), lang: String::new() };
            }
            SherpaOnnxAcceptWaveformOffline(st, SR, samples.as_ptr(), samples.len() as i32);
            SherpaOnnxDecodeOfflineStream(self.p, st);
            let r = SherpaOnnxGetOfflineStreamResult(st);
            let out = if r.is_null() {
                OfflineText { text: String::new(), lang: String::new() }
            } else {
                let t = OfflineText { text: copy_str((*r).text), lang: copy_str((*r).lang) };
                SherpaOnnxDestroyOfflineRecognizerResult(r);
                t
            };
            SherpaOnnxDestroyOfflineStream(st);
            out
        }
    }
}

impl Drop for OfflineAsr {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOfflineRecognizer(self.p) }
    }
}

pub struct Tts {
    p: *const SherpaOnnxOfflineTts,
}

unsafe impl Send for Tts {}

struct CbState<'a> {
    f: &'a mut dyn FnMut(&[f32]) -> bool,
}

unsafe extern "C" fn tts_cb(samples: *const f32, n: i32, _progress: f32, arg: *mut c_void) -> i32 {
    let st = unsafe { &mut *(arg as *mut CbState) };
    let s = if n > 0 && !samples.is_null() { unsafe { std::slice::from_raw_parts(samples, n as usize) } } else { &[][..] };
    if (st.f)(s) { 1 } else { 0 }
}

impl Tts {
    pub fn new(model: &str, tokens: &str, threads: i32) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxOfflineTtsConfig::default();
        c.model.vits.model = s.s(model);
        c.model.vits.tokens = s.s(tokens);
        c.model.vits.noise_scale = 0.667;
        c.model.vits.noise_scale_w = 0.8;
        c.model.vits.length_scale = 1.0;
        c.model.num_threads = threads.max(1);
        c.model.provider = cpu().as_ptr() as *const c_char;
        c.max_num_sentences = 1;
        c.silence_scale = 0.2;
        let p = unsafe { SherpaOnnxCreateOfflineTts(&c) };
        if p.is_null() { Err("speech synthesis could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn kokoro(model: &str, voices: &str, tokens: &str, threads: i32) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxOfflineTtsConfig::default();
        c.model.kokoro.model = s.s(model);
        c.model.kokoro.voices = s.s(voices);
        c.model.kokoro.tokens = s.s(tokens);
        c.model.kokoro.length_scale = 1.0;
        c.model.num_threads = threads.max(1);
        c.model.provider = cpu().as_ptr() as *const c_char;
        c.max_num_sentences = 1;
        c.silence_scale = 0.2;
        let p = unsafe { SherpaOnnxCreateOfflineTts(&c) };
        if p.is_null() { Err("speech synthesis could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { SherpaOnnxOfflineTtsSampleRate(self.p) as u32 }
    }

    pub fn speakers(&self) -> i32 {
        unsafe { SherpaOnnxOfflineTtsNumSpeakers(self.p) }
    }

    pub fn generate(&self, phonemes: &[String], sid: i32, speed: f32, f: &mut dyn FnMut(&[f32]) -> bool) {
        let mut cps: Vec<i32> = Vec::new();
        let mut lens: Vec<i32> = Vec::new();
        for sentence in phonemes {
            let before = cps.len();
            cps.extend(sentence.chars().map(|c| c as i32));
            let n = (cps.len() - before) as i32;
            if n > 0 {
                lens.push(n);
            }
        }
        if lens.is_empty() {
            return;
        }
        let empty = CString::default();
        let mut g = SherpaOnnxGenerationConfig::default();
        g.silence_scale = 0.2;
        g.speed = speed.clamp(0.5, 2.0);
        g.sid = sid;
        g.phoneme_codepoints = cps.as_ptr();
        g.phoneme_codepoints_lens = lens.as_ptr();
        g.phoneme_codepoints_num_sentences = lens.len() as i32;
        let mut st = CbState { f };
        unsafe {
            let a = SherpaOnnxOfflineTtsGenerateWithConfig(
                self.p,
                empty.as_ptr(),
                &g,
                Some(tts_cb),
                &mut st as *mut CbState as *mut c_void,
            );
            if !a.is_null() {
                SherpaOnnxDestroyOfflineTtsGeneratedAudio(a);
            }
        }
    }
}

impl Drop for Tts {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroyOfflineTts(self.p) }
    }
}

pub const SPEAKER_STEP: usize = 32_000;
pub const SPEAKER_TRIM: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    Cut,
    Fill,
    Mixed(usize),
}

pub fn speaker_length(n: usize, fit: Fit) -> usize {
    let at = |k: usize| k * SPEAKER_STEP - SPEAKER_TRIM;
    if n <= at(1) {
        return at(1);
    }
    let k = (n + SPEAKER_TRIM) / SPEAKER_STEP;
    let over = n - at(k);
    match fit {
        Fit::Cut => at(k),
        Fit::Fill => if over == 0 { at(k) } else { at(k + 1) },
        Fit::Mixed(limit) => if over <= limit { at(k) } else { at(k + 1) },
    }
}

pub fn fit_speaker(samples: &[f32], fit: Fit) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let want = speaker_length(samples.len(), fit);
    if want <= samples.len() {
        return samples[..want].to_vec();
    }
    samples.iter().cycle().take(want).copied().collect()
}

pub struct SpeakerEmbedder {
    p: *const SherpaOnnxSpeakerEmbeddingExtractor,
}

unsafe impl Send for SpeakerEmbedder {}

impl SpeakerEmbedder {
    pub fn new(model: &str) -> Result<Self, String> {
        let mut s = Strings::default();
        let mut c = SherpaOnnxSpeakerEmbeddingExtractorConfig::default();
        c.model = s.s(model);
        c.num_threads = 1;
        c.provider = cpu().as_ptr() as *const c_char;
        let p = unsafe { SherpaOnnxCreateSpeakerEmbeddingExtractor(&c) };
        if p.is_null() { Err("speaker model could not be created".into()) } else { Ok(Self { p }) }
    }

    pub fn embed(&self, samples: &[f32]) -> Option<Vec<f32>> {
        unsafe {
            let st = SherpaOnnxSpeakerEmbeddingExtractorCreateStream(self.p);
            if st.is_null() {
                return None;
            }
            SherpaOnnxOnlineStreamAcceptWaveform(st, SR, samples.as_ptr(), samples.len() as i32);
            SherpaOnnxOnlineStreamInputFinished(st);
            let out = if SherpaOnnxSpeakerEmbeddingExtractorIsReady(self.p, st) == 1 {
                let dim = SherpaOnnxSpeakerEmbeddingExtractorDim(self.p).max(0) as usize;
                let v = SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding(self.p, st);
                if v.is_null() {
                    None
                } else {
                    let e = std::slice::from_raw_parts(v, dim).to_vec();
                    SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding(v);
                    Some(e)
                }
            } else {
                None
            };
            SherpaOnnxDestroyOnlineStream(st);
            out
        }
    }
}

impl Drop for SpeakerEmbedder {
    fn drop(&mut self) {
        unsafe { SherpaOnnxDestroySpeakerEmbeddingExtractor(self.p) }
    }
}

pub fn read_wave(path: &str) -> Option<(Vec<f32>, i32)> {
    let p = CString::new(path).ok()?;
    unsafe {
        let w = SherpaOnnxReadWave(p.as_ptr());
        if w.is_null() {
            return None;
        }
        let n = (*w).num_samples.max(0) as usize;
        let v = if n > 0 { std::slice::from_raw_parts((*w).samples, n).to_vec() } else { Vec::new() };
        let sr = (*w).sample_rate;
        SherpaOnnxFreeWave(w);
        Some((v, sr))
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    #[test]
    fn speaker_clips_are_fitted_to_lengths_the_model_reads_correctly() {
        assert_eq!(speaker_length(16_000, Fit::Mixed(11_200)), 31_920, "a short clip is filled up to two seconds");
        assert_eq!(speaker_length(31_920, Fit::Mixed(11_200)), 31_920);
        assert_eq!(speaker_length(40_000, Fit::Mixed(11_200)), 31_920, "a little over is cut");
        assert_eq!(speaker_length(52_800, Fit::Mixed(11_200)), 63_920, "well over is filled to the next length");
        assert_eq!(speaker_length(128_000, Fit::Mixed(11_200)), 127_920);
        assert_eq!(speaker_length(52_800, Fit::Cut), 31_920);
        assert_eq!(speaker_length(32_000, Fit::Fill), 63_920);
        let clip: Vec<f32> = (0..20_000).map(|i| i as f32).collect();
        let f = fit_speaker(&clip, Fit::Mixed(11_200));
        assert_eq!(f.len(), 31_920);
        assert_eq!(f[20_000], 0.0, "filling repeats the clip from its start");
        assert_eq!(f[19_999], 19_999.0);
        assert!(fit_speaker(&[], Fit::Cut).is_empty());
        for n in [8_000usize, 31_921, 47_000, 90_000, 400_000] {
            for fit in [Fit::Cut, Fit::Fill, Fit::Mixed(11_200)] {
                let len = speaker_length(n, fit);
                assert_eq!((len + SPEAKER_TRIM) % SPEAKER_STEP, 0, "{n} {fit:?}");
            }
        }
    }
}
