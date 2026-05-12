//! Process management syscall handlers.
//!
//! Covers process lifecycle (exit, kill, spawn, fork, exec),
//! scheduling (yield, sleep), memory (sbrk, mmap, munmap),
//! waiting (waitpid), and threading.

#[allow(unused_imports)]
use super::helpers::{
    copy_user_bytes, is_user_range_accessible, is_valid_user_ptr, read_user_str,
    read_user_str_safe, resolve_path,
};
#[allow(unused_imports)]
use alloc::string::String;

#[inline(always)]
fn mmap_diag_mark(_mark: u8) {}

#[inline(always)]
fn mmap_diag_log(_label: &[u8], _tid: u32, _size: u32, _pd: u64, _addr: u32) {}

/// sys_exit - Terminate the current process
pub fn sys_exit(status: u32) -> u32 {
    // Single atomic lock acquisition — no TOCTOU gaps between reads
    let (tid, pd, _) = crate::task::scheduler::current_exit_info();
    crate::debug_println!("sys_exit({}) TID={}", status, tid);

    let clear_child_tid = crate::task::scheduler::current_thread_linux_clear_child_tid();
    if clear_child_tid != 0 && is_user_range_accessible(clear_child_tid, 4) {
        unsafe {
            *(clear_child_tid as *mut u32) = 0;
        }
    }

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
                FdKind::LinuxSocket { socket_id } => {
                    crate::syscall::linux::socket_decref(*socket_id);
                }
                FdKind::Tty | FdKind::PtySlave { .. } | FdKind::LinuxProc { .. } | FdKind::None => {
                }
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
    crate::net::udp::cleanup_for_thread(tid);

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
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let kernel_cr3 = crate::memory::virtual_mem::kernel_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) kernel_cr3);
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // ARM64: switch TTBR0_EL1 to kernel page table before destroying user PD
            let kernel_ttbr = crate::memory::virtual_mem::kernel_cr3();
            core::arch::asm!(
                "msr ttbr0_el1, {}",
                "isb",
                in(reg) kernel_ttbr,
                options(nostack),
            );
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
    if tid == 0 {
        return u32::MAX;
    }

    // Backward compat: old callers pass sig=0 (only one arg set), treat as SIGKILL
    let effective_sig = if sig == 0 {
        crate::ipc::signal::SIGKILL
    } else {
        sig
    };

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

/// sys_detach - Detach a child process so it is not cascade-killed on parent exit.
/// arg1 = child TID. Only the direct parent may detach a child.
/// Sets the child's parent_tid to 0 so it becomes a root process.
/// Returns 0 on success, u32::MAX if child not found or not owned.
pub fn sys_detach(child_tid: u32) -> u32 {
    let my_tid = crate::task::scheduler::current_tid();
    crate::task::scheduler::detach_child(my_tid, child_tid)
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
    let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
    let ticks = (ms as u64 * pit_hz as u64 / 1000) as u32;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let now = crate::arch::hal::timer_current_ticks();
    let wake_at = now.wrapping_add(ticks);
    crate::task::scheduler::sleep_until(wake_at);
    0
}

/// sys_sleep_us - Sleep for N microseconds.
///
/// For durations >= 1 ms, uses scheduler-based sleep (non-busy).
/// For sub-ms durations, uses TSC-based busy-wait for precision.
pub fn sys_sleep_us(us: u32) -> u32 {
    if us == 0 {
        return 0;
    }
    // For >= 1ms, use scheduler sleep (non-busy, efficient).
    if us >= 1000 {
        let ms = us / 1000;
        let pit_hz = crate::arch::hal::timer_frequency_hz() as u32;
        let ticks = (ms as u64 * pit_hz as u64 / 1000) as u32;
        let ticks = if ticks == 0 { 1 } else { ticks };
        let now = crate::arch::hal::timer_current_ticks();
        let wake_at = now.wrapping_add(ticks);
        crate::task::scheduler::sleep_until(wake_at);
        // Handle remaining sub-ms fraction.
        let remainder_us = us % 1000;
        if remainder_us > 0 {
            busy_wait_us(remainder_us);
        }
    } else {
        // Sub-ms: TSC busy-wait.
        busy_wait_us(us);
    }
    0
}

