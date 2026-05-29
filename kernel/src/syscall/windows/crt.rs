//! wxe CRT / Win32 helper services.
//!
//! These back the C runtime and the higher-level Win32 surface that cmd.exe
//! relies on: a header-tracked heap, UTF-16 <-> UTF-8 console I/O, environment
//! and directory queries, a minimal wide `printf`, and `mem*`. They return
//! their result directly in RAX (not an NTSTATUS) and are dispatched from
//! `super::dispatch`. Being a child module, this file can use the parent's
//! private helpers directly.

use super::{
    align_up, current_process_pd, dos_path_to_native, map_fd_handle, read_user_cstr_owned,
    read_user_u16, read_user_u64, read_user_utf16_z, stack_arg, write_u16_raw, write_u32_raw,
    write_u64_raw, write_user_u32, INVALID_HANDLE_VALUE, MAX_WXE_STRING, PAGE_SIZE,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, WXE_DRIVE_C_ROOT, WXE_PROCESSES,
};
use crate::syscall::SyscallRegs;
use alloc::string::String;
use alloc::vec::Vec;

const WXE_HEAP_HEADER: u64 = 16;
const WXE_HEAP_MAX_BLOCK: u64 = 0x4000_0000; // 1 GiB sanity cap

/// HeapAlloc/malloc/calloc backend: page-granular allocation with a 16-byte
/// header `[total_mapped, user_size]`. mmap returns zeroed pages, so the result
/// is also valid for calloc.
pub(super) fn wxe_heap_alloc(size: u64) -> u64 {
    let req = if size == 0 { 1 } else { size };
    if req > WXE_HEAP_MAX_BLOCK {
        return 0;
    }
    let total = align_up(req + WXE_HEAP_HEADER, PAGE_SIZE);
    let base = crate::syscall::handlers::sys_mmap_u64(total);
    if base == u64::MAX || base == 0 {
        return 0;
    }
    unsafe {
        write_u64_raw(base, 0, total);
        write_u64_raw(base, 8, req);
    }
    base + WXE_HEAP_HEADER
}

pub(super) fn wxe_heap_free(ptr: u64) -> u64 {
    if ptr == 0 {
        return 1;
    }
    let base = ptr - WXE_HEAP_HEADER;
    let total = read_user_u64(base).unwrap_or(0);
    if total >= WXE_HEAP_HEADER && total <= WXE_HEAP_MAX_BLOCK + PAGE_SIZE {
        let _ = crate::syscall::handlers::sys_munmap_u64(base, total);
    }
    1
}

pub(super) fn wxe_heap_realloc(ptr: u64, size: u64) -> u64 {
    if ptr == 0 {
        return wxe_heap_alloc(size);
    }
    let base = ptr - WXE_HEAP_HEADER;
    let old_size = read_user_u64(base + 8).unwrap_or(0);
    let new_ptr = wxe_heap_alloc(size);
    if new_ptr == 0 {
        return 0;
    }
    let copy = core::cmp::min(old_size, size) as usize;
    if copy > 0
        && crate::syscall::handlers::helpers::is_user_range_accessible(ptr, copy as u64)
        && crate::syscall::handlers::helpers::is_user_range_accessible(new_ptr, copy as u64)
    {
        unsafe {
            core::ptr::copy_nonoverlapping(ptr as *const u8, new_ptr as *mut u8, copy);
        }
    }
    wxe_heap_free(ptr);
    new_ptr
}

pub(super) fn wxe_heap_size(ptr: u64) -> u64 {
    if ptr == 0 {
        return 0;
    }
    read_user_u64(ptr - WXE_HEAP_HEADER + 8).unwrap_or(0)
}

