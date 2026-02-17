fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent().unwrap() // apps/
        .parent().unwrap() // project root
        .to_path_buf();
    let link_ld = project_root.join("libs").join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());

    // Link BearSSL x64 static library (includes anyos_tls.c wrapper)
    let bearssl_dir = project_root.join("third_party").join("bearssl").join("build_x64");
    println!("cargo:rustc-link-search=native={}", bearssl_dir.display());
    println!("cargo:rustc-link-lib=static=bearssl_x64");
}
