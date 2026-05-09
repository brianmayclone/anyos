//! Thread lifecycle: exit, kill, fault recovery.

use super::deferred::{DeferredThreadCleanup, DEFERRED_PD_DESTROY, DEFERRED_THREAD_CLEANUP};
use super::{
    clear_per_cpu_name, close_all_fds_for_thread, force_unlock_scheduler, get_cpu_id,
    is_scheduler_locked_by_cpu, schedule, update_per_cpu_name, PER_CPU_CURRENT_TID,
    PER_CPU_FPU_OWNER, PER_CPU_FPU_PTR, PER_CPU_HAS_THREAD, PER_CPU_IDLE_STACK_TOP,
    PER_CPU_IS_USER, PER_CPU_STACK_BOTTOM, PER_CPU_STACK_TOP, SCHEDULER,
};
use crate::memory::address::PhysAddr;
use crate::task::context::CpuContext;
use crate::task::thread::ThreadState;
use core::sync::atomic::Ordering;

#[inline]
fn safe_cpu_id(cpu_id: usize) -> usize {
    if cpu_id < super::MAX_CPUS {
        cpu_id
    } else {
        crate::serial_verbose_println!(
            "  WARN: scheduler lifecycle saw invalid CPU{}; using CPU0 recovery path",
            cpu_id
        );
        0
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn try_exit_diag_putc(c: u8) {
    unsafe {
        while crate::arch::x86::port::inb(0x3FD) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        crate::arch::x86::port::outb(0x3F8, c);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn try_exit_diag_mark(mark: u8) {
    try_exit_diag_putc(b'[');
    try_exit_diag_putc(b't');
    try_exit_diag_putc(b'x');
    try_exit_diag_putc(b':');
    try_exit_diag_putc(mark);
    try_exit_diag_putc(b']');
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn try_exit_diag_mark(_mark: u8) {}

fn prepare_idle_recovery_context<F>(
    cpu_id: usize,
    mut update_sched: F,
) -> (u64, Option<*const CpuContext>)
where
    F: FnMut(&mut super::Scheduler),
{
    let cpu_id = safe_cpu_id(cpu_id);
    let mut idle_stack_top: u64 = 0;
    let mut idle_ctx: Option<*const CpuContext> = None;

    if let Some(mut guard) = SCHEDULER.try_lock() {
        if let Some(ref mut sched) = *guard {
            update_sched(sched);

            let idx = sched.ensure_idle_thread(cpu_id);
            let idle_tid = sched.idle_tid[cpu_id];
            let kstack_top = sched.threads[idx].kernel_stack_top();
            let kstack_bottom = sched.threads[idx].kernel_stack_bottom();
            crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, kstack_top);
            idle_stack_top = kstack_top;
            sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
            sched.per_cpu[cpu_id].current_idx = Some(idx);
            sched.threads[idx].state = ThreadState::Running;
            PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
            PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
            PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
            PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
            PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
            PER_CPU_FPU_PTR[cpu_id].store(
                sched.threads[idx].fpu_state.data.as_ptr() as u64,
                Ordering::Relaxed,
            );
            idle_ctx = Some(&sched.threads[idx].context as *const CpuContext);
        }
    } else {
        let idle_st = PER_CPU_IDLE_STACK_TOP[cpu_id].load(Ordering::Relaxed);
        if idle_st >= super::KERNEL_ADDR_MIN {
            crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, idle_st);
            idle_stack_top = idle_st;
        }
    }

    (idle_stack_top, idle_ctx)
}

fn enter_idle_recovery(
    cpu_id: usize,
    idle_stack_top: u64,
    idle_ctx: Option<*const CpuContext>,
) -> ! {
    let cpu_id = safe_cpu_id(cpu_id);
    try_exit_diag_mark(b'S');
    if idle_ctx.is_none() {
        PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
        PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
        PER_CPU_CURRENT_TID[cpu_id].store(0, Ordering::Relaxed);
        clear_per_cpu_name(cpu_id);
    }

    if idle_ctx.is_some() {
        try_exit_diag_mark(b'T');
        // Fault recovery must not rely on the normal context_switch path.
        // By the time we arrive here we already redirected the scheduler's
        // current_tid/current_idx to the per-CPU idle thread, updated TSS.RSP0,
        // and switched back to the kernel page table. A direct jump onto the
        // idle stack is enough to let timer interrupts resume normal
        // scheduling, while avoiding a second fragile register/stack restore
        // from a context that may have been corrupted by the fault.
        PER_CPU_FPU_OWNER[cpu_id].store(0, Ordering::Relaxed);
        crate::arch::hal::fpu_set_trap();
        try_exit_diag_mark(b'U');
    }

    if idle_stack_top >= super::KERNEL_ADDR_MIN {
        try_exit_diag_mark(b'V');
        unsafe {
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!(
                "mov rsp, {0}", "sti", "2: hlt", "jmp 2b",
                in(reg) idle_stack_top, options(noreturn)
            );
            #[cfg(target_arch = "aarch64")]
            core::arch::asm!(
                "mov sp, {0}",
                "msr daifclr, #0xf",
                "2: wfi",
                "b 2b",
                in(reg) idle_stack_top, options(noreturn)
            );
        }
    } else {
        crate::arch::hal::enable_interrupts();
        loop {
            crate::arch::hal::halt();
        }
    }
}

/// Collect all child TIDs of a dying thread (direct children AND transitive
/// descendants via recursive parent chains). Called with SCHEDULER lock held.
/// Also marks all found children as Terminated and removes them from run queues.
fn collect_and_terminate_children(
    sched: &mut super::Scheduler,
    parent_tid: u32,
    tick: u32,
) -> alloc::vec::Vec<u32> {
    use alloc::vec::Vec;

    // BFS: find all direct and transitive children
    let mut to_kill: Vec<u32> = Vec::new();
    let mut queue: Vec<u32> = Vec::new();
    queue.push(parent_tid);

    while let Some(ptid) = queue.pop() {
        for i in 0..sched.threads.len() {
            let t = &sched.threads[i];
            if t.parent_tid == ptid
                && t.tid != parent_tid
                && t.state != ThreadState::Terminated
                && !t.is_idle
                && !sched.is_idle_tid(t.tid)
            {
                let child_tid = t.tid;
                if !to_kill.contains(&child_tid) {
                    to_kill.push(child_tid);
                    queue.push(child_tid); // Search for grandchildren too
                }
            }
        }
    }

    // Terminate all collected children
    for &child_tid in &to_kill {
        if let Some(idx) = sched.find_idx(child_tid) {
            sched.threads[idx].state = ThreadState::Terminated;
            sched.threads[idx].exit_code = Some(9); // SIGKILL
            sched.threads[idx].terminated_at_tick = Some(tick);
            sched.threads[idx].page_directory = None; // Parent owns PD
            sched.remove_from_all_queues(child_tid);

            // Wake anyone waiting for this child
            if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                sched.wake_thread_inner(waiter_tid);
            }
        }
    }

    to_kill
}

/// Clean up resources for a list of killed child TIDs.
/// Must be called WITHOUT the scheduler lock held.
fn cleanup_killed_children(children: &[u32]) {
    use crate::fs::fd_table::FdKind;

    for &child_tid in children {
        // Close all file descriptors
        let closed = close_all_fds_for_thread(child_tid);
        for kind in closed.iter() {
            match kind {
                FdKind::File { global_id } => crate::fs::vfs::decref(*global_id),
                FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(*pipe_id),
                FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(*pipe_id),
                FdKind::Tty | FdKind::None => {}
            }
        }
        // Clean up network state
        crate::net::tcp::cleanup_for_thread(child_tid);
        crate::net::udp::cleanup_for_thread(child_tid);
        // Clean up audio mixer channels
        crate::drivers::audio::mixer::close_channels_for_pid(child_tid);
    }
}

fn defer_fault_exit_cleanup(current_tid: u32, current_code: u32, child_tids: &[u32]) {
    let mut queue = match DEFERRED_THREAD_CLEANUP.try_lock() {
        Some(q) => q,
        None => {
            crate::serial_verbose_println!(
                "  WARN: fault-exit skipped deferred cleanup enqueue for tid={} (queue lock busy)",
                current_tid
            );
            return;
        }
    };
    for &child_tid in child_tids {
        queue.push(DeferredThreadCleanup {
            tid: child_tid,
            exit_code: 0,
            emit_exit_event: false,
        });
    }
    if current_tid != 0 {
        queue.push(DeferredThreadCleanup {
            tid: current_tid,
            exit_code: current_code,
            emit_exit_event: true,
        });
    }
}

fn defer_fault_pd_destroy(pd: PhysAddr, tid: u32) {
    if let Some(mut queue) = DEFERRED_PD_DESTROY.try_lock() {
        queue.push(pd, tid);
    } else {
        crate::serial_verbose_println!(
            "  WARN: fault-exit skipped deferred PD destroy for tid={} pd={:#x} (queue lock busy)",
            tid,
            pd.as_u64()
        );
    }
}

/// Terminate the current thread with an exit code. Wakes any waitpid waiter.
/// Also terminates all child threads (cascade kill) and frees the page directory
/// once no live threads remain in the address space.
pub fn exit_current(code: u32) {
    let my_cpu = safe_cpu_id(get_cpu_id());
    let mut tid = 0u32;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut killed_children: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
    crate::sched_diag::set(my_cpu, crate::sched_diag::PHASE_EXIT_CURRENT);

    {
        let mut guard = SCHEDULER.lock();
        let cpu_id = safe_cpu_id(get_cpu_id());
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return,
        };
        let current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return,
        };
        tid = current_tid;
        let idx = match sched.current_idx(cpu_id) {
            Some(i) => i,
            None => return,
        };
        if sched.threads[idx].is_idle || sched.is_idle_tid(current_tid) {
            return;
        }

        let parent_tid = sched.threads[idx].parent_tid;
        let tick = crate::arch::hal::timer_current_ticks();

        // ── Cascade kill: terminate all child threads ──────────────
        killed_children = collect_and_terminate_children(sched, current_tid, tick);
        if !killed_children.is_empty() {
            crate::serial_println!(
                "  exit_current(tid={}): cascade-killed {} child thread(s)",
                current_tid,
                killed_children.len()
            );
        }

        // ── Mark self as Terminated ───────────────────────────────
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(code);
        sched.threads[idx].terminated_at_tick = Some(tick);

        // ── Page directory cleanup ────────────────────────────────
        if let Some(pd) = sched.threads[idx].page_directory {
            if !sched.threads[idx].pd_shared {
                let has_live_siblings = sched.threads.iter().any(|t| {
                    t.tid != current_tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
                if !has_live_siblings {
                    pd_to_destroy = Some(pd);
                }
            }
        }
        sched.threads[idx].page_directory = None;

        // Wake any thread waiting via waitpid
        if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
            sched.wake_thread_inner(waiter_tid);
        }
        // Send SIGCHLD to parent
        if parent_tid != 0 {
            if let Some(parent_idx) = sched.find_idx(parent_tid) {
                sched.threads[parent_idx]
                    .signals
                    .send(crate::ipc::signal::SIGCHLD);
            }
        }
    } // SCHEDULER lock released here

    // ── Resource cleanup for killed children (outside lock) ───────
    cleanup_killed_children(&killed_children);

    if let Some(pd) = pd_to_destroy {
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        crate::arch::hal::switch_page_table(kernel_cr3);
        DEFERRED_PD_DESTROY.lock().push(pd, 0);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED,
        tid,
        code,
        0,
        0,
    ));
    schedule();
    loop {
        crate::arch::hal::halt();
    }
}

