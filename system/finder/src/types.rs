//! Core data types and layout constants for the Finder.

use anyos_std::icons;
use anyos_std::String;
use anyos_std::Vec;
use uisys_client::{ControlIcon, UiTextField};

use crate::icon_cache::IconCache;

// ── Layout ──

pub const SIDEBAR_W: u32 = 160;
pub const TOOLBAR_H: u32 = 36;
pub const SIDEBAR_HEADER_H: u32 = 28;
pub const SIDEBAR_ITEM_H: u32 = 32;
pub const ROW_H: i32 = 28;
pub const ICON_SIZE: u32 = 16;
pub const EVENT_MOUSE_SCROLL: u32 = 7;

pub const NAV_BTN_W: i32 = 32;
pub const NAV_BTN_H: i32 = 28;
pub const NAV_BTN_Y: i32 = 4;
pub const PATH_X: i32 = 112;
pub const PATH_H: u32 = 26;
pub const PATH_Y: i32 = 5;

pub const ICON_DISPLAY_SIZE: u32 = 16;

// ── Sidebar locations ──

pub const LOCATIONS: [(&str, &str); 6] = [
    ("Root", "/"),
    ("Applications", "/Applications"),
    ("Programs", "/System/bin"),
    ("System", "/System"),
    ("Libraries", "/Libraries"),
    ("Icons", "/System/icons"),
];

// ── File entry types ──

pub const TYPE_FILE: u8 = 0;
pub const TYPE_DIR: u8 = 1;

// ── Data structures ──

pub struct FileEntry {
    pub name: [u8; 56],
    pub name_len: usize,
    pub entry_type: u8,
    pub size: u32,
}

impl FileEntry {
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("???")
    }
}

pub struct NavIcons {
    pub back: Option<ControlIcon>,
    pub forward: Option<ControlIcon>,
    pub refresh: Option<ControlIcon>,
}

impl NavIcons {
    pub fn load() -> Self {
        let sz = 16;
        NavIcons {
            back: uisys_client::load_control_icon("left", sz),
            forward: uisys_client::load_control_icon("right", sz),
            refresh: uisys_client::load_control_icon("refresh", sz),
        }
    }
}

pub struct AppState {
    pub cwd: String,
    pub entries: Vec<FileEntry>,
    pub selected: Option<usize>,
    pub scroll_offset: u32,
    pub sidebar_sel: usize,
    pub last_click_idx: Option<usize>,
    pub last_click_tick: u32,
    pub history: Vec<String>,
    pub history_pos: usize,
    pub mimetypes: icons::MimeDb,
    pub icon_cache: IconCache,
    pub nav_icons: NavIcons,
    pub path_field: UiTextField,
}
