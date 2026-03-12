//! Syscall wrappers for libgl — delegates to libsyscall.

pub use libsyscall::{
    sbrk, mmap, munmap, exit, write_bytes,
    gpu_3d_has_hw, gpu_3d_hw_version, gpu_3d_submit, gpu_3d_sync,
    gpu_3d_surface_dma, gpu_3d_surface_dma_read,
    serial_print,
    syscall1, syscall3, syscall5,
};

const SYS_SYSINFO: u32 = 32;
const SYS_SLEEP_US: u32 = 36;
const SYS_THREAD_CREATE: u32 = 170;

/// Get the number of online CPUs.
pub fn cpu_count() -> u32 {
    // sysinfo(cmd=2) returns cpu count directly
    libsyscall::syscall3(SYS_SYSINFO, 2, 0, 0) as u32
}

/// Sleep for `us` microseconds (non-busy for >= 1ms).
pub fn sleep_us(us: u32) {
    syscall1(SYS_SLEEP_US, us as u64);
}

/// Create a new thread in the current process.
/// Returns TID (>0) on success, 0 on error.
pub fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32 {
    syscall5(
        SYS_THREAD_CREATE,
        entry as u64,
        stack_top as u64,
        name.as_ptr() as u64,
        name.len() as u64,
        0, // priority 0 = inherit
    ) as u32
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
