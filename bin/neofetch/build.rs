// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "none" {
        return;
    }

    println!("cargo:rerun-if-env-changed=ANYOS_VERSION");
    if let Ok(ver) = std::env::var("ANYOS_VERSION") {
        println!("cargo:rustc-env=ANYOS_VERSION={}", ver);
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = std::path::PathBuf::from(&manifest_dir)
        .parent()
        .unwrap() // bin/
        .parent()
        .unwrap() // project root
        .to_path_buf();
    let link_ld = project_root.join("libs").join("stdlib").join("link.ld");
    println!("cargo:rustc-link-arg=-T{}", link_ld.display());
    println!("cargo:rerun-if-changed={}", link_ld.display());
}
