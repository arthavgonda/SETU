use crate::control::ControlMsg;
use crate::core::Core;
use crate::router::Intent;
use crate::text::{normalize, number_at};
use crate::tools::Outcome;
use rusqlite::params;
use serde_json::{Value, json};
use setu_common::protocol::now_ms;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug)]
struct Countdown {
    id: u64,
    kind: &'static str,
    label: String,
    ends: i64,
    total: i64,
}

#[derive(Clone, Debug)]
struct Watch {
    started: i64,
    carried: i64,
    running: bool,
}

#[derive(Default)]
struct Inner {
    timers: Vec<Countdown>,
    watch: Option<Watch>,
}

#[derive(Default)]
pub struct Board {
    inner: Mutex<Inner>,
    seq: AtomicU64,
}

pub const FOCUS_MIN: i64 = 25;
pub const BREAK_MIN: i64 = 5;

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.to_owned(), args }
}

fn unit_secs(w: &str) -> Option<i64> {
    Some(match w {
        "second" | "seconds" | "sec" | "secs" => 1,
        "minute" | "minutes" | "min" | "mins" => 60,
        "hour" | "hours" | "hr" | "hrs" => 3600,
        _ => return None,
    })
}

pub fn duration_secs(text: &str) -> Option<i64> {
    let t = normalize(text);
    let t = t.replace("an hour and a half", "90 minutes").replace("one and a half hours", "90 minutes").replace("half an hour", "30 minutes").replace("half hour", "30 minutes").replace("quarter of an hour", "15 minutes").replace("a quarter hour", "15 minutes");
    let words: Vec<&str> = t.split_whitespace().collect();
    let mut total = 0i64;
    let mut i = 0;
    let mut any = false;
    while i < words.len() {
        let (n, used) = if matches!(words[i], "a" | "an") && words.get(i + 1).and_then(|w| unit_secs(w)).is_some() {
            (1, 1)
        } else if let Some(x) = number_at(&words[i..]) {
            x
        } else {
            i += 1;
            continue;
        };
        if let Some(u) = words.get(i + used).and_then(|w| unit_secs(w)) {
            total += n * u;
            any = true;
            i += used + 1;
        } else {
            i += used.max(1);
        }
    }
    (any && total > 0).then_some(total.min(24 * 3600))
}

pub fn spoken_duration(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let part = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    let mut v = Vec::new();
    if h > 0 {
        v.push(part(h, "hour"));
    }
    if m > 0 {
        v.push(part(m, "minute"));
    }
    if s > 0 && h == 0 {
        v.push(part(s, "second"));
    }
    if v.is_empty() {
        return "0 seconds".into();
    }
    v.join(" and ")
}

fn clock_at(text: &str, now: &jiff::Zoned) -> Option<jiff::Zoned> {
    let t = normalize(text);
    let words: Vec<&str> = t.split_whitespace().collect();
    let tomorrow = words.contains(&"tomorrow");
    let evening = words.iter().any(|w| matches!(*w, "tonight" | "evening"));
    let morning = words.contains(&"morning");
    let afternoon = words.contains(&"afternoon");
    let mut hour: Option<i64> = None;
    let mut minute = 0i64;
    let mut meridiem: Option<bool> = None;
    if words.contains(&"noon") || words.contains(&"midday") {
        hour = Some(12);
        meridiem = Some(true);
    } else if words.contains(&"midnight") {
        hour = Some(0);
        meridiem = Some(false);
    }
    let at = words.iter().position(|w| *w == "at");
    if hour.is_none()
        && let Some(i) = at
    {
        let rest = &words[i + 1..];
        let first = rest.first().copied().unwrap_or("");
        let (h, used) = if let Some((a, b)) = first.split_once(':') {
            minute = b.parse().ok()?;
            (a.parse().ok()?, 1)
        } else if first.len() >= 3 && first.chars().all(|c| c.is_ascii_digit()) {
            let v: i64 = first.parse().ok()?;
            minute = v % 100;
            (v / 100, 1)
        } else {
            number_at(rest)?
        };
        hour = Some(h);
        let mut k = used;
        if minute == 0 {
            match rest.get(k).copied() {
                Some("thirty") => {
                    minute = 30;
                    k += 1;
                }
                Some("fifteen") => {
                    minute = 15;
                    k += 1;
                }
                Some("forty") if rest.get(k + 1) == Some(&"five") => {
                    minute = 45;
                    k += 2;
                }
                Some(w) if w.chars().all(|c| c.is_ascii_digit()) && w.len() == 2 => {
                    minute = w.parse().unwrap_or(0);
                    k += 1;
                }
                _ => {}
            }
        }
        let mer = rest.get(k).copied().unwrap_or("");
        let mer2 = format!("{mer}{}", rest.get(k + 1).copied().unwrap_or(""));
        if matches!(mer, "pm" | "p.m") || mer2 == "pm" {
            meridiem = Some(true);
        } else if matches!(mer, "am" | "a.m") || mer2 == "am" {
            meridiem = Some(false);
        }
    }
    let mut h = hour?;
    if !(0..=23).contains(&h) || !(0..=59).contains(&minute) {
        return None;
    }
    if meridiem.is_none() && h < 12 && (evening || afternoon) {
        meridiem = Some(true);
    }
    if meridiem.is_none() && morning {
        meridiem = Some(false);
    }
    match meridiem {
        Some(true) if h < 12 => h += 12,
        Some(false) if h == 12 => h = 0,
        _ => {}
    }
    let mut day = now.date();
    if tomorrow {
        day = day.tomorrow().ok()?;
    }
    let make = |d: jiff::civil::Date, h: i64| d.at(h as i8, minute as i8, 0, 0).to_zoned(now.time_zone().clone()).ok();
    let mut cand = make(day, h)?;
    if !tomorrow && meridiem.is_none() && h < 12 && cand <= *now {
        let pm = make(day, h + 12)?;
        if pm > *now {
            cand = pm;
        }
    }
    if cand <= *now {
        cand = make(day.tomorrow().ok()?, h)?;
    }
    Some(cand)
}

