use super::*;

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::virtual_mem;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20B;
const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;

const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

const MAX_PE_SECTIONS: usize = 96;
const MAX_PE_IMAGE_SIZE: u64 = 512 * 1024 * 1024;

struct PeImageInfo {
    entry_rva: u32,
    image_base: u64,
    size_of_image: u32,
    size_of_headers: u32,
    section_count: u16,
    sections_offset: usize,
    import_rva: u32,
    import_size: u32,
    tls_rva: u32,
    tls_size: u32,
}

struct PeSection {
    virtual_address: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
    characteristics: u32,
}

/// Load and run a Windows x86_64 PE through wxe.
pub fn load_and_run_with_args(
    path: &str,
    name: &str,
    args: &str,
) -> Result<u32, &'static str> {
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
        return Err("wxe: Windows x86_64 ABI requires an x86_64 kernel");
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
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(path) {
        if !crate::fs::permissions::check_permission(
            uid,
            gid,
            mode,
            crate::fs::permissions::PERM_READ,
        ) {
            return Err("Permission denied");
        }
    }

    let data = match crate::fs::vfs::read_file_to_vec(path) {
        Ok(data) => data,
        Err(e) => {
            crate::serial_verbose_println!(
                "  wxe load_and_run: read_file_to_vec('{}') failed: {:?}",
                path,
                e
            );
            return Err("Failed to read program file");
        }
    };
    if data.is_empty() {
        return Err("Program file is empty");
    }

    let info = inspect_pe64(&data)?;
    if info.import_rva != 0 && info.import_size != 0 {
        crate::serial_verbose_println!(
            "wxe loader: '{}' imports PE DLLs (rva={:#x}, size={:#x}); DLL loader not wired yet",
            path,
            info.import_rva,
            info.import_size
        );
        return Err("wxe: PE imports require the WXE DLL loader");
    }
    if info.tls_rva != 0 && info.tls_size != 0 {
        return Err("wxe: PE TLS callbacks are not implemented yet");
    }

    let pd_phys = virtual_mem::create_user_page_directory_no_low_identity()
        .ok_or("Failed to create user page directory")?;
    let result = match load_pe_image_into_pd(&data, pd_phys, &info) {
        Ok(result) => result,
        Err(err) => {
            crate::memory::vma::destroy_process(pd_phys);
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
    crate::task::scheduler::set_thread_abi(tid, crate::task::abi::AbiPersonality::WindowsX86_64);
    crate::task::scheduler::set_thread_cwd(tid, "/System/var/wxe/drive_c");
    if result.user_pages > 0 {
        crate::task::scheduler::adjust_thread_user_pages(tid, result.user_pages as i32);
    }

    if !try_store_pending_program(tid, result.entry, result.stack_top, 0) {
        crate::serial_verbose_println!(
            "wxe load_and_run: pending-program table full for '{}' (tid={})",
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

    crate::serial_verbose_println!(
        "wxe spawn: '{}' -> T{} (pe32+, {} pages, image={:#x}, entry={:#x})",
        path,
        tid,
        result.user_pages,
        info.image_base,
        result.entry
    );

    super::apply_spawn_stdio(tid, stdio);
    crate::task::scheduler::wake_thread(tid);
    Ok(tid)
}

#[cfg(target_arch = "x86_64")]
fn load_pe_image_into_pd(
    data: &[u8],
    pd_phys: PhysAddr,
    info: &PeImageInfo,
) -> Result<super::LoadResult, &'static str> {
    let image_start = align_down(info.image_base, PAGE_SIZE);
    let image_delta = info.image_base - image_start;
    let image_size = image_delta
        .checked_add(info.size_of_image as u64)
        .ok_or("wxe: image size overflow")?;
    let image_pages = align_up(image_size, PAGE_SIZE) / PAGE_SIZE;

    let image_mapped = virtual_mem::map_pages_range_in_pd(
        pd_phys,
        VirtAddr::new(image_start),
        image_pages,
        PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag(),
        true,
    )?;

    let stack_aslr_offset = random_page_offset(ASLR_STACK_MAX_PAGES) as u64 * PAGE_SIZE;
    let aslr_stack_top = USER_STACK_TOP - stack_aslr_offset;
    let stack_bottom = aslr_stack_top - USER_STACK_PAGES * PAGE_SIZE;
    let stack_mapped = virtual_mem::map_pages_range_in_pd(
        pd_phys,
        VirtAddr::new(stack_bottom),
        USER_STACK_PAGES,
        PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag(),
        true,
    )?;

    let tramp_mapped = install_sigreturn_trampoline(pd_phys)?;

    let header_len = core::cmp::min(info.size_of_headers as usize, data.len());
    if header_len > 0 {
        copy_to_user_pd(pd_phys, info.image_base, &data[..header_len]);
    }

    for idx in 0..info.section_count as usize {
        let section = parse_section(data, info.sections_offset + idx * 40)?;
        copy_section_to_user(data, pd_phys, info.image_base, &section)?;
    }

    protect_mapped_page_range(
        pd_phys,
        image_start,
        image_pages,
        PAGE_USER | virtual_mem::page_nx_flag(),
    )?;
    for idx in 0..info.section_count as usize {
        let section = parse_section(data, info.sections_offset + idx * 40)?;
        protect_section(pd_phys, info.image_base, &section)?;
    }

    let entry = info
        .image_base
        .checked_add(info.entry_rva as u64)
        .ok_or("wxe: entry address overflow")?;
    let brk = image_start
        .checked_add(image_pages * PAGE_SIZE)
        .ok_or("wxe: image brk overflow")?;

    // Windows x64 callees expect a 32-byte home area. Entering via iretq is
    // jump-like, so place RSP in a call-compatible state (`RSP % 16 == 8`).
    let stack_top = (aslr_stack_top & !0xF).saturating_sub(8 + 32);

    Ok(super::LoadResult {
        entry,
        brk,
        user_pages: image_mapped + stack_mapped + tramp_mapped,
        stack_top,
    })
}

fn inspect_pe64(data: &[u8]) -> Result<PeImageInfo, &'static str> {
    if data.len() < 0x40 {
        return Err("wxe: file too small for DOS header");
    }
    if read_u16(data, 0)? != IMAGE_DOS_SIGNATURE {
        return Err("wxe: missing MZ DOS signature");
    }

    let pe_offset = read_u32(data, 0x3c)? as usize;
    if pe_offset.checked_add(24).map_or(true, |end| end > data.len()) {
        return Err("wxe: PE header offset out of bounds");
    }
    if read_u32(data, pe_offset)? != IMAGE_NT_SIGNATURE {
        return Err("wxe: missing PE signature");
    }

    let coff = pe_offset + 4;
    let machine = read_u16(data, coff)?;
    let section_count = read_u16(data, coff + 2)?;
    let optional_size = read_u16(data, coff + 16)? as usize;
    let opt = coff + 20;
    if opt.checked_add(optional_size).map_or(true, |end| end > data.len()) {
        return Err("wxe: optional header out of bounds");
    }
    if optional_size < 112 {
        return Err("wxe: optional header too small");
    }
    if section_count as usize > MAX_PE_SECTIONS {
        return Err("wxe: too many PE sections");
    }
    if machine != IMAGE_FILE_MACHINE_AMD64 {
        return Err("wxe: expected AMD64 PE image");
    }
    if read_u16(data, opt)? != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
        return Err("wxe: expected PE32+ optional header");
    }

    let subsystem = read_u16(data, opt + 68)?;
    if subsystem != IMAGE_SUBSYSTEM_WINDOWS_CUI {
        return Err("wxe: only console subsystem PE images are supported");
    }

    let image_base = read_u64(data, opt + 24)?;
    let size_of_image = read_u32(data, opt + 56)?;
    let size_of_headers = read_u32(data, opt + 60)?;
    validate_user_image_range(image_base, size_of_image as u64)?;

    let number_of_rva_and_sizes = read_u32(data, opt + 108)?;
    let data_dirs = opt + 112;
    let (import_rva, import_size) =
        read_data_dir(data, data_dirs, number_of_rva_and_sizes, 1)?;
    let (tls_rva, tls_size) = read_data_dir(data, data_dirs, number_of_rva_and_sizes, 9)?;

    let sections_offset = opt
        .checked_add(optional_size)
        .ok_or("wxe: section table offset overflow")?;
    let section_table_size = (section_count as usize)
        .checked_mul(40)
        .ok_or("wxe: section table size overflow")?;
    if sections_offset
        .checked_add(section_table_size)
        .map_or(true, |end| end > data.len())
    {
        return Err("wxe: section table out of bounds");
    }

    Ok(PeImageInfo {
        entry_rva: read_u32(data, opt + 16)?,
        image_base,
        size_of_image,
        size_of_headers,
        section_count,
        sections_offset,
        import_rva,
        import_size,
        tls_rva,
        tls_size,
    })
}

