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

#[derive(Clone, Copy)]
struct TerminatedChildCleanup {
    tid: u32,
    pd_to_destroy: Option<PhysAddr>,
}

const MAX_FAULT_PROCESS_CLEANUP: usize = 128;

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

#[inline]
fn kick_resched_cpu(cpu: usize) {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86::smp::resched_cpu(cpu);
    #[cfg(target_arch = "aarch64")]
    crate::arch::hal::send_ipi(cpu, 1);
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = cpu;
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
) -> alloc::vec::Vec<TerminatedChildCleanup> {
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

    // Terminate all collected children first.  We collect address spaces in a
    // second pass so sibling checks see every cascade-killed child as dead.
    // Once the parent is exiting/killed, none of these children has a live
    // waiter owner any more; orphan them so the deferred reaper may drop their
    // kernel stacks after the grace period.
    for &child_tid in &to_kill {
        if let Some(idx) = sched.find_idx(child_tid) {
            if sched.threads[idx].state != ThreadState::Terminated {
                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(9); // SIGKILL
                sched.threads[idx].terminated_at_tick = Some(tick);
            }
            sched.remove_from_all_queues(child_tid);

            // Wake anyone waiting for this child
            if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                sched.wake_thread_inner(waiter_tid);
            }
            sched.threads[idx].exit_waiter_tid = None;
            sched.threads[idx].retain_exit_status = false;
            sched.threads[idx].parent_tid = 0;
        }
    }

    let mut cleanup = Vec::new();
    let mut queued_pds = Vec::new();
    for &child_tid in &to_kill {
        let mut pd_to_destroy = None;
        if let Some(idx) = sched.find_idx(child_tid) {
            if let Some(pd) = sched.threads[idx].page_directory {
                let has_live_siblings = sched.threads.iter().any(|t| {
                    t.tid != child_tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
                if !has_live_siblings && !queued_pds.iter().any(|queued| *queued == pd) {
                    pd_to_destroy = Some(pd);
                    queued_pds.push(pd);
                }
                sched.threads[idx].page_directory = None;
            }
        }
        cleanup.push(TerminatedChildCleanup {
            tid: child_tid,
            pd_to_destroy,
        });
    }

    cleanup
}

fn cleanup_thread_resources(tid: u32) {
    use crate::fs::fd_table::FdKind;

    let closed = close_all_fds_for_thread(tid);
    for kind in closed.iter() {
        match kind {
            FdKind::File { global_id } => crate::fs::vfs::decref(*global_id),
            FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(*pipe_id),
            FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(*pipe_id),
            FdKind::LinuxSocket { socket_id } => crate::syscall::linux::socket_decref(*socket_id),
            FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxProc { .. }
            | FdKind::LinuxFramebuffer { .. }
            | FdKind::None => {}
        }
    }
    crate::net::tcp::cleanup_for_thread(tid);
    crate::net::udp::cleanup_for_thread(tid);
    crate::drivers::audio::mixer::close_channels_for_pid(tid);
}

/// Clean up resources for a list of killed child TIDs.
/// Must be called WITHOUT the scheduler lock held.
fn cleanup_killed_children(children: &[TerminatedChildCleanup]) {
    for child in children {
        cleanup_thread_resources(child.tid);
        if let Some(pd) = child.pd_to_destroy {
            crate::syscall::windows::cleanup_process(pd.as_u64());
            crate::task::env::cleanup(pd.as_u64());
            DEFERRED_PD_DESTROY.lock().push(pd, child.tid);
        }
    }
}

/// Linux execve() replaces the whole thread group.  Our LXE pthreads are
/// represented as scheduler threads sharing one page directory; if the execing
/// thread destroys that old page directory while a sibling is still queued or
/// running, the sibling can execute through freed page tables.
///
/// Returns true when the caller may immediately destroy `old_pd`.  If any
/// sibling was observed running on another CPU, the caller must leave the old
/// address space intact; the sibling has been marked terminated and kicked, but
/// may not have left userspace yet.
pub fn terminate_exec_siblings(current_tid: u32, old_pd: PhysAddr) -> bool {
    let tick = crate::arch::hal::timer_current_ticks();
    let mut cleanup_tids = alloc::vec::Vec::new();
    let mut kick_cpus = alloc::vec::Vec::new();
    let mut saw_running_sibling = false;

    {
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return true,
        };

        let mut idx = 0usize;
        while idx < sched.threads.len() {
            let should_kill = {
                let thread = &sched.threads[idx];
                thread.tid != current_tid
                    && thread.page_directory == Some(old_pd)
                    && !thread.is_idle
                    && !sched.is_idle_tid(thread.tid)
                    && thread.state != ThreadState::Terminated
            };
            if !should_kill {
                idx += 1;
                continue;
            }

            let tid = sched.threads[idx].tid;
            for (cpu_id, cpu) in sched.per_cpu.iter().enumerate() {
                if cpu.current_tid == Some(tid) {
                    saw_running_sibling = true;
                    if !kick_cpus.contains(&cpu_id) {
                        kick_cpus.push(cpu_id);
                    }
                }
            }

            sched.remove_from_all_queues(tid);
            let waiter = sched.threads[idx].exit_waiter_tid;
            sched.threads[idx].state = ThreadState::Terminated;
            sched.threads[idx].exit_code = Some(9);
            sched.threads[idx].terminated_at_tick = Some(tick);
            sched.threads[idx].page_directory = None;
            sched.threads[idx].exit_waiter_tid = None;
            sched.threads[idx].retain_exit_status = false;
            sched.threads[idx].parent_tid = 0;
            if let Some(waiter_tid) = waiter {
                sched.wake_thread_inner(waiter_tid);
            }
            cleanup_tids.push(tid);
            idx += 1;
        }
    }

    for cpu in kick_cpus {
        kick_resched_cpu(cpu);
    }
    for tid in cleanup_tids.iter().copied() {
        cleanup_thread_resources(tid);
    }
    if !cleanup_tids.is_empty() {
        crate::serial_verbose_println!(
            "lxe execve: terminated {} sibling thread(s) sharing old pd={:#x}{}",
            cleanup_tids.len(),
            old_pd.as_u64(),
            if saw_running_sibling {
                " (old pd retained: sibling still running)"
            } else {
                ""
            }
        );
    }

    !saw_running_sibling
}