/// TSC-based busy-wait for microsecond precision.
fn busy_wait_us(us: u32) {
    #[cfg(target_arch = "x86_64")]
    {
        let tsc_hz = crate::arch::x86::pit::tsc_hz();
        if tsc_hz == 0 {
            // No TSC calibration — fall back to yield.
            crate::task::scheduler::schedule();
            return;
        }
        let wait_cycles = us as u64 * tsc_hz / 1_000_000;
        let start = crate::arch::x86::pit::rdtsc();
        while crate::arch::x86::pit::rdtsc().wrapping_sub(start) < wait_cycles {
            core::hint::spin_loop();
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // AArch64: just yield — no TSC.
        let _ = us;
        crate::task::scheduler::schedule();
    }
}

/// sys_sbrk_u64 - Grow/shrink the process heap (full 64-bit ABI).
///
/// This is the SYS_SBRK entry point used by syscall_dispatch_64. brk
/// addresses can live anywhere in the canonical-low half (including
/// above 4 GiB, once programs are relinked into the upper half).
pub fn sys_sbrk_u64(increment: i64) -> u64 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::user_vmap::{DLIB_REGION_START, HEAP_LIMIT};
    use crate::memory::virtual_mem;

    let old_brk = crate::task::scheduler::current_thread_brk();
    if old_brk == 0 {
        if crate::task::scheduler::current_thread_abi()
            == crate::task::abi::AbiPersonality::LinuxX86_64
        {
            crate::serial_verbose_println!("lxe linux sbrk: reject old_brk=0 inc={}", increment);
        }
        return u64::MAX;
    }
    if increment == 0 {
        return old_brk;
    }

    const PAGE_SIZE: u64 = 4096;

    if increment > 0 {
        let new_brk = match old_brk.checked_add(increment as u64) {
            Some(v) => v,
            None => {
                if crate::task::scheduler::current_thread_abi()
                    == crate::task::abi::AbiPersonality::LinuxX86_64
                {
                    crate::serial_verbose_println!(
                        "lxe linux sbrk: reject overflow old={:#x} inc={}",
                        old_brk,
                        increment
                    );
                }
                return u64::MAX;
            }
        };

        if old_brk < HEAP_LIMIT {
            // Prevent low legacy heaps from growing into the DLIB region.
            // DLLs are demand-paged there; heap writes would corrupt their
            // export tables.
            if old_brk < DLIB_REGION_START && new_brk >= DLIB_REGION_START {
                if crate::task::scheduler::current_thread_abi()
                    == crate::task::abi::AbiPersonality::LinuxX86_64
                {
                    crate::serial_verbose_println!(
                        "lxe linux sbrk: reject dlib-cross old={:#x} new={:#x} inc={}",
                        old_brk,
                        new_brk,
                        increment
                    );
                }
                return u64::MAX;
            }

            // Prevent low legacy heaps from growing into the mmap region
            // (guard gap). Linux PIE/ET_DYN binaries have a high brk and are
            // handled by the high-heap guard below.
            if new_brk >= HEAP_LIMIT {
                if crate::task::scheduler::current_thread_abi()
                    == crate::task::abi::AbiPersonality::LinuxX86_64
                {
                    crate::serial_verbose_println!(
                        "lxe linux sbrk: reject low-limit old={:#x} new={:#x} inc={} limit={:#x}",
                        old_brk,
                        new_brk,
                        increment,
                        HEAP_LIMIT
                    );
                }
                return u64::MAX;
            }
        } else {
            // Linux ET_DYN/PIE binaries are loaded high in the canonical-low
            // half. Their brk must be allowed to grow there, but stop before
            // the fixed dynamic-loader area at 0x7000_0000_0000.
            const HIGH_HEAP_LIMIT: u64 = 0x0000_7000_0000_0000;
            if new_brk >= HIGH_HEAP_LIMIT {
                if crate::task::scheduler::current_thread_abi()
                    == crate::task::abi::AbiPersonality::LinuxX86_64
                {
                    crate::serial_verbose_println!(
                        "lxe linux sbrk: reject high-limit old={:#x} new={:#x} inc={} limit={:#x}",
                        old_brk,
                        new_brk,
                        increment,
                        HIGH_HEAP_LIMIT
                    );
                }
                return u64::MAX;
            }
        }

        let old_page_end = (old_brk + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let new_page_end = (new_brk + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        let mut addr = old_page_end;
        let mut pages_mapped = 0u32;
        while addr < new_page_end {
            // Skip pages already mapped (another thread sharing this PD
            // may have mapped them).
            if !virtual_mem::is_page_mapped(VirtAddr::new(addr)) {
                if let Some(phys) = physical::alloc_frame_with(physical::FrameAllocPolicy::Any) {
                    if !virtual_mem::zero_frame(phys) {
                        physical::free_frame(phys);
                        if crate::task::scheduler::current_thread_abi()
                            == crate::task::abi::AbiPersonality::LinuxX86_64
                        {
                            crate::serial_verbose_println!(
                                "lxe linux sbrk: reject zero-frame old={:#x} new={:#x} addr={:#x}",
                                old_brk,
                                new_brk,
                                addr
                            );
                        }
                        return u64::MAX;
                    }
                    if !virtual_mem::map_page(VirtAddr::new(addr), phys, 0x02 | 0x04) {
                        physical::free_frame(phys);
                        if crate::task::scheduler::current_thread_abi()
                            == crate::task::abi::AbiPersonality::LinuxX86_64
                        {
                            crate::serial_verbose_println!(
                                "lxe linux sbrk: reject map-page old={:#x} new={:#x} addr={:#x}",
                                old_brk,
                                new_brk,
                                addr
                            );
                        }
                        return u64::MAX;
                    }
                    pages_mapped += 1;
                } else {
                    if crate::task::scheduler::current_thread_abi()
                        == crate::task::abi::AbiPersonality::LinuxX86_64
                    {
                        crate::serial_verbose_println!(
                            "lxe linux sbrk: reject phys-oom old={:#x} new={:#x} addr={:#x}",
                            old_brk,
                            new_brk,
                            addr
                        );
                    }
                    return u64::MAX;
                }
            }
            addr += PAGE_SIZE;
        }

        if pages_mapped > 0 {
            crate::task::scheduler::adjust_current_user_pages(pages_mapped as i32);
        }

        // set_current_thread_brk also syncs brk across all sibling
        // threads sharing the same page directory.
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    } else {
        let decrement = (-increment) as u64;
        let new_brk = old_brk.saturating_sub(decrement);
        crate::task::scheduler::set_current_thread_brk(new_brk);
        old_brk
    }
}

/// sys_sbrk - Legacy u32 ABI (kept for in-kernel call sites that go
/// through dispatch_inner directly). Truncates to 32 bits — only valid
/// while user-space brk lives below 4 GiB.
pub fn sys_sbrk(increment: i32) -> u32 {
    let r = sys_sbrk_u64(increment as i64);
    if r == u64::MAX {
        u32::MAX
    } else {
        r as u32
    }
}

/// sys_mmap - Map anonymous pages into user address space.
/// arg1=size (bytes, rounded up to page boundary). Returns virtual address or u32::MAX on error.
///
/// Allocates physical frames, maps them with PAGE_USER|PAGE_WRITABLE, zeroes them.
/// Virtual addresses are found via the per-process VMA gap-finder, which reuses
/// freed regions (unlike the old bump-pointer allocator).
///
/// NOTE: kept for the legacy in-kernel u32-only call sites. The 64-bit
/// SYSCALL ABI for SYS_MMAP routes through `sys_mmap_u64` instead.
pub fn sys_mmap(size: u32) -> u32 {
    let addr = sys_mmap_impl(size as u64, false);
    if addr > u32::MAX as u64 {
        u32::MAX
    } else {
        addr as u32
    }
}

/// sys_mmap_u64 - Map anonymous pages, returning a full 64-bit user address.
///
/// This is the SYS_MMAP entry point used by syscall_dispatch_64. It routes
/// through the high-VA allocator (MMAP64 region, above 4 GiB) so the result
/// always fits in u64 but never collides with the legacy 32-bit
/// PROGRAM_LOAD_ADDR / DLIB region. Callers must propagate the address as
/// u64 throughout (libsyscall::mmap and stdlib::process::mmap).
pub fn sys_mmap_u64(size: u64) -> u64 {
    sys_mmap_impl(size, true)
}

/// sys_munmap_u64 - Unmap pages mapped via sys_mmap_u64 (high-VA region).
pub fn sys_munmap_u64(addr: u64, size: u64) -> u64 {
    sys_munmap_impl(addr, size, true)
}

/// sys_mmap64 - Map anonymous pages above 4 GiB for native 64-bit callers.
///
/// This is intentionally separate from `sys_mmap`: many existing syscalls still
/// take user pointers as `u32`, so the general allocator must keep returning
/// low addresses. Large native-only buffers, such as ASL guest RAM, can use this
/// high-VA path and pass the resulting pointer to 64-bit-aware syscalls.
pub fn sys_mmap64(size: u64) -> u64 {
    sys_mmap_impl(size, true)
}

fn sys_mmap_impl(size: u64, high: bool) -> u64 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    mmap_diag_mark(b'A');

    if size == 0 {
        mmap_diag_mark(b'Z');
        return u64::MAX;
    }

    const PAGE_SIZE: u64 = 4096;

    let aligned_size = match size.checked_add(PAGE_SIZE - 1) {
        Some(v) => v & !(PAGE_SIZE - 1),
        None => return u64::MAX,
    };
    let num_pages = aligned_size / PAGE_SIZE;
    let tid = crate::task::scheduler::debug_current_tid();
    mmap_diag_mark(b'B');

    // Find a free gap in the mmap virtual address space via the VMA registry.
    let pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd,
        None => {
            crate::serial_verbose_println!("sys_mmap: no page directory for current thread");
            mmap_diag_log(b"!mm-no-pd", tid, aligned_size as u32, 0, u32::MAX);
            return u64::MAX;
        }
    };
    mmap_diag_mark(b'C');
    let base = match if high {
        crate::memory::vma::alloc_region64(pd, aligned_size)
    } else {
        crate::memory::vma::alloc_region(pd, aligned_size as u32).map(|addr| addr as u64)
    } {
        Some(addr) => addr,
        None => {
            crate::serial_verbose_println!("sys_mmap: out of mmap virtual address space");
            mmap_diag_log(
                b"!mm-vma-full",
                tid,
                aligned_size as u32,
                pd.as_u64(),
                u32::MAX,
            );
            return u64::MAX;
        }
    };
    mmap_diag_mark(b'D');
    let valid_range = if high {
        base >= 0x0000_0001_0000_0000
            && base
                .checked_add(aligned_size)
                .map_or(false, |end| end <= 0x0000_4000_0000_0000)
    } else {
        base >= 0x7000_0000
            && base
                .checked_add(aligned_size)
                .map_or(false, |end| end <= 0xBF00_0000)
    };
    if !valid_range {
        crate::serial_println!(
            "sys_mmap: BUG: VMA returned invalid range base={:#x} size={:#x} pd={:#x}",
            base,
            aligned_size,
            pd.as_u64()
        );
        crate::memory::vma::free_region64(pd, base, aligned_size);
        mmap_diag_log(
            b"!mm-vma-bad",
            tid,
            aligned_size as u32,
            pd.as_u64(),
            base as u32,
        );
        return u64::MAX;
    }

    // Allocate and map physical pages.
    let mut addr = base;
    for _ in 0..num_pages {
        if let Some(phys) = physical::alloc_frame_with(physical::FrameAllocPolicy::Any) {
            if !virtual_mem::zero_frame(phys) {
                physical::free_frame(phys);
                let mut cleanup = base;
                while cleanup < addr {
                    let pte = virtual_mem::read_pte(VirtAddr::new(cleanup as u64));
                    if pte & 1 != 0 {
                        let phys_addr =
                            crate::memory::address::PhysAddr::new(pte & 0x000F_FFFF_FFFF_F000);
                        virtual_mem::unmap_page(VirtAddr::new(cleanup as u64));
                        physical::free_frame(phys_addr);
                    }
                    cleanup += PAGE_SIZE;
                }
                crate::memory::vma::free_region64(pd, base, aligned_size);
                return u64::MAX;
            }
            if virtual_mem::map_page(
                VirtAddr::new(addr as u64),
                phys,
                0x02 | 0x04, // PAGE_WRITABLE | PAGE_USER
            ) {
            } else {
                physical::free_frame(phys);
                let mut cleanup = base;
                while cleanup < addr {
                    let pte = virtual_mem::read_pte(VirtAddr::new(cleanup as u64));
                    if pte & 1 != 0 {
                        let phys_addr =
                            crate::memory::address::PhysAddr::new(pte & 0x000F_FFFF_FFFF_F000);
                        virtual_mem::unmap_page(VirtAddr::new(cleanup as u64));
                        physical::free_frame(phys_addr);
                    }
                    cleanup += PAGE_SIZE;
                }
                crate::memory::vma::free_region64(pd, base, aligned_size);
                crate::serial_println!(
                    "sys_mmap: map_page failed base={:#x} addr={:#x} size={:#x} pd={:#x}",
                    base,
                    addr,
                    aligned_size,
                    pd.as_u64()
                );
                mmap_diag_log(
                    b"!mm-map-fail",
                    tid,
                    aligned_size as u32,
                    pd.as_u64(),
                    addr as u32,
                );
                return u64::MAX;
            }
        } else {
            // Out of physical memory — unmap what we already mapped and free VMA.
            let mut cleanup = base;
            while cleanup < addr {
                let pte = virtual_mem::read_pte(VirtAddr::new(cleanup as u64));
                if pte & 1 != 0 {
                    let phys_addr =
                        crate::memory::address::PhysAddr::new(pte & 0x000F_FFFF_FFFF_F000);
                    virtual_mem::unmap_page(VirtAddr::new(cleanup as u64));
                    physical::free_frame(phys_addr);
                }
                cleanup += PAGE_SIZE;
            }
            crate::memory::vma::free_region64(pd, base, aligned_size);
            crate::serial_verbose_println!("sys_mmap: out of physical memory");
            mmap_diag_log(
                b"!mm-phys-oom",
                tid,
                aligned_size as u32,
                pd.as_u64(),
                addr as u32,
            );
            return u64::MAX;
        }
        addr += PAGE_SIZE;
    }

    mmap_diag_mark(b'E');
    crate::task::scheduler::adjust_current_user_pages(num_pages as i32);
    // Keep mmap_next in sync (threads sharing this PD read it).
    if !high {
        crate::task::scheduler::set_current_thread_mmap_next(base + aligned_size);
    }
    mmap_diag_mark(b'F');
    mmap_diag_log(
        b"+mm-ok",
        tid,
        aligned_size as u32,
        pd.as_u64(),
        base as u32,
    );

    // No TLB shootdown needed for mmap: x86-64 does not cache non-present
    // TLB entries (Intel SDM Vol. 3A §4.10.2.1).  When a sibling thread on
    // another CPU accesses a newly mapped address, the hardware page walker
    // will find the freshly installed PTE.  Only munmap (present→not-present
    // transitions) requires a remote TLB flush.

    base
}

