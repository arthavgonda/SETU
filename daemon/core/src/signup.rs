use crate::core::Core;
use crate::router::Intent;
use crate::tools::Outcome;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LAYER: &str = "vault";
const KEEP: Duration = Duration::from_secs(1200);

pub struct Provider {
    pub words: &'static [&'static str],
    pub title: &'static str,
    pub domain: &'static str,
    pub site: &'static str,
    pub url: &'static str,
}

pub const PROVIDERS: &[Provider] = &[
    Provider { words: &["gmail", "google", "google mail"], title: "Google", domain: "gmail.com", site: "google.com", url: "https://accounts.google.com/signup" },
    Provider { words: &["outlook", "hotmail", "microsoft"], title: "Microsoft", domain: "outlook.com", site: "live.com", url: "https://signup.live.com/" },
    Provider { words: &["proton", "protonmail", "proton mail"], title: "Proton", domain: "proton.me", site: "proton.me", url: "https://account.proton.me/mail/signup" },
    Provider { words: &["yahoo", "yahoo mail"], title: "Yahoo", domain: "yahoo.com", site: "yahoo.com", url: "https://login.yahoo.com/account/create" },
    Provider { words: &["zoho", "zoho mail"], title: "Zoho", domain: "zohomail.in", site: "zoho.com", url: "https://www.zoho.com/mail/signup.html" },
];

struct Draft {
    provider: usize,
    first: String,
    last: String,
    user: String,
    uid: Option<String>,
    at: Instant,
}

static DRAFT: Mutex<Option<Draft>> = Mutex::new(None);

fn pending() -> bool {
    DRAFT.lock().unwrap().as_ref().is_some_and(|d| d.at.elapsed() < KEEP)
}

fn provider_of(word: &str) -> Option<usize> {
    PROVIDERS.iter().position(|p| p.words.contains(&word))
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn user_part(spoken: &str) -> Option<String> {
    let mut s = format!(" {} ", spoken.trim().to_lowercase());
    for (a, b) in [(" at the rate ", "@"), (" at ", "@"), (" dot ", "."), (" underscore ", "_"), (" dash ", "-"), (" hyphen ", "-"), (" full stop ", "."), (" point ", ".")] {
        s = s.replace(a, b);
    }
    let joined: String = s.split_whitespace().collect();
    let local = joined.split('@').next().unwrap_or_default().trim_matches('.').to_owned();
    let ok = (3..=40).contains(&local.len()) && local.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c)) && local.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    ok.then_some(local)
}

fn name_parts(spoken: &str) -> Option<(String, String)> {
    let n = title_case(spoken.trim());
    let words: Vec<&str> = n.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 || !n.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '\'' || c == '-') {
        return None;
    }
    Some((words[0].to_owned(), words[1..].join(" ")))
}

fn strip_any<'a>(t: &'a str, pre: &[&str]) -> Option<&'a str> {
    pre.iter().find_map(|p| t.strip_prefix(p))
}

fn details(rest: &str) -> (Option<&str>, Option<&str>) {
    let mut name = None;
    let mut user = None;
    let mut body = rest.trim();
    for sep in [" with the address ", " with address ", " called ", " with the username ", " with the name of "] {
        if let Some((a, b)) = body.split_once(sep) {
            body = a;
            user = Some(b.trim());
            break;
        }
    }
    if let Some(x) = strip_any(body.trim(), &["for ", "in the name of ", "with the name "]) {
        name = Some(x.trim());
    }
    (name, user)
}

