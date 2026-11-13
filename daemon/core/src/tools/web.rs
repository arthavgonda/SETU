use super::apps::App;
use crate::core::Core;
use crate::text::{normalize, similarity};
use anyhow::{Result, anyhow};
use std::time::Duration;

pub struct Site {
    pub name: &'static str,
    pub words: &'static [&'static str],
    pub home: &'static str,
    pub search: Option<&'static str>,
}

pub const SITES: &[Site] = &[
    Site { name: "YouTube", words: &["youtube", "you tube", "yt"], home: "https://www.youtube.com", search: Some("https://www.youtube.com/results?search_query={q}") },
    Site { name: "YouTube Music", words: &["youtube music", "yt music"], home: "https://music.youtube.com", search: Some("https://music.youtube.com/search?q={q}") },
    Site { name: "Google", words: &["google", "google search"], home: "https://www.google.com", search: Some("https://www.google.com/search?q={q}") },
    Site { name: "Gmail", words: &["gmail", "g mail", "my email", "my mail", "google mail", "email"], home: "https://mail.google.com", search: Some("https://mail.google.com/mail/u/0/#search/{q}") },
    Site { name: "Google account", words: &["google account", "my google account", "google sign in", "google login"], home: "https://accounts.google.com/ServiceLogin", search: None },
    Site { name: "Google Drive", words: &["google drive", "drive", "my drive"], home: "https://drive.google.com", search: Some("https://drive.google.com/drive/search?q={q}") },
    Site { name: "Google Docs", words: &["google docs", "docs"], home: "https://docs.google.com/document", search: None },
    Site { name: "Google Sheets", words: &["google sheets", "sheets"], home: "https://docs.google.com/spreadsheets", search: None },
    Site { name: "Google Maps", words: &["google maps", "maps", "map"], home: "https://www.google.com/maps", search: Some("https://www.google.com/maps/search/{q}") },
    Site { name: "Google Translate", words: &["google translate", "translate", "translator"], home: "https://translate.google.com", search: Some("https://translate.google.com/?text={q}") },
    Site { name: "Google Calendar", words: &["google calendar", "calendar", "my calendar"], home: "https://calendar.google.com", search: None },
    Site { name: "Google Meet", words: &["google meet", "meet"], home: "https://meet.google.com", search: None },
    Site { name: "Google Photos", words: &["google photos", "photos"], home: "https://photos.google.com", search: Some("https://photos.google.com/search/{q}") },
    Site { name: "Netflix", words: &["netflix", "net flix"], home: "https://www.netflix.com/browse", search: Some("https://www.netflix.com/search?q={q}") },
    Site { name: "Prime Video", words: &["prime video", "amazon prime", "prime"], home: "https://www.primevideo.com", search: Some("https://www.primevideo.com/search/ref=atv_nb_sr?phrase={q}") },
    Site { name: "JioHotstar", words: &["hotstar", "jio hotstar", "disney hotstar", "disney plus"], home: "https://www.hotstar.com", search: Some("https://www.hotstar.com/in/explore?search_query={q}") },
    Site { name: "Spotify", words: &["spotify", "spotify web"], home: "https://open.spotify.com", search: Some("https://open.spotify.com/search/{q}") },
    Site { name: "GitHub", words: &["github", "git hub"], home: "https://github.com", search: Some("https://github.com/search?q={q}") },
    Site { name: "GitHub login", words: &["github login", "github sign in"], home: "https://github.com/login", search: None },
    Site { name: "Stack Overflow", words: &["stack overflow", "stackoverflow"], home: "https://stackoverflow.com", search: Some("https://stackoverflow.com/search?q={q}") },
    Site { name: "Wikipedia", words: &["wikipedia", "wiki"], home: "https://en.wikipedia.org", search: Some("https://en.wikipedia.org/w/index.php?search={q}") },
    Site { name: "Amazon", words: &["amazon", "amazon india"], home: "https://www.amazon.in", search: Some("https://www.amazon.in/s?k={q}") },
    Site { name: "Flipkart", words: &["flipkart", "flip kart"], home: "https://www.flipkart.com", search: Some("https://www.flipkart.com/search?q={q}") },
    Site { name: "WhatsApp Web", words: &["whatsapp", "whats app", "whatsapp web"], home: "https://web.whatsapp.com", search: None },
    Site { name: "Instagram", words: &["instagram", "insta"], home: "https://www.instagram.com", search: None },
    Site { name: "Facebook", words: &["facebook", "fb"], home: "https://www.facebook.com", search: Some("https://www.facebook.com/search/top?q={q}") },
    Site { name: "X", words: &["twitter", "x dot com", "x"], home: "https://x.com", search: Some("https://x.com/search?q={q}") },
    Site { name: "LinkedIn", words: &["linkedin", "linked in"], home: "https://www.linkedin.com", search: Some("https://www.linkedin.com/search/results/all/?keywords={q}") },
    Site { name: "Reddit", words: &["reddit"], home: "https://www.reddit.com", search: Some("https://www.reddit.com/search/?q={q}") },
    Site { name: "Outlook", words: &["outlook", "hotmail"], home: "https://outlook.live.com", search: None },
    Site { name: "ChatGPT", words: &["chatgpt", "chat gpt"], home: "https://chatgpt.com", search: None },
    Site { name: "Claude", words: &["claude", "claude ai"], home: "https://claude.ai", search: None },
    Site { name: "Bing", words: &["bing"], home: "https://www.bing.com", search: Some("https://www.bing.com/search?q={q}") },
    Site { name: "DuckDuckGo", words: &["duckduckgo", "duck duck go"], home: "https://duckduckgo.com", search: Some("https://duckduckgo.com/?q={q}") },
    Site { name: "News", words: &["google news", "news"], home: "https://news.google.com", search: Some("https://news.google.com/search?q={q}") },
    Site { name: "Weather", words: &["weather"], home: "https://www.google.com/search?q=weather", search: Some("https://www.google.com/search?q=weather+{q}") },
];

