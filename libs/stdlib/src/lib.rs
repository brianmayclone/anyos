#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), feature(alloc_error_handler))]

extern crate alloc;

// ── Platform-specific modules ────────────────────────────────────────────────

#[cfg(not(feature = "host"))]
mod raw;

// Modules with host-mode alternatives (use #[path] to swap implementations)
#[cfg(not(feature = "host"))]
pub mod fs;
#[cfg(feature = "host")]
#[path = "host/fs.rs"]
pub mod fs;

#[cfg(not(feature = "host"))]
pub mod heap;
#[cfg(feature = "host")]
#[path = "host/heap.rs"]
pub mod heap;

#[cfg(not(feature = "host"))]
pub mod io;
#[cfg(feature = "host")]
#[path = "host/io.rs"]
pub mod io;

#[cfg(not(feature = "host"))]
pub mod ipc;
#[cfg(feature = "host")]
#[path = "host/ipc.rs"]
pub mod ipc;

#[cfg(not(feature = "host"))]
pub mod process;
#[cfg(feature = "host")]
#[path = "host/process.rs"]
pub mod process;

#[cfg(not(feature = "host"))]
pub mod net;
#[cfg(feature = "host")]
#[path = "host/net.rs"]
pub mod net;

#[cfg(not(feature = "host"))]
pub mod sys;
#[cfg(feature = "host")]
#[path = "host/sys.rs"]
pub mod sys;

#[cfg(not(feature = "host"))]
pub mod i18n;
#[cfg(feature = "host")]
#[path = "host/i18n.rs"]
pub mod i18n;

#[cfg(not(feature = "host"))]
pub mod env;
#[cfg(feature = "host")]
#[path = "host/env.rs"]
pub mod env;

// ── Platform-independent modules ─────────────────────────────────────────────

pub mod args;
pub mod collections;
pub mod error;
pub mod fmt;
pub mod hashmap;
pub mod json;
pub mod path;
pub mod xml;

// ── anyOS-only modules (not needed on host) ──────────────────────────────────

#[cfg(not(feature = "host"))]
pub mod anim;
#[cfg(not(feature = "host"))]
pub mod audio;
#[cfg(not(feature = "host"))]
pub mod bundle;
#[cfg(not(feature = "host"))]
pub mod crypto;
#[cfg(not(feature = "host"))]
pub mod debug;
#[cfg(not(feature = "host"))]
pub mod dll;
#[cfg(not(feature = "host"))]
pub mod icons;
#[cfg(not(feature = "host"))]
pub mod kbd;
#[cfg(not(feature = "host"))]
pub mod log;
#[cfg(not(feature = "host"))]
pub mod permissions;
#[cfg(not(feature = "host"))]
pub mod prelude;
#[cfg(not(feature = "host"))]
pub mod shell;
#[cfg(not(feature = "host"))]
pub mod ui;
#[cfg(not(feature = "host"))]
pub mod users;

// ── Global app state macro ──────────────────────────────────────────────────

/// Declare a global `static mut APP: Option<T>` with a type-safe accessor.
///
/// Generates:
/// - `static mut APP: Option<$ty> = None;`
/// - `fn app() -> &'static mut $ty` (panics if uninitialized)
///
/// # Usage
/// ```ignore
/// anyos_std::global_app_state!(AppState);
///
/// fn main() {
///     unsafe { APP = Some(AppState { ... }); }
///     app().do_something();
/// }
/// ```
#[macro_export]
macro_rules! global_app_state {
    ($ty:ty) => {
        static mut APP: Option<$ty> = None;

        #[inline(always)]
        fn app() -> &'static mut $ty {
            unsafe {
                APP.as_mut()
                    .expect(concat!(stringify!($ty), " not initialized"))
            }
        }
    };
}

// Re-export alloc types for user convenience
pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};
pub use collections::HashSet;
pub use hashmap::HashMap;

/// Trait for main function return types (() or u32 exit code).
pub trait MainReturn {
    fn to_exit_code(self) -> u32;
}

impl MainReturn for () {
    fn to_exit_code(self) -> u32 {
        0
    }
}

impl MainReturn for u32 {
    fn to_exit_code(self) -> u32 {
        self
    }
}

impl MainReturn for error::Result<()> {
    fn to_exit_code(self) -> u32 {
        match self {
            Ok(()) => 0,
            Err(e) => {
                io::_print_str("Error: ");
                let _ = core::fmt::Write::write_fmt(&mut io::Stdout, format_args!("{}\n", e));
                1
            }
        }
    }
}

// ── Print macros ─────────────────────────────────────────────────────────────

/// Print formatted text to stdout (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::io::_print(format_args!($($arg)*)));
}

/// Print formatted text to stdout with a trailing newline.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// Entry point macro for anyOS user programs.
///
/// On anyOS: generates `_start` with heap init and panic/alloc error handlers.
/// On host: generates a normal `fn main()`.
#[cfg(not(feature = "host"))]
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

#[cfg(feature = "host")]
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        fn main() {
            let code = $crate::MainReturn::to_exit_code($main());
            if code != 0 {
                std::process::exit(code as i32);
            }
        }
    };
}

// ── Panic/alloc handlers (anyOS only) ────────────────────────────────────────

#[cfg(not(feature = "host"))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    io::_print_panic(info);
    process::exit(1);
}

#[cfg(not(feature = "host"))]
#[alloc_error_handler]
fn alloc_error(_layout: core::alloc::Layout) -> ! {
    io::_print_str("ALLOC ERROR: out of memory\n");
    process::exit(2);
}

#[cfg(all(test, feature = "host"))]
mod tests {
    #[test]
    fn host_smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
