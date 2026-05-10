use super::*;

fn map_linux_sigreturn_trampoline(
    pd_phys: crate::memory::address::PhysAddr,
) -> Result<u32, &'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        let mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(SIGRETURN_TRAMPOLINE_ADDR),
            1,
            PAGE_USER,
            true,
        )?;
        unsafe {
            let saved_flags: u64;
            core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
            let old_pt = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            let tramp = SIGRETURN_TRAMPOLINE_ADDR as *mut u8;
            tramp.offset(0).write_volatile(0xB8);
            tramp.offset(1).write_volatile(246);
            tramp.offset(2).write_volatile(0x00);
            tramp.offset(3).write_volatile(0x00);
            tramp.offset(4).write_volatile(0x00);
            tramp.offset(5).write_volatile(0x0F);
            tramp.offset(6).write_volatile(0x05);
            tramp.offset(7).write_volatile(0x90);
            core::arch::asm!("mov cr3, {}", in(reg) old_pt);
            core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
        }
        Ok(mapped)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = pd_phys;
        Ok(0)
    }
}

fn load_linux_image_into_pd(
    data: &[u8],
    pd_phys: crate::memory::address::PhysAddr,
    linux_rootfs: &str,
    argv: &[alloc::string::String],
    envp: &[alloc::string::String],
) -> Result<LoadResult, &'static str> {
    if data.is_empty() {
        return Err("licof: program file is empty");
    }

    let mut total_user_pages = map_linux_sigreturn_trampoline(pd_phys)?;

    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
    let stack_flags = PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag();

    let class = elf_class(data);
    if class != ELFCLASS64 {
        if class == ELFCLASS32 {
            return Err("licof: ELF32 Linux binaries are not supported");
        }
        if is_elf(data) {
            return Err("licof: unknown Linux ELF class");
        }
        return Err("licof: only ELF64 Linux binaries are supported");
    }

    let mut linux_elf = inspect_linux_elf64(data)?;
    let stack_mapped = virtual_mem::map_pages_range_in_pd(
        pd_phys,
        VirtAddr::new(stack_bottom),
        USER_STACK_PAGES,
        stack_flags,
        true,
    )?;

    let min_user_vaddr = 0x0001_0000;
    let main_load_bias = if linux_elf.is_dyn {
        LINUX_MAIN_DYN_BASE
    } else {
        0
    };
    let elf_result = load_elf64(data, pd_phys, min_user_vaddr, main_load_bias)?;
    let mut entry_point = elf_result.entry;
    let brk = elf_result.brk;
    total_user_pages += elf_result.pages_mapped + stack_mapped;

    linux_elf.entry = elf_result.entry;
    if linux_elf.phdr_addr != 0 {
        linux_elf.phdr_addr = linux_elf
            .phdr_addr
            .checked_add(main_load_bias)
            .ok_or("licof: AT_PHDR load bias overflow")?;
    }

    if let Some(ref interp_path) = linux_elf.interp_path {
        let translated_interp = licof_resolve_interp_path(linux_rootfs, interp_path);
        let resolved_interp = match licof_resolve_rootfs_path(linux_rootfs, &translated_interp) {
            Ok(path) => path,
            Err(err) => {
                crate::serial_println!(
                    "licof loader: failed to resolve PT_INTERP '{}' translated='{}': {}",
                    interp_path,
                    translated_interp,
                    err
                );
                return Err(err);
            }
        };
        let interp_data = crate::fs::vfs::read_file_to_vec(&resolved_interp).map_err(|_| {
            crate::serial_println!(
                "licof loader: failed to read PT_INTERP '{}' resolved='{}'",
                interp_path,
                resolved_interp
            );
            "licof: failed to read PT_INTERP from rootfs"
        })?;
        let interp_info = inspect_linux_elf64(&interp_data)?;
        if interp_info.interp_path.is_some() {
            return Err("licof: nested PT_INTERP is not supported");
        }
        let interp_load_bias = if interp_info.is_dyn {
            LINUX_INTERP_BASE
        } else {
            0
        };
        let interp_result = load_elf64(&interp_data, pd_phys, min_user_vaddr, interp_load_bias)?;
        entry_point = interp_result.entry;
        linux_elf.interp_base = interp_load_bias;
        total_user_pages += interp_result.pages_mapped;
    }

    let stack_top = write_linux_initial_stack_vectors(
        pd_phys,
        aslr_stack_top,
        stack_bottom,
        argv,
        envp,
        &linux_elf,
    )?;

    Ok(LoadResult {
        entry: entry_point,
        brk,
        user_pages: total_user_pages,
        stack_top,
    })
}

