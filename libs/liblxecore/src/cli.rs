use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, println, process};

use crate::config::LxeConfig;
use crate::daemon;
use crate::package::{
    install_deb, install_package, install_resolved_plan, package_installed, prefetch_install_plan,
    resolve_install_plan, sync_dpkg_status, InstallProgress,
};
use crate::readiness::{shell_availability, LxeShellAvailability};
use crate::rootfs::{
    ensure_rootfs_layout, find_linux_bash, find_linux_shell, linux_path_in_rootfs, path_exists,
    print_path_probe, repair_rootfs_runtime, write_bytes_atomic,
};

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

pub fn run_cli(raw: &str) {
    let mut config = LxeConfig::load();
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
        Some("init") => init(&mut config, true, verbose),
        Some("repair") => repair(&mut config),
        Some("run") => run(&mut config, &argv[1..]),
        Some("shell") => shell(&mut config, &argv[1..]),
        Some("pkg") => pkg(&mut config, &argv[1..], verbose),
        Some("apt") => apt(&mut config, &argv[1..], verbose),
        Some(cmd) => {
            log_error!("lxe: unknown command '{}'", cmd);
            usage();
        }
    }
}

fn usage() {
    println!("lxe - Linux Experience Extension");
    println!();
    println!("Usage:");
    println!("  lxe [--verbose] status");
    println!("  lxe [--verbose] init");
    println!("  lxe [--verbose] repair");
    println!("  lxe [--verbose] run <linux-elf64> [args...]");
    println!("  lxe [--verbose] shell [bash-args...]");
    println!("  lxe [--verbose] pkg install <file.deb>");
    println!("  lxe [--verbose] apt install <package> [package...]");
}

fn status(config: &LxeConfig) {
    println!("lxe status");
    println!("  abi: linux-x86_64 tier-0");
    println!("  root: {}", config.root);
    println!("  linux-base: {}", config.rootfs);
    println!(
        "  apt-source: {}/dists/{}/{}/binary-{}/Packages.gz",
        config.apt_base, config.apt_dist, config.apt_component, config.apt_arch
    );
    println!("  config: confd system/services/lxe");
    println!("  supported-package-data: data.tar.gz, data.tar.xz");
    match daemon::status() {
        Ok(status) => {
            println!("  lxed: ready");
            println!("  lxed-root: {}", status.root);
            println!("  lxed-rootfs: {}", status.rootfs);
            println!("  lxed-active-runs: {}", status.active_runs);
            println!("  lxed-active-processes: {}", status.active_processes);
            println!("  lxed-writer: {}", status.writer);
            println!("  lxed-syncs: {}", status.syncs);
        }
        Err(err) => println!("  lxed: unavailable ({})", err),
    }
}

fn run(config: &mut LxeConfig, args: &[&str]) {
    if args.is_empty() {
        log_error!("lxe run: missing Linux ELF64 path");
        return;
    }

    let lease = match daemon::acquire_run(config, "run") {
        Ok(lease) => lease,
        Err(err) => {
            log_fatal!("lxe run: lxed unavailable: {}", err);
            return;
        }
    };
    let path = resolve_run_path(&config.rootfs, args[0]);
    let child_args = join_args(&args[1..]);
    run_linux_process(config, "lxe run", &path, &child_args);
    lease.release();
}

fn shell(config: &mut LxeConfig, args: &[&str]) {
    let pty_bridge = args.first().copied() == Some("--pty-bridge");
    if !pty_bridge {
        open_shell_app(config);
        return;
    }

    let args = &args[1..];
    let lease = match daemon::acquire_run(config, "shell") {
        Ok(lease) => lease,
        Err(err) => {
            log_fatal!("lxe shell: lxed unavailable: {}", err);
            return;
        }
    };
    ensure_rootfs_layout(config);
    sync_dpkg_status(config, &config.rootfs);
    let Some(path) = find_linux_bash(&config.rootfs) else {
        log_error!("lxe shell: bash not found");
        log_warn!("lxe shell: run 'lxe init' or install bash first");
        lease.release();
        return;
    };
    let child_args = if args.is_empty() {
        String::from("-i")
    } else {
        join_args(args)
    };
    run_linux_process(config, "lxe shell", &path, &child_args);
    lease.release();
}

