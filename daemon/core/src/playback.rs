use crate::core::Core;
use crate::router::Intent;
use crate::tools::media::{self, Action, Player, Status};
use crate::tools::{Choice, FollowUp, Outcome};
use serde_json::{Value, json};
use std::sync::Arc;

const LAYER: &str = "media";

fn act(tool: &str, args: Value) -> Intent {
    Intent::Act { tool: tool.to_owned(), args }
}

const PAUSE_ANY: &[&str] = &[
    "pause", "pause it", "pause this", "pause that", "pause the video", "pause the song", "pause the music", "pause music", "pause playback", "pause the playback",
    "pause the media", "pause media", "pause the audio", "stop the music", "stop the song", "stop the video", "stop playing", "stop playback", "stop the playback",
    "stop the media", "stop media", "stop the audio", "hold it", "pause the stream", "stop the stream", "pause the tab", "pause this tab", "pause the movie", "stop the movie",
];

const PAUSE_ALL: &[&str] = &[
    "pause everything", "pause all", "pause all media", "pause all the media", "stop everything playing", "stop all media", "stop all the media", "pause all tabs",
    "pause all the tabs", "stop all music", "pause everything that's playing", "stop everything that's playing", "stop all the music", "pause all music", "stop all audio",
    "pause all audio", "stop all", "pause both", "stop both",
];

const RESUME_ANY: &[&str] = &[
    "play", "resume", "resume it", "resume playing", "resume playback", "resume the video", "resume the song", "resume the music", "resume music", "continue playing",
    "unpause", "unpause it", "play it", "play it again", "play again", "keep playing", "continue the video", "continue the song", "continue the music", "start playing again",
    "resume the media", "play the music", "play the video", "resume the stream", "resume the movie", "play the song",
];

const RESUME_ALL: &[&str] = &["resume everything", "resume all", "play everything", "resume all media", "unpause everything", "resume both", "play both"];

const LIST: &[&str] = &[
    "what's playing", "what is playing", "what's playing right now", "what is playing right now", "what is being played", "what's being played", "what song is this",
    "what song is playing", "which song is this", "which song is playing", "what am i listening to", "what media is playing", "is anything playing", "is something playing",
    "what's playing now", "what is playing now", "what video is this", "what's on", "what is on right now", "which media is playing", "what's this song", "name this song",
];

const NEXT: &[&str] = &["next song", "next track", "skip song", "skip this song", "skip the song", "next video", "play the next song", "play next", "skip", "skip this", "skip track", "skip it", "next one", "play the next one", "next episode"];

const PREV: &[&str] = &["previous song", "previous track", "last song", "play the previous song", "previous video", "go back a song", "play the last song", "previous one", "the previous song"];

const MEDIAISH: &[&str] = &[
    "youtube", "you tube", "youtube music", "spotify", "twitch", "netflix", "prime video", "hotstar", "jiohotstar", "soundcloud", "jiosaavn", "saavn", "gaana", "wynk",
    "vlc", "rhythmbox", "apple music", "music", "itunes", "vimeo", "the music", "the song", "the video", "the movie", "the stream", "media", "the tab", "the tabs",
    "audio", "the audio", "podcast", "the podcast", "audible", "deezer", "tidal", "amazon music", "mx player", "jiocinema", "zee5", "sonyliv", "kick", "tiktok", "instagram",
];

fn ordinal_word(w: &str) -> Option<usize> {
    let words = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth"];
    if let Some(i) = words.iter().position(|x| *x == w) {
        return Some(i + 1);
    }
    let digits: String = w.chars().take_while(char::is_ascii_digit).collect();
    let rest = &w[digits.len()..];
    if !digits.is_empty() && matches!(rest, "st" | "nd" | "rd" | "th") {
        return digits.parse().ok();
    }
    None
}

pub struct Target {
    pub text: String,
    pub all: bool,
    pub n: Option<usize>,
    pub browser: Option<String>,
}

pub fn target(raw: &str) -> Target {
    let mut t = raw.trim().to_owned();
    let mut all = false;
    let mut n = None;
    let mut browser = None;
    for sep in [" in ", " on "] {
        if let Some((a, b)) = t.rsplit_once(sep)
            && let Some(k) = crate::tools::web::browser_key(b)
        {
            browser = Some(k.to_owned());
            t = a.trim().to_owned();
        }
    }
    if let Some((a, b)) = t.split_once(" on ")
        && !b.is_empty()
        && matches!(a, "the song" | "the music" | "the video" | "the track" | "the podcast" | "the stream" | "the movie" | "music" | "it" | "that" | "this")
    {
        t = b.to_owned();
    }
    loop {
        let before = t.clone();
        for p in ["all of the ", "all the ", "all ", "every ", "both ", "the ", "my ", "that ", "this "] {
            if let Some(r) = t.strip_prefix(p) {
                if matches!(p, "all of the " | "all the " | "all " | "every " | "both ") {
                    all = true;
                }
                t = r.to_owned();
            }
        }
        if let Some((w, r)) = t.split_once(' ')
            && let Some(k) = ordinal_word(w)
        {
            n = Some(k);
            t = r.to_owned();
        }
        for s in [" tabs", " tab", " videos", " video", " songs", " song", " music", " player", " app", " one", " ones", " streams", " stream", " too", " as well", " also", " website", " site"] {
            if let Some(r) = t.strip_suffix(s)
                && !r.is_empty()
            {
                if matches!(s, " tabs" | " videos" | " songs" | " ones" | " streams") && n.is_none() {
                    all = true;
                }
                t = r.to_owned();
            }
        }
        if t == before {
            break;
        }
    }
    Target { text: t.trim().to_owned(), all, n, browser }
}

