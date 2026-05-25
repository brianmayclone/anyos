//! wxe Windows x86_64 NT syscall dispatch.
//!
//! The actual PE loader and WXE `ntdll.dll` own the service-number profile.
//! This dispatcher implements the first console-oriented NT service slice and
//! fails closed with Windows NTSTATUS values for everything else.

use super::SyscallRegs;
use crate::sync::spinlock::Spinlock;
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

const STATUS_SUCCESS: u32 = 0x0000_0000;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
const STATUS_NO_MEMORY: u32 = 0xC000_0017;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;

const WXE_NT_TERMINATE_PROCESS: u32 = 0x0001;
const WXE_NT_TERMINATE_THREAD: u32 = 0x0002;
const WXE_NT_CLOSE: u32 = 0x0003;
const WXE_NT_READ_FILE: u32 = 0x0004;
const WXE_NT_WRITE_FILE: u32 = 0x0005;
const WXE_NT_DELAY_EXECUTION: u32 = 0x0006;
const WXE_NT_QUERY_SYSTEM_TIME: u32 = 0x0007;
const WXE_NT_QUERY_PERFORMANCE_COUNTER: u32 = 0x0008;
const WXE_NT_ALLOCATE_VIRTUAL_MEMORY: u32 = 0x0009;
const WXE_NT_FREE_VIRTUAL_MEMORY: u32 = 0x000A;
const WXE_NT_PROTECT_VIRTUAL_MEMORY: u32 = 0x000B;
const WXE_NT_QUERY_INFORMATION_PROCESS: u32 = 0x000C;
const WXE_NT_QUERY_INFORMATION_THREAD: u32 = 0x000D;
const WXE_NT_CREATE_FILE: u32 = 0x000E;
const WXE_NT_OPEN_FILE: u32 = 0x000F;
const WXE_NT_QUERY_INFORMATION_FILE: u32 = 0x0010;

const WXE_PRIV_GET_MODULE_HANDLE_A: u32 = 0x0100;
const WXE_PRIV_GET_MODULE_HANDLE_W: u32 = 0x0101;
const WXE_PRIV_GET_PROC_ADDRESS: u32 = 0x0102;
const WXE_PRIV_LOAD_LIBRARY_A: u32 = 0x0103;
const WXE_PRIV_LOAD_LIBRARY_W: u32 = 0x0104;
const WXE_PRIV_CREATE_FILE_A: u32 = 0x0105;
const WXE_PRIV_CREATE_FILE_W: u32 = 0x0106;
const WXE_PRIV_GET_FILE_ATTRIBUTES_A: u32 = 0x0107;
const WXE_PRIV_GET_FILE_ATTRIBUTES_W: u32 = 0x0108;
const WXE_PRIV_GET_FILE_SIZE_EX: u32 = 0x0109;
const WXE_PRIV_SET_FILE_POINTER_EX: u32 = 0x010A;

const NT_CURRENT_PROCESS: u64 = 0xFFFF_FFFF_FFFF_FFFF;
const NT_CURRENT_THREAD: u64 = 0xFFFF_FFFF_FFFF_FFFE;
const STD_INPUT_HANDLE: u64 = 0xFFFF_FFFF_FFFF_FFF6; // (DWORD)-10 sign-extended
const STD_OUTPUT_HANDLE: u64 = 0xFFFF_FFFF_FFFF_FFF5; // (DWORD)-11 sign-extended
const STD_ERROR_HANDLE: u64 = 0xFFFF_FFFF_FFFF_FFF4; // (DWORD)-12 sign-extended

const PAGE_SIZE: u64 = 4096;
const MAX_NT_IO_COPY: u32 = 0x1000_0000;
const WINDOWS_TICK_HZ: u64 = 10_000_000;
const UNIX_TO_WINDOWS_EPOCH_SECS: u64 = 11_644_473_600;
const WXE_PEB_BASE: u64 = 0x0000_7FFE_E000_1000;
const WXE_TEB_BASE: u64 = 0x0000_7FFE_E000_0000;
const INVALID_HANDLE_VALUE: u64 = 0xFFFF_FFFF_FFFF_FFFF;
const INVALID_FILE_ATTRIBUTES: u64 = 0xFFFF_FFFF;
const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const GENERIC_WRITE: u64 = 0x4000_0000;
const FILE_WRITE_DATA: u64 = 0x0000_0002;
const CREATE_NEW: u64 = 1;
const CREATE_ALWAYS: u64 = 2;
const OPEN_EXISTING: u64 = 3;
const OPEN_ALWAYS: u64 = 4;
const TRUNCATE_EXISTING: u64 = 5;
const FILE_BEGIN: u64 = 0;
const FILE_CURRENT: u64 = 1;
const FILE_END: u64 = 2;
const MAX_WXE_STRING: usize = 4096;
const MAX_WXE_MODULES_PER_PROCESS: usize = 96;
const MAX_WXE_EXPORTS: usize = 8192;
const MAX_WXE_FORWARD_DEPTH: u32 = 8;
const WXE_DRIVE_C_ROOT: &str = "/System/var/wxe/drive_c";

#[derive(Clone)]
pub struct WxeProcessModuleInit {
    pub name: String,
    pub base: u64,
}

pub struct WxeProcessInit {
    pub pd: u64,
    pub peb_base: u64,
    pub teb_base: u64,
    pub main_image_base: u64,
    pub command_line_a: u64,
    pub command_line_w: u64,
    pub environment_a: u64,
    pub environment_w: u64,
    pub modules: Vec<WxeProcessModuleInit>,
}

#[derive(Clone)]
struct WxeProcessModule {
    name: String,
    base: u64,
}

