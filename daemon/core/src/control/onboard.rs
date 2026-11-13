use super::*;

pub struct Converse {
    pub step: String,
    pub screen: Value,
    pub reading: Option<Reading>,
    pub busy: bool,
}

pub struct Reading {
    pub text: String,
    pub done: Vec<Vec<f32>>,
    pub current: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clip {
    Utterance,
    Reading,
}

const NEXT: &[&str] = &[
    "next", "continue", "next page", "next step", "next screen", "go ahead", "go on", "move on", "proceed", "done", "go next", "carry on", "let's go", "let's continue", "go forward", "skip", "skip this", "skip for now",
];
const BACK: &[&str] = &["back", "go back", "previous", "previous step", "previous page", "previous screen", "previous stage", "last page", "take me back", "go to previous", "one step back"];
const REPEAT: &[&str] = &["repeat", "repeat that", "say that again", "once more", "again", "what did you say", "pardon", "come again", "explain", "explain again"];

const PARAGRAPHS: &[&str] = &[
    "Hey Setu, good morning. Today I want to plan my day calmly. I will check my messages, play some soft music, and take a short walk in the evening when the weather is pleasant.",
    "Hey Setu, please remember my voice. I like simple things, a warm cup of tea, a quiet room, and a good book. When I call your name, listen to me and help me quickly.",
    "Hey Setu, let us get started. I often open the browser, write a few emails, and set reminders for important work. It is nice to have a helper who understands how I speak.",
];

const READ_MIN_SECS: f32 = 3.0;

fn similarity(heard: &str, expected: &str) -> f32 {
    let h: Vec<String> = text::normalize(heard).split_whitespace().map(str::to_owned).collect();
    let e: std::collections::HashSet<String> = text::normalize(expected).split_whitespace().map(str::to_owned).collect();
    if h.is_empty() || e.is_empty() {
        return 0.0;
    }
    let hit = h.iter().filter(|w| e.contains(*w)).count();
    hit as f32 / e.len().min(h.len().max(1)).max(1) as f32
}

pub fn fallback_paragraph() -> String {
    PARAGRAPHS[(now_ms() as usize / 1000) % PARAGRAPHS.len()].to_owned()
}

pub fn shape_paragraph(raw: &str) -> Option<String> {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.to_lowercase().starts_with("hey setu") {
        return None;
    }
    let mut sentences: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        cur.push(c);
        if matches!(c, '.' | '!' | '?') {
            sentences.push(std::mem::take(&mut cur).trim().to_owned());
        }
    }
    let bad = |s: &str| {
        let l = s.to_lowercase();
        s.chars().any(|c| c.is_ascii_digit() || matches!(c, '*' | '#' | '"' | ':' | ';'))
            || ["my name", "named", "this is ", "i am a ", "i'm a ", "called "].iter().any(|k| l.contains(k))
    };
    let mut out: Vec<String> = Vec::new();
    let mut words = 0;
    for (i, s) in sentences.iter().enumerate() {
        if i > 0 && bad(s) {
            continue;
        }
        let n = s.split_whitespace().count();
        if words + n > 48 {
            break;
        }
        words += n;
        out.push(s.clone());
        if words >= 30 {
            break;
        }
    }
    let t = out.join(" ");
    valid_paragraph(&t).then_some(t)
}

pub fn valid_paragraph(t: &str) -> bool {
    let n = t.split_whitespace().count();
    t.to_lowercase().starts_with("hey setu") && (22..=60).contains(&n) && !t.chars().any(|c| c.is_ascii_digit() || matches!(c, '*' | '#' | '"'))
}

const STOP: &[&str] = &[
    "the", "and", "you", "your", "use", "please", "this", "that", "are", "does", "what", "instead", "for", "with", "change", "set", "make", "let", "lets", "let's", "just", "now", "some",
    "something", "different", "another", "one", "get", "have", "want", "would", "could", "can", "like", "i'd", "i'm", "it's", "its", "the", "will", "need", "should", "here", "there", "then", "than", "more", "less",
];

