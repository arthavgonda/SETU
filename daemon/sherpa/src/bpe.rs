use std::collections::HashMap;

const SPACE: char = '\u{2581}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Unigram,
    Bpe,
}

#[derive(Debug)]
pub struct SentencePiece {
    scores: HashMap<String, f32>,
    kind: Kind,
    max_len: usize,
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0;
    while *i < b.len() {
        let byte = b[*i];
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

fn fields(b: &[u8]) -> Option<Vec<(u64, u8, &[u8], u64)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let (num, wt) = (key >> 3, (key & 7) as u8);
        match wt {
            0 => {
                let v = varint(b, &mut i)?;
                out.push((num, wt, &b[0..0], v));
            }
            1 => {
                let end = i.checked_add(8)?;
                let s = b.get(i..end)?;
                i = end;
                out.push((num, wt, s, 0));
            }
            2 => {
                let n = varint(b, &mut i)? as usize;
                let end = i.checked_add(n)?;
                let s = b.get(i..end)?;
                i = end;
                out.push((num, wt, s, 0));
            }
            5 => {
                let end = i.checked_add(4)?;
                let s = b.get(i..end)?;
                i = end;
                out.push((num, wt, s, 0));
            }
            _ => return None,
        }
    }
    Some(out)
}

impl SentencePiece {
    pub fn load(path: &str) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        Self::parse(&bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let top = fields(bytes).ok_or("bpe model is not a valid protobuf")?;
        let mut scores = HashMap::new();
        let mut kind = Kind::Unigram;
        let mut max_len = 1;
        for (num, wt, data, _) in &top {
            if *num == 1 && *wt == 2 {
                let f = fields(data).ok_or("bad piece")?;
                let mut piece = None;
                let mut score = 0f32;
                let mut ty = 1u64;
                for (n, w, d, v) in f {
                    match (n, w) {
                        (1, 2) => piece = std::str::from_utf8(d).ok().map(str::to_owned),
                        (2, 5) => score = f32::from_le_bytes([d[0], d[1], d[2], d[3]]),
                        (3, 0) => ty = v,
                        _ => {}
                    }
                }
                if let Some(p) = piece {
                    if ty == 1 || ty == 4 {
                        max_len = max_len.max(p.chars().count());
                        scores.insert(p, score);
                    }
                }
            } else if *num == 2 && *wt == 2 {
                if let Some(f) = fields(data) {
                    for (n, w, _, v) in f {
                        if n == 3 && w == 0 && v == 2 {
                            kind = Kind::Bpe;
                        }
                    }
                }
            }
        }
        if scores.is_empty() {
            return Err("bpe model has no pieces".into());
        }
        Ok(Self { scores, kind, max_len })
    }

    pub fn encode(&self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for word in text.split_whitespace() {
            let w: String = std::iter::once(SPACE).chain(word.chars()).collect();
            match self.kind {
                Kind::Bpe => out.extend(self.bpe(&w)),
                Kind::Unigram => out.extend(self.unigram(&w)),
            }
        }
        out
    }

    fn bpe(&self, w: &str) -> Vec<String> {
        let mut parts: Vec<String> = w.chars().map(|c| c.to_string()).collect();
        loop {
            let mut best: Option<(usize, f32)> = None;
            for i in 0..parts.len().saturating_sub(1) {
                let merged = format!("{}{}", parts[i], parts[i + 1]);
                if let Some(&s) = self.scores.get(&merged) {
                    if best.is_none_or(|(_, bs)| s > bs) {
                        best = Some((i, s));
                    }
                }
            }
            let Some((i, _)) = best else { break };
            let right = parts.remove(i + 1);
            parts[i].push_str(&right);
        }
        parts
    }

    fn unigram(&self, w: &str) -> Vec<String> {
        let chars: Vec<char> = w.chars().collect();
        let n = chars.len();
        let mut best = vec![f32::NEG_INFINITY; n + 1];
        let mut back = vec![0usize; n + 1];
        best[0] = 0.0;
        for end in 1..=n {
            for start in end.saturating_sub(self.max_len)..end {
                if best[start] == f32::NEG_INFINITY {
                    continue;
                }
                let piece: String = chars[start..end].iter().collect();
                let s = match self.scores.get(&piece) {
                    Some(&s) => s,
                    None if end - start == 1 => -100.0,
                    None => continue,
                };
                if best[start] + s > best[end] {
                    best[end] = best[start] + s;
                    back[end] = start;
                }
            }
        }
        let mut out = Vec::new();
        let mut end = n;
        while end > 0 {
            let start = back[end];
            out.push(chars[start..end].iter().collect());
            end = start;
        }
        out.reverse();
        out
    }

    pub fn contains(&self, piece: &str) -> bool {
        self.scores.contains_key(piece)
    }
}
