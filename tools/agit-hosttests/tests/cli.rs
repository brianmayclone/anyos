use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("tools/agit-hosttests should live below repo root")
        .to_path_buf()
}

fn build_agit() -> PathBuf {
    let root = repo_root();
    let output = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("bin/agit/Cargo.toml"))
        .arg("--no-default-features")
        .arg("--features")
        .arg("host")
        .current_dir(&root)
        .output()
        .expect("failed to spawn cargo build for agit");

    assert_success("cargo build agit(host)", &output);
    root.join("target/debug/cgit")
}

fn temp_repo(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "agit-hosttest-{}-{}-{}",
        name,
        std::process::id(),
        unique
    ));
    fs::create_dir_all(&path).expect("failed to create temp repo");
    path
}

fn run(bin: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {:?} {:?}: {}", bin, args, err))
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn assert_success(label: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{} failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        label,
        output.status.code(),
        stdout(output),
        stderr(output)
    );
}

fn assert_clean_status(output: &Output) {
    assert_success("status --porcelain", output);
    assert_eq!("", stdout(output), "stderr:\n{}", stderr(output));
}

#[test]
fn porcelain_status_tracks_untracked_staged_and_modified_files() {
    let bin = build_agit();
    let repo = temp_repo("status");

    assert_success("init", &run(&bin, &repo, &["init", "."]));

    let branch = run(&bin, &repo, &["branch", "--show-current"]);
    assert_success("branch --show-current", &branch);
    assert_eq!("main\n", stdout(&branch));

    fs::write(repo.join("hello.txt"), b"hello\n").expect("failed to write hello.txt");

    let status = run(&bin, &repo, &["status", "--porcelain"]);
    assert_success("status --porcelain", &status);
    assert_eq!("?? hello.txt\n", stdout(&status));

    let argv0_compat_status = run(&bin, &repo, &["git", "status", "--porcelain"]);
    assert_success(
        "git status --porcelain argv0 compatibility",
        &argv0_compat_status,
    );
    assert_eq!("?? hello.txt\n", stdout(&argv0_compat_status));

    assert_success("add", &run(&bin, &repo, &["add", "hello.txt"]));

    let status = run(&bin, &repo, &["status", "--porcelain"]);
    assert_success("status --porcelain after add", &status);
    assert_eq!("A  hello.txt\n", stdout(&status));

    assert_success(
        "config user.name",
        &run(&bin, &repo, &["config", "user.name", "Agit Test"]),
    );
    assert_success(
        "config user.email",
        &run(
            &bin,
            &repo,
            &["config", "user.email", "agit@example.invalid"],
        ),
    );
    assert_success("commit", &run(&bin, &repo, &["commit", "-m", "initial"]));
    assert_clean_status(&run(&bin, &repo, &["status", "--porcelain"]));

    fs::write(repo.join("hello.txt"), b"hello again\n").expect("failed to modify hello.txt");
    let status = run(&bin, &repo, &["status", "--porcelain"]);
    assert_success("status --porcelain after modify", &status);
    assert_eq!(" M hello.txt\n", stdout(&status));
}
