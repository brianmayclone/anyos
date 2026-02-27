//! Theme colors for applications.
//!
//! Provides access to the current theme color palette.  The palette lives in
//! the server-side libanyui (loaded as a DLL into the same address space) and
//! is accessed via a raw pointer obtained from `anyui_get_theme_colors_ptr()`.
//! This gives zero-cost, always-in-sync reads with no serialization overhead.
//!
//! # Usage
//! ```rust
//! use libanyui_client as ui;
//! let tc = ui::theme::colors();
//! btn.set_system_icon("settings", ui::IconType::Outline, tc.text, 24);
//! ```

/// All semantic theme colors.
///
/// Layout **must** match `libs/libanyui/src/theme.rs` exactly — the client
/// reads the server-side struct through a raw pointer.
#[repr(C)]
pub struct ThemeColors {
    pub window_bg: u32,
    pub text: u32,
    pub text_secondary: u32,
    pub text_disabled: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub destructive: u32,
    pub success: u32,
    pub warning: u32,
    pub control_bg: u32,
    pub control_hover: u32,
    pub control_pressed: u32,
    pub input_bg: u32,
    pub input_border: u32,
    pub input_focus: u32,
    pub separator: u32,
    pub selection: u32,
    pub sidebar_bg: u32,
    pub card_bg: u32,
    pub card_border: u32,
    pub badge_red: u32,
    pub toggle_on: u32,
    pub toggle_off: u32,
    pub toggle_thumb: u32,
    pub scrollbar: u32,
    pub scrollbar_track: u32,
    pub check_mark: u32,
    // ── Extended fields (controls that previously hardcoded colors) ───
    pub toolbar_bg: u32,
    pub tab_inactive_bg: u32,
    pub tab_hover_bg: u32,
    pub tab_border_active: u32,
    pub editor_bg: u32,
    pub editor_line_hl: u32,
    pub editor_selection: u32,
    pub alt_row_bg: u32,
    pub placeholder_bg: u32,
}

/// Fallback dark palette — used only when the server-side pointer is NULL
/// (i.e. libanyui hasn't been initialised yet).
const DARK: ThemeColors = ThemeColors {
    window_bg:        0xFF1E1E1E,
    text:             0xFFE6E6E6,
    text_secondary:   0xFF969696,
    text_disabled:    0xFF5A5A5A,
    accent:           0xFF007AFF,
    accent_hover:     0xFF0A84FF,
    destructive:      0xFFFF3B30,
    success:          0xFF30D158,
    warning:          0xFFFFD60A,
    control_bg:       0xFF3C3C3C,
    control_hover:    0xFF484848,
    control_pressed:  0xFF2A2A2A,
    input_bg:         0xFF1A1A1A,
    input_border:     0xFF505050,
    input_focus:      0xFF007AFF,
    separator:        0xFF3D3D3D,
    selection:        0xFF0A54C4,
    sidebar_bg:       0xFF252525,
    card_bg:          0xFF2A2A2A,
    card_border:      0xFF3A3A3A,
    badge_red:        0xFFFF3B30,
    toggle_on:        0xFF30D158,
    toggle_off:       0xFF636366,
    toggle_thumb:     0xFFFFFFFF,
    scrollbar:        0xFF8E8E93,
    scrollbar_track:  0xFF3A3A3C,
    check_mark:       0xFFFFFFFF,
    toolbar_bg:       0xFF2C2C2E,
    tab_inactive_bg:  0xFF2D2D2D,
    tab_hover_bg:     0xFF383838,
    tab_border_active:0xFF007ACC,
    editor_bg:        0xFF1E1E1E,
    editor_line_hl:   0xFF2A2D2E,
    editor_selection: 0xFF264F78,
    alt_row_bg:       0xFF232323,
    placeholder_bg:   0xFF2A2A2A,
};

/// Get the current theme colors.
///
/// Reads from the server-side libanyui palette via a raw pointer.  Falls back
/// to a built-in dark palette if the pointer is NULL.
pub fn colors() -> &'static ThemeColors {
    let ptr = (crate::lib().get_theme_colors_ptr)();
    if ptr.is_null() {
        &DARK
    } else {
        unsafe { &*(ptr as *const ThemeColors) }
    }
}

/// Set the current theme (true = light, false = dark).
pub fn set_theme(light: bool) {
    (crate::lib().set_theme)(if light { 1 } else { 0 });
}

/// Check if the current theme is light.
pub fn is_light() -> bool {
    (crate::lib().get_theme)() != 0
}

/// Apply accent style overrides to both dark and light palettes.
pub fn apply_accent_style(dark_accent: u32, dark_hover: u32, light_accent: u32, light_hover: u32) {
    (crate::lib().apply_accent_style)(dark_accent, dark_hover, light_accent, light_hover);
}

// ── Color utility functions ──────────────────────────────────────────

/// Darken a color by subtracting `amount` from each RGB channel.
#[inline(always)]
pub fn darken(color: u32, amount: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = ((color >> 16) & 0xFF).saturating_sub(amount);
    let g = ((color >> 8) & 0xFF).saturating_sub(amount);
    let b = (color & 0xFF).saturating_sub(amount);
    a | (r << 16) | (g << 8) | b
}

/// Lighten a color by adding `amount` to each RGB channel.
#[inline(always)]
pub fn lighten(color: u32, amount: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = (((color >> 16) & 0xFF) + amount).min(255);
    let g = (((color >> 8) & 0xFF) + amount).min(255);
    let b = ((color & 0xFF) + amount).min(255);
    a | (r << 16) | (g << 8) | b
}

/// Set the alpha channel of a color (0 = transparent, 255 = opaque).
#[inline(always)]
pub fn with_alpha(color: u32, alpha: u32) -> u32 {
    (color & 0x00FFFFFF) | ((alpha & 0xFF) << 24)
}
