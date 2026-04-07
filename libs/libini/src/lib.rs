//! libini — INI/conf file parser for anyOS.
//!
//! Built as a `.so` shared library loaded via `dl_open`/`dl_sym`.
//! Provides handle-based API for parsing INI text from userspace programs.
//!
//! # Supported syntax
//!
//! ```text
//! # comment (also ; comment)
//! key = value          ← global (section "")
//!
//! [section]
//! key = value
//! bool_flag = yes
//! color = 0xFF00AA55
//! ```

#![no_std]
#![no_main]

extern crate alloc;

mod parse;
mod syscall;

use alloc::string::String;
use alloc::vec::Vec;

// ── Allocator ──────────────────────────────────────────────────────────────

libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

// ── Panic handler ──────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── Handle table ───────────────────────────────────────────────────────────

const MAX_HANDLES: usize = 16;

struct IniDoc {
    text: String,
}

static mut HANDLES: [Option<IniDoc>; MAX_HANDLES] = {
    const NONE: Option<IniDoc> = None;
    [NONE; MAX_HANDLES]
};

fn alloc_handle(doc: IniDoc) -> u32 {
    unsafe {
        for (i, slot) in HANDLES.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(doc);
                return (i + 1) as u32;
            }
        }
    }
    0
}

fn get_handle(h: u32) -> Option<&'static IniDoc> {
    if h == 0 || h as usize > MAX_HANDLES { return None; }
    unsafe { HANDLES[(h - 1) as usize].as_ref() }
}

fn free_handle(h: u32) {
    if h > 0 && (h as usize) <= MAX_HANDLES {
        unsafe { HANDLES[(h - 1) as usize] = None; }
    }
}

// ── FFI Exports ────────────────────────────────────────────────────────────

/// Parse INI text. Returns a handle (>0) or 0 on error.
#[no_mangle]
pub extern "C" fn libini_parse(text_ptr: *const u8, text_len: u32) -> u32 {
    if text_ptr.is_null() { return 0; }
    let text = unsafe { core::slice::from_raw_parts(text_ptr, text_len as usize) };
    let text = match core::str::from_utf8(text) {
        Ok(s) => String::from(s),
        Err(_) => return 0,
    };
    alloc_handle(IniDoc { text })
}

/// Close and free a parsed INI document.
#[no_mangle]
pub extern "C" fn libini_close(handle: u32) {
    free_handle(handle);
}

/// Look up a value by section + key. Writes to buf, returns byte count (0 if not found).
/// Section "" = global (pre-section) entries.
#[no_mangle]
pub extern "C" fn libini_get(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    key_ptr: *const u8, key_len: u32,
    buf_ptr: *mut u8, buf_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let key = unsafe { str_from_raw(key_ptr, key_len) };
    let ini = parse::Ini::parse(&doc.text);
    match ini.get(section, key) {
        Some(val) => {
            let copy_len = val.len().min(buf_len as usize);
            if !buf_ptr.is_null() && copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(val.as_ptr(), buf_ptr, copy_len);
                }
            }
            copy_len as u32
        }
        None => 0,
    }
}

/// Look up a value and parse as u32. Returns the value, or `default_val` if not found.
#[no_mangle]
pub extern "C" fn libini_get_u32(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    key_ptr: *const u8, key_len: u32,
    default_val: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return default_val };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let key = unsafe { str_from_raw(key_ptr, key_len) };
    let ini = parse::Ini::parse(&doc.text);
    ini.get_u32(section, key).unwrap_or(default_val)
}

/// Look up a value and parse as i32.
#[no_mangle]
pub extern "C" fn libini_get_i32(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    key_ptr: *const u8, key_len: u32,
    default_val: i32,
) -> i32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return default_val };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let key = unsafe { str_from_raw(key_ptr, key_len) };
    let ini = parse::Ini::parse(&doc.text);
    ini.get_i32(section, key).unwrap_or(default_val)
}

