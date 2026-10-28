fn main() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("link.ld");
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rustc-link-arg-bins=-T{}", script.display());
    println!("cargo:rustc-link-arg-bins=--no-pie");
    println!("cargo:rustc-link-arg-bins=--build-id=none");
    println!("cargo:rustc-link-arg-bins=--strip-debug");
}
