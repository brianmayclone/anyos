//! Theme colors for applications.
//!
//! Provides access to the current theme color palette. The theme flag
//! is stored in libanyui and read via DLL call; color palettes are
//! defined here to avoid serialization overhead.
//!
//! # Usage
//! ```rust
//! use libanyui_client as ui;
//! let tc = ui::theme::colors();
//! btn.set_system_icon("settings", ui::IconType::Outline, tc.text, 24);
//! ```

/// All semantic theme colors.
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
}

const DARK: ThemeColors = ThemeColors {
    window_bg:       0xFF1E1E1E,
    text:            0xFFE6E6E6,
    text_secondary:  0xFF969696,
    text_disabled:   0xFF5A5A5A,
    accent:          0xFF007AFF,
    accent_hover:    0xFF0A84FF,
    destructive:     0xFFFF3B30,
    success:         0xFF30D158,
    warning:         0xFFFFD60A,
    control_bg:      0xFF3C3C3C,
    control_hover:   0xFF484848,
    control_pressed: 0xFF2A2A2A,
    input_bg:        0xFF1A1A1A,
    input_border:    0xFF505050,
    input_focus:     0xFF007AFF,
    separator:       0xFF3D3D3D,
    selection:       0xFF0A54C4,
    sidebar_bg:      0xFF252525,
    card_bg:         0xFF2A2A2A,
    card_border:     0xFF3A3A3A,
    badge_red:       0xFFFF3B30,
    toggle_on:       0xFF30D158,
    toggle_off:      0xFF636366,
    toggle_thumb:    0xFFFFFFFF,
    scrollbar:       0xFF8E8E93,
    scrollbar_track: 0xFF3A3A3C,
    check_mark:      0xFFFFFFFF,
};

const LIGHT: ThemeColors = ThemeColors {
    window_bg:       0xFFF5F5F7,
    text:            0xFF1D1D1F,
    text_secondary:  0xFF86868B,
    text_disabled:   0xFFC7C7CC,
    accent:          0xFF007AFF,
    accent_hover:    0xFF0A84FF,
    destructive:     0xFFFF3B30,
    success:         0xFF34C759,
    warning:         0xFFFF9F0A,
    control_bg:      0xFFE5E5EA,
    control_hover:   0xFFD1D1D6,
    control_pressed: 0xFFC7C7CC,
    input_bg:        0xFFFFFFFF,
    input_border:    0xFFC7C7CC,
    input_focus:     0xFF007AFF,
    separator:       0xFFC6C6C8,
    selection:       0xFF007AFF,
    sidebar_bg:      0xFFF2F2F7,
    card_bg:         0xFFFFFFFF,
    card_border:     0xFFD1D1D6,
    badge_red:       0xFFFF3B30,
    toggle_on:       0xFF34C759,
    toggle_off:      0xFFE5E5EA,
    toggle_thumb:    0xFFFFFFFF,
    scrollbar:       0xFFA0A0A5,
    scrollbar_track: 0xFFE5E5EA,
    check_mark:      0xFFFFFFFF,
};

/// Get the current theme colors (queries libanyui for the active theme).
pub fn colors() -> &'static ThemeColors {
    let theme = (crate::lib().get_theme)();
    if theme == 0 { &DARK } else { &LIGHT }
}

/// Set the current theme (true = light, false = dark).
pub fn set_theme(light: bool) {
    (crate::lib().set_theme)(if light { 1 } else { 0 });
}

/// Check if the current theme is light.
pub fn is_light() -> bool {
    (crate::lib().get_theme)() != 0
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
