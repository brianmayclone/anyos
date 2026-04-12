//! Build script (build.rs) handling for ccargo.
//!
//! The current anyOS tree uses a small set of Cargo-style `build.rs` patterns:
//! - emit `cargo:rustc-link-arg=-T<.../libs/stdlib/link.ld>`
//! - emit `cargo:rerun-if-changed=<.../libs/stdlib/link.ld>`
//! - emit `cargo:rerun-if-env-changed=ANYOS_VERSION`
//! - optionally emit `cargo:rustc-env=ANYOS_VERSION=<value>`
//!
//! Rather than compiling those helper scripts with the in-system compiler,
//! we emulate the directives directly here. That keeps self-hosted builds
//! reliable while still respecting the information the scripts carry.

use crate::prelude::*;
use crate::fs;
use anyos_std::println;

/// Parsed output from a build script execution.
#[derive(Debug, Clone, Default)]
pub struct BuildScriptOutput {
    /// Cfg flags to pass to the compiler (from `cargo:rustc-cfg=...`).
    pub cfg_flags: Vec<String>,
    /// Linker arguments (from `cargo:rustc-link-arg=...`).
    pub link_args: Vec<String>,
    /// Link library names (from `cargo:rustc-link-lib=...`).
    pub link_libs: Vec<String>,
    /// Link search paths (from `cargo:rustc-link-search=...`).
    pub link_search: Vec<String>,
    /// Environment variables (from `cargo:rustc-env=KEY=VALUE`).
    pub env_vars: Vec<(String, String)>,
    /// Files that trigger a rebuild (from `cargo:rerun-if-changed=...`).
    pub rerun_if_changed: Vec<String>,
    /// Environment vars that trigger a rebuild (from `cargo:rerun-if-env-changed=...`).
    pub rerun_if_env_changed: Vec<String>,
    /// Warnings (from `cargo:warning=...`).
    pub warnings: Vec<String>,
}

/// Check if a crate has a build script and execute it.
/// Returns the parsed build script output, or None if no build.rs exists.
pub fn run_build_script(
    manifest_dir: &str,
    crate_name: &str,
    target_dir: &str,
    release: bool,
) -> Option<BuildScriptOutput> {
    let build_rs = format!("{}/build.rs", manifest_dir);
    if !fs::file_exists(&build_rs) {
        return None;
    }

    // Read build.rs source
    let source = fs::read_file(&build_rs)?;
    anyos_std::env::set("CARGO_MANIFEST_DIR", manifest_dir);
    anyos_std::env::set("OUT_DIR", &format!("{}/out", target_dir));
    anyos_std::env::set("TARGET", "x86_64-anyos");
    anyos_std::env::set("HOST", "x86_64-anyos");
    anyos_std::env::set("PROFILE", if release { "release" } else { "debug" });
    anyos_std::env::set("OPT_LEVEL", if release { "2" } else { "0" });
    fs::mkdir_p(&format!("{}/out", target_dir));

    Some(emulate_build_script_output(&source, manifest_dir, crate_name))
}

/// Parse cargo: directives from build script stdout.
pub fn parse_build_script_output(output: &str) -> BuildScriptOutput {
    let mut result = BuildScriptOutput::default();

    for line in output.lines() {
        let line = line.trim();
        if !line.starts_with("cargo:") {
            continue;
        }
        let directive = &line[6..];

        if let Some(val) = directive.strip_prefix("rustc-cfg=") {
            result.cfg_flags.push(val.to_string());
        } else if let Some(val) = directive.strip_prefix("rustc-link-arg=") {
            result.link_args.push(val.to_string());
        } else if let Some(val) = directive.strip_prefix("rustc-link-lib=") {
            result.link_libs.push(val.to_string());
        } else if let Some(val) = directive.strip_prefix("rustc-link-search=") {
            // Strip optional kind prefix like "native="
            let path = if let Some(eq_pos) = val.find('=') {
                &val[eq_pos + 1..]
            } else {
                val
            };
            result.link_search.push(path.to_string());
        } else if let Some(val) = directive.strip_prefix("rustc-env=") {
            if let Some(eq_pos) = val.find('=') {
                result.env_vars.push((
                    val[..eq_pos].to_string(),
                    val[eq_pos + 1..].to_string(),
                ));
            }
        } else if let Some(val) = directive.strip_prefix("rerun-if-changed=") {
            result.rerun_if_changed.push(val.to_string());
        } else if let Some(val) = directive.strip_prefix("rerun-if-env-changed=") {
            result.rerun_if_env_changed.push(val.to_string());
        } else if let Some(val) = directive.strip_prefix("warning=") {
            result.warnings.push(val.to_string());
            println!("warning: build script: {}", val);
        }
    }

    result
}

fn emulate_build_script_output(
    source: &str,
    manifest_dir: &str,
    crate_name: &str,
) -> BuildScriptOutput {
    let mut result = BuildScriptOutput::default();

    if source.contains("cargo:rerun-if-env-changed=ANYOS_VERSION") {
        result.rerun_if_env_changed.push(String::from("ANYOS_VERSION"));
        if let Some(ver) = env_value("ANYOS_VERSION") {
            result.env_vars.push((String::from("ANYOS_VERSION"), ver));
        }
    }

    if source.contains("cargo:rustc-link-arg=-T") {
        if let Some(link_ld) = find_stdlib_linker_script(manifest_dir) {
            result.link_args.push(format!("-T{}", link_ld));
            result.rerun_if_changed.push(link_ld);
        } else {
            println!(
                "ccargo: warning: could not resolve libs/stdlib/link.ld for `{}`",
                crate_name
            );
        }
    }

    if result.cfg_flags.is_empty()
        && result.link_args.is_empty()
        && result.env_vars.is_empty()
        && result.rerun_if_changed.is_empty()
        && result.rerun_if_env_changed.is_empty()
    {
        println!(
            "ccargo: warning: build script for `{}` uses unsupported patterns; ignoring it",
            crate_name
        );
    }

    result
}

fn find_stdlib_linker_script(manifest_dir: &str) -> Option<String> {
    let mut current = String::from(manifest_dir);
    loop {
        let candidate = format!("{}/libs/stdlib/link.ld", current);
        if fs::file_exists(&candidate) {
            return Some(candidate);
        }

        let parent = parent_dir(&current);
        if parent == current {
            break;
        }
        current = parent;
    }
    None
}

fn parent_dir(path: &str) -> String {
    if path == "/" {
        return String::from("/");
    }

    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => String::from("/"),
        Some(idx) => String::from(&trimmed[..idx]),
        None => String::from("."),
    }
}

fn env_value(key: &str) -> Option<String> {
    let mut buf = [0u8; 257];
    let len = anyos_std::env::get(key, &mut buf);
    if len == u32::MAX {
        return None;
    }
    core::str::from_utf8(&buf[..len as usize]).ok().map(String::from)
}
