//! Process management syscall handlers.
//!
//! Covers process lifecycle (exit, kill, spawn, fork, exec),
//! scheduling (yield, sleep), memory (sbrk, mmap, munmap),
//! waiting (waitpid), and threading.

use alloc::string::String;
use super::helpers::{is_valid_user_ptr, read_user_str, read_user_str_safe, resolve_path};

/// sys_exit - Terminate the current process
pub fn sys_exit(status: u32) -> u32 {
    // Single atomic lock acquisition — no TOCTOU gaps between reads
    let (tid, pd, _) = crate::task::scheduler::current_exit_info();
    crate::debug_println!("sys_exit({}) TID={}", status, tid);

    // Close all open file descriptors and decref global resources.
    // Must happen before destroying the page directory.
    {
        use crate::fs::fd_table::FdKind;
        let closed = crate::task::scheduler::close_all_fds_for_thread(tid);
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

    // Clean up shared memory mappings while still in user PD context.
    // Must happen BEFORE switching CR3, so unmap_page operates on the
    // correct page tables via recursive mapping.
    if pd.is_some() {
        crate::ipc::shared_memory::cleanup_process(tid);
    }

    // Clean up TCP connections/listeners owned by this thread
    crate::net::tcp::cleanup_for_thread(tid);

    // Clean up environment variables for this process
    if let Some(pd_phys) = pd {
        crate::task::env::cleanup(pd_phys.as_u64());
    }

    // Switch to kernel CR3 before exit_current — the scheduler will destroy
    // the user page directory exclusively (in exit_current) after marking the
    // thread Terminated and setting page_directory = None.  Destroying here
    // would double-free the PML4 frame: exit_current still sees Some(pd) and
    // calls destroy_user_page_directory again, by which time the frame may
    // have been reallocated to a new process.
    if pd.is_some() {
        unsafe {
            let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3);
        }
    }

    crate::task::scheduler::exit_current(status);
    0 // unreachable
}

/// sys_kill - Send a signal to a thread.
/// If sig == 0: check existence only.
/// If sig == SIGKILL: force-kill (existing behavior).
/// Otherwise: set pending signal bit on the target thread.
pub fn sys_kill(tid: u32, sig: u32) -> u32 {
    if tid == 0 { return u32::MAX; }

    // Backward compat: old callers pass sig=0 (only one arg set), treat as SIGKILL
    let effective_sig = if sig == 0 { crate::ipc::signal::SIGKILL } else { sig };

    if effective_sig == crate::ipc::signal::SIGKILL {
        // Force-kill — existing behavior
        crate::debug_println!("sys_kill({}, SIGKILL)", tid);
        return crate::task::scheduler::kill_thread(tid);
    }

    // Check if thread exists
    if !crate::task::scheduler::thread_exists(tid) {
        return u32::MAX;
    }

    // Send the signal (set pending bit)
    crate::debug_println!("sys_kill({}, {})", tid, effective_sig);
    if crate::task::scheduler::send_signal_to_thread(tid, effective_sig) {
        0
    } else {
        u32::MAX
    }
}

/// sys_getpid - Get current process ID
pub fn sys_getpid() -> u32 {
    crate::task::scheduler::current_tid()
}

/// sys_getppid - Get parent process ID
pub fn sys_getppid() -> u32 {
    crate::task::scheduler::current_parent_tid()
}

/// sys_yield - Yield the CPU to another thread
pub fn sys_yield() -> u32 {
    crate::task::scheduler::schedule();
    0
}

/// sys_sleep - Sleep for N milliseconds (blocking sleep with timer wake).
/// The thread is blocked and does not consume CPU until the timer wakes it.
pub fn sys_sleep(ms: u32) -> u32 {
    if ms == 0 {
        return 0;
    }
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let ticks = (ms as u64 * pit_hz as u64 / 1000) as u32;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let now = crate::arch::x86::pit::get_ticks();
    let wake_at = now.wrapping_add(ticks);
    crate::task::scheduler::sleep_until(wake_at);
    0
}