fn validate_user_image_range(image_base: u64, size_of_image: u64) -> Result<(), &'static str> {
    if image_base < PROGRAM_LOAD_ADDR {
        return Err("wxe: image base is below the user load floor");
    }
    if size_of_image == 0 || size_of_image > MAX_PE_IMAGE_SIZE {
        return Err("wxe: invalid PE image size");
    }
    let image_end = image_base
        .checked_add(size_of_image)
        .ok_or("wxe: image address overflow")?;
    if image_end >= USER_STACK_TOP {
        return Err("wxe: PE image overlaps reserved user address space");
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn copy_section_to_user(
    data: &[u8],
    pd_phys: PhysAddr,
    image_base: u64,
    section: &PeSection,
) -> Result<(), &'static str> {
    if section.raw_size == 0 {
        return Ok(());
    }
    let raw_start = section.raw_ptr as usize;
    let raw_size = section.raw_size as usize;
    if raw_start
        .checked_add(raw_size)
        .map_or(true, |end| end > data.len())
    {
        return Err("wxe: section raw data out of bounds");
    }
    let dst = image_base
        .checked_add(section.virtual_address as u64)
        .ok_or("wxe: section virtual address overflow")?;
    let section_bytes = core::cmp::min(raw_size, section.virtual_size.max(section.raw_size) as usize);
    copy_to_user_pd(pd_phys, dst, &data[raw_start..raw_start + section_bytes]);
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn protect_section(
    pd_phys: PhysAddr,
    image_base: u64,
    section: &PeSection,
) -> Result<(), &'static str> {
    let section_len = section.virtual_size.max(section.raw_size) as u64;
    if section_len == 0 {
        return Ok(());
    }
    let start = image_base
        .checked_add(section.virtual_address as u64)
        .ok_or("wxe: section protect address overflow")?;
    let end = start
        .checked_add(section_len)
        .ok_or("wxe: section protect size overflow")?;
    let page_start = align_down(start, PAGE_SIZE);
    let page_end = align_up(end, PAGE_SIZE);
    let pages = (page_end - page_start) / PAGE_SIZE;

    let mut flags = PAGE_USER;
    if section.characteristics & IMAGE_SCN_MEM_WRITE != 0 {
        flags |= PAGE_WRITABLE;
    }
    if section.characteristics & IMAGE_SCN_MEM_EXECUTE == 0 {
        flags |= virtual_mem::page_nx_flag();
    }
    protect_mapped_page_range(pd_phys, page_start, pages, flags)
}

