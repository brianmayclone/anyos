//! Menu data structures, constants, and SHM binary format parsing.

use alloc::string::String;
use alloc::vec::Vec;

// ── Theme Constants ──────────────────────────────────────────────────────────

pub(crate) const FONT_ID: u16 = 0;
pub(crate) const FONT_ID_BOLD: u16 = 1;
pub(crate) const FONT_SIZE: u16 = 13;

/// Menubar height in physical pixels (DPI-scaled).
/// Delegates to the canonical definition in `window::menubar_height()`.
#[inline(always)]
pub(crate) fn menubar_height() -> u32 { crate::desktop::window::menubar_height() }

/// Menu bar text color — theme-aware (dark text on light, light text on dark).
#[inline(always)]
pub(crate) fn color_menubar_text() -> u32 {
    if crate::desktop::theme::is_light() { 0xFF1D1D1F } else { 0xFFE0E0E0 }
}

/// Highlight overlay for the currently-open menu title.
#[inline(always)]
pub(crate) fn color_menubar_highlight() -> u32 {
    if crate::desktop::theme::is_light() { 0x30000000 } else { 0x40FFFFFF }
}

/// Dropdown background — translucent surface behind menu items.
#[inline(always)]
pub(crate) fn color_dropdown_bg() -> u32 {
    if crate::desktop::theme::is_light() { 0xF0F0F0F5 } else { 0xF0303035 }
}

/// Dropdown 1px border.
#[inline(always)]
pub(crate) fn color_dropdown_border() -> u32 {
    if crate::desktop::theme::is_light() { 0xFFD1D1D6 } else { 0xFF505055 }
}

pub(crate) const COLOR_HOVER_BG: u32 = 0xFF0058D0;

/// Separator line inside dropdown menus.
#[inline(always)]
pub(crate) fn color_separator() -> u32 {
    if crate::desktop::theme::is_light() { 0xFFD1D1D6 } else { 0xFF505055 }
}

/// Disabled (grayed-out) menu item text — theme-aware.
#[inline(always)]
pub(crate) fn color_disabled_text() -> u32 {
    if crate::desktop::theme::is_light() { 0xFFA0A0A5 } else { 0xFF707075 }
}

pub(crate) const COLOR_CHECK: u32 = 0xFF0A84FF;

pub(crate) const ITEM_HEIGHT: u32 = 24;
pub(crate) const SEPARATOR_HEIGHT: u32 = 9;
pub(crate) const DROPDOWN_PADDING: i32 = 4;
pub(crate) const MENU_TITLE_START_X: i32 = 30; // after logo + gap

/// Width of the system menu hit area in the menubar (logo + padding).
pub(crate) const SYSTEM_MENU_WIDTH: i32 = 28;

// ── Menu Item Flags ──────────────────────────────────────────────────────────

pub const MENU_FLAG_DISABLED: u32 = 0x01;
pub const MENU_FLAG_SEPARATOR: u32 = 0x02;
pub const MENU_FLAG_CHECKED: u32 = 0x04;

// ── Well-known App Menu Item IDs ─────────────────────────────────────────────

/// "About <App>" menu item ID (auto-generated app menu).
pub const APP_MENU_ABOUT: u32 = 0xFFF0;
/// "Hide <App>" menu item ID (auto-generated app menu).
pub const APP_MENU_HIDE: u32 = 0xFFF1;
/// "Quit <App>" menu item ID (auto-generated app menu).
pub const APP_MENU_QUIT: u32 = 0xFFF2;

// ── System Menu Item IDs ────────────────────────────────────────────────────

pub const SYS_MENU_ABOUT: u32 = 0xFFE0;
pub const SYS_MENU_SETTINGS: u32 = 0xFFE1;
pub const SYS_MENU_LOGOUT: u32 = 0xFFE2;
pub const SYS_MENU_SLEEP: u32 = 0xFFE3;
pub const SYS_MENU_RESTART: u32 = 0xFFE4;
pub const SYS_MENU_SHUTDOWN: u32 = 0xFFE5;

// ── Data Structures ──────────────────────────────────────────────────────────

pub struct MenuItem {
    pub item_id: u32,
    pub flags: u32,
    pub label: String,
}

impl MenuItem {
    pub fn is_disabled(&self) -> bool {
        self.flags & MENU_FLAG_DISABLED != 0
    }
    pub fn is_separator(&self) -> bool {
        self.flags & MENU_FLAG_SEPARATOR != 0
    }
    pub fn is_checked(&self) -> bool {
        self.flags & MENU_FLAG_CHECKED != 0
    }
}

pub struct Menu {
    pub title: String,
    pub items: Vec<MenuItem>,
}

pub struct MenuBarDef {
    pub menus: Vec<Menu>,
}

pub(crate) struct MenuTitleLayout {
    pub(crate) x: i32,
    pub(crate) width: u32,
    pub(crate) menu_idx: usize,
}

pub struct OpenDropdown {
    pub menu_idx: usize,
    pub owner_window_id: u32,
    pub layer_id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hover_idx: Option<usize>,
    pub(crate) items_y: Vec<i32>,
}

pub struct StatusIcon {
    pub owner_tid: u32,
    pub icon_id: u32,
    pub pixels: [u32; 256],
}

pub enum MenuBarHit {
    None,
    SystemMenu,
    MenuTitle { menu_idx: usize },
    StatusIcon { owner_tid: u32, icon_id: u32 },
}

// ── SHM Binary Format Parsing ────────────────────────────────────────────────

const MENU_MAGIC: u32 = 0x4D454E55; // 'MENU'

impl MenuBarDef {
    /// Parse a menu bar definition from raw SHM bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }

        let magic = read_u32(data, 0);
        if magic != MENU_MAGIC {
            return None;
        }

        let num_menus = read_u32(data, 4) as usize;
        if num_menus > 16 {
            return None;
        }

        let mut offset = 8usize;
        let mut menus = Vec::with_capacity(num_menus);

        for _ in 0..num_menus {
            if offset + 4 > data.len() {
                return None;
            }
            let title_len = read_u32(data, offset) as usize;
            offset += 4;
            if offset + title_len > data.len() || title_len > 64 {
                return None;
            }
            let title = core::str::from_utf8(&data[offset..offset + title_len]).ok()?;
            offset += title_len;
            offset = align4(offset);

            if offset + 4 > data.len() {
                return None;
            }
            let num_items = read_u32(data, offset) as usize;
            offset += 4;
            if num_items > 32 {
                return None;
            }

            let mut items = Vec::with_capacity(num_items);
            for _ in 0..num_items {
                if offset + 12 > data.len() {
                    return None;
                }
                let item_id = read_u32(data, offset);
                let flags = read_u32(data, offset + 4);
                let label_len = read_u32(data, offset + 8) as usize;
                offset += 12;
                if offset + label_len > data.len() || label_len > 64 {
                    return None;
                }
                let label = core::str::from_utf8(&data[offset..offset + label_len]).ok()?;
                offset += label_len;
                offset = align4(offset);

                items.push(MenuItem {
                    item_id,
                    flags,
                    label: String::from(label),
                });
            }

            menus.push(Menu {
                title: String::from(title),
                items,
            });
        }

        Some(MenuBarDef { menus })
    }
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}
