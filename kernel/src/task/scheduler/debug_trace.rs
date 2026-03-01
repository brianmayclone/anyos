//! Debug / Trace API (anyTrace) — debugger attach, breakpoints, single-step,
//! memory read/write via CR3-switch, page table walking.

use super::{SCHEDULER, PER_CPU_CURRENT_TID};
use crate::task::context::CpuContext;
use crate::task::thread::ThreadState;
use crate::arch::hal::MAX_CPUS;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

// =============================================================================
// Debug / Trace API (anyTrace)
// =============================================================================

/// Debug event types — must match userspace constants.
pub const DEBUG_EVENT_BREAKPOINT: u32 = 1;
pub const DEBUG_EVENT_SINGLE_STEP: u32 = 2;
pub const DEBUG_EVENT_EXIT: u32 = 3;

/// Attach `debugger_tid` to `target_tid`. The target is suspended (Blocked).
///
/// Rejects: self-attach, kernel/idle threads, already-attached threads.
/// Returns 0 on success, u32::MAX on error.
pub fn debug_attach(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    let thread = &sched.threads[idx];

    // Reject kernel threads, idle threads, terminated threads, already-attached
    if !thread.is_user || thread.is_idle || thread.state == ThreadState::Terminated {
        return u32::MAX;
    }
    if thread.debug_attached_by != 0 {
        return u32::MAX; // Already attached by another debugger
    }

    // Set debug attachment
    sched.threads[idx].debug_attached_by = debugger_tid;
    sched.threads[idx].debug_suspended = true;

    // If thread is Ready, remove from run queue and set Blocked
    if sched.threads[idx].state == ThreadState::Ready {
        // Remove from the run queue on its affinity CPU
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.remove(target_tid);
        sched.threads[idx].state = ThreadState::Blocked;
    } else if sched.threads[idx].state == ThreadState::Running {
        // Thread is currently running on some CPU — mark for suspend.
        // It will be blocked on next schedule tick.
        sched.threads[idx].state = ThreadState::Blocked;
        sched.threads[idx].context.save_complete = 0;
    }
    // If already Blocked (e.g., sleeping), just keep it blocked

    0
}

/// Detach `debugger_tid` from `target_tid`. Removes all breakpoints,
/// clears TF, and resumes the thread.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_detach(debugger_tid: u32, target_tid: u32) -> u32 {
    // First, collect breakpoint info under lock, then do CR3-switch outside lock
    let (bp_count, breakpoints, target_cr3);
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        // Save breakpoint info for removal
        bp_count = sched.threads[idx].debug_sw_bp_count;
        breakpoints = sched.threads[idx].debug_sw_breakpoints;
        target_cr3 = sched.threads[idx].context.get_page_table();

        // Clear TF from flags
        let flags = sched.threads[idx].context.get_flags() & !0x100;
        sched.threads[idx].context.set_flags(flags);
        // Recompute checksum after modifying flags
        sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();

        // Clear all debug state
        sched.threads[idx].debug_attached_by = 0;
        sched.threads[idx].debug_suspended = false;
        sched.threads[idx].debug_single_step = false;
        sched.threads[idx].debug_event = None;
        sched.threads[idx].debug_sw_bp_count = 0;
        sched.threads[idx].debug_sw_breakpoints = [(0, 0); 16];

        // Resume thread if it's blocked
        if sched.threads[idx].state == ThreadState::Blocked {
            sched.threads[idx].state = ThreadState::Ready;
            let cpu = sched.threads[idx].affinity_cpu;
            let n = sched.num_cpus();
            let target_cpu = if cpu < n { cpu } else { 0 };
            sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
        }
    }

    // Restore original bytes at breakpoint locations (outside scheduler lock)
    if bp_count > 0 && target_cr3 != 0 {
        restore_breakpoint_bytes(target_cr3, &breakpoints[..bp_count as usize]);
    }

    0
}

/// Suspend a debug-attached thread.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_suspend(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }

    sched.threads[idx].debug_suspended = true;

    if sched.threads[idx].state == ThreadState::Ready {
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.remove(target_tid);
        sched.threads[idx].state = ThreadState::Blocked;
    } else if sched.threads[idx].state == ThreadState::Running {
        sched.threads[idx].state = ThreadState::Blocked;
        sched.threads[idx].context.save_complete = 0;
    }

    0
}

/// Resume a suspended debug-attached thread.
/// If single_step is pending, sets RFLAGS.TF before resuming.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_resume(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX; // Not suspended
    }

    // If single-step is pending, set TF in flags (x86 RFLAGS / ARM64 PSTATE)
    if sched.threads[idx].debug_single_step {
        let flags = sched.threads[idx].context.get_flags() | 0x100; // TF bit
        sched.threads[idx].context.set_flags(flags);
        sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();
    }

    sched.threads[idx].debug_suspended = false;

    // Wake thread if it's blocked due to debug suspension
    if sched.threads[idx].state == ThreadState::Blocked {
        sched.threads[idx].state = ThreadState::Ready;
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
    }

    0
}

