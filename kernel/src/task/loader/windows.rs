use super::*;

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::virtual_mem;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20B;
const IMAGE_SUBSYSTEM_WINDOWS_CUI: u16 = 3;

const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;

const IMAGE_DIRECTORY_ENTRY_EXPORT: usize = 0;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_DIRECTORY_ENTRY_TLS: usize = 9;
const IMAGE_ORDINAL_FLAG64: u64 = 0x8000_0000_0000_0000;

const MAX_PE_SECTIONS: usize = 96;
const MAX_PE_IMAGE_SIZE: u64 = 512 * 1024 * 1024;
const MAX_WXE_DLLS: usize = 64;
const MAX_IMPORT_DESCRIPTORS: usize = 256;
const MAX_IMPORTS_PER_DLL: usize = 4096;
const MAX_EXPORTS_PER_DLL: usize = 8192;
const MAX_FORWARDER_DEPTH: u32 = 8;
const WXE_SYSTEM32: &str = "/System/var/wxe/drive_c/Windows/System32";
const WXE_DRIVE_C_ROOT: &str = "/System/var/wxe/drive_c";
const WXE_PROCESS_BLOCK_BASE: u64 = 0x0000_7FFE_E000_0000;
const WXE_TEB_BASE: u64 = WXE_PROCESS_BLOCK_BASE;
const WXE_PEB_BASE: u64 = WXE_PROCESS_BLOCK_BASE + 0x1000;
const WXE_PROCESS_PARAMS_BASE: u64 = WXE_PROCESS_BLOCK_BASE + 0x2000;
const WXE_STRINGS_BASE: u64 = WXE_PROCESS_BLOCK_BASE + 0x3000;
const WXE_PROCESS_BLOCK_PAGES: u64 = 8;
const WXE_PROCESS_BLOCK_SIZE: usize = (WXE_PROCESS_BLOCK_PAGES as usize) * PAGE_SIZE as usize;
const WXE_CMDLINE_A_ADDR: u64 = WXE_STRINGS_BASE;
const WXE_CMDLINE_W_ADDR: u64 = WXE_STRINGS_BASE + 0x0800;
const WXE_ENV_A_ADDR: u64 = WXE_STRINGS_BASE + 0x2000;
const WXE_ENV_W_ADDR: u64 = WXE_STRINGS_BASE + 0x3000;
const WXE_MAX_STRING_BYTES: usize = 0x0800;
const WXE_MAX_ENV_BYTES: usize = 0x1000;

#[derive(Clone, Copy)]
struct PeImageInfo {
    entry_rva: u32,
    image_base: u64,
    size_of_image: u32,
    size_of_headers: u32,
    section_count: u16,
    sections_offset: usize,
    export_rva: u32,
    export_size: u32,
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

struct WxeLoadedModule {
    name: String,
    data: Vec<u8>,
    info: PeImageInfo,
}

struct WxeLoadContext {
    modules: Vec<WxeLoadedModule>,
    user_pages: u32,
}

struct WxeRuntimePointers {
    pages: u32,
    command_line_a: u64,
    command_line_w: u64,
    environment_a: u64,
    environment_w: u64,
}

/// Load and run a Windows x86_64 PE through wxe.
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
    if info.tls_rva != 0 && info.tls_size != 0 {
        return Err("wxe: PE TLS callbacks are not implemented yet");
    }

    let pd_phys = virtual_mem::create_user_page_directory_no_low_identity()
        .ok_or("Failed to create user page directory")?;
    let result = match load_pe_image_into_pd(&data, pd_phys, &info, path, name, args) {
        Ok(result) => result,
        Err(err) => {
            crate::syscall::windows::cleanup_process(pd_phys.as_u64());
            crate::memory::vma::destroy_process(pd_phys);
            virtual_mem::destroy_user_page_directory(pd_phys);
            return Err(err);
        }
    };

