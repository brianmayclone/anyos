//! licof - Linux Compatibility Framework command line tool.

#![no_std]
#![no_main]

use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, println, process};

mod config;
mod elf;
mod model;
mod package;
mod rootfs;

use config::LicoConfig;
use package::{install_deb, install_package, package_installed};
use rootfs::{
    ensure_rootfs_layout, find_linux_shell, linux_path_in_rootfs, path_exists, print_path_probe,
    repair_rootfs_runtime,
};

anyos_std::entry!(main);

fn main() {
    let config = LicoConfig::load();
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let argv: Vec<&str> = raw.split_ascii_whitespace().collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(&config),
        Some("init") => init(&config, true),
        Some("repair") => repair(&config),
        Some("run") => run(&config, &argv[1..]),
        Some("shell") => shell(&config, &argv[1..]),
        Some("pkg") => pkg(&config, &argv[1..]),
        Some("apt") => apt(&config, &argv[1..]),
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
    println!("  licof init");
    println!("  licof repair");
    println!("  licof run <linux-elf64> [args...]");
    println!("  licof shell [shell-args...]");
    println!("  licof pkg install <file.deb>");
    println!("  licof apt install <package> [package...]");
}

fn status(config: &LicoConfig) {
    println!("licof status");
    println!("  abi: linux-x86_64 tier-0");
    println!("  root: {}", config.root);
    println!("  linux-base: {}", config.rootfs);
    println!(
        "  apt-source: {}/dists/{}/{}/binary-{}/Packages.gz",
        config.apt_base, config.apt_dist, config.apt_component, config.apt_arch
    );
    println!("  config: confd system/services/licof");
    println!("  supported-package-data: data.tar.gz, data.tar.xz");
}

fn run(config: &LicoConfig, args: &[&str]) {
    if args.is_empty() {
        println!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = resolve_run_path(&config.rootfs, args[0]);
    let child_args = join_args(&args[1..]);
    run_linux_process(config, "licof run", &path, &child_args);
}

fn shell(config: &LicoConfig, args: &[&str]) {
    ensure_rootfs_layout(config);
    let Some(path) = find_linux_shell(&config.rootfs) else {
        println!("licof shell: no Linux shell found");
        println!("licof shell: run 'licof init' or install bash/dash first");
        return;
    };
    let child_args = if args.is_empty() {
        String::from("-i")
    } else {
        join_args(args)
    };
    run_linux_process(config, "licof shell", &path, &child_args);
}

fn init(config: &LicoConfig, configure_password: bool) {
    ensure_rootfs_layout(config);

    println!("licof: Linux base ready at {}", config.rootfs);
    println!("licof: bootstrapping minimal Debian userland with apt");
    let bootstrapped = bootstrap_rootfs(config, &config.rootfs);
    fs::sync();
    if bootstrapped && configure_password {
        configure_root_password(config, &config.rootfs);
    } else if configure_password {
        println!("licof init: bootstrap incomplete; skipping root password setup");
    }
}

fn repair(config: &LicoConfig) {
    ensure_rootfs_layout(config);
    repair_rootfs_runtime(&config.rootfs);
    fs::sync();
    println!("licof repair: Linux base repaired at {}", config.rootfs);
}

fn resolve_run_path(rootfs: &str, path: &str) -> String {
    if path.starts_with("/System/")
        || path.starts_with("/Applications/")
        || path.starts_with("/Users/")
    {
        String::from(path)
    } else if path.starts_with('/') {
        linux_path_in_rootfs(rootfs, path)
    } else {
        alloc::format!("{}/{}", rootfs, path)
    }
}

fn pkg(config: &LicoConfig, args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                ensure_rootfs_layout(config);
                if install_deb(config, path, &config.rootfs, None) {
                    println!("licof pkg: installed '{}'", path);
                }
            } else {
                println!("licof pkg install: missing .deb path");
            }
        }
        _ => println!("licof pkg: expected install <file.deb>"),
    }
}

