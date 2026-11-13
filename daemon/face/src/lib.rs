mod image;
#[cfg(target_os = "macos")]
mod maccam;

pub use image::{Image, decode_png};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use setu_sherpa::ort::Model;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const MODELS: [&str; 2] = ["face-det", "face-rec"];
const SIZE: usize = 640;
const REF: [[f32; 2]; 5] = [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];
pub const DETECT: f32 = 0.75;
pub const MATCH: f32 = 0.42;
pub const SAME: f32 = 0.30;
pub const SHARP: f32 = 18.0;
pub const LIVE: f32 = 0.012;
pub const STILL: f32 = 0.012;
pub const LIVE_FRAMES: usize = 5;
pub const LIVE_OVER: usize = 3;
pub const LIVE_SHARE: f32 = 0.3;
pub const YAW: f32 = 0.09;
pub const PITCH: f32 = 0.035;
pub const MIN_EYES: f32 = 28.0;
pub const NOD: f32 = 0.05;
pub const NOD_YAW: f32 = 0.07;
pub const NOD_TIME: Duration = Duration::from_millis(2200);
pub const SIDES: [&str; 5] = ["center", "left", "right", "up", "down"];

type Pts = [[f32; 2]; 5];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    pub score: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub pts: Pts,
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn len(a: [f32; 2]) -> f32 {
    dot(a, a).sqrt()
}

fn mid(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

impl Face {
    pub fn eyes(&self) -> f32 {
        len(sub(self.pts[1], self.pts[0]))
    }

    pub fn pose(&self) -> (f32, f32) {
        pose(&self.pts)
    }

    fn iou(&self, o: &Face) -> f32 {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
        let union = self.w * self.h + o.w * o.h - inter;
        if union <= 0.0 { 0.0 } else { inter / union }
    }
}

pub fn pose(p: &Pts) -> (f32, f32) {
    let ex = sub(p[1], p[0]);
    let iod = len(ex).max(1e-3);
    let u = [ex[0] / iod, ex[1] / iod];
    let v = [-u[1], u[0]];
    let em = mid(p[0], p[1]);
    let mm = mid(p[3], p[4]);
    let center = mid(em, mm);
    let yaw = dot(sub(p[2], center), u) / iod;
    let down = dot(sub(mm, em), v).max(1e-3);
    let pitch = dot(sub(p[2], em), v) / down;
    (yaw, pitch)
}

fn solve(mut a: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let piv = (col..8).max_by(|&i, &j| a[i][col].abs().partial_cmp(&a[j][col].abs()).unwrap_or(std::cmp::Ordering::Equal))?;
        if a[piv][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, piv);
        let pivot = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / pivot[col];
                for (x, p) in row.iter_mut().zip(pivot.iter()).skip(col) {
                    *x -= f * p;
                }
            }
        }
    }
    let mut x = [0f64; 8];
    for i in 0..8 {
        x[i] = a[i][8] / a[i][i];
    }
    Some(x)
}

pub fn moved(a: &Pts, b: &Pts) -> f32 {
    let s = len(sub(b[1], b[0])).max(1e-3);
    (0..5).map(|i| len(sub(a[i], b[i])) / s).fold(0.0, f32::max)
}

#[derive(Clone)]
pub struct Gray {
    w: usize,
    h: usize,
    px: Vec<f32>,
}

impl Gray {
    pub fn of(img: &Image) -> Self {
        Self { w: img.w, h: img.h, px: img.rgb.as_chunks::<3>().0.iter().map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect() }
    }

    fn at(&self, x: i32, y: i32) -> f32 {
        self.px[(y.clamp(0, self.h as i32 - 1) as usize) * self.w + x.clamp(0, self.w as i32 - 1) as usize]
    }
}

fn similar(a: &Pts, b: &Pts) -> [f32; 6] {
    let am = [a.iter().map(|p| p[0]).sum::<f32>() / 5.0, a.iter().map(|p| p[1]).sum::<f32>() / 5.0];
    let bm = [b.iter().map(|p| p[0]).sum::<f32>() / 5.0, b.iter().map(|p| p[1]).sum::<f32>() / 5.0];
    let (mut na, mut nb, mut den) = (0f32, 0f32, 0f32);
    for i in 0..5 {
        let s = sub(a[i], am);
        let d = sub(b[i], bm);
        na += s[0] * d[0] + s[1] * d[1];
        nb += s[0] * d[1] - s[1] * d[0];
        den += s[0] * s[0] + s[1] * s[1];
    }
    let (x, y) = (na / den.max(1e-6), nb / den.max(1e-6));
    [x, -y, bm[0] - (x * am[0] - y * am[1]), y, x, bm[1] - (y * am[0] + x * am[1])]
}

fn grid(p: &Pts) -> Vec<[f32; 2]> {
    let ex = sub(p[1], p[0]);
    let iod = len(ex).max(1e-3);
    let u = [ex[0] / iod, ex[1] / iod];
    let v = [-u[1], u[0]];
    let em = mid(p[0], p[1]);
    let mut out = Vec::new();
    for j in 0..9 {
        let b = -0.55 + j as f32 * 0.22;
        for i in 0..9 {
            let a = -0.85 + i as f32 * 0.2125;
            out.push([em[0] + (a * u[0] + b * v[0]) * iod, em[1] + (a * u[1] + b * v[1]) * iod]);
        }
    }
    out
}

fn fit(src: &[[f32; 2]], dst: &[[f32; 2]], scale: f32, c: [f32; 2]) -> Option<[f64; 8]> {
    let mut m = [[0f64; 9]; 8];
    for (p, q) in src.iter().zip(dst) {
        let (x, y) = (((p[0] - c[0]) / scale) as f64, ((p[1] - c[1]) / scale) as f64);
        let (u, v) = (((q[0] - c[0]) / scale) as f64, ((q[1] - c[1]) / scale) as f64);
        for (r, b) in [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)] {
            for i in 0..8 {
                for j in 0..8 {
                    m[i][j] += r[i] * r[j];
                }
                m[i][8] += r[i] * b;
            }
        }
    }
    solve(m)
}

fn residual(h: &[f64; 8], p: [f32; 2], q: [f32; 2], scale: f32, c: [f32; 2]) -> f32 {
    let (x, y) = (((p[0] - c[0]) / scale) as f64, ((p[1] - c[1]) / scale) as f64);
    let w = h[6] * x + h[7] * y + 1.0;
    if w.abs() < 1e-6 {
        return f32::MAX;
    }
    let u = (h[0] * x + h[1] * y + h[2]) / w;
    let v = (h[3] * x + h[4] * y + h[5]) / w;
    let (du, dv) = (u - ((q[0] - c[0]) / scale) as f64, v - ((q[1] - c[1]) / scale) as f64);
    ((du * du + dv * dv).sqrt()) as f32
}