pub fn rules(t: &str) -> Option<Intent> {
    let act = |args: Value| Some(Intent::Act { tool: "vault.signup".into(), args });
    let starts = ["create a new ", "create a ", "create me a new ", "create me a ", "make a new ", "make me a new ", "make me a ", "make a ", "set up a new ", "open a new ", "sign me up for a new ", "sign me up for a ", "sign me up for ", "sign up for a new ", "sign up for a ", "sign up for ", "i want a new ", "i need a new "];
    if let Some(rest) = strip_any(t, &starts) {
        for (i, p) in PROVIDERS.iter().enumerate() {
            for w in p.words {
                let Some(after) = rest.strip_prefix(w) else { continue };
                let after = after.trim_start();
                let after = strip_any(after, &["email account", "mail account", "email address", "account", "email", "address", "id"]).map(str::trim_start).unwrap_or(after);
                if !after.is_empty() && !after.starts_with("for ") && !after.starts_with("with ") && !after.starts_with("called ") && !after.starts_with("in the name") {
                    continue;
                }
                let (name, user) = details(after);
                return act(json!({ "op": "start", "provider": i, "name": name, "user": user }));
            }
        }
        if let Some(after) = strip_any(rest, &["email account", "mail account", "email address", "email id", "email"]) {
            let (name, user) = details(after);
            return act(json!({ "op": "start", "name": name, "user": user }));
        }
    }
    if !pending() {
        return None;
    }
    if matches!(t, "cancel the new account" | "forget the new account" | "stop making the account" | "cancel the account" | "never mind the new account") {
        return act(json!({ "op": "cancel" }));
    }
    if matches!(t, "fill this page" | "fill in this page" | "fill the page" | "fill in the page" | "fill the sign up page" | "fill the next page" | "carry on with the new account" | "continue with the new account" | "fill it in" | "fill in the password" | "put the password in") {
        return act(json!({ "op": "fill" }));
    }
    let mut args = json!({ "op": "set" });
    let mut body = t;
    let mut found = false;
    if let Some((a, b)) = body.split_once(" and the address is ").or_else(|| body.split_once(" and address is ")).or_else(|| body.split_once(" and i want the address ")) {
        args["user"] = json!(b.trim());
        body = a;
        found = true;
    }
    if let Some(x) = strip_any(body, &["my name is ", "the name is ", "name is ", "use the name ", "put the name ", "it is for ", "it's for "]) {
        args["name"] = json!(x.trim());
        found = true;
    } else if let Some(x) = strip_any(body, &["the address is ", "address is ", "use the address ", "i want the address ", "i would like the address ", "make the address ", "try the address "]) {
        args["user"] = json!(x.trim());
        found = true;
    } else if let Some(p) = strip_any(body, &["use ", "make it a ", "make it "]).and_then(|x| provider_of(x.trim().trim_end_matches(" account"))) {
        args["provider"] = json!(p);
        found = true;
    }
    found.then(|| Intent::Act { tool: "vault.signup".into(), args })
}

pub fn label(args: &Value) -> String {
    match args.get("op").and_then(Value::as_str) {
        Some("cancel") => "Dropping the new account".into(),
        Some("fill") => "Filling in the page".into(),
        _ => "Setting up a new mail account".into(),
    }
}

pub fn strong_password() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    const SETS: [&[u8]; 4] = [b"abcdefghijkmnopqrstuvwxyz", b"ABCDEFGHJKLMNPQRSTUVWXYZ", b"23456789", b"!@#%^*-_=+?"];
    let rng = SystemRandom::new();
    let pick = |n: usize| -> usize {
        let limit = (u32::MAX / n as u32) * n as u32;
        loop {
            let mut b = [0u8; 4];
            if rng.fill(&mut b).is_err() {
                return fastrand::usize(..n);
            }
            let v = u32::from_le_bytes(b);
            if v < limit {
                return (v % n as u32) as usize;
            }
        }
    };
    let mut out: Vec<u8> = SETS.iter().map(|s| s[pick(s.len())]).collect();
    let all: Vec<u8> = SETS.iter().flat_map(|s| s.iter().copied()).collect();
    while out.len() < 20 {
        out.push(all[pick(all.len())]);
    }
    for i in (1..out.len()).rev() {
        out.swap(i, pick(i + 1));
    }
    String::from_utf8(out).unwrap_or_default()
}

