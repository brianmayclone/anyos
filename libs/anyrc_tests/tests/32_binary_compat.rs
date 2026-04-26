mod common;

use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use anyrc::linker::anyos;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

static CCARGO_LOCK: Mutex<()> = Mutex::new(());
static HOST_CCARGO: OnceLock<PathBuf> = OnceLock::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

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

fn compile_exe(source: &str, cfg_flags: Vec<String>) -> Vec<u8> {
    let options = CompileOptions {
        input: "compat.rs".to_string(),
        output: "compat".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        cfg_flags,
        ..CompileOptions::default()
    };
    compile(source, "compat.rs", &options).expect("compilation failed")
}

fn compile_linux_exe(source: &str) -> Vec<u8> {
    compile_exe(source, vec!["target_os=\"linux\"".to_string()])
}

fn compile_anyos_exe(source: &str) -> Vec<u8> {
    compile_exe(
        source,
        vec![
            "target_os=\"none\"".to_string(),
            "target_arch=\"x86_64\"".to_string(),
            "target_pointer_width=\"64\"".to_string(),
            "target_endian=\"little\"".to_string(),
        ],
    )
}

fn run_elf_bytes(exe_bytes: &[u8]) -> i32 {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("anyrc_binary_compat_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let exe_path = dir.join("compat");
    {
        let mut f = std::fs::File::create(&exe_path).unwrap();
        f.write_all(exe_bytes).unwrap();
        f.sync_all().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let status = common::run_executable(&exe_path);
    let _ = std::fs::remove_dir_all(&dir);
    status.code().unwrap_or(-1)
}

fn assert_linux_loader_contract(exe: &[u8]) {
    let hdr = anyos::parse_elf64_header(exe).expect("invalid ELF64 header");
    let loads = anyos::program_headers(exe, &hdr)
        .expect("invalid program headers")
        .into_iter()
        .filter(|ph| ph.ty == PT_LOAD)
        .collect::<Vec<_>>();
    assert!(!loads.is_empty(), "Linux executable has no PT_LOAD segment");
    assert!(
        loads.iter().any(|ph| contains_addr(ph, hdr.entry)),
        "entry point {:#x} is outside PT_LOAD segments",
        hdr.entry
    );
}

fn assert_anyos_loader_contract(exe: &[u8]) {
    anyos::validate_user_elf(exe).expect("invalid anyOS user ELF");
}

fn assert_generated_anyos_start_stub(exe: &[u8]) {
    anyos::validate_generated_start_stub(exe).expect("invalid generated anyOS _start stub");
}

fn build_ccargo_anyos_binary(crate_path: &str, extra_args: &[&str]) -> PathBuf {
    let _guard = CCARGO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = repo_root();
    let ccargo = host_ccargo(&root);

    println!(
        "ccargo build {crate_path} --target x86_64-anyos {}...",
        extra_args.join(" ")
    );
    let mut args = vec!["build", crate_path, "--target", "x86_64-anyos"];
    args.extend_from_slice(extra_args);
    let output = Command::new(ccargo)
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .args(args)
        .output()
        .expect("failed to run host ccargo");

    assert!(
        output.status.success(),
        "{crate_path} failed to build with ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );

    root.join(crate_path).join("target/debug")
}

fn assert_ccargo_default_anyos_binary_is_flat(crate_path: &str, binary_name: &str) {
    let target_dir = build_ccargo_anyos_binary(crate_path, &[]);
    let bin_path = target_dir.join(binary_name);
    let bin = std::fs::read(&bin_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {}", bin_path.display(), err));
    assert_ne!(&bin[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(
        bin[0], 0xe9,
        "flat binary should start with entry trampoline"
    );
    println!("{crate_path} default flat binary ... ok");
}

fn assert_ccargo_explicit_anyos_elf(crate_path: &str, binary_name: &str) {
    let root = repo_root();
    let target_dir = build_ccargo_anyos_binary(crate_path, &["--format", "elf"]);
    let exe_path = root.join(crate_path).join("target/debug").join(binary_name);
    let exe = std::fs::read(&exe_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {}", exe_path.display(), err));
    assert_anyos_loader_contract(&exe);
    let loads = anyos::program_headers(&exe, &anyos::parse_elf64_header(&exe).unwrap())
        .unwrap()
        .into_iter()
        .filter(|ph| ph.ty == PT_LOAD)
        .collect::<Vec<_>>();
    assert!(
        loads
            .iter()
            .any(|ph| (ph.flags & (PF_R | PF_X)) == (PF_R | PF_X) && (ph.flags & PF_W) == 0),
        "ccargo binary should contain an RX load segment"
    );
    assert!(
        loads
            .iter()
            .all(|ph| (ph.flags & (PF_W | PF_X)) != (PF_W | PF_X)),
        "ccargo binary should not emit RWX load segments"
    );
    assert_eq!(target_dir, exe_path.parent().unwrap());
    println!("{crate_path} explicit ELF compatibility ... ok");
}

fn contains_addr(ph: &anyos::ProgramHeader, addr: u64) -> bool {
    addr >= ph.vaddr && addr < ph.vaddr.saturating_add(ph.memsz)
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

#[test]
fn emitted_linux_abi_executables_run_on_host() {
    let cases = [
        (
            "arithmetic_and_calls",
            r#"
                fn add(a: i32, b: i32) -> i32 { a + b }
                fn main() -> i32 { add(30, 12) }
            "#,
            42,
        ),
        (
            "stack_struct_and_array",
            r#"
                struct Pair { a: i32, b: i32 }
                fn main() -> i32 {
                    let p = Pair { a: 5, b: 7 };
                    let xs = [10, 20, 30];
                    p.a + p.b + xs[1]
                }
            "#,
            32,
        ),
        (
            "branching",
            r#"
                fn main() -> i32 {
                    let mut acc = 0;
                    let mut i = 0;
                    while i < 5 {
                        acc = acc + i;
                        i = i + 1;
                    }
                    acc
                }
            "#,
            10,
        ),
    ];

    for (name, source, expected) in cases {
        let exe = compile_linux_exe(source);
        assert_linux_loader_contract(&exe);
        let actual = run_elf_bytes(&exe);
        assert_eq!(
            actual, expected,
            "{name}: expected exit code {expected}, got {actual}"
        );
        println!("{name} ... ok");
    }
}

#[test]
fn emitted_anyos_abi_executable_matches_loader_contract() {
    let exe = compile_anyos_exe("fn main() -> i32 { 7 }");
    assert_anyos_loader_contract(&exe);
    assert_generated_anyos_start_stub(&exe);
}

#[test]
fn ccargo_emitted_anyos_binaries_match_loader_contract() {
    assert_ccargo_default_anyos_binary_is_flat("bin/pwd", "pwd");
    assert_ccargo_explicit_anyos_elf("bin/pwd", "pwd");
    assert_ccargo_default_anyos_binary_is_flat("bin/true", "true_cmd");
    assert_ccargo_explicit_anyos_elf("bin/true", "true_cmd");
}