fn content(t: &str) -> Vec<String> {
    text::normalize(t).split_whitespace().filter(|w| w.len() >= 3 && !STOP.contains(w)).map(|w| w.trim_end_matches('s').to_owned()).collect()
}

fn options(screen: &Value) -> Vec<(String, String, Vec<String>)> {
    screen["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let id = o["id"].as_str()?.to_owned();
                    let mut label = o["label"].as_str().unwrap_or(&id).to_owned();
                    if let Some(h) = o["hint"].as_str().filter(|h| !h.is_empty()) {
                        label = format!("{label} ({h})");
                    }
                    let phrases = o["phrases"].as_array().map(|p| p.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                    Some((id, label, phrases))
                })
                .take(24)
                .collect()
        })
        .unwrap_or_default()
}

impl Control {
    pub(super) fn talk_ready(&self) -> bool {
        crate::handlers::slot_state(&self.core, "asr", None, None, 0)["status"] != "missing"
    }

    pub(super) fn voice_ready(&self) -> bool {
        let s = crate::pipeline::usable_tts(&self.core);
        s.provider != "local" || s.path.is_some() || (!s.models.is_empty() && s.models.iter().all(|m| self.core.store.is_installed(m)))
    }

    fn brain_ready(&self) -> bool {
        crate::handlers::slot_state(&self.core, "llm", None, None, 0)["status"] != "missing"
    }

    pub(super) fn conversing(&self) -> bool {
        self.converse.is_some() && self.windows > 0 && !self.muted && self.talk_ready()
    }

    pub(super) fn say_onboard(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        self.core.hub.emit("onboarding.say", json!({ "text": t }));
        if !self.voice_ready() {
            return;
        }
        self.announce(t.chars().take(600).collect());
    }

    pub(super) fn on_converse(&mut self, screen: Option<Value>, explain: bool) -> Value {
        let Some(screen) = screen else {
            self.converse = None;
            self.disarm(Timer::Reading);
            self.listen();
            self.emit_session();
            return json!({ "listening": false, "ready": self.talk_ready() });
        };
        let step = screen["step"].as_str().unwrap_or("").to_owned();
        let same = self.converse.as_ref().is_some_and(|c| c.step == step);
        if same {
            if let Some(c) = self.converse.as_mut() {
                c.screen = screen.clone();
            }
        } else {
            self.disarm(Timer::Reading);
            self.converse = Some(Converse { step: step.clone(), screen: screen.clone(), reading: None, busy: false });
        }
        if explain && self.s().bool("ui.readAloud") {
            self.explain(step, screen);
        }
        self.listen();
        self.emit_session();
        json!({ "listening": self.conversing(), "ready": self.talk_ready(), "voice": self.voice_ready(), "brain": self.brain_ready() })
    }

    fn explain(&mut self, step: String, screen: Value) {
        if !self.voice_ready() {
            return;
        }
        let summary = screen["summary"].as_str().unwrap_or("").to_owned();
        if !self.brain_ready() || screen["say"].as_str().is_some() {
            let line = screen["say"].as_str().map(str::to_owned).unwrap_or(summary);
            self.say_onboard(&line);
            return;
        }
        let core = self.core.clone();
        tokio::spawn(async move {
            let system = "You are Setu, a warm and friendly voice assistant from India, speaking to the person who is setting you up. \
Rephrase the screen description below as one or two short spoken sentences, under 30 words in total, in simple English, talking directly to them. \
Only use facts from the description, the choices and the state. Do not invent anything, do not greet them, and never use lists, emoji, markdown or quotes. \
End by inviting them to just tell you what they want. Answer as JSON {\"say\": \"...\"}.";
            let user = json!({ "screen": screen["title"], "about": summary, "choices": options(&screen).iter().map(|o| o.1.clone()).collect::<Vec<_>>(), "state": screen["state"] }).to_string();
            let text = match crate::pipeline::ask_json(&core, system, &user, Some(crate::pipeline::string_grammar("say")), 90, 0.2).await {
                Ok(v) => v["say"].as_str().map(str::to_owned).filter(|s| s.split_whitespace().count() >= 3).unwrap_or(summary),
                Err(e) => {
                    log::warn!("screen explanation failed: {e}");
                    summary
                }
            };
            core.tell(ControlMsg::Explained { step, text });
        });
    }

