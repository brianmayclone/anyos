fn main() {
    println!("cargo:rerun-if-env-changed=ANYOS_VERSION");
    if let Ok(ver) = std::env::var("ANYOS_VERSION") {
        println!("cargo:rustc-env=ANYOS_VERSION={}", ver);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent().unwrap() // system/
        .parent().unwrap() // project root
        .to_path_buf();
    let link_ld = project_root.join("libs").join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());
}
