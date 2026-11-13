use crate::core::Core;
use serde_json::{Value, json};

fn redact_line(l: &str) -> String {
    let mut out = String::with_capacity(l.len());
    for w in l.split(' ') {
        if !out.is_empty() {
            out.push(' ');
        }
        let lower = w.to_lowercase();
        if lower.starts_with("sk-") || lower.contains("bearer") || lower.contains("psk") || (w.len() > 40 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            out.push_str("[redacted]");
        } else {
            out.push_str(w);
        }
    }
    out
}

pub fn sections(core: &Core) -> Vec<(String, String)> {
    let mut v = Vec::new();
    v.push((
        "Versions".to_owned(),
        format!(
            "Setu {}\nModel manifest {}\nProtocol {}-{}\nBuild {}\nSpeech engine {}\nSystem {} {}\n",
            setu_common::VERSION,
            core.store.manifest.version,
            setu_common::PROTOCOL_MIN,
            setu_common::PROTOCOL_MAX,
            env!("SETU_BUILD"),
            setu_sherpa::version(),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    ));
    v.push(("Components".into(), serde_json::to_string_pretty(&core.daemon_state()).unwrap_or_default()));
    v.push(("Latency by stage".into(), serde_json::to_string_pretty(&json!({ "latency": core.metrics.latency(), "layers": crate::layers::latency(), "queues": core.metrics.queues() })).unwrap_or_default()));
    let mut s = core.settings.all_public();
    s.remove("files.folders");
    v.push(("Settings".into(), serde_json::to_string_pretty(&Value::Object(s)).unwrap_or_default()));
    let models: Vec<Value> = core.store.installed_list().iter().map(|m| json!({ "id": m.id, "bytes": m.bytes })).collect();
    v.push(("Downloaded models".into(), serde_json::to_string_pretty(&models).unwrap_or_default()));
    v.push(("Things that need attention".into(), serde_json::to_string_pretty(&core.attention.list()).unwrap_or_default()));
    let mut log = String::new();
    if let Ok(t) = std::fs::read_to_string(core.paths.logs.join("setu-daemon.log")) {
        let lines: Vec<&str> = t.lines().collect();
        for l in &lines[lines.len().saturating_sub(300)..] {
            log.push_str(&redact_line(l));
            log.push('\n');
        }
    }
    v.push(("Recent log lines (transcripts are never logged)".into(), log));
    v
}

pub fn preview(core: &Core) -> Value {
    json!({ "sections": sections(core).into_iter().map(|(n, c)| json!({ "name": n, "content": c })).collect::<Vec<_>>() })
}

pub fn export(core: &Core, path: &str) -> anyhow::Result<Value> {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        anyhow::bail!("Choose a full path for the file.");
    }
    let mut text = String::new();
    for (n, c) in sections(core) {
        text.push_str(&format!("== {n} ==\n{c}\n\n"));
    }
    std::fs::write(p, &text)?;
    Ok(json!({ "path": path, "bytes": text.len() }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn redacts_secrets() {
        assert_eq!(super::redact_line("key sk-abc123 ok"), "key [redacted] ok");
        assert!(super::redact_line(&format!("token {}", "a".repeat(50))).contains("[redacted]"));
    }
}