    pub(super) fn on_explained(&mut self, step: String, text: String) {
        if self.converse.as_ref().is_some_and(|c| c.step == step) {
            self.say_onboard(&text);
        }
    }

    pub(super) fn start_reading(&mut self, text: String) -> Result<Value, ApiError> {
        if crate::speaker::model_path(&self.core).is_none() {
            let _ = self.core.downloads.enqueue(&[crate::speaker::MODEL.into()]);
            return Err(ApiError::unavailable("The speaker model is still downloading. Try again in a moment."));
        }
        crate::wakeword::forget(&self.core);
        let c = self.converse.get_or_insert_with(|| Converse { step: "voice".into(), screen: json!({ "step": "voice" }), reading: None, busy: false });
        c.reading = Some(Reading { text: text.clone(), done: Vec::new(), current: Vec::new() });
        self.disarm(Timer::Reading);
        self.disarm(Timer::ReadingLong);
        self.listen();
        self.core.hub.emit("speaker.enroll", json!({ "phase": "listening", "count": 0, "needed": 3, "message": null }));
        Ok(json!({ "listening": self.conversing() }))
    }

    pub(super) fn on_utterance(&mut self, samples: Vec<f32>) {
        if !self.conversing() || self.echoing() {
            return;
        }
        let reading = self.converse.as_ref().is_some_and(|c| c.reading.is_some());
        if reading {
            if let Some(r) = self.converse.as_mut().and_then(|c| c.reading.as_mut()) {
                r.current.extend_from_slice(&samples);
                let secs = r.current.len() as f32 / crate::audio::SR as f32;
                self.core.hub.emit("speaker.enroll", json!({ "phase": "hearing", "count": r.done.len(), "needed": 3, "seconds": secs, "message": null }));
            }
            self.arm(Timer::Reading, Duration::from_millis(1300));
            self.arm(Timer::ReadingLong, Duration::from_millis(3500));
            return;
        }
        self.clip(Clip::Utterance, samples);
    }

    fn clip(&mut self, kind: Clip, samples: Vec<f32>) {
        let Some(c) = self.converse.as_mut() else { return };
        if c.busy && kind == Clip::Utterance {
            return;
        }
        c.busy = true;
        self.core.hub.emit("onboarding.thinking", json!({ "busy": true }));
        let core = self.core.clone();
        tokio::spawn(async move {
            let (text, online) = crate::pipeline::transcribe(&core, samples.clone()).await.unwrap_or_default();
            core.tell(ControlMsg::Clip { kind, text, online, samples });
        });
    }

    pub(super) fn on_reading_timer(&mut self, long: bool) {
        let Some(r) = self.converse.as_mut().and_then(|c| c.reading.as_mut()) else { return };
        let secs = r.current.len() as f32 / crate::audio::SR as f32;
        let expected = r.text.split_whitespace().count() as f32 / 2.6;
        if !long && secs < expected * 0.6 && secs >= 0.9 {
            return;
        }
        let audio = std::mem::take(&mut r.current);
        self.disarm(Timer::Reading);
        self.disarm(Timer::ReadingLong);
        if secs >= READ_MIN_SECS {
            self.clip(Clip::Reading, audio);
        } else if secs >= 0.35 {
            self.clip(Clip::Utterance, audio);
        }
    }

