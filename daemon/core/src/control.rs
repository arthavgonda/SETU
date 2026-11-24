use crate::audio::{AudioCmd, AudioEvent, Keyword, Mode, boost_for, threshold_for};
use crate::core::Core;
use crate::engines::{EngineEvent, Expect};
use crate::router::{self, Intent};
use crate::sounds::{self, Cue};
use crate::text::{self, normalize, strip_wake};
use crate::tools::{self, Outcome, Undo, codespeech};
use serde_json::{Map, Value, json};
use setu_common::engine::{EngineKind, FromEngine, ToEngine};
use setu_common::protocol::{ApiError, ApiResult, now_ms};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

mod onboard;

pub use onboard::{fallback_paragraph, shape_paragraph};

pub type Reply = oneshot::Sender<ApiResult>;

pub enum Req {
    Answer { id: String, answer: Value, via: String, reply: Reply },
    Cancel { id: Option<String>, reply: Reply },
    Undo { id: String, reply: Reply },
    Mute { muted: bool, minutes: Option<f64>, reply: Reply },
    Sleep { reply: Reply },
    WakeTest { start: bool, reply: Reply },
    Sample { voice: Option<String>, rate: Option<f64>, volume: Option<f64>, text: Option<String>, reply: Reply },
    Enroll { text: Option<String>, reply: Reply },
    Converse { screen: Option<Value>, explain: bool, reply: Reply },
    ConverseText { text: String, reply: Reply },
    Numbers { show: bool, reply: Reply },
    WifiScan { reply: Reply },
    WifiConnect { ssid: String, reply: Reply },
    WifiReveal { reply: Reply },
    CameraOpen { reply: Reply },
    CameraCapture { reply: Reply },
    CameraClose { reply: Reply },
    Confirm { spec: ConfirmSpec, then: Box<dyn FnOnce(Arc<Core>) + Send> },
    CorrectListen { history: i64, reply: Reply },
    Listen { chat: Option<i64>, reply: Reply },
    RunText { text: String, reply: Reply },
    RunIntent { intent: Intent, reply: Reply },
    RunChat { text: String, chat: i64, reply: Reply },
    Heard { text: String, score: Option<f32>, seconds: f32, addressed: bool, reply: Reply },
    ChatAnswer { inter: u64, text: String },
    Onboard { say: Option<String>, choices: Option<Vec<(String, Vec<String>)>>, reply: Reply },
    Snapshot { reply: oneshot::Sender<(Value, Value)> },
}

#[derive(Clone)]
pub struct ConfirmSpec {
    pub title: String,
    pub facts: Vec<String>,
    pub consequence: String,
    pub risk: &'static str,
    pub action_label: String,
    pub danger: bool,
}

