use crate::core::Core;
use crate::router::Intent;
use crate::tools::Outcome;
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const LAYER: &str = "protocol";

fn info(dir: &Path) -> Option<(u16, String)> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".setu-autoconfig")).ok()?).ok()?;
    Some((v.get("port")?.as_u64()? as u16, v.get("token")?.as_str()?.to_owned()))
}

pub async fn call(dir: &Path, op: &str, args: Value) -> Result<Value> {
    let (port, token) = info(dir).ok_or_else(|| anyhow!("Setu's Firefox script isn't running. Open Firefox on your Setu profile, or say set up Firefox scripting."))?;
    let span = crate::layers::effect(crate::layers::Layer::Protocol, "protocol.autoconfig");
    let r = async {
        let mut s = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(("127.0.0.1", port))).await.map_err(|_| anyhow!("Firefox didn't answer."))??;
        let msg = json!({ "id": 1, "token": token, "op": op, "args": args }).to_string() + "\n";
        s.write_all(msg.as_bytes()).await?;
        let mut lines = BufReader::new(s).lines();
        let line = tokio::time::timeout(Duration::from_secs(20), lines.next_line()).await.map_err(|_| anyhow!("Firefox didn't answer in time."))??.ok_or_else(|| anyhow!("Firefox closed the connection."))?;
        let v: Value = serde_json::from_str(&line)?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(anyhow!("{}", v.get("error").and_then(Value::as_str).unwrap_or("Firefox refused that.")))
        }
    }
    .await;
    span.end(r)
}

fn act(op: &str, args: Value) -> Option<Intent> {
    let mut a = args;
    a["op"] = json!(op);
    Some(Intent::Act { tool: "browser.fox".into(), args: a })
}

pub fn rules(t: &str) -> Option<Intent> {
    let named = t.contains("firefox");
    let t = t.trim_end_matches(" in firefox").trim_end_matches(" from firefox").trim_end_matches(" on firefox");
    if matches!(t, "what add ons do i have" | "what addons do i have" | "list my firefox add ons" | "list my firefox addons" | "show my firefox extensions" | "which extensions are in firefox" | "what extensions do i have" | "list my firefox extensions" | "what firefox add ons do i have") {
        return act("addons.list", json!({}));
    }
    if matches!(t, "set up firefox scripting" | "install the firefox script" | "install setu's firefox script" | "set up the firefox script") {
        return Some(Intent::Act { tool: "browser.foxsetup".into(), args: json!({}) });
    }
    if matches!(t, "open the firefox console" | "open the web console" | "open firefox devtools" | "open devtools" | "open dev tools" | "open the dev tools" | "open the devtools" | "open developer tools" | "open the developer tools" | "open firefox dev tools" | "show the developer tools" | "show devtools") {
        return act("devtools.open", json!({ "tool": "webconsole" }));
    }
    if matches!(t, "open the firefox inspector" | "open the inspector") {
        return act("devtools.open", json!({ "tool": "inspector" }));
    }
    for (p, op) in [("turn off the ", "addons.disable"), ("turn off ", "addons.disable"), ("disable the ", "addons.disable"), ("disable ", "addons.disable"), ("turn on the ", "addons.enable"), ("turn on ", "addons.enable"), ("enable the ", "addons.enable"), ("enable ", "addons.enable"), ("use the ", "addons.enable"), ("remove the ", "addons.remove"), ("uninstall the ", "addons.remove")] {
        if let Some(r) = t.strip_prefix(p) {
            for sfx in [" add on", " addon", " extension", " theme"] {
                if let Some(name) = r.strip_suffix(sfx)
                    && (named || sfx == " add on" || sfx == " addon")
                {
                    let theme = sfx == " theme";
                    return act(op, json!({ "name": name.trim(), "theme": theme }));
                }
            }
        }
    }
    None
}

pub fn firefox_dir(exe: &Path) -> Option<PathBuf> {
    let real = exe.canonicalize().ok()?;
    let dir = real.parent()?;
    if dir.join("application.ini").exists() {
        return Some(dir.to_path_buf());
    }
    let lib = PathBuf::from("/usr/lib64/firefox");
    lib.join("application.ini").exists().then_some(lib)
}

async fn find(dir: &Path, name: &str, theme: bool) -> Result<(String, String)> {
    let list = call(dir, "addons.list", json!({})).await?;
    let want = crate::text::normalize(name);
    let items: Vec<(String, String, String)> = list.as_array().cloned().unwrap_or_default().iter().filter_map(|a| Some((a.get("id")?.as_str()?.to_owned(), a.get("name")?.as_str()?.to_owned(), a.get("type")?.as_str()?.to_owned()))).collect();
    items
        .iter()
        .filter(|(_, _, ty)| !theme || ty == "theme")
        .map(|(id, n, _)| (crate::os::score(&want, &crate::text::normalize(n)).max(if crate::text::normalize(n).contains(&want) { 0.9 } else { 0.0 }), id, n))
        .filter(|(s, _, _)| *s >= 0.7)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, id, n)| (id.clone(), n.clone()))
        .ok_or_else(|| anyhow!("Firefox has no add-on called {name}."))
}

