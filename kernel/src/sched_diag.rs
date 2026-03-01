//! SCHEDULER lock phase tracker for SPIN TIMEOUT diagnostics.
//!
//! Each CPU records a phase ID immediately before acquiring the SCHEDULER
//! lock.  The lock-free SPIN TIMEOUT handler in `sync/spinlock.rs` reads
//! the owner CPU's phase via [`get`] to identify which scheduler function
//! is holding the lock when another CPU times out.
//!
//! Phases are stored as bare atomics — no locking, safe from an NMI-like
//! context such as the SPIN TIMEOUT path.

use core::sync::atomic::{AtomicU32, Ordering};
use crate::arch::hal::MAX_CPUS;

// ── Phase ID constants ────────────────────────────────────────────────────

pub const PHASE_IDLE:               u32 = 0;
pub const PHASE_SPAWN:              u32 = 1;
pub const PHASE_SPAWN_BLOCKED:      u32 = 2;
pub const PHASE_SCHEDULE_TIMER:     u32 = 3;
pub const PHASE_SCHEDULE_VOLUNTARY: u32 = 4;
pub const PHASE_EXIT_CURRENT:       u32 = 5;
pub const PHASE_TRY_EXIT_CURRENT:   u32 = 6;
pub const PHASE_KILL_THREAD:        u32 = 7;
pub const PHASE_WAITPID:            u32 = 8;
pub const PHASE_WAITPID_ANY:        u32 = 9;
pub const PHASE_TRY_WAITPID:        u32 = 10;
pub const PHASE_TRY_WAITPID_ANY:    u32 = 11;
pub const PHASE_SLEEP_UNTIL:        u32 = 12;
pub const PHASE_BLOCK_CURRENT:      u32 = 13;
pub const PHASE_WAKE_THREAD:        u32 = 14;
pub const PHASE_SET_THREAD_ARGS:    u32 = 15;
pub const PHASE_SET_THREAD_CWD:     u32 = 16;
pub const PHASE_SET_THREAD_PIPE:    u32 = 17;
pub const PHASE_GET_THREAD_INFO:    u32 = 18;
pub const PHASE_SET_THREAD_PRIORITY:u32 = 19;
pub const PHASE_CREATE_THREAD:      u32 = 20;
pub const PHASE_HAS_LIVE_PD_SIBS:   u32 = 21;
pub const PHASE_CURRENT_EXIT_INFO:  u32 = 22;
pub const PHASE_SET_THREAD_BRK:     u32 = 23;
pub const PHASE_SET_THREAD_MMAP:    u32 = 24;
pub const PHASE_DEFERRED_PD:        u32 = 25;

// ── Per-CPU phase array ───────────────────────────────────────────────────

/// Per-CPU atomic storing the phase ID of the function that most recently
/// acquired (or is about to acquire) the SCHEDULER lock on that CPU.
static PHASE: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(PHASE_IDLE);
    [INIT; MAX_CPUS]
};

/// Set the phase for a given CPU.  Call immediately before `SCHEDULER.lock()`.
#[inline(always)]
pub fn set(cpu: usize, phase: u32) {
    if cpu < MAX_CPUS {
        PHASE[cpu].store(phase, Ordering::Relaxed);
    }
}

/// Read the current phase for any CPU.  Lock-free; safe from SPIN TIMEOUT.
#[inline(always)]
pub fn get(cpu: usize) -> u32 {
    if cpu < MAX_CPUS {
        PHASE[cpu].load(Ordering::Relaxed)
    } else {
        PHASE_IDLE
    }
}

/// Return a short ASCII name for a phase ID (as a `&[u8]` for lock-free printing).
pub fn name(phase: u32) -> &'static [u8] {
    match phase {
        PHASE_IDLE               => b"idle",
        PHASE_SPAWN              => b"spawn",
        PHASE_SPAWN_BLOCKED      => b"spawn_blocked",
        PHASE_SCHEDULE_TIMER     => b"schedule/timer",
        PHASE_SCHEDULE_VOLUNTARY => b"schedule/voluntary",
        PHASE_EXIT_CURRENT       => b"exit_current",
        PHASE_TRY_EXIT_CURRENT   => b"try_exit_current",
        PHASE_KILL_THREAD        => b"kill_thread",
        PHASE_WAITPID            => b"waitpid",
        PHASE_WAITPID_ANY        => b"waitpid_any",
        PHASE_TRY_WAITPID        => b"try_waitpid",
        PHASE_TRY_WAITPID_ANY    => b"try_waitpid_any",
        PHASE_SLEEP_UNTIL        => b"sleep_until",
        PHASE_BLOCK_CURRENT      => b"block_current",
        PHASE_WAKE_THREAD        => b"wake_thread",
        PHASE_SET_THREAD_ARGS    => b"set_thread_args",
        PHASE_SET_THREAD_CWD     => b"set_thread_cwd",
        PHASE_SET_THREAD_PIPE    => b"set_thread_pipe",
        PHASE_GET_THREAD_INFO    => b"get_thread_info",
        PHASE_SET_THREAD_PRIORITY=> b"set_thread_priority",
        PHASE_CREATE_THREAD      => b"create_thread",
        PHASE_HAS_LIVE_PD_SIBS  => b"has_live_pd_sibs",
        PHASE_CURRENT_EXIT_INFO  => b"current_exit_info",
        PHASE_SET_THREAD_BRK     => b"set_thread_brk",
        PHASE_SET_THREAD_MMAP    => b"set_thread_mmap",
        PHASE_DEFERRED_PD        => b"deferred_pd_destroy",
        _                        => b"?",
    }
}
