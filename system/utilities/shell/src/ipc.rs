//! IPC protocol constants and polling for the Shell.

use alloc::string::String;

/// Apps send these to the "shell" channel to register menus.
pub const CMD_SET_MENU: u32 = 0x4001;
pub const CMD_UPDATE_MENU_ITEM: u32 = 0x4002;
pub const CMD_ADD_STATUS_ICON: u32 = 0x4003;
pub const CMD_REMOVE_STATUS_ICON: u32 = 0x4004;

/// Compositor broadcasts (Shell subscribes to compositor channel).
pub const EVT_FOCUS_CHANGED: u32 = 0x0062;
pub const EVT_WINDOW_CLOSED: u32 = 0x0061;

/// System events.
pub const EVT_PROCESS_SPAWNED: u32 = 0x0020;
pub const EVT_PROCESS_EXITED: u32 = 0x0021;
pub const EVT_RESOLUTION_CHANGED: u32 = 0x0040;

/// Unpack a process name from 3 event words (12 bytes, null-terminated).
pub fn unpack_event_name(w0: u32, w1: u32, w2: u32) -> String {
    let mut bytes = [0u8; 12];
    bytes[0..4].copy_from_slice(&w0.to_le_bytes());
    bytes[4..8].copy_from_slice(&w1.to_le_bytes());
    bytes[8..12].copy_from_slice(&w2.to_le_bytes());
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(12);
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}
