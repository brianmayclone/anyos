use super::*;

use alloc::vec::Vec;

const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;

const ET_DYN: u16 = 3;

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_FLAGS: u64 = 8;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;
const AT_PLATFORM: u64 = 15;
const AT_RANDOM: u64 = 25;
const AT_EXECFN: u64 = 31;

const LXE_ROOTFS: &str = "/System/var/lxe/rootfs";
const LINUX_MAIN_DYN_BASE: u64 = 0x0000_5555_0000_0000;
const LINUX_INTERP_BASE: u64 = 0x0000_7000_0000_0000;

struct LinuxElf64Info {
    entry: u64,
    phdr_addr: u64,
    phent: u64,
    phnum: u64,
    interp_path: Option<alloc::string::String>,
    interp_base: u64,
    is_dyn: bool,
}

fn inspect_linux_elf64(data: &[u8]) -> Result<LinuxElf64Info, &'static str> {
    if data.len() < core::mem::size_of::<Elf64Header>() {
        return Err("ELF64 file too small");
    }
    if !is_elf(data) || elf_class(data) != ELFCLASS64 {
        return Err("lxe: expected ELF64 binary");
    }

    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Header) };
    let entry = hdr.e_entry;
    let e_type = hdr.e_type;
    let ph_off = hdr.e_phoff as usize;
    let ph_size = hdr.e_phentsize as usize;
    let ph_num = hdr.e_phnum as usize;

    if ph_size < core::mem::size_of::<Elf64Phdr>() {
        return Err("lxe: ELF64 program header entry too small");
    }
    if ph_num == 0 {
        return Err("lxe: ELF64 has no program headers");
    }
    let ph_table_size = ph_size
        .checked_mul(ph_num)
        .ok_or("lxe: ELF64 program header table overflow")?;
    if ph_off
        .checked_add(ph_table_size)
        .map_or(true, |end| end > data.len())
    {
        return Err("lxe: ELF64 program header table out of bounds");
    }

    let mut phdr_addr = 0u64;
    let mut interp_path = None;
    for i in 0..ph_num {
        let ph_offset = ph_off + i * ph_size;
        let phdr = unsafe { &*(data.as_ptr().add(ph_offset) as *const Elf64Phdr) };
        let p_type = phdr.p_type;
        let p_offset = phdr.p_offset;
        let p_vaddr = phdr.p_vaddr;
        let p_filesz = phdr.p_filesz;

        if p_type == PT_PHDR {
            phdr_addr = p_vaddr;
        } else if p_type == PT_INTERP {
            let start = p_offset as usize;
            let len = p_filesz as usize;
            if len == 0 || len > 512 || start.checked_add(len).map_or(true, |end| end > data.len())
            {
                return Err("lxe: invalid PT_INTERP");
            }
            let bytes = &data[start..start + len];
            let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            let path =
                core::str::from_utf8(&bytes[..nul]).map_err(|_| "lxe: PT_INTERP is not UTF-8")?;
            interp_path = Some(alloc::string::String::from(path));
        } else if p_type == PT_LOAD && phdr_addr == 0 {
            if let Some(end) = p_offset.checked_add(p_filesz) {
                let ph_file_off = ph_off as u64;
                if ph_file_off >= p_offset && ph_file_off < end {
                    phdr_addr = p_vaddr + (ph_file_off - p_offset);
                }
            }
        }
    }

    Ok(LinuxElf64Info {
        entry,
        phdr_addr,
        phent: ph_size as u64,
        phnum: ph_num as u64,
        interp_path,
        interp_base: 0,
        is_dyn: e_type == ET_DYN,
    })
}

fn lxe_rootfs_for_binary(path: &str) -> alloc::string::String {
    if path == LXE_ROOTFS
        || (path.starts_with(LXE_ROOTFS) && path.as_bytes().get(LXE_ROOTFS.len()) == Some(&b'/'))
    {
        return alloc::string::String::from(LXE_ROOTFS);
    }
    alloc::string::String::from(LXE_ROOTFS)
}

