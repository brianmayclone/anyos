//! Build script (build.rs) execution and output parsing.
//!
//! Compiles and runs build.rs scripts, parsing their `cargo:` output directives
//! to extract linker arguments, cfg flags, environment variables, and other
//! build-time configuration.

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

    // Compile build.rs as a host binary
    let build_bin = format!("{}/build-script-{}", target_dir, crate_name);

    let options = anyrc::driver::CompileOptions {
        input: build_rs.clone(),
        output: build_bin.clone(),
        emit: anyrc::driver::EmitKind::Exe,
        opt_level: 0,
        crate_type: anyrc::driver::CrateType::Bin,
        crate_name: Some(format!("build_script_{}", crate_name)),
        src_dir: Some(manifest_dir.to_string()),
        extern_crates: Vec::new(),
        cfg_flags: Vec::new(),
        linker_script: None,
        link_args: Vec::new(),
        env_vars: Vec::new(),
        features: Vec::new(),
    };

    match anyrc::driver::compile(&source, &build_rs, &options) {
        Ok(bytes) => {
            fs::write_file(&build_bin, &bytes);
        }
        Err(errors) => {
            let source_map = anyrc::diagnostics::SourceMap::new(
                build_rs.clone(),
                source,
            );
            println!("ccargo: error compiling build script for `{}`", crate_name);
            for err in &errors {
                println!("{}", err.render(&source_map));
            }
            return None;
        }
    }

    // Set up environment variables for the build script
    anyos_std::env::set("CARGO_MANIFEST_DIR", manifest_dir);
    anyos_std::env::set("OUT_DIR", &format!("{}/out", target_dir));
    anyos_std::env::set("TARGET", "x86_64-anyos");
    anyos_std::env::set("HOST", "x86_64-anyos");
    anyos_std::env::set("PROFILE", if release { "release" } else { "debug" });
    anyos_std::env::set("OPT_LEVEL", if release { "2" } else { "0" });

    // Create OUT_DIR
    fs::mkdir_p(&format!("{}/out", target_dir));

    // Execute the build script and capture output
    let output_file = format!("{}/build-script-output-{}", target_dir, crate_name);
    let cmd = format!("{} > {}", build_bin, output_file);
    let status = anyos_std::process::exec(&build_bin, &cmd);

    if status != 0 {
        println!("ccargo: warning: build script for `{}` exited with {}", crate_name, status);
    }

    // Parse the output
    let output_str = fs::read_file(&output_file).unwrap_or_default();
    Some(parse_build_script_output(&output_str))
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
