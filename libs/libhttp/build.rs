fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let wrapper_src = std::path::PathBuf::from(&manifest_dir).join("anyos_tls.c");
    let bearssl_script = project_root.join("scripts").join("build_bearssl_x64.sh");
    let bearssl_dir = project_root.join("third_party").join("bearssl").join("build_x64");
    let bearssl_lib = bearssl_dir.join("libbearssl_x64.a");

    println!("cargo:rerun-if-changed={}", wrapper_src.display());
    println!("cargo:rerun-if-changed={}", bearssl_script.display());
    println!("cargo:rerun-if-changed={}", bearssl_lib.display());
}