/// sys_munmap - Unmap pages from user address space, freeing physical frames.
/// arg1=addr (must be page-aligned), arg2=size (bytes, rounded up to pages).
/// Returns 0 on success, u32::MAX on error.
pub fn sys_munmap(addr: u32, size: u32) -> u32 {
    if sys_munmap_impl(addr as u64, size as u64, false) == 0 {
        0
    } else {
        u32::MAX
    }
}

/// sys_munmap64 - Unmap pages from the native 64-bit high mmap region.
pub fn sys_munmap64(addr: u64, size: u64) -> u64 {
    sys_munmap_impl(addr, size, true)
}

fn sys_munmap_impl(addr: u64, size: u64, high: bool) -> u64 {
    use crate::memory::address::VirtAddr;
    use crate::memory::physical;
    use crate::memory::virtual_mem;

    use crate::memory::user_vmap::{MMAP64_BASE, MMAP_BASE, MMAP_LIMIT};
    const MMAP64_LIMIT: u64 = 0x0000_4000_0000_0000;
    const PAGE_SIZE: u64 = 4096;

    if size == 0 || addr & (PAGE_SIZE - 1) != 0 {
        return u64::MAX;
    }

    // Validate the range is within the mmap region
    let (range_base, range_limit) = if high {
        (MMAP64_BASE, MMAP64_LIMIT)
    } else {
        (MMAP_BASE, MMAP_LIMIT)
    };
    if addr < range_base || addr >= range_limit {
        return u64::MAX;
    }
    let aligned_size = match size.checked_add(PAGE_SIZE - 1) {
        Some(v) => v & !(PAGE_SIZE - 1),
        None => return u64::MAX,
    };
    if addr
        .checked_add(aligned_size)
        .map_or(true, |end| end > range_limit)
    {
        return u64::MAX;
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
        virtual_mem::reclaim_empty_user_tables(VirtAddr::new(addr as u64), aligned_size as u64);
    }

    if freed > 0 {
        crate::task::scheduler::adjust_current_user_pages(-(freed as i32));
    }

    // Update VMA bookkeeping so the freed virtual addresses can be reused.
    if let Some(pd) = crate::task::scheduler::current_thread_page_directory() {
        crate::memory::vma::free_region64(pd, addr, aligned_size);
    }

    // Only shared address spaces need a remote shootdown. `unmap_page()`
    // already invalidates the local CPU's TLB entry, and CPUs running other
    // page directories cannot legally use this translation. kstress is
    // typically single-threaded here, so an unconditional global shootdown on
    // every munmap needlessly drives the most fragile SMP path and can wedge
    // the whole machine if one CPU misses the ack.
    #[cfg(target_arch = "x86_64")]
    if freed > 0 && crate::task::scheduler::has_live_pd_siblings() {
        let cpu_mask = crate::task::scheduler::current_pd_active_cpu_mask();
        crate::arch::x86::smp::tlb_shootdown_mask(u64::MAX, cpu_mask);
    }

    0
}