fn generic(t: &str) -> bool {
    matches!(t, "" | "it" | "this" | "that" | "video" | "song" | "music" | "media" | "audio" | "playback" | "stream" | "movie" | "tab" | "tabs" | "everything" | "playing" | "sound")
}

fn targeted(tool: &str, rest: &str, verb: &str) -> Option<Intent> {
    let tg = target(rest);
    if generic(&tg.text) && tg.browser.is_none() {
        return Some(act(tool, json!({ "verb": verb, "all": tg.all || matches!(tg.text.as_str(), "everything") })));
    }
    Some(act(tool, json!({ "verb": verb, "target": if generic(&tg.text) { Value::Null } else { json!(tg.text) }, "all": tg.all, "n": tg.n, "browser": tg.browser })))
}

fn mediaish(rest: &str) -> bool {
    let tg = target(rest);
    let t = tg.text.as_str();
    tg.browser.is_some() && generic(t) || MEDIAISH.contains(&t) || MEDIAISH.contains(&rest) || media::known_source(t) || crate::tools::web::site(t).is_some_and(|s| matches!(s.name, "YouTube" | "YouTube Music" | "Spotify" | "Netflix" | "Prime Video" | "JioHotstar"))
}

fn other_song(t: &str) -> bool {
    let asks = ["another song", "a different song", "different song", "something else", "another track", "a different track", "change the song", "change this song", "change song", "change the track", "another video", "a different video", "another one"];
    let dislikes = ["don't like this song", "dont like this song", "do not like this song", "don't like this one", "not this song", "hate this song", "this song is boring", "i'm bored of this song"];
    (asks.iter().any(|a| t.contains(a)) && !t.contains(" by ") && !t.contains(" from ") && !t.starts_with("search")) || dislikes.iter().any(|d| t.contains(d))
}

const SYSTEM_SOUND: &[&str] = &["", "the", "my", "your", "system", "the system", "master", "the master", "overall", "the overall", "the sound", "sound", "the volume", "volume", "audio", "the audio", "everything", "all", "my computer", "the computer", "the pc", "my pc", "the laptop", "my laptop", "speakers", "the speakers", "the mic", "my mic", "the microphone", "my microphone", "microphone", "mic", "yourself", "setu", "you", "notifications", "the notifications", "it", "this"];

fn seconds(t: &str) -> Option<i64> {
    let n = crate::text::find_number(t).unwrap_or(if t.contains("a minute") { 1 } else { 10 });
    if t.contains("minute") { Some(n * 60) } else if t.contains("second") || t.contains("sec") { Some(n) } else { None }
}

fn sound_rules(t: &str) -> Option<Intent> {
    if matches!(t, "show the lyrics" | "show lyrics" | "show me the lyrics" | "turn on lyrics" | "turn the lyrics on" | "lyrics on" | "show the words of this song") {
        return Some(act("media.lyrics", json!({ "on": true })));
    }
    if matches!(t, "hide the lyrics" | "hide lyrics" | "turn off lyrics" | "turn the lyrics off" | "lyrics off" | "no lyrics") {
        return Some(act("media.lyrics", json!({ "on": false })));
    }
    if matches!(t, "what is making sound" | "what's making sound" | "whats making sound" | "what is making noise" | "what's making noise" | "what apps are playing" | "which apps are playing sound" | "what is playing sound" | "show audio" | "show me what's playing" | "what all is playing") {
        return Some(act("media.list", json!({ "all": true })));
    }
    for (p, sign) in [("skip forward ", 1i64), ("skip ahead ", 1), ("go forward ", 1), ("forward ", 1), ("fast forward ", 1), ("jump ahead ", 1), ("skip back ", -1), ("go back ", -1), ("rewind ", -1), ("jump back ", -1)] {
        if let Some(rest) = t.strip_prefix(p)
            && let Some(n) = seconds(rest)
        {
            return Some(act("media.seek", json!({ "seconds": sign * n })));
        }
    }
    if matches!(t, "rewind" | "rewind a bit" | "go back a bit in the song" | "go back a bit in the video") {
        return Some(act("media.seek", json!({ "seconds": -10 })));
    }
    let app_word = |x: &str| {
        let x = x.trim().trim_start_matches("the ");
        (!SYSTEM_SOUND.contains(&x) && !SYSTEM_SOUND.contains(&format!("the {x}").as_str()) && x.split_whitespace().count() <= 4).then(|| x.to_owned())
    };
    for (p, s, dir) in [("turn ", " down", "down"), ("turn ", " up", "up"), ("make ", " quieter", "down"), ("make ", " louder", "up")] {
        if let Some(mid) = t.strip_prefix(p).and_then(|r| r.strip_suffix(s))
            && let Some(app) = app_word(mid)
        {
            return Some(act("media.volume", json!({ "target": app, "dir": dir })));
        }
    }
    for (p, dir) in [("turn down ", "down"), ("turn up ", "up")] {
        if let Some(rest) = t.strip_prefix(p)
            && let Some(app) = app_word(rest)
        {
            return Some(act("media.volume", json!({ "target": app, "dir": dir })));
        }
    }
    for p in ["set the volume of ", "set volume of "] {
        if let Some(rest) = t.strip_prefix(p)
            && let Some((app, n)) = rest.rsplit_once(" to ")
            && let Some(level) = crate::text::find_number(n)
            && let Some(app) = app_word(app)
        {
            return Some(act("media.volume", json!({ "target": app, "level": level })));
        }
    }
    if let Some(rest) = t.strip_prefix("set ")
        && let Some((app, n)) = rest.split_once(" volume to ")
        && let Some(level) = crate::text::find_number(n)
        && let Some(app) = app_word(app)
    {
        return Some(act("media.volume", json!({ "target": app, "level": level })));
    }
    None
}