#[cfg(target_arch = "x86_64")]
fn protect_mapped_page_range(
    pd_phys: PhysAddr,
    start: u64,
    pages: u64,
    flags: u64,
) -> Result<(), &'static str> {
    for idx in 0..pages {
        let addr = start + idx * PAGE_SIZE;
        if !virtual_mem::set_page_flags_in_pd(pd_phys, VirtAddr::new(addr), flags) {
            return Err("wxe: failed to protect PE page");
        }
    }
    Ok(())
}

fn parse_section(data: &[u8], off: usize) -> Result<PeSection, &'static str> {
    Ok(PeSection {
        virtual_size: read_u32(data, off + 8)?,
        virtual_address: read_u32(data, off + 12)?,
        raw_size: read_u32(data, off + 16)?,
        raw_ptr: read_u32(data, off + 20)?,
        characteristics: read_u32(data, off + 36)?,
    })
}

#[cfg(target_arch = "x86_64")]
fn copy_to_user_pd(pd_phys: PhysAddr, dst: u64, src: &[u8]) {
    if src.is_empty() {
        return;
    }
    unsafe {
        let saved_flags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) saved_flags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));
        let old_pt = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

        core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len());

        core::arch::asm!("mov cr3, {}", in(reg) old_pt);
        core::arch::asm!("push {}; popfq", in(reg) saved_flags, options(nomem));
    }
}

fn read_data_dir(
    data: &[u8],
    data_dirs: usize,
    count: u32,
    index: usize,
) -> Result<(u32, u32), &'static str> {
    if count as usize <= index {
        return Ok((0, 0));
    }
    let off = data_dirs
        .checked_add(index * 8)
        .ok_or("wxe: data directory offset overflow")?;
    Ok((read_u32(data, off)?, read_u32(data, off + 4)?))
}

fn read_u16(data: &[u8], off: usize) -> Result<u16, &'static str> {
    let bytes = data.get(off..off + 2).ok_or("wxe: truncated u16")?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], off: usize) -> Result<u32, &'static str> {
    let bytes = data.get(off..off + 4).ok_or("wxe: truncated u32")?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], off: usize) -> Result<u64, &'static str> {
    let bytes = data.get(off..off + 8).ok_or("wxe: truncated u64")?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn align_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}
