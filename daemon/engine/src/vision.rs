use crate::llama::sys::*;
use crate::llama::{Context, Model, Sampler};
use crate::{Flags, Msg, Out, threads};
use crossbeam_channel::Receiver;
use setu_common::engine::{FromEngine, ToEngine, VisionConfig};
use std::ffi::{CStr, CString};
use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::Instant;

#[allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all, unnecessary_transmutes)]
mod ffi {
    use crate::llama::sys::*;
    include!(concat!(env!("OUT_DIR"), "/mtmd.rs"));
}

static LIB: OnceLock<Option<ffi::Mtmd>> = OnceLock::new();

fn names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["mtmd.dll", "libmtmd.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmtmd.dylib"]
    } else {
        &["libmtmd.so", "libmtmd.so.0"]
    }
}

pub fn load_library() -> bool {
    LIB.get_or_init(|| {
        for d in crate::llama::backend_dirs() {
            for n in names() {
                let p = d.join(n);
                if !p.exists() {
                    continue;
                }
                match unsafe { ffi::Mtmd::new(&p) } {
                    Ok(l) => {
                        unsafe { l.mtmd_helper_log_set(Some(crate::llama::quiet_log), std::ptr::null_mut()) };
                        return Some(l);
                    }
                    Err(e) => eprintln!("could not load {}: {e}", p.display()),
                }
            }
        }
        None
    })
    .is_some()
}

fn lib() -> Option<&'static ffi::Mtmd> {
    LIB.get().and_then(|l| l.as_ref())
}

struct Projector(*mut ffi::mtmd_context);

unsafe impl Send for Projector {}

impl Drop for Projector {
    fn drop(&mut self) {
        if let Some(l) = lib()
            && !self.0.is_null()
        {
            unsafe { l.mtmd_free(self.0) };
        }
    }
}

struct Chunks(*mut ffi::mtmd_input_chunks);

impl Drop for Chunks {
    fn drop(&mut self) {
        if let Some(l) = lib()
            && !self.0.is_null()
        {
            unsafe { l.mtmd_input_chunks_free(self.0) };
        }
    }
}

struct Bitmap(*mut ffi::mtmd_bitmap);

impl Drop for Bitmap {
    fn drop(&mut self) {
        if let Some(l) = lib()
            && !self.0.is_null()
        {
            unsafe { l.mtmd_bitmap_free(self.0) };
        }
    }
}

pub struct Server {
    out: Out,
    flags: Flags,
    proj: Option<Projector>,
    ctx: Option<Context>,
    model: Option<Model>,
    image: Option<Vec<u8>>,
}

pub fn chat(marker: &str, prompt: &str) -> String {
    format!("<|im_start|>user\n{marker}{prompt}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n")
}

impl Server {
    pub fn new(out: Out, flags: Flags) -> Self {
        Self { out, flags, proj: None, ctx: None, model: None, image: None }
    }

