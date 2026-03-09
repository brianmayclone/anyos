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
    };
    let exe_bytes = compile(src, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_test_arrays_{}_{}", std::process::id(), id));
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
    let status = std::process::Command::new(&exe_path)
        .status()
        .expect("failed to execute compiled binary");
    let _ = std::fs::remove_dir_all(&dir);
    let code = status.code().unwrap_or(-1);
    assert_eq!(code, expected, "expected exit code {}, got {}", expected, code);
}

#[test]
fn run_u32_max() {
    // u32::MAX = 0xFFFFFFFF; as i32 = -1; Linux exit code = lower 8 bits = 255
    assert_run_returns(r#"
        fn main() -> i32 {
            u32::MAX as i32
        }
    "#, 255);
}

#[test]
fn run_u8_max() {
    assert_run_returns(r#"
        fn main() -> i32 {
            u8::MAX as i32
        }
    "#, 255);
}

#[test]
fn run_array_init() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let a = [0i32; 4];
            a[0] as i32
        }
    "#, 0);
}

#[test]
fn run_array_write_read() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut a = [0i32; 4];
            a[2] = 42;
            a[2] as i32
        }
    "#, 42);
}

#[test]
fn run_array_sum() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut a = [0i32; 4];
            a[0] = 10;
            a[1] = 20;
            a[2] = 7;
            a[3] = 5;
            (a[0] + a[1] + a[2] + a[3]) as i32
        }
    "#, 42);
}

#[test]
fn run_array_len() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let a = [0i32; 5];
            a.len() as i32
        }
    "#, 5);
}