pub fn planarity(a: &Gray, pa: &Pts, b: &Gray, pb: &Pts) -> Option<f32> {
    const P: i32 = 5;
    const R: i32 = 8;
    let s = similar(pa, pb);
    let iod = len(sub(pb[1], pb[0])).max(1e-3);
    let (mut src, mut dst) = (Vec::new(), Vec::new());
    let n = ((2 * P + 1) * (2 * P + 1)) as f32;
    for p in grid(pa) {
        let (cx, cy) = (p[0].round() as i32, p[1].round() as i32);
        if cx - P < 0 || cy - P < 0 || cx + P >= a.w as i32 || cy + P >= a.h as i32 {
            continue;
        }
        let mut patch = Vec::with_capacity(n as usize);
        for dy in -P..=P {
            for dx in -P..=P {
                patch.push(a.at(cx + dx, cy + dy));
            }
        }
        let mean = patch.iter().sum::<f32>() / n;
        patch.iter_mut().for_each(|v| *v -= mean);
        let var = patch.iter().map(|v| v * v).sum::<f32>() / n;
        if var < 25.0 {
            continue;
        }
        let q = [s[0] * cx as f32 + s[1] * cy as f32 + s[2], s[3] * cx as f32 + s[4] * cy as f32 + s[5]];
        let (qx, qy) = (q[0].round() as i32, q[1].round() as i32);
        if qx - P - R < 0 || qy - P - R < 0 || qx + P + R >= b.w as i32 || qy + P + R >= b.h as i32 {
            continue;
        }
        let side = (2 * R + 1) as usize;
        let mut cost = vec![f32::MAX; side * side];
        let mut best = (f32::MAX, 0i32, 0i32);
        for oy in -R..=R {
            for ox in -R..=R {
                let mut sum = 0f32;
                let mut vals = [0f32; 121];
                let mut k = 0;
                for dy in -P..=P {
                    for dx in -P..=P {
                        let v = b.at(qx + ox + dx, qy + oy + dy);
                        vals[k] = v;
                        sum += v;
                        k += 1;
                    }
                }
                let m = sum / n;
                let c: f32 = patch.iter().zip(vals.iter()).map(|(x, y)| (x - (y - m)) * (x - (y - m))).sum();
                cost[((oy + R) as usize) * side + (ox + R) as usize] = c;
                if c < best.0 {
                    best = (c, ox, oy);
                }
            }
        }
        if best.0 / (n * var) > 0.35 || best.1.abs() == R || best.2.abs() == R {
            continue;
        }
        let at = |ox: i32, oy: i32| cost[((oy + R) as usize) * side + (ox + R) as usize];
        let sub = |l: f32, c: f32, r: f32| {
            let d = l - 2.0 * c + r;
            if d.abs() < 1e-6 { 0.0 } else { (0.5 * (l - r) / d).clamp(-0.5, 0.5) }
        };
        let fx = sub(at(best.1 - 1, best.2), best.0, at(best.1 + 1, best.2));
        let fy = sub(at(best.1, best.2 - 1), best.0, at(best.1, best.2 + 1));
        src.push([cx as f32, cy as f32]);
        dst.push([(qx + best.1) as f32 + fx, (qy + best.2) as f32 + fy]);
    }
    if src.len() < 16 {
        return None;
    }
    let c = mid(pa[0], pa[1]);
    let h = fit(&src, &dst, iod, c)?;
    let mut r: Vec<(f32, usize)> = src.iter().zip(&dst).enumerate().map(|(i, (p, q))| (residual(&h, *p, *q, iod, c), i)).collect();
    r.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let keep: Vec<usize> = r.iter().take(r.len() * 85 / 100).map(|x| x.1).collect();
    let (s2, d2): (Vec<[f32; 2]>, Vec<[f32; 2]>) = keep.iter().map(|i| (src[*i], dst[*i])).unzip();
    let h = fit(&s2, &d2, iod, c)?;
    let mut r: Vec<f32> = s2.iter().zip(&d2).map(|(p, q)| residual(&h, *p, *q, iod, c)).collect();
    r.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    Some(r[r.len() * 3 / 4])
}

#[derive(Default, Clone)]
pub struct Live {
    recent: Vec<(Gray, Pts)>,
    pub scores: Vec<f32>,
    pub depth: Vec<f32>,
    pub motion: f32,
    frames: usize,
}

impl Live {
    pub fn add(&mut self, img: &Image, p: &Pts) {
        let g = Gray::of(img);
        self.frames += 1;
        let mut tried = 0;
        let mut own = Vec::new();
        for (q, pq) in self.recent.iter().rev() {
            let m = moved(pq, p);
            self.motion = self.motion.max(m);
            if m >= 0.03 && tried < 3 {
                tried += 1;
                if let Some(s) = planarity(q, pq, &g, p) {
                    self.scores.push(s);
                    own.push(s);
                }
            }
        }
        if !own.is_empty() {
            own.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            self.depth.push(own[(own.len() - 1) / 2]);
        }
        self.recent.push((g, *p));
        if self.recent.len() > 8 {
            self.recent.remove(0);
        }
    }

    pub fn ok(&self) -> bool {
        self.rule(LIVE_FRAMES, LIVE_OVER, LIVE_SHARE)
    }

    pub fn rule(&self, frames: usize, over: usize, share: f32) -> bool {
        let n = self.depth.len();
        let good = self.depth.iter().filter(|s| **s >= LIVE).count();
        n >= frames && good >= over && good as f32 >= share * n as f32
    }

    pub fn still(&self) -> bool {
        self.frames >= 6 && self.motion < STILL
    }
}

#[derive(Debug, Clone, Default)]
pub struct Nod {
    base: Option<(f32, f32)>,
    down: Option<Duration>,
    done: bool,
}

impl Nod {
    pub fn add(&mut self, at: Duration, yaw: f32, pitch: f32) {
        if self.done {
            return;
        }
        let Some((by, bp)) = self.base else {
            self.base = Some((yaw, pitch));
            return;
        };
        let (dy, dp) = (yaw - by, pitch - bp);
        match self.down {
            None => {
                if dp >= NOD && dy.abs() <= NOD_YAW {
                    self.down = Some(at);
                } else if dp.abs() < NOD * 0.35 && dy.abs() <= NOD_YAW {
                    self.base = Some((by * 0.8 + yaw * 0.2, bp * 0.8 + pitch * 0.2));
                } else if dy.abs() > NOD_YAW || dp <= -NOD {
                    self.base = Some((yaw, pitch));
                }
            }
            Some(since) => {
                if dy.abs() > NOD_YAW || at.saturating_sub(since) > NOD_TIME {
                    self.base = Some((yaw, pitch));
                    self.down = None;
                } else if dp <= NOD * 0.4 {
                    self.done = true;
                }
            }
        }
    }

