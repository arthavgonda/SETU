use crate::text::normalize;

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Stop,
    Undo,
    Redo,
    DeleteLine,
    DeleteWord,
    Backspace(u32),
    GoToLine(u32),
    SelectLine(Option<u32>),
    Save,
    Run,
    Check,
    NextError,
    Format,
    Comment,
    Indent,
    Outdent,
    LineEnd,
    LineStart,
    Move(&'static str, u32),
    CopyLine,
    Paste,
    SelectAll,
    NewFile,
    Terminal,
    Escape,
    Enter,
}

const STOP: &[&str] = &[
    "stop coding", "stop dictation", "stop dictating", "exit coding mode", "exit coding", "stop typing", "end coding", "coding off", "done coding", "exit dictation", "stop code mode",
    "close coding mode", "finish coding", "stop writing code", "i'm done coding", "i am done coding",
];

const CHECK: &[&str] = &[
    "am i wrong somewhere", "am i wrong", "am i wrong anywhere", "am i doing something wrong", "is there any error", "is there an error", "are there any errors", "any errors", "any error",
    "check my code", "check the code", "check this code", "check code", "what's wrong", "what is wrong", "what's wrong with my code", "what is wrong with my code", "is my code correct",
    "is my code right", "is the code correct", "is there a mistake", "is there any mistake", "did i make a mistake", "did i make any mistake", "find errors", "find the error",
    "find mistakes", "is anything wrong", "where is the error", "where's the error", "where is the mistake", "why is it not working", "why isn't it working", "debug this",
];

const RUN: &[&str] = &["run", "run it", "run the code", "run my code", "run this", "run this code", "run the program", "run my program", "compile and run", "execute", "execute it", "execute the code", "start the program", "compile it", "build and run", "run the project", "run my project", "run the app", "run my app"];

fn num(words: &[&str]) -> Option<u32> {
    let mut best = None;
    for i in 0..words.len() {
        if let Some(n) = parse_number(&words[i..]).map(|(n, _)| n) {
            best = Some(n as u32);
            break;
        }
    }
    best
}

