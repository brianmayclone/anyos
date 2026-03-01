//! Thread configuration: user info, arch mode, page directory, brk, mmap,
//! args, cwd, stdout/stdin pipes.

use super::{get_cpu_id, SCHEDULER};
use crate::fs::fd_table::FdKind;
use crate::memory::address::PhysAddr;
use crate::task::thread::ThreadState;

/// Configure a thread as a user process.
pub fn set_thread_user_info(tid: u32, pd: PhysAddr, brk: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(pd);
        #[cfg(target_arch = "x86_64")]
        {
            thread.pcid = crate::memory::virtual_mem::allocate_pcid();
            thread.context.set_page_table(pd.as_u64() | thread.pcid as u64);
        }
        #[cfg(target_arch = "aarch64")]
        thread.context.set_page_table(pd.as_u64());
        thread.context.checksum = thread.context.compute_checksum();
        thread.is_user = true;
        thread.brk = brk;
        // Reserve fd 0/1/2 as Tty so pipe()/open() start at fd 3
        thread.fd_table.alloc_at(0, FdKind::Tty);
        thread.fd_table.alloc_at(1, FdKind::Tty);
        thread.fd_table.alloc_at(2, FdKind::Tty);
    }
}

/// Set the architecture mode for a thread.
pub fn set_thread_arch_mode(tid: u32, mode: crate::task::thread::ArchMode) {
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.arch_mode = mode;
    }
}

/// Get the current thread's page directory.
pub fn current_thread_page_directory() -> Option<PhysAddr> {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].page_directory;
        }
    }
    None
}

/// Get the page directory for a thread by TID.
pub fn thread_page_directory(tid: u32) -> Option<PhysAddr> {
    let guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.find_idx(tid) {
            return sched.threads[idx].page_directory;
        }
    }
    None
}

/// Check if the current thread has a shared page directory.
pub fn current_thread_pd_shared() -> bool {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].pd_shared;
        }
    }
    false
}

/// Check if any OTHER live thread shares the same page directory.
pub fn has_live_pd_siblings() -> bool {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_HAS_LIVE_PD_SIBS);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let tid = sched.threads[idx].tid;
            if let Some(pd) = sched.threads[idx].page_directory {
                return sched.threads.iter().any(|t| {
                    t.tid != tid
                        && t.page_directory == Some(pd)
                        && t.state != ThreadState::Terminated
                });
            }
        }
    }
    false
}

/// Atomically get all info needed for sys_exit.
pub fn current_exit_info() -> (u32, Option<PhysAddr>, bool) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_CURRENT_EXIT_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let tid = sched.threads[idx].tid;
            let pd = sched.threads[idx].page_directory;
            let pd_shared = sched.threads[idx].pd_shared;
            let has_siblings = if let Some(pd_addr) = pd {
                pd_shared || sched.threads.iter().any(|t| {
                    t.tid != tid
                        && t.page_directory == Some(pd_addr)
                        && t.state != ThreadState::Terminated
                })
            } else { false };
            let can_destroy = pd.is_some() && !pd_shared && !has_siblings;
            return (tid, pd, can_destroy);
        }
    }
    (0, None, false)
}

/// Get the current thread's program break.
pub fn current_thread_brk() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].brk;
        }
    }
    0
}

/// Set the current thread's program break, syncing across sibling threads.
pub fn set_current_thread_brk(brk: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].brk = brk;
            if let Some(pd) = sched.threads[idx].page_directory {
                let current_tid = sched.threads[idx].tid;
                for thread in sched.threads.iter_mut() {
                    if thread.tid != current_tid && thread.page_directory == Some(pd) {
                        thread.brk = brk;
                    }
                }
            }
        }
    }
}

/// Return the current thread's mmap bump pointer.
pub fn current_thread_mmap_next() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_MMAP);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].mmap_next;
        }
    }
    0
}

/// Set the current thread's mmap bump pointer, syncing across sibling threads.
pub fn set_current_thread_mmap_next(val: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_MMAP);
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].mmap_next = val;
            if let Some(pd) = sched.threads[idx].page_directory {
                let current_tid = sched.threads[idx].tid;
                for thread in sched.threads.iter_mut() {
                    if thread.tid != current_tid && thread.page_directory == Some(pd) {
                        thread.mmap_next = val;
                    }
                }
            }
        }
    }
}

/// Set thread args.
pub fn set_thread_args(tid: u32, args: &str) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_ARGS);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = args.as_bytes();
        let len = bytes.len().min(255);
        thread.args[..len].copy_from_slice(&bytes[..len]);
        thread.args[len] = 0;
    }
}

/// Get the current thread's args.
pub fn current_thread_args(buf: &mut [u8]) -> usize {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let args = &sched.threads[idx].args;
            let len = args.iter().position(|&b| b == 0).unwrap_or(256);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&args[..copy_len]);
            return copy_len;
        }
    }
    0
}

/// Set the current working directory for a thread.
pub fn set_thread_cwd(tid: u32, cwd: &str) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_CWD);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = cwd.as_bytes();
        let len = bytes.len().min(511);
        thread.cwd[..len].copy_from_slice(&bytes[..len]);
        thread.cwd[len] = 0;
    }
}

/// Get the current working directory for the running thread.
pub fn current_thread_cwd(buf: &mut [u8]) -> usize {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let cwd = &sched.threads[idx].cwd;
            let len = cwd.iter().position(|&b| b == 0).unwrap_or(512);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&cwd[..copy_len]);
            return copy_len;
        }
    }
    0
}

/// Set the stdout pipe for a thread.
pub fn set_thread_stdout_pipe(tid: u32, pipe_id: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PIPE);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdout_pipe = pipe_id;
    }
}

/// Get the current thread's stdout pipe.
pub fn current_thread_stdout_pipe() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].stdout_pipe;
        }
    }
    0
}

/// Set the stdin pipe for a thread.
pub fn set_thread_stdin_pipe(tid: u32, pipe_id: u32) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PIPE);
    let mut guard = SCHEDULER.lock();
    let sched = guard.as_mut().expect("Scheduler not initialized");
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.stdin_pipe = pipe_id;
    }
}

/// Get the current thread's stdin pipe.
pub fn current_thread_stdin_pipe() -> u32 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].stdin_pipe;
        }
    }
    0
}
