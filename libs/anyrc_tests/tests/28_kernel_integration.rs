//! Integration tests that compile kernel-like code patterns.
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
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
    let exe_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "anyrc_kernel_test_{}_{}_{:?}",
        std::process::id(),
        id,
        std::thread::current().id()
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
    let mut attempts = 0u32;
    let status = loop {
        match std::process::Command::new(&exe_path).status() {
            Ok(status) => break status,
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                attempts += 1;
                if attempts > 20 {
                    panic!("failed to execute compiled binary: {}", err);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(err) => panic!("failed to execute compiled binary: {}", err),
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    let code = status.code().unwrap_or(-1);
    assert_eq!(
        code, expected,
        "expected exit code {}, got {}",
        expected, code
    );
}

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

// ── Kernel address module ──

#[test]
fn kernel_phys_addr() {
    assert_run_returns(
        r#"
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
    "#,
        112,
    ); // 2 + 10 + 100
}

#[test]
fn debug_phys_addr_minimal() {
    // Minimal reproduction of failing kernel_phys_addr
    assert_run_returns(
        r#"
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
    "#,
        1,
    );
}

// ── Kernel hex output pattern ──

#[test]
fn kernel_hex_digits() {
    assert_run_returns(
        r#"
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
    "#,
        4,
    );
}

// ── Kernel spinlock pattern (simplified) ──

#[test]
fn kernel_spinlock_pattern() {
    assert_run_returns(
        r#"
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
    "#,
        11,
    );
}

// ── Kernel memory operations pattern ──

#[test]
fn kernel_bitfield_ops() {
    assert_run_returns(
        r#"
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
    "#,
        111,
    );
}

// ── Kernel array + for loop ──

#[test]
fn kernel_array_processing() {
    assert_run_returns(
        r#"
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
    "#,
        56,
    ); // 0+2+4+6+8+10+12+14 = 56
}

// ── Kernel enum dispatch pattern ──

#[test]
fn kernel_device_dispatch() {
    assert_run_returns(
        r#"
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
    "#,
        6,
    );
}

// ── Kernel module pattern ──

#[test]
fn kernel_module_structure() {
    assert_run_returns(
        r#"
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
    "#,
        8,
    ); // 16384/4096=4, +4 levels = 8
}

// ── Real kernel address.rs (adapted) ──

#[test]
fn debug_frame_index() {
    // PhysAddr(0x2000).frame_index() should be 2 — tuple struct with const fn
    assert_run_returns(
        r#"
        const FRAME_SIZE: u64 = 4096;

        struct PhysAddr(u64);

        impl PhysAddr {
            const fn new(addr: u64) -> Self { PhysAddr(addr) }
            const fn as_u64(self) -> u64 { self.0 }
            fn frame_index(self) -> u64 {
                self.as_u64() / FRAME_SIZE
            }
        }

        fn main() -> i32 {
            let p = PhysAddr::new(0x2000);
            p.frame_index() as i32
        }
    "#,
        2,
    );
}

#[test]
fn debug_as_u32() {
    // Tuple struct .0 access + u64 -> u32 cast
    assert_run_returns(
        r#"
        struct PhysAddr(u64);

        impl PhysAddr {
            fn new(addr: u64) -> Self { PhysAddr(addr) }
            fn as_u32(self) -> u32 { self.0 as u32 }
        }

        fn main() -> i32 {
            let p = PhysAddr::new(0x2000);
            if p.as_u32() == 0x2000 { 1 } else { 0 }
        }
    "#,
        1,
    );
}

#[test]
fn real_kernel_address_module() {
    // Actual kernel/src/memory/address.rs adapted:
    // - FRAME_SIZE inlined, derive removed, usize -> u64
    assert_run_returns(
        r#"
        const FRAME_SIZE: u64 = 4096;

        struct PhysAddr(u64);
        struct VirtAddr(u64);

        impl PhysAddr {
            fn new(addr: u64) -> PhysAddr { PhysAddr(addr) }
            fn as_u64(self) -> u64 { self.0 }
            fn as_u32(self) -> u32 { self.0 as u32 }
            fn frame_index(self) -> u64 { self.as_u64() / FRAME_SIZE }
            fn is_frame_aligned(self) -> bool { self.as_u64() % FRAME_SIZE == 0 }
            fn frame_align_down(self) -> PhysAddr {
                PhysAddr(self.0 & !(FRAME_SIZE - 1))
            }
            fn frame_align_up(self) -> PhysAddr {
                PhysAddr((self.0 + FRAME_SIZE - 1) & !(FRAME_SIZE - 1))
            }
        }

        impl VirtAddr {
            fn new(addr: u64) -> VirtAddr { VirtAddr(addr) }
            fn as_u64(self) -> u64 { self.0 }
            fn pml4_index(self) -> u64 { (self.as_u64() >> 39) & 0x1FF }
            fn pdpt_index(self) -> u64 { (self.as_u64() >> 30) & 0x1FF }
            fn pd_index(self) -> u64 { (self.as_u64() >> 21) & 0x1FF }
            fn pt_index(self) -> u64 { (self.as_u64() >> 12) & 0x1FF }
            fn page_offset(self) -> u64 { self.as_u64() & 0xFFF }
            fn is_page_aligned(self) -> bool { self.as_u64() % FRAME_SIZE == 0 }
        }

        fn main() -> i32 {
            let mut result: i32 = 0;

            let p1 = PhysAddr::new(0x2000);
            if p1.frame_index() == 2 { result += 1; }
            if p1.is_frame_aligned() { result += 2; }
            if p1.as_u32() == 0x2000 { result += 4; }

            let p2 = PhysAddr::new(0x2001);
            if !p2.is_frame_aligned() { result += 8; }
            if p2.frame_align_down().as_u64() == 0x2000 { result += 16; }
            if p2.frame_align_up().as_u64() == 0x3000 { result += 32; }

            let v = VirtAddr::new(0xFFFF_8000_0010_0ABC);
            if v.page_offset() == 0xABC { result += 64; }
            if v.is_page_aligned() == false { result += 128; }

            result
        }
    "#,
        255,
    );
}

// ── Real kernel port.rs (x86 I/O) ──

#[test]
fn real_kernel_port_io() {
    // Actual kernel/src/arch/x86/port.rs — compile only (can't run I/O port code on host)
    assert_compiles(
        r#"
        #![no_std]
        #![no_main]

        unsafe fn outb(port: u16, value: u8) {
            asm!("out dx, al", in("dx") port, in("al") value);
        }

        unsafe fn inb(port: u16) -> u8 {
            let value: u8;
            asm!("in al, dx", out("al") value, in("dx") port);
            value
        }

        unsafe fn outw(port: u16, value: u16) {
            asm!("out dx, ax", in("dx") port, in("ax") value);
        }

        unsafe fn inw(port: u16) -> u16 {
            let value: u16;
            asm!("in ax, dx", out("ax") value, in("dx") port);
            value
        }

        unsafe fn outl(port: u16, value: u32) {
            asm!("out dx, eax", in("dx") port, in("eax") value);
        }

        unsafe fn inl(port: u16) -> u32 {
            let value: u32;
            asm!("in eax, dx", out("eax") value, in("dx") port);
            value
        }

        unsafe fn io_wait() {
            outb(0x80, 0);
        }
    "#,
    );
}

// ── Real kernel boot_info.rs (adapted) ──

#[test]
fn real_kernel_boot_info() {
    // Actual kernel/src/boot_info.rs — structs + consts + method
    // Stripped: derive, doc comments, unsafe memory_map (needs core::slice)
    assert_compiles(
        r#"
        #![no_std]
        #![no_main]

        #[repr(C)]
        struct BootInfo {
            magic: u32,
            memory_map_addr: u32,
            memory_map_count: u32,
            framebuffer_addr: u32,
            framebuffer_pitch: u32,
            framebuffer_width: u32,
            framebuffer_height: u32,
            framebuffer_bpp: u8,
            boot_drive: u8,
            boot_mode: u8,
            _padding: u8,
            kernel_phys_start: u32,
            kernel_phys_end: u32,
            rsdp_addr: u32,
        }

        const BOOT_INFO_MAGIC: u32 = 0x414E594F;

        #[repr(C)]
        struct E820Entry {
            base_addr: u64,
            length: u64,
            entry_type: u32,
            acpi_extended: u32,
        }

        const E820_TYPE_USABLE: u32 = 1;
        const E820_TYPE_RESERVED: u32 = 2;
        const E820_TYPE_ACPI_RECLAIMABLE: u32 = 3;

        impl BootInfo {
            fn validate(&self) -> bool {
                self.magic == BOOT_INFO_MAGIC
            }
        }
    "#,
    );
}

// ── Combined kernel modules: address + boot_info ──

#[test]
fn real_kernel_combined_modules() {
    // Multiple kernel modules combined as inline mod blocks
    assert_run_returns(
        r#"
        mod memory {
            pub const FRAME_SIZE: u64 = 4096;

            pub struct PhysAddr(u64);
            pub struct VirtAddr(u64);

            impl PhysAddr {
                pub fn new(addr: u64) -> PhysAddr { PhysAddr(addr) }
                pub fn as_u64(self) -> u64 { self.0 }
                pub fn as_u32(self) -> u32 { self.0 as u32 }
                pub fn frame_index(self) -> u64 { self.as_u64() / FRAME_SIZE }
                pub fn is_frame_aligned(self) -> bool { self.as_u64() % FRAME_SIZE == 0 }
                pub fn frame_align_down(self) -> PhysAddr {
                    PhysAddr(self.0 & !(FRAME_SIZE - 1))
                }
                pub fn frame_align_up(self) -> PhysAddr {
                    PhysAddr((self.0 + FRAME_SIZE - 1) & !(FRAME_SIZE - 1))
                }
            }

            impl VirtAddr {
                pub fn new(addr: u64) -> VirtAddr { VirtAddr(addr) }
                pub fn as_u64(self) -> u64 { self.0 }
                pub fn pml4_index(self) -> u64 { (self.as_u64() >> 39) & 0x1FF }
                pub fn pdpt_index(self) -> u64 { (self.as_u64() >> 30) & 0x1FF }
                pub fn pd_index(self) -> u64 { (self.as_u64() >> 21) & 0x1FF }
                pub fn pt_index(self) -> u64 { (self.as_u64() >> 12) & 0x1FF }
                pub fn page_offset(self) -> u64 { self.as_u64() & 0xFFF }
            }
        }

        mod boot_info {
            pub const BOOT_INFO_MAGIC: u32 = 0x414E594F;
            pub const E820_TYPE_USABLE: u32 = 1;
            pub const E820_TYPE_RESERVED: u32 = 2;

            #[repr(C)]
            pub struct BootInfo {
                pub magic: u32,
                pub memory_map_addr: u32,
                pub memory_map_count: u32,
            }

            impl BootInfo {
                pub fn validate(&self) -> bool {
                    self.magic == BOOT_INFO_MAGIC
                }
            }
        }

        fn main() -> i32 {
            let mut result: i32 = 0;

            // Test PhysAddr from memory module
            let addr = memory::PhysAddr::new(0x3000);
            if addr.frame_index() == 3 { result += 1; }
            if addr.is_frame_aligned() { result += 2; }

            let unaligned = memory::PhysAddr::new(0x3456);
            if unaligned.frame_align_down().as_u64() == 0x3000 { result += 4; }
            if unaligned.frame_align_up().as_u64() == 0x4000 { result += 8; }

            // Test VirtAddr page table indices
            let virt = memory::VirtAddr::new(0x1234);
            if virt.page_offset() == 0x234 { result += 16; }
            if virt.pt_index() == 1 { result += 32; }

            // Test BootInfo from boot_info module
            if boot_info::BOOT_INFO_MAGIC == 0x414E594F { result += 64; }
            if boot_info::E820_TYPE_USABLE == 1 { result += 128; }

            result
        }
    "#,
        255,
    );
}

// ── Debug: &mut self method called twice ──

#[test]
fn debug_simple_field_set() {
    // Simplest possible: set a single field through &mut self method
    assert_run_returns(
        r#"
        struct S { val: i32 }
        impl S {
            fn set(&mut self, v: i32) { self.val = v; }
            fn get(&self) -> i32 { self.val }
        }
        fn main() -> i32 {
            let mut s = S { val: 0 };
            s.set(42);
            s.get()
        }
    "#,
        42,
    );
}

#[test]
fn debug_array_set_direct() {
    // Array set through method, read directly (no method)
    assert_run_returns(
        r#"
        struct Buf { data: [u8; 4] }
        impl Buf {
            fn new() -> Buf { Buf { data: [0u8; 4] } }
            fn set(&mut self, idx: i32, val: u8) {
                self.data[idx] = val;
            }
        }
        fn main() -> i32 {
            let mut b = Buf::new();
            b.set(0, 42);
            b.data[0] as i32
        }
    "#,
        42,
    );
}

#[test]
fn debug_array_get_method() {
    // Simplified: just test reading from a struct with method, using 0-init + set
    assert_run_returns(
        r#"
        struct Buf { data: [u8; 4] }
        impl Buf {
            fn get(&self, idx: i32) -> u8 {
                self.data[idx]
            }
        }
        fn main() -> i32 {
            let mut b = Buf { data: [0u8; 4] };
            b.data[0] = 42;
            b.get(0) as i32
        }
    "#,
        42,
    );
}

#[test]
fn debug_array_set_then_get_method() {
    // set through &mut self, then get through &self
    assert_run_returns(
        r#"
        struct Buf { data: [u8; 4] }
        impl Buf {
            fn set(&mut self, idx: i32, val: u8) {
                self.data[idx] = val;
            }
            fn get(&self, idx: i32) -> u8 {
                self.data[idx]
            }
        }
        fn main() -> i32 {
            let mut b = Buf { data: [0u8; 4] };
            b.set(0, 42);
            b.get(0) as i32
        }
    "#,
        42,
    );
}

#[test]
fn debug_mut_self_twice() {
    assert_run_returns(
        r#"
        struct Buf { data: [u8; 4] }
        impl Buf {
            fn new() -> Buf { Buf { data: [0u8; 4] } }
            fn set(&mut self, idx: i32, val: u8) {
                self.data[idx] = val;
            }
            fn get(&self, idx: i32) -> u8 {
                self.data[idx]
            }
        }
        fn main() -> i32 {
            let mut b = Buf::new();
            b.set(0, 42);
            b.get(0) as i32
        }
    "#,
        42,
    );
}

#[test]
fn debug_mut_self_in_module() {
    assert_run_returns(
        r#"
        mod stuff {
            pub struct Buf { pub data: [u8; 4] }
            impl Buf {
                pub fn new() -> Buf { Buf { data: [0u8; 4] } }
                pub fn set(&mut self, idx: i32, val: u8) {
                    self.data[idx] = val;
                }
                pub fn get(&self, idx: i32) -> u8 {
                    self.data[idx]
                }
            }
        }
        fn main() -> i32 {
            let mut b = stuff::Buf::new();
            b.set(0, 42);
            b.get(0) as i32
        }
    "#,
        42,
    );
}

#[test]
fn debug_self_method_in_method() {
    // While loop + sum of self.data[i] as i32
    assert_run_returns(
        r#"
        struct Buf { data: [u8; 4] }
        impl Buf {
            fn sum(&self) -> i32 {
                let mut i: i32 = 0;
                let mut s: i32 = 0;
                while i < 4 {
                    let v: u8 = self.data[i];
                    s = s + v as i32;
                    i = i + 1;
                }
                s
            }
        }
        fn main() -> i32 {
            let mut b = Buf { data: [0u8; 4] };
            b.data[0] = 1;
            b.data[1] = 2;
            b.data[2] = 3;
            b.data[3] = 4;
            b.sum()
        }
    "#,
        10,
    );
}

// ── Kernel bitmap frame allocator (simplified) ──

#[test]
fn real_kernel_frame_allocator() {
    assert_run_returns(
        r#"
        mod memory {
            pub const FRAME_SIZE: u64 = 4096;

            pub struct PhysAddr(u64);

            impl PhysAddr {
                pub fn new(addr: u64) -> PhysAddr { PhysAddr(addr) }
                pub fn as_u64(self) -> u64 { self.0 }
                pub fn frame_index(self) -> u64 { self.as_u64() / FRAME_SIZE }
            }

            pub struct FrameAllocator {
                pub bitmap: [u8; 32],
                pub total_frames: u64,
                pub free_frames: u64,
            }

            impl FrameAllocator {
                pub fn new() -> FrameAllocator {
                    FrameAllocator {
                        bitmap: [0u8; 32],
                        total_frames: 256,
                        free_frames: 256,
                    }
                }

                pub fn set_used(&mut self, frame: u64) {
                    let idx = (frame / 8) as i32;
                    let bit = (frame % 8) as u8;
                    let old = self.bitmap[idx];
                    self.bitmap[idx] = old | (1 << bit);
                    self.free_frames -= 1;
                }

                pub fn set_free(&mut self, frame: u64) {
                    let idx = (frame / 8) as i32;
                    let bit = (frame % 8) as u8;
                    let old = self.bitmap[idx];
                    self.bitmap[idx] = old & !(1 << bit);
                    self.free_frames += 1;
                }

                pub fn is_used(&self, frame: u64) -> bool {
                    let idx = (frame / 8) as i32;
                    let bit = (frame % 8) as u8;
                    self.bitmap[idx] & (1 << bit) != 0
                }

                pub fn alloc_frame(&mut self) -> u64 {
                    let mut frame: u64 = 0;
                    while frame < self.total_frames {
                        if !self.is_used(frame) {
                            self.set_used(frame);
                            return frame;
                        }
                        frame += 1;
                    }
                    0xFFFFFFFFFFFFFFFF
                }
            }
        }

        fn main() -> i32 {
            let mut alloc = memory::FrameAllocator {
                bitmap: [0u8; 32],
                total_frames: 256,
                free_frames: 256,
            };
            alloc.set_used(5);
            let f = alloc.alloc_frame();
            if f == 0 { 1 } else { 0 }
        }
    "#,
        1,
    );
}