/// Read the target thread's user-space register state into a user buffer.
///
/// The thread's `context` field stores the kernel-internal context (from
/// context_switch), whose RIP/RSP point into kernel code. The actual user
/// registers live on the thread's kernel stack in the SyscallRegs / ISR frame
/// pushed at kernel entry. This function extracts them.
///
/// Kernel stack layout from top (both SYSCALL and ISR paths):
///   kernel_stack_top - 8:  SS
///   kernel_stack_top - 16: user RSP
///   kernel_stack_top - 24: RFLAGS
///   kernel_stack_top - 32: CS
///   kernel_stack_top - 40: user RIP   (IRET frame — same position for both paths)
///   ... then GPRs + optional int_num/error_code
///
/// For SYSCALL path (SyscallRegs, 160 bytes total):
///   GPRs at kernel_stack_top - 160 .. kernel_stack_top - 48
///   Order (low→high): r15,r14,r13,r12,r11,r10,r9,r8,rbp,rdi,rsi,rdx,rcx,rbx,rax
///
/// For ISR path (InterruptFrame, 176 bytes total):
///   GPRs at kernel_stack_top - 176 .. kernel_stack_top - 64
///   + int_num at kernel_stack_top - 56, error_code at kernel_stack_top - 48
///   Same GPR order as SYSCALL.
///
/// Detection: Read kernel_stack_top - 56. If 0..255 and kernel_stack_top - 48
///   is 0 or a valid error code, assume ISR path. Otherwise SYSCALL path.
///
/// Returns number of bytes copied, or u32::MAX on error.
pub fn debug_get_regs(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let guard = SCHEDULER.lock();
    let sched = match guard.as_ref() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX; // Must be suspended to read registers
    }

    let thread = &sched.threads[idx];
    let ktop = thread.kernel_stack_top();
    let cr3 = thread.context.get_page_table();

    // Read IRET frame (always at fixed position from kernel_stack_top)
    let user_rip    = unsafe { *((ktop - 40) as *const u64) };
    let user_cs     = unsafe { *((ktop - 32) as *const u64) };
    let user_rflags = unsafe { *((ktop - 24) as *const u64) };
    let user_rsp    = unsafe { *((ktop - 16) as *const u64) };

    // Verify this looks like a user-mode IRET frame (CS has RPL=3)
    let is_user_frame = (user_cs & 3) == 3 && user_rip < 0x0000_8000_0000_0000;

    if !is_user_frame {
        // Fallback: copy kernel context as-is (thread may not have entered from user mode)
        let ctx = &thread.context;
        let ctx_ptr = ctx as *const CpuContext as *const u8;
        let copy_len = (size as usize).min(160);
        unsafe {
            let dst = buf_ptr as *mut u8;
            core::ptr::copy_nonoverlapping(ctx_ptr, dst, copy_len);
        }
        return copy_len as u32;
    }

    // Detect SYSCALL vs ISR path by checking the int_num/error_code slots
    // ISR path has int_num at ktop-56 (0..255) and error_code at ktop-48
    // SYSCALL path has rbx at ktop-56 and rax at ktop-48
    let val_at_56 = unsafe { *((ktop - 56) as *const u64) };
    let val_at_48 = unsafe { *((ktop - 48) as *const u64) };
    // Heuristic: if val_at_56 is a valid interrupt number (0..255) and
    // val_at_48 is 0 or a small error code, assume ISR path
    let is_isr_path = val_at_56 <= 255 && val_at_48 <= 0x1F;
    let gpr_base = if is_isr_path { ktop - 176 } else { ktop - 160 };

    // Read GPRs from the frame (order: r15,r14,...,r8,rbp,rdi,rsi,rdx,rcx,rbx,rax)
    let r15 = unsafe { *((gpr_base +   0) as *const u64) };
    let r14 = unsafe { *((gpr_base +   8) as *const u64) };
    let r13 = unsafe { *((gpr_base +  16) as *const u64) };
    let r12 = unsafe { *((gpr_base +  24) as *const u64) };
    let r11 = unsafe { *((gpr_base +  32) as *const u64) };
    let r10 = unsafe { *((gpr_base +  40) as *const u64) };
    let r9  = unsafe { *((gpr_base +  48) as *const u64) };
    let r8  = unsafe { *((gpr_base +  56) as *const u64) };
    let rbp = unsafe { *((gpr_base +  64) as *const u64) };
    let rdi = unsafe { *((gpr_base +  72) as *const u64) };
    let rsi = unsafe { *((gpr_base +  80) as *const u64) };
    let rdx = unsafe { *((gpr_base +  88) as *const u64) };
    let rcx = unsafe { *((gpr_base +  96) as *const u64) };
    let rbx = unsafe { *((gpr_base + 104) as *const u64) };
    let rax = unsafe { *((gpr_base + 112) as *const u64) };

    // Build CpuContext-compatible buffer for the debugger
    // Layout: rax,rbx,rcx,rdx,rsi,rdi,rbp,r8,r9,r10,r11,r12,r13,r14,r15,rsp,rip,rflags,cr3,reserved
    let user_ctx: [u64; 20] = [
        rax, rbx, rcx, rdx, rsi, rdi, rbp,
        r8, r9, r10, r11, r12, r13, r14, r15,
        user_rsp, user_rip, user_rflags, cr3, 0,
    ];

    let copy_len = (size as usize).min(160);
    unsafe {
        let src = user_ctx.as_ptr() as *const u8;
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, copy_len);
    }

    copy_len as u32
}