struct WxeProcessState {
    pd: u64,
    peb_base: u64,
    teb_base: u64,
    main_image_base: u64,
    command_line_a: u64,
    command_line_w: u64,
    environment_a: u64,
    environment_w: u64,
    modules: Vec<WxeProcessModule>,
}

static WXE_PROCESSES: Spinlock<Vec<WxeProcessState>> = Spinlock::new(Vec::new());

pub fn register_process(init: WxeProcessInit) {
    let mut modules = Vec::new();
    for module in init.modules.into_iter().take(MAX_WXE_MODULES_PER_PROCESS) {
        modules.push(WxeProcessModule {
            name: normalize_module_name(&module.name),
            base: module.base,
        });
    }

    let mut table = WXE_PROCESSES.lock();
    table.retain(|proc| proc.pd != init.pd);
    table.push(WxeProcessState {
        pd: init.pd,
        peb_base: init.peb_base,
        teb_base: init.teb_base,
        main_image_base: init.main_image_base,
        command_line_a: init.command_line_a,
        command_line_w: init.command_line_w,
        environment_a: init.environment_a,
        environment_w: init.environment_w,
        modules,
    });
}

pub fn cleanup_process(pd: u64) {
    let mut table = WXE_PROCESSES.lock();
    table.retain(|proc| proc.pd != pd);
}

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax as u32;
    let a1 = regs.r10;
    let a2 = regs.rdx;
    let a3 = regs.r8;
    let a4 = regs.r9;

    match nr {
        WXE_PRIV_GET_MODULE_HANDLE_A => return wxe_get_module_handle(a1, false),
        WXE_PRIV_GET_MODULE_HANDLE_W => return wxe_get_module_handle(a1, true),
        WXE_PRIV_GET_PROC_ADDRESS => return wxe_get_proc_address(a1, a2),
        WXE_PRIV_LOAD_LIBRARY_A => return wxe_load_library(a1, false),
        WXE_PRIV_LOAD_LIBRARY_W => return wxe_load_library(a1, true),
        WXE_PRIV_CREATE_FILE_A => return wxe_create_file(regs, a1, a2, false),
        WXE_PRIV_CREATE_FILE_W => return wxe_create_file(regs, a1, a2, true),
        WXE_PRIV_GET_FILE_ATTRIBUTES_A => return wxe_get_file_attributes(a1, false),
        WXE_PRIV_GET_FILE_ATTRIBUTES_W => return wxe_get_file_attributes(a1, true),
        WXE_PRIV_GET_FILE_SIZE_EX => return wxe_get_file_size_ex(a1, a2),
        WXE_PRIV_SET_FILE_POINTER_EX => return wxe_set_file_pointer_ex(a1, a2, a3, a4),
        _ => {}
    }

    let status = match nr {
        WXE_NT_TERMINATE_PROCESS => nt_terminate_process(a1, a2),
        WXE_NT_TERMINATE_THREAD => nt_terminate_thread(a1, a2),
        WXE_NT_CLOSE => nt_close(a1),
        WXE_NT_READ_FILE => nt_read_file(regs, a1, a2, a3, a4),
        WXE_NT_WRITE_FILE => nt_write_file(regs, a1, a2, a3, a4),
        WXE_NT_DELAY_EXECUTION => nt_delay_execution(a1, a2),
        WXE_NT_QUERY_SYSTEM_TIME => nt_query_system_time(a1),
        WXE_NT_QUERY_PERFORMANCE_COUNTER => nt_query_performance_counter(a1, a2),
        WXE_NT_ALLOCATE_VIRTUAL_MEMORY => nt_allocate_virtual_memory(regs, a1, a2, a3, a4),
        WXE_NT_FREE_VIRTUAL_MEMORY => nt_free_virtual_memory(a1, a2, a3, a4),
        WXE_NT_PROTECT_VIRTUAL_MEMORY => nt_protect_virtual_memory(regs, a1, a2, a3, a4),
        WXE_NT_QUERY_INFORMATION_PROCESS => nt_query_information_process(a1, a2, a3, a4),
        WXE_NT_QUERY_INFORMATION_THREAD => nt_query_information_thread(a1, a2, a3, a4),
        WXE_NT_CREATE_FILE => nt_create_file(regs, a1, a2, a3, a4),
        WXE_NT_OPEN_FILE => nt_open_file(regs, a1, a2, a3, a4),
        WXE_NT_QUERY_INFORMATION_FILE => nt_query_information_file(regs, a1, a2, a3, a4),
        _ => {
            crate::serial_verbose_println!(
                "wxe nt: unsupported service {}({:#x}) rip={:#x} rsp={:#x} args={:#x},{:#x},{:#x},{:#x}",
                service_name(nr),
                nr,
                regs.rip,
                regs.rsp,
                a1,
                a2,
                a3,
                a4
            );
            STATUS_NOT_IMPLEMENTED
        }
    };

    status as u64
}

