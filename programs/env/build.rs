fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent()
        .unwrap() // programs/
        .parent()
        .unwrap() // project root
        .to_path_buf();
    let link_ld = project_root.join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());
}
