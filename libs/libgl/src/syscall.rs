//! Kernel syscall wrappers for libgl.
//!
//! Minimal syscall set: sbrk for heap allocation, exit for panic handler.

use core::arch::asm;
use core::fmt::{self, Write};

const SYS_EXIT: u64 = 1;
const SYS_WRITE: u64 = 2;
const SYS_SBRK: u64 = 9;
const SYS_GPU_3D_SUBMIT: u64 = 512;
const SYS_GPU_3D_QUERY: u64 = 513;
const SYS_GPU_3D_SYNC: u64 = 514;

#[inline(always)]
fn syscall1(num: u64, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[inline(always)]
fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            in("r10") a2,
            in("rdx") a3,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

/// Write bytes to stdout (fd=1).
pub fn write_bytes(buf: &[u8]) {
    syscall3(SYS_WRITE, 1, buf.as_ptr() as u64, buf.len() as u64);
}

struct SerialWriter;
impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_bytes(s.as_bytes());
        Ok(())
    }
}

pub fn _serial_print(args: fmt::Arguments) {
    let _ = SerialWriter.write_fmt(args);
}

/// Print to serial output (same as anyos_std::println but for libgl .so).
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        $crate::syscall::_serial_print(format_args!($($arg)*));
        $crate::syscall::write_bytes(b"\n");
    }};
}

/// Exit the current process.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

/// Grow the heap. Returns new break address or u64::MAX on failure.
pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}

// ── GPU 3D Acceleration ──────────────────────────────

#[inline(always)]
fn syscall2(num: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            in("r10") a2,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

/// Query whether 3D hardware acceleration is available.
pub fn gpu_3d_has_hw() -> bool {
    syscall1(SYS_GPU_3D_QUERY, 0) != 0
}

/// Query the 3D hardware version.
pub fn gpu_3d_hw_version() -> u32 {
    syscall1(SYS_GPU_3D_QUERY, 1) as u32
}

/// Submit raw SVGA3D command words to the GPU. Returns 0 on success.
pub fn gpu_3d_submit(words: &[u32]) -> u32 {
    if words.is_empty() { return 0; }
    syscall2(SYS_GPU_3D_SUBMIT, words.as_ptr() as u64, words.len() as u64) as u32
}

/// Wait for all pending 3D commands to complete.
pub fn gpu_3d_sync() {
    syscall1(SYS_GPU_3D_SYNC, 0);
}

const SYS_GPU_3D_SURFACE_DMA: u64 = 515;

#[inline(always)]
fn syscall5(num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            in("r10") a2,
            in("rdx") a3,
            in("rsi") a4,
            in("rdi") a5,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

/// Upload data to a GPU surface via kernel-mediated DMA.
/// Returns 0 on success.
pub fn gpu_3d_surface_dma(sid: u32, data: &[u8], width: u32, height: u32) -> u32 {
    if data.is_empty() { return 0; }
    syscall5(
        SYS_GPU_3D_SURFACE_DMA,
        sid as u64,
        data.as_ptr() as u64,
        data.len() as u64,
        width as u64,
        height as u64,
    ) as u32
}