/// sys_waitpid - Wait for a process to exit. Returns exit code.
/// arg1 = tid (u32::MAX = -1 = wait for any child)
/// arg2 = child_tid_ptr (optional: if non-zero and tid==-1, write actual child TID here)
/// arg3 = options (bit 0 = WNOHANG)
pub fn sys_waitpid(tid: u32, child_tid_ptr: u64, options: u32) -> u32 {
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
                unsafe {
                    *(child_tid_ptr as *mut u32) = child_tid;
                }
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

fn attach_stdio_pty(tid: u32, stdout_pipe: u32, stdin_pipe: u32) {
    if stdout_pipe == 0 || stdin_pipe == 0 {
        return;
    }
    let pty_id = crate::ipc::pty::create(stdin_pipe, stdout_pipe);
    if pty_id != 0 {
        crate::task::scheduler::attach_thread_pty(tid, pty_id);
    }
}

fn inherit_or_attach_stdio_pty(tid: u32, stdout_pipe: u32, stdin_pipe: u32) {
    let inherited = crate::task::scheduler::current_thread_pty_id();
    if inherited != 0 {
        crate::task::scheduler::attach_thread_pty(tid, inherited);
    } else {
        attach_stdio_pty(tid, stdout_pipe, stdin_pipe);
    }
}

/// sys_spawn - Spawn a new process from a filesystem path.
/// arg1=path_ptr, arg2=stdout_pipe_id (0=none), arg3=args_ptr (0=none), arg4=stdin_pipe_id (0=none)
/// Returns TID, u32::MAX on error, or PERM_NEEDED if a permission dialog is required.
pub fn sys_spawn(path_ptr: u64, stdout_pipe: u32, args_ptr: u64, stdin_pipe: u32) -> u32 {
    let path = unsafe { read_user_str(path_ptr) };
    let args = if args_ptr != 0 {
        unsafe { read_user_str(args_ptr) }
    } else {
        ""
    };
    crate::debug_println!(
        "sys_spawn: path='{}' pipe={} args_ptr={:#x} stdin_pipe={}",
        path,
        stdout_pipe,
        args_ptr,
        stdin_pipe
    );
    let raw_name = path.rsplit('/').next().unwrap_or(path);
    // Strip ".app" suffix so process name is clean (e.g. "Calculator" not "Calculator.app")
    let name = if raw_name.ends_with(".app") {
        &raw_name[..raw_name.len() - 4]
    } else {
        raw_name
    };

    // ── .app bundles may only be spawned by the Sessionhost ──
    if path.ends_with(".app") {
        let caller_tid = crate::task::scheduler::current_tid();
        // Allow: Sessionhost, init (tid 1), and the compositor (system services)
        let is_allowed =
            caller_tid == 1 || super::is_sessionhost(caller_tid) || super::is_compositor();
        if !is_allowed {
            crate::serial_verbose_println!(
                "sys_spawn: DENIED .app spawn from TID {} (not sessionhost) path='{}'",
                caller_tid,
                path
            );
            return u32::MAX;
        }
    }

    // ── Runtime permission check for .app bundles ──
    // On ISO 9660 (live-CD) boot the root filesystem is read-only, so permission
    // files cannot be persisted.  Skip the dialog and auto-grant all declared
    // capabilities — the user already chose to boot the live image.
    let live_cd = crate::fs::vfs::root_is_iso9660();

    if path.ends_with(".app") {
        if let Some(config) = crate::task::app_config::parse_info_conf(path) {
            let declared = if let Some(ref cap_str) = config.capabilities {
                crate::task::capabilities::parse_capabilities(cap_str)
            } else {
                crate::task::capabilities::CAP_DEFAULT
            };
            let sensitive_requested = declared & crate::task::capabilities::CAP_SENSITIVE;

            // System apps (capabilities=all) bypass the dialog entirely
            if declared != crate::task::capabilities::CAP_ALL
                && sensitive_requested != 0
                && !live_cd
            {
                let uid = crate::task::scheduler::current_thread_uid();
                let app_id = config.id.as_deref().unwrap_or(name);

                if crate::task::permissions::read_stored_perms(uid, app_id).is_none() {
                    // No permission file — store pending info and return PERM_NEEDED
                    let app_name = config.name.as_deref().unwrap_or(name);
                    let caps_hex = alloc::format!("{:x}", declared);
                    // Format: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path"
                    let pending =
                        alloc::format!("{}\x1F{}\x1F{}\x1F{}", app_id, app_name, caps_hex, path);
                    crate::task::scheduler::set_current_perm_pending(pending.as_bytes());
                    crate::serial_verbose_println!(
                        "sys_spawn: PERM_NEEDED for '{}' (app_id={}, caps={:#x})",
                        path,
                        app_id,
                        declared
                    );
                    return PERM_NEEDED;
                }
            } else if declared != crate::task::capabilities::CAP_ALL
                && sensitive_requested == 0
                && !live_cd
            {
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
            attach_stdio_pty(tid, stdout_pipe, stdin_pipe);
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
            crate::serial_verbose_println!("sys_spawn: FAILED path='{}': {}", path, e);
            u32::MAX
        }
    }
}

/// sys_lxe_spawn - Spawn a Linux x86_64 process through lxe.
/// arg1=path_ptr, arg2=args_ptr (0=none). Returns TID or u32::MAX on error.
pub fn sys_lxe_spawn(path_ptr: u64, args_ptr: u64) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(path) if !path.is_empty() => path,
        _ => return u32::MAX,
    };
    let args = if args_ptr != 0 {
        read_user_str_safe(args_ptr).unwrap_or("")
    } else {
        ""
    };
    let name = path.rsplit('/').next().unwrap_or("linux");

    match crate::task::loader::load_and_run_linux_with_args(path, name, args) {
        Ok(tid) => {
            let stdout_pipe = crate::task::scheduler::current_thread_stdout_pipe();
            let stdin_pipe = crate::task::scheduler::current_thread_stdin_pipe();
            if stdout_pipe != 0 {
                crate::task::scheduler::set_thread_stdout_pipe(tid, stdout_pipe);
            }
            if stdin_pipe != 0 {
                crate::task::scheduler::set_thread_stdin_pipe(tid, stdin_pipe);
            }
            inherit_or_attach_stdio_pty(tid, stdout_pipe, stdin_pipe);
            crate::task::scheduler::set_thread_cwd(tid, "/");
            if let Some(parent_pd) = crate::task::scheduler::current_thread_page_directory() {
                if let Some(child_pd) = crate::task::scheduler::thread_page_directory(tid) {
                    crate::task::env::clone_env(parent_pd.0, child_pd.0);
                }
            }
            tid
        }
        Err(e) => {
            crate::serial_verbose_println!("sys_lxe_spawn: FAILED path='{}': {}", path, e);
            u32::MAX
        }
    }
}

