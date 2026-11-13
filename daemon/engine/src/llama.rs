#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all)]

#[allow(unnecessary_transmutes)]
pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/llama.rs"));
}

use setu_common::engine::Device;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use sys::*;

static INIT: Once = Once::new();
static GPU: AtomicBool = AtomicBool::new(false);

pub fn set_gpu(on: bool) {
    GPU.store(on, Ordering::Relaxed);
}

pub fn gpu() -> bool {
    GPU.load(Ordering::Relaxed)
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

pub fn devices() -> Vec<Device> {
    init();
    let mut out = Vec::new();
    unsafe {
        for i in 0..ggml_backend_dev_count() {
            let d = ggml_backend_dev_get(i);
            if d.is_null() {
                continue;
            }
            let t = ggml_backend_dev_type(d);
            let kind = match t {
                ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU => "gpu",
                ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU => "igpu",
                ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU => "cpu",
                _ => "accel",
            };
            let (mut free, mut total) = (0usize, 0usize);
            ggml_backend_dev_memory(d, &mut free, &mut total);
            let reg = ggml_backend_dev_backend_reg(d);
            out.push(Device {
                name: text(ggml_backend_dev_description(d)),
                kind: kind.into(),
                backend: if reg.is_null() { String::new() } else { text(ggml_backend_reg_name(reg)) },
                total: total as u64,
                free: free as u64,
            });
        }
    }
    out
}

fn best_gpu() -> Option<Device> {
    devices().into_iter().filter(|d| d.kind == "gpu" || d.kind == "igpu").max_by_key(|d| (d.kind == "gpu", d.free))
}

fn layer_count(path: &CString) -> i32 {
    unsafe {
        let mut mp = llama_model_default_params();
        mp.vocab_only = true;
        mp.n_gpu_layers = 0;
        let m = llama_model_load_from_file(path.as_ptr(), mp);
        if m.is_null() {
            return 0;
        }
        let n = llama_model_n_layer(m);
        llama_model_free(m);
        n
    }
}

fn plan_layers(path: &CString, file: u64, context_bytes: u64) -> (i32, Option<Device>) {
    if !GPU.load(Ordering::Relaxed) {
        return (0, None);
    }
    let Some(g) = best_gpu() else { return (0, None) };
    let reserve = context_bytes + 512 * 1024 * 1024;
    if g.free == 0 || g.free <= reserve {
        return (0, None);
    }
    if file + file / 10 + reserve <= g.free {
        return (-1, Some(g));
    }
    let n = layer_count(path);
    if n <= 0 {
        return (0, None);
    }
    let per = (file / (n as u64 + 1)).max(1);
    let fit = ((g.free - reserve) / per).min(n as u64) as i32;
    if fit < n / 4 { (0, None) } else { (fit, Some(g)) }
}

pub unsafe extern "C" fn quiet_log(level: ggml_log_level, text: *const c_char, _: *mut c_void) {
    if level >= ggml_log_level_GGML_LOG_LEVEL_ERROR && !text.is_null() {
        let t = unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy();
        eprint!("{t}");
    }
}

unsafe extern "C" fn abort_cb(data: *mut c_void) -> bool {
    if data.is_null() {
        return false;
    }
    unsafe { &*(data as *const AtomicBool) }.load(Ordering::Relaxed)
}

pub fn backend_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var("SETU_LIB_DIR") {
        v.push(std::path::PathBuf::from(d));
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(d) = me.parent() {
            v.push(d.join("../lib/setu"));
            v.push(d.to_path_buf());
            v.push(d.join("../Frameworks"));
        }
    }
    v.push(std::path::PathBuf::from(env!("SETU_LLAMA_LIB")));
    v
}

pub fn init() {
    INIT.call_once(|| unsafe {
        llama_log_set(Some(quiet_log), std::ptr::null_mut());
        for d in backend_dirs() {
            if d.join(if cfg!(windows) { "ggml-base.dll" } else if cfg!(target_os = "macos") { "libggml-base.dylib" } else { "libggml-base.so" }).exists() {
                if let Ok(c) = CString::new(d.to_string_lossy().as_bytes()) {
                    ggml_backend_load_all_from_path(c.as_ptr());
                    break;
                }
            }
        }
        llama_backend_init();
    });
}

