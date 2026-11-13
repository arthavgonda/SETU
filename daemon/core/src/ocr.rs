use anyhow::{Result, anyhow};
use setu_sherpa::ort::Model;
use std::path::Path;

pub use setu_face::Image;
#[cfg(not(windows))]
pub use setu_face::decode_png;

pub const DICT: &str = "0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~!\"#$%&'()*+,-./ ";
const LIMIT: usize = 1280;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub score: f32,
}

pub struct Ocr {
    det: Model,
    rec: Model,
    dict: Vec<char>,
}

fn round32(v: f32) -> usize {
    (((v / 32.0).round() as usize).max(1)) * 32
}

pub struct Boxes {
    pub list: Vec<(f64, f64, f64, f64, f32)>,
}

pub fn boxes_from_map(prob: &[f32], mw: usize, mh: usize, sx: f64, sy: f64, iw: usize, ih: usize) -> Boxes {
    let mut seen = vec![false; mw * mh];
    let mut list = Vec::new();
    let mut stack = Vec::new();
    for start in 0..mw * mh {
        if seen[start] || prob[start] <= 0.3 {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut sum = 0f32;
        let mut n = 0usize;
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % mw, i / mw);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            sum += prob[i];
            n += 1;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= mw as i64 || ny >= mh as i64 {
                        continue;
                    }
                    let j = ny as usize * mw + nx as usize;
                    if !seen[j] && prob[j] > 0.3 {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
        let score = sum / n as f32;
        if bw < 3.0 || bh < 3.0 || score < 0.5 {
            continue;
        }
        let d = (bw * bh) * 1.6 / (2.0 * (bw + bh));
        let (fx0, fy0) = (((x0 as f64 - d) / sx).max(0.0), ((y0 as f64 - d) / sy).max(0.0));
        let (fx1, fy1) = (((x1 as f64 + 1.0 + d) / sx).min(iw as f64), ((y1 as f64 + 1.0 + d) / sy).min(ih as f64));
        list.push((fx0, fy0, fx1 - fx0, fy1 - fy0, score));
    }
    list.sort_by(|a, b| {
        let same_row = (a.1 - b.1).abs() < a.3.min(b.3) * 0.5;
        if same_row { a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal) } else { a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal) }
    });
    Boxes { list }
}

pub fn ctc(probs: &[f32], steps: usize, classes: usize, dict: &[char]) -> (String, f32) {
    let mut out = String::new();
    let mut prev = 0usize;
    let mut conf = 0f32;
    let mut kept = 0usize;
    for t in 0..steps {
        let row = &probs[t * classes..(t + 1) * classes];
        let (idx, p) = row.iter().enumerate().fold((0usize, f32::MIN), |acc, (i, v)| if *v > acc.1 { (i, *v) } else { acc });
        if idx != 0 && idx != prev {
            let c = dict.get(idx - 1).copied().unwrap_or(' ');
            if c == ' ' && out.ends_with(' ') {
                prev = idx;
                continue;
            }
            out.push(c);
            conf += p;
            kept += 1;
        }
        prev = idx;
    }
    (out.trim().to_owned(), if kept == 0 { 0.0 } else { conf / kept as f32 })
}

impl Ocr {
    pub fn load(det: &Path, rec: &Path) -> Result<Self> {
        let threads = (crate::work::cores() / 2).clamp(1, 4) as i32;
        let det = Model::new(&det.to_string_lossy(), threads).map_err(|e| anyhow!("The text finder didn't load: {e}"))?;
        let rec = Model::new(&rec.to_string_lossy(), threads).map_err(|e| anyhow!("The text reader didn't load: {e}"))?;
        Ok(Ocr { det, rec, dict: DICT.chars().collect() })
    }

    pub fn find(&self, img: &Image) -> Result<Boxes> {
        let scale = (LIMIT as f32 / img.w.max(img.h) as f32).min(1.0);
        let (nw, nh) = (round32(img.w as f32 * scale), round32(img.h as f32 * scale));
        let (sx, sy) = (nw as f64 / img.w as f64, nh as f64 / img.h as f64);
        let mut data = vec![0f32; 3 * nw * nh];
        for y in 0..nh {
            for x in 0..nw {
                let p = img.sample((x as f32 + 0.5) / sx as f32 - 0.5, (y as f32 + 0.5) / sy as f32 - 0.5);
                for k in 0..3 {
                    data[k * nw * nh + y * nw + x] = (p[k] / 255.0 - MEAN[k]) / STD[k];
                }
            }
        }
        let (prob, shape) = self.det.run(&mut data, &[1, 3, nh as i64, nw as i64]).map_err(|e| anyhow!("{e}"))?;
        let (mh, mw) = (shape.get(2).copied().unwrap_or(nh as i64) as usize, shape.get(3).copied().unwrap_or(nw as i64) as usize);
        Ok(boxes_from_map(&prob, mw, mh, mw as f64 / img.w as f64, mh as f64 / img.h as f64, img.w, img.h))
    }

