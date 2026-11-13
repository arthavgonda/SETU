use crate::layers::{self, Layer};
use crate::os::{Node, Scope, Win};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    pub fn parse(s: &str) -> Self {
        match s {
            "right" => Self::Right,
            "middle" => Self::Middle,
            _ => Self::Left,
        }
    }

    fn verb(self, count: u8) -> &'static str {
        match (self, count) {
            (Self::Right, _) => "Right-clicked",
            (Self::Middle, _) => "Middle-clicked",
            (_, 2) => "Double-clicked",
            (_, 3) => "Triple-clicked",
            _ => "Clicked",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Space {
    Screen,
    Window(Win),
}

#[derive(Debug, Clone, PartialEq)]
pub struct At {
    pub x: f64,
    pub y: f64,
    pub space: Space,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ev {
    Move(f64, f64),
    Down(Button, u8),
    Up(Button, u8),
    Wheel { dx: f64, dy: f64, smooth: bool, last: bool },
    Pause(u64),
}

static EPOCH: AtomicU64 = AtomicU64::new(0);
static HELD: Mutex<Vec<Button>> = Mutex::new(Vec::new());
static GESTURE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn place(win: &Win, n: &Node) -> At {
    let x = n.x as f64 + n.w.max(1) as f64 / 2.0;
    let y = n.y as f64 + n.h.max(1) as f64 / 2.0;
    match crate::os::window_origin(win).await {
        Some((ox, oy)) => At { x: x - ox as f64, y: y - oy as f64, space: Space::Window(win.clone()) },
        None => At { x, y, space: Space::Screen },
    }
}

pub async fn window_point(win: &Win, x: f64, y: f64) -> At {
    match crate::os::window_origin(win).await {
        Some(_) => At { x, y, space: Space::Window(win.clone()) },
        None => match crate::os::client_origin(win).await.or(crate::os::window_box(win).await.map(|(x, y, _, _)| (x, y))) {
            Some((ox, oy)) if ox != 0 || oy != 0 => At { x: ox as f64 + x, y: oy as f64 + y, space: Space::Screen },
            _ => At { x, y, space: Space::Window(win.clone()) },
        },
    }
}

pub fn shifted(a: &At, dx: f64, dy: f64) -> At {
    At { x: a.x + dx, y: a.y + dy, space: a.space.clone() }
}

fn held(b: Button, down: bool) {
    let mut g = HELD.lock().unwrap();
    g.retain(|x| *x != b);
    if down {
        g.push(b);
    }
}

pub fn holding() -> bool {
    !HELD.lock().unwrap().is_empty()
}

fn ease(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

pub fn click_plan(at: Option<(f64, f64)>, b: Button, count: u8) -> Vec<Ev> {
    let mut v = Vec::new();
    if let Some((x, y)) = at {
        v.push(Ev::Move(x, y));
        v.push(Ev::Pause(35));
    }
    let n = count.clamp(1, 3);
    for i in 1..=n {
        v.push(Ev::Down(b, i));
        v.push(Ev::Pause(45));
        v.push(Ev::Up(b, i));
        if i < n {
            v.push(Ev::Pause(70));
        }
    }
    v
}

pub fn drag_plan(from: (f64, f64), to: (f64, f64), b: Button) -> Vec<Ev> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = dx.hypot(dy).max(1.0);
    let (ux, uy) = (dx / len, dy / len);
    let mut v = vec![Ev::Move(from.0, from.1), Ev::Pause(60), Ev::Down(b, 1), Ev::Pause(90)];
    let mut start = from;
    for d in [4.0, 9.0] {
        if d < len / 2.0 {
            start = (from.0 + ux * d, from.1 + uy * d);
            v.push(Ev::Move(start.0, start.1));
            v.push(Ev::Pause(16));
        }
    }
    let (rx, ry) = (to.0 - start.0, to.1 - start.1);
    let steps = ((len / 25.0) as usize).clamp(12, 30);
    let total = (150.0 + len * 0.3).clamp(180.0, 450.0);
    let each = (total / steps as f64).round() as u64;
    for i in 1..=steps {
        let t = ease(i as f64 / steps as f64);
        v.push(Ev::Move(start.0 + rx * t, start.1 + ry * t));
        v.push(Ev::Pause(each));
    }
    v.push(Ev::Move(to.0, to.1));
    v.push(Ev::Pause(140));
    v.push(Ev::Up(b, 1));
    v
}

pub fn scroll_plan(at: Option<(f64, f64)>, dx: f64, dy: f64, smooth: bool) -> Vec<Ev> {
    let mut v = Vec::new();
    if let Some((x, y)) = at {
        v.push(Ev::Move(x, y));
        v.push(Ev::Pause(40));
    }
    if smooth {
        let steps = ((dx.abs().max(dy.abs()) * 6.0) as usize).clamp(12, 60);
        for _ in 0..steps {
            v.push(Ev::Wheel { dx: dx / steps as f64, dy: dy / steps as f64, smooth: true, last: false });
            v.push(Ev::Pause(30));
        }
        v.push(Ev::Wheel { dx: 0.0, dy: 0.0, smooth: true, last: true });
    } else {
        v.push(Ev::Wheel { dx: dx.round(), dy: dy.round(), smooth: false, last: true });
    }
    v
}

async fn run(stage: &'static str, space: Space, plan: Vec<Ev>) -> Result<()> {
    let _one = GESTURE.lock().await;
    let epoch = EPOCH.load(Ordering::SeqCst);
    let first = plan.iter().find_map(|e| if let Ev::Move(x, y) = e { Some((*x, *y)) } else { None });
    imp::busy();
    let s = layers::effect(Layer::Pointer, stage);
    let r = drive(&space, first, &plan, epoch).await;
    let r = s.end(r);
    if r.is_err() && holding() {
        let _ = let_go().await;
    }
    imp::idle();
    crate::os::disturbed();
    r
}

async fn drive(space: &Space, first: Option<(f64, f64)>, plan: &[Ev], epoch: u64) -> Result<()> {
    imp::prepare(space, first).await?;
    for ev in plan {
        if EPOCH.load(Ordering::SeqCst) != epoch {
            return Err(anyhow!("Stopped."));
        }
        match *ev {
            Ev::Pause(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
            e => {
                let t0 = Instant::now();
                imp::send(e).await?;
                layers::record("pointer.event", t0.elapsed().as_micros() as u64);
                match e {
                    Ev::Down(b, _) => held(b, true),
                    Ev::Up(b, _) => held(b, false),
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

async fn let_go() -> Result<()> {
    let list: Vec<Button> = HELD.lock().unwrap().drain(..).collect();
    for b in list {
        imp::send(Ev::Up(b, 1)).await?;
    }
    Ok(())
}

pub fn release_all() {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    if !holding() {
        return;
    }
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        h.spawn(async {
            let _ = let_go().await;
            imp::idle();
        });
    }
}

fn spot(at: &Option<At>) -> (Space, Option<(f64, f64)>) {
    match at {
        Some(a) => (a.space.clone(), Some((a.x, a.y))),
        None => (Space::Screen, None),
    }
}

pub async fn click(at: Option<At>, b: Button, count: u8) -> Result<()> {
    let (space, p) = spot(&at);
    run("pointer.click", space, click_plan(p, b, count)).await
}

pub async fn hover(at: At) -> Result<()> {
    run("pointer.move", at.space.clone(), vec![Ev::Move(at.x, at.y)]).await
}

pub async fn hold(at: Option<At>, b: Button) -> Result<()> {
    let (space, p) = spot(&at);
    let mut plan = Vec::new();
    if let Some((x, y)) = p {
        plan.push(Ev::Move(x, y));
        plan.push(Ev::Pause(40));
    }
    plan.push(Ev::Down(b, 1));
    run("pointer.hold", space, plan).await
}

pub async fn release() -> Result<bool> {
    if !holding() {
        return Ok(false);
    }
    let _one = GESTURE.lock().await;
    let s = layers::effect(Layer::Pointer, "pointer.release");
    let r = s.end(let_go().await);
    imp::idle();
    r.map(|_| true)
}

pub async fn drag(from: At, to: At, b: Button) -> Result<()> {
    if from.space != to.space {
        return Err(anyhow!("I can only drag within one window at a time."));
    }
    run("pointer.drag", from.space.clone(), drag_plan((from.x, from.y), (to.x, to.y), b)).await
}

pub async fn scroll(at: Option<At>, dx: f64, dy: f64, smooth: bool) -> Result<()> {
    let (space, p) = spot(&at);
    run("pointer.scroll", space, scroll_plan(p, dx, dy, smooth)).await
}

pub fn status() -> &'static str {
    imp::status()
}

#[cfg(target_os = "macos")]
pub fn main_display_width() -> Option<f64> {
    imp::main_display_width()
}

fn pick(nodes: &[Node], name: &str, roles: &[&str]) -> Option<Node> {
    let n = name.trim().trim_start_matches("the ").trim();
    match n {
        "this" | "it" | "that" | "this one" | "that one" | "here" => nodes.iter().find(|x| x.focused).or_else(|| nodes.iter().find(|x| x.selected)).cloned(),
        "this tab" | "current tab" | "tab" | "selected tab" => nodes.iter().find(|x| x.role == "tab" && x.selected).cloned(),
        _ => crate::os::find(nodes, n, roles).cloned(),
    }
}

pub async fn locate(name: &str, roles: &[&str]) -> Result<(Win, Node)> {
    locate_with(name, roles).await.map(|(w, n, _)| (w, n))
}

async fn locate_with(name: &str, roles: &[&str]) -> Result<(Win, Node, Vec<Node>)> {
    let wins = crate::os::ui_targets(false).await?;
    let mut problem = None;
    for w in &wins {
        crate::os::wake(w).await;
        match crate::os::tree(w, Scope { limit: 600, text: false, hidden: false }).await {
            Ok(nodes) => {
                if let Some(n) = pick(&nodes, name, roles) {
                    return Ok((w.clone(), n, nodes));
                }
            }
            Err(e) => {
                problem.get_or_insert(e.to_string());
            }
        }
    }
    match problem {
        Some(p) if wins.len() == 1 => Err(anyhow!(p)),
        _ => Err(anyhow!("I couldn't find {} on the screen.", name.trim().trim_start_matches("the "))),
    }
}

fn label(n: &Node) -> String {
    if n.name.trim().is_empty() { "it".into() } else { n.name.trim().to_owned() }
}

fn direction(word: &str, amount: &str, n: &Node) -> Option<(f64, f64)> {
    let base = match amount {
        "bit" | "little" | "little bit" => 80.0,
        "lot" | "far" => 480.0,
        _ if n.role == "tab" => (n.w as f64 * 1.6).max(120.0),
        _ => 200.0,
    };
    Some(match word {
        "right" => (base, 0.0),
        "left" => (-base, 0.0),
        "up" | "top" => (0.0, -base),
        "down" | "bottom" => (0.0, base),
        _ => return None,
    })
}

pub async fn tool(tool: &str, args: &Value) -> Result<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let roles: Vec<String> = args.get("roles").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let roles: Vec<&str> = roles.iter().map(String::as_str).collect();
    let b = Button::parse(&s("button"));
    let count = args.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3) as u8;
    match tool {
        "pointer.click" => {
            if s("name").is_empty() {
                click(None, b, count).await?;
                return Ok(format!("{} here", b.verb(count)));
            }
            let (w, n) = locate(&s("name"), &roles).await?;
            if b == Button::Right && crate::os::act(&n, crate::os::Do::Menu).await.is_ok() {
                return Ok(format!("Opened the menu for {}", label(&n)));
            }
            click(Some(place(&w, &n).await), b, count).await?;
            Ok(format!("{} {}", b.verb(count), label(&n)))
        }
        "pointer.hover" => {
            let (w, n) = locate(&s("name"), &roles).await?;
            hover(place(&w, &n).await).await?;
            Ok(format!("The pointer is on {}", label(&n)))
        }
        "pointer.hold" => {
            if s("name").is_empty() {
                hold(None, b).await?;
                return Ok("Holding the mouse button. Say let go when you're done".into());
            }
            let (w, n) = locate(&s("name"), &roles).await?;
            hold(Some(place(&w, &n).await), b).await?;
            Ok(format!("Holding the mouse button on {}. Say let go when you're done", label(&n)))
        }
        "pointer.release" => Ok(if release().await? { "Let go".into() } else { "Nothing was held".into() }),
        "pointer.drag" => {
            let (w, n, seen) = locate_with(&s("name"), &roles).await?;
            let from = place(&w, &n).await;
            let (to, what) = if !s("to").is_empty() {
                let (w2, m) = match pick(&seen, &s("to"), &[]).filter(|m| m.id != n.id) {
                    Some(m) => (w.clone(), m),
                    None => locate(&s("to"), &[]).await?,
                };
                if w2.id != w.id {
                    return Err(anyhow!("{} and {} are in different windows. I can only drag within one window for now.", label(&n), label(&m)));
                }
                (place(&w2, &m).await, format!("onto {}", label(&m)))
            } else {
                let (dx, dy) = direction(&s("direction"), &s("amount"), &n).ok_or_else(|| anyhow!("Which way should I drag it?"))?;
                (shifted(&from, dx, dy), format!("to the {}", s("direction")))
            };
            drag(from, to, b).await?;
            Ok(format!("Dragged {} {what}", label(&n)))
        }
        "pointer.scroll" => {
            let amount = args.get("amount").and_then(Value::as_f64).unwrap_or(5.0);
            let (dx, dy) = match s("direction").as_str() {
                "up" => (0.0, -amount),
                "left" => (-amount, 0.0),
                "right" => (amount, 0.0),
                _ => (0.0, amount),
            };
            let smooth = args.get("smooth").and_then(Value::as_bool).unwrap_or(true);
            let at = if s("name").is_empty() {
                let w = crate::os::working().await?;
                let f = crate::os::window_box(&w).await;
                match f {
                    Some((x, y, wd, ht)) if wd > 0 && ht > 0 => Some(place(&w, &Node { x, y, w: wd, h: ht, ..Default::default() }).await),
                    _ => None,
                }
            } else {
                let (w, n) = locate(&s("name"), &roles).await?;
                Some(place(&w, &n).await)
            };
            scroll(at, dx, dy, smooth).await?;
            let dir = if s("direction").is_empty() { "down".to_owned() } else { s("direction") };
            Ok(format!("Scrolled {dir}{}", if smooth { " slowly" } else { "" }))
        }
        other => Err(anyhow!("I don't know the pointer action {other}.")),
    }
}

pub fn rules(t: &str) -> Option<crate::router::Intent> {
    let t = t.replace("double-click", "double click").replace("right-click", "right click").replace("middle-click", "middle click").replace("triple-click", "triple click");
    let t = t.trim().trim_end_matches(['.', '!', '?']).trim();
    let act = |tool: &str, args: Value| Some(crate::router::Intent::Act { tool: tool.into(), args });
    let here = |r: &str| matches!(r, "here" | "it here" | "the mouse" | "the mouse here" | "where the mouse is" | "where the pointer is" | "at the pointer");
    for (p, button, count) in [("double click ", "left", 2u64), ("double tap ", "left", 2), ("triple click ", "left", 3), ("middle click ", "middle", 1), ("right click ", "right", 1), ("left click ", "left", 1), ("click ", "left", 1)] {
        if let Some(r) = t.strip_prefix(p) {
            let r = r.trim_start_matches("on ").trim();
            if here(r) {
                return act("pointer.click", json!({ "button": button, "count": count }));
            }
            if button == "right" || (button == "left" && count == 1) {
                continue;
            }
            let name = r.trim_start_matches("the ").trim();
            if name.is_empty() || name.split_whitespace().count() > 8 {
                return None;
            }
            return act("pointer.click", json!({ "name": name, "button": button, "count": count }));
        }
    }
    if matches!(t, "double click" | "triple click" | "middle click" | "left click" | "click the mouse" | "click the mouse button") {
        let (button, count) = match t { "double click" => ("left", 2), "triple click" => ("left", 3), "middle click" => ("middle", 1), _ => ("left", 1) };
        return act("pointer.click", json!({ "button": button, "count": count }));
    }
    if matches!(
        t,
        "hold the mouse" | "hold the mouse here" | "hold the mouse down" | "hold the mouse button" | "hold down the mouse" | "hold down the mouse button" | "hold the left mouse button" | "press and hold" | "press and hold here" | "hold the mouse button down" | "keep the mouse pressed"
    ) {
        return act("pointer.hold", json!({}));
    }
    if matches!(t, "let go" | "let go of the mouse" | "let go of the mouse button" | "release the mouse" | "release the mouse button" | "let the mouse go" | "drop it" | "drop it here" | "release it") {
        return act("pointer.release", json!({}));
    }
    for p in ["hover over ", "hover on ", "hover ", "move the mouse to ", "move the mouse over ", "move the pointer to ", "point at ", "point to ", "put the mouse on ", "mouse over "] {
        if let Some(r) = t.strip_prefix(p) {
            let name = r.trim_start_matches("the ").trim();
            if name.is_empty() || name.split_whitespace().count() > 8 {
                return None;
            }
            return act("pointer.hover", json!({ "name": name }));
        }
    }
    if let Some(r) = t.strip_prefix("drag ") {
        for sep in [" onto ", " into ", " over to ", " on to ", " to "] {
            if let Some((what, rest)) = r.split_once(sep) {
                let what = what.trim().trim_start_matches("the ").trim();
                let rest = rest.trim().trim_start_matches("the ").trim();
                let (amount, dir) = match rest.split_once(' ') {
                    Some((a, d)) if matches!(a, "bit" | "little" | "lot" | "far") => (a, d.trim_start_matches("of ").trim()),
                    _ => ("", rest),
                };
                let dir = dir.trim_end_matches(" a bit").trim_end_matches(" a little").trim_end_matches(" a lot").trim_end_matches(" side");
                let amount = if rest.ends_with(" a bit") || rest.ends_with(" a little") { "bit" } else if rest.ends_with(" a lot") { "lot" } else { amount };
                if what.is_empty() {
                    return None;
                }
                if matches!(dir, "right" | "left" | "up" | "down" | "top" | "bottom") {
                    return act("pointer.drag", json!({ "name": what, "direction": dir, "amount": amount }));
                }
                if rest.is_empty() || rest.split_whitespace().count() > 8 {
                    return None;
                }
                return act("pointer.drag", json!({ "name": what, "to": rest }));
            }
        }
        for dir in ["right", "left", "up", "down"] {
            if let Some(what) = r.strip_suffix(&format!(" {dir}")) {
                return act("pointer.drag", json!({ "name": what.trim().trim_start_matches("the "), "direction": dir, "amount": "" }));
            }
        }
    }
    if let Some(r) = t.strip_prefix("scroll ") {
        let mut words: Vec<&str> = r.split_whitespace().collect();
        let smooth_words = ["slowly", "smoothly", "gently", "slow"];
        let smooth = words.iter().any(|w| smooth_words.contains(w));
        let dir = words.iter().find(|w| matches!(**w, "up" | "down" | "left" | "right")).copied();
        words.retain(|w| !smooth_words.contains(w) && !matches!(*w, "up" | "down" | "left" | "right" | "a" | "bit" | "little" | "the" | "page" | "please"));
        if let Some(d) = dir
            && words.is_empty()
            && (smooth || matches!(d, "left" | "right"))
        {
            let little = r.contains("a bit") || r.contains("a little");
            return act("pointer.scroll", json!({ "direction": d, "smooth": smooth, "amount": if little { 2.0 } else { 5.0 } }));
        }
    }
    None
}

async fn probe_win(v: &Value) -> Result<Win> {
    let want = v.get("win").and_then(Value::as_str).unwrap_or_default();
    let wins = crate::os::windows().await?;
    if want.is_empty() {
        return wins.iter().find(|w| w.front).cloned().ok_or_else(|| anyhow!("no window"));
    }
    wins.iter().find(|w| w.title == want || w.id == want).or_else(|| wins.iter().find(|w| w.title.contains(want))).cloned().ok_or_else(|| anyhow!("no window"))
}

async fn probe_at(v: &Value, name: &str) -> Result<(Node, At)> {
    let roles: Vec<&str> = v.get("roles").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let w = probe_win(v).await?;
    let nodes = crate::os::tree(&w, Scope { limit: 600, text: false, hidden: false }).await?;
    let n = pick(&nodes, name, &roles).ok_or_else(|| anyhow!("not found"))?;
    let at = place(&w, &n).await;
    Ok((n, at))
}

fn at_json(a: &At) -> Value {
    json!({ "x": a.x, "y": a.y, "space": match &a.space { Space::Screen => "screen", Space::Window(_) => "window" } })
}

pub async fn probe(v: &Value) -> Result<Value> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let f = |k: &str| v.get(k).and_then(Value::as_f64);
    let b = Button::parse(&s("button"));
    let t0 = Instant::now();
    let r = match s("do").as_str() {
        "status" => json!({ "status": status() }),
        "where" => {
            let (n, at) = probe_at(v, &s("name")).await?;
            json!({ "node": n, "at": at_json(&at) })
        }
        "click" => {
            let at = if s("name").is_empty() { None } else { Some(probe_at(v, &s("name")).await?.1) };
            click(at.clone(), b, v.get("count").and_then(Value::as_u64).unwrap_or(1) as u8).await?;
            json!({ "at": at.as_ref().map(at_json) })
        }
        "move" => {
            let w = probe_win(v).await?;
            let at = if s("space") == "screen" { At { x: f("x").unwrap_or(0.0), y: f("y").unwrap_or(0.0), space: Space::Screen } } else { window_point(&w, f("x").unwrap_or(0.0), f("y").unwrap_or(0.0)).await };
            hover(at.clone()).await?;
            json!({ "at": at_json(&at) })
        }
        "hover" => {
            let (_, at) = probe_at(v, &s("name")).await?;
            hover(at.clone()).await?;
            json!({ "at": at_json(&at) })
        }
        "drag" => {
            let from = if s("name").is_empty() { window_point(&probe_win(v).await?, f("x").unwrap_or(0.0), f("y").unwrap_or(0.0)).await } else { probe_at(v, &s("name")).await?.1 };
            let to = if s("to").is_empty() { shifted(&from, f("dx").unwrap_or(0.0), f("dy").unwrap_or(0.0)) } else { probe_at(v, &s("to")).await?.1 };
            drag(from.clone(), to.clone(), b).await?;
            json!({ "from": at_json(&from), "to": at_json(&to) })
        }
        "scroll" => {
            let at = if s("name").is_empty() { None } else { Some(probe_at(v, &s("name")).await?.1) };
            scroll(at, f("dx").unwrap_or(0.0), f("dy").unwrap_or(0.0), v.get("smooth").and_then(Value::as_bool).unwrap_or(false)).await?;
            json!({ "ok": true })
        }
        "hold" => {
            let at = if s("name").is_empty() { None } else { Some(probe_at(v, &s("name")).await?.1) };
            hold(at, b).await?;
            json!({ "held": holding() })
        }
        "release" => json!({ "released": release().await? }),
        "stop" => {
            let t = Instant::now();
            release_all();
            let mut waited = 0;
            while holding() && waited < 100 {
                tokio::time::sleep(Duration::from_millis(1)).await;
                waited += 1;
            }
            json!({ "held": holding(), "us": t.elapsed().as_micros() as u64 })
        }
        "plan" => {
            let p = match s("kind").as_str() {
                "drag" => drag_plan((f("x").unwrap_or(0.0), f("y").unwrap_or(0.0)), (f("x2").unwrap_or(0.0), f("y2").unwrap_or(0.0)), b),
                "scroll" => scroll_plan(None, f("dx").unwrap_or(0.0), f("dy").unwrap_or(0.0), true),
                _ => click_plan(None, b, v.get("count").and_then(Value::as_u64).unwrap_or(1) as u8),
            };
            json!({ "events": p.len() })
        }
        other => return Err(anyhow!("unknown pointer op {other}")),
    };
    let mut r = r;
    r["ms"] = json!(t0.elapsed().as_millis() as u64);
    r["layers"] = layers::samples();
    Ok(r)
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Button, Ev, Space};
    use anyhow::{Result, anyhow};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use zbus::Connection;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const RD: &str = "org.gnome.Mutter.RemoteDesktop";
    const RDS: &str = "org.gnome.Mutter.RemoteDesktop.Session";
    const SC: &str = "org.gnome.Mutter.ScreenCast";
    const SCS: &str = "org.gnome.Mutter.ScreenCast.Session";
    const STREAM: &str = "org.gnome.Mutter.ScreenCast.Stream";
    const LINGER: Duration = Duration::from_secs(5);
    const PRIMED: Duration = Duration::from_millis(60);

    #[derive(Clone, PartialEq, Debug)]
    enum Key {
        Any,
        Window(String),
        Monitor(String),
    }

    #[derive(Clone, Debug)]
    struct Mon {
        connector: String,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        scale: f64,
    }

    struct Live {
        c: Connection,
        rd: OwnedObjectPath,
        stream: Option<String>,
        key: Key,
        mon: Option<Mon>,
        factor: f64,
    }

    static LIVE: tokio::sync::Mutex<Option<Live>> = tokio::sync::Mutex::const_new(None);
    static IDLE: AtomicU64 = AtomicU64::new(0);

    fn x11() -> bool {
        std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_some()
    }

    fn code(b: Button) -> i32 {
        match b {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
        }
    }

    pub fn status() -> &'static str {
        if x11() {
            return if crate::tools::apps::which("xdotool").is_some() { "ready" } else { "needs_xdotool" };
        }
        if std::env::var_os("WAYLAND_DISPLAY").is_some() { "ready" } else { "unavailable" }
    }

    pub fn busy() {
        IDLE.fetch_add(1, Ordering::SeqCst);
    }

    pub fn idle() {
        let mine = IDLE.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move {
                tokio::time::sleep(LINGER).await;
                if IDLE.load(Ordering::SeqCst) != mine || super::holding() {
                    return;
                }
                let mut g = LIVE.lock().await;
                if IDLE.load(Ordering::SeqCst) != mine || super::holding() {
                    return;
                }
                if let Some(l) = g.take() {
                    close(&l).await;
                }
            });
        }
    }

    async fn close(l: &Live) {
        let _ = tokio::time::timeout(Duration::from_secs(2), l.c.call_method(Some(RD), l.rd.as_str(), Some(RDS), "Stop", &())).await;
    }

    fn first_int(v: &Value) -> Option<i32> {
        match v {
            Value::Structure(s) => match s.fields().first() {
                Some(Value::I32(n)) => Some(*n),
                _ => None,
            },
            Value::Value(inner) => first_int(inner),
            _ => None,
        }
    }

    async fn monitors(c: &Connection) -> Result<Vec<Mon>> {
        type Mode = (String, i32, i32, f64, f64, Vec<f64>, HashMap<String, OwnedValue>);
        type Monitor = ((String, String, String, String), Vec<Mode>, HashMap<String, OwnedValue>);
        type Logical = (i32, i32, f64, u32, bool, Vec<(String, String, String, String)>, HashMap<String, OwnedValue>);
        let r = c.call_method(Some("org.gnome.Mutter.DisplayConfig"), "/org/gnome/Mutter/DisplayConfig", Some("org.gnome.Mutter.DisplayConfig"), "GetCurrentState", &()).await?;
        let (_, mons, logical, props): (u32, Vec<Monitor>, Vec<Logical>, HashMap<String, OwnedValue>) = r.body().deserialize()?;
        let physical = props.get("layout-mode").is_some_and(|v| matches!(&**v, Value::U32(2)));
        let mut out = Vec::new();
        for (x, y, scale, transform, _, list, _) in logical {
            let Some(spec) = list.first() else { continue };
            let Some((_, modes, _)) = mons.iter().find(|(s, _, _)| s.0 == spec.0) else { continue };
            let Some(m) = modes.iter().find(|m| m.6.get("is-current").is_some_and(|v| matches!(&**v, Value::Bool(true)))) else { continue };
            let (mut w, mut h) = (m.1 as f64, m.2 as f64);
            if transform % 2 == 1 {
                std::mem::swap(&mut w, &mut h);
            }
            let s = if physical { 1.0 } else { scale.max(0.1) };
            out.push(Mon { connector: spec.0.clone(), x: x as f64, y: y as f64, w: w / s, h: h / s, scale });
        }
        Ok(out)
    }

    async fn monitor_at(x: f64, y: f64) -> Result<Mon> {
        let c = Connection::session().await?;
        let list = monitors(&c).await?;
        list.iter().find(|m| x >= m.x && y >= m.y && x < m.x + m.w && y < m.y + m.h).or_else(|| list.first()).cloned().ok_or_else(|| anyhow!("I can't see any screen to point at."))
    }

    async fn front(w: &crate::os::Win) -> Result<()> {
        if crate::os::in_front(w, 250).await {
            return Ok(());
        }
        let name = if w.app.is_empty() { w.title.clone() } else { w.app.clone() };
        let _ = crate::tools::wm::focus(&name, None).await;
        if crate::os::in_front(w, 2500).await {
            tokio::time::sleep(Duration::from_millis(150)).await;
            return Ok(());
        }
        Err(anyhow!("{name} isn't in front, so I can't point at it."))
    }

    fn started(e: zbus::Error) -> anyhow::Error {
        let t = e.to_string();
        if t.contains("pipewire") || t.contains("PipeWire") {
            anyhow!("GNOME couldn't start pointer control because its screen sharing service, PipeWire, isn't running.")
        } else {
            anyhow!("GNOME didn't allow pointer control: {}", t.rsplit(": ").next().unwrap_or(&t))
        }
    }

    async fn open(key: Key, mon: Option<Mon>) -> Result<Live> {
        use futures_util::StreamExt;
        let s = crate::layers::read(crate::layers::Layer::Pointer, "pointer.session");
        let r = async {
            let c = Connection::session().await?;
            let rd: OwnedObjectPath = c
                .call_method(Some(RD), "/org/gnome/Mutter/RemoteDesktop", Some(RD), "CreateSession", &())
                .await
                .map_err(|_| anyhow!("I can't move the pointer on this desktop yet. It works on GNOME, Windows and macOS."))?
                .body()
                .deserialize()?;
            let mut stream = None;
            let mut factor = 1.0;
            if key != Key::Any {
                let sid: OwnedValue = c.call_method(Some(RD), rd.as_str(), Some("org.freedesktop.DBus.Properties"), "Get", &(RDS, "SessionId")).await?.body().deserialize()?;
                let sid: String = match &*sid {
                    Value::Str(s) => s.to_string(),
                    _ => return Err(anyhow!("GNOME gave no session id")),
                };
                let mut props: HashMap<&str, Value> = HashMap::new();
                props.insert("remote-desktop-session-id", Value::from(sid));
                let sc: OwnedObjectPath = c.call_method(Some(SC), "/org/gnome/Mutter/ScreenCast", Some(SC), "CreateSession", &(props,)).await?.body().deserialize()?;
                let none: HashMap<&str, Value> = HashMap::new();
                let st: OwnedObjectPath = match &key {
                    Key::Window(_) => c.call_method(Some(SC), sc.as_str(), Some(SCS), "RecordWindow", &(none,)).await?.body().deserialize()?,
                    Key::Monitor(m) => c.call_method(Some(SC), sc.as_str(), Some(SCS), "RecordMonitor", &(m.as_str(), none)).await?.body().deserialize()?,
                    Key::Any => unreachable!(),
                };
                let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(STREAM)?.member("PipeWireStreamAdded")?.path(st.as_str())?.build();
                let mut added = zbus::MessageStream::for_match_rule(rule, &c, Some(4)).await?;
                c.call_method(Some(RD), rd.as_str(), Some(RDS), "Start", &()).await.map_err(started)?;
                let _ = tokio::time::timeout(Duration::from_millis(1500), added.next()).await;
                if let Some(m) = &mon
                    && let Ok(reply) = c.call_method(Some(SC), st.as_str(), Some("org.freedesktop.DBus.Properties"), "Get", &(STREAM, "Parameters")).await
                    && let Ok(v) = reply.body().deserialize::<OwnedValue>()
                    && let Ok(p) = HashMap::<String, OwnedValue>::try_from(v)
                    && let Some(width) = p.get("size").and_then(|v| first_int(v))
                    && m.w > 0.0
                {
                    factor = width as f64 / m.w;
                } else if let Some(m) = &mon {
                    factor = m.scale;
                }
                stream = Some(st.to_string());
            } else {
                c.call_method(Some(RD), rd.as_str(), Some(RDS), "Start", &()).await.map_err(started)?;
            }
            c.call_method(Some(RD), rd.as_str(), Some(RDS), "NotifyPointerMotionRelative", &(0.0f64, 0.0f64)).await.map_err(|e| anyhow!("GNOME didn't give Setu a pointer: {e}"))?;
            tokio::time::sleep(PRIMED).await;
            Ok::<Live, anyhow::Error>(Live { c, rd, stream, key, mon, factor })
        }
        .await;
        s.end(r)
    }

    pub async fn prepare(space: &Space, first: Option<(f64, f64)>) -> Result<()> {
        if x11() {
            return Ok(());
        }
        let (key, mon) = match (space, first) {
            (_, None) => (Key::Any, None),
            (Space::Window(w), Some(_)) => (Key::Window(w.id.clone()), None),
            (Space::Screen, Some((x, y))) => {
                let m = monitor_at(x, y).await?;
                (Key::Monitor(m.connector.clone()), Some(m))
            }
        };
        let mut g = LIVE.lock().await;
        if let Some(l) = g.as_ref()
            && (l.key == key || key == Key::Any)
        {
            return Ok(());
        }
        if super::holding() {
            return Err(anyhow!("The mouse button is still held. Say let go first."));
        }
        if let Some(old) = g.take() {
            close(&old).await;
        }
        if let Space::Window(w) = space
            && first.is_some()
        {
            front(w).await?;
        }
        *g = Some(open(key, mon).await?);
        Ok(())
    }

    pub async fn send(ev: Ev) -> Result<()> {
        if x11() {
            return xdo(ev).await;
        }
        let mut g = LIVE.lock().await;
        if g.is_none() {
            *g = Some(open(Key::Any, None).await?);
        }
        let l = g.as_mut().ok_or_else(|| anyhow!("no session"))?;
        let path = l.rd.as_str().to_owned();
        let c = l.c.clone();
        let call = |method: &'static str| (c.clone(), path.clone(), method);
        match ev {
            Ev::Move(x, y) => {
                let st = l.stream.clone().ok_or_else(|| anyhow!("The pointer needs a window to aim at."))?;
                let (sx, sy) = match &l.mon {
                    Some(m) => ((x - m.x) * l.factor, (y - m.y) * l.factor),
                    None => (x, y),
                };
                let (c, p, m) = call("NotifyPointerMotionAbsolute");
                c.call_method(Some(RD), p.as_str(), Some(RDS), m, &(st.as_str(), sx, sy)).await.map_err(|e| anyhow!("The pointer didn't move: {e}"))?;
            }
            Ev::Down(b, _) | Ev::Up(b, _) => {
                let (c, p, m) = call("NotifyPointerButton");
                c.call_method(Some(RD), p.as_str(), Some(RDS), m, &(code(b), matches!(ev, Ev::Down(..)))).await.map_err(|e| anyhow!("The mouse button didn't respond: {e}"))?;
            }
            Ev::Wheel { dx, dy, smooth, last } => {
                if smooth {
                    let flags: u32 = 4 | if last { 1 } else { 0 };
                    let (c, p, m) = call("NotifyPointerAxis");
                    c.call_method(Some(RD), p.as_str(), Some(RDS), m, &(dx * 10.0, dy * 10.0, flags)).await.map_err(|e| anyhow!("Scrolling didn't work: {e}"))?;
                } else {
                    for (axis, steps) in [(0u32, dy as i32), (1u32, dx as i32)] {
                        if steps != 0 {
                            let (c, p, m) = call("NotifyPointerAxisDiscrete");
                            c.call_method(Some(RD), p.as_str(), Some(RDS), m, &(axis, steps)).await.map_err(|e| anyhow!("Scrolling didn't work: {e}"))?;
                        }
                    }
                }
            }
            Ev::Pause(_) => {}
        }
        Ok(())
    }

    async fn xdo(ev: Ev) -> Result<()> {
        let num = |b: Button| match b {
            Button::Left => "1",
            Button::Right => "3",
            Button::Middle => "2",
        };
        let args: Vec<String> = match ev {
            Ev::Move(x, y) => vec!["mousemove".into(), format!("{}", x.round() as i64), format!("{}", y.round() as i64)],
            Ev::Down(b, _) => vec!["mousedown".into(), num(b).into()],
            Ev::Up(b, _) => vec!["mouseup".into(), num(b).into()],
            Ev::Wheel { dx, dy, .. } if dx == 0.0 && dy == 0.0 => return Ok(()),
            Ev::Wheel { dx, dy, .. } => {
                let (n, button) = if dy.abs() >= dx.abs() { (dy, if dy > 0.0 { "5" } else { "4" }) } else { (dx, if dx > 0.0 { "7" } else { "6" }) };
                let times = (n.abs().round() as i64).max(1);
                vec!["click".into(), "--repeat".into(), times.to_string(), button.into()]
            }
            Ev::Pause(_) => return Ok(()),
        };
        if crate::tools::apps::which("xdotool").is_none() {
            return Err(anyhow!("Pointer control on X11 needs xdotool. Install it and try again."));
        }
        let span = crate::layers::effect(crate::layers::Layer::Command, "command.xdotool");
        let ok = crate::layers::blocking(move || std::process::Command::new("xdotool").args(&args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false))
            .await
            .unwrap_or(false);
        span.end_bool(ok);
        if ok { Ok(()) } else { Err(anyhow!("xdotool couldn't move the pointer.")) }
    }
}

