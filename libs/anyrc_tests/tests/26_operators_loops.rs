mod common;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn assert_run_returns(src: &str, expected: i32) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let exe_bytes = compile(src, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_test_oploop_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let exe_path = dir.join("test_exe");
    {
        let mut f = std::fs::File::create(&exe_path).unwrap();
        f.write_all(&exe_bytes).unwrap();
        f.sync_all().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let status = common::run_executable(&exe_path);
    let _ = std::fs::remove_dir_all(&dir);
    let code = status.code().unwrap_or(-1);
    assert_eq!(code, expected, "expected exit code {}, got {}", expected, code);
}

#[test]
fn run_bitwise_not() {
    // !0u64 = 0xFFFFFFFFFFFFFFFF, low byte = 255
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: u64 = 0;
            let y: u64 = !x;
            y as i32
        }
    "#, 255);
}

#[test]
fn run_compound_add() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut x: i32 = 10;
            x += 5;
            x
        }
    "#, 15);
}

#[test]
fn run_compound_shift() {
    // 3 << 2 = 12, 12 >> 1 = 6
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut x: i32 = 3;
            x <<= 2;
            x >>= 1;
            x
        }
    "#, 6);
}

#[test]
fn run_compound_or() {
    // 0xF0 | 0x0F = 0xFF = 255
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut x: i32 = 0xF0;
            x |= 0x0F;
            x
        }
    "#, 255);
}

#[test]
fn run_modulo() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i32 = 17;
            let y: i32 = x % 5;
            y
        }
    "#, 2);
}

#[test]
fn run_for_range() {
    // sum of 0..5 = 0+1+2+3+4 = 10
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut sum: i32 = 0;
            for i in 0..5 {
                sum += i;
            }
            sum
        }
    "#, 10);
}

#[test]
fn run_bitwise_xor() {
    // 0xFF ^ 0x0F = 0xF0 = 240
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i32 = 0xFF ^ 0x0F;
            x
        }
    "#, 240);
}

#[test]
fn run_alignment_mask() {
    // addr & !(4096 - 1) with addr = 4100 => 4096, exit code = 4096 % 256 = 0
    // Use smaller values to get meaningful exit code:
    // addr = 0x37 (55), mask = !(16 - 1) = !15 = 0xFFFF..FFF0
    // 55 & 0xFFF0 = 48
    assert_run_returns(r#"
        fn main() -> i32 {
            let addr: i64 = 55;
            let aligned: i64 = addr & !(16 as i64 - 1);
            aligned as i32
        }
    "#, 48);
}
