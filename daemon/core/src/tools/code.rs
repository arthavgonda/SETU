use super::shell::{find_program, output};
use crate::text::{normalize, similarity};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn gui(args: &[&str]) -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(target_os = "linux") {
        v.push(crate::tools::apps::A11Y_FLAG.to_owned());
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
            v.push("--ozone-platform=wayland".to_owned());
        }
    }
    v.extend(args.iter().map(|a| (*a).to_owned()));
    v
}

fn run_gui(args: &[&str], secs: u64) -> Result<String> {
    let all = gui(args);
    let refs: Vec<&str> = all.iter().map(String::as_str).collect();
    run_cli(&refs, secs)
}

pub fn open_new_window(dir: &Path, file: Option<&Path>) -> Result<()> {
    let d = dir.to_string_lossy().into_owned();
    match file {
        Some(f) => run_gui(&["-n", &d, &f.to_string_lossy()], 20).map(|_| ()),
        None => run_gui(&["-n", &d], 20).map(|_| ()),
    }
}

pub fn reuse(args: &[&str]) -> Result<()> {
    let mut a = vec!["--reuse-window"];
    a.extend_from_slice(args);
    run_gui(&a, 20).map(|_| ())
}

pub fn starter(language: &str) -> Option<(&'static str, &'static str)> {
    let l = language.to_lowercase();
    Some(match l.trim() {
        "python" | "py" => ("main.py", ""),
        "javascript" | "js" | "node" | "node js" | "nodejs" => ("index.js", ""),
        "typescript" | "ts" => ("index.ts", ""),
        "c" => ("main.c", "#include <stdio.h>\n\nint main(void) {\n    return 0;\n}\n"),
        "c plus plus" | "c++" | "cpp" | "c p p" => ("main.cpp", "#include <iostream>\n\nint main() {\n    return 0;\n}\n"),
        "java" => ("Main.java", "public class Main {\n    public static void main(String[] args) {\n    }\n}\n"),
        "go" | "golang" => ("main.go", "package main\n\nfunc main() {\n}\n"),
        "rust" => ("main.rs", "fn main() {\n}\n"),
        "html" | "web" | "website" => ("index.html", "<!DOCTYPE html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title></title>\n</head>\n<body>\n</body>\n</html>\n"),
        "ruby" => ("main.rb", ""),
        "php" => ("index.php", "<?php\n"),
        "kotlin" => ("Main.kt", "fun main() {\n}\n"),
        "swift" => ("main.swift", ""),
        "c sharp" | "c#" | "csharp" => ("Program.cs", ""),
        "bash" | "shell" => ("main.sh", "#!/usr/bin/env bash\n"),
        _ => return None,
    })
}

pub fn cli() -> Option<PathBuf> {
    let mut extra = Vec::new();
    if cfg!(windows) {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            extra.push(PathBuf::from(l).join("Programs").join("Microsoft VS Code").join("bin"));
        }
        for v in ["ProgramFiles", "ProgramW6432"] {
            if let Some(p) = std::env::var_os(v) {
                extra.push(PathBuf::from(p).join("Microsoft VS Code").join("bin"));
            }
        }
        return find_program(&["code.cmd"], &extra);
    }
    if cfg!(target_os = "macos") {
        extra.push(PathBuf::from("/Applications/Visual Studio Code.app/Contents/Resources/app/bin"));
        if let Some(h) = dirs::home_dir() {
            extra.push(h.join("Applications/Visual Studio Code.app/Contents/Resources/app/bin"));
        }
    }
    find_program(&["code", "codium", "code-oss"], &extra)
}

fn flatpak_code() -> bool {
    cfg!(target_os = "linux") && Path::new("/var/lib/flatpak/app/com.visualstudio.code").exists() || dirs::data_dir().is_some_and(|d| d.join("flatpak/app/com.visualstudio.code").exists())
}

pub fn run_cli(args: &[&str], secs: u64) -> Result<String> {
    if let Some(c) = cli() {
        let span = crate::layers::effect(crate::layers::Layer::Channel, "channel.cli");
        let (ok, out, err) = super::shell::output_quiet(&c.to_string_lossy(), args, None, Duration::from_secs(secs))?;
        span.end_bool(ok);
        return if ok { Ok(out) } else { Err(anyhow!("{}", err.lines().last().unwrap_or("VS Code reported a problem.").trim())) };
    }
    if flatpak_code() {
        let mut full = vec!["run", "com.visualstudio.code"];
        full.extend_from_slice(args);
        let span = crate::layers::effect(crate::layers::Layer::Channel, "channel.cli");
        let (ok, out, err) = super::shell::output_quiet("flatpak", &full, None, Duration::from_secs(secs))?;
        span.end_bool(ok);
        return if ok { Ok(out) } else { Err(anyhow!("{}", err.lines().last().unwrap_or("VS Code reported a problem.").trim())) };
    }
    Err(anyhow!("VS Code isn't installed. Say install VS Code."))
}

pub fn installed() -> bool {
    cli().is_some() || flatpak_code()
}