const FILL_JS: &str = "JSON.stringify(((F,L,U,P)=>{const vis=e=>{const r=e.getBoundingClientRect();const s=getComputedStyle(e);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'};const set=(e,v)=>{e.focus();Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}))};const lab=e=>{let a='';try{const l=e.id&&document.querySelector('label[for=\"'+CSS.escape(e.id)+'\"]');if(l)a=l.innerText||''}catch(x){}const w=e.closest('label');return (a+' '+((w&&w.innerText)||'')).trim()};const key=e=>[e.autocomplete,e.name,e.id,e.getAttribute('aria-label'),e.placeholder,lab(e)].join(' ').toLowerCase();const ins=[...document.querySelectorAll('input')].filter(vis).filter(e=>!['hidden','submit','button','checkbox','radio','file','image','reset'].includes((e.type||'').toLowerCase()));const did=[];const left=[];const add=(a,w)=>{if(!a.includes(w))a.push(w)};for(const e of ins){const k=key(e);const t=(e.type||'text').toLowerCase();if(t==='password'){if(P){set(e,P);add(did,'password')}continue}if(/given|first.?name|firstname|fname/.test(k)){if(F){if(e.value!==F)set(e,F);add(did,'first name')}else add(left,'first name');continue}if(/family|last.?name|lastname|surname|lname/.test(k)){if(L){if(e.value!==L)set(e,L);add(did,'last name')}else add(left,'last name');continue}if(t==='tel'||/phone|mobile/.test(k)){add(left,'phone number');continue}if(/birth|bday|dob|\\bday\\b|\\bmonth\\b|\\byear\\b/.test(k)){add(left,'date of birth');continue}if(/code|otp|verif|captcha/.test(k)){add(left,'verification code');continue}if(/user.?name|e.?mail|address|login|member|alias/.test(k)){if(U){if(e.value!==U)set(e,U);add(did,'address')}else add(left,'address');continue}if(/name/.test(k)){const n=(F+' '+L).trim();if(n){if(e.value!==n)set(e,n);add(did,'name')}else add(left,'name');continue}if(!e.value)add(left,(lab(e)||e.getAttribute('aria-label')||e.placeholder||e.name||'another field').trim().slice(0,40).toLowerCase())}for(const s of [...document.querySelectorAll('select')].filter(vis)){const k=key(s);if(/birth|month|day|year/.test(k))add(left,'date of birth');else if(/gender|sex/.test(k))add(left,'gender')}const puzzle=!!document.querySelector('iframe[src*=\"captcha\" i],iframe[title*=\"captcha\" i],[class*=\"captcha\" i],[id*=\"captcha\" i]');return {did,left,puzzle,fields:ins.length}})(F,L,U,P))";

fn fill_js(first: &str, last: &str, user: &str, pass: &str) -> String {
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    FILL_JS.replacen("(F,L,U,P))", &format!("({},{},{},{}))", q(first), q(last), q(user), q(pass)), 1)
}

fn target_url(p: &Provider) -> String {
    if std::env::var_os("SETU_BENCH").is_some()
        && let Ok(u) = std::env::var("SETU_SIGNUP_URL")
    {
        return u;
    }
    p.url.to_owned()
}

fn allowed_host(p: &Provider, host: &str) -> bool {
    let bench = crate::vault::host_of(&target_url(p));
    crate::vault::site_matches(p.site, host) || (std::env::var_os("SETU_BENCH").is_some() && !bench.is_empty() && bench == host)
}

