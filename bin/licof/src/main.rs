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
use package::{install_deb, install_package, package_installed, InstallProgress};
use rootfs::{
    ensure_rootfs_layout, find_linux_shell, linux_path_in_rootfs, path_exists, print_path_probe,
    repair_rootfs_runtime,
};

anyos_std::entry!(main);

macro_rules! log_ok {
    ($($arg:tt)*) => {
        println!("[OK]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        println!("[WARN]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        println!("[ERROR]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_fatal {
    ($($arg:tt)*) => {
        println!("[FATAL]\t{}", alloc::format!($($arg)*))
    };
}

fn main() {
    let config = LicoConfig::load();
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let mut verbose = false;
    let argv: Vec<&str> = raw
        .split_ascii_whitespace()
        .filter(|arg| {
            if *arg == "--verbose" || *arg == "-v" {
                verbose = true;
                false
            } else {
                true
            }
        })
        .collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(&config),
        Some("init") => init(&config, true, verbose),
        Some("repair") => repair(&config),
        Some("run") => run(&config, &argv[1..]),
        Some("shell") => shell(&config, &argv[1..]),
        Some("pkg") => pkg(&config, &argv[1..], verbose),
        Some("apt") => apt(&config, &argv[1..], verbose),
        Some(cmd) => {
            log_error!("licof: unknown command '{}'", cmd);
            usage();
        }
    }
}

fn usage() {
    println!("licof - Linux Compatibility Framework");
    println!();
    println!("Usage:");
    println!("  licof [--verbose] status");
    println!("  licof [--verbose] init");
    println!("  licof [--verbose] repair");
    println!("  licof [--verbose] run <linux-elf64> [args...]");
    println!("  licof [--verbose] shell [shell-args...]");
    println!("  licof [--verbose] pkg install <file.deb>");
    println!("  licof [--verbose] apt install <package> [package...]");
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
        log_error!("licof run: missing Linux ELF64 path");
        return;
    }

    let path = resolve_run_path(&config.rootfs, args[0]);
    let child_args = join_args(&args[1..]);
    run_linux_process(config, "licof run", &path, &child_args);
}

fn shell(config: &LicoConfig, args: &[&str]) {
    ensure_rootfs_layout(config);
    let Some(path) = find_linux_shell(&config.rootfs) else {
        log_error!("licof shell: no Linux shell found");
        log_warn!("licof shell: run 'licof init' or install bash/dash first");
        return;
    };
    let child_args = if args.is_empty() {
        String::from("-i")
    } else {
        join_args(args)
    };
    run_linux_process(config, "licof shell", &path, &child_args);
}

fn init(config: &LicoConfig, configure_password: bool, verbose: bool) {
    ensure_rootfs_layout(config);

    log_ok!("linux base ready at {}", config.rootfs);
    log_ok!("bootstrapping minimal Debian userland with apt");
    let bootstrapped = bootstrap_rootfs(config, &config.rootfs, verbose);
    fs::sync();
    if bootstrapped && configure_password {
        configure_root_password(config, &config.rootfs);
    } else if configure_password {
        log_warn!("licof init: bootstrap incomplete; skipping root password setup");
    }
}

fn repair(config: &LicoConfig) {
    ensure_rootfs_layout(config);
    repair_rootfs_runtime(&config.rootfs);
    fs::sync();
    log_ok!("licof repair: Linux base repaired at {}", config.rootfs);
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

fn pkg(config: &LicoConfig, args: &[&str], verbose: bool) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                ensure_rootfs_layout(config);
                let mut progress = InstallProgress::new(verbose, 1, "package");
                progress.set_overall(0, 1);
                if install_deb(config, path, &config.rootfs, None, &mut progress) {
                    progress.set_overall(1, 1);
                    progress.finish();
                    log_ok!("licof pkg: installed '{}'", path);
                } else {
                    progress.finish();
                }
            } else {
                log_error!("licof pkg install: missing .deb path");
            }
        }
        _ => log_error!("licof pkg: expected install <file.deb>"),
    }
}