pub fn command(spoken: &str) -> Option<Cmd> {
    let t = normalize(spoken);
    let t = t.trim_start_matches("please ").trim_end_matches(" please").trim();
    let t = t.trim_start_matches("setu ").trim_start_matches("hey setu ").trim();
    if t.is_empty() {
        return None;
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    if STOP.contains(&t) {
        return Some(Cmd::Stop);
    }
    if CHECK.contains(&t) || CHECK.iter().any(|c| c.len() > 12 && t.starts_with(c)) {
        return Some(Cmd::Check);
    }
    if RUN.contains(&t) {
        return Some(Cmd::Run);
    }
    Some(match t {
        "undo" | "undo that" | "undo it" | "scratch that" => Cmd::Undo,
        "redo" | "redo that" => Cmd::Redo,
        "delete line" | "delete this line" | "remove line" | "remove this line" | "clear line" | "delete the line" => Cmd::DeleteLine,
        "delete word" | "delete last word" | "delete the last word" | "remove last word" => Cmd::DeleteWord,
        "backspace" | "back space" => Cmd::Backspace(1),
        "save" | "save it" | "save file" | "save the file" | "save this" => Cmd::Save,
        "next error" | "next problem" | "next mistake" => Cmd::NextError,
        "format" | "format document" | "format the code" | "format code" | "format this" => Cmd::Format,
        "comment line" | "comment this line" | "comment that" | "comment it" | "uncomment" | "uncomment line" | "toggle comment" => Cmd::Comment,
        "indent" | "indent line" | "indent this" => Cmd::Indent,
        "outdent" | "unindent" | "dedent" => Cmd::Outdent,
        "end of line" | "go to end" | "go to end of line" | "line end" | "end" => Cmd::LineEnd,
        "start of line" | "beginning of line" | "go to start" | "line start" | "home" => Cmd::LineStart,
        "copy line" | "copy this line" | "duplicate line" => Cmd::CopyLine,
        "paste" | "paste it" | "paste that" => Cmd::Paste,
        "select all" | "select everything" => Cmd::SelectAll,
        "select line" | "select this line" => Cmd::SelectLine(None),
        "new file" | "create new file" | "create a new file" => Cmd::NewFile,
        "open terminal" | "show terminal" | "terminal" => Cmd::Terminal,
        "escape" | "cancel that" => Cmd::Escape,
        "press enter" | "hit enter" => Cmd::Enter,
        _ => {
            if (t.starts_with("backspace ") || t.starts_with("back space ") || t.starts_with("delete ") && t.ends_with("characters"))
                && let Some(n) = num(&words)
            {
                return Some(Cmd::Backspace(n.clamp(1, 200)));
            }
            for p in ["go to line ", "jump to line ", "line number ", "goto line ", "move to line ", "take me to line "] {
                if let Some(r) = t.strip_prefix(p)
                    && let Some(n) = num(&r.split_whitespace().collect::<Vec<_>>())
                {
                    return Some(Cmd::GoToLine(n.max(1)));
                }
            }
            if words.len() == 2
                && words[0] == "line"
                && let Some(n) = num(&words[1..])
            {
                return Some(Cmd::GoToLine(n.max(1)));
            }
            if let Some(r) = t.strip_prefix("select line ")
                && let Some(n) = num(&r.split_whitespace().collect::<Vec<_>>())
            {
                return Some(Cmd::SelectLine(Some(n.max(1))));
            }
            let dir = ["up", "down", "left", "right"].into_iter().find(|d| words.first() == Some(d) || words.len() >= 2 && words[0] == "move" && words[1] == *d || words.len() >= 2 && words[0] == "go" && words[1] == *d);
            if let Some(d) = dir
                && words.len() <= 5
                && words.iter().all(|w| matches!(*w, "up" | "down" | "left" | "right" | "move" | "go" | "line" | "lines" | "by" | "character" | "characters") || parse_number(&[w]).is_some())
            {
                return Some(Cmd::Move(d, num(&words).unwrap_or(1).clamp(1, 500)));
            }
            return None;
        }
    })
}

pub fn starts_dictation(t: &str) -> bool {
    let t = normalize(t);
    [
        "start coding", "coding mode", "code mode", "start dictation", "dictation mode", "let's code", "lets code", "let us code", "start writing code", "write code", "i want to code",
        "start code mode", "enter coding mode", "turn on coding mode", "begin coding", "start programming", "voice coding",
    ]
    .contains(&t.trim_start_matches("please ").trim_end_matches(" please").trim_end_matches(" in vs code").trim_end_matches(" in vscode").trim())
}

const SYMBOLS: &[(&str, &str)] = &[
    ("less than or equal to", "<="),
    ("greater than or equal to", ">="),
    ("less than or equals", "<="),
    ("greater than or equals", ">="),
    ("is not equal to", "!="),
    ("not equal to", "!="),
    ("is equal to", "=="),
    ("double equals", "=="),
    ("double equal", "=="),
    ("equals equals", "=="),
    ("equal equal", "=="),
    ("triple equals", "==="),
    ("not equals", "!="),
    ("not equal", "!="),
    ("bang equals", "!="),
    ("plus equals", "+="),
    ("plus equal", "+="),
    ("minus equals", "-="),
    ("minus equal", "-="),
    ("times equals", "*="),
    ("divide equals", "/="),
    ("plus plus", "++"),
    ("minus minus", "--"),
    ("less than less than", "<<"),
    ("greater than greater than", ">>"),
    ("left shift", "<<"),
    ("right shift", ">>"),
    ("double less than", "<<"),
    ("double greater than", ">>"),
    ("insertion operator", "<<"),
    ("extraction operator", ">>"),
    ("less or equal", "<="),
    ("greater or equal", ">="),
    ("less than", "<"),
    ("greater than", ">"),
    ("and and", "&&"),
    ("logical and", "&&"),
    ("or or", "||"),
    ("logical or", "||"),
    ("double colon", "::"),
    ("scope resolution", "::"),
    ("fat arrow", "=>"),
    ("arrow", "->"),
    ("open parenthesis", "("),
    ("open paren", "("),
    ("open parentheses", "("),
    ("left parenthesis", "("),
    ("left paren", "("),
    ("open round bracket", "("),
    ("open bracket", "("),
    ("close parenthesis", ")"),
    ("close paren", ")"),
    ("close parentheses", ")"),
    ("right parenthesis", ")"),
    ("right paren", ")"),
    ("close round bracket", ")"),
    ("close bracket", ")"),
    ("empty parentheses", "()"),
    ("empty parens", "()"),
    ("parens", "()"),
    ("open square bracket", "["),
    ("open square", "["),
    ("left square bracket", "["),
    ("close square bracket", "]"),
    ("close square", "]"),
    ("right square bracket", "]"),
    ("open curly brace", "{"),
    ("open curly bracket", "{"),
    ("open curly", "{"),
    ("open brace", "{"),
    ("left brace", "{"),
    ("curly open", "{"),
    ("close curly brace", "}"),
    ("close curly bracket", "}"),
    ("close curly", "}"),
    ("close brace", "}"),
    ("right brace", "}"),
    ("curly close", "}"),
    ("open angle bracket", "<"),
    ("close angle bracket", ">"),
    ("double quote", "\""),
    ("double quotes", "\""),
    ("single quote", "'"),
    ("quote", "\""),
    ("quotes", "\""),
    ("apostrophe", "'"),
    ("backtick", "`"),
    ("back tick", "`"),
    ("semicolon", ";"),
    ("semi colon", ";"),
    ("semi", ";"),
    ("colon", ":"),
    ("comma", ","),
    ("dot", "."),
    ("period", "."),
    ("full stop", "."),
    ("equals", "="),
    ("equal", "="),
    ("equal sign", "="),
    ("equals sign", "="),
    ("assign", "="),
    ("plus", "+"),
    ("minus", "-"),
    ("dash", "-"),
    ("hyphen", "-"),
    ("times", "*"),
    ("asterisk", "*"),
    ("star", "*"),
    ("multiplied by", "*"),
    ("multiply", "*"),
    ("divided by", "/"),
    ("divide", "/"),
    ("slash", "/"),
    ("forward slash", "/"),
    ("backslash", "\\"),
    ("back slash", "\\"),
    ("modulo", "%"),
    ("modulus", "%"),
    ("percent", "%"),
    ("mod", "%"),
    ("ampersand", "&"),
    ("pipe", "|"),
    ("vertical bar", "|"),
    ("caret", "^"),
    ("tilde", "~"),
    ("exclamation mark", "!"),
    ("exclamation", "!"),
    ("bang", "!"),
    ("question mark", "?"),
    ("hash", "#"),
    ("hashtag", "#"),
    ("pound sign", "#"),
    ("at sign", "@"),
    ("at the rate", "@"),
    ("dollar sign", "$"),
    ("dollar", "$"),
    ("underscore", "_"),
    ("new line", "\n"),
    ("newline", "\n"),
    ("next line", "\n"),
    ("line break", "\n"),
    ("enter", "\n"),
    ("tab", "\t"),
];

const ONES: &[&str] = &["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TENS: &[&str] = &["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

fn small(w: &str) -> Option<i64> {
    if let Ok(n) = w.parse::<i64>() {
        return Some(n);
    }
    if let Some(i) = ONES.iter().position(|x| *x == w) {
        return Some(i as i64);
    }
    TENS.iter().position(|x| !x.is_empty() && *x == w).map(|i| i as i64 * 10)
}

pub fn parse_number(words: &[&str]) -> Option<(i64, usize)> {
    let mut total = 0i64;
    let mut current = 0i64;
    let mut used = 0usize;
    let mut any = false;
    while used < words.len() {
        let w = words[used];
        if let Ok(n) = w.parse::<i64>() {
            if any {
                break;
            }
            return Some((n, 1));
        }
        match w {
            "hundred" if any => current *= 100,
            "thousand" if any => {
                total += current * 1000;
                current = 0;
            }
            "and" if any && words.get(used + 1).is_some_and(|n| small(n).is_some()) => {}
            _ => match small(w) {
                Some(n) if n < 10 && current % 10 == 0 && current >= 20 && current % 100 != 0 => current += n,
                Some(n) if !any || current == 0 && total > 0 || (current % 100 == 0 && current > 0 && n < 100) => current += n,
                _ => break,
            },
        }
        any = true;
        used += 1;
    }
    any.then_some((total + current, used))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Num(String),
    Sym(String),
}

fn case_words(mode: &str, words: &[String]) -> String {
    match mode {
        "camel" => words.iter().enumerate().map(|(i, w)| if i == 0 { w.to_lowercase() } else { cap(w) }).collect(),
        "pascal" => words.iter().map(|w| cap(w)).collect(),
        "snake" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("_"),
        "kebab" => words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>().join("-"),
        "constant" => words.iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join("_"),
        "upper" => words.iter().map(|w| w.to_uppercase()).collect::<Vec<_>>().join(""),
        _ => words.join(""),
    }
}

fn cap(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn split_raw(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in raw.chars() {
        if c.is_alphanumeric() || c == '\'' || c == '_' {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn render(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches(['.', '?', '!']).trim();
    let shouting = trimmed.chars().filter(|c| c.is_alphabetic()).count() > 3 && trimmed.chars().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase());
    let pieces: Vec<String> = split_raw(trimmed)
        .into_iter()
        .map(|w| if w.chars().all(|c| c.is_alphabetic() || c == '\'') && (shouting || w.chars().next().is_some_and(|c| c.is_uppercase())) { w.to_lowercase() } else { w })
        .collect();
    let lower: Vec<String> = pieces.iter().map(|p| p.to_lowercase()).collect();
    let mut toks: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        if lower[i] == "literally"
            && let Some(next) = pieces.get(i + 1)
        {
            toks.push(Tok::Word(next.clone()));
            i += 2;
            continue;
        }
        let mode = match (lower[i].as_str(), lower.get(i + 1).map(String::as_str)) {
            ("camel", Some("case")) => Some(("camel", 2)),
            ("pascal", Some("case")) => Some(("pascal", 2)),
            ("snake", Some("case")) => Some(("snake", 2)),
            ("kebab", Some("case")) => Some(("kebab", 2)),
            ("constant", Some("case")) | ("screaming", Some("snake")) => Some(("constant", 2)),
            ("all", Some("caps")) | ("upper", Some("case")) => Some(("upper", 2)),
            _ => None,
        };
        if let Some((m, skip)) = mode {
            let mut j = i + skip;
            let mut ws = Vec::new();
            while j < pieces.len() && pieces[j].chars().all(|c| c.is_alphanumeric()) && symbol_at(&lower, j).is_none() && small(&lower[j]).is_none() {
                ws.push(pieces[j].clone());
                j += 1;
            }
            if !ws.is_empty() {
                toks.push(Tok::Word(case_words(m, &ws)));
            }
            i = j;
            continue;
        }
        if matches!(lower[i].as_str(), "capital" | "cap" | "caps" | "uppercase")
            && let Some(next) = pieces.get(i + 1)
        {
            toks.push(Tok::Word(cap(next)));
            i += 2;
            continue;
        }
        if let Some((sym, used)) = symbol_at(&lower, i) {
            toks.push(Tok::Sym(sym.to_owned()));
            i += used;
            continue;
        }
        let words: Vec<&str> = lower[i..].iter().map(String::as_str).collect();
        if let Some((n, used)) = parse_number(&words)
            && (used > 1 || small(&lower[i]).is_some())
        {
            let mut s = n.to_string();
            let mut k = i + used;
            if matches!(lower.get(k).map(String::as_str), Some("point")) {
                let mut frac = String::new();
                let mut m = k + 1;
                while let Some(d) = lower.get(m).and_then(|w| small(w)).filter(|d| (0..10).contains(d)) {
                    frac.push_str(&d.to_string());
                    m += 1;
                }
                if !frac.is_empty() {
                    s = format!("{s}.{frac}");
                    k = m;
                }
            }
            toks.push(Tok::Num(s));
            i = k;
            continue;
        }
        let p = &pieces[i];
        if p.chars().count() == 1 && !p.chars().next().unwrap().is_alphanumeric() {
            toks.push(Tok::Sym(p.clone()));
        } else {
            toks.push(Tok::Word(p.clone()));
        }
        i += 1;
    }
    join(&toks)
}

fn hash_comments(lang: &str) -> bool {
    matches!(lang, "python" | "shellscript" | "sh" | "bash" | "ruby" | "perl" | "r" | "yaml" | "toml" | "dockerfile" | "makefile" | "powershell" | "coffeescript" | "julia" | "elixir" | "nim" | "properties" | "ini")
}

pub fn comment(text: &str, lang: &str) -> String {
    let body = text.trim().trim_end_matches(['.', '!']).trim();
    let mut chars = body.chars();
    let body = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    let line = match lang {
        l if hash_comments(l) => format!("# {body}"),
        "sql" | "lua" | "haskell" | "ada" | "elm" => format!("-- {body}"),
        "html" | "xml" | "markdown" | "vue-html" | "svg" => format!("<!-- {body} -->"),
        "css" | "scss" | "less" => format!("/* {body} */"),
        "bat" => format!("REM {body}"),
        "vb" => format!("' {body}"),
        "matlab" | "latex" | "tex" | "erlang" => format!("% {body}"),
        "lisp" | "clojure" | "scheme" => format!(";; {body}"),
        _ => format!("// {body}"),
    };
    format!("{line}\n")
}

pub fn comment_request(t: &str) -> Option<String> {
    let t = normalize(t);
    let t = t.trim_start_matches("please ").trim();
    for p in ["add a comment saying ", "add a comment that says ", "add a comment ", "add comment ", "write a comment saying ", "write a comment ", "comment saying ", "comment that says ", "put a comment saying ", "put a comment ", "insert a comment ", "insert comment ", "comment "] {
        if let Some(r) = t.strip_prefix(p) {
            let r = r.trim_start_matches("that ").trim_start_matches("saying ").trim();
            if r.is_empty() || matches!(r, "line" | "this line" | "that" | "it" | "out" | "this" | "the line") || r.split_whitespace().count() < 2 && matches!(r, "out" | "here") {
                return None;
            }
            return Some(r.to_owned());
        }
    }
    None
}

pub fn write_request(t: &str) -> bool {
    let t = normalize(t);
    let t = t.trim_start_matches("please ").trim_start_matches("can you ").trim_start_matches("could you ").trim();
    const STARTS: &[&str] = &[
        "write a function", "write me a function", "write the function", "write a class", "write me a class", "write code", "write some code", "write me code", "write a program", "write me a program",
        "write a script", "write a loop", "write a method", "write a test", "write tests", "write a unit test", "generate a function", "generate code", "generate the code", "generate a class",
        "generate a program", "create a function", "create a class", "create a method", "implement a function", "implement a class", "implement the function", "add a function", "add a method",
        "make a function", "write a for loop", "write a while loop", "write an if statement", "write a component", "write a query", "write an sql query", "write a regex", "write a regular expression",
    ];
    STARTS.iter().any(|p| t.starts_with(p)) && t.split_whitespace().count() >= 4
}

fn depth(code: &str) -> (i32, bool) {
    let (mut d, mut quote, mut esc) = (0i32, None::<char>, false);
    for c in code.chars() {
        if let Some(q) = quote {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quote = Some(c),
            '(' | '[' => d += 1,
            ')' | ']' => d -= 1,
            _ => {}
        }
    }
    (d, quote.is_some())
}

pub fn layout(code: &str, lang: &str) -> String {
    let t = code.trim_end_matches([' ', '\t']);
    if t.is_empty() || t.ends_with('\n') {
        return t.to_owned();
    }
    let (open, quoted) = depth(t);
    if open > 0 || quoted {
        return t.to_owned();
    }
    let last = t.chars().last().unwrap_or(' ');
    if matches!(last, ',' | '+' | '-' | '*' | '/' | '=' | '&' | '|' | '.' | '<' | '>' | '!' | '%' | '\\' | '?') {
        return t.to_owned();
    }
    let lower = t.trim_start().to_lowercase();
    if hash_comments(lang) || lang == "python" {
        let opener = ["def ", "class ", "if ", "elif ", "else", "for ", "while ", "try", "except", "finally", "with ", "async ", "match ", "case "].iter().any(|k| lower.starts_with(k));
        if opener && last != ':' && lang == "python" {
            return t.to_owned();
        }
    }
    format!("{t}\n")
}

fn symbol_at(lower: &[String], i: usize) -> Option<(&'static str, usize)> {
    for (phrase, sym) in SYMBOLS {
        let parts: Vec<&str> = phrase.split(' ').collect();
        if i + parts.len() <= lower.len() && parts.iter().enumerate().all(|(k, p)| lower[i + k] == *p) {
            return Some((sym, parts.len()));
        }
    }
    None
}

const KEYWORDS_SPACED: &[&str] = &["if", "for", "while", "switch", "return", "catch", "elif", "and", "or", "not", "in", "else", "do"];

fn join(toks: &[Tok]) -> String {
    let mut out = String::new();
    let mut quote_open = false;
    let mut include = false;
    let mut prev: Option<&Tok> = None;
    for t in toks {
        let text = match t {
            Tok::Word(w) | Tok::Num(w) | Tok::Sym(w) => w.as_str(),
        };
        let mut space = !out.is_empty() && !out.ends_with(['\n', '\t', ' ']);
        if let Some(p) = prev {
            let ptext = match p {
                Tok::Word(w) | Tok::Num(w) | Tok::Sym(w) => w.as_str(),
            };
            let word_before = matches!(p, Tok::Word(_) | Tok::Num(_));
            if let Tok::Sym(s) = t {
                match s.as_str() {
                    ";" | "," | ")" | "]" | "." | ":" | "::" | "->" | "\n" | "\t" => space = false,
                    "++" | "--" if word_before => space = false,
                    "(" => space = space && (!word_before || KEYWORDS_SPACED.contains(&ptext)),
                    "[" => space = space && !word_before,
                    "\"" | "'" | "`" => space = space && !quote_open,
                    "<" | ">" if include => space = false,
                    _ => {}
                }
            }
            if let Tok::Sym(ps) = p {
                match ps.as_str() {
                    "(" | "[" | "." | "::" | "->" | "!" | "~" | "#" | "\n" | "\t" | "@" => space = false,
                    "\"" | "'" | "`" if quote_open => space = false,
                    "<" if include => space = false,
                    "-" | "+" | "*" | "&" if matches!(prev_sym_context(&out), Some(true)) => space = false,
                    _ => {}
                }
            }
            if include && ptext == "include" && text == "<" {
                space = true;
            }
        }
        if let Tok::Sym(s) = t
            && matches!(s.as_str(), "\"" | "'" | "`")
        {
            quote_open = !quote_open;
        }
        if text == "\n" {
            include = false;
            while out.ends_with(' ') {
                out.pop();
            }
        }
        if let Some(Tok::Sym(ps)) = prev
            && ps == "#"
            && text == "include"
        {
            include = true;
        }
        if space {
            out.push(' ');
        }
        out.push_str(text);
        prev = Some(t);
    }
    out
}

fn prev_sym_context(out: &str) -> Option<bool> {
    let trimmed = out.trim_end();
    let before = trimmed.chars().rev().nth(1)?;
    Some(matches!(before, '(' | '=' | ',' | '[' | ' ') && trimmed.len() >= 2 && trimmed.chars().rev().nth(2).is_none_or(|c| c == ' ' || c == '(' || c == '='))
}

pub fn spoken_email(t: &str) -> Option<String> {
    let n = normalize(t);
    if !(n.contains(" at ") || n.contains(" at the rate ")) || !(n.contains(" dot ") || n.contains(".com")) {
        return None;
    }
    let mut s = format!(" {n} ");
    for (a, b) in [(" at the rate ", "@"), (" at ", "@"), (" dot ", "."), (" underscore ", "_"), (" dash ", "-"), (" hyphen ", "-")] {
        s = s.replace(a, b);
    }
    let joined: String = s.split_whitespace().collect();
    (joined.matches('@').count() == 1 && joined.contains('.')).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lays_out_lines_by_language() {
        assert_eq!(layout("def greet(name):", "python"), "def greet(name):\n");
        assert_eq!(layout("print(\"hello\")", "python"), "print(\"hello\")\n");
        assert_eq!(layout("def greet(name", "python"), "def greet(name");
        assert_eq!(layout("if x > 5", "python"), "if x > 5");
        assert_eq!(layout("total = a +", "python"), "total = a +");
        assert_eq!(layout("int main() {", "cpp"), "int main() {\n");
        assert_eq!(layout("return 0;", "cpp"), "return 0;\n");
        assert_eq!(layout("let s = \"a, b", "javascript"), "let s = \"a, b");
        assert_eq!(layout("x = 1", "javascript"), "x = 1\n");
    }

    #[test]
    fn comments_follow_the_language() {
        assert_eq!(comment("adds two numbers", "python"), "# Adds two numbers\n");
        assert_eq!(comment("adds two numbers.", "rust"), "// Adds two numbers\n");
        assert_eq!(comment("main layout", "html"), "<!-- Main layout -->\n");
        assert_eq!(comment_request("add a comment saying this function adds numbers").as_deref(), Some("this function adds numbers"));
        assert_eq!(comment_request("comment this line"), None);
        assert_eq!(comment_request("comment it"), None);
        assert!(write_request("write a function that reverses a string"));
        assert!(write_request("please generate code to read a csv file"));
        assert!(!write_request("write code"));
        assert!(!write_request("print hello world"));
    }

    #[test]
    fn renders_code_literally() {
        assert_eq!(render("int i equals ten"), "int i = 10");
        assert_eq!(render("int i = 10"), "int i = 10");
        assert_eq!(render("INT I EQUALS TEN"), "int i = 10");
        assert_eq!(render("Int i equals 10."), "int i = 10");
        assert_eq!(render("int i equals ten semicolon"), "int i = 10;");
        assert_eq!(render("for open paren int i equals zero semicolon i less than n semicolon i plus plus close paren"), "for (int i = 0; i < n; i++)");
        assert_eq!(render("hash include less than iostream greater than"), "#include <iostream>");
        assert_eq!(render("std double colon cout less than less than x"), "std::cout << x");
        assert_eq!(render("print open paren quote hello world quote close paren"), "print(\"hello world\")");
        assert_eq!(render("int camel case max value equals one hundred twenty three"), "int maxValue = 123");
        assert_eq!(render("float pi equals three point one four"), "float pi = 3.14");
        assert_eq!(render("snake case user name equals capital john"), "user_name = John");
        assert_eq!(render("if open paren x double equals five close paren open curly brace"), "if (x == 5) {");
        assert_eq!(render("return zero semicolon new line close curly brace"), "return 0;\n}");
        assert_eq!(render("arr open square bracket i close square bracket"), "arr[i]");
        assert_eq!(render("cout"), "cout");
    }

    #[test]
    fn recognises_commands() {
        assert_eq!(command("Hey Setu, am I wrong somewhere?"), Some(Cmd::Check));
        assert_eq!(command("am i wrong somewhere"), Some(Cmd::Check));
        assert_eq!(command("check my code"), Some(Cmd::Check));
        assert_eq!(command("go to line twelve"), Some(Cmd::GoToLine(12)));
        assert_eq!(command("line 5"), Some(Cmd::GoToLine(5)));
        assert_eq!(command("run it"), Some(Cmd::Run));
        assert_eq!(command("stop coding"), Some(Cmd::Stop));
        assert_eq!(command("backspace three"), Some(Cmd::Backspace(3)));
        assert_eq!(command("move up two lines"), Some(Cmd::Move("up", 2)));
        assert_eq!(command("int i equals ten"), None);
        assert_eq!(command("select line four"), Some(Cmd::SelectLine(Some(4))));
        assert!(starts_dictation("start coding"));
        assert!(starts_dictation("let's code in vs code"));
        assert!(!starts_dictation("open vs code"));
    }

    #[test]
    fn numbers_and_email() {
        assert_eq!(parse_number(&["one", "hundred", "twenty", "three"]), Some((123, 4)));
        assert_eq!(parse_number(&["twenty", "five"]), Some((25, 2)));
        assert_eq!(parse_number(&["ten"]), Some((10, 1)));
        assert_eq!(parse_number(&["two", "thousand", "and", "five"]), Some((2005, 4)));
        assert_eq!(spoken_email("anoop dot mehra at gmail dot com").as_deref(), Some("anoop.mehra@gmail.com"));
        assert_eq!(spoken_email("open the door"), None);
    }
}