    pub fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(m) = rx.recv() {
            match m {
                Msg::Op(op) => self.op(op),
                Msg::Bytes(b) => self.image = Some(b),
                _ => {}
            }
        }
    }

    fn drop_all(&mut self) {
        self.proj = None;
        self.ctx = None;
        self.model = None;
        self.image = None;
    }

    fn op(&mut self, op: ToEngine) {
        match op {
            ToEngine::LoadVision { config } => self.load(config),
            ToEngine::Unload => {
                self.drop_all();
                self.out.send(&FromEngine::Unloaded);
            }
            ToEngine::VisionAsk { id, prompt, grammar, max_tokens } => self.ask(id, &prompt, grammar.as_deref(), max_tokens),
            _ => {}
        }
    }

    fn load(&mut self, c: VisionConfig) {
        let t0 = Instant::now();
        self.drop_all();
        let Some(l) = lib() else {
            return self.out.error("The screen-understanding library isn't installed with Setu.", true);
        };
        self.flags.busy();
        let r = (|| -> Result<(Model, Context, Projector), String> {
            let model = Model::load(&c.model, c.gpu_layers)?;
            let ctx = Context::new(&model, c.context.max(2048), threads(c.threads), self.flags.cancel.clone())?;
            let path = CString::new(c.projector.as_str()).map_err(|e| e.to_string())?;
            let proj = unsafe {
                let mut p = l.mtmd_context_params_default();
                p.use_gpu = crate::llama::gpu() && c.gpu_layers != 0;
                p.n_threads = threads(c.threads);
                p.print_timings = false;
                p.warmup = false;
                if c.min_tokens > 0 {
                    p.image_min_tokens = c.min_tokens;
                }
                l.mtmd_init_from_file(path.as_ptr(), model.raw(), p)
            };
            if proj.is_null() {
                return Err("The picture model could not be loaded.".into());
            }
            let proj = Projector(proj);
            if !unsafe { l.mtmd_support_vision(proj.0) } {
                return Err("That model can't read pictures.".into());
            }
            Ok((model, ctx, proj))
        })();
        self.flags.idle();
        match r {
            Ok((model, ctx, proj)) => {
                let device = model.device.clone();
                self.model = Some(model);
                self.ctx = Some(ctx);
                self.proj = Some(proj);
                self.out.send(&FromEngine::Loaded { ram: crate::rss(), ms: t0.elapsed().as_millis() as u64, device });
            }
            Err(e) => self.out.error(e, true),
        }
    }

    fn ask(&mut self, id: u64, prompt: &str, grammar: Option<&str>, max_tokens: u32) {
        let t0 = Instant::now();
        let Some(img) = self.image.take() else {
            return self.out.error("No picture came with the question.", false);
        };
        let (Some(l), Some(model), Some(ctx), Some(proj)) = (lib(), self.model.as_ref(), self.ctx.as_mut(), self.proj.as_ref()) else {
            return self.out.error("The picture model is not loaded.", false);
        };
        self.flags.cancel.store(false, Ordering::Relaxed);
        self.flags.busy();
        let r = (|| -> Result<(String, u64, u32), String> {
            ctx.clear();
            let bitmap = Bitmap(unsafe { l.mtmd_helper_bitmap_init_from_buf(proj.0, img.as_ptr(), img.len(), false, l.mtmd_helper_init_opt_default()) }.bitmap);
            if bitmap.0.is_null() {
                return Err("The picture couldn't be read.".into());
            }
            let marker = unsafe { CStr::from_ptr(l.mtmd_default_marker()) }.to_string_lossy().into_owned();
            let text = CString::new(chat(&marker, prompt)).map_err(|e| e.to_string())?;
            let input = ffi::mtmd_input_text { text: text.as_ptr(), text_len: text.as_bytes().len(), add_special: true, parse_special: true };
            let chunks = Chunks(unsafe { l.mtmd_input_chunks_init() });
            let maps = [bitmap.0 as *const ffi::mtmd_bitmap];
            let t = unsafe { l.mtmd_tokenize(proj.0, chunks.0, &input, maps.as_ptr(), 1) };
            if t != 0 {
                return Err(format!("The picture couldn't be prepared ({t})."));
            }
            let n_tokens = unsafe { l.mtmd_helper_get_n_tokens(chunks.0) };
            if n_tokens + max_tokens as usize + 8 >= ctx.n_ctx() as usize {
                return Err("The picture is too large for the model.".into());
            }
            let te = Instant::now();
            let mut n_past: llama_pos = 0;
            let e = unsafe { l.mtmd_helper_eval_chunks(proj.0, ctx.raw(), chunks.0, 0, 0, 512, true, &mut n_past) };
            let encode_ms = te.elapsed().as_millis() as u64;
            if e != 0 {
                return Err(if self.flags.cancel.load(Ordering::Relaxed) { "cancelled".into() } else { format!("The picture couldn't be read ({e}).") });
            }
            ctx.set_pos(n_past);
            let mut sampler = Sampler::new(model, grammar, 0.0, id as u32)?;
            let mut bytes = Vec::new();
            for _ in 0..max_tokens.max(1) {
                if self.flags.cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                let tok = sampler.sample(ctx);
                if model.is_eog(tok) {
                    break;
                }
                model.piece(tok, &mut bytes);
                ctx.decode_at(tok)?;
            }
            Ok((String::from_utf8_lossy(&bytes).into_owned(), encode_ms, n_tokens as u32))
        })();
        self.flags.cancel.store(false, Ordering::Relaxed);
        self.flags.idle();
        match r {
            Ok((text, encode_ms, image_tokens)) => self.out.send(&FromEngine::VisionResult { id, text, ms: t0.elapsed().as_millis() as u64, encode_ms, image_tokens }),
            Err(e) => self.out.error(e, false),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prompt_is_chatml_with_the_picture_first_and_no_thinking() {
        let p = super::chat("<__media__>", "Find the Save button.");
        assert!(p.starts_with("<|im_start|>user\n<__media__>Find the Save button.<|im_end|>"));
        assert!(p.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));
    }
}
