fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "none" {
        return;
    }

    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let stdlib = std::path::Path::new(&manifest).join("../../libs/stdlib/link.ld");
    println!("cargo:rustc-link-arg=-T{}", stdlib.display());
}
