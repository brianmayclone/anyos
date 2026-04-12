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
    let bins_dir = repo_root().join("bin");
    let mut dirs = Vec::new();
    for entry in fs::read_dir(&bins_dir).expect("read bin dir") {
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
    let mut failures = Vec::new();
    for dir in bin_package_dirs() {
        if let Err(err) = build_ok(&dir, None) {
            failures.push(err);
        }
    }
    if !failures.is_empty() {
        panic!("ccargo bin sweep failures:\n{}", failures.join("\n"));
    }
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