    pub fn done(&self) -> bool {
        self.done
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Templates {
    pub angles: BTreeMap<String, Vec<Vec<f32>>>,
    #[serde(default)]
    pub learned: Vec<Vec<f32>>,
    #[serde(default)]
    pub enrolled: i64,
}

impl Templates {
    pub fn all(&self) -> impl Iterator<Item = &Vec<f32>> {
        self.angles.values().flatten().chain(self.learned.iter())
    }

    pub fn best(&self, v: &[f32]) -> f32 {
        self.all().map(|t| cosine(t, v)).fold(-1.0, f32::max)
    }

    pub fn merged(people: &[Templates]) -> Templates {
        let mut t = Templates::default();
        for (i, p) in people.iter().enumerate() {
            t.angles.insert(format!("person{i}"), p.all().cloned().collect());
        }
        t
    }

    pub fn learn(&mut self, v: &[f32]) -> bool {
        let s = self.best(v);
        if !(MATCH..0.9).contains(&s) {
            return false;
        }
        self.learned.push(v.to_vec());
        if self.learned.len() > 20 {
            self.learned.remove(0);
        }
        true
    }
}

pub struct Models {
    det: Model,
    rec: Model,
    names: Vec<String>,
}

fn similarity(src: &Pts) -> [f32; 6] {
    let sm = [src.iter().map(|p| p[0]).sum::<f32>() / 5.0, src.iter().map(|p| p[1]).sum::<f32>() / 5.0];
    let dm = [REF.iter().map(|p| p[0]).sum::<f32>() / 5.0, REF.iter().map(|p| p[1]).sum::<f32>() / 5.0];
    let (mut na, mut nb, mut den) = (0f32, 0f32, 0f32);
    for i in 0..5 {
        let s = sub(src[i], sm);
        let d = sub(REF[i], dm);
        na += s[0] * d[0] + s[1] * d[1];
        nb += s[0] * d[1] - s[1] * d[0];
        den += s[0] * s[0] + s[1] * s[1];
    }
    let den = den.max(1e-6);
    let (a, b) = (na / den, nb / den);
    let tx = dm[0] - (a * sm[0] - b * sm[1]);
    let ty = dm[1] - (b * sm[0] + a * sm[1]);
    [a, -b, tx, b, a, ty]
}

pub fn align(img: &Image, pts: &Pts) -> Image {
    let m = similarity(pts);
    let (a, b) = (m[0], m[3]);
    let k = (a * a + b * b).max(1e-9);
    let mut rgb = vec![0u8; 112 * 112 * 3];
    for y in 0..112 {
        for x in 0..112 {
            let (qx, qy) = (x as f32 - m[2], y as f32 - m[5]);
            let sx = (a * qx + b * qy) / k;
            let sy = (-b * qx + a * qy) / k;
            let i = (y * 112 + x) * 3;
            if sx < -1.0 || sy < -1.0 || sx > img.w as f32 || sy > img.h as f32 {
                continue;
            }
            let p = img.sample(sx, sy);
            rgb[i] = p[0].round() as u8;
            rgb[i + 1] = p[1].round() as u8;
            rgb[i + 2] = p[2].round() as u8;
        }
    }
    Image { w: 112, h: 112, rgb }
}

pub fn sharpness(img: &Image) -> f32 {
    let g = |x: usize, y: usize| {
        let p = img.px(x, y);
        0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
    };
    let (x0, x1, y0, y1) = (img.w / 7, img.w - img.w / 7, img.h / 7, img.h - img.h / 7);
    let mut vals = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0.max(1)..y1.min(img.h - 1) {
        for x in x0.max(1)..x1.min(img.w - 1) {
            vals.push(g(x - 1, y) + g(x + 1, y) + g(x, y - 1) + g(x, y + 1) - 4.0 * g(x, y));
        }
    }
    if vals.is_empty() {
        return 0.0;
    }
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / vals.len() as f32
}

impl Models {
    pub fn load(det: &Path, rec: &Path, threads: i32) -> Result<Self> {
        let det = Model::new(&det.to_string_lossy(), threads).map_err(|e| anyhow!("The face finder couldn't load: {e}"))?;
        let rec = Model::new(&rec.to_string_lossy(), threads).map_err(|e| anyhow!("The face matcher couldn't load: {e}"))?;
        let names = det.output_names();
        Ok(Self { det, rec, names })
    }

    pub fn detect(&self, img: &Image) -> Result<Vec<Face>> {
        self.detect_in(img, (0.0, 0.0, img.w as f32, img.h as f32), SIZE)
    }

    pub fn detect_in(&self, img: &Image, region: (f32, f32, f32, f32), size: usize) -> Result<Vec<Face>> {
        let (rx, ry) = (region.0.max(0.0), region.1.max(0.0));
        let (rw, rh) = ((region.2).min(img.w as f32 - rx), (region.3).min(img.h as f32 - ry));
        if img.w == 0 || img.h == 0 || rw < 8.0 || rh < 8.0 {
            return Ok(Vec::new());
        }
        let scale = size as f32 / rw.max(rh);
        let (nw, nh) = (((rw * scale).round() as usize).min(size), ((rh * scale).round() as usize).min(size));
        let plane = size * size;
        let mut data = vec![0f32; 3 * plane];
        let direct = (scale - 1.0).abs() < 1e-6 && rx == 0.0 && ry == 0.0;
        for y in 0..nh {
            for x in 0..nw {
                let p = if direct {
                    let q = img.px(x, y);
                    [q[0] as f32, q[1] as f32, q[2] as f32]
                } else {
                    img.sample(rx + (x as f32 + 0.5) / scale - 0.5, ry + (y as f32 + 0.5) / scale - 0.5)
                };
                let i = y * size + x;
                data[i] = p[2];
                data[plane + i] = p[1];
                data[2 * plane + i] = p[0];
            }
        }
        let outs = self.det.run_all(&mut data, &[1, 3, size as i64, size as i64]).map_err(|e| anyhow!("{e}"))?;
        let mut faces = nms(decode(&outs, &self.names, scale, DETECT * 0.8, size));
        for f in &mut faces {
            f.x += rx;
            f.y += ry;
            for p in &mut f.pts {
                p[0] += rx;
                p[1] += ry;
            }
        }
        Ok(faces)
    }

    pub fn embed(&self, img: &Image, f: &Face) -> Result<(Vec<f32>, f32)> {
        let crop = align(img, &f.pts);
        let sharp = sharpness(&crop);
        let plane = 112 * 112;
        let mut data = vec![0f32; 3 * plane];
        for i in 0..plane {
            data[i] = crop.rgb[i * 3] as f32;
            data[plane + i] = crop.rgb[i * 3 + 1] as f32;
            data[2 * plane + i] = crop.rgb[i * 3 + 2] as f32;
        }
        let (v, _) = self.rec.run(&mut data, &[1, 3, 112, 112]).map_err(|e| anyhow!("{e}"))?;
        Ok((unit(v), sharp))
    }
}

pub fn decode(outs: &[(Vec<f32>, Vec<i64>)], names: &[String], scale: f32, thr: f32, size: usize) -> Vec<Face> {
    let mut faces = Vec::new();
    for stride in [8usize, 16, 32] {
        let get = |p: &str| names.iter().position(|n| *n == format!("{p}_{stride}")).and_then(|i| outs.get(i)).map(|o| &o.0);
        let (Some(cls), Some(obj), Some(bbox), Some(kps)) = (get("cls"), get("obj"), get("bbox"), get("kps")) else { continue };
        let cols = size / stride;
        let n = (size / stride) * cols;
        let s = stride as f32;
        for idx in 0..n.min(cls.len()).min(obj.len()).min(bbox.len() / 4).min(kps.len() / 10) {
            let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
            if score < thr {
                continue;
            }
            let (r, c) = ((idx / cols) as f32, (idx % cols) as f32);
            let cx = (c + bbox[idx * 4]) * s;
            let cy = (r + bbox[idx * 4 + 1]) * s;
            let w = bbox[idx * 4 + 2].exp() * s;
            let h = bbox[idx * 4 + 3].exp() * s;
            let mut pts = [[0f32; 2]; 5];
            for (k, p) in pts.iter_mut().enumerate() {
                *p = [(kps[idx * 10 + 2 * k] + c) * s / scale, (kps[idx * 10 + 2 * k + 1] + r) * s / scale];
            }
            faces.push(Face { score, x: (cx - w / 2.0) / scale, y: (cy - h / 2.0) / scale, w: w / scale, h: h / scale, pts });
        }
    }
    faces
}

pub fn nms(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep: Vec<Face> = Vec::new();
    for f in faces {
        if keep.len() >= 20 {
            break;
        }
        if keep.iter().all(|k| k.iou(&f) < 0.3) {
            keep.push(f);
        }
    }
    keep
}

pub trait Frames: Send {
    fn next(&mut self) -> Result<Option<Image>>;
}

pub fn jpeg_rgb(data: &[u8]) -> Option<Image> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB));
    let px = d.decode().ok()?;
    let info = d.info()?;
    let (w, h) = (info.width as usize, info.height as usize);
    (px.len() >= w * h * 3).then_some(Image { w, h, rgb: px })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn yuyv_rgb(w: usize, h: usize, data: &[u8]) -> Image {
    let mut rgb = vec![0u8; w * h * 3];
    let clamp = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    for (i, q) in data.as_chunks::<4>().0.iter().take(w * h / 2).enumerate() {
        let (y0, u, y1, v) = (q[0] as f32, q[1] as f32 - 128.0, q[2] as f32, q[3] as f32 - 128.0);
        for (k, y) in [y0, y1].into_iter().enumerate() {
            let o = (i * 2 + k) * 3;
            rgb[o] = clamp(y + 1.402 * v);
            rgb[o + 1] = clamp(y - 0.344 * u - 0.714 * v);
            rgb[o + 2] = clamp(y + 1.772 * u);
        }
    }
    Image { w, h, rgb }
}

