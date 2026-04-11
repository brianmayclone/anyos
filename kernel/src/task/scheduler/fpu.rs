//! Lazy FPU/SSE/AVX switching via CR0.TS (#NM handler).

use super::{get_cpu_id, PER_CPU_CURRENT_TID, PER_CPU_FPU_OWNER, PER_CPU_FPU_PTR};
use core::sync::atomic::Ordering;

/// Save the live user FP/SIMD state when entering the kernel from EL0 on ARM64.
///
/// ARM64 exceptions do not automatically preserve NEON/FP registers. If a user
/// thread owns the live FP state and enters EL1 via SVC/IRQ, later kernel work
/// can otherwise observe or clobber those registers before the next lazy
/// restore. We snapshot the live state here and re-arm trapping so the thread
/// reloads its FP state lazily on first use after returning to EL0.
#[cfg(target_arch = "aarch64")]
pub fn save_live_user_fpu_on_exception_entry() {
    let cpu_id = get_cpu_id();
    let current_tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);

    if current_tid != 0 && fpu_owner == current_tid {
        let fpu_ptr = PER_CPU_FPU_PTR[cpu_id].load(Ordering::Relaxed);
        if fpu_ptr != 0 {
            crate::arch::hal::fpu_save(fpu_ptr as *mut u8);
        }
        PER_CPU_FPU_OWNER[cpu_id].store(0, Ordering::Relaxed);
    }

    crate::arch::hal::fpu_set_trap();
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
pub fn save_live_user_fpu_on_exception_entry() {}

/// Ensure the current ARM64 user thread returns to EL0 with usable FP/SIMD state.
///
/// User code may execute FP/NEON instructions immediately in function epilogues
/// (for example restoring d8-d15) right after `eret`. Returning with CPACR
/// still trapping FP would fault before the lazy handler ever has a chance to
/// rebuild a normal userspace execution context, so we restore here.
#[cfg(target_arch = "aarch64")]
pub fn prepare_user_fpu_return() {
    let cpu_id = get_cpu_id();
    let current_tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    if current_tid == 0 {
        crate::arch::hal::fpu_clear_trap();
        return;
    }

    let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);
    if fpu_owner != current_tid {
        let fpu_ptr = PER_CPU_FPU_PTR[cpu_id].load(Ordering::Relaxed);
        if fpu_ptr != 0 {
            crate::arch::hal::fpu_restore(fpu_ptr as *const u8);
            PER_CPU_FPU_OWNER[cpu_id].store(current_tid, Ordering::Relaxed);
        }
    }

    crate::arch::hal::fpu_clear_trap();
}

#[cfg(not(target_arch = "aarch64"))]
#[inline]
pub fn prepare_user_fpu_return() {}

/// Handle Device Not Available exception (#NM, ISR 7).
/// Called when a thread executes an FPU/SSE instruction with CR0.TS set.
/// Loads the thread's FPU state and clears TS so the instruction can retry.
pub fn handle_device_not_available() {
    let cpu_id = get_cpu_id();
    let current_tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    let fpu_owner = PER_CPU_FPU_OWNER[cpu_id].load(Ordering::Relaxed);

    // If this thread's state is already loaded, just clear the trap
    if fpu_owner == current_tid && current_tid != 0 {
        crate::arch::hal::fpu_clear_trap();
        return;
    }

    // Clear trap first — restore also traps with trap set
    crate::arch::hal::fpu_clear_trap();

    // Load this thread's FPU/SSE/AVX state
    let fpu_ptr = PER_CPU_FPU_PTR[cpu_id].load(Ordering::Relaxed);
    if fpu_ptr != 0 {
        crate::arch::hal::fpu_restore(fpu_ptr as *const u8);
        PER_CPU_FPU_OWNER[cpu_id].store(current_tid, Ordering::Relaxed);
    }
}
