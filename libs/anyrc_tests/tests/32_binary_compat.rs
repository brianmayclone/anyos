mod common;

use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const ANYOS_USER_BASE: u64 = 0x0800_0000;
const KERNEL_SPACE_BASE: u64 = 0xffff_8000_0000_0000;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

static CCARGO_LOCK: Mutex<()> = Mutex::new(());
static HOST_CCARGO: OnceLock<PathBuf> = OnceLock::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Elf64Header {
    entry: u64,
    phoff: usize,
    phentsize: usize,
    phnum: usize,
}

#[derive(Debug)]
struct ProgramHeader {
    ty: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

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

fn parse_elf64_header(data: &[u8]) -> Elf64Header {
    assert!(data.len() >= 64, "ELF file is too small");
    assert_eq!(&data[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(data[4], 2, "expected ELFCLASS64");
    assert_eq!(data[5], 1, "expected little-endian ELF");
    assert_eq!(u16_at(data, 16), 2, "expected ET_EXEC");
    assert_eq!(u16_at(data, 18), 0x3e, "expected EM_X86_64");
    assert_eq!(u16_at(data, 52), 64, "unexpected ELF header size");

    Elf64Header {
        entry: u64_at(data, 24),
        phoff: u64_at(data, 32) as usize,
        phentsize: u16_at(data, 54) as usize,
        phnum: u16_at(data, 56) as usize,
    }
}

fn program_headers(data: &[u8], hdr: &Elf64Header) -> Vec<ProgramHeader> {
    assert_eq!(hdr.phentsize, 56, "unexpected program header size");
    assert!(hdr.phnum > 0, "executable has no program headers");
    let table_end = hdr.phoff + hdr.phentsize * hdr.phnum;
    assert!(
        table_end <= data.len(),
        "program header table is outside the file"
    );

    (0..hdr.phnum)
        .map(|idx| {
            let off = hdr.phoff + idx * hdr.phentsize;
            ProgramHeader {
                ty: u32_at(data, off),
                flags: u32_at(data, off + 4),
                offset: u64_at(data, off + 8),
                vaddr: u64_at(data, off + 16),
                filesz: u64_at(data, off + 32),
                memsz: u64_at(data, off + 40),
                align: u64_at(data, off + 48),
            }
        })
        .collect()
}

fn assert_linux_loader_contract(exe: &[u8]) {
    let hdr = parse_elf64_header(exe);
    let loads = program_headers(exe, &hdr)
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
    let hdr = parse_elf64_header(exe);
    let loads = program_headers(exe, &hdr)
        .into_iter()
        .filter(|ph| ph.ty == PT_LOAD)
        .collect::<Vec<_>>();
    assert!(!loads.is_empty(), "anyOS executable has no PT_LOAD segment");

    let mut executable_entry_segment = None;
    for ph in &loads {
        assert!(
            ph.vaddr >= ANYOS_USER_BASE,
            "PT_LOAD segment starts below 128 MiB identity-map boundary: {:#x}",
            ph.vaddr
        );
        assert!(
            ph.vaddr < KERNEL_SPACE_BASE,
            "PT_LOAD segment starts in kernel space: {:#x}",
            ph.vaddr
        );
        assert!(
            ph.vaddr.checked_add(ph.memsz).unwrap_or(u64::MAX) <= KERNEL_SPACE_BASE,
            "PT_LOAD segment crosses into kernel space"
        );
        assert!(ph.filesz <= ph.memsz, "p_filesz must not exceed p_memsz");
        assert!(
            ph.offset + ph.filesz <= exe.len() as u64,
            "PT_LOAD segment reads past end of file"
        );
        assert!(
            ph.align == 0 || ph.align.is_power_of_two(),
            "p_align must be zero or a power of two"
        );
        if ph.align > 1 {
            assert_eq!(
                ph.offset % ph.align,
                ph.vaddr % ph.align,
                "p_offset and p_vaddr must be congruent modulo p_align"
            );
        }
        if contains_addr(ph, hdr.entry) {
            executable_entry_segment = Some(ph);
        }
    }

    let entry_segment = executable_entry_segment
        .unwrap_or_else(|| panic!("entry point {:#x} is outside PT_LOAD segments", hdr.entry));
    assert_ne!(
        entry_segment.flags & PF_X,
        0,
        "entry point must live in an executable segment"
    );
    assert_ne!(
        entry_segment.flags & PF_R,
        0,
        "entry segment must be readable"
    );

    let entry_file_offset = entry_segment.offset + (hdr.entry - entry_segment.vaddr);
    assert!(
        entry_file_offset as usize + 20 <= exe.len(),
        "entry stub is outside the file"
    );
}

fn assert_generated_anyos_start_stub(exe: &[u8]) {
    let hdr = parse_elf64_header(exe);
    let entry_segment = program_headers(exe, &hdr)
        .into_iter()
        .find(|ph| ph.ty == PT_LOAD && contains_addr(ph, hdr.entry))
        .expect("entry point is outside PT_LOAD segments");
    let entry_file_offset = (entry_segment.offset + (hdr.entry - entry_segment.vaddr)) as usize;
    let stub = &exe[entry_file_offset..entry_file_offset + 20];

    assert_eq!(stub[0], 0xe8, "generated _start must call main first");
    assert_eq!(
        &stub[5..20],
        &[
            0x48, 0x89, 0xc7, // mov rdi, rax
            0x48, 0x89, 0xfb, // mov rbx, rdi
            0x48, 0xc7, 0xc0, 0x01, 0x00, 0x00, 0x00, // mov rax, SYS_EXIT
            0x0f, 0x05, // syscall
        ],
        "generated _start must use the anyOS syscall ABI"
    );
}

fn assert_ccargo_anyos_binary(crate_path: &str, binary_name: &str) {
    let _guard = CCARGO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = repo_root();
    let ccargo = host_ccargo(&root);

    println!("ccargo build {crate_path} --target x86_64-anyos ...");
    let output = Command::new(ccargo)
        .current_dir(&root)
        .env("CARGO_TERM_COLOR", "never")
        .args(["build", crate_path, "--target", "x86_64-anyos"])
        .output()
        .expect("failed to run host ccargo");

    assert!(
        output.status.success(),
        "{crate_path} failed to build with ccargo\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        truncate_output(&String::from_utf8_lossy(&output.stdout)),
        truncate_output(&String::from_utf8_lossy(&output.stderr))
    );

    let exe_path = root.join(crate_path).join("target/debug").join(binary_name);
    let exe = std::fs::read(&exe_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {}", exe_path.display(), err));
    assert_anyos_loader_contract(&exe);
    assert!(
        program_headers(&exe, &parse_elf64_header(&exe))
            .iter()
            .any(|ph| ph.ty == PT_LOAD && (ph.flags & (PF_R | PF_W | PF_X)) == (PF_R | PF_W | PF_X)),
        "ccargo binary should currently be emitted as a single RWX PT_LOAD segment"
    );
    println!("{crate_path} binary compatibility ... ok");
}

fn contains_addr(ph: &ProgramHeader, addr: u64) -> bool {
    addr >= ph.vaddr && addr < ph.vaddr.saturating_add(ph.memsz)
}

fn u16_at(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(data[off..off + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(data[off..off + 8].try_into().unwrap())
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
    assert_ccargo_anyos_binary("bin/pwd", "pwd");
    assert_ccargo_anyos_binary("bin/true", "true_cmd");
}
