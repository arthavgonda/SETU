use serde_json::{Value, json};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
use windows::core::{BOOL, BSTR};

const BUTTONS: &[&str] = &["install", "install now", "upgrade", "update", "next", "continue", "finish", "done", "close", "ok", "i agree", "i accept", "accept", "agree", "accept and install", "agree and install", "decline", "skip", "no thanks", "skip all", "don't install", "do not install"];
const FOLDER_WORDS: &[&str] = &["folder", "location", "directory", "destination", "path", "install to", "install in"];

pub fn clean(s: &str) -> String {
    let t: String = s.chars().filter(|c| !matches!(c, '&' | '_' | '>' | '<' | '…' | '.' | ':' | '!')).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn setupish(title: &str) -> bool {
    let t = title.to_lowercase();
    ["setup", "install", "wizard", "installer"].iter().any(|w| t.contains(w))
}

pub fn pathish(v: &str) -> bool {
    let b = v.as_bytes();
    v.len() > 3 && v.len() < 260 && b[1] == b':' && b[2] == b'\\' && b[0].is_ascii_alphabetic() && !v.contains(['"', '<', '>', '|', '*', '?', '\n'])
}

pub fn allowed(role: &str, name: &str, op: &str, value: &str) -> bool {
    let n = clean(name);
    match op {
        "invoke" => role == "button" && BUTTONS.contains(&n.as_str()),
        "toggle" => role == "checkbox",
        "select" => role == "radio",
        "set" => role == "field" && FOLDER_WORDS.iter().any(|w| n.contains(w)) && pathish(value),
        _ => false,
    }
}

pub fn request(args: &Value) -> Result<Value, String> {
    let pid = args.get("pid").and_then(Value::as_u64).filter(|p| *p > 4 && *p < u32::MAX as u64).ok_or("A setup window is needed.")?;
    let op = args.get("op").and_then(Value::as_str).unwrap_or("read");
    match op {
        "read" => Ok(json!({ "op": "setup.read", "args": { "pid": pid } })),
        "invoke" | "toggle" | "select" | "set" => {
            let i = args.get("index").and_then(Value::as_u64).ok_or("Which control?")?;
            let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
            let value = args.get("value").and_then(Value::as_str).unwrap_or_default();
            if name.len() > 200 || value.len() > 260 {
                return Err("That control is not one Setu drives.".into());
            }
            Ok(json!({ "op": "setup.act", "args": { "pid": pid, "index": i, "name": name, "do": op, "value": value } }))
        }
        _ => Err("That setup step is not one Setu drives.".into()),
    }
}

fn window_of(pid: u32) -> Option<(HWND, String)> {
    struct Find {
        pid: u32,
        hit: Option<(HWND, String)>,
    }
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let f = unsafe { &mut *(l.0 as *mut Find) };
        let mut p = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut p)) };
        if p == f.pid && unsafe { IsWindowVisible(h) }.as_bool() {
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
            let title = String::from_utf16_lossy(&buf[..n]);
            if setupish(&title) {
                f.hit = Some((h, title));
                return false.into();
            }
        }
        true.into()
    }
    let mut f = Find { pid, hit: None };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut f as *mut Find as isize));
    }
    f.hit
}

fn role_of(t: UIA_CONTROLTYPE_ID) -> Option<&'static str> {
    Some(match t {
        x if x == UIA_ButtonControlTypeId => "button",
        x if x == UIA_CheckBoxControlTypeId => "checkbox",
        x if x == UIA_RadioButtonControlTypeId => "radio",
        x if x == UIA_EditControlTypeId => "field",
        x if x == UIA_TextControlTypeId => "text",
        x if x == UIA_DocumentControlTypeId => "document",
        x if x == UIA_ProgressBarControlTypeId => "progress",
        _ => return None,
    })
}

