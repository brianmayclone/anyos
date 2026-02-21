//! Theme color palette and sizing constants.
//!
//! Colors are dynamic — they change based on the system theme (dark/light).
//! The theme value is read from the uisys DLL's shared page (volatile).

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

/// Address of the `theme` field in the UisysExports struct (DLL base + 12).
/// Written by compositor, shared across all processes via DLL shared pages.
const THEME_ADDR: *const u32 = 0x0400_000C as *const u32;

/// Get the current theme colors (reads theme flag from uisys DLL page).
#[inline(always)]
pub fn colors() -> &'static ThemeColors {
    let theme = unsafe { core::ptr::read_volatile(THEME_ADDR) };
    if theme == 0 { &DARK } else { &LIGHT }
}

/// Check if the current theme is light.
#[inline(always)]
pub fn is_light() -> bool {
    let val: u32 = unsafe { core::ptr::read_volatile(THEME_ADDR) };
    val != 0
}

// ── Sizing constants (theme-independent) ────────────────────────────

pub const BUTTON_HEIGHT: u32     = 28;
pub const BUTTON_PADDING_H: u32  = 16;
pub const BUTTON_CORNER: u32     = 6;
pub const INPUT_HEIGHT: u32      = 28;
pub const INPUT_CORNER: u32      = 6;
pub const TOGGLE_WIDTH: u32      = 36;
pub const TOGGLE_HEIGHT: u32     = 20;
pub const TOGGLE_THUMB_SIZE: u32 = 16;
pub const CHECKBOX_SIZE: u32     = 18;
pub const RADIO_SIZE: u32        = 18;
pub const FONT_SIZE_SMALL: u16   = 13;
pub const FONT_SIZE_DEFAULT: u16 = 13;
pub const FONT_SIZE_NORMAL: u16  = 16;
pub const FONT_SIZE_LARGE: u16   = 20;
pub const FONT_SIZE_TITLE: u16   = 24;
pub const CARD_CORNER: u32       = 8;
pub const TOOLTIP_CORNER: u32    = 6;
pub const ALERT_CORNER: u32      = 10;