pub enum ControlMsg {
    Audio(AudioEvent),
    Engine(EngineEvent),
    Req(Req),
    Settings(Map<String, Value>),
    Final { asr: u64, text: String, ms: u64, err: Option<String>, speaker: Option<crate::speaker::Heard> },
    AsrReady { asr: u64, ok: Result<(), String> },
    Routed { inter: u64, intent: Intent, source: &'static str, ms: u64, notice: Option<String>, say: Option<String> },
    Aside { inter: u64, line: Option<String> },
    Finished { inter: u64, action: String, intent: Box<Intent>, outcome: Box<Outcome>, ms: u64, trace: Box<crate::layers::Report> },
    Timer { kind: Timer, generation: u64 },
    Roles(usize),
    Announce(String),
    Wifi(tools::wifi::Event),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Camera(tools::camera::Event),
    NumbersFound(Result<Vec<tools::atspi::Element>, String>),
    InstallFound { inter: u64, result: Result<tools::install::Package, String> },
    Enrolled(Result<Vec<f32>, String>),
    Perceived(String),
    Clip { kind: onboard::Clip, text: String, online: String, samples: Vec<f32> },
    Interpreted { step: String, action: String, reply: String },
    Explained { step: String, text: String },
    ModelsChanged,
    Prepared { inter: u64, intent: Intent, spec: Option<ConfirmSpec>, err: Option<String> },
    RunFailed { reply: String },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timer {
    Prompt,
    Grace,
    Active,
    Unmute,
    WakeTest,
    Camera,
    Numbers,
    Unload,
    Password,
    Reading,
    ReadingLong,
    Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    Dormant,
    Capturing,
    Grace,
    Active,
}

impl Session {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dormant => "dormant",
            Self::Capturing => "capturing",
            Self::Grace => "grace",
            Self::Active => "active",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Listening,
    Thinking,
    Acting,
    Asking,
    Confirming,
    Speaking,
}

struct Interaction {
    id: u64,
    asr: u64,
    started: Instant,
    phase: Phase,
    woke: bool,
    by_name: bool,
    waited: bool,
    said: String,
    record: bool,
    action: Option<String>,
    cancel: Arc<AtomicBool>,
    tts: Option<u64>,
    strict: bool,
    voice: Option<f32>,
    spoken: bool,
    narrated: bool,
    chat: Option<i64>,
    answer: Option<String>,
    logged: bool,
    silent: bool,
    carried: bool,
    aside: bool,
    held: Option<String>,
    acted: bool,
    told: bool,
}

enum Purpose {
    App { verb: &'static str, choices: Vec<String> },
    Missing { app: String },
    Networks,
    Options,
    Then { choices: Vec<tools::Choice>, cleanup: Option<Intent> },
}

enum Pending {
    Install(tools::install::Package),
    Url(String),
    WifiQr { ssid: String, password: Vec<char> },
    External(Box<dyn FnOnce(Arc<Core>) + Send>),
    OpenUi(String),
    Act(Intent),
    Grant { item: &'static str, then: Intent },
}

enum PromptKind {
    Ask { purpose: Purpose, retried: bool },
    Confirm { pending: Option<Pending>, voice: bool },
}

struct Prompt {
    id: String,
    options: Vec<String>,
    kind: PromptKind,
    look: &'static str,
    focus: Option<usize>,
}

struct Streamed {
    inter: u64,
    offset: usize,
    words: usize,
    last_tts: Option<u64>,
    done: bool,
    open: bool,
}

struct WifiFlow {
    networks: Vec<tools::wifi::Network>,
    ssid: Option<String>,
    password: Vec<char>,
    revealed: bool,
}

pub struct Control {
    core: Arc<Core>,
    rx: mpsc::Receiver<ControlMsg>,
    session: Session,
    reason: String,
    active_until: Option<i64>,
    inter: Option<Interaction>,
    next: u64,
    asr_next: u64,
    prompt: Option<Prompt>,
    gens: HashMap<Timer, u64>,
    last_reply: Option<String>,
    understood: Option<Instant>,
    last_said: Option<(String, Option<i64>)>,
    undo: VecDeque<(String, Undo, String)>,
    muted: bool,
    muted_until: Option<i64>,
    wake_test: Option<u32>,
    enroll: Option<Vec<Vec<f32>>>,
    enroll_style: Option<String>,
    pending_style: Option<String>,
    wifi: Option<WifiFlow>,
    camera: Option<tools::camera::Camera>,
    numbers: Option<Vec<tools::atspi::Element>>,
    windows: usize,
    grace_id: Option<u64>,
    echo_name: bool,
    echo_until: Option<Instant>,
    onboard: Vec<(String, Vec<String>)>,
    mic_ok: bool,
    mic_denied: bool,
    correction_for: Option<i64>,
    converse: Option<onboard::Converse>,
    shutdown: bool,
    dictation: bool,
    plain: bool,
    dict_chunks: Vec<(usize, Option<char>)>,
    dict_last: Option<char>,
    stream: Option<Streamed>,
    secret: Option<Vec<char>>,
    end_silence: f32,
    rejected: VecDeque<Instant>,
    plan: VecDeque<String>,
    plan_answers: Vec<String>,
    plan_total: usize,
    in_plan: bool,
}

const OPEN_ENDS: &[&str] = &[
    "and", "or", "but", "the", "a", "an", "to", "of", "in", "on", "for", "with", "from", "into", "my", "your", "this", "that", "then", "so", "because", "open", "play", "search", "type",
    "close", "set", "turn", "go", "is", "are", "what", "how", "please", "um", "uh", "like", "about", "at", "by", "called", "named", "saying", "write", "find", "switch", "move", "put",
];

fn endpoint(base: f32, text: &str, complete: bool) -> f32 {
    let Some(last) = text.split_whitespace().last() else { return base };
    if OPEN_ENDS.contains(&last) {
        return (base * 2.0).clamp(1.0, 2.2);
    }
    if complete {
        return (base * 0.65).max(0.4);
    }
    base
}

fn narration(intent: &Intent, label: &str) -> Option<(String, Cue)> {
    let line = |l: &str| {
        let l = l.trim();
        (!l.is_empty()).then(|| if l.ends_with(['.', '!', '?']) { l.to_owned() } else { format!("{l}.") })
    };
    match intent {
        Intent::OpenApp { .. } => Some((line(label)?, Cue::Open)),
        Intent::CloseApp { .. } => Some((line(label)?, Cue::Close)),
        Intent::Act { tool, .. } => match tool.as_str() {
            "web.open" | "web.play" | "web.search" | "code.open" | "code.start" | "code.new_file" | "code.project" | "files.open" | "files.openfile" | "files.openfolder" | "files.latest" | "browser.window" | "browser.settings" => {
                Some((line(label)?, Cue::Open))
            }
            "window.close" | "browser.tab.close" => None,
            _ => None,
        },
        _ => None,
    }
}

fn quiet_cue(intent: &Intent) -> Option<Cue> {
    match intent {
        Intent::Act { tool, .. } if tool == "browser.tab.close" || tool == "window.close" => Some(Cue::Close),
        Intent::Act { tool, .. } if tool == "browser.tab.new" || tool == "browser.open" => Some(Cue::Open),
        _ => None,
    }
}

fn phrase_kw(tag: &str, phrase: &str, sens: f32, extra: f32) -> Keyword {
    Keyword { tag: tag.into(), phrase: phrase.into(), boost: boost_for(sens), threshold: (threshold_for(sens) + extra).clamp(0.05, 0.7) }
}

fn choice_kw(tag: &str, phrase: &str, sens: f32) -> Keyword {
    Keyword { tag: tag.into(), phrase: phrase.into(), boost: boost_for(sens) + 1.0, threshold: threshold_for(sens) }
}

fn name_kw(phrase: &str, sens: f32) -> Keyword {
    Keyword { tag: "NAME".into(), phrase: phrase.into(), boost: boost_for(sens) + 0.5, threshold: (threshold_for(sens) - 0.02).clamp(0.05, 0.7) }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn numeric_answer(t: &str) -> bool {
    let ord = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
    let mut any = false;
    for w in t.split_whitespace() {
        if w.parse::<u32>().is_ok() || ord.contains(&w) || text::find_number(w).is_some() {
            any = true;
        } else if !matches!(w, "number" | "option" | "the" | "one" | "choice" | "no" | "pick" | "choose") {
            return false;
        }
    }
    any
}

fn pick_choice(t: &str, choices: &[tools::Choice]) -> Option<usize> {
    let words: Vec<&str> = t.split_whitespace().collect();
    let has = |phrase: &str| {
        let p: Vec<&str> = phrase.split_whitespace().collect();
        !p.is_empty() && words.windows(p.len()).any(|w| w == p.as_slice())
    };
    let mut best: Option<(usize, usize)> = None;
    for (k, c) in choices.iter().enumerate() {
        let label = normalize(&c.label);
        for w in c.words.iter().map(|w| normalize(w)).chain(std::iter::once(label)) {
            let len = w.split_whitespace().count();
            if has(&w) && best.is_none_or(|(_, b)| len > b) {
                best = Some((k + 1, len));
            }
        }
    }
    best.map(|(k, _)| k)
}

fn ordinal(t: &str) -> Option<usize> {
    let words = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth"];
    words.iter().position(|w| t.split_whitespace().any(|x| x == *w)).map(|i| i + 1)
}

impl Control {
    pub fn new(core: Arc<Core>, rx: mpsc::Receiver<ControlMsg>) -> Self {
        Self {
            core,
            rx,
            session: Session::Dormant,
            reason: "start".into(),
            active_until: None,
            inter: None,
            next: 1,
            asr_next: 1,
            prompt: None,
            gens: HashMap::new(),
            last_reply: None,
            understood: None,
            last_said: None,
            undo: VecDeque::new(),
            muted: false,
            muted_until: None,
            wake_test: None,
            enroll: None,
            enroll_style: None,
            pending_style: None,
            wifi: None,
            camera: None,
            numbers: None,
            windows: 0,
            grace_id: None,
            echo_name: false,
            echo_until: None,
            onboard: Vec::new(),
            mic_ok: true,
            mic_denied: false,
            correction_for: None,
            converse: None,
            shutdown: false,
            dictation: false,
            plain: false,
            dict_chunks: Vec::new(),
            dict_last: None,
            stream: None,
            secret: None,
            end_silence: 0.7,
            rejected: VecDeque::new(),
            plan: VecDeque::new(),
            plan_answers: Vec::new(),
            plan_total: 0,
            in_plan: false,
        }
    }

    pub async fn run(mut self) {
        self.apply_audio_settings();
        self.listen();
        self.emit_session();
        while let Some(m) = self.rx.recv().await {
            self.handle(m).await;
            if self.shutdown {
                break;
            }
        }
    }

    fn arm(&mut self, kind: Timer, after: Duration) {
        let g = self.gens.entry(kind).or_insert(0);
        *g += 1;
        let generation = *g;
        let tx = self.core.control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let _ = tx.send(ControlMsg::Timer { kind, generation }).await;
        });
    }

    fn disarm(&mut self, kind: Timer) {
        *self.gens.entry(kind).or_insert(0) += 1;
    }

    fn scaled(&self, base: Duration) -> Option<Duration> {
        self.core.settings.timeout_scale().map(|s| base.mul_f64(s))
    }

    fn s(&self) -> &crate::settings::Settings {
        &self.core.settings
    }

    fn sens(&self) -> f32 {
        self.s().f64("wake.sensitivity") as f32
    }

    fn wake_set(&self) -> Vec<Keyword> {
        let sens = self.sens();
        let name = self.s().name();
        let mut v = vec![phrase_kw("WAKE", &self.s().phrase(), sens, 0.0)];
        if self.s().bool("wake.useHey") {
            v.push(phrase_kw("WAKE", &format!("Okay {name}"), sens, 0.03));
            for variant in crate::wakeadapt::variants(self.s()) {
                v.push(phrase_kw("WAKE", &format!("Hey {variant}"), sens, 0.02));
            }
        }
        v
    }

    fn name_kw(&self) -> Keyword {
        name_kw(&self.s().name(), self.sens())
    }

    fn in_wizard(&self) -> bool {
        self.windows > 0 && (!self.s().bool("setup.completed") || self.converse.is_some())
    }

    fn stop_kw(&self) -> Keyword {
        phrase_kw("STOP", &self.s().str("voice.stopWord"), 0.5, 0.0)
    }

    fn dormant_set(&self) -> Vec<Keyword> {
        let mut v = self.wake_set();
        if self.in_wizard() {
            for (t, p, sens) in [
                ("NEXT", "continue", 0.75),
                ("NEXT", "go to the next step", 0.75),
                ("NEXT", "next", 0.5),
                ("BACK", "previous step", 0.75),
                ("BACK", "take me back", 0.75),
                ("BACK", "go back", 0.5),
                ("REPEAT", "once more", 0.75),
                ("REPEAT", "repeat that", 0.5),
                ("SKIP", "skip for now", 0.75),
                ("SKIP", "skip this", 0.5),
            ] {
                v.push(choice_kw(t, p, sens));
            }
            for (k, (_, phrases)) in self.onboard.iter().enumerate() {
                for p in phrases {
                    v.push(choice_kw(&format!("O{k}"), p, 0.75));
                }
            }
        }
        v
    }

    fn active_set(&self) -> Vec<Keyword> {
        let mut v = self.wake_set();
        if self.s().bool("wake.useHey") {
            v.push(self.name_kw());
            for variant in crate::wakeadapt::variants(self.s()) {
                v.push(name_kw(&variant, self.sens()));
            }
        }
        v
    }

    fn speaking_set(&self) -> Vec<Keyword> {
        let mut v = self.active_set();
        v.push(self.stop_kw());
        v
    }

    fn confirm_set(&self) -> Vec<Keyword> {
        let mut v = vec![
            phrase_kw("YES", "yes", 0.5, 0.0),
            phrase_kw("YES", "confirm", 0.5, 0.0),
            phrase_kw("NO", "no", 0.5, 0.0),
            phrase_kw("NO", "cancel", 0.5, 0.0),
            self.stop_kw(),
        ];
        v.extend(self.wake_set());
        v
    }

    fn camera_set(&self) -> Vec<Keyword> {
        vec![
            phrase_kw("CLICK", "click", 0.5, 0.0),
            phrase_kw("CLICK", "take photo", 0.5, 0.0),
            phrase_kw("CLICK", "take a photo", 0.5, 0.0),
            phrase_kw("CLICK", "capture", 0.5, 0.0),
            phrase_kw("NO", "cancel", 0.5, 0.0),
            self.stop_kw(),
        ]
    }

    fn numbers_set(&self) -> Vec<Keyword> {
        let words = [
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty",
        ];
        let count = self.numbers.as_ref().map(|e| e.len()).unwrap_or(0).min(words.len());
        let mut v: Vec<Keyword> = words.iter().take(count).enumerate().map(|(i, w)| phrase_kw(&format!("N{}", i + 1), w, 0.5, 0.0)).collect();
        v.push(phrase_kw("HIDE", "hide numbers", 0.5, 0.0));
        v.push(self.stop_kw());
        v.extend(self.active_set());
        v
    }

    fn listen(&mut self) {
        let mode = if self.muted {
            Mode::Off
        } else if self.wake_test.is_some() {
            Mode::Spot(self.wake_set())
        } else if self.enroll.is_some() {
            Mode::Enroll
        } else if self.conversing() && self.inter.is_none() {
            Mode::Converse
        } else if self.camera.is_some() {
            Mode::Spot(self.camera_set())
        } else if self.numbers.is_some() {
            Mode::Spot(self.numbers_set())
        } else if let Some(p) = &self.prompt {
            match p.kind {
                PromptKind::Confirm { voice: true, .. } => Mode::Spot(self.confirm_set()),
                PromptKind::Confirm { .. } => Mode::Spot(self.speaking_set()),
                PromptKind::Ask { .. } => Mode::Spot(self.speaking_set()),
            }
        } else if self.inter.as_ref().is_some_and(|i| matches!(i.phase, Phase::Speaking | Phase::Acting | Phase::Thinking)) {
            Mode::Spot(self.speaking_set())
        } else {
            match self.session {
                Session::Dormant => Mode::Spot(self.dormant_set()),
                Session::Active => Mode::Spot(self.active_set()),
                Session::Grace => Mode::Await { id: self.grace_id.unwrap_or(0) },
                Session::Capturing => return,
            }
        };
        self.core.audio.send(AudioCmd::Mode(mode));
    }

    fn apply_audio_settings(&self) {
        let s = self.s();
        let gain = s.f64("audio.gain") as f32 * if s.bool("audio.quietVoice") { 2.0 } else { 1.0 };
        self.core.audio.send(AudioCmd::Device(s.str("audio.inputDevice")));
        self.core.audio.send(AudioCmd::Gain(gain));
        self.core.audio.send(AudioCmd::AutoGain(s.bool("audio.autoGain")));
        self.core.audio.send(AudioCmd::Denoise(s.bool("audio.noiseReduction")));
        self.core.audio.send(AudioCmd::EchoCancel(s.bool("audio.echoCancel")));
        self.core.audio.send(AudioCmd::EndSilence(s.f64("session.endSilence") as f32));
        self.core.audio.send(AudioCmd::Wake { sensitivity: self.sens(), long_window: s.bool("wake.longWindow") });
        if s.bool("speaker.verification") {
            self.core.audio.send(AudioCmd::Verify(crate::speaker::load_centroid(&self.core.db)));
        } else {
            self.core.audio.send(AudioCmd::Verify(None));
        }
        self.core.audio.send(AudioCmd::Listener(crate::wakeword::listener_paths(&self.core)));
        self.core.audio.send(AudioCmd::Learned(crate::wakeword::learned(s)));
    }

    fn strip_wake(&self, text: &str) -> String {
        crate::wakeword::strip_with(text, &crate::wakeword::learned(self.s()), self.core.audio.narrowband()).unwrap_or_else(|| strip_wake(text, &self.s().name()))
    }

    fn addressed(&self, heard: &str) -> bool {
        crate::wakeword::prefix_with(heard, &crate::wakeword::learned(self.s()), self.core.audio.narrowband()).is_some()
            || text::names_first(heard, &self.s().name())
            || crate::wakeadapt::variants(self.s()).iter().any(|v| text::names_first(heard, v))
    }

    fn session_json(&self) -> Value {
        let mode = if self.muted {
            "paused"
        } else if (self.wifi.as_ref().is_some_and(|w| w.ssid.is_some()) || self.secret.is_some()) && self.inter.as_ref().is_some_and(|i| !i.record) {
            "password"
        } else if self.in_wizard() {
            "wizard"
        } else if self.dictation && self.plain {
            "dictation"
        } else if self.dictation {
            "coding"
        } else {
            "normal"
        };
        json!({
            "state": self.session.as_str(),
            "reason": self.reason,
            "activeUntil": self.active_until,
            "interaction": self.inter.as_ref().map(|i| i.id),
            "mode": mode,
            "conversing": self.conversing(),
        })
    }

    fn emit_session(&self) {
        self.core.hub.emit("session.state", self.session_json());
    }

    pub fn mic_json(&self) -> Value {
        let dev = self.s().str("audio.inputDevice");
        let name = if dev.is_empty() {
            "System default".to_owned()
        } else {
            setu_audio::devices().inputs.into_iter().find(|d| d.id == dev).map(|d| d.name).unwrap_or(dev.clone())
        };
        json!({
            "muted": self.muted,
            "mutedUntil": self.muted_until,
            "available": self.mic_ok,
            "permission": if self.mic_ok { "granted" } else if self.mic_denied { "denied" } else { "unknown" },
            "device": { "id": dev, "name": name },
            "quietVoice": self.s().bool("audio.quietVoice"),
            "narrowband": self.core.audio.narrowband(),
        })
    }

    fn emit_mic(&self) {
        self.core.hub.emit("state.mic", self.mic_json());
    }

    fn unduck() {
        if crate::tools::streams::ducked() {
            tokio::spawn(crate::tools::streams::duck(false));
        }
    }

    fn set_session(&mut self, s: Session, reason: &str) {
        let was = self.session;
        if s != was && self.s().bool("audio.duck") {
            match s {
                Session::Capturing => {
                    tokio::spawn(crate::tools::streams::duck(true));
                }
                Session::Active | Session::Dormant if crate::tools::streams::ducked() => {
                    tokio::spawn(crate::tools::streams::duck(false));
                }
                _ => {}
            }
        }
        self.session = s;
        self.reason = reason.into();
        if s != Session::Active {
            self.active_until = None;
        }
        self.emit_session();
    }

    async fn handle(&mut self, m: ControlMsg) {
        match m {
            ControlMsg::Audio(e) => self.on_audio(e),
            ControlMsg::Engine(e) => self.on_engine(e),
            ControlMsg::Req(r) => self.on_req(r),
            ControlMsg::Settings(c) => self.on_settings(c),
            ControlMsg::Final { asr, text, ms, err, speaker } => self.on_final(asr, text, ms, err, speaker),
            ControlMsg::AsrReady { asr, ok } => self.on_asr_ready(asr, ok),
            ControlMsg::Routed { inter, intent, source, ms, notice, say } => {
                let acts = Self::acts(&intent);
                let chatty = acts && say.is_none() && self.inter.as_ref().is_some_and(|i| i.id == inter && text::social(&i.said));
                let label = chatty.then(|| tools::describe_with(&self.core, &intent).2);
                let alongside = say.filter(|_| acts);
                self.on_routed(inter, intent, source, ms, notice);
                if let Some(line) = alongside {
                    self.say_alongside(inter, line);
                } else if let Some(label) = label {
                    self.small_talk(inter, label);
                }
            }
            ControlMsg::Aside { inter, line } => self.on_aside(inter, line),
            ControlMsg::Finished { inter, action, intent, outcome, ms, trace } => self.on_finished(inter, action, *intent, *outcome, ms, *trace),
            ControlMsg::Timer { kind, generation } => {
                if self.gens.get(&kind) == Some(&generation) {
                    self.on_timer(kind);
                }
            }
            ControlMsg::Announce(text) => self.announce(text),
            ControlMsg::Roles(windows) => {
                let changed = (windows > 0) != (self.windows > 0);
                self.windows = windows;
                if windows == 0 && self.wake_test.take().is_some() {
                    self.disarm(Timer::WakeTest);
                    self.core.audio.send(AudioCmd::Learn(false));
                }
                if changed {
                    self.listen();
                    self.emit_session();
                }
            }
            ControlMsg::Wifi(e) => self.on_wifi(e),
            ControlMsg::Camera(e) => self.on_camera(e),
            ControlMsg::NumbersFound(r) => self.on_numbers(r),
            ControlMsg::InstallFound { inter, result } => self.on_install_found(inter, result),
            ControlMsg::Enrolled(r) => self.on_enrolled(r),
            ControlMsg::Perceived(t) => {
                if let Some(v) = crate::wakeadapt::learn(&self.core, &t) {
                    log::info!("learned how the name sounds ({} variants)", v);
                    self.core.hub.emit("wake.test", json!({ "result": "adapted", "confidence": null, "streak": self.wake_test.unwrap_or(0), "t": now_ms() }));
                    self.listen();
                }
            }
            ControlMsg::Clip { kind, text, online, samples } => self.on_clip(kind, text, online, samples),
            ControlMsg::Interpreted { step, action, reply } => self.on_interpreted(step, action, reply),
            ControlMsg::Explained { step, text } => self.on_explained(step, text),
            ControlMsg::ModelsChanged => {
                self.apply_audio_settings();
                self.listen();
            }
            ControlMsg::Prepared { inter, intent, spec, err } => self.on_prepared(inter, intent, spec, err),
            ControlMsg::RunFailed { reply } => self.on_run_failed(reply),
            ControlMsg::Shutdown => {
                self.cancel_all(None, false);
                if let Some(c) = self.camera.take() {
                    c.close();
                }
                self.core.audio.send(AudioCmd::Mode(Mode::Off));
                self.shutdown = true;
            }
        }
    }

    fn on_audio(&mut self, e: AudioEvent) {
        match e {
            AudioEvent::Keyword { tag, confidence, loud, strict } => self.on_keyword(&tag, confidence, loud, strict),
            AudioEvent::SpeechEnd { id } => self.on_speech_end(id),
            AudioEvent::NoSpeech { id } => self.on_no_speech(id),
            AudioEvent::Onset { id } => {
                if self.session == Session::Grace && self.prompt.is_none() && self.inter.is_none() && self.grace_id == Some(id) {
                    self.begin(false, 0.7);
                }
            }
            AudioEvent::EnrollSample { samples } => {
                if let Some(style) = self.enroll_style.take() {
                    self.enroll = None;
                    self.disarm(Timer::Style);
                    self.listen();
                    let core = self.core.clone();
                    crate::layers::blocking(move || {
                        let msg = match crate::speaker::add_style(&core, &style, &samples) {
                            Ok(_) => format!("Got it. I know your {} now.", crate::voicekey::style_word(&style)),
                            Err(e) => e,
                        };
                        core.tell(ControlMsg::Announce(msg));
                    });
                } else if let Some(list) = self.enroll.as_mut() {
                    list.push(samples);
                    let n = list.len();
                    self.core.hub.emit("speaker.enroll", json!({ "phase": "sample", "count": n, "needed": 3, "message": format!("Sample {n} of 3 recorded.") }));
                    if n >= 3 {
                        let samples = list.clone();
                        let core = self.core.clone();
                        crate::layers::blocking(move || {
                            let r = crate::speaker::enroll(&core, &samples);
                            core.tell(ControlMsg::Enrolled(r));
                        });
                    }
                }
            }
            AudioEvent::Missed { samples } => {
                if self.wake_test.is_some() && self.s().bool("privacy.learn") {
                    let core = self.core.clone();
                    tokio::spawn(async move {
                        if let Some(t) = crate::wakeadapt::transcribe(&core, samples).await {
                            core.tell(ControlMsg::Perceived(t));
                        }
                    });
                }
            }
            AudioEvent::Utterance { samples } => self.on_utterance(samples),
            AudioEvent::Heard { text } => {
                if self.enroll.is_some() || self.wake_test.is_some() {
                    self.learn_wake(&text);
                }
            }
            AudioEvent::DeviceLost { message, denied } => {
                self.mic_ok = false;
                self.mic_denied = denied;
                let issue = crate::attention::Issue::new("mic.missing", "danger", message);
                let issue = if denied {
                    issue.action("Turn on", "permissions.request", json!({ "item": "microphone" }))
                } else {
                    issue.action("Choose device", "ui.open", json!({ "page": "settings/voice" }))
                };
                self.core.attention.raise(&self.core.hub, issue);
                self.emit_mic();
            }
            AudioEvent::DeviceOk => {
                crate::rt::raise_capture(&self.core);
                if !self.mic_ok {
                    self.mic_ok = true;
                    self.mic_denied = false;
                    self.core.attention.clear(&self.core.hub, "mic.missing");
                    self.emit_mic();
                }
            }
            AudioEvent::Overrun(n) => {
                self.core.metrics.dropped_audio.store(n, Ordering::Relaxed);
                log::warn!("audio consumer fell behind; {n} samples dropped so far");
            }
            AudioEvent::Bandwidth { narrow } => {
                self.emit_mic();
                if narrow && self.s().bool("audio.wideband") {
                    let core = self.core.clone();
                    std::thread::spawn(move || {
                        if let Some(name) = crate::tools::system::prefer_wideband_headset() {
                            log::info!("switched {name} to wideband speech");
                            core.audio.send(AudioCmd::DevicesChanged);
                        }
                    });
                }
            }
        }
    }

    fn on_keyword(&mut self, tag: &str, confidence: f32, loud: bool, strict: bool) {
        log::info!("keyword {tag} ({confidence:.2})");
        if let Some(streak) = self.wake_test.as_mut() {
            if tag == "WAKE" {
                *streak += 1;
                let s = *streak;
                self.core.hub.emit(
                    "wake.test",
                    json!({ "result": "heard", "confidence": if loud { "clear" } else { "faint" }, "streak": s, "t": now_ms() }),
                );
                self.arm(Timer::WakeTest, Duration::from_secs(8));
            }
            return;
        }
        match tag {
            "NEXT" | "BACK" | "REPEAT" | "SKIP" => {
                if self.echoing() {
                    log::info!("ignored {tag} in the assistant's own speech");
                    return;
                }
                self.core.hub.emit("wizard.command", json!({ "command": tag.to_lowercase() }));
                self.listen();
            }
            t if t.starts_with('O') && t[1..].parse::<usize>().is_ok() => {
                if self.echoing() {
                    return;
                }
                if let Some((id, _)) = self.onboard.get(t[1..].parse::<usize>().unwrap_or(usize::MAX)) {
                    self.core.hub.emit("onboarding.answer", json!({ "id": id }));
                }
                self.listen();
            }
            "STOP" => {
                self.cancel_all(Some("Stopped."), true);
            }
            "HIDE" => self.hide_numbers(),
            "CLICK" => self.camera_capture(),
            "YES" | "NO" => {
                if let Some(p) = &self.prompt
                    && matches!(p.kind, PromptKind::Confirm { voice: true, .. }) {
                        let id = p.id.clone();
                        let _ = self.answer(&id, &json!(if tag == "YES" { "yes" } else { "no" }), "voice");
                        return;
                    }
                if tag == "NO" && self.camera.is_some() {
                    self.close_camera(Some("Camera closed."));
                }
            }
            t if t.starts_with('N') && t[1..].parse::<usize>().is_ok() => {
                let n = t[1..].parse::<usize>().unwrap_or(0);
                self.click_number(n);
            }
            "WAKE" | "NAME" => {
                if self.echoing() {
                    log::info!("ignored the wake phrase in the assistant's own speech");
                    return;
                }
                if self.camera.is_some() || self.numbers.is_some() {
                    self.close_camera(None);
                    self.hide_numbers();
                }
                if let Some(i) = &self.inter
                    && (i.phase == Phase::Speaking || i.tts.is_some()) {
                        self.core.engines.tts.send(&ToEngine::TtsStop);
                    }
                if self.prompt.is_some() {
                    self.close_prompt("cancelled", None);
                }
                if let Some(i) = self.inter.take() {
                    i.cancel.store(true, Ordering::Relaxed);
                    self.core.hub.emit("interaction.ended", json!({ "interaction": i.id, "outcome": "cancelled" }));
                }
                self.core.hub.emit("wake.detected", json!({ "confidence": confidence, "t": now_ms(), "phrase": self.s().phrase(), "interaction": self.next }));
                self.begin_from(true, tag == "NAME", 1.6);
                if let Some(i) = self.inter.as_mut() {
                    i.strict = strict;
                }
            }
            _ => {}
        }
    }

    fn learn_wake(&mut self, text: &str) {
        if !self.s().bool("privacy.learn") {
            return;
        }
        if let Some(n) = crate::wakeword::learn(&self.core, text) {
            log::info!("learned how you say the wake phrase ({n} spellings)");
            self.core.audio.send(AudioCmd::Learned(crate::wakeword::learned(self.s())));
        }
    }

    fn begin(&mut self, woke: bool, preroll: f32) {
        self.begin_from(woke, false, preroll)
    }

    fn begin_from(&mut self, woke: bool, by_name: bool, preroll: f32) {
        self.disarm(Timer::Grace);
        self.disarm(Timer::Active);
        let id = self.next;
        self.next += 1;
        let asr = self.asr_next;
        self.asr_next += 1;
        self.inter = Some(Interaction {
            id,
            asr,
            started: Instant::now(),
            phase: Phase::Listening,
            woke,
            by_name,
            waited: false,
            said: String::new(),
            record: true,
            action: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tts: None,
            strict: false,
            voice: None,
            spoken: false,
            narrated: false,
            chat: None,
            answer: None,
            logged: false,
            silent: false,
            carried: false,
            aside: false,
            held: None,
            acted: false,
            told: false,
        });
        self.core.downloads.hold.store(true, Ordering::Relaxed);
        self.set_session(Session::Capturing, if woke { "wake" } else { "continued" });
        if woke {
            sounds::play(&self.core, Cue::Wake);
        }
        self.start_capture(asr, preroll);
        self.preload();
        tokio::spawn(crate::tools::warm());
    }

    fn set_end_silence(&mut self, v: f32) {
        if (v - self.end_silence).abs() > 0.01 {
            self.end_silence = v;
            self.core.audio.send(AudioCmd::EndSilence(v));
        }
    }

    fn start_capture(&mut self, asr: u64, preroll: f32) {
        let base = self.s().f64("session.endSilence") as f32;
        self.end_silence = -1.0;
        self.set_end_silence(base);
        let remote = self.core.slot("asr").provider != "local";
        self.core.feed.open(asr, remote || crate::speaker::wanted(&self.core));
        self.core.audio.send(AudioCmd::Mode(Mode::Capture { id: asr, preroll }));
        let core = self.core.clone();
        tokio::spawn(async move {
            let ok = crate::pipeline::ensure_asr(&core).await;
            if ok.is_ok() && core.engines.asr.is_running() && core.feed.is_open(asr) {
                core.engines.asr.send(&ToEngine::AsrBegin { id: asr, chars: false });
            }
            core.tell(ControlMsg::AsrReady { asr, ok: ok.map_err(|e| e.to_string()) });
        });
    }

    fn preload(&self) {
        let core = self.core.clone();
        tokio::spawn(async move {
            let _ = crate::pipeline::ensure_tts(&core).await;
        });
        let core = self.core.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            crate::pipeline::warm(&core).await;
        });
    }

    fn on_asr_ready(&mut self, asr: u64, ok: Result<(), String>) {
        match ok {
            Ok(()) => self.core.feed.ready(asr),
            Err(e) => {
                if self.inter.as_ref().is_some_and(|i| i.asr == asr) {
                    log::warn!("speech recognition is not ready: {e}");
                    let msg = if self.core.downloads.active() {
                        "Speech recognition is still downloading. Try again when the download finishes.".to_owned()
                    } else {
                        "Speech recognition isn't ready. Download the speech model in Settings, Models.".to_owned()
                    };
                    let fix = json!({ "label": "Open Models", "method": "ui.open", "params": { "page": "settings/models" } });
                    if self.prompt.is_some() {
                        let id = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
                        self.core.hub.emit("notice.problem", json!({ "interaction": id, "message": msg, "action": fix }));
                        if let Some(i) = self.inter.as_mut() {
                            i.phase = Phase::Asking;
                        }
                        return;
                    }
                    self.problem(&msg, Some(fix));
                }
            }
        }
    }

    fn on_speech_end(&mut self, asr: u64) {
        let Some(i) = self.inter.as_mut() else { return };
        if i.asr != asr || i.phase != Phase::Listening {
            return;
        }
        i.phase = Phase::Thinking;
        let chars = !i.record;
        if i.record && !self.dictation && self.prompt.is_none() {
            sounds::play(&self.core, Cue::Heard);
        }
        self.set_session(Session::Grace, "speech ended");
        self.core.audio.send(AudioCmd::Mode(Mode::Spot(self.speaking_set())));
        let core = self.core.clone();
        let started = Instant::now();
        tokio::spawn(async move {
            let slot = core.slot("asr");
            if slot.provider == "local" && !core.feed.wait_ready(asr, Duration::from_secs(60)).await {
                core.tell(ControlMsg::Final { asr, text: String::new(), ms: 0, err: Some("speech recognition did not start".into()), speaker: None });
                return;
            }
            let audio = core.feed.close();
            let speaker = if crate::speaker::wanted(&core) && !audio.is_empty() {
                let c = core.clone();
                let a = audio.clone();
                crate::layers::blocking(move || crate::speaker::hear(&c, &a)).await.ok().flatten()
            } else {
                None
            };
            let r = if slot.provider != "local" && !audio.is_empty() {
                crate::remote::transcribe(&core, &slot, audio).await.map(|t| (t, 0))
            } else {
                match core.engines.asr.request(&ToEngine::AsrEnd { id: asr }, Expect::Final(asr), Duration::from_secs(15)).await {
                    Ok(FromEngine::AsrFinal { text, online, ms, .. }) => Ok((crate::text::repair_tail(&text, &online), ms)),
                    Ok(_) => Err(anyhow::anyhow!("unexpected reply")),
                    Err(e) => Err(e),
                }
            };
            let _ = chars;
            match r {
                Ok((text, engine_ms)) => {
                    core.metrics.record("final_transcript", ms(started.elapsed()).max(engine_ms));
                    core.tell(ControlMsg::Final { asr, text, ms: ms(started.elapsed()), err: None, speaker });
                }
                Err(e) => core.tell(ControlMsg::Final { asr, text: String::new(), ms: 0, err: Some(e.to_string()), speaker }),
            }
        });
    }

    fn on_no_speech(&mut self, asr: u64) {
        let Some(i) = &self.inter else { return };
        if i.asr != asr || i.phase != Phase::Listening {
            return;
        }
        self.core.feed.close();
        self.core.engines.asr.send(&ToEngine::AsrCancel { id: asr });
        if self.prompt.is_some() {
            return;
        }
        let id = i.id;
        self.inter = None;
        self.close_prompt("cancelled", Some("Cancelled."));
        self.core.hub.emit("interaction.ended", json!({ "interaction": id, "outcome": "discarded" }));
        self.enter_active("no speech");
    }

    fn on_final(&mut self, asr: u64, raw: String, _ms: u64, err: Option<String>, speaker: Option<crate::speaker::Heard>) {
        let Some(i) = self.inter.as_ref() else { return };
        if i.asr != asr {
            return;
        }
        let score = speaker.as_ref().map(|h| h.score);
        if crate::speaker::active(&self.core) && self.prompt.is_none() {
            let level = if i.strict { crate::speaker::Level::Strict } else { crate::speaker::Level::Normal };
            let other = !crate::speaker::decide(speaker.as_ref(), crate::speaker::recent(), level);
            if other {
                log::info!("ignored a request from another voice ({})", speaker.as_ref().map(|h| format!("{:.2} over {:.1} s", h.score, h.seconds)).unwrap_or_else(|| "too short".into()));
                let (id, woke) = (i.id, i.woke);
                self.inter = None;
                self.core.hub.emit("interaction.ended", json!({ "interaction": id, "outcome": "discarded" }));
                let now = Instant::now();
                self.rejected.retain(|t| now.duration_since(*t) < Duration::from_secs(90));
                self.rejected.push_back(now);
                if self.rejected.len() >= 2 {
                    self.rejected.clear();
                    self.core.attention.raise(
                        &self.core.hub,
                        crate::attention::Issue::new("speaker.unknown", "warning", "Setu didn't recognise your voice twice. If that was you, record your voice again.")
                            .action("Record again", "ui.open", json!({ "page": "settings/voice" }))
                            .action("Dismiss", "attention.dismiss", json!({ "code": "speaker.unknown" })),
                    );
                }
                self.core.hub.emit("speaker.rejected", json!({ "interaction": id, "t": now_ms() }));
                if woke {
                    self.go_dormant("another voice", false);
                } else {
                    self.enter_active("another voice");
                }
                return;
            }
        }
        if let Some(h) = &speaker {
            crate::speaker::note(h);
        }
        if let Some(m) = self.inter.as_mut() {
            m.voice = score;
            m.spoken = true;
        }
        if let Some(e) = err {
            log::warn!("final transcript failed: {e}");
            self.problem("I couldn't understand that. Please say it again.", None);
            return;
        }
        let Some(i) = self.inter.as_ref() else { return };
        let name = self.s().name();
        let password = (self.wifi.as_ref().is_some_and(|w| w.ssid.is_some()) || self.secret.is_some()) && !i.record;
        let heard = normalize(&raw);
        let addressed = self.addressed(&heard);
        if i.by_name && !password && !addressed {
            log::info!("the name was not confirmed by speech recognition; discarding");
            let id = i.id;
            self.inter = None;
            self.core.hub.emit("interaction.ended", json!({ "interaction": id, "outcome": "discarded" }));
            self.enter_active("not addressed");
            return;
        }
        if let Some(m) = self.inter.as_mut() {
            m.by_name = false;
        }
        let Some(i) = self.inter.as_ref() else { return };
        let woke = i.woke;
        let by_name = i.by_name;
        if woke && !by_name && !password && self.s().bool("privacy.learn") && crate::wakeadapt::learn(&self.core, &normalize(&raw)).is_some() {
            self.listen();
        }
        let Some(i) = self.inter.as_ref() else { return };
        let text = if password {
            raw.trim().to_owned()
        } else if i.woke {
            self.personalize(&self.strip_wake(&normalize(&raw)))
        } else {
            self.personalize(&text::strip_lead(&normalize(&raw), &name))
        };
        log::info!("final transcript {}", crate::logging::redact(&text));
        if self.prompt.as_ref().is_some_and(|p| matches!(p.kind, PromptKind::Confirm { voice: true, .. })) {
            let pid = self.prompt.as_ref().map(|p| p.id.clone()).unwrap_or_default();
            match text::yes_no(&text) {
                Some(yes) => {
                    let _ = self.answer(&pid, &json!(if yes { "yes" } else { "no" }), "voice");
                }
                None if text.is_empty() => self.listen_for_answer(),
                None => self.say("Please say yes or no.", "confirm"),
            }
            return;
        }
        if text.is_empty() {
            if i.woke && !i.waited && i.started.elapsed() < Duration::from_secs(4) && self.prompt.is_none() {
                let asr = self.asr_next;
                self.asr_next += 1;
                if let Some(m) = self.inter.as_mut() {
                    m.asr = asr;
                    m.phase = Phase::Listening;
                    m.waited = true;
                }
                self.set_session(Session::Capturing, "waiting for the request");
                self.start_capture(asr, 0.0);
                return;
            }
            let id = i.id;
            self.inter = None;
            self.core.hub.emit("interaction.ended", json!({ "interaction": id, "outcome": "discarded" }));
            self.enter_grace(id);
            return;
        }
        let id = i.id;
        if password {
            self.core.hub.emit("transcript.final", json!({ "interaction": id, "text": "", "language": "en" }));
            if self.secret.is_some() {
                self.on_secret(&text);
            } else {
                self.on_password(&text);
            }
            return;
        }
        let display = text::display_case(&text);
        self.core.hub.emit("transcript.final", json!({ "interaction": id, "text": display, "language": self.core.lang() }));
        if let Some(i) = self.inter.as_mut() {
            i.said = display.clone();
        }
        if let Some(h) = self.correction_for.take() {
            self.save_correction(h, &text);
            self.say("Thanks. I'll remember that.", "done");
            return;
        }
        if self.prompt.as_ref().is_some_and(|p| matches!(p.kind, PromptKind::Ask { .. })) {
            let pid = self.prompt.as_ref().unwrap().id.clone();
            let _ = self.answer(&pid, &json!(text), "voice");
            return;
        }
        if self.dictation {
            let spoken = normalize(&raw).split_whitespace().count();
            let kept = text.split_whitespace().count();
            let drop = spoken.saturating_sub(kept);
            let raw_rest: String = raw.split_whitespace().skip(drop).collect::<Vec<_>>().join(" ");
            self.dictate(id, &text, &raw_rest, addressed);
            return;
        }
        self.route(id, text);
    }

    fn end_dictation(&mut self) {
        self.dictation = false;
        self.plain = false;
        self.dict_chunks.clear();
        self.dict_last = None;
        self.emit_session();
    }

    fn dictate_text(&mut self, inter: u64, text: &str, raw: &str, addressed: bool) {
        let a = |tool: &str, args: Value| Intent::Act { tool: tool.to_owned(), args };
        let intent = match crate::dictate::command(text) {
            Some(crate::dictate::Cmd::Stop) => {
                self.end_dictation();
                self.say("Dictation is off.", "done");
                return;
            }
            Some(crate::dictate::Cmd::Scratch) => match self.dict_chunks.pop() {
                Some((n, before)) if n > 0 => {
                    self.dict_last = before;
                    a("key", json!({ "keys": "backspace", "times": n, "label": "Removed that" }))
                }
                _ => {
                    self.say("There is nothing to scratch.", "done");
                    return;
                }
            },
            Some(crate::dictate::Cmd::Key(keys, label)) => {
                if keys == "enter" {
                    self.dict_last = Some('\n');
                }
                a("key", json!({ "keys": keys, "label": label }))
            }
            None if addressed => {
                if let Some(i) = router::rules(text, &self.s().str("voice.stopWord"), self.numbers_shown()).filter(|i| !matches!(i, Intent::TypeText { .. })) {
                    self.on_routed(inter, i, "rules", 0, None);
                } else {
                    self.route(inter, text.to_owned());
                }
                return;
            }
            None => {
                let typed = crate::dictate::render(raw, self.dict_last);
                if typed.is_empty() {
                    self.finish("discarded");
                    return;
                }
                if let Some(i) = self.inter.as_mut() {
                    i.record = false;
                }
                self.dict_chunks.push((typed.chars().count(), self.dict_last));
                if self.dict_chunks.len() > 40 {
                    self.dict_chunks.remove(0);
                }
                self.dict_last = typed.chars().last();
                Intent::TypeText { text: typed }
            }
        };
        self.core.hub.emit("intent.resolved", json!({ "interaction": inter, "action": intent.action(), "target": intent.target(), "arguments": intent.args(), "layer": "accessibility", "source": "rules" }));
        self.execute(intent);
    }

    fn dictate(&mut self, inter: u64, text: &str, raw: &str, addressed: bool) {
        if self.plain {
            self.dictate_text(inter, text, raw, addressed);
            return;
        }
        let a = |tool: &str, args: Value| Intent::Act { tool: tool.to_owned(), args };
        let intent = match codespeech::command(text) {
            Some(codespeech::Cmd::Stop) => {
                self.end_dictation();
                self.say("Coding mode is off.", "done");
                return;
            }
            Some(codespeech::Cmd::Check) => a("code.check", json!({})),
            Some(codespeech::Cmd::NextError) => a("code.check", json!({ "next": true })),
            Some(codespeech::Cmd::Run) => a("code.run", json!({})),
            Some(codespeech::Cmd::GoToLine(n)) => a("code.goto", json!({ "line": n })),
            Some(codespeech::Cmd::SelectLine(n)) => a("code.edit", json!({ "op": "select_line", "n": n })),
            Some(codespeech::Cmd::Move(d, n)) => a("code.edit", json!({ "op": "move", "dir": d, "n": n })),
            Some(codespeech::Cmd::Backspace(n)) => a("code.edit", json!({ "op": "backspace", "n": n })),
            Some(cmd) => {
                let op = match cmd {
                    codespeech::Cmd::Undo => "undo",
                    codespeech::Cmd::Redo => "redo",
                    codespeech::Cmd::DeleteLine => "delete_line",
                    codespeech::Cmd::DeleteWord => "delete_word",
                    codespeech::Cmd::Save => "save",
                    codespeech::Cmd::Format => "format",
                    codespeech::Cmd::Comment => "comment",
                    codespeech::Cmd::Indent => "indent",
                    codespeech::Cmd::Outdent => "outdent",
                    codespeech::Cmd::LineEnd => "line_end",
                    codespeech::Cmd::LineStart => "line_start",
                    codespeech::Cmd::CopyLine => "copy_line",
                    codespeech::Cmd::Paste => "paste",
                    codespeech::Cmd::SelectAll => "select_all",
                    codespeech::Cmd::NewFile => "new_file",
                    codespeech::Cmd::Terminal => "terminal",
                    codespeech::Cmd::Escape => "escape",
                    _ => "enter",
                };
                a("code.edit", json!({ "op": op }))
            }
            None if addressed => {
                if let Some(i) = router::rules(text, &self.s().str("voice.stopWord"), self.numbers_shown()).filter(|i| !matches!(i, Intent::TypeText { .. })) {
                    self.on_routed(inter, i, "rules", 0, None);
                } else {
                    self.route(inter, text.to_owned());
                }
                return;
            }
            None if codespeech::write_request(text) => a("code.write", json!({ "request": text })),
            None if codespeech::comment_request(text).is_some() => a("code.comment", json!({ "text": codespeech::comment_request(text).unwrap_or_default() })),
            None => {
                let code = codespeech::render(raw);
                if code.trim().is_empty() {
                    self.finish("discarded");
                    return;
                }
                if let Some(i) = self.inter.as_mut() {
                    i.record = false;
                }
                a("code.insert", json!({ "text": code, "layout": true }))
            }
        };
        self.core.hub.emit("intent.resolved", json!({ "interaction": inter, "action": "act", "target": intent.target(), "arguments": intent.args(), "layer": "editor", "source": "rules" }));
        self.execute(intent);
    }

    fn on_llm_partial(&mut self, id: u64, text: &str) {
        if id & crate::pipeline::CHAT_BIT != 0 {
            let inter = id & !crate::pipeline::CHAT_BIT;
            if let Some(chat) = self.inter.as_ref().filter(|i| i.id == inter).and_then(|i| i.chat) {
                let so_far = crate::pipeline::partial_answer(text);
                if !so_far.trim().is_empty() {
                    self.core.hub.emit("chat.delta", json!({ "chat": chat, "interaction": inter.to_string(), "text": so_far }));
                }
            }
            return;
        }
        let inter = id & !crate::pipeline::STREAM_BIT;
        if !self.inter.as_ref().is_some_and(|i| i.id == inter && i.phase == Phase::Thinking && !i.silent) || !self.s().bool("tts.speakReplies") {
            return;
        }
        if self.stream.as_ref().is_none_or(|s| s.inter != inter) {
            self.stream = Some(Streamed { inter, offset: 0, words: 0, last_tts: None, done: false, open: false });
        }
        let offset = self.stream.as_ref().map(|s| s.offset).unwrap_or(0);
        let Some((chunk, end)) = text::stream_chunk(text, offset) else { return };
        let words = chunk.split_whitespace().count();
        let line = text::plain_reply(&chunk);
        let local = crate::pipeline::streaming(&self.core);
        let Some(st) = self.stream.as_mut() else { return };
        st.offset = end;
        st.words += words;
        if local
            && st.open
            && !st.done
            && let Some(id) = st.last_tts
        {
            crate::pipeline::speak_more(&self.core, id, vec![line]);
            return;
        }
        let tts_id = now_ms() as u64 ^ (inter << 20);
        st.last_tts = Some(tts_id);
        st.done = false;
        st.open = local;
        let core = self.core.clone();
        tokio::spawn(async move {
            let r = if local { crate::pipeline::speak_open(&core, tts_id, vec![line]).await } else { crate::pipeline::speak(&core, tts_id, vec![line]).await };
            if r.is_err() {
                let _ = core.control.send(ControlMsg::Engine(EngineEvent::TtsDone { id: tts_id, first_audio_ms: 0 })).await;
            }
        });
    }

    fn small_talk(&mut self, inter: u64, doing: String) {
        let Some(i) = self.inter.as_mut().filter(|i| i.id == inter) else { return };
        i.aside = true;
        let said = i.said.clone();
        let core = self.core.clone();
        tokio::spawn(async move {
            let line = match tokio::time::timeout(Duration::from_secs(12), crate::pipeline::small_talk(&core, inter, &said, &doing)).await {
                Ok(Ok(l)) => Some(l),
                _ => None,
            };
            core.tell(ControlMsg::Aside { inter, line });
        });
    }

    fn holding(&mut self, inter: u64, reply: Option<String>) -> bool {
        let Some(i) = self.inter.as_mut().filter(|i| i.id == inter && i.aside) else { return false };
        i.held = reply;
        i.acted = true;
        i.phase = Phase::Speaking;
        true
    }

    fn on_aside(&mut self, inter: u64, line: Option<String>) {
        let Some(i) = self.inter.as_mut().filter(|i| i.id == inter && i.aside) else { return };
        i.aside = false;
        let (held, acted) = (i.held.take(), i.acted);
        match (line, held) {
            (Some(l), Some(h)) => self.say(&format!("{l} {h}"), "done"),
            (Some(l), None) if acted => self.say(&l, "done"),
            (Some(l), None) => self.say_alongside(inter, l),
            (None, Some(h)) => self.say(&h, "done"),
            (None, None) if acted => self.finish("done"),
            (None, None) => {}
        }
    }

    fn acts(intent: &Intent) -> bool {
        !matches!(intent, Intent::Reply { .. } | Intent::Unknown | Intent::Cancel | Intent::Sleep | Intent::Ask { .. } | Intent::Repeat | Intent::Undo | Intent::Correct { .. } | Intent::Time | Intent::Date)
    }

    fn say_alongside(&mut self, inter: u64, line: String) {
        let speak = self.s().bool("tts.speakReplies");
        let Some(i) = self.inter.as_mut().filter(|i| i.id == inter) else {
            self.stream = None;
            return;
        };
        i.told = true;
        i.answer = Some(line.clone());
        if !speak || i.silent {
            self.stream = None;
            self.last_reply = Some(line);
            return;
        }
        i.narrated = true;
        let streamed = self.stream.take().filter(|s| s.inter == inter && s.words > 0);
        let id = match streamed {
            Some(st) if st.open && !st.done && st.last_tts.is_some() => {
                let words: Vec<&str> = line.split_whitespace().collect();
                let rest = words[st.words.min(words.len())..].join(" ");
                let id = st.last_tts.unwrap_or(0);
                if !rest.trim().is_empty() {
                    crate::pipeline::speak_more(&self.core, id, text::lines(&rest));
                }
                crate::pipeline::speak_end(&self.core, id);
                id
            }
            Some(st) if st.last_tts.is_some() => st.last_tts.unwrap_or(0),
            _ => {
                let id = (now_ms() as u64) ^ (inter << 24) ^ 0x5341_5953;
                let core = self.core.clone();
                let lines = text::lines(&line);
                tokio::spawn(async move {
                    if crate::pipeline::speak(&core, id, lines).await.is_err() {
                        let _ = core.control.send(ControlMsg::Engine(EngineEvent::TtsDone { id, first_audio_ms: 0 })).await;
                    }
                });
                id
            }
        };
        if let Some(i) = self.inter.as_mut() {
            i.tts = Some(id);
        }
        self.core.hub.emit("reply.speak", json!({ "id": id.to_string(), "interaction": inter, "text": line, "lines": text::lines(&line), "spoken": true }));
        self.last_reply = Some(line);
    }

    fn reply(&mut self, full: String) {
        let current = self.inter.as_ref().map(|i| i.id);
        let streamed = self.stream.take().filter(|s| Some(s.inter) == current && s.words > 0);
        let Some(st) = streamed else {
            self.say(&full, "done");
            return;
        };
        let words: Vec<&str> = full.split_whitespace().collect();
        let rest = words[st.words.min(words.len())..].join(" ");
        let live = st.open && !st.done;
        if let Some(i) = self.inter.as_mut() {
            i.answer = Some(full.clone());
        }
        if !rest.trim().is_empty() {
            if live && let Some(id) = st.last_tts {
                crate::pipeline::speak_more(&self.core, id, text::lines(&rest));
                crate::pipeline::speak_end(&self.core, id);
                let inter = current.unwrap_or(0);
                self.core.hub.emit("reply.speak", json!({ "id": id.to_string(), "interaction": inter, "text": full, "lines": text::lines(&full), "spoken": true }));
                self.last_reply = Some(full);
                if let Some(i) = self.inter.as_mut() {
                    i.phase = Phase::Speaking;
                    i.tts = Some(id);
                }
                self.listen();
                return;
            }
            self.say(&rest, "done");
            self.last_reply = Some(full);
            return;
        }
        if live && let Some(id) = st.last_tts {
            crate::pipeline::speak_end(&self.core, id);
        }
        self.last_reply = Some(full.clone());
        let inter = current.unwrap_or(0);
        self.core.hub.emit("reply.speak", json!({ "id": st.last_tts.unwrap_or(0).to_string(), "interaction": inter, "text": full, "lines": text::lines(&full), "spoken": !st.done }));
        if st.done {
            self.core.hub.emit("reply.done", json!({ "id": st.last_tts.unwrap_or(0).to_string() }));
            self.finish("done");
            return;
        }
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Speaking;
            i.tts = st.last_tts;
        }
        self.listen();
    }