/// sys_sbrk - Grow/shrink the process heap
pub fn sys_sbrk(increment: i32) -> u32 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    let old_brk = crate::task::scheduler::current_thread_brk();
    if old_brk == 0 {
        return u32::MAX;
    }
    if increment == 0 {
        return old_brk;
    }

    let page_size = 4096u32;

    if increment > 0 {
        let new_brk = old_brk + increment as u32;

        // Prevent heap from growing into the DLIB region (0x0400_0000 - 0x07FF_FFFF).
        // DLLs are demand-paged there; heap writes would corrupt their export tables.
        const DLIB_REGION_START: u32 = 0x0400_0000;
        if old_brk < DLIB_REGION_START && new_brk >= DLIB_REGION_START {
            return u32::MAX;
        }

        // Prevent heap from growing into the stack region.
        // Stack: USER_STACK_TOP (0x0C000000) - ASLR (up to 1 MiB) - 8 MiB.
        // Minimum stack bottom ≈ 0x0B700000.  Leave a 1 MiB guard gap so that
        // a large sbrk cannot silently overwrite saved registers / return
        // addresses on the stack (causes GPF with corrupted RIP).
        const HEAP_LIMIT: u32 = 0x0B60_0000;
        if new_brk >= HEAP_LIMIT {
            return u32::MAX;
        }
        let old_page_end = (old_brk + page_size - 1) & !(page_size - 1);
        let new_page_end = (new_brk + page_size - 1) & !(page_size - 1);

        let mut addr = old_page_end;
        let mut pages_mapped = 0u32;
        while addr < new_page_end {
            // Skip pages already mapped (another thread sharing this PD may have mapped them)
            if !virtual_mem::is_page_mapped(VirtAddr::new(addr as u64)) {
                if let Some(phys) = physical::alloc_frame() {
                    virtual_mem::map_page(VirtAddr::new(addr as u64), phys, 0x02 | 0x04);
                    unsafe { core::ptr::write_bytes(addr as *mut u8, 0, page_size as usize); }
                    pages_mapped += 1;
                } else {
                    return u32::MAX;
                }
            }
            addr += page_size;
        }

        if pages_mapped > 0 {
            crate::task::scheduler::adjust_current_user_pages(pages_mapped as i32);
        }

        // set_current_thread_brk also syncs brk across all sibling threads
        // sharing the same page directory (with cli to prevent timer deadlock).
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    } else {
        let decrement = (-increment) as u32;
        let new_brk = old_brk.saturating_sub(decrement);
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    }
}

/// sys_mmap - Map anonymous pages into user address space.
/// arg1=size (bytes, rounded up to page boundary). Returns virtual address or u32::MAX on error.
///
/// Allocates physical frames, maps them with PAGE_USER|PAGE_WRITABLE, zeroes them.
/// Virtual addresses are assigned from a per-process bump pointer starting at 0x20000000.
pub fn sys_mmap(size: u32) -> u32 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    if size == 0 {
        return u32::MAX;
    }

    const MMAP_BASE: u32 = 0x2000_0000;
    const MMAP_LIMIT: u32 = 0x4000_0000; // 512 MiB mmap region
    const PAGE_SIZE: u32 = 4096;

    let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let num_pages = aligned_size / PAGE_SIZE;

    let mmap_next = crate::task::scheduler::current_thread_mmap_next();
    let base = if mmap_next < MMAP_BASE { MMAP_BASE } else { mmap_next };

    if base.checked_add(aligned_size).map_or(true, |end| end > MMAP_LIMIT) {
        crate::serial_println!("sys_mmap: out of mmap virtual address space");
        return u32::MAX;
    }

    // Allocate and map pages
    let mut addr = base;
    for _ in 0..num_pages {
        if let Some(phys) = physical::alloc_frame() {
            virtual_mem::map_page(
                VirtAddr::new(addr as u64),
                phys,
                0x02 | 0x04, // PAGE_WRITABLE | PAGE_USER
            );
            unsafe { core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE as usize); }
        } else {
            // Out of physical memory — unmap what we already mapped
            let mut cleanup = base;
            while cleanup < addr {
                let pte = virtual_mem::read_pte(VirtAddr::new(cleanup as u64));
                if pte & 1 != 0 {
                    let phys_addr = crate::memory::address::PhysAddr::new(pte & 0x000F_FFFF_FFFF_F000);
                    virtual_mem::unmap_page(VirtAddr::new(cleanup as u64));
                    physical::free_frame(phys_addr);
                }
                cleanup += PAGE_SIZE;
            }
            crate::serial_println!("sys_mmap: out of physical memory");
            return u32::MAX;
        }
        addr += PAGE_SIZE;
    }

    crate::task::scheduler::adjust_current_user_pages(num_pages as i32);
    crate::task::scheduler::set_current_thread_mmap_next(base + aligned_size);

    base
}

