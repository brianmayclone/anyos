//! Theme color palette and sizing constants.
//!
//! Colors are dynamic — they change based on the system theme (dark/light).
//! Sizing constants are theme-independent and remain `pub const`.

struct ThemeColors {
    window_bg: u32,
    text: u32,
    text_secondary: u32,
    text_disabled: u32,
    accent: u32,
    accent_hover: u32,
    destructive: u32,
    success: u32,
    warning: u32,
    control_bg: u32,
    control_hover: u32,
    control_pressed: u32,
    input_bg: u32,
    input_border: u32,
    input_focus: u32,
    separator: u32,
    selection: u32,
    sidebar_bg: u32,
    card_bg: u32,
    card_border: u32,
    badge_red: u32,
    toggle_on: u32,
    toggle_off: u32,
    toggle_thumb: u32,
    scrollbar: u32,
    scrollbar_track: u32,
    check_mark: u32,
}

const DARK: ThemeColors = ThemeColors {
    window_bg:      0xFF1E1E1E,
    text:           0xFFE6E6E6,
    text_secondary: 0xFF969696,
    text_disabled:  0xFF5A5A5A,
    accent:         0xFF007AFF,
    accent_hover:   0xFF0A84FF,
    destructive:    0xFFFF3B30,
    success:        0xFF30D158,
    warning:        0xFFFFD60A,
    control_bg:     0xFF3C3C3C,
    control_hover:  0xFF484848,
    control_pressed:0xFF2A2A2A,
    input_bg:       0xFF1A1A1A,
    input_border:   0xFF505050,
    input_focus:    0xFF007AFF,
    separator:      0xFF3D3D3D,
    selection:      0xFF0A54C4,
    sidebar_bg:     0xFF252525,
    card_bg:        0xFF2A2A2A,
    card_border:    0xFF3A3A3A,
    badge_red:      0xFFFF3B30,
    toggle_on:      0xFF30D158,
    toggle_off:     0xFF636366,
    toggle_thumb:   0xFFFFFFFF,
    scrollbar:      0xFF8E8E93,
    scrollbar_track:0xFF3A3A3C,
    check_mark:     0xFFFFFFFF,
};

const LIGHT: ThemeColors = ThemeColors {
    window_bg:      0xFFF5F5F7,
    text:           0xFF1D1D1F,
    text_secondary: 0xFF86868B,
    text_disabled:  0xFFC7C7CC,
    accent:         0xFF007AFF,
    accent_hover:   0xFF0A84FF,
    destructive:    0xFFFF3B30,
    success:        0xFF34C759,
    warning:        0xFFFF9F0A,
    control_bg:     0xFFE5E5EA,
    control_hover:  0xFFD1D1D6,
    control_pressed:0xFFC7C7CC,
    input_bg:       0xFFFFFFFF,
    input_border:   0xFFC7C7CC,
    input_focus:    0xFF007AFF,
    separator:      0xFFC6C6C8,
    selection:      0xFF007AFF,
    sidebar_bg:     0xFFF2F2F7,
    card_bg:        0xFFFFFFFF,
    card_border:    0xFFD1D1D6,
    badge_red:      0xFFFF3B30,
    toggle_on:      0xFF34C759,
    toggle_off:     0xFFE5E5EA,
    toggle_thumb:   0xFFFFFFFF,
    scrollbar:      0xFFA0A0A5,
    scrollbar_track:0xFFE5E5EA,
    check_mark:     0xFFFFFFFF,
};

/// Address of the `theme` field in the UisysExports struct (DLL base + 12).
/// Written by compositor, shared across all processes via DLL shared pages.
const THEME_ADDR: *const u32 = 0x0400_000C as *const u32;

#[inline(always)]
fn colors() -> &'static ThemeColors {
    let theme = unsafe { core::ptr::read_volatile(THEME_ADDR) };
    if theme == 0 { &DARK } else { &LIGHT }
}

// --- Color accessors (dynamic, theme-aware) ---

#[inline(always)] pub fn WINDOW_BG() -> u32       { colors().window_bg }
#[inline(always)] pub fn TEXT() -> u32             { colors().text }
#[inline(always)] pub fn TEXT_SECONDARY() -> u32   { colors().text_secondary }
#[inline(always)] pub fn TEXT_DISABLED() -> u32    { colors().text_disabled }
#[inline(always)] pub fn ACCENT() -> u32           { colors().accent }
#[inline(always)] pub fn ACCENT_HOVER() -> u32     { colors().accent_hover }
#[inline(always)] pub fn DESTRUCTIVE() -> u32      { colors().destructive }
#[inline(always)] pub fn SUCCESS() -> u32          { colors().success }
#[inline(always)] pub fn WARNING() -> u32          { colors().warning }
#[inline(always)] pub fn CONTROL_BG() -> u32       { colors().control_bg }
#[inline(always)] pub fn CONTROL_HOVER() -> u32    { colors().control_hover }
#[inline(always)] pub fn CONTROL_PRESSED() -> u32  { colors().control_pressed }
#[inline(always)] pub fn INPUT_BG() -> u32         { colors().input_bg }
#[inline(always)] pub fn INPUT_BORDER() -> u32     { colors().input_border }
#[inline(always)] pub fn INPUT_FOCUS() -> u32      { colors().input_focus }
#[inline(always)] pub fn SEPARATOR() -> u32        { colors().separator }
#[inline(always)] pub fn SELECTION() -> u32        { colors().selection }
#[inline(always)] pub fn SIDEBAR_BG() -> u32       { colors().sidebar_bg }
#[inline(always)] pub fn CARD_BG() -> u32          { colors().card_bg }
#[inline(always)] pub fn CARD_BORDER() -> u32      { colors().card_border }
#[inline(always)] pub fn BADGE_RED() -> u32        { colors().badge_red }
#[inline(always)] pub fn TOGGLE_ON() -> u32        { colors().toggle_on }
#[inline(always)] pub fn TOGGLE_OFF() -> u32       { colors().toggle_off }
#[inline(always)] pub fn TOGGLE_THUMB() -> u32     { colors().toggle_thumb }
#[inline(always)] pub fn SCROLLBAR() -> u32        { colors().scrollbar }
#[inline(always)] pub fn SCROLLBAR_TRACK() -> u32  { colors().scrollbar_track }
#[inline(always)] pub fn CHECK_MARK() -> u32       { colors().check_mark }

// --- Sizing (theme-independent) ---

pub const BUTTON_HEIGHT: u32     = 28;
pub const BUTTON_PADDING_H: u32  = 16;
pub const BUTTON_CORNER: u32     = 6;
pub const INPUT_HEIGHT: u32      = 28;
pub const TOGGLE_WIDTH: u32      = 36;
pub const TOGGLE_HEIGHT: u32     = 20;
pub const TOGGLE_THUMB_SIZE: u32 = 16;
pub const CHECKBOX_SIZE: u32     = 18;
pub const FONT_SIZE_SMALL: u32   = 13;
pub const FONT_SIZE_NORMAL: u32  = 16;
pub const FONT_SIZE_LARGE: u32   = 20;
pub const FONT_SIZE_TITLE: u32   = 24;
pub const CHAR_WIDTH: u32        = 8;
pub const CHAR_HEIGHT: u32       = 16;
