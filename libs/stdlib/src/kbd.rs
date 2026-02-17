//! Keyboard layout management.

use crate::raw::*;

/// Layout info as returned by the kernel (16 bytes, matches kernel repr(C)).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayoutInfo {
    pub id: u32,
    pub code: [u8; 8],  // e.g. b"de-DE\0\0\0"
    pub label: [u8; 4], // e.g. b"DE\0\0"
}

/// Get the currently active keyboard layout ID.
pub fn get_layout() -> u32 {
    syscall0(SYS_KBD_GET_LAYOUT)
}

/// Set the active keyboard layout by ID. Returns 0 on success.
pub fn set_layout(id: u32) -> u32 {
    syscall1(SYS_KBD_SET_LAYOUT, id as u64)
}

/// List available keyboard layouts. Writes up to `buf.len()` entries.
/// Returns the number of entries written.
pub fn list_layouts(buf: &mut [LayoutInfo]) -> u32 {
    syscall2(SYS_KBD_LIST_LAYOUTS, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Convert a layout label ([u8; 4]) to a &str (null-terminated).
pub fn label_str(label: &[u8; 4]) -> &str {
    let len = label.iter().position(|&b| b == 0).unwrap_or(4);
    core::str::from_utf8(&label[..len]).unwrap_or("??")
}

/// Convert a layout code ([u8; 8]) to a &str (null-terminated).
pub fn code_str(code: &[u8; 8]) -> &str {
    let len = code.iter().position(|&b| b == 0).unwrap_or(8);
    core::str::from_utf8(&code[..len]).unwrap_or("??")
}
