//! fork() helpers — snapshot parent state, copy fields to child, exec update.

use super::{get_cpu_id, SCHEDULER};
use crate::memory::address::PhysAddr;

/// Snapshot of a thread's state needed for fork().
/// All fields captured under a single scheduler lock to prevent TOCTOU.
pub struct ForkSnapshot {
    pub pd: PhysAddr,
    pub brk: u32,
    pub arch_mode: crate::task::thread::ArchMode,
    pub args: [u8; 256],
    pub cwd: [u8; 512],
    pub capabilities: crate::task::capabilities::CapSet,
    pub uid: u16,
    pub gid: u16,
    pub stdout_pipe: u32,
    pub stdin_pipe: u32,
    pub fpu_data: [u8; crate::task::thread::FPU_STATE_SIZE],
    pub mmap_next: u32,
    pub user_pages: u32,
    pub priority: u8,
    pub name: [u8; 32],
    pub fd_table: crate::fs::fd_table::FdTable,
    pub signals: crate::ipc::signal::SignalState,
    pub parent_tid: u32,
}

/// Capture all fork-relevant fields from the current thread in a single lock.
pub fn current_thread_fork_snapshot() -> Option<ForkSnapshot> {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let sched = guard.as_ref()?;
    let cpu = get_cpu_id();
    let idx = sched.current_idx(cpu)?;
    let thread = &sched.threads[idx];
    let pd = thread.page_directory?;
    Some(ForkSnapshot {
        pd,
        brk: thread.brk,
        arch_mode: thread.arch_mode,
        args: thread.args,
        cwd: thread.cwd,
        capabilities: thread.capabilities,
        uid: thread.uid,
        gid: thread.gid,
        stdout_pipe: thread.stdout_pipe,
        stdin_pipe: thread.stdin_pipe,
        fpu_data: thread.fpu_state.data,
        mmap_next: thread.mmap_next,
        user_pages: thread.user_pages,
        priority: thread.priority,
        name: thread.name,
        fd_table: thread.fd_table.clone(),
        signals: thread.signals.clone(),
        parent_tid: thread.tid,
    })
}

/// Set the FPU state on a thread (for fork child).
pub fn set_thread_fpu_state(tid: u32, data: &[u8; crate::task::thread::FPU_STATE_SIZE]) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.fpu_state.data = *data;
    }
}

/// Set mmap_next on a thread (for fork child).
pub fn set_thread_mmap_next(tid: u32, val: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.mmap_next = val;
    }
}

/// Set user_pages count on a thread (for fork child).
pub fn set_thread_user_pages(tid: u32, val: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.user_pages = val;
    }
}

/// Update thread state for exec(): new PD, reset brk/mmap/fpu, change arch mode.
pub fn exec_update_thread(
    tid: u32,
    new_pd: PhysAddr,
    brk: u32,
    arch_mode: crate::task::thread::ArchMode,
    user_pages: u32,
) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(new_pd);
        #[cfg(target_arch = "x86_64")]
        {
            thread.pcid = crate::memory::virtual_mem::allocate_pcid();
            thread.context.set_page_table(new_pd.as_u64() | thread.pcid as u64);
        }
        #[cfg(target_arch = "aarch64")]
        thread.context.set_page_table(new_pd.as_u64());
        thread.brk = brk;
        // ASLR: randomize the mmap base within [0x20000000, 0x20000000 + 16 MiB)
        let mmap_rand = crate::task::loader::random_page_offset(
            crate::task::loader::ASLR_MMAP_MAX_PAGES,
        );
        thread.mmap_next = 0x7000_0000u32.wrapping_add(mmap_rand * 4096);
        thread.fpu_state = crate::task::thread::FxState::new_default();
        thread.user_pages = user_pages;
        thread.arch_mode = arch_mode;
        thread.context.checksum = thread.context.compute_checksum();
    }
}

/// Get the DR1 watch address (kept for compat — returns 0, no watchpoint in Mach scheduler).
pub fn get_dr1_watch_addr() -> u64 { 0 }
