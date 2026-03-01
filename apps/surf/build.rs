fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent().unwrap() // apps/
        .parent().unwrap() // project root
        .to_path_buf();
    let link_ld = project_root.join("libs").join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // Link BearSSL static library (architecture-specific)
    if target_arch == "x86_64" {
        let bearssl_dir = project_root.join("third_party").join("bearssl").join("build_x64");
        println!("cargo:rustc-link-search=native={}", bearssl_dir.display());
        println!("cargo:rustc-link-lib=static=bearssl_x64");
    } else if target_arch == "aarch64" {
        // ARM64 BearSSL not yet built — TLS support disabled at link time.
        // Build third_party/bearssl for aarch64 to enable.
    }
}
