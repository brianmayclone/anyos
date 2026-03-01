//! Thread creation: spawn, spawn_blocked, create_thread_in_current_process.

use super::{get_cpu_id, clamp_priority, SCHEDULER};
use crate::task::thread::Thread;
use alloc::boxed::Box;

/// Create a new kernel thread and add it to the ready queue.
pub fn spawn(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        // Box the thread BEFORE acquiring SCHEDULER — prevents ALLOCATOR
        // contention (from concurrent clone_pd) from holding SCHEDULER for
        // 100-400 ms and causing SPIN TIMEOUT on other CPUs.
        let thread = Box::new(Thread::new(entry, priority, name));
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        sched.add_thread(thread)
    };
    // Debug output OUTSIDE the lock — serial I/O takes ~3ms at 115200 baud
    // and holding the lock that long starves CPU 0's reap_terminated.
    #[cfg(feature = "debug_verbose")]
    crate::serial_println!("  Spawned thread '{}' (TID={})", name, tid);
    emit_spawn_event(tid, name);
    tid
}

/// Spawn a kernel thread in Blocked state (not added to any ready queue).
/// The thread will NOT run until [`wake_thread`] is called.
pub fn spawn_blocked(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        // Box the thread BEFORE acquiring SCHEDULER — same reasoning as spawn().
        let thread = Box::new(Thread::new(entry, priority, name));
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN_BLOCKED);
        let mut sched = SCHEDULER.lock();
        let sched = sched.as_mut().expect("Scheduler not initialized");
        sched.add_thread_blocked(thread)
    };
    // Debug output OUTSIDE the lock (serial I/O is slow).
    #[cfg(feature = "debug_verbose")]
    crate::serial_println!("  Spawned thread '{}' (TID={}, blocked)", name, tid);
    emit_spawn_event(tid, name);
    tid
}

/// Helper: emit EVT_PROCESS_SPAWNED with the thread name packed into u32 words.
fn emit_spawn_event(tid: u32, name: &str) {
    let name_bytes = name.as_bytes();
    let mut p2: u32 = 0;
    let mut p3: u32 = 0;
    let mut p4: u32 = 0;
    for i in 0..name_bytes.len().min(12) {
        let word = match i / 4 { 0 => &mut p2, 1 => &mut p3, _ => &mut p4 };
        *word |= (name_bytes[i] as u32) << ((i % 4) * 8);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED, tid, p2, p3, p4,
    ));
}

/// Create a new thread within the same address space as the currently running thread.
pub fn create_thread_in_current_process(entry_rip: u64, user_rsp: u64, name: &str, priority: u8) -> u32 {
    let (pd, arch_mode, brk, parent_pri, parent_cwd, parent_caps, parent_uid, parent_gid, parent_pcid, parent_mmap_next) = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() { Some(s) => s, None => return 0 };
        let current_tid = match sched.per_cpu[cpu_id].current_tid { Some(t) => t, None => return 0 };
        let idx = match sched.find_idx(current_tid) { Some(i) => i, None => return 0 };
        let thread = &sched.threads[idx];
        let pd = match thread.page_directory { Some(pd) => pd, None => return 0 };
        (pd, thread.arch_mode, thread.brk, thread.priority, thread.cwd, thread.capabilities, thread.uid, thread.gid, thread.pcid, thread.mmap_next)
    };

    let effective_pri = if priority == 0 { parent_pri } else { priority };
    let effective_pri = clamp_priority(effective_pri, name);
    let tid = spawn_blocked(crate::task::loader::thread_create_trampoline, effective_pri, name);

    {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let mut guard = SCHEDULER.lock();
        let sched = guard.as_mut().expect("Scheduler not initialized");
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.pcid = parent_pcid; // Same address space = same PCID
            #[cfg(target_arch = "x86_64")]
            thread.context.set_page_table(pd.as_u64() | parent_pcid as u64);
            #[cfg(target_arch = "aarch64")]
            thread.context.set_page_table(pd.as_u64());
            thread.context.checksum = thread.context.compute_checksum();
            thread.is_user = true;
            thread.brk = brk;
            thread.arch_mode = arch_mode;
            thread.pd_shared = true;
            thread.cwd = parent_cwd;
            thread.capabilities = parent_caps;
            thread.uid = parent_uid;
            thread.gid = parent_gid;
            thread.mmap_next = parent_mmap_next;
        }
    }

    crate::task::loader::store_pending_thread(tid, entry_rip, user_rsp);
    super::wake_thread(tid);
    tid
}