/// WriteConsoleW: convert `count` UTF-16 units to UTF-8 and write to the fd.
pub(super) fn wxe_console_write_w(handle: u64, buffer: u64, count_chars: u64, written_ptr: u64) -> u64 {
    let Some(fd) = map_fd_handle(handle) else {
        return 0;
    };
    let count = (count_chars as usize).min(1 << 20);
    let mut units = Vec::with_capacity(count.min(4096));
    for i in 0..count {
        match read_user_u16(buffer + (i as u64) * 2) {
            Some(u) => units.push(u),
            None => break,
        }
    }
    let text = String::from_utf16_lossy(&units);
    crate::syscall::handlers::write_fd_kernel(fd, text.as_bytes());
    if written_ptr != 0 {
        let _ = write_user_u32(written_ptr, count as u32);
    }
    1
}

/// WriteConsoleA / raw byte write to the console fd.
pub(super) fn wxe_console_write_a(handle: u64, buffer: u64, count_bytes: u64, written_ptr: u64) -> u64 {
    let Some(fd) = map_fd_handle(handle) else {
        return 0;
    };
    let count = (count_bytes as usize).min(1 << 20);
    let Some(bytes) = crate::syscall::handlers::helpers::copy_user_bytes(buffer, count, count.max(1))
    else {
        return 0;
    };
    crate::syscall::handlers::write_fd_kernel(fd, &bytes);
    if written_ptr != 0 {
        let _ = write_user_u32(written_ptr, count as u32);
    }
    1
}

/// ReadConsoleW: read one line of UTF-8 input and convert it to UTF-16.
pub(super) fn wxe_console_read_w(handle: u64, buffer: u64, count_chars: u64, read_ptr: u64) -> u64 {
    let Some(fd) = map_fd_handle(handle) else {
        return 0;
    };
    let count = count_chars as usize;
    if count == 0 {
        if read_ptr != 0 {
            let _ = write_user_u32(read_ptr, 0);
        }
        return 1;
    }
    let max_bytes = count.saturating_mul(4) + 8;
    let mut line: Vec<u8> = Vec::new();
    let mut one = [0u8; 1];
    loop {
        let n = crate::syscall::handlers::read_fd_kernel(fd, &mut one, true);
        if n == 0 {
            crate::syscall::handlers::sys_sleep(5);
            continue;
        }
        // The buffer is one byte: anything other than 1 is an error / EAGAIN
        // sentinel (e.g. u32::MAX), not a real read.
        if n != 1 {
            break;
        }
        let c = one[0];
        if c == b'\r' {
            continue;
        }
        line.push(c);
        if c == b'\n' || line.len() >= max_bytes {
            break;
        }
    }
    let text = String::from_utf8_lossy(&line).into_owned();
    let mut units = 0usize;
    for u in text.encode_utf16() {
        if units >= count {
            break;
        }
        if !write_user_u16(buffer + (units as u64) * 2, u) {
            break;
        }
        units += 1;
    }
    if read_ptr != 0 {
        let _ = write_user_u32(read_ptr, units as u32);
    }
    1
}

/// GetConsoleScreenBufferInfo: report an 80x25 console so callers that size
/// their output to the window have plausible numbers.
pub(super) fn wxe_get_console_screen_info(_handle: u64, info_ptr: u64) -> u64 {
    if info_ptr == 0 || !crate::syscall::handlers::helpers::is_user_range_accessible(info_ptr, 22) {
        return 0;
    }
    unsafe {
        write_u16_raw(info_ptr, 0, 80); // dwSize.X
        write_u16_raw(info_ptr, 2, 25); // dwSize.Y
        write_u16_raw(info_ptr, 4, 0); // dwCursorPosition.X
        write_u16_raw(info_ptr, 6, 0); // dwCursorPosition.Y
        write_u16_raw(info_ptr, 8, 0x07); // wAttributes
        write_u16_raw(info_ptr, 10, 0); // srWindow.Left
        write_u16_raw(info_ptr, 12, 0); // srWindow.Top
        write_u16_raw(info_ptr, 14, 79); // srWindow.Right
        write_u16_raw(info_ptr, 16, 24); // srWindow.Bottom
        write_u16_raw(info_ptr, 18, 80); // dwMaximumWindowSize.X
        write_u16_raw(info_ptr, 20, 25); // dwMaximumWindowSize.Y
    }
    1
}