    let tid = crate::task::scheduler::spawn_blocked(user_thread_trampoline, 100, name);
    if tid == 0 {
        crate::syscall::windows::cleanup_process(pd_phys.as_u64());
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
        crate::syscall::windows::cleanup_process(pd_phys.as_u64());
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
    path: &str,
    name: &str,
    args: &str,
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

    let mut imports = WxeLoadContext {
        modules: Vec::new(),
        user_pages: 0,
    };
    if info.import_rva != 0 && info.import_size != 0 {
        resolve_imports_for_image(pd_phys, data, info, info.image_base, &mut imports, "main")?;
    }

    let runtime = install_wxe_process_block(pd_phys, info, &imports, path, name, args)?;
    crate::serial_verbose_println!(
        "wxe loader: process block peb={:#x} teb={:#x} cmdA={:#x} cmdW={:#x} envA={:#x} envW={:#x}",
        WXE_PEB_BASE,
        WXE_TEB_BASE,
        runtime.command_line_a,
        runtime.command_line_w,
        runtime.environment_a,
        runtime.environment_w
    );

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
        user_pages: image_mapped + stack_mapped + tramp_mapped + imports.user_pages + runtime.pages,
        stack_top,
    })
}

#[cfg(target_arch = "x86_64")]
fn install_wxe_process_block(
    pd_phys: PhysAddr,
    info: &PeImageInfo,
    imports: &WxeLoadContext,
    path: &str,
    name: &str,
    args: &str,
) -> Result<WxeRuntimePointers, &'static str> {
    let pages = virtual_mem::map_pages_range_in_pd(
        pd_phys,
        VirtAddr::new(WXE_PROCESS_BLOCK_BASE),
        WXE_PROCESS_BLOCK_PAGES,
        PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag(),
        true,
    )?;

    let mut block = alloc::vec![0u8; WXE_PROCESS_BLOCK_SIZE];
    let image_path = native_to_windows_image_path(path);
    let command_line = build_windows_command_line(&image_path, args);
    write_ascii_z(
        &mut block,
        WXE_CMDLINE_A_ADDR,
        &command_line,
        WXE_MAX_STRING_BYTES,
    )?;
    let command_line_w_len = write_utf16_z(
        &mut block,
        WXE_CMDLINE_W_ADDR,
        &command_line,
        WXE_MAX_STRING_BYTES,
    )?;

    let env_entries = default_environment_entries();
    write_env_a(&mut block, WXE_ENV_A_ADDR, &env_entries, WXE_MAX_ENV_BYTES)?;
    write_env_w(&mut block, WXE_ENV_W_ADDR, &env_entries, WXE_MAX_ENV_BYTES)?;

    write_teb(&mut block);
    write_peb(&mut block, info.image_base);
    write_process_parameters(&mut block, command_line_w_len, &image_path)?;

    copy_to_user_pd(pd_phys, WXE_PROCESS_BLOCK_BASE, &block);

    let mut modules = Vec::new();
    modules.push(crate::syscall::windows::WxeProcessModuleInit {
        name: String::from(name),
        base: info.image_base,
    });
    modules.push(crate::syscall::windows::WxeProcessModuleInit {
        name: windows_basename(&image_path),
        base: info.image_base,
    });
    for module in &imports.modules {
        modules.push(crate::syscall::windows::WxeProcessModuleInit {
            name: module.name.clone(),
            base: module.info.image_base,
        });
    }

    crate::syscall::windows::register_process(crate::syscall::windows::WxeProcessInit {
        pd: pd_phys.as_u64(),
        peb_base: WXE_PEB_BASE,
        teb_base: WXE_TEB_BASE,
        main_image_base: info.image_base,
        command_line_a: WXE_CMDLINE_A_ADDR,
        command_line_w: WXE_CMDLINE_W_ADDR,
        environment_a: WXE_ENV_A_ADDR,
        environment_w: WXE_ENV_W_ADDR,
        modules,
    });

    Ok(WxeRuntimePointers {
        pages,
        command_line_a: WXE_CMDLINE_A_ADDR,
        command_line_w: WXE_CMDLINE_W_ADDR,
        environment_a: WXE_ENV_A_ADDR,
        environment_w: WXE_ENV_W_ADDR,
    })
}

fn write_teb(block: &mut [u8]) {
    put_block_u64(block, WXE_TEB_BASE + 0x08, USER_STACK_TOP);
    put_block_u64(
        block,
        WXE_TEB_BASE + 0x10,
        USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE,
    );
    put_block_u64(block, WXE_TEB_BASE + 0x30, WXE_TEB_BASE);
    put_block_u64(block, WXE_TEB_BASE + 0x60, WXE_PEB_BASE);
}