/// sys_getargs - Get command-line arguments for the current process.
/// arg1=buf_ptr, arg2=buf_size. Returns bytes written.
pub fn sys_getargs(buf_ptr: u64, buf_size: u32) -> u32 {
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
///
/// Currently x86_64-only: relies on IRETQ to restore the child's register
/// state. ARM64 fork will use ERET with a separate register save/restore path.
#[cfg(target_arch = "x86_64")]
pub fn sys_fork(regs: &super::super::SyscallRegs) -> u32 {
    sys_fork_with_child_tidptr(regs, 0, 0)
}

#[cfg(target_arch = "x86_64")]
pub fn sys_fork_with_child_tidptr(
    regs: &super::super::SyscallRegs,
    child_tidptr: u64,
    clear_child_tidptr: u64,
) -> u32 {
    use crate::memory::virtual_mem;
    use crate::task::dll;
    use crate::task::env;
    use crate::task::loader::{fork_child_trampoline, store_pending_fork, ForkChildRegs};
    use crate::task::scheduler;

    // 1. Capture parent state in a single lock
    let snap = match scheduler::current_thread_fork_snapshot() {
        Some(s) => s,
        None => {
            crate::serial_verbose_println!("sys_fork: failed to snapshot current thread");
            return u32::MAX;
        }
    };

    let t_fork0 = crate::arch::hal::timer_current_ticks();
    let parent_tid = scheduler::current_tid();

    // 2. Clone user address space
    let child_pd = match if snap.abi == crate::task::abi::AbiPersonality::LinuxX86_64 {
        virtual_mem::clone_user_page_directory_no_low_identity(snap.pd)
    } else {
        virtual_mem::clone_user_page_directory(snap.pd)
    } {
        Some(pd) => pd,
        None => {
            crate::serial_verbose_println!("sys_fork: clone_user_page_directory failed (OOM)");
            return u32::MAX;
        }
    };
    let t_fork_cloned = crate::arch::hal::timer_current_ticks();

    // Clone VMA table from parent to child process.
    crate::memory::vma::clone_for_fork(snap.pd, child_pd);

    // 3. Build child name: "parent_name(fork)"
    let name_len = snap
        .name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(snap.name.len());
    let parent_name = core::str::from_utf8(&snap.name[..name_len]).unwrap_or("?");

    // 4. Spawn child thread in Blocked state (prevents SMP race)
    let child_tid = scheduler::spawn_blocked(fork_child_trampoline, snap.priority, parent_name);
    if child_tid == 0 {
        crate::memory::virtual_mem::destroy_user_page_directory(child_pd);
        crate::memory::vma::destroy_process(child_pd);
        crate::serial_verbose_println!("sys_fork: child thread allocation failed");
        return u32::MAX;
    }

    // 5. Copy thread metadata to child
    scheduler::set_thread_user_info(child_tid, child_pd, snap.brk);

    // CWD
    let cwd_len = snap.cwd.iter().position(|&b| b == 0).unwrap_or(0);
    if cwd_len > 0 {
        if let Ok(cwd) = core::str::from_utf8(&snap.cwd[..cwd_len]) {
            scheduler::set_thread_cwd(child_tid, cwd);
        }
    }

    let linux_rootfs_len = snap.linux_rootfs.iter().position(|&b| b == 0).unwrap_or(0);
    if linux_rootfs_len > 0 {
        if let Ok(rootfs) = core::str::from_utf8(&snap.linux_rootfs[..linux_rootfs_len]) {
            scheduler::set_thread_linux_rootfs(child_tid, rootfs);
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
    scheduler::set_thread_abi(child_tid, snap.abi);
    if snap.abi == crate::task::abi::AbiPersonality::LinuxX86_64 {
        scheduler::set_thread_linux_fs_base(child_tid, snap.linux_fs_base);
        scheduler::set_thread_linux_clear_child_tid(child_tid, clear_child_tidptr);
    }

    // Pipes
    if snap.stdout_pipe != 0 {
        scheduler::set_thread_stdout_pipe(child_tid, snap.stdout_pipe);
    }
    if snap.stdin_pipe != 0 {
        scheduler::set_thread_stdin_pipe(child_tid, snap.stdin_pipe);
    }
    if snap.pty_id != 0 {
        scheduler::set_thread_pty_id(child_tid, snap.pty_id);
    }

    // FPU state, mmap_next, user_pages
    scheduler::set_thread_fpu_state(child_tid, &snap.fpu_data);
    scheduler::set_thread_mmap_next(child_tid, snap.mmap_next);
    scheduler::set_thread_user_pages(child_tid, snap.user_pages);

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
                FdKind::LinuxSocket { socket_id } => {
                    crate::syscall::linux::socket_incref(socket_id);
                }
                FdKind::Tty | FdKind::PtySlave { .. } | FdKind::LinuxProc { .. } | FdKind::None => {
                }
            }
        }
        scheduler::set_thread_fd_table(child_tid, fd_table);
    }

    // Child's parent_tid = the forking thread's TID (not grandparent)
    scheduler::set_thread_parent_tid(child_tid, scheduler::current_tid());
    // Inherit dispositions and blocked mask, but not pending signals. POSIX
    // fork starts the child with an empty pending-signal set.
    let mut child_signals = snap.signals.clone();
    child_signals.pending = 0;
    scheduler::set_thread_signals(child_tid, child_signals);

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
        fs_base: snap.linux_fs_base,
        child_tidptr,
    };
    store_pending_fork(child_tid, child_regs);

    // 9. Wake child — it will run fork_child_trampoline and IRETQ with RAX=0
    scheduler::wake_thread(child_tid);
    let t_fork_total = crate::arch::hal::timer_current_ticks();
    crate::serial_verbose_println!(
        "sys_fork: T{} → T{} clone={}ms total={}ms",
        parent_tid,
        child_tid,
        t_fork_cloned.wrapping_sub(t_fork0),
        t_fork_total.wrapping_sub(t_fork0)
    );

    // Parent returns child TID
    child_tid
}

