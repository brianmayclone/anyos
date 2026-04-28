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
    let asm = assemble_kernel_x86_objects(&root);
    let mut args: Vec<String> = vec![
        String::from("build"),
        String::from("kernel"),
        String::from("--target"),
        String::from("x86_64-anyos"),
    ];
    for object in &asm.objects {
        args.push(String::from("--link-arg"));
        args.push(object.clone());
    }

    println!("ccargo build kernel --target x86_64-anyos ...");
    let output = Command::new(ccargo)
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .env("ANYOS_AP_TRAMPOLINE", &asm.ap_trampoline)
        .args(args)
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

struct KernelAsm {
    objects: Vec<String>,
    ap_trampoline: String,
}

fn assemble_kernel_x86_objects(root: &Path) -> KernelAsm {
    let out_dir = root.join("target/anyrc-tests/kernel-asm");
    std::fs::create_dir_all(&out_dir).expect("failed to create kernel asm test dir");

    let sources = ["boot", "interrupts", "context_switch", "syscall_fast"];
    let mut objects = Vec::new();
    for name in sources {
        let src = root.join(format!("kernel/asm/x86/{}.asm", name));
        let obj = out_dir.join(format!("kernel_asm_{}.o", name));
        run_nasm(root, &["-w-all", "-f", "elf64", "-o"], &obj, &src);
        objects.push(obj.to_string_lossy().into_owned());
    }

    let trampoline_src = root.join("kernel/asm/x86/ap_trampoline.asm");
    let trampoline_out = out_dir.join("ap_trampoline.bin");
    run_nasm(
        root,
        &["-w-all", "-f", "bin", "-o"],
        &trampoline_out,
        &trampoline_src,
    );

    KernelAsm {
        objects,
        ap_trampoline: trampoline_out.to_string_lossy().into_owned(),
    }
}

fn run_nasm(root: &Path, prefix: &[&str], output: &Path, source: &Path) {
    let mut cmd = Command::new("nasm");
    cmd.current_dir(root).args(prefix).arg(output).arg(source);
    let output_result = cmd.output().expect("failed to run nasm");
    assert!(
        output_result.status.success(),
        "nasm failed for {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        source.display(),
        output_result.status,
        truncate_output(&String::from_utf8_lossy(&output_result.stdout)),
        truncate_output(&String::from_utf8_lossy(&output_result.stderr))
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
