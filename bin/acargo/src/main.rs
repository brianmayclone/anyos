#![no_std]
#![cfg_attr(not(feature = "host"), no_main)]

//! ccargo — Rust build system for anyOS
//!
//! Compatible subset of Cargo functionality for building Rust projects
//! natively on anyOS using the crust compiler.

#[cfg(feature = "host")]
extern crate alloc;

#[cfg(not(feature = "host"))]
anyos_std::entry!(main);

/// Prelude: common types from alloc for no_std usage.
mod prelude {
    pub use alloc::boxed::Box;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
    pub use alloc::format;
}

mod toml;
mod manifest;
mod resolve;
mod build;
mod build_script;
mod workspace;
mod fingerprint;
mod jobs;
mod scaffold;
mod registry;
mod semver;
mod lockfile;
mod fs;

use prelude::*;
use anyos_std::println;

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let arg_tokens = anyos_std::args::tokenize(raw);
    let mut args: Vec<&str> = arg_tokens.iter().map(|arg| arg.as_str()).collect();

    if let Some(first) = args.first().copied() {
        if is_ccargo_argv0(first) {
            args.remove(0);
        }
    }

    if args.is_empty() {
        print_usage();
        return;
    }

    let command = args[0];

    // Parse global flags
    let mut release = false;
    let mut verbose = false;
    let mut is_lib = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut run_args: Vec<&str> = Vec::new();
    let mut after_dashdash = false;
    let mut features: Vec<String> = Vec::new();
    let mut target: Option<String> = None;
    let mut bin: Option<String> = None;
    let mut jobs: u32 = 1;

    let mut i = 1;
    while i < args.len() {
        if after_dashdash {
            run_args.push(args[i]);
        } else if args[i] == "--" {
            after_dashdash = true;
        } else if args[i] == "--release" || args[i] == "-r" {
            release = true;
        } else if args[i] == "--verbose" || args[i] == "-v" {
            verbose = true;
        } else if args[i] == "--lib" {
            is_lib = true;
        } else if args[i] == "--features" || args[i] == "-F" {
            i += 1;
            if i < args.len() {
                // Comma-separated features
                for feat in args[i].split(',') {
                    let feat = feat.trim();
                    if !feat.is_empty() {
                        features.push(String::from(feat));
                    }
                }
            }
        } else if args[i] == "--target" {
            i += 1;
            if i < args.len() {
                target = Some(String::from(args[i]));
            }
        } else if args[i] == "--bin" {
            i += 1;
            if i < args.len() {
                bin = Some(String::from(args[i]));
            } else {
                println!("ccargo: --bin expects a binary target name");
                return;
            }
        } else if args[i] == "--jobs" || args[i] == "-j" {
            i += 1;
            if i < args.len() {
                jobs = parse_u32_simple(args[i]);
            }
        } else if args[i] == "--all-features" {
            // Will be resolved against manifest features in build
            features.push(String::from("__all__"));
        } else if args[i] == "--no-default-features" {
            features.push(String::from("__no_default__"));
        } else if args[i] == "--help" || args[i] == "-h" {
            print_usage();
            return;
        } else if !args[i].starts_with('-') {
            positional.push(args[i]);
        } else {
            println!("ccargo: unknown option: {}", args[i]);
            return;
        }
        i += 1;
    }

    match command {
        "build" | "b" => {
            cmd_build(&positional, release, verbose, &features, target, bin);
        }
        "run" => {
            cmd_run(&positional, &run_args, release, verbose, &features, target, bin);
        }
        "new" => {
            cmd_new(&positional, is_lib);
        }
        "init" => {
            cmd_init(&positional, is_lib);
        }
        "clean" => {
            cmd_clean(&positional);
        }
        "check" | "c" => {
            cmd_check(&positional, release, &features, target, bin);
        }
        "test" | "t" => {
            cmd_test(&positional, release, &features, target, bin);
        }
        "bench" => {
            cmd_bench(&positional, &features, target);
        }
        "fetch" => {
            cmd_fetch(&positional);
        }
        "update" => {
            cmd_update(&positional);
        }
        "search" => {
            cmd_search(&positional);
        }
        "doc" => {
            println!("ccargo: doc generation not yet implemented");
        }
        "tree" => {
            cmd_tree(&positional);
        }
        "metadata" => {
            cmd_metadata(&positional);
        }
        "--version" | "-V" => {
            println!("ccargo 0.2.0 (anyOS)");
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        other => {
            println!("ccargo: unknown command `{}`", other);
            println!();
            print_usage();
        }
    }
}