fn apt(config: &LicoConfig, args: &[&str], verbose: bool) {
    match args.first().copied() {
        Some("install") => {
            let packages = &args[1..];
            if packages.is_empty() {
                log_error!("licof apt install: missing package name");
                return;
            }
            ensure_rootfs_layout(config);
            let mut progress = InstallProgress::new(verbose, packages.len() as u32, "packages");
            progress.set_overall(0, packages.len() as u32);
            let mut done = 0u32;
            for pkg in packages {
                if install_package(config, pkg, &config.rootfs, 0, &mut progress) {
                    done += 1;
                } else {
                    progress.finish();
                }
                progress.set_overall(done, packages.len() as u32);
            }
            progress.finish();
        }
        _ => log_error!("licof apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(config: &LicoConfig, rootfs: &str, verbose: bool) -> bool {
    let mut failed = Vec::new();
    let mut progress = InstallProgress::new(verbose, config.bootstrap_seed.len() as u32, "seeds");
    progress.set_overall(0, config.bootstrap_seed.len() as u32);
    write_bootstrap_state(config, rootfs, "running", &failed);

    let mut done = 0u32;
    for pkg in &config.bootstrap_seed {
        if package_installed(config, pkg, rootfs) {
            done += 1;
            progress.set_overall(done, config.bootstrap_seed.len() as u32);
            write_bootstrap_state(config, rootfs, "running", &failed);
            continue;
        }
        log_ok!("licof init: bootstrap installing missing seed '{}'", pkg);
        if install_package(config, pkg, rootfs, 0, &mut progress) {
            done += 1;
        } else {
            progress.finish();
            log_error!("licof init: bootstrap seed '{}' failed", pkg);
            failed.push(pkg.clone());
        }
        progress.set_overall(done, config.bootstrap_seed.len() as u32);
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
        progress.set_overall(
            config.bootstrap_seed.len() as u32,
            config.bootstrap_seed.len() as u32,
        );
        progress.finish();
        log_ok!("licof init: bootstrap complete");
    } else {
        progress.finish();
        log_error!("licof init: bootstrap incomplete; see diagnostics above");
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
        log_error!("licof init: bootstrap missing Linux shell (/bin/bash, /bin/dash or /bin/sh)");
        for linux_path in ["/bin/bash", "/usr/bin/bash", "/bin/dash", "/bin/sh"] {
            let path = linux_path_in_rootfs(rootfs, linux_path);
            print_path_probe("licof init", &path);
        }
        ok = false;
    }
    if find_passwd_binary(rootfs).is_none() {
        log_error!("licof init: bootstrap missing passwd binary");
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
        log_warn!("licof init: passwd binary not found; root password not configured");
        log_warn!("licof init: try later after 'licof apt install passwd'");
        return;
    };

    if fs::isatty(0) != 1 || fs::isatty(1) != 1 {
        log_warn!("licof init: root password setup needs an interactive terminal");
        log_warn!("licof init: run later: licof run {} root", passwd);
        return;
    }

    log_ok!("licof init: starting passwd for root");
    let code = run_linux_process(config, "licof passwd", &passwd, "root");
    if code == Some(0) {
        log_ok!("licof init: root password configured");
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
        log_fatal!("{}: failed to start '{}'", label, path);
        log_error!(
            "{}: check the diagnostics above and missing Linux syscalls in kernel/src/syscall/linux/",
            label
        );
        return None;
    }

    let code = process::waitpid(tid);
    if code == process::STILL_RUNNING {
        log_warn!("{}: process {} is still running after waitpid", label, tid);
        None
    } else if code == u32::MAX {
        log_error!("{}: wait failed for process {}", label, tid);
        None
    } else {
        if code != 0 {
            log_error!("{}: '{}' exited with status {}", label, path, code);
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
