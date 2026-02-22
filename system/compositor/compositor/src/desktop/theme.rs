//! Theme — color palette, theme switching, and button animation helpers.

/// Address of the `theme` field in the uisys.dlib export struct.
/// RO pages are shared across all processes — reads via volatile, writes via syscall.
const UISYS_BASE: u64 = 0x0400_0000;
const UISYS_THEME_OFFSET: u32 = 0x0C; // offset of `theme` field in export struct
const UISYS_THEME_ADDR: *const u32 = (UISYS_BASE as usize + UISYS_THEME_OFFSET as usize) as *const u32;

/// Cached theme value — refreshed once per frame via `refresh_theme_cache()`.
static mut CACHED_IS_LIGHT: bool = false;

/// Refresh the cached theme value from the shared DLIB page.
/// Called once per frame in the render loop — avoids 28K+ volatile reads per menubar render.
pub(crate) fn refresh_theme_cache() {
    unsafe { CACHED_IS_LIGHT = core::ptr::read_volatile(UISYS_THEME_ADDR) != 0; }
}

/// Read the cached theme (0 = dark, 1 = light).
#[inline(always)]
pub(crate) fn is_light() -> bool {
    unsafe { CACHED_IS_LIGHT }
}

/// Set the theme via kernel-mediated write to the shared RO DLIB page.
pub fn set_theme(value: u32) {
    anyos_std::dll::set_dll_u32(UISYS_BASE, UISYS_THEME_OFFSET, value);
}

// ── Desktop Background ─────────────────────────────────────────────────────

pub(crate) const COLOR_DESKTOP_BG: u32 = 0xFF1E1E1E;

// ── Menubar Colors ─────────────────────────────────────────────────────────

#[inline(always)]
pub(crate) fn color_menubar_bg() -> u32 {
    if is_light() { 0xB3F0F0F5 } else { 0xB3303035 }
}

#[inline(always)]
pub(crate) fn color_menubar_border() -> u32 {
    if is_light() { 0xFFD1D1D6 } else { 0xFF404045 }
}

#[inline(always)]
pub(crate) fn color_menubar_text() -> u32 {
    if is_light() { 0xFF1D1D1F } else { 0xFFE0E0E0 }
}

// ── Title Bar Colors ───────────────────────────────────────────────────────

#[inline(always)]
pub(crate) fn color_titlebar_focused() -> u32 {
    if is_light() { 0xFFE8E8E8 } else { 0xFF3C3C3C }
}

#[inline(always)]
pub(crate) fn color_titlebar_unfocused() -> u32 {
    if is_light() { 0xFFF0F0F0 } else { 0xFF2A2A2A }
}

#[inline(always)]
pub(crate) fn color_titlebar_text() -> u32 {
    if is_light() { 0xFF1D1D1F } else { 0xFFE0E0E0 }
}

// ── Window Colors ──────────────────────────────────────────────────────────

#[inline(always)]
pub(crate) fn color_window_bg() -> u32 {
    if is_light() { 0xFFF5F5F7 } else { 0xFF1E1E1E }
}

#[inline(always)]
pub(crate) fn color_window_border() -> u32 {
    if is_light() { 0xFFD1D1D6 } else { 0xFF4A4A4E }
}

#[inline(always)]
pub(crate) fn color_btn_unfocused() -> u32 {
    if is_light() { 0xFFC7C7CC } else { 0xFF5A5A5E }
}

// ── Traffic Light Buttons ──────────────────────────────────────────────────

pub(crate) const COLOR_CLOSE_BTN: u32 = 0xFFFF5F56;
pub(crate) const COLOR_MIN_BTN: u32 = 0xFFFEBD2E;
pub(crate) const COLOR_MAX_BTN: u32 = 0xFF27C93F;

pub(crate) const COLOR_CLOSE_HOVER: u32 = 0xFFFF7B73;
pub(crate) const COLOR_MIN_HOVER: u32 = 0xFFFECE56;
pub(crate) const COLOR_MAX_HOVER: u32 = 0xFF3DD654;
pub(crate) const COLOR_CLOSE_PRESS: u32 = 0xFFCC4C45;
pub(crate) const COLOR_MIN_PRESS: u32 = 0xFFCB9724;
pub(crate) const COLOR_MAX_PRESS: u32 = 0xFF1FA030;

// ── System Font ────────────────────────────────────────────────────────────

pub(crate) const FONT_ID: u16 = 0;
pub(crate) const FONT_ID_BOLD: u16 = 1;
pub(crate) const FONT_SIZE: u16 = 13;

// ── Title Bar Layout ───────────────────────────────────────────────────────

pub(crate) const TITLE_BTN_SIZE: u32 = 12;
pub(crate) const TITLE_BTN_Y: u32 = 8;
pub(crate) const TITLE_BTN_SPACING: u32 = 20;

// ── Button Animation Helpers ───────────────────────────────────────────────

/// Compute a unique animation ID for a window button.
/// Encodes window_id and button index (0=close, 1=min, 2=max) into one u32.
pub(crate) fn button_anim_id(window_id: u32, btn: u8) -> u32 {
    window_id.wrapping_mul(4).wrapping_add(btn as u32)
}

/// Look up hover target colour for a button index.
pub(crate) fn button_hover_color(btn: u8) -> u32 {
    match btn {
        0 => COLOR_CLOSE_HOVER,
        1 => COLOR_MIN_HOVER,
        _ => COLOR_MAX_HOVER,
    }
}

/// Look up press target colour for a button index.
pub(crate) fn button_press_color(btn: u8) -> u32 {
    match btn {
        0 => COLOR_CLOSE_PRESS,
        1 => COLOR_MIN_PRESS,
        _ => COLOR_MAX_PRESS,
    }
}