    pub fn recognise(&self, img: &Image, b: (f64, f64, f64, f64)) -> Result<(String, f32)> {
        let (x, y, w, h) = b;
        let th = 48usize;
        let tw = ((th as f64 * w / h.max(1.0)).ceil() as usize).clamp(16, 2400);
        let mut data = vec![0f32; 3 * th * tw];
        for yy in 0..th {
            for xx in 0..tw {
                let p = img.sample((x + (xx as f64 + 0.5) * w / tw as f64 - 0.5) as f32, (y + (yy as f64 + 0.5) * h / th as f64 - 0.5) as f32);
                for k in 0..3 {
                    data[k * th * tw + yy * tw + xx] = (p[k] / 255.0 - 0.5) / 0.5;
                }
            }
        }
        let (probs, shape) = self.rec.run(&mut data, &[1, 3, th as i64, tw as i64]).map_err(|e| anyhow!("{e}"))?;
        let steps = shape.get(1).copied().unwrap_or(0) as usize;
        let classes = shape.get(2).copied().unwrap_or(0) as usize;
        if steps == 0 || classes == 0 {
            return Ok((String::new(), 0.0));
        }
        Ok(ctc(&probs, steps, classes, &self.dict))
    }

    #[cfg(test)]
    pub fn read(&self, img: &Image) -> Result<Vec<Line>> {
        if img.w < 8 || img.h < 8 {
            return Ok(Vec::new());
        }
        let boxes = self.find(img)?;
        let mut out = Vec::new();
        for (x, y, w, h, _) in boxes.list.into_iter().take(400) {
            let (text, score) = self.recognise(img, (x, y, w, h))?;
            if text.is_empty() || score < 0.5 {
                continue;
            }
            out.push(Line { text, x, y, w, h, score });
        }
        Ok(out)
    }
}

const TILE: usize = 64;
const REMEMBER: usize = 4000;

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29)
}

fn fold(bytes: &[u8], mut h: u64) -> u64 {
    let mut it = bytes.chunks_exact(8);
    for c in &mut it {
        h = mix(h, u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]));
    }
    for b in it.remainder() {
        h = mix(h, *b as u64);
    }
    h
}

pub fn tiles(img: &Image) -> (usize, Vec<u64>) {
    let (cols, rows) = (img.w.div_ceil(TILE), img.h.div_ceil(TILE));
    let mut t = vec![0u64; cols * rows];
    for y in 0..img.h {
        let row = &img.rgb[y * img.w * 3..(y + 1) * img.w * 3];
        let ty = y / TILE;
        for tx in 0..cols {
            let (a, b) = (tx * TILE * 3, ((tx + 1) * TILE).min(img.w) * 3);
            let k = ty * cols + tx;
            t[k] = fold(&row[a..b], t[k] ^ y as u64);
        }
    }
    (cols, t)
}

pub fn bands(before: &[u64], now: &[u64], cols: usize, h: usize, lines: &[Line]) -> Vec<(usize, usize)> {
    let rows = now.len() / cols.max(1);
    let changed: Vec<bool> = (0..rows).map(|r| (0..cols).any(|c| before.get(r * cols + c) != now.get(r * cols + c))).collect();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut r = 0;
    while r < rows {
        if !changed[r] {
            r += 1;
            continue;
        }
        let start = r;
        while r < rows && changed[r] {
            r += 1;
        }
        out.push(((start * TILE).saturating_sub(TILE / 2), (r * TILE + TILE / 2).min(h)));
    }
    loop {
        let mut grew = false;
        for b in out.iter_mut() {
            for l in lines {
                let (ly0, ly1) = (l.y.max(0.0) as usize, ((l.y + l.h).ceil() as usize + 2).min(h));
                if ly1 > b.0 && ly0 < b.1 && (ly0 < b.0 || ly1 > b.1) {
                    b.0 = b.0.min(ly0.saturating_sub(2));
                    b.1 = b.1.max(ly1);
                    grew = true;
                }
            }
        }
        out.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for b in out {
            match merged.last_mut() {
                Some(m) if b.0 <= m.1 => m.1 = m.1.max(b.1),
                _ => merged.push(b),
            }
        }
        out = merged;
        if !grew {
            return out;
        }
    }
}

