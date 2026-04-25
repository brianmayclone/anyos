use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static CCARGO_LOCK: Mutex<()> = Mutex::new(());

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("anyrc_tests should live below libs/")
        .to_path_buf()
}

fn assert_ccargo_builds_bin(app: &str) {
    let root = repo_root();
    let package_path = format!("bin/{app}");
    let output = Command::new(root.join("scripts/test-ccargo"))
        .current_dir(&root)
        .env("CCARGO_FILTER", "0")
        .env("CARGO_TERM_COLOR", "never")
        .args(["build", package_path.as_str(), "--target", "x86_64-anyos"])
        .output()
        .expect("failed to run scripts/test-ccargo");

    assert!(
        output.status.success(),
        "ccargo failed for {package_path}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn ccargo_builds_tiny_bin_apps() {
    let _guard = CCARGO_LOCK.lock().unwrap();

    for app in ["false", "true", "echo", "pwd", "clear"] {
        assert_ccargo_builds_bin(app);
    }
}

#[test]
fn ccargo_builds_file_bin_apps() {
    let _guard = CCARGO_LOCK.lock().unwrap();

    for app in ["cat", "mkdir", "rm", "ls"] {
        assert_ccargo_builds_bin(app);
    }
}
