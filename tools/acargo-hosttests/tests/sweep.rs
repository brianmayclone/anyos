use acargo::build::{self, BuildConfig};
use std::panic::{self, AssertUnwindSafe};
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf()
}

fn make_config(target: Option<&str>) -> BuildConfig {
    BuildConfig {
        release: false,
        verbose: false,
        jobs: 1,
        cfg_flags: Vec::new(),
        features: Vec::new(),
        linker_script: None,
        link_args: Vec::new(),
        env_vars: Vec::new(),
        target: target.map(str::to_string),
    }
}

fn bin_package_dirs() -> Vec<PathBuf> {
    package_dirs(&repo_root().join("bin"))
}

fn app_package_dirs() -> Vec<PathBuf> {
    package_dirs(&repo_root().join("apps"))
}

fn system_package_dirs() -> Vec<PathBuf> {
    let root = repo_root();
    let mut dirs = package_dirs(&root.join("system"));
    dirs.extend(package_dirs(&root.join("system/compositor")));
    dirs.sort();
    dirs
}

fn gpu_driver_package_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(repo_root().join("drivers/gpu/svga3d"));
    dirs.push(repo_root().join("drivers/gpu/virgl"));
    dirs
}

fn package_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(dir).expect("read package dir") {
        let entry = entry.expect("bin dir entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("Cargo.toml").is_file() {
            dirs.push(path);
        }
    }
    dirs.sort();
    dirs
}

fn run_sweep(label: &str, dirs: Vec<PathBuf>, target: Option<&str>) {
    let mut failures = Vec::new();
    for dir in dirs {
        if let Err(err) = build_ok(&dir, target) {
            failures.push(err);
        }
    }
    if !failures.is_empty() {
        panic!("ccargo {} sweep failures:\n{}", label, failures.join("\n"));
    }
}

fn build_ok(dir: &Path, target: Option<&str>) -> Result<(), String> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        build::build(dir.to_str().expect("utf8 path"), &make_config(target))
    }));
    let result = match result {
        Ok(result) => result,
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                String::from("unknown panic")
            };
            return Err(format!("panic while building {}: {}", dir.display(), msg));
        }
    };
    if result.success {
        Ok(())
    } else {
        Err(format!("build failed for {}", dir.display()))
    }
}

fn build_repo_lib(rel: &str, target: Option<&str>) -> Result<(), String> {
    build_ok(&repo_root().join(rel), target)
}

#[test]
fn compile_all_bin_packages_with_ccargo() {
    run_sweep("bin", bin_package_dirs(), None);
}

#[test]
fn compile_all_app_packages_with_ccargo() {
    run_sweep("app", app_package_dirs(), None);
}

#[test]
fn compile_all_userspace_system_packages_with_ccargo() {
    let kernel_target_dirs = [
        repo_root().join("system/fontd"),
        repo_root().join("system/sessionhost"),
        repo_root().join("system/desktopd"),
        repo_root().join("system/crashdialog"),
        repo_root().join("system/compositor/dock"),
    ];

    let dirs = system_package_dirs()
        .into_iter()
        .filter(|dir| !kernel_target_dirs.contains(dir))
        .collect();
    run_sweep("userspace system", dirs, None);
}

#[test]
fn compile_all_kernel_target_system_packages_with_ccargo() {
    run_sweep(
        "kernel-target system",
        vec![
            repo_root().join("system/fontd"),
            repo_root().join("system/sessionhost"),
            repo_root().join("system/desktopd"),
            repo_root().join("system/crashdialog"),
            repo_root().join("system/compositor/dock"),
        ],
        Some("x86_64-anyos"),
    );
}

#[test]
fn compile_all_gpu_drivers_with_ccargo() {
    run_sweep("gpu driver", gpu_driver_package_dirs(), None);
}

#[test]
fn compile_kernel_with_ccargo() {
    let kernel_dir = repo_root().join("kernel");
    if let Err(err) = build_ok(&kernel_dir, Some("x86_64-anyos")) {
        panic!("{err}");
    }
}

#[test]
fn compile_anyos_std_with_ccargo() {
    if let Err(err) = build_repo_lib("libs/stdlib", None) {
        panic!("{err}");
    }
}

#[test]
fn compile_libstd_with_ccargo() {
    if let Err(err) = build_repo_lib("libs/libstd", None) {
        panic!("{err}");
    }
}