pub fn due_time(text: &str, now: &jiff::Zoned) -> Option<i64> {
    let t = normalize(text);
    if let Some(i) = t.find(" in ").map(|i| i + 4).or_else(|| t.strip_prefix("in ").map(|_| 3))
        && let Some(d) = duration_secs(&t[i..])
    {
        return Some(now.timestamp().as_millisecond() + d * 1000);
    }
    clock_at(&t, now).map(|z| z.timestamp().as_millisecond())
}

fn strip_time(text: &str) -> String {
    let t = normalize(text);
    let mut words: Vec<&str> = t.split_whitespace().collect();
    if let Some(i) = words.iter().position(|w| matches!(*w, "at" | "in" | "tomorrow" | "tonight")) {
        let tail = words[i..].join(" ");
        if duration_secs(&tail).is_some() || clock_at(&tail, &jiff::Zoned::now()).is_some() || matches!(words[i], "tomorrow" | "tonight") {
            words.truncate(i);
        }
    }
    words.join(" ").trim_start_matches("to ").trim().to_owned()
}

fn many_todos(t: &str) -> bool {
    let asks = ["make a to do list", "make my to do list", "create a to do list", "make a todo list", "create a todo list", "make a list", "prepare a to do list", "write a to do list", "make to do list", "create to do list", "make a checklist", "make a task list"];
    let Some(at) = asks.iter().filter_map(|a| t.find(a)).min() else { return false };
    let before = t[..at].split_whitespace().count();
    let after = &t[at..];
    let extra = asks.iter().filter_map(|a| after.strip_prefix(a)).map(|r| r.split_whitespace().count()).max().unwrap_or(0);
    before >= 4 || extra >= 3 || after.contains(" of these") || after.contains(" these things") || after.contains(" following") || after.contains(" with ") || after.contains(" for ")
}

pub fn split_tasks(t: &str) -> Vec<String> {
    split_body(&task_body(t))
}

fn task_body(t: &str) -> String {
    let asks = ["make a to do list", "make my to do list", "create a to do list", "make a todo list", "create a todo list", "make a list", "prepare a to do list", "write a to do list", "make to do list", "create to do list", "make a checklist", "make a task list"];
    let (before, after) = match asks.iter().filter_map(|a| t.find(a).map(|i| (i, i + a.len()))).min() {
        Some((i, j)) => (&t[..i], &t[j..]),
        None => (t, ""),
    };
    let mut lead = before.trim().trim_end_matches([',', ';']).trim().to_owned();
    for _ in 0..3 {
        for p in ["i want to", "i would like to", "i'd like to", "can you", "could you", "please", "now", "so", "ok", "okay", "i need to", "help me", "let's", "lets"] {
            if lead == p {
                lead.clear();
            } else if let Some(r) = lead.strip_prefix(&format!("{p} ")) {
                lead = r.trim().to_owned();
            }
        }
    }
    if lead.split_whitespace().count() >= 2 {
        lead
    } else {
        let mut a = after.trim().trim_start_matches([',', ';', ':']).trim().to_owned();
        for p in ["of these things", "of these", "with these", "with", "for", "of", ":", ","] {
            if let Some(r) = a.strip_prefix(p) {
                a = r.trim().to_owned();
            }
        }
        a
    }
}

fn split_body(body: &str) -> Vec<String> {
    let mut parts = vec![format!(" {body} ")];
    for sep in [" and after that ", " after that ", " and then ", " then ", " and ", ", ", "; "] {
        parts = parts.into_iter().flat_map(|p| p.split(sep).map(|x| format!(" {} ", x.trim())).collect::<Vec<_>>()).collect();
    }
    let leads = ["i am determined", "i'm determined", "i am going to", "i'm going to", "i will", "i'll", "i have to", "i need to", "i want to", "i must", "i should", "first", "then", "next", "finally", "also", "do", "to"];
    let mut out = Vec::new();
    for p in parts {
        let mut x = p.trim().to_owned();
        for _ in 0..4 {
            for l in leads {
                if x == l {
                    x.clear();
                } else if let Some(r) = x.strip_prefix(&format!("{l} ")) {
                    x = r.trim().to_owned();
                }
            }
        }
        if !x.is_empty() && x.split_whitespace().count() <= 14 {
            out.push(crate::text::sentence_case(&x));
        }
    }
    out.dedup();
    out.truncate(12);
    out
}

async fn extract_tasks(core: &Core, text: &str) -> Vec<String> {
    let system = "You turn a message into a to-do list. Return every separate task the person mentions, in order, each as a short phrase of at most six words that starts with a verb. Leave out the request to make the list. Answer as JSON: {\"items\": [\"...\"]}.";
    let body = task_body(text);
    let input = if body.split_whitespace().count() >= 2 { body } else { text.to_owned() };
    let r = tokio::time::timeout(Duration::from_secs(12), crate::pipeline::ask_json(core, system, &input, Some(crate::pipeline::list_grammar("items")), 160, 0.0)).await;
    let items: Vec<String> = match r {
        Ok(Ok(v)) => v.get("items").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|s| crate::text::sentence_case(s.trim().trim_end_matches('.'))).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
        _ => Vec::new(),
    };
    if items.is_empty() { split_tasks(text) } else { items }
}

