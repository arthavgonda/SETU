fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-lib=dylib=pulse-simple");
        println!("cargo:rustc-link-lib=dylib=pulse");
    }
}