pub fn preview(img: &Image) -> Option<Vec<u8>> {
    let (tw, th) = (320usize, (320 * img.h / img.w.max(1)).clamp(1, 320));
    let mut small = vec![0u8; tw * th * 3];
    for y in 0..th {
        for x in 0..tw {
            let p = img.px((tw - 1 - x) * img.w / tw, y * img.h / th);
            small[(y * tw + x) * 3..(y * tw + x) * 3 + 3].copy_from_slice(&p);
        }
    }
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 70).encode(&small, tw as u16, th as u16, jpeg_encoder::ColorType::Rgb).ok()?;
    Some(out)
}

pub struct Folder {
    files: Vec<std::path::PathBuf>,
    at: usize,
    last: Instant,
}

impl Folder {
    pub fn open(dir: &Path) -> Result<Self> {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("jpg" | "jpeg" | "png"))).collect();
        files.sort();
        if files.is_empty() {
            return Err(anyhow!("No camera was found. Connect a camera, then try again."));
        }
        Ok(Self { files, at: 0, last: Instant::now() - Duration::from_secs(1) })
    }
}

impl Frames for Folder {
    fn next(&mut self) -> Result<Option<Image>> {
        let Some(p) = self.files.get(self.at) else { return Ok(None) };
        self.at += 1;
        let wait = Duration::from_millis(66).saturating_sub(self.last.elapsed());
        std::thread::sleep(wait);
        self.last = Instant::now();
        let bytes = std::fs::read(p)?;
        let img = if p.extension().and_then(|e| e.to_str()) == Some("png") { decode_png(&bytes)? } else { jpeg_rgb(&bytes).ok_or_else(|| anyhow!("a bench frame is not a picture"))? };
        Ok(Some(img))
    }
}

#[cfg(target_os = "linux")]
pub struct Camera {
    stream: v4l::io::mmap::Stream<'static>,
    mjpg: bool,
    w: usize,
    h: usize,
    period: Duration,
    _dev: v4l::Device,
}

#[cfg(target_os = "linux")]
const EXPOSURE_AUTO_PRIORITY: u32 = 0x009a_0903;

pub fn pick_mode(modes: &[(u32, u32, f64)]) -> Option<usize> {
    let off = |w: u32, h: u32| (w as i64 - 640).abs() + (h as i64 - 480).abs();
    modes
        .iter()
        .enumerate()
        .filter(|(_, (w, h, _))| *w >= 320 && *h >= 240)
        .min_by(|(_, a), (_, b)| off(a.0, a.1).cmp(&off(b.0, b.1)).then(b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)))
        .map(|(i, _)| i)
}

#[cfg(target_os = "linux")]
pub fn fastest(intervals: &[v4l::frameinterval::FrameIntervalEnum]) -> Option<v4l::Fraction> {
    use v4l::frameinterval::FrameIntervalEnum;
    let secs = |f: &v4l::Fraction| if f.denominator == 0 { f64::MAX } else { f.numerator as f64 / f.denominator as f64 };
    intervals
        .iter()
        .map(|i| match i {
            FrameIntervalEnum::Discrete(f) => *f,
            FrameIntervalEnum::Stepwise(s) => s.min,
        })
        .filter(|f| f.numerator > 0 && f.denominator > 0)
        .min_by(|a, b| secs(a).partial_cmp(&secs(b)).unwrap_or(std::cmp::Ordering::Equal))
}

#[cfg(target_os = "linux")]
fn monotonic_now() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

#[cfg(target_os = "linux")]
impl Camera {
    pub fn open() -> Result<Self> {
        use v4l::buffer::Type;
        use v4l::video::Capture;
        let mut list: Vec<String> = std::fs::read_dir("/dev")?.flatten().map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.starts_with("/dev/video")).collect();
        list.sort();
        let path = list
            .into_iter()
            .find(|p| v4l::Device::with_path(p).ok().and_then(|d| d.query_caps().ok()).is_some_and(|c| c.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE)))
            .ok_or_else(|| anyhow!("No camera was found. Connect a camera, then try again."))?;
        let dev = v4l::Device::with_path(&path).map_err(|_| anyhow!("The camera can't be opened. Check the Camera permission."))?;
        let mut fmt = dev.format()?;
        fmt.width = 640;
        fmt.height = 480;
        fmt.fourcc = v4l::FourCC::new(b"MJPG");
        let fmt = match dev.set_format(&fmt) {
            Ok(f) if f.fourcc == v4l::FourCC::new(b"MJPG") => f,
            _ => {
                let mut f = dev.format()?;
                f.width = 640;
                f.height = 480;
                f.fourcc = v4l::FourCC::new(b"YUYV");
                dev.set_format(&f).map_err(|e| anyhow!("The camera format could not be set: {e}"))?
            }
        };
        let mut period = Duration::from_millis(33);
        if let Ok(list) = dev.enum_frameintervals(fmt.fourcc, fmt.width, fmt.height)
            && let Some(best) = fastest(&list.into_iter().map(|i| i.interval).collect::<Vec<_>>())
            && let Ok(p) = dev.set_params(&v4l::video::capture::Parameters::new(best))
            && p.interval.denominator > 0
        {
            period = Duration::from_secs_f64(p.interval.numerator as f64 / p.interval.denominator as f64);
        }
        let _ = dev.set_control(v4l::control::Control { id: EXPOSURE_AUTO_PRIORITY, value: v4l::control::Value::Boolean(false) });
        let stream = v4l::io::mmap::Stream::with_buffers(&dev, Type::VideoCapture, 2).map_err(|e| anyhow!("The camera could not start: {e}"))?;
        Ok(Self { stream, mjpg: fmt.fourcc == v4l::FourCC::new(b"MJPG"), w: fmt.width as usize, h: fmt.height as usize, period, _dev: dev })
    }
}

#[cfg(target_os = "linux")]
impl Frames for Camera {
    fn next(&mut self) -> Result<Option<Image>> {
        use v4l::io::traits::CaptureStream;
        let mut skipped = 0;
        for _ in 0..7 {
            let (buf, meta) = self.stream.next().map_err(|_| anyhow!("The camera stopped sending pictures."))?;
            let taken = Duration::new(meta.timestamp.sec.max(0) as u64, (meta.timestamp.usec.clamp(0, 999_999) as u32) * 1000);
            let age = monotonic_now().saturating_sub(taken);
            if skipped < 2 && !taken.is_zero() && age > self.period.mul_f32(1.5) && age < Duration::from_secs(5) {
                skipped += 1;
                continue;
            }
            let data = &buf[..(meta.bytesused as usize).min(buf.len())];
            let img = if self.mjpg { jpeg_rgb(data) } else { (data.len() >= self.w * self.h * 2).then(|| yuyv_rgb(self.w, self.h, data)) };
            if img.is_some() {
                return Ok(img);
            }
        }
        Err(anyhow!("The camera's pictures can't be read."))
    }
}