pub fn app_volume(t: &str) -> bool {
    matches!(sound_rules(t), Some(Intent::Act { ref tool, .. }) if tool == "media.volume")
}

pub fn rules(t: &str) -> Option<Intent> {
    if let Some(i) = sound_rules(t) {
        return Some(i);
    }
    for p in ["pause everything except ", "pause everything but ", "pause all except ", "pause all but ", "stop everything except ", "pause all the sound except ", "pause everything apart from ", "pause everything other than "] {
        if let Some(rest) = t.strip_prefix(p) {
            let keep = rest.trim().trim_start_matches("the ").trim();
            if !keep.is_empty() {
                let verb = if t.starts_with("stop") { "stop" } else { "pause" };
                return Some(act("media.pause", json!({ "verb": verb, "all": true, "except": keep })));
            }
        }
    }
    if PAUSE_ANY.contains(&t) {
        let verb = if t.starts_with("stop") { "stop" } else { "pause" };
        return Some(act("media.pause", json!({ "verb": verb })));
    }
    if matches!(t, "pause this tab" | "pause the current tab" | "stop this tab" | "pause this video" | "pause the video in this tab" | "stop this video" | "pause this page") {
        let verb = if t.starts_with("stop") { "stop" } else { "pause" };
        return Some(act("media.pause", json!({ "verb": verb, "active": true })));
    }
    if PAUSE_ALL.contains(&t) {
        let verb = if t.starts_with("stop") { "stop" } else { "pause" };
        return Some(act("media.pause", json!({ "verb": verb, "all": true })));
    }
    if RESUME_ANY.contains(&t) {
        return Some(act("media.play", json!({ "verb": "play" })));
    }
    if RESUME_ALL.contains(&t) {
        return Some(act("media.play", json!({ "verb": "play", "all": true })));
    }
    if LIST.contains(&t) || t.starts_with("what's playing in ") || t.starts_with("what is playing in ") || t.starts_with("what's playing on ") || t.starts_with("what is playing on ") {
        return Some(act("media.list", json!({})));
    }
    if NEXT.contains(&t) || other_song(t) {
        return Some(act("media.next", json!({})));
    }
    if PREV.contains(&t) {
        return Some(act("media.previous", json!({})));
    }
    if let Some(rest) = t.strip_prefix("pause ") {
        return targeted("media.pause", rest, "pause");
    }
    if let Some(rest) = t.strip_prefix("stop ").filter(|r| mediaish(r)) {
        return targeted("media.pause", rest, "stop");
    }
    for p in ["resume ", "unpause ", "continue "] {
        if let Some(rest) = t.strip_prefix(p) {
            return targeted("media.play", rest, "play");
        }
    }
    if let Some(rest) = t.strip_prefix("play ").filter(|r| matches!(*r, "it again" | "the paused video" | "the paused song" | "what i paused" | "what you paused")) {
        return targeted("media.play", rest, "play");
    }
    for p in ["skip ", "next song on ", "next track on ", "next video on "] {
        if let Some(rest) = t.strip_prefix(p).filter(|r| mediaish(r)) {
            let tg = target(rest.trim_start_matches("the song on ").trim_start_matches("to the next song on "));
            return Some(act("media.next", json!({ "target": tg.text })));
        }
    }
    for p in ["mute ", "unmute "] {
        if let Some(rest) = t.strip_prefix(p) {
            let tool = if p == "mute " { "media.mute" } else { "media.unmute" };
            if matches!(rest, "this tab" | "the tab" | "tab" | "the current tab" | "current tab" | "this page") {
                return Some(act(tool, json!({ "active": true })));
            }
            if matches!(rest, "all tabs" | "all the tabs" | "every tab" | "the other tabs" | "other tabs" | "all other tabs") {
                return Some(act(tool, json!({ "all": true, "others": rest.contains("other") })));
            }
            let app = rest.trim_start_matches("the ");
            if !(mediaish(rest) || rest.ends_with(" tab") || rest.ends_with(" tabs")) && !SYSTEM_SOUND.contains(&rest) && !SYSTEM_SOUND.contains(&app) && app.split_whitespace().count() <= 4 {
                return Some(act(tool, json!({ "target": app, "app": true })));
            }
            if mediaish(rest) || rest.ends_with(" tab") || rest.ends_with(" tabs") {
                let tg = target(rest);
                if !generic(&tg.text) {
                    return Some(act(tool, json!({ "target": tg.text, "browser": tg.browser })));
                }
            }
        }
    }
    None
}

fn s(args: &Value, k: &str) -> String {
    args.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned()
}

fn keys(args: &Value) -> Option<Vec<String>> {
    args.get("keys").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
}