fn nt_terminate_process(process_handle: u64, exit_status: u64) -> u32 {
    if process_handle == 0 || process_handle == NT_CURRENT_PROCESS {
        crate::syscall::handlers::sys_exit(exit_status as u32);
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}

fn nt_terminate_thread(thread_handle: u64, exit_status: u64) -> u32 {
    if thread_handle == 0 || thread_handle == NT_CURRENT_THREAD {
        crate::syscall::handlers::sys_exit(exit_status as u32);
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_HANDLE
    }
}

fn nt_close(handle: u64) -> u32 {
    match map_fd_handle(handle) {
        Some(fd) if fd <= 2 => STATUS_SUCCESS,
        Some(fd) => {
            if crate::syscall::handlers::sys_close(fd) == 0 {
                STATUS_SUCCESS
            } else {
                STATUS_INVALID_HANDLE
            }
        }
        None => STATUS_INVALID_HANDLE,
    }
}

fn nt_write_file(
    regs: &SyscallRegs,
    file_handle: u64,
    _event: u64,
    _apc: u64,
    _apc_ctx: u64,
) -> u32 {
    let io_status = stack_arg(regs, 5).unwrap_or(0);
    let buffer = match stack_arg(regs, 6) {
        Some(v) => v,
        None => return STATUS_ACCESS_VIOLATION,
    };
    let length = match stack_arg(regs, 7) {
        Some(v) if v <= MAX_NT_IO_COPY as u64 => v as u32,
        _ => return STATUS_INVALID_PARAMETER,
    };
    let Some(fd) = map_fd_handle(file_handle) else {
        return STATUS_INVALID_HANDLE;
    };

    let n = crate::syscall::handlers::sys_write(fd, buffer, length);
    if is_syscall_error(n) {
        write_io_status(io_status, STATUS_INVALID_HANDLE, 0);
        STATUS_INVALID_HANDLE
    } else {
        write_io_status(io_status, STATUS_SUCCESS, n as u64);
        STATUS_SUCCESS
    }
}

fn nt_read_file(
    regs: &SyscallRegs,
    file_handle: u64,
    _event: u64,
    _apc: u64,
    _apc_ctx: u64,
) -> u32 {
    let io_status = stack_arg(regs, 5).unwrap_or(0);
    let buffer = match stack_arg(regs, 6) {
        Some(v) => v,
        None => return STATUS_ACCESS_VIOLATION,
    };
    let length = match stack_arg(regs, 7) {
        Some(v) if v <= MAX_NT_IO_COPY as u64 => v as u32,
        _ => return STATUS_INVALID_PARAMETER,
    };
    let Some(fd) = map_fd_handle(file_handle) else {
        return STATUS_INVALID_HANDLE;
    };

    let n = crate::syscall::handlers::sys_read(fd, buffer, length);
    if is_syscall_error(n) {
        write_io_status(io_status, STATUS_INVALID_HANDLE, 0);
        STATUS_INVALID_HANDLE
    } else {
        write_io_status(io_status, STATUS_SUCCESS, n as u64);
        STATUS_SUCCESS
    }
}

fn nt_delay_execution(_alertable: u64, delay_interval_ptr: u64) -> u32 {
    let Some(interval) = read_user_i64(delay_interval_ptr) else {
        return STATUS_ACCESS_VIOLATION;
    };

    if interval < 0 {
        let ticks_100ns = interval.wrapping_neg() as u64;
        let mut ms = ticks_100ns / 10_000;
        if ticks_100ns != 0 && ms == 0 {
            ms = 1;
        }
        crate::syscall::handlers::sys_sleep(ms.min(u32::MAX as u64) as u32);
        STATUS_SUCCESS
    } else {
        STATUS_NOT_SUPPORTED
    }
}

fn nt_query_system_time(system_time_ptr: u64) -> u32 {
    let unix_secs = crate::time::wall_clock_unix_secs();
    let filetime = unix_secs
        .saturating_add(UNIX_TO_WINDOWS_EPOCH_SECS)
        .saturating_mul(WINDOWS_TICK_HZ);
    if write_user_u64(system_time_ptr, filetime) {
        STATUS_SUCCESS
    } else {
        STATUS_ACCESS_VIOLATION
    }
}

fn nt_query_performance_counter(counter_ptr: u64, frequency_ptr: u64) -> u32 {
    let counter = crate::syscall::handlers::sys_uptime_ms() as u64;
    if !write_user_u64(counter_ptr, counter) {
        return STATUS_ACCESS_VIOLATION;
    }
    if frequency_ptr != 0 && !write_user_u64(frequency_ptr, 1000) {
        return STATUS_ACCESS_VIOLATION;
    }
    STATUS_SUCCESS
}

fn nt_allocate_virtual_memory(
    regs: &SyscallRegs,
    process_handle: u64,
    base_address_ptr: u64,
    _zero_bits: u64,
    region_size_ptr: u64,
) -> u32 {
    if process_handle != 0 && process_handle != NT_CURRENT_PROCESS {
        return STATUS_INVALID_HANDLE;
    }
    let _allocation_type = stack_arg(regs, 5).unwrap_or(0);
    let _protect = stack_arg(regs, 6).unwrap_or(0);
    let Some(size) = read_user_u64(region_size_ptr) else {
        return STATUS_ACCESS_VIOLATION;
    };
    if size == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    let aligned = align_up(size, PAGE_SIZE);
    let addr = crate::syscall::handlers::sys_mmap_u64(aligned);
    if addr == u64::MAX {
        return STATUS_NO_MEMORY;
    }
    if !write_user_u64(base_address_ptr, addr) || !write_user_u64(region_size_ptr, aligned) {
        let _ = crate::syscall::handlers::sys_munmap_u64(addr, aligned);
        return STATUS_ACCESS_VIOLATION;
    }
    STATUS_SUCCESS
}

fn nt_free_virtual_memory(
    process_handle: u64,
    base_address_ptr: u64,
    region_size_ptr: u64,
    _free_type: u64,
) -> u32 {
    if process_handle != 0 && process_handle != NT_CURRENT_PROCESS {
        return STATUS_INVALID_HANDLE;
    }
    let Some(base) = read_user_u64(base_address_ptr) else {
        return STATUS_ACCESS_VIOLATION;
    };
    let Some(size) = read_user_u64(region_size_ptr) else {
        return STATUS_ACCESS_VIOLATION;
    };
    if base == 0 || size == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    let aligned = align_up(size, PAGE_SIZE);
    if crate::syscall::handlers::sys_munmap_u64(base, aligned) == 0 {
        let _ = write_user_u64(base_address_ptr, 0);
        let _ = write_user_u64(region_size_ptr, 0);
        STATUS_SUCCESS
    } else {
        STATUS_INVALID_PARAMETER
    }
}

fn nt_protect_virtual_memory(
    regs: &SyscallRegs,
    process_handle: u64,
    base_address_ptr: u64,
    region_size_ptr: u64,
    new_protect: u64,
) -> u32 {
    if process_handle != 0 && process_handle != NT_CURRENT_PROCESS {
        return STATUS_INVALID_HANDLE;
    }
    if read_user_u64(base_address_ptr).is_none() || read_user_u64(region_size_ptr).is_none() {
        return STATUS_ACCESS_VIOLATION;
    }
    let old_protect_ptr = stack_arg(regs, 5).unwrap_or(0);
    let _ = new_protect;
    if old_protect_ptr != 0 && !write_user_u32(old_protect_ptr, 0x04) {
        return STATUS_ACCESS_VIOLATION;
    }
    STATUS_SUCCESS
}

fn wxe_get_module_handle(name_ptr: u64, wide: bool) -> u64 {
    let Some(pd) = current_process_pd() else {
        return 0;
    };

    let wanted = if name_ptr == 0 {
        None
    } else {
        let Some(name) = read_wxe_string(name_ptr, wide) else {
            return 0;
        };
        Some(normalize_module_name(&name))
    };

    let table = WXE_PROCESSES.lock();
    let Some(proc) = table.iter().find(|proc| proc.pd == pd) else {
        return 0;
    };
    if wanted.as_ref().map_or(true, |name| name.is_empty()) {
        return proc.main_image_base;
    }
    let Some(wanted) = wanted.as_ref() else {
        return proc.main_image_base;
    };
    proc.modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case(wanted))
        .map(|module| module.base)
        .unwrap_or(0)
}