fn lxe_resolve_interp_path(rootfs: &str, path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        alloc::format!("{}{}", rootfs, path)
    } else {
        alloc::format!("{}/{}", rootfs, path)
    }
}

fn lxe_resolve_rootfs_path(
    rootfs: &str,
    path: &str,
) -> Result<alloc::string::String, &'static str> {
    let translated = crate::fs::path::normalize(path);
    if !lxe_path_under_rootfs(rootfs, &translated) {
        return Ok(translated);
    }
    lxe_resolve_rootfs_path_inner(rootfs, &translated, 0)
}

fn lxe_resolve_rootfs_path_inner(
    rootfs: &str,
    path: &str,
    depth: u32,
) -> Result<alloc::string::String, &'static str> {
    if depth > 16 {
        return Err("lxe: too many rootfs symlinks");
    }

    let normalized = crate::fs::path::normalize(path);
    let rel = lxe_rootfs_relative(rootfs, &normalized);
    let components: Vec<&str> = rel
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let mut current = alloc::string::String::from(rootfs);
    if components.is_empty() {
        return Ok(current);
    }

    for (idx, component) in components.iter().enumerate() {
        let candidate = lxe_join_path(&current, component);
        match crate::fs::vfs::lstat(&candidate) {
            Ok(st) if st.is_symlink => {
                let target = crate::fs::vfs::readlink(&candidate)
                    .map_err(|_| "lxe: failed to read rootfs symlink")?;
                let parent = lxe_parent_path(&candidate);
                let next =
                    lxe_resolve_link_target(rootfs, &parent, &target, &components[idx + 1..]);
                return lxe_resolve_rootfs_path_inner(rootfs, &next, depth + 1);
            }
            Ok(_) => {
                current = candidate;
            }
            Err(crate::fs::vfs::FsError::NotFound) => {
                return Err("lxe: rootfs path not found");
            }
            Err(_) => return Err("lxe: failed to resolve rootfs path"),
        }
    }
    Ok(current)
}

fn lxe_path_under_rootfs(rootfs: &str, path: &str) -> bool {
    path == rootfs || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
}

fn lxe_rootfs_relative<'a>(rootfs: &str, path: &'a str) -> &'a str {
    if path == rootfs {
        ""
    } else if path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        &path[rootfs.len() + 1..]
    } else {
        path.trim_start_matches('/')
    }
}

fn lxe_join_path(base: &str, component: &str) -> alloc::string::String {
    if base == "/" {
        alloc::format!("/{}", component)
    } else if base.ends_with('/') {
        alloc::format!("{}{}", base, component)
    } else {
        alloc::format!("{}/{}", base, component)
    }
}

fn lxe_parent_path(path: &str) -> alloc::string::String {
    let normalized = crate::fs::path::normalize(path);
    match normalized.rfind('/') {
        Some(0) | None => alloc::string::String::from("/"),
        Some(idx) => alloc::string::String::from(&normalized[..idx]),
    }
}

fn lxe_resolve_link_target(
    rootfs: &str,
    parent: &str,
    target: &str,
    remaining: &[&str],
) -> alloc::string::String {
    let mut path = if target.starts_with('/') {
        if target == "/" {
            alloc::string::String::from(rootfs)
        } else {
            alloc::format!("{}{}", rootfs, target)
        }
    } else {
        lxe_join_path(parent, target)
    };
    for component in remaining {
        path = lxe_join_path(&path, component);
    }
    crate::fs::path::normalize(&path)
}

fn copy_to_user_pd(pd_phys: crate::memory::address::PhysAddr, dst: u64, src: &[u8]) {
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

        core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len());

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
}

fn push_linux_u64(buf: &mut alloc::vec::Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_ne_bytes());
}

