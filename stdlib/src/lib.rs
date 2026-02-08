#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

mod raw;

pub mod dll;
pub mod fs;
pub mod heap;
pub mod io;
pub mod ipc;
pub mod net;
pub mod process;
pub mod sys;
pub mod ui;

// Re-export alloc types for user convenience
pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};

/// Entry point macro for .anyOS user programs.
///
/// Generates `_start` entry point and `extern crate alloc`.
/// The stdlib provides `#[panic_handler]` and `#[global_allocator]`.
///
/// Usage:
/// ```ignore
/// #![no_std]
/// #![no_main]
/// anyos_std::entry!(main);
/// fn main() { ... }
/// ```
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        extern crate alloc;

        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            $crate::heap::init();
            $main();
            $crate::process::exit(0);
        }
    };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    io::_print_panic(info);
    process::exit(1);
}

#[alloc_error_handler]
fn alloc_error(_layout: core::alloc::Layout) -> ! {
    io::_print_str("ALLOC ERROR: out of memory\n");
    process::exit(2);
}
