use crate::hub::Hub;
use serde_json::{Value, json};
use setu_common::protocol::now_ms;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Issue {
    pub code: String,
    pub level: &'static str,
    pub message: String,
    pub actions: Vec<Value>,
    pub since: i64,
}

impl Issue {
    pub fn new(code: &str, level: &'static str, message: impl Into<String>) -> Self {
        Self { code: code.into(), level, message: message.into(), actions: Vec::new(), since: now_ms() }
    }

    pub fn action(mut self, label: &str, method: &str, params: Value) -> Self {
        self.actions.push(json!({ "label": label, "method": method, "params": params }));
        self
    }

    fn json(&self) -> Value {
        json!({ "code": self.code, "level": self.level, "message": self.message, "actions": self.actions, "since": self.since })
    }
}

#[derive(Default)]
pub struct Attention {
    issues: Mutex<Vec<Issue>>,
}

impl Attention {
    pub fn raise(&self, hub: &Hub, issue: Issue) {
        let mut g = self.issues.lock().unwrap();
        if let Some(existing) = g.iter_mut().find(|i| i.code == issue.code) {
            if existing.message == issue.message {
                return;
            }
            *existing = issue;
        } else {
            g.push(issue);
        }
        let list = g.iter().map(Issue::json).collect::<Vec<_>>();
        drop(g);
        hub.emit("attention.changed", json!({ "issues": list }));
    }

    pub fn clear(&self, hub: &Hub, code: &str) {
        let mut g = self.issues.lock().unwrap();
        let before = g.len();
        g.retain(|i| i.code != code);
        if g.len() == before {
            return;
        }
        let list = g.iter().map(Issue::json).collect::<Vec<_>>();
        drop(g);
        hub.emit("attention.changed", json!({ "issues": list }));
    }

    pub fn list(&self) -> Value {
        Value::Array(self.issues.lock().unwrap().iter().map(Issue::json).collect())
    }
}