#[cfg(windows)]
pub struct Camera {
    reader: windows::Win32::Media::MediaFoundation::IMFSourceReader,
    w: usize,
    h: usize,
    stride: i32,
}

#[cfg(windows)]
unsafe impl Send for Camera {}

#[cfg(windows)]
impl Camera {
    pub fn open() -> Result<Self> {
        use windows::Win32::Media::MediaFoundation::*;
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| anyhow!("The camera system didn't start: {e}"))?;
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 1)?;
            let attrs = attrs.ok_or_else(|| anyhow!("The camera system didn't start."))?;
            attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFEnumDeviceSources(&attrs, &mut list, &mut count)?;
            if count == 0 || list.is_null() {
                if !list.is_null() {
                    CoTaskMemFree(Some(list as *const _));
                }
                return Err(anyhow!("No camera was found. Connect a camera, then try again."));
            }
            let devices = std::slice::from_raw_parts_mut(list, count as usize);
            let first = devices[0].take();
            for d in devices.iter_mut() {
                d.take();
            }
            CoTaskMemFree(Some(list as *const _));
            let first = first.ok_or_else(|| anyhow!("No camera was found. Connect a camera, then try again."))?;
            let source: IMFMediaSource = first.ActivateObject().map_err(|_| anyhow!("The camera can't be opened. Check Camera access in Privacy settings."))?;
            let mut ra: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut ra, 2)?;
            let ra = ra.ok_or_else(|| anyhow!("The camera system didn't start."))?;
            ra.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            let _ = ra.SetUINT32(&MF_LOW_LATENCY, 1);
            let reader = MFCreateSourceReaderFromMediaSource(&source, &ra)?;
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            let mut offered: Vec<(u32, u32, f64, IMFMediaType)> = Vec::new();
            for i in 0..256u32 {
                let Ok(t) = reader.GetNativeMediaType(stream, i) else { break };
                let size = t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
                let rate = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
                let (num, den) = ((rate >> 32) as u32, (rate & 0xffff_ffff) as u32);
                offered.push(((size >> 32) as u32, (size & 0xffff_ffff) as u32, if den == 0 { 0.0 } else { num as f64 / den as f64 }, t));
            }
            if let Some(i) = pick_mode(&offered.iter().map(|(w, h, f, _)| (*w, *h, *f)).collect::<Vec<_>>()) {
                let _ = reader.SetCurrentMediaType(stream, None, &offered[i].3);
            }
            let want = MFCreateMediaType()?;
            want.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            want.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(stream, None, &want).map_err(|e| anyhow!("The camera format could not be set: {e}"))?;
            let now = reader.GetCurrentMediaType(stream)?;
            let size = now.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (w, h) = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
            let stride = now.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or((w * 4) as i32);
            if w == 0 || h == 0 {
                return Err(anyhow!("The camera reported no picture size."));
            }
            Ok(Self { reader, w, h, stride })
        }
    }
}

#[cfg(windows)]
impl Frames for Camera {
    fn next(&mut self) -> Result<Option<Image>> {
        use windows::Win32::Media::MediaFoundation::*;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        for _ in 0..60 {
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            unsafe { self.reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)) }.map_err(|_| anyhow!("The camera stopped sending pictures."))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Err(anyhow!("The camera stopped sending pictures."));
            }
            let Some(sample) = sample else { continue };
            let buf = unsafe { sample.ConvertToContiguousBuffer() }?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut len = 0u32;
            unsafe { buf.Lock(&mut ptr, None, Some(&mut len)) }?;
            let data = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
            let row = self.stride.unsigned_abs() as usize;
            let img = (row >= self.w * 4 && data.len() >= row * self.h).then(|| {
                let mut rgb = vec![0u8; self.w * self.h * 3];
                for y in 0..self.h {
                    let sy = if self.stride < 0 { self.h - 1 - y } else { y };
                    let line = &data[sy * row..sy * row + self.w * 4];
                    for x in 0..self.w {
                        let (b, g, r) = (line[x * 4], line[x * 4 + 1], line[x * 4 + 2]);
                        rgb[(y * self.w + x) * 3..(y * self.w + x) * 3 + 3].copy_from_slice(&[r, g, b]);
                    }
                }
                Image { w: self.w, h: self.h, rgb }
            });
            unsafe {
                let _ = buf.Unlock();
            }
            if img.is_some() {
                return Ok(img);
            }
        }
        Err(anyhow!("The camera's pictures can't be read."))
    }
}

#[cfg(windows)]
impl Drop for Camera {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Media::MediaFoundation::MFShutdown();
        }
    }
}

pub fn open_frames() -> Result<Box<dyn Frames>> {
    if std::env::var_os("SETU_BENCH").is_some()
        && let Some(dir) = std::env::var_os("SETU_FACE_FRAMES")
    {
        let dir = std::path::PathBuf::from(dir);
        let pick = std::fs::read_to_string(dir.join("use")).unwrap_or_default();
        return Ok(Box::new(Folder::open(&dir.join(pick.trim()))?));
    }
    #[cfg(any(target_os = "linux", windows))]
    {
        Ok(Box::new(Camera::open()?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(maccam::Camera::open()?))
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        Err(anyhow!("Setu can't use the camera on this system yet."))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Me(f32),
    NotMe,
    NoFace,
    Still,
    Blurry,
    NoNod,
    Cancelled,
}

impl Verdict {
    pub fn say(&self) -> &'static str {
        match self {
            Verdict::Me(_) => "That's you.",
            Verdict::NotMe => "That doesn't look like you.",
            Verdict::NoFace => "I couldn't see your face.",
            Verdict::Still => "That looked like a still picture. Move your head a little and try again.",
            Verdict::Blurry => "The picture was too blurry. Find some light and try again.",
            Verdict::NoNod => "I saw you, but not a nod.",
            Verdict::Cancelled => "Stopped.",
        }
    }
}

pub struct Tick<'a> {
    pub img: &'a Image,
    pub steps: &'a BTreeMap<String, bool>,
    pub message: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub nod: bool,
    pub no_face: Option<Duration>,
}

pub fn check(models: &Models, frames: &mut dyn Frames, t: &Templates, cancel: &AtomicBool, limit: Duration, tick: &mut dyn FnMut(Tick)) -> Result<(Verdict, Option<Vec<f32>>)> {
    check_with(models, frames, t, cancel, limit, Options::default(), tick)
}

pub const MARGIN: f32 = 0.06;

pub fn identify(people: &[Templates], v: &[f32]) -> Option<usize> {
    let mut scores: Vec<(usize, f32)> = people.iter().map(|p| p.best(v)).enumerate().collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (who, top) = *scores.first()?;
    let next = scores.get(1).map(|s| s.1).unwrap_or(-1.0);
    (top >= MATCH && top - next >= MARGIN).then_some(who)
}

