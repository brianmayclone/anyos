//! Theme color palette and sizing constants.
//!
//! Colors are dynamic — they change based on the system theme (dark/light)
//! and are loaded from `/System/compositor/themes/{dark,light}.conf` at init
//! time.  Falls back to built-in defaults if the files are missing.
//!
//! The theme value is stored locally and set via `set_theme()`.

use alloc::vec::Vec;

// ── ThemeColors struct ──────────────────────────────────────────────────────

/// All semantic theme colors.
///
/// Each field is an ARGB `u32` (`0xAARRGGBB`).  Controls reference these
/// values on every `render()` so that theme switches take effect immediately.
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

// ── Built-in default palettes (overwritten by .conf at runtime) ─────────

static mut DARK: ThemeColors = ThemeColors {
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
    editor_selection:  0xFF264F78,
    alt_row_bg:       0xFF232323,
    placeholder_bg:   0xFF2A2A2A,
};

static mut LIGHT: ThemeColors = ThemeColors {
    window_bg:        0xFFF5F5F7,
    text:             0xFF1D1D1F,
    text_secondary:   0xFF86868B,
    text_disabled:    0xFFC7C7CC,
    accent:           0xFF007AFF,
    accent_hover:     0xFF0A84FF,
    destructive:      0xFFFF3B30,
    success:          0xFF34C759,
    warning:          0xFFFF9F0A,
    control_bg:       0xFFE5E5EA,
    control_hover:    0xFFD1D1D6,
    control_pressed:  0xFFC7C7CC,
    input_bg:         0xFFFFFFFF,
    input_border:     0xFFC7C7CC,
    input_focus:      0xFF007AFF,
    separator:        0xFFC6C6C8,
    selection:        0xFF007AFF,
    sidebar_bg:       0xFFF2F2F7,
    card_bg:          0xFFFFFFFF,
    card_border:      0xFFD1D1D6,
    badge_red:        0xFFFF3B30,
    toggle_on:        0xFF34C759,
    toggle_off:       0xFFE5E5EA,
    toggle_thumb:     0xFFFFFFFF,
    scrollbar:        0xFFA0A0A5,
    scrollbar_track:  0xFFE5E5EA,
    check_mark:       0xFFFFFFFF,
    toolbar_bg:       0xFFE8E8ED,
    tab_inactive_bg:  0xFFE0E0E0,
    tab_hover_bg:     0xFFD0D0D0,
    tab_border_active:0xFF007ACC,
    editor_bg:        0xFFF5F5F7,
    editor_line_hl:   0xFFE8E8EC,
    editor_selection:  0xFFBBDEFB,
    alt_row_bg:       0xFFF0F0F2,
    placeholder_bg:   0xFFE0E0E0,
};

// ── Theme flag and accessors ────────────────────────────────────────────────

/// Address of the theme flag in the shared uisys DLIB export page.
///
/// The compositor writes here via `dll::set_dll_u32()`. All apps read the
/// same page (mapped RO), so theme changes propagate system-wide.
const THEME_SHARED_ADDR: *const u32 = 0x0400_000C as *const u32;

/// Fallback for when the shared page isn't mapped yet (e.g. during init).
static mut CURRENT_THEME_LOCAL: u32 = 0;

/// Set the local theme fallback (used before shared page is available).
pub fn set_theme(light: bool) {
    unsafe { CURRENT_THEME_LOCAL = if light { 1 } else { 0 }; }
}

/// Get the current theme flag (0 = dark, 1 = light).
///
/// Reads from the shared uisys DLIB page so all processes stay in sync.
pub fn get_theme() -> u32 {
    unsafe { core::ptr::read_volatile(THEME_SHARED_ADDR) }
}

/// Get the current theme colors.
///
/// Reads the theme flag from the shared page, so all apps always use the
/// correct palette regardless of which process changed the theme.
#[inline(always)]
pub fn colors() -> &'static ThemeColors {
    let t = unsafe { core::ptr::read_volatile(THEME_SHARED_ADDR) };
    unsafe { if t == 0 { &DARK } else { &LIGHT } }
}

