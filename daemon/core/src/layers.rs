use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    System,
    Channel,
    Protocol,
    Accessibility,
    Keyboard,
    Pointer,
    Vision,
    Service,
    Command,
    Setu,
}

impl Layer {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Channel => "channel",
            Self::Protocol => "protocol",
            Self::Accessibility => "accessibility",
            Self::Keyboard => "keyboard",
            Self::Pointer => "pointer",
            Self::Vision => "vision",
            Self::Service => "service",
            Self::Command => "command",
            Self::Setu => "setu",
        }
    }
}

const KEEP: usize = 400;

struct Ring {
    stage: &'static str,
    values: VecDeque<u64>,
}

static STAGES: Mutex<Vec<Ring>> = Mutex::new(Vec::new());

pub fn record(stage: &'static str, us: u64) {
    let mut g = STAGES.lock().unwrap();
    let i = match g.iter().position(|r| r.stage == stage) {
        Some(i) => i,
        None => {
            g.push(Ring { stage, values: VecDeque::new() });
            g.len() - 1
        }
    };
    let q = &mut g[i].values;
    if q.len() == KEEP {
        q.pop_front();
    }
    q.push_back(us);
}

pub fn latency() -> Value {
    let g = STAGES.lock().unwrap();
    let mut m = Map::new();
    for r in g.iter() {
        if r.values.is_empty() {
            continue;
        }
        let mut v: Vec<u64> = r.values.iter().copied().collect();
        v.sort_unstable();
        let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        m.insert(r.stage.into(), json!({ "lastUs": r.values.back(), "p50Us": pct(0.5), "p95Us": pct(0.95), "maxUs": v[v.len() - 1], "count": v.len() }));
    }
    Value::Object(m)
}

pub fn samples() -> Value {
    let g = STAGES.lock().unwrap();
    let mut m = Map::new();
    for r in g.iter() {
        m.insert(r.stage.into(), json!(r.values.iter().copied().collect::<Vec<u64>>()));
    }
    Value::Object(m)
}

#[derive(Default)]
struct State {
    sent_us: Option<u64>,
    tried: Vec<Layer>,
    served: Option<Layer>,
    fallback: Option<Layer>,
    read: Option<Layer>,
    read_us: Option<u64>,
    once: Vec<&'static str>,
    begun_us: Option<u64>,
    stages: Vec<(&'static str, u64, bool, u64)>,
}

pub struct Trace {
    start: Instant,
    state: Mutex<State>,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub served: Option<Layer>,
    pub tried: Vec<Layer>,
    pub sent_us: Option<u64>,
    pub begun_us: Option<u64>,
    pub stages: Vec<(&'static str, u64, bool, u64)>,
}

impl Report {
    pub fn json(&self) -> Value {
        json!({
            "served": self.served.map(Layer::as_str),
            "tried": self.tried.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
            "sentUs": self.sent_us,
            "begunUs": self.begun_us,
            "stages": self.stages.iter().map(|(s, us, ok, at)| json!({ "stage": s, "us": us, "ok": ok, "atUs": at })).collect::<Vec<_>>(),
        })
    }
}

impl Trace {
    pub fn new(start: Instant) -> Arc<Self> {
        Arc::new(Self { start, state: Mutex::new(State::default()) })
    }

    pub fn begun(&self) {
        let mut g = self.state.lock().unwrap();
        if g.begun_us.is_none() {
            g.begun_us = Some(self.start.elapsed().as_micros() as u64);
        }
    }

    fn looked(&self) {
        let mut g = self.state.lock().unwrap();
        if g.read_us.is_none() {
            g.read_us = Some(self.start.elapsed().as_micros() as u64);
        }
    }

    fn touch(&self, layer: Layer) {
        let mut g = self.state.lock().unwrap();
        if g.sent_us.is_none() {
            g.sent_us = Some(self.start.elapsed().as_micros() as u64);
        }
        if !g.tried.contains(&layer) {
            g.tried.push(layer);
        }
    }

    fn close(&self, layer: Layer, stage: &'static str, t0: Instant, us: u64, effect: bool, ok: bool) {
        let at = t0.saturating_duration_since(self.start).as_micros() as u64;
        let mut g = self.state.lock().unwrap();
        if effect && ok {
            g.served = Some(layer);
        }
        if !effect && ok {
            g.read = Some(layer);
        }
        if g.stages.len() < 64 {
            g.stages.push((stage, us, ok, at));
        }
    }