pub fn check_with(models: &Models, frames: &mut dyn Frames, t: &Templates, cancel: &AtomicBool, limit: Duration, opts: Options, tick: &mut dyn FnMut(Tick)) -> Result<(Verdict, Option<Vec<f32>>)> {
    let start = Instant::now();
    let mut live = Live::default();
    let mut nod = Nod::default();
    let (mut matched, mut other, mut blurry, mut faces) = (0usize, 0usize, 0usize, 0usize);
    let mut best: Option<(f32, Vec<f32>)> = None;
    let steps = BTreeMap::new();
    let mut hint: Option<&str> = None;
    while start.elapsed() < limit {
        if cancel.load(Ordering::Relaxed) {
            return Ok((Verdict::Cancelled, None));
        }
        let Some(img) = frames.next()? else { break };
        let found: Vec<Face> = models.detect(&img)?.into_iter().filter(|f| f.score >= DETECT && f.eyes() >= MIN_EYES).take(4).collect();
        faces += found.len().min(1);
        for f in &found {
            let (v, sharp) = models.embed(&img, f)?;
            if sharp < SHARP {
                blurry += 1;
                continue;
            }
            let s = t.best(&v);
            if s >= MATCH {
                matched += 1;
                live.add(&img, &f.pts);
                let (yaw, pitch) = f.pose();
                nod.add(start.elapsed(), yaw, pitch);
                if best.as_ref().is_none_or(|(b, _)| s > *b) {
                    best = Some((s, v));
                }
            } else {
                other += 1;
            }
        }
        if matched >= 3 && live.ok() && (!opts.nod || nod.done()) {
            let (s, v) = best.take().unwrap_or((MATCH, Vec::new()));
            return Ok((Verdict::Me(s), Some(v)));
        }
        if matched >= 12 && live.still() {
            return Ok((Verdict::Still, None));
        }
        if faces == 0 && opts.no_face.is_some_and(|d| start.elapsed() >= d) {
            return Ok((Verdict::NoFace, None));
        }
        hint = if matched >= 3 && opts.nod && live.ok() {
            Some("Nod to approve.")
        } else if matched >= 3 {
            Some("Move your head a little.")
        } else if faces == 0 && start.elapsed() > Duration::from_secs(2) {
            Some("Look at the camera.")
        } else {
            hint
        };
        tick(Tick { img: &img, steps: &steps, message: hint });
    }
    Ok((
        if matched >= 3 && live.ok() {
            Verdict::NoNod
        } else if matched >= 3 {
            Verdict::Still
        } else if other > 0 {
            Verdict::NotMe
        } else if blurry > 0 {
            Verdict::Blurry
        } else {
            Verdict::NoFace
        },
        None,
    ))
}

fn side(base: (f32, f32), now: (f32, f32)) -> Option<&'static str> {
    let (dy, dp) = (now.0 - base.0, now.1 - base.1);
    if dy.abs() >= dp.abs() * 1.4 {
        if dy <= -YAW {
            return Some("right");
        }
        if dy >= YAW {
            return Some("left");
        }
    } else {
        if dp <= -PITCH {
            return Some("up");
        }
        if dp >= PITCH {
            return Some("down");
        }
    }
    None
}