/// Check if the current theme is light.
#[inline(always)]
pub fn is_light() -> bool {
    unsafe { core::ptr::read_volatile(THEME_SHARED_ADDR) != 0 }
}

/// Return a raw pointer to the current theme palette.
///
/// Used by the client library so it can read the live palette without
/// duplicating the color data.
pub fn colors_ptr() -> *const ThemeColors {
    colors() as *const ThemeColors
}

// ── .conf file loading ──────────────────────────────────────────────────────

/// Paths for on-disk theme definitions.
const DARK_CONF_PATH: &str = "/System/compositor/themes/dark.conf";
const LIGHT_CONF_PATH: &str = "/System/compositor/themes/light.conf";

/// Path for the user's current accent style preference.
const CURRENT_STYLE_PATH: &str = "/System/compositor/themes/current_style";
/// Directory containing accent style `.conf` files.
const STYLE_DIR: &str = "/System/compositor/themes/style/";

/// Load theme palettes from disk.  Missing or malformed files are silently
/// ignored — the built-in defaults remain in effect for any keys not found.
///
/// Also loads the current accent style preference if set.
pub fn load_from_disk() {
    if let Some(data) = read_file(DARK_CONF_PATH) {
        unsafe { parse_conf_into(&data, &mut DARK); }
    }
    if let Some(data) = read_file(LIGHT_CONF_PATH) {
        unsafe { parse_conf_into(&data, &mut LIGHT); }
    }
    load_current_style();
}

/// Read the current style name from disk and apply accent overrides.
fn load_current_style() {
    let Some(data) = read_file(CURRENT_STYLE_PATH) else { return };
    let name = match core::str::from_utf8(&data) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };
    if name.is_empty() { return; }

    let mut path_buf = [0u8; 128];
    let prefix = STYLE_DIR.as_bytes();
    let suffix = b".conf";
    let total = prefix.len() + name.len() + suffix.len();
    if total > path_buf.len() { return; }

    path_buf[..prefix.len()].copy_from_slice(prefix);
    path_buf[prefix.len()..prefix.len() + name.len()].copy_from_slice(name.as_bytes());
    path_buf[prefix.len() + name.len()..total].copy_from_slice(suffix);

    let path = match core::str::from_utf8(&path_buf[..total]) {
        Ok(s) => s,
        Err(_) => return,
    };

    let Some(style_data) = read_file(path) else { return };
    if let Some((da, dh, la, lh)) = parse_style_conf(&style_data) {
        apply_accent_style(da, dh, la, lh);
    }
}

/// Parse an accent style `.conf` file.
///
/// Returns `(dark_accent, dark_hover, light_accent, light_hover)`.
pub fn parse_style_conf(data: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let text = core::str::from_utf8(data).ok()?;
    let mut da = None;
    let mut dh = None;
    let mut la = None;
    let mut lh = None;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let val_str = line[eq + 1..].trim();
        let Some(val) = parse_hex_color(val_str) else { continue };

        match key {
            "ACCENT_DARK" => da = Some(val),
            "ACCENT_HOVER_DARK" => dh = Some(val),
            "ACCENT_LIGHT" => la = Some(val),
            "ACCENT_HOVER_LIGHT" => lh = Some(val),
            _ => {}
        }
    }

    Some((da?, dh?, la?, lh?))
}

/// Apply accent style overrides to **both** palettes.
///
/// `dark_accent`/`dark_hover` override DARK, `light_accent`/`light_hover` override LIGHT.
pub fn apply_accent_style(dark_accent: u32, dark_hover: u32, light_accent: u32, light_hover: u32) {
    unsafe {
        DARK.accent = dark_accent;
        DARK.accent_hover = dark_hover;
        DARK.input_focus = dark_accent;
        LIGHT.accent = light_accent;
        LIGHT.accent_hover = light_hover;
        LIGHT.input_focus = light_accent;
    }
}