/// Overwrite the target thread's CpuContext from a user buffer.
///
/// Validates that RIP is in user-space and masks dangerous RFLAGS bits.
/// Returns 0 on success, u32::MAX on error.
pub fn debug_set_regs(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX;
    }

    let copy_len = (size as usize).min(160);

    // Read new values from user buffer into a temporary context
    let mut new_ctx = sched.threads[idx].context;
    unsafe {
        let src = buf_ptr as *const u8;
        let dst = &mut new_ctx as *mut CpuContext as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, copy_len);
    }

    // Validate: PC must be in user-space (below kernel half)
    if new_ctx.get_pc() >= 0x0000_8000_0000_0000 {
        return u32::MAX;
    }
    // SP must be in user-space
    if new_ctx.get_sp() >= 0x0000_8000_0000_0000 {
        return u32::MAX;
    }

    // Mask dangerous flags bits — preserve only safe user flags
    // Keep: CF(0), PF(2), AF(4), ZF(6), SF(7), TF(8, for single-step), DF(10), OF(11), IF(9)
    // IOPL must stay 0 (ring-3), VM must stay 0, VIF/VIP must stay 0
    const SAFE_FLAGS: u64 = 0xCD5; // CF|PF|AF|ZF|SF|DF|OF
    const IF_FLAG: u64 = 0x200;
    new_ctx.set_flags((new_ctx.get_flags() & SAFE_FLAGS) | IF_FLAG); // Always keep IF=1 for user

    // Preserve page table base — debugger cannot change address space
    new_ctx.set_page_table(sched.threads[idx].context.get_page_table());

    // Recompute integrity fields
    new_ctx.canary = crate::task::context::CANARY_MAGIC;
    new_ctx.save_complete = 1;
    new_ctx.checksum = new_ctx.compute_checksum();

    sched.threads[idx].context = new_ctx;

    0
}

/// Read memory from the target thread's address space using CR3-switch.
///
/// Returns number of bytes read, or u32::MAX on error.
pub fn debug_read_mem(debugger_tid: u32, target_tid: u32, addr: u64, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.get_page_table();
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Perform CR3-switch read (outside scheduler lock to avoid contention)
    let bytes_read = cr3_switch_read(target_cr3, addr, buf_ptr, size);
    bytes_read
}

/// Write memory into the target thread's address space using CR3-switch.
///
/// Returns number of bytes written, or u32::MAX on error.
pub fn debug_write_mem(debugger_tid: u32, target_tid: u32, addr: u64, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.get_page_table();
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    let bytes_written = cr3_switch_write(target_cr3, addr, buf_ptr, size);
    bytes_written
}

/// Set a software breakpoint (INT3) at `addr` in the target's address space.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_set_breakpoint(debugger_tid: u32, target_tid: u32, addr: u64) -> u32 {
    let target_cr3;
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }
        if !sched.threads[idx].debug_suspended {
            return u32::MAX;
        }

        // Check if breakpoint already exists
        for i in 0..sched.threads[idx].debug_sw_bp_count as usize {
            if sched.threads[idx].debug_sw_breakpoints[i].0 == addr {
                return 0; // Already set
            }
        }

        // Check capacity
        if sched.threads[idx].debug_sw_bp_count >= 16 {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.get_page_table();
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Read original byte via CR3-switch
    let mut original_byte: u8 = 0;
    let read = cr3_switch_read(target_cr3, addr, &mut original_byte as *mut u8 as u64, 1);
    if read != 1 {
        return u32::MAX;
    }

    // Write INT3 (0xCC) via CR3-switch
    let int3: u8 = 0xCC;
    let written = cr3_switch_write(target_cr3, addr, &int3 as *const u8 as u64, 1);
    if written != 1 {
        return u32::MAX;
    }

    // Record breakpoint in thread struct
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };
        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };
        let bp_count = sched.threads[idx].debug_sw_bp_count as usize;
        sched.threads[idx].debug_sw_breakpoints[bp_count] = (addr, original_byte);
        sched.threads[idx].debug_sw_bp_count += 1;
    }

    0
}