/// GetStartupInfoW: zero the STARTUPINFOW and set its cb field.
pub(super) fn wxe_get_startup_info_w(ptr: u64) -> u64 {
    const STARTUPINFOW_SIZE: u64 = 104;
    if ptr == 0
        || !crate::syscall::handlers::helpers::is_user_range_accessible(ptr, STARTUPINFOW_SIZE)
    {
        return 0;
    }
    unsafe {
        core::ptr::write_bytes(ptr as *mut u8, 0, STARTUPINFOW_SIZE as usize);
        write_u32_raw(ptr, 0, STARTUPINFOW_SIZE as u32); // cb
    }
    0
}

/// GetEnvironmentVariableW. Returns the character count (excluding the NUL) on
/// success, or 0 if the variable is not set.
pub(super) fn wxe_get_env_var_w(name_ptr: u64, buf_ptr: u64, size_chars: u64) -> u64 {
    let Some(name) = read_user_utf16_z(name_ptr, MAX_WXE_STRING) else {
        return 0;
    };
    let Some(pd) = current_process_pd() else {
        return 0;
    };
    let Some(value) = crate::task::env::get(pd, &name) else {
        return 0;
    };
    let needed = value.encode_utf16().count();
    if buf_ptr == 0 || (size_chars as usize) <= needed {
        // Caller is probing for the required size (including NUL).
        return (needed + 1) as u64;
    }
    let written = write_user_wide(buf_ptr, &value, size_chars as usize);
    written as u64
}

/// SetEnvironmentVariableW. value_ptr == 0 removes the variable.
pub(super) fn wxe_set_env_var_w(name_ptr: u64, value_ptr: u64) -> u64 {
    let Some(name) = read_user_utf16_z(name_ptr, MAX_WXE_STRING) else {
        return 0;
    };
    let Some(pd) = current_process_pd() else {
        return 0;
    };
    if value_ptr == 0 {
        crate::task::env::unset(pd, &name);
    } else {
        let value = read_user_utf16_z(value_ptr, MAX_WXE_STRING).unwrap_or_default();
        crate::task::env::set(pd, &name, &value);
    }
    1
}

/// GetModuleFileNameW: report the main image DOS path for the NULL/main module.
pub(super) fn wxe_get_module_filename_w(_module: u64, buf_ptr: u64, size_chars: u64) -> u64 {
    let image_path = {
        let Some(pd) = current_process_pd() else {
            return 0;
        };
        let table = WXE_PROCESSES.lock();
        match table.iter().find(|p| p.pd == pd) {
            Some(proc) => proc.image_path.clone(),
            None => return 0,
        }
    };
    if buf_ptr == 0 || size_chars == 0 {
        return 0;
    }
    write_user_wide(buf_ptr, &image_path, size_chars as usize) as u64
}

/// ExpandEnvironmentStringsW: expand `%VAR%` references using the process env.
pub(super) fn wxe_expand_env_w(src_ptr: u64, dst_ptr: u64, size_chars: u64) -> u64 {
    let Some(src) = read_user_utf16_z(src_ptr, MAX_WXE_STRING) else {
        return 0;
    };
    let pd = current_process_pd().unwrap_or(0);
    let mut out = String::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if name.is_empty() {
                    out.push('%');
                } else if let Some(val) = crate::task::env::get(pd, &name) {
                    out.push_str(&val);
                } else {
                    out.push('%');
                    out.push_str(&name);
                    out.push('%');
                }
                i += end + 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    let needed = out.encode_utf16().count() + 1;
    if dst_ptr == 0 || (size_chars as usize) < needed {
        return needed as u64;
    }
    let _ = write_user_wide(dst_ptr, &out, size_chars as usize);
    needed as u64
}

