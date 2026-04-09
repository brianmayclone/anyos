#![no_std]
#![no_main]

//! acargo — Rust build system for anyOS
//!
//! Compatible subset of Cargo functionality for building Rust projects
//! natively on anyOS using the anyrc compiler.

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
mod fs;

use prelude::*;
use anyos_std::println;

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args: Vec<&str> = raw.split_whitespace().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    let command = args[1];

    // Parse global flags
    let mut release = false;
    let mut verbose = false;
    let mut is_lib = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut run_args: Vec<&str> = Vec::new();
    let mut after_dashdash = false;
    let mut features: Vec<String> = Vec::new();
    let mut target: Option<String> = None;
    let mut jobs: u32 = 1;

    let mut i = 2;
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
            println!("acargo: unknown option: {}", args[i]);
            return;
        }
        i += 1;
    }

    match command {
        "build" | "b" => {
            cmd_build(&positional, release, verbose, &features, target);
        }
        "run" => {
            cmd_run(&positional, &run_args, release, verbose, &features, target);
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
            cmd_check(&positional, release, &features, target);
        }
        "test" | "t" => {
            cmd_test(&positional, release, &features, target);
        }
        "bench" => {
            cmd_bench(&positional, &features, target);
        }
        "doc" => {
            println!("acargo: doc generation not yet implemented");
        }
        "tree" => {
            cmd_tree(&positional);
        }
        "metadata" => {
            cmd_metadata(&positional);
        }
        "--version" | "-V" => {
            println!("acargo 0.2.0 (anyOS)");
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        other => {
            println!("acargo: unknown command `{}`", other);
            println!();
            print_usage();
        }
    }
}

fn make_config(release: bool, features: &[String], target: Option<String>) -> build::BuildConfig {
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
    }
}

fn cmd_build(positional: &[&str], release: bool, verbose: bool, features: &[String], target: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let config = make_config(release, features, target);
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }
}

fn cmd_run(positional: &[&str], run_args: &[&str], release: bool, verbose: bool, features: &[String], target: Option<String>) {
    let dir = ".";
    let config = make_config(release, features, target);
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
        println!("acargo: error: no binary target found");
        anyos_std::process::exit(1);
    }
}

fn cmd_new(positional: &[&str], is_lib: bool) {
    if positional.is_empty() {
        println!("acargo: error: missing project name");
        println!("Usage: acargo new <name> [--lib]");
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

fn cmd_check(positional: &[&str], release: bool, features: &[String], target: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    // Check mode: compile but emit object files only (no linking)
    let mut config = make_config(release, features, target);
    let result = build::build(dir, &config);
    if result.success {
        println!("    Finished checking {} crate(s)", result.compiled);
    } else {
        anyos_std::process::exit(1);
    }
}

fn cmd_test(positional: &[&str], release: bool, features: &[String], target: Option<String>) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let mut feat = features.to_vec();
    // Automatically enable kunit feature for test builds if available
    if !feat.iter().any(|f| f == "kunit") {
        feat.push(String::from("kunit"));
    }
    let config = make_config(release, &feat, target);
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }
    if let Some(ref bin_path) = result.bin_path {
        println!("     Running tests `{}`", bin_path);
        let status = anyos_std::process::exec(bin_path, bin_path);
        anyos_std::process::exit(status);
    } else {
        println!("acargo: no test binary found");
    }
}

fn cmd_bench(positional: &[&str], features: &[String], target: Option<String>) {
    println!("acargo: bench not yet implemented");
}

fn cmd_tree(positional: &[&str]) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let nodes = resolve::resolve(dir);
    if nodes.is_empty() {
        println!("acargo: no packages found");
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
        println!("acargo: cannot read {}", manifest_path);
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
    println!("acargo — Rust build system for anyOS");
    println!();
    println!("Usage: acargo <COMMAND> [OPTIONS]");
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
    println!("  --jobs, -j <N>         Number of parallel jobs");
    println!("  --lib                  Create a library project (with new/init)");
    println!("  --                     Pass remaining args to the binary (with run)");
}