/// Clear a software breakpoint, restoring the original byte.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_clr_breakpoint(debugger_tid: u32, target_tid: u32, addr: u64) -> u32 {
    let target_cr3;
    let original_byte;
    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        // Find the breakpoint
        let bp_count = sched.threads[idx].debug_sw_bp_count as usize;
        let bp_pos = (0..bp_count)
            .find(|&i| sched.threads[idx].debug_sw_breakpoints[i].0 == addr);

        let bp_pos = match bp_pos {
            Some(p) => p,
            None => return u32::MAX, // Breakpoint not found
        };

        original_byte = sched.threads[idx].debug_sw_breakpoints[bp_pos].1;
        target_cr3 = sched.threads[idx].context.get_page_table();

        // Remove from array by shifting
        for i in bp_pos..bp_count - 1 {
            sched.threads[idx].debug_sw_breakpoints[i] =
                sched.threads[idx].debug_sw_breakpoints[i + 1];
        }
        sched.threads[idx].debug_sw_breakpoints[bp_count - 1] = (0, 0);
        sched.threads[idx].debug_sw_bp_count -= 1;
    }

    // Restore original byte
    if target_cr3 != 0 {
        cr3_switch_write(target_cr3, addr, &original_byte as *const u8 as u64, 1);
    }

    0
}

/// Mark the target for single-step execution.
///
/// Returns 0 on success, u32::MAX on error.
pub fn debug_single_step(debugger_tid: u32, target_tid: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }
    if !sched.threads[idx].debug_suspended {
        return u32::MAX;
    }

    sched.threads[idx].debug_single_step = true;

    // Set TF in saved flags so it takes effect on resume
    let flags = sched.threads[idx].context.get_flags() | 0x100;
    sched.threads[idx].context.set_flags(flags);
    sched.threads[idx].context.checksum = sched.threads[idx].context.compute_checksum();

    // Resume the thread so it executes one instruction
    sched.threads[idx].debug_suspended = false;
    if sched.threads[idx].state == ThreadState::Blocked {
        sched.threads[idx].state = ThreadState::Ready;
        let cpu = sched.threads[idx].affinity_cpu;
        let n = sched.num_cpus();
        let target_cpu = if cpu < n { cpu } else { 0 };
        sched.per_cpu[target_cpu].run_queue.enqueue(target_tid, sched.threads[idx].priority);
    }

    0
}

/// Walk the target's page tables and return memory regions.
///
/// Each region is 24 bytes: (start: u64, end: u64, flags: u64).
/// Returns number of regions written.
pub fn debug_get_mem_map(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let target_cr3;
    {
        let guard = SCHEDULER.lock();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
            Some(i) => i,
            None => return u32::MAX,
        };

        if sched.threads[idx].debug_attached_by != debugger_tid {
            return u32::MAX;
        }

        target_cr3 = sched.threads[idx].context.get_page_table();
    }

    if target_cr3 == 0 {
        return u32::MAX;
    }

    // Walk page tables under CR3-switch
    let max_regions = (size as usize) / 24;
    let regions = cr3_switch_walk_pages(target_cr3, max_regions);

    // Copy results to user buffer
    let count = regions.len().min(max_regions);
    for i in 0..count {
        let (start, end, flags) = regions[i];
        let offset = (i * 24) as u64;
        unsafe {
            let dst = (buf_ptr + offset) as *mut u64;
            dst.write(start);
            dst.add(1).write(end);
            dst.add(2).write(flags);
        }
    }

    count as u32
}

/// Poll for a pending debug event on the target thread.
///
/// Returns event type (1=BP, 2=step, 3=exit), 0 if no event, u32::MAX on error.
pub fn debug_wait_event(debugger_tid: u32, target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let idx = match sched.threads.iter().position(|t| t.tid == target_tid) {
        Some(i) => i,
        None => return u32::MAX,
    };

    if sched.threads[idx].debug_attached_by != debugger_tid {
        return u32::MAX;
    }

    let event = sched.threads[idx].debug_event.take();
    match event {
        Some((event_type, addr)) => {
            // Write event data to user buffer if provided
            if buf_ptr != 0 && size >= 12 {
                unsafe {
                    let dst = buf_ptr as *mut u32;
                    dst.write(event_type);
                    let addr_dst = (buf_ptr + 4) as *mut u64;
                    addr_dst.write(addr);
                }
            }
            event_type
        }
        None => 0,
    }
}