fn narrow(mut cands: Vec<Player>, args: &Value) -> Result<Vec<Player>, String> {
    if let Some(k) = keys(args) {
        cands.retain(|p| k.contains(&p.key));
        return Ok(cands);
    }
    if args.get("active").and_then(Value::as_bool).unwrap_or(false) {
        let mine: Vec<Player> = cands.iter().filter(|p| p.active).cloned().collect();
        return if mine.is_empty() { Err("this tab".into()) } else { Ok(mine) };
    }
    if s(args, "target").is_empty() && cands.iter().any(|p| !p.muted) {
        cands.retain(|p| !p.muted);
    }
    let browser = s(args, "browser");
    if !browser.is_empty() {
        cands.retain(|p| p.browser.as_deref() == Some(browser.as_str()));
    }
    let target = s(args, "target");
    if !target.is_empty() {
        let picked: Vec<Player> = media::select(&cands, &target).into_iter().cloned().collect();
        if picked.is_empty() {
            return Err(target);
        }
        cands = picked;
    }
    if let Some(n) = args.get("n").and_then(Value::as_u64).filter(|n| *n >= 1) {
        return Ok(cands.into_iter().nth(n as usize - 1).into_iter().collect());
    }
    Ok(cands)
}

fn names(ps: &[Player]) -> String {
    let gs = media::groups(ps);
    media::join_and(&gs.iter().map(|g| if g.players.len() > 1 { format!("{} {} tabs", media::count_word(g.players.len()), g.source) } else { g.source.clone() }).collect::<Vec<_>>())
}

fn ask_which(tool: &str, verb: &str, cands: &[Player]) -> FollowUp {
    let gs = media::groups(cands);
    let n = gs.len();
    let descs: Vec<String> = gs.iter().map(media::Group::describe).collect();
    let what = if verb == "play" { "paused" } else { "playing" };
    let question = if n == 2 {
        format!("There are two media {what}, one is {} and the other is {}. Which one should I {verb}?", descs[0], descs[1])
    } else {
        format!("There are {} media {what}: {}. Which one should I {verb}?", media::count_word(n), media::join_and(&descs))
    };
    let mut choices: Vec<Choice> = gs
        .iter()
        .map(|g| {
            let mut words: Vec<String> = vec![g.source.to_lowercase()];
            if g.players.iter().all(|p| p.app == g.players[0].app) && g.players[0].app != g.source && !g.players[0].is_tab() {
                words.push(g.players[0].app.to_lowercase());
            }
            if let Some(site) = crate::tools::web::SITES.iter().find(|s| s.name == g.source) {
                words.extend(site.words.iter().map(|w| (*w).to_owned()));
            }
            Choice { label: g.source.clone(), words, then: act(tool, json!({ "verb": verb, "keys": g.players.iter().map(|p| p.key.clone()).collect::<Vec<_>>() })), icon: String::new() }
        })
        .collect();
    let all: Vec<String> = cands.iter().map(|p| p.key.clone()).collect();
    choices.push(Choice::new(if n == 2 { "Both" } else { "All of them" }, &["all", "both", "everything", "all of them", "both of them", "every one", "all media"], act(tool, json!({ "verb": verb, "keys": all, "all": true }))));
    FollowUp { question: question.clone(), spoken: Some(question), choices, cleanup: None, kind: "", focus: None }
}

fn ask_all_or_one(tool: &str, verb: &str, group: &[Player]) -> FollowUp {
    let source = group[0].source.clone();
    let what = if verb == "play" { "paused" } else { "playing" };
    let noun = if group.iter().all(Player::is_tab) { "tabs" } else { "players" };
    let question = format!("There are {} {source} {noun} {what}. Should I {verb} all of them, or a specific one?", media::count_word(group.len()));
    let keys: Vec<String> = group.iter().map(|p| p.key.clone()).collect();
    let mut choices = vec![
        Choice::new("All of them", &["all", "both", "every", "everything", "all of them", "all tabs", "both of them", "all of it"], act(tool, json!({ "verb": verb, "keys": keys, "all": true }))),
        Choice::new("A specific one", &["specific", "a specific one", "particular", "one of them", "just one", "only one", "single", "a particular one", "specific one", "let me choose", "let me pick"], act("media.pick", json!({ "tool": tool, "verb": verb, "keys": keys }))),
    ];
    for p in group {
        let t = crate::text::normalize(&p.short_title());
        let words: Vec<String> = if t.split_whitespace().count() >= 2 { vec![t.clone()] } else { Vec::new() };
        choices.push(Choice { label: p.short_title(), words, then: act(tool, json!({ "verb": verb, "keys": [p.key.clone()] })), icon: String::new() });
    }
    FollowUp { question: question.clone(), spoken: Some(question), choices, cleanup: None, kind: "", focus: None }
}

fn ask_pick(tool: &str, verb: &str, group: &[Player]) -> FollowUp {
    let choices: Vec<Choice> = group
        .iter()
        .map(|p| {
            let t = crate::text::normalize(&p.short_title());
            let mut words: Vec<String> = Vec::new();
            if t.split_whitespace().count() >= 1 {
                words.push(t.clone());
            }
            for w in t.split_whitespace().filter(|w| w.len() >= 5) {
                if group.iter().filter(|o| crate::text::normalize(&o.title).split_whitespace().any(|x| x == w)).count() == 1 {
                    words.push(w.to_owned());
                }
            }
            Choice { label: p.short_title(), words, then: act(tool, json!({ "verb": verb, "keys": [p.key.clone()] })), icon: String::new() }
        })
        .collect();
    FollowUp { question: format!("Which one should I {verb}?"), spoken: None, choices, cleanup: None, kind: "", focus: None }
}

