use crate::router::Intent;
use serde_json::json;

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Stop,
    Scratch,
    Key(&'static str, &'static str),
}

pub fn rules(t: &str) -> Option<Intent> {
    let t = t.trim_start_matches("please ").trim_end_matches(" please");
    matches!(
        t,
        "dictate"
            | "start dictating"
            | "start dictation"
            | "dictation mode"
            | "take dictation"
            | "take a dictation"
            | "dictate here"
            | "dictate into this"
            | "dictate in this app"
            | "type what i say"
            | "write what i say"
            | "write down what i say"
            | "turn on dictation"
            | "begin dictation"
            | "start typing what i say"
            | "let me dictate"
            | "i want to dictate"
    )
    .then(|| Intent::Act { tool: "text.dictate".into(), args: json!({}) })
}

pub fn command(t: &str) -> Option<Cmd> {
    let t = t.trim_start_matches("please ").trim_end_matches(" please");
    Some(match t {
        "stop dictating" | "stop dictation" | "end dictation" | "dictation off" | "finish dictation" | "done dictating" | "exit dictation" | "turn off dictation" | "stop typing what i say" | "that is all for dictation" => Cmd::Stop,
        "scratch that" | "delete that" | "undo that" | "remove that" | "strike that" | "scrap that" => Cmd::Scratch,
        "press enter" | "press return" => Cmd::Key("enter", "Pressed Enter"),
        "press tab" => Cmd::Key("tab", "Next field"),
        "select all" | "select everything" => Cmd::Key("ctrl a", "Selected all"),
        "delete the last word" | "delete last word" => Cmd::Key("word delete", "Deleted the last word"),
        "undo typing" => Cmd::Key("ctrl z", "Undone"),
        "go to the end of the line" => Cmd::Key("line end", "At the end of the line"),
        "go to the start of the line" => Cmd::Key("line start", "At the start of the line"),
        _ => return None,
    })
}

const MARKS: &[(&[&str], &str)] = &[
    (&["new", "paragraph"], "\n\n"),
    (&["next", "paragraph"], "\n\n"),
    (&["new", "line"], "\n"),
    (&["next", "line"], "\n"),
    (&["full", "stop"], "."),
    (&["question", "mark"], "?"),
    (&["exclamation", "mark"], "!"),
    (&["exclamation", "point"], "!"),
    (&["semi", "colon"], ";"),
    (&["open", "bracket"], "("),
    (&["close", "bracket"], ")"),
    (&["open", "parenthesis"], "("),
    (&["close", "parenthesis"], ")"),
    (&["open", "quote"], "\u{1}\""),
    (&["open", "quotes"], "\u{1}\""),
    (&["close", "quote"], "\""),
    (&["close", "quotes"], "\""),
    (&["dot", "dot", "dot"], "…"),
    (&["at", "sign"], "@"),
    (&["percent", "sign"], "%"),
    (&["comma"], ","),
    (&["period"], "."),
    (&["colon"], ":"),
    (&["semicolon"], ";"),
    (&["hyphen"], "\u{2}-"),
    (&["ellipsis"], "…"),
    (&["ampersand"], "&"),
];

fn bare(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn closes(s: &str) -> bool {
    matches!(s, "," | "." | "?" | "!" | ":" | ";" | ")" | "\"" | "…" | "%")
}

fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '?' | '!' | '\n' | '…')
}

pub fn render(raw: &str, last: Option<char>) -> String {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let mut hit = None;
        for (seq, out) in MARKS {
            if i + seq.len() <= words.len() && seq.iter().enumerate().all(|(k, s)| bare(words[i + k]) == *s) {
                hit = Some((seq.len(), *out));
                break;
            }
        }
        match hit {
            Some((n, out)) => {
                tokens.push(out.to_owned());
                i += n;
            }
            None => {
                tokens.push(words[i].to_owned());
                i += 1;
            }
        }
    }
    let mut out = String::new();
    let mut prev = last;
    let mut glue = false;
    for tok in tokens {
        let (opens, joins, text) = match tok.strip_prefix('\u{1}') {
            Some(t) => (true, false, t.to_owned()),
            None => match tok.strip_prefix('\u{2}') {
                Some(t) => (false, true, t.to_owned()),
                None => (tok == "(", false, tok),
            },
        };
        let newline = text.starts_with('\n');
        let attach = closes(&text) && !opens;
        let space = match prev {
            None => false,
            Some(c) => !(newline || attach || joins || glue || c == '\n' || c == '(' || c.is_whitespace()),
        };
        if space {
            out.push(' ');
        }
        let cap = !newline && !attach && prev.is_none_or(ends_sentence) || (!newline && !attach && prev == Some(' ') && out.trim_end().chars().last().or(last).is_none_or(ends_sentence));
        if cap {
            let mut c = text.chars();
            if let Some(f) = c.next() {
                out.extend(f.to_uppercase());
                out.push_str(c.as_str());
            }
        } else {
            out.push_str(&text);
        }
        glue = opens || joins;
        prev = out.chars().last().or(prev);
        if attach
            && let Some(t) = out.trim_end().chars().last()
        {
            prev = Some(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_starts_and_stops_by_phrase() {
        assert!(rules("dictate").is_some());
        assert!(rules("start dictation").is_some());
        assert!(rules("please take dictation").is_some());
        assert!(rules("dictate a letter to riya").is_none());
        assert_eq!(command("stop dictating"), Some(Cmd::Stop));
        assert_eq!(command("scratch that"), Some(Cmd::Scratch));
        assert_eq!(command("press enter"), Some(Cmd::Key("enter", "Pressed Enter")));
        assert_eq!(command("save"), None, "single words are text, not commands");
        assert_eq!(command("i will stop dictating soon"), None);
    }

    #[test]
    fn spoken_marks_become_punctuation_and_sentences_are_spaced() {
        assert_eq!(render("hello there comma how are you question mark", None), "Hello there, how are you?");
        assert_eq!(render("i am fine", Some('?')), " I am fine");
        assert_eq!(render("and you", Some('e')), " and you");
        assert_eq!(render("full stop", Some('u')), ".");
        assert_eq!(render("first point new line second point", None), "First point\nSecond point");
        assert_eq!(render("thanks new paragraph regards", Some('.')), " Thanks\n\nRegards");
        assert_eq!(render("he said open quote go close quote period", None), "He said \"go\".");
        assert_eq!(render("call me open bracket after six close bracket", Some('.')), " Call me (after six)");
        assert_eq!(render("twenty percent sign off", None), "Twenty% off");
        assert_eq!(render("Meet Riya at 5 PM.", Some('.')), " Meet Riya at 5 PM.");
        assert_eq!(render("well hyphen known", Some(' ')), "well-known");
        assert_eq!(render("", Some('a')), "");
        assert_eq!(render("next line", Some('a')), "\n");
        assert_eq!(render("ok", Some('\n')), "Ok");
    }
}
