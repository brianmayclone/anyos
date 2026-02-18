fn main() {
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

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{}/link.ld", manifest_dir);
    println!("cargo:rerun-if-changed=link.ld");
}