    fn start_dictation(&mut self) {
        self.dictation = true;
        self.emit_session();
    }

    fn on_secret(&mut self, text: &str) {
        let ops = crate::password::parse(text);
        let Some(buf) = self.secret.as_mut() else { return };
        let mut speak: Option<String> = None;
        let mut done = false;
        for op in &ops {
            match op {
                crate::password::Op::Connect => done = true,
                crate::password::Op::ReadBack => speak = Some(if buf.is_empty() { "Nothing yet.".into() } else { crate::password::spell(buf) }),
                crate::password::Op::Cancel => {
                    self.cancel_all(Some("Cancelled."), true);
                    return;
                }
                crate::password::Op::Show | crate::password::Op::Hide => {}
                op => {
                    crate::password::apply(buf, op);
                }
            }
        }
        if done {
            let mut secret: String = self.secret.take().unwrap_or_default().into_iter().collect();
            self.disarm(Timer::Password);
            if let Some(i) = self.inter.as_mut() {
                i.phase = Phase::Acting;
            }
            let core = self.core.clone();
            tokio::spawn(async move {
                let r = match tools::atspi::type_text(&secret).await {
                    Ok(()) => Ok(()),
                    Err(_) => tools::input::type_text(&secret).await,
                };
                unsafe { secret.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
                let msg = match r {
                    Ok(()) => "I typed it. Say press enter to sign in.".to_owned(),
                    Err(e) => format!("I couldn't type it. {e}"),
                };
                core.tell(ControlMsg::Announce(msg));
            });
            self.finish("done");
            return;
        }
        let len = self.secret.as_ref().map(Vec::len).unwrap_or(0);
        if ops.is_empty() {
            speak = Some("I didn't catch any characters. Spell it, then say done.".into());
        }
        match speak {
            Some(t) => self.say(&t, "password"),
            None => {
                let _ = len;
                self.listen_for_password();
            }
        }
    }

    fn act(&mut self, inter: u64, tool: String, args: Value) {
        if tool == "ui.secret" {
            if let Some(i) = self.inter.as_mut() {
                i.record = false;
            }
            self.secret = Some(Vec::new());
            self.arm(Timer::Password, Duration::from_secs(90));
            self.emit_session();
            self.say("Spell your password. I won't show it or save it. Say done when you've finished.", "password");
            return;
        }
        let needs = needs_confirm(&tool, &args);
        if !needs {
            self.execute(Intent::Act { tool, args });
            return;
        }
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Thinking;
        }
        let core = self.core.clone();
        tokio::spawn(async move {
            let r = prepare(&core, &tool, args).await;
            let msg = match r {
                Ok((args, spec)) => ControlMsg::Prepared { inter, intent: Intent::Act { tool, args }, spec, err: None },
                Err(e) => ControlMsg::Prepared { inter, intent: Intent::Unknown, spec: None, err: Some(e) },
            };
            core.tell(msg);
        });
    }