fn write_peb(block: &mut [u8], image_base: u64) {
    put_block_u8(block, WXE_PEB_BASE + 0x02, 0);
    put_block_u64(block, WXE_PEB_BASE + 0x10, image_base);
    put_block_u64(block, WXE_PEB_BASE + 0x18, 0);
    put_block_u64(block, WXE_PEB_BASE + 0x20, WXE_PROCESS_PARAMS_BASE);
    put_block_u32(block, WXE_PEB_BASE + 0x118, 10);
    put_block_u32(block, WXE_PEB_BASE + 0x11C, 0);
    put_block_u32(block, WXE_PEB_BASE + 0x120, 19041);
}

fn write_process_parameters(
    block: &mut [u8],
    command_line_w_len: usize,
    image_path: &str,
) -> Result<(), &'static str> {
    let image_path_addr = WXE_CMDLINE_W_ADDR + 0x0800;
    let image_path_w_len = write_utf16_z(block, image_path_addr, image_path, 0x0400)?;

    put_block_u32(block, WXE_PROCESS_PARAMS_BASE, 0x0900);
    put_block_u32(block, WXE_PROCESS_PARAMS_BASE + 0x04, 0x0900);
    put_block_u32(block, WXE_PROCESS_PARAMS_BASE + 0x08, 1);
    put_unicode_string(
        block,
        WXE_PROCESS_PARAMS_BASE + 0x60,
        image_path_addr,
        image_path_w_len,
    );
    put_unicode_string(
        block,
        WXE_PROCESS_PARAMS_BASE + 0x70,
        WXE_CMDLINE_W_ADDR,
        command_line_w_len,
    );
    put_block_u64(block, WXE_PROCESS_PARAMS_BASE + 0x80, WXE_ENV_W_ADDR);
    Ok(())
}

fn put_unicode_string(block: &mut [u8], addr: u64, buffer: u64, byte_len: usize) {
    let len = byte_len.min(u16::MAX as usize) as u16;
    let max_len = len.saturating_add(2);
    put_block_u16(block, addr, len);
    put_block_u16(block, addr + 2, max_len);
    put_block_u64(block, addr + 8, buffer);
}

fn write_ascii_z(
    block: &mut [u8],
    addr: u64,
    value: &str,
    max_bytes: usize,
) -> Result<usize, &'static str> {
    let off = block_offset(addr).ok_or("wxe: process block address OOB")?;
    if max_bytes == 0 || off >= block.len() {
        return Err("wxe: process block string OOB");
    }
    let len = value.len().min(max_bytes - 1);
    if off + len + 1 > block.len() {
        return Err("wxe: process block string overflow");
    }
    block[off..off + len].copy_from_slice(&value.as_bytes()[..len]);
    block[off + len] = 0;
    Ok(len)
}

fn write_utf16_z(
    block: &mut [u8],
    addr: u64,
    value: &str,
    max_bytes: usize,
) -> Result<usize, &'static str> {
    let off = block_offset(addr).ok_or("wxe: process block address OOB")?;
    let max_units = max_bytes / 2;
    if max_units == 0 || off >= block.len() {
        return Err("wxe: process block UTF-16 string OOB");
    }
    let mut units = 0usize;
    for ch in value.encode_utf16().take(max_units - 1) {
        let pos = off + units * 2;
        if pos + 2 > block.len() {
            return Err("wxe: process block UTF-16 string overflow");
        }
        block[pos..pos + 2].copy_from_slice(&ch.to_le_bytes());
        units += 1;
    }
    let nul = off + units * 2;
    if nul + 2 > block.len() {
        return Err("wxe: process block UTF-16 string overflow");
    }
    block[nul] = 0;
    block[nul + 1] = 0;
    Ok(units * 2)
}

fn write_env_a(
    block: &mut [u8],
    addr: u64,
    entries: &[String],
    max_bytes: usize,
) -> Result<(), &'static str> {
    let off = block_offset(addr).ok_or("wxe: process block env OOB")?;
    let end = off.checked_add(max_bytes).ok_or("wxe: env overflow")?;
    if end > block.len() || max_bytes < 2 {
        return Err("wxe: process block env overflow");
    }
    let mut pos = off;
    for entry in entries {
        let bytes = entry.as_bytes();
        if pos + bytes.len() + 1 >= end {
            break;
        }
        block[pos..pos + bytes.len()].copy_from_slice(bytes);
        pos += bytes.len();
        block[pos] = 0;
        pos += 1;
    }
    block[pos.min(end - 1)] = 0;
    Ok(())
}