fn wxe_load_library(name_ptr: u64, wide: bool) -> u64 {
    // Dynamic PE mapping is intentionally not done from the syscall handler yet.
    // Return handles only for modules the loader already imported into this PD.
    wxe_get_module_handle(name_ptr, wide)
}

fn wxe_get_proc_address(module_base: u64, proc_name_or_ordinal: u64) -> u64 {
    if module_base == 0 || proc_name_or_ordinal == 0 {
        return 0;
    }
    if proc_name_or_ordinal <= 0xFFFF {
        resolve_user_export_by_ordinal(module_base, proc_name_or_ordinal as u16, 0).unwrap_or(0)
    } else {
        let Some(name) = read_user_cstr_owned(proc_name_or_ordinal, 512) else {
            return 0;
        };
        resolve_user_export_by_name(module_base, &name, 0).unwrap_or(0)
    }
}

fn wxe_create_file(regs: &SyscallRegs, name_ptr: u64, desired_access: u64, wide: bool) -> u64 {
    let Some(name) = read_wxe_string(name_ptr, wide) else {
        return INVALID_HANDLE_VALUE;
    };
    let creation = stack_arg(regs, 5).unwrap_or(OPEN_EXISTING);
    open_wxe_file_handle(&name, desired_access, creation).unwrap_or(INVALID_HANDLE_VALUE)
}

fn wxe_get_file_attributes(name_ptr: u64, wide: bool) -> u64 {
    let Some(name) = read_wxe_string(name_ptr, wide) else {
        return INVALID_FILE_ATTRIBUTES;
    };
    let Some(native) = dos_path_to_native(&name) else {
        return INVALID_FILE_ATTRIBUTES;
    };
    match crate::fs::vfs::stat(&native) {
        Ok(st) => {
            let mut attrs = if st.file_type == crate::fs::file::FileType::Directory {
                FILE_ATTRIBUTE_DIRECTORY
            } else {
                FILE_ATTRIBUTE_NORMAL
            };
            if st.mode & 0o222 == 0 {
                attrs |= FILE_ATTRIBUTE_READONLY;
            }
            attrs as u64
        }
        Err(_) => INVALID_FILE_ATTRIBUTES,
    }
}

fn wxe_get_file_size_ex(handle: u64, size_ptr: u64) -> u64 {
    if size_ptr == 0 {
        return 0;
    }
    let Some(fd) = map_fd_handle(handle) else {
        return 0;
    };
    let size = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                match crate::fs::vfs::fstat(global_id) {
                    Ok((_ty, size, _pos, _mtime)) => size as u64,
                    Err(_) => return 0,
                }
            }
            _ => 0,
        },
        None if fd <= 2 => 0,
        None => return 0,
    };
    if write_user_u64(size_ptr, size) {
        1
    } else {
        0
    }
}

fn wxe_set_file_pointer_ex(handle: u64, distance: u64, new_position_ptr: u64, method: u64) -> u64 {
    let Some(fd) = map_fd_handle(handle) else {
        return 0;
    };
    let whence = match method {
        FILE_BEGIN => 0,
        FILE_CURRENT => 1,
        FILE_END => 2,
        _ => return 0,
    };
    let pos = crate::syscall::handlers::sys_lseek(fd, distance as u32, whence);
    if is_syscall_error(pos) {
        return 0;
    }
    if new_position_ptr != 0 && !write_user_u64(new_position_ptr, pos as u64) {
        return 0;
    }
    1
}