fn open_shell_app(config: &LxeConfig) {
    match shell_availability(config) {
        LxeShellAvailability::Ready { .. } => {}
        LxeShellAvailability::MissingRootfs { rootfs } => {
            log_error!("lxe shell: LXE is not installed at {}", rootfs);
            log_warn!("lxe shell: run 'lxe init' first");
            return;
        }
        LxeShellAvailability::MissingBash { rootfs } => {
            log_error!("lxe shell: bash is not installed in {}", rootfs);
            log_warn!("lxe shell: run 'lxe apt install bash' or re-run 'lxe init'");
            return;
        }
    }

    let tid = process::launch_app("/Applications/Management/LXE Shell.app/LXE Shell", "");
    if tid == u32::MAX {
        log_fatal!("lxe shell: failed to open LXE Shell.app");
        log_warn!("lxe shell: internal fallback for diagnostics: lxe shell --pty-bridge");
    }
}

fn init(config: &mut LxeConfig, configure_password: bool, verbose: bool) {
    let lease = match daemon::acquire_write(config, "init") {
        Ok(lease) => lease,
        Err(err) => {
            log_fatal!("lxe init: lxed unavailable: {}", err);
            return;
        }
    };
    ensure_rootfs_layout(config);

    log_ok!("linux base ready at {}", config.rootfs);
    log_ok!("bootstrapping minimal Debian userland with apt");
    let bootstrapped = bootstrap_rootfs(config, &config.rootfs, verbose);
    fs::sync();
    if bootstrapped && configure_password {
        configure_root_password(config, &config.rootfs);
    } else if configure_password {
        log_warn!("lxe init: bootstrap incomplete; skipping root password setup");
    }
    lease.release();
}

