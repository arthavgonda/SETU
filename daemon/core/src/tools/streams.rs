use anyhow::Result;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stream {
    pub index: u32,
    pub app: String,
    pub title: String,
    pub pid: Option<u32>,
    pub binary: String,
    pub corked: bool,
    pub muted: bool,
    pub role: String,
    pub volume: f64,
    pub channels: u8,
    #[serde(default)]
    pub sink: u32,
    #[serde(default)]
    pub sounding: Option<bool>,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const LOUD: f32 = 0.003;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const QUIET_AFTER: Duration = Duration::from_secs(3);
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const SETTLE: Duration = Duration::from_millis(500);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
pub struct Level {
    pub started: std::time::Instant,
    pub loud: Option<std::time::Instant>,
    pub told: Option<bool>,
    pub fed: bool,
}

impl Level {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn state(&self, now: std::time::Instant) -> Option<bool> {
        match self.loud {
            Some(t) if now.duration_since(t) < QUIET_AFTER => Some(true),
            _ if self.fed && now.duration_since(self.started) >= SETTLE => Some(false),
            _ => None,
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sink {
    pub name: String,
    pub description: String,
    pub default: bool,
    pub volume: f64,
    pub muted: bool,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const IDLE: Duration = Duration::from_secs(30);

static ON_CHANGE: Mutex<Option<fn()>> = Mutex::new(None);

pub fn on_change(f: fn()) {
    *ON_CHANGE.lock().unwrap() = Some(f);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn changed() {
    if let Some(f) = *ON_CHANGE.lock().unwrap() {
        f();
    }
}

pub fn own(s: &Stream) -> bool {
    let me = std::process::id();
    s.pid == Some(me) || s.app.eq_ignore_ascii_case("setu") || s.binary.starts_with("setu")
}

pub async fn list() -> Result<Vec<Stream>> {
    let all = imp::list().await?;
    Ok(all.into_iter().filter(|s| !own(s)).collect())
}

pub async fn warm() {
    let _ = imp::list().await;
}

pub async fn settle() {
    let Ok(mine) = list().await else { return };
    if !mine.iter().any(|s| !s.corked && !s.muted && s.sounding.is_none()) || !imp::metering() {
        return;
    }
    #[cfg(windows)]
    {
        let _ = crate::layers::blocking(|| imp::listen(SETTLE + Duration::from_millis(150))).await;
    }
    #[cfg(not(windows))]
    tokio::time::sleep(SETTLE + Duration::from_millis(100)).await;
}

pub async fn set_mute(index: u32, mute: bool) -> Result<()> {
    imp::set_mute(index, mute).await
}

pub async fn set_volume(s: &Stream, level: f64) -> Result<()> {
    imp::set_volume(s.index, s.channels.max(1), level.clamp(0.0, 1.5)).await
}

const DUCK_TO: f64 = 0.3;
static DUCKED: Mutex<Vec<(u32, f64, f64)>> = Mutex::new(Vec::new());
static DUCKING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn ducked() -> bool {
    !DUCKED.lock().unwrap().is_empty()
}

pub async fn duck(on: bool) {
    let _one = DUCKING.lock().await;
    if on {
        if ducked() {
            return;
        }
        let Ok(all) = list().await else { return };
        let mut done = Vec::new();
        for st in all.iter().filter(|s| !s.corked && !s.muted && s.sounding != Some(false) && s.volume > 0.05) {
            let to = (st.volume * DUCK_TO).max(0.02);
            if set_volume(st, to).await.is_ok() {
                done.push((st.index, st.volume, to));
            }
        }
        *DUCKED.lock().unwrap() = done;
        return;
    }
    let saved = std::mem::take(&mut *DUCKED.lock().unwrap());
    if saved.is_empty() {
        return;
    }
    let now = list().await.unwrap_or_default();
    for (index, before, set) in saved {
        if let Some(st) = now.iter().find(|s| s.index == index)
            && (st.volume - set).abs() < 0.03
        {
            let _ = set_volume(st, before).await;
        }
    }
}

#[cfg(target_os = "linux")]
pub fn sinks() -> Result<Vec<Sink>> {
    imp::sinks()
}

#[cfg(target_os = "linux")]
pub fn set_output(name: &str) -> Result<()> {
    imp::set_output(name)
}

#[cfg(target_os = "linux")]
pub fn master(level: Option<f64>, mute: Option<bool>) -> Result<Sink> {
    imp::master(level, mute)
}

pub fn connected() -> bool {
    imp::connected()
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Sink, Stream};
    use anyhow::{Result, anyhow};
    use libpulse_binding as pulse;
    use pulse::callbacks::ListResult;
    use pulse::context::subscribe::{Facility, InterestMaskSet};
    use pulse::context::{Context, FlagSet, State};
    use pulse::mainloop::threaded::Mainloop;
    use pulse::proplist::Proplist;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};

    enum Req {
        List(tokio::sync::oneshot::Sender<Result<Vec<Stream>>>),
        Mute(u32, bool, tokio::sync::oneshot::Sender<Result<()>>),
        Volume(u32, u8, f64, tokio::sync::oneshot::Sender<Result<()>>),
        Sinks(Sender<Result<Vec<Sink>>>),
        Output(String, Sender<Result<()>>),
        Master(Option<f64>, Option<bool>, Sender<Result<Sink>>),
    }

    fn ask<T>(make: impl Fn(Sender<Result<T>>) -> Req, label: &'static str, effect: bool) -> Result<T> {
        let (tx, rx) = channel();
        let slot = Mutex::new(Some(tx));
        send(|| make(slot.lock().unwrap().take().unwrap_or_else(|| channel().0)))?;
        let span = if effect { crate::layers::effect(crate::layers::Layer::System, label) } else { crate::layers::read(crate::layers::Layer::System, label) };
        span.end(rx.recv_timeout(std::time::Duration::from_secs(2)).map_err(|_| anyhow!("The sound server took too long."))?)
    }

    pub fn sinks() -> Result<Vec<Sink>> {
        ask(Req::Sinks, "system.sound.outputs", false)
    }

    pub fn set_output(name: &str) -> Result<()> {
        let n = name.to_owned();
        ask(move |tx| Req::Output(n.clone(), tx), "system.sound.output", true)
    }

    pub fn master(level: Option<f64>, mute: Option<bool>) -> Result<Sink> {
        ask(move |tx| Req::Master(level, mute, tx), if level.is_some() || mute.is_some() { "system.volume" } else { "system.volume.read" }, level.is_some() || mute.is_some())
    }

    static TX: Mutex<Option<Sender<Req>>> = Mutex::new(None);

    pub fn connected() -> bool {
        TX.lock().unwrap().is_some()
    }

    fn sender() -> Result<Sender<Req>> {
        let mut g = TX.lock().unwrap();
        if let Some(tx) = g.as_ref() {
            return Ok(tx.clone());
        }
        let (tx, rx) = channel();
        std::thread::Builder::new().name("setu-sound".into()).spawn(move || worker(rx)).map_err(|e| anyhow!("Couldn't watch the sound server: {e}"))?;
        *g = Some(tx.clone());
        Ok(tx)
    }

    fn send(make: impl Fn() -> Req) -> Result<()> {
        for _ in 0..2 {
            let tx = sender()?;
            if tx.send(make()).is_ok() {
                return Ok(());
            }
            TX.lock().unwrap().take();
        }
        Err(anyhow!("The sound server isn't answering."))
    }

    pub async fn list() -> Result<Vec<Stream>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let slot = Mutex::new(Some(tx));
        send(|| Req::List(slot.lock().unwrap().take().unwrap_or_else(|| tokio::sync::oneshot::channel().0)))?;
        let s = crate::layers::read(crate::layers::Layer::System, "system.sound.streams");
        s.end(tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.map_err(|_| anyhow!("The sound server took too long."))?.map_err(|_| anyhow!("The sound server stopped answering."))?)
    }

    pub async fn set_mute(index: u32, mute: bool) -> Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let slot = Mutex::new(Some(tx));
        send(|| Req::Mute(index, mute, slot.lock().unwrap().take().unwrap_or_else(|| tokio::sync::oneshot::channel().0)))?;
        let s = crate::layers::effect(crate::layers::Layer::System, "system.sound.mute");
        s.end(tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.map_err(|_| anyhow!("The sound server took too long."))?.map_err(|_| anyhow!("The sound server stopped answering."))?)
    }

    pub async fn set_volume(index: u32, channels: u8, level: f64) -> Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let slot = Mutex::new(Some(tx));
        send(|| Req::Volume(index, channels, level, slot.lock().unwrap().take().unwrap_or_else(|| tokio::sync::oneshot::channel().0)))?;
        let s = crate::layers::effect(crate::layers::Layer::System, "system.sound.volume");
        s.end(tokio::time::timeout(std::time::Duration::from_secs(2), rx).await.map_err(|_| anyhow!("The sound server took too long."))?.map_err(|_| anyhow!("The sound server stopped answering."))?)
    }

    struct Conn {
        ml: Rc<RefCell<Mainloop>>,
        ctx: Rc<RefCell<Context>>,
        meters: RefCell<std::collections::HashMap<u32, Meter>>,
        monitors: RefCell<std::collections::HashMap<u32, u32>>,
        sinks: RefCell<Option<Vec<Sink>>>,
    }

    static SINKS_STALE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

    fn sinks_now(c: &Conn) -> Result<Vec<Sink>> {
        use std::sync::atomic::Ordering::SeqCst;
        if !SINKS_STALE.load(SeqCst)
            && let Some(v) = c.sinks.borrow().clone()
        {
            return Ok(v);
        }
        SINKS_STALE.store(false, SeqCst);
        let v = read_sinks(c)?;
        *c.sinks.borrow_mut() = (!SINKS_STALE.load(SeqCst)).then(|| v.clone());
        Ok(v)
    }

    fn sinks_changed(c: &Conn) {
        SINKS_STALE.store(true, std::sync::atomic::Ordering::SeqCst);
        c.sinks.borrow_mut().take();
    }

    fn open() -> Result<Conn> {
        let ml = Rc::new(RefCell::new(Mainloop::new().ok_or_else(|| anyhow!("No sound server main loop."))?));
        let mut props = Proplist::new().ok_or_else(|| anyhow!("No property list."))?;
        let _ = props.set_str("application.name", "Setu");
        let _ = props.set_str("application.id", "ai.setu.daemon");
        let ctx = Rc::new(RefCell::new(Context::new_with_proplist(&*ml.borrow(), "Setu", &props).ok_or_else(|| anyhow!("No sound server context."))?));
        {
            let ml2 = ml.clone();
            ctx.borrow_mut().set_state_callback(Some(Box::new(move || unsafe { (*ml2.as_ptr()).signal(false) })));
        }
        ctx.borrow_mut().connect(None, FlagSet::NOAUTOSPAWN, None).map_err(|e| anyhow!("The sound server isn't running: {e}"))?;
        ml.borrow_mut().lock();
        if let Err(e) = ml.borrow_mut().start() {
            ml.borrow_mut().unlock();
            return Err(anyhow!("The sound server loop didn't start: {e}"));
        }
        loop {
            match ctx.borrow().get_state() {
                State::Ready => break,
                State::Failed | State::Terminated => {
                    ml.borrow_mut().unlock();
                    ml.borrow_mut().stop();
                    return Err(anyhow!("The sound server refused the connection."));
                }
                _ => ml.borrow_mut().wait(),
            }
        }
        ctx.borrow_mut().set_state_callback(None);
        SINKS_STALE.store(true, std::sync::atomic::Ordering::SeqCst);
        ctx.borrow_mut().set_subscribe_callback(Some(Box::new(|fac, _op, _idx| match fac {
            Some(Facility::SinkInput) => super::changed(),
            Some(Facility::Sink) | Some(Facility::Server) => SINKS_STALE.store(true, std::sync::atomic::Ordering::SeqCst),
            _ => {}
        })));
        ctx.borrow_mut().subscribe(InterestMaskSet::SINK_INPUT | InterestMaskSet::SINK | InterestMaskSet::SERVER, |_| {});
        ml.borrow_mut().unlock();
        Ok(Conn { ml, ctx, meters: RefCell::new(std::collections::HashMap::new()), monitors: RefCell::new(std::collections::HashMap::new()), sinks: RefCell::new(None) })
    }

    fn read(c: &Conn) -> Result<Vec<Stream>> {
        let out = Rc::new(RefCell::new(Vec::new()));
        let done = Rc::new(RefCell::new(false));
        c.ml.borrow_mut().lock();
        let op = {
            let out = out.clone();
            let done = done.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().get_sink_input_info_list(move |r| match r {
                ListResult::Item(i) => {
                    let p = &i.proplist;
                    let app = p.get_str("application.name").unwrap_or_default();
                    out.borrow_mut().push(Stream {
                        index: i.index,
                        app: if app.is_empty() { i.name.as_deref().unwrap_or("").to_owned() } else { app },
                        title: p.get_str("media.name").unwrap_or_default(),
                        pid: p.get_str("application.process.id").and_then(|v| v.parse().ok()),
                        binary: p.get_str("application.process.binary").unwrap_or_default(),
                        corked: i.corked,
                        muted: i.mute,
                        role: p.get_str("media.role").unwrap_or_default(),
                        volume: i.volume.avg().0 as f64 / pulse::volume::Volume::NORMAL.0 as f64,
                        channels: i.volume.len(),
                        sink: i.sink,
                        sounding: None,
                    });
                }
                _ => {
                    *done.borrow_mut() = true;
                    unsafe { (*ml.as_ptr()).signal(false) };
                }
            })
        };
        while !*done.borrow() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        let mut v = out.borrow().clone();
        sync_meters(c, &v);
        let now = std::time::Instant::now();
        Levels::with(|m| {
            for st in v.iter_mut() {
                st.sounding = m.get(&st.index).and_then(|l| l.state(now));
            }
        });
        Ok(v)
    }

    static LEVELS: Mutex<Option<std::collections::HashMap<u32, super::Level>>> = Mutex::new(None);

    struct Levels;

    impl Levels {
        fn with<R>(f: impl FnOnce(&mut std::collections::HashMap<u32, super::Level>) -> R) -> R {
            let mut g = LEVELS.lock().unwrap();
            f(g.get_or_insert_with(std::collections::HashMap::new))
        }
    }

    pub fn metering() -> bool {
        Levels::with(|m| !m.is_empty())
    }

    fn heard(index: u32, peak: f32) {
        let now = std::time::Instant::now();
        let flipped = Levels::with(|m| {
            let Some(l) = m.get_mut(&index) else { return false };
            l.fed = true;
            if peak >= super::LOUD {
                l.loud = Some(now);
            }
            let st = l.state(now);
            if st.is_some() && st != l.told {
                l.told = st;
                return true;
            }
            false
        });
        if flipped {
            super::changed();
        }
    }

    pub fn flips() -> bool {
        let now = std::time::Instant::now();
        Levels::with(|m| {
            let mut any = false;
            for l in m.values_mut() {
                let st = l.state(now);
                if st.is_some() && st != l.told {
                    l.told = st;
                    any = true;
                }
            }
            any
        })
    }

    struct Meter {
        stream: Rc<RefCell<pulse::stream::Stream>>,
    }

    fn monitor_of(c: &Conn, sink: u32) -> Option<u32> {
        if let Some(m) = c.monitors.borrow().get(&sink) {
            return Some(*m);
        }
        let out = Rc::new(RefCell::new(None::<Option<u32>>));
        c.ml.borrow_mut().lock();
        let op = {
            let out = out.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().get_sink_info_by_index(sink, move |r| {
                match r {
                    ListResult::Item(i) => *out.borrow_mut() = Some(Some(i.monitor_source)),
                    _ => {
                        if out.borrow().is_none() {
                            *out.borrow_mut() = Some(None);
                        }
                    }
                }
                unsafe { (*ml.as_ptr()).signal(false) };
            })
        };
        while out.borrow().is_none() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        let m = out.borrow().flatten();
        if let Some(m) = m {
            c.monitors.borrow_mut().insert(sink, m);
        }
        m
    }

    fn open_meter(c: &Conn, index: u32, source: u32) -> Option<Meter> {
        use pulse::stream::{FlagSet as SF, PeekResult, Stream as PaStream};
        let spec = pulse::sample::Spec { format: pulse::sample::Format::F32le, channels: 1, rate: 25 };
        c.ml.borrow_mut().lock();
        let made = (|| {
            let mut st = PaStream::new(&mut c.ctx.borrow_mut(), "Setu sound meter", &spec, None)?;
            st.set_monitor_stream(index).ok()?;
            let rc = Rc::new(RefCell::new(st));
            let weak = Rc::downgrade(&rc);
            rc.borrow_mut().set_read_callback(Some(Box::new(move |_| {
                let Some(s) = weak.upgrade() else { return };
                let Ok(mut s) = s.try_borrow_mut() else { return };
                let mut peak = 0f32;
                let mut got = false;
                while let Ok(r) = s.peek() {
                    match r {
                        PeekResult::Data(d) => {
                            for ch in d.chunks_exact(4) {
                                peak = peak.max(f32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]).abs());
                            }
                            got = true;
                            let _ = s.discard();
                        }
                        PeekResult::Hole(_) => {
                            let _ = s.discard();
                        }
                        PeekResult::Empty => break,
                    }
                }
                if got {
                    heard(index, peak);
                }
            })));
            let attr = pulse::def::BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: 4 };
            let dev = source.to_string();
            rc.borrow_mut().connect_record(Some(&dev), Some(&attr), SF::DONT_MOVE | SF::PEAK_DETECT | SF::ADJUST_LATENCY).ok()?;
            Some(Meter { stream: rc })
        })();
        c.ml.borrow_mut().unlock();
        made
    }

    fn close_meter(c: &Conn, m: Meter) {
        c.ml.borrow_mut().lock();
        if let Ok(mut s) = m.stream.try_borrow_mut() {
            s.set_read_callback(None);
            let _ = s.disconnect();
        }
        drop(m);
        c.ml.borrow_mut().unlock();
    }

    fn sync_meters(c: &Conn, streams: &[Stream]) {
        let wanted: Vec<&Stream> = streams.iter().filter(|s| !s.corked && !s.muted && !super::own(s)).collect();
        let gone: Vec<u32> = c.meters.borrow().keys().filter(|k| !wanted.iter().any(|s| s.index == **k)).copied().collect();
        for k in gone {
            if let Some(m) = c.meters.borrow_mut().remove(&k) {
                close_meter(c, m);
            }
            Levels::with(|l| l.remove(&k));
        }
        for s in wanted {
            if c.meters.borrow().contains_key(&s.index) {
                continue;
            }
            let Some(src) = monitor_of(c, s.sink) else { continue };
            if let Some(m) = open_meter(c, s.index, src) {
                Levels::with(|l| l.insert(s.index, super::Level { started: std::time::Instant::now(), loud: None, told: None, fed: false }));
                c.meters.borrow_mut().insert(s.index, m);
            }
        }
    }

    fn mute(c: &Conn, index: u32, on: bool) -> Result<()> {
        let ok = Rc::new(RefCell::new(None));
        c.ml.borrow_mut().lock();
        let op = {
            let ok = ok.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().set_sink_input_mute(index, on, Some(Box::new(move |s| {
                *ok.borrow_mut() = Some(s);
                unsafe { (*ml.as_ptr()).signal(false) };
            })))
        };
        while ok.borrow().is_none() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        let r = *ok.borrow();
        match r {
            Some(true) => Ok(()),
            _ => Err(anyhow!("That sound couldn't be muted.")),
        }
    }

    fn volume(c: &Conn, index: u32, channels: u8, level: f64) -> Result<()> {
        let mut v = pulse::volume::ChannelVolumes::default();
        v.set(channels, pulse::volume::Volume((pulse::volume::Volume::NORMAL.0 as f64 * level).round() as u32));
        let ok = Rc::new(RefCell::new(None));
        c.ml.borrow_mut().lock();
        let op = {
            let ok = ok.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().set_sink_input_volume(index, &v, Some(Box::new(move |s| {
                *ok.borrow_mut() = Some(s);
                unsafe { (*ml.as_ptr()).signal(false) };
            })))
        };
        while ok.borrow().is_none() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        let r = *ok.borrow();
        match r {
            Some(true) => Ok(()),
            _ => Err(anyhow!("That sound's volume couldn't be changed.")),
        }
    }

    fn wait_bool(c: &Conn, done: &Rc<RefCell<Option<bool>>>, running: impl Fn() -> bool) {
        while done.borrow().is_none() && running() {
            c.ml.borrow_mut().wait();
        }
    }

    fn default_name(c: &Conn) -> String {
        let out = Rc::new(RefCell::new(None::<String>));
        c.ml.borrow_mut().lock();
        let op = {
            let out = out.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().get_server_info(move |i| {
                *out.borrow_mut() = Some(i.default_sink_name.as_deref().unwrap_or("").to_owned());
                unsafe { (*ml.as_ptr()).signal(false) };
            })
        };
        while out.borrow().is_none() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        out.borrow().clone().unwrap_or_default()
    }

    fn read_sinks(c: &Conn) -> Result<Vec<Sink>> {
        let def = default_name(c);
        let out = Rc::new(RefCell::new(Vec::new()));
        let done = Rc::new(RefCell::new(false));
        c.ml.borrow_mut().lock();
        let op = {
            let out = out.clone();
            let done = done.clone();
            let ml = c.ml.clone();
            c.ctx.borrow().introspect().get_sink_info_list(move |r| match r {
                ListResult::Item(i) => {
                    let name = i.name.as_deref().unwrap_or("").to_owned();
                    out.borrow_mut().push(Sink {
                        default: name == def,
                        description: i.description.as_deref().unwrap_or(&name).to_owned(),
                        name,
                        volume: i.volume.avg().0 as f64 / pulse::volume::Volume::NORMAL.0 as f64,
                        muted: i.mute,
                    });
                }
                _ => {
                    *done.borrow_mut() = true;
                    unsafe { (*ml.as_ptr()).signal(false) };
                }
            })
        };
        while !*done.borrow() && op.get_state() == pulse::operation::State::Running {
            c.ml.borrow_mut().wait();
        }
        c.ml.borrow_mut().unlock();
        let v = out.borrow().clone();
        Ok(v)
    }

    fn output(c: &Conn, name: &str) -> Result<()> {
        let ok = Rc::new(RefCell::new(None));
        c.ml.borrow_mut().lock();
        let op = {
            let ok = ok.clone();
            let ml = c.ml.clone();
            c.ctx.borrow_mut().set_default_sink(name, move |s| {
                *ok.borrow_mut() = Some(s);
                unsafe { (*ml.as_ptr()).signal(false) };
            })
        };
        wait_bool(c, &ok, || op.get_state() == pulse::operation::State::Running);
        c.ml.borrow_mut().unlock();
        if *ok.borrow() != Some(true) {
            return Err(anyhow!("That sound output couldn't be chosen."));
        }
        for st in read(c)? {
            let moved = Rc::new(RefCell::new(None));
            c.ml.borrow_mut().lock();
            let op = {
                let moved = moved.clone();
                let ml = c.ml.clone();
                c.ctx.borrow().introspect().move_sink_input_by_name(st.index, name, Some(Box::new(move |s| {
                    *moved.borrow_mut() = Some(s);
                    unsafe { (*ml.as_ptr()).signal(false) };
                })))
            };
            wait_bool(c, &moved, || op.get_state() == pulse::operation::State::Running);
            c.ml.borrow_mut().unlock();
        }
        Ok(())
    }

    fn master_set(c: &Conn, level: Option<f64>, mute: Option<bool>) -> Result<Sink> {
        let sinks = sinks_now(c)?;
        if level.is_some() || mute.is_some() {
            sinks_changed(c);
        }
        let cur = sinks.iter().find(|s| s.default).or_else(|| sinks.first()).cloned().ok_or_else(|| anyhow!("There is no sound output."))?;
        if let Some(l) = level {
            let channels = {
                let n = Rc::new(RefCell::new(None::<u8>));
                c.ml.borrow_mut().lock();
                let op = {
                    let n = n.clone();
                    let ml = c.ml.clone();
                    c.ctx.borrow().introspect().get_sink_info_by_name(&cur.name, move |r| {
                        if let ListResult::Item(i) = r {
                            *n.borrow_mut() = Some(i.volume.len());
                        }
                        unsafe { (*ml.as_ptr()).signal(false) };
                    })
                };
                while n.borrow().is_none() && op.get_state() == pulse::operation::State::Running {
                    c.ml.borrow_mut().wait();
                }
                c.ml.borrow_mut().unlock();
                n.take().unwrap_or(2)
            };
            let mut v = pulse::volume::ChannelVolumes::default();
            v.set(channels.max(1), pulse::volume::Volume((pulse::volume::Volume::NORMAL.0 as f64 * l.clamp(0.0, 1.0)).round() as u32));
            let ok = Rc::new(RefCell::new(None));
            c.ml.borrow_mut().lock();
            let op = {
                let ok = ok.clone();
                let ml = c.ml.clone();
                c.ctx.borrow().introspect().set_sink_volume_by_name(&cur.name, &v, Some(Box::new(move |s| {
                    *ok.borrow_mut() = Some(s);
                    unsafe { (*ml.as_ptr()).signal(false) };
                })))
            };
            wait_bool(c, &ok, || op.get_state() == pulse::operation::State::Running);
            c.ml.borrow_mut().unlock();
            if *ok.borrow() != Some(true) {
                return Err(anyhow!("The volume couldn't be changed."));
            }
        }
        if let Some(m) = mute {
            let ok = Rc::new(RefCell::new(None));
            c.ml.borrow_mut().lock();
            let op = {
                let ok = ok.clone();
                let ml = c.ml.clone();
                c.ctx.borrow().introspect().set_sink_mute_by_name(&cur.name, m, Some(Box::new(move |s| {
                    *ok.borrow_mut() = Some(s);
                    unsafe { (*ml.as_ptr()).signal(false) };
                })))
            };
            wait_bool(c, &ok, || op.get_state() == pulse::operation::State::Running);
            c.ml.borrow_mut().unlock();
            if *ok.borrow() != Some(true) {
                return Err(anyhow!("The sound couldn't be muted."));
            }
        }
        if level.is_none() && mute.is_none() {
            return Ok(cur);
        }
        sinks_now(c)?.into_iter().find(|s| s.name == cur.name).ok_or_else(|| anyhow!("The sound output went away."))
    }

    fn close(c: Conn) {
        let all: Vec<u32> = c.meters.borrow().keys().copied().collect();
        for k in all {
            if let Some(m) = c.meters.borrow_mut().remove(&k) {
                close_meter(&c, m);
            }
        }
        Levels::with(|l| l.clear());
        c.ml.borrow_mut().lock();
        c.ctx.borrow_mut().set_subscribe_callback(None);
        c.ctx.borrow_mut().disconnect();
        c.ml.borrow_mut().unlock();
        c.ml.borrow_mut().stop();
    }

    fn worker(rx: Receiver<Req>) {
        let mut conn: Option<Conn> = None;
        let mut playing = false;
        let mut last_read = std::time::Instant::now();
        loop {
            let metering = conn.as_ref().is_some_and(|c| !c.meters.borrow().is_empty());
            let wait = if metering { std::time::Duration::from_millis(500) } else { super::IDLE };
            let req = match rx.recv_timeout(wait) {
                Ok(r) => r,
                Err(RecvTimeoutError::Timeout) if (playing || metering) && conn.is_some() => {
                    if metering && flips() {
                        super::changed();
                    }
                    if let Some(c) = conn.as_ref()
                        && last_read.elapsed() >= super::IDLE
                    {
                        playing = read(c).map(|v| v.iter().any(|s| !s.corked && !super::own(s))).unwrap_or(false);
                        last_read = std::time::Instant::now();
                    }
                    continue;
                }
                Err(_) => break,
            };
            if conn.is_none() {
                match open() {
                    Ok(c) => conn = Some(c),
                    Err(e) => {
                        let msg = e.to_string();
                        match req {
                            Req::List(tx) => {
                                let _ = tx.send(Err(anyhow!(msg)));
                            }
                            Req::Mute(_, _, tx) | Req::Volume(_, _, _, tx) => {
                                let _ = tx.send(Err(anyhow!(msg)));
                            }
                            Req::Sinks(tx) => {
                                let _ = tx.send(Err(anyhow!(msg)));
                            }
                            Req::Output(_, tx) => {
                                let _ = tx.send(Err(anyhow!(msg)));
                            }
                            Req::Master(_, _, tx) => {
                                let _ = tx.send(Err(anyhow!(msg)));
                            }
                        }
                        break;
                    }
                }
            }
            let Some(c) = conn.as_ref() else { break };
            match req {
                Req::List(tx) => {
                    let r = read(c);
                    if let Ok(v) = &r {
                        playing = v.iter().any(|s| !s.corked && !super::own(s));
                    }
                    last_read = std::time::Instant::now();
                    let _ = tx.send(r);
                }
                Req::Mute(i, on, tx) => {
                    let _ = tx.send(mute(c, i, on));
                }
                Req::Volume(i, ch, level, tx) => {
                    let _ = tx.send(volume(c, i, ch, level));
                }
                Req::Sinks(tx) => {
                    let _ = tx.send(sinks_now(c));
                }
                Req::Output(name, tx) => {
                    sinks_changed(c);
                    let _ = tx.send(output(c, &name));
                    sinks_changed(c);
                }
                Req::Master(level, mute, tx) => {
                    let _ = tx.send(master_set(c, level, mute));
                }
            }
        }
        TX.lock().unwrap().take();
        if let Some(c) = conn.take() {
            close(c);
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::Stream;
    use anyhow::{Result, anyhow};
    use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
    use windows::Win32::Media::Audio::{AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static LEVELS: Mutex<Option<HashMap<u32, super::Level>>> = Mutex::new(None);

    fn heard(pid: u32, peak: Option<f32>, active: bool) -> Option<bool> {
        let now = Instant::now();
        let mut g = LEVELS.lock().unwrap();
        let m = g.get_or_insert_with(HashMap::new);
        if !active {
            m.remove(&pid);
            return None;
        }
        let l = m.entry(pid).or_insert(super::Level { started: now, loud: None, told: None, fed: false });
        if let Some(p) = peak {
            l.fed = true;
            if p >= super::LOUD {
                l.loud = Some(now);
            }
        }
        l.state(now)
    }

    pub fn listen(d: Duration) {
        let end = Instant::now() + d;
        while Instant::now() < end {
            let _ = sessions();
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    use windows::core::{Interface, PWSTR};

    pub fn connected() -> bool {
        false
    }

    pub fn metering() -> bool {
        LEVELS.lock().unwrap().as_ref().is_some_and(|m| !m.is_empty())
    }

    fn exe(pid: u32) -> String {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
            let mut buf = [0u16; 512];
            let mut n = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
            let _ = windows::Win32::Foundation::CloseHandle(h);
            if !ok {
                return String::new();
            }
            let path = String::from_utf16_lossy(&buf[..n as usize]);
            std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        }
    }

    fn sessions() -> Result<Vec<(Stream, ISimpleAudioVolume)>> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| anyhow!("{e}"))?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| anyhow!("{e}"))?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).map_err(|e| anyhow!("{e}"))?;
            let list = manager.GetSessionEnumerator().map_err(|e| anyhow!("{e}"))?;
            let mut out = Vec::new();
            for i in 0..list.GetCount().unwrap_or(0) {
                let Ok(control) = list.GetSession(i) else { continue };
                let Ok(c2) = control.cast::<IAudioSessionControl2>() else { continue };
                if c2.IsSystemSoundsSession().is_ok() && c2.IsSystemSoundsSession() == windows::Win32::Foundation::S_OK {
                    continue;
                }
                let pid = c2.GetProcessId().unwrap_or(0);
                if pid == 0 {
                    continue;
                }
                let Ok(vol) = control.cast::<ISimpleAudioVolume>() else { continue };
                let active = control.GetState().map(|s| s == AudioSessionStateActive).unwrap_or(false);
                let peak = control.cast::<IAudioMeterInformation>().ok().and_then(|m| m.GetPeakValue().ok());
                let sounding = heard(pid, peak, active);
                let name = exe(pid);
                let shown = control.GetDisplayName().ok().and_then(|p| p.to_string().ok()).filter(|n| !n.is_empty() && !n.starts_with('@')).unwrap_or_else(|| name.clone());
                let stream = Stream {
                    index: pid,
                    app: shown,
                    title: String::new(),
                    pid: Some(pid),
                    binary: name.to_lowercase(),
                    corked: !active,
                    muted: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                    role: String::new(),
                    volume: vol.GetMasterVolume().unwrap_or(1.0) as f64,
                    channels: 2,
                    sink: 0,
                    sounding,
                };
                out.push((stream, vol));
            }
            Ok(out)
        }
    }

    pub async fn list() -> Result<Vec<Stream>> {
        let s = crate::layers::read(crate::layers::Layer::System, "system.sound.streams");
        let r = crate::layers::blocking(|| sessions().map(|v| v.into_iter().map(|(s, _)| s).collect())).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        s.end(r)
    }

    fn apply(pid: u32, f: impl Fn(&ISimpleAudioVolume) -> windows::core::Result<()>) -> Result<()> {
        let mut n = 0;
        for (s, v) in sessions()? {
            if s.pid == Some(pid) && f(&v).is_ok() {
                n += 1;
            }
        }
        if n > 0 { Ok(()) } else { Err(anyhow!("That app isn't making sound any more.")) }
    }

    pub async fn set_mute(index: u32, mute: bool) -> Result<()> {
        let s = crate::layers::effect(crate::layers::Layer::System, "system.sound.mute");
        let r = crate::layers::blocking(move || apply(index, |v| unsafe { v.SetMute(mute, std::ptr::null()) })).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        s.end(r)
    }

    pub async fn set_volume(index: u32, _channels: u8, level: f64) -> Result<()> {
        let s = crate::layers::effect(crate::layers::Layer::System, "system.sound.volume");
        let l = level.clamp(0.0, 1.0) as f32;
        let r = crate::layers::blocking(move || apply(index, |v| unsafe { v.SetMasterVolume(l, std::ptr::null()) })).await.map_err(|e| anyhow!("{e}")).and_then(|r| r);
        s.end(r)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Stream;
    use anyhow::{Result, anyhow};
    use std::collections::HashMap;
    use std::ffi::{CString, c_char, c_void};
    use std::sync::Mutex;

    type Obj = u32;
    type Id = *mut c_void;

    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(id: Obj, addr: *const Addr, qsize: u32, qdata: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(id: Obj, addr: *const Addr, qsize: u32, qdata: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    }

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {}

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    const SYSTEM: Obj = 1;
    const MAIN: u32 = 0;
    static TAPS: Mutex<Option<HashMap<u32, Obj>>> = Mutex::new(None);

    fn four(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }

    fn addr(sel: &[u8; 4]) -> Addr {
        Addr { selector: four(sel), scope: four(b"glob"), element: MAIN }
    }

    fn get<T: Copy + Default>(id: Obj, sel: &[u8; 4], q: Option<&u32>) -> Option<T> {
        let a = addr(sel);
        let mut v = T::default();
        let mut size = std::mem::size_of::<T>() as u32;
        let (qs, qd) = q.map(|x| (4u32, x as *const u32 as *const c_void)).unwrap_or((0, std::ptr::null()));
        let r = unsafe { AudioObjectGetPropertyData(id, &a, qs, qd, &mut size, &mut v as *mut T as *mut c_void) };
        (r == 0).then_some(v)
    }

    fn processes() -> Vec<Obj> {
        let a = addr(b"prs#");
        let mut size = 0u32;
        if unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &a, 0, std::ptr::null(), &mut size) } != 0 || size == 0 {
            return Vec::new();
        }
        let mut v = vec![0u32; size as usize / 4];
        if unsafe { AudioObjectGetPropertyData(SYSTEM, &a, 0, std::ptr::null(), &mut size, v.as_mut_ptr() as *mut c_void) } != 0 {
            return Vec::new();
        }
        v.truncate(size as usize / 4);
        v
    }

    fn name_of(pid: i32) -> String {
        let mut buf = [0u8; 256];
        let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
        if n <= 0 { String::new() } else { String::from_utf8_lossy(&buf[..n as usize]).into_owned() }
    }

    fn sel(n: &str) -> Id {
        let c = CString::new(n).unwrap_or_default();
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn class(n: &str) -> Id {
        let c = CString::new(n).unwrap_or_default();
        unsafe { objc_getClass(c.as_ptr()) }
    }

    fn symbol(n: &str) -> *mut c_void {
        let c = CString::new(n).unwrap_or_default();
        unsafe { dlsym((-2isize) as *mut c_void, c.as_ptr()) }
    }

    fn tap(object: Obj) -> Result<Obj> {
        let create = symbol("AudioHardwareCreateProcessTap");
        let cls = class("CATapDescription");
        if create.is_null() || cls.is_null() {
            return Err(anyhow!("Muting one app needs macOS 14.2 or later."));
        }
        unsafe {
            let send0: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send_u32: unsafe extern "C" fn(Id, Id, u32) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send_id: unsafe extern "C" fn(Id, Id, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send_i: unsafe extern "C" fn(Id, Id, isize) = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let send_b: unsafe extern "C" fn(Id, Id, bool) = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let num = send_u32(class("NSNumber"), sel("numberWithUnsignedInt:"), object);
            let list = send_id(class("NSArray"), sel("arrayWithObject:"), num);
            let desc = send_id(send0(cls, sel("alloc")), sel("initStereoMixdownOfProcesses:"), list);
            if desc.is_null() {
                return Err(anyhow!("macOS refused to tap that app."));
            }
            send_i(desc, sel("setMuteBehavior:"), 1);
            send_b(desc, sel("setPrivate:"), true);
            let f: unsafe extern "C" fn(Id, *mut Obj) -> i32 = std::mem::transmute(create);
            let mut id: Obj = 0;
            let r = f(desc, &mut id);
            send0(desc, sel("release"));
            if r != 0 {
                return Err(anyhow!("macOS didn't allow muting that app. Allow Setu to record system audio in Privacy and Security."));
            }
            Ok(id)
        }
    }

    fn untap(id: Obj) {
        let destroy = symbol("AudioHardwareDestroyProcessTap");
        if !destroy.is_null() {
            let f: unsafe extern "C" fn(Obj) -> i32 = unsafe { std::mem::transmute(destroy) };
            unsafe { f(id) };
        }
    }

    fn read() -> Vec<Stream> {
        let taps = TAPS.lock().unwrap();
        let muted = |pid: u32| taps.as_ref().is_some_and(|m| m.contains_key(&pid));
        processes()
            .into_iter()
            .filter_map(|p| {
                let pid = get::<i32>(p, b"ppid", None)?;
                let running = get::<u32>(p, b"piro", None).unwrap_or(0) != 0;
                if pid <= 0 || !(running || muted(pid as u32)) {
                    return None;
                }
                let name = name_of(pid);
                Some(Stream { index: pid as u32, app: name.clone(), title: String::new(), pid: Some(pid as u32), binary: name.to_lowercase(), corked: false, muted: muted(pid as u32), role: String::new(), volume: 1.0, channels: 2, sink: 0, sounding: None })
            })
            .collect()
    }

    pub fn connected() -> bool {
        false
    }

    pub fn metering() -> bool {
        false
    }

    pub async fn list() -> Result<Vec<Stream>> {
        crate::layers::blocking(read).await.map_err(|e| anyhow!("{e}"))
    }

    pub async fn set_mute(index: u32, mute: bool) -> Result<()> {
        crate::layers::blocking(move || {
            let mut g = TAPS.lock().unwrap();
            let m = g.get_or_insert_with(HashMap::new);
            if !mute {
                if let Some(t) = m.remove(&index) {
                    untap(t);
                }
                return Ok(());
            }
            if m.contains_key(&index) {
                return Ok(());
            }
            let object = get::<u32>(SYSTEM, b"id2p", Some(&index)).filter(|o| *o != 0).ok_or_else(|| anyhow!("That app isn't making sound any more."))?;
            let t = tap(object)?;
            m.insert(index, t);
            Ok(())
        })
        .await
        .map_err(|e| anyhow!("{e}"))?
    }

    pub async fn set_volume(_index: u32, _channels: u8, _level: f64) -> Result<()> {
        Err(anyhow!("One app's volume isn't available on this system yet."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setu_never_lists_its_own_sound() {
        let mine = Stream { index: 1, app: "Setu".into(), title: "speech".into(), pid: Some(std::process::id()), binary: "setu-daemon".into(), corked: false, muted: false, role: String::new(), volume: 1.0, channels: 2, sink: 0, sounding: None };
        let other = Stream { index: 2, app: "Steam".into(), title: "game".into(), pid: Some(1), binary: "steam".into(), corked: false, muted: false, role: "game".into(), volume: 1.0, channels: 2, sink: 0, sounding: None };
        assert!(own(&mine));
        assert!(!own(&other));
    }

    #[test]
    fn a_stream_is_sounding_until_three_quiet_seconds_and_unknown_at_first() {
        let t0 = std::time::Instant::now();
        let l = Level { started: t0, loud: None, told: None, fed: true };
        assert_eq!(l.state(t0 + Duration::from_millis(100)), None);
        assert_eq!(l.state(t0 + Duration::from_millis(900)), Some(false));
        let unfed = Level { fed: false, ..l };
        assert_eq!(unfed.state(t0 + Duration::from_secs(10)), None, "a meter that never heard anything proves nothing");
        let l = Level { started: t0, loud: Some(t0 + Duration::from_millis(200)), told: None, fed: true };
        assert_eq!(l.state(t0 + Duration::from_millis(300)), Some(true));
        assert_eq!(l.state(t0 + Duration::from_millis(3100)), Some(true));
        assert_eq!(l.state(t0 + Duration::from_millis(3300)), Some(false));
    }
}