/// Try to terminate the current thread (non-blocking lock acquisition).
/// Also cascade-kills all child threads.
///
/// This path is used by fault handlers. On success it does NOT re-enter the
/// generic scheduler; instead it switches directly to the per-CPU idle thread.
pub fn try_exit_current(code: u32) -> bool {
    let my_cpu = safe_cpu_id(get_cpu_id());
    let mut tid = 0u32;
    let mut idle_stack_top: u64 = 0;
    let mut idle_ctx: Option<*const CpuContext> = None;
    crate::sched_diag::set(my_cpu, crate::sched_diag::PHASE_TRY_EXIT_CURRENT);
    try_exit_diag_mark(b'A');

    {
        try_exit_diag_mark(b'B');
        let mut guard = match SCHEDULER.try_lock() {
            Some(g) => g,
            None => return false,
        };
        try_exit_diag_mark(b'C');
        let cpu_id = safe_cpu_id(get_cpu_id());
        try_exit_diag_mark(b'D');
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };
        try_exit_diag_mark(b'E');
        let current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return false,
        };
        try_exit_diag_mark(b'F');
        tid = current_tid;
        let idx = match sched.current_idx(cpu_id) {
            Some(i) => i,
            None => return false,
        };
        if sched.threads[idx].is_idle || sched.is_idle_tid(current_tid) {
            return false;
        }
        try_exit_diag_mark(b'G');

        let tick = crate::arch::hal::timer_current_ticks();
        try_exit_diag_mark(b'H');

        // Fault-exit must stay extremely small and non-blocking. Avoid child
        // scans, waiter wakeups, SIGCHLD delivery, and page-directory sibling
        // checks here: after a corrupted userspace fault, those traversals were
        // a recurring source of secondary deadlocks and re-entrant faults while
        // still holding the scheduler lock. A small leak is acceptable here.
        sched.remove_from_all_queues(current_tid);
        try_exit_diag_mark(b'I');
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(code);
        sched.threads[idx].terminated_at_tick = Some(tick);
        sched.threads[idx].page_directory = None;
        sched.threads[idx].exit_waiter_tid = None;
        sched.threads[idx].retain_exit_status = true;
        try_exit_diag_mark(b'J');

        try_exit_diag_mark(b'K');
        let idle_idx = sched.ensure_idle_thread(cpu_id);
        let idle_tid = sched.idle_tid[cpu_id];
        try_exit_diag_mark(b'L');
        let kstack_top = sched.threads[idle_idx].kernel_stack_top();
        let kstack_bottom = sched.threads[idle_idx].kernel_stack_bottom();
        crate::arch::hal::set_kernel_stack_for_cpu(cpu_id, kstack_top);
        idle_stack_top = kstack_top;
        sched.per_cpu[cpu_id].current_tid = Some(idle_tid);
        sched.per_cpu[cpu_id].current_idx = Some(idle_idx);
        sched.threads[idle_idx].state = ThreadState::Running;
        PER_CPU_CURRENT_TID[cpu_id].store(idle_tid, Ordering::Relaxed);
        PER_CPU_HAS_THREAD[cpu_id].store(false, Ordering::Relaxed);
        PER_CPU_IS_USER[cpu_id].store(false, Ordering::Relaxed);
        PER_CPU_STACK_BOTTOM[cpu_id].store(kstack_bottom, Ordering::Relaxed);
        PER_CPU_STACK_TOP[cpu_id].store(kstack_top, Ordering::Relaxed);
        PER_CPU_FPU_PTR[cpu_id].store(
            sched.threads[idle_idx].fpu_state.data.as_ptr() as u64,
            Ordering::Relaxed,
        );
        update_per_cpu_name(cpu_id, &sched.threads[idle_idx].name);
        idle_ctx = Some(&sched.threads[idle_idx].context as *const CpuContext);
        try_exit_diag_mark(b'M');
    } // Lock released
    try_exit_diag_mark(b'P');

    let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kernel_cr3);
    try_exit_diag_mark(b'Q');
    defer_fault_exit_cleanup(tid, code, &[]);
    try_exit_diag_mark(b'R');
    crate::sched_diag::set(my_cpu, crate::sched_diag::PHASE_IDLE);
    enter_idle_recovery(my_cpu, idle_stack_top, idle_ctx);
}