pub struct Model {
    m: *mut llama_model,
    vocab: *const llama_vocab,
    pub device: String,
}

unsafe impl Send for Model {}

impl Model {
    pub fn load(path: &str, gpu_layers: i32) -> Result<Self, String> {
        Self::load_with(path, gpu_layers, 256 * 1024 * 1024)
    }

    pub fn load_with(path: &str, gpu_layers: i32, context_bytes: u64) -> Result<Self, String> {
        init();
        let p = CString::new(path).map_err(|e| e.to_string())?;
        let file = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let (layers, gpu) = if gpu_layers == 0 { (0, None) } else { plan_layers(&p, file, context_bytes) };
        let mut none: [ggml_backend_dev_t; 1] = [std::ptr::null_mut()];
        let cpu_only = none.as_mut_ptr();
        let open = |layers: i32| unsafe {
            let mut mp = llama_model_default_params();
            mp.n_gpu_layers = layers;
            if layers == 0 {
                mp.devices = cpu_only;
            }
            mp.load_mode = llama_load_mode_LLAMA_LOAD_MODE_MMAP;
            llama_model_load_from_file(p.as_ptr(), mp)
        };
        let mut m = open(layers);
        let mut device = match (&gpu, layers) {
            (Some(g), -1) => format!("{} ({})", g.name, g.backend),
            (Some(g), n) if n > 0 => format!("{} ({}, {n} layers)", g.name, g.backend),
            _ => "CPU".to_owned(),
        };
        if m.is_null() && layers != 0 {
            eprintln!("loading on the graphics card failed; using the processor");
            m = open(0);
            device = "CPU".to_owned();
        }
        if m.is_null() {
            return Err("The language model file could not be loaded.".into());
        }
        let vocab = unsafe { llama_model_get_vocab(m) };
        Ok(Self { m, vocab, device })
    }

    pub fn token(&self, text: &str) -> Option<llama_token> {
        let t = self.tokenize(text, false);
        (t.len() == 1).then(|| t[0])
    }

    pub fn size(&self) -> u64 {
        unsafe { llama_model_size(self.m) }
    }

    pub fn raw(&self) -> *mut llama_model {
        self.m
    }

    pub fn snapshot_needed(&self) -> bool {
        unsafe { llama_model_is_recurrent(self.m) || llama_model_is_hybrid(self.m) }
    }

    pub fn tokenize(&self, text: &str, add_special: bool) -> Vec<llama_token> {
        let mut out = vec![0 as llama_token; text.len() + 8];
        let n = unsafe {
            llama_tokenize(self.vocab, text.as_ptr() as *const c_char, text.len() as i32, out.as_mut_ptr(), out.len() as i32, add_special, true)
        };
        if n < 0 {
            out.resize((-n) as usize, 0);
            let n2 = unsafe {
                llama_tokenize(self.vocab, text.as_ptr() as *const c_char, text.len() as i32, out.as_mut_ptr(), out.len() as i32, add_special, true)
            };
            out.truncate(n2.max(0) as usize);
        } else {
            out.truncate(n as usize);
        }
        out
    }

    pub fn piece(&self, tok: llama_token, out: &mut Vec<u8>) {
        let mut buf = [0u8; 128];
        let n = unsafe { llama_token_to_piece(self.vocab, tok, buf.as_mut_ptr() as *mut c_char, buf.len() as i32, 0, false) };
        if n > 0 {
            out.extend_from_slice(&buf[..n as usize]);
        } else if n < 0 {
            let mut big = vec![0u8; (-n) as usize];
            let n2 = unsafe { llama_token_to_piece(self.vocab, tok, big.as_mut_ptr() as *mut c_char, big.len() as i32, 0, false) };
            if n2 > 0 {
                out.extend_from_slice(&big[..n2 as usize]);
            }
        }
    }

