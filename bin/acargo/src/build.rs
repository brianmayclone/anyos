//! Build engine: compiles crates in dependency order using anyrc.

use crate::prelude::*;
use anyos_std::println;
use anyos_std::collections::HashMap;
use crate::resolve::{self, BuildNode};
use crate::manifest::CrateKind;
use crate::fs;

/// Build configuration.
pub struct BuildConfig {
    pub release: bool,
    pub verbose: bool,
    pub jobs: u32,
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
    let nodes = resolve::resolve(root_dir);
    if nodes.is_empty() {
        println!("acargo: error: no packages found");
        return BuildResult { success: false, bin_path: None, compiled: 0 };
    }

    let order = resolve::topological_sort(&nodes);

    // Setup output directories
    let target_dir = format!("{}/target", root_dir);
    let profile_dir = if config.release {
        format!("{}/release", target_dir)
    } else {
        format!("{}/debug", target_dir)
    };
    let deps_dir = format!("{}/deps", profile_dir);

    fs::mkdir_p(&target_dir);
    fs::mkdir_p(&profile_dir);
    fs::mkdir_p(&deps_dir);

    // Track built crates: normalized name -> rlib path
    let mut built_rlibs: HashMap<String, String> = HashMap::new();
    let mut final_bin_path: Option<String> = None;
    let mut compiled = 0usize;

    for &idx in &order {
        let node = &nodes[idx];

        // Read source file
        let source = match fs::read_file(&node.src_file) {
            Some(s) => s,
            None => {
                println!("error[E0001]: cannot read `{}`", node.src_file);
                return BuildResult { success: false, bin_path: None, compiled };
            }
        };

        let is_lib = node.manifest.crate_type == CrateKind::Lib;
        let norm_name = node.name.replace('-', "_");

        // Determine output format
        let emit = if is_lib {
            anyrc::driver::EmitKind::Rlib
        } else {
            anyrc::driver::EmitKind::Exe
        };

        let output_path = if is_lib {
            format!("{}/lib{}.rlib", deps_dir, norm_name)
        } else {
            let bin_name = node.manifest.bin_name.as_deref().unwrap_or(&node.name);
            let path = format!("{}/{}", profile_dir, bin_name);
            final_bin_path = Some(path.clone());
            path
        };

        let opt = if config.release {
            node.manifest.opt_level_release
        } else {
            node.manifest.opt_level_dev
        };

        // Collect extern crate references
        let mut externs = Vec::new();
        for &dep_idx in &node.deps {
            let dep_norm = nodes[dep_idx].name.replace('-', "_");
            if let Some(rlib_path) = built_rlibs.get(&dep_norm) {
                externs.push(anyrc::driver::ExternCrateSpec {
                    name: dep_norm,
                    rlib_path: rlib_path.clone(),
                });
            }
        }

        // Derive source directory for module resolution
        let src_dir = if let Some(pos) = node.src_file.rfind('/') {
            String::from(&node.src_file[..pos])
        } else {
            String::from(".")
        };

        if config.verbose {
            println!("   Compiling {} v{}", node.name, node.manifest.version);
        }

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
            src_dir: Some(src_dir),
            extern_crates: externs,
        };

        match anyrc::driver::compile(&source, &node.src_file, &options) {
            Ok(bytes) => {
                fs::write_file(&output_path, &bytes);
                if is_lib {
                    built_rlibs.insert(norm_name, output_path);
                }
                compiled += 1;
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
    println!("    Finished `{}` profile [{}] target(s) in {} crate(s)", profile_name, opt_label, compiled);

    BuildResult {
        success: true,
        bin_path: final_bin_path,
        compiled,
    }
}
