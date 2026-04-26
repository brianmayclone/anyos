use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

static CCARGO_LOCK: Mutex<()> = Mutex::new(());
static HOST_CCARGO: OnceLock<PathBuf> = OnceLock::new();

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("anyrc_tests should live below libs/")
        .to_path_buf()
}

fn host_ccargo(root: &Path) -> &'static PathBuf {
    HOST_CCARGO.get_or_init(|| build_host_ccargo(root))
}

fn build_host_ccargo(root: &Path) -> PathBuf {
    let output = Command::new("cargo")
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .args([
            "build",
            "--manifest-path",
            "bin/acargo/Cargo.toml",
            "--features",
            "host",
            "--bin",
            "ccargo",
        ])
        .output()
        .expect("failed to build host ccargo");

    assert!(
        output.status.success(),
        "failed to build host ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );

    root.join("target/debug/ccargo")
}

#[test]
fn kernel_workspace() {
    let _guard = CCARGO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = repo_root();
    let ccargo = host_ccargo(&root);

    println!("ccargo build kernel --target x86_64-anyos ...");
    let output = Command::new(ccargo)
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["build", "kernel", "--target", "x86_64-anyos"])
        .output()
        .expect("failed to run host ccargo");

    if output.status.success() {
        println!("kernel ... ok");
        return;
    }

    println!("kernel ... not ok");
    panic!(
        "kernel failed to build with ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );
}

fn truncate_output(output: &str) -> String {
    const MAX: usize = 8 * 1024;
    if output.len() <= MAX {
        return output.to_string();
    }
    format!(
        "{}... <truncated {} bytes>",
        &output[..MAX],
        output.len() - MAX
    )
}