/// Saved by interrupts.asm before the recovery path replaces a bad RSP.
#[no_mangle]
pub static mut BAD_RSP_SAVED: u64 = 0;

/// Recovery function called from interrupts.asm when an ISR fires with corrupt RSP.
/// Kills the faulting thread, repairs TSS.RSP0, and enters the idle loop.
/// This function never returns.
#[no_mangle]
pub extern "C" fn bad_rsp_recovery() -> ! {
    let cpu_id = safe_cpu_id(crate::arch::hal::cpu_id());
    let tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    crate::serial_verbose_println!(
        "!RSP RECOVERY on CPU {} — killing TID={}, entering idle",
        cpu_id,
        tid
    );

    let bad_rsp = unsafe { BAD_RSP_SAVED };
    let tss_rsp0 = crate::arch::hal::get_kernel_stack_for_cpu(cpu_id);
    crate::serial_verbose_println!("  bad_rsp={:#018x} TSS.RSP0={:#018x}", bad_rsp, tss_rsp0,);

    crate::arch::hal::irq_eoi();

    let (idle_stack_top, idle_ctx) = prepare_idle_recovery_context(cpu_id, |sched| {
        if let Some(current_tid) = sched.per_cpu[cpu_id].current_tid {
            if let Some(idx) = sched.find_idx(current_tid) {
                if sched.threads[idx].critical {
                    crate::serial_verbose_println!(
                        "  CRITICAL thread '{}' (TID={}) spared",
                        sched.threads[idx].name_str(),
                        current_tid,
                    );
                    sched.threads[idx].state = ThreadState::Ready;
                    sched.threads[idx].context.save_complete = 1;
                    let pri = sched.threads[idx].priority;
                    sched.per_cpu[cpu_id].run_queue.enqueue(current_tid, pri);
                } else if !sched.threads[idx].is_idle && !sched.is_idle_tid(current_tid) {
                    sched.threads[idx].state = ThreadState::Terminated;
                    sched.threads[idx].exit_code = Some(139);
                    sched.threads[idx].terminated_at_tick =
                        Some(crate::arch::hal::timer_current_ticks());
                    if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                        sched.wake_thread_inner(waiter_tid);
                    }
                }
            }
            sched.per_cpu[cpu_id].current_tid = None;
            sched.per_cpu[cpu_id].current_idx = None;
        }
    });

    let kcr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kcr3);
    enter_idle_recovery(cpu_id, idle_stack_top, idle_ctx);
}