fn write_env_w(
    block: &mut [u8],
    addr: u64,
    entries: &[String],
    max_bytes: usize,
) -> Result<(), &'static str> {
    let off = block_offset(addr).ok_or("wxe: process block env OOB")?;
    let end = off.checked_add(max_bytes).ok_or("wxe: env overflow")?;
    if end > block.len() || max_bytes < 4 {
        return Err("wxe: process block env overflow");
    }
    let mut pos = off;
    for entry in entries {
        for unit in entry.encode_utf16() {
            if pos + 4 >= end {
                block[pos] = 0;
                block[pos + 1] = 0;
                return Ok(());
            }
            block[pos..pos + 2].copy_from_slice(&unit.to_le_bytes());
            pos += 2;
        }
        block[pos] = 0;
        block[pos + 1] = 0;
        pos += 2;
    }
    block[pos.min(end - 2)] = 0;
    block[(pos + 1).min(end - 1)] = 0;
    Ok(())
}

fn default_environment_entries() -> Vec<String> {
    let mut entries = Vec::new();
    entries.push(String::from("COMSPEC=C:\\Windows\\System32\\cmd.exe"));
    entries.push(String::from("SystemDrive=C:"));
    entries.push(String::from("SystemRoot=C:\\Windows"));
    entries.push(String::from("WINDIR=C:\\Windows"));
    entries.push(String::from("USERPROFILE=C:\\Users\\Default"));
    entries.push(String::from("TEMP=C:\\Windows\\Temp"));
    entries.push(String::from("TMP=C:\\Windows\\Temp"));
    entries.push(String::from("PATHEXT=.COM;.EXE;.BAT;.CMD"));
    entries.push(String::from(
        "PATH=C:\\Windows\\System32;C:\\Windows;C:\\Program Files\\Sysinternals;C:\\Program Files\\Microsoft\\Edit",
    ));
    entries
}

fn native_to_windows_image_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(WXE_DRIVE_C_ROOT) {
        let mut out = String::from("C:");
        if rest.is_empty() {
            out.push('\\');
        } else {
            for ch in rest.chars() {
                out.push(if ch == '/' { '\\' } else { ch });
            }
        }
        out
    } else {
        String::from(path)
    }
}

fn build_windows_command_line(image_path: &str, args: &str) -> String {
    let mut out = String::new();
    quote_windows_arg(image_path, &mut out);
    if !args.trim().is_empty() {
        out.push(' ');
        out.push_str(args.trim());
    }
    out
}