pub async fn run(core: &Arc<Core>, tool: &str, args: &Value) -> Outcome {
    let dir = crate::cdp::profile_dir(&core.paths.data, "firefox");
    if tool == "browser.foxsetup" {
        let Some(app) = crate::tools::web::find_browser(core, Some("firefox")) else { return Outcome::fail("Firefox isn't installed.", LAYER) };
        let exe = crate::cdp::exe_of(&app.exec, &app.path);
        let Some(install) = firefox_dir(&exe) else { return Outcome::fail("I couldn't find Firefox's install folder. Flatpak and Snap Firefox can't take the script; BiDi still works.", LAYER) };
        return match crate::privileged::call("firefox.autoconfig.install", json!({ "dir": install }), Duration::from_secs(60)).await {
            Ok(_) => Outcome::ok_with("Installed the Firefox script", LAYER, None).with_reply("Done. Setu's script is in Firefox and works only in your Setu profile. Restart Firefox to start it.".into()),
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        };
    }
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let theme = args.get("theme").and_then(Value::as_bool).unwrap_or(false);
    match s("op").as_str() {
        "addons.list" => match call(&dir, "addons.list", json!({})).await {
            Ok(list) => {
                let all = list.as_array().cloned().unwrap_or_default();
                let ext: Vec<String> = all.iter().filter(|a| a.get("type").and_then(Value::as_str) == Some("extension") && a.get("builtin").and_then(Value::as_bool) != Some(true)).filter_map(|a| a.get("name").and_then(Value::as_str).map(|n| if a.get("enabled").and_then(Value::as_bool) == Some(true) { n.to_owned() } else { format!("{n} (off)") })).collect();
                let themes: Vec<String> = all.iter().filter(|a| a.get("type").and_then(Value::as_str) == Some("theme")).filter_map(|a| a.get("name").and_then(Value::as_str).map(|n| if a.get("enabled").and_then(Value::as_bool) == Some(true) { format!("{n} (in use)") } else { n.to_owned() })).collect();
                let e = if ext.is_empty() { "No extensions are installed.".to_owned() } else { format!("Extensions: {}.", ext.join(", ")) };
                Outcome::ok_with("Listed Firefox add-ons", LAYER, None).with_reply(format!("{e} Themes: {}.", themes.join(", ")))
            }
            Err(e) => Outcome::fail(&e.to_string(), LAYER),
        },
        op @ ("addons.enable" | "addons.disable" | "addons.remove") => {
            let (id, name) = match find(&dir, &s("name"), theme).await {
                Ok(x) => x,
                Err(e) => return Outcome::fail(&e.to_string(), LAYER),
            };
            match call(&dir, op, json!({ "id": id })).await {
                Ok(_) => Outcome::ok_with(&format!("{name} {}", match op { "addons.enable" => "on", "addons.disable" => "off", _ => "removed" }), LAYER, None).with_reply(match op {
                    "addons.enable" if theme => format!("Firefox now uses the {name} theme."),
                    "addons.enable" => format!("{name} is on in Firefox."),
                    "addons.disable" => format!("{name} is off in Firefox."),
                    _ => format!("Removed {name} from Firefox."),
                }),
                Err(e) => Outcome::fail(&e.to_string(), LAYER),
            }
        }
        "devtools.open" => {
            let firefox_front = crate::os::front().await.is_ok_and(|w| w.app.to_lowercase().contains("firefox"));
            if (firefox_front || crate::proto::any().is_some_and(|c| matches!(c, crate::proto::Conn::Bidi(_)))) && call(&dir, "devtools.open", json!({ "tool": s("tool") })).await.is_ok() {
                return Outcome::ok_with("Opened DevTools", LAYER, None);
            }
            let keys = if s("tool") == "inspector" { vec![crate::tools::input::K::Ctrl, crate::tools::input::K::Shift, crate::tools::input::K::Ch('c')] } else { vec![crate::tools::input::K::F(12)] };
            match crate::tools::input::press(&keys).await {
                Ok(()) => Outcome::ok_with("Opened DevTools", "keyboard", None),
                Err(e) => Outcome::fail(&e.to_string(), "keyboard"),
            }
        }
        _ => Outcome::fail("I don't know that Firefox command.", LAYER),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firefox_phrases() {
        let op = |t: &str| match rules(t) {
            Some(Intent::Act { args, .. }) => args.get("op").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        assert_eq!(op("what add ons do i have in firefox").as_deref(), Some("addons.list"));
        assert_eq!(op("turn off the ublock origin extension in firefox").as_deref(), Some("addons.disable"));
        assert_eq!(op("turn off the ublock origin add on").as_deref(), Some("addons.disable"));
        assert!(op("turn off the dash to dock extension").is_none());
        assert!(op("turn off the dark theme").is_none());
        assert_eq!(op("use the dark theme in firefox").as_deref(), Some("addons.enable"));
        assert_eq!(op("open the firefox console").as_deref(), Some("devtools.open"));
        assert!(rules("turn off the lights").is_none());
    }
}