fn nt_create_file(
    regs: &SyscallRegs,
    file_handle_ptr: u64,
    desired_access: u64,
    object_attributes: u64,
    io_status: u64,
) -> u32 {
    let Some(path) = read_object_attributes_path(object_attributes) else {
        write_io_status(io_status, STATUS_OBJECT_NAME_NOT_FOUND, 0);
        return STATUS_OBJECT_NAME_NOT_FOUND;
    };
    let creation = stack_arg(regs, 8).unwrap_or(OPEN_EXISTING);
    match open_wxe_file_handle(&path, desired_access, creation) {
        Some(handle) if write_user_u64(file_handle_ptr, handle) => {
            write_io_status(io_status, STATUS_SUCCESS, 1);
            STATUS_SUCCESS
        }
        _ => {
            write_io_status(io_status, STATUS_OBJECT_NAME_NOT_FOUND, 0);
            STATUS_OBJECT_NAME_NOT_FOUND
        }
    }
}

fn nt_open_file(
    _regs: &SyscallRegs,
    file_handle_ptr: u64,
    desired_access: u64,
    object_attributes: u64,
    io_status: u64,
) -> u32 {
    let Some(path) = read_object_attributes_path(object_attributes) else {
        write_io_status(io_status, STATUS_OBJECT_NAME_NOT_FOUND, 0);
        return STATUS_OBJECT_NAME_NOT_FOUND;
    };
    match open_wxe_file_handle(&path, desired_access, OPEN_EXISTING) {
        Some(handle) if write_user_u64(file_handle_ptr, handle) => {
            write_io_status(io_status, STATUS_SUCCESS, 1);
            STATUS_SUCCESS
        }
        _ => {
            write_io_status(io_status, STATUS_OBJECT_NAME_NOT_FOUND, 0);
            STATUS_OBJECT_NAME_NOT_FOUND
        }
    }
}

fn nt_query_information_file(
    regs: &SyscallRegs,
    file_handle: u64,
    io_status: u64,
    file_info: u64,
    length: u64,
) -> u32 {
    let class = stack_arg(regs, 5).unwrap_or(5);
    if class != 5 {
        write_io_status(io_status, STATUS_NOT_IMPLEMENTED, 0);
        return STATUS_NOT_IMPLEMENTED;
    }
    let Some(fd) = map_fd_handle(file_handle) else {
        write_io_status(io_status, STATUS_INVALID_HANDLE, 0);
        return STATUS_INVALID_HANDLE;
    };
    if length < 24 || !crate::syscall::handlers::helpers::is_user_range_accessible(file_info, 24) {
        write_io_status(io_status, STATUS_ACCESS_VIOLATION, 0);
        return STATUS_ACCESS_VIOLATION;
    }
    let (size, is_dir) = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                match crate::fs::vfs::fstat(global_id) {
                    Ok((file_type, size, _pos, _mtime)) => (
                        size as u64,
                        file_type == crate::fs::file::FileType::Directory,
                    ),
                    Err(_) => {
                        write_io_status(io_status, STATUS_INVALID_HANDLE, 0);
                        return STATUS_INVALID_HANDLE;
                    }
                }
            }
            _ => (0, false),
        },
        None if fd <= 2 => (0, false),
        None => {
            write_io_status(io_status, STATUS_INVALID_HANDLE, 0);
            return STATUS_INVALID_HANDLE;
        }
    };

    unsafe {
        write_u64_raw(file_info, 0, size);
        write_u64_raw(file_info, 8, size);
        write_u32_raw(file_info, 16, 1);
        write_u8_raw(file_info, 20, 0);
        write_u8_raw(file_info, 21, if is_dir { 1 } else { 0 });
        write_u16_raw(file_info, 22, 0);
    }
    write_io_status(io_status, STATUS_SUCCESS, 24);
    STATUS_SUCCESS
}

fn open_wxe_file_handle(path: &str, desired_access: u64, creation: u64) -> Option<u64> {
    if is_console_input_name(path) {
        return Some(STD_INPUT_HANDLE);
    }
    if is_console_output_name(path) {
        return Some(STD_OUTPUT_HANDLE);
    }
    let native = dos_path_to_native(path)?;
    let wants_write = (desired_access & (GENERIC_WRITE | FILE_WRITE_DATA)) != 0;
    let create = matches!(creation, CREATE_NEW | CREATE_ALWAYS | OPEN_ALWAYS);
    let truncate = matches!(creation, CREATE_ALWAYS | TRUNCATE_EXISTING);
    let flags = crate::fs::file::FileFlags {
        read: true,
        write: wants_write || create || truncate,
        append: false,
        create,
        truncate,
        sync: false,
    };

    let global_id = crate::fs::vfs::open(&native, flags).ok()?;
    match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::File { global_id })
    {
        Some(fd) => Some(fd as u64),
        None => {
            crate::fs::vfs::decref(global_id);
            None
        }
    }
}

fn read_object_attributes_path(object_attributes: u64) -> Option<String> {
    if object_attributes == 0 {
        return None;
    }
    let unicode_string_ptr = read_user_u64(object_attributes + 16)?;
    read_unicode_string(unicode_string_ptr)
}

