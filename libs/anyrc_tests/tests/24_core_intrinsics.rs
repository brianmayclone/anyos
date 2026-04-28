mod common;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
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
        ..CompileOptions::default()
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
        ..CompileOptions::default()
    };
    let exe_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "anyrc_test_intrinsics_{}_{}",
        std::process::id(),
        id
    ));
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
    assert_eq!(
        code, expected,
        "expected exit code {}, got {}",
        expected, code
    );
}

// ── Compilation tests ──

#[test]
fn compile_null_mut() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let p = core::ptr::null_mut::<i32>();
            0
        }
    "#,
    );
}

#[test]
fn compile_size_of() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let s = core::mem::size_of::<i32>();
            0
        }
    "#,
    );
}

#[test]
fn compile_write_volatile() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let mut x: i32 = 0;
            let p = &mut x as *mut i32;
            unsafe { core::ptr::write_volatile(p, 42); }
            0
        }
    "#,
    );
}

#[test]
fn compile_use_import() {
    assert_compiles(
        r#"
        use core::ptr::null_mut;
        fn main() -> i32 {
            let p = null_mut::<i32>();
            0
        }
    "#,
    );
}

#[test]
fn compile_char_from_u32() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let c = core::char::from_u32(65);
            0
        }
    "#,
    );
}

#[test]
fn compile_maybe_uninit_assume_init_array() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let values: [core::mem::MaybeUninit<u32>; 4] =
                unsafe { core::mem::MaybeUninit::uninit().assume_init() };
            0
        }
    "#,
    );
}

#[test]
fn compile_alloc_rc_weak_import() {
    assert_compiles(
        r#"
        extern crate alloc;
        use alloc::rc::{Rc, Weak};

        struct Cell {
            value: u32,
        }

        fn main() -> i32 {
            let rc = Rc::new(Cell { value: 1 });
            let weak: Weak<Cell> = Rc::downgrade(&rc);
            0
        }
    "#,
    );
}

#[test]
fn compile_option_or_and_raw_pointer_helpers() {
    assert_compiles(
        r#"
        extern crate alloc;

        use alloc::rc::Rc;

        fn main() -> i32 {
            let a = Some(1usize);
            let b = None.or(a);
            let _ = b.unwrap_or(0);

            let rc = Rc::new(7usize);
            let _p = Rc::as_ptr(&rc);
            let _raw = Rc::into_raw(rc);

            let value = 3usize;
            let _ptr = core::ptr::from_ref(&value);
            let _one = core::slice::from_ref(&value);
            let _f = f64::from_bits(0);
            let _g = f32::from_bits(0);
            0
        }
    "#,
    );
}

#[test]
fn compile_string_from_utf8_lossy_and_try_from() {
    assert_compiles(
        r#"
        extern crate alloc;

        use alloc::string::String;

        fn parse(bytes: &[u8], value: i64) -> String {
            let Ok(_n) = usize::try_from(value) else {
                return String::new();
            };
            String::from_utf8_lossy(bytes).into_owned()
        }

        fn main() -> i32 {
            let bytes = [65u8, 66u8];
            let _s = parse(&bytes, 7);
            0
        }
    "#,
    );
}

#[test]
fn run_primitive_assoc_from() {
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let b: u8 = 7;
            let n: u32 = u32::from(b);
            n as i32
        }
    "#,
        7,
    );
}

#[test]
fn compile_u32_from_ne_bytes() {
    assert_compiles(
        r#"
        fn main() -> i32 {
            let n = u32::from_ne_bytes([42u8, 0u8, 0u8, 0u8]);
            n as i32
        }
    "#,
    );
}

#[test]
fn compile_null_mut_with_typed_raw_ptr_field_access() {
    assert_compiles(
        r#"
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
    "#,
    );
}

// ── Runtime tests ──

#[test]
fn run_null_mut() {
    // null_mut returns 0, used as exit code
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let p = core::ptr::null_mut::<i32>();
            p as i32
        }
    "#,
        0,
    );
}

#[test]
fn run_size_of() {
    // size_of returns 8 (all types are 8 bytes in our compiler)
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let s = core::mem::size_of::<i64>();
            s as i32
        }
    "#,
        8,
    );
}

#[test]
fn run_spin_loop() {
    // spin_loop() should not crash, return 0
    assert_run_returns(
        r#"
        fn main() -> i32 {
            core::hint::spin_loop();
            0
        }
    "#,
        0,
    );
}

#[test]
fn run_forget() {
    // forget is a no-op, should not crash
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let x: i32 = 42;
            core::mem::forget(x);
            0
        }
    "#,
        0,
    );
}

#[test]
fn run_use_import_null_mut() {
    assert_run_returns(
        r#"
        use core::ptr::null_mut;
        fn main() -> i32 {
            let p = null_mut::<i32>();
            p as i32
        }
    "#,
        0,
    );
}

#[test]
fn run_core_ptr_copy_u64_counts_elements() {
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let src: u64 = 42;
            let mut dst: u64 = 0;
            unsafe {
                core::ptr::copy(&src as *const u64, &mut dst as *mut u64, 1);
            }
            dst as i32
        }
    "#,
        42,
    );
}

#[test]
fn run_atomic_fence_is_emit_builtin() {
    assert_run_returns(
        r#"
        fn main() -> i32 {
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            0
        }
    "#,
        0,
    );
}

#[test]
fn run_range_contains_uses_item_value_for_both_bounds() {
    assert_run_returns(
        r#"
        fn main() -> i32 {
            let range = 10usize..50usize;
            let yes = 42usize;
            let no = 5usize;
            if range.contains(&yes) && !range.contains(&no) {
                42
            } else {
                1
            }
        }
    "#,
        42,
    );
}

#[test]
fn run_vecdeque_len_is_canonical_alloc_method() {
    assert_run_returns(
        r#"
        extern crate alloc;
        use alloc::collections::VecDeque;

        fn main() -> i32 {
            let mut queue = VecDeque::new();
            queue.push_back(10usize);
            queue.push_back(32usize);
            queue.len() as i32
        }
    "#,
        2,
    );
}
