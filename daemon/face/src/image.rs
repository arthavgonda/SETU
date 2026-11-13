use anyhow::{Result, anyhow};

#[derive(Debug, Clone)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

impl Image {
    pub fn px(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y.min(self.h - 1) * self.w + x.min(self.w - 1)) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    pub fn sample(&self, fx: f32, fy: f32) -> [f32; 3] {
        let x0 = fx.floor().max(0.0) as usize;
        let y0 = fy.floor().max(0.0) as usize;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let ax = (fx - x0 as f32).clamp(0.0, 1.0);
        let ay = (fy - y0 as f32).clamp(0.0, 1.0);
        let (a, b, c, d) = (self.px(x0, y0), self.px(x1, y0), self.px(x0, y1), self.px(x1, y1));
        let mut out = [0f32; 3];
        for k in 0..3 {
            let top = a[k] as f32 * (1.0 - ax) + b[k] as f32 * ax;
            let bot = c[k] as f32 * (1.0 - ax) + d[k] as f32 * ax;
            out[k] = top * (1.0 - ay) + bot * ay;
        }
        out
    }

    pub fn from_rgba(w: usize, h: usize, rgba: &[u8]) -> Self {
        let mut rgb = Vec::with_capacity(w * h * 3);
        for p in rgba.as_chunks::<4>().0.iter().take(w * h) {
            rgb.extend_from_slice(&p[..3]);
        }
        Image { w, h, rgb }
    }
}

pub fn decode_png(bytes: &[u8]) -> Result<Image> {
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info()?;
    let mut buf = vec![0u8; r.output_buffer_size()];
    let info = r.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let data = &buf[..info.buffer_size()];
    Ok(match info.color_type {
        png::ColorType::Rgba => Image::from_rgba(w, h, data),
        png::ColorType::Rgb => Image { w, h, rgb: data.to_vec() },
        png::ColorType::Grayscale => Image { w, h, rgb: data.iter().flat_map(|g| [*g, *g, *g]).collect() },
        png::ColorType::GrayscaleAlpha => Image { w, h, rgb: data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0]]).collect() },
        png::ColorType::Indexed => return Err(anyhow!("That picture uses a palette I can't read.")),
    })
}
