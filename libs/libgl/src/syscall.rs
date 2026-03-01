//! Syscall wrappers for libgl — delegates to libsyscall.

pub use libsyscall::{
    sbrk, mmap, munmap, exit, write_bytes,
    gpu_3d_has_hw, gpu_3d_hw_version, gpu_3d_submit, gpu_3d_sync,
    gpu_3d_surface_dma, gpu_3d_surface_dma_read,
    serial_print,
};

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