fn band(img: &Image, y0: usize, y1: usize) -> Image {
    Image { w: img.w, h: y1 - y0, rgb: img.rgb[y0 * img.w * 3..y1 * img.w * 3].to_vec() }
}

fn ordered(mut lines: Vec<Line>) -> Vec<Line> {
    lines.sort_by(|a, b| {
        let same_row = (a.y - b.y).abs() < a.h.min(b.h) * 0.5;
        if same_row { a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal) } else { a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal) }
    });
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Same,
    Bands(usize),
    Full,
}

#[derive(Default)]
pub struct Memo {
    key: String,
    seq: Option<u64>,
    w: usize,
    h: usize,
    tiles: Vec<u64>,
    lines: Vec<Line>,
    seen: std::collections::HashMap<u64, (String, f32)>,
    order: std::collections::VecDeque<u64>,
}

fn crop_hash(img: &Image, b: (f64, f64, f64, f64)) -> u64 {
    let (x0, y0) = (b.0.max(0.0) as usize, b.1.max(0.0) as usize);
    let (x1, y1) = (((b.0 + b.2).ceil() as usize).min(img.w), ((b.1 + b.3).ceil() as usize).min(img.h));
    let mut h = mix(0x5345_5455, ((x1.saturating_sub(x0)) as u64) << 32 | y1.saturating_sub(y0) as u64);
    for y in y0..y1 {
        h = fold(&img.rgb[(y * img.w + x0) * 3..(y * img.w + x1.max(x0)) * 3], h);
    }
    h
}

impl Ocr {
    fn recall(&self, img: &Image, b: (f64, f64, f64, f64), memo: &mut Memo) -> Result<(String, f32)> {
        let k = crop_hash(img, b);
        if let Some(hit) = memo.seen.get(&k) {
            return Ok(hit.clone());
        }
        let got = self.recognise(img, b)?;
        if memo.order.len() >= REMEMBER
            && let Some(old) = memo.order.pop_front()
        {
            memo.seen.remove(&old);
        }
        memo.order.push_back(k);
        memo.seen.insert(k, got.clone());
        Ok(got)
    }

    fn lines_in(&self, img: &Image, dy: f64, memo: &mut Memo) -> Result<Vec<Line>> {
        if img.w < 8 || img.h < 8 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (x, y, w, h, _) in self.find(img)?.list.into_iter().take(400) {
            let (text, score) = self.recall(img, (x, y, w, h), memo)?;
            if text.is_empty() || score < 0.5 {
                continue;
            }
            out.push(Line { text, x, y: y + dy, w, h, score });
        }
        Ok(out)
    }

    pub fn read_changed(&self, img: &Image, key: &str, seq: Option<u64>, memo: &mut Memo) -> Result<(Vec<Line>, Pass)> {
        let same_window = memo.key == key && memo.w == img.w && memo.h == img.h && !memo.tiles.is_empty();
        if same_window && seq.is_some() && memo.seq == seq {
            return Ok((memo.lines.clone(), Pass::Same));
        }
        let (cols, now) = tiles(img);
        if same_window && now == memo.tiles {
            memo.seq = seq;
            return Ok((memo.lines.clone(), Pass::Same));
        }
        let parts = if same_window { bands(&memo.tiles, &now, cols, img.h, &memo.lines) } else { Vec::new() };
        let changed: usize = parts.iter().map(|(a, b)| b - a).sum();
        let (lines, pass) = if same_window && !parts.is_empty() && changed * 10 < img.h * 6 {
            let mut keep: Vec<Line> = memo.lines.iter().filter(|l| !parts.iter().any(|(a, b)| l.y + l.h > *a as f64 && l.y < *b as f64)).cloned().collect();
            for (a, b) in &parts {
                let fresh = self.lines_in(&band(img, *a, *b), *a as f64, memo)?;
                keep.extend(fresh);
            }
            (ordered(keep), Pass::Bands(parts.len()))
        } else {
            (ordered(self.lines_in(img, 0.0, memo)?), Pass::Full)
        };
        memo.key = key.to_owned();
        memo.seq = seq;
        memo.w = img.w;
        memo.h = img.h;
        memo.tiles = now;
        memo.lines = lines.clone();
        Ok((lines, pass))
    }
}

pub fn join(lines: &[Line]) -> String {
    let mut out = String::new();
    let mut last: Option<&Line> = None;
    for l in lines {
        if let Some(p) = last {
            out.push_str(if (l.y - p.y).abs() < p.h.min(l.h) * 0.5 { "  " } else { "\n" });
        }
        out.push_str(&l.text);
        last = Some(l);
    }
    out
}

