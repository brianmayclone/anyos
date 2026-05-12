use super::*;

/// Load a binary (ELF64/flat) into an already-created page directory.
/// Maps code segments + user stack. Returns entry point, brk, page count.
pub fn load_binary_into_pd(
    data: &[u8],
    pd_phys: crate::memory::address::PhysAddr,
) -> Result<LoadResult, &'static str> {
    if data.is_empty() {
        return Err("Program data is empty");
    }

    let mut total_user_pages: u32 = 0;
    // ASLR: randomize the stack top within the 8 MiB region.
    // The offset is subtracted from USER_STACK_TOP so the stack starts a
    // random number of pages below the fixed top.  The full 8 MiB is still
    // allocated, so the random gap is simply unused guard space above.
    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
    // Stack is data — writable but never executed.
    let stack_flags = PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag();

    #[cfg(target_arch = "x86_64")]
    {
        // Map a signal-return trampoline page (USER | EXECUTABLE, no NX).
        // Contains `mov eax, SYS_SIGRETURN; int 0x80; nop` so signal handlers
        // can return without executing code on the NX-protected stack.
        let tramp_mapped = install_sigreturn_trampoline(pd_phys)?;
        total_user_pages += tramp_mapped;
    }

    // Guard page: leave the bottom-most page of the stack region UNMAPPED.
    // If user code overflows the stack, it touches this unmapped page and
    // triggers a page fault -> the kernel kills the thread with SIGSEGV
    // instead of corrupting memory or crashing the kernel.
    let guard_pages: u64 = 1;
    let _stack_guard_bottom = stack_bottom; // unmapped guard page
    let stack_usable_bottom = stack_bottom + guard_pages * PAGE_SIZE;
    let stack_usable_pages = USER_STACK_PAGES - guard_pages;

    let class = elf_class(data);
    if class == ELFCLASS64 {
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_usable_bottom),
            stack_usable_pages,
            stack_flags,
            true,
        )?;
        let elf_result = load_elf64(data, pd_phys, 0x0800_0000, 0)?;
        total_user_pages += elf_result.pages_mapped + stack_mapped;
        Ok(LoadResult {
            entry: elf_result.entry,
            brk: elf_result.brk,
            user_pages: total_user_pages,
            // x86-64 ABI: RSP % 16 == 8 at function entry (simulates `call` push).
            stack_top: aslr_stack_top - 8,
        })
    } else if class == ELFCLASS32 {
        Err("ELF32 binaries are no longer supported (32-bit user space removed)")
    } else if is_elf(data) {
        Err("Unknown ELF class (only ELF64 is supported)")
    } else {
        // Flat binary: no ELF headers so we cannot know which sections are
        // code vs. data. Map everything RWX for backwards compatibility.
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let code_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(PROGRAM_LOAD_ADDR),
            code_pages,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            stack_flags,
            true,
        )?;

        // Copy binary data into the new address space
        unsafe {
            #[cfg(target_arch = "x86_64")]
            let saved_flags: u64;
            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
                core::arch::asm!("cli", options(nomem, nostack));
            }
            #[cfg(target_arch = "aarch64")]
            let saved_daif: u64;
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("mrs {}, daif", out(reg) saved_daif, options(nomem, nostack));
                core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            }
            let old_pt = virtual_mem::current_cr3();
            #[cfg(target_arch = "x86_64")]
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) pd_phys.as_u64(), options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
            }

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            #[cfg(target_arch = "aarch64")]
            sync_user_text_range_for_exec(PROGRAM_LOAD_ADDR, data.len());

            #[cfg(target_arch = "x86_64")]
            {
                core::arch::asm!("mov cr3, {}", in(reg) old_pt);
                core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
            }
            #[cfg(target_arch = "aarch64")]
            {
                core::arch::asm!("msr ttbr0_el1, {}", in(reg) old_pt, options(nomem, nostack));
                core::arch::asm!("isb", options(nomem, nostack));
                core::arch::asm!("msr daif, {}", in(reg) saved_daif, options(nomem, nostack));
            }
        }

        total_user_pages += code_mapped + stack_mapped;
        Ok(LoadResult {
            entry: PROGRAM_LOAD_ADDR,
            brk: PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE,
            user_pages: total_user_pages,
            stack_top: aslr_stack_top - 8,
        })
    }
}

/// Replace the current process with a new binary loaded from `data`.
/// On success, never returns (jumps to user mode in new address space).
/// On failure, returns an error string and the old process continues.
pub fn exec_current_process(data: &[u8], args: &str) -> &'static str {
    let tid = crate::task::scheduler::current_tid();

    // Get old PD before we replace it
    let old_pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd,
        None => return "exec: no page directory on current thread",
    };

    // Create fresh page directory
    let new_pd = match virtual_mem::create_user_page_directory() {
        Some(pd) => pd,
        None => return "exec: failed to create page directory (OOM)",
    };

    // Load binary into new PD
    let result = match load_binary_into_pd(data, new_pd) {
        Ok(r) => r,
        Err(e) => {
            virtual_mem::destroy_user_page_directory(new_pd);
            return e;
        }
    };

    // Map DLLs into new address space
    crate::task::dll::map_all_dlls_into(new_pd);

    // Update thread metadata (PD, brk, FPU reset, mmap reset)
    crate::task::scheduler::exec_update_thread(tid, new_pd, result.brk, result.user_pages);

    // Set new args (clear old args first)
    crate::task::scheduler::set_thread_args(tid, args);

    // Rekey environment from old PD to new PD (move entries in-place)
    crate::task::env::rekey_env(old_pd.0, new_pd.0);

    // Switch page table to new address space and destroy old one
    unsafe {
        #[cfg(target_arch = "x86_64")]
        {
            core::arch::asm!("cli", options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) new_pd.as_u64());
        }
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) new_pd.as_u64(), options(nomem, nostack));
            core::arch::asm!("isb", options(nomem, nostack));
        }
    }

    // Destroy old PD (safe: we're now running on new PD, kernel pages are shared)
    virtual_mem::destroy_user_page_directory(old_pd);

    // Re-enable interrupts and jump to user mode (never returns).
    // stack_top already includes ABI alignment (-8) and ASLR offset.
    let user_stack = result.stack_top;

    crate::serial_verbose_println!(
        "exec: T{} -> (elf64, {} pages, entry={:#x})",
        tid,
        result.user_pages,
        result.entry
    );

    #[cfg(target_arch = "x86_64")]
    unsafe {
        jump_to_user_mode(result.entry, user_stack);
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        jump_to_user_mode(result.entry, user_stack, 0);
    }
}
