use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const KEEP: usize = 200;

pub const STAGES: &[&str] = &["final_transcript", "rules_router", "model_router", "action_start", "first_audio", "total", "wake_to_event"];

#[derive(Default)]
pub struct Metrics {
    stages: Mutex<HashMap<&'static str, VecDeque<u64>>>,
    queues: Mutex<HashMap<&'static str, (usize, usize)>>,
    pub dropped_audio: AtomicU64,
}

impl Metrics {
    pub fn record(&self, stage: &'static str, ms: u64) {
        let mut g = self.stages.lock().unwrap();
        let q = g.entry(stage).or_default();
        if q.len() == KEEP {
            q.pop_front();
        }
        q.push_back(ms);
    }

    pub fn queue(&self, name: &'static str, depth: usize, capacity: usize) {
        self.queues.lock().unwrap().insert(name, (depth, capacity));
    }

    pub fn latency(&self) -> Value {
        let g = self.stages.lock().unwrap();
        let mut m = Map::new();
        for s in STAGES {
            let Some(q) = g.get(s) else { continue };
            if q.is_empty() {
                continue;
            }
            let mut v: Vec<u64> = q.iter().copied().collect();
            v.sort_unstable();
            let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
            m.insert((*s).into(), json!({ "lastMs": q.back(), "p50Ms": pct(0.5), "p95Ms": pct(0.95), "count": v.len() }));
        }
        Value::Object(m)
    }

    pub fn queues(&self) -> Value {
        let g = self.queues.lock().unwrap();
        let mut m = Map::new();
        for (k, (d, c)) in g.iter() {
            m.insert((*k).into(), json!({ "depth": d, "capacity": c, "dropped": if *k == "audio" { self.dropped_audio.load(Ordering::Relaxed) } else { 0 } }));
        }
        Value::Object(m)
    }
}

pub fn rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/statm")
            && let Some(p) = s.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()) {
                return p * 4096;
            }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut sys = sysinfo::System::new();
        let pid = sysinfo::Pid::from_u32(std::process::id());
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        sys.process(pid).map(|p| p.memory()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let m = Metrics::default();
        for i in 1..=100 {
            m.record("total", i);
        }
        let l = m.latency();
        assert_eq!(l["total"]["p50Ms"], 51);
        assert_eq!(l["total"]["p95Ms"], 95);
        assert_eq!(l["total"]["lastMs"], 100);
    }
}
