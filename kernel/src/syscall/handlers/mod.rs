//! Syscall handler implementations, organized by category.
//!
//! Each submodule groups related syscall handlers. All `pub fn sys_*` functions
//! are re-exported so that `super::handlers::sys_*` continues to resolve
//! unchanged from `syscall/mod.rs`.

mod debug;
mod device;
mod disk;
mod display;
mod filesystem;
mod helpers;
mod io;
mod ipc;
#[cfg(target_arch = "x86_64")]
mod monitor;
mod net;
mod platform;
mod process;
mod security;
mod signal;
mod system;

pub use debug::*;
pub use device::*;
pub use disk::*;
pub use display::*;
pub use filesystem::*;
pub use io::*;
pub use ipc::*;
#[cfg(target_arch = "x86_64")]
pub use monitor::*;
pub use net::*;
pub use platform::*;
pub use process::*;
pub use security::*;
pub use signal::*;
pub use system::*;

// =========================================================================
// Shared compositor state
// =========================================================================
// These statics are accessed from both `display` (registration, is_compositor
// guard) and `ipc` (wake_compositor_if_blocked). They live here so both
// submodules can reference them via `super::`.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// TID of the registered compositor process. 0 = none registered.
pub(crate) static COMPOSITOR_TID: AtomicU32 = AtomicU32::new(0);

/// Page directory (CR3) of the registered compositor. 0 = none.
/// Used to identify compositor child threads (render thread etc.)
/// that share the same address space.
pub(crate) static COMPOSITOR_PD: AtomicU64 = AtomicU64::new(0);

/// Check if the current thread belongs to the compositor process.
/// Returns true if the calling thread is the compositor's management thread
/// OR any child thread sharing the same page directory (e.g. render thread).
///
/// Lock-free: reads CR3 directly instead of acquiring the SCHEDULER lock.
/// This is critical because the render thread calls GPU commands at 60Hz
/// and each call checks is_compositor() — lock contention would be severe.
pub(super) fn is_compositor() -> bool {
    let comp_pd = COMPOSITOR_PD.load(Ordering::Relaxed);
    if comp_pd == 0 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        let current_cr3: u64;
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
        }
        // CR3 bits [12..] are the physical page directory address; mask off flags in low 12 bits
        return (current_cr3 & !0xFFF) == comp_pd;
    }
    #[cfg(target_arch = "aarch64")]
    {
        // On AArch64 use TTBR0_EL1 as the equivalent page-directory identifier
        let ttbr0: u64;
        unsafe {
            core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0);
        }
        return (ttbr0 & !0xFFF) == comp_pd;
    }
    #[allow(unreachable_code)]
    false
}