    fn on_prepared(&mut self, inter: u64, intent: Intent, spec: Option<ConfirmSpec>, err: Option<String>) {
        if !self.inter.as_ref().is_some_and(|i| i.id == inter) {
            return;
        }
        if let Some(e) = err {
            self.problem(&e, None);
            return;
        }
        match spec {
            Some(spec) => self.confirm(spec, Some(Pending::Act(intent))),
            None => self.execute(intent),
        }
    }

    fn on_run_failed(&mut self, reply: String) {
        if !self.s().bool("code.checkOnRun") || self.inter.is_some() || self.prompt.is_some() || self.muted {
            return;
        }
        log::info!("the code Setu ran stopped with an error");
        self.disarm(Timer::Grace);
        self.quiet_interaction(Phase::Thinking, true, false);
        let id = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
        self.core.hub.emit("transcript.final", json!({ "interaction": id, "text": "Checking why it stopped", "language": self.core.lang() }));
        self.execute(Intent::Act { tool: "code.failed".into(), args: json!({ "reply": reply }) });
    }

    fn personalize(&self, text: &str) -> String {
        if text.is_empty() || !self.s().bool("privacy.learn") {
            return text.to_owned();
        }
        let t = text.to_owned();
        self.core
            .db
            .read(move |c| {
                let mut st = c.prepare("SELECT heard, meant FROM corrections ORDER BY time DESC LIMIT 200")?;
                let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
                for (heard, meant) in rows {
                    let h = normalize(&heard);
                    if !h.is_empty() && h.split_whitespace().count() <= 4 && t.contains(&h) && h != normalize(&meant) {
                        return Ok(t.replace(&h, &normalize(&meant)));
                    }
                }
                Ok(t)
            })
            .unwrap_or_else(|_| text.to_owned())
    }

    fn save_correction(&self, history: i64, meant: &str) {
        if !self.s().bool("privacy.learn") {
            return;
        }
        let meant = meant.to_owned();
        self.core.db.write(move |c| {
            let heard: String = c.query_row("SELECT said FROM history WHERE id = ?1", [history], |r| r.get(0)).unwrap_or_default();
            c.execute("INSERT INTO corrections(time, heard, meant, history_id) VALUES(?1, ?2, ?3, ?4)", rusqlite::params![now_ms(), heard, meant, history])?;
            Ok(())
        });
    }

    fn route(&mut self, inter: u64, text: String) {
        let mut text = text;
        if !self.in_plan && self.plan.is_empty() {
            let known = router::rules(&text, &self.s().str("voice.stopWord"), self.numbers_shown()).is_some();
            let steps = crate::plan::split(&normalize(&text), known);
            if steps.len() > 1 {
                self.in_plan = true;
                self.plan_total = steps.len();
                self.plan_answers.clear();
                self.plan = steps[1..].iter().cloned().collect();
                text = steps[0].clone();
                self.core.hub.emit("action.progress", json!({ "interaction": inter, "step": 1, "total": self.plan_total, "label": text::sentence_case(&text) }));
            }
        }
        if let Some(answer) = crate::memory::direct_answer(&self.core, &normalize(&text)) {
            if let Some(i) = self.inter.as_mut() {
                i.phase = Phase::Speaking;
            }
            self.say(&answer, "done");
            return;
        }
        let t0 = Instant::now();
        let numbers = self.numbers_shown();
        let open_it: &[(&str, u64)] = &[("open it", 1), ("open that", 1), ("open the first one", 1), ("open the first", 1), ("open the second one", 2), ("open the second", 2), ("open the third one", 3), ("open the third", 3), ("yes open it", 1)];
        let plain = normalize(&text);
        if let Some((_, n)) = open_it.iter().find(|(p, _)| *p == plain.as_str())
            && !self.core.last_paths.lock().unwrap().is_empty()
        {
            self.on_routed(inter, Intent::Act { tool: "files.open_last".into(), args: json!({ "n": n }) }, "rules", 0, None);
            return;
        }
        if let Some(intent) = router::rules(&text, &self.s().str("voice.stopWord"), numbers) {
            crate::layers::record("rules_router", t0.elapsed().as_micros() as u64);
            let ms = ms(t0.elapsed());
            self.core.metrics.record("rules_router", ms);
            let label = (Self::acts(&intent) && !self.in_plan && text::social(&text)).then(|| tools::describe_with(&self.core, &intent).2);
            self.on_routed(inter, intent, "rules", ms, None);
            if let Some(label) = label {
                self.small_talk(inter, label);
            }
            return;
        }
        let core = self.core.clone();
        tokio::spawn(async move {
            let t0 = Instant::now();
            let r = crate::pipeline::model_route(&core, inter, &text).await;
            let ms = ms(t0.elapsed());
            core.metrics.record("model_router", ms);
            core.tell(ControlMsg::Routed { inter, intent: r.intent, source: r.source, ms, notice: r.notice, say: r.say });
        });
    }

