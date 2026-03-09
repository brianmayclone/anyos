//! Integration tests that compile kernel-like code patterns.
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
    let dir = std::env::temp_dir().join(format!("anyrc_kernel_test_{}_{}", std::process::id(), id));
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

// ── Kernel address module ──

#[test]
fn kernel_phys_addr() {
    assert_run_returns(r#"
        const FRAME_SIZE: u64 = 4096;

        struct PhysAddr { val: u64 }

        impl PhysAddr {
            fn new(addr: u64) -> PhysAddr { PhysAddr { val: addr } }
            fn as_u64(self) -> u64 { self.val }
            fn frame_index(self) -> i32 {
                (self.as_u64() / FRAME_SIZE) as i32
            }
            fn is_frame_aligned(self) -> bool {
                self.as_u64() % FRAME_SIZE == 0
            }
            fn frame_align_down(self) -> PhysAddr {
                PhysAddr { val: self.val & !(FRAME_SIZE - 1) }
            }
        }

        fn main() -> i32 {
            let a1 = PhysAddr::new(8192);
            let a2 = PhysAddr::new(8193);
            let idx = a1.frame_index();
            let aligned = a1.is_frame_aligned();
            let down = a2.frame_align_down();
            
            let mut result: i32 = idx;
            if aligned { result += 10; }
            if down.as_u64() == 8192 { result += 100; }
            result
        }
    "#, 112);  // 2 + 10 + 100
}

#[test]
fn debug_phys_addr_minimal() {
    // Minimal reproduction of failing kernel_phys_addr
    assert_run_returns(r#"
        const FRAME_SIZE: u64 = 4096;
        struct PhysAddr { val: u64 }
        impl PhysAddr {
            fn new(addr: u64) -> PhysAddr { PhysAddr { val: addr } }
            fn as_u64(self) -> u64 { self.val }
            fn frame_align_down(self) -> PhysAddr {
                PhysAddr { val: self.val & !(FRAME_SIZE - 1) }
            }
        }
        fn main() -> i32 {
            let a = PhysAddr::new(8193);
            let down = a.frame_align_down();
            if down.as_u64() == 8192 { 1 } else { 0 }
        }
    "#, 1);
}

// ── Kernel hex output pattern ──

#[test]
fn kernel_hex_digits() {
    assert_run_returns(r#"
        fn hex_digit(n: u64) -> i32 {
            let d = (n & 0xF) as u8;
            if d < 10 { (48 + d) as i32 } else { (97 + d - 10) as i32 }
        }

        fn main() -> i32 {
            let d0 = hex_digit(0);
            let d9 = hex_digit(9);
            let da = hex_digit(10);
            let df = hex_digit(15);
            
            let mut ok: i32 = 0;
            if d0 == 48 { ok += 1; }
            if d9 == 57 { ok += 1; }
            if da == 97 { ok += 1; }
            if df == 102 { ok += 1; }
            ok
        }
    "#, 4);
}

// ── Kernel spinlock pattern (simplified) ──

#[test]
fn kernel_spinlock_pattern() {
    assert_run_returns(r#"
        use core::sync::atomic::{AtomicBool, Ordering};

        struct Spinlock {
            locked: AtomicBool,
        }

        impl Spinlock {
            fn new() -> Spinlock {
                Spinlock { locked: AtomicBool::new(false) }
            }
            fn acquire(&self) {
                self.locked.store(true, Ordering::Acquire);
            }
            fn release(&self) {
                self.locked.store(false, Ordering::Release);
            }
            fn is_held(&self) -> bool {
                self.locked.load(Ordering::Relaxed)
            }
        }

        fn main() -> i32 {
            let lock = Spinlock::new();
            let mut sum: i32 = 0;

            lock.acquire();
            if lock.is_held() { sum += 1; }
            lock.release();
            if !lock.is_held() { sum += 10; }

            sum
        }
    "#, 11);
}

// ── Kernel memory operations pattern ──

#[test]
fn kernel_bitfield_ops() {
    assert_run_returns(r#"
        const PRESENT: u64 = 1;
        const WRITABLE: u64 = 1 << 1;
        const USER: u64 = 1 << 2;

        fn make_entry(addr: u64, flags: u64) -> u64 {
            (addr & !(4096 - 1)) | flags
        }

        fn is_present(entry: u64) -> bool {
            entry & PRESENT != 0
        }

        fn is_writable(entry: u64) -> bool {
            entry & WRITABLE != 0
        }

        fn main() -> i32 {
            let entry = make_entry(0x1000, PRESENT | WRITABLE);
            let mut r: i32 = 0;
            if is_present(entry) { r += 1; }
            if is_writable(entry) { r += 10; }
            if entry & USER == 0 { r += 100; }
            r
        }
    "#, 111);
}

// ── Kernel array + for loop ──

#[test]
fn kernel_array_processing() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let mut buf = [0i32; 8];
            
            for i in 0..8 {
                buf[i] = i as i32 * 2;
            }
            
            let mut sum: i32 = 0;
            for i in 0..8 {
                sum += buf[i];
            }
            sum
        }
    "#, 56);  // 0+2+4+6+8+10+12+14 = 56
}

// ── Kernel enum dispatch pattern ──

#[test]
fn kernel_device_dispatch() {
    assert_run_returns(r#"
        enum DeviceType {
            Serial,
            Vga,
            Network,
        }

        fn device_priority(dev: DeviceType) -> i32 {
            match dev {
                DeviceType::Serial => 1,
                DeviceType::Vga => 2,
                DeviceType::Network => 3,
            }
        }

        fn main() -> i32 {
            let s = device_priority(DeviceType::Serial);
            let v = device_priority(DeviceType::Vga);
            let n = device_priority(DeviceType::Network);
            s + v + n
        }
    "#, 6);
}

// ── Kernel module pattern ──

#[test]
fn kernel_module_structure() {
    assert_run_returns(r#"
        mod memory {
            pub const FRAME_SIZE: u64 = 4096;
            
            pub struct PhysAddr { pub val: u64 }
            
            impl PhysAddr {
                pub fn new(addr: u64) -> PhysAddr { PhysAddr { val: addr } }
                pub fn frame_number(self) -> u64 { self.val / FRAME_SIZE }
            }
        }

        mod arch {
            pub fn page_table_levels() -> i32 { 4 }
        }

        fn main() -> i32 {
            let addr = memory::PhysAddr::new(16384);
            let frame = addr.frame_number();
            let levels = arch::page_table_levels();
            (frame as i32) + levels
        }
    "#, 8);  // 16384/4096=4, +4 levels = 8
}
