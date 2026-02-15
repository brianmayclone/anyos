fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let link_ld = std::path::PathBuf::from(&manifest_dir).join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());
}