fn defer_fault_exit_cleanup(
    current_tid: u32,
    current_code: u32,
    current_abi_tag: u32,
    child_tids: &[u32],
) {
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
            abi_tag: 0,
            emit_exit_event: false,
        });
    }
    if current_tid != 0 {
        queue.push(DeferredThreadCleanup {
            tid: current_tid,
            exit_code: current_code,
            abi_tag: current_abi_tag,
            emit_exit_event: true,
        });
    }
}

fn terminate_same_pd_threads_for_fault(
    sched: &mut super::Scheduler,
    pd: PhysAddr,
    current_tid: u32,
    code: u32,
    tick: u32,
    cleanup_tids: &mut [u32; MAX_FAULT_PROCESS_CLEANUP],
) -> usize {
    let mut cleanup_count = 0usize;
    for idx in 0..sched.threads.len() {
        let should_kill = {
            let t = &sched.threads[idx];
            t.tid != current_tid
                && t.page_directory == Some(pd)
                && !t.is_idle
                && !sched.is_idle_tid(t.tid)
        };
        if !should_kill {
            continue;
        }

        let tid = sched.threads[idx].tid;
        let waiter = sched.threads[idx].exit_waiter_tid;
        sched.remove_from_all_queues(tid);
        if sched.threads[idx].state != ThreadState::Terminated {
            sched.threads[idx].state = ThreadState::Terminated;
            sched.threads[idx].exit_code = Some(code);
            sched.threads[idx].terminated_at_tick = Some(tick);
        }
        sched.threads[idx].page_directory = None;
        sched.threads[idx].exit_waiter_tid = None;
        sched.threads[idx].retain_exit_status = false;
        sched.threads[idx].parent_tid = 0;
        if let Some(waiter_tid) = waiter {
            sched.wake_thread_inner(waiter_tid);
        }
        if cleanup_count < cleanup_tids.len() {
            cleanup_tids[cleanup_count] = tid;
            cleanup_count += 1;
        } else {
            crate::serial_verbose_println!(
                "  WARN: fault-exit cleanup list full, tid={} cleanup deferred only by reap",
                tid
            );
        }
    }
    cleanup_count
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
    let tid: u32;
    let abi_tag: u32;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let killed_children: alloc::vec::Vec<TerminatedChildCleanup>;
    let mut wake_kick_cpu: Option<usize> = None;
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
        abi_tag = sched.threads[idx].abi.event_tag();

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
        crate::serial_verbose_println!(
            "lxe exit: tid={} parent={} code={} waiter={} children_killed={}",
            current_tid,
            parent_tid,
            code,
            sched.threads[idx].exit_waiter_tid.unwrap_or(0),
            killed_children.len()
        );

        // ── Page directory cleanup ────────────────────────────────
        if let Some(pd) = sched.threads[idx].page_directory {
            let has_live_siblings = sched.threads.iter().any(|t| {
                t.tid != current_tid
                    && t.page_directory == Some(pd)
                    && t.state != ThreadState::Terminated
            });
            if !has_live_siblings {
                pd_to_destroy = Some(pd);
            }
        }
        if sched.threads[idx].pd_shared {
            sched.threads[idx].parent_tid = 0;
            sched.threads[idx].retain_exit_status = false;
        }
        sched.threads[idx].page_directory = None;

        // Wake any thread waiting via waitpid
        if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
            wake_kick_cpu = sched.wake_thread_inner(waiter_tid);
        }
        // Send SIGCHLD to parent
        if parent_tid != 0 {
            let child_uid = sched.threads[idx].uid as u32;
            if let Some(parent_idx) = sched.find_idx(parent_tid) {
                sched.threads[parent_idx].signals.send_with_info(
                    crate::ipc::signal::SIGCHLD,
                    crate::ipc::signal::SignalInfo {
                        code: 1, // CLD_EXITED
                        pid: current_tid,
                        uid: child_uid,
                        status: code,
                    },
                );
            }
        }
    } // SCHEDULER lock released here

    if let Some(cpu) = wake_kick_cpu {
        kick_resched_cpu(cpu);
    }

    // ── Resource cleanup for killed children (outside lock) ───────
    cleanup_killed_children(&killed_children);

    // Resource cleanup for the exiting thread itself.  Reaping only drops the
    // ThreadBox; kernel resources referenced by its FD table must be released
    // while the TID is still known.
    cleanup_thread_resources(tid);

    if let Some(pd) = pd_to_destroy {
        crate::syscall::windows::cleanup_process(pd.as_u64());
        crate::task::env::cleanup(pd.as_u64());
        let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
        crate::arch::hal::switch_page_table(kernel_cr3);
        DEFERRED_PD_DESTROY.lock().push(pd, tid);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_EXITED,
        tid,
        code,
        abi_tag,
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
    let tid: u32;
    let abi_tag: u32;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut killed_sibling_tids = [0u32; MAX_FAULT_PROCESS_CLEANUP];
    let mut killed_sibling_count = 0usize;
    let idle_stack_top: u64;
    let idle_ctx: Option<*const CpuContext>;
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
        abi_tag = sched.threads[idx].abi.event_tag();
        try_exit_diag_mark(b'G');

        let tick = crate::arch::hal::timer_current_ticks();
        try_exit_diag_mark(b'H');

        // Fault-exit must stay small and non-blocking.  We still preserve the
        // dying address space for deferred teardown; otherwise a faulting LXE
        // process can leak its entire userspace mapping.
        if let Some(pd) = sched.threads[idx].page_directory {
            killed_sibling_count = terminate_same_pd_threads_for_fault(
                sched,
                pd,
                current_tid,
                code,
                tick,
                &mut killed_sibling_tids,
            );
            let has_live_siblings = sched.threads.iter().any(|t| {
                t.tid != current_tid
                    && t.page_directory == Some(pd)
                    && t.state != ThreadState::Terminated
            });
            if !has_live_siblings {
                pd_to_destroy = Some(pd);
            }
        }
        sched.remove_from_all_queues(current_tid);
        try_exit_diag_mark(b'I');
        sched.threads[idx].state = ThreadState::Terminated;
        sched.threads[idx].exit_code = Some(code);
        sched.threads[idx].terminated_at_tick = Some(tick);
        sched.threads[idx].page_directory = None;
        sched.threads[idx].exit_waiter_tid = None;
        sched.threads[idx].retain_exit_status = false;
        sched.threads[idx].parent_tid = 0;
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
        update_per_cpu_name(
            cpu_id,
            &sched.threads[idle_idx].name,
            sched.threads[idle_idx].abi,
            &sched.threads[idle_idx].linux_rootfs,
        );
        idle_ctx = Some(&sched.threads[idle_idx].context as *const CpuContext);
        try_exit_diag_mark(b'M');
    } // Lock released
    try_exit_diag_mark(b'P');

    let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kernel_cr3);
    try_exit_diag_mark(b'Q');
    defer_fault_exit_cleanup(
        tid,
        code,
        abi_tag,
        &killed_sibling_tids[..killed_sibling_count],
    );
    if let Some(pd) = pd_to_destroy {
        crate::syscall::windows::cleanup_process(pd.as_u64());
        crate::task::env::cleanup(pd.as_u64());
        defer_fault_pd_destroy(pd, tid);
    }
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
    let mut abi_tag = 0u32;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut killed_thread = false;
    let mut killed_sibling_tids = [0u32; MAX_FAULT_PROCESS_CLEANUP];
    let mut killed_sibling_count = 0usize;
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
                    abi_tag = sched.threads[idx].abi.event_tag();
                    if let Some(pd) = sched.threads[idx].page_directory {
                        killed_sibling_count = terminate_same_pd_threads_for_fault(
                            sched,
                            pd,
                            current_tid,
                            139,
                            crate::arch::hal::timer_current_ticks(),
                            &mut killed_sibling_tids,
                        );
                        let has_live_siblings = sched.threads.iter().any(|t| {
                            t.tid != current_tid
                                && t.page_directory == Some(pd)
                                && t.state != ThreadState::Terminated
                        });
                        if !has_live_siblings {
                            pd_to_destroy = Some(pd);
                        }
                    }
                    sched.remove_from_all_queues(current_tid);
                    sched.threads[idx].state = ThreadState::Terminated;
                    sched.threads[idx].exit_code = Some(139);
                    sched.threads[idx].terminated_at_tick =
                        Some(crate::arch::hal::timer_current_ticks());
                    if let Some(waiter_tid) = sched.threads[idx].exit_waiter_tid {
                        sched.wake_thread_inner(waiter_tid);
                    }
                    sched.threads[idx].page_directory = None;
                    sched.threads[idx].exit_waiter_tid = None;
                    sched.threads[idx].retain_exit_status = false;
                    sched.threads[idx].parent_tid = 0;
                    killed_thread = true;
                }
            }
            sched.per_cpu[cpu_id].current_tid = None;
            sched.per_cpu[cpu_id].current_idx = None;
        }
    });

    let kcr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kcr3);
    if killed_thread {
        defer_fault_exit_cleanup(
            tid,
            139,
            abi_tag,
            &killed_sibling_tids[..killed_sibling_count],
        );
        if let Some(pd) = pd_to_destroy {
            crate::syscall::windows::cleanup_process(pd.as_u64());
            crate::task::env::cleanup(pd.as_u64());
            defer_fault_pd_destroy(pd, tid);
        }
    }
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
    let mut abi_tag = 0u32;
    let mut pd_to_destroy: Option<PhysAddr> = None;
    let mut killed_sibling_tids = [0u32; MAX_FAULT_PROCESS_CLEANUP];
    let mut killed_sibling_count = 0usize;
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
                abi_tag = sched.threads[idx].abi.event_tag();
                if let Some(pd) = sched.threads[idx].page_directory {
                    killed_sibling_count = terminate_same_pd_threads_for_fault(
                        sched,
                        pd,
                        tid,
                        signal,
                        tick,
                        &mut killed_sibling_tids,
                    );
                    let has_live_siblings = sched.threads.iter().any(|t| {
                        t.tid != tid
                            && t.page_directory == Some(pd)
                            && t.state != ThreadState::Terminated
                    });
                    if !has_live_siblings {
                        pd_to_destroy = Some(pd);
                    }
                }
                sched.remove_from_all_queues(tid);
                sched.threads[idx].state = ThreadState::Terminated;
                sched.threads[idx].exit_code = Some(signal);
                sched.threads[idx].terminated_at_tick = Some(tick);
                sched.threads[idx].page_directory = None;
                sched.threads[idx].exit_waiter_tid = None;
                sched.threads[idx].retain_exit_status = false;
                sched.threads[idx].parent_tid = 0;
            }
        }
        sched.per_cpu[cpu_id].current_tid = None;
        sched.per_cpu[cpu_id].current_idx = None;
    });

    let kcr3 = crate::memory::virtual_mem::kernel_cr3();
    crate::arch::hal::switch_page_table(kcr3);
    defer_fault_exit_cleanup(
        tid,
        signal,
        abi_tag,
        &killed_sibling_tids[..killed_sibling_count],
    );
    if let Some(pd) = pd_to_destroy {
        crate::syscall::windows::cleanup_process(pd.as_u64());
        crate::task::env::cleanup(pd.as_u64());
        defer_fault_pd_destroy(pd, tid);
    }
    enter_idle_recovery(cpu_id, idle_stack_top, idle_ctx);
}