    fn answer_in_chat(&mut self, inter: u64, chat: i64, fallback: String, action: bool) {
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Thinking;
        }
        let core = self.core.clone();
        tokio::spawn(async move {
            let convo = crate::threads::context(&core, chat, 4);
            let asked = convo.lines().rev().find_map(|l| l.strip_prefix("User: ")).unwrap_or_default().to_owned();
            let recall = crate::memory::context(&core, &asked, Some(chat));
            let answer = match crate::pipeline::chat_answer(&core, inter, &convo, &recall, action).await {
                Ok(a) if !a.trim().is_empty() => a,
                _ => fallback,
            };
            let _ = core.control.send(ControlMsg::Req(Req::ChatAnswer { inter, text: answer })).await;
        });
    }

    fn intent_layer(intent: &Intent) -> &'static str {
        match intent {
            Intent::Act { tool, .. } => crate::skills::layer(tool),
            Intent::Numbers { .. } | Intent::TypeText { .. } => "accessibility",
            Intent::Install { .. } | Intent::Volume { .. } => "shell",
            _ => "registered",
        }
    }

    fn on_routed(&mut self, inter: u64, intent: Intent, source: &'static str, _ms: u64, notice: Option<String>) {
        if !self.inter.as_ref().is_some_and(|i| i.id == inter) {
            return;
        }
        self.understood = Some(Instant::now());
        if let Some(n) = notice {
            self.core.hub.emit("notice.problem", json!({ "interaction": inter, "message": n, "action": { "label": "Open Models", "method": "ui.open", "params": { "page": "settings/models" } } }));
        }
        if matches!(intent, Intent::Unknown) {
            if let Some(chat) = self.inter.as_ref().and_then(|i| i.chat) {
                self.answer_in_chat(inter, chat, "I can't do that yet.".into(), true);
                return;
            }
            self.problem("I can't do that yet.", None);
            return;
        }
        let intent = match intent {
            Intent::Act { tool, mut args } if tool == "memory.add" => {
                if let Some(said) = self.inter.as_ref().map(|i| i.said.clone())
                    && let Some(t) = args.get("text").and_then(Value::as_str).map(str::to_owned)
                {
                    args["text"] = json!(crate::memory::keep_case(&said, &t));
                }
                Intent::Act { tool, args }
            }
            other => other,
        };
        let mut names = crate::wakeadapt::variants(self.s());
        names.insert(0, self.s().name());
        let intent = match intent {
            Intent::OpenApp { target, role } => match router::self_page(&target, &names) {
                Some(page) => Intent::Internal { kind: "ui.open", args: json!({ "page": page }) },
                None => Intent::OpenApp { target, role },
            },
            Intent::CloseApp { target } if router::self_page(&target, &names).is_some() || (target.is_empty() && self.windows > 0) => Intent::Internal { kind: "ui.close", args: Value::Null },
            Intent::CloseApp { target } if target.is_empty() => Intent::Act { tool: "wm.close".into(), args: json!({}) },
            other => other,
        };
        self.core.hub.emit(
            "intent.resolved",
            json!({ "interaction": inter, "action": intent.action(), "target": intent.target(), "arguments": intent.args(), "layer": Self::intent_layer(&intent), "source": source }),
        );
        if sensitive(&intent)
            && crate::speaker::active(&self.core)
            && self.inter.as_ref().is_some_and(|i| i.spoken && !i.voice.is_some_and(|s| s >= crate::speaker::ACCEPT_STRICT))
        {
            log::info!("a sensitive request was held back: the voice match was not strict enough");
            self.say("For that I need to be sure it's you. Say it again, a little longer, in your normal voice.", "done");
            return;
        }
        match intent {
            Intent::Cancel => {
                let paused = crate::jobs::halt_all();
                self.cancel_all(Some(if paused > 0 { "Cancelled. Downloads are paused; say resume the download to carry on." } else { "Cancelled." }), true);
            }
            Intent::Sleep => self.go_dormant("asked", true),
            Intent::Repeat => match self.last_reply.clone() {
                Some(r) => self.say(&r, "done"),
                None => self.say("I haven't said anything yet.", "done"),
            },
            Intent::Undo => match self.undo.back().map(|u| u.0.clone()) {
                Some(id) => {
                    let r = self.do_undo(&id);
                    match r {
                        Ok(m) => self.say(&m, "done"),
                        Err(e) => self.problem(&e.message, None),
                    }
                }
                None => self.problem("There is nothing to undo.", None),
            },
            Intent::Correct { text } => {
                if let Some((heard, Some(h))) = self.last_said.clone()
                    && self.s().bool("privacy.learn") {
                        let meant = text.clone();
                        self.core.db.write(move |c| {
                            c.execute("INSERT INTO corrections(time, heard, meant, history_id) VALUES(?1, ?2, ?3, ?4)", rusqlite::params![now_ms(), heard, meant, h])?;
                            Ok(())
                        });
                    }
                if let Some(i) = self.inter.as_mut() {
                    i.said = text::sentence_case(&text);
                }
                self.route(inter, text);
            }
            Intent::Ask { question, options } => self.ask(&question, options, Purpose::Options),
            Intent::Internal { kind: "ui.open", args } => self.open_ui(args["page"].as_str().unwrap_or("home").to_owned()),
            Intent::Internal { kind: "ui.close", .. } => self.close_ui(),
            Intent::OpenApp { target, role } => self.open_app(&target, role),
            Intent::CloseApp { target } => self.close_app(&target),
            Intent::Install { app } => self.install(inter, &app),
            Intent::Wifi { action, network } => self.wifi_intent(action, network),
            Intent::Act { tool, args } => self.act(inter, tool, args),
            Intent::ScanQr => self.open_camera(),
            Intent::Numbers { action: "show", .. } => self.show_numbers(),
            Intent::Numbers { action: "hide", .. } => {
                self.hide_numbers();
                if crate::browse::PAGE_NUMBERS.swap(false, Ordering::Relaxed) {
                    self.run_quietly(Intent::Act { tool: "browser.page".into(), args: json!({ "action": "numbers.clear" }) });
                }
                self.finish("done");
            }
            Intent::Numbers { n: Some(n), .. } => self.click_number(n.max(0) as usize),
            Intent::MuteMic { minutes } => {
                self.finish("done");
                self.set_muted(true, minutes.map(|m| m as f64));
            }
            Intent::Reply { text } => match self.inter.as_ref().and_then(|i| i.chat.map(|c| (i.id, c))) {
                Some((inter, chat)) => self.answer_in_chat(inter, chat, text, false),
                None => self.reply(text),
            },
            Intent::Time => self.say(&tools::system::time_sentence(), "done"),
            Intent::Date => self.say(&tools::system::date_sentence(), "done"),
            other => self.execute(other),
        }
    }

    fn execute(&mut self, intent: Intent) {
        let (verb, target, label) = tools::describe_with(&self.core, &intent);
        let narrate = narration(&intent, &label);
        let speak = self.s().bool("tts.speakReplies");
        let Some(i) = self.inter.as_mut() else { return };
        crate::os::set_typed(i.silent);
        let action = format!("a-{}-{}", i.id, now_ms() % 100_000);
        i.phase = Phase::Acting;
        i.action = Some(action.clone());
        let cancel = i.cancel.clone();
        let inter = i.id;
        let started = i.started;
        self.core.metrics.record("action_start", ms(started.elapsed()));
        self.core.hub.emit(
            "action.started",
            json!({ "id": action, "interaction": inter, "verb": verb, "target": target, "label": label, "step": null, "total": null, "cancellable": tools::cancellable(&intent) }),
        );
        if let Some((line, cue)) = narrate {
            sounds::play(&self.core, cue);
            if speak && i.tts.is_none() {
                let id = (now_ms() as u64) ^ (inter << 24) ^ 0x4E41_5252;
                i.tts = Some(id);
                i.narrated = true;
                let core = self.core.clone();
                tokio::spawn(async move {
                    if crate::pipeline::speak(&core, id, vec![line]).await.is_err() {
                        let _ = core.control.send(ControlMsg::Engine(EngineEvent::TtsDone { id, first_audio_ms: 0 })).await;
                    }
                });
            }
        } else if let Some(cue) = quiet_cue(&intent) {
            sounds::play(&self.core, cue);
        }
        self.listen();
        let core = self.core.clone();
        let trace = crate::layers::Trace::new(self.understood.take().unwrap_or_else(Instant::now));
        tokio::spawn(async move {
            let t0 = Instant::now();
            trace.begun();
            let limit = match &intent {
                Intent::Act { tool, .. } if tool.starts_with("face.") || tool.starts_with("vault.") => 150,
                _ => 60,
            };
            let fut = crate::layers::scope(trace.clone(), tools::run(&core, intent.clone(), cancel));
            let outcome = match tokio::time::timeout(Duration::from_secs(limit), fut).await {
                Ok(o) => o,
                Err(_) => Outcome::fail("That action didn't respond.", "registered"),
            };
            core.tell(ControlMsg::Finished { inter, action, intent: Box::new(intent), outcome: Box::new(outcome), ms: ms(t0.elapsed()), trace: Box::new(trace.report()) });
        });
    }

    fn on_finished(&mut self, inter: u64, action: String, intent: Intent, outcome: Outcome, dur: u64, trace: crate::layers::Report) {
        let result = if outcome.ok { "success" } else if outcome.cancelled { "cancelled" } else { "failed" };
        if let Some(us) = trace.sent_us {
            crate::layers::record("action_sent", us);
        }
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            crate::tools::warm().await;
        });
        if outcome.ok
            && outcome.app.as_deref().is_some_and(crate::os::is_setu)
            && matches!(&intent, Intent::Act { tool, .. } if tool.starts_with("ui.") || tool == "browser.tab.activate")
            && let Some(i) = self.inter.as_mut().filter(|i| i.id == inter && i.chat.is_none())
        {
            i.record = false;
            self.core.threads.take(inter);
        }
        self.core.hub.emit(
            "action.finished",
            json!({ "id": action, "interaction": inter, "result": result, "message": outcome.message, "undoable": outcome.undo.is_some(), "durationMs": dur, "layer": trace.served.map(crate::layers::Layer::as_str), "trace": trace.json() }),
        );
        let said = self.inter.as_ref().filter(|i| i.id == inter).map(|i| (i.said.clone(), i.record, i.started));
        let hist = if let Some((s, record, started)) = said.clone() {
            if record {
                self.core.metrics.record("total", ms(started.elapsed()));
                self.record(&s, &outcome, result, &intent, &action, dur)
            } else {
                None
            }
        } else {
            None
        };
        if outcome.ok && matches!(&intent, Intent::Act { tool, .. } if tool == "code.dictate") {
            self.start_dictation();
        }
        if outcome.ok && matches!(&intent, Intent::Act { tool, .. } if tool == "text.dictate") {
            self.start_dictation();
            self.plain = outcome.did != "Coding mode on";
            self.dict_chunks.clear();
            self.dict_last = None;
            self.emit_session();
        }
        if outcome.ok
            && let Intent::Act { tool, args } = &intent
            && tool == "voice.learn"
        {
            self.pending_style = args.get("style").and_then(Value::as_str).map(str::to_owned);
        }
        if let Some(u) = outcome.undo.clone() {
            self.undo.push_back((action.clone(), u, outcome.did.clone()));
            while self.undo.len() > 20 {
                self.undo.pop_front();
            }
            self.sync_undoable();
        }
        self.last_said = said.map(|(s, _, _)| (s, hist));
        if !self.inter.as_ref().is_some_and(|i| i.id == inter) {
            return;
        }
        let (narrated, talking) = self.inter.as_ref().map(|i| (i.narrated, i.tts.is_some())).unwrap_or((false, false));
        if outcome.ok
            && outcome.reply.is_none()
            && outcome.ask.is_none()
            && let Some(i) = self.inter.as_mut()
        {
            let did = sentence(&outcome.did);
            i.answer = Some(match i.answer.take().filter(|_| i.told) {
                Some(said) => format!("{said} {did}"),
                None => did,
            });
        }
        if outcome.ok {
            if let Some(f) = outcome.ask.clone() {
                self.follow_up(*f);
                return;
            }
            match outcome.reply.clone() {
                Some(r) => {
                    if !self.holding(inter, Some(r.clone())) {
                        self.say(&r, "done")
                    }
                }
                None => {
                    if !narrated && quiet_cue(&intent).is_none() {
                        sounds::play(&self.core, Cue::Done);
                    }
                    if talking {
                        if let Some(i) = self.inter.as_mut() {
                            i.phase = Phase::Speaking;
                        }
                        return;
                    }
                    if self.holding(inter, None) {
                        return;
                    }
                    self.finish("done")
                }
            }
        } else if outcome.cancelled {
            self.finish("cancelled");
        } else {
            if let Some(i) = self.inter.as_mut() {
                i.aside = false;
            }
            if narrated && talking {
                self.core.engines.tts.send(&ToEngine::TtsStop);
                if let Some(i) = self.inter.as_mut() {
                    i.tts = None;
                }
            }
            if let Some(item) = outcome.need.filter(|n| *n != "extension")
                && self.ask_permission(item, intent)
            {
                return;
            }
            self.problem(&outcome.message, outcome.fix.clone());
        }
    }

    fn record(&self, said: &str, o: &Outcome, result: &str, intent: &Intent, action: &str, dur: u64) -> Option<i64> {
        if !self.s().bool("history.keep") {
            return None;
        }
        let row = crate::storage::HistoryRow {
            time: now_ms(),
            said: said.to_owned(),
            did: o.did.clone(),
            result: result.to_owned(),
            layer: o.layer.to_owned(),
            duration_ms: dur as i64,
            app: o.app.clone(),
            action: Some(json!({ "id": action, "action": intent.action(), "target": intent.target(), "arguments": intent.args() })),
            undoable: o.undo.is_some(),
        };
        let r = row.clone();
        let id = self.core.db.write_sync(move |c| Ok(crate::storage::insert_history(c, &r)?)).ok()?;
        self.core.hub.emit(
            "history.added",
            json!({ "id": id.to_string(), "time": row.time, "said": row.said, "did": row.did, "result": row.result, "layer": row.layer, "durationMs": row.duration_ms, "app": row.app, "action": row.action, "undoable": row.undoable }),
        );
        Some(id)
    }

    fn say(&mut self, text: &str, after: &'static str) {
        let Some(i) = self.inter.as_mut() else { return };
        let lines = text::lines(text);
        if matches!(after, "done" | "problem") {
            i.answer = Some(text.to_owned());
        }
        let speak = self.core.settings.bool("tts.speakReplies") && !i.silent;
        let id = now_ms() as u64;
        i.phase = Phase::Speaking;
        let inter = i.id;
        self.last_reply = Some(text.to_owned());
        let name = normalize(&self.core.settings.name());
        self.echo_name = speak && normalize(text).contains(&name);
        self.core.hub.emit("reply.speak", json!({ "id": id.to_string(), "interaction": inter, "text": text, "lines": lines, "spoken": speak }));
        if speak {
            i.tts = Some(id);
            self.listen();
            let core = self.core.clone();
            let lines2 = lines.clone();
            tokio::spawn(async move {
                if crate::pipeline::speak(&core, id, lines2).await.is_err() {
                    let _ = core.control.send(ControlMsg::Engine(EngineEvent::TtsDone { id, first_audio_ms: 0 })).await;
                }
            });
        } else {
            self.core.hub.emit("reply.done", json!({ "id": id.to_string() }));
            self.after_speech(after);
        }
    }

    fn after_speech(&mut self, after: &'static str) {
        if self.prompt.is_some() {
            if let Some(p) = &self.prompt
                && matches!(p.kind, PromptKind::Ask { .. } | PromptKind::Confirm { voice: true, .. }) {
                    self.listen_for_answer();
                    return;
                }
            self.listen();
            return;
        }
        if (self.wifi.as_ref().is_some_and(|w| w.ssid.is_some()) || self.secret.is_some()) && self.inter.as_ref().is_some_and(|i| !i.record) {
            self.listen_for_password();
            return;
        }
        if self.camera.is_some() || self.numbers.is_some() {
            if let Some(i) = self.inter.as_mut() {
                i.phase = Phase::Acting;
            }
            self.listen();
            return;
        }
        self.finish(after);
    }

    fn finish(&mut self, outcome: &str) {
        if outcome == "done"
            && self.prompt.is_none()
            && self.inter.is_some()
            && let Some(next) = self.plan.pop_front()
        {
            let step = self.plan_total - self.plan.len();
            let i = self.inter.as_mut().unwrap();
            if let Some(a) = i.answer.take() {
                self.plan_answers.push(a);
            }
            i.phase = Phase::Thinking;
            i.action = None;
            i.tts = None;
            i.narrated = false;
            let id = i.id;
            self.core.hub.emit("action.progress", json!({ "interaction": id, "step": step, "total": self.plan_total, "label": text::sentence_case(&next) }));
            self.route(id, next);
            return;
        }
        if self.in_plan {
            let mut all = std::mem::take(&mut self.plan_answers);
            if let Some(i) = self.inter.as_mut() {
                if let Some(a) = i.answer.take() {
                    all.push(a);
                }
                if !all.is_empty() {
                    i.answer = Some(all.join(" "));
                }
            }
        }
        self.plan.clear();
        self.in_plan = false;
        let Some(i) = self.inter.take() else {
            self.enter_active("done");
            return;
        };
        if i.record || i.chat.is_some() {
            let answer = i.answer.clone().unwrap_or_else(|| match outcome {
                "cancelled" => "Cancelled.".into(),
                _ => String::new(),
            });
            if !i.said.trim().is_empty() || i.chat.is_some() || (i.carried && !answer.trim().is_empty()) {
                crate::threads::record_turn(&self.core, i.id, &i.said, &answer, i.logged);
            }
        }
        self.core.hub.emit("interaction.ended", json!({ "interaction": i.id, "outcome": outcome }));
        self.enter_grace(i.id);
    }

    fn enter_grace(&mut self, id: u64) {
        self.inter = None;
        self.grace_id = Some(id);
        if let Some(style) = self.pending_style.take() {
            self.enroll_style = Some(style);
            self.enroll = Some(Vec::new());
            self.arm(Timer::Style, Duration::from_secs(25));
        }
        self.core.downloads.hold.store(false, Ordering::Relaxed);
        self.core.downloads.wake();
        self.set_session(Session::Grace, if self.dictation { "coding" } else { "grace" });
        let secs = if self.dictation { (self.s().get("session.activeMinutes").as_i64().unwrap_or(10).max(1) * 60) as f64 } else { self.s().f64("session.graceSeconds").max(1.0) };
        self.arm(Timer::Grace, Duration::from_secs_f64(secs));
        self.listen();
        self.schedule_unload();
    }

    fn enter_active(&mut self, reason: &str) {
        self.core.downloads.hold.store(false, Ordering::Relaxed);
        self.core.downloads.wake();
        self.inter = None;
        self.grace_id = None;
        let mins = self.s().get("session.activeMinutes").as_i64().unwrap_or(10).max(1) as u64;
        self.session = Session::Active;
        Self::unduck();
        self.reason = reason.into();
        self.active_until = Some(now_ms() + (mins as i64) * 60_000);
        self.arm(Timer::Active, Duration::from_secs(mins * 60));
        self.emit_session();
        self.listen();
        self.schedule_unload();
    }

    fn go_dormant(&mut self, reason: &str, announce: bool) {
        self.disarm(Timer::Grace);
        self.disarm(Timer::Active);
        if announce {
            let phrase = self.s().phrase();
            let line = format!("Going to sleep. Say {phrase} to wake me.");
            self.quiet_interaction(Phase::Speaking, false, false);
            self.say(&line, "sleep");
        }
        self.inter = None;
        self.session = Session::Dormant;
        Self::unduck();
        self.reason = reason.into();
        self.active_until = None;
        self.core.downloads.hold.store(false, Ordering::Relaxed);
        self.core.downloads.wake();
        self.emit_session();
        self.listen();
    }

    fn problem(&mut self, message: &str, action: Option<Value>) {
        sounds::play(&self.core, Cue::Error);
        if let Some(i) = self.inter.as_mut() {
            i.answer = Some(message.to_owned());
        }
        let inter = self.inter.as_ref().map(|i| i.id);
        self.core.hub.emit("notice.problem", json!({ "interaction": inter, "message": message, "action": action }));
        if self.s().bool("tts.speakReplies") && self.inter.is_some() {
            self.say(message, "problem");
        } else {
            self.finish("problem");
        }
    }

    fn cancel_all(&mut self, message: Option<&str>, announce: bool) -> (Vec<String>, Vec<String>) {
        if announce && (self.inter.is_some() || self.prompt.is_some()) {
            sounds::play(&self.core, Cue::Cancel);
        }
        self.stream = None;
        self.plan.clear();
        self.plan_answers.clear();
        self.in_plan = false;
        if let Some(mut sec) = self.secret.take() {
            sec.iter_mut().for_each(|c| *c = '\0');
        }
        crate::tools::pointer::release_all();
        let mut cancelled = Vec::new();
        let mut not = Vec::new();
        self.core.engines.tts.send(&ToEngine::TtsStop);
        if let Some(p) = self.prompt.take() {
            self.cleanup_prompt(&p);
            self.disarm(Timer::Prompt);
            self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "cancelled", "message": message }));
        }
        if self.wifi.take().is_some() {
            self.disarm(Timer::Password);
            self.core.hub.emit("wifi.state", json!({ "phase": "closed", "networks": [], "ssid": null, "length": 0, "revealed": null, "message": null }));
        }
        if let Some(i) = self.inter.take() {
            i.cancel.store(true, Ordering::Relaxed);
            if i.asr != 0 {
                self.core.feed.close();
                self.core.engines.asr.send(&ToEngine::AsrCancel { id: i.asr });
            }
            self.core.engines.llm.send(&ToEngine::LlmCancel { id: i.id });
            if let Some(a) = &i.action {
                cancelled.push(a.clone());
            }
            self.core.hub.emit("interaction.ended", json!({ "interaction": i.id, "outcome": "cancelled" }));
            if announce
                && let Some(m) = message {
                    self.core.hub.emit("reply.speak", json!({ "id": now_ms().to_string(), "interaction": i.id, "text": m, "lines": [m], "spoken": false }));
                }
            let _ = &mut not;
            self.enter_grace(i.id);
        } else if self.session == Session::Capturing {
            self.enter_active("cancelled");
        }
        self.close_camera(None);
        self.hide_numbers();
        (cancelled, not)
    }

    fn ask(&mut self, question: &str, options: Vec<String>, purpose: Purpose) {
        self.ask_spoken(question, options, purpose, None, (Vec::new(), ""), None);
    }

    fn follow_up(&mut self, f: tools::FollowUp) {
        let labels: Vec<String> = f.choices.iter().map(|c| c.label.clone()).collect();
        let icons: Vec<String> = f.choices.iter().map(|c| c.icon.clone()).collect();
        self.ask_spoken(&f.question, labels, Purpose::Then { choices: f.choices, cleanup: f.cleanup }, f.spoken, (icons, f.kind), f.focus);
    }

    fn cleanup_prompt(&self, p: &Prompt) {
        if let PromptKind::Ask { purpose: Purpose::Then { cleanup: Some(intent), .. }, .. } = &p.kind {
            self.run_quietly(intent.clone());
        }
    }

    fn run_quietly(&self, intent: Intent) {
        let core = self.core.clone();
        tokio::spawn(async move {
            let _ = tools::run(&core, intent, Arc::new(AtomicBool::new(false))).await;
        });
    }

    fn ask_spoken(&mut self, question: &str, options: Vec<String>, purpose: Purpose, spoken_line: Option<String>, look: (Vec<String>, &'static str), focus: Option<usize>) {
        let id = format!("p-{}", now_ms());
        self.understood = None;
        let Some(i) = self.inter.as_mut() else { return };
        i.phase = Phase::Asking;
        let inter = i.id;
        let (icons, kind) = look;
        let opts: Vec<Value> = options.iter().enumerate().map(|(k, l)| json!({ "n": k + 1, "label": l, "icon": icons.get(k).cloned().unwrap_or_default() })).collect();
        let timeout = self.scaled(Duration::from_secs(if kind == "browsers" { 45 } else { 20 }));
        self.core.hub.emit(
            "prompt.ask",
            json!({ "id": id, "interaction": inter, "question": question, "options": opts, "kind": kind, "focus": focus, "timeoutMs": timeout.map(|d| d.as_millis() as u64) }),
        );
        self.prompt = Some(Prompt { id, options: options.clone(), kind: PromptKind::Ask { purpose, retried: false }, look: kind, focus });
        if let Some(t) = timeout {
            self.arm(Timer::Prompt, t);
        }
        let spoken: Vec<String> = options.iter().enumerate().map(|(k, o)| format!("{}, {}", k + 1, o)).collect();
        let full = match spoken_line {
            Some(line) => line,
            None if options.len() > 1 => format!("{question} {}.", spoken.join(". ")),
            None => question.to_owned(),
        };
        self.say(&full, "ask");
    }

    fn listen_for_answer(&mut self) {
        let asr = self.asr_next;
        self.asr_next += 1;
        if let Some(i) = self.inter.as_mut() {
            i.asr = asr;
            i.phase = Phase::Listening;
        }
        self.set_session(Session::Capturing, "answer");
        self.start_capture(asr, 0.2);
    }

    fn confirm(&mut self, spec: ConfirmSpec, pending: Option<Pending>) {
        self.understood = None;
        let id = format!("c-{}", now_ms());
        let woke = self.inter.as_ref().is_some_and(|i| i.woke);
        let method = self.s().str("confirm.method");
        let voice = woke && (method == "voice" || method == "any");
        self.quiet_interaction(Phase::Confirming, false, true);
        let i = self.inter.as_mut().unwrap();
        i.phase = Phase::Confirming;
        let inter = i.id;
        let timeout = self.scaled(Duration::from_secs(20));
        self.core.hub.emit(
            "prompt.confirm",
            json!({
                "id": id, "interaction": inter, "title": spec.title, "facts": spec.facts, "consequence": spec.consequence,
                "risk": spec.risk, "actionLabel": spec.action_label, "danger": spec.danger,
                "timeoutMs": timeout.map(|d| d.as_millis() as u64), "voiceAllowed": voice,
                "origin": if woke || self.windows == 0 { "voice" } else { "window" }
            }),
        );
        self.prompt = Some(Prompt { id, options: vec![], kind: PromptKind::Confirm { pending, voice }, look: "", focus: None });
        if let Some(t) = timeout {
            self.arm(Timer::Prompt, t);
        }
        if self.s().bool("tts.readConfirmations") {
            let tail = if voice { " Say yes or no." } else { "" };
            let mut line = spec.title.clone();
            if !line.ends_with(['.', '?', '!']) {
                line.push('.');
            }
            if !spec.consequence.is_empty() {
                line.push(' ');
                line.push_str(&spec.consequence);
            }
            line.push_str(tail);
            self.say(&line, "confirm");
        } else {
            self.listen();
        }
    }

    fn close_prompt(&mut self, outcome: &str, message: Option<&str>) {
        if let Some(p) = self.prompt.take() {
            self.cleanup_prompt(&p);
            self.disarm(Timer::Prompt);
            self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": outcome, "message": message }));
        }
    }

    fn answer(&mut self, id: &str, answer: &Value, via: &str) -> Result<(), ApiError> {
        let p = self.prompt.as_ref().ok_or_else(|| ApiError::not_found("There is no question waiting for an answer."))?;
        if p.id != id {
            return Err(ApiError::not_found("That question has already closed."));
        }
        match &p.kind {
            PromptKind::Confirm { .. } => {
                let yes = match answer {
                    Value::String(s) => matches!(s.as_str(), "yes" | "confirm"),
                    Value::Number(n) => n.as_i64() == Some(1),
                    _ => false,
                };
                if yes {
                    let method = self.s().str("confirm.method");
                    let allowed = match via {
                        "voice" => matches!(p.kind, PromptKind::Confirm { voice: true, .. }),
                        "key" | "click" => method == "key" || method == "any" || method == "voice",
                        "switch" => method == "switch" || method == "any",
                        _ => false,
                    };
                    if !allowed {
                        return Err(ApiError::invalid("That way of confirming is turned off in Accessibility settings."));
                    }
                }
                let p = self.prompt.take().unwrap();
                self.disarm(Timer::Prompt);
                self.core.engines.tts.send(&ToEngine::TtsStop);
                self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "answered", "message": null }));
                let PromptKind::Confirm { pending, .. } = p.kind else { unreachable!() };
                if yes {
                    self.run_pending(pending);
                } else {
                    if let Some(Pending::WifiQr { mut password, .. }) = pending {
                        password.iter_mut().for_each(|c| *c = '\0');
                    }
                    self.say("Cancelled. Nothing was changed.", "cancelled");
                }
                Ok(())
            }
            PromptKind::Ask { .. } => {
                let text = match answer {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                };
                let t = normalize(&text);
                if p.look.starts_with("look") {
                    let stop = matches!(t.as_str(), "cancel" | "stop" | "never mind" | "nevermind" | "close" | "close it" | "close the window" | "forget it") || t == self.s().str("voice.stopWord");
                    let then = match (p.look, stop) {
                        ("look-import", true) => match &p.kind {
                            PromptKind::Ask { purpose: Purpose::Then { choices, .. }, .. } => choices.last().map(|c| c.then.clone()),
                            _ => None,
                        },
                        (_, true) => Some(Intent::Act { tool: "look.step".into(), args: json!({ "op": "cancel" }) }),
                        ("look-name", false) if matches!(t.as_str(), "back" | "go back" | "change the picture") => Some(Intent::Act { tool: "look.step".into(), args: json!({ "op": "back" }) }),
                        ("look-name", false) => Some(Intent::Act { tool: "look.step".into(), args: json!({ "op": "name", "value": text }) }),
                        _ => None,
                    };
                    if let Some(intent) = then {
                        self.prompt.take();
                        self.disarm(Timer::Prompt);
                        self.core.engines.tts.send(&ToEngine::TtsStop);
                        self.core.hub.emit("prompt.closed", json!({ "id": id, "outcome": "answered", "message": null }));
                        self.execute(intent);
                        return Ok(());
                    }
                    let worded = match &p.kind {
                        PromptKind::Ask { purpose: Purpose::Then { choices, .. }, .. } => pick_choice(&t, choices),
                        _ => None,
                    };
                    let Some(k) = worded else {
                        let hint = match p.look {
                            "look" => "Say next, previous, a picture's name, or this one.",
                            "look-confirm" => "Say yes to create it, or change the name.",
                            _ => "Say which profiles to bring over, or start empty.",
                        };
                        self.say(hint, "ask");
                        return Ok(());
                    };
                    let p = self.prompt.take().unwrap();
                    self.disarm(Timer::Prompt);
                    self.core.engines.tts.send(&ToEngine::TtsStop);
                    self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "answered", "message": null }));
                    let choice = p.options.get(k - 1).cloned().unwrap_or_default();
                    let PromptKind::Ask { purpose, .. } = p.kind else { unreachable!() };
                    self.on_choice(purpose, k, choice);
                    return Ok(());
                }
                if matches!(t.as_str(), "cancel" | "none" | "neither" | "stop" | "never mind" | "no") || t == self.s().str("voice.stopWord") {
                    self.close_prompt("cancelled", Some("Cancelled."));
                    self.say("Cancelled.", "cancelled");
                    return Ok(());
                }
                let n = p.options.len();
                if (p.look == "profiles" || p.look == "folders")
                    && let Some(f) = p.focus
                {
                    let step: Option<i64> = match t.as_str() {
                        "back" if p.look == "folders" => None,
                        "left" | "go left" | "move left" | "previous" | "previous one" | "back" | "the left one" | "one left" | "up" => Some(-1),
                        "right" | "go right" | "move right" | "next" | "next one" | "the right one" | "one right" | "down" => Some(1),
                        _ => None,
                    };
                    if let Some(d) = step {
                        let nf = (f as i64 + d).clamp(0, n as i64 - 1) as usize;
                        let id = p.id.clone();
                        let label = p.options.get(nf).cloned().unwrap_or_default();
                        if let Some(pm) = self.prompt.as_mut() {
                            pm.focus = Some(nf);
                        }
                        self.core.hub.emit("prompt.update", json!({ "id": id, "focus": nf }));
                        self.say(&label, "ask");
                        return Ok(());
                    }
                    if matches!(t.as_str(), "this one" | "this" | "that one" | "open it" | "open this" | "open this one" | "yes" | "okay" | "ok" | "select" | "select this" | "choose this" | "open" | "this profile" | "go") {
                        let p = self.prompt.take().unwrap();
                        self.disarm(Timer::Prompt);
                        self.core.engines.tts.send(&ToEngine::TtsStop);
                        self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "answered", "message": null }));
                        let PromptKind::Ask { purpose, .. } = p.kind else { unreachable!() };
                        let choice = p.options.get(f).cloned().unwrap_or_default();
                        self.on_choice(purpose, f + 1, choice);
                        return Ok(());
                    }
                }
                let worded = match &p.kind {
                    PromptKind::Ask { purpose: Purpose::Then { choices, .. }, .. } if !numeric_answer(&t) => pick_choice(&t, choices),
                    _ => None,
                };
                let pick = worded.or_else(|| {
                    text::find_number(&t)
                        .map(|v| v as usize)
                        .or_else(|| ordinal(&t))
                        .filter(|v| (1..=n).contains(v))
                        .or_else(|| {
                            let best = p.options.iter().enumerate().map(|(k, o)| (k + 1, text::similarity(&t, o))).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                            best.filter(|(_, s)| *s >= 0.8).map(|(k, _)| k)
                        })
                });
                let Some(k) = pick else {
                    if let Some(Prompt { kind: PromptKind::Ask { retried, .. }, .. }) = self.prompt.as_mut()
                        && !*retried {
                            *retried = true;
                            self.say("Say the number of your choice, or say cancel.", "ask");
                            return Ok(());
                        }
                    self.close_prompt("cancelled", Some("Cancelled."));
                    self.say("Cancelled.", "cancelled");
                    return Ok(());
                };
                let p = self.prompt.take().unwrap();
                self.disarm(Timer::Prompt);
                self.core.engines.tts.send(&ToEngine::TtsStop);
                self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "answered", "message": null }));
                let choice = p.options[k - 1].clone();
                let PromptKind::Ask { purpose, .. } = p.kind else { unreachable!() };
                self.on_choice(purpose, k, choice);
                Ok(())
            }
        }
    }

    fn on_choice(&mut self, purpose: Purpose, k: usize, choice: String) {
        match purpose {
            Purpose::App { verb, choices } => {
                if self.s().bool("privacy.learn") {
                    let key = format!("choice:{}", choices.join("|"));
                    let val = choice.clone();
                    self.core.db.write(move |c| {
                        crate::storage::pref_bump(c, &key, &val)?;
                        Ok(())
                    });
                }
                let app = self.core.apps.all().into_iter().find(|a| a.name == choice);
                match (verb, app) {
                    ("open", Some(a)) => self.execute(Intent::OpenApp { target: a.id, role: None }),
                    ("close", Some(a)) => self.execute(Intent::CloseApp { target: a.id }),
                    _ => self.problem("That app is no longer available.", None),
                }
            }
            Purpose::Missing { app } => match k {
                1 => {
                    let inter = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
                    self.install(inter, &app)
                }
                2 => {
                    let names: Vec<String> = self.core.apps.all().into_iter().filter(|a| !a.hidden).take(4).map(|a| a.name).collect();
                    if names.len() >= 2 {
                        self.ask("Which app?", names.clone(), Purpose::App { verb: "open", choices: names });
                    } else {
                        self.problem("No other apps were found.", None);
                    }
                }
                _ => self.say("Cancelled.", "cancelled"),
            },
            Purpose::Networks => {
                if let Some(w) = self.wifi.as_ref()
                    && let Some(net) = w.networks.get(k - 1).cloned() {
                        self.wifi_connect(net);
                    }
            }
            Purpose::Options => self.route(self.inter.as_ref().map(|i| i.id).unwrap_or(0), format!("open {choice}")),
            Purpose::Then { mut choices, .. } => {
                if k >= 1 && k <= choices.len() {
                    let c = choices.swap_remove(k - 1);
                    self.execute(c.then);
                }
            }
        }
    }

    fn run_pending(&mut self, p: Option<Pending>) {
        match p {
            Some(Pending::Install(pkg)) => self.execute(Intent::Internal { kind: "install_package", args: serde_json::to_value(&pkg).unwrap_or(Value::Null) }),
            Some(Pending::Url(u)) => self.execute(Intent::Internal { kind: "open_url", args: json!({ "url": u }) }),
            Some(Pending::WifiQr { ssid, password }) => {
                let net = tools::wifi::Network { n: 1, ssid: ssid.clone(), secure: !password.is_empty(), saved: false, signal: 0, enterprise: false, path: String::new() };
                self.wifi = Some(WifiFlow { networks: vec![net.clone()], ssid: Some(ssid), password, revealed: false });
                self.connect_now();
            }
            Some(Pending::External(f)) => {
                f(self.core.clone());
                self.finish("done");
            }
            Some(Pending::Act(intent)) => self.execute(intent),
            Some(Pending::Grant { item, then }) => {
                self.grant(item);
                if matches!(then, Intent::Numbers { .. }) {
                    self.show_numbers();
                } else {
                    self.execute(then);
                }
            }
            Some(Pending::OpenUi(page)) => match tools::launch_ui(&self.core, &page) {
                Ok(()) => self.say("Opening the Setu app.", "done"),
                Err(e) => {
                    log::warn!("the window could not be started: {e}");
                    self.problem("I couldn't open the Setu app.", None);
                }
            },
            None => self.finish("done"),
        }
    }

    fn open_ui(&mut self, page: String) {
        if self.windows > 0 {
            self.core.hub.emit("ui.command", json!({ "command": "raise", "page": page }));
            self.say("The Setu app is already open.", "done");
            return;
        }
        let spec = ConfirmSpec {
            title: "Should I open the Setu app?".into(),
            facts: vec![],
            consequence: String::new(),
            risk: "low",
            action_label: "Open".into(),
            danger: false,
        };
        self.confirm(spec, Some(Pending::OpenUi(page)));
    }

    fn close_ui(&mut self) {
        if self.windows == 0 {
            self.say("The Setu app isn't open. I'm still listening.", "done");
            return;
        }
        self.core.hub.emit("ui.command", json!({ "command": "close" }));
        self.say("Closed the Setu app. I'm still listening.", "done");
    }

    fn echoing(&self) -> bool {
        self.echo_name || self.echo_until.is_some_and(|t| Instant::now() < t)
    }

    fn announce(&mut self, text: String) {
        let tts_ready = self.voice_ready();
        let rate = if tts_ready { self.s().f64("tts.rate").clamp(0.5, 2.0) } else { 0.9 };
        let est = (text.chars().count() as f64 * 65.0 / rate) as u64;
        self.echo_until = Some(Instant::now() + Duration::from_millis(est + 1500));
        let core = self.core.clone();
        if tts_ready {
            tokio::spawn(async move {
                if let Err(e) = crate::pipeline::sample(&core, None, None, None, text).await {
                    log::warn!("announcement failed: {e}");
                }
            });
        } else {
            std::thread::spawn(move || tools::fallback_speak(&core, &text));
        }
    }

    fn open_app(&mut self, target: &str, role: Option<&'static str>) {
        let defaults = self.core.defaults();
        if let Some(r) = role
            && let Some(app) = self.core.apps.role_default(r, &defaults) {
                self.execute(Intent::OpenApp { target: app.id, role: None });
                return;
            }
        let found = self.core.apps.find(target, None, &defaults);
        if found.is_empty() {
            if let Some(site) = tools::web::site(target) {
                self.act(self.inter.as_ref().map(|i| i.id).unwrap_or(0), "web.open".into(), json!({ "site": site.name }));
                return;
            }
            if let Some(url) = tools::web::spoken_url(target) {
                self.act(self.inter.as_ref().map(|i| i.id).unwrap_or(0), "web.open".into(), json!({ "url": url }));
                return;
            }
            if let Some(place) = tools::files::location(target) {
                self.act(self.inter.as_ref().map(|i| i.id).unwrap_or(0), "files.open".into(), json!({ "where": place.to_string_lossy() }));
                return;
            }
            self.missing(target);
            return;
        }
        let top = found[0].1;
        let close: Vec<&(crate::tools::apps::App, f64)> = found.iter().filter(|(_, s)| top - s < 0.04).collect();
        if close.len() > 1 && top < 0.995 {
            let names: Vec<String> = close.iter().map(|(a, _)| a.name.clone()).collect();
            let key = format!("choice:{}", names.join("|"));
            if let Ok(Some((pref, count))) = self.core.db.read(|c| Ok(crate::storage::pref_get(c, &key)))
                && count >= 2
                    && let Some((a, _)) = close.iter().find(|(a, _)| a.name == pref) {
                        self.execute(Intent::OpenApp { target: a.id.clone(), role: None });
                        return;
                    }
            let q = if names.len() == 2 { format!("Which one: {} or {}?", names[0], names[1]) } else { "Which one?".to_owned() };
            self.ask(&q, names.clone(), Purpose::App { verb: "open", choices: names });
            return;
        }
        self.execute(Intent::OpenApp { target: found[0].0.id.clone(), role: None });
    }

    fn close_app(&mut self, target: &str) {
        let found = self.core.apps.find(target, None, &self.core.defaults());
        match found.first() {
            Some((a, _)) => self.execute(Intent::CloseApp { target: a.id.clone() }),
            None => self.problem(&format!("I couldn't find {}.", text::sentence_case(target)), None),
        }
    }

    fn missing(&mut self, target: &str) {
        let name = text::sentence_case(target);
        let n = name.clone();
        self.core.db.write(move |c| {
            c.execute(
                "INSERT INTO missing_apps(name, count, last_asked) VALUES(?1, 1, ?2) ON CONFLICT(name) DO UPDATE SET count = count + 1, last_asked = excluded.last_asked, dismissed = 0",
                rusqlite::params![n, now_ms()],
            )?;
            Ok(())
        });
        self.core.hub.emit("apps.changed", json!({ "count": self.core.apps.count(), "delta": 0, "scanning": false }));
        self.ask(&format!("I couldn't find {name}. Install it, or pick another app?"), vec!["Install".into(), "Pick another".into(), "Cancel".into()], Purpose::Missing { app: name });
    }

    pub fn install(&mut self, inter: u64, app: &str) {
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Thinking;
        }
        let core = self.core.clone();
        let app = app.to_owned();
        tokio::spawn(async move {
            let result = crate::layers::blocking(move || tools::install::search(&app)).await.unwrap_or_else(|e| Err(e.to_string()));
            core.tell(ControlMsg::InstallFound { inter, result });
        });
    }

    fn on_install_found(&mut self, inter: u64, result: Result<tools::install::Package, String>) {
        if !self.inter.as_ref().is_some_and(|i| i.id == inter) && inter != 0 {
            return;
        }
        match result {
            Ok(pkg) => {
                let spec = tools::install::confirm_spec(&pkg);
                self.confirm(spec, Some(Pending::Install(pkg)));
            }
            Err(e) => self.problem(&e, None),
        }
    }

    fn wifi_intent(&mut self, action: &'static str, network: Option<String>) {
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Thinking;
        }
        let core = self.core.clone();
        match action {
            "disconnect" => self.execute(Intent::Wifi { action: "disconnect", network: None }),
            _ => {
                self.wifi = Some(WifiFlow { networks: vec![], ssid: None, password: vec![], revealed: false });
                self.core.hub.emit("wifi.state", json!({ "phase": "scanning", "networks": [], "ssid": null, "length": 0, "revealed": null, "message": null }));
                tokio::spawn(async move {
                    let r = tools::wifi::scan().await.map_err(|e| e.to_string());
                    core.tell(ControlMsg::Wifi(tools::wifi::Event::Scanned { result: r, wanted: network }));
                });
            }
        }
    }

    fn on_wifi(&mut self, e: tools::wifi::Event) {
        match e {
            tools::wifi::Event::Scanned { result, wanted } => {
                let nets = match result {
                    Ok(n) => n,
                    Err(e) => {
                        self.wifi = None;
                        self.problem(&e, None);
                        return;
                    }
                };
                if nets.is_empty() {
                    self.wifi = None;
                    self.problem("No Wi-Fi networks were found.", None);
                    return;
                }
                let json_nets: Vec<Value> = nets.iter().map(|n| json!({ "n": n.n, "ssid": n.ssid, "secure": n.secure, "saved": n.saved, "signal": n.signal })).collect();
                self.core.hub.emit("wifi.state", json!({ "phase": "choose", "networks": json_nets, "ssid": null, "length": 0, "revealed": null, "message": null }));
                if let Some(w) = self.wifi.as_mut() {
                    w.networks = nets.clone();
                }
                if let Some(want) = wanted {
                    let best = nets.iter().map(|n| (n, text::similarity(&want, &n.ssid))).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                    if let Some((n, s)) = best
                        && s >= 0.8 {
                            let n = n.clone();
                            self.wifi_connect(n);
                            return;
                        }
                }
                let names: Vec<String> = nets.iter().take(6).map(|n| n.ssid.clone()).collect();
                let q = format!("I found {} networks. Which one?", names.len());
                self.ask(&q, names, Purpose::Networks);
            }
            tools::wifi::Event::Connected { ssid, result } => {
                let flow_pw = self.wifi.as_mut().map(|w| std::mem::take(&mut w.password));
                if let Some(mut pw) = flow_pw {
                    pw.iter_mut().for_each(|c| *c = '\0');
                }
                match result {
                    Ok(()) => {
                        self.wifi = None;
                        self.disarm(Timer::Password);
                        self.core.hub.emit("wifi.state", json!({ "phase": "connected", "networks": [], "ssid": ssid, "length": 0, "revealed": null, "message": format!("Connected to {ssid}") }));
                        let o = Outcome::ok_with(&format!("Connected to {ssid}"), "registered", None);
                        if let Some(i) = self.inter.as_mut() {
                            i.record = true;
                            i.said = "Connect to Wi-Fi".into();
                        }
                        let said = self.inter.as_ref().map(|i| i.said.clone()).unwrap_or_default();
                        self.record(&said, &o, "success", &Intent::Wifi { action: "connect", network: Some(ssid.clone()) }, "", 0);
                        self.say(&format!("Connected to {ssid}."), "done");
                    }
                    Err(tools::wifi::ConnectError::Rejected) => {
                        self.core.hub.emit("wifi.state", json!({ "phase": "password", "networks": [], "ssid": ssid, "length": 0, "revealed": null, "message": "The password was rejected. Say it again, or choose another network." }));
                        if let Some(i) = self.inter.as_mut() {
                            i.record = false;
                        }
                        self.say("The password was rejected. Say it again, or choose another network.", "password");
                    }
                    Err(tools::wifi::ConnectError::Other(e)) => {
                        self.wifi = None;
                        self.core.hub.emit("wifi.state", json!({ "phase": "failed", "networks": [], "ssid": ssid, "length": 0, "revealed": null, "message": e }));
                        self.problem(&e, None);
                    }
                }
            }
        }
    }

    fn wifi_connect(&mut self, net: tools::wifi::Network) {
        if net.enterprise {
            self.wifi = None;
            self.problem("This network needs a username and certificate. Enterprise networks aren't supported in this version.", None);
            return;
        }
        if let Some(w) = self.wifi.as_mut() {
            w.ssid = Some(net.ssid.clone());
            w.password.clear();
        }
        if net.saved || !net.secure {
            self.connect_now();
            return;
        }
        if let Some(i) = self.inter.as_mut() {
            i.record = false;
        }
        self.core.hub.emit("wifi.state", json!({ "phase": "password", "networks": [], "ssid": net.ssid, "length": 0, "revealed": null, "message": null }));
        self.emit_session();
        self.say("Say the password.", "password");
    }

    fn listen_for_password(&mut self) {
        let asr = self.asr_next;
        self.asr_next += 1;
        if let Some(i) = self.inter.as_mut() {
            i.asr = asr;
            i.phase = Phase::Listening;
        }
        self.arm(Timer::Password, Duration::from_secs(90));
        self.set_session(Session::Capturing, "password");
        self.start_capture(asr, 0.2);
    }

    fn on_password(&mut self, text: &str) {
        let ops = crate::password::parse(text);
        let Some(w) = self.wifi.as_mut() else { return };
        let ssid = w.ssid.clone().unwrap_or_default();
        let mut speak: Option<String> = None;
        let mut connect = false;
        for op in &ops {
            match op {
                crate::password::Op::Connect => connect = true,
                crate::password::Op::Show => w.revealed = true,
                crate::password::Op::Hide => w.revealed = false,
                crate::password::Op::ReadBack => speak = Some(if w.password.is_empty() { "The password is empty.".into() } else { crate::password::spell(&w.password) }),
                crate::password::Op::Cancel => {
                    self.cancel_all(Some("Cancelled."), true);
                    return;
                }
                op => {
                    crate::password::apply(&mut w.password, op);
                }
            }
        }
        let revealed: Option<String> = w.revealed.then(|| w.password.iter().collect());
        let len = w.password.len();
        self.core.hub.emit("wifi.state", json!({ "phase": "password", "networks": [], "ssid": ssid, "length": len, "revealed": revealed, "message": null }));
        if connect {
            self.connect_now();
            return;
        }
        if ops.is_empty() {
            speak = Some("I didn't catch any characters. Spell the password, then say connect.".into());
        }
        match speak {
            Some(s) => self.say(&s, "password"),
            None => self.listen_for_password(),
        }
    }

    fn connect_now(&mut self) {
        let Some(w) = self.wifi.as_mut() else { return };
        let Some(ssid) = w.ssid.clone() else { return };
        let pw: Vec<char> = w.password.clone();
        let net = w.networks.iter().find(|n| n.ssid == ssid).cloned();
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Acting;
        }
        self.core.hub.emit("wifi.state", json!({ "phase": "connecting", "networks": [], "ssid": ssid, "length": pw.len(), "revealed": null, "message": null }));
        let core = self.core.clone();
        tokio::spawn(async move {
            let mut secret: String = pw.iter().collect();
            let r = tools::wifi::connect(&ssid, net.as_ref(), if secret.is_empty() { None } else { Some(&secret) }).await;
            unsafe { secret.as_bytes_mut().iter_mut().for_each(|b| *b = 0) };
            core.tell(ControlMsg::Wifi(tools::wifi::Event::Connected { ssid, result: r }));
        });
    }

    fn open_camera(&mut self) {
        if self.camera.is_some() {
            self.say("The camera is already on.", "camera");
            return;
        }
        let tx = self.core.control.clone();
        match tools::camera::Camera::open(self.core.hub.clone(), tx) {
            Ok(c) => {
                self.camera = Some(c);
                if let Some(i) = self.inter.as_mut() {
                    i.phase = Phase::Acting;
                    i.record = true;
                }
                self.arm(Timer::Camera, Duration::from_secs(60));
                self.core.hub.emit("camera.state", json!({ "open": true, "frame": null, "secondsLeft": 60, "message": "Camera on. Say click to take the photo. Say cancel to close." }));
                self.say("Camera on. Say click to take the photo. Say cancel to close.", "camera");
            }
            Err(e) => self.problem(&e, Some(json!({ "label": "Open Permissions", "method": "ui.open", "params": { "page": "settings/permissions" } }))),
        }
    }

    fn camera_capture(&mut self) {
        if let Some(c) = &self.camera {
            c.capture();
        }
    }

    fn close_camera(&mut self, message: Option<&str>) {
        if let Some(c) = self.camera.take() {
            c.close();
            self.disarm(Timer::Camera);
            self.core.hub.emit("camera.state", json!({ "open": false, "frame": null, "secondsLeft": null, "message": message }));
            if let Some(m) = message
                && self.inter.is_some() {
                    self.say(m, "done");
                }
        }
    }

    fn on_camera(&mut self, e: tools::camera::Event) {
        match e {
            tools::camera::Event::Decoded(r) => match r {
                None => self.say("I couldn't read a QR code. Move closer or add light, then say click again.", "camera"),
                Some(tools::camera::Qr::Wifi { ssid, password }) => {
                    self.close_camera(None);
                    let spec = ConfirmSpec {
                        title: format!("Connect to {ssid}?"),
                        facts: vec![format!("network  {ssid}"), format!("security  {}", if password.is_empty() { "open" } else { "password from the code, hidden" })],
                        consequence: "This joins the network using the details in the QR code.".into(),
                        risk: "medium",
                        action_label: "Connect".into(),
                        danger: false,
                    };
                    self.confirm(spec, Some(Pending::WifiQr { ssid, password: password.chars().collect() }));
                }
                Some(tools::camera::Qr::Text(t)) => {
                    self.close_camera(None);
                    let is_url = t.starts_with("https://") || t.starts_with("http://");
                    if is_url {
                        let spec = ConfirmSpec {
                            title: "Open this link?".into(),
                            facts: vec![t.chars().take(300).collect()],
                            consequence: "The link opens in your default browser. Check that you trust it.".into(),
                            risk: "medium",
                            action_label: "Open link".into(),
                            danger: false,
                        };
                        self.confirm(spec, Some(Pending::Url(t)));
                    } else {
                        let shown: String = t.chars().filter(|c| !c.is_control() || *c == ' ').take(400).collect();
                        self.say(&format!("The code says: {shown}"), "done");
                    }
                }
            },
            tools::camera::Event::Failed(e) => {
                self.close_camera(None);
                self.problem(&e, None);
            }
        }
    }

    fn grant(&mut self, item: &'static str) {
        let key = match item {
            "control" => "permissions.control",
            "screen" => "privacy.screenReading",
            _ => return,
        };
        let mut m = serde_json::Map::new();
        m.insert(key.to_owned(), json!(true));
        let changed = self.core.settings.apply(&m);
        if !changed.is_empty() {
            self.core.db.save_settings(changed.clone());
            self.core.hub.emit("settings.changed", json!({ "changes": changed }));
        }
        #[cfg(target_os = "linux")]
        if item == "control" {
            tokio::spawn(async {
                let _ = tools::atspi::set_enabled(true).await;
            });
        }
        self.core.hub.emit("permissions.state", json!({ "tier": "", "item": item, "status": "granted" }));
    }

    fn ask_permission(&mut self, item: &'static str, then: Intent) -> bool {
        let (title, consequence) = match item {
            "control" => ("Let Setu work inside your apps?", "I'll read the buttons in the window you're using, then click and type for you. You can turn this off in Settings."),
            "screen" => ("Let Setu read your screen?", "I'll read the text in the window you ask about. Nothing is recorded or kept."),
            _ => return false,
        };
        let spec = ConfirmSpec { title: title.into(), facts: vec![], consequence: consequence.into(), risk: "medium", action_label: "Allow".into(), danger: false };
        self.confirm(spec, Some(Pending::Grant { item, then }));
        true
    }

    fn show_numbers(&mut self) {
        if !self.s().bool("permissions.control") {
            self.ask_permission("control", Intent::Numbers { action: "show", n: None });
            return;
        }
        if let Some(i) = self.inter.as_mut() {
            i.phase = Phase::Acting;
        }
        let core = self.core.clone();
        tokio::spawn(async move {
            let r = tools::atspi::elements().await.map_err(|e| e.to_string());
            core.tell(ControlMsg::NumbersFound(r));
        });
    }

    fn on_numbers(&mut self, r: Result<Vec<tools::atspi::Element>, String>) {
        match r {
            Ok(list) if !list.is_empty() => {
                crate::browserui::forget_numbers();
                let els: Vec<Value> = list.iter().map(|e| json!({ "n": e.n, "x": e.x, "y": e.y, "w": e.w, "h": e.h, "label": e.label })).collect();
                self.core.hub.emit("numbers.state", json!({ "visible": true, "elements": els }));
                let n = list.len();
                self.numbers = Some(list);
                self.arm(Timer::Numbers, Duration::from_secs(30));
                self.say(&format!("{n} numbers shown. Say a number to click it."), "numbers");
            }
            Ok(_) => self.problem("I couldn't find anything to click in the active window.", None),
            Err(e) => self.problem(&e, None),
        }
    }

    fn numbers_shown(&self) -> bool {
        self.numbers.is_some() || crate::browse::PAGE_NUMBERS.load(Ordering::Relaxed)
    }

    fn hide_numbers(&mut self) {
        if self.numbers.take().is_some() {
            self.disarm(Timer::Numbers);
            self.core.hub.emit("numbers.state", json!({ "visible": false, "elements": [] }));
            self.listen();
        }
    }

    fn click_number(&mut self, n: usize) {
        let Some(list) = self.numbers.clone() else {
            if crate::browse::PAGE_NUMBERS.load(Ordering::Relaxed) && self.inter.is_some() {
                self.execute(Intent::Act { tool: "browser.page".into(), args: json!({ "action": "click", "ref": { "number": n }, "what": format!("number {n}") }) });
                return;
            }
            if self.inter.is_some() {
                self.problem("Numbers aren't shown. Say show numbers first.", None);
            }
            return;
        };
        let Some(el) = list.into_iter().find(|e| e.n == n) else {
            self.say(&format!("There is no number {n}."), "numbers");
            return;
        };
        self.hide_numbers();
        if self.inter.is_none() {
            self.begin_silent();
        }
        self.execute(Intent::Internal { kind: "click", args: serde_json::to_value(&el).unwrap_or(Value::Null) });
    }

    fn begin_silent(&mut self) {
        self.inter = None;
        self.quiet_interaction(Phase::Acting, true, true);
    }

    fn sync_undoable(&self) {
        *self.core.undoable.lock().unwrap() = self.undo.iter().map(|(a, _, _)| a.clone()).collect();
    }

    fn quiet_interaction(&mut self, phase: Phase, woke: bool, record: bool) {
        if self.inter.is_some() {
            return;
        }
        self.inter = Some(Interaction {
            id: self.next,
            asr: 0,
            started: Instant::now(),
            phase,
            woke,
            by_name: false,
            waited: false,
            said: String::new(),
            record,
            action: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tts: None,
            strict: false,
            voice: None,
            spoken: false,
            narrated: false,
            chat: None,
            answer: None,
            logged: false,
            silent: false,
            carried: false,
            aside: false,
            held: None,
            acted: false,
            told: false,
        });
        self.next += 1;
    }

    fn heard(&mut self, raw: String, score: Option<f32>, seconds: f32, addressed: bool) -> Result<Value, ApiError> {
        if self.inter.is_some() || self.prompt.is_some() {
            return Err(ApiError::new(setu_common::protocol::ErrorCode::Busy, "Finish the current request first."));
        }
        self.disarm(Timer::Grace);
        self.quiet_interaction(Phase::Thinking, addressed, true);
        let asr = self.asr_next;
        self.asr_next += 1;
        let id = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
        if let Some(i) = self.inter.as_mut() {
            i.asr = asr;
        }
        self.on_final(asr, raw, 0, None, score.map(|s| crate::speaker::Heard { score: s, seconds }));
        Ok(json!({ "interaction": id }))
    }

    fn run_chat(&mut self, text: &str, chat: i64) -> Result<Value, ApiError> {
        if self.inter.is_some() || self.prompt.is_some() {
            return Err(ApiError::new(setu_common::protocol::ErrorCode::Busy, "I'm still on the last request. Ask again in a moment."));
        }
        let r = self.run_text_inner(text, Some(chat))?;
        Ok(r)
    }

    fn run_text(&mut self, text: &str) -> Result<Value, ApiError> {
        self.run_text_inner(text, None)
    }

    fn run_intent(&mut self, intent: Intent) -> Result<Value, ApiError> {
        if self.prompt.as_ref().is_some_and(|p| p.look.starts_with("look")) {
            let p = self.prompt.take().unwrap();
            self.disarm(Timer::Prompt);
            self.core.engines.tts.send(&ToEngine::TtsStop);
            self.core.hub.emit("prompt.closed", json!({ "id": p.id, "outcome": "answered", "message": null }));
        } else if self.prompt.is_some() || self.inter.as_ref().is_some_and(|i| !matches!(i.phase, Phase::Speaking)) {
            return Err(ApiError::new(setu_common::protocol::ErrorCode::Busy, "Finish the current request first."));
        }
        if self.inter.as_ref().is_some_and(|i| matches!(i.phase, Phase::Speaking)) {
            self.core.engines.tts.send(&ToEngine::TtsStop);
            self.finish("done");
        }
        self.disarm(Timer::Grace);
        self.quiet_interaction(Phase::Thinking, false, true);
        if let Some(i) = self.inter.as_mut() {
            i.carried = true;
        }
        let id = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
        self.execute(intent);
        Ok(json!({ "interaction": id }))
    }

    fn run_text_inner(&mut self, text: &str, chat: Option<i64>) -> Result<Value, ApiError> {
        if self.inter.is_some() || self.prompt.is_some() {
            return Err(ApiError::new(setu_common::protocol::ErrorCode::Busy, "Finish the current request first."));
        }
        let t = self.personalize(&normalize(text));
        if t.is_empty() {
            return Err(ApiError::invalid("Type a command."));
        }
        self.disarm(Timer::Grace);
        self.quiet_interaction(Phase::Thinking, false, true);
        let display = if chat.is_some() { text.trim().to_owned() } else { text::display_case(&t) };
        let id = self.inter.as_ref().map(|i| i.id).unwrap_or(0);
        if let Some(i) = self.inter.as_mut() {
            i.said = display.clone();
            if let Some(c) = chat {
                i.chat = Some(c);
                i.logged = true;
                i.silent = true;
                self.core.threads.bind(i.id, c);
            }
        }
        self.core.hub.emit("transcript.final", json!({ "interaction": id, "text": display, "language": self.core.lang() }));
        if self.dictation
            && !self.plain
            && chat.is_some()
            && codespeech::command(&t).is_none()
            && !codespeech::starts_dictation(&t)
            && router::rules(&t, &self.s().str("voice.stopWord"), self.numbers_shown()).is_none_or(|i| matches!(i, Intent::TypeText { .. } | Intent::Unknown))
        {
            self.on_routed(id, Intent::Act { tool: "code.compose".into(), args: json!({ "code": text.trim() }) }, "rules", 0, None);
            return Ok(json!({ "interaction": id }));
        }
        let routed = if chat.is_some() && !text.trim().is_empty() { text.trim().to_owned() } else { t };
        self.route(id, routed);
        Ok(json!({ "interaction": id }))
    }

    fn set_muted(&mut self, muted: bool, minutes: Option<f64>) {
        self.muted = muted;
        self.disarm(Timer::Unmute);
        if muted {
            self.cancel_all(None, false);
            self.muted_until = minutes.map(|m| now_ms() + (m * 60_000.0) as i64);
            if let Some(m) = minutes {
                self.arm(Timer::Unmute, Duration::from_secs_f64(m * 60.0));
            }
            self.session = Session::Dormant;
            Self::unduck();
            self.reason = "muted".into();
        } else {
            self.muted_until = None;
            self.reason = "unmuted".into();
        }
        self.listen();
        self.emit_mic();
        self.emit_session();
    }

    fn do_undo(&mut self, id: &str) -> Result<String, ApiError> {
        let idx = self.undo.iter().position(|(a, _, _)| a == id).ok_or_else(|| ApiError::not_found("That action can't be undone."))?;
        let (_, u, _) = self.undo.remove(idx).unwrap();
        self.sync_undoable();
        let core = self.core.clone();
        let msg = tools::undo(&core, u).map_err(|e| ApiError::new(setu_common::protocol::ErrorCode::Internal, e.to_string()))?;
        Ok(msg)
    }

    fn schedule_unload(&mut self) {
        let mins = self.s().get("models.unloadMinutes").as_i64().unwrap_or(5);
        if mins > 0 {
            self.arm(Timer::Unload, Duration::from_secs(mins as u64 * 60 + 1));
        }
    }

    fn on_timer(&mut self, kind: Timer) {
        match kind {
            Timer::Reading => self.on_reading_timer(false),
            Timer::ReadingLong => self.on_reading_timer(true),
            Timer::Prompt => {
                if self.prompt.as_ref().is_some_and(|p| p.look.starts_with("look")) {
                    self.close_prompt("timeout", None);
                    self.core.engines.tts.send(&ToEngine::TtsStop);
                    self.core.feed.close();
                    if self.inter.is_some() {
                        self.finish("done");
                    }
                } else if self.prompt.is_some() {
                    self.close_prompt("timeout", Some("Cancelled."));
                    self.core.engines.tts.send(&ToEngine::TtsStop);
                    self.core.feed.close();
                    if self.inter.is_some() {
                        self.say("Cancelled. Nothing was changed.", "cancelled");
                    }
                }
            }
            Timer::Grace => {
                if self.session == Session::Grace && self.prompt.is_none() {
                    if self.dictation {
                        log::info!("{} ended after a long silence", if self.plain { "dictation" } else { "coding mode" });
                        self.end_dictation();
                    }
                    self.enter_active("grace ended");
                }
            }
            Timer::Active => {
                if self.session == Session::Active {
                    self.go_dormant("timeout", true);
                }
            }
            Timer::Unmute => self.set_muted(false, None),
            Timer::WakeTest => {
                if self.wake_test.is_some() {
                    self.wake_test = Some(0);
                    self.core.hub.emit("wake.test", json!({ "result": "not_heard", "confidence": null, "streak": 0, "t": now_ms() }));
                    self.arm(Timer::WakeTest, Duration::from_secs(8));
                }
            }
            Timer::Camera => self.close_camera(Some("Camera closed. Say scan the QR code to try again.")),
            Timer::Numbers => {
                self.hide_numbers();
            }
            Timer::Style => {
                if self.enroll_style.take().is_some() {
                    self.enroll = None;
                    self.listen();
                    self.announce("I didn't hear a sentence, so nothing was added to your voice.".into());
                }
            }
            Timer::Password => {
                if self.wifi.as_ref().is_some_and(|w| w.ssid.is_some()) || self.secret.is_some() {
                    self.cancel_all(Some("Cancelled."), true);
                }
            }
            Timer::Unload => {
                let mins = self.s().get("models.unloadMinutes").as_i64().unwrap_or(5);
                if mins <= 0 || self.inter.is_some() {
                    return;
                }
                let idle = Duration::from_secs(mins as u64 * 60);
                for e in self.core.engines.all() {
                    if e.kind != EngineKind::Llm {
                        continue;
                    }
                    if e.is_running() && e.idle_for() >= idle {
                        log::info!("unloading the {} engine after {mins} idle minutes", e.kind.as_str());
                        e.stop();
                    }
                }
            }
        }
    }

    fn on_engine(&mut self, e: EngineEvent) {
        match e {
            EngineEvent::Partial { id, text } => {
                let Some(i) = &self.inter else { return };
                if !(i.asr == id && i.phase == Phase::Listening && i.record) {
                    return;
                }
                let inter = i.id;
                let t = self.strip_wake(&normalize(&text));
                let complete = !t.is_empty() && router::rules(&t, &self.s().str("voice.stopWord"), self.numbers_shown()).is_some();
                if t.split_whitespace().count() >= 2 && self.prompt.is_none() && !complete {
                    let core = self.core.clone();
                    tokio::spawn(async move { crate::pipeline::warm(&core).await });
                }
                if !t.is_empty() {
                    self.core.hub.emit("transcript.partial", json!({ "interaction": inter, "text": text::sentence_case(&t), "language": self.core.lang() }));
                }
                if !self.dictation && self.prompt.is_none() {
                    let base = self.s().f64("session.endSilence") as f32;
                    self.set_end_silence(endpoint(base, &t, complete));
                }
            }
            EngineEvent::TtsLine { id, index } => self.core.hub.emit("reply.line", json!({ "id": id.to_string(), "index": index })),
            EngineEvent::TtsLevel { level } => self.core.hub.emit("tts.level", json!({ "level": level })),
            EngineEvent::LlmPartial { id, text } => self.on_llm_partial(id, &text),
            EngineEvent::TtsDone { id, first_audio_ms, .. } => {
                if let Some(st) = self.stream.as_mut()
                    && st.last_tts == Some(id)
                {
                    st.done = true;
                    st.open = false;
                }
                if self.echo_name {
                    self.echo_name = false;
                    self.echo_until = Some(Instant::now() + Duration::from_millis(800));
                }
                if first_audio_ms > 0 {
                    self.core.metrics.record("first_audio", first_audio_ms);
                }
                self.core.hub.emit("reply.done", json!({ "id": id.to_string() }));
                let Some(i) = self.inter.as_mut() else {
                    if self.converse.is_some() {
                        self.echo_until = Some(Instant::now() + Duration::from_millis(350));
                    }
                    self.listen();
                    return;
                };
                if i.tts != Some(id) {
                    return;
                }
                i.tts = None;
                if i.phase == Phase::Acting {
                    return;
                }
                let after = if self.prompt.is_some() { "ask" } else { "done" };
                self.after_speech(after);
            }
            EngineEvent::Status { kind, status, message, ram } => {
                if kind == EngineKind::Llm && status != "loaded" {
                    crate::pipeline::reset_prefix();
                }
                if kind != EngineKind::Vision {
                    let slot = kind.as_str();
                    self.core.hub.emit("models.state", crate::handlers::slot_state(&self.core, slot, Some(status), message.as_deref(), ram));
                }
                self.core.emit_daemon();
            }
            EngineEvent::Degraded { kind } => {
                let what = match kind {
                    EngineKind::Asr => "The speech engine",
                    EngineKind::Llm => "The language model",
                    EngineKind::Tts => "The voice",
                    EngineKind::Vision => "The screen picture reader",
                };
                self.core.attention.raise(
                    &self.core.hub,
                    crate::attention::Issue::new("engine.degraded", "warning", format!("{what} restarted several times and is paused. Simple commands still work.")).action(
                        "View log",
                        "ui.open",
                        json!({ "page": "settings/general" }),
                    ),
                );
                self.core.emit_daemon();
            }
            EngineEvent::Recovered => {
                self.core.attention.clear(&self.core.hub, "engine.degraded");
                self.core.emit_daemon();
            }
        }
    }

    fn on_settings(&mut self, c: Map<String, Value>) {
        let audio_keys = [
            "audio.inputDevice",
            "audio.gain",
            "audio.autoGain",
            "audio.quietVoice",
            "audio.noiseReduction",
            "audio.echoCancel",
            "session.endSilence",
            "wake.sensitivity",
            "wake.longWindow",
            "speaker.verification",
        ];
        if c.keys().any(|k| audio_keys.contains(&k.as_str())) {
            self.apply_audio_settings();
        }
        if c.contains_key("models.gpu") {
            crate::gpu::apply(&self.core);
        }
        if c.get("setup.completed").and_then(Value::as_bool) == Some(true) && self.s().bool("setup.rerun") {
            self.core.settings.set_internal("setup.rerun", json!(false));
            let mut m = Map::new();
            m.insert("setup.rerun".into(), json!(false));
            self.core.db.save_settings(m);
        }
        if c.keys().any(|k| ["assistant.name", "wake.useHey", "voice.stopWord", "wake.sensitivity", "setup.completed"].contains(&k.as_str())) {
            self.listen();
            self.emit_session();
        }
        if c.contains_key("audio.inputDevice") || c.contains_key("audio.quietVoice") {
            self.emit_mic();
        }
        if c.contains_key("ui.language") {
            self.core.apps.set_language(&self.core.lang());
        }
        if c.keys().any(|k| k == "ui.language" || k == "models.tier" || k == "apps.defaults") {
            crate::pipeline::reset_prefix();
        }
        if c.contains_key("session.activeMinutes") && self.session == Session::Active {
            self.enter_active("settings changed");
        }
    }

    fn on_enrolled(&mut self, r: Result<Vec<f32>, String>) {
        self.enroll = None;
        let reading = self.converse.as_ref().is_some_and(|c| c.reading.is_some());
        match r {
            Ok(c) => {
                self.core.hub.emit("speaker.enroll", json!({ "phase": "done", "count": 3, "needed": 3, "message": null }));
                if self.s().bool("speaker.verification") || reading {
                    self.core.audio.send(AudioCmd::Verify(Some(c)));
                }
                self.finish_reading(true);
            }
            Err(e) => {
                self.core.hub.emit("speaker.enroll", json!({ "phase": "failed", "count": 0, "needed": 3, "message": e }));
                self.finish_reading(false);
            }
        }
        self.listen();
    }

    fn on_req(&mut self, r: Req) {
        match r {
            Req::Answer { id, answer, via, reply } => {
                let _ = reply.send(self.answer(&id, &answer, &via).map(|_| json!({})));
            }
            Req::Cancel { id, reply } => {
                if let Some(target) = id {
                    let current = self.inter.as_ref().and_then(|i| i.action.clone());
                    if current.as_deref() != Some(target.as_str()) {
                        let _ = reply.send(Ok(json!({ "cancelled": [], "notCancellable": [target] })));
                        return;
                    }
                }
                let (c, n) = self.cancel_all(Some("Stopped."), true);
                let _ = reply.send(Ok(json!({ "cancelled": c, "notCancellable": n })));
            }
            Req::Undo { id, reply } => {
                let r = self.do_undo(&id).map(|m| json!({ "message": m }));
                let _ = reply.send(r);
            }
            Req::Mute { muted, minutes, reply } => {
                self.set_muted(muted, minutes);
                let _ = reply.send(Ok(self.mic_json()));
            }
            Req::Sleep { reply } => {
                self.cancel_all(None, false);
                self.go_dormant("tray", false);
                let _ = reply.send(Ok(self.session_json()));
            }
            Req::WakeTest { start, reply } => {
                if start {
                    self.wake_test = Some(0);
                    self.arm(Timer::WakeTest, Duration::from_secs(8));
                } else {
                    self.wake_test = None;
                    self.disarm(Timer::WakeTest);
                }
                self.core.audio.send(AudioCmd::Learn(start));
                self.listen();
                let _ = reply.send(Ok(json!({})));
            }
            Req::Onboard { say, choices, reply } => {
                if let Some(c) = choices {
                    self.onboard = c;
                }
                if let Some(t) = say.filter(|t| !t.trim().is_empty()) {
                    self.say_onboard(&t);
                }
                self.listen();
                let listening = self.in_wizard() && !self.muted && self.mic_ok;
                let _ = reply.send(Ok(json!({ "listening": listening })));
            }
            Req::Sample { voice, rate, volume, text, reply } => {
                let core = self.core.clone();
                let line = text.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| format!("This is how I sound. Say {} when you need me.", self.s().phrase()));
                let speed = rate.unwrap_or(self.s().f64("tts.rate")).clamp(0.5, 2.0);
                let est = (line.chars().count() as f64 * 65.0 / speed) as u64;
                self.echo_until = Some(Instant::now() + Duration::from_millis(est + 1500));
                tokio::spawn(async move {
                    let r = crate::pipeline::sample(&core, voice, rate, volume, line).await;
                    let _ = reply.send(r.map_err(|e| ApiError::unavailable(e.to_string())));
                });
            }
            Req::Converse { screen, explain, reply } => {
                let _ = reply.send(Ok(self.on_converse(screen, explain)));
            }
            Req::ConverseText { text, reply } => {
                if self.converse.is_none() {
                    let _ = reply.send(Err(ApiError::unavailable("Nothing is listening for an answer right now.")));
                    return;
                }
                self.on_clip(onboard::Clip::Utterance, text, String::new(), Vec::new());
                let _ = reply.send(Ok(json!({})));
            }
            Req::Enroll { text: Some(text), reply } => {
                let _ = reply.send(self.start_reading(text));
            }
            Req::Enroll { text: None, reply } => {
                match crate::speaker::model_path(&self.core) {
                    Some(_) => {
                        self.enroll = Some(Vec::new());
                        self.listen();
                        self.core.hub.emit("speaker.enroll", json!({ "phase": "listening", "count": 0, "needed": 3, "message": "Say a sentence of a few words, three times." }));
                        let _ = reply.send(Ok(json!({})));
                    }
                    None => {
                        let _ = self.core.downloads.enqueue(&["speaker-campplus-en".into()]);
                        let _ = reply.send(Err(ApiError::unavailable("The speaker model is downloading (30 MB). Try again when it finishes.")));
                    }
                }
            }
            Req::Numbers { show, reply } => {
                if show {
                    self.show_numbers();
                } else {
                    self.hide_numbers();
                }
                let _ = reply.send(Ok(json!({ "count": self.numbers.as_ref().map(|n| n.len()).unwrap_or(0) })));
            }
            Req::WifiScan { reply } => {
                tokio::spawn(async move {
                    let r = tools::wifi::scan().await;
                    let _ = reply.send(
                        r.map(|n| json!({ "networks": n.iter().map(|x| json!({ "n": x.n, "ssid": x.ssid, "secure": x.secure, "saved": x.saved, "signal": x.signal })).collect::<Vec<_>>() }))
                            .map_err(|e| ApiError::unavailable(e.to_string())),
                    );
                });
            }
            Req::WifiConnect { ssid, reply } => {
                if self.inter.is_none() {
                    self.begin_silent();
                }
                self.wifi_intent("connect", Some(ssid));
                let _ = reply.send(Ok(json!({})));
            }
            Req::WifiReveal { reply } => {
                if let Some(w) = self.wifi.as_mut() {
                    w.revealed = true;
                    let shown: String = w.password.iter().collect();
                    self.core.hub.emit(
                        "wifi.state",
                        json!({ "phase": "password", "networks": [], "ssid": w.ssid, "length": w.password.len(), "revealed": shown, "message": null }),
                    );
                    let _ = reply.send(Ok(json!({})));
                } else {
                    let _ = reply.send(Err(ApiError::not_found("No password is being entered.")));
                }
            }
            Req::CameraOpen { reply } => {
                if self.inter.is_none() {
                    self.begin_silent();
                }
                self.open_camera();
                let _ = reply.send(Ok(json!({ "open": self.camera.is_some(), "secondsLeft": 60 })));
            }
            Req::CameraCapture { reply } => {
                self.camera_capture();
                let _ = reply.send(Ok(json!({})));
            }
            Req::CameraClose { reply } => {
                self.close_camera(Some("Camera closed."));
                let _ = reply.send(Ok(json!({})));
            }
            Req::Confirm { spec, then } => {
                if self.prompt.is_some() {
                    self.close_prompt("cancelled", None);
                }
                self.confirm(spec, Some(Pending::External(then)));
            }
            Req::Heard { text, score, seconds, addressed, reply } => {
                let _ = reply.send(self.heard(text, score, seconds, addressed));
            }
            Req::RunText { text, reply } => {
                let r = self.run_text(&text);
                let _ = reply.send(r);
            }
            Req::RunIntent { intent, reply } => {
                let r = self.run_intent(intent);
                let _ = reply.send(r);
            }
            Req::RunChat { text, chat, reply } => {
                let r = self.run_chat(&text, chat);
                let _ = reply.send(r);
            }
            Req::ChatAnswer { inter, text } => {
                if self.inter.as_ref().is_some_and(|i| i.id == inter) {
                    self.say(&text, "done");
                }
            }
            Req::Listen { chat, reply } => {
                if self.muted {
                    let _ = reply.send(Err(ApiError::invalid("The microphone is off. Turn it on first.")));
                    return;
                }
                self.cancel_all(None, false);
                self.begin(true, 0.0);
                let id = self.inter.as_ref().map(|i| i.id);
                if let (Some(i), Some(c)) = (id, chat) {
                    self.core.threads.bind(i, c);
                }
                let _ = reply.send(Ok(json!({ "interaction": id })));
            }
            Req::CorrectListen { history, reply } => {
                self.correction_for = Some(history);
                self.cancel_all(None, false);
                self.begin(true, 0.0);
                let _ = reply.send(Ok(json!({})));
            }
            Req::Snapshot { reply } => {
                let _ = reply.send((self.session_json(), self.mic_json()));
            }
        }
    }
}