fn quote_windows_arg(arg: &str, out: &mut String) {
    if arg.bytes().any(|b| b == b' ' || b == b'\t' || b == b'"') {
        out.push('"');
        for ch in arg.chars() {
            if ch == '"' {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push('"');
    } else {
        out.push_str(arg);
    }
}

fn windows_basename(path: &str) -> String {
    path.rsplit(|c| c == '\\' || c == '/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn block_offset(addr: u64) -> Option<usize> {
    if addr < WXE_PROCESS_BLOCK_BASE {
        return None;
    }
    let off = (addr - WXE_PROCESS_BLOCK_BASE) as usize;
    if off < WXE_PROCESS_BLOCK_SIZE {
        Some(off)
    } else {
        None
    }
}

fn put_block_u8(block: &mut [u8], addr: u64, value: u8) {
    if let Some(off) = block_offset(addr) {
        block[off] = value;
    }
}

fn put_block_u16(block: &mut [u8], addr: u64, value: u16) {
    if let Some(off) = block_offset(addr) {
        block[off..off + 2].copy_from_slice(&value.to_le_bytes());
    }
}

fn put_block_u32(block: &mut [u8], addr: u64, value: u32) {
    if let Some(off) = block_offset(addr) {
        block[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn put_block_u64(block: &mut [u8], addr: u64, value: u64) {
    if let Some(off) = block_offset(addr) {
        block[off..off + 8].copy_from_slice(&value.to_le_bytes());
    }
}

#[cfg(target_arch = "x86_64")]
fn map_pe_module_into_pd(
    data: &[u8],
    pd_phys: PhysAddr,
    info: &PeImageInfo,
) -> Result<u32, &'static str> {
    let image_start = align_down(info.image_base, PAGE_SIZE);
    let image_delta = info.image_base - image_start;
    let image_size = image_delta
        .checked_add(info.size_of_image as u64)
        .ok_or("wxe: DLL image size overflow")?;
    let image_pages = align_up(image_size, PAGE_SIZE) / PAGE_SIZE;

    let mapped = virtual_mem::map_pages_range_in_pd(
        pd_phys,
        VirtAddr::new(image_start),
        image_pages,
        PAGE_WRITABLE | PAGE_USER | virtual_mem::page_nx_flag(),
        true,
    )?;

    let header_len = core::cmp::min(info.size_of_headers as usize, data.len());
    if header_len > 0 {
        copy_to_user_pd(pd_phys, info.image_base, &data[..header_len]);
    }

    for idx in 0..info.section_count as usize {
        let section = parse_section(data, info.sections_offset + idx * 40)?;
        copy_section_to_user(data, pd_phys, info.image_base, &section)?;
    }

    Ok(mapped)
}

#[cfg(target_arch = "x86_64")]
fn protect_pe_image(
    pd_phys: PhysAddr,
    data: &[u8],
    info: &PeImageInfo,
) -> Result<(), &'static str> {
    let image_start = align_down(info.image_base, PAGE_SIZE);
    let image_delta = info.image_base - image_start;
    let image_size = image_delta
        .checked_add(info.size_of_image as u64)
        .ok_or("wxe: image protect size overflow")?;
    let image_pages = align_up(image_size, PAGE_SIZE) / PAGE_SIZE;

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
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn resolve_imports_for_image(
    pd_phys: PhysAddr,
    data: &[u8],
    info: &PeImageInfo,
    image_base: u64,
    ctx: &mut WxeLoadContext,
    image_name: &str,
) -> Result<(), &'static str> {
    if info.import_rva == 0 || info.import_size == 0 {
        return Ok(());
    }

    for desc_idx in 0..MAX_IMPORT_DESCRIPTORS {
        let desc_rva = info
            .import_rva
            .checked_add((desc_idx * 20) as u32)
            .ok_or("wxe: import descriptor RVA overflow")?;
        if (desc_rva - info.import_rva) >= info.import_size {
            return Err("wxe: import descriptor table is not terminated");
        }
        let desc_off =
            rva_to_file_offset(data, info, desc_rva, 20).ok_or("wxe: import descriptor OOB")?;
        let original_first_thunk = read_u32(data, desc_off)?;
        let name_rva = read_u32(data, desc_off + 12)?;
        let first_thunk = read_u32(data, desc_off + 16)?;

        if original_first_thunk == 0 && name_rva == 0 && first_thunk == 0 {
            return Ok(());
        }
        if name_rva == 0 || first_thunk == 0 {
            return Err("wxe: malformed import descriptor");
        }

        let dll_name = read_cstr_rva(data, info, name_rva, 260)?;
        let module_idx = ensure_wxe_module_loaded(ctx, pd_phys, &dll_name)?;
        let lookup_rva = if original_first_thunk != 0 {
            original_first_thunk
        } else {
            first_thunk
        };

        for thunk_idx in 0..MAX_IMPORTS_PER_DLL {
            let thunk_rva = lookup_rva
                .checked_add((thunk_idx * 8) as u32)
                .ok_or("wxe: import thunk RVA overflow")?;
            let thunk_off =
                rva_to_file_offset(data, info, thunk_rva, 8).ok_or("wxe: import thunk OOB")?;
            let thunk = read_u64(data, thunk_off)?;
            if thunk == 0 {
                break;
            }

            let target = if thunk & IMAGE_ORDINAL_FLAG64 != 0 {
                let ordinal = (thunk & 0xFFFF) as u16;
                resolve_export_by_ordinal(ctx, pd_phys, module_idx, ordinal, 0)?
            } else {
                let import_name_rva = (thunk as u32)
                    .checked_add(2)
                    .ok_or("wxe: import name RVA overflow")?;
                let import_name = read_cstr_rva(data, info, import_name_rva, 512)?;
                resolve_export_by_name(ctx, pd_phys, module_idx, &import_name, 0)?
            };

            let iat_rva = first_thunk
                .checked_add((thunk_idx * 8) as u32)
                .ok_or("wxe: IAT RVA overflow")?;
            let iat_addr = image_base
                .checked_add(iat_rva as u64)
                .ok_or("wxe: IAT address overflow")?;
            copy_to_user_pd(pd_phys, iat_addr, &target.to_le_bytes());
        }
    }

    crate::serial_verbose_println!(
        "wxe loader: '{}' import descriptor limit reached",
        image_name
    );
    Err("wxe: too many import descriptors")
}

#[cfg(target_arch = "x86_64")]
fn ensure_wxe_module_loaded(
    ctx: &mut WxeLoadContext,
    pd_phys: PhysAddr,
    raw_name: &str,
) -> Result<usize, &'static str> {
    let name = normalize_dll_name(raw_name);
    if let Some(idx) = find_loaded_module(ctx, &name) {
        return Ok(idx);
    }
    if ctx.modules.len() >= MAX_WXE_DLLS {
        return Err("wxe: too many imported DLLs");
    }

    let path = wxe_system32_path(&name);
    let data = crate::fs::vfs::read_file_to_vec(&path).map_err(|_| "wxe: imported DLL missing")?;
    let info = inspect_pe64(&data)?;
    if info.tls_rva != 0 && info.tls_size != 0 {
        return Err("wxe: DLL TLS callbacks are not implemented yet");
    }

    let mapped_pages = map_pe_module_into_pd(&data, pd_phys, &info)?;
    ctx.user_pages = ctx.user_pages.saturating_add(mapped_pages);

    let idx = ctx.modules.len();
    ctx.modules.push(WxeLoadedModule { name, data, info });

    let module_info = ctx.modules[idx].info;
    if module_info.import_rva != 0 && module_info.import_size != 0 {
        let module_data = ctx.modules[idx].data.clone();
        let module_name = ctx.modules[idx].name.clone();
        resolve_imports_for_image(
            pd_phys,
            &module_data,
            &module_info,
            module_info.image_base,
            ctx,
            &module_name,
        )?;
    }

    let module_data = ctx.modules[idx].data.clone();
    protect_pe_image(pd_phys, &module_data, &module_info)?;

    crate::serial_verbose_println!(
        "wxe loader: mapped DLL '{}' base={:#x} pages={}",
        ctx.modules[idx].name,
        module_info.image_base,
        mapped_pages
    );
    Ok(idx)
}

fn find_loaded_module(ctx: &WxeLoadContext, name: &str) -> Option<usize> {
    ctx.modules
        .iter()
        .position(|module| module.name.eq_ignore_ascii_case(name))
}

#[cfg(target_arch = "x86_64")]
fn resolve_export_by_name(
    ctx: &mut WxeLoadContext,
    pd_phys: PhysAddr,
    module_idx: usize,
    name: &str,
    depth: u32,
) -> Result<u64, &'static str> {
    if depth > MAX_FORWARDER_DEPTH {
        return Err("wxe: export forwarder depth exceeded");
    }

    let ordinal_index = {
        let module = ctx
            .modules
            .get(module_idx)
            .ok_or("wxe: imported DLL index invalid")?;
        if module.info.export_rva == 0 || module.info.export_size == 0 {
            return Err("wxe: imported DLL has no exports");
        }
        let dir_off = rva_to_file_offset(&module.data, &module.info, module.info.export_rva, 40)
            .ok_or("wxe: export directory OOB")?;
        let number_of_names = read_u32(&module.data, dir_off + 24)? as usize;
        let address_of_names = read_u32(&module.data, dir_off + 32)?;
        let address_of_ordinals = read_u32(&module.data, dir_off + 36)?;
        if number_of_names > MAX_EXPORTS_PER_DLL {
            return Err("wxe: export name table too large");
        }

        let mut found = None;
        for idx in 0..number_of_names {
            let name_ptr_off = rva_to_file_offset(
                &module.data,
                &module.info,
                address_of_names + (idx * 4) as u32,
                4,
            )
            .ok_or("wxe: export name table OOB")?;
            let export_name_rva = read_u32(&module.data, name_ptr_off)?;
            let export_name = read_cstr_rva(&module.data, &module.info, export_name_rva, 512)?;
            if export_name == name {
                let ord_off = rva_to_file_offset(
                    &module.data,
                    &module.info,
                    address_of_ordinals + (idx * 2) as u32,
                    2,
                )
                .ok_or("wxe: export ordinal table OOB")?;
                found = Some(read_u16(&module.data, ord_off)? as u32);
                break;
            }
        }
        found
    };

    match ordinal_index {
        Some(index) => resolve_export_by_ordinal_index(ctx, pd_phys, module_idx, index, depth),
        None => Err("wxe: imported export not found"),
    }
}

#[cfg(target_arch = "x86_64")]
fn resolve_export_by_ordinal(
    ctx: &mut WxeLoadContext,
    pd_phys: PhysAddr,
    module_idx: usize,
    ordinal: u16,
    depth: u32,
) -> Result<u64, &'static str> {
    let ordinal_index = {
        let module = ctx
            .modules
            .get(module_idx)
            .ok_or("wxe: imported DLL index invalid")?;
        let dir_off = rva_to_file_offset(&module.data, &module.info, module.info.export_rva, 40)
            .ok_or("wxe: export directory OOB")?;
        let base = read_u32(&module.data, dir_off + 16)?;
        let ordinal = ordinal as u32;
        if ordinal < base {
            return Err("wxe: export ordinal below base");
        }
        ordinal - base
    };
    resolve_export_by_ordinal_index(ctx, pd_phys, module_idx, ordinal_index, depth)
}

#[cfg(target_arch = "x86_64")]
fn resolve_export_by_ordinal_index(
    ctx: &mut WxeLoadContext,
    pd_phys: PhysAddr,
    module_idx: usize,
    ordinal_index: u32,
    depth: u32,
) -> Result<u64, &'static str> {
    if depth > MAX_FORWARDER_DEPTH {
        return Err("wxe: export forwarder depth exceeded");
    }

    let (function_rva, module_base, export_rva, export_size, forwarder) = {
        let module = ctx
            .modules
            .get(module_idx)
            .ok_or("wxe: imported DLL index invalid")?;
        let dir_off = rva_to_file_offset(&module.data, &module.info, module.info.export_rva, 40)
            .ok_or("wxe: export directory OOB")?;
        let number_of_functions = read_u32(&module.data, dir_off + 20)?;
        let address_of_functions = read_u32(&module.data, dir_off + 28)?;
        if ordinal_index >= number_of_functions
            || number_of_functions as usize > MAX_EXPORTS_PER_DLL
        {
            return Err("wxe: export ordinal out of range");
        }
        let func_off = rva_to_file_offset(
            &module.data,
            &module.info,
            address_of_functions + ordinal_index * 4,
            4,
        )
        .ok_or("wxe: export address table OOB")?;
        let function_rva = read_u32(&module.data, func_off)?;
        let forwarder = if function_rva >= module.info.export_rva
            && function_rva
                < module
                    .info
                    .export_rva
                    .saturating_add(module.info.export_size)
        {
            Some(read_cstr_rva(
                &module.data,
                &module.info,
                function_rva,
                512,
            )?)
        } else {
            None
        };
        (
            function_rva,
            module.info.image_base,
            module.info.export_rva,
            module.info.export_size,
            forwarder,
        )
    };

    if let Some(target) = forwarder {
        resolve_forwarder(ctx, pd_phys, &target, depth + 1)
    } else if function_rva >= export_rva && function_rva < export_rva.saturating_add(export_size) {
        Err("wxe: malformed export forwarder")
    } else {
        module_base
            .checked_add(function_rva as u64)
            .ok_or("wxe: export address overflow")
    }
}