fn join(list: &[String]) -> String {
    crate::skills::join_names_pub(list)
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn report(v: &Value) -> String {
    let did = strings(v, "did");
    let left = strings(v, "left");
    let mut s = if did.is_empty() { "I found nothing on this page that I can fill.".to_owned() } else { format!("I filled in the {}.", join(&did)) };
    if !left.is_empty() {
        s.push_str(&format!(" The {} {} yours to fill.", join(&left), if left.len() == 1 { "is" } else { "are" }));
    }
    if v.get("puzzle").and_then(Value::as_bool) == Some(true) {
        s.push_str(" There is a picture puzzle on the page; that one is yours too.");
    }
    s
}

async fn fill(core: &Arc<Core>) -> Result<Value, String> {
    let (pi, first, last, user, uid) = {
        let g = DRAFT.lock().unwrap();
        let d = g.as_ref().filter(|d| d.at.elapsed() < KEEP).ok_or("There is no new account in progress. Say create a new Gmail account.")?;
        (d.provider, d.first.clone(), d.last.clone(), d.user.clone(), d.uid.clone())
    };
    let p = &PROVIDERS[pi];
    let conn = crate::proto::any().ok_or("That needs your Setu browser. Say set up my browser first.")?;
    let tab = crate::proto::active_tab(&conn).await.map_err(|e| e.to_string())?;
    let host = crate::vault::host_of(&tab.url);
    if !allowed_host(p, &host) {
        return Err(format!("This page isn't {}'s, so I won't put the new password here.", p.title));
    }
    let uid = uid.ok_or("The new password isn't saved yet.")?;
    let mut pass = crate::vault::secret(core, &uid).await.map_err(|e| e.to_string())?;
    let mut js = fill_js(&first, &last, &user, &pass);
    crate::vault::wipe(&mut pass);
    let r = conn.eval_json(&tab.id, &js).await;
    crate::vault::wipe(&mut js);
    r.map_err(|e| e.to_string())
}

async fn begin(core: &Arc<Core>) -> Outcome {
    let (pi, user, old) = {
        let g = DRAFT.lock().unwrap();
        let Some(d) = g.as_ref() else { return Outcome::fail("There is no new account in progress.", LAYER) };
        (d.provider, d.user.clone(), d.uid.clone())
    };
    let p = &PROVIDERS[pi];
    let Some(conn) = crate::proto::any() else {
        return Outcome::fail("Creating an account needs your Setu browser, so I can fill the page without anyone hearing the password. Say set up my browser, then ask me again.", LAYER);
    };
    let account = format!("{user}@{}", p.domain);
    let mut password = strong_password();
    let saved = crate::vault::save(core, p.title, &account, p.site, "web", &password).await;
    crate::vault::wipe(&mut password);
    let entry = match saved {
        Ok(e) => e,
        Err(e) => return Outcome::fail(&format!("I couldn't save the new password in the vault, so I haven't started. {e}"), LAYER),
    };
    if let Some(o) = old.filter(|o| *o != entry.uid) {
        let _ = crate::vault::forget(core, &o);
    }
    crate::vault::refresh(core);
    if let Some(d) = DRAFT.lock().unwrap().as_mut() {
        d.uid = Some(entry.uid.clone());
        d.at = Instant::now();
    }
    let tab = match crate::proto::active_tab(&conn).await {
        Ok(t) => t,
        Err(e) => return Outcome::fail(&e.to_string(), LAYER),
    };
    let url = target_url(p);
    if !allowed_host(p, &crate::vault::host_of(&tab.url))
        && let Err(e) = conn.navigate(&tab.id, &url).await
    {
        return Outcome::fail(&e.to_string(), LAYER);
    }
    let mut opened = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let on_site = crate::proto::active_tab(&conn).await.is_ok_and(|t| allowed_host(p, &crate::vault::host_of(&t.url)));
        if on_site && conn.eval_json(&tab.id, "JSON.stringify(document.readyState==='complete'&&document.querySelectorAll('input').length>0)").await.ok().and_then(|v| v.as_bool()) == Some(true) {
            opened = true;
            break;
        }
    }
    let saved_line = format!("I made a strong password and saved it in the vault as {}, {account}, before anything is sent.", p.title);
    if !opened {
        return Outcome::ok_with("Saved the new password", LAYER, None).with_reply(format!("{saved_line} The sign-up page didn't open fully. Open it, then say fill this page."));
    }
    match fill(core).await {
        Ok(v) => Outcome::ok_with(&format!("Filled in the {} sign-up page", p.title), LAYER, None).with_reply(format!("{saved_line} {} I haven't pressed anything: check the page and go on when you're ready. On the next page, say fill this page. Phone checks and picture puzzles are yours.", report(&v))),
        Err(e) => Outcome::ok_with("Saved the new password", LAYER, None).with_reply(format!("{saved_line} {e}")),
    }
}