/// sys_fork - Create a child process that is a copy of the parent on AArch64.
///
/// The child returns via `eret` using the saved EL0 exception frame, with X0=0.
#[cfg(target_arch = "aarch64")]
pub fn sys_fork(frame: &crate::arch::arm64::exceptions::ExceptionFrame) -> u32 {
    use crate::memory::virtual_mem;
    use crate::task::dll;
    use crate::task::env;
    use crate::task::loader::{fork_child_trampoline, store_pending_fork, ForkChildRegs};
    use crate::task::scheduler;

    let snap = match scheduler::current_thread_fork_snapshot() {
        Some(s) => s,
        None => {
            crate::serial_verbose_println!("sys_fork: failed to snapshot current thread");
            return u32::MAX;
        }
    };

    let t_fork0 = crate::arch::hal::timer_current_ticks();
    let parent_tid = scheduler::current_tid();

    let child_pd = match virtual_mem::clone_user_page_directory(snap.pd) {
        Some(pd) => pd,
        None => {
            crate::serial_verbose_println!("sys_fork: clone_user_page_directory failed (OOM)");
            return u32::MAX;
        }
    };
    let t_fork_cloned = crate::arch::hal::timer_current_ticks();

    crate::memory::vma::clone_for_fork(snap.pd, child_pd);

    let name_len = snap
        .name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(snap.name.len());
    let parent_name = core::str::from_utf8(&snap.name[..name_len]).unwrap_or("?");

    let child_tid = scheduler::spawn_blocked(fork_child_trampoline, snap.priority, parent_name);
    if child_tid == 0 {
        crate::memory::virtual_mem::destroy_user_page_directory(child_pd);
        crate::memory::vma::destroy_process(child_pd);
        crate::serial_verbose_println!("sys_fork: child thread allocation failed");
        return u32::MAX;
    }

    scheduler::set_thread_user_info(child_tid, child_pd, snap.brk);

    let cwd_len = snap.cwd.iter().position(|&b| b == 0).unwrap_or(0);
    if cwd_len > 0 {
        if let Ok(cwd) = core::str::from_utf8(&snap.cwd[..cwd_len]) {
            scheduler::set_thread_cwd(child_tid, cwd);
        }
    }

    let linux_rootfs_len = snap.linux_rootfs.iter().position(|&b| b == 0).unwrap_or(0);
    if linux_rootfs_len > 0 {
        if let Ok(rootfs) = core::str::from_utf8(&snap.linux_rootfs[..linux_rootfs_len]) {
            scheduler::set_thread_linux_rootfs(child_tid, rootfs);
        }
    }

    let args_len = snap.args.iter().position(|&b| b == 0).unwrap_or(0);
    if args_len > 0 {
        if let Ok(args) = core::str::from_utf8(&snap.args[..args_len]) {
            scheduler::set_thread_args(child_tid, args);
        }
    }

    scheduler::set_thread_capabilities(child_tid, snap.capabilities);
    scheduler::set_thread_identity(child_tid, snap.uid, snap.gid);
    scheduler::set_thread_abi(child_tid, snap.abi);
    if snap.abi == crate::task::abi::AbiPersonality::LinuxX86_64 {
        scheduler::set_thread_linux_fs_base(child_tid, snap.linux_fs_base);
    }

    if snap.stdout_pipe != 0 {
        scheduler::set_thread_stdout_pipe(child_tid, snap.stdout_pipe);
    }
    if snap.stdin_pipe != 0 {
        scheduler::set_thread_stdin_pipe(child_tid, snap.stdin_pipe);
    }
    if snap.pty_id != 0 {
        scheduler::set_thread_pty_id(child_tid, snap.pty_id);
    }

    scheduler::set_thread_fpu_state(child_tid, &snap.fpu_data);
    scheduler::set_thread_mmap_next(child_tid, snap.mmap_next);
    scheduler::set_thread_user_pages(child_tid, snap.user_pages);

    {
        use crate::fs::fd_table::FdKind;
        let fd_table = snap.fd_table.clone();
        for entry in fd_table.iter_open() {
            match entry.kind {
                FdKind::File { global_id } => crate::fs::vfs::incref(global_id),
                FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::incref_read(pipe_id),
                FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::incref_write(pipe_id),
                FdKind::LinuxSocket { socket_id } => {
                    crate::syscall::linux::socket_incref(socket_id)
                }
                FdKind::Tty | FdKind::PtySlave { .. } | FdKind::LinuxProc { .. } | FdKind::None => {
                }
            }
        }
        scheduler::set_thread_fd_table(child_tid, fd_table);
    }

    scheduler::set_thread_parent_tid(child_tid, scheduler::current_tid());
    scheduler::set_thread_signals(child_tid, snap.signals.clone());

    env::clone_env(snap.pd.0, child_pd.0);
    dll::map_all_dlls_into(child_pd);

    let tpidr_el0: u64;
    unsafe {
        core::arch::asm!(
            "mrs {}, tpidr_el0",
            out(reg) tpidr_el0,
            options(nomem, nostack),
        );
    }

    let child_regs = ForkChildRegs {
        x: frame.x,
        sp_el0: frame.sp_el0,
        elr_el1: frame.elr_el1,
        spsr_el1: frame.spsr_el1,
        tpidr_el0,
    };
    store_pending_fork(child_tid, child_regs);

    scheduler::wake_thread(child_tid);
    let t_fork_total = crate::arch::hal::timer_current_ticks();
    crate::serial_verbose_println!(
        "sys_fork: T{} -> T{} clone={}ms total={}ms",
        parent_tid,
        child_tid,
        t_fork_cloned.wrapping_sub(t_fork0),
        t_fork_total.wrapping_sub(t_fork0),
    );

    child_tid
}