fn apt(config: &LicoConfig, args: &[&str]) {
    match args.first().copied() {
        Some("install") => {
            let packages = &args[1..];
            if packages.is_empty() {
                println!("licof apt install: missing package name");
                return;
            }
            ensure_rootfs_layout(config);
            for pkg in packages {
                install_package(config, pkg, &config.rootfs, 0);
            }
        }
        _ => println!("licof apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(config: &LicoConfig, rootfs: &str) -> bool {
    let mut failed = Vec::new();
    write_bootstrap_state(config, rootfs, "running", &failed);

    for pkg in &config.bootstrap_seed {
        if package_installed(config, pkg, rootfs) {
            write_bootstrap_state(config, rootfs, "running", &failed);
            continue;
        }
        println!("licof init: bootstrap installing missing seed '{}'", pkg);
        if !install_package(config, pkg, rootfs, 0) {
            println!("licof init: bootstrap seed '{}' failed", pkg);
            failed.push(pkg.clone());
        }
        write_bootstrap_state(config, rootfs, "running", &failed);
    }

    let mut ok = bootstrap_packages_complete(config, rootfs);
    if ok && !verify_bootstrap_integrity(rootfs) {
        ok = false;
    }
    write_bootstrap_state(
        config,
        rootfs,
        if ok { "complete" } else { "incomplete" },
        &failed,
    );
    if ok {
        println!("licof init: bootstrap complete");
    } else {
        println!("licof init: bootstrap incomplete; see diagnostics above");
    }
    ok
}

fn bootstrap_packages_complete(config: &LicoConfig, rootfs: &str) -> bool {
    for pkg in &config.bootstrap_seed {
        if !package_installed(config, pkg, rootfs) {
            return false;
        }
    }
    true
}

fn write_bootstrap_state(
    config: &LicoConfig,
    rootfs: &str,
    status: &str,
    failed_this_run: &[String],
) {
    let mut body = String::new();
    body.push_str("Status: ");
    body.push_str(status);
    body.push('\n');
    body.push_str("RootFS: ");
    body.push_str(rootfs);
    body.push('\n');

    for pkg in &config.bootstrap_seed {
        if package_installed(config, pkg, rootfs) {
            body.push_str("Installed: ");
        } else {
            body.push_str("Missing: ");
        }
        body.push_str(pkg);
        body.push('\n');
    }

    for pkg in failed_this_run {
        if !package_installed(config, pkg, rootfs) {
            body.push_str("Failed: ");
            body.push_str(pkg);
            body.push('\n');
        }
    }

    let state_path = alloc::format!("{}/bootstrap-state", config.db);
    let _ = fs::write_bytes(&state_path, body.as_bytes());
    fs::sync();
}

fn verify_bootstrap_integrity(rootfs: &str) -> bool {
    let mut ok = true;
    if find_linux_shell(rootfs).is_none() {
        println!("licof init: bootstrap missing Linux shell (/bin/bash, /bin/dash or /bin/sh)");
        for linux_path in ["/bin/bash", "/usr/bin/bash", "/bin/dash", "/bin/sh"] {
            let path = linux_path_in_rootfs(rootfs, linux_path);
            print_path_probe("licof init", &path);
        }
        ok = false;
    }
    if find_passwd_binary(rootfs).is_none() {
        println!("licof init: bootstrap missing passwd binary");
        for linux_path in ["/usr/bin/passwd", "/bin/passwd"] {
            let path = linux_path_in_rootfs(rootfs, linux_path);
            print_path_probe("licof init", &path);
        }
        ok = false;
    }
    ok
}

fn configure_root_password(config: &LicoConfig, rootfs: &str) {
    let Some(passwd) = find_passwd_binary(rootfs) else {
        println!("licof init: passwd binary not found; root password not configured");
        println!("licof init: try later after 'licof apt install passwd'");
        return;
    };

    if fs::isatty(0) != 1 || fs::isatty(1) != 1 {
        println!("licof init: root password setup needs an interactive terminal");
        println!("licof init: run later: licof run {} root", passwd);
        return;
    }

    println!("licof init: starting passwd for root");
    let code = run_linux_process(config, "licof passwd", &passwd, "root");
    if code == Some(0) {
        println!("licof init: root password configured");
    }
}

fn find_passwd_binary(rootfs: &str) -> Option<String> {
    for linux_path in ["/usr/bin/passwd", "/bin/passwd"] {
        let path = linux_path_in_rootfs(rootfs, linux_path);
        if path_exists(&path) {
            return Some(path);
        }
    }
    None
}

fn run_linux_process(config: &LicoConfig, label: &str, path: &str, args: &str) -> Option<u32> {
    elf::diagnose_linux_binary(config, label, path);
    let tid = process::licof_spawn(path, args);
    if tid == u32::MAX {
        println!("{}: failed to start '{}'", label, path);
        println!("{}: check the diagnostics above and missing Linux syscalls in kernel/src/syscall/linux/", label);
        return None;
    }

    let code = process::waitpid(tid);
    if code == process::STILL_RUNNING {
        println!("{}: process {} is still running after waitpid", label, tid);
        None
    } else if code == u32::MAX {
        println!("{}: wait failed for process {}", label, tid);
        None
    } else {
        if code != 0 {
            println!("{}: '{}' exited with status {}", label, path, code);
        }
        Some(code)
    }
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