/// Kill a thread by TID. Returns 0 on success, u32::MAX on error.
/// Also cascade-kills all child threads of the target.
pub fn kill_thread(tid: u32) -> u32 {
    if tid == 0 {
        return u32::MAX;
    }

    let mut pd_to_destroy: Option<PhysAddr> = None;
    let abi_tag: u32;
    let is_current;
    let running_on_other_cpu;
    let killed_children: alloc::vec::Vec<TerminatedChildCleanup>;
    let mut wake_kick_cpu: Option<usize> = None;

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
        abi_tag = sched.threads[target_idx].abi.event_tag();

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
            let has_live_siblings = sched.threads.iter().any(|t| {
                t.tid != tid && t.page_directory == Some(pd) && t.state != ThreadState::Terminated
            });
            if !has_live_siblings {
                pd_to_destroy = Some(pd);
            }
            if sched.threads[target_idx].pd_shared {
                sched.threads[target_idx].parent_tid = 0;
                sched.threads[target_idx].retain_exit_status = false;
            }
            sched.threads[target_idx].page_directory = None;
        }

        if let Some(waiter_tid) = sched.threads[target_idx].exit_waiter_tid {
            wake_kick_cpu = sched.wake_thread_inner(waiter_tid);
        }
    }

    if is_current {
        guard.release_no_irq_restore();
    } else {
        drop(guard);
    }

    if let Some(cpu) = wake_kick_cpu {
        kick_resched_cpu(cpu);
    }

    // ── Resource cleanup for killed children ──────────────────────
    cleanup_killed_children(&killed_children);

    // Resource cleanup for the target thread itself (FDs, TCP/UDP, audio).
    cleanup_thread_resources(tid);
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
    if let Some(pd) = pd_to_destroy {
        crate::syscall::windows::cleanup_process(pd.as_u64());
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
        abi_tag,
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