fn decref_fd_kind(kind: crate::fs::fd_table::FdKind) {
    use crate::fs::fd_table::FdKind;
    match kind {
        FdKind::File { global_id } => crate::fs::vfs::decref(global_id),
        FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(pipe_id),
        FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(pipe_id),
        FdKind::Tty | FdKind::LinuxProc { .. } | FdKind::None => {}
    }
}

fn close_current_cloexec_fds() {
    for kind in crate::task::scheduler::current_fd_close_cloexec() {
        decref_fd_kind(kind);
    }
}

/// Replace the current Linux ABI process image with an ELF64 Linux binary.
/// On success, never returns. On failure, returns an error string and leaves
/// the original image running.
pub fn exec_current_linux_process(
    load_path: &str,
    argv: &[alloc::string::String],
    envp: &[alloc::string::String],
) -> &'static str {
    let tid = crate::task::scheduler::current_tid();
    let old_pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd,
        None => return "licof execve: no page directory on current thread",
    };

    let data = match crate::fs::vfs::read_file_to_vec(load_path) {
        Ok(data) => data,
        Err(_) => return "licof execve: failed to read program file",
    };
    let new_pd = match virtual_mem::create_user_page_directory_no_low_identity() {
        Some(pd) => pd,
        None => return "licof execve: failed to create page directory",
    };

    let linux_rootfs = LICOF_ROOTFS;
    let result = match load_linux_image_into_pd(&data, new_pd, linux_rootfs, argv, envp) {
        Ok(result) => result,
        Err(err) => {
            crate::memory::vma::destroy_process(new_pd);
            virtual_mem::destroy_user_page_directory(new_pd);
            return err;
        }
    };

    let mmap_rand = random_page_offset(ASLR_MMAP_MAX_PAGES);
    let mmap_start = MMAP_BASE.wrapping_add(mmap_rand as u64 * 4096);
    crate::memory::vma::init_process(new_pd, mmap_start);
    close_current_cloexec_fds();

    crate::task::scheduler::exec_update_thread(tid, new_pd, result.brk, result.user_pages);
    crate::task::scheduler::set_thread_mmap_next(tid, mmap_start);
    crate::task::scheduler::set_thread_abi(tid, crate::task::abi::AbiPersonality::LinuxX86_64);
    crate::task::scheduler::set_thread_linux_rootfs(tid, linux_rootfs);
    crate::task::scheduler::set_thread_linux_fs_base(tid, 0);
    if let Some(arg0) = argv.first() {
        crate::task::scheduler::set_thread_args(tid, arg0);
    }
    crate::task::env::rekey_env(old_pd.0, new_pd.0);

    unsafe {
        #[cfg(target_arch = "x86_64")]
        {
            core::arch::asm!("cli", options(nomem, nostack));
            core::arch::asm!("mov cr3, {}", in(reg) new_pd.as_u64());
            crate::arch::x86::power::wrmsr(0xC000_0100, 0);
        }
        #[cfg(target_arch = "aarch64")]
        {
            core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) new_pd.as_u64(), options(nomem, nostack));
            core::arch::asm!("isb", options(nomem, nostack));
        }
    }

    crate::memory::vma::destroy_process(old_pd);
    virtual_mem::destroy_user_page_directory(old_pd);

    #[cfg(target_arch = "x86_64")]
    unsafe {
        jump_to_user_mode(result.entry, result.stack_top)
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        jump_to_user_mode(result.entry, result.stack_top)
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        "licof execve: unsupported architecture"
    }
}