/// GetCurrentDirectoryW: report the current directory as a DOS path.
pub(super) fn wxe_get_current_dir_w(size_chars: u64, buf_ptr: u64) -> u64 {
    let mut cwd_buf = [0u8; 512];
    let len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
    let native = core::str::from_utf8(&cwd_buf[..len]).unwrap_or(WXE_DRIVE_C_ROOT);
    let dos = native_to_dos_path(native);
    let needed = dos.encode_utf16().count() + 1;
    if buf_ptr == 0 || (size_chars as usize) < needed {
        return needed as u64;
    }
    write_user_wide(buf_ptr, &dos, size_chars as usize) as u64
}

/// SetCurrentDirectoryW.
pub(super) fn wxe_set_current_dir_w(path_ptr: u64) -> u64 {
    let Some(path) = read_user_utf16_z(path_ptr, MAX_WXE_STRING) else {
        return 0;
    };
    let Some(native) = dos_path_to_native(&path) else {
        return 0;
    };
    if crate::fs::vfs::stat(&native).is_err() {
        return 0;
    }
    let tid = crate::task::scheduler::current_tid();
    crate::task::scheduler::set_thread_cwd(tid, &native);
    1
}

/// memcpy / memmove backend. Returns `dst` (the C return value). `overlap`
/// selects the overlap-safe copy used by memmove.
pub(super) fn wxe_memcpy(dst: u64, src: u64, n: u64, overlap: bool) -> u64 {
    let len = n as usize;
    if len > 0
        && crate::syscall::handlers::helpers::is_user_range_accessible(dst, n)
        && crate::syscall::handlers::helpers::is_user_range_accessible(src, n)
    {
        unsafe {
            if overlap {
                core::ptr::copy(src as *const u8, dst as *mut u8, len);
            } else {
                core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, len);
            }
        }
    }
    dst
}

/// memset backend. Returns `dst`.
pub(super) fn wxe_memset(dst: u64, value: u64, n: u64) -> u64 {
    let len = n as usize;
    if len > 0 && crate::syscall::handlers::helpers::is_user_range_accessible(dst, n) {
        unsafe {
            core::ptr::write_bytes(dst as *mut u8, value as u8, len);
        }
    }
    dst
}

/// memcmp backend. Returns the C comparison result sign-extended into RAX.
pub(super) fn wxe_memcmp(a: u64, b: u64, n: u64) -> u64 {
    let len = n as usize;
    if len == 0
        || !crate::syscall::handlers::helpers::is_user_range_accessible(a, n)
        || !crate::syscall::handlers::helpers::is_user_range_accessible(b, n)
    {
        return 0;
    }
    let sa = unsafe { core::slice::from_raw_parts(a as *const u8, len) };
    let sb = unsafe { core::slice::from_raw_parts(b as *const u8, len) };
    for i in 0..len {
        if sa[i] != sb[i] {
            return (sa[i] as i32 - sb[i] as i32) as i64 as u64;
        }
    }
    0
}

/// _get_osfhandle: map CRT fd 0/1/2 to the std console handles.
pub(super) fn wxe_osf_handle(fd: u64) -> u64 {
    match fd {
        0 => STD_INPUT_HANDLE,
        1 => STD_OUTPUT_HANDLE,
        2 => STD_ERROR_HANDLE,
        _ => INVALID_HANDLE_VALUE,
    }
}