pub fn list_extensions() -> Vec<String> {
    run_cli(&["--list-extensions"], 30).map(|o| o.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

pub fn install_extension(id: &str) -> Result<()> {
    run_cli(&["--install-extension", id], 180).map(|_| ())
}

pub fn uninstall_extension(id: &str) -> Result<()> {
    run_cli(&["--uninstall-extension", id], 120).map(|_| ())
}

pub fn open_path(path: &Path) -> Result<()> {
    run_gui(&["--reuse-window", &path.to_string_lossy()], 20).map(|_| ())
}

pub const KNOWN: &[(&[&str], &str, &str)] = &[
    (&["python"], "ms-python.python", "Python"),
    (&["pylance"], "ms-python.vscode-pylance", "Pylance"),
    (&["c plus plus", "c++", "cpp", "c", "c and c plus plus", "c c++", "c cpp"], "ms-vscode.cpptools", "C/C++"),
    (&["c++ extension pack", "c plus plus extension pack"], "ms-vscode.cpptools-extension-pack", "C/C++ Extension Pack"),
    (&["code runner", "coderunner"], "formulahendry.code-runner", "Code Runner"),
    (&["prettier"], "esbenp.prettier-vscode", "Prettier"),
    (&["eslint", "es lint"], "dbaeumer.vscode-eslint", "ESLint"),
    (&["live server", "liveserver"], "ritwickdey.liveserver", "Live Server"),
    (&["gitlens", "git lens"], "eamodio.gitlens", "GitLens"),
    (&["java", "java extension pack", "java pack"], "vscjava.vscode-java-pack", "Java Extension Pack"),
    (&["go", "golang"], "golang.go", "Go"),
    (&["rust", "rust analyzer", "rust analyser"], "rust-lang.rust-analyzer", "rust-analyzer"),
    (&["docker"], "ms-azuretools.vscode-docker", "Docker"),
    (&["jupyter", "jupyter notebooks"], "ms-toolsai.jupyter", "Jupyter"),
    (&["copilot", "github copilot"], "github.copilot", "GitHub Copilot"),
    (&["copilot chat"], "github.copilot-chat", "GitHub Copilot Chat"),
    (&["material icon theme", "material icons"], "pkief.material-icon-theme", "Material Icon Theme"),
    (&["one dark pro", "one dark"], "zhuangtongfa.material-theme", "One Dark Pro"),
    (&["dracula", "dracula theme"], "dracula-theme.theme-dracula", "Dracula"),
    (&["tailwind", "tailwind css"], "bradlc.vscode-tailwindcss", "Tailwind CSS IntelliSense"),
    (&["vim"], "vscodevim.vim", "Vim"),
    (&["c sharp", "c#", "csharp"], "ms-dotnettools.csharp", "C#"),
    (&["php"], "bmewburn.vscode-intelephense-client", "PHP Intelephense"),
    (&["flutter"], "dart-code.flutter", "Flutter"),
    (&["dart"], "dart-code.dart-code", "Dart"),
    (&["kotlin"], "fwcd.kotlin", "Kotlin"),
    (&["remote ssh", "ssh"], "ms-vscode-remote.remote-ssh", "Remote - SSH"),
    (&["wsl"], "ms-vscode-remote.remote-wsl", "WSL"),
    (&["dev containers", "devcontainers"], "ms-vscode-remote.remote-containers", "Dev Containers"),
    (&["markdown", "markdown all in one"], "yzhang.markdown-all-in-one", "Markdown All in One"),
    (&["error lens", "errorlens"], "usernamehw.errorlens", "Error Lens"),
    (&["path intellisense"], "christian-kohler.path-intellisense", "Path Intellisense"),
    (&["auto rename tag"], "formulahendry.auto-rename-tag", "Auto Rename Tag"),
    (&["cmake", "cmake tools"], "ms-vscode.cmake-tools", "CMake Tools"),
    (&["html css support", "html css"], "ecmel.vscode-html-css", "HTML CSS Support"),
    (&["thunder client"], "rangav.vscode-thunder-client", "Thunder Client"),
    (&["rest client"], "humao.rest-client", "REST Client"),
    (&["yaml"], "redhat.vscode-yaml", "YAML"),
    (&["xml"], "redhat.vscode-xml", "XML"),
    (&["sql", "sql tools", "sqltools"], "mtxr.sqltools", "SQLTools"),
    (&["hindi language pack", "hindi"], "ms-ceintl.vscode-language-pack-hi", "Hindi Language Pack"),
];

pub fn known_extension(spoken: &str) -> Option<(String, String)> {
    let t = normalize(spoken);
    let t = t.trim_start_matches("the ").trim_end_matches(" extension").trim_end_matches(" plugin").trim_end_matches(" extensions").trim();
    KNOWN.iter().find(|(w, _, _)| w.contains(&t)).map(|(_, id, name)| (id.to_string(), name.to_string()))
}

pub fn marketplace(query: &str) -> Option<(String, String, String)> {
    let body = json!({
        "filters": [{ "criteria": [{ "filterType": 8, "value": "Microsoft.VisualStudio.Code" }, { "filterType": 10, "value": query }], "pageNumber": 1, "pageSize": 8, "sortBy": 0, "sortOrder": 0 }],
        "assetTypes": [],
        "flags": 914
    });
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(4))).timeout_global(Some(Duration::from_secs(8))).build().into();
    let mut resp = agent
        .post("https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json;api-version=3.0-preview.1")
        .send(body.to_string())
        .ok()?;
    let v: Value = serde_json::from_str(&resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().ok()?).ok()?;
    let list = v.pointer("/results/0/extensions")?.as_array()?;
    let mut best: Option<(f64, String, String, String)> = None;
    for (rank, e) in list.iter().enumerate() {
        let publisher = e.pointer("/publisher/publisherName").and_then(Value::as_str).unwrap_or_default();
        let pub_name = e.pointer("/publisher/displayName").and_then(Value::as_str).unwrap_or(publisher);
        let name = e.get("extensionName").and_then(Value::as_str).unwrap_or_default();
        let display = e.get("displayName").and_then(Value::as_str).unwrap_or(name);
        if publisher.is_empty() || name.is_empty() {
            continue;
        }
        let installs = e
            .get("statistics")
            .and_then(Value::as_array)
            .and_then(|s| s.iter().find(|x| x.get("statisticName").and_then(Value::as_str) == Some("install")))
            .and_then(|x| x.get("value").and_then(Value::as_f64))
            .unwrap_or(0.0);
        let sim = similarity(query, display).max(similarity(query, name));
        let score = sim * 2.0 + (installs.max(1.0).log10() / 8.0) - rank as f64 * 0.15;
        if best.as_ref().is_none_or(|(b, ..)| score > *b) {
            best = Some((score, format!("{publisher}.{name}"), display.to_owned(), pub_name.to_owned()));
        }
    }
    best.map(|(_, id, name, publisher)| (id, name, publisher))
}

