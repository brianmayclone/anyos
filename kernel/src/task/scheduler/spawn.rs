//! Thread creation: spawn, spawn_blocked, create_thread_in_current_process.

use super::{alloc_thread_box, clamp_priority, get_cpu_id, SCHEDULER};
use crate::fs::fd_table::{FdKind, FdTable};
use crate::task::thread::Thread;

/// Create a new kernel thread and add it to the ready queue.
pub fn spawn(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        // Allocate the Thread object BEFORE acquiring SCHEDULER. The object
        // comes from the task::Thread slab cache, keeping scheduler metadata
        // isolated from the general heap.
        let thread = match Thread::new(entry, priority, name).and_then(alloc_thread_box) {
            Some(thread) => thread,
            None => return 0,
        };
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN);
        let mut sched = SCHEDULER.lock();
        let sched = match sched.as_mut() {
            Some(s) => s,
            None => return 0,
        };
        sched.add_thread(thread)
    };
    // Debug output OUTSIDE the lock — serial I/O takes ~3ms at 115200 baud
    // and holding the lock that long starves CPU 0's reap_terminated.
    #[cfg(feature = "debug_verbose")]
    crate::serial_verbose_println!("  Spawned thread '{}' (TID={})", name, tid);
    emit_spawn_event(tid, name);
    tid
}

/// Spawn a kernel thread in Blocked state (not added to any ready queue).
/// The thread will NOT run until [`wake_thread`] is called.
pub fn spawn_blocked(entry: extern "C" fn(), priority: u8, name: &str) -> u32 {
    let priority = clamp_priority(priority, name);
    let tid = {
        // Allocate the Thread object BEFORE acquiring SCHEDULER — same
        // reasoning as spawn().
        let thread = match Thread::new(entry, priority, name).and_then(alloc_thread_box) {
            Some(thread) => thread,
            None => return 0,
        };
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SPAWN_BLOCKED);
        let mut sched = SCHEDULER.lock();
        let sched = match sched.as_mut() {
            Some(s) => s,
            None => return 0,
        };
        let new_tid = sched.add_thread_blocked(thread);
        // Set parent_tid inside the same lock (avoids separate lock acquisition).
        let caller_tid = sched.per_cpu[get_cpu_id()].current_tid.unwrap_or(0);
        if caller_tid != 0 {
            if let Some(t) = sched.threads.iter_mut().find(|t| t.tid == new_tid) {
                t.parent_tid = caller_tid;
            }
        }
        new_tid
    };
    // Debug output OUTSIDE the lock (serial I/O is slow).
    #[cfg(feature = "debug_verbose")]
    crate::serial_verbose_println!("  Spawned thread '{}' (TID={}, blocked)", name, tid);
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
        let word = match i / 4 {
            0 => &mut p2,
            1 => &mut p3,
            _ => &mut p4,
        };
        *word |= (name_bytes[i] as u32) << ((i % 4) * 8);
    }
    crate::ipc::event_bus::system_emit(crate::ipc::event_bus::EventData::new(
        crate::ipc::event_bus::EVT_PROCESS_SPAWNED,
        tid,
        p2,
        p3,
        p4,
    ));
}

/// Create a new thread within the same address space as the currently running thread.
pub fn create_thread_in_current_process(
    entry_rip: u64,
    user_rsp: u64,
    user_lr: u64,
    name: &str,
    priority: u8,
) -> u32 {
    let (
        pd,
        brk,
        parent_pri,
        parent_abi,
        parent_cwd,
        parent_caps,
        parent_uid,
        parent_gid,
        parent_pcid,
        parent_mmap_next,
        parent_fd_table,
    ) = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return 0,
        };
        let current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return 0,
        };
        let idx = match sched.find_idx(current_tid) {
            Some(i) => i,
            None => return 0,
        };
        let thread = &sched.threads[idx];
        let pd = match thread.page_directory {
            Some(pd) => pd,
            None => return 0,
        };
        let fd_table = thread.fd_table.lock().clone();
        (
            pd,
            thread.brk,
            thread.priority,
            thread.abi,
            thread.cwd,
            thread.capabilities,
            thread.uid,
            thread.gid,
            thread.pcid,
            thread.mmap_next,
            fd_table,
        )
    };

    let effective_pri = if priority == 0 { parent_pri } else { priority };
    let effective_pri = clamp_priority(effective_pri, name);
    let tid = spawn_blocked(
        crate::task::loader::thread_create_trampoline,
        effective_pri,
        name,
    );
    if tid == 0 {
        return 0;
    }

    let fd_table_installed = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return 0,
        };
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.pcid = parent_pcid; // Same address space = same PCID
            #[cfg(target_arch = "x86_64")]
            thread
                .context
                .set_page_table(pd.as_u64() | parent_pcid as u64);
            #[cfg(target_arch = "aarch64")]
            thread.context.set_page_table(pd.as_u64());
            thread.context.checksum = thread.context.compute_checksum();
            thread.is_user = true;
            thread.brk = brk;
            thread.abi = parent_abi;
            thread.pd_shared = true;
            thread.cwd = parent_cwd;
            thread.capabilities = parent_caps;
            thread.uid = parent_uid;
            thread.gid = parent_gid;
            thread.mmap_next = parent_mmap_next;
            thread.fd_table =
                alloc::sync::Arc::new(crate::sync::spinlock::Spinlock::new(parent_fd_table.clone()));
            true
        } else {
            false
        }
    };
    if !fd_table_installed {
        super::kill_thread(tid);
        return 0;
    }
    incref_fd_table(&parent_fd_table);

    if !crate::task::loader::store_pending_thread(tid, entry_rip, user_rsp, user_lr) {
        crate::serial_println!(
            "create_thread_in_current_process: pending-program table full for tid={}",
            tid
        );
        super::kill_thread(tid);
        return 0;
    }
    super::wake_thread(tid);
    tid
}