pub const BROWSERS: &[(&str, &[&str])] = &[
    ("brave", &["brave", "brave browser", "breve", "brave web browser"]),
    ("chrome", &["chrome", "google chrome", "crome"]),
    ("edge", &["edge", "microsoft edge", "ms edge"]),
    ("firefox", &["firefox", "fire fox", "mozilla", "mozilla firefox"]),
    ("opera", &["opera", "opera gx"]),
    ("vivaldi", &["vivaldi"]),
    ("safari", &["safari"]),
    ("chromium", &["chromium"]),
    ("librewolf", &["librewolf"]),
    ("zen", &["zen browser", "zen"]),
    ("arc", &["arc", "arc browser"]),
];

pub fn browser_key(spoken: &str) -> Option<&'static str> {
    let t = normalize(spoken);
    let t = t.trim_start_matches("the ").trim_end_matches(" browser").trim();
    BROWSERS.iter().find(|(_, words)| words.iter().any(|w| *w == t || w.trim_end_matches(" browser") == t)).map(|(k, _)| *k)
}

pub fn site(spoken: &str) -> Option<&'static Site> {
    let t = normalize(spoken);
    let t = t.trim_start_matches("the ").trim_start_matches("my ").trim();
    let t = t.trim_end_matches(" website").trim_end_matches(" site").trim_end_matches(" dot com").trim_end_matches(" com").trim();
    if t.is_empty() {
        return None;
    }
    for s in SITES {
        if s.words.contains(&t) {
            return Some(s);
        }
    }
    let full = normalize(spoken);
    for s in SITES {
        if s.words.iter().any(|w| w.len() >= 5 && full.trim_start_matches("my ") == *w) {
            return Some(s);
        }
    }
    SITES.iter().filter(|s| s.words.iter().any(|w| w.len() >= 5 && similarity(t, w) >= 0.9)).max_by(|a, b| {
        let sa = a.words.iter().map(|w| similarity(t, w)).fold(0.0, f64::max);
        let sb = b.words.iter().map(|w| similarity(t, w)).fold(0.0, f64::max);
        sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
    })
}

const TLDS: &[&str] = &["com", "in", "org", "net", "io", "dev", "co", "ai", "app", "edu", "gov", "me", "info", "tv", "uk", "us", "xyz", "so", "gg"];

pub fn spoken_url(spoken: &str) -> Option<String> {
    let raw = spoken.trim().trim_end_matches(['.', '?', '!']);
    let lower = raw.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(raw.to_owned());
    }
    let t = normalize(raw);
    let mut s = format!(" {t} ");
    for (a, b) in [(" dot ", "."), (" slash ", "/"), (" dash ", "-"), (" hyphen ", "-"), (" underscore ", "_"), (" colon ", ":")] {
        while s.contains(a) {
            s = s.replace(a, b);
        }
    }
    let joined: String = s.split_whitespace().collect::<Vec<_>>().join("");
    let candidate = if raw.contains('.') && !raw.contains(' ') { lower.clone() } else { joined };
    let host = candidate.split('/').next().unwrap_or("");
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) && TLDS.contains(parts.last().unwrap()) {
        return Some(format!("https://{candidate}"));
    }
    None
}

pub fn encode(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 3);
    for b in q.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn search_url(site: Option<&Site>, q: &str) -> String {
    let template = site.and_then(|s| s.search).unwrap_or("https://www.google.com/search?q={q}");
    let enc = if template.contains("/search/{q}") || template.contains("#search/") { encode(q).replace('+', "%20") } else { encode(q) };
    template.replace("{q}", &enc)
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(3))).timeout_global(Some(Duration::from_secs(secs))).build().into()
}