/// Read a small file into a `Vec<u8>`.  Returns `None` on failure.
fn read_file(path: &str) -> Option<Vec<u8>> {
    use crate::syscall;
    let fd = syscall::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    // Theme files should be well under 4 KiB.
    let mut buf = alloc::vec![0u8; 4096];
    let n = syscall::read(fd, &mut buf);
    syscall::close(fd);
    if n == 0 || n == u32::MAX {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

/// Parse `KEY=0xAARRGGBB` lines into a `ThemeColors` struct.
fn parse_conf_into(data: &[u8], tc: &mut ThemeColors) {
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return,
    };

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let val_str = line[eq + 1..].trim();
        let Some(val) = parse_hex_color(val_str) else { continue };

        match key {
            "WINDOW_BG"        => tc.window_bg = val,
            "TEXT"             => tc.text = val,
            "TEXT_SECONDARY"   => tc.text_secondary = val,
            "TEXT_DISABLED"    => tc.text_disabled = val,
            "ACCENT"           => tc.accent = val,
            "ACCENT_HOVER"     => tc.accent_hover = val,
            "DESTRUCTIVE"      => tc.destructive = val,
            "SUCCESS"          => tc.success = val,
            "WARNING"          => tc.warning = val,
            "CONTROL_BG"       => tc.control_bg = val,
            "CONTROL_HOVER"    => tc.control_hover = val,
            "CONTROL_PRESSED"  => tc.control_pressed = val,
            "INPUT_BG"         => tc.input_bg = val,
            "INPUT_BORDER"     => tc.input_border = val,
            "INPUT_FOCUS"      => tc.input_focus = val,
            "SEPARATOR"        => tc.separator = val,
            "SELECTION"        => tc.selection = val,
            "SIDEBAR_BG"       => tc.sidebar_bg = val,
            "CARD_BG"          => tc.card_bg = val,
            "CARD_BORDER"      => tc.card_border = val,
            "BADGE_RED"        => tc.badge_red = val,
            "TOGGLE_ON"        => tc.toggle_on = val,
            "TOGGLE_OFF"       => tc.toggle_off = val,
            "TOGGLE_THUMB"     => tc.toggle_thumb = val,
            "SCROLLBAR"        => tc.scrollbar = val,
            "SCROLLBAR_TRACK"  => tc.scrollbar_track = val,
            "CHECK_MARK"       => tc.check_mark = val,
            "TOOLBAR_BG"       => tc.toolbar_bg = val,
            "TAB_INACTIVE_BG"  => tc.tab_inactive_bg = val,
            "TAB_HOVER_BG"     => tc.tab_hover_bg = val,
            "TAB_BORDER_ACTIVE"=> tc.tab_border_active = val,
            "EDITOR_BG"        => tc.editor_bg = val,
            "EDITOR_LINE_HL"   => tc.editor_line_hl = val,
            "EDITOR_SELECTION"  => tc.editor_selection = val,
            "ALT_ROW_BG"       => tc.alt_row_bg = val,
            "PLACEHOLDER_BG"   => tc.placeholder_bg = val,
            _ => {} // unknown key — silently skip
        }
    }
}

/// Parse a `0xAARRGGBB` hex string into a `u32`.
fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if hex.len() != 8 {
        return None;
    }
    let mut val: u32 = 0;
    for &b in hex.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        val = (val << 4) | digit as u32;
    }
    Some(val)
}

// ── Color utility functions (zero-cost color math) ───────────────────

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

/// Shadow spread for floating elements (Alert, Tooltip, ContextMenu).
/// Small value to keep SDF cost low on software rendering.
pub const POPUP_SHADOW_SPREAD: i32 = 8;
/// Shadow alpha for floating elements.
pub const POPUP_SHADOW_ALPHA: u32 = 50;
/// Shadow Y-offset for floating elements (light comes from above).
pub const POPUP_SHADOW_OFFSET_Y: i32 = 2;