async fn fallback_key(action: Action) -> Outcome {
    let key = match action {
        Action::Next => crate::tools::input::K::NextTrack,
        Action::Previous => crate::tools::input::K::PrevTrack,
        _ => crate::tools::input::K::PlayPause,
    };
    let label = match action {
        Action::Pause => "Paused",
        Action::Play => "Playing",
        Action::Next => "Next track",
        _ => "Previous track",
    };
    match crate::tools::input::press(&[key]).await {
        Ok(()) => Outcome::ok_with(label, "keyboard", None),
        Err(e) => Outcome::fail(&e.to_string(), "keyboard"),
    }
}

fn past(verb: &str) -> &'static str {
    match verb {
        "stop" => "Stopped",
        "play" => "Resumed",
        _ => "Paused",
    }
}

async fn pause_tab(target: &str) -> Option<Outcome> {
    let c = crate::proto::any()?;
    let tabs = c.tabs().await.ok()?;
    let hits: Vec<crate::cdp::Tab> = crate::cdp::find_tab(&tabs, target).into_iter().cloned().collect();
    if hits.is_empty() {
        return None;
    }
    let mut paused = Vec::new();
    for t in &hits {
        if c.pause_media(&t.id).await.unwrap_or(0) > 0 {
            paused.push(t.title.clone());
        }
    }
    Some(if paused.is_empty() {
        Outcome::ok_with("Nothing to pause", LAYER, None).with_reply(format!("{} isn't playing anything.", hits[0].title))
    } else {
        Outcome::ok_with(&format!("Paused {}", media::join_and(&paused)), LAYER, None)
    })
}

async fn pause(args: &Value) -> Outcome {
    let verb = if s(args, "verb") == "stop" { "stop" } else { "pause" };
    if !s(args, "target").is_empty()
        && let Some(o) = pause_tab(&s(args, "target")).await
    {
        return o;
    }
    let listing = media::list().await;
    let playing: Vec<Player> = listing.players.iter().filter(|p| p.status == Status::Playing).cloned().collect();
    if !listing.known {
        return fallback_key(Action::Pause).await;
    }
    let explicit = keys(args).is_some();
    let except = s(args, "except");
    let playing: Vec<Player> = if except.is_empty() {
        playing
    } else {
        let keep: Vec<String> = media::select(&playing, &except).iter().map(|p| p.key.clone()).collect();
        if keep.is_empty() {
            return Outcome::ok_with("Nothing to keep", LAYER, None).with_reply(format!("I don't see {except} playing, so nothing was paused."));
        }
        playing.into_iter().filter(|p| !keep.contains(&p.key)).collect()
    };
    let cands = match narrow(playing, args) {
        Ok(c) => c,
        Err(target) => {
            let any = media::select(&listing.players, &target);
            return match any.first() {
                Some(p) => Outcome::ok_with(&format!("{} isn't playing", p.source), LAYER, None).with_reply(format!("{} isn't playing right now.", p.source)),
                None => Outcome::ok_with("Nothing to pause", LAYER, None).with_reply(format!("I don't see {target} playing.")),
            };
        }
    };
    if cands.is_empty() {
        let line = if explicit { "That has already stopped." } else { "Nothing is playing right now." };
        return Outcome::ok_with("Nothing to pause", LAYER, None).with_reply(line.to_owned());
    }
    let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    let gs = media::groups(&cands);
    if cands.len() > 1 && !all {
        let f = if gs.len() == 1 { ask_all_or_one("media.pause", verb, &cands) } else { ask_which("media.pause", verb, &cands) };
        return Outcome::asking("Asked which media to pause", LAYER, f);
    }
    let n = media::control(&cands, Action::Pause).await;
    if n == 0 {
        return Outcome::fail("That player didn't respond.", LAYER);
    }
    let paused: Vec<String> = cands.iter().map(|p| p.key.clone()).collect();
    media::remember_paused(&paused);
    let (muted, played): (Vec<Player>, Vec<Player>) = cands.iter().cloned().partition(|p| p.key.starts_with("stream:"));
    let line = match (played.is_empty(), muted.is_empty()) {
        (true, _) => format!("Muted {}", names(&muted)),
        (_, true) => format!("{} {}", past(verb), names(&played)),
        _ => format!("{} {} and muted {}", past(verb), names(&played), names(&muted)),
    };
    Outcome::ok_with(&line, LAYER, cands.first().map(|p| p.app.clone()))
}

