//! Syscall handler implementations, organized by category.
//!
//! Each submodule groups related syscall handlers. All `pub fn sys_*` functions
//! are re-exported so that `super::handlers::sys_*` continues to resolve
//! unchanged from `syscall/mod.rs`.

mod helpers;
mod process;
mod io;
mod filesystem;
mod net;
mod ipc;
mod device;
mod display;
mod security;
mod signal;
mod system;
mod disk;
mod debug;

pub use process::*;
pub use io::*;
pub use filesystem::*;
pub use net::*;
pub use ipc::*;
pub use device::*;
pub use display::*;
pub use security::*;
pub use signal::*;
pub use system::*;
pub use disk::*;
pub use debug::*;

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
    let current_cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) current_cr3);
    }
    // CR3 bits [12..] are the physical page directory address; mask off flags in low 12 bits
    (current_cr3 & !0xFFF) == comp_pd
}
