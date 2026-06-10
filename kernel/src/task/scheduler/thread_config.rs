//! Thread configuration: user info, arch mode, page directory, brk, mmap,
//! args, cwd, stdout/stdin pipes.

use super::{get_cpu_id, SCHEDULER};
use crate::fs::fd_table::FdKind;
use crate::memory::address::PhysAddr;
use crate::task::abi::AbiPersonality;
use crate::task::thread::ThreadState;

/// Configure a thread as a user process.
pub fn set_thread_user_info(tid: u32, pd: PhysAddr, brk: u64) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_GET_THREAD_INFO);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.page_directory = Some(pd);
        #[cfg(target_arch = "x86_64")]
        {
            thread.pcid = crate::memory::virtual_mem::allocate_pcid();
            let pcid = thread.pcid as u64;
            thread.context.set_page_table(pd.as_u64() | pcid);
        }
        #[cfg(target_arch = "aarch64")]
        thread.context.set_page_table(pd.as_u64());
        thread.context.checksum = thread.context.compute_checksum();
        thread.is_user = true;
        thread.brk = brk;
        thread.brk_start = brk;
        // Reserve fd 0/1/2 so pipe()/open() start at fd 3.
        let stdio = if thread.pty_id != 0 {
            FdKind::PtySlave {
                pty_id: thread.pty_id,
            }
        } else {
            FdKind::Tty
        };
        {
            let mut fdt = thread.fd_table.lock();
            fdt.alloc_at(0, stdio);
            fdt.alloc_at(1, stdio);
            fdt.alloc_at(2, stdio);
        }
    }
}

/// Set the ABI personality for a thread.
pub fn set_thread_abi(tid: u32, abi: AbiPersonality) {
    let cpu_id = get_cpu_id();
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.abi = abi;
        if abi == AbiPersonality::LinuxX86_64
            && thread.priority > crate::task::abi::LINUX_MAX_USER_PRIORITY
        {
            crate::serial_verbose_println!(
                "lxe priority: clamp tid={} {} -> {}",
                tid,
                thread.priority,
                crate::task::abi::LINUX_MAX_USER_PRIORITY
            );
            thread.priority = crate::task::abi::LINUX_MAX_USER_PRIORITY;
        }
    }
    // If we just retagged the thread currently running on THIS CPU, refresh the
    // lock-free per-CPU cache so the next syscall dispatches under the new ABI
    // immediately, without waiting for a context switch to repopulate it.
    let is_current = match sched.current_idx(cpu_id) {
        Some(idx) => sched.threads[idx].tid == tid,
        None => false,
    };
    if is_current {
        super::store_per_cpu_abi(cpu_id, abi);
    }
}

/// Get the current thread's ABI personality.
///
/// This is the hottest branch in syscall dispatch.  The fast path is lock-free
/// but TID-validated: it trusts the per-CPU ABI cache only when the cached owner
/// matches the thread currently running on this CPU and stays stable for the
/// whole read.  On a miss we fall back to the scheduler lock and refresh the
/// cache while interrupts are still disabled, so the next syscall is cheap.
pub fn current_thread_abi() -> AbiPersonality {
    let flags = crate::arch::hal::save_and_disable_interrupts();
    let cpu_id = get_cpu_id();

    if let Some(abi) = super::load_per_cpu_abi_validated(cpu_id) {
        crate::arch::hal::restore_interrupt_state(flags);
        return abi;
    }

    let abi = {
        let guard = SCHEDULER.lock();
        if let Some(sched) = guard.as_ref() {
            if let Some(idx) = sched.current_idx(cpu_id) {
                sched.threads[idx].abi
            } else {
                AbiPersonality::AnyOs
            }
        } else {
            AbiPersonality::AnyOs
        }
    };
    super::store_per_cpu_abi(cpu_id, abi);
    crate::arch::hal::restore_interrupt_state(flags);
    abi
}