fn repair(config: &mut LxeConfig) {
    let lease = match daemon::acquire_write(config, "repair") {
        Ok(lease) => lease,
        Err(err) => {
            log_fatal!("lxe repair: lxed unavailable: {}", err);
            return;
        }
    };
    ensure_rootfs_layout(config);
    repair_rootfs_runtime(&config.rootfs);
    fs::sync();
    log_ok!("lxe repair: Linux base repaired at {}", config.rootfs);
    lease.release();
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

fn pkg(config: &mut LxeConfig, args: &[&str], verbose: bool) {
    match args.first().copied() {
        Some("install") => {
            if let Some(path) = args.get(1) {
                let lease = match daemon::acquire_write(config, "pkg") {
                    Ok(lease) => lease,
                    Err(err) => {
                        log_fatal!("lxe pkg: lxed unavailable: {}", err);
                        return;
                    }
                };
                ensure_rootfs_layout(config);
                let mut progress = InstallProgress::new(verbose, 1, "package");
                progress.set_overall(0, 1);
                if install_deb(config, path, &config.rootfs, None, &mut progress) {
                    progress.set_overall(1, 1);
                    progress.finish();
                    log_ok!("lxe pkg: installed '{}'", path);
                } else {
                    progress.finish();
                }
                lease.release();
            } else {
                log_error!("lxe pkg install: missing .deb path");
            }
        }
        _ => log_error!("lxe pkg: expected install <file.deb>"),
    }
}

fn apt(config: &mut LxeConfig, args: &[&str], verbose: bool) {
    match args.first().copied() {
        Some("install") => {
            let packages = &args[1..];
            if packages.is_empty() {
                log_error!("lxe apt install: missing package name");
                return;
            }
            let lease = match daemon::acquire_write(config, "apt") {
                Ok(lease) => lease,
                Err(err) => {
                    log_fatal!("lxe apt: lxed unavailable: {}", err);
                    return;
                }
            };
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
            sync_dpkg_status(config, &config.rootfs);
            lease.release();
        }
        _ => log_error!("lxe apt: expected install <package>"),
    }
}

fn bootstrap_rootfs(config: &LxeConfig, rootfs: &str, verbose: bool) -> bool {
    let mut failed = Vec::new();
    let mut progress = InstallProgress::new(verbose, config.bootstrap_seed.len() as u32, "seeds");
    progress.set_overall(0, config.bootstrap_seed.len() as u32);
    write_bootstrap_state(config, rootfs, "running", &failed);

    let mut done = 0u32;
    let mut plan_installed = false;
    let plan_packages = config.bootstrap_seed.clone();
    if let Some(plan) = resolve_install_plan(config, &plan_packages, rootfs, &mut progress) {
        if !plan.is_empty() {
            log_ok!(
                "lxe init: downloading {} package archives before install",
                plan.len()
            );
        }
        if !prefetch_install_plan(config, &plan, &mut progress) {
            log_warn!(
                "lxe init: parallel prefetch incomplete; continuing with on-demand downloads"
            );
        }
        if install_resolved_plan(config, &plan, rootfs, &mut progress) {
            plan_installed = true;
            done = count_installed_bootstrap_packages(config, rootfs);
            progress.set_overall(done, config.bootstrap_seed.len() as u32);
            write_bootstrap_state(config, rootfs, "running", &failed);
        } else {
            log_warn!("lxe init: planned install incomplete; falling back to recursive installer");
        }
    } else {
        log_warn!("lxe init: could not build full download plan; falling back to serial resolver");
    }

    if !plan_installed {
        for pkg in &config.bootstrap_seed {
            if package_installed(config, pkg, rootfs) {
                done += 1;
                progress.set_overall(done, config.bootstrap_seed.len() as u32);
                write_bootstrap_state(config, rootfs, "running", &failed);
                continue;
            }
            log_ok!("lxe init: bootstrap installing missing seed '{}'", pkg);
            if install_package(config, pkg, rootfs, 0, &mut progress) {
                done += 1;
            } else {
                progress.finish();
                log_error!("lxe init: bootstrap seed '{}' failed", pkg);
                failed.push(pkg.clone());
            }
            progress.set_overall(done, config.bootstrap_seed.len() as u32);
            write_bootstrap_state(config, rootfs, "running", &failed);
        }
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
        log_ok!("lxe init: bootstrap complete");
    } else {
        progress.finish();
        log_error!("lxe init: bootstrap incomplete; see diagnostics above");
    }
    ok
}

fn bootstrap_packages_complete(config: &LxeConfig, rootfs: &str) -> bool {
    for pkg in &config.bootstrap_seed {
        if !package_installed(config, pkg, rootfs) {
            return false;
        }
    }
    true
}

fn count_installed_bootstrap_packages(config: &LxeConfig, rootfs: &str) -> u32 {
    let mut installed = 0u32;
    for pkg in &config.bootstrap_seed {
        if package_installed(config, pkg, rootfs) {
            installed += 1;
        }
    }
    installed
}

fn write_bootstrap_state(
    config: &LxeConfig,
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
    let _ = write_bytes_atomic(&state_path, body.as_bytes());
    fs::sync();
}

fn verify_bootstrap_integrity(rootfs: &str) -> bool {
    let mut ok = true;
    if find_linux_shell(rootfs).is_none() {
        log_error!("lxe init: bootstrap missing Linux shell (/bin/bash, /bin/dash or /bin/sh)");
        for linux_path in ["/bin/bash", "/usr/bin/bash", "/bin/dash", "/bin/sh"] {
            let path = linux_path_in_rootfs(rootfs, linux_path);
            print_path_probe("lxe init", &path);
        }
        ok = false;
    }
    if find_passwd_binary(rootfs).is_none() {
        log_error!("lxe init: bootstrap missing passwd binary");
        for linux_path in ["/usr/bin/passwd", "/bin/passwd"] {
            let path = linux_path_in_rootfs(rootfs, linux_path);
            print_path_probe("lxe init", &path);
        }
        ok = false;
    }
    ok
}

fn configure_root_password(config: &LxeConfig, rootfs: &str) {
    let Some(passwd) = find_passwd_binary(rootfs) else {
        log_warn!("lxe init: passwd binary not found; root password not configured");
        log_warn!("lxe init: try later after 'lxe apt install passwd'");
        return;
    };

    if fs::isatty(0) != 1 || fs::isatty(1) != 1 {
        log_warn!("lxe init: root password setup needs an interactive terminal");
        log_warn!("lxe init: run later: lxe run {} root", passwd);
        return;
    }

    log_ok!("lxe init: starting passwd for root");
    let code = run_linux_process(config, "lxe passwd", &passwd, "root");
    if code == Some(0) {
        log_ok!("lxe init: root password configured");
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

fn run_linux_process(config: &LxeConfig, label: &str, path: &str, args: &str) -> Option<u32> {
    crate::elf::diagnose_linux_binary(config, label, path);
    let tid = process::lxe_spawn(path, args);
    if tid == u32::MAX {
        log_fatal!("{}: failed to start '{}'", label, path);
        log_error!(
            "{}: check the diagnostics above and missing Linux syscalls in kernel/src/syscall/linux/",
            label
        );
        return None;
    }

    if let Err(err) = daemon::track_process(tid) {
        log_warn!(
            "{}: lxed process tracking failed for {}: {}",
            label,
            tid,
            err
        );
    }
    let code = process::waitpid(tid);
    daemon::untrack_process(tid);
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