/// sys_munmap - Unmap pages from user address space, freeing physical frames.
/// arg1=addr (must be page-aligned), arg2=size (bytes, rounded up to pages).
/// Returns 0 on success, u32::MAX on error.
pub fn sys_munmap(addr: u32, size: u32) -> u32 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    const MMAP_BASE: u32 = 0x2000_0000;
    const MMAP_LIMIT: u32 = 0x4000_0000;
    const PAGE_SIZE: u32 = 4096;

    if size == 0 || addr & (PAGE_SIZE - 1) != 0 {
        return u32::MAX;
    }

    // Validate the range is within the mmap region
    if addr < MMAP_BASE || addr >= MMAP_LIMIT {
        return u32::MAX;
    }
    let aligned_size = (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    if addr.checked_add(aligned_size).map_or(true, |end| end > MMAP_LIMIT) {
        return u32::MAX;
    }

    let num_pages = aligned_size / PAGE_SIZE;
    let mut freed = 0u32;
    let mut page_addr = addr;

    for _ in 0..num_pages {
        let pte = virtual_mem::read_pte(VirtAddr::new(page_addr as u64));
        if pte & 1 != 0 {
            let phys_addr = crate::memory::address::PhysAddr::new(pte & 0x000F_FFFF_FFFF_F000);
            virtual_mem::unmap_page(VirtAddr::new(page_addr as u64));
            physical::free_frame(phys_addr);
            freed += 1;
        }
        page_addr += PAGE_SIZE;
    }

    if freed > 0 {
        crate::task::scheduler::adjust_current_user_pages(-(freed as i32));
    }

    0
}

/// sys_waitpid - Wait for a process to exit. Returns exit code.
/// arg1 = tid (u32::MAX = -1 = wait for any child)
/// arg2 = child_tid_ptr (optional: if non-zero and tid==-1, write actual child TID here)
/// arg3 = options (bit 0 = WNOHANG)
pub fn sys_waitpid(tid: u32, child_tid_ptr: u32, options: u32) -> u32 {
    let wnohang = (options & 1) != 0;
    if tid == u32::MAX {
        // waitpid(-1): wait for any child
        let (child_tid, code) = if wnohang {
            crate::task::scheduler::try_waitpid_any()
        } else {
            crate::task::scheduler::waitpid_any()
        };
        // Write actual child TID to user pointer (if provided)
        if child_tid_ptr != 0 && child_tid != u32::MAX && child_tid != u32::MAX - 1 {
            if is_valid_user_ptr(child_tid_ptr as u64, 4) {
                unsafe { *(child_tid_ptr as *mut u32) = child_tid; }
            }
        }
        code
    } else if wnohang {
        crate::task::scheduler::try_waitpid(tid)
    } else {
        crate::task::scheduler::waitpid(tid)
    }
}

/// sys_try_waitpid - Non-blocking check if process exited.
/// Returns exit code if terminated, or u32::MAX-1 if still running.
pub fn sys_try_waitpid(tid: u32) -> u32 {
    crate::task::scheduler::try_waitpid(tid)
}

/// Sentinel return value from sys_spawn: the .app needs a permission dialog first.
const PERM_NEEDED: u32 = u32::MAX - 2;

