use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = env::var("SHERPA_ONNX_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../third_party/sherpa-onnx"));
    let root = root.canonicalize().expect("sherpa-onnx directory not found; run scripts/fetch-deps.sh");
    let include = root.join("include");
    let lib = root.join("lib");
    println!("cargo:rerun-if-changed={}", include.display());
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_DIR");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    println!("cargo:lib_dir={}", lib.display());
    println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{}", lib.display());
    let bindings = bindgen::Builder::default()
        .header(include.join("sherpa-onnx/c-api/c-api.h").to_string_lossy())
        .clang_arg(format!("-I{}", include.display()))
        .allowlist_function("SherpaOnnx.*")
        .allowlist_type("SherpaOnnx.*")
        .derive_default(true)
        .layout_tests(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("bindgen failed for sherpa-onnx");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out.join("sherpa.rs")).unwrap();
    let ort = env::var("ORT_INCLUDE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../third_party/onnxruntime/include"))
        .canonicalize()
        .expect("onnxruntime headers not found; run scripts/fetch-deps.sh");
    println!("cargo:rerun-if-changed={}", ort.display());
    println!("cargo:rerun-if-env-changed=ORT_INCLUDE_DIR");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");
    let ob = bindgen::Builder::default()
        .header(ort.join("onnxruntime_c_api.h").to_string_lossy())
        .clang_arg(format!("-I{}", ort.display()))
        .allowlist_function("OrtGetApiBase")
        .allowlist_type("OrtApi")
        .allowlist_type("OrtApiBase")
        .allowlist_type("ONNXTensorElementDataType")
        .allowlist_type("OrtAllocatorType")
        .allowlist_type("OrtMemType")
        .allowlist_type("OrtLoggingLevel")
        .allowlist_type("GraphOptimizationLevel")
        .allowlist_var("ORT_API_VERSION")
        .generate_comments(false)
        .derive_default(true)
        .layout_tests(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("bindgen failed for onnxruntime");
    ob.write_to_file(out.join("ort.rs")).unwrap();
}