pub fn best_match<'a>(lines: &'a [Line], want: &str) -> Option<&'a Line> {
    let w = crate::text::normalize(want);
    if w.is_empty() {
        return None;
    }
    lines
        .iter()
        .map(|l| {
            let t = crate::text::normalize(&l.text);
            let s = if t == w {
                1.0
            } else if t.contains(&w) {
                0.9 - (t.len() - w.len()) as f64 / (t.len() as f64 * 10.0)
            } else {
                crate::os::score(&w, &t)
            };
            (s, l)
        })
        .filter(|(s, _)| *s >= 0.75)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, l)| l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctc_collapses_repeats_and_blanks() {
        let dict: Vec<char> = DICT.chars().collect();
        let classes = dict.len() + 1;
        let idx = |c: char| dict.iter().position(|d| *d == c).unwrap() + 1;
        let seq = [idx('O'), idx('O'), 0, idx('K'), idx('K'), 0, 0];
        let mut probs = vec![0f32; seq.len() * classes];
        for (t, k) in seq.iter().enumerate() {
            probs[t * classes + k] = 0.9;
        }
        assert_eq!(ctc(&probs, seq.len(), classes, &dict).0, "OK");
        assert_eq!(DICT.chars().count(), 95);
    }

    #[test]
    fn a_text_region_becomes_one_box_in_image_coordinates() {
        let (mw, mh) = (40, 20);
        let mut prob = vec![0f32; mw * mh];
        for y in 8..12 {
            for x in 5..30 {
                prob[y * mw + x] = 0.9;
            }
        }
        let b = boxes_from_map(&prob, mw, mh, 0.5, 0.5, 80, 40);
        assert_eq!(b.list.len(), 1);
        let (x, y, w, h, _) = b.list[0];
        assert!(x < 10.0 && y < 16.0 && w > 50.0 && h > 8.0);
    }

    #[test]
    #[ignore]
    fn reads_a_real_picture() {
        let dir = std::path::PathBuf::from(std::env::var("SETU_OCR_MODELS").unwrap());
        let img = decode_png(&std::fs::read(std::env::var("SETU_OCR_IMAGE").unwrap()).unwrap()).unwrap();
        let ocr = Ocr::load(&dir.join("en_PP-OCRv3_det_infer.onnx"), &dir.join("en_PP-OCRv3_rec_infer.onnx")).unwrap();
        let t = std::time::Instant::now();
        let lines = ocr.read(&img).unwrap();
        println!("OCR {} x {} in {:?}", img.w, img.h, t.elapsed());
        for l in &lines {
            println!("{:>6.0} {:>6.0} {:>5.0} {:>5.0} {:.2} {}", l.x, l.y, l.w, l.h, l.score, l.text);
        }
    }

    #[test]
    fn only_the_rows_that_changed_are_read_again() {
        let mut img = Image { w: 300, h: 400, rgb: vec![20u8; 300 * 400 * 3] };
        let (cols, before) = tiles(&img);
        assert_eq!(tiles(&img).1, before);
        for y in 200..215 {
            for x in 10..120 {
                img.rgb[(y * 300 + x) * 3] = 240;
            }
        }
        let (_, after) = tiles(&img);
        let b = bands(&before, &after, cols, 400, &[]);
        assert_eq!(b.len(), 1);
        assert!(b[0].0 <= 200 && b[0].1 >= 215 && b[0].1 - b[0].0 <= 3 * TILE, "{b:?}");
        let tall = Line { text: "Total".into(), x: 0.0, y: 150.0, w: 80.0, h: 60.0, score: 0.9 };
        let b2 = bands(&before, &after, cols, 400, std::slice::from_ref(&tall));
        assert!(b2[0].0 <= 150, "a line that crosses the band is read whole: {b2:?}");
        assert_eq!(crop_hash(&img, (10.0, 200.0, 50.0, 10.0)), crop_hash(&img, (10.0, 200.0, 50.0, 10.0)));
        assert_ne!(crop_hash(&img, (10.0, 200.0, 50.0, 10.0)), crop_hash(&img, (200.0, 200.0, 50.0, 10.0)));
    }

    #[test]
    fn spoken_text_finds_its_line() {
        let l = |t: &str, y: f64| Line { text: t.into(), x: 0.0, y, w: 10.0, h: 10.0, score: 0.9 };
        let lines = vec![l("Cancel", 0.0), l("Continue", 20.0), l("Continue later", 40.0)];
        assert_eq!(best_match(&lines, "continue").map(|l| l.y), Some(20.0));
        assert!(best_match(&lines, "delete").is_none());
    }
}
