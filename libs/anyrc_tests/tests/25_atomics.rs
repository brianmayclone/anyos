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
    let dir = std::env::temp_dir().join(format!("anyrc_test_atomics_{}_{}", std::process::id(), id));
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

// ── Parse/compile tests ──

#[test]
fn parse_atomic_import() {
    assert_compiles(r#"
        use core::sync::atomic::{AtomicBool, Ordering};
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_atomic_new() {
    assert_compiles(r#"
        use core::sync::atomic::AtomicBool;
        fn main() -> i32 {
            let x = AtomicBool::new(false);
            0
        }
    "#);
}

#[test]
fn compile_atomic_load_store() {
    assert_compiles(r#"
        use core::sync::atomic::{AtomicBool, Ordering};
        fn main() -> i32 {
            let mut x = AtomicBool::new(false);
            x.store(true, Ordering::Release);
            let v = x.load(Ordering::Acquire);
            0
        }
    "#);
}

#[test]
fn compile_atomic_u32() {
    assert_compiles(r#"
        use core::sync::atomic::{AtomicU32, Ordering};
        fn main() -> i32 {
            let mut x = AtomicU32::new(0);
            x.store(42, Ordering::SeqCst);
            let v = x.load(Ordering::SeqCst);
            0
        }
    "#);
}

#[test]
fn compile_ordering_import() {
    // Ordering variants should be accepted as expressions
    assert_compiles(r#"
        use core::sync::atomic::Ordering;
        fn main() -> i32 {
            let _r = Ordering::Relaxed;
            let _a = Ordering::Acquire;
            0
        }
    "#);
}

// ── Runtime tests ──

#[test]
fn run_atomic_store_load() {
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicU32, Ordering};
        fn main() -> i32 {
            let mut x = AtomicU32::new(0);
            x.store(42, Ordering::Relaxed);
            let v = x.load(Ordering::Relaxed);
            v as i32
        }
    "#, 42);
}

#[test]
fn run_atomic_new_value() {
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicU32, Ordering};
        fn main() -> i32 {
            let mut x = AtomicU32::new(7);
            let v = x.load(Ordering::Relaxed);
            v as i32
        }
    "#, 7);
}

#[test]
fn run_atomic_fetch_add() {
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicU32, Ordering};
        fn main() -> i32 {
            let mut x = AtomicU32::new(10);
            let old = x.fetch_add(5, Ordering::Relaxed);
            old as i32
        }
    "#, 10);
}

#[test]
fn run_atomic_fetch_add_new_value() {
    // After fetch_add, the new value should be old + delta
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicU32, Ordering};
        fn main() -> i32 {
            let mut x = AtomicU32::new(10);
            x.fetch_add(5, Ordering::Relaxed);
            let v = x.load(Ordering::Relaxed);
            v as i32
        }
    "#, 15);
}

// ── Alloc type import tests (compile only) ──

#[test]
fn compile_vec_import() {
    assert_compiles(r#"
        use alloc::vec::Vec;
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_box_import() {
    assert_compiles(r#"
        use alloc::boxed::Box;
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn compile_string_import() {
    assert_compiles(r#"
        use alloc::string::String;
        fn main() -> i32 { 0 }
    "#);
}

#[test]
fn run_kernel_address_module() {
    assert_run_returns(r#"
        struct PhysAddr { val: u64 }
        impl PhysAddr {
            fn new(addr: u64) -> PhysAddr { PhysAddr { val: addr } }
            fn as_u64(self) -> u64 { self.val }
            fn frame_index(self) -> i32 {
                let v = self.as_u64();
                let fs: u64 = 4096;
                (v / fs) as i32
            }
        }
        fn main() -> i32 {
            let addr = PhysAddr::new(8192);
            addr.frame_index()
        }
    "#, 2);
}

#[test]
fn run_kernel_mini_spinlock() {
    // Simplified spinlock pattern from kernel
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicBool, Ordering};

        struct Spinlock { locked: AtomicBool }

        impl Spinlock {
            fn new() -> Spinlock {
                Spinlock { locked: AtomicBool::new(false) }
            }
            fn lock(&self) {
                self.locked.store(true, Ordering::Acquire);
            }
            fn unlock(&self) {
                self.locked.store(false, Ordering::Release);
            }
            fn is_locked(&self) -> i32 {
                let v = self.locked.load(Ordering::Relaxed);
                if v { 1 } else { 0 }
            }
        }

        fn main() -> i32 {
            let s = Spinlock::new();
            s.lock();
            let r1 = s.is_locked();
            s.unlock();
            let r2 = s.is_locked();
            r1 + r2
        }
    "#, 1);
}

#[test]
fn run_kernel_port_io() {
    // Kernel-like: unsafe, inline asm, static, bitwise ops
    assert_run_returns(r#"
        static mut TEST_PORT: u32 = 0;

        fn outb(port: u16, val: u8) {
            unsafe { TEST_PORT = (port as u32) | ((val as u32) << 16); }
        }

        fn inb(port: u16) -> u8 {
            unsafe { (TEST_PORT >> 16) as u8 }
        }

        fn main() -> i32 {
            outb(0x3F8, 42);
            let v = inb(0x3F8);
            v as i32
        }
    "#, 42);
}