pub fn setting_for(spoken: &str) -> Option<(String, Value, String)> {
    let t = normalize(spoken);
    let on = !(t.contains(" off") || t.starts_with("turn off") || t.starts_with("disable") || t.starts_with("hide") || t.starts_with("stop") || t.contains("no "));
    let n = crate::text::find_number(&t);
    let s = |k: &str, v: Value, d: &str| Some((k.to_owned(), v, d.to_owned()));
    if t.contains("auto save") || t.contains("autosave") {
        return s("files.autoSave", json!(if on { "afterDelay" } else { "off" }), if on { "Auto save is on" } else { "Auto save is off" });
    }
    if t.contains("word wrap") || t.contains("wrap lines") || t.contains("line wrap") {
        return s("editor.wordWrap", json!(if on { "on" } else { "off" }), if on { "Word wrap is on" } else { "Word wrap is off" });
    }
    if t.contains("format on save") {
        return s("editor.formatOnSave", json!(on), if on { "Format on save is on" } else { "Format on save is off" });
    }
    if t.contains("minimap") || t.contains("mini map") {
        return s("editor.minimap.enabled", json!(on), if on { "The minimap is on" } else { "The minimap is off" });
    }
    if t.contains("line number") {
        return s("editor.lineNumbers", json!(if on { "on" } else { "off" }), if on { "Line numbers are on" } else { "Line numbers are off" });
    }
    if t.contains("sticky scroll") {
        return s("editor.stickyScroll.enabled", json!(on), if on { "Sticky scroll is on" } else { "Sticky scroll is off" });
    }
    if t.contains("bracket") && t.contains("colo") {
        return s("editor.bracketPairColorization.enabled", json!(on), if on { "Bracket colours are on" } else { "Bracket colours are off" });
    }
    if t.contains("terminal") && t.contains("font size") {
        let v = n?.clamp(6, 40);
        return s("terminal.integrated.fontSize", json!(v), &format!("Terminal font size is {v}"));
    }
    if t.contains("font size") || t.contains("text size") {
        let v = n?.clamp(6, 60);
        return s("editor.fontSize", json!(v), &format!("Font size is {v}"));
    }
    if t.contains("tab size") || t.contains("indent size") || t.contains("indentation size") {
        let v = n?.clamp(1, 16);
        return s("editor.tabSize", json!(v), &format!("Tab size is {v}"));
    }
    if t.contains("theme") || t.contains("dark mode") || t.contains("light mode") {
        if t.contains("high contrast") {
            return s("workbench.colorTheme", json!("Default High Contrast"), "High contrast theme is on");
        }
        if t.contains("dark") {
            return s("workbench.colorTheme", json!("Default Dark Modern"), "Dark theme is on");
        }
        if t.contains("light") {
            return s("workbench.colorTheme", json!("Default Light Modern"), "Light theme is on");
        }
    }
    if t.contains("cursor") && t.contains("blink") {
        return s("editor.cursorBlinking", json!(if on { "blink" } else { "solid" }), if on { "The cursor blinks" } else { "The cursor stays still" });
    }
    if t.contains("render whitespace") || t.contains("show whitespace") || t.contains("show spaces") {
        return s("editor.renderWhitespace", json!(if on { "all" } else { "selection" }), if on { "Whitespace is shown" } else { "Whitespace is hidden" });
    }
    None
}

pub fn user_settings() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        dirs::config_dir()?
    } else if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library/Application Support")
    } else {
        let flat = dirs::home_dir()?.join(".var/app/com.visualstudio.code/config");
        if flat.join("Code/User").is_dir() && cli().is_none() { flat } else { dirs::config_dir()? }
    };
    Some(base.join("Code").join("User").join("settings.json"))
}

fn skip_space(c: &[char], mut i: usize) -> usize {
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if i + 1 < c.len() && c[i] == '/' && c[i + 1] == '/' {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < c.len() && c[i] == '/' && c[i + 1] == '*' {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(c.len());
            continue;
        }
        return i;
    }
}

