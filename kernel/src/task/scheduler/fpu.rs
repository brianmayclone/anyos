//! Lazy FPU/SSE/AVX switching via CR0.TS (#NM handler).

use super::{get_cpu_id, PER_CPU_CURRENT_TID, PER_CPU_FPU_OWNER, PER_CPU_FPU_PTR};
use core::sync::atomic::Ordering;

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
