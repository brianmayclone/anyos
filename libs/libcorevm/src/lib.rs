//! libcorevm — Virtual machine library for anyOS.
//!
//! Provides device models, I/O dispatch, guest physical memory, and interrupt
//! management. The CPU execution backend is provided by hardware virtualization
//! via the `backend` module.
//!
//! # Architecture
//!
//! - **Backend** (`backend/`) — hardware virtualization backends (KVM, etc.)
//! - **Memory** (`memory/`) — guest RAM, MMIO dispatch
//! - **Devices** (`devices/`) — emulated hardware (SVGA, PS/2, E1000, etc.)
//! - **I/O** (`io.rs`) — port I/O dispatch
//! - **Interrupts** (`interrupts.rs`) — interrupt controller interface
//!
//! # C ABI
//!
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.
//! The new FFI layer will be added in a subsequent task.

#![cfg_attr(not(any(feature = "host_test", feature = "std")), no_std)]
#![cfg_attr(not(any(feature = "host_test", feature = "std")), no_main)]

extern crate alloc;
#[cfg(not(any(feature = "host_test", feature = "std")))]
extern crate libheap;

pub mod error;
pub mod flags;
pub mod registers;
pub mod instruction;
pub mod memory;
pub mod interrupts;
pub mod io;
pub mod devices;
pub mod backend;
pub mod vm;
pub mod ffi;

/// Syscall wrappers for the allocator, panic handler, debug output, and
/// file I/O (used by the IDE controller for on-demand disk access).
pub(crate) mod syscall {
    pub use libsyscall::{sbrk, mmap, munmap, exit, serial_print, write_bytes};
    pub use libsyscall::{open, read, write, lseek, close};
}

/// Print a formatted line to the serial console (stdout fd=1).
macro_rules! vm_log {
    ($($arg:tt)*) => {{
        #[cfg(not(feature = "host_test"))]
        {
            libsyscall::serial_print(format_args!("[corevm] "));
            libsyscall::serial_print(format_args!($($arg)*));
            libsyscall::write_bytes(b"\n");
        }
    }};
}

#[cfg(not(any(feature = "host_test", feature = "std")))]
libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

#[cfg(not(any(feature = "host_test", feature = "std")))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── Public re-exports ──

pub use error::{VmError, Result};
pub use memory::{GuestMemory, MemoryBus};
pub use memory::mmio::MmioHandler;
pub use memory::flat::FlatMemory;
pub use io::{IoDispatch, IoHandler};
pub use interrupts::InterruptController;
pub use registers::{RegisterFile, SegReg};
pub use flags::OperandSize;