#[cfg(target_arch = "x86_64")]
fn resolve_forwarder(
    ctx: &mut WxeLoadContext,
    pd_phys: PhysAddr,
    target: &str,
    depth: u32,
) -> Result<u64, &'static str> {
    let dot = target.find('.').ok_or("wxe: malformed export forwarder")?;
    let mut dll_name = String::from(&target[..dot]);
    if !dll_name.ends_with(".dll") && !dll_name.ends_with(".DLL") {
        dll_name.push_str(".dll");
    }
    let export = &target[dot + 1..];
    let module_idx = ensure_wxe_module_loaded(ctx, pd_phys, &dll_name)?;
    if let Some(ordinal) = export.strip_prefix('#') {
        let ordinal = parse_decimal_u16(ordinal).ok_or("wxe: malformed forwarder ordinal")?;
        resolve_export_by_ordinal(ctx, pd_phys, module_idx, ordinal, depth)
    } else {
        resolve_export_by_name(ctx, pd_phys, module_idx, export, depth)
    }
}

fn wxe_system32_path(name: &str) -> String {
    alloc::format!("{}/{}", WXE_SYSTEM32, name)
}

fn normalize_dll_name(name: &str) -> String {
    let trimmed = name
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or(name);
    let mut out = String::new();
    for byte in trimmed.bytes() {
        out.push(byte.to_ascii_lowercase() as char);
    }
    if !out.ends_with(".dll") {
        out.push_str(".dll");
    }
    out
}