/// __stdio_common_vswprintf: minimal wide printf into a user buffer.
///
/// Follows the MSVC contract that matters for buffer safety: on a size query
/// (count == 0) it returns the would-be length; when the result fits it writes
/// `result + NUL` and returns the length; when it does NOT fit it truncates,
/// writes `count-1 + NUL`, and returns -1 so the caller grows its buffer and
/// retries (instead of indexing past the end and smashing the stack canary).
pub(super) fn wxe_vswprintf(
    regs: &SyscallRegs,
    _options: u64,
    buffer: u64,
    count: u64,
    format: u64,
) -> u64 {
    let valist = stack_arg(regs, 6).unwrap_or(0);
    let Some(fmt) = read_user_utf16_z(format, 8192) else {
        return u64::MAX;
    };
    let formatted = wxe_format_wide(&fmt, valist);
    let wide: Vec<u16> = formatted.encode_utf16().collect();
    let total = wide.len();
    let cap = count as usize;

    if buffer == 0 || cap == 0 {
        // Size query: report the required length (excluding the terminator).
        return total as u64;
    }

    let truncated = total >= cap;
    let n = if truncated { cap - 1 } else { total };
    let mut bytes = Vec::with_capacity(n * 2 + 2);
    for &u in &wide[..n] {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes.extend_from_slice(&[0u8, 0u8]);
    crate::syscall::handlers::helpers::copy_to_user_bytes(buffer, &bytes, bytes.len());

    if truncated {
        (-1i64) as u64
    } else {
        total as u64
    }
}

/// Tiny printf-style formatter for the wide CRT path. Reads varargs as 8-byte
/// slots from `valist`. Handles the conversions cmd.exe actually emits.
pub(super) fn wxe_format_wide(fmt: &str, valist: u64) -> String {
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    let mut argi = 0usize;
    let mut next = |argi: &mut usize| -> u64 {
        let v = if valist != 0 {
            read_user_u64(valist + (*argi as u64) * 8).unwrap_or(0)
        } else {
            0
        };
        *argi += 1;
        v
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if chars.get(i) == Some(&'%') {
            out.push('%');
            i += 1;
            continue;
        }
        // flags
        let (mut left, mut zero, mut plus, mut space, mut alt) = (false, false, false, false, false);
        loop {
            match chars.get(i) {
                Some('-') => left = true,
                Some('0') => zero = true,
                Some('+') => plus = true,
                Some(' ') => space = true,
                Some('#') => alt = true,
                _ => break,
            }
            i += 1;
        }
        // width
        let mut width = 0usize;
        let mut have_width = false;
        if chars.get(i) == Some(&'*') {
            width = next(&mut argi) as i32 as usize;
            have_width = true;
            i += 1;
        } else {
            while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
                width = width * 10 + d as usize;
                have_width = true;
                i += 1;
            }
        }
        // precision
        let mut prec: Option<usize> = None;
        if chars.get(i) == Some(&'.') {
            i += 1;
            let mut p = 0usize;
            if chars.get(i) == Some(&'*') {
                p = next(&mut argi) as i32 as usize;
                i += 1;
            } else {
                while let Some(d) = chars.get(i).and_then(|c| c.to_digit(10)) {
                    p = p * 10 + d as usize;
                    i += 1;
                }
            }
            prec = Some(p);
        }
        // length modifiers (Win64: 'l' is 32-bit, 'll'/'I64'/'z' are 64-bit)
        let mut len64 = false;
        let mut narrow = false;
        loop {
            match chars.get(i) {
                Some('l') => {
                    if chars.get(i + 1) == Some(&'l') {
                        len64 = true;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                Some('h') => {
                    narrow = true;
                    i += 1;
                    if chars.get(i) == Some(&'h') {
                        i += 1;
                    }
                }
                Some('w') => i += 1,
                Some('I') => {
                    if chars.get(i + 1) == Some(&'6') && chars.get(i + 2) == Some(&'4') {
                        len64 = true;
                        i += 3;
                    } else if chars.get(i + 1) == Some(&'3') && chars.get(i + 2) == Some(&'2') {
                        i += 3;
                    } else {
                        len64 = true;
                        i += 1;
                    }
                }
                Some('z') | Some('t') | Some('j') => {
                    len64 = true;
                    i += 1;
                }
                Some('L') => i += 1,
                _ => break,
            }
        }
        let Some(&conv) = chars.get(i) else { break };
        i += 1;
        let mut field = String::new();
        let mut numeric = false;
        match conv {
            'd' | 'i' => {
                numeric = true;
                let raw = next(&mut argi);
                let v = if len64 {
                    raw as i64
                } else {
                    raw as u32 as i32 as i64
                };
                if v < 0 {
                    field.push('-');
                } else if plus {
                    field.push('+');
                } else if space {
                    field.push(' ');
                }
                field.push_str(&alloc::format!("{}", v.unsigned_abs()));
            }
            'u' => {
                numeric = true;
                let raw = next(&mut argi);
                let v = if len64 { raw } else { raw as u32 as u64 };
                field = alloc::format!("{}", v);
            }
            'o' => {
                numeric = true;
                let raw = next(&mut argi);
                let v = if len64 { raw } else { raw as u32 as u64 };
                field = alloc::format!("{:o}", v);
            }
            'x' => {
                numeric = true;
                let raw = next(&mut argi);
                let v = if len64 { raw } else { raw as u32 as u64 };
                field = alloc::format!("{:x}", v);
                if alt && v != 0 {
                    field = alloc::format!("0x{}", field);
                }
            }
            'X' => {
                numeric = true;
                let raw = next(&mut argi);
                let v = if len64 { raw } else { raw as u32 as u64 };
                field = alloc::format!("{:X}", v);
                if alt && v != 0 {
                    field = alloc::format!("0X{}", field);
                }
            }
            'p' => {
                let raw = next(&mut argi);
                field = alloc::format!("{:016X}", raw);
            }
            'c' => {
                let raw = next(&mut argi);
                if narrow {
                    field.push((raw as u8) as char);
                } else if let Some(ch) = char::from_u32((raw as u16) as u32) {
                    field.push(ch);
                }
            }
            's' => {
                let ptr = next(&mut argi);
                let s = if narrow {
                    read_user_cstr_owned(ptr, 65536).unwrap_or_default()
                } else {
                    read_user_utf16_z(ptr, 65536).unwrap_or_default()
                };
                field = match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                };
            }
            'S' => {
                let ptr = next(&mut argi);
                let s = read_user_cstr_owned(ptr, 65536).unwrap_or_default();
                field = match prec {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                };
            }
            other => {
                field.push('%');
                field.push(other);
            }
        }
        let field_len = field.chars().count();
        if have_width && field_len < width {
            let pad = width - field_len;
            if left {
                out.push_str(&field);
                for _ in 0..pad {
                    out.push(' ');
                }
            } else if zero && numeric && prec.is_none() {
                // Zero-pad after any sign character.
                let mut chs = field.chars();
                let sign = matches!(field.chars().next(), Some('+') | Some('-') | Some(' '));
                if sign {
                    if let Some(s) = chs.next() {
                        out.push(s);
                    }
                }
                for _ in 0..pad {
                    out.push('0');
                }
                out.push_str(chs.as_str());
            } else {
                for _ in 0..pad {
                    out.push(' ');
                }
                out.push_str(&field);
            }
        } else {
            out.push_str(&field);
        }
    }
    out
}

/// Write a NUL-terminated UTF-16 string into a user buffer of `max_units` wide
/// characters. Returns the number of units written excluding the terminator.
fn write_user_wide(ptr: u64, value: &str, max_units: usize) -> usize {
    if ptr == 0 || max_units == 0 {
        return 0;
    }
    let mut bytes = Vec::new();
    let mut units = 0usize;
    for u in value.encode_utf16() {
        if units + 1 >= max_units {
            break;
        }
        bytes.extend_from_slice(&u.to_le_bytes());
        units += 1;
    }
    bytes.extend_from_slice(&[0u8, 0u8]);
    if crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &bytes, bytes.len()) {
        units
    } else {
        0
    }
}

fn write_user_u16(ptr: u64, value: u16) -> bool {
    crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &value.to_le_bytes(), 2)
}

/// Best-effort native → DOS path mapping (inverse of `dos_path_to_native`).
fn native_to_dos_path(native: &str) -> String {
    if let Some(rest) = native.strip_prefix(WXE_DRIVE_C_ROOT) {
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
        let mut out = String::from("C:");
        for ch in native.chars() {
            out.push(if ch == '/' { '\\' } else { ch });
        }
        out
    }
}