/// Set the lxe rootfs used by Linux ABI path translation.
pub fn set_thread_linux_rootfs(tid: u32, rootfs: &str) {
    let cpu_id = get_cpu_id();
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = rootfs.as_bytes();
        let len = bytes.len().min(511);
        thread.linux_rootfs = [0u8; 512];
        thread.linux_rootfs[..len].copy_from_slice(&bytes[..len]);
    }
    // If we just changed the rootfs of the thread running on THIS CPU, refresh
    // the lock-free per-CPU cache so the next path syscall sees the new prefix
    // without waiting for a context switch.
    if let Some(idx) = sched.current_idx(cpu_id) {
        if sched.threads[idx].tid == tid {
            let t = &sched.threads[idx];
            super::store_per_cpu_linux_rootfs(cpu_id, t.abi, &t.linux_rootfs);
        }
    }
}

/// Copy the current thread's lxe rootfs into `buf`.
///
/// NOTE: reverted to the lock-based read (see `current_thread_abi` above for
/// why the lock-free per-CPU cache read was unsafe without TID validation).
pub fn current_thread_linux_rootfs(buf: &mut [u8]) -> usize {
    // Fast path: lock-free per-CPU rootfs cache. It is refreshed on every
    // context switch (store_per_cpu_linux_rootfs in update_per_cpu_name) and
    // whenever the running thread retargets its rootfs. Reading it with
    // interrupts disabled pins us to the current CPU and the thread currently
    // running on it — ourselves — so the cached value is authoritative and no
    // scheduler lock is needed. Same lock-free pattern as current_thread_abi().
    // This matters because path translation calls this on EVERY filesystem
    // syscall (openat/stat/access/...); the global IRQ-off scheduler spinlock
    // here serialized all LXE file I/O against scheduling on every CPU.
    let flags = crate::arch::hal::save_and_disable_interrupts();
    let cpu_id = get_cpu_id();
    let len = super::load_per_cpu_linux_rootfs(cpu_id, buf);
    crate::arch::hal::restore_interrupt_state(flags);
    if len > 0 {
        return len;
    }

    // Slow path: cache reports no Linux rootfs (non-Linux thread, or rootfs
    // genuinely unset). Confirm under the lock so behavior is unchanged.
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            let rootfs = &sched.threads[idx].linux_rootfs;
            let len = rootfs.iter().position(|&b| b == 0).unwrap_or(512);
            let copy_len = len.min(buf.len());
            buf[..copy_len].copy_from_slice(&rootfs[..copy_len]);
            return copy_len;
        }
    }
    0
}

/// Set the current Linux thread's x86_64 FS base.
pub fn set_thread_linux_fs_base(tid: u32, fs_base: u64) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.linux_fs_base = fs_base;
    }
}

/// Set the current Linux thread's x86_64 FS base.
pub fn set_current_thread_linux_fs_base(fs_base: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].linux_fs_base = fs_base;
        }
    }
}

/// Read the current scheduler-selected thread's Linux FS base.
pub fn current_thread_linux_fs_base() -> u64 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].linux_fs_base;
        }
    }
    0
}

/// Set a thread's Windows x86_64 user GS base (TEB) used by wxe processes.
pub fn set_thread_windows_gs_base(tid: u32, gs_base: u64) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.windows_gs_base = gs_base;
    }
}

/// Read the current scheduler-selected thread's Windows GS base (TEB).
/// Returns 0 for non-Windows threads.
pub fn current_thread_windows_gs_base() -> u64 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].windows_gs_base;
        }
    }
    0
}

/// Set the Linux clear_child_tid pointer for a thread.
pub fn set_thread_linux_clear_child_tid(tid: u32, clear_child_tid: u64) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.linux_clear_child_tid = clear_child_tid;
    }
}

/// Set the current Linux thread's clear_child_tid pointer.
pub fn set_current_thread_linux_clear_child_tid(clear_child_tid: u64) {
    let mut guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_mut() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            sched.threads[idx].linux_clear_child_tid = clear_child_tid;
        }
    }
}

/// Read the current Linux thread's clear_child_tid pointer.
pub fn current_thread_linux_clear_child_tid() -> u64 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].linux_clear_child_tid;
        }
    }
    0
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

/// Return the page-table value the current thread should actually run with.
///
/// On x86_64 this includes the PCID bits stored in the saved context. Using the
/// bare PML4 physical address after execve would make the process run as PCID 0;
/// with no-flush context switches that can leave stale translations from another
/// address space visible under the same PCID.
#[cfg(target_arch = "x86_64")]
pub fn current_thread_context_page_table() -> Option<u64> {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return Some(sched.threads[idx].context.get_page_table());
        }
    }
    None
}