/// Fallback recovery when try_exit_current fails. Kills thread and enters idle.
pub fn fault_kill_and_idle(signal: u32) -> ! {
    let cpu_id = safe_cpu_id(crate::arch::hal::cpu_id());
    let tid = PER_CPU_CURRENT_TID[cpu_id].load(Ordering::Relaxed);
    fault_kill_tid_and_idle(tid, signal)
}

/// Fallback recovery for a known faulting thread. Kills `tid` and parks this CPU
/// on its idle stack, even if per-CPU current TID metadata has already drifted.
pub fn fault_kill_tid_and_idle(tid: u32, signal: u32) -> ! {
    let cpu_id = safe_cpu_id(crate::arch::hal::cpu_id());
    crate::serial_verbose_println!(
        "  FALLBACK: manual kill TID={} signal={} on CPU {}",
        tid,
        signal,
        cpu_id
    );

    let cpu = cpu_id as u32;
    if is_scheduler_locked_by_cpu(cpu) {
        unsafe {
            force_unlock_scheduler();
        }
    }
    if crate::memory::physical::is_allocator_locked_by_cpu(cpu) {
        unsafe {
            crate::memory::physical::force_unlock_allocator();
        }
        crate::serial_verbose_println!("  RECOVERED: force-released physical allocator lock");
    }
    if crate::task::dll::is_dll_locked_by_cpu(cpu) {
        unsafe {
            crate::task::dll::force_unlock_dlls();
        }
        crate::serial_verbose_println!("  RECOVERED: force-released LOADED_DLLS lock");
    }

    let (idle_stack_top, idle_ctx) = prepare_idle_recovery_context(cpu_id, |sched| {
        if let Some(idx) = sched.find_idx(tid) {
            if !sched.threads[idx].is_idle && !sched.is_idle_tid(tid) {
                let tick = crate::arch::hal::timer_current_ticks();
                sched.remove_from_all_queues(tid);
                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(signal);
                sched.threads[idx].terminated_at_tick = Some(tick);
                sched.threads[idx].page_directory = None;
                sched.threads[idx].exit_waiter_tid = None;
                sched.threads[idx].retain_exit_status = true;
            }
        }
        sched.per_cpu[cpu_id].current_tid = None;
        sched.per_cpu[cpu_id].current_idx = None;
    });

    let kcr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kcr3);
    defer_fault_exit_cleanup(tid, signal, &[]);
    enter_idle_recovery(cpu_id, idle_stack_top, idle_ctx);
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on error.
/// Also cascade-kills all child threads of the target.
pub fn kill_thread(tid: u32) -> u32 {
    if tid == 0 {
        return u32::MAX;
    }

    let mut pd_to_destroy: Option<PhysAddr> = None;
    let is_current;
    let running_on_other_cpu;
    let mut killed_children: alloc::vec::Vec<u32>;

    crate::sched_diag::set(
        safe_cpu_id(get_cpu_id()),
        crate::sched_diag::PHASE_KILL_THREAD,
    );
    let mut guard = SCHEDULER.lock();
    {
        let cpu_id = safe_cpu_id(get_cpu_id());
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return u32::MAX,
        };

        let target_idx = match sched.find_idx(tid) {
            Some(idx) => idx,
            None => return u32::MAX,
        };
        if sched.threads[target_idx].is_idle || sched.is_idle_tid(tid) {
            return u32::MAX;
        }

        is_current = sched.per_cpu[cpu_id].current_tid == Some(tid);
        running_on_other_cpu = !is_current
            && sched
                .per_cpu
                .iter()
                .enumerate()
                .any(|(i, cpu)| i != cpu_id && cpu.current_tid == Some(tid));

        let tick = crate::arch::hal::timer_current_ticks();

        // ── Cascade kill: terminate all child threads ──────────────
        killed_children = collect_and_terminate_children(sched, tid, tick);
        if !killed_children.is_empty() {
            crate::serial_println!(
                "  kill_thread(tid={}): cascade-killed {} child thread(s)",
                tid,
                killed_children.len()
            );
        }

        sched.threads[target_idx].state = ThreadState::Terminated;
        sched.threads[target_idx].exit_code = Some(u32::MAX - 1);
        sched.threads[target_idx].terminated_at_tick = Some(tick);
        sched.remove_from_all_queues(tid);

        if let Some(pd) = sched.threads[target_idx].page_directory {
            if sched.threads[target_idx].pd_shared {
                sched.threads[target_idx].page_directory = None;
            } else {
                let has_live_siblings = sched.threads.iter().any(|t| {
                    t.tid != tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
                if has_live_siblings {
                    sched.threads[target_idx].page_directory = None;
                } else {
                    pd_to_destroy = Some(pd);
                    sched.threads[target_idx].page_directory = None;
                }
            }
        }

        if let Some(waiter_tid) = sched.threads[target_idx].exit_waiter_tid {
            sched.wake_thread_inner(waiter_tid);
        }
    }

    if is_current {
        guard.release_no_irq_restore();
    } else {
        drop(guard);
    }

    // ── Resource cleanup for killed children ──────────────────────
    cleanup_killed_children(&killed_children);

    // Resource cleanup for the target thread itself (FDs, shared memory, TCP, env).
    {
        use crate::fs::fd_table::FdKind;
        let closed = close_all_fds_for_thread(tid);
        for kind in closed.iter() {
            match kind {
                FdKind::File { global_id } => {
                    crate::fs::vfs::decref(*global_id);
                }
                FdKind::PipeRead { pipe_id } => {
                    crate::ipc::anon_pipe::decref_read(*pipe_id);
                }
                FdKind::PipeWrite { pipe_id } => {
                    crate::ipc::anon_pipe::decref_write(*pipe_id);
                }
                FdKind::Tty | FdKind::None => {}
            }
        }
    }
    if let Some(pd) = pd_to_destroy {
        if is_current {
            crate::ipc::shared_memory::cleanup_process(tid);
        } else if !running_on_other_cpu {
            {
                let rflags = crate::arch::hal::save_and_disable_interrupts();
                let old_cr3 = crate::arch::hal::current_page_table();
                crate::arch::hal::switch_page_table(pd.as_u64());
                crate::ipc::shared_memory::cleanup_process(tid);
                crate::arch::hal::switch_page_table(old_cr3);
                crate::arch::hal::restore_interrupt_state(rflags);
            }
        }
    }
    crate::net::tcp::cleanup_for_thread(tid);
    crate::net::udp::cleanup_for_thread(tid);
    // Clean up audio mixer channels
    crate::drivers::audio::mixer::close_channels_for_pid(tid);
    if let Some(pd) = pd_to_destroy {
        crate::task::env::cleanup(pd.as_u64());
    }

    if let Some(pd) = pd_to_destroy {
        if running_on_other_cpu {
            DEFERRED_PD_DESTROY.lock().push(pd, tid);
        } else {
            if is_current {
                let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
                crate::arch::hal::switch_page_table(kernel_cr3);
            }
            DEFERRED_PD_DESTROY.lock().push(pd, 0);
        }
    }

    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED,
        tid,
        u32::MAX - 1,
        0,
        0,
    ));

    if is_current {
        schedule();
        loop {
            crate::arch::hal::halt();
        }
    }
    0
}
