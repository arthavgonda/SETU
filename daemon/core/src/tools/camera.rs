#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
use crate::control::ControlMsg;
use crate::hub::Hub;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Qr {
    Wifi { ssid: String, password: String },
    Text(String),
}

#[derive(Debug)]
pub enum Event {
    Decoded(Option<Qr>),
    Failed(String),
}

pub struct Camera {
    stop: Arc<AtomicBool>,
    snap: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse(content: &str) -> Qr {
    if let Some(body) = content.strip_prefix("WIFI:") {
        let mut ssid = String::new();
        let mut password = String::new();
        let mut field = String::new();
        let mut esc = false;
        let mut parts = Vec::new();
        for c in body.chars() {
            if esc {
                field.push('\\');
                field.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == ';' {
                parts.push(std::mem::take(&mut field));
            } else {
                field.push(c);
            }
        }
        for p in parts {
            if let Some(v) = p.strip_prefix("S:") {
                ssid = unescape(v);
            } else if let Some(v) = p.strip_prefix("P:") {
                password = unescape(v);
            }
        }
        if !ssid.is_empty() {
            return Qr::Wifi { ssid, password };
        }
    }
    Qr::Text(content.to_owned())
}

pub fn decode_gray(w: usize, h: usize, gray: &[u8]) -> Option<String> {
    if w == 0 || h == 0 || gray.len() < w * h {
        return None;
    }
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| gray[y * w + x]);
    for g in img.detect_grids() {
        if let Ok((_, content)) = g.decode() {
            return Some(content);
        }
    }
    None
}

fn jpeg_to_gray(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma));
    let px = d.decode().ok()?;
    let info = d.info()?;
    Some((info.width as usize, info.height as usize, px))
}

fn gray_preview(w: usize, h: usize, gray: &[u8]) -> Option<Vec<u8>> {
    let (tw, th) = (480usize, 360usize);
    let mut small = vec![0u8; tw * th];
    for y in 0..th {
        for x in 0..tw {
            small[y * tw + x] = gray[(y * h / th) * w + (x * w / tw)];
        }
    }
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 70).encode(&small, tw as u16, th as u16, jpeg_encoder::ColorType::Luma).ok()?;
    Some(out)
}

#[cfg(target_os = "linux")]
fn find_device() -> Option<String> {
    let mut list: Vec<String> = std::fs::read_dir("/dev").ok()?.flatten().map(|e| e.path().to_string_lossy().into_owned()).filter(|p| p.starts_with("/dev/video")).collect();
    list.sort();
    for p in list {
        if let Ok(d) = v4l::Device::with_path(&p)
            && let Ok(c) = d.query_caps()
                && c.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE) {
                    return Some(p);
                }
    }
    None
}