async fn resume(args: &Value) -> Outcome {
    let listing = media::list().await;
    if !listing.known {
        return fallback_key(Action::Play).await;
    }
    let paused: Vec<Player> = listing.players.iter().filter(|p| p.status == Status::Paused).cloned().collect();
    let has_filter = keys(args).is_some() || !s(args, "target").is_empty() || !s(args, "browser").is_empty();
    let mut cands = match narrow(paused.clone(), args) {
        Ok(c) => c,
        Err(target) => {
            let any = media::select(&listing.players, &target);
            return match any.first() {
                Some(p) if p.status == Status::Playing => Outcome::ok_with(&format!("{} is already playing", p.source), LAYER, None).with_reply(format!("{} is already playing.", p.source)),
                _ => Outcome::ok_with("Nothing to resume", LAYER, None).with_reply(format!("I don't see {target} paused.")),
            };
        }
    };
    let mut all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    if !has_filter {
        let mine = media::recently_paused();
        let ours: Vec<Player> = cands.iter().filter(|p| mine.contains(&p.key)).cloned().collect();
        if !ours.is_empty() {
            cands = ours;
            all = true;
        }
    }
    if cands.is_empty() {
        if listing.players.iter().any(|p| p.status == Status::Playing) && !has_filter {
            return Outcome::ok_with("Already playing", LAYER, None).with_reply("It's already playing.".into());
        }
        return Outcome::ok_with("Nothing to resume", LAYER, None).with_reply("Nothing is paused right now.".into());
    }
    if cands.len() > 1 && !all {
        let gs = media::groups(&cands);
        let f = if gs.len() == 1 { ask_all_or_one("media.play", "play", &cands) } else { ask_which("media.play", "play", &cands) };
        return Outcome::asking("Asked which media to resume", LAYER, f);
    }
    let n = media::control(&cands, Action::Play).await;
    if n == 0 {
        return Outcome::fail("That player didn't respond.", LAYER);
    }
    media::forget_paused(&cands.iter().map(|p| p.key.clone()).collect::<Vec<_>>());
    let word = if cands.iter().all(|p| p.key.starts_with("stream:")) { "Unmuted" } else { "Resumed" };
    Outcome::ok_with(&format!("{word} {}", names(&cands)), LAYER, cands.first().map(|p| p.app.clone()))
}

async fn skip(args: &Value, action: Action) -> Outcome {
    let listing = media::list().await;
    if !listing.known {
        return fallback_key(action).await;
    }
    let playing: Vec<Player> = listing.players.iter().filter(|p| p.status == Status::Playing).cloned().collect();
    let pool = if playing.is_empty() && (keys(args).is_some() || !s(args, "target").is_empty()) { listing.players.clone() } else { playing };
    let cands = match narrow(pool, args) {
        Ok(c) => c,
        Err(target) => return Outcome::ok_with("Nothing to skip", LAYER, None).with_reply(format!("I don't see {target} playing.")),
    };
    if cands.is_empty() {
        return Outcome::ok_with("Nothing to skip", LAYER, None).with_reply("Nothing is playing right now.".into());
    }
    let tool = if action == Action::Next { "media.next" } else { "media.previous" };
    if cands.len() > 1 {
        let gs = media::groups(&cands);
        let verb = if action == Action::Next { "skip" } else { "go back on" };
        let f = if gs.len() == 1 { ask_pick(tool, verb, &cands) } else { ask_which(tool, verb, &cands) };
        let mut f = f;
        f.choices.retain(|c| c.label != "Both" && c.label != "All of them");
        return Outcome::asking("Asked which player", LAYER, f);
    }
    let n = media::control(&cands, action).await;
    if n == 0 {
        let p = &cands[0];
        if p.is_tab() {
            return Outcome::fail(&format!("{} doesn't have a next button I can press.", p.source), LAYER);
        }
        return Outcome::fail("That player didn't respond.", LAYER);
    }
    Outcome::ok_with(if action == Action::Next { "Next track" } else { "Previous track" }, LAYER, cands.first().map(|p| p.app.clone()))
}

fn describe(p: &Player) -> String {
    let what = if p.title.is_empty() {
        String::new()
    } else if p.artist.is_empty() || p.is_tab() {
        format!(" {}", p.short_title())
    } else {
        format!(" {} by {}", p.short_title(), p.artist)
    };
    let place = if p.is_tab() { format!(" in {}", p.app) } else { String::new() };
    format!("{}{place} is {}{what}", p.source, if p.status == Status::Playing { "playing" } else { "paused on" })
}

async fn list() -> Outcome {
    let listing = media::list().await;
    if !listing.known {
        return Outcome::ok_with("Checked media", LAYER, None).with_reply("I can't see which media is playing on this computer.".into());
    }
    let playing: Vec<&Player> = listing.players.iter().filter(|p| p.status == Status::Playing).collect();
    let paused: Vec<&Player> = listing.players.iter().filter(|p| p.status == Status::Paused).collect();
    let line = if !playing.is_empty() {
        let parts: Vec<String> = playing.iter().take(4).map(|p| describe(p)).collect();
        let more = if playing.len() > 4 { format!(" and {} more", playing.len() - 4) } else { String::new() };
        format!("{}{more}.", crate::text::sentence_case(&media::join_and(&parts)))
    } else if !paused.is_empty() {
        let gs: Vec<String> = media::groups(&paused.iter().map(|p| (*p).clone()).collect::<Vec<_>>()).iter().map(media::Group::describe).collect();
        format!("Nothing is playing. {} {} paused.", crate::text::sentence_case(&media::join_and(&gs)), if gs.len() == 1 && paused.len() == 1 { "is" } else { "are" })
    } else {
        "Nothing is playing right now.".into()
    };
    crate::nowplaying::open_list();
    Outcome::ok_with("Checked what's playing", LAYER, None).with_reply(line)
}

async fn pick(args: &Value, core: &Arc<Core>) -> Outcome {
    let tool = s(args, "tool");
    let tool = if tool.is_empty() { "media.pause".to_owned() } else { tool };
    let verb = s(args, "verb");
    let listing = media::list().await;
    let k = keys(args).unwrap_or_default();
    let group: Vec<Player> = listing.players.into_iter().filter(|p| k.contains(&p.key)).collect();
    match group.len() {
        0 => Outcome::ok_with("Nothing left", LAYER, None).with_reply("Those have already stopped.".into()),
        1 => Box::pin(run(core, &tool, json!({ "verb": verb, "keys": [group[0].key.clone()] }))).await,
        _ => Outcome::asking("Asked which tab", LAYER, ask_pick(&tool, if verb.is_empty() { "pause" } else { &verb }, &group)),
    }
}

