const MARKERS: &[&str] = &[
    " and after that ",
    " after that ",
    " and then ",
    " then after that ",
    " afterwards ",
    " after which ",
    " once that's done ",
    " once done ",
    " when that's done ",
    " and also ",
    " followed by ",
    " then ",
];

const VERBS: &[&str] = &[
    "open", "close", "search", "play", "pause", "resume", "stop", "start", "put", "move", "snap", "maximize", "maximise", "minimize", "minimise", "create", "make", "write", "type", "add",
    "remind", "set", "take", "send", "show", "go", "switch", "install", "download", "turn", "mute", "unmute", "increase", "decrease", "raise", "lower", "find", "delete", "save", "copy",
    "paste", "scroll", "click", "select", "read", "tell", "call", "message", "text", "email", "run", "launch", "navigate", "visit", "check", "bookmark", "reload", "refresh", "skip",
    "arrange", "split", "restore", "focus", "lock", "connect", "disconnect", "enable", "disable", "reopen", "duplicate", "pin", "zoom", "print",
];

const FILLERS: &[&str] = &[
    "i want you to ", "i want u to ", "i would like you to ", "i'd like you to ", "i need you to ", "can you please ", "can you ", "can u ", "could you please ", "could you ", "would you ",
    "please ", "now ", "alright ", "all right ", "okay ", "ok ", "so ", "and ", "then ", "also ", "just ", "next ",
];

const WHOLE: &[&str] = &["remind me", "take a note", "make a note", "note that", "remember", "type ", "write a ", "write the ", "write me ", "dictate", "add "];
const LISTS: &[&str] = &["to do list", "todo list", "to-do list", "checklist", "check list", "shopping list"];
const QUERIES: &[&str] = &["search for ", "search ", "google ", "look up ", "play ", "find "];

pub fn clean(part: &str) -> String {
    let mut p = part.trim().trim_matches(',').trim().to_owned();
    for _ in 0..4 {
        let before = p.clone();
        for f in FILLERS {
            if let Some(r) = p.strip_prefix(f) {
                p = r.trim().to_owned();
            }
        }
        if p == before {
            break;
        }
    }
    p
}

fn starts_with_verb(p: &str) -> bool {
    p.split_whitespace().next().is_some_and(|w| VERBS.contains(&w))
}

pub fn split(t: &str, whole_known: bool) -> Vec<String> {
    let t = t.trim();
    if t.split_whitespace().count() < 4 {
        return Vec::new();
    }
    let lead = clean(t);
    if WHOLE.iter().any(|w| lead.starts_with(w)) || LISTS.iter().any(|w| t.contains(w)) {
        return Vec::new();
    }
    let query = QUERIES.iter().any(|w| lead.starts_with(w));
    let mut parts: Vec<String> = vec![format!(" {t} ")];
    for m in MARKERS {
        parts = parts
            .into_iter()
            .flat_map(|p| {
                let pieces: Vec<String> = p.split(m).map(|x| format!(" {} ", x.trim())).collect();
                let ok = pieces.len() > 1 && pieces.iter().all(|x| x.split_whitespace().count() >= 1);
                if ok { pieces } else { vec![p] }
            })
            .collect();
    }
    let explicit = parts.len() > 1;
    if !explicit && whole_known {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let words: Vec<&str> = p.split_whitespace().collect();
        let mut cur: Vec<&str> = Vec::new();
        for (k, w) in words.iter().enumerate() {
            let next_verb = words.get(k + 1).is_some_and(|n| VERBS.contains(n));
            if *w == "and" && next_verb && !query && !cur.is_empty() && starts_with_verb(&clean(&cur.join(" "))) && cur.len() >= 2 {
                out.push(cur.join(" "));
                cur.clear();
                continue;
            }
            cur.push(w);
        }
        if !cur.is_empty() {
            out.push(cur.join(" "));
        }
    }
    let steps: Vec<String> = out.iter().map(|p| clean(p)).filter(|p| !p.is_empty()).collect();
    if steps.len() < 2 || steps.iter().any(|s| s.split_whitespace().count() > 24) {
        return Vec::new();
    }
    steps.into_iter().take(5).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_requests_with_several_steps() {
        assert_eq!(
            split("can u open new window in brave and search for mr beast after that i want u to put two brave windows side by side", false),
            vec!["open new window in brave", "search for mr beast", "put two brave windows side by side"]
        );
        assert_eq!(split("open firefox and then play some music", false), vec!["open firefox", "play some music"]);
        assert!(split("open youtube and play arijit singh", true).is_empty());
        assert!(split("search for tom and jerry", false).is_empty());
        assert!(split("turn off wifi and bluetooth", false).is_empty());
        assert!(split("i am determined i will do 2 hour coding after that i will drink water and after that i will do to play make a to do list of these things", false).is_empty());
        assert!(split("remind me to call mom and then buy milk", false).is_empty());
        assert_eq!(split("close spotify then mute the volume", false), vec!["close spotify", "mute the volume"]);
        assert!(split("open firefox", false).is_empty());
        assert_eq!(split("search for mr beast and then open the first video", false), vec!["search for mr beast", "open the first video"]);
    }
}