    pub(super) fn on_clip(&mut self, kind: Clip, text: String, online: String, samples: Vec<f32>) {
        let Some(c) = self.converse.as_mut() else { return };
        c.busy = false;
        self.core.hub.emit("onboarding.thinking", json!({ "busy": false }));
        let t = text.trim().to_owned();
        if t.is_empty() {
            return;
        }
        self.core.hub.emit("onboarding.heard", json!({ "text": text::sentence_case(&t) }));
        if kind == Clip::Reading {
            let accepted = self.converse.as_mut().and_then(|c| c.reading.as_mut()).map(|r| {
                let sim = similarity(&t, &r.text);
                log::info!("enrollment reading matched {:.0}%", sim * 100.0);
                if sim >= 0.35 {
                    r.done.push(samples);
                    Some((r.done.len(), r.done.clone()))
                } else {
                    None
                }
            });
            match accepted {
                Some(Some((n, all))) => {
                    let first: String = online.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
                    self.learn_wake(&first);
                    self.core.hub.emit("speaker.enroll", json!({ "phase": "sample", "count": n, "needed": 3, "message": null }));
                    if n >= 3 {
                        let core = self.core.clone();
                        crate::layers::blocking(move || {
                            let res = crate::speaker::enroll(&core, &all);
                            core.tell(ControlMsg::Enrolled(res));
                        });
                    } else {
                        self.say_onboard(if n == 1 { "That's one. Please read it once more." } else { "Two done. One last time." });
                    }
                    return;
                }
                Some(None) if t.split_whitespace().count() > 8 => {
                    self.say_onboard("I couldn't follow that. Please read the paragraph on the screen.");
                    return;
                }
                _ => {}
            }
        }
        if let Some(a) = self.quick_intent(&t) {
            self.intent(&a, "");
            return;
        }
        if !self.brain_ready() {
            return;
        }
        let Some(c) = self.converse.as_mut() else { return };
        c.busy = true;
        let step = c.step.clone();
        let screen = c.screen.clone();
        let core = self.core.clone();
        tokio::spawn(async move {
            let opts = options(&screen);
            let mut ids: Vec<String> = opts.iter().map(|o| o.0.clone()).collect();
            ids.extend(["next", "back", "repeat", "none"].map(String::from));
            let system = "You are Setu, a friendly voice assistant. A person is setting you up and just said something while looking at a screen. \
Choose the one action id from the list that best matches what they want. The words come from speech recognition and may be slightly wrong. \
Use next only when they clearly want to continue or agree. Use repeat when they ask what the screen is or ask a question about it. Use none when they talk about something unrelated. \
Also write a short, warm spoken reply under 15 words that confirms the action or answers their question. Never use emoji.\n\
Examples:\n\
They said: take me back -> {\"action\": \"back\", \"reply\": \"Sure, going back.\"}\n\
They said: yes that's fine, carry on -> {\"action\": \"next\", \"reply\": \"Great, let's continue.\"}\n\
They said: what am I supposed to do here -> {\"action\": \"repeat\", \"reply\": \"Let me explain.\"}\n\
They said: my tea is getting cold -> {\"action\": \"none\", \"reply\": \"Sorry, I didn't get that.\"}";
            let mut user = format!("Screen: {}. {}\n", screen["title"].as_str().unwrap_or(""), screen["summary"].as_str().unwrap_or(""));
            if let Some(st) = screen["state"].as_object().filter(|o| !o.is_empty()) {
                user.push_str(&format!("Now on screen: {}\n", Value::Object(st.clone())));
            }
            user.push_str("Actions:\n");
            for (id, label, _) in &opts {
                user.push_str(&format!("- {id}: {label}\n"));
            }
            user.push_str("- next: continue to the next screen\n- back: go back to the previous screen\n- repeat: explain this screen or answer a question about it\n- none: nothing here matches\n");
            user.push_str(&format!("They said: {t}"));
            let (action, reply) = match crate::pipeline::ask_json(&core, system, &user, Some(crate::pipeline::choice_grammar(&ids)), 80, 0.0).await {
                Ok(v) => (v["action"].as_str().unwrap_or("none").to_owned(), v["reply"].as_str().unwrap_or("").to_owned()),
                Err(e) => {
                    log::warn!("could not interpret a setup answer: {e}");
                    ("none".to_owned(), String::new())
                }
            };
            let action = if ids.contains(&action) { action } else { "none".to_owned() };
            core.tell(ControlMsg::Interpreted { step, action, reply });
        });
    }