fn stream_matches(st: &crate::tools::streams::Stream, target: &str) -> bool {
    let t = crate::text::normalize(target);
    if t.is_empty() {
        return false;
    }
    let a = crate::text::normalize(&st.app);
    let b = crate::text::normalize(&st.binary);
    let title = crate::text::normalize(&st.title);
    a == t || b == t || st.role == t || crate::text::similarity(&a, &t) >= 0.8 || (t.len() >= 3 && (a.contains(&t) || b.contains(&t))) || (t.len() >= 4 && title.contains(&t))
}

async fn streams_named(target: &str) -> Vec<crate::tools::streams::Stream> {
    crate::tools::streams::list().await.unwrap_or_default().into_iter().filter(|st| stream_matches(st, target)).collect()
}

async fn mute(args: &Value, on: bool) -> Outcome {
    let target = if args.get("active").and_then(Value::as_bool).unwrap_or(false) { String::new() } else { s(args, "target") };
    let site = crate::tools::web::site(&target).is_some() || !s(args, "browser").is_empty() || target.ends_with(" tab");
    if !target.is_empty() && !site {
        let hits = streams_named(&target).await;
        if !hits.is_empty() {
            let mut n = 0;
            for st in &hits {
                if crate::tools::streams::set_mute(st.index, on).await.is_ok() {
                    n += 1;
                }
            }
            let name = if hits[0].app.is_empty() { target.clone() } else { hits[0].app.clone() };
            return if n > 0 { Outcome::ok_with(&format!("{} {name}", if on { "Muted" } else { "Unmuted" }), LAYER, Some(name.clone())) } else { Outcome::fail(&format!("{name} couldn't be {}.", if on { "muted" } else { "unmuted" }), LAYER) };
        }
        if args.get("app").and_then(Value::as_bool).unwrap_or(false) {
            return Outcome::ok_with("Nothing to mute", LAYER, None).with_reply(format!("I don't hear {target} playing."));
        }
    }
    match crate::browserui::mute(&target, on).await {
        Some(o) => o,
        None => Outcome::fail("I don't see a browser window.", LAYER),
    }
}

async fn volume(args: &Value) -> Outcome {
    let target = s(args, "target");
    let hits = streams_named(&target).await;
    let Some(first) = hits.first() else {
        return Outcome::ok_with("Nothing to change", LAYER, None).with_reply(format!("I don't hear {target} playing."));
    };
    let level = match (args.get("level").and_then(Value::as_f64), s(args, "dir").as_str()) {
        (Some(l), _) => (l / 100.0).clamp(0.0, 1.5),
        (None, "up") => (first.volume + 0.2).min(1.0_f64.max(first.volume)),
        (None, _) => (first.volume - 0.2).max(0.05),
    };
    let mut n = 0;
    for st in &hits {
        if crate::tools::streams::set_volume(st, level).await.is_ok() {
            n += 1;
        }
    }
    let name = if first.app.is_empty() { target.clone() } else { first.app.clone() };
    if n == 0 {
        return Outcome::fail(&format!("{name}'s volume couldn't be changed."), LAYER);
    }
    Outcome::ok_with(&format!("{name} is at {} percent", (level * 100.0).round() as i64), LAYER, Some(name.clone()))
}

async fn seek(args: &Value) -> Outcome {
    let secs = args.get("seconds").and_then(Value::as_i64).unwrap_or(10);
    let listing = media::list().await;
    let Some(p) = listing.players.iter().find(|p| p.status == Status::Playing && p.key.starts_with("os:")).or_else(|| listing.players.iter().find(|p| p.key.starts_with("os:"))) else {
        return Outcome::ok_with("Nothing to skip in", LAYER, None).with_reply("Nothing is playing right now.".into());
    };
    if media::seek(&p.key, secs).await {
        let what = if secs >= 0 { "forward" } else { "back" };
        Outcome::ok_with(&format!("Skipped {what} {} seconds in {}", secs.abs(), p.source), LAYER, Some(p.app.clone()))
    } else {
        Outcome::fail(&format!("{} can't skip like that.", p.source), LAYER)
    }
}

pub async fn run(core: &Arc<Core>, tool: &str, args: Value) -> Outcome {
    match tool {
        "media.volume" => volume(&args).await,
        "media.seek" => seek(&args).await,
        "media.pause" => pause(&args).await,
        "media.play" => resume(&args).await,
        "media.next" => skip(&args, Action::Next).await,
        "media.previous" => skip(&args, Action::Previous).await,
        "media.list" => {
            crate::tools::streams::settle().await;
            list().await
        }
        "media.lyrics" => {
            let on = args.get("on").and_then(Value::as_bool).unwrap_or(true);
            core.settings.set_internal("media.lyrics", json!(on));
            let mut m = serde_json::Map::new();
            m.insert("media.lyrics".into(), json!(on));
            core.db.save_settings(m.clone());
            core.hub.emit("settings.changed", json!({ "changes": m }));
            if on {
                crate::nowplaying::open_list();
                Outcome::ok_with("Lyrics on", LAYER, None).with_reply("Lyrics are on. They show in the notch while a song that has them is playing.".into())
            } else {
                Outcome::ok_with("Lyrics off", LAYER, None).with_reply("Lyrics are off.".into())
            }
        }
        "media.pick" => pick(&args, core).await,
        "media.mute" => mute(&args, true).await,
        "media.unmute" => mute(&args, false).await,
        _ => Outcome::fail("I don't know that media command.", LAYER),
    }
}