impl Camera {
    #[cfg(target_os = "linux")]
    pub fn open(hub: Arc<Hub>, tx: mpsc::Sender<ControlMsg>) -> Result<Self, String> {
        use v4l::buffer::Type;
        use v4l::io::traits::CaptureStream;
        use v4l::video::Capture;
        let path = find_device().ok_or("No camera was found. Connect a camera, then try again.")?;
        let dev = v4l::Device::with_path(&path).map_err(|_| "The camera can't be opened. Check the Camera permission.".to_string())?;
        let mut fmt = dev.format().map_err(|e| e.to_string())?;
        fmt.width = 640;
        fmt.height = 480;
        fmt.fourcc = v4l::FourCC::new(b"MJPG");
        let fmt = match dev.set_format(&fmt) {
            Ok(f) if f.fourcc == v4l::FourCC::new(b"MJPG") => f,
            _ => {
                let mut f = dev.format().map_err(|e| e.to_string())?;
                f.width = 640;
                f.height = 480;
                f.fourcc = v4l::FourCC::new(b"YUYV");
                dev.set_format(&f).map_err(|e| format!("The camera format could not be set: {e}"))?
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let snap = Arc::new(AtomicBool::new(false));
        let (s1, s2) = (stop.clone(), snap.clone());
        let thread = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                let mjpg = fmt.fourcc == v4l::FourCC::new(b"MJPG");
                let (w, h) = (fmt.width as usize, fmt.height as usize);
                let mut stream = match v4l::io::mmap::Stream::with_buffers(&dev, Type::VideoCapture, 4) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.blocking_send(ControlMsg::Camera(Event::Failed(format!("The camera could not start: {e}"))));
                        return;
                    }
                };
                let started = Instant::now();
                let mut last = Instant::now() - Duration::from_secs(1);
                while !s1.load(Ordering::Relaxed) {
                    let (buf, meta) = match stream.next() {
                        Ok(f) => f,
                        Err(_) => {
                            let _ = tx.blocking_send(ControlMsg::Camera(Event::Failed("The camera stopped sending pictures.".into())));
                            return;
                        }
                    };
                    let data = &buf[..(meta.bytesused as usize).min(buf.len())];
                    let want_snap = s2.swap(false, Ordering::Relaxed);
                    let want_preview = last.elapsed() >= Duration::from_millis(100) && hub.has("camera.state");
                    if !want_snap && !want_preview {
                        continue;
                    }
                    let gray = if mjpg {
                        jpeg_to_gray(data)
                    } else {
                        Some((w, h, data.as_chunks::<2>().0.iter().map(|p| p[0]).collect::<Vec<u8>>()))
                    };
                    if want_preview {
                        last = Instant::now();
                        let jpeg = if mjpg && data.len() < 700_000 { Some(data.to_vec()) } else { gray.as_ref().and_then(|(gw, gh, g)| gray_preview(*gw, *gh, g)) };
                        if let Some(j) = jpeg {
                            use base64::Engine;
                            let left = 60u64.saturating_sub(started.elapsed().as_secs());
                            hub.emit(
                                "camera.state",
                                serde_json::json!({ "open": true, "frame": base64::engine::general_purpose::STANDARD.encode(&j), "secondsLeft": left, "message": null }),
                            );
                        }
                    }
                    if want_snap {
                        let r = gray.and_then(|(gw, gh, g)| decode_gray(gw, gh, &g)).map(|c| parse(&c));
                        let _ = tx.blocking_send(ControlMsg::Camera(Event::Decoded(r)));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { stop, snap, thread: Some(thread) })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn open(hub: Arc<Hub>, tx: mpsc::Sender<ControlMsg>) -> Result<Self, String> {
        let mut frames = setu_face::open_frames().map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let snap = Arc::new(AtomicBool::new(false));
        let (s1, s2) = (stop.clone(), snap.clone());
        let thread = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || {
                let started = Instant::now();
                let mut last = Instant::now() - Duration::from_secs(1);
                while !s1.load(Ordering::Relaxed) {
                    let img = match frames.next() {
                        Ok(Some(i)) => i,
                        Ok(None) => {
                            std::thread::sleep(Duration::from_millis(30));
                            continue;
                        }
                        Err(e) => {
                            let _ = tx.blocking_send(ControlMsg::Camera(Event::Failed(e.to_string())));
                            return;
                        }
                    };
                    let want_snap = s2.swap(false, Ordering::Relaxed);
                    let want_preview = last.elapsed() >= Duration::from_millis(100) && hub.has("camera.state");
                    if !want_snap && !want_preview {
                        continue;
                    }
                    let gray: Vec<u8> = img.rgb.chunks_exact(3).map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8).collect();
                    if want_preview {
                        last = Instant::now();
                        if let Some(j) = gray_preview(img.w, img.h, &gray) {
                            use base64::Engine;
                            let left = 60u64.saturating_sub(started.elapsed().as_secs());
                            hub.emit(
                                "camera.state",
                                serde_json::json!({ "open": true, "frame": base64::engine::general_purpose::STANDARD.encode(&j), "secondsLeft": left, "message": null }),
                            );
                        }
                    }
                    if want_snap {
                        let r = decode_gray(img.w, img.h, &gray).map(|c| parse(&c));
                        let _ = tx.blocking_send(ControlMsg::Camera(Event::Decoded(r)));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { stop, snap, thread: Some(thread) })
    }

    pub fn capture(&self) {
        self.snap.store(true, Ordering::Relaxed);
    }

    pub fn close(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wifi_codes() {
        match parse("WIFI:T:WPA;S:Home\\;Net;P:se\\:cret;H:false;;") {
            Qr::Wifi { ssid, password } => {
                assert_eq!(ssid, "Home;Net");
                assert_eq!(password, "se:cret");
            }
            _ => panic!(),
        }
        assert!(matches!(parse("https://example.org"), Qr::Text(_)));
    }

    #[test]
    fn decodes_generated_qr() {
        let qr = qrcode::QrCode::new(b"WIFI:T:WPA;S:Cafe;P:latte42;;").unwrap();
        let n = qr.width();
        let (scale, quiet) = (6usize, 4usize);
        let size = (n + quiet * 2) * scale;
        let colors = qr.to_colors();
        let mut img = vec![255u8; size * size];
        for y in 0..n {
            for x in 0..n {
                if colors[y * n + x] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            img[((y + quiet) * scale + dy) * size + (x + quiet) * scale + dx] = 0;
                        }
                    }
                }
            }
        }
        let text = decode_gray(size, size, &img).expect("decoded");
        match parse(&text) {
            Qr::Wifi { ssid, password } => {
                assert_eq!(ssid, "Cafe");
                assert_eq!(password, "latte42");
            }
            _ => panic!(),
        }
        assert!(decode_gray(10, 10, &[255u8; 100]).is_none());
    }
}
