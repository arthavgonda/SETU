use crate::actions::{Host, plan};
use crate::exec::{Mode, execute};
use serde_json::{Value, json};

pub type Extra<'a> = &'a dyn Fn(&str, &Value, u32) -> Option<Result<Value, String>>;

pub fn handle(line: &str, owner: u32, host: &Host, mode: &Mode, secure: &dyn Fn(&Value) -> Result<Value, String>, extra: Extra) -> Value {
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return json!({ "id": null, "ok": false, "error": "That request isn't readable." }),
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let action = msg.get("action").and_then(Value::as_str).unwrap_or_default();
    let mut args = msg.get("args").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object() {
        args = json!({});
    }
    if action == "ping" {
        return json!({ "id": id, "ok": true, "result": null });
    }
    if action == "hello" {
        return json!({ "id": id, "ok": true, "result": { "version": env!("CARGO_PKG_VERSION"), "actions": crate::actions::ACTIONS, "face": cfg!(any(target_os = "linux", windows)), "journal": matches!(mode, Mode::Journal(_)) } });
    }
    if let Some(r) = extra(action, &args, owner) {
        return match r {
            Ok(v) => json!({ "id": id, "ok": true, "result": v }),
            Err(e) => json!({ "id": id, "ok": false, "error": e }),
        };
    }
    args["owner"] = json!(owner);
    let strict = matches!(mode, Mode::Real);
    match plan(action, &args, host, strict).and_then(|steps| execute(action, &steps, mode, secure)) {
        Ok(v) => json!({ "id": id, "ok": true, "result": v }),
        Err(e) => json!({ "id": id, "ok": false, "error": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_malformed_requests_fail_cleanly() {
        let t = tempfile::tempdir().unwrap();
        let mode = Mode::Journal(t.path().join("j"));
        let host = Host { pm: None, firewalld: false, backlights: vec![], stage_dir: t.path().into() };
        let none = |_: &Value| Ok(Value::Null);
        let no: Extra = &|_, _, _| None;
        assert_eq!(handle("not json", 1, &host, &mode, &none, no)["ok"], false);
        assert_eq!(handle(r#"{"id":3,"action":"shell.run","args":{"cmd":"id"}}"#, 1, &host, &mode, &none, no)["ok"], false);
        assert_eq!(handle(r#"{"id":1,"action":"hello"}"#, 1, &host, &mode, &none, no)["result"]["journal"], true);
        assert_eq!(handle(r#"{"id":2,"action":"ping"}"#, 1, &host, &mode, &none, no)["ok"], true);
        let extra: Extra = &|a, _, o| (a == "face.status").then(|| Ok(json!({ "owner": o })));
        assert_eq!(handle(r#"{"id":5,"action":"face.status"}"#, 9, &host, &mode, &none, extra)["result"]["owner"], 9);
        let owner = handle(r#"{"id":4,"action":"package.install_file","args":{"path":"/x.rpm","owner":0}}"#, 7, &host, &mode, &none, no);
        assert_eq!(owner["ok"], false);
    }
}