fn is_ccargo_argv0(arg: &str) -> bool {
    let name = arg.rsplit('/').next().unwrap_or(arg);
    matches!(name, "ccargo" | "cargo" | "acargo")
}

fn make_config(release: bool, features: &[String], target: Option<String>, bin: Option<String>) -> build::BuildConfig {
    build::BuildConfig {
        release,
        verbose: true,
        jobs: 1,
        cfg_flags: Vec::new(),
        features: features.to_vec(),
        linker_script: None,
        link_args: Vec::new(),
        env_vars: Vec::new(),
        target,
        bin,
    }
}

fn cmd_build(positional: &[&str], release: bool, verbose: bool, features: &[String], target: Option<String>, bin: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let config = make_config(release, features, target, bin);
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }
}

fn cmd_run(positional: &[&str], run_args: &[&str], release: bool, verbose: bool, features: &[String], target: Option<String>, bin: Option<String>) {
    let dir = ".";
    let config = make_config(release, features, target, bin);
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }

    if let Some(ref bin_path) = result.bin_path {
        println!("     Running `{}`", bin_path);
        let mut cmd = String::from(bin_path.as_str());
        for arg in run_args {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        let status = anyos_std::process::exec(bin_path, &cmd);
        anyos_std::process::exit(status);
    } else {
        println!("ccargo: error: no binary target found");
        anyos_std::process::exit(1);
    }
}

fn cmd_new(positional: &[&str], is_lib: bool) {
    if positional.is_empty() {
        println!("ccargo: error: missing project name");
        println!("Usage: ccargo new <name> [--lib]");
        return;
    }
    scaffold::new_project(positional[0], is_lib);
}

fn cmd_init(positional: &[&str], is_lib: bool) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    scaffold::init_project(dir, is_lib);
}

fn cmd_clean(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let target_dir = format!("{}/target", dir);
    fs::rm_rf(&target_dir);
    println!("     Removed target directory");
}

fn cmd_check(positional: &[&str], release: bool, features: &[String], target: Option<String>, bin: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    // Check mode: compile but emit object files only (no linking)
    let mut config = make_config(release, features, target, bin);
    let result = build::build(dir, &config);
    if result.success {
        println!("    Finished checking {} crate(s)", result.compiled);
    } else {
        anyos_std::process::exit(1);
    }
}

fn cmd_test(positional: &[&str], release: bool, features: &[String], target: Option<String>, bin: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let mut feat = features.to_vec();
    // Automatically enable kunit feature for test builds if available
    if !feat.iter().any(|f| f == "kunit") {
        feat.push(String::from("kunit"));
    }
    let config = make_config(release, &feat, target, bin);
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }
    if let Some(ref bin_path) = result.bin_path {
        println!("     Running tests `{}`", bin_path);
        let status = anyos_std::process::exec(bin_path, bin_path);
        anyos_std::process::exit(status);
    } else {
        println!("ccargo: no test binary found");
    }
}

fn cmd_bench(positional: &[&str], features: &[String], target: Option<String>) {
    println!("ccargo: bench not yet implemented");
}

fn cmd_tree(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let nodes = resolve::resolve(dir, &[]);
    if nodes.is_empty() {
        println!("ccargo: no packages found");
        return;
    }
    let root = &nodes[nodes.len() - 1];
    println!("{} v{}", root.name, root.manifest.version);
    print_tree_deps(&nodes, nodes.len() - 1, "", true);
}

