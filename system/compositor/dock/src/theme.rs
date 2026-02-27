//! Dock theme constants — colors, fonts, dimensions.

use crate::settings::DockSettings;

// ── Dynamic geometry ──

/// Computed dock geometry derived from [`DockSettings`].
pub struct DockGeometry {
    pub icon_size: u32,
    pub dock_height: u32,
    pub tooltip_h: u32,
    pub margin: u32,
    pub total_h: u32,
    pub icon_spacing: u32,
    pub h_padding: u32,
    pub border_radius: i32,
    pub position: u32,
}

impl DockGeometry {
    /// Compute geometry from settings.
    pub fn from_settings(s: &DockSettings) -> Self {
        let icon_size = s.icon_size;
        let dock_height = icon_size + 16;
        let tooltip_h = 28u32;
        // Magnified icons grow above the pill; reserve space for overflow + tooltip
        let mag_overflow = if s.magnification && s.mag_size > icon_size {
            s.mag_size - icon_size
        } else {
            0
        };
        let margin = 8 + tooltip_h + mag_overflow;
        let total_h = dock_height + margin;
        let icon_spacing = (icon_size / 5).clamp(6, 14);
        let h_padding = 12u32;
        let br = (icon_size / 3) as i32;
        let border_radius = br.clamp(8, 24);
        Self {
            icon_size,
            dock_height,
            tooltip_h,
            margin,
            total_h,
            icon_spacing,
            h_padding,
            border_radius,
            position: s.position,
        }
    }
}

static mut GEOMETRY: Option<DockGeometry> = None;

/// Get the current dock geometry (must call `set_geometry` first).
pub fn geometry() -> &'static DockGeometry {
    unsafe { GEOMETRY.as_ref().unwrap() }
}

/// Set the dock geometry (call on startup and when settings change).
pub fn set_geometry(g: DockGeometry) {
    unsafe { GEOMETRY = Some(g); }
}

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