/// Get extended thread information.
///
/// Layout (128 bytes):
///   0: parent_tid (u32)
///   4: state (u32)
///   8: priority (u32)
///  12: cpu_ticks (u32)
///  16: last_cpu (u32)
///  20: user_pages (u32)
///  24: brk (u32)
///  28: mmap_next (u32)
///  32: rip (u64)
///  40: rsp (u64)
///  48: cr3 (u64)
///  56: io_read_bytes (u64)
///  64: io_write_bytes (u64)
///  72: capabilities (u32)
///  76: uid (u16)
///  78: gid (u16)
///  80: debug_attached_by (u32)
///  84: name (32 bytes)
/// 116: arch_mode (u32)
/// 120: reserved (8 bytes)
///
/// Returns number of bytes written.
pub fn thread_info_ex(target_tid: u32, buf_ptr: u64, size: u32) -> u32 {
    let guard = SCHEDULER.lock();
    let sched = match guard.as_ref() {
        Some(s) => s,
        None => return u32::MAX,
    };

    let thread = match sched.threads.iter().find(|t| t.tid == target_tid) {
        Some(t) => t,
        None => return u32::MAX,
    };

    let write_len = (size as usize).min(128);
    let mut buf = [0u8; 128];

    // Pack fields into buffer
    let state_u32: u32 = match thread.state {
        ThreadState::Ready => 0,
        ThreadState::Running => 1,
        ThreadState::Blocked => 2,
        ThreadState::Terminated => 3,
    };

    // Helper to write u32 LE at offset
    fn put_u32(buf: &mut [u8], off: usize, val: u32) {
        if off + 4 <= buf.len() {
            buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }
    }
    fn put_u64(buf: &mut [u8], off: usize, val: u64) {
        if off + 8 <= buf.len() {
            buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
        }
    }
    fn put_u16(buf: &mut [u8], off: usize, val: u16) {
        if off + 2 <= buf.len() {
            buf[off..off + 2].copy_from_slice(&val.to_le_bytes());
        }
    }

    put_u32(&mut buf, 0, thread.parent_tid);
    put_u32(&mut buf, 4, state_u32);
    put_u32(&mut buf, 8, thread.priority as u32);
    put_u32(&mut buf, 12, thread.cpu_ticks);
    put_u32(&mut buf, 16, thread.last_cpu as u32);
    put_u32(&mut buf, 20, thread.user_pages);
    put_u32(&mut buf, 24, thread.brk);
    put_u32(&mut buf, 28, thread.mmap_next);
    // Extract user-space RIP/RSP from kernel stack IRET frame (kernel_stack_top - 40/16)
    // context.rip/rsp contain kernel-internal addresses from context_switch.
    let ktop = thread.kernel_stack_top();
    let user_rip = unsafe { *((ktop - 40) as *const u64) };
    let user_cs  = unsafe { *((ktop - 32) as *const u64) };
    let user_rsp = unsafe { *((ktop - 16) as *const u64) };
    // Use user-space values if the IRET frame looks valid (CS has RPL=3)
    let (rip_val, rsp_val) = if (user_cs & 3) == 3 && user_rip < 0x0000_8000_0000_0000 {
        (user_rip, user_rsp)
    } else {
        (thread.context.get_pc(), thread.context.get_sp())
    };
    put_u64(&mut buf, 32, rip_val);
    put_u64(&mut buf, 40, rsp_val);
    put_u64(&mut buf, 48, thread.context.get_page_table());
    put_u64(&mut buf, 56, thread.io_read_bytes);
    put_u64(&mut buf, 64, thread.io_write_bytes);
    put_u32(&mut buf, 72, thread.capabilities);
    put_u16(&mut buf, 76, thread.uid);
    put_u16(&mut buf, 78, thread.gid);
    put_u32(&mut buf, 80, thread.debug_attached_by);
    // Copy name (32 bytes at offset 84)
    let name_end = 84 + 32;
    if name_end <= buf.len() {
        buf[84..name_end].copy_from_slice(&thread.name);
    }
    let arch_mode_u32: u32 = match thread.arch_mode {
        crate::task::thread::ArchMode::Native64 => 0,
        crate::task::thread::ArchMode::Compat32 => 1,
    };
    put_u32(&mut buf, 116, arch_mode_u32);

    // Copy to user buffer
    unsafe {
        let dst = buf_ptr as *mut u8;
        core::ptr::copy_nonoverlapping(buf.as_ptr(), dst, write_len);
    }

    write_len as u32
}