fn rva_to_file_offset(data: &[u8], info: &PeImageInfo, rva: u32, len: usize) -> Option<usize> {
    let rva_end = (rva as u64).checked_add(len as u64)?;
    if rva_end <= info.size_of_headers as u64 && (rva_end as usize) <= data.len() {
        return Some(rva as usize);
    }

    for idx in 0..info.section_count as usize {
        let section = parse_section(data, info.sections_offset + idx * 40).ok()?;
        let section_start = section.virtual_address as u64;
        let section_span = section.virtual_size.max(section.raw_size) as u64;
        let section_end = section_start.checked_add(section_span)?;
        if rva as u64 >= section_start && rva_end <= section_end {
            let delta = (rva as u64).checked_sub(section_start)?;
            if delta.checked_add(len as u64)? > section.raw_size as u64 {
                return None;
            }
            let raw = (section.raw_ptr as u64).checked_add(delta)? as usize;
            if raw.checked_add(len).map_or(false, |end| end <= data.len()) {
                return Some(raw);
            }
        }
    }
    None
}

fn read_cstr_rva(
    data: &[u8],
    info: &PeImageInfo,
    rva: u32,
    max_len: usize,
) -> Result<String, &'static str> {
    let off = rva_to_file_offset(data, info, rva, 1).ok_or("wxe: string RVA OOB")?;
    let cap = core::cmp::min(data.len(), off.saturating_add(max_len));
    let mut end = off;
    while end < cap && data[end] != 0 {
        end += 1;
    }
    if end == cap {
        return Err("wxe: unterminated PE string");
    }
    let text = core::str::from_utf8(&data[off..end]).map_err(|_| "wxe: PE string is not UTF-8")?;
    Ok(text.to_string())
}