pub fn label(tool: &str, args: &Value) -> String {
    let t = s(args, "target");
    match tool {
        "media.pause" => if t.is_empty() { "Pausing".into() } else { format!("Pausing {t}") },
        "media.play" => if t.is_empty() { "Resuming".into() } else { format!("Resuming {t}") },
        "media.next" => "Next track".into(),
        "media.previous" => "Previous track".into(),
        "media.list" => "Checking what's playing".into(),
        "media.lyrics" => if args.get("on").and_then(Value::as_bool) == Some(false) { "Hiding the lyrics".into() } else { "Showing the lyrics".into() },
        "media.volume" => format!("Changing {t}'s volume"),
        "media.seek" => "Skipping".into(),
        "media.pick" => "Choosing".into(),
        "media.mute" => if t.is_empty() { "Muting the tab".into() } else { format!("Muting {t}") },
        "media.unmute" => if t.is_empty() { "Unmuting the tab".into() } else { format!("Unmuting {t}") },
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(i: Option<Intent>) -> (String, Value) {
        match i {
            Some(Intent::Act { tool, args }) => (tool, args),
            other => panic!("not a skill: {other:?}"),
        }
    }

    #[test]
    fn pause_phrases() {
        assert_eq!(tool(rules("pause")).0, "media.pause");
        assert_eq!(tool(rules("stop the music")).1["verb"], "stop");
        assert_eq!(tool(rules("pause everything")).1["all"], true);
        let (t, a) = tool(rules("pause youtube"));
        assert_eq!((t.as_str(), a["target"].as_str()), ("media.pause", Some("youtube")));
        let (_, a) = tool(rules("pause all youtube tabs"));
        assert_eq!((a["target"].as_str(), a["all"].as_bool()), (Some("youtube"), Some(true)));
        let (_, a) = tool(rules("stop the second youtube tab"));
        assert_eq!((a["target"].as_str(), a["n"].as_u64()), (Some("youtube"), Some(2)));
        let (_, a) = tool(rules("pause youtube in brave"));
        assert_eq!((a["target"].as_str(), a["browser"].as_str()), (Some("youtube"), Some("brave")));
        let (_, a) = tool(rules("pause the song on spotify"));
        assert_eq!(a["target"].as_str(), Some("spotify"));
        assert_eq!(tool(rules("stop spotify")).1["target"], "spotify");
        assert_eq!(tool(rules("stop twitch")).1["target"], "twitch");
        assert!(rules("stop the download").is_none());
        assert!(rules("stop recording").is_none());
        assert!(rules("stop").is_none());
    }

    #[test]
    fn other_phrases() {
        assert_eq!(tool(rules("resume")).0, "media.play");
        assert_eq!(tool(rules("play")).0, "media.play");
        assert_eq!(tool(rules("resume spotify")).1["target"], "spotify");
        assert_eq!(tool(rules("what's playing")).0, "media.list");
        assert_eq!(tool(rules("next song")).0, "media.next");
        assert_eq!(tool(rules("alright i don't like this song choose something else please")).0, "media.next");
        assert_eq!(tool(rules("okay can u play another song instead of slow it down")).0, "media.next");
        assert!(rules("play something else by arijit singh").is_none_or(|i| !matches!(i, Intent::Act { ref tool, .. } if tool == "media.next")));
        assert_eq!(tool(rules("mute youtube")).0, "media.mute");
        assert_eq!(tool(rules("mute this tab")).1["active"], true);
        assert_eq!(tool(rules("unmute twitch")).0, "media.unmute");
        assert!(rules("play arijit singh").is_none());
        assert!(rules("mute").is_none());
    }

    #[test]
    fn questions_read_naturally() {
        let mk = |id: i64, url: &str, title: &str| Player {
            key: format!("os:brave.instance{id}"),
            source: media::site_name(url),
            app: "Brave".into(),
            title: title.into(),
            artist: String::new(),
            status: Status::Playing,
            browser: Some("brave".into()),
            muted: false,
            active: false,
        };
        let spotify = Player { key: "os:spotify".into(), source: "Spotify".into(), app: "Spotify".into(), title: "Song".into(), artist: "A".into(), status: Status::Playing, browser: None, muted: false, active: false };
        let two = vec![mk(1, "https://www.youtube.com/watch?v=1", "Tutorial - YouTube"), spotify.clone()];
        let f = ask_which("media.pause", "stop", &two);
        assert_eq!(f.question, "There are two media playing, one is YouTube and the other is Spotify. Which one should I stop?");
        assert_eq!(f.choices.len(), 3);
        let bunch = vec![mk(1, "https://www.youtube.com/watch?v=1", "A - YouTube"), mk(2, "https://www.youtube.com/watch?v=2", "B - YouTube"), spotify];
        let f = ask_which("media.pause", "pause", &bunch);
        assert_eq!(f.question, "There are two media playing, one is a bunch of YouTube tabs and the other is Spotify. Which one should I pause?");
        let f = ask_all_or_one("media.pause", "stop", &bunch[..2]);
        assert_eq!(f.question, "There are two YouTube tabs playing. Should I stop all of them, or a specific one?");
        assert_eq!(f.choices[0].then, act("media.pause", json!({ "verb": "stop", "keys": ["os:brave.instance1", "os:brave.instance2"], "all": true })));
    }
}