pub async fn run(core: &Arc<Core>, args: &Value) -> Outcome {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned);
    match args.get("op").and_then(Value::as_str).unwrap_or("start") {
        "cancel" => {
            let had = DRAFT.lock().unwrap().take();
            match had.and_then(|d| d.uid.map(|u| (u, d.provider))) {
                Some((_, pi)) => Outcome::ok_with("Dropped the new account", LAYER, None).with_reply(format!("Okay, I've stopped. The password I made is still in the vault under {}; say forget my {} password if you don't want it.", PROVIDERS[pi].title, PROVIDERS[pi].title.to_lowercase())),
                None => Outcome::ok_with("Dropped the new account", LAYER, None).with_reply("Okay, I've stopped. Nothing was saved or sent.".into()),
            }
        }
        "fill" => match fill(core).await {
            Ok(v) => Outcome::ok_with("Filled in the page", LAYER, None).with_reply(format!("{} I haven't pressed anything.", report(&v))),
            Err(e) => Outcome::fail(&e, LAYER),
        },
        op => {
            let provider = args.get("provider").and_then(Value::as_u64).map(|p| p as usize).filter(|p| *p < PROVIDERS.len());
            let name = match s("name") {
                Some(n) => match name_parts(&n) {
                    Some(x) => Some(x),
                    None => return Outcome::fail("I didn't catch that as a name. Say the name is, then the first and last name.", LAYER),
                },
                None => None,
            };
            let user = match s("user") {
                Some(u) => match user_part(&u) {
                    Some(x) => Some(x),
                    None => return Outcome::fail("That address won't work. Use letters, numbers and dots, at least three of them: say the address is, then the part before the at sign.", LAYER),
                },
                None => None,
            };
            let (missing_provider, need_name, need_user, title) = {
                let mut g = DRAFT.lock().unwrap();
                if op == "start" || g.as_ref().is_none_or(|d| d.at.elapsed() >= KEEP) {
                    *g = Some(Draft { provider: provider.unwrap_or(usize::MAX), first: String::new(), last: String::new(), user: String::new(), uid: None, at: Instant::now() });
                }
                let d = g.as_mut().unwrap();
                if let Some(p) = provider {
                    d.provider = p;
                }
                if let Some((f, l)) = name {
                    d.first = f;
                    d.last = l;
                }
                if let Some(u) = user {
                    d.user = u;
                }
                d.at = Instant::now();
                (d.provider == usize::MAX, d.first.is_empty(), d.user.is_empty(), PROVIDERS.get(d.provider).map(|p| p.title).unwrap_or("mail"))
            };
            if missing_provider {
                return Outcome::ok_with("Asked which mail service", LAYER, None).with_reply("Which mail service should it be? Say use Gmail, use Outlook, use Proton or use Yahoo.".into());
            }
            match (need_name, need_user) {
                (true, true) => Outcome::ok_with("Asked for the details", LAYER, None).with_reply(format!("A new {title} account. What name should be on it, and which address would you like? Say the name is, then the name; and the address is, then the part before the at sign.")),
                (true, false) => Outcome::ok_with("Asked for the name", LAYER, None).with_reply("What name should be on it? Say the name is, then the first and last name.".into()),
                (false, true) => Outcome::ok_with("Asked for the address", LAYER, None).with_reply("Which address would you like? Say the address is, then the part before the at sign.".into()),
                (false, false) => begin(core).await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(t: &str) -> Option<Value> {
        match rules(t) {
            Some(Intent::Act { tool, args }) if tool == "vault.signup" => Some(args),
            _ => None,
        }
    }

    #[test]
    fn asking_for_a_new_account_names_the_service_and_details() {
        let a = args("create a new gmail account").unwrap();
        assert_eq!((a["op"].as_str(), a["provider"].as_u64()), (Some("start"), Some(0)));
        assert!(a["name"].is_null() && a["user"].is_null());
        let a = args("create a new outlook account for asha rao with the address asha dot rao 92").unwrap();
        assert_eq!(a["provider"], 1);
        assert_eq!(a["name"], "asha rao");
        assert_eq!(a["user"], "asha dot rao 92");
        assert_eq!(args("sign me up for proton mail").unwrap()["provider"], 2);
        assert!(args("create a new email account").unwrap()["provider"].is_null());
        assert!(args("create a new gmail filter").is_none());
        assert!(args("create a new folder").is_none());
    }

    #[test]
    fn spoken_addresses_and_names_are_cleaned() {
        assert_eq!(user_part("asha dot rao 92").as_deref(), Some("asha.rao92"));
        assert_eq!(user_part("asha underscore rao at gmail dot com").as_deref(), Some("asha_rao"));
        assert_eq!(user_part("a b"), None, "too short");
        assert_eq!(user_part("asha rao!"), None);
        assert_eq!(name_parts("asha rao"), Some(("Asha".into(), "Rao".into())));
        assert_eq!(name_parts("mary anne de souza"), Some(("Mary".into(), "Anne De Souza".into())));
        assert_eq!(name_parts("asha 2"), None);
    }

    #[test]
    fn made_passwords_are_long_mixed_and_different() {
        let a = strong_password();
        let b = strong_password();
        assert_eq!(a.len(), 20);
        assert_ne!(a, b);
        for p in [&a, &b] {
            assert!(p.chars().any(|c| c.is_ascii_lowercase()) && p.chars().any(|c| c.is_ascii_uppercase()) && p.chars().any(|c| c.is_ascii_digit()) && p.chars().any(|c| !c.is_ascii_alphanumeric()), "{p}");
            assert!(!p.contains(['"', '\\', ' ', '\'']));
        }
    }

    #[test]
    fn the_fill_script_carries_values_as_quoted_text() {
        let js = fill_js("A\"b", "O'Neil", "x.y", "p\\w");
        assert!(js.ends_with("(\"A\\\"b\",\"O'Neil\",\"x.y\",\"p\\\\w\"))"));
        assert!(!js.contains("(F,L,U,P))"));
    }
}