fn print_tree_deps(nodes: &[resolve::BuildNode], idx: usize, prefix: &str, last: bool) {
    let node = &nodes[idx];
    for (i, &dep_idx) in node.deps.iter().enumerate() {
        let is_last = i == node.deps.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let dep = &nodes[dep_idx];
        println!("{}{}{} v{}", prefix, connector, dep.name, dep.manifest.version);
        let new_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
        print_tree_deps(nodes, dep_idx, &new_prefix, is_last);
    }
}

fn cmd_fetch(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    println!("      Fetching dependencies for {}", dir);
    registry::init_cache();
    // Resolve triggers the fetch automatically
    let nodes = resolve::resolve(dir, &[]);
    let reg_count = nodes.iter().filter(|n| n.from_registry).count();
    if reg_count > 0 {
        println!("      Fetched {} registry crate(s)", reg_count);
    } else {
        println!("      No registry dependencies to fetch");
    }
}

fn cmd_update(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };

    if positional.len() > 1 && positional[0] == "--aggressive" {
        // Clear all index caches to force re-fetch
        registry::clean_cache();
        println!("      Cleaned registry cache, re-fetching...");
    } else {
        // Just invalidate index entries to get fresh versions
        let manifest_path = format!("{}/Cargo.toml", dir);
        if let Some(toml_src) = fs::read_file(&manifest_path) {
            let table = crate::toml::parse(&toml_src);
            let mf = crate::manifest::parse(&table);
            for dep in &mf.dependencies {
                if dep.path.is_none() {
                    registry::invalidate_index(&dep.name);
                }
            }
        }
    }

    // Delete existing lock file to force re-resolution
    let lock_path = format!("{}/Cargo.lock", dir);
    if fs::file_exists(&lock_path) {
        anyos_std::fs::unlink(&lock_path);
    }

    // Re-resolve
    registry::init_cache();
    let nodes = resolve::resolve(dir, &[]);
    let reg_count = nodes.iter().filter(|n| n.from_registry).count();
    println!("      Updated {} registry crate(s)", reg_count);
}

fn cmd_search(positional: &[&str]) {
    if positional.is_empty() {
        println!("Usage: ccargo search <query>");
        return;
    }
    let query = positional[0];
    println!("      Searching crates.io for `{}`...", query);

    // Use the crates.io API for search
    libhttp_client::init();
    let url = format!("https://crates.io/api/v1/crates?q={}&per_page=10", query);
    if let Some(data) = libhttp_client::get(&url) {
        if let Ok(text) = alloc::string::String::from_utf8(data) {
            // Simple extraction of name+description from API response
            // The response is JSON with a "crates" array
            print_search_results(&text);
        }
    } else {
        println!("ccargo: error: could not reach crates.io");
    }
}

