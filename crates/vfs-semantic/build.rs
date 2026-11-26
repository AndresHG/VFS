//! The `vfs-embed` worker links ONNX Runtime statically. On macOS its Core ML helper calls
//! `__isPlatformVersionAtLeast`, which lives in clang's runtime library; rustc links with
//! `-nodefaultlibs`, so that library is added here, for the worker binary only.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let embedder = std::env::var_os("CARGO_FEATURE_EMBEDDER").is_some();
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if !(embedder && macos) {
        return;
    }
    let out = std::process::Command::new("xcrun")
        .args(["clang", "--print-runtime-dir"])
        .output()
        .expect("xcrun clang --print-runtime-dir");
    let dir = String::from_utf8(out.stdout).expect("utf-8 path");
    let lib = std::path::Path::new(dir.trim()).join("libclang_rt.osx.a");
    assert!(lib.is_file(), "missing {}", lib.display());
    println!("cargo:rustc-link-arg-bins={}", lib.display());
}