    pub fn report(&self) -> Report {
        let g = self.state.lock().unwrap();
        let mut tried = g.tried.clone();
        let served = g.served.or(g.fallback).or(g.read);
        if g.served.is_none()
            && let Some(f) = served
            && !tried.contains(&f)
        {
            tried.push(f);
        }
        Report { served, tried, sent_us: g.sent_us.or(g.read_us), begun_us: g.begun_us, stages: g.stages.clone() }
    }
}

tokio::task_local! {
    static TASK: Arc<Trace>;
}

thread_local! {
    static THREAD: RefCell<Option<Arc<Trace>>> = const { RefCell::new(None) };
}

fn current() -> Option<Arc<Trace>> {
    TASK.try_with(|t| t.clone()).ok().or_else(|| THREAD.with(|t| t.borrow().clone()))
}

pub async fn scope<F: Future>(trace: Arc<Trace>, f: F) -> F::Output {
    TASK.scope(trace, f).await
}

struct Restore(Option<Arc<Trace>>);

impl Drop for Restore {
    fn drop(&mut self) {
        let prev = self.0.take();
        THREAD.with(|c| *c.borrow_mut() = prev);
    }
}

pub fn blocking<F, R>(f: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let trace = current();
    tokio::task::spawn_blocking(move || {
        let _restore = Restore(THREAD.with(|c| c.replace(trace)));
        f()
    })
}

pub fn traced<F, R>(f: F) -> impl FnOnce() -> R + Send + 'static
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let trace = current();
    move || {
        let _restore = Restore(THREAD.with(|c| c.replace(trace)));
        f()
    }
}

#[allow(dead_code)]
pub fn spawn<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match current() {
        Some(t) => tokio::spawn(TASK.scope(t, f)),
        None => tokio::spawn(f),
    }
}

pub struct Span {
    layer: Layer,
    stage: &'static str,
    effect: bool,
    t0: Instant,
    trace: Option<Arc<Trace>>,
    open: bool,
}

fn span(layer: Layer, stage: &'static str, effect: bool) -> Span {
    let trace = current();
    if let Some(t) = &trace {
        if effect {
            t.touch(layer);
        } else {
            t.looked();
        }
    }
    Span { layer, stage, effect, t0: Instant::now(), trace, open: true }
}

pub fn effect(layer: Layer, stage: &'static str) -> Span {
    span(layer, stage, true)
}

pub fn read(layer: Layer, stage: &'static str) -> Span {
    span(layer, stage, false)
}

impl Span {
    fn finish(&mut self, ok: bool) {
        if !self.open {
            return;
        }
        self.open = false;
        let us = self.t0.elapsed().as_micros() as u64;
        record(self.stage, us);
        if let Some(t) = &self.trace {
            t.close(self.layer, self.stage, self.t0, us, self.effect, ok);
        }
    }

    pub fn set(&mut self, layer: Layer, stage: &'static str) {
        self.layer = layer;
        self.stage = stage;
        if self.effect && let Some(t) = &self.trace {
            t.touch(layer);
        }
    }

    pub fn ok(mut self) {
        self.finish(true);
    }

    pub fn forget(mut self) {
        self.open = false;
    }

    pub fn end<T, E>(mut self, r: Result<T, E>) -> Result<T, E> {
        self.finish(r.is_ok());
        r
    }

    pub fn end_bool(mut self, ok: bool) -> bool {
        self.finish(ok);
        ok
    }

    pub fn end_some<T>(mut self, r: Option<T>) -> Option<T> {
        self.finish(r.is_some());
        r
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        self.finish(false);
    }
}

#[allow(dead_code)]
pub fn note(layer: Layer, stage: &'static str) {
    effect(layer, stage).ok();
}

pub fn once(key: &'static str) -> bool {
    match current() {
        Some(t) => {
            let mut g = t.state.lock().unwrap();
            if g.once.contains(&key) {
                false
            } else {
                g.once.push(key);
                true
            }
        }
        None => false,
    }
}

pub fn fallback(layer: Layer) {
    if let Some(t) = current() {
        let mut g = t.state.lock().unwrap();
        g.fallback = Some(layer);
        if g.sent_us.is_none() {
            g.sent_us = Some(t.start.elapsed().as_micros() as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_last_effect_that_worked_served_the_action() {
        let t = Trace::new(Instant::now());
        scope(t.clone(), async {
            let s = effect(Layer::Accessibility, "test.accessibility.act");
            let _ = s.end::<(), ()>(Err(()));
            read(Layer::Accessibility, "test.accessibility.find").ok();
            let h = blocking(|| effect(Layer::Keyboard, "test.keyboard.press").ok());
            h.await.unwrap();
        })
        .await;
        let r = t.report();
        assert_eq!(r.served, Some(Layer::Keyboard));
        assert_eq!(r.tried, vec![Layer::Accessibility, Layer::Keyboard]);
        assert!(r.sent_us.is_some());
        assert_eq!(r.stages.len(), 3);
        assert!(latency().get("test.keyboard.press").is_some());
    }

    #[tokio::test]
    async fn work_outside_an_action_is_timed_but_not_attributed() {
        let t = Trace::new(Instant::now());
        note(Layer::Command, "test.command.outside");
        let h = tokio::spawn(async { note(Layer::Command, "test.command.spawned") });
        h.await.unwrap();
        scope(t.clone(), async {
            spawn(async { note(Layer::System, "test.system.inside") }).await.unwrap();
        })
        .await;
        let r = t.report();
        assert_eq!(r.tried, vec![Layer::System]);
        assert_eq!(r.served, Some(Layer::System));
        assert!(latency().get("test.command.outside").is_some());
    }
}