fn gather(pid: u32) -> Result<(Vec<Value>, Vec<IUIAutomationElement>), String> {
    let (hwnd, _) = window_of(pid).ok_or("That window isn't a setup window, so Setu won't act on it.")?;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let root = uia.ElementFromHandle(hwnd).map_err(|e| e.to_string())?;
        let all = root.FindAll(TreeScope_Descendants, &uia.CreateTrueCondition().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut nodes = Vec::new();
        let mut els = Vec::new();
        for i in 0..all.Length().unwrap_or(0).min(800) {
            let Ok(el) = all.GetElement(i) else { continue };
            let Some(role) = el.CurrentControlType().ok().and_then(role_of) else { continue };
            let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let enabled = el.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(true);
            let checked = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).ok().and_then(|p| p.CurrentToggleState().ok()).map(|s| s == ToggleState_On)
                .or_else(|| el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).ok().and_then(|p| p.CurrentIsSelected().ok()).map(|b| b.as_bool()).filter(|_| role == "radio"));
            let value = if role == "progress" {
                el.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId)
                    .ok()
                    .and_then(|p| {
                        let (v, lo, hi) = (p.CurrentValue().ok()?, p.CurrentMinimum().unwrap_or(0.0), p.CurrentMaximum().ok()?);
                        (hi > lo).then(|| format!("{:.0}%", ((v - lo) / (hi - lo) * 100.0).clamp(0.0, 100.0)))
                    })
                    .unwrap_or_default()
            } else if matches!(role, "field" | "document") {
                el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|p| p.CurrentValue().ok()).map(|b| b.to_string().chars().take(4000).collect()).unwrap_or_default()
            } else {
                String::new()
            };
            let editable = role == "field" && el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|p| p.CurrentIsReadOnly().ok()).is_some_and(|r| !r.as_bool());
            nodes.push(json!({ "id": format!("setup:{pid}:{}", els.len()), "role": role, "name": name, "value": value, "enabled": enabled, "checked": checked, "editable": editable }));
            els.push(el);
        }
        Ok((nodes, els))
    }
}

fn act(args: &Value) -> Result<Value, String> {
    let pid = args.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32;
    let i = args.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX) as usize;
    let want = args.get("name").and_then(Value::as_str).unwrap_or_default();
    let op = args.get("do").and_then(Value::as_str).unwrap_or_default();
    let value = args.get("value").and_then(Value::as_str).unwrap_or_default();
    let (nodes, els) = gather(pid)?;
    let (Some(n), Some(el)) = (nodes.get(i), els.get(i)) else { return Err("That control is gone.".into()) };
    let role = n.get("role").and_then(Value::as_str).unwrap_or_default();
    let name = n.get("name").and_then(Value::as_str).unwrap_or_default();
    if name != want {
        return Err("The setup page changed.".into());
    }
    if !allowed(role, name, op, value) {
        return Err("That control is not one Setu drives.".into());
    }
    unsafe {
        match op {
            "invoke" => el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId).and_then(|p| p.Invoke()),
            "toggle" => el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId).and_then(|p| p.Toggle()),
            "select" => el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).and_then(|p| p.Select()),
            _ => el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).and_then(|p| p.SetValue(&BSTR::from(value))),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(json!({ "done": true }))
}

pub fn helper(op: &str, args: &Value) -> Result<Value, String> {
    match op {
        "setup.read" => {
            let pid = args.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32;
            let (title, nodes) = (window_of(pid).map(|(_, t)| t).unwrap_or_default(), gather(pid)?.0);
            Ok(json!({ "title": title, "nodes": nodes }))
        }
        "setup.act" => act(args),
        _ => Err("unknown setup step".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wizard_steps_are_allowed() {
        assert!(allowed("button", "&Next >", "invoke", ""));
        assert!(allowed("button", "I Agree", "invoke", ""));
        assert!(!allowed("button", "Yes", "invoke", ""));
        assert!(!allowed("button", "Run", "invoke", ""));
        assert!(allowed("checkbox", "Launch App", "toggle", ""));
        assert!(!allowed("button", "Next", "toggle", ""));
        assert!(allowed("field", "Destination folder", "set", r"D:\Games\App"));
        assert!(!allowed("field", "Destination folder", "set", "cmd /c del"));
        assert!(!allowed("field", "Command", "set", r"C:\x"));
        assert!(setupish("Setup - Bench App") && !setupish("Administrator: Command Prompt"));
    }
}