fn parse_decimal_u16(value: &str) -> Option<u16> {
    let mut out = 0u32;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
        if out > u16::MAX as u32 {
            return None;
        }
    }
    Some(out as u16)
}

fn inspect_pe64(data: &[u8]) -> Result<PeImageInfo, &'static str> {
    if data.len() < 0x40 {
        return Err("wxe: file too small for DOS header");
    }
    if read_u16(data, 0)? != IMAGE_DOS_SIGNATURE {
        return Err("wxe: missing MZ DOS signature");
    }

    let pe_offset = read_u32(data, 0x3c)? as usize;
    if pe_offset
        .checked_add(24)
        .map_or(true, |end| end > data.len())
    {
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
    if opt
        .checked_add(optional_size)
        .map_or(true, |end| end > data.len())
    {
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
    let (export_rva, export_size) = read_data_dir(
        data,
        data_dirs,
        number_of_rva_and_sizes,
        IMAGE_DIRECTORY_ENTRY_EXPORT,
    )?;
    let (import_rva, import_size) = read_data_dir(
        data,
        data_dirs,
        number_of_rva_and_sizes,
        IMAGE_DIRECTORY_ENTRY_IMPORT,
    )?;
    let (tls_rva, tls_size) = read_data_dir(
        data,
        data_dirs,
        number_of_rva_and_sizes,
        IMAGE_DIRECTORY_ENTRY_TLS,
    )?;

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
        export_rva,
        export_size,
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
    let section_bytes = core::cmp::min(
        raw_size,
        section.virtual_size.max(section.raw_size) as usize,
    );
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