fn string_end(c: &[char], mut i: usize) -> usize {
    i += 1;
    while i < c.len() {
        match c[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    c.len()
}

fn value_end(c: &[char], mut i: usize) -> usize {
    let mut depth = 0i32;
    while i < c.len() {
        match c[i] {
            '"' => {
                i = string_end(c, i);
                continue;
            }
            '/' if i + 1 < c.len() && (c[i + 1] == '/' || c[i + 1] == '*') => {
                if depth == 0 {
                    return i;
                }
                i = skip_space(c, i);
                continue;
            }
            '{' | '[' => depth += 1,
            '}' | ']' if depth == 0 => return i,
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    i
}

pub fn set_jsonc(text: &str, key: &str, value: &Value) -> Option<String> {
    let c: Vec<char> = text.chars().collect();
    let rendered = value.to_string();
    let mut i = skip_space(&c, 0);
    if i >= c.len() {
        return Some(format!("{{\n    {}: {rendered}\n}}\n", json!(key)));
    }
    if c[i] != '{' {
        return None;
    }
    let open = i;
    i += 1;
    let mut members = 0;
    loop {
        i = skip_space(&c, i);
        if i >= c.len() {
            return None;
        }
        if c[i] == '}' {
            break;
        }
        if c[i] == ',' {
            i += 1;
            continue;
        }
        if c[i] != '"' {
            return None;
        }
        let end = string_end(&c, i);
        let name: String = c[i + 1..end.saturating_sub(1).max(i + 1)].iter().collect();
        i = skip_space(&c, end);
        if i >= c.len() || c[i] != ':' {
            return None;
        }
        let start = skip_space(&c, i + 1);
        let stop = value_end(&c, start);
        members += 1;
        if name == key {
            let mut before: String = c[..start].iter().collect();
            let mut tail_start = stop;
            while tail_start > start && c[tail_start - 1].is_whitespace() {
                tail_start -= 1;
            }
            before.push_str(&rendered);
            before.extend(c[tail_start..].iter());
            return Some(before);
        }
        i = stop;
    }
    let head: String = c[..=open].iter().collect();
    let rest: String = c[open + 1..].iter().collect();
    Some(format!("{head}\n    {}: {rendered}{}{rest}", json!(key), if members > 0 { "," } else { "\n" }))
}

pub fn write_setting(key: &str, value: &Value) -> Result<()> {
    let path = user_settings().ok_or_else(|| anyhow!("VS Code settings were not found."))?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let next = match serde_json::from_str::<Value>(text.trim()) {
        Ok(Value::Object(mut o)) => {
            o.insert(key.to_owned(), value.clone());
            serde_json::to_string_pretty(&Value::Object(o))? + "\n"
        }
        _ => set_jsonc(&text, key, value).ok_or_else(|| anyhow!("I couldn't read your VS Code settings file. Fix it in VS Code, then ask me again."))?,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.setu-tmp");
    std::fs::write(&tmp, next)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    C,
    Cpp,
    Python,
    JavaScript,
    TypeScript,
    Java,
    Go,
    Rust,
    Other,
}

pub fn lang(path: &Path, id: &str) -> Lang {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match (id, ext.as_str()) {
        ("c", _) | (_, "c") => Lang::C,
        ("cpp", _) | (_, "cpp" | "cc" | "cxx" | "hpp" | "hh" | "h") => Lang::Cpp,
        ("python", _) | (_, "py" | "pyw") => Lang::Python,
        ("javascript", _) | (_, "js" | "mjs" | "cjs") => Lang::JavaScript,
        ("typescript", _) | (_, "ts" | "mts") => Lang::TypeScript,
        ("java", _) | (_, "java") => Lang::Java,
        ("go", _) | (_, "go") => Lang::Go,
        ("rust", _) | (_, "rs") => Lang::Rust,
        _ => Lang::Other,
    }
}

pub fn lang_name(l: Lang) -> &'static str {
    match l {
        Lang::C => "C",
        Lang::Cpp => "C++",
        Lang::Python => "Python",
        Lang::JavaScript => "JavaScript",
        Lang::TypeScript => "TypeScript",
        Lang::Java => "Java",
        Lang::Go => "Go",
        Lang::Rust => "Rust",
        Lang::Other => "this language",
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub line: u32,
    pub col: u32,
    pub message: String,
}

fn python() -> Option<PathBuf> {
    if cfg!(windows) { find_program(&["python.exe", "py.exe"], &[]) } else { find_program(&["python3", "python"], &[]) }
}

pub fn compiler_for(l: Lang) -> Option<PathBuf> {
    let mingw: Vec<PathBuf> = if cfg!(windows) {
        ["C:\\msys64\\ucrt64\\bin", "C:\\msys64\\mingw64\\bin", "C:\\MinGW\\bin", "C:\\mingw64\\bin", "C:\\TDM-GCC-64\\bin"].iter().map(PathBuf::from).collect()
    } else {
        Vec::new()
    };
    match l {
        Lang::C => find_program(if cfg!(windows) { &["gcc.exe", "clang.exe"] } else { &["gcc", "clang", "cc"] }, &mingw),
        Lang::Cpp => find_program(if cfg!(windows) { &["g++.exe", "clang++.exe"] } else { &["g++", "clang++", "c++"] }, &mingw),
        Lang::Python => python(),
        Lang::JavaScript => find_program(if cfg!(windows) { &["node.exe"] } else { &["node"] }, &[]),
        Lang::Java => find_program(if cfg!(windows) { &["javac.exe"] } else { &["javac"] }, &[]),
        Lang::Go => find_program(if cfg!(windows) { &["gofmt.exe"] } else { &["gofmt"] }, &[]),
        Lang::Rust => find_program(if cfg!(windows) { &["cargo.exe", "rustc.exe"] } else { &["cargo", "rustc"] }, &dirs::home_dir().map(|h| vec![h.join(".cargo").join("bin")]).unwrap_or_default()),
        _ => None,
    }
}

fn parse_gcc(out: &str) -> Vec<Problem> {
    let mut v = Vec::new();
    for line in out.lines() {
        let Some(idx) = line.find(": error: ").or_else(|| line.find(": fatal error: ")) else { continue };
        let head = &line[..idx];
        let msg = line[idx..].trim_start_matches(": fatal error: ").trim_start_matches(": error: ").to_owned();
        let mut parts = head.rsplitn(3, ':');
        let col = parts.next().and_then(|c| c.trim().parse::<u32>().ok());
        let ln = parts.next().and_then(|c| c.trim().parse::<u32>().ok());
        match (ln, col) {
            (Some(l), Some(c)) => v.push(Problem { line: l, col: c, message: msg }),
            _ => {
                if let Some(l) = head.rsplit(':').next().and_then(|c| c.trim().parse::<u32>().ok()) {
                    v.push(Problem { line: l, col: 1, message: msg });
                }
            }
        }
    }
    v
}

fn parse_python(out: &str) -> Vec<Problem> {
    let mut line = None;
    let mut col = 1;
    let mut msg = None;
    let lines: Vec<&str> = out.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        if let Some(p) = l.find(", line ") {
            line = l[p + 7..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().ok();
            if let Some(caret) = lines.get(i + 2).filter(|c| c.trim() == "^" || c.trim().starts_with('^')) {
                col = caret.find('^').map(|c| c.saturating_sub(3) as u32 + 1).unwrap_or(1);
            }
        }
        if l.contains("Error:") && (l.starts_with("SyntaxError") || l.starts_with("IndentationError") || l.starts_with("TabError") || l.contains("Error: ")) {
            msg = Some(l.trim().to_owned());
        }
    }
    match (line, msg) {
        (Some(l), Some(m)) => vec![Problem { line: l, col, message: m }],
        _ => Vec::new(),
    }
}

fn parse_node(out: &str) -> Vec<Problem> {
    let first = out.lines().next().unwrap_or("");
    let line = first.rsplit(':').next().and_then(|n| n.trim().parse::<u32>().ok());
    let msg = out.lines().find(|l| l.contains("Error:")).map(|l| l.trim().to_owned());
    match (line, msg) {
        (Some(l), Some(m)) => vec![Problem { line: l, col: 1, message: m }],
        _ => Vec::new(),
    }
}

fn parse_javac(out: &str) -> Vec<Problem> {
    let mut v = Vec::new();
    let lines: Vec<&str> = out.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let Some(idx) = l.find(": error: ") else { continue };
        let line = l[..idx].rsplit(':').next().and_then(|n| n.trim().parse::<u32>().ok()).unwrap_or(1);
        let mut msg = l[idx + 9..].to_owned();
        if msg.contains("cannot find symbol")
            && let Some(sym) = lines.iter().skip(i + 1).take(4).find(|x| x.trim_start().starts_with("symbol:"))
        {
            msg = format!("cannot find symbol {}", sym.trim().trim_start_matches("symbol:").trim());
        }
        let col = lines.get(i + 2).and_then(|c| c.find('^')).map(|c| c as u32 + 1).unwrap_or(1);
        v.push(Problem { line, col, message: msg });
    }
    v
}

fn parse_short(out: &str) -> Vec<Problem> {
    let mut v = Vec::new();
    for l in out.lines() {
        let Some(idx) = l.find(": error") else { continue };
        let head = &l[..idx];
        let mut parts = head.rsplitn(3, ':');
        let col = parts.next().and_then(|c| c.trim().parse::<u32>().ok()).unwrap_or(1);
        let line = parts.next().and_then(|c| c.trim().parse::<u32>().ok());
        if let Some(line) = line {
            v.push(Problem { line, col, message: l[idx..].trim_start_matches(": error").trim_start_matches(|c: char| c == ':' || c == '[' || c.is_ascii_alphanumeric() || c == ']').trim().to_owned() });
        }
    }
    v
}

fn parse_gofmt(out: &str) -> Vec<Problem> {
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(4, ':');
            let _file = parts.next()?;
            let line = parts.next()?.trim().parse::<u32>().ok()?;
            let col = parts.next()?.trim().parse::<u32>().ok()?;
            Some(Problem { line, col, message: parts.next().unwrap_or("").trim().to_owned() })
        })
        .collect()
}

pub fn check(path: &Path, text: Option<&str>, language_id: &str) -> Result<Vec<Problem>> {
    let l = lang(path, language_id);
    let tool = compiler_for(l).ok_or_else(|| {
        anyhow!(match l {
            Lang::C | Lang::Cpp => "I can't check C or C++ code because no compiler is installed. Say install MinGW, or install the C++ extension in VS Code.".to_owned(),
            Lang::Other => "I can only check C, C++, Python, JavaScript, Java, Go and Rust files myself.".to_owned(),
            other => format!("I can't check {} code because its tools aren't installed.", lang_name(other)),
        })
    })?;
    let dir = tempfile::tempdir()?;
    let name = path.file_name().map(|n| n.to_owned()).unwrap_or_else(|| "main.txt".into());
    let src = dir.path().join(&name);
    match text {
        Some(t) => std::fs::write(&src, t)?,
        None => {
            std::fs::copy(path, &src)?;
        }
    }
    let inc = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let s = src.to_string_lossy().into_owned();
    let tool_s = tool.to_string_lossy().into_owned();
    let secs = Duration::from_secs(30);
    let (ok, out, err) = match l {
        Lang::C => output(&tool_s, &["-fsyntax-only", "-fdiagnostics-color=never", "-I", &inc, &s], None, secs)?,
        Lang::Cpp => output(&tool_s, &["-fsyntax-only", "-std=c++17", "-fdiagnostics-color=never", "-I", &inc, &s], None, secs)?,
        Lang::Python => output(&tool_s, &["-m", "py_compile", &s], None, secs)?,
        Lang::JavaScript => output(&tool_s, &["--check", &s], None, secs)?,
        Lang::Java => output(&tool_s, &["-d", &dir.path().to_string_lossy(), "-Xlint:none", &s], None, secs)?,
        Lang::Go => output(&tool_s, &["-e", &s], None, secs)?,
        Lang::Rust => {
            let project = path.ancestors().find(|a| a.join("Cargo.toml").is_file());
            match project {
                Some(p) if tool_s.contains("cargo") => output(&tool_s, &["check", "--quiet", "--message-format", "short"], Some(p), Duration::from_secs(180))?,
                _ => {
                    let rustc = find_program(if cfg!(windows) { &["rustc.exe"] } else { &["rustc"] }, &dirs::home_dir().map(|h| vec![h.join(".cargo").join("bin")]).unwrap_or_default()).unwrap_or(tool.clone());
                    output(&rustc.to_string_lossy(), &["--error-format", "short", "--emit", "metadata", "--out-dir", &dir.path().to_string_lossy(), &s], None, secs)?
                }
            }
        }
        Lang::TypeScript | Lang::Other => return Ok(Vec::new()),
    };
    if ok {
        return Ok(Vec::new());
    }
    let all = format!("{err}\n{out}");
    let mut found = match l {
        Lang::C | Lang::Cpp => parse_gcc(&all),
        Lang::Python => parse_python(&all),
        Lang::JavaScript => parse_node(&all),
        Lang::Java => parse_javac(&all),
        Lang::Go => parse_gofmt(&all),
        Lang::Rust => {
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            parse_short(&all).into_iter().filter(|_| !file.is_empty()).collect()
        }
        _ => Vec::new(),
    };
    if found.is_empty() {
        let line = all.lines().find(|x| !x.trim().is_empty()).unwrap_or("The check failed.").trim().to_owned();
        found.push(Problem { line: 1, col: 1, message: line });
    }
    let source = std::fs::read_to_string(&src).unwrap_or_default();
    for p in found.iter_mut() {
        place(p, &source);
    }
    Ok(found)
}

fn error_tail(flat: &str, from: usize) -> String {
    let rest = &flat[from..];
    let mut end = rest.len();
    for stop in [" at ", "bash-", " $ ", "$ ", " > ", " PS ", "    at "] {
        if let Some(i) = rest.find(stop)
            && i > 0
            && i < end
        {
            end = i;
        }
    }
    rest[..end].chars().take(160).collect::<String>().trim().trim_end_matches('.').to_owned()
}

fn last_error(flat: &str) -> Option<usize> {
    for marker in ["Error: ", "Exception: ", "Error:", "Exception"] {
        if let Some(i) = flat.rfind(marker) {
            let head = &flat[..i];
            let start = head.char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '.').last().map(|(j, _)| j).unwrap_or(i);
            return Some(start);
        }
    }
    None
}

fn number_after(flat: &str, at: usize) -> Option<u32> {
    flat[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

pub fn run_error(l: Lang, out: &str) -> Option<Problem> {
    run_error_in(l, out, &out.replace(['\n', '\t'], " ")).or_else(|| run_error_in(l, &out.replace('\n', ""), &out.replace('\n', "").replace('\t', " ")))
}

fn run_error_in(l: Lang, out: &str, flat: &str) -> Option<Problem> {
    let flat = flat.to_owned();
    match l {
        Lang::Python => {
            let at = flat.rfind(", line ")?;
            let line = number_after(&flat, at + 7)?;
            let from = last_error(&flat).filter(|i| *i > at)?;
            Some(Problem { line, col: 1, message: error_tail(&flat, from) })
        }
        Lang::JavaScript | Lang::TypeScript => {
            let from = last_error(&flat)?;
            let mut line = None;
            for ext in [".js:", ".mjs:", ".cjs:", ".ts:"] {
                if let Some(i) = flat[..from].rfind(ext) {
                    line = number_after(&flat, i + ext.len());
                    if line.is_some() {
                        break;
                    }
                }
            }
            Some(Problem { line: line.unwrap_or(0), col: 1, message: error_tail(&flat, from) })
        }
        Lang::Java => {
            let at = flat.find("Exception in thread")?;
            let quote = flat[at..].find("\" ").map(|q| at + q + 2).unwrap_or(at);
            let line = flat[at..].find(".java:").and_then(|i| number_after(&flat, at + i + 6)).unwrap_or(0);
            Some(Problem { line, col: 1, message: error_tail(&flat, quote) })
        }
        _ => {
            if let Some(p) = parse_gcc(out).into_iter().next() {
                return Some(p);
            }
            if flat.contains("Segmentation fault") || flat.contains("core dumped") {
                return Some(Problem { line: 0, col: 1, message: "the program crashed with a segmentation fault".into() });
            }
            if let Some(i) = flat.find("panicked at ") {
                let line = flat[i..].find(".rs:").and_then(|j| number_after(&flat, i + j + 4)).unwrap_or(0);
                return Some(Problem { line, col: 1, message: error_tail(&flat, i) });
            }
            None
        }
    }
}

pub fn place(p: &mut Problem, source: &str) {
    let m = p.message.to_lowercase().replace(['\u{2018}', '\u{2019}'], "'");
    let separator = (m.contains("';'") || m.contains("','") || m.contains("')'")) && (m.contains("before") || m.contains("expected"));
    if !separator || p.line < 2 {
        return;
    }
    let lines: Vec<&str> = source.lines().collect();
    let Some(row) = lines.get(p.line as usize - 1) else { return };
    let lead: String = row.chars().take(p.col.saturating_sub(1) as usize).collect();
    if !lead.trim().is_empty() {
        return;
    }
    if let Some(k) = (0..p.line as usize - 1).rev().find(|k| !lines[*k].trim().is_empty()) {
        p.line = k as u32 + 1;
        p.col = lines[k].trim_end().chars().count() as u32 + 1;
    }
}

fn quoted(msg: &str) -> Option<String> {
    for (open, close) in [('\'', '\''), ('\u{2018}', '\u{2019}'), ('`', '`'), ('"', '"')] {
        if let Some(a) = msg.find(open) {
            let rest = &msg[a + open.len_utf8()..];
            if let Some(b) = rest.find(close) {
                let name = &rest[..b];
                if !name.is_empty() && name.len() < 40 {
                    return Some(name.to_owned());
                }
            }
        }
    }
    None
}

pub fn explain(p: &Problem) -> String {
    let m = p.message.to_lowercase().replace(['\u{2018}', '\u{2019}', '`', '"'], "'");
    let what = if m.contains("';'") && (m.contains("expected") || m.contains("missing")) {
        "you forgot a semicolon".to_owned()
    } else if m.contains("expected '}'") || m.contains("'}' expected") || m.contains("unclosed delimiter") || m.contains("reached end of file while parsing") || m.contains("expected declaration or statement at end of input") {
        "a closing curly brace is missing".to_owned()
    } else if m.contains("expected ')'") || m.contains("')' expected") || m.contains("was never closed") && m.contains('(') {
        "a closing bracket is missing".to_owned()
    } else if m.contains("expected ']'") || m.contains("']' expected") {
        "a closing square bracket is missing".to_owned()
    } else if m.contains("expected ':'") {
        "you forgot a colon".to_owned()
    } else if m.contains("indent") {
        "the indentation is wrong".to_owned()
    } else if m.contains("missing terminating") || m.contains("unterminated string") || m.contains("unclosed string") || m.contains("eol while scanning") || m.contains("unterminated") {
        "a quote is not closed".to_owned()
    } else if m.contains("not declared") || m.contains("undeclared identifier") || m.contains("cannot find symbol") || m.contains("cannot find value") || m.contains("undefined:") || m.contains("is not defined") || m.contains("unknown type name") {
        let name = quoted(&p.message).or_else(|| p.message.split_whitespace().last().map(str::to_owned)).unwrap_or_else(|| "a name".into());
        let name = name.trim_start_matches("variable ").trim_start_matches("method ").trim_start_matches("class ").to_owned();
        format!("{name} is not declared")
    } else if m.contains("no such file or directory") && (m.contains(".h") || m.contains("include")) {
        format!("the file {} can't be found", quoted(&p.message).unwrap_or_else(|| "you included".into()))
    } else if m.contains("redefinition") || m.contains("already defined") || m.contains("redeclared") || m.contains("conflicting declaration") {
        format!("{} is declared twice", quoted(&p.message).unwrap_or_else(|| "something".into()))
    } else if m.contains("cannot convert") || m.contains("incompatible types") || m.contains("mismatched types") || m.contains("invalid conversion") {
        "the types don't match".to_owned()
    } else if m.contains("expected primary-expression") || m.contains("expected expression") || m.contains("invalid syntax") || m.contains("unexpected token") || m.contains("illegal start of expression") {
        "something is missing or extra here".to_owned()
    } else if m.contains("too few arguments") || m.contains("too many arguments") || m.contains("no matching function") {
        "the function is called with the wrong arguments".to_owned()
    } else if m.contains("return") && m.contains("void") {
        "the return doesn't match the function type".to_owned()
    } else {
        let clean = p.message.trim().trim_start_matches("SyntaxError: ").trim_start_matches("error: ").trim_end_matches('.');
        let short: String = clean.chars().take(90).collect();
        format!("there's a problem: {short}")
    };
    format!("Yes, {what} on line {}.", p.line)
}

pub struct RunPlan {
    pub steps: Vec<String>,
    pub dir: PathBuf,
}

fn q(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

pub fn run_plan(path: &Path, language_id: &str) -> Option<RunPlan> {
    let l = lang(path, language_id);
    let dir = path.parent()?.to_path_buf();
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let exe = if cfg!(windows) { dir.join(format!("{stem}.exe")) } else { dir.join(&stem) };
    let run_exe = if cfg!(windows) { format!("& {}", q(&exe)) } else { q(&exe) };
    let steps = match l {
        Lang::C => vec![format!("gcc {} -o {}", q(path), q(&exe)), run_exe],
        Lang::Cpp => vec![format!("g++ -std=c++17 {} -o {}", q(path), q(&exe)), run_exe],
        Lang::Python => vec![format!("{} {}", if cfg!(windows) { "python" } else { "python3" }, q(path))],
        Lang::JavaScript => vec![format!("node {}", q(path))],
        Lang::TypeScript => vec![format!("npx tsx {}", q(path))],
        Lang::Java => vec![format!("java {}", q(path))],
        Lang::Go => vec![format!("go run {}", q(path))],
        Lang::Rust => match path.ancestors().find(|a| a.join("Cargo.toml").is_file()) {
            Some(p) => return Some(RunPlan { steps: vec!["cargo run".into()], dir: p.to_path_buf() }),
            None => vec![format!("rustc {} -o {}", q(path), q(&exe)), run_exe],
        },
        Lang::Other => return None,
    };
    Some(RunPlan { steps, dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_common_mistakes() {
        let p = |m: &str| Problem { line: 5, col: 14, message: m.into() };
        assert_eq!(explain(&p("expected ';' before 'return'")), "Yes, you forgot a semicolon on line 5.");
        assert_eq!(explain(&p("expected \u{2018};\u{2019} at end of declaration")), "Yes, you forgot a semicolon on line 5.");
        assert_eq!(explain(&p("';' expected")), "Yes, you forgot a semicolon on line 5.");
        assert_eq!(explain(&p("'count' was not declared in this scope")), "Yes, count is not declared on line 5.");
        assert_eq!(explain(&p("use of undeclared identifier 'x'")), "Yes, x is not declared on line 5.");
        assert_eq!(explain(&p("SyntaxError: expected ':'")), "Yes, you forgot a colon on line 5.");
        assert_eq!(explain(&p("expected '}' at end of input")), "Yes, a closing curly brace is missing on line 5.");
        assert!(explain(&p("something odd happened")).starts_with("Yes, there's a problem: something odd happened"));
        assert_eq!(explain(&p("expected ',' or ';' before 'std'")), "Yes, you forgot a semicolon on line 5.");
        assert_eq!(explain(&p("expected a ';'")), "Yes, you forgot a semicolon on line 5.");
        let mut q = Problem { line: 4, col: 5, message: "expected ',' or ';' before 'std'".into() };
        place(&mut q, "int main() {\n    int i = 10\n\n    std::cout << i;\n}\n");
        assert_eq!((q.line, q.col), (2, 15));
    }

    #[test]
    fn parses_compiler_output() {
        let gcc = "/tmp/x/main.cpp: In function 'int main()':\n/tmp/x/main.cpp:5:15: error: expected ';' before 'return'\n    5 |     int i = 10\n";
        assert_eq!(parse_gcc(gcc), vec![Problem { line: 5, col: 15, message: "expected ';' before 'return'".into() }]);
        let win = "C:\\Users\\a\\main.cpp:7:3: error: 'x' was not declared in this scope";
        assert_eq!(parse_gcc(win)[0].line, 7);
        let py = "  File \"/tmp/a.py\", line 3\n    if x\n        ^\nSyntaxError: expected ':'\n";
        assert_eq!(parse_python(py)[0].line, 3);
        assert!(parse_python(py)[0].message.contains("expected ':'"));
        let javac = "Main.java:4: error: ';' expected\n        int x = 5\n                 ^\n1 error\n";
        assert_eq!(parse_javac(javac)[0].line, 4);
        assert_eq!(parse_gofmt("/tmp/main.go:6:2: expected '}', found 'EOF'")[0].line, 6);
    }

    #[test]
    fn reads_run_errors_from_the_terminal() {
        let py = "python3 \"/home/u/demo/a.py\" Traceback (most recent call last):\n  File \"/home/u/demo/a.py\", line 3, in <module>\n    print(1/0)\nZeroDivisionError: division by zero\nbash-5.3$";
        let p = run_error(Lang::Python, py).unwrap();
        assert_eq!((p.line, p.message.as_str()), (3, "ZeroDivisionError: division by zero"));
        let js = "node \"/home/u/demo/a.js\"\n/home/u/demo/a.js:2\nfoo();\n^\n\nReferenceError: foo is not defined\n    at Object.<anonymous> (/home/u/demo/a.js:2:1)";
        let p = run_error(Lang::JavaScript, js).unwrap();
        assert_eq!((p.line, p.message.as_str()), (2, "ReferenceError: foo is not defined"));
        let java = "java Main.java\nException in thread \"main\" java.lang.ArithmeticException: / by zero\n\tat Main.main(Main.java:5)";
        let p = run_error(Lang::Java, java).unwrap();
        assert_eq!((p.line, p.message.as_str()), (5, "java.lang.ArithmeticException: / by zero"));
        let cpp = "cd \"/x\" && g++ main.cpp -o main && ./main\nmain.cpp:4:5: error: 'y' was not declared in this scope\n";
        assert_eq!(run_error(Lang::Cpp, cpp).map(|p| p.line), Some(4));
        assert_eq!(run_error(Lang::C, "./main\nSegmentation fault (core dumped)").map(|p| p.line), Some(0));
        assert_eq!(run_error(Lang::Rust, "thread 'main' panicked at src/main.rs:7:5:\nattempt to divide by zero").map(|p| p.line), Some(7));
        let wrapped = "Traceback (most recent call last):\nFile \"/tmp/a/demo/crash.py\", l\nine 2, in <module>\nprint(x / 0)\n~~^~~\nZeroDivisionError: division by zero";
        let p = run_error(Lang::Python, wrapped).unwrap();
        assert_eq!((p.line, p.message.as_str()), (2, "ZeroDivisionError: division by zero"));
        assert!(run_error(Lang::Python, "python3 a.py\nhello\nbash-5.3$").is_none());
        assert!(run_error(Lang::Cpp, "./main\n42\n").is_none());
    }

    #[test]
    fn edits_settings_with_comments() {
        let text = "{\n    // keep me\n    \"editor.fontSize\": 14, /* size */\n    \"files.autoSave\": \"off\"\n}\n";
        let a = set_jsonc(text, "editor.fontSize", &json!(16)).unwrap();
        assert!(a.contains("\"editor.fontSize\": 16,") && a.contains("// keep me") && a.contains("/* size */"));
        let b = set_jsonc(text, "files.autoSave", &json!("afterDelay")).unwrap();
        assert!(b.contains("\"files.autoSave\": \"afterDelay\"\n}"));
        let c = set_jsonc(text, "editor.wordWrap", &json!("on")).unwrap();
        assert!(c.starts_with("{\n    \"editor.wordWrap\": \"on\",\n    // keep me"));
        assert_eq!(set_jsonc("", "a", &json!(1)).unwrap(), "{\n    \"a\": 1\n}\n");
        assert_eq!(set_jsonc("{}", "a", &json!(true)).unwrap(), "{\n    \"a\": true\n}");
        assert!(set_jsonc("[1, 2]", "a", &json!(1)).is_none());
        let nested = "{ \"x\": { \"y\": [1, {\"z\": 2}] }, \"k\": 1 }";
        assert_eq!(set_jsonc(nested, "k", &json!(5)).unwrap(), "{ \"x\": { \"y\": [1, {\"z\": 2}] }, \"k\": 5 }");
    }

    #[test]
    fn finds_extensions_and_settings() {
        assert_eq!(known_extension("python").map(|x| x.0), Some("ms-python.python".into()));
        assert_eq!(known_extension("the c plus plus extension").map(|x| x.0), Some("ms-vscode.cpptools".into()));
        assert_eq!(known_extension("prettier"), Some(("esbenp.prettier-vscode".into(), "Prettier".into())));
        assert_eq!(setting_for("turn on auto save").map(|x| x.0), Some("files.autoSave".into()));
        assert_eq!(setting_for("set font size to sixteen").map(|x| x.1), Some(json!(16)));
        assert_eq!(setting_for("turn off the minimap").map(|x| x.1), Some(json!(false)));
        assert_eq!(setting_for("change theme to dark").map(|x| x.1), Some(json!("Default Dark Modern")));
        assert!(setting_for("make me a sandwich").is_none());
    }

    #[test]
    fn checks_real_code_when_a_compiler_exists() {
        if compiler_for(Lang::Cpp).is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("main.cpp");
        std::fs::write(&f, "#include <iostream>\nint main() {\n    int i = 10\n    std::cout << i;\n    return 0;\n}\n").unwrap();
        let probs = check(&f, None, "cpp").unwrap();
        assert!(!probs.is_empty());
        assert_eq!((probs[0].line, probs[0].col), (3, 15));
        assert_eq!(explain(&probs[0]), "Yes, you forgot a semicolon on line 3.");
        std::fs::write(&f, "int main() { int i = 10; return i; }\n").unwrap();
        assert!(check(&f, None, "cpp").unwrap().is_empty());
    }
}
