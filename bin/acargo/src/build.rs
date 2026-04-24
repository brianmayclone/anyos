//! Build engine: compiles crates in dependency order using crust.
//!
//! Supports build scripts (build.rs), feature resolution, incremental
//! compilation via mtime fingerprinting, and dependency-ordered compilation.

use crate::prelude::*;
use anyos_std::println;
use anyos_std::collections::HashMap;
use crate::resolve::{self, BuildNode};
use crate::manifest::CrateKind;
use crate::build_script;
use crate::workspace;
use crate::fingerprint;
use crate::fs;

/// Build configuration.
pub struct BuildConfig {
    pub release: bool,
    pub verbose: bool,
    pub jobs: u32,
    /// Cfg flags passed to the compiler (e.g. target_arch="x86_64").
    pub cfg_flags: Vec<String>,
    /// Active feature flags (from --features or default features).
    pub features: Vec<String>,
    /// Linker script path.
    pub linker_script: Option<String>,
    /// Additional linker arguments (object files, etc.).
    pub link_args: Vec<String>,
    /// Environment variables for env!() macro.
    pub env_vars: Vec<(String, String)>,
    /// Target specification (e.g. "x86_64-anyos").
    pub target: Option<String>,
}

/// Build result.
pub struct BuildResult {
    pub success: bool,
    /// Path to the final binary (if bin crate).
    pub bin_path: Option<String>,
    /// Number of crates compiled.
    pub compiled: usize,
}