fn todo_word(t: &str) -> bool {
    t.contains("to do") || t.contains("todo") || t.contains("task list") || t.contains("tasks") || t.contains("checklist")
}

pub fn rules(t: &str) -> Option<Intent> {
    let t = t.trim_end_matches(" please").trim();
    for p in ["set a timer for ", "set timer for ", "set the timer for ", "start a timer for ", "timer for ", "set an alarm for ", "count down ", "countdown "] {
        if let Some(r) = t.strip_prefix(p)
            && let Some(secs) = duration_secs(r)
        {
            let label = r.split_once(" for ").or_else(|| r.split_once(" called ")).map(|(_, l)| l.trim().to_owned()).filter(|l| duration_secs(l).is_none()).unwrap_or_default();
            return Some(act("board.timer", json!({ "seconds": secs, "label": label })));
        }
    }
    for p in ["set a ", "start a ", "set ", "start "] {
        if let Some(r) = t.strip_prefix(p)
            && let Some(x) = r.strip_suffix(" timer")
            && let Some(secs) = duration_secs(&x.replace('-', " "))
        {
            return Some(act("board.timer", json!({ "seconds": secs, "label": "" })));
        }
    }
    if matches!(t, "cancel the timer" | "stop the timer" | "cancel my timer" | "cancel timer" | "stop timer" | "delete the timer" | "cancel all timers" | "stop all timers" | "turn off the timer" | "remove the timer") {
        return Some(act("board.timer.cancel", json!({ "all": t.contains("all") })));
    }
    if matches!(t, "how much time is left" | "how much time left" | "time left" | "how long is left" | "how long left on the timer" | "how much time is left on the timer" | "how long is left on the timer" | "check the timer" | "timer status") {
        return Some(act("board.timer.status", json!({})));
    }
    if matches!(t, "start a stopwatch" | "start the stopwatch" | "start stopwatch" | "stopwatch" | "start timing" | "resume the stopwatch" | "continue the stopwatch") {
        return Some(act("board.stopwatch", json!({ "op": "start" })));
    }
    if matches!(t, "stop the stopwatch" | "stop stopwatch" | "pause the stopwatch" | "pause stopwatch" | "stop timing") {
        return Some(act("board.stopwatch", json!({ "op": "stop" })));
    }
    if matches!(t, "reset the stopwatch" | "reset stopwatch" | "clear the stopwatch" | "close the stopwatch") {
        return Some(act("board.stopwatch", json!({ "op": "reset" })));
    }
    if matches!(t, "how long has the stopwatch been running" | "stopwatch time" | "check the stopwatch" | "what's the stopwatch at" | "what is the stopwatch at") {
        return Some(act("board.stopwatch", json!({ "op": "status" })));
    }
    for p in ["start a focus session", "start focus session", "start focusing", "start focus", "start a focus timer", "start the focus timer", "start a pomodoro", "start pomodoro", "start the pomodoro", "focus for ", "let's focus", "time to focus", "begin a focus session"] {
        if t == p.trim_end() || t.starts_with(p) && p.ends_with(' ') || t.starts_with(&format!("{p} for ")) {
            let mins = duration_secs(t).map(|s| (s / 60).max(1)).unwrap_or(FOCUS_MIN);
            return Some(act("board.focus", json!({ "op": "start", "minutes": mins })));
        }
    }
    if matches!(t, "take a break" | "start a break" | "start my break" | "break time" | "time for a break" | "i need a break") {
        return Some(act("board.focus", json!({ "op": "break", "minutes": BREAK_MIN })));
    }
    if matches!(t, "stop focus" | "stop focusing" | "end focus" | "stop the focus timer" | "end the focus session" | "stop the focus session" | "stop pomodoro" | "stop the pomodoro" | "end the break" | "stop the break" | "skip the break") {
        return Some(act("board.focus", json!({ "op": "stop" })));
    }
    for p in ["remind me to ", "remind me ", "set a reminder to ", "set a reminder for ", "create a reminder to ", "add a reminder to "] {
        if let Some(r) = t.strip_prefix(p) {
            let lead = ["in ", "at ", "tomorrow", "tonight", "this evening"].iter().any(|x| r.starts_with(x));
            let what = match (lead, r.split_once(" to ")) {
                (true, Some((_, x))) => x.to_owned(),
                (true, None) => String::new(),
                _ => strip_time(r),
            };
            let what = what.trim_start_matches("to ").trim_start_matches("that ").trim();
            return Some(act("board.remind", json!({ "text": what, "when": r })));
        }
    }
    if matches!(t, "show my reminders" | "what are my reminders" | "list my reminders" | "read my reminders" | "any reminders") {
        return Some(act("board.reminders", json!({})));
    }
    for p in ["take a note that ", "take a note ", "make a note that ", "make a note ", "note that ", "add a note that ", "add a note ", "write down that ", "write down ", "jot down ", "save a note ", "new note "] {
        if let Some(r) = t.strip_prefix(p) {
            let r = r.trim_start_matches("saying ").trim();
            if !r.is_empty() {
                return Some(act("board.note", json!({ "text": r })));
            }
        }
    }
    if matches!(t, "show my notes" | "read my notes" | "what are my notes" | "open my notes" | "list my notes" | "show notes" | "my notes" | "read the notes") {
        return Some(act("board.notes", json!({})));
    }
    if matches!(t, "delete the last note" | "remove the last note" | "delete my last note") {
        return Some(act("board.notes", json!({ "deleteLast": true })));
    }
    if many_todos(t) {
        return Some(act("board.todo", json!({ "op": "many", "text": t })));
    }
    if todo_word(t) {
        for p in ["add ", "put "] {
            if let Some(r) = t.strip_prefix(p) {
                for s in [" to my to do list", " to the to do list", " to my todo list", " to the todo list", " to my to do", " to my todo", " to my tasks", " to my task list", " on my to do list", " on my todo list", " on the to do list", " to my checklist", " to do"] {
                    if let Some(x) = r.strip_suffix(s) {
                        let x = x.trim();
                        if !x.is_empty() {
                            return Some(act("board.todo", json!({ "op": "add", "text": x })));
                        }
                    }
                }
            }
        }
        for p in ["add a to do ", "add a todo ", "add a task ", "new to do ", "new todo ", "new task "] {
            if let Some(r) = t.strip_prefix(p) {
                return Some(act("board.todo", json!({ "op": "add", "text": r.trim_start_matches("to ").trim() })));
            }
        }
        if matches!(t, "show my to do list" | "show my todo list" | "what's on my to do list" | "what is on my to do list" | "read my to do list" | "show my tasks" | "what are my tasks" | "my to do list" | "open my to do list" | "show the to do list" | "what's on my todo list" | "list my tasks" | "read my tasks") {
            return Some(act("board.todo", json!({ "op": "list" })));
        }
        if matches!(t, "clear my to do list" | "clear the to do list" | "clear completed tasks" | "clear done tasks" | "remove finished tasks" | "clear my todo list") {
            return Some(act("board.todo", json!({ "op": "clear" })));
        }
    }
    for p in ["mark ", "check off ", "tick off ", "cross off ", "i finished ", "i've finished ", "i have finished ", "i did ", "i've done ", "complete ", "finish "] {
        if let Some(r) = t.strip_prefix(p) {
            let x = r.trim_end_matches(" as done").trim_end_matches(" as complete").trim_end_matches(" as completed").trim_end_matches(" done").trim_end_matches(" off").trim_end_matches(" from my to do list").trim_end_matches(" on my to do list").trim();
            if !x.is_empty() && (p.starts_with("mark") && r.contains(" as ") || !p.starts_with("mark") && !p.starts_with("finish") && !p.starts_with("complete") || todo_word(t)) {
                return Some(act("board.todo", json!({ "op": "done", "text": x })));
            }
        }
    }
    for p in ["remove ", "delete "] {
        if let Some(r) = t.strip_prefix(p)
            && todo_word(t)
        {
            let x = r.split(" from ").next().unwrap_or(r).trim();
            if !x.is_empty() {
                return Some(act("board.todo", json!({ "op": "remove", "text": x })));
            }
        }
    }
    None
}

