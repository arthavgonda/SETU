use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let bundled = manifest.join("../../third_party/bundled");
    println!("cargo:rustc-env=SETU_DEV_ASSETS={}", bundled.display());
    println!("cargo:rustc-env=SETU_BUILD={}", env::var("SETU_BUILD_ID").unwrap_or_else(|_| "dev".into()));
    println!("cargo:rerun-if-env-changed=SETU_PACKAGING");
    println!("cargo:rerun-if-env-changed=SETU_BUILD_ID");
    println!("cargo:rerun-if-changed=assets");
    let pictures = manifest.join("../../assets");
    let mut looks = String::new();
    for (name, dir) in [("AVATARS", "avatars"), ("BACKGROUNDS", "avatars_bg")] {
        let d = pictures.join(dir);
        println!("cargo:rerun-if-changed={}", d.display());
        let mut stems: Vec<String> = std::fs::read_dir(&d)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png"))).filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect())
            .unwrap_or_default();
        stems.sort();
        looks.push_str(&format!("pub const {name}: &[&str] = &{stems:?};\n"));
    }
    std::fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("looks.rs"), looks).unwrap();
    let sherpa = env::var("DEP_SHERPA_ONNX_C_API_LIB_DIR").unwrap_or_default();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,--disable-new-dtags");
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN/../lib/setu");
        if env::var("SETU_PACKAGING").is_err() && !sherpa.is_empty() {
            println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{sherpa}");
        }
    }
}