pub fn youtube_video(query: &str) -> Option<(String, String)> {
    let url = format!("https://www.youtube.com/results?search_query={}&sp=EgIQAQ%253D%253D", encode(query));
    let mut resp = agent(5)
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Cookie", "CONSENT=YES+1; SOCS=CAI")
        .call()
        .ok()?;
    let body = resp.body_mut().with_config().limit(6 * 1024 * 1024).read_to_string().ok()?;
    first_video(&body)
}

pub fn first_video(body: &str) -> Option<(String, String)> {
    let key = "\"videoRenderer\":{\"videoId\":\"";
    let start = body.find(key)? + key.len();
    let id: String = body[start..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if id.len() != 11 {
        return None;
    }
    let rest = &body[start..(start + 4000).min(body.len())];
    let title = rest
        .find("\"title\":{\"runs\":[{\"text\":\"")
        .map(|i| {
            let s = &rest[i + 26..];
            s.chars().take_while(|c| *c != '"').collect::<String>()
        })
        .unwrap_or_default();
    Some((id, title))
}

pub fn find_browser(core: &Core, spoken: Option<&str>) -> Option<App> {
    let defaults = core.defaults();
    match spoken.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => {
            let key = browser_key(name);
            let apps = core.apps.all();
            if let Some(k) = key {
                let words = BROWSERS.iter().find(|(b, _)| *b == k).map(|(_, w)| *w).unwrap_or(&[]);
                let mut best: Option<(f64, App)> = None;
                for a in apps.iter().filter(|a| !a.hidden) {
                    let n = normalize(&a.name);
                    let id = a.id.to_lowercase();
                    let mut score = words.iter().map(|w| similarity(w, &n)).fold(0.0, f64::max);
                    if n.split_whitespace().any(|w| w == k) || id.contains(k) {
                        score = score.max(0.93);
                    }
                    if a.roles.iter().any(|r| r == "browser") {
                        score += 0.05;
                    }
                    if n.contains("private") || n.contains("incognito") || n.contains("profile") || n.contains("uninstall") {
                        score -= 0.3;
                    }
                    if score >= 0.85 && best.as_ref().is_none_or(|(b, _)| score > *b) {
                        best = Some((score, a.clone()));
                    }
                }
                if let Some((_, a)) = best {
                    return Some(a);
                }
            }
            core.apps.find(name, None, &defaults).into_iter().map(|(a, _)| a).find(|a| a.roles.iter().any(|r| r == "browser") || key.is_some())
        }
        None => {
            let pref = core.settings.str("web.browser");
            if !pref.is_empty()
                && let Some(a) = core.apps.get(&pref)
            {
                return Some(a);
            }
            core.apps.role_default("browser", &defaults)
        }
    }
}

pub fn open(core: &Core, browser: Option<&App>, url: &str) -> Result<String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(anyhow!("Only web links can be opened."));
    }
    match browser {
        Some(b) => {
            super::shell::open_with(b, &[url.to_owned()])?;
            if core.settings.bool("privacy.learn") {
                core.apps.bump(&b.id);
            }
            Ok(b.name.clone())
        }
        None => {
            super::shell::open(url)?;
            Ok("your browser".into())
        }
    }
}

pub fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").trim_start_matches("www.").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sites_and_urls() {
        assert_eq!(site("youtube").map(|s| s.name), Some("YouTube"));
        assert_eq!(site("you tube").map(|s| s.name), Some("YouTube"));
        assert_eq!(site("netflix").map(|s| s.name), Some("Netflix"));
        assert_eq!(site("my gmail").map(|s| s.name), Some("Gmail"));
        assert_eq!(site("github.com").map(|s| s.name), Some("GitHub"));
        assert!(site("pizza hut").is_none());
        assert_eq!(spoken_url("google dot com").as_deref(), Some("https://google.com"));
        assert_eq!(spoken_url("github.com/anoop").as_deref(), Some("https://github.com/anoop"));
        assert_eq!(spoken_url("example dot co dot in").as_deref(), Some("https://example.co.in"));
        assert_eq!(spoken_url("open the door"), None);
        assert_eq!(browser_key("brave"), Some("brave"));
        assert_eq!(browser_key("google chrome"), Some("chrome"));
        assert_eq!(browser_key("the edge browser"), Some("edge"));
        assert_eq!(browser_key("notepad"), None);
        assert_eq!(search_url(site("youtube"), "arijit singh songs"), "https://www.youtube.com/results?search_query=arijit+singh+songs");
        assert_eq!(host("https://www.youtube.com/watch?v=x"), "youtube.com");
    }

    #[test]
    fn reads_first_video() {
        let body = r#"xx"videoRenderer":{"videoId":"rRsGgIQPqLM","thumbnail":{},"title":{"runs":[{"text":"Best Of Arijit"}]}}"#;
        assert_eq!(first_video(body), Some(("rRsGgIQPqLM".into(), "Best Of Arijit".into())));
        assert_eq!(first_video("nothing"), None);
    }
}