fn write_linux_initial_stack_vectors(
    pd_phys: crate::memory::address::PhysAddr,
    stack_top: u64,
    stack_bottom: u64,
    argv: &[alloc::string::String],
    envp: &[alloc::string::String],
    elf: &LinuxElf64Info,
) -> Result<u64, &'static str> {
    if argv.is_empty() {
        return Err("lxe: empty argv");
    }
    if argv.len() > 64 {
        return Err("lxe: too many argv entries");
    }
    if envp.len() > 128 {
        return Err("lxe: too many envp entries");
    }

    let mut string_bytes = 0usize;
    for s in argv.iter().chain(envp.iter()) {
        string_bytes = string_bytes
            .checked_add(s.len() + 1)
            .ok_or("lxe: initial stack string size overflow")?;
    }
    if string_bytes > 64 * 1024 {
        return Err("lxe: initial stack strings too large");
    }

    let mut sp = stack_top;
    let mut payload_chunks = alloc::vec::Vec::<(u64, alloc::vec::Vec<u8>)>::new();
    let mut argv_ptrs = alloc::vec::Vec::<u64>::new();
    let mut env_ptrs = alloc::vec::Vec::<u64>::new();

    for s in envp.iter().rev() {
        let bytes = s.as_bytes();
        sp = sp
            .checked_sub((bytes.len() + 1) as u64)
            .ok_or("lxe: initial stack underflow")?;
        let mut chunk = alloc::vec::Vec::<u8>::new();
        chunk.extend_from_slice(bytes);
        chunk.push(0);
        payload_chunks.push((sp, chunk));
        env_ptrs.push(sp);
    }
    env_ptrs.reverse();

    for s in argv.iter().rev() {
        let bytes = s.as_bytes();
        sp = sp
            .checked_sub((bytes.len() + 1) as u64)
            .ok_or("lxe: initial stack underflow")?;
        let mut chunk = alloc::vec::Vec::<u8>::new();
        chunk.extend_from_slice(bytes);
        chunk.push(0);
        payload_chunks.push((sp, chunk));
        argv_ptrs.push(sp);
    }
    argv_ptrs.reverse();

    let platform = b"x86_64\0";
    sp = sp
        .checked_sub(platform.len() as u64)
        .ok_or("lxe: initial stack underflow")?;
    let platform_ptr = sp;
    let mut platform_chunk = alloc::vec::Vec::<u8>::new();
    platform_chunk.extend_from_slice(platform);
    payload_chunks.push((sp, platform_chunk));

    let mut random_bytes = [0u8; 16];
    let seed = counter_entropy()
        ^ ((crate::arch::hal::timer_current_ticks() as u64) << 32)
        ^ crate::task::scheduler::current_tid() as u64;
    for i in 0..2 {
        let value = seed.rotate_left((i as u32) * 23) ^ (0x9E37_79B9_7F4A_7C15u64 * (i as u64 + 1));
        random_bytes[i * 8..i * 8 + 8].copy_from_slice(&value.to_ne_bytes());
    }
    sp = sp
        .checked_sub(random_bytes.len() as u64)
        .ok_or("lxe: initial stack underflow")?;
    let random_ptr = sp;
    let mut random_chunk = alloc::vec::Vec::<u8>::new();
    random_chunk.extend_from_slice(&random_bytes);
    payload_chunks.push((sp, random_chunk));

    sp &= !0xF;

    let mut auxv = alloc::vec::Vec::<(u64, u64)>::new();
    if elf.phdr_addr != 0 {
        auxv.push((AT_PHDR, elf.phdr_addr));
    }
    auxv.push((AT_PHENT, elf.phent));
    auxv.push((AT_PHNUM, elf.phnum));
    auxv.push((AT_PAGESZ, PAGE_SIZE));
    auxv.push((AT_BASE, elf.interp_base));
    auxv.push((AT_FLAGS, 0));
    auxv.push((AT_ENTRY, elf.entry));
    auxv.push((AT_UID, 0));
    auxv.push((AT_EUID, 0));
    auxv.push((AT_GID, 0));
    auxv.push((AT_EGID, 0));
    auxv.push((AT_PLATFORM, platform_ptr));
    auxv.push((AT_RANDOM, random_ptr));
    auxv.push((AT_EXECFN, argv_ptrs[0]));
    auxv.push((AT_NULL, 0));

    let vector_bytes = 8 + (argv_ptrs.len() + 1) * 8 + (env_ptrs.len() + 1) * 8 + auxv.len() * 16;
    if sp
        .checked_sub(vector_bytes as u64)
        .ok_or("lxe: initial stack underflow")?
        & 0xF
        != 0
    {
        sp = sp
            .checked_sub(8)
            .ok_or("lxe: initial stack alignment underflow")?;
    }
    sp = sp
        .checked_sub(vector_bytes as u64)
        .ok_or("lxe: initial stack underflow")?;
    if sp < stack_bottom {
        return Err("lxe: initial stack exceeds mapped stack");
    }

    let mut vectors = alloc::vec::Vec::<u8>::new();
    push_linux_u64(&mut vectors, argv_ptrs.len() as u64);
    for ptr in argv_ptrs {
        push_linux_u64(&mut vectors, ptr);
    }
    push_linux_u64(&mut vectors, 0);
    for ptr in env_ptrs {
        push_linux_u64(&mut vectors, ptr);
    }
    push_linux_u64(&mut vectors, 0);
    for (key, value) in auxv {
        push_linux_u64(&mut vectors, key);
        push_linux_u64(&mut vectors, value);
    }

    for (addr, bytes) in &payload_chunks {
        let end = addr
            .checked_add(bytes.len() as u64)
            .ok_or("lxe: initial stack payload overflow")?;
        if *addr < stack_bottom || end > stack_top {
            return Err("lxe: initial stack payload outside mapped stack");
        }
    }

    for (addr, bytes) in payload_chunks {
        copy_to_user_pd(pd_phys, addr, &bytes);
    }
    copy_to_user_pd(pd_phys, sp, &vectors);
    Ok(sp)
}

