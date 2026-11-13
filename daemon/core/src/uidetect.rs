use crate::ocr::{Image, Line};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Button,
    Field,
    Icon,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub label: String,
}

impl Element {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Debug, Clone, Copy)]
struct Blob {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    n: usize,
}

impl Blob {
    fn w(&self) -> usize {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> usize {
        self.y1 - self.y0 + 1
    }
}

fn luma(img: &Image) -> Vec<u8> {
    img.rgb.chunks_exact(3).map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8).collect()
}

fn edges(g: &[u8], w: usize, h: usize) -> Vec<bool> {
    let mut e = vec![false; w * h];
    if w < 3 || h < 3 {
        return e;
    }
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let dx = (g[i + 1] as i32 - g[i - 1] as i32).abs();
            let dy = (g[i + w] as i32 - g[i - w] as i32).abs();
            e[i] = dx + dy > 36;
        }
    }
    e
}

fn blobs(e: &[bool], w: usize, h: usize) -> Vec<Blob> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !e[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut b = Blob { x0: usize::MAX, y0: usize::MAX, x1: 0, y1: 0, n: 0 };
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            b.x0 = b.x0.min(x);
            b.y0 = b.y0.min(y);
            b.x1 = b.x1.max(x);
            b.y1 = b.y1.max(y);
            b.n += 1;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if e[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        if b.n >= 8 {
            out.push(b);
        }
    }
    out
}

fn side(e: &[bool], w: usize, h: usize, fixed: usize, from: usize, to: usize, horizontal: bool) -> f32 {
    let mut hit = 0usize;
    let span = to.saturating_sub(from) + 1;
    for t in from..=to {
        let near = (fixed.saturating_sub(1)..=fixed + 1).any(|f| {
            let (x, y) = if horizontal { (t, f) } else { (f, t) };
            x < w && y < h && e[y * w + x]
        });
        hit += near as usize;
    }
    hit as f32 / span.max(1) as f32
}

fn outlined(e: &[bool], w: usize, h: usize, b: &Blob) -> bool {
    let inset = |a: usize, c: usize| (a + (c - a) / 10, c - (c - a) / 10);
    let (hx0, hx1) = inset(b.x0, b.x1);
    let (vy0, vy1) = inset(b.y0, b.y1);
    [side(e, w, h, b.y0, hx0, hx1, true), side(e, w, h, b.y1, hx0, hx1, true), side(e, w, h, b.x0, vy0, vy1, false), side(e, w, h, b.x1, vy0, vy1, false)].iter().all(|s| *s >= 0.8)
}

fn inside(l: &Line, b: &Blob) -> bool {
    let (cx, cy) = (l.x + l.w / 2.0, l.y + l.h / 2.0);
    cx > b.x0 as f64 && cx < b.x1 as f64 && cy > b.y0 as f64 && cy < b.y1 as f64
}

fn overlaps(l: &Line, b: &Blob) -> bool {
    l.x < b.x1 as f64 && l.x + l.w > b.x0 as f64 && l.y < b.y1 as f64 && l.y + l.h > b.y0 as f64
}

fn label_near(lines: &[Line], b: &Blob) -> String {
    let (x0, y0, x1, y1) = (b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64);
    let h = y1 - y0;
    let left = lines
        .iter()
        .filter(|l| l.x + l.w <= x0 + 2.0 && x0 - (l.x + l.w) < 3.0 * h.max(30.0) && l.y < y1 && l.y + l.h > y0)
        .max_by(|a, b| (a.x + a.w).partial_cmp(&(b.x + b.w)).unwrap_or(std::cmp::Ordering::Equal));
    let above = lines
        .iter()
        .filter(|l| l.y + l.h <= y0 + 2.0 && y0 - (l.y + l.h) < 1.5 * h.max(20.0) && l.x < x1 && l.x + l.w > x0)
        .max_by(|a, b| (a.y + a.h).partial_cmp(&(b.y + b.h)).unwrap_or(std::cmp::Ordering::Equal));
    left.or(above).map(|l| l.text.trim().trim_end_matches(':').to_owned()).unwrap_or_default()
}

