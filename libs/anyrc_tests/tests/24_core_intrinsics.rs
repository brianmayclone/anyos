use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn assert_compiles(src: &str) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    compile(src, "test.rs", &options).expect("compilation failed");
}

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
    let dir = std::env::temp_dir().join(format!("anyrc_test_intrinsics_{}_{}", std::process::id(), id));
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

// ── Compilation tests ──

#[test]
fn compile_null_mut() {
    assert_compiles(r#"
        fn main() -> i32 {
            let p = core::ptr::null_mut::<i32>();
            0
        }
    "#);
}

#[test]
fn compile_size_of() {
    assert_compiles(r#"
        fn main() -> i32 {
            let s = core::mem::size_of::<i32>();
            0
        }
    "#);
}

#[test]
fn compile_write_volatile() {
    assert_compiles(r#"
        fn main() -> i32 {
            let mut x: i32 = 0;
            let p = &mut x as *mut i32;
            unsafe { core::ptr::write_volatile(p, 42); }
            0
        }
    "#);
}

#[test]
fn compile_use_import() {
    assert_compiles(r#"
        use core::ptr::null_mut;
        fn main() -> i32 {
            let p = null_mut::<i32>();
            0
        }
    "#);
}

#[test]
fn compile_null_mut_with_typed_raw_ptr_field_access() {
    assert_compiles(r#"
        struct Node {
            next: *mut Node,
        }

        fn main() -> i32 {
            let mut prev: *mut Node = core::ptr::null_mut();
            let curr: *mut Node = core::ptr::null_mut();
            if prev.is_null() {
                prev = curr;
            } else {
                unsafe { (*prev).next = curr; }
            }
            0
        }
    "#);
}

// ── Runtime tests ──

#[test]
fn run_null_mut() {
    // null_mut returns 0, used as exit code
    assert_run_returns(r#"
        fn main() -> i32 {
            let p = core::ptr::null_mut::<i32>();
            p as i32
        }
    "#, 0);
}

#[test]
fn run_size_of() {
    // size_of returns 8 (all types are 8 bytes in our compiler)
    assert_run_returns(r#"
        fn main() -> i32 {
            let s = core::mem::size_of::<i64>();
            s as i32
        }
    "#, 8);
}

#[test]
fn run_spin_loop() {
    // spin_loop() should not crash, return 0
    assert_run_returns(r#"
        fn main() -> i32 {
            core::hint::spin_loop();
            0
        }
    "#, 0);
}

#[test]
fn run_forget() {
    // forget is a no-op, should not crash
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i32 = 42;
            core::mem::forget(x);
            0
        }
    "#, 0);
}

#[test]
fn run_use_import_null_mut() {
    assert_run_returns(r#"
        use core::ptr::null_mut;
        fn main() -> i32 {
            let p = null_mut::<i32>();
            p as i32
        }
    "#, 0);
}
