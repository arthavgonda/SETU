use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=SETU_PACKAGING");
    let sherpa = env::var("DEP_SHERPA_ONNX_C_API_LIB_DIR").unwrap_or_default();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") && env::var("SETU_PACKAGING").is_err() && !sherpa.is_empty() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{sherpa}");
    }
}