/// sys_spawn - Spawn a new process from a filesystem path.
/// arg1=path_ptr, arg2=stdout_pipe_id (0=none), arg3=args_ptr (0=none), arg4=stdin_pipe_id (0=none)
/// Returns TID, u32::MAX on error, or PERM_NEEDED if a permission dialog is required.
pub fn sys_spawn(path_ptr: u32, stdout_pipe: u32, args_ptr: u32, stdin_pipe: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let args = if args_ptr != 0 {
        unsafe { read_user_str(args_ptr) }
    } else {
        ""
    };
    crate::debug_println!("sys_spawn: path='{}' pipe={} args_ptr={:#x} stdin_pipe={}", path, stdout_pipe, args_ptr, stdin_pipe);
    let raw_name = path.rsplit('/').next().unwrap_or(path);
    // Strip ".app" suffix so process name is clean (e.g. "Calculator" not "Calculator.app")
    let name = if raw_name.ends_with(".app") {
        &raw_name[..raw_name.len() - 4]
    } else {
        raw_name
    };

    // ── Runtime permission check for .app bundles ──
    if path.ends_with(".app") {
        if let Some(config) = crate::task::app_config::parse_info_conf(path) {
            let declared = if let Some(ref cap_str) = config.capabilities {
                crate::task::capabilities::parse_capabilities(cap_str)
            } else {
                crate::task::capabilities::CAP_DEFAULT
            };
            let sensitive_requested = declared & crate::task::capabilities::CAP_SENSITIVE;

            // System apps (capabilities=all) bypass the dialog entirely
            if declared != crate::task::capabilities::CAP_ALL && sensitive_requested != 0 {
                let uid = crate::task::scheduler::current_thread_uid();
                let app_id = config.id.as_deref().unwrap_or(name);

                if crate::task::permissions::read_stored_perms(uid, app_id).is_none() {
                    // No permission file — store pending info and return PERM_NEEDED
                    let app_name = config.name.as_deref().unwrap_or(name);
                    let caps_hex = alloc::format!("{:x}", declared);
                    // Format: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path"
                    let pending = alloc::format!(
                        "{}\x1F{}\x1F{}\x1F{}",
                        app_id, app_name, caps_hex, path
                    );
                    crate::task::scheduler::set_current_perm_pending(pending.as_bytes());
                    crate::serial_println!(
                        "sys_spawn: PERM_NEEDED for '{}' (app_id={}, caps={:#x})",
                        path, app_id, declared
                    );
                    return PERM_NEEDED;
                }
            } else if declared != crate::task::capabilities::CAP_ALL && sensitive_requested == 0 {
                // Only auto-granted caps — auto-create empty permission file
                let uid = crate::task::scheduler::current_thread_uid();
                let app_id = config.id.as_deref().unwrap_or(name);
                if crate::task::permissions::read_stored_perms(uid, app_id).is_none() {
                    crate::task::permissions::write_stored_perms(uid, app_id, 0);
                }
            }
        }
    }

    match crate::task::loader::load_and_run_with_args(path, name, args) {
        Ok(tid) => {
            // Inherit parent's cwd — but NOT for .app bundles, which already
            // had their CWD set from Info.conf inside load_and_run_with_args.
            if !path.ends_with(".app") {
                let mut cwd_buf = [0u8; 512];
                let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
                if cwd_len > 0 {
                    if let Ok(cwd) = core::str::from_utf8(&cwd_buf[..cwd_len]) {
                        crate::task::scheduler::set_thread_cwd(tid, cwd);
                    }
                }
            }
            if stdout_pipe != 0 {
                crate::task::scheduler::set_thread_stdout_pipe(tid, stdout_pipe);
            }
            if stdin_pipe != 0 {
                crate::task::scheduler::set_thread_stdin_pipe(tid, stdin_pipe);
            }
            // Inherit parent's environment variables
            if let Some(parent_pd) = crate::task::scheduler::current_thread_page_directory() {
                if let Some(child_pd) = crate::task::scheduler::thread_page_directory(tid) {
                    crate::task::env::clone_env(parent_pd.0, child_pd.0);
                }
            }
            crate::debug_println!("sys_spawn: returning TID={}", tid);
            tid
        }
        Err(e) => {
            crate::serial_println!("sys_spawn: FAILED: {}", e);
            u32::MAX
        }
    }
}

/// sys_getargs - Get command-line arguments for the current process.
/// arg1=buf_ptr, arg2=buf_size. Returns bytes written.
pub fn sys_getargs(buf_ptr: u32, buf_size: u32) -> u32 {
    if buf_ptr == 0 || buf_size == 0 || !is_valid_user_ptr(buf_ptr as u64, buf_size as u64) {
        return 0;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_size as usize) };
    crate::task::scheduler::current_thread_args(buf) as u32
}