/// Called from ISR 1 (#DB) or ISR 3 (#BP) when a debug-attached thread
/// hits a breakpoint or completes a single-step.
///
/// This is called from interrupt context — uses lock-free per-CPU TID lookup
/// and deferred wake for the debugger.
pub fn debug_auto_suspend(tid: u32, event_type: u32, addr: u64) {
    // Must use try_lock since we're in interrupt context
    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_mut() {
            if let Some(idx) = sched.threads.iter().position(|t| t.tid == tid) {
                if sched.threads[idx].debug_attached_by != 0 {
                    sched.threads[idx].debug_suspended = true;
                    sched.threads[idx].debug_event = Some((event_type, addr));
                    sched.threads[idx].debug_single_step = false;

                    // Clear TF so the thread doesn't immediately step again
                    let flags = sched.threads[idx].context.get_flags() & !0x100;
                    sched.threads[idx].context.set_flags(flags);
                    sched.threads[idx].context.checksum =
                        sched.threads[idx].context.compute_checksum();

                    // Block the thread — it will be suspended until debugger resumes it
                    if sched.threads[idx].state == ThreadState::Running {
                        sched.threads[idx].context.save_complete = 0;
                        sched.threads[idx].state = ThreadState::Blocked;
                    }
                }
            }
        }
    }
    // If lock failed, the thread will continue running (no debug event recorded).
    // This is acceptable — the debugger will retry.
}

/// Check if the current thread on this CPU is debug-attached.
/// Lock-free: reads per-CPU TID atomically.
pub fn is_debug_attached_current() -> bool {
    let cpu_id = crate::arch::hal::cpu_id();
    let tid = if cpu_id < MAX_CPUS {
        PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed)
    } else {
        return false;
    };
    if tid == 0 {
        return false;
    }

    // Must use try_lock since this may be called from ISR context
    if let Some(guard) = SCHEDULER.try_lock() {
        if let Some(sched) = guard.as_ref() {
            if let Some(thread) = sched.threads.iter().find(|t| t.tid == tid) {
                return thread.debug_attached_by != 0;
            }
        }
    }
    false
}

// ---- CR3-switch helpers for cross-process memory access ----

/// Read `size` bytes from another process's address space.
/// Uses the cli -> page table switch -> copy -> restore pattern.
///
/// Returns number of bytes actually read.
fn cr3_switch_read(target_cr3: u64, src_addr: u64, dst_addr: u64, size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    let len = size.min(4096) as usize;

    // Use kernel stack buffer as intermediate: after switching to the target's
    // page table the debugger's user-space pages are no longer mapped, so we
    // cannot write directly into dst_addr.  Kernel stack addresses (higher-half)
    // are accessible regardless of which page table is active.
    let mut tmp = [0u8; 4096];
    let mut read = 0usize;

    #[cfg(target_arch = "x86_64")]
    unsafe {
        let rflags = crate::arch::hal::save_and_disable_interrupts();
        let old_cr3 = crate::arch::hal::current_page_table();
        crate::arch::hal::switch_page_table(target_cr3);

        // Read page-by-page, checking mappings via recursive page tables
        // to avoid #PF on unmapped addresses.
        let src = src_addr as *const u8;
        while read < len {
            let cur_addr = src_addr + read as u64;
            if !is_page_present_recursive(cur_addr) {
                break; // Stop at first unmapped page
            }
            // Read until end of this 4K page or end of requested range
            let page_end = (cur_addr & !0xFFF) + 0x1000;
            let chunk_end = core::cmp::min(page_end as usize, src_addr as usize + len);
            let chunk_len = chunk_end - cur_addr as usize;
            for _ in 0..chunk_len {
                tmp[read] = src.add(read).read_volatile();
                read += 1;
            }
        }

        crate::arch::hal::switch_page_table(old_cr3);
        crate::arch::hal::restore_interrupt_state(rflags);
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        let rflags = crate::arch::hal::save_and_disable_interrupts();
        let old_cr3 = crate::arch::hal::current_page_table();
        crate::arch::hal::switch_page_table(target_cr3);

        // On ARM64, directly copy from the target address space.
        // TODO: implement page-present check for ARM64 page tables.
        let src = src_addr as *const u8;
        while read < len {
            tmp[read] = src.add(read).read_volatile();
            read += 1;
        }

        crate::arch::hal::switch_page_table(old_cr3);
        crate::arch::hal::restore_interrupt_state(rflags);
    }

    // Copy from kernel buffer to debugger's user buffer (now back in
    // debugger's address space).
    if read > 0 {
        unsafe {
            let dst = dst_addr as *mut u8;
            for i in 0..read {
                dst.add(i).write(tmp[i]);
            }
        }
    }

    read as u32
}

