fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let stdlib = std::path::Path::new(&manifest).join("../../stdlib/link.ld");
    println!("cargo:rustc-link-arg=-T{}", stdlib.display());
}