/// sys_fork - Create a child process that is a copy of the parent.
/// Called with the full SyscallRegs frame so we can save/restore all user
/// registers in the child. The child returns 0, the parent returns child TID.
///
/// `regs` is the register frame pushed by the INT 0x80 or SYSCALL stub.
pub fn sys_fork(regs: &super::super::SyscallRegs) -> u32 {
    use crate::task::scheduler;
    use crate::task::loader::{ForkChildRegs, store_pending_fork, fork_child_trampoline};
    use crate::task::env;
    use crate::task::dll;
    use crate::memory::virtual_mem;

    // 1. Capture parent state in a single lock
    let snap = match scheduler::current_thread_fork_snapshot() {
        Some(s) => s,
        None => {
            crate::serial_println!("sys_fork: failed to snapshot current thread");
            return u32::MAX;
        }
    };

    let t_fork0 = crate::arch::x86::pit::get_ticks();
    let parent_tid = scheduler::current_tid();

    // 2. Clone user address space
    let child_pd = match virtual_mem::clone_user_page_directory(snap.pd) {
        Some(pd) => pd,
        None => {
            crate::serial_println!("sys_fork: clone_user_page_directory failed (OOM)");
            return u32::MAX;
        }
    };
    let t_fork_cloned = crate::arch::x86::pit::get_ticks();

    // 3. Build child name: "parent_name(fork)"
    let name_len = snap.name.iter().position(|&b| b == 0).unwrap_or(snap.name.len());
    let parent_name = core::str::from_utf8(&snap.name[..name_len]).unwrap_or("?");

    // 4. Spawn child thread in Blocked state (prevents SMP race)
    let child_tid = scheduler::spawn_blocked(
        fork_child_trampoline,
        snap.priority,
        parent_name,
    );

    // 5. Copy thread metadata to child
    scheduler::set_thread_user_info(child_tid, child_pd, snap.brk);

    // CWD
    let cwd_len = snap.cwd.iter().position(|&b| b == 0).unwrap_or(0);
    if cwd_len > 0 {
        if let Ok(cwd) = core::str::from_utf8(&snap.cwd[..cwd_len]) {
            scheduler::set_thread_cwd(child_tid, cwd);
        }
    }

    // Args
    let args_len = snap.args.iter().position(|&b| b == 0).unwrap_or(0);
    if args_len > 0 {
        if let Ok(args) = core::str::from_utf8(&snap.args[..args_len]) {
            scheduler::set_thread_args(child_tid, args);
        }
    }

    // Capabilities, identity
    scheduler::set_thread_capabilities(child_tid, snap.capabilities);
    scheduler::set_thread_identity(child_tid, snap.uid, snap.gid);

    // Pipes
    if snap.stdout_pipe != 0 {
        scheduler::set_thread_stdout_pipe(child_tid, snap.stdout_pipe);
    }
    if snap.stdin_pipe != 0 {
        scheduler::set_thread_stdin_pipe(child_tid, snap.stdin_pipe);
    }

    // FPU state, mmap_next, user_pages, arch_mode
    scheduler::set_thread_fpu_state(child_tid, &snap.fpu_data);
    scheduler::set_thread_mmap_next(child_tid, snap.mmap_next);
    scheduler::set_thread_user_pages(child_tid, snap.user_pages);
    scheduler::set_thread_arch_mode(child_tid, snap.arch_mode);

    // FD table: clone parent's table and incref all global resources
    {
        use crate::fs::fd_table::FdKind;
        let fd_table = snap.fd_table.clone();
        for entry in fd_table.iter_open() {
            match entry.kind {
                FdKind::File { global_id } => {
                    crate::fs::vfs::incref(global_id);
                }
                FdKind::PipeRead { pipe_id } => {
                    crate::ipc::anon_pipe::incref_read(pipe_id);
                }
                FdKind::PipeWrite { pipe_id } => {
                    crate::ipc::anon_pipe::incref_write(pipe_id);
                }
                FdKind::Tty | FdKind::None => {}
            }
        }
        scheduler::set_thread_fd_table(child_tid, fd_table);
    }

    // Child's parent_tid = the forking thread's TID (not grandparent)
    scheduler::set_thread_parent_tid(child_tid, scheduler::current_tid());
    // Inherit signal handlers and blocked mask from parent (pending cleared)
    scheduler::set_thread_signals(child_tid, snap.signals.clone());

    // 6. Clone environment
    env::clone_env(snap.pd.0, child_pd.0);

    // 7. Map DLLs into child address space (RO pages shared)
    dll::map_all_dlls_into(child_pd);

    // 8. Build ForkChildRegs from parent's register frame
    let child_regs = ForkChildRegs {
        rbx: regs.rbx,
        rcx: regs.rcx,
        rdx: regs.rdx,
        rsi: regs.rsi,
        rdi: regs.rdi,
        rbp: regs.rbp,
        r8: regs.r8,
        r9: regs.r9,
        r10: regs.r10,
        r11: regs.r11,
        r12: regs.r12,
        r13: regs.r13,
        r14: regs.r14,
        r15: regs.r15,
        // IRETQ frame from parent's interrupt/SYSCALL frame
        rip: regs.rip,
        cs: regs.cs,
        rflags: regs.rflags,
        rsp: regs.rsp,
        ss: regs.ss,
    };
    store_pending_fork(child_tid, child_regs);

    // 9. Wake child — it will run fork_child_trampoline and IRETQ with RAX=0
    scheduler::wake_thread(child_tid);
    let t_fork_total = crate::arch::x86::pit::get_ticks();
    crate::serial_println!("sys_fork: T{} → T{} clone={}ms total={}ms",
        parent_tid, child_tid,
        t_fork_cloned.wrapping_sub(t_fork0),
        t_fork_total.wrapping_sub(t_fork0));

    // Parent returns child TID
    child_tid
}