/// Check if a virtual address has a present page mapping using the recursive
/// page table structure.  Must be called with the target CR3 already active
/// and interrupts disabled.
///
/// x86_64 only: uses recursive page table mapping at PML4[510].
#[cfg(target_arch = "x86_64")]
unsafe fn is_page_present_recursive(vaddr: u64) -> bool {
    use crate::memory::address::VirtAddr;
    let v = VirtAddr::new(vaddr);
    let ri = 510u64; // RECURSIVE_INDEX
    let pml4i = v.pml4_index() as u64;
    let pdpti = v.pdpt_index() as u64;
    let pdi = v.pd_index() as u64;

    // PML4 — recursive_pml4_base = ri<<39 | ri<<30 | ri<<21 | ri<<12
    let pml4_ptr = 0xFFFF_FF7F_BFDF_E000u64 as *const u64;
    let pml4e = pml4_ptr.add(v.pml4_index()).read_volatile();
    if pml4e & 1 == 0 { return false; }

    // PDPT — recursive_pdpt_base = ri<<39 | ri<<30 | ri<<21 | pml4i<<12
    let pdpt_ptr = sign_extend_addr(ri << 39 | ri << 30 | ri << 21 | pml4i << 12) as *const u64;
    let pdpte = pdpt_ptr.add(v.pdpt_index()).read_volatile();
    if pdpte & 1 == 0 { return false; }

    // PD — recursive_pd_base = ri<<39 | ri<<30 | pml4i<<21 | pdpti<<12
    let pd_ptr = sign_extend_addr(ri << 39 | ri << 30 | pml4i << 21 | pdpti << 12) as *const u64;
    let pde = pd_ptr.add(v.pd_index()).read_volatile();
    if pde & 1 == 0 { return false; }

    // Check for 2 MiB huge page (PS bit)
    if pde & (1 << 7) != 0 { return true; }

    // PT — recursive_pt_base = ri<<39 | pml4i<<30 | pdpti<<21 | pdi<<12
    let pt_ptr = sign_extend_addr(ri << 39 | pml4i << 30 | pdpti << 21 | pdi << 12) as *const u64;
    let pte = pt_ptr.add(v.pt_index()).read_volatile();
    pte & 1 != 0
}

/// Sign-extend a 48-bit virtual address to canonical 64-bit form.
#[cfg(target_arch = "x86_64")]
fn sign_extend_addr(addr: u64) -> u64 {
    if addr & (1u64 << 47) != 0 {
        addr | 0xFFFF_0000_0000_0000
    } else {
        addr & 0x0000_FFFF_FFFF_FFFF
    }
}

/// Write `size` bytes into another process's address space.
///
/// Uses a kernel stack buffer as intermediate because the source buffer
/// (in the debugger's address space) is not mapped under the target's page table.
fn cr3_switch_write(target_cr3: u64, dst_addr: u64, src_addr: u64, size: u32) -> u32 {
    if size == 0 {
        return 0;
    }
    let len = size.min(4096) as usize;

    // Copy source data into kernel stack buffer first (while still in
    // debugger's address space).
    let mut tmp = [0u8; 4096];
    unsafe {
        let src = src_addr as *const u8;
        for i in 0..len {
            tmp[i] = src.add(i).read();
        }
    }

    let mut written = 0usize;

    #[cfg(target_arch = "x86_64")]
    unsafe {
        let rflags = crate::arch::hal::save_and_disable_interrupts();
        let old_cr3 = crate::arch::hal::current_page_table();
        crate::arch::hal::switch_page_table(target_cr3);

        let dst = dst_addr as *mut u8;
        while written < len {
            let cur_addr = dst_addr + written as u64;
            if !is_page_present_recursive(cur_addr) {
                break;
            }
            let page_end = (cur_addr & !0xFFF) + 0x1000;
            let chunk_end = core::cmp::min(page_end as usize, dst_addr as usize + len);
            let chunk_len = chunk_end - cur_addr as usize;
            for _ in 0..chunk_len {
                dst.add(written).write_volatile(tmp[written]);
                written += 1;
            }
        }

        crate::arch::hal::switch_page_table(old_cr3);
        crate::arch::hal::restore_interrupt_state(rflags);
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        let rflags = crate::arch::hal::save_and_disable_interrupts();
        let old_cr3 = crate::arch::hal::current_page_table();
        crate::arch::hal::switch_page_table(target_cr3);

        // On ARM64, directly write to the target address space.
        // TODO: implement page-present check for ARM64 page tables.
        let dst = dst_addr as *mut u8;
        while written < len {
            dst.add(written).write_volatile(tmp[written]);
            written += 1;
        }

        crate::arch::hal::switch_page_table(old_cr3);
        crate::arch::hal::restore_interrupt_state(rflags);
    }

    written as u32
}

/// Restore original bytes at breakpoint locations via CR3-switch.
fn restore_breakpoint_bytes(target_cr3: u64, breakpoints: &[(u64, u8)]) {
    for &(addr, original_byte) in breakpoints {
        if addr != 0 {
            cr3_switch_write(target_cr3, addr, &original_byte as *const u8 as u64, 1);
        }
    }
}

