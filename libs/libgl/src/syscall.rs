//! Syscall wrappers for libgl — delegates to libsyscall.

pub use libsyscall::{
    dll_load, exit, gpu_3d_has_hw, gpu_3d_hw_version, gpu_3d_submit, gpu_3d_surface_dma,
    gpu_3d_surface_dma_read, gpu_3d_sync, gpu_query_type, mmap, munmap, sbrk, serial_print,
    syscall0, syscall1, syscall2, syscall3, syscall5, uptime_ms, write_bytes,
};

const SYS_GETPID: u32 = 6;
const SYS_KILL: u32 = 13;
const SYS_SYSINFO: u32 = 32;
const SYS_YIELD: u32 = 7;
const SYS_SLEEP: u32 = 8;
const SYS_SLEEP_US: u32 = 36;
const SYS_THREAD_CREATE: u32 = 170;

/// Get the number of online CPUs.
pub fn cpu_count() -> u32 {
    // sysinfo(cmd=2) returns cpu count directly
    let ret = libsyscall::syscall3(SYS_SYSINFO, 2, 0, 0);
    if (ret as i64) < 0 || ret == u64::MAX || ret == u32::MAX as u64 {
        0
    } else {
        ret as u32
    }
}

/// Sleep for `us` microseconds (non-busy for >= 1ms).
pub fn sleep_us(us: u32) {
    syscall1(SYS_SLEEP_US, us as u64);
}

pub fn sleep_ms(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

pub fn getpid() -> u32 {
    syscall0(SYS_GETPID) as u32
}

pub fn kill(tid: u32, sig: u32) -> u32 {
    let ret = syscall2(SYS_KILL, tid as u64, sig as u64);
    if (ret as i64) < 0 || ret == u64::MAX || ret == u32::MAX as u64 {
        u32::MAX
    } else {
        ret as u32
    }
}

/// Create a new thread in the current process.
/// Returns TID (>0) on success, 0 on error.
pub fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32 {
    let ret = syscall5(
        SYS_THREAD_CREATE,
        entry as u64,
        stack_top as u64,
        name.as_ptr() as u64,
        name.len() as u64,
        0, // priority 0 = inherit
    );
    if (ret as i64) < 0 || ret == u64::MAX || ret == u32::MAX as u64 {
        0
    } else {
        ret as u32
    }
}

pub fn _serial_print(args: core::fmt::Arguments) {
    serial_print(args);
}

/// Print to serial output (same as anyos_std::println but for libgl .so).
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        $crate::syscall::_serial_print(format_args!($($arg)*));
        $crate::syscall::write_bytes(b"\n");
    }};
}
