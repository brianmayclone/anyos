#![no_std]
#![feature(alloc_error_handler)]

extern crate alloc;

mod raw;

pub mod args;
pub mod anim;
pub mod hashmap;
pub mod audio;
pub mod bundle;
pub mod crypto;
pub mod dll;
pub mod env;
pub mod error;
pub mod fs;
pub mod heap;
pub mod icons;
pub mod io;
pub mod ipc;
pub mod kbd;
pub mod net;
pub mod permissions;
pub mod prelude;
pub mod process;
pub mod sys;
pub mod ui;
pub mod users;

// Re-export alloc types for user convenience
pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};
pub use hashmap::HashMap;

/// Trait for main function return types (() or u32 exit code).
pub trait MainReturn {
    fn to_exit_code(self) -> u32;
}

impl MainReturn for () {
    fn to_exit_code(self) -> u32 { 0 }
}

impl MainReturn for u32 {
    fn to_exit_code(self) -> u32 { self }
}

impl MainReturn for error::Result<()> {
    fn to_exit_code(self) -> u32 {
        match self {
            Ok(()) => 0,
            Err(e) => {
                io::_print_str("Error: ");
                // Use Display impl via format_args
                let _ = core::fmt::Write::write_fmt(
                    &mut io::Stdout,
                    format_args!("{}\n", e),
                );
                1
            }
        }
    }
}

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
/// fn main() { ... }                              // returns exit code 0
/// fn main() -> u32 { 42 }                       // returns exit code 42
/// fn main() -> anyos_std::error::Result<()> { Ok(()) }  // Result support
/// ```
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        extern crate alloc;

        #[no_mangle]
        pub extern "C" fn _start() -> ! {
            $crate::heap::init();
            let code = $crate::MainReturn::to_exit_code($main());
            $crate::process::exit(code);
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
