//! IPC protocol definitions and request dispatch.
//!
//! # Event channel: "fontd"
//!
//! ## CMD_LOAD_BY_NAME (0x6001)
//! Load a font by filename from `/System/fonts/`.
//!   evt[0] = 0x6001
//!   evt[1] = requester sub_id
//!   evt[2] = shm_id containing filename (null-terminated, e.g. "sfpro.ttf\0")
//!   evt[3] = 0
//!   evt[4] = 0
//!
//! Response: EVT_FONT_READY
//!   evt[0] = 0x6002
//!   evt[1] = shm_id with font data (0 = failed)
//!   evt[2] = data size in bytes
//!   evt[3] = 0
//!   evt[4] = 0
//!
//! ## CMD_LOAD_BY_PATH (0x6003)
//! Load a font by absolute path (for user-installed fonts).
//!   evt[0] = 0x6003
//!   evt[1] = requester sub_id
//!   evt[2] = shm_id containing full path (null-terminated)
//!   evt[3] = 0
//!   evt[4] = 0
//!
//! Response: EVT_FONT_READY (same as above)
//!
//! ## CMD_LIST_FONTS (0x6005)
//! List all available system fonts.
//!   evt[0] = 0x6005
//!   evt[1] = requester sub_id
//!
//! Response: EVT_FONT_LIST
//!   evt[0] = 0x6006
//!   evt[1] = shm_id with font list (newline-separated filenames)
//!   evt[2] = data size in bytes
//!   evt[3] = font count

use anyos_std::{format, ipc, String};
use crate::cache;
use crate::loader;

pub const CMD_LOAD_BY_NAME: u32 = 0x6001;
pub const EVT_FONT_READY: u32 = 0x6002;
pub const CMD_LOAD_BY_PATH: u32 = 0x6003;
pub const CMD_LIST_FONTS: u32 = 0x6005;
pub const EVT_FONT_LIST: u32 = 0x6006;

/// Base directory for system fonts.
pub const FONT_DIR: &str = "/System/fonts";

/// Handle an incoming request and produce a response event.
pub fn handle_request(evt: &[u32; 5]) -> [u32; 5] {
    match evt[0] {
        CMD_LOAD_BY_NAME => {
            let name_shm = evt[2];
            let name = read_string_from_shm(name_shm);
            if name.is_empty() {
                return [EVT_FONT_READY, 0, 0, 0, 0];
            }
            let path = format!("{}/{}", FONT_DIR, name);
            let (shm_id, size) = load_font(&path);
            [EVT_FONT_READY, shm_id, size, 0, 0]
        }
        CMD_LOAD_BY_PATH => {
            let path_shm = evt[2];
            let path = read_string_from_shm(path_shm);
            if path.is_empty() {
                return [EVT_FONT_READY, 0, 0, 0, 0];
            }
            let (shm_id, size) = load_font(&path);
            [EVT_FONT_READY, shm_id, size, 0, 0]
        }
        CMD_LIST_FONTS => {
            let (shm_id, size, count) = list_fonts();
            [EVT_FONT_LIST, shm_id, size, count, 0]
        }
        _ => [0, 0, 0, 0, 0],
    }
}

/// Load a font by full path — checks cache first, then loads from disk.
fn load_font(path: &str) -> (u32, u32) {
    // Check cache
    if let Some((shm_id, size)) = cache::lookup(path) {
        return (shm_id, size);
    }

    anyos_std::println!("[fontd] loading: {}", path);

    match loader::load_to_shm(path) {
        Some((shm_id, size)) => {
            anyos_std::println!("[fontd]   -> shm={} ({} KB)", shm_id, size / 1024);
            cache::insert(path, shm_id, size);
            (shm_id, size)
        }
        None => {
            anyos_std::println!("[fontd]   -> FAILED: {}", path);
            (0, 0)
        }
    }
}

/// List all .ttf/.otf files in the system font directory.
fn list_fonts() -> (u32, u32, u32) {
    let mut buf = [0u8; 256 * 64];
    let count = anyos_std::fs::readdir(FONT_DIR, &mut buf);
    if count == u32::MAX { return (0, 0, 0); }

    let mut list = String::new();
    let mut font_count = 0u32;

    for i in 0..count as usize {
        let off = i * 64;
        let entry_type = buf[off];
        let name_len = buf[off + 1] as usize;
        if entry_type != 0 || name_len == 0 || name_len > 56 { continue; } // skip dirs
        let name = match core::str::from_utf8(&buf[off + 8..off + 8 + name_len]) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if name.ends_with(".ttf") || name.ends_with(".otf") || name.ends_with(".woff2") {
            if !list.is_empty() { list.push('\n'); }
            list.push_str(name);
            font_count += 1;
        }
    }

    if list.is_empty() { return (0, 0, 0); }

    // Put the list into SHM
    let size = list.len() as u32;
    let shm_id = ipc::shm_create(size);
    if shm_id == 0 { return (0, 0, 0); }
    let addr = ipc::shm_map(shm_id);
    if addr == 0 { ipc::shm_destroy(shm_id); return (0, 0, 0); }

    let dst = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, size as usize) };
    dst.copy_from_slice(list.as_bytes());
    // Keep mapped so client can read it

    (shm_id, size, font_count)
}

/// Read a null-terminated string from a SHM region. Unmaps and destroys the SHM after reading.
fn read_string_from_shm(shm_id: u32) -> String {
    if shm_id == 0 { return String::new(); }
    let addr = ipc::shm_map(shm_id);
    if addr == 0 { return String::new(); }

    let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, 256) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(256);
    let s = core::str::from_utf8(&bytes[..len]).unwrap_or("").trim();
    let result = String::from(s);

    ipc::shm_unmap(shm_id);
    result
}
