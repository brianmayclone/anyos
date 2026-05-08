//! licof - Linux Compatibility Framework command line tool.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, println, process};

const ROOT: &str = "/System/var/licof";
const ROOTFS_DEFAULT: &str = "/System/var/licof/rootfs/default";
const CACHE: &str = "/System/var/licof/cache";
const DB: &str = "/System/var/licof/db";

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let argv: Vec<&str> = raw.split_ascii_whitespace().collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(),
        Some("run") => run(&argv[1..]),
        Some("rootfs") => rootfs(&argv[1..]),
        Some("pkg") => pkg(&argv[1..]),
        Some("apt") => apt(&argv[1..]),
        Some(cmd) => {
            println!("licof: unknown command '{}'", cmd);
            usage();
        }
    }
}

fn usage() {
    println!("licof - Linux Compatibility Framework");
    println!();
    println!("Usage:");
    println!("  licof status");
    println!("  licof run <linux-elf64> [args...]");
    println!("  licof rootfs create [name]");
    println!("  licof rootfs list");
    println!("  licof pkg install <file.deb>");
    println!("  licof apt install <package>");
}

fn status() {
    println!("licof status");
    println!("  abi: linux-x86_64 tier-0");
    println!("  root: {}", ROOT);
    println!("  default-rootfs: {}", ROOTFS_DEFAULT);
    println!("  supported-syscalls: read, write, open, openat, close, brk, mmap, munmap, getpid, exit");
}

fn run(args: &[&str]) {
    if args.is_empty() {
        println!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = args[0];
    let child_args = join_args(&args[1..]);
    let tid = process::licof_spawn(path, &child_args);
    if tid == u32::MAX {
        println!("licof run: failed to start '{}'", path);
        return;
    }

    let code = process::waitpid(tid);
    if code == process::STILL_RUNNING {
        println!("licof run: process {} is still running", tid);
    } else if code == u32::MAX {
        println!("licof run: wait failed for process {}", tid);
    }
}

fn rootfs(args: &[&str]) {
    match args.first().copied() {
        Some("create") => {
            let name = args.get(1).copied().unwrap_or("default");
            create_rootfs(name);
        }
        Some("list") => {
            println!("default  {}", ROOTFS_DEFAULT);
        }
        _ => {
            println!("licof rootfs: expected create or list");
        }
    }
}

fn create_rootfs(name: &str) {
    ensure_dir(ROOT);
    ensure_dir("/System/var/licof/rootfs");
    ensure_dir(CACHE);
    ensure_dir(DB);

    let rootfs = if name == "default" {
        String::from(ROOTFS_DEFAULT)
    } else {
        alloc::format!("/System/var/licof/rootfs/{}", name)
    };
    ensure_dir(&rootfs);
    ensure_dir(&alloc::format!("{}/bin", rootfs));
    ensure_dir(&alloc::format!("{}/lib", rootfs));
    ensure_dir(&alloc::format!("{}/lib64", rootfs));
    ensure_dir(&alloc::format!("{}/usr", rootfs));
    ensure_dir(&alloc::format!("{}/usr/bin", rootfs));
    ensure_dir(&alloc::format!("{}/etc", rootfs));
    println!("licof: rootfs '{}' ready at {}", name, rootfs);
}

fn pkg(args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                println!("licof pkg: Debian package ingestion queued for '{}'", path);
                println!("licof pkg: extractor/database implementation is next roadmap item");
            } else {
                println!("licof pkg install: missing .deb path");
            }
        }
        _ => println!("licof pkg: expected install <file.deb>"),
    }
}

fn apt(args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(pkg) = args.get(1) {
                println!("licof apt: resolver backend not ready yet for '{}'", pkg);
            } else {
                println!("licof apt install: missing package name");
            }
        }
        _ => println!("licof apt: expected install <package>"),
    }
}

fn ensure_dir(path: &str) {
    let mut stat_buf = [0u32; 7];
    if fs::stat(path, &mut stat_buf) == 0 {
        return;
    }
    let _ = fs::mkdir(path);
}

fn join_args(args: &[&str]) -> String {
    let mut out = String::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx != 0 {
            out.push(' ');
        }
        out.push_str(arg);
    }
    out
}