#[cfg(windows)]
mod imp {
    use super::{Button, Ev, Space};
    use anyhow::{Result, anyhow};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN};

    fn input(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
    }

    fn post(i: INPUT) -> Result<()> {
        let sent = unsafe { SendInput(&[i], std::mem::size_of::<INPUT>() as i32) };
        if sent == 1 { Ok(()) } else { Err(anyhow!("Windows blocked the mouse. The window may run as administrator.")) }
    }

    pub fn status() -> &'static str {
        "ready"
    }

    pub fn busy() {}

    pub fn idle() {}

    pub async fn prepare(_space: &Space, _first: Option<(f64, f64)>) -> Result<()> {
        Ok(())
    }

    pub async fn send(ev: Ev) -> Result<()> {
        match ev {
            Ev::Move(x, y) => {
                let (vx, vy, vw, vh) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
                let nx = ((x - vx as f64) * 65535.0 / (vw.max(2) - 1) as f64).round() as i32;
                let ny = ((y - vy as f64) * 65535.0 / (vh.max(2) - 1) as f64).round() as i32;
                post(input(nx, ny, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK))
            }
            Ev::Down(b, _) => post(input(0, 0, 0, match b {
                Button::Left => MOUSEEVENTF_LEFTDOWN,
                Button::Right => MOUSEEVENTF_RIGHTDOWN,
                Button::Middle => MOUSEEVENTF_MIDDLEDOWN,
            })),
            Ev::Up(b, _) => post(input(0, 0, 0, match b {
                Button::Left => MOUSEEVENTF_LEFTUP,
                Button::Right => MOUSEEVENTF_RIGHTUP,
                Button::Middle => MOUSEEVENTF_MIDDLEUP,
            })),
            Ev::Wheel { dx, dy, .. } => {
                if dy != 0.0 {
                    post(input(0, 0, (-dy * 120.0).round() as i32, MOUSEEVENTF_WHEEL))?;
                }
                if dx != 0.0 {
                    post(input(0, 0, (dx * 120.0).round() as i32, MOUSEEVENTF_HWHEEL))?;
                }
                Ok(())
            }
            Ev::Pause(_) => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Button, Ev, Space};
    use anyhow::{Result, anyhow};
    use std::ffi::c_void;
    use std::sync::Mutex;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *const c_void;
        fn CGEventGetLocation(e: *const c_void) -> CGPoint;
        fn CGEventCreateMouseEvent(source: *const c_void, kind: u32, at: CGPoint, button: u32) -> *const c_void;
        fn CGEventCreateScrollWheelEvent2(source: *const c_void, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> *const c_void;
        fn CGEventSetIntegerValueField(e: *const c_void, field: u32, value: i64);
        fn CGEventPost(tap: u32, e: *const c_void);
        fn CGMainDisplayID() -> u32;
        fn CGDisplayBounds(display: u32) -> CGRect;
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGPoint,
    }

    pub fn main_display_width() -> Option<f64> {
        let r = unsafe { CGDisplayBounds(CGMainDisplayID()) };
        (r.size.x > 0.0).then_some(r.size.x)
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    static AT: Mutex<Option<(f64, f64)>> = Mutex::new(None);
    static DOWN: Mutex<Vec<Button>> = Mutex::new(Vec::new());

    fn here() -> CGPoint {
        if let Some((x, y)) = *AT.lock().unwrap() {
            return CGPoint { x, y };
        }
        unsafe {
            let e = CGEventCreate(std::ptr::null());
            if e.is_null() {
                return CGPoint::default();
            }
            let p = CGEventGetLocation(e);
            CFRelease(e);
            p
        }
    }

    fn post(e: *const c_void) -> Result<()> {
        if e.is_null() {
            return Err(anyhow!("macOS blocked the mouse. Allow Setu in Privacy & Security, Accessibility."));
        }
        unsafe {
            CGEventPost(0, e);
            CFRelease(e);
        }
        Ok(())
    }

    fn num(b: Button) -> u32 {
        match b {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
        }
    }

    pub fn status() -> &'static str {
        "ready"
    }

    pub fn busy() {}

    pub fn idle() {
        *AT.lock().unwrap() = None;
    }

    pub async fn prepare(_space: &Space, _first: Option<(f64, f64)>) -> Result<()> {
        Ok(())
    }

    pub async fn send(ev: Ev) -> Result<()> {
        match ev {
            Ev::Move(x, y) => {
                *AT.lock().unwrap() = Some((x, y));
                let held = DOWN.lock().unwrap().first().copied();
                let (kind, button) = match held {
                    Some(Button::Left) => (6, 0),
                    Some(Button::Right) => (7, 1),
                    Some(Button::Middle) => (27, 2),
                    None => (5, 0),
                };
                post(unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, CGPoint { x, y }, button) })
            }
            Ev::Down(b, n) | Ev::Up(b, n) => {
                let down = matches!(ev, Ev::Down(..));
                let kind = match (b, down) {
                    (Button::Left, true) => 1,
                    (Button::Left, false) => 2,
                    (Button::Right, true) => 3,
                    (Button::Right, false) => 4,
                    (Button::Middle, true) => 25,
                    (Button::Middle, false) => 26,
                };
                let e = unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, here(), num(b)) };
                if !e.is_null() {
                    unsafe { CGEventSetIntegerValueField(e, 1, n.max(1) as i64) };
                }
                post(e)?;
                let mut d = DOWN.lock().unwrap();
                d.retain(|x| *x != b);
                if down {
                    d.push(b);
                }
                Ok(())
            }
            Ev::Wheel { dx, dy, .. } if dx == 0.0 && dy == 0.0 => Ok(()),
            Ev::Wheel { dx, dy, smooth, .. } => {
                let (units, k) = if smooth { (0, 40.0) } else { (1, 3.0) };
                post(unsafe { CGEventCreateScrollWheelEvent2(std::ptr::null(), units, 2, (-dy * k).round() as i32, (-dx * k).round() as i32, 0) })
            }
            Ev::Pause(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_of(t: &str) -> Option<(String, Value)> {
        match rules(t) {
            Some(crate::router::Intent::Act { tool, args }) => Some((tool, args)),
            _ => None,
        }
    }

    #[test]
    fn understands_pointer_phrases() {
        assert_eq!(tool_of("double click continue").map(|x| (x.0, x.1["count"].clone(), x.1["name"].clone())), Some(("pointer.click".into(), json!(2), json!("continue"))));
        assert_eq!(tool_of("double-click on the readme file").map(|x| x.1["name"].clone()), Some(json!("readme file")));
        assert_eq!(tool_of("right click here").map(|x| (x.0, x.1["button"].clone())), Some(("pointer.click".into(), json!("right"))));
        assert_eq!(tool_of("right click the file"), None);
        assert_eq!(tool_of("click continue"), None);
        assert_eq!(tool_of("middle click the link").map(|x| x.1["button"].clone()), Some(json!("middle")));
        assert_eq!(tool_of("hold the mouse here").map(|x| x.0), Some("pointer.hold".into()));
        assert_eq!(tool_of("let go").map(|x| x.0), Some("pointer.release".into()));
        assert_eq!(tool_of("hover over the menu").map(|x| (x.0, x.1["name"].clone())), Some(("pointer.hover".into(), json!("menu"))));
        let d = tool_of("drag this tab to the right").unwrap();
        assert_eq!((d.0.as_str(), d.1["name"].clone(), d.1["direction"].clone()), ("pointer.drag", json!("this tab"), json!("right")));
        let d = tool_of("drag this to the left a bit").unwrap();
        assert_eq!((d.1["direction"].clone(), d.1["amount"].clone()), (json!("left"), json!("bit")));
        let d = tool_of("drag the report onto the trash").unwrap();
        assert_eq!((d.1["name"].clone(), d.1["to"].clone()), (json!("report"), json!("trash")));
        assert_eq!(tool_of("drag this right").map(|x| x.1["direction"].clone()), Some(json!("right")));
        let s = tool_of("scroll down slowly").unwrap();
        assert_eq!((s.0.as_str(), s.1["direction"].clone(), s.1["smooth"].clone()), ("pointer.scroll", json!("down"), json!(true)));
        assert_eq!(tool_of("scroll right").map(|x| (x.1["direction"].clone(), x.1["smooth"].clone())), Some((json!("right"), json!(false))));
        assert_eq!(tool_of("scroll down"), None);
        assert_eq!(tool_of("scroll down a bit"), None);
    }

    #[test]
    fn drags_move_like_a_hand() {
        let p = drag_plan((100.0, 100.0), (400.0, 100.0), Button::Left);
        let moves = p.iter().filter(|e| matches!(e, Ev::Move(..))).count();
        let ms: u64 = p.iter().map(|e| if let Ev::Pause(m) = e { *m } else { 0 }).sum();
        assert!(moves >= 12, "{moves} moves");
        assert!((300..=900).contains(&ms), "{ms} ms");
        let down = p.iter().position(|e| matches!(e, Ev::Down(..))).unwrap();
        let up = p.iter().position(|e| matches!(e, Ev::Up(..))).unwrap();
        assert!(down < up && matches!(p[up - 1], Ev::Pause(m) if m >= 100));
        let xs: Vec<f64> = p.iter().filter_map(|e| if let Ev::Move(x, _) = e { Some(*x) } else { None }).collect();
        assert!(xs.windows(2).all(|w| w[1] >= w[0]), "{xs:?}");
        assert_eq!(p.iter().rev().find_map(|e| if let Ev::Move(x, y) = e { Some((*x, *y)) } else { None }), Some((400.0, 100.0)));
        let c = click_plan(Some((5.0, 5.0)), Button::Left, 2);
        assert_eq!(c.iter().filter(|e| matches!(e, Ev::Down(..))).count(), 2);
        let s = scroll_plan(None, 0.0, 5.0, true);
        let total: f64 = s.iter().map(|e| if let Ev::Wheel { dy, .. } = e { *dy } else { 0.0 }).sum();
        assert!((total - 5.0).abs() < 1e-9);
        assert!(matches!(s.iter().rev().find(|e| matches!(e, Ev::Wheel { .. })), Some(Ev::Wheel { last: true, .. })));
    }
}
