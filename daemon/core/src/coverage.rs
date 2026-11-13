use crate::router::{self, Intent};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

static MAP: &str = include_str!("../assets/coverage.json");

pub fn os_key() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

pub fn routed(text: &str) -> Option<String> {
    match router::rules(text, "stop", false) {
        Some(Intent::Act { tool, .. }) => Some(tool),
        Some(i) => Some(i.action().to_owned()),
        None => None,
    }
}

fn per_os<'a>(v: &'a Value, os: &str) -> &'a str {
    match v {
        Value::String(s) => s.as_str(),
        Value::Object(o) => o.get(os).and_then(Value::as_str).unwrap_or("gap"),
        _ => "gap",
    }
}

pub struct Entry {
    pub id: String,
    pub area: String,
    pub what: String,
    pub say: Option<String>,
    pub tool: Option<String>,
    pub target: String,
    pub today: String,
    pub phase: Option<i64>,
    pub note: String,
}

pub fn entries(os: &str) -> Vec<Entry> {
    let map: Value = serde_json::from_str(MAP).unwrap_or(Value::Null);
    let s = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
    map.get("actions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| Entry {
                    id: s(e, "id").unwrap_or_default(),
                    area: s(e, "area").unwrap_or_default(),
                    what: s(e, "what").unwrap_or_default(),
                    say: s(e, "say"),
                    tool: s(e, "tool"),
                    target: per_os(e.get("target").unwrap_or(&Value::Null), os).to_owned(),
                    today: per_os(e.get("today").unwrap_or(&Value::Null), os).to_owned(),
                    phase: match e.get("phase") {
                        Some(Value::Object(o)) => o.get(os).and_then(Value::as_i64),
                        Some(v) => v.as_i64(),
                        None => None,
                    },
                    note: s(e, "note").unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct Area {
    total: usize,
    covered: usize,
    at_target: usize,
    out: usize,
    understood: usize,
    spoken: usize,
}

pub fn check(os: &str) -> Value {
    let list = entries(os);
    let mut areas: BTreeMap<String, Area> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut wrong = Vec::new();
    let mut by_layer: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_phase: BTreeMap<i64, usize> = BTreeMap::new();
    for e in &list {
        let a = areas.entry(e.area.clone()).or_default();
        let got = e.say.as_deref().and_then(routed);
        let route_ok = match (&e.say, &e.tool) {
            (None, _) => true,
            (Some(_), want) => got == *want,
        };
        if e.today == "out" {
            a.out += 1;
        } else {
            a.total += 1;
            if e.today != "gap" {
                a.covered += 1;
                *by_layer.entry(e.today.clone()).or_default() += 1;
            }
            if e.today == e.target {
                a.at_target += 1;
            } else if let Some(p) = e.phase {
                *by_phase.entry(p).or_default() += 1;
            }
            if e.say.is_some() {
                a.spoken += 1;
                if got.is_some() {
                    a.understood += 1;
                }
            }
        }
        if !route_ok {
            wrong.push(json!({ "id": e.id, "say": e.say, "map": e.tool, "rules": got }));
        }
        rows.push(json!({ "id": e.id, "area": e.area, "what": e.what, "say": e.say, "tool": e.tool, "routed": got, "routeOk": route_ok, "today": e.today, "target": e.target, "phase": e.phase, "note": e.note }));
    }
    let mut am = Map::new();
    let mut sum = Area::default();
    for (k, a) in &areas {
        am.insert(k.clone(), json!({ "actions": a.total, "covered": a.covered, "atTarget": a.at_target, "outOfScope": a.out, "spoken": a.spoken, "understood": a.understood }));
        sum.total += a.total;
        sum.covered += a.covered;
        sum.at_target += a.at_target;
        sum.out += a.out;
        sum.spoken += a.spoken;
        sum.understood += a.understood;
    }
    json!({
        "os": os,
        "version": setu_common::VERSION,
        "areas": am,
        "total": { "actions": sum.total, "covered": sum.covered, "atTarget": sum.at_target, "outOfScope": sum.out, "spoken": sum.spoken, "understood": sum.understood },
        "byLayer": by_layer,
        "toDoByPhase": by_phase.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<String, Value>>(),
        "wrong": wrong,
        "entries": rows,
    })
}

async fn probes() -> Value {
    let mut m = Map::new();
    let ms = |t: Instant| (t.elapsed().as_micros() as f64 / 100.0).round() / 10.0;
    let t = Instant::now();
    let ok = tokio::time::timeout(std::time::Duration::from_secs(4), crate::os::ready()).await.is_ok_and(|r| r.is_ok());
    m.insert("accessibility.ready".into(), json!({ "ok": ok, "ms": ms(t) }));
    let t = Instant::now();
    let wins = tokio::time::timeout(std::time::Duration::from_secs(6), crate::os::windows()).await.ok().and_then(Result::ok);
    m.insert("accessibility.windows".into(), json!({ "ok": wins.is_some(), "count": wins.as_ref().map(Vec::len), "ms": ms(t) }));
    m.insert("keyboard".into(), json!({ "ok": crate::tools::input::status() == "ready", "status": crate::tools::input::status() }));
    let t = Instant::now();
    let l = crate::tools::media::list().await;
    m.insert("media.sessions".into(), json!({ "ok": l.known, "count": l.players.len(), "ms": ms(t) }));
    let t = Instant::now();
    let v = crate::layers::blocking(crate::tools::system::volume).await.ok().and_then(Result::ok);
    m.insert("volume.read".into(), json!({ "ok": v.is_some(), "ms": ms(t) }));
    let t = Instant::now();
    let b = crate::tools::system::backlight().is_ok();
    m.insert("brightness.read".into(), json!({ "ok": b, "ms": ms(t) }));
    m.insert("pointer".into(), json!({ "ok": true, "status": crate::tools::pointer::status() }));
    m.insert("vision".into(), json!({ "ok": false, "status": "not built" }));
    let service = if crate::privileged::installed() { crate::privileged::call("ping", json!({}), std::time::Duration::from_secs(2)).await.is_ok() } else { false };
    m.insert("service".into(), json!({ "ok": service, "status": if service { "running" } else if crate::privileged::installed() { "not answering" } else { "not set up" } }));
    let connected = crate::proto::any().is_some();
    m.insert("protocol".into(), json!({ "ok": connected, "status": if connected { "connected" } else { "built, no Setu profile running" } }));
    Value::Object(m)
}

fn pct(a: usize, b: usize) -> String {
    (a * 100 + b / 2).checked_div(b).map(|v| format!("{v}%")).unwrap_or_else(|| "-".into())
}

fn print_table(r: &Value) {
    println!("Setu {} coverage on {}", r["version"].as_str().unwrap_or(""), r["os"].as_str().unwrap_or(""));
    println!();
    println!("{:<12} {:>7} {:>8} {:>6} {:>10} {:>6} {:>11}", "area", "actions", "covered", "", "at target", "", "understood");
    let line = |name: &str, a: &Value| {
        let n = |k: &str| a[k].as_u64().unwrap_or(0) as usize;
        println!(
            "{:<12} {:>7} {:>8} {:>6} {:>10} {:>6} {:>7}/{:<3}",
            name,
            n("actions"),
            n("covered"),
            pct(n("covered"), n("actions")),
            n("atTarget"),
            pct(n("atTarget"), n("actions")),
            n("understood"),
            n("spoken")
        );
    };
    let mut out = Vec::new();
    if let Some(o) = r["areas"].as_object() {
        for (k, a) in o {
            if a["actions"].as_u64() == Some(0) && a["outOfScope"].as_u64().unwrap_or(0) > 0 {
                out.push(format!("{k} {}", a["outOfScope"]));
                continue;
            }
            line(k, a);
        }
    }
    line("all", &r["total"]);
    if !out.is_empty() {
        println!("out of scope, no software reaches it: {}", out.join(", "));
    }
    println!();
    println!("Served today, by layer:");
    if let Some(o) = r["byLayer"].as_object() {
        for (k, v) in o {
            println!("  {k:<14} {v}");
        }
    }
    println!();
    println!("Not at target yet, by the phase that closes it:");
    if let Some(o) = r["toDoByPhase"].as_object() {
        for (k, v) in o {
            println!("  Phase {k:<4} {v}");
        }
    }
    if let Some(w) = r["wrong"].as_array().filter(|w| !w.is_empty()) {
        println!();
        println!("The map and the rules disagree on {} entries:", w.len());
        for x in w {
            println!("  {}: \"{}\" map says {}, rules say {}", x["id"].as_str().unwrap_or(""), x["say"].as_str().unwrap_or(""), x["map"], x["rules"]);
        }
    }
    if let Some(p) = r["probes"].as_object() {
        println!();
        println!("Layers on this PC:");
        for (k, v) in p {
            let extra = v.get("ms").map(|m| format!("{m} ms")).or_else(|| v.get("status").and_then(Value::as_str).map(str::to_owned)).unwrap_or_default();
            println!("  {:<24} {:<5} {}", k, if v["ok"].as_bool().unwrap_or(false) { "ok" } else { "no" }, extra);
        }
    }
}

pub async fn run(args: &[String]) -> i32 {
    let os = args.iter().position(|a| a == "--os").and_then(|i| args.get(i + 1)).map(String::as_str).unwrap_or(os_key()).to_owned();
    let mut r = check(&os);
    if os == os_key() && !args.iter().any(|a| a == "--no-probes") {
        r["probes"] = probes().await;
    }
    if args.iter().any(|a| a == "--json") {
        println!("{r}");
    } else {
        print_table(&r);
    }
    if r["wrong"].as_array().is_some_and(|w| !w.is_empty()) { 1 } else { 0 }
}

pub fn route_lines() -> i32 {
    use std::io::BufRead;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let v = match router::rules(t, "stop", false) {
            Some(Intent::Act { tool, args }) => json!({ "say": t, "tool": tool, "args": args }),
            Some(i) => json!({ "say": t, "tool": i.action(), "args": i.args() }),
            None => json!({ "say": t, "tool": null }),
        };
        println!("{v}");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_matches_the_rules_on_every_system() {
        for os in ["linux", "windows", "macos"] {
            let r = check(os);
            assert!(r["total"]["actions"].as_u64().unwrap_or(0) > 100, "{os}: the map is too small");
            assert_eq!(r["wrong"].as_array().map(Vec::len), Some(0), "{os}: {}", r["wrong"]);
        }
    }

    #[test]
    fn every_entry_is_complete() {
        let layers = ["system", "channel", "protocol", "accessibility", "keyboard", "pointer", "vision", "service", "command", "setu", "gap", "out"];
        let mut seen = std::collections::HashSet::new();
        for os in ["linux", "windows", "macos"] {
            for e in entries(os) {
                assert!(!e.id.is_empty() && !e.area.is_empty() && !e.what.is_empty(), "{}", e.id);
                assert!(layers.contains(&e.today.as_str()), "{} today {}", e.id, e.today);
                assert!(layers.contains(&e.target.as_str()) && e.target != "gap" && e.target != "command", "{} target {}", e.id, e.target);
                if e.today != e.target && e.today != "out" {
                    assert!(e.phase.is_some() || e.note.len() > 20, "{} is not at target and names no phase or reason", e.id);
                }
                if os == "linux" {
                    assert!(seen.insert(e.id.clone()), "{} appears twice", e.id);
                }
            }
        }
    }
}