fn read_unicode_string(ptr: u64) -> Option<String> {
    if ptr == 0 {
        return None;
    }
    let len_bytes = read_user_u16(ptr)? as usize;
    let buffer = read_user_u64(ptr + 8)?;
    if len_bytes == 0 || len_bytes > MAX_WXE_STRING * 2 || (len_bytes & 1) != 0 {
        return None;
    }
    let mut out = String::new();
    for idx in 0..(len_bytes / 2) {
        let unit = read_user_u16(buffer + (idx * 2) as u64)?;
        if let Some(ch) = char::from_u32(unit as u32) {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    Some(out)
}

fn service_name(nr: u32) -> &'static str {
    match nr {
        WXE_NT_TERMINATE_PROCESS => "NtTerminateProcess",
        WXE_NT_TERMINATE_THREAD => "NtTerminateThread",
        WXE_NT_CLOSE => "NtClose",
        WXE_NT_READ_FILE => "NtReadFile",
        WXE_NT_WRITE_FILE => "NtWriteFile",
        WXE_NT_DELAY_EXECUTION => "NtDelayExecution",
        WXE_NT_QUERY_SYSTEM_TIME => "NtQuerySystemTime",
        WXE_NT_QUERY_PERFORMANCE_COUNTER => "NtQueryPerformanceCounter",
        WXE_NT_ALLOCATE_VIRTUAL_MEMORY => "NtAllocateVirtualMemory",
        WXE_NT_FREE_VIRTUAL_MEMORY => "NtFreeVirtualMemory",
        WXE_NT_PROTECT_VIRTUAL_MEMORY => "NtProtectVirtualMemory",
        WXE_NT_QUERY_INFORMATION_PROCESS => "NtQueryInformationProcess",
        WXE_NT_QUERY_INFORMATION_THREAD => "NtQueryInformationThread",
        WXE_NT_CREATE_FILE => "NtCreateFile",
        WXE_NT_OPEN_FILE => "NtOpenFile",
        WXE_NT_QUERY_INFORMATION_FILE => "NtQueryInformationFile",
        WXE_PRIV_GET_MODULE_HANDLE_A => "WxeGetModuleHandleA",
        WXE_PRIV_GET_MODULE_HANDLE_W => "WxeGetModuleHandleW",
        WXE_PRIV_GET_PROC_ADDRESS => "WxeGetProcAddress",
        WXE_PRIV_LOAD_LIBRARY_A => "WxeLoadLibraryA",
        WXE_PRIV_LOAD_LIBRARY_W => "WxeLoadLibraryW",
        WXE_PRIV_CREATE_FILE_A => "WxeCreateFileA",
        WXE_PRIV_CREATE_FILE_W => "WxeCreateFileW",
        WXE_PRIV_GET_FILE_ATTRIBUTES_A => "WxeGetFileAttributesA",
        WXE_PRIV_GET_FILE_ATTRIBUTES_W => "WxeGetFileAttributesW",
        WXE_PRIV_GET_FILE_SIZE_EX => "WxeGetFileSizeEx",
        WXE_PRIV_SET_FILE_POINTER_EX => "WxeSetFilePointerEx",
        _ => "unknown",
    }
}

fn nt_query_information_process(
    process_handle: u64,
    class: u64,
    info_ptr: u64,
    info_len: u64,
) -> u32 {
    if process_handle != 0 && process_handle != NT_CURRENT_PROCESS {
        return STATUS_INVALID_HANDLE;
    }
    match class {
        0 => write_process_basic_info(info_ptr, info_len),
        _ => STATUS_NOT_IMPLEMENTED,
    }
}

fn nt_query_information_thread(
    thread_handle: u64,
    class: u64,
    info_ptr: u64,
    info_len: u64,
) -> u32 {
    if thread_handle != 0 && thread_handle != NT_CURRENT_THREAD {
        return STATUS_INVALID_HANDLE;
    }
    match class {
        0 => write_thread_basic_info(info_ptr, info_len),
        _ => STATUS_NOT_IMPLEMENTED,
    }
}

fn write_process_basic_info(ptr: u64, len: u64) -> u32 {
    if len < 48 || !crate::syscall::handlers::helpers::is_user_range_accessible(ptr, 48) {
        return STATUS_ACCESS_VIOLATION;
    }
    let tid = crate::task::scheduler::current_tid() as u64;
    let parent = crate::task::scheduler::current_parent_tid() as u64;
    unsafe {
        write_u32_raw(ptr, 0, STATUS_SUCCESS);
        write_u32_raw(ptr, 4, 0);
        write_u64_raw(ptr, 8, current_process_peb_base());
        write_u64_raw(ptr, 16, 1); // Affinity mask
        write_i32_raw(ptr, 24, 8); // Base priority
        write_u32_raw(ptr, 28, 0);
        write_u64_raw(ptr, 32, tid);
        write_u64_raw(ptr, 40, parent);
    }
    STATUS_SUCCESS
}

fn write_thread_basic_info(ptr: u64, len: u64) -> u32 {
    if len < 48 || !crate::syscall::handlers::helpers::is_user_range_accessible(ptr, 48) {
        return STATUS_ACCESS_VIOLATION;
    }
    let tid = crate::task::scheduler::current_tid() as u64;
    unsafe {
        write_u32_raw(ptr, 0, STATUS_SUCCESS);
        write_u32_raw(ptr, 4, 0);
        write_u64_raw(ptr, 8, current_process_teb_base());
        write_u64_raw(ptr, 16, tid);
        write_u64_raw(ptr, 24, 0);
        write_i32_raw(ptr, 32, 8);
        write_i32_raw(ptr, 36, 8);
        write_u32_raw(ptr, 40, 0);
        write_u32_raw(ptr, 44, 0);
    }
    STATUS_SUCCESS
}

fn current_process_pd() -> Option<u64> {
    crate::task::scheduler::current_thread_page_directory().map(|pd| pd.as_u64())
}

fn current_process_peb_base() -> u64 {
    let Some(pd) = current_process_pd() else {
        return WXE_PEB_BASE;
    };
    let table = WXE_PROCESSES.lock();
    table
        .iter()
        .find(|proc| proc.pd == pd)
        .map(|proc| proc.peb_base)
        .unwrap_or(WXE_PEB_BASE)
}

fn current_process_teb_base() -> u64 {
    let Some(pd) = current_process_pd() else {
        return WXE_TEB_BASE;
    };
    let table = WXE_PROCESSES.lock();
    table
        .iter()
        .find(|proc| proc.pd == pd)
        .map(|proc| proc.teb_base)
        .unwrap_or(WXE_TEB_BASE)
}

fn read_wxe_string(ptr: u64, wide: bool) -> Option<String> {
    if wide {
        read_user_utf16_z(ptr, MAX_WXE_STRING)
    } else {
        read_user_cstr_owned(ptr, MAX_WXE_STRING)
    }
}

fn read_user_cstr_owned(ptr: u64, max_len: usize) -> Option<String> {
    if ptr == 0 || max_len == 0 {
        return None;
    }
    let mut out = Vec::new();
    for idx in 0..max_len {
        let byte = crate::syscall::handlers::helpers::copy_user_bytes(ptr + idx as u64, 1, 1)?[0];
        if byte == 0 {
            return core::str::from_utf8(&out).ok().map(ToString::to_string);
        }
        out.push(byte);
    }
    None
}

fn read_user_utf16_z(ptr: u64, max_units: usize) -> Option<String> {
    if ptr == 0 || max_units == 0 {
        return None;
    }
    let mut out = String::new();
    for idx in 0..max_units {
        let unit = read_user_u16(ptr + (idx * 2) as u64)?;
        if unit == 0 {
            return Some(out);
        }
        if let Some(ch) = char::from_u32(unit as u32) {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    None
}

fn normalize_module_name(name: &str) -> String {
    let trimmed = name
        .rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or(name)
        .trim();
    let mut out = String::new();
    for byte in trimmed.bytes() {
        out.push(byte.to_ascii_lowercase() as char);
    }
    if !out.is_empty() && !out.ends_with(".dll") && !out.ends_with(".exe") {
        out.push_str(".dll");
    }
    out
}

fn dos_path_to_native(path: &str) -> Option<String> {
    let mut raw = path.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with('/') {
        return Some(crate::fs::path::normalize(raw));
    }
    if let Some(rest) = raw.strip_prefix("\\\\?\\") {
        raw = rest;
    }
    if let Some(rest) = raw.strip_prefix("\\??\\") {
        raw = rest;
    }
    if let Some(rest) = raw.strip_prefix("\\\\.\\") {
        raw = rest;
    }

    let mut native = String::new();
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        if bytes[0].to_ascii_uppercase() != b'C' {
            return None;
        }
        native.push_str(WXE_DRIVE_C_ROOT);
        raw = &raw[2..];
        if raw.starts_with('\\') || raw.starts_with('/') {
            raw = &raw[1..];
        }
    } else if raw.starts_with('\\') {
        native.push_str(WXE_DRIVE_C_ROOT);
        raw = &raw[1..];
    } else {
        let mut cwd_buf = [0u8; 512];
        let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
        let cwd = core::str::from_utf8(&cwd_buf[..cwd_len]).unwrap_or(WXE_DRIVE_C_ROOT);
        native.push_str(cwd);
        if !native.ends_with('/') {
            native.push('/');
        }
    }

    if !raw.is_empty() {
        if !native.ends_with('/') {
            native.push('/');
        }
        for ch in raw.chars() {
            native.push(if ch == '\\' { '/' } else { ch });
        }
    }
    Some(crate::fs::path::normalize(&native))
}

fn is_console_input_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("CONIN$") || name.eq_ignore_ascii_case("CON")
}

fn is_console_output_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("CONOUT$")
        || name.eq_ignore_ascii_case("CON")
        || name.eq_ignore_ascii_case("NUL")
}