impl Board {
    fn next(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed) + 1
    }
}

fn timers_json(v: &[Countdown]) -> Vec<Value> {
    v.iter().map(|c| json!({ "id": c.id, "kind": c.kind, "label": c.label, "endsAt": c.ends, "totalMs": c.total })).collect()
}

pub fn state(core: &Core) -> Value {
    let (timers, watch) = {
        let g = core.board.inner.lock().unwrap();
        (timers_json(&g.timers), g.watch.clone())
    };
    let now = now_ms();
    let (todos, notes, reminders) = core
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT id, text, done FROM todos ORDER BY done ASC, time DESC LIMIT 8")?;
            let todos: Vec<Value> = st.query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "text": r.get::<_, String>(1)?, "done": r.get::<_, i64>(2)? != 0 })))?.flatten().collect();
            let mut st = c.prepare("SELECT id, text, time FROM notes ORDER BY time DESC LIMIT 5")?;
            let notes: Vec<Value> = st.query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "text": r.get::<_, String>(1)?, "time": r.get::<_, i64>(2)? })))?.flatten().collect();
            let mut st = c.prepare("SELECT id, text, due FROM reminders WHERE fired = 0 AND due > ?1 ORDER BY due ASC LIMIT 5")?;
            let reminders: Vec<Value> = st.query_map([now - 60_000], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "text": r.get::<_, String>(1)?, "at": r.get::<_, i64>(2)? })))?.flatten().collect();
            let done: i64 = c.query_row("SELECT count(*) FROM todos WHERE done = 1 AND done_at > ?1", [now - 86_400_000], |r| r.get(0))?;
            let total: i64 = c.query_row("SELECT count(*) FROM todos WHERE done = 0 OR done_at > ?1", [now - 86_400_000], |r| r.get(0))?;
            Ok((json!({ "items": todos, "doneToday": done, "total": total }), notes, reminders))
        })
        .unwrap_or_else(|_| (json!({ "items": [], "doneToday": 0, "total": 0 }), Vec::new(), Vec::new()));
    json!({
        "timers": timers,
        "stopwatch": watch.map(|w| json!({ "startedAt": w.started, "carriedMs": w.carried, "running": w.running })),
        "todos": todos,
        "notes": notes,
        "reminders": reminders,
        "now": now,
    })
}

pub fn emit(core: &Core) {
    core.hub.emit("notch.state", state(core));
}