fn sentence(did: &str) -> String {
    let t = did.trim();
    if t.is_empty() || t.ends_with(['.', '!', '?']) { t.to_owned() } else { format!("{t}.") }
}

pub fn spawn(core: Arc<Core>, rx: mpsc::Receiver<ControlMsg>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(Control::new(core, rx).run())
}

pub async fn ask(core: &Core, f: impl FnOnce(Reply) -> Req) -> ApiResult {
    let (tx, rx) = oneshot::channel();
    core.control.send(ControlMsg::Req(f(tx))).await.map_err(|_| ApiError::unavailable("The assistant is shutting down."))?;
    match tokio::time::timeout(Duration::from_secs(20), rx).await {
        Ok(Ok(r)) => r,
        _ => Err(ApiError::new(setu_common::protocol::ErrorCode::Timeout, "The assistant did not answer in time.")),
    }
}

fn needs_confirm(tool: &str, args: &Value) -> bool {
    matches!(tool, "code.extension" | "git.push" | "git.commit_push" | "terminal.run" | "files.delete" | "files.move" | "files.copy" | "messages.send" | "admin.setup" | "admin.remove" | "admin.run" | "jobs.uninstall" | "browser.foxsetup")
        || (tool == "doc.mail" && args.get("send").and_then(Value::as_bool) == Some(true))
        || (tool == "browser.fox" && args.get("op").and_then(Value::as_str) == Some("addons.remove"))
        || (tool == "browser.form" && args.get("op").and_then(Value::as_str) == Some("submit"))
        || crate::syscfg::needs_confirm(tool)
        || crate::vault::needs_confirm(tool)
        || tool == "face.forget"
        || crate::faceid::root_confirm(tool, args).is_some()
}