    pub(super) fn on_interpreted(&mut self, step: String, action: String, reply: String) {
        let Some(c) = self.converse.as_mut() else { return };
        c.busy = false;
        if c.step != step {
            return;
        }
        if action == "none" {
            self.say_onboard(if reply.trim().is_empty() { "Sorry, I didn't catch that." } else { &reply });
            return;
        }
        self.intent(&action, &reply);
    }

    fn intent(&mut self, action: &str, reply: &str) {
        log::info!("setup answer: {action}");
        if action == "repeat" {
            if let Some(c) = self.converse.as_ref() {
                let (step, screen) = (c.step.clone(), c.screen.clone());
                self.explain(step, screen);
            }
            return;
        }
        self.core.hub.emit("onboarding.intent", json!({ "action": action }));
        if !reply.trim().is_empty() {
            self.say_onboard(reply);
        }
    }

    fn quick_intent(&self, heard: &str) -> Option<String> {
        let t = text::normalize(heard);
        let t = t.trim_start_matches("setu ").trim_start_matches("okay ").trim_start_matches("ok ").trim_end_matches(" please").trim_start_matches("please ").trim().to_owned();
        if t.is_empty() {
            return None;
        }
        let c = self.converse.as_ref()?;
        for (id, label, phrases) in options(&c.screen) {
            let l = text::normalize(&label);
            if t == l || phrases.iter().any(|p| text::normalize(p) == t) {
                return Some(id);
            }
        }
        if NEXT.contains(&t.as_str()) {
            return Some("next".into());
        }
        if BACK.contains(&t.as_str()) {
            return Some("back".into());
        }
        if REPEAT.contains(&t.as_str()) {
            return Some("repeat".into());
        }
        let said = content(&t);
        if said.is_empty() {
            return None;
        }
        let mut best: Option<(usize, String)> = None;
        let mut tie = false;
        for (id, label, phrases) in options(&c.screen) {
            let mut words = content(&label);
            for p in &phrases {
                words.extend(content(p));
            }
            let score = said.iter().filter(|w| words.contains(w)).count();
            if score == 0 {
                continue;
            }
            match &best {
                Some((b, _)) if score < *b => {}
                Some((b, _)) if score == *b => tie = true,
                _ => {
                    best = Some((score, id));
                    tie = false;
                }
            }
        }
        if tie { None } else { best.map(|b| b.1) }
    }

    pub(super) fn finish_reading(&mut self, ok: bool) {
        let reading = self.converse.as_mut().and_then(|c| c.reading.take()).is_some();
        if !reading {
            return;
        }
        if ok {
            let core = self.core.clone();
            tokio::spawn(async move {
                let _ = crate::handlers::settings_set(&core, &json!({ "key": "speaker.verification", "value": true })).await;
            });
            self.say_onboard("Done. I know your voice now, so I'll only respond to you.");
        } else {
            self.say_onboard("Sorry, I couldn't learn your voice. Let's try again in a quieter spot.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_words_ignore_filler() {
        assert_eq!(content("I'd like a male voice instead"), vec!["male", "voice"]);
        assert_eq!(content("can you use a bigger model"), vec!["bigger", "model"]);
        assert!(content("go on then").is_empty());
    }

    #[test]
    fn readings_are_recognised() {
        let p = PARAGRAPHS[0];
        assert!(similarity("hey say to good morning today i want to plan my day calmly i will check my messages", p) > 0.5);
        assert!(similarity("go back", p) < 0.35);
        for p in PARAGRAPHS {
            assert!(valid_paragraph(p), "{p}");
        }
        assert!(!valid_paragraph("Hello there."));
        let long = "Hey Setu, hello. I am a young Indian boy named Aarav, and I eat breakfast at home. I like rice and dal every morning with my family. Then I drink a cup of sweet tea and read the newspaper slowly. After that I walk to the market to buy fresh vegetables and fruit for the day ahead. It is busy there.";
        let shaped = shape_paragraph(long).unwrap();
        assert!(shaped.starts_with("Hey Setu, hello."), "{shaped}");
        assert!(!shaped.contains("Aarav"));
        assert!(shaped.split_whitespace().count() <= 48);
        assert!(shape_paragraph("Hello, I am here.").is_none());
    }
}