pub fn enrol(models: &Models, frames: &mut dyn Frames, cancel: &AtomicBool, limit: Duration, tick: &mut dyn FnMut(Tick)) -> Result<Templates, String> {
    let start = Instant::now();
    let mut live = Live::default();
    let mut t = Templates::default();
    let mut steps: BTreeMap<String, bool> = SIDES.iter().map(|s| ((*s).to_owned(), false)).collect();
    let mut base: Option<(f32, f32)> = None;
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut message: Option<&str> = Some("Look at the camera.");
    while start.elapsed() < limit {
        if cancel.load(Ordering::Relaxed) {
            return Err("Stopped. Nothing was saved.".into());
        }
        let img = match frames.next() {
            Ok(Some(i)) => i,
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        };
        let found: Vec<Face> = models.detect(&img).map_err(|e| e.to_string())?.into_iter().filter(|f| f.score >= DETECT).collect();
        if found.len() > 1 {
            message = Some("I can see more than one person. I need only you.");
            tick(Tick { img: &img, steps: &steps, message });
            continue;
        }
        let Some(f) = found.first() else {
            message = Some("Look at the camera.");
            tick(Tick { img: &img, steps: &steps, message });
            continue;
        };
        if f.eyes() < MIN_EYES {
            message = Some("Come a little closer.");
            tick(Tick { img: &img, steps: &steps, message });
            continue;
        }
        let (v, sharp) = models.embed(&img, f).map_err(|e| e.to_string())?;
        if sharp < SHARP {
            message = Some("Hold still for a moment. It's a little blurry.");
            tick(Tick { img: &img, steps: &steps, message });
            continue;
        }
        let p = f.pose();
        let center: Vec<Vec<f32>> = t.angles.get("center").cloned().unwrap_or_default();
        if !center.is_empty() && center.iter().map(|c| cosine(c, &v)).fold(-1.0, f32::max) < SAME {
            message = Some("I need only you in front of the camera.");
            tick(Tick { img: &img, steps: &steps, message });
            continue;
        }
        live.add(&img, &f.pts);
        let slot = match base {
            None if p.0.abs() < 0.08 => {
                base = Some(p);
                Some("center")
            }
            None => {
                message = Some("Look straight at the camera.");
                None
            }
            Some(b) => match side(b, p) {
                Some(s) => Some(s),
                None if (p.0 - b.0).abs() < 0.04 && (p.1 - b.1).abs() < 0.03 => Some("center"),
                None => None,
            },
        };
        if let Some(s) = slot {
            let list = t.angles.entry(s.to_owned()).or_default();
            if list.len() < 3 && last.elapsed() >= Duration::from_millis(120) {
                list.push(v);
                last = Instant::now();
            }
            if list.len() >= 2 {
                steps.insert(s.to_owned(), true);
            }
            message = Some(match steps.iter().find(|(_, d)| !**d).map(|(k, _)| k.as_str()) {
                None => "All done.",
                Some("center") => "Look straight at the camera.",
                Some(_) => "Turn your head slowly: left, right, up and down.",
            });
        }
        tick(Tick { img: &img, steps: &steps, message });
        if steps.values().all(|d| *d) {
            if !live.ok() {
                return Err("That looked like a flat picture, not a face. Try again in front of the camera yourself.".into());
            }
            t.enrolled = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            return Ok(t);
        }
    }
    let left: Vec<&str> = steps.iter().filter(|(_, d)| !**d).map(|(k, _)| k.as_str()).collect();
    Err(if base.is_none() { "I couldn't see your face clearly. Sit in front of the camera with some light, then try again.".into() } else { format!("I didn't see you turn {}. Try again, turning slowly.", left.iter().filter(|s| **s != "center").cloned().collect::<Vec<_>>().join(", ")) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_mode_nearest_640_by_480_with_the_most_frames_wins() {
        let modes = [(1920, 1080, 30.0), (640, 480, 30.0), (640, 480, 60.0), (320, 240, 120.0), (160, 120, 30.0)];
        assert_eq!(pick_mode(&modes), Some(2));
        assert_eq!(pick_mode(&[(1280, 720, 30.0), (1920, 1080, 60.0)]), Some(0));
        assert_eq!(pick_mode(&[(160, 120, 30.0)]), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_fastest_frame_interval_is_chosen() {
        use v4l::frameinterval::{FrameIntervalEnum, Stepwise};
        let f = |n, d| v4l::Fraction::new(n, d);
        let got = fastest(&[FrameIntervalEnum::Discrete(f(1, 30)), FrameIntervalEnum::Discrete(f(1, 60)), FrameIntervalEnum::Discrete(f(1, 15))]).unwrap();
        assert_eq!((got.numerator, got.denominator), (1, 60));
        let step = fastest(&[FrameIntervalEnum::Stepwise(Stepwise { min: f(1, 90), max: f(1, 5), step: f(1, 90) })]).unwrap();
        assert_eq!(step.denominator, 90);
    }

    fn project(pts3: &[[f32; 3]; 5], yaw: f32, pitch: f32, tilt: f32, shift: [f32; 2], f: f32) -> Pts {
        let mut o = [[0f32; 2]; 5];
        for (i, p) in pts3.iter().enumerate() {
            let (x, y, z) = (p[0], p[1], p[2]);
            let (x1, z1) = (x * yaw.cos() + z * yaw.sin(), -x * yaw.sin() + z * yaw.cos());
            let (y2, z2) = (y * pitch.cos() - z1 * pitch.sin(), y * pitch.sin() + z1 * pitch.cos());
            let (x3, y3) = (x1 * tilt.cos() - y2 * tilt.sin(), x1 * tilt.sin() + y2 * tilt.cos());
            let d = f / (f - z2);
            o[i] = [320.0 + shift[0] + x3 * d, 240.0 + shift[1] + y3 * d];
        }
        o
    }

    const HEAD: [[f32; 3]; 5] = [[-32.0, -10.0, 0.0], [32.0, -10.0, 0.0], [0.0, 22.0, 24.0], [-24.0, 48.0, 2.0], [24.0, 48.0, 2.0]];

    fn texture() -> Image {
        let (w, h) = (640usize, 480usize);
        let mut rng = fastrand::Rng::with_seed(11);
        let raw: Vec<f32> = (0..w * h).map(|_| rng.f32() * 255.0).collect();
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        sum += raw[((y as i32 + dy).clamp(0, h as i32 - 1) as usize) * w + (x as i32 + dx).clamp(0, w as i32 - 1) as usize];
                    }
                }
                let v = (sum / 9.0) as u8;
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[v, v, v]);
            }
        }
        Image { w, h, rgb }
    }

    fn warp(img: &Image, back: impl Fn(f32, f32) -> (f32, f32)) -> Image {
        let mut rgb = vec![0u8; img.w * img.h * 3];
        for y in 0..img.h {
            for x in 0..img.w {
                let (sx, sy) = back(x as f32, y as f32);
                let p = img.sample(sx, sy);
                rgb[(y * img.w + x) * 3..(y * img.w + x) * 3 + 3].copy_from_slice(&[p[0] as u8, p[1] as u8, p[2] as u8]);
            }
        }
        Image { w: img.w, h: img.h, rgb }
    }

    const FACE: Pts = [[280.0, 220.0], [380.0, 220.0], [330.0, 270.0], [290.0, 320.0], [370.0, 320.0]];

    fn bump(x: f32, y: f32) -> f32 {
        let (dx, dy) = ((x - 330.0) / 150.0, (y - 260.0) / 190.0);
        let n = (-((x - 330.0).powi(2) + (y - 270.0).powi(2)) / (2.0 * 30f32.powi(2))).exp();
        110.0 * (1.0 - dx * dx - dy * dy).max(0.0).sqrt() + 50.0 * n
    }

    #[test]
    fn a_turning_head_shows_depth_and_a_photo_does_not() {
        let a = texture();
        let ga = Gray::of(&a);
        let g = [[1.02f32, 0.05, -14.0], [-0.03, 0.99, 6.0], [0.00012, -0.00008, 1.0]];
        let det = g[0][0] * (g[1][1] * g[2][2] - g[1][2] * g[2][1]) - g[0][1] * (g[1][0] * g[2][2] - g[1][2] * g[2][0]) + g[0][2] * (g[1][0] * g[2][1] - g[1][1] * g[2][0]);
        let inv = [
            [(g[1][1] * g[2][2] - g[1][2] * g[2][1]) / det, (g[0][2] * g[2][1] - g[0][1] * g[2][2]) / det, (g[0][1] * g[1][2] - g[0][2] * g[1][1]) / det],
            [(g[1][2] * g[2][0] - g[1][0] * g[2][2]) / det, (g[0][0] * g[2][2] - g[0][2] * g[2][0]) / det, (g[0][2] * g[1][0] - g[0][0] * g[1][2]) / det],
            [(g[1][0] * g[2][1] - g[1][1] * g[2][0]) / det, (g[0][1] * g[2][0] - g[0][0] * g[2][1]) / det, (g[0][0] * g[1][1] - g[0][1] * g[1][0]) / det],
        ];
        let map = |m: &[[f32; 3]; 3], x: f32, y: f32| {
            let w = m[2][0] * x + m[2][1] * y + m[2][2];
            ((m[0][0] * x + m[0][1] * y + m[0][2]) / w, (m[1][0] * x + m[1][1] * y + m[1][2]) / w)
        };
        let flat = warp(&a, |x, y| map(&g, x, y));
        let pf: Pts = FACE.map(|p| {
            let (x, y) = map(&inv, p[0], p[1]);
            [x, y]
        });
        let photo = planarity(&ga, &FACE, &Gray::of(&flat), &pf).unwrap();
        let k = 0.16f32;
        let turned = warp(&a, |u, v| {
            let mut x = u;
            for _ in 0..20 {
                x += 0.8 * (u - (x + k * bump(x, v)));
            }
            (x, v)
        });
        let pt: Pts = FACE.map(|p| [p[0] + k * bump(p[0], p[1]), p[1]]);
        let head = planarity(&ga, &FACE, &Gray::of(&turned), &pt).unwrap();
        assert!(photo < LIVE / 2.0, "photo {photo}");
        assert!(head > LIVE * 1.5, "head {head}");
        let mut live = Live::default();
        for _ in 0..8 {
            live.add(&a, &FACE);
        }
        assert!(live.still() && !live.ok());
    }

    #[test]
    fn pose_follows_the_head() {
        let front = pose(&project(&HEAD, 0.0, 0.0, 0.0, [0.0, 0.0], 500.0));
        let turned = pose(&project(&HEAD, 0.35, 0.0, 0.0, [0.0, 0.0], 500.0));
        let up = pose(&project(&HEAD, 0.0, -0.3, 0.0, [0.0, 0.0], 500.0));
        assert!(front.0.abs() < 0.02);
        assert!(side(front, turned).is_some(), "{front:?} {turned:?}");
        assert!(side(front, up).is_some(), "{front:?} {up:?}");
        assert_ne!(side(front, turned), side(front, up));
        assert_eq!(side(front, front), None);
    }

    #[test]
    fn a_nod_is_down_and_back_without_turning() {
        let ms = Duration::from_millis;
        let feed = |seq: &[(u64, f32, f32)]| {
            let mut n = Nod::default();
            for (t, yaw, pitch) in seq {
                n.add(ms(*t), *yaw, *pitch);
            }
            n.done()
        };
        assert!(feed(&[(0, 0.0, 0.50), (70, 0.0, 0.51), (140, 0.01, 0.54), (210, 0.01, 0.58), (280, 0.0, 0.57), (350, 0.0, 0.53), (420, 0.0, 0.50)]));
        assert!(!feed(&[(0, 0.0, 0.50), (70, 0.0, 0.51), (140, 0.0, 0.50), (210, 0.01, 0.52), (280, 0.0, 0.50)]), "holding still is not a nod");
        assert!(!feed(&[(0, 0.0, 0.50), (70, 0.0, 0.54), (140, 0.0, 0.58), (210, 0.0, 0.58), (2600, 0.0, 0.58), (2700, 0.0, 0.50)]), "looking down for long, then up, is not a nod");
        assert!(!feed(&[(0, 0.0, 0.50), (70, 0.05, 0.53), (140, 0.12, 0.57), (210, 0.15, 0.58), (280, 0.1, 0.52), (350, 0.0, 0.50)]), "turning the head away is not a nod");
        assert!(!feed(&[(0, 0.0, 0.50), (70, 0.0, 0.46), (140, 0.0, 0.42), (210, 0.0, 0.46), (280, 0.0, 0.50)]), "looking up and back is not a nod");
        assert!(feed(&[(0, 0.2, 0.40), (70, 0.2, 0.40), (140, 0.2, 0.46), (210, 0.21, 0.47), (280, 0.2, 0.41)]), "a nod with the head already turned a little still counts");
    }

    #[test]
    fn templates_learn_only_close_matches() {
        let mut t = Templates::default();
        let a = unit(vec![1.0, 0.0, 0.0]);
        t.angles.insert("center".into(), vec![a.clone()]);
        assert!(!t.learn(&a));
        assert!(t.learn(&unit(vec![1.0, 0.8, 0.0])));
        assert!(!t.learn(&unit(vec![0.0, 0.0, 1.0])));
        assert_eq!(t.learned.len(), 1);
        assert!((t.best(&a) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn similarity_maps_reference_points_onto_themselves() {
        let m = similarity(&REF);
        assert!((m[0] - 1.0).abs() < 1e-4 && m[3].abs() < 1e-4 && m[2].abs() < 1e-3 && m[5].abs() < 1e-3, "{m:?}");
    }

    #[test]
    fn models_find_and_match_faces() {
        let Some(dir) = std::env::var_os("SETU_FACE_MODELS") else { return };
        let dir = std::path::PathBuf::from(dir);
        let m = Models::load(&dir.join("face_detection_yunet_2023mar.onnx"), &dir.join("face_recognition_sface_2021dec.onnx"), 2).unwrap();
        let load = |n: &str| jpeg_rgb(&std::fs::read(dir.join(n)).unwrap()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".jpg")).collect();
        names.sort();
        for n in &names {
            let img = load(n);
            let t = Instant::now();
            let faces: Vec<Face> = m.detect(&img).unwrap().into_iter().filter(|f| f.score >= DETECT).collect();
            let ms = t.elapsed().as_millis();
            for f in &faces {
                let (_, sharp) = m.embed(&img, f).unwrap();
                println!("{n}: {}x{} face {:.2} at {:.0},{:.0} {:.0}x{:.0} eyes {:.0} pose {:?} sharp {sharp:.0} pts {:?} in {ms} ms", img.w, img.h, f.score, f.x, f.y, f.w, f.h, f.eyes(), f.pose(), f.pts);
            }
        }
        let demo = load("sface_demo.jpg");
        let faces: Vec<Face> = m.detect(&demo).unwrap().into_iter().filter(|f| f.score >= DETECT).collect();
        let left = faces.iter().find(|f| f.x < 500.0).unwrap();
        let (a, _) = m.embed(&demo, left).unwrap();
        let mut scores: Vec<(f32, f32)> = faces.iter().filter(|f| f.x > 512.0).map(|f| (f.x, cosine(&a, &m.embed(&demo, f).unwrap().0))).collect();
        scores.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        println!("demo scores {scores:?}");
        let same = scores.iter().map(|s| s.1).fold(-1.0, f32::max);
        assert!(same >= MATCH, "{scores:?}");
        assert_eq!(scores.iter().filter(|s| s.1 >= MATCH).count(), 1, "{scores:?}");
        let lena = load("lena.jpg");
        let lf = m.detect(&lena).unwrap().into_iter().find(|f| f.score >= DETECT).unwrap();
        assert!(cosine(&a, &m.embed(&lena, &lf).unwrap().0) < MATCH);
        let Some(seq) = std::env::var_os("SETU_FACE_SEQ") else { return };
        let seq = std::path::PathBuf::from(seq);
        let stop = AtomicBool::new(false);
        let mut said = Vec::new();
        let t = enrol(&m, &mut Folder::open(&seq.join("enrol_me")).unwrap(), &stop, Duration::from_secs(60), &mut |k: Tick| {
            if let Some(s) = k.message
                && said.last() != Some(&s.to_owned())
            {
                said.push(s.to_owned());
            }
        });
        println!("enrol said {said:?}");
        let t = t.unwrap();
        println!("enrolled {:?}", t.angles.iter().map(|(k, v)| (k.clone(), v.len())).collect::<Vec<_>>());
        for (name, want) in [("check_me", "me"), ("check_other", "not"), ("check_photo", "still"), ("none", "none")] {
            let mut f = Folder::open(&seq.join(name)).unwrap();
            let (v, _) = check(&m, &mut f, &t, &stop, Duration::from_secs(20), &mut |_| {}).unwrap();
            println!("{name}: {v:?}");
            match want {
                "me" => assert!(matches!(v, Verdict::Me(_)), "{name} {v:?}"),
                "not" => assert_eq!(v, Verdict::NotMe, "{name}"),
                "still" => assert_eq!(v, Verdict::Still, "{name}"),
                _ => assert_eq!(v, Verdict::NoFace, "{name}"),
            }
        }
        let mut two = Vec::new();
        let r = enrol(&m, &mut Folder::open(&seq.join("enrol_two")).unwrap(), &stop, Duration::from_secs(20), &mut |k: Tick| two.extend(k.message.map(str::to_owned)));
        assert!(r.is_err() && two.iter().any(|s| s.contains("more than one person")), "{r:?} {two:?}");
    }

    #[test]
    fn depth_numbers() {
        let (Some(dir), Some(seq)) = (std::env::var_os("SETU_FACE_MODELS"), std::env::var_os("SETU_FACE_SEQ")) else { return };
        let dir = std::path::PathBuf::from(dir);
        let m = Models::load(&dir.join("face_detection_yunet_2023mar.onnx"), &dir.join("face_recognition_sface_2021dec.onnx"), 2).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&seq).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "none" && n != "enrol_two").collect();
        names.sort();
        for name in &names {
            let mut f = Folder::open(&std::path::PathBuf::from(&seq).join(name)).unwrap();
            let mut live = Live::default();
            let mut times = Vec::new();
            let rules = [(LIVE_FRAMES, LIVE_OVER, LIVE_SHARE)];
            let mut first: Vec<Option<usize>> = vec![None; rules.len()];
            let mut seen = 0;
            while let Some(img) = f.next().unwrap() {
                let t = Instant::now();
                seen += 1;
                if let Some(face) = m.detect(&img).unwrap().into_iter().find(|f| f.score >= DETECT) {
                    let _ = m.embed(&img, &face).unwrap();
                    live.add(&img, &face.pts);
                    for (i, (a, b, c)) in rules.iter().enumerate() {
                        if first[i].is_none() && live.rule(*a, *b, *c) {
                            first[i] = Some(seen);
                        }
                    }
                }
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let good = live.depth.iter().filter(|x| **x >= LIVE).count();
            let top = live.depth.iter().cloned().fold(0.0, f32::max);
            println!("{name}: frames {} scored {} over {} share {:.2} highest {:.4} first ok per rule {:?}", seen, live.depth.len(), good, good as f32 / live.depth.len().max(1) as f32, top, first);
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!("{name}: frame ms p50 {:.1} p95 {:.1}", times[times.len() / 2], times[(times.len() - 1) * 95 / 100]);
            let mut d = live.scores.clone();
            d.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let pc = |q: f32| d.get(((d.len().max(1) - 1) as f32 * q) as usize).copied().unwrap_or(0.0);
            println!("{name}: pairs {} p50 {:.4} p90 {:.4} max {:.4} over {} motion {:.3} ok {}", d.len(), pc(0.5), pc(0.9), pc(1.0), d.iter().filter(|x| **x >= LIVE).count(), live.motion, live.ok());
        }
    }
}