    pub fn is_eog(&self, tok: llama_token) -> bool {
        unsafe { llama_vocab_is_eog(self.vocab, tok) }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe { llama_model_free(self.m) }
    }
}

pub struct Context {
    c: *mut llama_context,
    batch: u32,
    abort: std::sync::Arc<AtomicBool>,
    pos: i32,
}

unsafe impl Send for Context {}

impl Context {
    pub fn new(model: &Model, n_ctx: u32, threads: i32, abort: std::sync::Arc<AtomicBool>) -> Result<Self, String> {
        let mut cp = unsafe { llama_context_default_params() };
        cp.n_ctx = n_ctx;
        cp.n_batch = 512;
        cp.n_ubatch = 512;
        cp.n_seq_max = 1;
        cp.n_threads = threads;
        cp.n_threads_batch = threads;
        cp.no_perf = true;
        let c = unsafe { llama_init_from_model(model.m, cp) };
        if c.is_null() {
            return Err("The language model context could not be created.".into());
        }
        unsafe { llama_set_abort_callback(c, Some(abort_cb), std::sync::Arc::as_ptr(&abort) as *mut c_void) };
        Ok(Self { c, batch: 512, abort, pos: 0 })
    }

    pub fn n_ctx(&self) -> u32 {
        unsafe { llama_n_ctx(self.c) }
    }

    pub fn pos(&self) -> i32 {
        self.pos
    }

    pub fn clear(&mut self) {
        unsafe { llama_memory_clear(llama_get_memory(self.c), true) };
        self.pos = 0;
    }

    pub fn truncate(&mut self, pos: i32) -> bool {
        let ok = unsafe { llama_memory_seq_rm(llama_get_memory(self.c), 0, pos, -1) };
        if ok {
            self.pos = pos;
        }
        ok
    }

    pub fn decode(&mut self, tokens: &[llama_token]) -> Result<(), String> {
        if self.pos as usize + tokens.len() >= self.n_ctx() as usize {
            return Err("The request is longer than the model context.".into());
        }
        for chunk in tokens.chunks(self.batch as usize) {
            let mut v = chunk.to_vec();
            let b = unsafe { llama_batch_get_one(v.as_mut_ptr(), v.len() as i32) };
            let r = unsafe { llama_decode(self.c, b) };
            if r != 0 {
                return Err(if r == 2 { "cancelled".into() } else { format!("decode failed ({r})") });
            }
            self.pos += chunk.len() as i32;
        }
        Ok(())
    }

    pub fn save(&self) -> Vec<u8> {
        unsafe {
            let n = llama_state_seq_get_size(self.c, 0);
            let mut buf = vec![0u8; n];
            let w = llama_state_seq_get_data(self.c, buf.as_mut_ptr(), n, 0);
            buf.truncate(w);
            buf
        }
    }

    pub fn restore(&mut self, state: &[u8], pos: i32) -> bool {
        unsafe {
            llama_memory_clear(llama_get_memory(self.c), true);
            let r = llama_state_seq_set_data(self.c, state.as_ptr(), state.len(), 0);
            if r == 0 {
                self.pos = 0;
                return false;
            }
        }
        self.pos = pos;
        true
    }

    pub fn raw(&self) -> *mut llama_context {
        self.c
    }

    pub fn set_pos(&mut self, pos: i32) {
        self.pos = pos;
    }

    pub fn decode_at(&mut self, tok: llama_token) -> Result<(), String> {
        if self.pos as usize + 1 >= self.n_ctx() as usize {
            return Err("The request is longer than the model context.".into());
        }
        let r = unsafe {
            let b = llama_batch_init(1, 0, 1);
            *b.token = tok;
            *b.pos = self.pos;
            *b.n_seq_id = 1;
            **b.seq_id = 0;
            *b.logits = 1;
            let mut b = b;
            b.n_tokens = 1;
            let r = llama_decode(self.c, b);
            llama_batch_free(b);
            r
        };
        if r != 0 {
            return Err(if r == 2 { "cancelled".into() } else { format!("decode failed ({r})") });
        }
        self.pos += 1;
        Ok(())
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { llama_free(self.c) }
    }
}