fn print_search_results(json: &str) {
    // Very basic extraction: find "name":"..." and "description":"..." pairs
    // Full JSON parsing would be better but this works for display
    let mut pos = 0;
    let bytes = json.as_bytes();
    let mut count = 0;

    while pos < bytes.len() && count < 10 {
        // Find next "name":"
        if let Some(name_start) = json[pos..].find("\"name\":\"") {
            let abs_start = pos + name_start + 8;
            if let Some(name_end) = json[abs_start..].find('"') {
                let name = &json[abs_start..abs_start + name_end];

                // Find "max_version":"
                let search_from = abs_start + name_end;
                let version = if let Some(ver_start) = json[search_from..].find("\"max_version\":\"") {
                    let abs_ver = search_from + ver_start + 15;
                    if let Some(ver_end) = json[abs_ver..].find('"') {
                        &json[abs_ver..abs_ver + ver_end]
                    } else {
                        "?"
                    }
                } else {
                    "?"
                };

                // Find "description":"
                let desc = if let Some(desc_start) = json[search_from..].find("\"description\":\"") {
                    let abs_desc = search_from + desc_start + 15;
                    if let Some(desc_end) = json[abs_desc..].find('"') {
                        let d = &json[abs_desc..abs_desc + desc_end];
                        if d.len() > 60 { &d[..60] } else { d }
                    } else {
                        ""
                    }
                } else {
                    ""
                };

                // Skip internal fields
                if !name.contains("_") || name.len() > 2 {
                    println!("  {} = \"{}\"  # {}", name, version, desc);
                    count += 1;
                }

                pos = abs_start + name_end + 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    if count == 0 {
        println!("  No results found");
    }
}

fn cmd_metadata(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let manifest_path = format!("{}/Cargo.toml", dir);
    if let Some(toml_src) = fs::read_file(&manifest_path) {
        let toml_table = crate::toml::parse(&toml_src);
        let mf = crate::manifest::parse(&toml_table);
        println!("{{");
        println!("  \"name\": \"{}\",", mf.name);
        println!("  \"version\": \"{}\",", mf.version);
        println!("  \"edition\": \"{}\",", mf.edition);
        println!("  \"dependencies\": [");
        for (i, dep) in mf.dependencies.iter().enumerate() {
            let comma = if i + 1 < mf.dependencies.len() { "," } else { "" };
            if let Some(ref path) = dep.path {
                println!("    {{ \"name\": \"{}\", \"path\": \"{}\" }}{}", dep.name, path, comma);
            } else if let Some(ref ver) = dep.version {
                println!("    {{ \"name\": \"{}\", \"version\": \"{}\" }}{}", dep.name, ver, comma);
            } else {
                println!("    {{ \"name\": \"{}\" }}{}", dep.name, comma);
            }
        }
        println!("  ],");
        println!("  \"features\": {{");
        for (i, (feat_name, feat_deps)) in mf.features.iter().enumerate() {
            let comma = if i + 1 < mf.features.len() { "," } else { "" };
            let deps_str: Vec<&str> = feat_deps.iter().map(|s| s.as_str()).collect();
            println!("    \"{}\": [{}]{}", feat_name, deps_str.join(", "), comma);
        }
        println!("  }}");
        println!("}}");
    } else {
        println!("ccargo: cannot read {}", manifest_path);
    }
}

fn parse_u32_simple(s: &str) -> u32 {
    let mut n = 0u32;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            n = n * 10 + (b - b'0') as u32;
        }
    }
    n
}

fn print_usage() {
    println!("ccargo — Rust build system for anyOS");
    println!();
    println!("Usage: ccargo <COMMAND> [OPTIONS]");
    println!();
    println!("Commands:");
    println!("  build, b       Compile the current package");
    println!("  run            Build and run the binary");
    println!("  check, c       Check for errors without producing output");
    println!("  test, t        Build and run tests");
    println!("  bench          Build and run benchmarks");
    println!("  new <name>     Create a new package");
    println!("  init [dir]     Initialize package in existing directory");
    println!("  clean          Remove build artifacts");
    println!("  fetch          Download registry dependencies");
    println!("  update         Update registry dependencies to latest");
    println!("  search <query> Search crates.io");
    println!("  tree           Display dependency tree");
    println!("  metadata       Output package metadata as JSON");
    println!("  doc            Generate documentation");
    println!("  help           Print this message");
    println!();
    println!("Options:");
    println!("  --release, -r          Build with optimizations");
    println!("  --verbose, -v          Show detailed output");
    println!("  --features, -F <LIST>  Comma-separated features to enable");
    println!("  --all-features         Enable all features");
    println!("  --no-default-features  Disable default features");
    println!("  --target <SPEC>        Target specification");
    println!("  --bin <NAME>           Select binary target");
    println!("  --jobs, -j <N>         Number of parallel jobs");
    println!("  --lib                  Create a library project (with new/init)");
    println!("  --                     Pass remaining args to the binary (with run)");
}