struct UserExportDirectory {
    export_rva: u32,
    export_size: u32,
    ordinal_base: u32,
    number_of_functions: u32,
    number_of_names: u32,
    address_of_functions: u32,
    address_of_names: u32,
    address_of_ordinals: u32,
}

fn read_user_export_directory(module_base: u64) -> Option<UserExportDirectory> {
    if read_user_u16(module_base)? != 0x5A4D {
        return None;
    }
    let pe_off = read_user_u32(module_base + 0x3C)? as u64;
    let pe = module_base.checked_add(pe_off)?;
    if read_user_u32(pe)? != 0x0000_4550 {
        return None;
    }
    let coff = pe + 4;
    if read_user_u16(coff)? != 0x8664 {
        return None;
    }
    let opt = coff + 20;
    if read_user_u16(opt)? != 0x20B {
        return None;
    }
    let number_of_rva_and_sizes = read_user_u32(opt + 108)?;
    if number_of_rva_and_sizes == 0 {
        return None;
    }
    let export_rva = read_user_u32(opt + 112)?;
    let export_size = read_user_u32(opt + 116)?;
    if export_rva == 0 || export_size == 0 {
        return None;
    }
    let dir = module_base.checked_add(export_rva as u64)?;
    Some(UserExportDirectory {
        export_rva,
        export_size,
        ordinal_base: read_user_u32(dir + 16)?,
        number_of_functions: read_user_u32(dir + 20)?,
        number_of_names: read_user_u32(dir + 24)?,
        address_of_functions: read_user_u32(dir + 28)?,
        address_of_names: read_user_u32(dir + 32)?,
        address_of_ordinals: read_user_u32(dir + 36)?,
    })
}

fn resolve_user_export_by_name(module_base: u64, name: &str, depth: u32) -> Option<u64> {
    if depth > MAX_WXE_FORWARD_DEPTH {
        return None;
    }
    let export = read_user_export_directory(module_base)?;
    if export.number_of_names as usize > MAX_WXE_EXPORTS {
        return None;
    }
    for idx in 0..export.number_of_names {
        let name_rva =
            read_user_u32(module_base + export.address_of_names as u64 + idx as u64 * 4)?;
        let export_name = read_user_cstr_owned(module_base + name_rva as u64, 512)?;
        if export_name == name {
            let ordinal_index =
                read_user_u16(module_base + export.address_of_ordinals as u64 + idx as u64 * 2)?
                    as u32;
            return resolve_user_export_by_ordinal_index(
                module_base,
                &export,
                ordinal_index,
                depth,
            );
        }
    }
    None
}