fn map_linux_sigreturn_trampoline(
    pd_phys: crate::memory::address::PhysAddr,
) -> Result<u32, &'static str> {
    install_sigreturn_trampoline(pd_phys)
}

fn load_linux_image_into_pd(
    data: &[u8],
    pd_phys: crate::memory::address::PhysAddr,
    linux_rootfs: &str,
    argv: &[alloc::string::String],
    envp: &[alloc::string::String],
) -> Result<LoadResult, &'static str> {
    if data.is_empty() {
        return Err("lxe: program file is empty");
    }

    let mut total_user_pages = map_linux_sigreturn_trampoline(pd_phys)?;

    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
    let stack_flags = PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag();

    let class = elf_class(data);
    if class != ELFCLASS64 {
        if class == ELFCLASS32 {
            return Err("lxe: ELF32 Linux binaries are not supported");
        }
        if is_elf(data) {
            return Err("lxe: unknown Linux ELF class");
        }
        return Err("lxe: only ELF64 Linux binaries are supported");
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
            .ok_or("lxe: AT_PHDR load bias overflow")?;
    }

    if let Some(ref interp_path) = linux_elf.interp_path {
        let translated_interp = lxe_resolve_interp_path(linux_rootfs, interp_path);
        let resolved_interp = match lxe_resolve_rootfs_path(linux_rootfs, &translated_interp) {
            Ok(path) => path,
            Err(err) => {
                crate::serial_verbose_println!(
                    "lxe loader: failed to resolve PT_INTERP '{}' translated='{}': {}",
                    interp_path,
                    translated_interp,
                    err
                );
                return Err(err);
            }
        };
        let interp_data = crate::fs::vfs::read_file_to_vec(&resolved_interp).map_err(|_| {
            crate::serial_verbose_println!(
                "lxe loader: failed to read PT_INTERP '{}' resolved='{}'",
                interp_path,
                resolved_interp
            );
            "lxe: failed to read PT_INTERP from rootfs"
        })?;
        let interp_info = inspect_linux_elf64(&interp_data)?;
        if interp_info.interp_path.is_some() {
            return Err("lxe: nested PT_INTERP is not supported");
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

fn build_spawn_argv(path: &str, args: &str) -> alloc::vec::Vec<alloc::string::String> {
    let mut argv = alloc::vec::Vec::<alloc::string::String>::new();
    argv.push(alloc::string::String::from(path));
    for arg in args.split_ascii_whitespace() {
        if !arg.is_empty() {
            argv.push(alloc::string::String::from(arg));
        }
    }
    argv
}

fn default_envp() -> alloc::vec::Vec<alloc::string::String> {
    alloc::vec![
        alloc::string::String::from("PATH=/usr/bin:/bin"),
        alloc::string::String::from("HOME=/root"),
        alloc::string::String::from("PWD=/"),
        alloc::string::String::from("TERM=xterm-256color"),
        alloc::string::String::from("SHELL=/bin/bash"),
        alloc::string::String::from("USER=root"),
        alloc::string::String::from("LOGNAME=root"),
        alloc::string::String::from("LANG=C"),
        alloc::string::String::from("LC_ALL=C"),
        alloc::string::String::from("LD_HWCAP_MASK=0"),
        alloc::string::String::from("PS1=# "),
        alloc::string::String::from("LXE=1"),
    ]
}

/// Load and run a Linux x86_64 ELF through lxe.
pub fn load_and_run_with_args(path: &str, name: &str, args: &str) -> Result<u32, &'static str> {
    load_and_run_with_args_stdio(path, name, args, super::SpawnStdio::NONE)
}

pub fn load_and_run_with_args_stdio(
    path: &str,
    name: &str,
    args: &str,
    stdio: super::SpawnStdio,
) -> Result<u32, &'static str> {
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (path, name, args, stdio);
        return Err("lxe: Linux x86_64 ABI requires an x86_64 kernel");
    }

    #[cfg(target_arch = "x86_64")]
    load_and_run_with_args_x86_64(path, name, args, stdio)
}

