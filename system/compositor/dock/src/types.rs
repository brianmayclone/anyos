//! Core data types for the dock.

use alloc::string::String;
use alloc::vec::Vec;

pub const CMD_FOCUS_BY_TID: u32 = 0x100A;
pub const STILL_RUNNING: u32 = u32::MAX - 1;

/// A decoded icon image ready for blitting.
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// A single dock item (pinned app or transient running app).
pub struct DockItem {
    pub name: String,
    pub bin_path: String,
    pub icon: Option<Icon>,
    pub running: bool,
    /// TID of spawned process (0 = not running).
    pub tid: u32,
    /// true = from config file, false = transient (running only).
    pub pinned: bool,
}