pub fn detect(img: &Image, lines: &[Line]) -> Vec<Element> {
    let (w, h) = (img.w, img.h);
    if w < 16 || h < 16 || img.rgb.len() < w * h * 3 {
        return Vec::new();
    }
    let g = luma(img);
    let e = edges(&g, w, h);
    let all = blobs(&e, w, h);
    let mut boxes: Vec<(Blob, Kind, String)> = Vec::new();
    for b in &all {
        let (bw, bh) = (b.w(), b.h());
        if bw < 24 || bh < 14 || bh > 140 || bw > w * 7 / 10 || !outlined(&e, w, h, b) {
            continue;
        }
        let text: Vec<&Line> = lines.iter().filter(|l| inside(l, b)).collect();
        let (bx0, bx1) = (b.x0 as f64, b.x1 as f64);
        let kind = match text.first() {
            Some(_) => {
                let tx0 = text.iter().map(|l| l.x).fold(f64::MAX, f64::min);
                let tx1 = text.iter().map(|l| l.x + l.w).fold(f64::MIN, f64::max);
                let centred = ((tx0 + tx1) / 2.0 - (bx0 + bx1) / 2.0).abs() <= bw as f64 * 0.15;
                let left_set = tx0 - bx0 <= bw as f64 * 0.2 && (bx1 - tx1) >= bw as f64 * 0.3;
                if left_set && bw >= bh * 2 {
                    Kind::Field
                } else if centred {
                    Kind::Button
                } else {
                    continue;
                }
            }
            None if bw >= bh * 3 && bh <= 80 => Kind::Field,
            None => continue,
        };
        let label = match kind {
            Kind::Button => text.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" "),
            _ => {
                let near = label_near(lines, b);
                let placeholder = text.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" ");
                if near.is_empty() { placeholder } else { near }
            }
        };
        boxes.push((*b, kind, label));
    }
    let mut keep: Vec<(Blob, Kind, String)> = Vec::new();
    boxes.sort_by_key(|(b, _, _)| b.w() * b.h());
    for (b, k, l) in boxes {
        let nested = keep.iter().any(|(o, ok, ol)| *ok == k && *ol == l && o.x0 >= b.x0 && o.y0 >= b.y0 && o.x1 <= b.x1 && o.y1 <= b.y1);
        if !nested {
            keep.push((b, k, l));
        }
    }
    let mut out: Vec<Element> = keep.iter().map(|(b, k, l)| Element { kind: *k, x: b.x0 as f64, y: b.y0 as f64, w: b.w() as f64, h: b.h() as f64, label: l.clone() }).collect();
    for b in &all {
        let (bw, bh) = (b.w(), b.h());
        if !(12..=64).contains(&bw) || !(12..=64).contains(&bh) || bw > bh * 2 || bh > bw * 2 || b.n * 12 < bw * bh {
            continue;
        }
        if lines.iter().any(|l| overlaps(l, b)) || keep.iter().any(|(o, _, _)| b.x0 >= o.x0 && b.x1 <= o.x1 && b.y0 >= o.y0 && b.y1 <= o.y1) {
            continue;
        }
        if out.iter().any(|o| o.kind == Kind::Icon && (o.x - b.x0 as f64).abs() < 4.0 && (o.y - b.y0 as f64).abs() < 4.0) {
            continue;
        }
        out.push(Element { kind: Kind::Icon, x: b.x0 as f64, y: b.y0 as f64, w: bw as f64, h: bh as f64, label: String::new() });
    }
    out.sort_by(|a, b| (a.y, a.x).partial_cmp(&(b.y, b.x)).unwrap_or(std::cmp::Ordering::Equal));
    out
}

pub fn field<'a>(els: &'a [Element], name: &str) -> Option<&'a Element> {
    let want = crate::text::normalize(name);
    let want = want.trim_end_matches(" field").trim_end_matches(" box").trim().to_owned();
    let fields: Vec<&Element> = els.iter().filter(|e| e.kind == Kind::Field).collect();
    let scored = fields
        .iter()
        .map(|e| {
            let l = crate::text::normalize(&e.label);
            let s = if want.is_empty() {
                0.0
            } else if l == want {
                1.0
            } else if l.contains(&want) || (!l.is_empty() && want.contains(&l)) {
                0.9
            } else {
                crate::os::score(&want, &l)
            };
            (s, *e)
        })
        .filter(|(s, _)| *s >= 0.6)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, e)| e);
    scored.or_else(|| (fields.len() == 1).then(|| fields[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: usize, h: usize, bg: [u8; 3]) -> Image {
        Image { w, h, rgb: bg.iter().copied().cycle().take(w * h * 3).collect() }
    }

    fn fill(img: &mut Image, x: usize, y: usize, w: usize, h: usize, c: [u8; 3]) {
        for yy in y..y + h {
            for xx in x..x + w {
                img.rgb[(yy * img.w + xx) * 3..(yy * img.w + xx) * 3 + 3].copy_from_slice(&c);
            }
        }
    }

    fn line(text: &str, x: f64, y: f64, w: f64, h: f64) -> Line {
        Line { text: text.into(), x, y, w, h, score: 0.9 }
    }

    #[test]
    fn finds_a_button_a_field_with_its_label_and_an_icon() {
        let mut img = canvas(800, 500, [20, 25, 40]);
        fill(&mut img, 300, 300, 300, 70, [50, 115, 230]);
        fill(&mut img, 260, 200, 300, 44, [200, 200, 200]);
        fill(&mut img, 262, 202, 296, 40, [250, 250, 250]);
        fill(&mut img, 700, 40, 28, 28, [230, 230, 230]);
        fill(&mut img, 708, 48, 12, 12, [20, 25, 40]);
        let lines = vec![line("Start Game", 380.0, 320.0, 140.0, 30.0), line("Name", 180.0, 210.0, 60.0, 24.0), line("Your name", 272.0, 210.0, 100.0, 24.0)];
        let els = detect(&img, &lines);
        let button = els.iter().find(|e| e.kind == Kind::Button).expect("button");
        assert_eq!(button.label, "Start Game");
        assert!((button.x - 300.0).abs() < 4.0 && (button.w - 300.0).abs() < 6.0);
        let f = field(&els, "name field").expect("field");
        assert_eq!(f.label, "Name");
        assert!((f.y - 200.0).abs() < 4.0);
        assert!(els.iter().any(|e| e.kind == Kind::Icon && (e.x - 700.0).abs() < 4.0), "{els:?}");
        assert!(!els.iter().any(|e| e.kind == Kind::Icon && e.x > 370.0 && e.x < 530.0 && e.y > 310.0 && e.y < 360.0), "text is not an icon");
    }

    #[test]
    fn an_empty_wide_box_is_a_field_and_the_only_one_is_used() {
        let mut img = canvas(600, 300, [240, 240, 240]);
        fill(&mut img, 100, 100, 300, 36, [120, 120, 120]);
        fill(&mut img, 101, 101, 298, 34, [255, 255, 255]);
        let els = detect(&img, &[]);
        assert_eq!(els.iter().filter(|e| e.kind == Kind::Field).count(), 1, "{els:?}");
        assert!(field(&els, "search").is_some());
    }
}