pub fn alert(core: &Core, kind: &str, text: &str) {
    crate::sounds::play(core, crate::sounds::Cue::Chime);
    core.hub.emit("notch.alert", json!({ "kind": kind, "text": text, "t": now_ms() }));
    core.tell(ControlMsg::Announce(text.to_owned()));
}

fn start(core: &Arc<Core>, kind: &'static str, secs: i64, label: String) -> u64 {
    let id = core.board.next();
    let now = now_ms();
    let c = Countdown { id, kind, label, ends: now + secs * 1000, total: secs * 1000 };
    {
        let mut g = core.board.inner.lock().unwrap();
        if kind != "timer" {
            g.timers.retain(|t| t.kind == "timer");
        }
        g.timers.push(c.clone());
    }
    emit(core);
    let core = core.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(secs as u64 * 1000)).await;
        let fired = {
            let mut g = core.board.inner.lock().unwrap();
            let had = g.timers.iter().any(|t| t.id == id);
            g.timers.retain(|t| t.id != id);
            had
        };
        if !fired {
            return;
        }
        emit(&core);
        match c.kind {
            "focus" => {
                alert(&core, "focus", &format!("Focus time is over. Take a {BREAK_MIN} minute break."));
                start(&core, "break", BREAK_MIN * 60, String::new());
            }
            "break" => alert(&core, "break", "Your break is over. Ready to focus again?"),
            _ => {
                let what = if c.label.is_empty() { format!("Your {} timer is done.", spoken_duration(c.total / 1000)) } else { format!("Your {} timer is done.", c.label) };
                alert(&core, "timer", &what);
            }
        }
    });
    id
}