/// Look up a value and parse as boolean (yes/true/1/on → 1, no/false/0/off → 0).
/// Returns `default_val` if not found or not a valid boolean.
#[no_mangle]
pub extern "C" fn libini_get_bool(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    key_ptr: *const u8, key_len: u32,
    default_val: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return default_val };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let key = unsafe { str_from_raw(key_ptr, key_len) };
    let ini = parse::Ini::parse(&doc.text);
    match ini.get_bool(section, key) {
        Some(true) => 1,
        Some(false) => 0,
        None => default_val,
    }
}

/// Look up a value and parse as hex u32 (0xAARRGGBB).
#[no_mangle]
pub extern "C" fn libini_get_hex(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    key_ptr: *const u8, key_len: u32,
    default_val: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return default_val };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let key = unsafe { str_from_raw(key_ptr, key_len) };
    let ini = parse::Ini::parse(&doc.text);
    ini.get_hex(section, key).unwrap_or(default_val)
}

/// Check if a section exists. Returns 1 if found, 0 if not.
#[no_mangle]
pub extern "C" fn libini_has_section(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let ini = parse::Ini::parse(&doc.text);
    if ini.has_section(section) { 1 } else { 0 }
}

/// Count the number of sections. Returns section count.
#[no_mangle]
pub extern "C" fn libini_section_count(handle: u32) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let ini = parse::Ini::parse(&doc.text);
    ini.sections().count() as u32
}

/// Get section name by index. Writes to buf, returns byte count.
#[no_mangle]
pub extern "C" fn libini_section_name(
    handle: u32, index: u32,
    buf_ptr: *mut u8, buf_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let ini = parse::Ini::parse(&doc.text);
    match ini.sections().nth(index as usize) {
        Some(name) => {
            let copy_len = name.len().min(buf_len as usize);
            if !buf_ptr.is_null() && copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(name.as_ptr(), buf_ptr, copy_len);
                }
            }
            copy_len as u32
        }
        None => 0,
    }
}

/// Count entries in a section (or global if section is empty).
#[no_mangle]
pub extern "C" fn libini_entry_count(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let ini = parse::Ini::parse(&doc.text);
    ini.section(section).count() as u32
}

/// Get key name of entry at `index` within a section. Writes to buf, returns byte count.
#[no_mangle]
pub extern "C" fn libini_entry_key(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    index: u32,
    buf_ptr: *mut u8, buf_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let ini = parse::Ini::parse(&doc.text);
    match ini.section(section).nth(index as usize) {
        Some(entry) => write_str(entry.key, buf_ptr, buf_len),
        None => 0,
    }
}

/// Get value of entry at `index` within a section. Writes to buf, returns byte count.
#[no_mangle]
pub extern "C" fn libini_entry_value(
    handle: u32,
    section_ptr: *const u8, section_len: u32,
    index: u32,
    buf_ptr: *mut u8, buf_len: u32,
) -> u32 {
    let doc = match get_handle(handle) { Some(d) => d, None => return 0 };
    let section = unsafe { str_from_raw(section_ptr, section_len) };
    let ini = parse::Ini::parse(&doc.text);
    match ini.section(section).nth(index as usize) {
        Some(entry) => write_str(entry.value, buf_ptr, buf_len),
        None => 0,
    }
}

// ── Internal helpers ───────────────────────────────────────────────────────

unsafe fn str_from_raw<'a>(ptr: *const u8, len: u32) -> &'a str {
    if ptr.is_null() || len == 0 { return ""; }
    let bytes = core::slice::from_raw_parts(ptr, len as usize);
    core::str::from_utf8_unchecked(bytes)
}

fn write_str(s: &str, buf_ptr: *mut u8, buf_len: u32) -> u32 {
    let copy_len = s.len().min(buf_len as usize);
    if !buf_ptr.is_null() && copy_len > 0 {
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), buf_ptr, copy_len); }
    }
    copy_len as u32
}