#[cfg(target_arch = "x86_64")]
fn load_and_run_with_args_x86_64(
    path: &str,
    name: &str,
    args: &str,
    stdio: super::SpawnStdio,
) -> Result<u32, &'static str> {
    let linux_rootfs = lxe_rootfs_for_binary(path);
    let load_path = match lxe_resolve_rootfs_path(&linux_rootfs, path) {
        Ok(path) => path,
        Err(err) => {
            crate::serial_verbose_println!(
                "lxe loader: failed to resolve binary path='{}': {}",
                path,
                err
            );
            return Err(err);
        }
    };

    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&load_path) {
        if !crate::fs::permissions::check_permission(
            uid,
            gid,
            mode,
            crate::fs::permissions::PERM_READ,
        ) {
            return Err("Permission denied");
        }
    }

    let data = match crate::fs::vfs::read_file_to_vec(&load_path) {
        Ok(data) => data,
        Err(e) => {
            crate::serial_verbose_println!(
                "  lxe load_and_run: read_file_to_vec('{}') failed: {:?}",
                load_path,
                e
            );
            return Err("Failed to read program file");
        }
    };
    if data.is_empty() {
        return Err("Program file is empty");
    }

    let pd_phys = virtual_mem::create_user_page_directory_no_low_identity()
        .ok_or("Failed to create user page directory")?;
    let argv = build_spawn_argv(path, args);
    let envp = default_envp();
    let result = match load_linux_image_into_pd(&data, pd_phys, &linux_rootfs, &argv, &envp) {
        Ok(result) => result,
        Err(err) => {
            virtual_mem::destroy_user_page_directory(pd_phys);
            return Err(err);
        }
    };

    let tid = crate::task::scheduler::spawn_blocked(user_thread_trampoline, 100, name);
    if tid == 0 {
        crate::memory::vma::destroy_process(pd_phys);
        virtual_mem::destroy_user_page_directory(pd_phys);
        return Err("Failed to create thread");
    }

    let mmap_rand = random_page_offset(ASLR_MMAP_MAX_PAGES);
    let mmap_start = MMAP_BASE.wrapping_add(mmap_rand as u64 * PAGE_SIZE);
    crate::task::scheduler::set_thread_mmap_next(tid, mmap_start);
    crate::memory::vma::init_process(pd_phys, mmap_start);
    crate::task::scheduler::set_thread_user_info(tid, pd_phys, result.brk);
    crate::task::scheduler::set_thread_abi(tid, crate::task::abi::AbiPersonality::LinuxX86_64);
    crate::task::scheduler::set_thread_linux_rootfs(tid, &linux_rootfs);
    crate::task::scheduler::set_thread_cwd(tid, "/");
    if result.user_pages > 0 {
        crate::task::scheduler::adjust_thread_user_pages(tid, result.user_pages as i32);
    }

    if !try_store_pending_program(tid, result.entry, result.stack_top, 0) {
        crate::serial_verbose_println!(
            "lxe load_and_run: pending-program table full for '{}' (tid={})",
            path,
            tid
        );
        crate::task::scheduler::kill_thread(tid);
        return Err("Too many pending programs");
    }
    if !args.is_empty() {
        crate::task::scheduler::set_thread_args(tid, args);
    }

    let parent_caps = crate::task::scheduler::current_thread_capabilities();
    let caps = if parent_caps == 0 {
        crate::task::capabilities::CAP_ALL
    } else {
        parent_caps | crate::task::capabilities::CAP_AUTO_GRANTED
    };
    crate::task::scheduler::set_thread_capabilities(tid, caps);

    let uid = crate::task::scheduler::current_thread_uid();
    let gid = crate::task::scheduler::current_thread_gid();
    crate::task::scheduler::set_thread_identity(tid, uid, gid);

    let fmt = if is_elf(&data) { "elf64" } else { "flat" };
    crate::serial_verbose_println!(
        "spawn: '{}' -> T{} ({}, {} pages, entry={:#x})",
        path,
        tid,
        fmt,
        result.user_pages,
        result.entry
    );

    // Linux userspace probes terminal state very early (isatty/ioctl TCGETS).
    // Attach PTY stdio before wake so bash/readline never observes legacy fds.
    super::apply_spawn_stdio(tid, stdio);

    crate::task::scheduler::wake_thread(tid);
    Ok(tid)
}

fn decref_fd_kind(kind: crate::fs::fd_table::FdKind) {
    use crate::fs::fd_table::FdKind;
    match kind {
        FdKind::File { global_id } => crate::fs::vfs::decref(global_id),
        FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(pipe_id),
        FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(pipe_id),
        FdKind::LinuxSocket { socket_id } => crate::syscall::linux::socket_decref(socket_id),
        FdKind::Tty | FdKind::PtySlave { .. } | FdKind::LinuxProc { .. } | FdKind::None => {}
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
        None => return "lxe execve: no page directory on current thread",
    };

    let data = match crate::fs::vfs::read_file_to_vec(load_path) {
        Ok(data) => data,
        Err(_) => return "lxe execve: failed to read program file",
    };
    let new_pd = match virtual_mem::create_user_page_directory_no_low_identity() {
        Some(pd) => pd,
        None => return "lxe execve: failed to create page directory",
    };

    let linux_rootfs = LXE_ROOTFS;
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

    crate::task::scheduler::current_signal_reset_caught_for_exec();
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
        "lxe execve: unsupported architecture"
    }
}
