#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![allow(dead_code)]
//! Crate root for the anyOS kernel.
//!
//! The actual staged boot flow lives in [`boot`], which keeps the crate root
//! intentionally thin and makes the initialization sequence easier to review.

extern crate alloc;

mod arch;
mod boot;
mod boot_info;
mod crypto;
mod drivers;
mod fs;
mod graphics;
mod ipc;
#[cfg(feature = "kunit")]
mod kunit;
mod memory;
mod net;
mod panic;
pub mod sched_diag;
mod sync;
mod syscall;
mod task;

pub use boot::{boot_mode, GPU_ACCEL, GPU_HW_CURSOR, NOGUI, SETUP_MODE};

/// Kernel entry point called from assembly after boot.
///
/// Receives the physical address of the [`BootInfo`] struct.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: u64) -> ! {
    boot::kernel_main(boot_info_addr)
}
