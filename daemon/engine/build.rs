use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = env::var("LLAMA_CPP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../llama.cpp"))
        .canonicalize()
        .expect("llama.cpp directory not found; set LLAMA_CPP_DIR");
    let lib = env::var("LLAMA_CPP_LIB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../build/llama-cpp/bin"))
        .canonicalize()
        .expect("llama.cpp libraries not found; run scripts/build-llama.sh");
    println!("cargo:rerun-if-env-changed=SETU_PACKAGING");
    println!("cargo:rerun-if-env-changed=LLAMA_CPP_DIR");
    println!("cargo:rerun-if-env-changed=LLAMA_CPP_LIB");
    println!("cargo:rerun-if-changed={}", root.join("include/llama.h").display());
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=llama");
    println!("cargo:rustc-link-lib=dylib=ggml");
    println!("cargo:rustc-link-lib=dylib=ggml-base");
    println!("cargo:rustc-env=SETU_LLAMA_LIB={}", lib.display());
    let sherpa = env::var("DEP_SHERPA_ONNX_C_API_LIB_DIR").unwrap_or_default();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,--disable-new-dtags");
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib/setu");
        if env::var("SETU_PACKAGING").is_err() {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{}", lib.display());
            if !sherpa.is_empty() {
                println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{sherpa}");
            }
        }
    }
    let bindings = bindgen::Builder::default()
        .header(root.join("include/llama.h").to_string_lossy())
        .clang_arg(format!("-I{}", root.join("include").display()))
        .clang_arg(format!("-I{}", root.join("ggml/include").display()))
        .allowlist_function("llama_.*")
        .allowlist_type("llama_.*")
        .allowlist_function("ggml_backend_load_all_from_path")
        .allowlist_function("ggml_backend_dev_.*")
        .allowlist_function("ggml_backend_reg_name")
        .allowlist_type("ggml_backend_dev_type")
        .derive_default(true)
        .layout_tests(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("bindgen failed for llama.h");
    bindings.write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("llama.rs")).unwrap();
    let mtmd = root.join("tools/mtmd");
    println!("cargo:rerun-if-changed={}", mtmd.join("mtmd.h").display());
    println!("cargo:rerun-if-changed={}", mtmd.join("mtmd-helper.h").display());
    let vision = bindgen::Builder::default()
        .header(mtmd.join("mtmd.h").to_string_lossy())
        .header(mtmd.join("mtmd-helper.h").to_string_lossy())
        .clang_arg(format!("-I{}", root.join("include").display()))
        .clang_arg(format!("-I{}", root.join("ggml/include").display()))
        .clang_arg(format!("-I{}", mtmd.display()))
        .allowlist_function("mtmd_context_params_default")
        .allowlist_function("mtmd_init_from_file")
        .allowlist_function("mtmd_free")
        .allowlist_function("mtmd_default_marker")
        .allowlist_function("mtmd_support_vision")
        .allowlist_function("mtmd_bitmap_free")
        .allowlist_function("mtmd_input_chunks_init")
        .allowlist_function("mtmd_input_chunks_free")
        .allowlist_function("mtmd_tokenize")
        .allowlist_function("mtmd_helper_bitmap_init_from_buf")
        .allowlist_function("mtmd_helper_init_opt_default")
        .allowlist_function("mtmd_helper_get_n_tokens")
        .allowlist_function("mtmd_helper_eval_chunks")
        .allowlist_function("mtmd_helper_log_set")
        .blocklist_type("llama_.*")
        .blocklist_type("ggml_.*")
        .blocklist_type("_bindgen_ty_.*")
        .dynamic_library_name("Mtmd")
        .dynamic_link_require_all(true)
        .derive_default(true)
        .layout_tests(false)
        .rust_edition(bindgen::RustEdition::Edition2024)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("bindgen failed for mtmd.h");
    vision.write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("mtmd.rs")).unwrap();
}