/// Walk page tables under page-table-base switch to enumerate mapped memory regions.
///
/// Returns a Vec of (start_addr, end_addr, flags) tuples.
fn cr3_switch_walk_pages(target_cr3: u64, max_regions: usize) -> Vec<(u64, u64, u64)> {
    #[allow(unused_mut)]
    let mut regions: Vec<(u64, u64, u64)> = Vec::new();

    if max_regions == 0 {
        return regions;
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        let rflags = crate::arch::hal::save_and_disable_interrupts();
        let old_cr3 = crate::arch::hal::current_page_table();
        crate::arch::hal::switch_page_table(target_cr3);

        // Walk PML4 entries 0..255 (user-space half only)
        let pml4 = 0xFFFF_FF7F_BFDF_E000u64 as *const u64; // RECURSIVE_PML4_BASE

        let mut cur_start: u64 = 0;
        let mut cur_flags: u64 = 0;
        let mut cur_end: u64 = 0;
        let mut in_region = false;

        'outer: for pml4i in 0..256usize {
            let pml4e = pml4.add(pml4i).read_volatile();
            if pml4e & 1 == 0 { // PAGE_PRESENT
                if in_region {
                    regions.push((cur_start, cur_end, cur_flags));
                    in_region = false;
                    if regions.len() >= max_regions { break 'outer; }
                }
                continue;
            }

            // PDPT entries
            let pdpt_base = 0xFFFF_FF7F_BFC0_0000u64 + (pml4i as u64) * 0x1000;
            let pdpt = pdpt_base as *const u64;

            for pdpti in 0..512usize {
                let pdpte = pdpt.add(pdpti).read_volatile();
                if pdpte & 1 == 0 {
                    if in_region {
                        regions.push((cur_start, cur_end, cur_flags));
                        in_region = false;
                        if regions.len() >= max_regions { break 'outer; }
                    }
                    continue;
                }

                // PD entries
                let pd_base = 0xFFFF_FF7F_8000_0000u64
                    + (pml4i as u64) * 0x20_0000
                    + (pdpti as u64) * 0x1000;
                let pd = pd_base as *const u64;

                for pdi in 0..512usize {
                    let pde = pd.add(pdi).read_volatile();
                    if pde & 1 == 0 {
                        if in_region {
                            regions.push((cur_start, cur_end, cur_flags));
                            in_region = false;
                            if regions.len() >= max_regions { break 'outer; }
                        }
                        continue;
                    }

                    // Check for 2 MiB huge page (PS bit)
                    if pde & 0x80 != 0 {
                        let page_start = ((pml4i as u64) << 39)
                            | ((pdpti as u64) << 30)
                            | ((pdi as u64) << 21);
                        let page_end = page_start + 0x20_0000; // 2 MiB
                        let page_flags = pde & 0x8000_0000_0000_001F; // P|RW|US|PWT|PCD + NX

                        if in_region && page_flags == cur_flags && page_start == cur_end {
                            cur_end = page_end;
                        } else {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            cur_start = page_start;
                            cur_end = page_end;
                            cur_flags = page_flags;
                            in_region = true;
                        }
                        continue;
                    }

                    // PT entries
                    let pt_base = 0xFFFF_FF00_0000_0000u64
                        + (pml4i as u64) * 0x4000_0000
                        + (pdpti as u64) * 0x20_0000
                        + (pdi as u64) * 0x1000;
                    let pt = pt_base as *const u64;

                    for pti in 0..512usize {
                        let pte = pt.add(pti).read_volatile();
                        if pte & 1 == 0 {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                in_region = false;
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            continue;
                        }

                        let page_start = ((pml4i as u64) << 39)
                            | ((pdpti as u64) << 30)
                            | ((pdi as u64) << 21)
                            | ((pti as u64) << 12);
                        let page_end = page_start + 0x1000; // 4 KiB
                        let page_flags = pte & 0x8000_0000_0000_001F;

                        if in_region && page_flags == cur_flags && page_start == cur_end {
                            cur_end = page_end;
                        } else {
                            if in_region {
                                regions.push((cur_start, cur_end, cur_flags));
                                if regions.len() >= max_regions { break 'outer; }
                            }
                            cur_start = page_start;
                            cur_end = page_end;
                            cur_flags = page_flags;
                            in_region = true;
                        }
                    }
                }
            }
        }

        if in_region && regions.len() < max_regions {
            regions.push((cur_start, cur_end, cur_flags));
        }

        crate::arch::hal::switch_page_table(old_cr3);
        crate::arch::hal::restore_interrupt_state(rflags);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // TODO: implement ARM64 page table walking for debug memory map.
        // For now return empty regions.
        let _ = target_cr3;
    }

    regions
}