/// sys_exec - Replace the current process with a new program.
/// arg1 = path_ptr (null-terminated), arg2 = args_ptr (null-terminated, 0=none).
/// On success, never returns (process is replaced).
/// On failure, returns u32::MAX.
pub fn sys_exec(path_ptr: u32, args_ptr: u32) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });
    let args_str;
    let args = if args_ptr != 0 {
        args_str = unsafe { read_user_str(args_ptr) };
        args_str
    } else {
        ""
    };

    crate::serial_println!(
        "sys_exec: T{} path='{}' args='{}'",
        crate::task::scheduler::current_tid(), path, args
    );

    // Read the binary from the filesystem
    let data = match crate::fs::vfs::read_file_to_vec(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::serial_println!("sys_exec: read_file_to_vec('{}') failed: {:?}", path, e);
            return u32::MAX;
        }
    };

    // exec_current_process only returns on error (on success it jumps to user mode)
    let err = crate::task::loader::exec_current_process(&data, args);
    crate::serial_println!("sys_exec: FAILED: {}", err);
    u32::MAX
}

/// sys_thread_create - Create a new thread within the current process.
/// arg1=entry_rip, arg2=user_rsp, arg3=name_ptr, arg4=name_len, arg5=priority.
/// Returns TID of new thread, or 0 on error.
pub fn sys_thread_create(entry_rip: u32, user_rsp: u32, name_ptr: u32, name_len: u32, priority: u32) -> u32 {
    let entry = entry_rip as u64;
    let rsp = user_rsp as u64;

    // Basic validation: entry must be in user space, rsp must be in user space and aligned
    if entry == 0 || entry >= 0x0000_8000_0000_0000 {
        return 0;
    }
    if rsp == 0 || rsp >= 0x0000_8000_0000_0000 || rsp & 7 != 0 {
        return 0;
    }

    // Read thread name from user space (max 31 chars)
    let mut name_buf = [0u8; 32];
    let len = (name_len as usize).min(31);
    if name_ptr != 0 && len > 0 {
        let src = name_ptr as *const u8;
        // Validate pointer is in user space
        if (src as u64) < 0x0000_8000_0000_0000 {
            unsafe {
                core::ptr::copy_nonoverlapping(src, name_buf.as_mut_ptr(), len);
            }
        }
    }
    let name = core::str::from_utf8(&name_buf[..len]).unwrap_or("thread");

    // Priority: 0 means inherit from parent (handled by scheduler), 1-255 = explicit
    let pri = if priority > 0 && priority <= 255 { priority as u8 } else { 0 };

    let tid = crate::task::scheduler::create_thread_in_current_process(entry, rsp, name, pri);
    crate::debug_println!("sys_thread_create: entry={:#x} rsp={:#x} name={} pri={} -> TID={}", entry, rsp, name, pri, tid);
    tid
}

/// SYS_SET_PRIORITY: Change the priority of a thread.
/// arg1 = tid (0 = self), arg2 = new priority (1-255)
/// Returns 0 on success, u32::MAX on error.
pub fn sys_set_priority(tid: u32, priority: u32) -> u32 {
    if priority > 127 {
        return u32::MAX;
    }
    let target_tid = if tid == 0 {
        crate::task::scheduler::current_tid()
    } else {
        tid
    };
    if target_tid == 0 {
        return u32::MAX;
    }
    crate::task::scheduler::set_thread_priority(target_tid, priority as u8);
    0
}

/// SYS_SET_CRITICAL: Mark the calling thread as critical (won't be killed by RSP recovery).
pub fn sys_set_critical() -> u32 {
    let tid = crate::task::scheduler::current_tid();
    if tid == 0 {
        return u32::MAX;
    }
    crate::task::scheduler::set_thread_critical(tid);
    0
}
