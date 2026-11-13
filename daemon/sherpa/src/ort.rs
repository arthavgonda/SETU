#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case, dead_code, clippy::all)]

pub mod sys {
    include!(concat!(env!("OUT_DIR"), "/ort.rs"));
}

use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use sys::*;

fn api() -> Result<&'static OrtApi, String> {
    unsafe {
        let base = OrtGetApiBase();
        if base.is_null() {
            return Err("ONNX Runtime is not available".into());
        }
        let get = (*base).GetApi.ok_or("ONNX Runtime has no GetApi")?;
        let a = get(ORT_API_VERSION);
        if a.is_null() { Err("ONNX Runtime is too old".into()) } else { Ok(&*a) }
    }
}

fn check(a: &OrtApi, st: *mut OrtStatus) -> Result<(), String> {
    if st.is_null() {
        return Ok(());
    }
    unsafe {
        let msg = a.GetErrorMessage.map(|f| f(st)).filter(|p| !p.is_null()).map(|p| CStr::from_ptr(p).to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(r) = a.ReleaseStatus {
            r(st);
        }
        Err(if msg.is_empty() { "ONNX Runtime error".into() } else { msg })
    }
}

macro_rules! call {
    ($a:expr, $f:ident, $($arg:expr),*) => {
        check($a, unsafe { ($a.$f.ok_or(concat!("missing ", stringify!($f)))?)($($arg),*) })
    };
}

pub struct SnacDecoder {
    a: &'static OrtApi,
    env: *mut OrtEnv,
    session: *mut OrtSession,
    mem: *mut OrtMemoryInfo,
    inputs: Vec<CString>,
    output: CString,
}

unsafe impl Send for SnacDecoder {}

impl SnacDecoder {
    pub fn new(path: &str, threads: i32) -> Result<Self, String> {
        let a = api()?;
        let mut env: *mut OrtEnv = ptr::null_mut();
        let id = CString::new("setu").unwrap();
        call!(a, CreateEnv, OrtLoggingLevel_ORT_LOGGING_LEVEL_ERROR, id.as_ptr(), &mut env)?;
        let mut opts: *mut OrtSessionOptions = ptr::null_mut();
        call!(a, CreateSessionOptions, &mut opts)?;
        let built = (|| -> Result<*mut OrtSession, String> {
            call!(a, SetIntraOpNumThreads, opts, threads.max(1))?;
            call!(a, SetSessionGraphOptimizationLevel, opts, GraphOptimizationLevel_ORT_ENABLE_ALL)?;
            let mut session: *mut OrtSession = ptr::null_mut();
            #[cfg(windows)]
            let p: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            #[cfg(not(windows))]
            let p = CString::new(path).map_err(|e| e.to_string())?;
            call!(a, CreateSession, env, p.as_ptr() as *const _, opts, &mut session)?;
            Ok(session)
        })();
        unsafe {
            if let Some(r) = a.ReleaseSessionOptions {
                r(opts);
            }
        }
        let session = match built {
            Ok(s) => s,
            Err(e) => {
                unsafe {
                    if let Some(r) = a.ReleaseEnv {
                        r(env);
                    }
                }
                return Err(e);
            }
        };
        let mut me = Self { a, env, session, mem: ptr::null_mut(), inputs: Vec::new(), output: CString::default() };
        call!(a, CreateCpuMemoryInfo, OrtAllocatorType_OrtArenaAllocator, OrtMemType_OrtMemTypeDefault, &mut me.mem)?;
        let mut alloc: *mut OrtAllocator = ptr::null_mut();
        call!(a, GetAllocatorWithDefaultOptions, &mut alloc)?;
        let mut n = 0usize;
        call!(a, SessionGetInputCount, session, &mut n)?;
        for i in 0..n {
            let mut name: *mut c_char = ptr::null_mut();
            call!(a, SessionGetInputName, session, i, alloc, &mut name)?;
            me.inputs.push(unsafe { CStr::from_ptr(name) }.to_owned());
            call!(a, AllocatorFree, alloc, name as *mut c_void)?;
        }
        if me.inputs.len() != 3 {
            return Err(format!("the audio decoder has {} inputs, expected 3", me.inputs.len()));
        }
        me.inputs.sort();
        let mut name: *mut c_char = ptr::null_mut();
        call!(a, SessionGetOutputName, session, 0, alloc, &mut name)?;
        me.output = unsafe { CStr::from_ptr(name) }.to_owned();
        call!(a, AllocatorFree, alloc, name as *mut c_void)?;
        Ok(me)
    }

    pub fn decode(&self, c0: &[i64], c1: &[i64], c2: &[i64]) -> Result<Vec<f32>, String> {
        let a = self.a;
        let mut values: Vec<*mut OrtValue> = Vec::new();
        let release = |values: &mut Vec<*mut OrtValue>| unsafe {
            if let Some(r) = a.ReleaseValue {
                for v in values.drain(..) {
                    if !v.is_null() {
                        r(v);
                    }
                }
            }
        };
        let mut bufs = [c0.to_vec(), c1.to_vec(), c2.to_vec()];
        for b in bufs.iter_mut() {
            let shape = [1i64, b.len() as i64];
            let mut v: *mut OrtValue = ptr::null_mut();
            let r = call!(
                a,
                CreateTensorWithDataAsOrtValue,
                self.mem,
                b.as_mut_ptr() as *mut c_void,
                b.len() * 8,
                shape.as_ptr(),
                2,
                ONNXTensorElementDataType_ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64,
                &mut v
            );
            if let Err(e) = r {
                release(&mut values);
                return Err(e);
            }
            values.push(v);
        }
        let names: Vec<*const c_char> = self.inputs.iter().map(|c| c.as_ptr()).collect();
        let out_names = [self.output.as_ptr()];
        let mut out: *mut OrtValue = ptr::null_mut();
        let r = call!(a, Run, self.session, ptr::null(), names.as_ptr(), values.as_ptr() as *const *const OrtValue, 3, out_names.as_ptr(), 1, &mut out);
        release(&mut values);
        r?;
        let result = (|| -> Result<Vec<f32>, String> {
            let mut info: *mut OrtTensorTypeAndShapeInfo = ptr::null_mut();
            call!(a, GetTensorTypeAndShape, out, &mut info)?;
            let mut n = 0usize;
            let r = call!(a, GetTensorShapeElementCount, info, &mut n);
            unsafe {
                if let Some(f) = a.ReleaseTensorTypeAndShapeInfo {
                    f(info);
                }
            }
            r?;
            let mut data: *mut c_void = ptr::null_mut();
            call!(a, GetTensorMutableData, out, &mut data)?;
            Ok(unsafe { std::slice::from_raw_parts(data as *const f32, n) }.to_vec())
        })();
        release(&mut vec![out]);
        result
    }
}

impl Drop for SnacDecoder {
    fn drop(&mut self) {
        unsafe {
            if let Some(r) = self.a.ReleaseMemoryInfo
                && !self.mem.is_null() {
                    r(self.mem);
                }
            if let Some(r) = self.a.ReleaseSession {
                r(self.session);
            }
            if let Some(r) = self.a.ReleaseEnv {
                r(self.env);
            }
        }
    }
}

pub struct Model {
    a: &'static OrtApi,
    env: *mut OrtEnv,
    session: *mut OrtSession,
    mem: *mut OrtMemoryInfo,
    input: CString,
    output: CString,
    outputs: Vec<CString>,
    dims: Vec<i64>,
}

unsafe impl Send for Model {}
unsafe impl Sync for Model {}

impl Model {
    pub fn new(path: &str, threads: i32) -> Result<Self, String> {
        let a = api()?;
        let mut env: *mut OrtEnv = ptr::null_mut();
        let id = CString::new("setu-vision").unwrap();
        call!(a, CreateEnv, OrtLoggingLevel_ORT_LOGGING_LEVEL_ERROR, id.as_ptr(), &mut env)?;
        let mut opts: *mut OrtSessionOptions = ptr::null_mut();
        call!(a, CreateSessionOptions, &mut opts)?;
        let built = (|| -> Result<*mut OrtSession, String> {
            call!(a, SetIntraOpNumThreads, opts, threads.max(1))?;
            call!(a, SetSessionGraphOptimizationLevel, opts, GraphOptimizationLevel_ORT_ENABLE_ALL)?;
            let mut session: *mut OrtSession = ptr::null_mut();
            #[cfg(windows)]
            let p: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            #[cfg(not(windows))]
            let p = CString::new(path).map_err(|e| e.to_string())?;
            call!(a, CreateSession, env, p.as_ptr() as *const _, opts, &mut session)?;
            Ok(session)
        })();
        unsafe {
            if let Some(r) = a.ReleaseSessionOptions {
                r(opts);
            }
        }
        let session = match built {
            Ok(s) => s,
            Err(e) => {
                unsafe {
                    if let Some(r) = a.ReleaseEnv {
                        r(env);
                    }
                }
                return Err(e);
            }
        };
        let mut me = Self { a, env, session, mem: ptr::null_mut(), input: CString::default(), output: CString::default(), outputs: Vec::new(), dims: Vec::new() };
        call!(a, CreateCpuMemoryInfo, OrtAllocatorType_OrtArenaAllocator, OrtMemType_OrtMemTypeDefault, &mut me.mem)?;
        let mut alloc: *mut OrtAllocator = ptr::null_mut();
        call!(a, GetAllocatorWithDefaultOptions, &mut alloc)?;
        let mut name: *mut c_char = ptr::null_mut();
        call!(a, SessionGetInputName, session, 0, alloc, &mut name)?;
        me.input = unsafe { CStr::from_ptr(name) }.to_owned();
        call!(a, AllocatorFree, alloc, name as *mut c_void)?;
        let mut count = 0usize;
        call!(a, SessionGetOutputCount, session, &mut count)?;
        for i in 0..count {
            let mut name: *mut c_char = ptr::null_mut();
            call!(a, SessionGetOutputName, session, i, alloc, &mut name)?;
            me.outputs.push(unsafe { CStr::from_ptr(name) }.to_owned());
            call!(a, AllocatorFree, alloc, name as *mut c_void)?;
        }
        me.output = me.outputs.first().cloned().ok_or("the model has no outputs")?;
        let mut ti: *mut OrtTypeInfo = ptr::null_mut();
        if call!(a, SessionGetInputTypeInfo, session, 0, &mut ti).is_ok() {
            let mut tsi: *const OrtTensorTypeAndShapeInfo = ptr::null();
            if call!(a, CastTypeInfoToTensorInfo, ti, &mut tsi).is_ok() && !tsi.is_null() {
                let mut n = 0usize;
                if call!(a, GetDimensionsCount, tsi, &mut n).is_ok() {
                    let mut d = vec![0i64; n];
                    if call!(a, GetDimensions, tsi, d.as_mut_ptr(), n).is_ok() {
                        me.dims = d;
                    }
                }
            }
            unsafe {
                if let Some(f) = a.ReleaseTypeInfo {
                    f(ti);
                }
            }
        }
        Ok(me)
    }

    pub fn input_dims(&self) -> &[i64] {
        &self.dims
    }

    pub fn output_names(&self) -> Vec<String> {
        self.outputs.iter().map(|c| c.to_string_lossy().into_owned()).collect()
    }

    pub fn run(&self, data: &mut [f32], shape: &[i64]) -> Result<(Vec<f32>, Vec<i64>), String> {
        self.run_named(data, shape, std::slice::from_ref(&self.output)).map(|mut v| v.remove(0))
    }

    pub fn run_all(&self, data: &mut [f32], shape: &[i64]) -> Result<Vec<(Vec<f32>, Vec<i64>)>, String> {
        self.run_named(data, shape, &self.outputs)
    }

    fn run_named(&self, data: &mut [f32], shape: &[i64], outputs: &[CString]) -> Result<Vec<(Vec<f32>, Vec<i64>)>, String> {
        let a = self.a;
        let mut input: *mut OrtValue = ptr::null_mut();
        call!(a, CreateTensorWithDataAsOrtValue, self.mem, data.as_mut_ptr() as *mut c_void, data.len() * 4, shape.as_ptr(), shape.len(), ONNXTensorElementDataType_ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &mut input)?;
        let names = [self.input.as_ptr()];
        let out_names: Vec<*const c_char> = outputs.iter().map(|c| c.as_ptr()).collect();
        let mut outs: Vec<*mut OrtValue> = vec![ptr::null_mut(); outputs.len()];
        let values = [input as *const OrtValue];
        let r = call!(a, Run, self.session, ptr::null(), names.as_ptr(), values.as_ptr(), 1, out_names.as_ptr(), outputs.len(), outs.as_mut_ptr());
        unsafe {
            if let Some(f) = a.ReleaseValue {
                f(input);
            }
        }
        r?;
        let mut all = Vec::with_capacity(outs.len());
        let mut err = None;
        for out in outs {
            if err.is_none() {
                match self.read(out) {
                    Ok(v) => all.push(v),
                    Err(e) => err = Some(e),
                }
            }
            unsafe {
                if let Some(f) = a.ReleaseValue
                    && !out.is_null()
                {
                    f(out);
                }
            }
        }
        match err {
            Some(e) => Err(e),
            None => Ok(all),
        }
    }

    fn read(&self, out: *mut OrtValue) -> Result<(Vec<f32>, Vec<i64>), String> {
        let a = self.a;
        let result = (|| -> Result<(Vec<f32>, Vec<i64>), String> {
            let mut info: *mut OrtTensorTypeAndShapeInfo = ptr::null_mut();
            call!(a, GetTensorTypeAndShape, out, &mut info)?;
            let mut dims = 0usize;
            let mut n = 0usize;
            let r1 = call!(a, GetDimensionsCount, info, &mut dims);
            let mut shape = vec![0i64; dims];
            let r2 = if r1.is_ok() { call!(a, GetDimensions, info, shape.as_mut_ptr(), dims) } else { Ok(()) };
            let r3 = call!(a, GetTensorShapeElementCount, info, &mut n);
            unsafe {
                if let Some(f) = a.ReleaseTensorTypeAndShapeInfo {
                    f(info);
                }
            }
            r1?;
            r2?;
            r3?;
            let mut p: *mut c_void = ptr::null_mut();
            call!(a, GetTensorMutableData, out, &mut p)?;
            Ok((unsafe { std::slice::from_raw_parts(p as *const f32, n) }.to_vec(), shape))
        })();
        result
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            if let Some(r) = self.a.ReleaseMemoryInfo
                && !self.mem.is_null()
            {
                r(self.mem);
            }
            if let Some(r) = self.a.ReleaseSession {
                r(self.session);
            }
            if let Some(r) = self.a.ReleaseEnv {
                r(self.env);
            }
        }
    }
}