fn resolve_user_export_by_ordinal(module_base: u64, ordinal: u16, depth: u32) -> Option<u64> {
    let export = read_user_export_directory(module_base)?;
    let ordinal = ordinal as u32;
    if ordinal < export.ordinal_base {
        return None;
    }
    resolve_user_export_by_ordinal_index(module_base, &export, ordinal - export.ordinal_base, depth)
}

fn resolve_user_export_by_ordinal_index(
    module_base: u64,
    export: &UserExportDirectory,
    ordinal_index: u32,
    depth: u32,
) -> Option<u64> {
    if depth > MAX_WXE_FORWARD_DEPTH
        || ordinal_index >= export.number_of_functions
        || export.number_of_functions as usize > MAX_WXE_EXPORTS
    {
        return None;
    }
    let function_rva =
        read_user_u32(module_base + export.address_of_functions as u64 + ordinal_index as u64 * 4)?;
    let export_end = export.export_rva.checked_add(export.export_size)?;
    if function_rva >= export.export_rva && function_rva < export_end {
        let forwarder = read_user_cstr_owned(module_base + function_rva as u64, 512)?;
        resolve_user_forwarder(&forwarder, depth + 1)
    } else {
        module_base.checked_add(function_rva as u64)
    }
}

fn resolve_user_forwarder(target: &str, depth: u32) -> Option<u64> {
    let dot = target.find('.')?;
    let mut module_name = String::from(&target[..dot]);
    if !module_name.ends_with(".dll") && !module_name.ends_with(".DLL") {
        module_name.push_str(".dll");
    }
    let export_name = &target[dot + 1..];
    let module_base = lookup_current_module_base(&module_name)?;
    if let Some(ordinal) = export_name.strip_prefix('#') {
        let ordinal = parse_decimal_u16(ordinal)?;
        resolve_user_export_by_ordinal(module_base, ordinal, depth)
    } else {
        resolve_user_export_by_name(module_base, export_name, depth)
    }
}

fn lookup_current_module_base(name: &str) -> Option<u64> {
    let pd = current_process_pd()?;
    let wanted = normalize_module_name(name);
    let table = WXE_PROCESSES.lock();
    let proc = table.iter().find(|proc| proc.pd == pd)?;
    proc.modules
        .iter()
        .find(|module| module.name.eq_ignore_ascii_case(&wanted))
        .map(|module| module.base)
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

fn map_fd_handle(handle: u64) -> Option<u32> {
    match handle {
        STD_INPUT_HANDLE => Some(0),
        STD_OUTPUT_HANDLE => Some(1),
        STD_ERROR_HANDLE => Some(2),
        h if h <= u32::MAX as u64 => Some(h as u32),
        _ => None,
    }
}

fn is_syscall_error(value: u32) -> bool {
    value == u32::MAX || (value as i32) < 0
}

fn write_io_status(ptr: u64, status: u32, information: u64) {
    if ptr == 0 || !crate::syscall::handlers::helpers::is_user_range_accessible(ptr, 16) {
        return;
    }
    unsafe {
        write_u32_raw(ptr, 0, status);
        write_u32_raw(ptr, 4, 0);
        write_u64_raw(ptr, 8, information);
    }
}

fn stack_arg(regs: &SyscallRegs, one_based_arg: u32) -> Option<u64> {
    if one_based_arg <= 4 {
        return None;
    }
    let offset = 8 + 32 + (one_based_arg as u64 - 5) * 8;
    read_user_u64(regs.rsp.checked_add(offset)?)
}

fn read_user_u16(ptr: u64) -> Option<u16> {
    let bytes = crate::syscall::handlers::helpers::copy_user_bytes(ptr, 2, 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_user_u32(ptr: u64) -> Option<u32> {
    let bytes = crate::syscall::handlers::helpers::copy_user_bytes(ptr, 4, 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_user_u64(ptr: u64) -> Option<u64> {
    let bytes = crate::syscall::handlers::helpers::copy_user_bytes(ptr, 8, 8)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_user_i64(ptr: u64) -> Option<i64> {
    read_user_u64(ptr).map(|v| v as i64)
}

fn write_user_u32(ptr: u64, value: u32) -> bool {
    crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &value.to_le_bytes(), 4)
}

fn write_user_u64(ptr: u64, value: u64) -> bool {
    crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &value.to_le_bytes(), 8)
}

unsafe fn write_u32_raw(base: u64, off: u64, value: u32) {
    core::ptr::write_unaligned((base + off) as *mut u32, value);
}

unsafe fn write_u8_raw(base: u64, off: u64, value: u8) {
    core::ptr::write_unaligned((base + off) as *mut u8, value);
}

unsafe fn write_u16_raw(base: u64, off: u64, value: u16) {
    core::ptr::write_unaligned((base + off) as *mut u16, value);
}

unsafe fn write_i32_raw(base: u64, off: u64, value: i32) {
    core::ptr::write_unaligned((base + off) as *mut i32, value);
}

unsafe fn write_u64_raw(base: u64, off: u64, value: u64) {
    core::ptr::write_unaligned((base + off) as *mut u64, value);
}

fn align_up(value: u64, align: u64) -> u64 {
    value
        .checked_add(align - 1)
        .map(|v| v & !(align - 1))
        .unwrap_or(u64::MAX)
}
