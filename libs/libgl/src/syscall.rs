//! Kernel syscall wrappers for libgl.
//!
//! Minimal syscall set: sbrk for heap allocation, exit for panic handler.

use core::arch::asm;
use core::fmt::{self, Write};

const SYS_EXIT: u64 = 1;
const SYS_WRITE: u64 = 4;
const SYS_SBRK: u64 = 9;

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
            "mov rdi, {a2}",
            "mov rsi, {a3}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            a2 = in(reg) a2,
            a3 = in(reg) a3,
            inlateout("rax") num => ret,
            out("rcx") _,
            out("r11") _,
            out("rdi") _,
            out("rsi") _,
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