/// Build all crates in a project.
pub fn build(root_dir: &str, config: &BuildConfig) -> BuildResult {
    let root_dir = fs::absolutize(root_dir);
    let nodes = resolve::resolve(&root_dir, &config.features);
    if nodes.is_empty() {
        println!("ccargo: error: no packages found");
        return BuildResult { success: false, bin_path: None, compiled: 0 };
    }

    let order = resolve::topological_sort(&nodes);
    if order.len() != nodes.len() {
        println!("ccargo: error: dependency graph is not acyclic");
        return BuildResult { success: false, bin_path: None, compiled: 0 };
    }

    // Setup output directories
    let target_dir = format!("{}/target", root_dir);
    let profile_dir = if config.release {
        format!("{}/release", target_dir)
    } else {
        format!("{}/debug", target_dir)
    };
    let deps_dir = format!("{}/deps", profile_dir);
    let build_dir = format!("{}/build", profile_dir);
    let fp_dir = format!("{}/.fingerprint", profile_dir);

    fs::mkdir_p(&target_dir);
    fs::mkdir_p(&profile_dir);
    fs::mkdir_p(&deps_dir);
    fs::mkdir_p(&build_dir);
    fs::mkdir_p(&fp_dir);

    // Build cfg flags shared by all crates. Cargo feature cfgs are added per crate.
    let mut all_cfg_flags = config.cfg_flags.clone();
    // Add target_arch cfg if target is specified
    if let Some(ref target) = config.target {
        if target.starts_with("x86_64") {
            all_cfg_flags.push(String::from("target_arch=\"x86_64\""));
            all_cfg_flags.push(String::from("target_pointer_width=\"64\""));
            all_cfg_flags.push(String::from("target_endian=\"little\""));
        } else if target.starts_with("aarch64") {
            all_cfg_flags.push(String::from("target_arch=\"aarch64\""));
            all_cfg_flags.push(String::from("target_pointer_width=\"64\""));
            all_cfg_flags.push(String::from("target_endian=\"little\""));
        }
        if target.contains("anyos") {
            all_cfg_flags.push(String::from("target_os=\"anyos\""));
        }
    } else {
        // Default to x86_64 on anyOS
        all_cfg_flags.push(String::from("target_arch=\"x86_64\""));
        all_cfg_flags.push(String::from("target_pointer_width=\"64\""));
        all_cfg_flags.push(String::from("target_endian=\"little\""));
        all_cfg_flags.push(String::from("target_os=\"anyos\""));
    }

    // Track built crates: normalized name -> rlib path
    let mut built_rlibs: HashMap<String, String> = HashMap::new();
    let mut final_bin_path: Option<String> = None;
    let mut compiled = 0usize;
    let mut skipped = 0usize;

    // Accumulate build script outputs
    let mut global_link_args: Vec<String> = config.link_args.clone();
    let mut global_env_vars: Vec<(String, String)> = config.env_vars.clone();
    let mut global_linker_script = config.linker_script.clone();

    for &idx in &order {
        let node = &nodes[idx];
        let resolved_features = node.active_features.clone();
        let is_lib = node.manifest.crate_type == CrateKind::Lib;
        let norm_name = node.name.replace('-', "_");

        let output_path = if is_lib {
            format!("{}/lib{}.rlib", deps_dir, norm_name)
        } else {
            let bin_name = node.manifest.bin_name.as_deref().unwrap_or(&node.name);
            let path = format!("{}/{}", profile_dir, bin_name);
            final_bin_path = Some(path.clone());
            path
        };

        // Derive source directory for module resolution
        let src_dir = if let Some(pos) = node.src_file.rfind('/') {
            String::from(&node.src_file[..pos])
        } else {
            String::from(".")
        };

        // Check for build script (build.rs) and execute if present
        let build_script_output = build_script::run_build_script(
            &node.manifest_dir,
            &norm_name,
            &build_dir,
            config.release,
            &resolved_features,
        );

        // Merge build script outputs into our compile options
        let mut crate_cfg_flags = all_cfg_flags.clone();
        for feat in &resolved_features {
            crate_cfg_flags.push(format!("feature=\"{}\"", feat));
        }
        let mut crate_link_args = Vec::new();
        let mut crate_env_vars = global_env_vars.clone();
        let mut crate_linker_script = global_linker_script.clone();

        if let Some(ref bs_output) = build_script_output {
            for cfg in &bs_output.cfg_flags {
                crate_cfg_flags.push(cfg.clone());
            }
            for arg in &bs_output.link_args {
                // Check if this is a linker script reference
                if arg.starts_with("-T") {
                    crate_linker_script = Some(arg[2..].to_string());
                } else {
                    crate_link_args.push(arg.clone());
                    global_link_args.push(arg.clone());
                }
            }
            for (key, val) in &bs_output.env_vars {
                crate_env_vars.push((key.clone(), val.clone()));
                // Also set in process env for subsequent build scripts
                anyos_std::env::set(key, val);
            }
        }

        // Add accumulated global link args for the final binary
        if !is_lib {
            crate_link_args.extend(global_link_args.iter().cloned());
        }

        // Incremental compilation: check fingerprint
        let opt_hash = fingerprint::hash_options(
            if config.release { node.manifest.opt_level_release } else { node.manifest.opt_level_dev },
            &crate_cfg_flags,
            &resolved_features,
            config.release,
        );

        if fingerprint::is_fresh(&fp_dir, &norm_name, &node.src_file, &output_path, &src_dir) {
            if config.verbose {
                println!("       Fresh {} v{}", node.name, node.manifest.version);
            }
            if is_lib {
                built_rlibs.insert(norm_name, output_path);
            }
            skipped += 1;
            continue;
        }

        // Read source file
        let source = match fs::read_file(&node.src_file) {
            Some(s) => s,
            None => {
                println!("error[E0001]: cannot read `{}`", node.src_file);
                return BuildResult { success: false, bin_path: None, compiled };
            }
        };

        let opt = if config.release {
            node.manifest.opt_level_release
        } else {
            node.manifest.opt_level_dev
        };

        // Collect extern crate references. anyrc currently consumes source
        // interface wrappers, so re-exported APIs need transitive dependency
        // interfaces to be visible to downstream crates.
        let mut externs = Vec::new();
        let mut extern_indices = Vec::new();
        collect_transitive_dep_indices(&nodes, idx, &mut extern_indices);
        for dep_idx in extern_indices {
            let dep_norm = nodes[dep_idx].name.replace('-', "_");
            if let Some(rlib_path) = built_rlibs.get(&dep_norm) {
                externs.push(anyrc::driver::ExternCrateSpec {
                    name: dep_norm,
                    rlib_path: rlib_path.clone(),
                });
            }
        }

        if config.verbose {
            println!("   Compiling {} v{}", node.name, node.manifest.version);
        }

        let emit = if is_lib {
            anyrc::driver::EmitKind::Rlib
        } else {
            anyrc::driver::EmitKind::Exe
        };

        let options = anyrc::driver::CompileOptions {
            input: node.src_file.clone(),
            output: output_path.clone(),
            emit,
            opt_level: opt,
            crate_type: if is_lib {
                anyrc::driver::CrateType::Lib
            } else {
                anyrc::driver::CrateType::Bin
            },
            crate_name: Some(norm_name.clone()),
            src_dir: Some(src_dir.clone()),
            extern_crates: externs,
            cfg_flags: crate_cfg_flags,
            linker_script: if !is_lib { crate_linker_script } else { None },
            link_args: if !is_lib { crate_link_args } else { Vec::new() },
            env_vars: crate_env_vars,
            features: resolved_features.clone(),
        };

        match anyrc::driver::compile(&source, &node.src_file, &options) {
            Ok(bytes) => {
                fs::write_file(&output_path, &bytes);
                if is_lib {
                    built_rlibs.insert(norm_name.clone(), output_path.clone());
                }
                compiled += 1;

                // Write fingerprint for successful compilation
                fingerprint::write_fingerprint(&fp_dir, &norm_name, &node.src_file, &output_path, opt_hash);
            }
            Err(errors) => {
                let source_map = anyrc::diagnostics::SourceMap::new(
                    node.src_file.clone(),
                    source,
                );
                println!("error: could not compile `{}`", node.name);
                for err in &errors {
                    println!("{}", err.render(&source_map));
                }
                return BuildResult { success: false, bin_path: None, compiled };
            }
        }
    }

    // Summary
    let profile_name = if config.release { "release" } else { "dev" };
    let opt_label = if config.release { "optimized" } else { "unoptimized" };
    if skipped > 0 {
        println!("    Finished `{}` profile [{}] target(s) — {} compiled, {} fresh",
            profile_name, opt_label, compiled, skipped);
    } else {
        println!("    Finished `{}` profile [{}] target(s) in {} crate(s)",
            profile_name, opt_label, compiled);
    }

    BuildResult {
        success: true,
        bin_path: final_bin_path,
        compiled,
    }
}

fn collect_transitive_dep_indices(nodes: &[BuildNode], idx: usize, out: &mut Vec<usize>) {
    for &dep_idx in &nodes[idx].deps {
        if out.contains(&dep_idx) {
            continue;
        }
        collect_transitive_dep_indices(nodes, dep_idx, out);
        out.push(dep_idx);
    }
}