#[cfg(not(target_arch = "x86_64"))]
pub fn current_thread_context_page_table() -> Option<u64> {
    current_thread_page_directory().map(|pd| pd.as_u64())
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

/// Return a logical CPU mask for CPUs currently running threads that share the
/// current thread's page directory. The calling CPU may be present in the mask;
/// x86 TLB shootdown code skips it because local invalidation already happened.
pub fn current_pd_active_cpu_mask() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    let Some(sched) = guard.as_ref() else {
        return 0;
    };
    let Some(idx) = sched.current_idx(cpu_id) else {
        return 0;
    };
    let Some(pd) = sched.threads[idx].page_directory else {
        return 0;
    };

    let mut mask = 0u32;
    for cpu in 0..crate::arch::hal::MAX_CPUS.min(32) {
        let Some(tid) = sched.per_cpu[cpu].current_tid else {
            continue;
        };
        let Some(tidx) = sched.find_idx(tid) else {
            continue;
        };
        let thread = &sched.threads[tidx];
        if thread.state != ThreadState::Terminated && thread.page_directory == Some(pd) {
            mask |= 1u32 << cpu;
        }
    }
    mask
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
                pd_shared
                    || sched.threads.iter().any(|t| {
                        t.tid != tid
                            && t.page_directory == Some(pd_addr)
                            && t.state != ThreadState::Terminated
                    })
            } else {
                false
            };
            let can_destroy = pd.is_some() && !pd_shared && !has_siblings;
            return (tid, pd, can_destroy);
        }
    }
    (0, None, false)
}

/// Get the current thread's program break.
pub fn current_thread_brk() -> u64 {
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

/// Get the lowest legal program break for the current process image.
pub fn current_thread_brk_start() -> u64 {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].brk_start;
        }
    }
    0
}

/// Set the current thread's program break, syncing across sibling threads.
pub fn set_current_thread_brk(brk: u64) {
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

/// Set the initial/minimum program break for a thread.
pub fn set_thread_brk_start(tid: u32, brk_start: u64) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_BRK);
    let mut guard = SCHEDULER.lock();
    if let Some(sched) = guard.as_mut() {
        if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
            thread.brk_start = brk_start;
        }
    }
}

/// Return the current thread's mmap bump pointer.
pub fn current_thread_mmap_next() -> u64 {
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
pub fn set_current_thread_mmap_next(val: u64) {
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
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = args.as_bytes();
        let len = bytes.len().min(255);
        thread.args[..len].copy_from_slice(&bytes[..len]);
        thread.args[len] = 0;
    }
}

/// Set thread name.
pub fn set_thread_name(tid: u32, name: &str) {
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_ARGS);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        thread.name[..len].copy_from_slice(&bytes[..len]);
        thread.name[len] = 0;
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
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
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
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
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
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
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

/// Attach a PTY slave to fd 0/1/2 for a thread.
pub fn attach_thread_pty(tid: u32, pty_id: u32) {
    if pty_id == 0 {
        return;
    }
    crate::sched_diag::set(get_cpu_id(), crate::sched_diag::PHASE_SET_THREAD_PIPE);
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.pty_id = pty_id;
        let stdio = FdKind::PtySlave { pty_id };
        {
            let mut fdt = thread.fd_table.lock();
            fdt.alloc_at(0, stdio);
            fdt.alloc_at(1, stdio);
            fdt.alloc_at(2, stdio);
        }
    }
}

/// Record a PTY id without rewriting the fd table (used after fork fd cloning).
pub fn set_thread_pty_id(tid: u32, pty_id: u32) {
    let mut guard = SCHEDULER.lock();
    let sched = match guard.as_mut() {
        Some(s) => s,
        None => return,
    };
    if let Some(thread) = sched.threads.iter_mut().find(|t| t.tid == tid) {
        thread.pty_id = pty_id;
    }
}

/// Get the current thread's PTY id, or 0 if it has no PTY.
pub fn current_thread_pty_id() -> u32 {
    let guard = SCHEDULER.lock();
    let cpu_id = get_cpu_id();
    if let Some(sched) = guard.as_ref() {
        if let Some(idx) = sched.current_idx(cpu_id) {
            return sched.threads[idx].pty_id;
        }
    }
    0
}
