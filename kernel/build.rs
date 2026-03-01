fn main() {
    println!("cargo:rerun-if-env-changed=ANYOS_VERSION");
    if let Ok(ver) = std::env::var("ANYOS_VERSION") {
        println!("cargo:rustc-env=ANYOS_VERSION={}", ver);
    }

    println!("cargo:rerun-if-env-changed=ANYOS_ASM_OBJECTS");
    if let Ok(objects) = std::env::var("ANYOS_ASM_OBJECTS") {
        for obj in objects.split(',') {
            let obj = obj.trim();
            if !obj.is_empty() {
                println!("cargo:rustc-link-arg={}", obj);
                println!("cargo:rerun-if-changed={}", obj);
            }
        }
    }

    // Pass AP trampoline binary path as a cfg variable for include_bytes!
    println!("cargo:rerun-if-env-changed=ANYOS_AP_TRAMPOLINE");
    if let Ok(path) = std::env::var("ANYOS_AP_TRAMPOLINE") {
        println!("cargo:rustc-env=ANYOS_AP_TRAMPOLINE={}", path);
        println!("cargo:rerun-if-changed={}", path);
    }

    // Select linker script based on target architecture
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = std::env::var("TARGET").unwrap_or_default();

    if target.starts_with("aarch64") {
        println!("cargo:rustc-link-arg=-T{}/link_arm64.ld", manifest_dir);
        println!("cargo:rerun-if-changed=link_arm64.ld");
    } else {
        println!("cargo:rustc-link-arg=-T{}/link.ld", manifest_dir);
        println!("cargo:rerun-if-changed=link.ld");
    }
}