fn schedule(core: &Arc<Core>, id: i64, text: String, due: i64) {
    let core = core.clone();
    tokio::spawn(async move {
        let wait = (due - now_ms()).max(0) as u64;
        tokio::time::sleep(Duration::from_millis(wait)).await;
        let live = core
            .db
            .read(move |c| Ok(c.query_row("SELECT fired FROM reminders WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).unwrap_or(1) == 0))
            .unwrap_or(false);
        if !live {
            return;
        }
        core.db.write(move |c| {
            c.execute("UPDATE reminders SET fired = 1 WHERE id = ?1", [id])?;
            Ok(())
        });
        alert(&core, "reminder", &format!("Reminder: {text}."));
        emit(&core);
    });
}

pub fn restore(core: &Arc<Core>) {
    let now = now_ms();
    let pending: Vec<(i64, String, i64)> = core
        .db
        .read(move |c| {
            let mut st = c.prepare("SELECT id, text, due FROM reminders WHERE fired = 0 AND due > ?1")?;
            let v = st.query_map([now - 6 * 3_600_000], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.flatten().collect();
            Ok(v)
        })
        .unwrap_or_default();
    for (id, text, due) in pending {
        schedule(core, id, text, due);
    }
}

fn ok(did: &str) -> Outcome {
    Outcome::ok_with(did, "registered", None)
}

fn says(did: &str, reply: String) -> Outcome {
    Outcome::ok_with(did, "registered", None).with_reply(reply)
}

fn left_text(ms: i64) -> String {
    spoken_duration(((ms + 999) / 1000).max(0))
}

fn best_match(items: &[(i64, String)], said: &str) -> Option<i64> {
    let s = normalize(said);
    items
        .iter()
        .map(|(id, text)| (*id, crate::text::similarity(&s, text).max(if normalize(text).contains(&s) { 0.9 } else { 0.0 })))
        .filter(|(_, score)| *score >= 0.55)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(id, _)| id)
}

pub async fn run(core: &Arc<Core>, tool: &str, args: Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned();
    let n = |k: &str| args.get(k).and_then(Value::as_i64);
    match tool {
        "board.timer" => {
            let secs = n("seconds").unwrap_or(60).clamp(1, 24 * 3600);
            let label = s("label");
            start(core, "timer", secs, label.clone());
            let what = spoken_duration(secs);
            says(&format!("Timer for {what}"), if label.is_empty() { format!("Timer set for {what}.") } else { format!("{} timer set for {what}.", crate::text::sentence_case(&label)) })
        }
        "board.timer.cancel" => {
            let removed = {
                let mut g = core.board.inner.lock().unwrap();
                let before = g.timers.len();
                if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
                    g.timers.retain(|t| t.kind != "timer");
                } else if let Some(pos) = g.timers.iter().rposition(|t| t.kind == "timer") {
                    g.timers.remove(pos);
                }
                before - g.timers.len()
            };
            emit(core);
            if removed == 0 {
                return Outcome::fail("There is no timer running.", "registered");
            }
            ok(if removed == 1 { "Timer cancelled" } else { "Timers cancelled" })
        }
        "board.timer.status" => {
            let now = now_ms();
            let t = core.board.inner.lock().unwrap().timers.iter().min_by_key(|t| t.ends).cloned();
            match t {
                Some(t) => says("Checked the timer", format!("{} left.", crate::text::sentence_case(&left_text(t.ends - now)))),
                None => Outcome::fail("There is no timer running.", "registered"),
            }
        }
        "board.stopwatch" => {
            let now = now_ms();
            let op = s("op");
            let mut g = core.board.inner.lock().unwrap();
            let r = match op.as_str() {
                "start" => {
                    match g.watch.as_mut() {
                        Some(w) if !w.running => {
                            w.started = now;
                            w.running = true;
                        }
                        Some(_) => {}
                        None => g.watch = Some(Watch { started: now, carried: 0, running: true }),
                    }
                    ok("Stopwatch started")
                }
                "stop" => match g.watch.as_mut() {
                    Some(w) if w.running => {
                        w.carried += now - w.started;
                        w.running = false;
                        let total = w.carried;
                        says("Stopwatch stopped", format!("Stopped at {}.", left_text(total)))
                    }
                    _ => Outcome::fail("The stopwatch isn't running.", "registered"),
                },
                "reset" => {
                    g.watch = None;
                    ok("Stopwatch reset")
                }
                _ => match g.watch.as_ref() {
                    Some(w) => {
                        let total = w.carried + if w.running { now - w.started } else { 0 };
                        says("Checked the stopwatch", format!("{}.", crate::text::sentence_case(&left_text(total))))
                    }
                    None => Outcome::fail("The stopwatch isn't running.", "registered"),
                },
            };
            drop(g);
            emit(core);
            r
        }
        "board.focus" => match s("op").as_str() {
            "start" => {
                let mins = n("minutes").unwrap_or(FOCUS_MIN).clamp(1, 240);
                start(core, "focus", mins * 60, String::new());
                says("Focus started", format!("Focus for {mins} minutes. I'll tell you when it's time for a break."))
            }
            "break" => {
                let mins = n("minutes").unwrap_or(BREAK_MIN).clamp(1, 60);
                start(core, "break", mins * 60, String::new());
                says("Break started", format!("Enjoy a {mins} minute break."))
            }
            _ => {
                let had = {
                    let mut g = core.board.inner.lock().unwrap();
                    let before = g.timers.len();
                    g.timers.retain(|t| t.kind == "timer");
                    before != g.timers.len()
                };
                emit(core);
                if had { ok("Focus stopped") } else { Outcome::fail("There is no focus session running.", "registered") }
            }
        },
        "board.remind" => {
            let text = s("text");
            let when = s("when");
            if text.is_empty() {
                return Outcome::fail("What should I remind you about? Say, for example, remind me to call mom at six.", "registered");
            }
            let now = jiff::Zoned::now();
            let Some(due) = due_time(&when, &now) else {
                return Outcome::fail("When should I remind you? Say, for example, remind me to call mom at six.", "registered");
            };
            let t2 = text.clone();
            let id = core
                .db
                .write_sync(move |c| {
                    c.execute("INSERT INTO reminders(time, due, text) VALUES(?1, ?2, ?3)", params![now_ms(), due, t2])?;
                    Ok(c.last_insert_rowid())
                })
                .unwrap_or(0);
            schedule(core, id, text.clone(), due);
            emit(core);
            let at = jiff::Timestamp::from_millisecond(due).map(|ts| ts.to_zoned(now.time_zone().clone())).ok();
            let spoken = at
                .map(|z| {
                    let day = if z.date() == now.date() { String::new() } else if Some(z.date()) == now.date().tomorrow().ok() { " tomorrow".into() } else { format!(" on {}", z.strftime("%A")) };
                    format!("{}{day}", z.strftime("%-I:%M %p"))
                })
                .unwrap_or_default();
            says(&format!("Reminder at {spoken}"), format!("I'll remind you to {text} at {spoken}."))
        }
        "board.reminders" => {
            let st = state(core);
            let list: Vec<String> = st["reminders"].as_array().map(|a| a.iter().filter_map(|r| r["text"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
            if list.is_empty() {
                return says("No reminders", "You have no reminders.".into());
            }
            says("Read the reminders", format!("You have {} reminder{}: {}.", list.len(), if list.len() == 1 { "" } else { "s" }, list.join(", ")))
        }
        "board.note" => {
            let text = crate::text::sentence_case(&s("text"));
            let t2 = text.clone();
            core.db.write(move |c| {
                c.execute("INSERT INTO notes(time, text) VALUES(?1, ?2)", params![now_ms(), t2])?;
                Ok(())
            });
            tokio::time::sleep(Duration::from_millis(60)).await;
            emit(core);
            core.hub.emit("notch.alert", json!({ "kind": "notes", "text": text, "t": now_ms() }));
            ok("Note saved")
        }
        "board.notes" => {
            if args.get("deleteLast").and_then(Value::as_bool).unwrap_or(false) {
                core.db.write(|c| {
                    c.execute("DELETE FROM notes WHERE id = (SELECT id FROM notes ORDER BY time DESC LIMIT 1)", [])?;
                    Ok(())
                });
                tokio::time::sleep(Duration::from_millis(60)).await;
                emit(core);
                return ok("Deleted the last note");
            }
            let st = state(core);
            core.hub.emit("notch.alert", json!({ "kind": "notes", "text": "", "t": now_ms() }));
            let list: Vec<String> = st["notes"].as_array().map(|a| a.iter().take(3).filter_map(|r| r["text"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
            if list.is_empty() {
                return says("No notes", "You have no notes yet. Say take a note, then what to write.".into());
            }
            says("Read the notes", format!("Your latest notes: {}.", list.join(". ")))
        }
        "board.todo" => {
            let op = s("op");
            let text = s("text");
            let items: Vec<(i64, String)> = core
                .db
                .read(|c| {
                    let mut st = c.prepare("SELECT id, text FROM todos WHERE done = 0 ORDER BY time DESC LIMIT 100")?;
                    let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
                    Ok(v)
                })
                .unwrap_or_default();
            let out = match op.as_str() {
                "many" => {
                    let given: Vec<String> = args.get("items").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|x| crate::text::sentence_case(x.trim().trim_end_matches('.'))).filter(|x| !x.is_empty()).take(12).collect()).unwrap_or_default();
                    let tasks = if given.is_empty() { extract_tasks(core, &text).await } else { given };
                    if tasks.is_empty() {
                        Outcome::fail("I couldn't find the tasks in that. Say, for example, add buy milk to my to-do list.", "registered")
                    } else {
                        let rows = tasks.clone();
                        let _ = core.db.write_sync(move |c| {
                            let tx = c.transaction()?;
                            let now = now_ms();
                            for (k, t) in rows.iter().enumerate() {
                                tx.execute("INSERT INTO todos(time, text) VALUES(?1, ?2)", params![now + k as i64, t])?;
                            }
                            tx.commit()?;
                            Ok(())
                        });
                        let n = tasks.len();
                        let spoken = match tasks.split_last() {
                            Some((last, rest)) if !rest.is_empty() => format!("{} and {}", rest.join(", "), last.to_lowercase()),
                            _ => tasks.join(""),
                        };
                        says(&format!("Added {n} things to your to-do list"), format!("Added {n} thing{} to your to-do list: {spoken}.", if n == 1 { "" } else { "s" }))
                    }
                }
                "add" => {
                    let t2 = crate::text::sentence_case(&text);
                    let shown = t2.clone();
                    core.db.write(move |c| {
                        c.execute("INSERT INTO todos(time, text) VALUES(?1, ?2)", params![now_ms(), t2])?;
                        Ok(())
                    });
                    ok(&format!("Added {shown} to your to-do list"))
                }
                "done" | "remove" => match best_match(&items, &text) {
                    Some(id) => {
                        let label = items.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone()).unwrap_or_default();
                        let remove = op == "remove";
                        core.db.write(move |c| {
                            if remove {
                                c.execute("DELETE FROM todos WHERE id = ?1", [id])?;
                            } else {
                                c.execute("UPDATE todos SET done = 1, done_at = ?2 WHERE id = ?1", params![id, now_ms()])?;
                            }
                            Ok(())
                        });
                        if remove { ok(&format!("Removed {label}")) } else { ok(&format!("Marked {label} as done")) }
                    }
                    None => Outcome::fail(&format!("I couldn't find {text} on your to-do list."), "registered"),
                },
                "clear" => {
                    core.db.write(|c| {
                        c.execute("DELETE FROM todos WHERE done = 1", [])?;
                        Ok(())
                    });
                    ok("Cleared finished tasks")
                }
                _ => {
                    let open: Vec<String> = items.iter().take(5).map(|(_, t)| t.clone()).collect();
                    if open.is_empty() {
                        says("Read the to-do list", "Your to-do list is empty.".into())
                    } else {
                        says("Read the to-do list", format!("You have {} thing{} to do: {}.", items.len(), if items.len() == 1 { "" } else { "s" }, open.join(", ")))
                    }
                }
            };
            tokio::time::sleep(Duration::from_millis(60)).await;
            emit(core);
            core.hub.emit("notch.alert", json!({ "kind": "todos", "text": "", "t": now_ms() }));
            out
        }
        _ => Outcome::fail("I can't do that yet.", "registered"),
    }
}

pub async fn action(core: &Arc<Core>, p: &Value) -> setu_common::protocol::ApiResult {
    use setu_common::protocol::ApiError;
    let what = p.get("action").and_then(Value::as_str).unwrap_or_default();
    let id = p.get("id").and_then(Value::as_i64);
    match what {
        "timer.cancel" => {
            core.board.inner.lock().unwrap().timers.retain(|t| Some(t.id as i64) != id);
            crate::sounds::play(core, crate::sounds::Cue::Cancel);
        }
        "focus.stop" => {
            core.board.inner.lock().unwrap().timers.retain(|t| t.kind == "timer");
            crate::sounds::play(core, crate::sounds::Cue::Cancel);
        }
        "focus.break" => {
            start(core, "break", BREAK_MIN * 60, String::new());
            crate::sounds::play(core, crate::sounds::Cue::Toggle);
        }
        "focus.start" => {
            start(core, "focus", FOCUS_MIN * 60, String::new());
            crate::sounds::play(core, crate::sounds::Cue::Toggle);
        }
        "stopwatch.start" | "stopwatch.stop" | "stopwatch.reset" => {
            let op = what.trim_start_matches("stopwatch.");
            let _ = run(core, "board.stopwatch", json!({ "op": op })).await;
            crate::sounds::play(core, crate::sounds::Cue::Toggle);
        }
        "todo.toggle" => {
            let Some(id) = id else { return Err(ApiError::invalid("Which task?")) };
            core.db
                .write_async(move |c| {
                    c.execute("UPDATE todos SET done = 1 - done, done_at = CASE WHEN done = 0 THEN ?2 ELSE NULL END WHERE id = ?1", params![id, now_ms()])?;
                    Ok(())
                })
                .await
                .map_err(|e| ApiError::unavailable(e.to_string()))?;
            crate::sounds::play(core, crate::sounds::Cue::Toggle);
        }
        "reminder.cancel" => {
            let Some(id) = id else { return Err(ApiError::invalid("Which reminder?")) };
            core.db
                .write_async(move |c| {
                    c.execute("UPDATE reminders SET fired = 1 WHERE id = ?1", [id])?;
                    Ok(())
                })
                .await
                .map_err(|e| ApiError::unavailable(e.to_string()))?;
        }
        _ => return Err(ApiError::invalid("Unknown notch action.")),
    }
    emit(core);
    Ok(state(core))
}

pub fn label(tool: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    match tool {
        "board.timer" => format!("Timer for {}", spoken_duration(args.get("seconds").and_then(Value::as_i64).unwrap_or(60))),
        "board.timer.cancel" => "Cancelling the timer".into(),
        "board.timer.status" => "Checking the timer".into(),
        "board.stopwatch" => "Stopwatch".into(),
        "board.focus" => match s("op").as_str() {
            "start" => "Focus time".into(),
            "break" => "Break time".into(),
            _ => "Stopping focus".into(),
        },
        "board.remind" => format!("Reminder: {}", s("text")),
        "board.reminders" => "Reminders".into(),
        "board.note" => "Saving the note".into(),
        "board.notes" => "Notes".into(),
        "board.todo" => match s("op").as_str() {
            "add" => format!("Adding {}", s("text")),
            "done" => format!("Marking {} as done", s("text")),
            "remove" => format!("Removing {}", s("text")),
            _ => "To-do list".into(),
        },
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(t: &str) -> (String, Value) {
        match rules(&normalize(t)) {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => (format!("{other:?}"), Value::Null),
        }
    }

    #[test]
    fn reads_durations() {
        assert_eq!(duration_secs("10 minutes"), Some(600));
        assert_eq!(duration_secs("ten minutes"), Some(600));
        assert_eq!(duration_secs("an hour"), Some(3600));
        assert_eq!(duration_secs("1 minute 30 seconds"), Some(90));
        assert_eq!(duration_secs("half an hour"), Some(1800));
        assert_eq!(duration_secs("two hours and fifteen minutes"), Some(8100));
        assert_eq!(duration_secs("pasta"), None);
        assert_eq!(spoken_duration(90), "1 minute and 30 seconds");
        assert_eq!(spoken_duration(3600), "1 hour");
    }

    #[test]
    fn understands_notch_commands() {
        let (t, a) = tool("set a timer for 10 minutes");
        assert_eq!((t.as_str(), a["seconds"].as_i64()), ("board.timer", Some(600)));
        let (_, a) = tool("set a timer for 5 minutes for the pasta");
        assert_eq!(a["label"], "the pasta");
        assert_eq!(tool("start a 15 minute timer").1["seconds"], 900);
        assert_eq!(tool("cancel the timer").0, "board.timer.cancel");
        assert_eq!(tool("start a stopwatch").1["op"], "start");
        assert_eq!(tool("stop the stopwatch").1["op"], "stop");
        let (t, a) = tool("start focus");
        assert_eq!((t.as_str(), a["minutes"].as_i64()), ("board.focus", Some(25)));
        assert_eq!(tool("focus for 50 minutes").1["minutes"], 50);
        assert_eq!(tool("take a break").1["op"], "break");
        let (t, a) = tool("remind me to call mom at six");
        assert_eq!((t.as_str(), a["text"].as_str()), ("board.remind", Some("call mom")));
        assert_eq!(tool("remind me in 10 minutes to check the oven").1["text"], "check the oven");
        let (t, a) = tool("take a note the wifi password is on the fridge");
        assert_eq!((t.as_str(), a["text"].as_str()), ("board.note", Some("the wifi password is on the fridge")));
        assert_eq!(tool("show my notes").0, "board.notes");
        let (t, a) = tool("add buy milk to my to-do list");
        assert_eq!((t.as_str(), a["op"].as_str(), a["text"].as_str()), ("board.todo", Some("add"), Some("buy milk")));
        let long = "i am determined i will do 2 hour coding after that i will drink water and after that i will do to play make a to do list of these things";
        let (t, a) = tool(long);
        assert_eq!((t.as_str(), a["op"].as_str()), ("board.todo", Some("many")));
        assert_eq!(split_tasks(long), vec!["2 hour coding", "Drink water", "Play"]);
        assert_eq!(split_tasks("make a to do list with buy milk, call mom and finish the report"), vec!["Buy milk", "Call mom", "Finish the report"]);
        assert_eq!(tool("mark buy milk as done").1["op"], "done");
        assert_eq!(tool("what's on my to do list").1["op"], "list");
        assert_eq!(tool("open firefox").0, "None");
        assert_eq!(tool("focus mode").0, "None");
    }

    #[test]
    fn finds_due_times() {
        let tz = jiff::tz::TimeZone::UTC;
        let now = jiff::civil::date(2026, 11, 6).at(9, 0, 0, 0).to_zoned(tz).unwrap();
        let at = |t: &str| due_time(t, &now).and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok()).map(|ts| ts.to_zoned(jiff::tz::TimeZone::UTC).strftime("%d %H:%M").to_string());
        assert_eq!(at("at six").as_deref(), Some("06 18:00"));
        assert_eq!(at("at 6 am").as_deref(), Some("07 06:00"));
        assert_eq!(at("at 10").as_deref(), Some("06 10:00"));
        assert_eq!(at("at 6:30 pm").as_deref(), Some("06 18:30"));
        assert_eq!(at("at six thirty").as_deref(), Some("06 18:30"));
        assert_eq!(at("tomorrow at 9").as_deref(), Some("07 09:00"));
        assert_eq!(at("in 10 minutes").as_deref(), Some("06 09:10"));
        assert_eq!(at("at noon").as_deref(), Some("06 12:00"));
        assert_eq!(at("soon"), None);
    }
}