pub struct Sampler {
    s: *mut llama_sampler,
    grammar: *mut llama_sampler,
    n_vocab: usize,
    scratch: Vec<llama_token_data>,
}

impl Sampler {
    pub fn new(model: &Model, grammar: Option<&str>, temperature: f32, seed: u32) -> Result<Self, String> {
        unsafe {
            let s = llama_sampler_chain_init(llama_sampler_chain_default_params());
            let mut gp = std::ptr::null_mut();
            if let Some(g) = grammar {
                let gs = CString::new(g).map_err(|e| e.to_string())?;
                let root = CString::new("root").unwrap();
                gp = llama_sampler_init_grammar(model.vocab, gs.as_ptr(), root.as_ptr());
                if gp.is_null() {
                    llama_sampler_free(s);
                    return Err("The grammar is invalid.".into());
                }
            }
            if temperature > 0.0 {
                llama_sampler_chain_add(s, llama_sampler_init_top_k(40));
                llama_sampler_chain_add(s, llama_sampler_init_top_p(0.9, 1));
                llama_sampler_chain_add(s, llama_sampler_init_temp(temperature));
                llama_sampler_chain_add(s, llama_sampler_init_dist(seed));
            } else {
                llama_sampler_chain_add(s, llama_sampler_init_greedy());
            }
            Ok(Self { s, grammar: gp, n_vocab: llama_vocab_n_tokens(model.vocab).max(0) as usize, scratch: Vec::new() })
        }
    }

    pub fn speech(model: &Model, temperature: f32, top_p: f32, repeat: f32, seed: u32) -> Self {
        unsafe {
            let s = llama_sampler_chain_init(llama_sampler_chain_default_params());
            llama_sampler_chain_add(s, llama_sampler_init_top_k(50));
            llama_sampler_chain_add(s, llama_sampler_init_penalties(llama_vocab_n_tokens(model.vocab), 64, repeat, 0.0, 0.0));
            llama_sampler_chain_add(s, llama_sampler_init_top_p(top_p, 1));
            llama_sampler_chain_add(s, llama_sampler_init_temp(temperature));
            llama_sampler_chain_add(s, llama_sampler_init_dist(seed));
            Self { s, grammar: std::ptr::null_mut(), n_vocab: 0, scratch: Vec::new() }
        }
    }

    pub fn sample(&mut self, ctx: &Context) -> llama_token {
        unsafe {
            let t = llama_sampler_sample(self.s, ctx.c, -1);
            if self.grammar.is_null() {
                return t;
            }
            let mut one = [llama_token_data { id: t, logit: 1.0, p: 0.0 }];
            let mut arr = llama_token_data_array { data: one.as_mut_ptr(), size: 1, selected: -1, sorted: false };
            llama_sampler_apply(self.grammar, &mut arr);
            if one[0].logit.is_finite() {
                llama_sampler_accept(self.grammar, t);
                return t;
            }
            let logits = llama_get_logits_ith(ctx.c, -1);
            if logits.is_null() || self.n_vocab == 0 {
                return t;
            }
            let l = std::slice::from_raw_parts(logits, self.n_vocab);
            self.scratch.clear();
            self.scratch.extend(l.iter().enumerate().map(|(i, &v)| llama_token_data { id: i as llama_token, logit: v, p: 0.0 }));
            let mut arr = llama_token_data_array { data: self.scratch.as_mut_ptr(), size: self.scratch.len(), selected: -1, sorted: false };
            llama_sampler_apply(self.grammar, &mut arr);
            llama_sampler_apply(self.s, &mut arr);
            let pick = if arr.selected >= 0 && (arr.selected as usize) < arr.size { (*arr.data.add(arr.selected as usize)).id } else { t };
            llama_sampler_accept(self.grammar, pick);
            llama_sampler_accept(self.s, pick);
            pick
        }
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        unsafe {
            llama_sampler_free(self.s);
            if !self.grammar.is_null() {
                llama_sampler_free(self.grammar);
            }
        }
    }
}