/// sys_exec - Replace the current process with a new program.
/// arg1 = path_ptr (null-terminated), arg2 = args_ptr (null-terminated, 0=none).
/// On success, never returns (process is replaced).
/// On failure, returns u32::MAX.
pub fn sys_exec(path_ptr: u64, args_ptr: u64) -> u32 {
    let path = resolve_path(unsafe { read_user_str(path_ptr) });
    let args_str;
    let args = if args_ptr != 0 {
        args_str = unsafe { read_user_str(args_ptr) };
        args_str
    } else {
        ""
    };

    crate::serial_verbose_println!(
        "sys_exec: T{} path='{}' args='{}'",
        crate::task::scheduler::current_tid(),
        path,
        args
    );

    // Read the binary from the filesystem
    let data = match crate::fs::vfs::read_file_to_vec(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::serial_verbose_println!(
                "sys_exec: read_file_to_vec('{}') failed: {:?}",
                path,
                e
            );
            return u32::MAX;
        }
    };

    // exec_current_process only returns on error (on success it jumps to user mode)
    let err = crate::task::loader::exec_current_process(&data, args);
    crate::serial_verbose_println!("sys_exec: FAILED: {}", err);
    u32::MAX
}

/// sys_thread_create - Create a new thread within the current process.
/// arg1=entry_rip, arg2=user_rsp, arg3=name_ptr, arg4=name_len, arg5=priority.
/// Returns TID of new thread, or 0 on error.
pub fn sys_thread_create(
    entry_rip: u64,
    user_rsp: u64,
    name_ptr: u64,
    name_len: u32,
    priority: u32,
) -> u32 {
    let entry = entry_rip;
    let rsp = user_rsp;
    let mut user_lr = 0u64;

    // Basic validation: entry must be mapped user memory, rsp must point
    // at a mapped user stack slot and obey the platform ABI alignment.
    if entry == 0 || !is_user_range_accessible(entry, 1) {
        return 0;
    }
    #[cfg(target_arch = "x86_64")]
    let stack_ok = rsp != 0 && rsp < 0x0000_8000_0000_0000 && (rsp & 7) == 0;
    #[cfg(target_arch = "aarch64")]
    let stack_ok = rsp != 0 && rsp < 0x0000_8000_0000_0000 && (rsp & 0xF) == 0;
    if !stack_ok {
        return 0;
    }

    #[cfg(target_arch = "x86_64")]
    {
        if !is_user_range_accessible(rsp, 8) {
            return 0;
        }
        let return_pc = unsafe { core::ptr::read_unaligned(rsp as *const u64) };
        if return_pc == 0 || !is_user_range_accessible(return_pc, 1) {
            return 0;
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        let lr_ptr = rsp.wrapping_sub(8);
        if !is_user_range_accessible(lr_ptr, 8) {
            return 0;
        }
        unsafe {
            user_lr = core::ptr::read_unaligned(lr_ptr as *const u64);
        }
        if user_lr == 0 || !is_user_range_accessible(user_lr, 1) {
            return 0;
        }
    }

    // Read thread name from user space (max 31 chars)
    let mut name_buf = [0u8; 32];
    let len = (name_len as usize).min(31);
    if name_ptr != 0 && len > 0 {
        if let Some(bytes) = copy_user_bytes(name_ptr, len, 31) {
            name_buf[..bytes.len()].copy_from_slice(&bytes);
        }
    }
    let name = core::str::from_utf8(&name_buf[..len]).unwrap_or("thread");

    // Priority: 0 means inherit from parent (handled by scheduler), 1-255 = explicit
    let pri = if priority > 0 && priority <= 255 {
        priority as u8
    } else {
        0
    };

    let tid =
        crate::task::scheduler::create_thread_in_current_process(entry, rsp, user_lr, name, pri);
    crate::debug_println!(
        "sys_thread_create: entry={:#x} rsp={:#x} name={} pri={} -> TID={}",
        entry,
        rsp,
        name,
        pri,
        tid
    );
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
