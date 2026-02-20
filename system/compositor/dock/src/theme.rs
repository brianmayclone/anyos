//! Dock theme constants — colors, fonts, dimensions.

// ── Dock geometry ──

pub const DOCK_HEIGHT: u32 = 64;
pub const DOCK_TOOLTIP_H: u32 = 28;
pub const DOCK_MARGIN: u32 = 8 + DOCK_TOOLTIP_H;
pub const DOCK_TOTAL_H: u32 = DOCK_HEIGHT + DOCK_MARGIN;
pub const DOCK_ICON_SIZE: u32 = 48;
pub const DOCK_ICON_SPACING: u32 = 6;
pub const DOCK_BORDER_RADIUS: i32 = 16;
pub const DOCK_H_PADDING: u32 = 12;

// ── Colors (ARGB) ──

const THEME_ADDR: *const u32 = 0x0400_000C as *const u32;

#[inline(always)]
pub fn is_light() -> bool {
    unsafe { core::ptr::read_volatile(THEME_ADDR) != 0 }
}

#[inline(always)]
pub fn dock_bg() -> u32 {
    if is_light() { 0xB3F0F0F5 } else { 0xB3303035 }
}

pub const COLOR_WHITE: u32 = 0xFFFFFFFF;
pub const COLOR_TRANSPARENT: u32 = 0x00000000;
pub const COLOR_HIGHLIGHT: u32 = 0x19FFFFFF;

#[inline(always)]
pub fn tooltip_bg() -> u32 {
    if is_light() { 0xE6F0F0F5 } else { 0xE6303035 }
}

#[inline(always)]
pub fn tooltip_text() -> u32 {
    if is_light() { 0xFF1D1D1F } else { 0xFFFFFFFF }
}

pub const TOOLTIP_PAD: u32 = 8;

// ── Font ──

pub const FONT_ID: u16 = 0;
pub const FONT_SIZE: u16 = 12;