/// Create a Linux ABI thread from clone()/clone3() inside the current address
/// space. The child resumes through the fork return trampoline so the syscall
/// returns twice: child sees RAX=0, parent sees the new TID.
#[cfg(target_arch = "x86_64")]
pub fn create_linux_clone_thread(
    regs: crate::task::loader::ForkChildRegs,
    clear_child_tid: u64,
) -> u32 {
    let (
        pd,
        brk,
        brk_start,
        parent_pri,
        parent_cwd,
        parent_rootfs,
        parent_caps,
        parent_uid,
        parent_gid,
        parent_linux_real_uid,
        parent_linux_saved_uid,
        parent_linux_fs_uid,
        parent_linux_real_gid,
        parent_linux_saved_gid,
        parent_linux_fs_gid,
        parent_stdout_pipe,
        parent_stdin_pipe,
        parent_pty_id,
        parent_pcid,
        parent_mmap_next,
        parent_user_pages,
        parent_fd_table,
        parent_signals,
        parent_args,
        parent_name,
        parent_fpu_data,
    ) = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let guard = SCHEDULER.lock();
        let cpu_id = get_cpu_id();
        let sched = match guard.as_ref() {
            Some(s) => s,
            None => return 0,
        };
        let current_tid = match sched.per_cpu[cpu_id].current_tid {
            Some(t) => t,
            None => return 0,
        };
        let idx = match sched.find_idx(current_tid) {
            Some(i) => i,
            None => return 0,
        };
        let thread = &sched.threads[idx];
        if thread.abi != crate::task::abi::AbiPersonality::LinuxX86_64 {
            return 0;
        }
        let pd = match thread.page_directory {
            Some(pd) => pd,
            None => return 0,
        };
        let fd_table = thread.fd_table.lock().clone();
        (
            pd,
            thread.brk,
            thread.brk_start,
            thread.priority,
            thread.cwd,
            thread.linux_rootfs,
            thread.capabilities,
            thread.uid,
            thread.gid,
            thread.linux_real_uid,
            thread.linux_saved_uid,
            thread.linux_fs_uid,
            thread.linux_real_gid,
            thread.linux_saved_gid,
            thread.linux_fs_gid,
            thread.stdout_pipe,
            thread.stdin_pipe,
            thread.pty_id,
            thread.pcid,
            thread.mmap_next,
            thread.user_pages,
            fd_table,
            thread.signals.clone(),
            thread.args,
            thread.name,
            thread.fpu_state.data,
        )
    };

    let name_len = parent_name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(parent_name.len());
    let name = core::str::from_utf8(&parent_name[..name_len]).unwrap_or("linux-thread");
    let priority = parent_pri.min(crate::task::abi::LINUX_MAX_USER_PRIORITY);
    let tid = spawn_blocked(crate::task::loader::fork_child_trampoline, priority, name);
    if tid == 0 {
        return 0;
    }

    let installed = {
        crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CREATE_THREAD);
        let mut guard = SCHEDULER.lock();
        let sched = match guard.as_mut() {
            Some(s) => s,
            None => return 0,
        };
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.page_directory = Some(pd);
            thread.pcid = parent_pcid;
            thread
                .context
                .set_page_table(pd.as_u64() | parent_pcid as u64);
            thread.context.checksum = thread.context.compute_checksum();
            thread.is_user = true;
            thread.brk = brk;
            thread.brk_start = brk_start;
            thread.abi = crate::task::abi::AbiPersonality::LinuxX86_64;
            thread.pd_shared = true;
            thread.cwd = parent_cwd;
            thread.linux_rootfs = parent_rootfs;
            thread.capabilities = parent_caps;
            thread.uid = parent_uid;
            thread.gid = parent_gid;
            thread.linux_real_uid = parent_linux_real_uid;
            thread.linux_saved_uid = parent_linux_saved_uid;
            thread.linux_fs_uid = parent_linux_fs_uid;
            thread.linux_real_gid = parent_linux_real_gid;
            thread.linux_saved_gid = parent_linux_saved_gid;
            thread.linux_fs_gid = parent_linux_fs_gid;
            thread.stdout_pipe = parent_stdout_pipe;
            thread.stdin_pipe = parent_stdin_pipe;
            thread.pty_id = parent_pty_id;
            thread.mmap_next = parent_mmap_next;
            thread.user_pages = parent_user_pages;
            thread.fd_table =
                alloc::sync::Arc::new(crate::sync::spinlock::Spinlock::new(parent_fd_table.clone()));
            let mut signals = parent_signals.clone();
            signals.pending = 0;
            thread.signals = signals;
            thread.args = parent_args;
            thread.fpu_state.data = parent_fpu_data;
            thread.linux_fs_base = regs.fs_base;
            thread.linux_clear_child_tid = clear_child_tid;
            true
        } else {
            false
        }
    };
    if !installed {
        super::kill_thread(tid);
        return 0;
    }

    incref_fd_table(&parent_fd_table);
    crate::task::loader::store_pending_fork(tid, regs);
    super::wake_thread(tid);
    tid
}

fn incref_fd_table(table: &FdTable) {
    for entry in table.iter_open() {
        match entry.kind {
            FdKind::File { global_id } => crate::fs::vfs::incref(global_id),
            FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::incref_read(pipe_id),
            FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::incref_write(pipe_id),
            FdKind::LinuxSocket { socket_id } => crate::syscall::linux::socket_incref(socket_id),
            FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxProc { .. }
            | FdKind::LinuxFramebuffer { .. }
            | FdKind::None => {}
        }
    }
}