pub fn sensitive(intent: &Intent) -> bool {
    match intent {
        Intent::Install { .. } => true,
        Intent::Act { tool, args } => needs_confirm(tool, args) || tool.starts_with("vault.") || matches!(tool.as_str(), "jobs.install" | "jobs.download" | "face.signin" | "face.admin" | "face.voicekey" | "voice.learn"),
        _ => false,
    }
}

async fn prepare(core: &Arc<Core>, tool: &str, mut args: Value) -> Result<(Value, Option<ConfirmSpec>), String> {
    let s = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let spec = |title: String, facts: Vec<String>, consequence: &str, label: &str, danger: bool| ConfirmSpec {
        title,
        facts,
        consequence: consequence.to_owned(),
        risk: if danger { "high" } else { "medium" },
        action_label: label.to_owned(),
        danger,
    };
    match tool {
        "admin.setup" => {
            if crate::privileged::installed() {
                return Ok((args, None));
            }
            if crate::privileged::helper_binary().is_none() {
                return Err("The system helper isn't part of this install.".into());
            }
            Ok((args, Some(spec("Set up admin control?".into(), vec!["A small helper runs with administrator rights and does only a fixed list of jobs.".into(), "I'll still ask you before every change.".into()], "You'll be asked for your password once.", "Set up", false))))
        }
        "admin.remove" => Ok((args, Some(spec("Remove admin control?".into(), vec![], "Admin jobs will need the password each time again.", "Remove", false)))),
        "vault.forget" => {
            let title = crate::vault::confirm_title(core, &args)?;
            Ok((args, Some(spec(title, vec![], "It's removed from the vault on this computer.", "Forget", true))))
        }
        "face.forget" => Ok((args, Some(spec("Forget your face?".into(), vec![], "Passwords will need your system password until you set it up again.", "Forget", true)))),
        "face.signin" | "face.admin" => match crate::faceid::root_confirm(tool, &args) {
            Some((title, facts, consequence)) => Ok((args, Some(spec(title, facts, &consequence, "Turn on", true)))),
            None => Ok((args, None)),
        },
        "browser.form" => Ok((args, Some(spec("Send this form?".into(), vec![], "It goes to the website.", "Send", false)))),
        "doc.mail" => Ok((args, Some(spec("Send this email now?".into(), vec![], "", "Send", false)))),
        "browser.fox" => Ok((args.clone(), Some(spec(format!("Remove the {} add-on from Firefox?", s(&args, "name")), vec![], "", "Remove", true)))),
        "browser.foxsetup" => {
            if !crate::privileged::installed() {
                return Err("That needs admin control, because the script goes into Firefox's own folder. Say set up admin control first.".into());
            }
            Ok((args, Some(spec("Install Setu's script into Firefox?".into(), vec!["It runs only in your Setu profile and lets me manage add-ons, settings and DevTools.".into()], "Firefox needs a restart afterwards.", "Install", false))))
        }
        "jobs.uninstall" => {
            let app = s(&args, "app");
            let found = core.apps.find(&app, None, &core.defaults());
            let Some((a, _)) = found.into_iter().find(|(_, sc)| *sc >= 0.8) else { return Err(format!("{} isn't installed.", crate::text::sentence_case(&app))) };
            Ok((args, Some(spec(format!("Uninstall {}?", a.name), vec![], "Its own settings and files in your home folder stay.", "Uninstall", true))))
        }
        t if crate::syscfg::needs_confirm(t) => {
            let (title, facts, consequence, label, danger) = crate::syscfg::confirm(core, t, &args).await?;
            Ok((args, Some(spec(title, facts, &consequence, &label, danger))))
        }
        "admin.run" => {
            let action = s(&args, "action");
            let inner = args.get("args").cloned().unwrap_or_else(|| json!({}));
            if !crate::privileged::installed() {
                return Err("That needs admin control. Say set up admin control first; you'll be asked for your password once.".into());
            }
            let (q, _) = crate::privileged::describe(&action, &inner);
            let danger = crate::privileged::danger(&action, &inner);
            Ok((args, Some(spec(q, vec![], "", if danger { "Do it" } else { "Yes" }, danger))))
        }
        "messages.send" => {
            let spec = crate::messaging::confirm(&args);
            Ok((args, spec))
        }
        "code.extension" => {
            let name = s(&args, "name");
            let found = match tools::code::known_extension(&name) {
                Some((id, label)) => Some((id, label, String::new())),
                None => {
                    let n = name.clone();
                    crate::layers::blocking(move || tools::code::marketplace(&n)).await.ok().flatten()
                }
            };
            let Some((id, label, publisher)) = found else { return Err(format!("I couldn't find a VS Code extension called {name}.")) };
            if !tools::code::installed() {
                return Err("VS Code isn't installed. Say install VS Code first.".into());
            }
            args["id"] = json!(id);
            args["label"] = json!(label);
            let by = if publisher.is_empty() { String::new() } else { format!(" by {publisher}") };
            Ok((args, Some(spec(format!("Install the {label} extension{by} in VS Code?"), vec![id], "", "Install", false))))
        }
        "git.push" | "git.commit_push" => {
            let dir = crate::skills::project_dir(core).and_then(|d| tools::git::root(&d)).ok_or("Which project? Open it in VS Code, or say: set project folder to, then the folder name.")?;
            let d2 = dir.clone();
            let st = crate::layers::blocking(move || tools::git::status(&d2)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            let project = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if tool == "git.push" {
                if st.upstream && st.ahead == 0 {
                    return Err("Everything is already on GitHub. There is nothing to push.".into());
                }
                let what = if st.upstream { format!("{} commit{}", st.ahead, if st.ahead == 1 { "" } else { "s" }) } else { format!("the {} branch", st.branch) };
                Ok((args, Some(spec(format!("Push {what} from {project} to GitHub?"), vec![format!("branch  {}", st.branch)], "", "Push", false))))
            } else {
                if st.changed.is_empty() {
                    return Err("There is nothing new to commit.".into());
                }
                Ok((args, Some(spec(format!("Commit {} changed file{} and push {project} to GitHub?", st.changed.len(), if st.changed.len() == 1 { "" } else { "s" }), vec![format!("branch  {}", st.branch)], "", "Commit and push", false))))
            }
        }
        "terminal.run" => {
            let cmd = s(&args, "command");
            let folder = crate::skills::project_dir(core).map(|d| d.display().to_string()).unwrap_or_else(|| "your home folder".into());
            Ok((args, Some(spec(format!("Run {cmd}?"), vec![cmd.clone(), format!("in  {folder}")], "", "Run", false))))
        }
        "files.delete" | "files.move" | "files.copy" => {
            let name = s(&args, "name");
            let n = name.clone();
            let hits = crate::layers::blocking(move || tools::files::search(&n, None, None, Duration::from_millis(3500))).await.unwrap_or_default();
            let Some(top) = hits.into_iter().next() else { return Err(format!("I couldn't find {name}.")) };
            args["path"] = json!(top.path.to_string_lossy());
            let shown = tools::files::spoken_path(&top.path);
            match tool {
                "files.delete" => Ok((args, Some(spec(format!("Move {shown} to the trash?"), vec![top.path.display().to_string()], "You can restore it from the trash.", "Move to trash", true)))),
                "files.move" => {
                    let to = s(&args, "to");
                    Ok((args, Some(spec(format!("Move {shown} to {to}?"), vec![top.path.display().to_string()], "", "Move", false))))
                }
                _ => Ok((args, None)),
            }
        }
        _ => Ok((args, None)),
    }
}
