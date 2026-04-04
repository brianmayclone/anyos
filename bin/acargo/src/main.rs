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
            cmd_build(&positional, release, verbose);
        }
        "run" => {
            cmd_run(&positional, &run_args, release, verbose);
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
            println!("acargo: check not yet implemented, running build");
            cmd_build(&positional, release, verbose);
        }
        "--version" | "-V" => {
            println!("acargo 0.1.0 (anyOS)");
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

fn cmd_build(positional: &[&str], release: bool, verbose: bool) {
    let dir = if positional.is_empty() { "." } else { positional[0] };
    let config = build::BuildConfig {
        release,
        verbose: true, // always show progress
        jobs: 1,
    };
    let result = build::build(dir, &config);
    if !result.success {
        anyos_std::process::exit(1);
    }
}

fn cmd_run(positional: &[&str], run_args: &[&str], release: bool, verbose: bool) {
    let dir = ".";
    let config = build::BuildConfig {
        release,
        verbose: true,
        jobs: 1,
    };
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

fn print_usage() {
    println!("acargo — Rust build system for anyOS");
    println!();
    println!("Usage: acargo <COMMAND> [OPTIONS]");
    println!();
    println!("Commands:");
    println!("  build, b       Compile the current package");
    println!("  run            Build and run the binary");
    println!("  new <name>     Create a new package");
    println!("  init [dir]     Initialize package in existing directory");
    println!("  clean          Remove build artifacts");
    println!("  check, c       Check for errors without producing output");
    println!("  help           Print this message");
    println!();
    println!("Options:");
    println!("  --release, -r  Build with optimizations");
    println!("  --verbose, -v  Show detailed output");
    println!("  --lib          Create a library project (with new/init)");
    println!("  --             Pass remaining args to the binary (with run)");
}
