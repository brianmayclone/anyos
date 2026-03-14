#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;
use anyos_std::process;
use anyos_std::fs;
use anyos_std::ipc;
use alloc::string::ToString;
use alloc::boxed::Box;
use libanyui_client as anyui;
use anyui::{Widget, Control, MOD_CTRL, MOD_SHIFT, MOD_ALT,
    KEY_ENTER, KEY_BACKSPACE, KEY_TAB, KEY_ESCAPE,
    KEY_UP, KEY_DOWN, KEY_LEFT, KEY_RIGHT,
    KEY_DELETE, KEY_HOME, KEY_END, KEY_PAGE_UP, KEY_PAGE_DOWN};

anyos_std::entry!(main);

// ─── Constants ───────────────────────────────────────────────────────────────
const DEFAULT_CELL_W: u16 = 8;
const DEFAULT_CELL_H: u16 = 16;
const TEXT_PAD: u16 = 4;
const MAX_SCROLLBACK: usize = 5000;

// Colors (ARGB)
const COLOR_BG: u32 = 0xFF1E1E28;
const COLOR_FG: u32 = 0xFFCCCCCC;
const COLOR_PROMPT: u32 = 0xFF64FF64;
const COLOR_TITLE: u32 = 0xFF00C8FF;
const COLOR_DIM: u32 = 0xFF969696;

// Selection highlight colors
const COLOR_SELECT_BG: u32 = 0xFF3A5070;
const COLOR_SELECT_FG: u32 = 0xFFFFFFFF;

// Search highlight color
const COLOR_SEARCH_BG: u32 = 0xFFAA6600;
const COLOR_SEARCH_CURRENT_BG: u32 = 0xFFFF9900;

// Scrollbar
const SCROLLBAR_WIDTH: u32 = 10;
const SCROLLBAR_TRACK_COLOR: u32 = 0xFF2A2A2A;
const SCROLLBAR_THUMB_COLOR: u32 = 0xFF606060;
const SCROLLBAR_THUMB_HOVER_COLOR: u32 = 0xFF808080;
const SCROLLBAR_MIN_THUMB_H: u32 = 20;

// ─── Font (Andale Mono via compositor's libfont, font_id=4) ──────────────────

const DEFAULT_FONT_SIZE: u16 = 13;
const FONT_ID_MONO: u32 = 4; // SYSTEM_FONT_MONO in libfont = Andale Mono
const MIN_FONT_SIZE: u16 = 8;
const MAX_FONT_SIZE: u16 = 32;

// ─── Color Themes ────────────────────────────────────────────────────────────

struct Theme {
    name: &'static str,
    bg: u32,
    fg: u32,
    prompt: u32,
    title: u32,
    dim: u32,
    select_bg: u32,
    select_fg: u32,
    // ANSI 8-color palette
    ansi_colors: [u32; 8],
    ansi_bright: [u32; 8],
}

const THEMES: &[Theme] = &[
    Theme {
        name: "Default",
        bg: 0xFF1E1E28, fg: 0xFFCCCCCC, prompt: 0xFF64FF64, title: 0xFF00C8FF, dim: 0xFF969696,
        select_bg: 0xFF3A5070, select_fg: 0xFFFFFFFF,
        ansi_colors: [0xFF505050, 0xFFFF5555, 0xFF50FA7B, 0xFFF1FA8C, 0xFF6272A4, 0xFFFF79C6, 0xFF8BE9FD, 0xFFCCCCCC],
        ansi_bright: [0xFF6A6A6A, 0xFFFF6E6E, 0xFF69FF94, 0xFFFFFFA5, 0xFF7B8ABD, 0xFFFF92DF, 0xFFA4F0FF, 0xFFFFFFFF],
    },
    Theme {
        name: "Dracula",
        bg: 0xFF282A36, fg: 0xFFF8F8F2, prompt: 0xFF50FA7B, title: 0xFF8BE9FD, dim: 0xFF6272A4,
        select_bg: 0xFF44475A, select_fg: 0xFFF8F8F2,
        ansi_colors: [0xFF21222C, 0xFFFF5555, 0xFF50FA7B, 0xFFF1FA8C, 0xFFBD93F9, 0xFFFF79C6, 0xFF8BE9FD, 0xFFF8F8F2],
        ansi_bright: [0xFF6272A4, 0xFFFF6E6E, 0xFF69FF94, 0xFFFFFFA5, 0xFFD6ACFF, 0xFFFF92DF, 0xFFA4F0FF, 0xFFFFFFFF],
    },
    Theme {
        name: "Solarized Dark",
        bg: 0xFF002B36, fg: 0xFF839496, prompt: 0xFF859900, title: 0xFF268BD2, dim: 0xFF586E75,
        select_bg: 0xFF073642, select_fg: 0xFF93A1A1,
        ansi_colors: [0xFF073642, 0xFFDC322F, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFFD33682, 0xFF2AA198, 0xFFEEE8D5],
        ansi_bright: [0xFF586E75, 0xFFCB4B16, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFF6C71C4, 0xFF2AA198, 0xFFFDF6E3],
    },
    Theme {
        name: "Solarized Light",
        bg: 0xFFFDF6E3, fg: 0xFF657B83, prompt: 0xFF859900, title: 0xFF268BD2, dim: 0xFF93A1A1,
        select_bg: 0xFFEEE8D5, select_fg: 0xFF073642,
        ansi_colors: [0xFF073642, 0xFFDC322F, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFFD33682, 0xFF2AA198, 0xFFEEE8D5],
        ansi_bright: [0xFF586E75, 0xFFCB4B16, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFF6C71C4, 0xFF2AA198, 0xFFFDF6E3],
    },
    Theme {
        name: "Monokai",
        bg: 0xFF272822, fg: 0xFFF8F8F2, prompt: 0xFFA6E22E, title: 0xFF66D9EF, dim: 0xFF75715E,
        select_bg: 0xFF49483E, select_fg: 0xFFF8F8F2,
        ansi_colors: [0xFF272822, 0xFFF92672, 0xFFA6E22E, 0xFFF4BF75, 0xFF66D9EF, 0xFFAE81FF, 0xFFA1EFE4, 0xFFF8F8F2],
        ansi_bright: [0xFF75715E, 0xFFF92672, 0xFFA6E22E, 0xFFF4BF75, 0xFF66D9EF, 0xFFAE81FF, 0xFFA1EFE4, 0xFFF9F8F5],
    },
    Theme {
        name: "Nord",
        bg: 0xFF2E3440, fg: 0xFFD8DEE9, prompt: 0xFFA3BE8C, title: 0xFF88C0D0, dim: 0xFF4C566A,
        select_bg: 0xFF434C5E, select_fg: 0xFFECEFF4,
        ansi_colors: [0xFF3B4252, 0xFFBF616A, 0xFFA3BE8C, 0xFFEBCB8B, 0xFF81A1C1, 0xFFB48EAD, 0xFF88C0D0, 0xFFE5E9F0],
        ansi_bright: [0xFF4C566A, 0xFFBF616A, 0xFFA3BE8C, 0xFFEBCB8B, 0xFF81A1C1, 0xFFB48EAD, 0xFF8FBCBB, 0xFFECEFF4],
    },
    Theme {
        name: "Gruvbox",
        bg: 0xFF282828, fg: 0xFFEBDBB2, prompt: 0xFFB8BB26, title: 0xFF83A598, dim: 0xFF928374,
        select_bg: 0xFF3C3836, select_fg: 0xFFFBF1C7,
        ansi_colors: [0xFF282828, 0xFFCC241D, 0xFF98971A, 0xFFD79921, 0xFF458588, 0xFFB16286, 0xFF689D6A, 0xFFA89984],
        ansi_bright: [0xFF928374, 0xFFFB4934, 0xFFB8BB26, 0xFFFABD2F, 0xFF83A598, 0xFFD3869B, 0xFF8EC07C, 0xFFEBDBB2],
    },
    Theme {
        name: "One Dark",
        bg: 0xFF282C34, fg: 0xFFABB2BF, prompt: 0xFF98C379, title: 0xFF61AFEF, dim: 0xFF5C6370,
        select_bg: 0xFF3E4451, select_fg: 0xFFFFFFFF,
        ansi_colors: [0xFF282C34, 0xFFE06C75, 0xFF98C379, 0xFFE5C07B, 0xFF61AFEF, 0xFFC678DD, 0xFF56B6C2, 0xFFABB2BF],
        ansi_bright: [0xFF5C6370, 0xFFE06C75, 0xFF98C379, 0xFFE5C07B, 0xFF61AFEF, 0xFFC678DD, 0xFF56B6C2, 0xFFFFFFFF],
    },
];

// ─── Cell Structure ─────────────────────────────────────────────────────────

/// Attributes packed into a u8 bitfield.
const ATTR_BOLD: u8       = 0x01;
const ATTR_ITALIC: u8     = 0x02;
const ATTR_UNDERLINE: u8  = 0x04;
const ATTR_BLINK: u8      = 0x08;
const ATTR_REVERSE: u8    = 0x10;
const ATTR_STRIKETHROUGH: u8 = 0x20;

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: u32,
    bg: u32,
    attr: u8,
    /// Cell display width: 0 = continuation of wide char, 1 = normal, 2 = wide (CJK)
    width: u8,
    /// Combining/diacritical character (0 = none)
    combining: char,
    /// Hyperlink ID: 0 = no link, >0 = index into hyperlink table
    link_id: u16,
}

impl Cell {
    fn blank(fg: u32, bg: u32) -> Self {
        Cell { ch: ' ', fg, bg, attr: 0, width: 1, combining: '\0', link_id: 0 }
    }
}

/// Determine the display width of a Unicode character.
/// Returns 2 for CJK wide characters, 0 for combining/zero-width, 1 otherwise.
fn char_width(c: char) -> u8 {
    let cp = c as u32;
    // Zero-width and combining characters
    if is_combining(c) || cp == 0x200B || cp == 0x200C || cp == 0x200D || cp == 0xFEFF {
        return 0;
    }
    // CJK wide character ranges
    if (cp >= 0x1100 && cp <= 0x115F)   // Hangul Jamo
        || cp == 0x2329 || cp == 0x232A  // Angle brackets
        || (cp >= 0x2E80 && cp <= 0x303E) // CJK Radicals, Kangxi, Ideographic Description, CJK Symbols
        || (cp >= 0x3040 && cp <= 0x33BF) // Hiragana, Katakana, Bopomofo, Hangul Compat Jamo, Kanbun, CJK Strokes
        || (cp >= 0x3400 && cp <= 0x4DBF) // CJK Unified Ext A
        || (cp >= 0x4E00 && cp <= 0xA4CF) // CJK Unified, Yi Syllables/Radicals
        || (cp >= 0xA960 && cp <= 0xA97C) // Hangul Jamo Extended-A
        || (cp >= 0xAC00 && cp <= 0xD7A3) // Hangul Syllables
        || (cp >= 0xF900 && cp <= 0xFAFF) // CJK Compatibility Ideographs
        || (cp >= 0xFE10 && cp <= 0xFE6F) // Vertical forms, CJK Compat Forms, Small Form Variants
        || (cp >= 0xFF01 && cp <= 0xFF60) // Fullwidth Forms
        || (cp >= 0xFFE0 && cp <= 0xFFE6) // Fullwidth Signs
        || (cp >= 0x1F300 && cp <= 0x1F9FF) // Emoji (Miscellaneous Symbols, Emoticons, etc.)
        || (cp >= 0x20000 && cp <= 0x2FA1F) // CJK Unified Ext B-F, Compat Supplement
        || (cp >= 0x30000 && cp <= 0x323AF) // CJK Unified Ext G-I
    {
        return 2;
    }
    1
}

/// Check if a character is a Unicode combining character (zero-width).
fn is_combining(c: char) -> bool {
    let cp = c as u32;
    (cp >= 0x0300 && cp <= 0x036F)   // Combining Diacritical Marks
    || (cp >= 0x0483 && cp <= 0x0489) // Cyrillic combining
    || (cp >= 0x0591 && cp <= 0x05BD) // Hebrew combining
    || (cp >= 0x05BF && cp <= 0x05BF)
    || (cp >= 0x05C1 && cp <= 0x05C2)
    || (cp >= 0x05C4 && cp <= 0x05C5)
    || (cp >= 0x05C7 && cp <= 0x05C7)
    || (cp >= 0x0610 && cp <= 0x061A) // Arabic combining
    || (cp >= 0x064B && cp <= 0x065F)
    || cp == 0x0670
    || (cp >= 0x06D6 && cp <= 0x06DC)
    || (cp >= 0x06DF && cp <= 0x06E4)
    || (cp >= 0x06E7 && cp <= 0x06E8)
    || (cp >= 0x06EA && cp <= 0x06ED)
    || (cp >= 0x0730 && cp <= 0x074A) // Syriac combining
    || (cp >= 0x07A6 && cp <= 0x07B0) // Thaana combining
    || (cp >= 0x0900 && cp <= 0x0903) // Devanagari combining
    || (cp >= 0x093A && cp <= 0x094F)
    || (cp >= 0x0951 && cp <= 0x0957)
    || (cp >= 0x0962 && cp <= 0x0963)
    || (cp >= 0x0E31 && cp <= 0x0E31) // Thai combining
    || (cp >= 0x0E34 && cp <= 0x0E3A)
    || (cp >= 0x0E47 && cp <= 0x0E4E)
    || (cp >= 0x1AB0 && cp <= 0x1AFF) // Combining Diacritical Marks Extended
    || (cp >= 0x1DC0 && cp <= 0x1DFF) // Combining Diacritical Marks Supplement
    || (cp >= 0x20D0 && cp <= 0x20FF) // Combining Diacritical Marks for Symbols
    || (cp >= 0xFE00 && cp <= 0xFE0F) // Variation Selectors
    || (cp >= 0xFE20 && cp <= 0xFE2F) // Combining Half Marks
    || (cp >= 0xE0100 && cp <= 0xE01EF) // Variation Selectors Supplement
}

/// Convert a 256-color index to an ARGB color.
fn color_256(idx: u32, colors: &[u32; 8], bright: &[u32; 8]) -> u32 {
    if idx < 8 {
        colors[idx as usize]
    } else if idx < 16 {
        bright[(idx - 8) as usize]
    } else if idx < 232 {
        // 216 color cube: 6×6×6
        let i = idx - 16;
        let r = (i / 36) % 6;
        let g = (i / 6) % 6;
        let b = i % 6;
        let r = if r == 0 { 0u8 } else { (r * 40 + 55) as u8 };
        let g = if g == 0 { 0u8 } else { (g * 40 + 55) as u8 };
        let b = if b == 0 { 0u8 } else { (b * 40 + 55) as u8 };
        0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
    } else {
        // Grayscale: 24 shades from 8 to 238
        let s = ((idx - 232) * 10 + 8).min(255) as u8;
        0xFF000000 | (s as u32) << 16 | (s as u32) << 8 | s as u32
    }
}

/// Fill a rectangle in a raw pixel buffer.
fn fill_rect_buf(pixels: *mut u32, stride: u32, buf_h: u32, x: i32, y: i32, w: u16, h: u16, color: u32) {
    for row in 0..h as i32 {
        let py = y + row;
        if py < 0 || py >= buf_h as i32 { continue; }
        for col in 0..w as i32 {
            let px = x + col;
            if px >= 0 && px < stride as i32 {
                let di = py as usize * stride as usize + px as usize;
                unsafe { *pixels.add(di) = color; }
            }
        }
    }
}

// ─── Text Selection ──────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct CellPos {
    row: usize, // absolute line index in TerminalBuffer.lines[]
    col: usize, // column index within that line
}

struct Selection {
    dragging: bool,
    anchor: CellPos,  // where the drag started
    active: CellPos,  // current drag endpoint
}

impl Selection {
    /// Return (start, end) in reading order.
    fn ordered(&self) -> (CellPos, CellPos) {
        if self.anchor.row < self.active.row
            || (self.anchor.row == self.active.row && self.anchor.col <= self.active.col)
        {
            (self.anchor, self.active)
        } else {
            (self.active, self.anchor)
        }
    }
}

// ─── Selection Helpers ───────────────────────────────────────────────────────

/// Convert pixel coordinates to a cell position in the buffer.
fn pixel_to_cell_in_rect(px: u32, py: u32, buf: &TerminalBuffer, rect: Rect, cell_w: u16, cell_h: u16) -> CellPos {
    let cell_w = cell_w as u32;
    let cell_h = cell_h as u32;
    let local_x = px.saturating_sub(rect.x as u32);
    let local_y = py.saturating_sub(rect.y as u32);
    let col = if local_x > TEXT_PAD as u32 {
        ((local_x - TEXT_PAD as u32) / cell_w) as usize
    } else {
        0
    };
    let screen_row = if local_y > TEXT_PAD as u32 {
        ((local_y - TEXT_PAD as u32) / cell_h) as usize
    } else {
        0
    };
    let row = buf.scroll_offset + screen_row;
    let row = row.min(buf.lines.len().saturating_sub(1));
    let col = col.min(buf.cols.saturating_sub(1));
    CellPos { row, col }
}

/// Extract the word at a given cell position from the terminal buffer.
/// A "word" is a contiguous run of non-space characters.
fn word_at_cell(buf: &TerminalBuffer, row: usize, col: usize) -> String {
    if row >= buf.lines.len() { return String::new(); }
    let line = &buf.lines[row];
    if col >= line.len() { return String::new(); }
    if line[col].ch == ' ' { return String::new(); }
    // Find word boundaries
    let mut start = col;
    while start > 0 && line[start - 1].ch != ' ' { start -= 1; }
    let mut end = col;
    while end + 1 < line.len() && line[end + 1].ch != ' ' { end += 1; }
    let mut word = String::new();
    for i in start..=end { word.push(line[i].ch); }
    word
}

/// Extract the selected text from the terminal buffer.
fn extract_selected_text(buf: &TerminalBuffer, sel: &Selection) -> String {
    let (start, end) = sel.ordered();
    let mut result = String::new();
    for row in start.row..=end.row {
        if row >= buf.lines.len() {
            break;
        }
        let line = &buf.lines[row];
        let c0 = if row == start.row { start.col } else { 0 };
        let c1 = if row == end.row { end.col + 1 } else { line.len() };
        let c1 = c1.min(line.len());
        let mut line_text = String::new();
        for col in c0..c1 {
            line_text.push(line[col].ch);
        }
        // Trim trailing spaces
        while line_text.ends_with(' ') {
            line_text.pop();
        }
        if row > start.row {
            result.push('\n');
        }
        result.push_str(&line_text);
    }
    result
}

// ─── Output Redirect — delegated to anyos_std::shell ─────────────────────────

use anyos_std::shell::Redirect;
use anyos_std::shell::InputRedirect;

/// Parse input redirect — delegates to anyos_std::shell.
fn parse_input_redirect(line: &str, cwd: &str) -> (String, Option<InputRedirect>) {
    anyos_std::shell::parse_input_redirect(line, cwd)
}

/// Parse output redirect operators — delegates to anyos_std::shell.
fn parse_redirects(line: &str, cwd: &str) -> (String, Option<Redirect>) {
    anyos_std::shell::parse_redirects(line, cwd)
}

/// Write captured output to redirect target — delegates to anyos_std::shell.
fn write_redirect(redirect: &mut Redirect, data: &str) {
    anyos_std::shell::write_redirect(redirect, data)
}

/// Tokenize + expand (vars, tildes, globs) — delegates to anyos_std::shell.
fn expand_args(raw: &str, cwd: &str) -> Vec<String> {
    anyos_std::shell::expand_args(raw, cwd)
}

/// Re-join tokens with shell quoting — delegates to anyos_std::shell.
fn shell_join(tokens: &[String]) -> String {
    anyos_std::shell::join(tokens)
}

// ─── Terminal Buffer ─────────────────────────────────────────────────────────

struct TerminalBuffer {
    lines: Vec<Vec<Cell>>,
    cols: usize,
    visible_rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_offset: usize,
    current_fg: u32,
    current_bg: u32,
    current_attr: u8,
    // ANSI escape sequence parser state
    ansi_state: u8,          // 0=Normal, 1=Escape(\x1B), 2=CSI([), 3=OSC(]), 4=CSI ? prefix, 5=DCS
    ansi_params: [u8; 64],
    ansi_param_len: usize,
    // Capture mode: when set, write_char appends to this instead of terminal lines
    capture: Option<String>,
    // Cursor visibility
    cursor_visible: bool,
    // Alternate screen buffer
    alt_lines: Option<Vec<Vec<Cell>>>,
    alt_cursor_row: usize,
    alt_cursor_col: usize,
    alt_scroll_offset: usize,
    // Scroll region (top, bottom) — 0-based inclusive
    scroll_top: usize,
    scroll_bottom: usize, // 0 means "use visible_rows - 1"
    // OSC sequence accumulator
    osc_buf: Vec<u8>,
    // Bracketed paste mode
    bracketed_paste: bool,
    // Mouse tracking mode: 0=off, 1000=basic, 1002=button, 1003=any, 1006=SGR
    mouse_mode: u16,
    mouse_sgr: bool,
    // Hyperlinks (OSC 8)
    hyperlinks: Vec<String>,  // index 0 unused, 1..N = URLs
    current_link_id: u16,
}

impl TerminalBuffer {
    fn new(cols: usize, rows: usize) -> Self {
        let mut lines = Vec::new();
        lines.push(Vec::new());
        TerminalBuffer {
            lines,
            cols,
            visible_rows: rows,
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            current_fg: COLOR_FG,
            current_bg: 0,
            current_attr: 0,
            ansi_state: 0,
            ansi_params: [0; 64],
            ansi_param_len: 0,
            capture: None,
            cursor_visible: true,
            alt_lines: None,
            alt_cursor_row: 0,
            alt_cursor_col: 0,
            alt_scroll_offset: 0,
            scroll_top: 0,
            scroll_bottom: 0,
            osc_buf: Vec::new(),
            bracketed_paste: false,
            mouse_mode: 0,
            mouse_sgr: false,
            hyperlinks: Vec::new(),
            current_link_id: 0,
        }
    }

    fn ensure_line(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    /// Get effective scroll bottom (0 means end of visible area).
    fn effective_scroll_bottom(&self) -> usize {
        if self.scroll_bottom == 0 { self.visible_rows.saturating_sub(1) } else { self.scroll_bottom }
    }

    /// Scroll lines within the scroll region up by one.
    fn scroll_region_up(&mut self) {
        let top = self.scroll_top + self.scroll_offset;
        let bot = self.effective_scroll_bottom() + self.scroll_offset;
        if top < self.lines.len() && bot < self.lines.len() && top < bot {
            self.lines.remove(top);
            while self.lines.len() <= bot {
                self.lines.push(Vec::new());
            }
            self.lines.insert(bot, Vec::new());
        }
    }

    /// Scroll lines within the scroll region down by one.
    fn scroll_region_down(&mut self) {
        let top = self.scroll_top + self.scroll_offset;
        let bot = self.effective_scroll_bottom() + self.scroll_offset;
        if top < self.lines.len() && bot < self.lines.len() && top < bot {
            if bot < self.lines.len() {
                self.lines.remove(bot);
            }
            self.lines.insert(top, Vec::new());
        }
    }

    /// Enter alternate screen buffer.
    fn enter_alt_screen(&mut self) {
        if self.alt_lines.is_some() { return; }
        self.alt_cursor_row = self.cursor_row;
        self.alt_cursor_col = self.cursor_col;
        self.alt_scroll_offset = self.scroll_offset;
        let saved = core::mem::replace(&mut self.lines, Vec::new());
        self.alt_lines = Some(saved);
        // Initialize alt screen with empty lines
        for _ in 0..self.visible_rows {
            self.lines.push(Vec::new());
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
    }

    /// Leave alternate screen buffer, restoring the main buffer.
    fn leave_alt_screen(&mut self) {
        if let Some(saved) = self.alt_lines.take() {
            self.lines = saved;
            self.cursor_row = self.alt_cursor_row;
            self.cursor_col = self.alt_cursor_col;
            self.scroll_offset = self.alt_scroll_offset;
        }
    }

    fn write_char(&mut self, ch: char) {
        // Capture mode: redirect output to capture buffer
        if let Some(ref mut cap) = self.capture {
            match self.ansi_state {
                1 => {
                    match ch {
                        '[' => { self.ansi_state = 2; }
                        ']' => { self.ansi_state = 3; }
                        _ => { self.ansi_state = 0; }
                    }
                    return;
                }
                2 | 4 => {
                    let c = ch as u32;
                    if (c >= b'0' as u32 && c <= b'9' as u32) || c == b';' as u32 || ch == '?' {
                        return;
                    }
                    self.ansi_state = 0;
                    return;
                }
                3 => {
                    // OSC in capture mode: discard until BEL or ST
                    if ch == '\x07' { self.ansi_state = 0; }
                    return;
                }
                _ => {}
            }
            match ch {
                '\x1B' => { self.ansi_state = 1; }
                '\n' => { cap.push('\n'); }
                '\r' => {}
                '\x07' => {} // BEL
                _ => { cap.push(ch); }
            }
            return;
        }

        // ANSI escape sequence state machine
        match self.ansi_state {
            1 => {
                // Saw \x1B
                match ch {
                    '[' => {
                        self.ansi_state = 2;
                        self.ansi_param_len = 0;
                    }
                    ']' => {
                        self.ansi_state = 3; // OSC
                        self.osc_buf.clear();
                    }
                    '(' | ')' | '*' | '+' => {
                        self.ansi_state = 5; // charset designation, eat next char
                    }
                    '7' => {
                        // DECSC — save cursor (simplified)
                        self.ansi_state = 0;
                    }
                    '8' => {
                        // DECRC — restore cursor (simplified)
                        self.ansi_state = 0;
                    }
                    'M' => {
                        // Reverse index (scroll down / cursor up)
                        if self.cursor_row > self.scroll_top + self.scroll_offset {
                            self.cursor_row -= 1;
                        } else {
                            self.scroll_region_down();
                        }
                        self.ansi_state = 0;
                    }
                    'c' => {
                        // RIS — full reset
                        self.current_fg = COLOR_FG;
                        self.current_bg = 0;
                        self.current_attr = 0;
                        self.cursor_visible = true;
                        self.scroll_top = 0;
                        self.scroll_bottom = 0;
                        self.ansi_state = 0;
                    }
                    _ => {
                        self.ansi_state = 0;
                    }
                }
                return;
            }
            2 => {
                // Inside CSI sequence
                if ch == '?' {
                    self.ansi_state = 4; // DEC private mode prefix
                    self.ansi_param_len = 0;
                    return;
                }
                let c = ch as u32;
                if (c >= b'0' as u32 && c <= b'9' as u32) || c == b';' as u32 {
                    if self.ansi_param_len < self.ansi_params.len() {
                        self.ansi_params[self.ansi_param_len] = ch as u8;
                        self.ansi_param_len += 1;
                    }
                    return;
                }
                self.ansi_state = 0;
                self.dispatch_csi(ch);
                return;
            }
            3 => {
                // OSC sequence — collect until BEL (\x07) or ST (\x1B\\)
                if ch == '\x07' {
                    self.ansi_state = 0;
                    self.dispatch_osc();
                } else if ch == '\x1B' {
                    // Possible ST (\x1B\\) — simplified: just end OSC
                    self.ansi_state = 0;
                    self.dispatch_osc();
                } else {
                    if self.osc_buf.len() < 256 {
                        self.osc_buf.push(ch as u8);
                    }
                }
                return;
            }
            4 => {
                // DEC private mode CSI ? ... h/l
                let c = ch as u32;
                if (c >= b'0' as u32 && c <= b'9' as u32) || c == b';' as u32 {
                    if self.ansi_param_len < self.ansi_params.len() {
                        self.ansi_params[self.ansi_param_len] = ch as u8;
                        self.ansi_param_len += 1;
                    }
                    return;
                }
                self.ansi_state = 0;
                self.dispatch_dec_private(ch);
                return;
            }
            5 => {
                // Charset designation — eat one char
                self.ansi_state = 0;
                return;
            }
            _ => {}
        }

        // Normal character processing
        match ch {
            '\x1B' => {
                self.ansi_state = 1;
            }
            '\x07' => {
                // BEL — could trigger visual bell, for now ignore
            }
            '\t' => {
                // Tab: advance to next 8-column stop
                let next_tab = (self.cursor_col + 8) & !7;
                let next_tab = next_tab.min(self.cols.saturating_sub(1));
                self.cursor_col = next_tab;
            }
            '\n' => {
                let bot = self.effective_scroll_bottom() + self.scroll_offset;
                if self.cursor_row == bot {
                    // At bottom of scroll region — scroll up
                    self.scroll_region_up();
                } else {
                    self.cursor_row += 1;
                    self.ensure_line(self.cursor_row);
                    if self.cursor_row >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                    }
                }
                self.cursor_col = 0;
                if self.alt_lines.is_none() && self.lines.len() > MAX_SCROLLBACK {
                    let excess = self.lines.len() - MAX_SCROLLBACK;
                    self.lines.drain(0..excess);
                    self.cursor_row = self.cursor_row.saturating_sub(excess);
                    self.scroll_offset = self.scroll_offset.saturating_sub(excess);
                }
            }
            '\r' => {
                self.cursor_col = 0;
            }
            '\x08' => {
                if self.cursor_col > 0 {
                    self.cursor_col -= 1;
                }
            }
            _ => {
                let cw = char_width(ch);

                // Combining character: attach to previous cell
                if cw == 0 && is_combining(ch) {
                    if self.cursor_col > 0 {
                        self.ensure_line(self.cursor_row);
                        let line = &mut self.lines[self.cursor_row];
                        if self.cursor_col - 1 < line.len() {
                            let prev = &mut line[self.cursor_col - 1];
                            if prev.combining == '\0' {
                                prev.combining = ch;
                            }
                        }
                    }
                    return;
                }

                // Zero-width non-combining chars (ZWJ, ZWNJ, etc.): ignore
                if cw == 0 {
                    return;
                }

                // Wide character: check if we have room for 2 cells
                if cw == 2 && self.cursor_col + 1 >= self.cols {
                    // Not enough room on this line — wrap to next line
                    // Fill remaining cell with space
                    self.ensure_line(self.cursor_row);
                    let line = &mut self.lines[self.cursor_row];
                    let blank = Cell::blank(self.current_fg, 0);
                    while line.len() <= self.cursor_col {
                        line.push(blank);
                    }
                    line[self.cursor_col] = Cell::blank(self.current_fg, self.current_bg);
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                    self.ensure_line(self.cursor_row);
                    if self.cursor_row >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                    }
                }

                self.ensure_line(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                let blank = Cell::blank(self.current_fg, 0);
                while line.len() <= self.cursor_col {
                    line.push(blank);
                }
                line[self.cursor_col] = Cell {
                    ch,
                    fg: self.current_fg,
                    bg: self.current_bg,
                    attr: self.current_attr,
                    width: cw,
                    combining: '\0',
                    link_id: self.current_link_id,
                };
                self.cursor_col += 1;

                // Wide character: place continuation cell
                if cw == 2 {
                    let line = &mut self.lines[self.cursor_row];
                    while line.len() <= self.cursor_col {
                        line.push(blank);
                    }
                    line[self.cursor_col] = Cell {
                        ch: '\0',
                        fg: self.current_fg,
                        bg: self.current_bg,
                        attr: self.current_attr,
                        width: 0,
                        combining: '\0',
                        link_id: 0,
                    };
                    self.cursor_col += 1;
                }

                if self.cursor_col >= self.cols {
                    self.cursor_col = 0;
                    self.cursor_row += 1;
                    self.ensure_line(self.cursor_row);
                    if self.cursor_row >= self.scroll_offset + self.visible_rows {
                        self.scroll_offset = self.cursor_row - self.visible_rows + 1;
                    }
                }
            }
        }
    }

    /// Parse CSI numeric params from the raw byte buffer.
    fn parse_csi_params(&self) -> (Vec<u32>, usize) {
        let params = &self.ansi_params[..self.ansi_param_len];
        let mut nums = Vec::new();
        let mut val = 0u32;
        let mut has_val = false;
        for &b in params {
            if b == b';' {
                nums.push(val);
                val = 0;
                has_val = false;
            } else if b >= b'0' && b <= b'9' {
                val = val * 10 + (b - b'0') as u32;
                has_val = true;
            }
        }
        if has_val || nums.is_empty() {
            nums.push(val);
        }
        let count = nums.len();
        (nums, count)
    }

    /// Dispatch a CSI escape sequence command.
    fn dispatch_csi(&mut self, cmd: char) {
        let (nums, num_count) = self.parse_csi_params();

        // Theme-aware ANSI color palettes
        let colors = app().active_profile.ansi_colors();
        let bright = app().active_profile.ansi_bright();

        match cmd {
            'J' => {
                let mode = if num_count > 0 { nums[0] } else { 0 };
                match mode {
                    0 => {
                        // Erase from cursor to end of screen
                        self.ensure_line(self.cursor_row);
                        let line = &mut self.lines[self.cursor_row];
                        line.truncate(self.cursor_col);
                        let last = self.lines.len();
                        if self.cursor_row + 1 < last {
                            for r in (self.cursor_row + 1)..last {
                                self.lines[r].clear();
                            }
                        }
                    }
                    1 => {
                        // Erase from start to cursor
                        let blank = Cell::blank(self.current_fg, 0);
                        for r in 0..self.cursor_row {
                            if r < self.lines.len() { self.lines[r].clear(); }
                        }
                        self.ensure_line(self.cursor_row);
                        let line = &mut self.lines[self.cursor_row];
                        for i in 0..=self.cursor_col.min(line.len().saturating_sub(1)) {
                            if i < line.len() { line[i] = blank; }
                        }
                    }
                    2 | 3 => {
                        if self.alt_lines.is_some() {
                            // In alt screen, just clear visible area
                            for line in &mut self.lines {
                                line.clear();
                            }
                            self.cursor_row = self.scroll_offset;
                            self.cursor_col = 0;
                        } else {
                            self.lines.clear();
                            self.lines.push(Vec::new());
                            self.cursor_row = 0;
                            self.cursor_col = 0;
                            self.scroll_offset = 0;
                        }
                    }
                    _ => {}
                }
            }
            'H' | 'f' => {
                let row = if num_count > 0 && nums[0] > 0 { nums[0] as usize - 1 } else { 0 };
                let col = if num_count > 1 && nums[1] > 0 { nums[1] as usize - 1 } else { 0 };
                self.cursor_row = self.scroll_offset + row;
                self.cursor_col = col.min(self.cols.saturating_sub(1));
                self.ensure_line(self.cursor_row);
            }
            'K' => {
                let mode = if num_count > 0 { nums[0] } else { 0 };
                self.ensure_line(self.cursor_row);
                let blank = Cell::blank(self.current_fg, 0);
                let line = &mut self.lines[self.cursor_row];
                match mode {
                    0 => { line.truncate(self.cursor_col); }
                    1 => {
                        for i in 0..=self.cursor_col.min(line.len().saturating_sub(1)) {
                            if i < line.len() { line[i] = blank; }
                        }
                    }
                    2 => { line.clear(); }
                    _ => {}
                }
            }
            'A' => {
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_row += n;
                self.ensure_line(self.cursor_row);
            }
            'C' => {
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_col = (self.cursor_col + n).min(self.cols.saturating_sub(1));
            }
            'D' => {
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            'E' => {
                // Cursor Next Line
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_row += n;
                self.cursor_col = 0;
                self.ensure_line(self.cursor_row);
            }
            'F' => {
                // Cursor Previous Line
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.cursor_col = 0;
            }
            'G' => {
                // Cursor Horizontal Absolute
                let col = if num_count > 0 && nums[0] > 0 { nums[0] as usize - 1 } else { 0 };
                self.cursor_col = col.min(self.cols.saturating_sub(1));
            }
            'd' => {
                // Vertical Line Position Absolute (VPA)
                let row = if num_count > 0 && nums[0] > 0 { nums[0] as usize - 1 } else { 0 };
                self.cursor_row = self.scroll_offset + row;
                self.ensure_line(self.cursor_row);
            }
            'L' => {
                // Insert Lines
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                let bot = self.effective_scroll_bottom() + self.scroll_offset;
                for _ in 0..n {
                    if bot < self.lines.len() {
                        self.lines.remove(bot);
                    }
                    if self.cursor_row <= self.lines.len() {
                        self.lines.insert(self.cursor_row, Vec::new());
                    }
                }
                self.cursor_col = 0;
            }
            'M' => {
                // Delete Lines
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                let bot = self.effective_scroll_bottom() + self.scroll_offset;
                for _ in 0..n {
                    if self.cursor_row < self.lines.len() {
                        self.lines.remove(self.cursor_row);
                    }
                    while self.lines.len() <= bot {
                        self.lines.push(Vec::new());
                    }
                    self.lines.insert(bot, Vec::new());
                }
                self.cursor_col = 0;
            }
            '@' => {
                // Insert Characters (ICH)
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.ensure_line(self.cursor_row);
                let blank = Cell::blank(self.current_fg, 0);
                let line = &mut self.lines[self.cursor_row];
                for _ in 0..n {
                    if self.cursor_col < line.len() {
                        line.insert(self.cursor_col, blank);
                    }
                }
                line.truncate(self.cols);
            }
            'P' => {
                // Delete Characters (DCH)
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.ensure_line(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                for _ in 0..n {
                    if self.cursor_col < line.len() {
                        line.remove(self.cursor_col);
                    }
                }
            }
            'X' => {
                // Erase Characters (ECH)
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                self.ensure_line(self.cursor_row);
                let blank = Cell::blank(self.current_fg, 0);
                let line = &mut self.lines[self.cursor_row];
                for i in 0..n {
                    let col = self.cursor_col + i;
                    if col < line.len() {
                        line[col] = blank;
                    }
                }
            }
            'S' => {
                // Scroll Up
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                for _ in 0..n { self.scroll_region_up(); }
            }
            'T' => {
                // Scroll Down
                let n = if num_count > 0 && nums[0] > 0 { nums[0] as usize } else { 1 };
                for _ in 0..n { self.scroll_region_down(); }
            }
            'r' => {
                // DECSTBM — Set Scrolling Region
                let top = if num_count > 0 && nums[0] > 0 { nums[0] as usize - 1 } else { 0 };
                let bot = if num_count > 1 && nums[1] > 0 { nums[1] as usize - 1 } else { 0 };
                self.scroll_top = top;
                self.scroll_bottom = bot;
                // Move cursor to home
                self.cursor_row = self.scroll_offset;
                self.cursor_col = 0;
            }
            'm' => {
                // SGR (Select Graphic Rendition) — full color + attribute support
                if num_count == 0 || (num_count == 1 && nums[0] == 0) {
                    self.current_fg = COLOR_FG;
                    self.current_bg = 0;
                    self.current_attr = 0;
                } else {
                    let mut idx = 0;
                    while idx < num_count {
                        match nums[idx] {
                            0 => { self.current_fg = COLOR_FG; self.current_bg = 0; self.current_attr = 0; }
                            1 => { self.current_attr |= ATTR_BOLD; }
                            3 => { self.current_attr |= ATTR_ITALIC; }
                            4 => { self.current_attr |= ATTR_UNDERLINE; }
                            5 => { self.current_attr |= ATTR_BLINK; }
                            7 => { self.current_attr |= ATTR_REVERSE; }
                            9 => { self.current_attr |= ATTR_STRIKETHROUGH; }
                            22 => { self.current_attr &= !ATTR_BOLD; }
                            23 => { self.current_attr &= !ATTR_ITALIC; }
                            24 => { self.current_attr &= !ATTR_UNDERLINE; }
                            25 => { self.current_attr &= !ATTR_BLINK; }
                            27 => { self.current_attr &= !ATTR_REVERSE; }
                            29 => { self.current_attr &= !ATTR_STRIKETHROUGH; }
                            // Foreground colors
                            30..=37 => {
                                let c = (nums[idx] - 30) as usize;
                                self.current_fg = if self.current_attr & ATTR_BOLD != 0 { bright[c] } else { colors[c] };
                            }
                            38 => {
                                // Extended foreground: 38;5;N (256-color) or 38;2;R;G;B (true-color)
                                if idx + 1 < num_count {
                                    if nums[idx + 1] == 5 && idx + 2 < num_count {
                                        self.current_fg = color_256(nums[idx + 2], &colors, &bright);
                                        idx += 2;
                                    } else if nums[idx + 1] == 2 && idx + 4 < num_count {
                                        let r = nums[idx + 2].min(255) as u8;
                                        let g = nums[idx + 3].min(255) as u8;
                                        let b = nums[idx + 4].min(255) as u8;
                                        self.current_fg = 0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
                                        idx += 4;
                                    }
                                }
                            }
                            39 => { self.current_fg = COLOR_FG; }
                            // Background colors
                            40..=47 => {
                                let c = (nums[idx] - 40) as usize;
                                self.current_bg = colors[c];
                            }
                            48 => {
                                // Extended background: 48;5;N or 48;2;R;G;B
                                if idx + 1 < num_count {
                                    if nums[idx + 1] == 5 && idx + 2 < num_count {
                                        self.current_bg = color_256(nums[idx + 2], &colors, &bright);
                                        idx += 2;
                                    } else if nums[idx + 1] == 2 && idx + 4 < num_count {
                                        let r = nums[idx + 2].min(255) as u8;
                                        let g = nums[idx + 3].min(255) as u8;
                                        let b = nums[idx + 4].min(255) as u8;
                                        self.current_bg = 0xFF000000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
                                        idx += 4;
                                    }
                                }
                            }
                            49 => { self.current_bg = 0; }
                            // Bright foreground
                            90..=97 => {
                                self.current_fg = bright[(nums[idx] - 90) as usize];
                            }
                            // Bright background
                            100..=107 => {
                                self.current_bg = bright[(nums[idx] - 100) as usize];
                            }
                            _ => {}
                        }
                        idx += 1;
                    }
                }
            }
            'n' => {
                // Device Status Report — we don't respond, but avoid crashing
            }
            'c' => {
                // Send Device Attributes — ignore
            }
            'h' | 'l' => {
                // Set/Reset Mode (non-DEC) — ignore most
            }
            _ => {} // Unknown CSI command, ignore
        }
    }

    /// Dispatch DEC private mode sequences (CSI ? ... h/l).
    fn dispatch_dec_private(&mut self, cmd: char) {
        let (nums, num_count) = self.parse_csi_params();
        let set = cmd == 'h'; // h=set, l=reset

        for i in 0..num_count {
            match nums[i] {
                25 => {
                    // DECTCEM — cursor visibility
                    self.cursor_visible = set;
                }
                1049 => {
                    // Alternate screen buffer + save/restore cursor
                    if set { self.enter_alt_screen(); } else { self.leave_alt_screen(); }
                }
                1047 | 47 => {
                    // Alternate screen buffer (without cursor save)
                    if set { self.enter_alt_screen(); } else { self.leave_alt_screen(); }
                }
                1000 => {
                    // Mouse tracking (basic)
                    self.mouse_mode = if set { 1000 } else { 0 };
                }
                1002 => {
                    // Mouse tracking (button events)
                    self.mouse_mode = if set { 1002 } else { 0 };
                }
                1003 => {
                    // Mouse tracking (all events)
                    self.mouse_mode = if set { 1003 } else { 0 };
                }
                1006 => {
                    // SGR mouse mode
                    self.mouse_sgr = set;
                }
                2004 => {
                    // Bracketed paste mode
                    self.bracketed_paste = set;
                }
                7 => {
                    // Auto-wrap — always on, ignore
                }
                1 => {
                    // DECCKM — cursor key mode, ignore for now
                }
                12 => {
                    // Blinking cursor — ignore
                }
                _ => {} // Unknown DEC private mode
            }
        }
    }

    /// Dispatch OSC (Operating System Command) sequence.
    fn dispatch_osc(&mut self) {
        // OSC format: "code;data"
        let buf = &self.osc_buf;
        if buf.is_empty() { return; }
        // Find the semicolon separator
        let mut sep = 0;
        for i in 0..buf.len() {
            if buf[i] == b';' {
                sep = i;
                break;
            }
        }
        if sep == 0 { return; }
        // Parse command number
        let mut cmd_num = 0u32;
        for i in 0..sep {
            if buf[i] >= b'0' && buf[i] <= b'9' {
                cmd_num = cmd_num * 10 + (buf[i] - b'0') as u32;
            }
        }
        let data = &buf[sep + 1..];
        match cmd_num {
            0 | 1 | 2 => {
                // Set window title
                if let Ok(title) = core::str::from_utf8(data) {
                    let a = app();
                    if a.active_tab < a.tabs.len() {
                        a.tabs[a.active_tab].title = String::from(title);
                    }
                    // Also set the window title
                    anyui::Control::from_id(a.win_id).set_text(title);
                }
            }
            8 => {
                // OSC 8 — Hyperlinks: \x1B]8;params;url\x07
                // data format: "params;url"
                // If url is empty, close the hyperlink
                if let Ok(data_str) = core::str::from_utf8(data) {
                    // Find the semicolon between params and url
                    if let Some(semi) = data_str.find(';') {
                        let url = &data_str[semi + 1..];
                        if url.is_empty() {
                            // Close hyperlink
                            self.current_link_id = 0;
                        } else {
                            // Open hyperlink — add URL to table
                            if self.hyperlinks.is_empty() {
                                self.hyperlinks.push(String::new()); // index 0 = no link
                            }
                            let id = self.hyperlinks.len() as u16;
                            self.hyperlinks.push(String::from(url));
                            self.current_link_id = id;
                        }
                    } else if data_str.is_empty() {
                        self.current_link_id = 0;
                    }
                }
            }
            _ => {} // Unknown OSC
        }
    }

    /// Get the URL for a given link_id, if any.
    fn get_hyperlink(&self, link_id: u16) -> Option<&str> {
        if link_id == 0 { return None; }
        self.hyperlinks.get(link_id as usize).map(|s| s.as_str())
    }

    fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            if self.cursor_row < self.lines.len() {
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col < line.len() {
                    line.remove(self.cursor_col);
                }
            }
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    fn scroll_down(&mut self, lines: usize) {
        let max_offset = self.lines.len().saturating_sub(self.visible_rows);
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
    }

    /// Clamp scroll_offset so cursor is visible and no blank space at bottom.
    fn clamp_scroll(&mut self) {
        // Ensure cursor is within visible window
        if self.cursor_row >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = self.cursor_row.saturating_sub(self.visible_rows - 1);
        }
        // Don't scroll past content
        let max_offset = self.lines.len().saturating_sub(self.visible_rows);
        if self.scroll_offset > max_offset {
            self.scroll_offset = max_offset;
        }
    }
}

// ─── Foreground process tracker ──────────────────────────────────────────────

struct ForegroundProcess {
    tid: u32,
    pipe_id: u32,
    /// Stdin pipe for forwarding keyboard input to the child process.
    /// 0 means no stdin pipe.
    stdin_pipe: u32,
    /// Intermediate pipe IDs from a pipeline (cmd1 | cmd2 | cmd3).
    /// Closed when the pipeline exits.
    extra_pipes: Vec<u32>,
    /// Output redirect: if set, pipe output goes to file instead of terminal.
    redirect: Option<Redirect>,
    /// The command line that spawned this process (for job control display).
    command: String,
}

// ─── Profile ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Profile {
    name: String,
    color_bg: u32,
    color_fg: u32,
    color_prompt: u32,
    color_title: u32,
    color_dim: u32,
    color_select_bg: u32,
    color_select_fg: u32,
    win_w: u32,
    win_h: u32,
    scrollback: usize,
    is_default: bool,
    saved_tabs: Vec<SavedTab>,
    font_size: u16,
    bg_alpha: u8,
    theme_name: String,
    cursor_blink_ms: u32, // 0 = no blink, default 500ms
}

impl Profile {
    fn default_profile() -> Self {
        Self {
            name: String::from("Default"),
            color_bg: COLOR_BG,
            color_fg: COLOR_FG,
            color_prompt: COLOR_PROMPT,
            color_title: COLOR_TITLE,
            color_dim: COLOR_DIM,
            color_select_bg: COLOR_SELECT_BG,
            color_select_fg: COLOR_SELECT_FG,
            win_w: 640,
            win_h: 400,
            scrollback: MAX_SCROLLBACK,
            is_default: true,
            saved_tabs: Vec::new(),
            font_size: DEFAULT_FONT_SIZE,
            bg_alpha: 255,
            theme_name: String::from("Default"),
            cursor_blink_ms: 500,
        }
    }

    /// Get the ANSI standard 8 colors from the current theme.
    fn ansi_colors(&self) -> [u32; 8] {
        for theme in THEMES {
            if theme.name == self.theme_name.as_str() {
                return theme.ansi_colors;
            }
        }
        THEMES[0].ansi_colors
    }

    /// Get the ANSI bright 8 colors from the current theme.
    fn ansi_bright(&self) -> [u32; 8] {
        for theme in THEMES {
            if theme.name == self.theme_name.as_str() {
                return theme.ansi_bright;
            }
        }
        THEMES[0].ansi_bright
    }

    fn to_json(&self) -> anyos_std::json::Value {
        use anyos_std::json::Value;
        let mut obj = Value::new_object();
        obj.set("name", Value::from(self.name.as_str()));
        obj.set("color_bg", Value::from(format!("{:08X}", self.color_bg).as_str()));
        obj.set("color_fg", Value::from(format!("{:08X}", self.color_fg).as_str()));
        obj.set("color_prompt", Value::from(format!("{:08X}", self.color_prompt).as_str()));
        obj.set("color_title", Value::from(format!("{:08X}", self.color_title).as_str()));
        obj.set("color_dim", Value::from(format!("{:08X}", self.color_dim).as_str()));
        obj.set("color_select_bg", Value::from(format!("{:08X}", self.color_select_bg).as_str()));
        obj.set("color_select_fg", Value::from(format!("{:08X}", self.color_select_fg).as_str()));
        obj.set("win_w", Value::from(self.win_w as i64));
        obj.set("win_h", Value::from(self.win_h as i64));
        obj.set("scrollback", Value::from(self.scrollback as i64));
        obj.set("is_default", Value::from(self.is_default));
        obj.set("font_size", Value::from(self.font_size as i64));
        obj.set("bg_alpha", Value::from(self.bg_alpha as i64));
        obj.set("cursor_blink_ms", Value::from(self.cursor_blink_ms as i64));
        obj.set("theme", Value::from(self.theme_name.as_str()));
        if !self.saved_tabs.is_empty() {
            let mut tabs_arr = Value::new_array();
            for t in &self.saved_tabs {
                tabs_arr.push(t.to_json());
            }
            obj.set("tabs", tabs_arr);
        }
        obj
    }

    fn from_json(val: &anyos_std::json::Value) -> Self {
        let mut p = Self::default_profile();
        if let Some(s) = val["name"].as_str() { p.name = String::from(s); }
        if let Some(s) = val["color_bg"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_bg = c; } }
        if let Some(s) = val["color_fg"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_fg = c; } }
        if let Some(s) = val["color_prompt"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_prompt = c; } }
        if let Some(s) = val["color_title"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_title = c; } }
        if let Some(s) = val["color_dim"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_dim = c; } }
        if let Some(s) = val["color_select_bg"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_select_bg = c; } }
        if let Some(s) = val["color_select_fg"].as_str() { if let Some(c) = parse_hex_color(s) { p.color_select_fg = c; } }
        if let Some(n) = val["win_w"].as_i64() { p.win_w = n as u32; }
        if let Some(n) = val["win_h"].as_i64() { p.win_h = n as u32; }
        if let Some(n) = val["scrollback"].as_i64() { p.scrollback = n as usize; }
        if let Some(b) = val["is_default"].as_bool() { p.is_default = b; }
        if let Some(n) = val["font_size"].as_i64() { p.font_size = n as u16; }
        if let Some(n) = val["bg_alpha"].as_i64() { p.bg_alpha = n as u8; }
        if let Some(n) = val["cursor_blink_ms"].as_i64() { p.cursor_blink_ms = n as u32; }
        if let Some(s) = val["theme"].as_str() { p.theme_name = String::from(s); }
        if let Some(arr) = val["tabs"].as_array() {
            for item in arr {
                p.saved_tabs.push(SavedTab::from_json(item));
            }
        }
        p
    }
}

fn parse_hex_color(s: &str) -> Option<u32> {
    let s = s.trim();
    let s = if s.starts_with("0x") || s.starts_with("0X") { &s[2..] } else { s };
    if s.len() != 8 { return None; }
    let mut val = 0u32;
    for b in s.bytes() {
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

fn profiles_config_path() -> String {
    let mut home_buf = [0u8; 128];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX {
        core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/")
    } else {
        "/"
    };
    format!("{}/.config/terminal/profiles.json", home)
}

fn load_profiles() -> Vec<Profile> {
    let path = profiles_config_path();
    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        return alloc::vec![Profile::default_profile()];
    }
    let mut buf = [0u8; 32768];
    let n = fs::read(fd, &mut buf);
    fs::close(fd);
    if n == 0 || n == u32::MAX {
        return alloc::vec![Profile::default_profile()];
    }
    let text = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s.trim(),
        Err(_) => return alloc::vec![Profile::default_profile()],
    };
    let val = match anyos_std::json::Value::parse(text) {
        Ok(v) => v,
        Err(_) => return alloc::vec![Profile::default_profile()],
    };
    let arr = match val["profiles"].as_array() {
        Some(a) => a,
        None => return alloc::vec![Profile::default_profile()],
    };
    let mut profiles = Vec::new();
    for item in arr {
        profiles.push(Profile::from_json(item));
    }
    if profiles.is_empty() {
        profiles.push(Profile::default_profile());
    }
    profiles
}

/// Recursively create directories (like mkdir -p).
fn mkdir_p(path: &str) {
    let mut accumulated = String::new();
    for component in path.split('/') {
        if component.is_empty() {
            accumulated.push('/');
            continue;
        }
        if !accumulated.is_empty() && !accumulated.ends_with('/') {
            accumulated.push('/');
        }
        accumulated.push_str(component);
        fs::mkdir(&accumulated);
    }
}

fn save_profiles(profiles: &[Profile]) {
    use anyos_std::json::Value;
    let path = profiles_config_path();

    // Ensure directory exists (recursive)
    if let Some(pos) = path.rfind('/') {
        mkdir_p(&path[..pos]);
    }

    let mut root = Value::new_object();
    let mut arr = Value::new_array();
    for p in profiles {
        arr.push(p.to_json());
    }
    root.set("profiles", arr);
    let json = root.to_json_string_pretty();
    let _ = fs::write_bytes(&path, json.as_bytes());
}

// ─── Scrollback Persistence (per tab/split) ─────────────────────────────────

fn recovery_file_path() -> String {
    let mut home_buf = [0u8; 128];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX {
        core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/")
    } else {
        "/"
    };
    format!("{}/.config/terminal/recovery.json", home)
}

/// Extract scrollback text from a terminal buffer (last MAX_SCROLLBACK lines).
fn extract_scrollback(buf: &TerminalBuffer) -> String {
    let mut text = String::new();
    for line in &buf.lines {
        for cell in line {
            if cell.ch != '\0' && cell.width > 0 {
                text.push(cell.ch);
            }
            if cell.combining != '\0' {
                text.push(cell.combining);
            }
        }
        // Trim trailing spaces
        while text.ends_with(' ') {
            text.pop();
        }
        text.push('\n');
    }
    // Trim trailing empty lines
    while text.ends_with("\n\n") {
        text.pop();
    }
    text
}

/// Display restored scrollback text in a terminal buffer as dim text.
fn restore_scrollback_into(buf: &mut TerminalBuffer, scrollback: &str) {
    if scrollback.is_empty() { return; }

    let saved_fg = buf.current_fg;
    buf.current_fg = 0xFF606060; // dim gray for old output

    buf.write_str("── Vorherige Sitzung ──\n");

    // Only show last visible_rows lines
    let all_lines: Vec<&str> = scrollback.lines().collect();
    let max_lines = buf.visible_rows.saturating_sub(2);
    let start = if all_lines.len() > max_lines {
        all_lines.len() - max_lines
    } else {
        0
    };

    for line in &all_lines[start..] {
        buf.write_str(line);
        buf.write_char('\n');
    }

    buf.write_str("───────────────────────\n");
    buf.current_fg = saved_fg;
}

/// Save full recovery state: all tabs with their splits, cwd, and scrollback.
fn save_recovery(a: &TerminalApp) {
    use anyos_std::json::Value;

    let path = recovery_file_path();
    let mut root = Value::new_object();
    let mut tabs_arr = Value::new_array();

    for tab in &a.tabs {
        let mut tab_obj = Value::new_object();
        tab_obj.set("title", Value::from(tab.title.as_str()));
        tab_obj.set("layout", pane_to_json_with_scrollback(&tab.root));
        tabs_arr.push(tab_obj);
    }

    root.set("tabs", tabs_arr);
    root.set("active_tab", Value::from(a.active_tab as i64));

    let json = root.to_json_string_pretty();

    // Ensure directory exists (recursive)
    if let Some(pos) = path.rfind('/') {
        mkdir_p(&path[..pos]);
    }
    let _ = fs::write_bytes(&path, json.as_bytes());
}

/// Serialize a PaneNode to JSON including scrollback text per leaf.
fn pane_to_json_with_scrollback(node: &PaneNode) -> anyos_std::json::Value {
    use anyos_std::json::Value;
    match node {
        PaneNode::Leaf { session, .. } => {
            let mut obj = Value::new_object();
            obj.set("type", Value::from("leaf"));
            obj.set("cwd", Value::from(session.shell.cwd.as_str()));
            obj.set("scrollback", Value::from(extract_scrollback(&session.buf).as_str()));
            obj
        }
        PaneNode::HSplit { left, right, ratio } => {
            let mut obj = Value::new_object();
            obj.set("type", Value::from("hsplit"));
            obj.set("ratio", Value::from((*ratio as f64 * 1000.0) as i64));
            obj.set("left", pane_to_json_with_scrollback(left));
            obj.set("right", pane_to_json_with_scrollback(right));
            obj
        }
        PaneNode::VSplit { top, bottom, ratio } => {
            let mut obj = Value::new_object();
            obj.set("type", Value::from("vsplit"));
            obj.set("ratio", Value::from((*ratio as f64 * 1000.0) as i64));
            obj.set("top", pane_to_json_with_scrollback(top));
            obj.set("bottom", pane_to_json_with_scrollback(bottom));
            obj
        }
    }
}

/// Load recovery state. Returns saved tabs if recovery file exists.
fn load_recovery() -> Option<Vec<SavedTab>> {
    let path = recovery_file_path();
    let fd = fs::open(&path, 0);
    if fd == u32::MAX { return None; }

    // Read up to 512KB recovery data
    let mut data = alloc::vec![0u8; 512 * 1024];
    let n = fs::read(fd, &mut data);
    fs::close(fd);
    if n == 0 || n == u32::MAX { return None; }

    let text = core::str::from_utf8(&data[..n as usize]).ok()?;
    let val = anyos_std::json::Value::parse(text.trim()).ok()?;
    let tabs_arr = val["tabs"].as_array()?;

    let mut tabs = Vec::new();
    for item in tabs_arr {
        tabs.push(SavedTab {
            title: String::from(item["title"].as_str().unwrap_or("Tab")),
            layout: item["layout"].clone(),
        });
    }

    // Delete recovery file after loading
    fs::unlink(&path);

    if tabs.is_empty() { None } else { Some(tabs) }
}

fn find_default_profile(profiles: &[Profile]) -> Profile {
    for p in profiles {
        if p.is_default { return p.clone(); }
    }
    profiles.first().cloned().unwrap_or_else(Profile::default_profile)
}

// ─── Session ─────────────────────────────────────────────────────────────────

/// Here-document state: collecting lines until delimiter is seen.
struct HereDocState {
    delimiter: String,
    command: String,  // the command to feed the here-doc to (e.g. "cat")
    lines: Vec<String>,
}

struct Session {
    buf: TerminalBuffer,
    shell: Shell,
    fg_proc: Option<ForegroundProcess>,
    selection: Option<Selection>,
    su_pending_user: Option<String>,
    su_password: String,
    dirty: bool,
    scrollbar_dragging: bool,
    heredoc: Option<HereDocState>,
    font_size: u16,
    cell_w: u16,
    cell_h: u16,
    color_prompt: u32,
    color_fg: u32,
}

/// Compute cell dimensions from font size by measuring the actual font metrics.
fn cell_dims_for_font(font_size: u16) -> (u16, u16) {
    let (w, h) = anyui::measure_text("W", FONT_ID_MONO as u16, font_size);
    let cw = if w > 0 { w as u16 } else { ((font_size as u32 * 8 + 6) / 13) as u16 };
    let ch = if h > 0 { h as u16 } else { ((font_size as u32 * 16 + 6) / 13) as u16 };
    (cw.max(4), ch.max(8))
}

impl Session {
    fn new(cols: usize, rows: usize, aliases: Vec<(String, String)>, font_size: u16, color_prompt: u32, color_fg: u32) -> Self {
        let (cell_w, cell_h) = cell_dims_for_font(font_size);
        let mut shell = Shell::new();
        shell.aliases = aliases;
        Session {
            buf: TerminalBuffer::new(cols, rows),
            shell,
            fg_proc: None,
            selection: None,
            su_pending_user: None,
            su_password: String::new(),
            dirty: false,
            scrollbar_dragging: false,
            heredoc: None,
            font_size,
            cell_w,
            cell_h,
            color_prompt,
            color_fg,
        }
    }

    fn set_font_size(&mut self, size: u16) {
        self.font_size = size.max(MIN_FONT_SIZE).min(MAX_FONT_SIZE);
        let (cw, ch) = cell_dims_for_font(self.font_size);
        self.cell_w = cw;
        self.cell_h = ch;
    }

    fn show_prompt(&mut self) {
        self.buf.current_fg = self.color_prompt;
        let prompt = self.shell.prompt();
        self.buf.write_str(&prompt);
        self.buf.current_fg = self.color_fg;
    }
}

// ─── Tab / Pane structures ───────────────────────────────────────────────────

const PANE_BORDER: u16 = 2;

const COLOR_PANE_BORDER: u32 = 0xFF444444;
const COLOR_PANE_ACTIVE_BORDER: u32 = 0xFF50FA7B;

type PaneId = u32;

#[derive(Clone, Copy)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

enum PaneNode {
    Leaf { id: PaneId, session: Session },
    HSplit { left: Box<PaneNode>, right: Box<PaneNode>, ratio: f32 },
    VSplit { top: Box<PaneNode>, bottom: Box<PaneNode>, ratio: f32 },
}

impl PaneNode {
    fn find_session(&self, pane_id: PaneId) -> Option<&Session> {
        match self {
            PaneNode::Leaf { id, session } => {
                if *id == pane_id { Some(session) } else { None }
            }
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                left.find_session(pane_id).or_else(|| right.find_session(pane_id))
            }
        }
    }

    fn find_session_mut(&mut self, pane_id: PaneId) -> Option<&mut Session> {
        match self {
            PaneNode::Leaf { id, session } => {
                if *id == pane_id { Some(session) } else { None }
            }
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                if let Some(s) = left.find_session_mut(pane_id) {
                    Some(s)
                } else {
                    right.find_session_mut(pane_id)
                }
            }
        }
    }

    fn for_each_session_mut(&mut self, f: impl Fn(&mut Session) + Copy) {
        match self {
            PaneNode::Leaf { session, .. } => f(session),
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                left.for_each_session_mut(f);
                right.for_each_session_mut(f);
            }
        }
    }

    fn first_pane_id(&self) -> PaneId {
        match self {
            PaneNode::Leaf { id, .. } => *id,
            PaneNode::HSplit { left, .. } => left.first_pane_id(),
            PaneNode::VSplit { top, .. } => top.first_pane_id(),
        }
    }

    /// Collect all pane IDs in this subtree.
    fn collect_pane_ids(&self, out: &mut Vec<PaneId>) {
        match self {
            PaneNode::Leaf { id, .. } => out.push(*id),
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                left.collect_pane_ids(out);
                right.collect_pane_ids(out);
            }
        }
    }

    /// Collect all sessions in this subtree.
    fn collect_sessions(&self) -> Vec<&Session> {
        let mut out = Vec::new();
        self.collect_sessions_inner(&mut out);
        out
    }

    fn collect_sessions_inner<'a>(&'a self, out: &mut Vec<&'a Session>) {
        match self {
            PaneNode::Leaf { session, .. } => out.push(session),
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                left.collect_sessions_inner(out);
                right.collect_sessions_inner(out);
            }
        }
    }

    /// Layout: assign rects to each pane. Returns Vec<(PaneId, Rect)>.
    fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut result = Vec::new();
        self.layout_inner(area, &mut result);
        result
    }

    fn layout_inner(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            PaneNode::Leaf { id, .. } => {
                out.push((*id, area));
            }
            PaneNode::HSplit { left, right, ratio } => {
                let left_w = ((area.w as f32) * ratio) as u16;
                let right_w = area.w.saturating_sub(left_w + PANE_BORDER);
                left.layout_inner(Rect { x: area.x, y: area.y, w: left_w, h: area.h }, out);
                right.layout_inner(Rect { x: area.x + left_w + PANE_BORDER, y: area.y, w: right_w, h: area.h }, out);
            }
            PaneNode::VSplit { top, bottom, ratio } => {
                let top_h = ((area.h as f32) * ratio) as u16;
                let bottom_h = area.h.saturating_sub(top_h + PANE_BORDER);
                top.layout_inner(Rect { x: area.x, y: area.y, w: area.w, h: top_h }, out);
                bottom.layout_inner(Rect { x: area.x, y: area.y + top_h + PANE_BORDER, w: area.w, h: bottom_h }, out);
            }
        }
    }

    /// Find which pane contains a pixel coordinate.
    fn pane_at_point(&self, area: Rect, px: u16, py: u16) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, .. } => {
                if px >= area.x && px < area.x + area.w && py >= area.y && py < area.y + area.h {
                    Some(*id)
                } else {
                    None
                }
            }
            PaneNode::HSplit { left, right, ratio } => {
                let left_w = ((area.w as f32) * ratio) as u16;
                let right_w = area.w.saturating_sub(left_w + PANE_BORDER);
                let left_area = Rect { x: area.x, y: area.y, w: left_w, h: area.h };
                let right_area = Rect { x: area.x + left_w + PANE_BORDER, y: area.y, w: right_w, h: area.h };
                left.pane_at_point(left_area, px, py).or_else(|| right.pane_at_point(right_area, px, py))
            }
            PaneNode::VSplit { top, bottom, ratio } => {
                let top_h = ((area.h as f32) * ratio) as u16;
                let bottom_h = area.h.saturating_sub(top_h + PANE_BORDER);
                let top_area = Rect { x: area.x, y: area.y, w: area.w, h: top_h };
                let bottom_area = Rect { x: area.x, y: area.y + top_h + PANE_BORDER, w: area.w, h: bottom_h };
                top.pane_at_point(top_area, px, py).or_else(|| bottom.pane_at_point(bottom_area, px, py))
            }
        }
    }
}

struct Tab {
    root: PaneNode,
    active_pane_id: PaneId,
    title: String,
}

// ─── Layout serialization ────────────────────────────────────────────────────

/// Serializable description of a pane layout (without live session data).
/// Used to save/restore tab layouts in profiles.

impl PaneNode {
    /// Serialize the pane tree structure to JSON (layout only, no session data).
    fn layout_to_json(&self) -> anyos_std::json::Value {
        use anyos_std::json::Value;
        match self {
            PaneNode::Leaf { session, .. } => {
                let mut obj = Value::new_object();
                obj.set("type", Value::from("leaf"));
                obj.set("cwd", Value::from(session.shell.cwd.as_str()));
                obj
            }
            PaneNode::HSplit { left, right, ratio } => {
                let mut obj = Value::new_object();
                obj.set("type", Value::from("hsplit"));
                obj.set("ratio", Value::from((*ratio as f64 * 1000.0) as i64));
                obj.set("left", left.layout_to_json());
                obj.set("right", right.layout_to_json());
                obj
            }
            PaneNode::VSplit { top, bottom, ratio } => {
                let mut obj = Value::new_object();
                obj.set("type", Value::from("vsplit"));
                obj.set("ratio", Value::from((*ratio as f64 * 1000.0) as i64));
                obj.set("top", top.layout_to_json());
                obj.set("bottom", bottom.layout_to_json());
                obj
            }
        }
    }
}

/// Saved tab description (title + pane layout tree).
#[derive(Clone)]
struct SavedTab {
    title: String,
    layout: anyos_std::json::Value,
}

impl SavedTab {
    fn to_json(&self) -> anyos_std::json::Value {
        use anyos_std::json::Value;
        let mut obj = Value::new_object();
        obj.set("title", Value::from(self.title.as_str()));
        obj.set("layout", self.layout.clone());
        obj
    }

    fn from_json(val: &anyos_std::json::Value) -> Self {
        Self {
            title: String::from(val["title"].as_str().unwrap_or("Tab")),
            layout: val["layout"].clone(),
        }
    }
}

/// Recreate a PaneNode tree from a saved layout JSON, allocating fresh sessions.
/// Profile defaults needed during tree rebuild (before APP is initialized).
struct SessionDefaults {
    font_size: u16,
    color_prompt: u32,
    color_fg: u32,
}

fn rebuild_pane_tree(
    layout: &anyos_std::json::Value,
    next_id: &mut PaneId,
    cols: usize,
    rows: usize,
    aliases: &[(String, String)],
    defaults: &SessionDefaults,
) -> PaneNode {
    let typ = layout["type"].as_str().unwrap_or("leaf");
    match typ {
        "hsplit" => {
            let ratio = layout["ratio"].as_i64().unwrap_or(500) as f32 / 1000.0;
            let left_cols = ((cols as f32) * ratio) as usize;
            let right_cols = cols.saturating_sub(left_cols);
            let left = rebuild_pane_tree(&layout["left"], next_id, left_cols.max(1), rows, aliases, defaults);
            let right = rebuild_pane_tree(&layout["right"], next_id, right_cols.max(1), rows, aliases, defaults);
            PaneNode::HSplit { left: Box::new(left), right: Box::new(right), ratio }
        }
        "vsplit" => {
            let ratio = layout["ratio"].as_i64().unwrap_or(500) as f32 / 1000.0;
            let top_rows = ((rows as f32) * ratio) as usize;
            let bot_rows = rows.saturating_sub(top_rows);
            let top = rebuild_pane_tree(&layout["top"], next_id, cols, top_rows.max(1), aliases, defaults);
            let bottom = rebuild_pane_tree(&layout["bottom"], next_id, cols, bot_rows.max(1), aliases, defaults);
            PaneNode::VSplit { top: Box::new(top), bottom: Box::new(bottom), ratio }
        }
        _ => {
            // leaf
            let id = *next_id;
            *next_id += 1;
            let aliases_vec: Vec<(String, String)> = aliases.iter().map(|(a, b)| (a.clone(), b.clone())).collect();
            let mut session = Session::new(cols.max(1), rows.max(1), aliases_vec, defaults.font_size, defaults.color_prompt, defaults.color_fg);
            // Restore cwd if saved
            if let Some(cwd) = layout["cwd"].as_str() {
                if !cwd.is_empty() {
                    session.shell.cwd = String::from(cwd);
                }
            }
            // Restore scrollback if saved
            if let Some(scrollback) = layout["scrollback"].as_str() {
                restore_scrollback_into(&mut session.buf, scrollback);
            }
            session.show_prompt();
            PaneNode::Leaf { id, session }
        }
    }
}

// ─── Simple Regex Engine ─────────────────────────────────────────────────────

/// Match a single pattern "atom" at the given position.
/// Returns how many bytes were consumed, or None if no match.
fn regex_atom_match(pattern: &[u8], pi: usize, text: &[u8], ti: usize) -> Option<usize> {
    if pi >= pattern.len() { return Some(0); }
    if ti >= text.len() {
        if pattern[pi] == b'$' { return Some(0); }
        return None;
    }
    let ch = text[ti];
    match pattern[pi] {
        b'.' => Some(1),
        b'\\' if pi + 1 < pattern.len() => {
            match pattern[pi + 1] {
                b'd' => if ch >= b'0' && ch <= b'9' { Some(1) } else { None },
                b'w' => if ch.is_ascii_alphanumeric() || ch == b'_' { Some(1) } else { None },
                b's' => if ch == b' ' || ch == b'\t' { Some(1) } else { None },
                b'D' => if ch < b'0' || ch > b'9' { Some(1) } else { None },
                b'W' => if !ch.is_ascii_alphanumeric() && ch != b'_' { Some(1) } else { None },
                b'S' => if ch != b' ' && ch != b'\t' { Some(1) } else { None },
                other => if ch == other { Some(1) } else { None },
            }
        }
        b'[' => {
            // Character class [abc] or [a-z] or [^abc]
            let mut j = pi + 1;
            let negate = j < pattern.len() && pattern[j] == b'^';
            if negate { j += 1; }
            let mut matched = false;
            while j < pattern.len() && pattern[j] != b']' {
                if j + 2 < pattern.len() && pattern[j + 1] == b'-' {
                    if ch >= pattern[j] && ch <= pattern[j + 2] { matched = true; }
                    j += 3;
                } else {
                    if ch == pattern[j] { matched = true; }
                    j += 1;
                }
            }
            if negate { matched = !matched; }
            if matched { Some(1) } else { None }
        }
        literal => {
            let lc = if literal >= b'A' && literal <= b'Z' { literal + 32 } else { literal };
            if ch == lc { Some(1) } else { None }
        }
    }
}

/// Get the size of a pattern atom (how many bytes it occupies in the pattern).
fn regex_atom_size(pattern: &[u8], pi: usize) -> usize {
    if pi >= pattern.len() { return 0; }
    match pattern[pi] {
        b'\\' => 2,
        b'[' => {
            let mut j = pi + 1;
            while j < pattern.len() && pattern[j] != b']' { j += 1; }
            j + 1 - pi
        }
        _ => 1,
    }
}

/// Try to match a simple regex pattern starting at position `start` in `text`.
/// Returns Some(match_length) on success.
fn simple_regex_match(pattern: &str, text: &[u8], start: usize) -> Option<usize> {
    let pat = pattern.as_bytes();
    let mut pi = 0;
    let mut ti = start;

    // Handle ^ anchor
    if pi < pat.len() && pat[pi] == b'^' {
        if start != 0 { return None; }
        pi += 1;
    }

    while pi < pat.len() {
        if pat[pi] == b'$' {
            if ti == text.len() { return Some(ti - start); }
            return None;
        }

        let atom_size = regex_atom_size(pat, pi);
        let next_pi = pi + atom_size;

        // Check for quantifier
        if next_pi < pat.len() {
            match pat[next_pi] {
                b'*' => {
                    // 0 or more
                    let mut count = 0;
                    while regex_atom_match(pat, pi, text, ti + count).is_some() {
                        count += 1;
                    }
                    // Try from most to least (greedy)
                    while count > 0 {
                        if let Some(rest) = simple_regex_match_inner(pat, next_pi + 1, text, ti + count) {
                            return Some(ti + count + rest - start);
                        }
                        count -= 1;
                    }
                    // Try with 0 matches
                    if let Some(rest) = simple_regex_match_inner(pat, next_pi + 1, text, ti) {
                        return Some(ti + rest - start);
                    }
                    return None;
                }
                b'+' => {
                    // 1 or more
                    let mut count = 0;
                    while regex_atom_match(pat, pi, text, ti + count).is_some() {
                        count += 1;
                    }
                    if count == 0 { return None; }
                    while count > 0 {
                        if let Some(rest) = simple_regex_match_inner(pat, next_pi + 1, text, ti + count) {
                            return Some(ti + count + rest - start);
                        }
                        count -= 1;
                    }
                    return None;
                }
                b'?' => {
                    // 0 or 1
                    if let Some(_) = regex_atom_match(pat, pi, text, ti) {
                        if let Some(rest) = simple_regex_match_inner(pat, next_pi + 1, text, ti + 1) {
                            return Some(ti + 1 + rest - start);
                        }
                    }
                    if let Some(rest) = simple_regex_match_inner(pat, next_pi + 1, text, ti) {
                        return Some(ti + rest - start);
                    }
                    return None;
                }
                _ => {}
            }
        }

        // No quantifier — must match exactly once
        if let Some(consumed) = regex_atom_match(pat, pi, text, ti) {
            ti += consumed;
            pi = next_pi;
        } else {
            return None;
        }
    }

    Some(ti - start)
}

fn simple_regex_match_inner(pat: &[u8], pi: usize, text: &[u8], ti: usize) -> Option<usize> {
    if pi >= pat.len() { return Some(0); }
    let remaining = core::str::from_utf8(&pat[pi..]).unwrap_or("");
    simple_regex_match(remaining, text, ti).map(|len| len)
}

// ─── Search State ────────────────────────────────────────────────────────────

struct SearchState {
    active: bool,
    query: String,
    matches: Vec<(usize, usize, usize)>, // (line_idx, col_start, col_end)
    current_match: usize,
    regex_mode: bool,
}

impl SearchState {
    fn new() -> Self {
        Self { active: false, query: String::new(), matches: Vec::new(), current_match: 0, regex_mode: false }
    }

    fn search(&mut self, buf: &TerminalBuffer) {
        self.matches.clear();
        self.current_match = 0;
        if self.query.is_empty() { return; }

        if self.regex_mode {
            self.search_regex(buf);
        } else {
            self.search_literal(buf);
        }
    }

    fn search_literal(&mut self, buf: &TerminalBuffer) {
        let q = self.query.as_bytes();
        for (line_idx, line) in buf.lines.iter().enumerate() {
            let line_len = line.len();
            if line_len < q.len() { continue; }
            for col in 0..=line_len - q.len() {
                let mut found = true;
                for (i, &qb) in q.iter().enumerate() {
                    let ch = line[col + i].ch as u8;
                    let a = if qb >= b'A' && qb <= b'Z' { qb + 32 } else { qb };
                    let b = if ch >= b'A' && ch <= b'Z' { ch + 32 } else { ch };
                    if a != b { found = false; break; }
                }
                if found {
                    self.matches.push((line_idx, col, col + q.len()));
                }
            }
        }
    }

    /// Simple regex search supporting: . (any), \d (digit), \w (word), \s (space),
    /// * (0+ prev), + (1+ prev), ? (0-1 prev), [...] (char class), ^ (start), $ (end)
    fn search_regex(&mut self, buf: &TerminalBuffer) {
        let pattern = &self.query;
        for (line_idx, line) in buf.lines.iter().enumerate() {
            let line_str: String = line.iter().map(|c| c.ch).collect();
            let line_lower: String = line_str.chars().map(|c| {
                if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }
            }).collect();
            // Try to match at each position
            let bytes = line_lower.as_bytes();
            for col in 0..bytes.len() {
                if let Some(match_len) = simple_regex_match(pattern, bytes, col) {
                    if match_len > 0 {
                        self.matches.push((line_idx, col, col + match_len));
                    }
                }
            }
        }
    }

    fn next_match(&mut self) {
        if !self.matches.is_empty() {
            self.current_match = (self.current_match + 1) % self.matches.len();
        }
    }

    fn prev_match(&mut self) {
        if !self.matches.is_empty() {
            if self.current_match == 0 {
                self.current_match = self.matches.len() - 1;
            } else {
                self.current_match -= 1;
            }
        }
    }
}

// ─── Reverse Search State ────────────────────────────────────────────────────

struct ReverseSearchState {
    active: bool,
    query: String,
    result: Option<String>,
    result_index: Option<usize>,
}

impl ReverseSearchState {
    fn new() -> Self {
        Self { active: false, query: String::new(), result: None, result_index: None }
    }

    fn search(&mut self, history: &[String]) {
        self.result = None;
        self.result_index = None;
        if self.query.is_empty() { return; }
        let q_lower: String = self.query.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
        for (i, entry) in history.iter().enumerate().rev() {
            let entry_lower: String = entry.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
            if entry_lower.contains(q_lower.as_str()) {
                self.result = Some(entry.clone());
                self.result_index = Some(i);
                return;
            }
        }
    }
}

struct TerminalApp {
    win_id: u32,
    canvas_w: u32,
    canvas_h: u32,
    tabs: Vec<Tab>,
    active_tab: usize,
    next_pane_id: PaneId,
    profiles: Vec<Profile>,
    active_profile: Profile,
    search: SearchState,
    reverse_search: ReverseSearchState,
    bg_alpha: u8,           // 0-255 (255 = fully opaque)
    theme_name: String,
    cursor_blink_on: bool,      // current blink phase (true = visible)
    cursor_blink_elapsed: u32,  // ms since last toggle
    autosave_counter: u32,      // counts timer ticks for periodic scrollback save
}

impl TerminalApp {
    fn content_area(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            w: self.canvas_w as u16,
            h: self.canvas_h as u16,
        }
    }

    fn content_cols_rows(&self) -> (usize, usize) {
        let area = self.content_area();
        let (cw, ch) = self.active_cell_dims();
        let usable_w = area.w.saturating_sub(SCROLLBAR_WIDTH as u16);
        let cols = (usable_w.saturating_sub(TEXT_PAD * 2) / cw) as usize;
        let rows = (area.h.saturating_sub(TEXT_PAD * 2) / ch) as usize;
        (cols.max(1), rows.max(1))
    }

    /// Get font_size/cell_w/cell_h of the active pane's session (fallback to profile default).
    fn active_cell_dims(&self) -> (u16, u16) {
        let tab = &self.tabs[self.active_tab];
        if let Some(sess) = tab.root.find_session(tab.active_pane_id) {
            (sess.cell_w, sess.cell_h)
        } else {
            cell_dims_for_font(self.active_profile.font_size)
        }
    }

    fn active_font_size(&self) -> u16 {
        let tab = &self.tabs[self.active_tab];
        if let Some(sess) = tab.root.find_session(tab.active_pane_id) {
            sess.font_size
        } else {
            self.active_profile.font_size
        }
    }

    fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    fn active_session(&self) -> Option<&Session> {
        let tab = self.active_tab();
        tab.root.find_session(tab.active_pane_id)
    }

    fn active_session_mut(&mut self) -> Option<&mut Session> {
        let pane_id = self.tabs[self.active_tab].active_pane_id;
        self.tabs[self.active_tab].root.find_session_mut(pane_id)
    }

    fn alloc_pane_id(&mut self) -> PaneId {
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        id
    }

    fn new_session(&self, cols: usize, rows: usize) -> Session {
        let aliases = load_dotenv();
        Session::new(cols, rows, aliases, self.active_profile.font_size, self.active_profile.color_prompt, self.active_profile.color_fg)
    }

    /// Snapshot current tabs into SavedTab list (for profile saving).
    fn snapshot_tabs(&self) -> Vec<SavedTab> {
        let mut saved = Vec::new();
        for tab in &self.tabs {
            saved.push(SavedTab {
                title: tab.title.clone(),
                layout: tab.root.layout_to_json(),
            });
        }
        saved
    }

    /// Restore tabs from a profile's saved_tabs. Replaces current tabs.
    fn restore_tabs(&mut self, saved_tabs: &[SavedTab]) {
        if saved_tabs.is_empty() { return; }
        let aliases = load_dotenv();
        let alias_pairs: Vec<(String, String)> = aliases;
        let (cols, rows) = self.content_cols_rows();

        let mut new_tabs = Vec::new();
        for st in saved_tabs {
            let mut next_id = self.next_pane_id;
            let defaults = SessionDefaults {
                font_size: self.active_profile.font_size,
                color_prompt: self.active_profile.color_prompt,
                color_fg: self.active_profile.color_fg,
            };
            let root = rebuild_pane_tree(&st.layout, &mut next_id, cols, rows, &alias_pairs, &defaults);
            self.next_pane_id = next_id;
            let active_pane = root.first_pane_id();
            new_tabs.push(Tab {
                root,
                active_pane_id: active_pane,
                title: st.title.clone(),
            });
        }
        self.tabs = new_tabs;
        self.active_tab = 0;
    }

    fn new_tab(&mut self) {
        let (cols, rows) = self.content_cols_rows();
        let pane_id = self.alloc_pane_id();
        let mut session = self.new_session(cols, rows);

        // Welcome message for new tabs
        session.buf.current_fg = self.active_profile.color_dim;
        session.buf.write_str("New tab\n\n");
        session.show_prompt();

        let tab = Tab {
            root: PaneNode::Leaf { id: pane_id, session },
            active_pane_id: pane_id,
            title: format!("Tab {}", self.tabs.len() + 1),
        };
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, idx: usize) {
        if self.tabs.len() <= 1 {
            return; // Don't close last tab — handled by caller
        }
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
    }

    /// Split the active pane horizontally (side by side).
    fn split_horizontal(&mut self) {
        let pane_id = self.tabs[self.active_tab].active_pane_id;
        let (cols, rows) = self.content_cols_rows();
        let new_id = self.alloc_pane_id();
        let mut new_session = self.new_session(cols, rows / 2);
        new_session.show_prompt();
        let mut opt = Some(new_session);

        let tab = &mut self.tabs[self.active_tab];
        Self::split_node(&mut tab.root, pane_id, new_id, &mut opt, false);
        tab.active_pane_id = new_id;
    }

    /// Split the active pane vertically (side by side).
    fn split_vertical(&mut self) {
        let pane_id = self.tabs[self.active_tab].active_pane_id;
        let (cols, rows) = self.content_cols_rows();
        let new_id = self.alloc_pane_id();
        let mut new_session = self.new_session(cols / 2, rows);
        new_session.show_prompt();
        let mut opt = Some(new_session);

        let tab = &mut self.tabs[self.active_tab];
        Self::split_node(&mut tab.root, pane_id, new_id, &mut opt, true);
        tab.active_pane_id = new_id;
    }

    fn split_node(node: &mut PaneNode, target_id: PaneId, new_id: PaneId, new_session: &mut Option<Session>, horizontal: bool) -> bool {
        match node {
            PaneNode::Leaf { id, .. } => {
                if *id != target_id {
                    return false;
                }
                let sess = match new_session.take() {
                    Some(s) => s,
                    None => return false,
                };
                let old_node = core::mem::replace(node, PaneNode::Leaf { id: 0, session: Session::new(1, 1, Vec::new(), 14, 0xFF50FA7B, 0xFFCCCCCC) });
                let new_leaf = PaneNode::Leaf { id: new_id, session: sess };
                if horizontal {
                    *node = PaneNode::HSplit {
                        left: Box::new(old_node),
                        right: Box::new(new_leaf),
                        ratio: 0.5,
                    };
                } else {
                    *node = PaneNode::VSplit {
                        top: Box::new(old_node),
                        bottom: Box::new(new_leaf),
                        ratio: 0.5,
                    };
                }
                true
            }
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                Self::split_node(left, target_id, new_id, new_session, horizontal)
                    || Self::split_node(right, target_id, new_id, new_session, horizontal)
            }
        }
    }

    /// Close a pane. Returns true if the pane was removed.
    fn close_pane(&mut self, pane_id: PaneId) -> bool {
        let tab = &mut self.tabs[self.active_tab];
        // If it's the only pane, don't close (caller should close tab)
        let mut ids = Vec::new();
        tab.root.collect_pane_ids(&mut ids);
        if ids.len() <= 1 {
            return false;
        }

        if Self::remove_node(&mut tab.root, pane_id) {
            // If active pane was closed, pick first remaining
            if tab.active_pane_id == pane_id {
                tab.active_pane_id = tab.root.first_pane_id();
            }
            true
        } else {
            false
        }
    }

    fn remove_node(node: &mut PaneNode, target_id: PaneId) -> bool {
        match node {
            PaneNode::Leaf { .. } => false,
            PaneNode::HSplit { left, right, .. } | PaneNode::VSplit { top: left, bottom: right, .. } => {
                // Check if left child is the target leaf
                if let PaneNode::Leaf { id, .. } = left.as_ref() {
                    if *id == target_id {
                        // Replace this split with the right child
                        let right_node = core::mem::replace(right.as_mut(), PaneNode::Leaf { id: 0, session: Session::new(1, 1, Vec::new(), 14, 0xFF50FA7B, 0xFFCCCCCC) });
                        *node = right_node;
                        return true;
                    }
                }
                // Check if right child is the target leaf
                if let PaneNode::Leaf { id, .. } = right.as_ref() {
                    if *id == target_id {
                        let left_node = core::mem::replace(left.as_mut(), PaneNode::Leaf { id: 0, session: Session::new(1, 1, Vec::new(), 14, 0xFF50FA7B, 0xFFCCCCCC) });
                        *node = left_node;
                        return true;
                    }
                }
                // Recurse
                Self::remove_node(left, target_id) || Self::remove_node(right, target_id)
            }
        }
    }

    /// Update cols/rows for all panes after resize or split.
    fn update_pane_sizes(&mut self) {
        let area = self.content_area();
        for tab in &mut self.tabs {
            let layout = tab.root.layout(area);
            for (pane_id, rect) in layout {
                if let Some(sess) = tab.root.find_session_mut(pane_id) {
                    let cw = sess.cell_w;
                    let ch = sess.cell_h;
                    // Reserve space for scrollbar on the right
                    let usable_w = rect.w.saturating_sub(SCROLLBAR_WIDTH as u16);
                    let new_cols = (usable_w.saturating_sub(TEXT_PAD * 2) / cw) as usize;
                    let new_rows = (rect.h.saturating_sub(TEXT_PAD * 2) / ch) as usize;
                    sess.buf.cols = new_cols.max(1);
                    sess.buf.visible_rows = new_rows.max(1);
                    // Clamp scroll_offset so cursor stays visible
                    sess.buf.clamp_scroll();
                }
            }
        }
    }

    /// Change font size for the active pane and recalculate.
    fn set_font_size(&mut self, size: u16) {
        if let Some(sess) = self.active_session_mut() {
            sess.set_font_size(size);
        }
        self.update_pane_sizes();
    }

    /// Set font size on ALL sessions (used when switching profiles).
    fn set_font_size_all(&mut self, size: u16) {
        for tab in &mut self.tabs {
            tab.root.for_each_session_mut(|sess| {
                sess.set_font_size(size);
            });
        }
        self.update_pane_sizes();
    }

    /// Apply a theme by name.
    fn apply_theme(&mut self, theme: &Theme) {
        self.active_profile.color_bg = theme.bg;
        self.active_profile.color_fg = theme.fg;
        self.active_profile.color_prompt = theme.prompt;
        self.active_profile.color_title = theme.title;
        self.active_profile.color_dim = theme.dim;
        self.active_profile.color_select_bg = theme.select_bg;
        self.active_profile.color_select_fg = theme.select_fg;
        self.theme_name = String::from(theme.name);
    }

    /// Apply background alpha (transparency).
    fn apply_bg_alpha(&mut self) {
        let alpha = self.bg_alpha as u32;
        let bg = self.active_profile.color_bg;
        self.active_profile.color_bg = (alpha << 24) | (bg & 0x00FFFFFF);
    }

    /// Update tab title to show cwd.
    fn update_tab_title_from_cwd(&mut self) {
        let pane_id = self.tabs[self.active_tab].active_pane_id;
        if let Some(sess) = self.tabs[self.active_tab].root.find_session(pane_id) {
            let cwd = &sess.shell.cwd;
            // Show last path component or "/" for root
            let short = if cwd == "/" {
                "/"
            } else {
                cwd.rsplit('/').next().unwrap_or(cwd.as_str())
            };
            self.tabs[self.active_tab].title = format!("{} - {}", short, cwd);
        }
    }
}

// ─── Environment / PATH helpers ──────────────────────────────────────────────

/// Read a file into a buffer. Returns the number of bytes read.
fn read_file_to_buf(path: &str, buf: &mut [u8]) -> usize {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return 0;
    }
    let mut total = 0usize;
    loop {
        let n = fs::read(fd, &mut buf[total..]);
        if n == 0 || n == u32::MAX {
            break;
        }
        total += n as usize;
        if total >= buf.len() {
            break;
        }
    }
    fs::close(fd);
    total
}

/// Source an env file — supports:
///   KEY=VALUE
///   export KEY=VALUE
///   alias NAME=VALUE
///   source /path/to/file
///   # comments
/// `depth` prevents infinite recursion.
/// `aliases` collects alias definitions if provided.
fn source_env_file(path: &str, depth: u32, aliases: &mut Vec<(String, String)>) {
    if depth > 4 {
        return; // prevent infinite source loops
    }
    let mut data = [0u8; 4096];
    let total = read_file_to_buf(path, &mut data);
    if total == 0 {
        return;
    }

    let text = match core::str::from_utf8(&data[..total]) {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle 'source /path/to/file'
        if line.starts_with("source ") {
            let target = line[7..].trim();
            if !target.is_empty() {
                source_env_file(target, depth + 1, aliases);
            }
            continue;
        }

        // Handle 'alias NAME=VALUE' or "alias NAME='VALUE'"
        if line.starts_with("alias ") {
            let alias_def = line[6..].trim();
            if let Some(eq) = alias_def.find('=') {
                let name = alias_def[..eq].trim();
                let mut val = alias_def[eq + 1..].trim();
                // Strip surrounding quotes
                if (val.starts_with('\'') && val.ends_with('\''))
                    || (val.starts_with('"') && val.ends_with('"'))
                {
                    val = &val[1..val.len() - 1];
                }
                if !name.is_empty() {
                    // Update existing or add new
                    if let Some(existing) = aliases.iter_mut().find(|(n, _)| n == name) {
                        existing.1 = String::from(val);
                    } else {
                        aliases.push((String::from(name), String::from(val)));
                    }
                }
            }
            continue;
        }

        // Strip optional 'export ' prefix
        let assignment = if line.starts_with("export ") {
            line[7..].trim()
        } else {
            line
        };

        if let Some(eq) = assignment.find('=') {
            let key = assignment[..eq].trim();
            let val = assignment[eq + 1..].trim();
            if !key.is_empty() {
                anyos_std::env::set(key, val);
            }
        }
    }
}

/// Load system and user env files. Returns collected aliases.
fn load_dotenv() -> Vec<(String, String)> {
    let mut aliases = Vec::new();

    // 1. System environment
    source_env_file("/System/env", 0, &mut aliases);

    // 2. User environment — determine username from uid
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 32];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            if username != "root" {
                let user_env = format!("/Users/{}/env", username);
                source_env_file(&user_env, 0, &mut aliases);
                // Update HOME and USER based on actual identity
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
                anyos_std::env::set("USER", username);
            }
        }
    }

    aliases
}

/// Resolve a bare command name via PATH — delegates to anyos_std::shell.
fn resolve_from_path(cmd: &str) -> Option<String> {
    anyos_std::shell::resolve_from_path(cmd)
}

// ─── Shell ───────────────────────────────────────────────────────────────────

// ─── Logical Operators for Command Chaining ──────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum LogicalOp {
    None,       // first command
    And,        // &&
    Or,         // ||
    Semicolon,  // ;
}

/// Split a command line on `&&`, `||`, and `;` operators.
/// Returns None if there is only a single command (no operators found).
/// Respects quoting: operators inside quotes are not split.
fn split_logical_operators(line: &str) -> Option<Vec<(LogicalOp, String)>> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result: Vec<(LogicalOp, String)> = Vec::new();
    let mut current_op = LogicalOp::None;
    let mut start = 0;
    let mut i = 0;
    let mut found_op = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        match bytes[i] {
            b'\'' if !in_double_quote => { in_single_quote = !in_single_quote; i += 1; }
            b'"' if !in_single_quote => { in_double_quote = !in_double_quote; i += 1; }
            b'\\' if !in_single_quote => { i += 2; } // skip escaped char
            b'&' if !in_single_quote && !in_double_quote && i + 1 < len && bytes[i + 1] == b'&' => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::And;
                i += 2;
                start = i;
                found_op = true;
            }
            b'|' if !in_single_quote && !in_double_quote && i + 1 < len && bytes[i + 1] == b'|' => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::Or;
                i += 2;
                start = i;
                found_op = true;
            }
            b';' if !in_single_quote && !in_double_quote => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::Semicolon;
                i += 1;
                start = i;
                found_op = true;
            }
            _ => { i += 1; }
        }
    }

    if !found_op {
        return None;
    }

    // Add final segment
    let cmd = String::from(line[start..].trim());
    if !cmd.is_empty() {
        result.push((current_op, cmd));
    }

    Some(result)
}

// ─── Background Job ──────────────────────────────────────────────────────────

struct BackgroundJob {
    job_id: u32,
    tid: u32,
    command: String,
    finished: bool,
    exit_code: u32,
    stopped: bool,
    /// Stdout pipe (needed to resume as foreground)
    pipe_id: u32,
    /// Stdin pipe (needed to resume as foreground)
    stdin_pipe: u32,
    /// Extra pipes from pipeline
    extra_pipes: Vec<u32>,
}

// ─── Shell ──────────────────────────────────────────────────────────────────

struct Shell {
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    cwd: String,
    aliases: Vec<(String, String)>,
    bg_jobs: Vec<BackgroundJob>,
    next_job_id: u32,
    /// Monotone counter for unique pipe names (shared with anyos_std::shell::run_pipeline).
    pipe_counter: u32,
}

impl Shell {
    fn new() -> Self {
        Shell {
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            cwd: String::from("/"),
            aliases: Vec::new(),
            bg_jobs: Vec::new(),
            next_job_id: 1,
            pipe_counter: 0,
        }
    }

    fn prompt(&self) -> String {
        format!("{}> ", self.cwd)
    }

    fn insert_char(&mut self, c: char) {
        if self.cursor >= self.input.len() {
            self.input.push(c);
        } else {
            self.input.insert(self.cursor, c);
        }
        self.cursor += 1;
        self.history_index = None;
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => self.history.len() - 1,
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    fn history_down(&mut self) {
        match self.history_index {
            None => return,
            Some(i) => {
                if i + 1 >= self.history.len() {
                    self.history_index = None;
                    self.input.clear();
                    self.cursor = 0;
                } else {
                    self.history_index = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                    self.cursor = self.input.len();
                }
            }
        }
    }

    fn cursor_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    fn cursor_end(&mut self) {
        self.cursor = self.input.len();
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    /// Execute command. Returns (should_continue, optional foreground process, optional pending su username).
    fn submit(&mut self, buf: &mut TerminalBuffer) -> (bool, Option<ForegroundProcess>, Option<String>) {
        let raw_line = self.input.trim_matches(|c: char| c == ' ').to_string();
        buf.write_char('\n');

        if !raw_line.is_empty() {
            if self.history.last().map_or(true, |last| *last != raw_line) {
                self.history.push(raw_line.clone());
                if self.history.len() > 64 {
                    self.history.remove(0);
                }
            }
        }

        self.input.clear();
        self.cursor = 0;
        self.history_index = None;

        if raw_line.is_empty() {
            return (true, None, None);
        }

        // Check for logical operators (&&, ||, ;)
        // Split on these and execute sequentially
        if let Some(chain) = split_logical_operators(&raw_line) {
            let mut last_success = true;
            for (op, cmd_str) in chain {
                let should_run = match op {
                    LogicalOp::None | LogicalOp::Semicolon => true,
                    LogicalOp::And => last_success,
                    LogicalOp::Or => !last_success,
                };
                if !should_run { continue; }
                let cmd_str = cmd_str.trim().to_string();
                if cmd_str.is_empty() { continue; }
                // Execute each sub-command via recursive submit
                let saved_input = core::mem::replace(&mut self.input, cmd_str);
                let saved_cursor = self.cursor;
                self.cursor = self.input.len();
                let (cont, fg, su) = self.submit(buf);
                self.input = saved_input;
                self.cursor = saved_cursor;
                // For now, assume builtins succeed. External commands we can't chain.
                if fg.is_some() || su.is_some() {
                    // External process — can't chain further, return it
                    return (cont, fg, su);
                }
                if !cont { return (false, None, None); }
                last_success = true; // builtins always succeed for now
            }
            return (true, None, None);
        }

        // Expand aliases: replace first word if it matches an alias
        let expanded_line = {
            let first_word = raw_line.split_whitespace().next().unwrap_or("");
            if let Some((_, val)) = self.aliases.iter().find(|(n, _)| n == first_word) {
                let rest = raw_line[first_word.len()..].to_string();
                format!("{}{}", val, rest)
            } else {
                raw_line.clone()
            }
        };

        // Parse input redirect (< file) first, then output redirects
        let (expanded_line, input_redir) = parse_input_redirect(&expanded_line, &self.cwd);

        // Parse output redirects
        let (line, mut redirect) = parse_redirects(&expanded_line, &self.cwd);

        // If there's an output redirect, enable capture mode for builtins
        if redirect.is_some() {
            buf.capture = Some(String::new());
        }

        // If there's an input redirect, read the file and make it available as stdin
        let input_data = if let Some(ref ir) = input_redir {
            fs::read_to_string(&ir.source).ok()
        } else {
            None
        };

        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let raw_args = parts.next().unwrap_or("");

        // POSIX-style expansions: tilde (~) and glob (*, ?, [...])
        let tokens = expand_args(raw_args, &self.cwd);
        let expanded_args_buf = tokens.join(" ");
        let args = expanded_args_buf.as_str();

        match cmd {
            "help" => self.cmd_help(buf),
            "echo" => {
                buf.current_fg = COLOR_FG;
                buf.write_str(args);
                buf.write_char('\n');
            }
            "clear" => buf.clear(),
            "uname" => {
                buf.current_fg = COLOR_FG;
                buf.write_str(".anyOS v");
                buf.write_str(env!("ANYOS_VERSION"));
                buf.write_str(" x86_64\n");
            }
            "cd" => self.cmd_cd(args, buf),
            "pwd" => {
                buf.current_fg = COLOR_FG;
                buf.write_str(&self.cwd);
                buf.write_char('\n');
            }
            "set" => self.cmd_set(args, buf),
            "export" => self.cmd_export(args, buf),
            "unset" => self.cmd_unset(args, buf),
            "alias" => self.cmd_alias(args, buf),
            "unalias" => self.cmd_unalias(args, buf),
            "eval" => {
                // Concatenate all args and execute as a command
                let eval_line = args.trim().to_string();
                if !eval_line.is_empty() {
                    // Disable capture temporarily — eval will set up its own
                    buf.capture = None;
                    let saved_input = core::mem::replace(&mut self.input, eval_line);
                    let saved_cursor = self.cursor;
                    self.cursor = self.input.len();
                    let (cont, fg, su) = self.submit(buf);
                    self.input = saved_input;
                    self.cursor = saved_cursor;
                    if !cont {
                        return (false, None, None);
                    }
                    if fg.is_some() || su.is_some() {
                        return (cont, fg, su);
                    }
                }
                return (true, None, None);
            }
            "source" | "." => self.cmd_source(args, buf),
            "su" => {
                // Disable capture for su (interactive)
                if let Some(captured) = buf.capture.take() {
                    if let Some(ref mut redir) = redirect {
                        write_redirect(redir, &captured);
                    }
                }
                let pending = self.cmd_su(args, buf);
                if pending.is_some() {
                    return (true, None, pending);
                }
            }
            "jobs" => self.cmd_jobs(buf),
            "fg" => {
                // Bring last background/stopped job to foreground
                if let Some(job) = self.bg_jobs.iter().rev().find(|j| !j.finished) {
                    let tid = job.tid;
                    let cmd = job.command.clone();
                    let was_stopped = job.stopped;
                    let has_pipe = job.pipe_id != 0;
                    let pipe_id = job.pipe_id;
                    let stdin_pipe = job.stdin_pipe;
                    let extra_pipes = job.extra_pipes.clone();

                    buf.current_fg = COLOR_FG;
                    if !cmd.is_empty() {
                        buf.write_str(&cmd);
                    } else {
                        let fallback = format!("TID {}", tid);
                        buf.write_str(&fallback);
                    }
                    buf.write_char('\n');

                    if was_stopped {
                        // Resume the stopped process with SIGCONT
                        process::send_signal(tid, process::SIGCONT);
                    }
                    // Mark job as done in bg_jobs and re-attach as foreground
                    for job in &mut self.bg_jobs {
                        if job.tid == tid { job.finished = true; }
                    }
                    // Always return as ForegroundProcess so the event loop
                    // can handle Ctrl+C / Ctrl+Z while waiting for exit.
                    return (true, Some(ForegroundProcess {
                        tid, pipe_id, stdin_pipe, extra_pipes,
                        redirect: None, command: cmd,
                    }), None);
                } else {
                    buf.current_fg = COLOR_FG;
                    buf.write_str("fg: no current job\n");
                }
            }
            "bg" => {
                // Resume last stopped job in background
                if let Some(job) = self.bg_jobs.iter_mut().rev().find(|j| j.stopped && !j.finished) {
                    let tid = job.tid;
                    let job_id = job.job_id;
                    job.stopped = false;
                    process::send_signal(tid, process::SIGCONT);
                    buf.current_fg = COLOR_DIM;
                    let msg = format!("[{}]  {} &\n", job_id, tid);
                    buf.write_str(&msg);
                } else {
                    buf.current_fg = COLOR_FG;
                    buf.write_str("bg: no stopped job\n");
                }
            }
            "profile" => cmd_profile(args, buf),
            "theme" => cmd_theme(args, buf),
            "opacity" => cmd_opacity(args, buf),
            "exit" => return (false, None, None),
            "reboot" => {
                buf.current_fg = COLOR_FG;
                buf.write_str("Rebooting...\n");
                process::reboot();
            }
            "shutdown" | "poweroff" => {
                buf.current_fg = COLOR_FG;
                buf.write_str("Shutting down...\n");
                process::shutdown();
            }
            _ => {
                // Disable capture for external commands — they use pipe redirect instead
                buf.capture = None;

                // Check for pipeline: "cmd1 | cmd2 | cmd3"
                if line.contains('|') {
                    if let Some(fp) = self.execute_pipeline(&line, redirect, buf) {
                        return (true, Some(fp), None);
                    }
                    return (true, None, None);
                }

                // Check for background suffix: "cmd &"
                let (cmd_line, background) = if line.ends_with(" &") || line.ends_with("\t&") {
                    (&line[..line.len() - 2], true)
                } else if line.ends_with('&') && line.len() > 1 {
                    (&line[..line.len() - 1], true)
                } else {
                    (line.as_str(), false)
                };

                // Re-parse cmd and args from the (possibly trimmed) line
                let mut bg_parts = cmd_line.splitn(2, ' ');
                let bg_cmd = bg_parts.next().unwrap_or("");
                let raw_bg_args = bg_parts.next().unwrap_or("");

                // POSIX-style expansions: tilde and glob
                let mut bg_tokens = expand_args(raw_bg_args, &self.cwd);

                // Default args for specific commands (e.g. ls defaults to cwd)
                if bg_tokens.is_empty() {
                    if bg_cmd == "ls" {
                        bg_tokens.push(String::from("--color"));
                        bg_tokens.push(String::from(self.cwd.as_str()));
                    }
                } else if bg_cmd == "ls" && bg_tokens.iter().all(|t| t.starts_with('-')) {
                    // ls with only flags, no path — append cwd
                    bg_tokens.insert(0, String::from("--color"));
                    bg_tokens.push(String::from(self.cwd.as_str()));
                } else if bg_cmd == "ls" {
                    bg_tokens.insert(0, String::from("--color"));
                }

                // Resolve command path:
                // - Absolute paths (/foo/bar) used as-is
                // - Relative paths (./foo, ../foo) resolved against cwd
                // - Bare names resolved via PATH
                let path = if bg_cmd.starts_with('/') {
                    String::from(bg_cmd)
                } else if bg_cmd.starts_with("./") || bg_cmd.starts_with("../") {
                    let rel = bg_cmd.strip_prefix("./").unwrap_or(bg_cmd);
                    if self.cwd == "/" {
                        format!("/{}", rel)
                    } else {
                        format!("{}/{}", self.cwd, rel)
                    }
                } else {
                    match resolve_from_path(bg_cmd) {
                        Some(p) => p,
                        None => format!("/System/bin/{}", bg_cmd),
                    }
                };

                // Build full args string with program name as argv[0],
                // quoting tokens that contain spaces (POSIX-compliant).
                let full_args_buf;
                let bg_args_quoted = shell_join(&bg_tokens);
                let full_args = if bg_args_quoted.is_empty() {
                    bg_cmd
                } else {
                    full_args_buf = format!("{} {}", bg_cmd, bg_args_quoted);
                    &full_args_buf
                };

                if background {
                    // Background: spawn without pipe or waiting
                    let tid = process::spawn(&path, full_args);
                    if tid == u32::MAX {
                        buf.current_fg = COLOR_FG;
                        buf.write_str("Unknown command: ");
                        buf.write_str(bg_cmd);
                        buf.write_str("\n");
                    } else {
                        let job_id = self.next_job_id;
                        self.next_job_id += 1;
                        self.bg_jobs.push(BackgroundJob {
                            job_id,
                            tid,
                            command: String::from(cmd_line),
                            finished: false,
                            exit_code: 0,
                            stopped: false,
                            pipe_id: 0,
                            stdin_pipe: 0,
                            extra_pipes: Vec::new(),
                        });
                        buf.current_fg = COLOR_DIM;
                        let msg = format!("[{}] {}\n", job_id, tid);
                        buf.write_str(&msg);
                    }
                } else {
                    // Foreground: capture output via pipe, poll in main loop
                    let pipe_name = format!("term:stdout:{}", bg_cmd);
                    let pipe_id = ipc::pipe_create(&pipe_name);
                    let stdin_name = format!("term:stdin:{}", bg_cmd);
                    let stdin_pipe = ipc::pipe_create(&stdin_name);

                    let tid = process::spawn_piped_full(&path, full_args, pipe_id, stdin_pipe);
                    if tid == u32::MAX {
                        ipc::pipe_close(pipe_id);
                        ipc::pipe_close(stdin_pipe);
                        buf.current_fg = COLOR_FG;
                        buf.write_str("Unknown command: ");
                        buf.write_str(bg_cmd);
                        buf.write_str("\nType 'help' for available commands.\n");
                    } else {
                        // If there's input redirection, write file contents to stdin pipe
                        if let Some(ref data) = input_data {
                            ipc::pipe_write(stdin_pipe, data.as_bytes());
                            // Don't close the pipe here — close() destroys it with all
                            // buffered data before the child can read.  The pipe will be
                            // cleaned up when the foreground process exits (poll_fg_processes).
                            return (true, Some(ForegroundProcess { tid, pipe_id, stdin_pipe, extra_pipes: Vec::new(), redirect: redirect.clone(), command: String::from(cmd_line) }), None);
                        }
                        return (true, Some(ForegroundProcess { tid, pipe_id, stdin_pipe, extra_pipes: Vec::new(), redirect: redirect.clone(), command: String::from(cmd_line) }), None);
                    }
                }
            }
        }

        // Report finished background jobs
        for job in &mut self.bg_jobs {
            if !job.finished {
                let status = process::try_waitpid(job.tid);
                if status != process::STILL_RUNNING {
                    job.finished = true;
                    job.exit_code = status;
                    buf.current_fg = COLOR_DIM;
                    let done_str = if status == 0 { "Done" } else { "Exit" };
                    let msg = format!("[{}]  {}  {}\n", job.job_id, done_str, job.command);
                    buf.write_str(&msg);
                }
            }
        }
        self.bg_jobs.retain(|j| !j.finished);

        // Flush capture buffer for builtin commands with redirect
        if let Some(captured) = buf.capture.take() {
            if let Some(ref mut redir) = redirect {
                write_redirect(redir, &captured);
            }
        }

        (true, None, None)
    }

    /// Execute a pipeline (cmd1 | cmd2 | cmd3).
    /// Returns a ForegroundProcess tracking the last command + display pipe.
    fn execute_pipeline(&mut self, line: &str, redirect: Option<Redirect>, buf: &mut TerminalBuffer) -> Option<ForegroundProcess> {
        match anyos_std::shell::run_pipeline(line, &self.cwd, &mut self.pipe_counter) {
            Some(r) => Some(ForegroundProcess {
                tid: r.last_tid,
                pipe_id: r.display_pipe,
                stdin_pipe: 0,
                extra_pipes: r.extra_pipes,
                redirect,
                command: String::from(line),
            }),
            None => {
                buf.current_fg = COLOR_FG;
                buf.write_str("pipe: command not found\n");
                None
            }
        }
    }

    fn cmd_jobs(&mut self, buf: &mut TerminalBuffer) {
        // Update status of all jobs
        for job in &mut self.bg_jobs {
            if !job.finished {
                let status = process::try_waitpid(job.tid);
                if status == process::STOPPED {
                    job.stopped = true;
                } else if status != process::STILL_RUNNING {
                    job.finished = true;
                    job.exit_code = status;
                    // Close pipes for finished jobs
                    if job.pipe_id != 0 { ipc::pipe_close(job.pipe_id); job.pipe_id = 0; }
                    if job.stdin_pipe != 0 { ipc::pipe_close(job.stdin_pipe); job.stdin_pipe = 0; }
                    for &p in &job.extra_pipes { ipc::pipe_close(p); }
                    job.extra_pipes.clear();
                }
            }
        }
        if self.bg_jobs.is_empty() {
            buf.current_fg = COLOR_FG;
            buf.write_str("No jobs\n");
            return;
        }
        let last_id = self.bg_jobs.iter().rev().find(|j| !j.finished).map(|j| j.job_id).unwrap_or(0);
        for job in &self.bg_jobs {
            buf.current_fg = COLOR_FG;
            let marker = if job.job_id == last_id { "+" } else { "-" };
            let (status, suffix) = if job.finished {
                if job.exit_code == 0 { ("Fertig", "") } else { ("Beendet", "") }
            } else if job.stopped {
                ("Angehalten", "")
            } else {
                ("Läuft", " &")
            };
            let cmd_display = if job.command.is_empty() {
                format!("TID {}", job.tid)
            } else {
                format!("{}{}", job.command, suffix)
            };
            let line = format!("[{}]{}  {:<20}{}\n", job.job_id, marker, status, cmd_display);
            buf.write_str(&line);
        }
        // Clean up finished jobs
        self.bg_jobs.retain(|j| !j.finished);
    }

    fn cmd_help(&self, buf: &mut TerminalBuffer) {
        buf.current_fg = COLOR_TITLE;
        buf.write_str(".anyOS Terminal - Befehle:\n");
        buf.current_fg = COLOR_FG;
        buf.write_str("\n");
        buf.write_str("  Eingebaut:\n");
        buf.write_str("    help     Diese Hilfe\n");
        buf.write_str("    echo     Text ausgeben\n");
        buf.write_str("    clear    Bildschirm loeschen\n");
        buf.write_str("    cd       Verzeichnis wechseln\n");
        buf.write_str("    pwd      Aktuelles Verzeichnis\n");
        buf.write_str("    set      Umgebungsvariable setzen\n");
        buf.write_str("    export   Variable exportieren\n");
        buf.write_str("    unset    Variable loeschen\n");
        buf.write_str("    alias    Alias definieren/auflisten\n");
        buf.write_str("    eval     Argumente ausfuehren\n");
        buf.write_str("    uname    Systemidentifikation\n");
        buf.write_str("    profile  Profile verwalten\n");
        buf.write_str("    theme    Farbschema aendern\n");
        buf.write_str("    opacity  Transparenz einstellen (0-100)\n");
        buf.write_str("    jobs     Hintergrund-Jobs auflisten\n");
        buf.write_str("    fg       Job in Vordergrund bringen\n");
        buf.write_str("    bg       Gestoppten Job fortsetzen\n");
        buf.write_str("    exit     Terminal beenden\n");
        buf.write_str("\n");
        buf.write_str("  Shortcuts:\n");
        buf.write_str("    Ctrl+Z        Prozess anhalten\n");
        buf.write_str("    Ctrl+Shift+T  Neuer Tab\n");
        buf.write_str("    Ctrl+Shift+W  Tab schliessen\n");
        buf.write_str("    Ctrl+Shift+R  Tab umbenennen\n");
        buf.write_str("    Ctrl+Shift+F  Suchen\n");
        buf.write_str("    Ctrl+R        Verlauf durchsuchen\n");
        buf.write_str("    Ctrl+Tab      Naechster Tab\n");
        buf.write_str("    Ctrl+Shift+O  Horizontal teilen\n");
        buf.write_str("    Ctrl+Shift+E  Vertikal teilen\n");
        buf.write_str("    Ctrl+Shift+X  Pane schliessen\n");
        buf.write_str("    Ctrl+Plus     Schrift vergroessern\n");
        buf.write_str("    Ctrl+Minus    Schrift verkleinern\n");
        buf.write_str("    Ctrl+0        Schrift zuruecksetzen\n");
        buf.write_str("    PageUp/Down   Scrollback navigieren\n");
        buf.write_str("    Alt+Klick     Datei oeffnen\n");
        buf.write_str("    Ctrl+,        Einstellungen\n");
        buf.write_str("    Ctrl+Klick    URL oeffnen\n");
        buf.write_str("    Doppelklick   Tab umbenennen\n");
        buf.write_str("\n");
        buf.write_str("  Tip: & fuer Hintergrundprozesse\n");
        buf.write_str("  Tip: | fuer Pipes: ls | cat\n");
        buf.write_str("  Tip: > fuer Redirect: echo hello > file\n");
    }

    fn cmd_alias(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let args = args.trim();
        if args.is_empty() {
            // List all aliases
            buf.current_fg = COLOR_FG;
            if self.aliases.is_empty() {
                buf.write_str("No aliases defined.\n");
            } else {
                for (name, val) in &self.aliases {
                    buf.write_str("alias ");
                    buf.write_str(name);
                    buf.write_str("='");
                    buf.write_str(val);
                    buf.write_str("'\n");
                }
            }
            return;
        }
        // alias name=value or alias name='value with spaces'
        if let Some(eq) = args.find('=') {
            let name = args[..eq].trim();
            let mut val = args[eq + 1..].trim();
            // Strip surrounding quotes
            if (val.starts_with('\'') && val.ends_with('\''))
                || (val.starts_with('"') && val.ends_with('"'))
            {
                val = &val[1..val.len() - 1];
            }
            if name.is_empty() {
                buf.current_fg = COLOR_FG;
                buf.write_str("alias: invalid name\n");
                return;
            }
            // Update existing or add new
            if let Some(existing) = self.aliases.iter_mut().find(|(n, _)| n == name) {
                existing.1 = String::from(val);
            } else {
                self.aliases.push((String::from(name), String::from(val)));
            }
        } else {
            // Show value of specific alias
            if let Some((_, val)) = self.aliases.iter().find(|(n, _)| n == args) {
                buf.current_fg = COLOR_FG;
                buf.write_str("alias ");
                buf.write_str(args);
                buf.write_str("='");
                buf.write_str(val);
                buf.write_str("'\n");
            } else {
                buf.current_fg = COLOR_FG;
                buf.write_str("alias: '");
                buf.write_str(args);
                buf.write_str("' not found\n");
            }
        }
    }

    fn cmd_unalias(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let name = args.trim();
        if name.is_empty() {
            buf.current_fg = COLOR_FG;
            buf.write_str("usage: unalias <name>\n");
            return;
        }
        if name == "-a" {
            // Remove all aliases
            self.aliases.clear();
            return;
        }
        let before = self.aliases.len();
        self.aliases.retain(|(n, _)| n != name);
        if self.aliases.len() == before {
            buf.current_fg = COLOR_FG;
            buf.write_str("unalias: '");
            buf.write_str(name);
            buf.write_str("' not found\n");
        }
    }

    fn cmd_set(&self, args: &str, buf: &mut TerminalBuffer) {
        let args = args.trim();
        if args.is_empty() {
            // List all variables
            let mut env_buf = [0u8; 4096];
            let total = anyos_std::env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            buf.current_fg = COLOR_FG;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    buf.write_str(entry);
                    buf.write_char('\n');
                }
                offset += end + 1;
            }
            return;
        }
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if key.is_empty() {
                buf.current_fg = COLOR_FG;
                buf.write_str("set: invalid variable name\n");
                return;
            }
            anyos_std::env::set(key, value);
        } else {
            // Show value of a single variable
            let mut val_buf = [0u8; 256];
            let len = anyos_std::env::get(args, &mut val_buf);
            buf.current_fg = COLOR_FG;
            if len != u32::MAX {
                let val = core::str::from_utf8(&val_buf[..len as usize]).unwrap_or("");
                buf.write_str(args);
                buf.write_char('=');
                buf.write_str(val);
                buf.write_char('\n');
            } else {
                buf.write_str("set: '");
                buf.write_str(args);
                buf.write_str("' not set\n");
            }
        }
    }

    fn cmd_export(&self, args: &str, buf: &mut TerminalBuffer) {
        let args = args.trim();
        if args.is_empty() {
            // List all with "export" prefix
            let mut env_buf = [0u8; 4096];
            let total = anyos_std::env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            buf.current_fg = COLOR_FG;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    buf.write_str("export ");
                    buf.write_str(entry);
                    buf.write_char('\n');
                }
                offset += end + 1;
            }
            return;
        }
        // Same as set — all env vars are "exported" (inherited by child processes)
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if !key.is_empty() {
                anyos_std::env::set(key, value);
            }
        } else {
            // Mark existing var as exported (no-op since all are exported)
            let mut val_buf = [0u8; 256];
            let len = anyos_std::env::get(args, &mut val_buf);
            if len == u32::MAX {
                anyos_std::env::set(args, "");
            }
        }
    }

    fn cmd_unset(&self, args: &str, buf: &mut TerminalBuffer) {
        let key = args.trim();
        if key.is_empty() {
            buf.current_fg = COLOR_FG;
            buf.write_str("Usage: unset VARIABLE\n");
            return;
        }
        anyos_std::env::unset(key);
    }

    fn cmd_cd(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let target = args.trim();
        if target.is_empty() || target == "/" {
            self.cwd = String::from("/");
            anyos_std::env::set("PWD", "/");
            fs::chdir("/");
            app().update_tab_title_from_cwd();
            update_tab_labels();
            return;
        }

        // Resolve path relative to cwd
        let new_path = if target.starts_with('/') {
            String::from(target)
        } else if target == ".." {
            // Go up one level
            if self.cwd == "/" {
                return;
            }
            let trimmed = self.cwd.trim_end_matches('/');
            match trimmed.rfind('/') {
                Some(0) => String::from("/"),
                Some(pos) => String::from(&trimmed[..pos]),
                None => String::from("/"),
            }
        } else {
            if self.cwd == "/" {
                format!("/{}", target)
            } else {
                format!("{}/{}", self.cwd, target)
            }
        };

        // Verify directory exists via stat
        let mut stat_buf = [0u32; 7];
        let ret = fs::stat(&new_path, &mut stat_buf);
        if ret != 0 {
            buf.current_fg = COLOR_FG;
            buf.write_str("cd: ");
            buf.write_str(&new_path);
            buf.write_str(": No such directory\n");
            return;
        }
        // stat_buf[0] = type: 0=regular file, 1=directory
        if stat_buf[0] != 1 {
            buf.current_fg = COLOR_FG;
            buf.write_str("cd: ");
            buf.write_str(&new_path);
            buf.write_str(": Not a directory\n");
            return;
        }

        self.cwd = new_path;
        anyos_std::env::set("PWD", &self.cwd);
        fs::chdir(&self.cwd);
        app().update_tab_title_from_cwd();
        update_tab_labels();
    }

    fn cmd_source(&mut self, args: &str, buf: &mut TerminalBuffer) {
        let path = args.trim();
        if path.is_empty() {
            buf.current_fg = COLOR_FG;
            buf.write_str("usage: source <file>\n");
            return;
        }

        let mut data = [0u8; 4096];
        let total = read_file_to_buf(path, &mut data);
        if total == 0 {
            buf.current_fg = COLOR_FG;
            buf.write_str("source: cannot read '");
            buf.write_str(path);
            buf.write_str("'\n");
            return;
        }

        let text = match core::str::from_utf8(&data[..total]) {
            Ok(s) => s,
            Err(_) => {
                buf.current_fg = COLOR_FG;
                buf.write_str("source: invalid UTF-8\n");
                return;
            }
        };

        for line in text.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse command and args
            let mut parts = line.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let cmd_args = parts.next().unwrap_or("");

            match cmd {
                "export" => self.cmd_export(cmd_args, buf),
                "set" => self.cmd_set(cmd_args, buf),
                "unset" => self.cmd_unset(cmd_args, buf),
                "alias" => self.cmd_alias(cmd_args, buf),
                "unalias" => self.cmd_unalias(cmd_args, buf),
                "cd" => self.cmd_cd(cmd_args, buf),
                "echo" => {
                    buf.current_fg = COLOR_FG;
                    buf.write_str(cmd_args);
                    buf.write_char('\n');
                }
                "source" | "." => self.cmd_source(cmd_args, buf),
                _ => {
                    // Handle KEY=VALUE assignment (no command prefix)
                    if let Some(eq) = line.find('=') {
                        if !line[..eq].contains(' ') {
                            let key = line[..eq].trim();
                            let val = line[eq + 1..].trim();
                            if !key.is_empty() {
                                anyos_std::env::set(key, val);
                            }
                            continue;
                        }
                    }

                    // External command — resolve path, spawn, and wait
                    let resolved = if cmd.starts_with('/') {
                        String::from(cmd)
                    } else if cmd.starts_with("./") || cmd.starts_with("../") {
                        if self.cwd == "/" {
                            format!("/{}", cmd.trim_start_matches("./"))
                        } else {
                            format!("{}/{}", self.cwd, cmd)
                        }
                    } else {
                        match resolve_from_path(cmd) {
                            Some(p) => p,
                            None => format!("/System/bin/{}", cmd),
                        }
                    };

                    let full_args = if cmd_args.is_empty() {
                        String::from(cmd)
                    } else {
                        format!("{} {}", cmd, cmd_args)
                    };

                    let tid = process::spawn(&resolved, &full_args);
                    if tid != u32::MAX {
                        process::waitpid(tid);
                    }
                }
            }
        }
    }

    /// Execute `su`. If password is given as argument, authenticate immediately.
    /// Otherwise, return the target username so the caller can prompt for a password.
    fn cmd_su(&self, args: &str, buf: &mut TerminalBuffer) -> Option<String> {
        let parts: Vec<&str> = args.split_whitespace().collect();
        let username = if parts.is_empty() { "root" } else { parts[0] };

        if parts.len() > 1 {
            // Password given as argument — authenticate immediately
            let password = parts[1];
            Self::do_su(username, password, buf);
            None
        } else {
            // Need to prompt for password interactively
            buf.current_fg = COLOR_FG;
            buf.write_str("Password: ");
            Some(String::from(username))
        }
    }

    fn do_su(username: &str, password: &str, buf: &mut TerminalBuffer) {
        buf.current_fg = COLOR_FG;
        if process::authenticate(username, password) {
            anyos_std::env::set("USER", username);
            let uid = process::getuid();
            if uid == 0 {
                anyos_std::env::set("HOME", "/");
            } else {
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
            }
            buf.write_str("Switched to user '");
            buf.write_str(username);
            buf.write_str("'.\n");
        } else {
            buf.write_str("su: authentication failed for '");
            buf.write_str(username);
            buf.write_str("'\n");
        }
    }
}

// ─── Builtins & Completion ───────────────────────────────────────────────────

const BUILTINS: &[&str] = &[
    "help", "echo", "clear", "uname", "cd", "pwd",
    "set", "export", "unset", "source", "su", "exit", "reboot",
    "shutdown", "poweroff", "profile", "theme", "opacity",
];

/// Erase the input portion of the current display line and rewrite it.
fn redraw_input_line(buf: &mut TerminalBuffer, shell: &Shell) {
    let prompt_len = shell.prompt().len();
    if buf.cursor_row < buf.lines.len() {
        buf.lines[buf.cursor_row].truncate(prompt_len);
    }
    buf.cursor_col = prompt_len;
    buf.current_fg = COLOR_FG;
    buf.write_str(&shell.input);
    buf.cursor_col = prompt_len + shell.cursor;
}

/// List directory entries as (name, is_directory) pairs.
fn list_dir_entries(path: &str) -> Vec<(String, bool)> {
    let mut entries = Vec::new();
    let mut dir_buf = [0u8; 64 * 256]; // 256 entries max (covers /System/bin/ etc.)
    let count = fs::readdir(path, &mut dir_buf);
    if count == u32::MAX {
        return entries;
    }
    let max = (count as usize).min(dir_buf.len() / 64);
    for i in 0..max {
        let off = i * 64;
        let entry_type = dir_buf[off];
        let name_len = dir_buf[off + 1] as usize;
        let name_bytes = &dir_buf[off + 8..off + 8 + name_len.min(56)];
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            entries.push((String::from(name), entry_type == 1));
        }
    }
    entries
}

/// Find the longest common prefix among a set of strings.
fn common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = &items[0];
    let mut len = first.len();
    for item in &items[1..] {
        len = len.min(item.len());
        for (i, (a, b)) in first.bytes().zip(item.bytes()).enumerate() {
            if i >= len { break; }
            if a != b {
                len = i;
                break;
            }
        }
    }
    String::from(&first[..len])
}

/// Complete a command name (first word on the line).
fn complete_command(prefix: &str) -> Vec<String> {
    let mut matches = Vec::new();
    for &b in BUILTINS {
        if b.starts_with(prefix) {
            matches.push(String::from(b));
        }
    }
    let mut path_buf = [0u8; 256];
    let plen = anyos_std::env::get("PATH", &mut path_buf);
    if plen != u32::MAX {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..plen as usize]) {
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() { continue; }
                for (name, _) in list_dir_entries(dir) {
                    if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
                        matches.push(name);
                    }
                }
            }
        }
    }
    for (name, _) in list_dir_entries("/System/bin") {
        if name.starts_with(prefix) && !matches.iter().any(|m| *m == name) {
            matches.push(name);
        }
    }
    matches.sort();
    matches
}

/// Complete a file or directory path (argument position).
fn complete_path(word: &str, cwd: &str) -> Vec<String> {
    let (dir_prefix, file_prefix) = if let Some(slash_pos) = word.rfind('/') {
        (&word[..slash_pos + 1], &word[slash_pos + 1..])
    } else {
        ("", word)
    };
    let search_dir = if dir_prefix.is_empty() {
        String::from(cwd)
    } else if dir_prefix.starts_with('/') {
        let p = dir_prefix.trim_end_matches('/');
        if p.is_empty() { String::from("/") } else { String::from(p) }
    } else {
        if cwd == "/" {
            format!("/{}", dir_prefix.trim_end_matches('/'))
        } else {
            format!("{}/{}", cwd, dir_prefix.trim_end_matches('/'))
        }
    };
    let entries = list_dir_entries(&search_dir);
    let mut matches = Vec::new();
    for (name, is_dir) in entries {
        if name.starts_with(file_prefix) {
            let completion = if is_dir {
                format!("{}{}/", dir_prefix, name)
            } else {
                format!("{}{}", dir_prefix, name)
            };
            matches.push(completion);
        }
    }
    matches.sort();
    matches
}

/// Handle Tab key for autocompletion.
fn handle_tab(shell: &mut Shell, buf: &mut TerminalBuffer) {
    let before_cursor = &shell.input[..shell.cursor];
    let word_start = before_cursor.rfind(' ').map(|i| i + 1).unwrap_or(0);
    let word = String::from(&before_cursor[word_start..]);
    let is_command = !before_cursor[..word_start].contains(|c: char| c != ' ');

    // Strip redirect operators from the word so "< file" and "> file" complete paths
    let stripped = word.trim_start_matches(|c: char| c == '<' || c == '>');
    let prefix_len = word.len() - stripped.len(); // number of chars stripped

    let completions = if is_command {
        complete_command(&word)
    } else if prefix_len > 0 {
        // After redirect operator: always complete paths, using stripped word
        complete_path(stripped, &shell.cwd)
    } else {
        complete_path(&word, &shell.cwd)
    };

    if completions.is_empty() {
        return;
    }

    // When a redirect prefix was stripped, completions match against `stripped`
    let match_len = if prefix_len > 0 { stripped.len() } else { word.len() };

    if completions.len() == 1 {
        let completion = &completions[0];
        if completion.len() > match_len {
            let remaining = String::from(&completion[match_len..]);
            for ch in remaining.chars() {
                shell.insert_char(ch);
            }
        }
        if !completion.ends_with('/') {
            shell.insert_char(' ');
        }
        redraw_input_line(buf, shell);
    } else {
        let common = common_prefix(&completions);
        if common.len() > match_len {
            let remaining = String::from(&common[match_len..]);
            for ch in remaining.chars() {
                shell.insert_char(ch);
            }
        }
        buf.write_char('\n');
        let fg = app().active_profile.color_fg;
        for c in &completions {
            let is_dir = c.ends_with('/');
            let display = c.rsplit('/').nth(if is_dir { 1 } else { 0 }).unwrap_or(c);
            if is_dir {
                buf.current_fg = COLOR_TITLE; // blue for directories
                buf.write_str(display);
                buf.write_str("/  ");
            } else {
                buf.current_fg = fg;
                buf.write_str(display);
                buf.write_str("  ");
            }
        }
        buf.write_char('\n');
        buf.current_fg = COLOR_PROMPT;
        let prompt = shell.prompt();
        buf.write_str(&prompt);
        buf.current_fg = COLOR_FG;
        buf.write_str(&shell.input);
        let prompt_len = prompt.len();
        buf.cursor_col = prompt_len + shell.cursor;
    }
}

// ─── Theme command ───────────────────────────────────────────────────────────

fn cmd_theme(args: &str, buf: &mut TerminalBuffer) {
    let name = args.trim();
    let a = app();
    if name.is_empty() || name == "list" {
        buf.current_fg = a.active_profile.color_title;
        buf.write_str("Verfuegbare Themes:\n");
        buf.current_fg = a.active_profile.color_fg;
        for theme in THEMES {
            buf.write_str("  ");
            if a.theme_name == theme.name {
                buf.write_str("* ");
            } else {
                buf.write_str("  ");
            }
            buf.write_str(theme.name);
            buf.write_char('\n');
        }
        buf.write_str("\nUsage: theme <name>\n");
        return;
    }
    // Find theme (case-insensitive)
    let name_lower: String = name.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
    if let Some(theme) = THEMES.iter().find(|t| {
        let tn: String = t.name.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
        tn == name_lower
    }) {
        a.apply_theme(theme);
        if a.bg_alpha < 255 { a.apply_bg_alpha(); }
        buf.current_fg = a.active_profile.color_fg;
        buf.write_str("Theme: ");
        buf.write_str(theme.name);
        buf.write_char('\n');
        redraw();
    } else {
        buf.current_fg = a.active_profile.color_fg;
        buf.write_str("Unbekanntes Theme: ");
        buf.write_str(name);
        buf.write_str("\nVerwende 'theme list' fuer eine Liste.\n");
    }
}

fn cmd_opacity(args: &str, buf: &mut TerminalBuffer) {
    let val = args.trim();
    let a = app();
    if val.is_empty() {
        buf.current_fg = a.active_profile.color_fg;
        let pct = (a.bg_alpha as u32 * 100) / 255;
        buf.write_str(&format!("Opacity: {}% ({})\n", pct, a.bg_alpha));
        buf.write_str("Usage: opacity <0-100>\n");
        return;
    }
    if let Ok(pct) = val.parse::<u32>() {
        let pct = pct.min(100);
        a.bg_alpha = ((pct * 255 + 50) / 100) as u8;
        a.apply_bg_alpha();
        buf.current_fg = a.active_profile.color_fg;
        buf.write_str(&format!("Opacity: {}%\n", pct));
        redraw();
    } else {
        buf.current_fg = a.active_profile.color_fg;
        buf.write_str("Usage: opacity <0-100>\n");
    }
}

// ─── Profile command ─────────────────────────────────────────────────────────

/// Count leaf panes in a layout JSON.
fn count_panes(layout: &anyos_std::json::Value) -> usize {
    let typ = layout["type"].as_str().unwrap_or("leaf");
    match typ {
        "hsplit" => count_panes(&layout["left"]) + count_panes(&layout["right"]),
        "vsplit" => count_panes(&layout["top"]) + count_panes(&layout["bottom"]),
        _ => 1,
    }
}

fn cmd_profile(args: &str, buf: &mut TerminalBuffer) {
    let args = args.trim();
    let a = app();
    let prof = &a.active_profile;

    if args.is_empty() || args == "list" {
        buf.current_fg = prof.color_title;
        buf.write_str("Terminal Profile:\n");
        buf.current_fg = prof.color_fg;
        for p in a.profiles.iter() {
            buf.write_str("  ");
            if p.name == a.active_profile.name {
                buf.write_str("* ");
            } else {
                buf.write_str("  ");
            }
            buf.write_str(&p.name);
            let mut info = String::new();
            if p.is_default {
                info.push_str(" (Standard)");
            }
            if !p.saved_tabs.is_empty() {
                let tab_count = p.saved_tabs.len();
                let pane_count: usize = p.saved_tabs.iter().map(|t| count_panes(&t.layout)).sum();
                info.push_str(&format!(" [{} Tabs, {} Panes]", tab_count, pane_count));
            }
            if !info.is_empty() {
                buf.current_fg = prof.color_dim;
                buf.write_str(&info);
                buf.current_fg = prof.color_fg;
            }
            buf.write_char('\n');
        }
        buf.write_str("\n");
        buf.current_fg = prof.color_dim;
        buf.write_str("Befehle:\n");
        buf.write_str("  profile save [name]      Aktuelles Layout speichern\n");
        buf.write_str("  profile use <name>       Profil laden (stellt Tabs wieder her)\n");
        buf.write_str("  profile new <name>       Neues Profil erstellen\n");
        buf.write_str("  profile delete <name>    Profil loeschen\n");
        buf.write_str("  profile default <name>   Standardprofil setzen\n");
        buf.write_str("  profile set [key] [val]  Einstellung aendern\n");
        buf.current_fg = prof.color_fg;
        return;
    }

    let mut parts = args.splitn(2, ' ');
    let subcmd = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match subcmd {
        "use" => {
            if rest.is_empty() {
                buf.current_fg = prof.color_fg;
                buf.write_str("Usage: profile use <name>\n");
                return;
            }
            let a = app();
            if let Some(p) = a.profiles.iter().find(|p| p.name == rest) {
                let profile = p.clone();
                a.active_profile = profile.clone();
                a.set_font_size_all(profile.font_size);
                a.bg_alpha = profile.bg_alpha;
                a.theme_name = profile.theme_name.clone();
                if a.bg_alpha < 255 { a.apply_bg_alpha(); }
                // Restore tabs if the profile has saved layout
                if !profile.saved_tabs.is_empty() {
                    a.restore_tabs(&profile.saved_tabs);
                    a.update_pane_sizes();
                    update_tab_labels();
                }
                buf.current_fg = a.active_profile.color_fg;
                buf.write_str("Profil geladen: ");
                buf.write_str(rest);
                if !profile.saved_tabs.is_empty() {
                    buf.write_str(&format!(" ({} Tabs wiederhergestellt)", profile.saved_tabs.len()));
                }
                buf.write_char('\n');
                redraw();
            } else {
                buf.current_fg = prof.color_fg;
                buf.write_str("Profil nicht gefunden: ");
                buf.write_str(rest);
                buf.write_char('\n');
            }
        }
        "save" => {
            let a = app();
            let name = if rest.is_empty() { a.active_profile.name.as_str() } else { rest };
            // Snapshot current tabs into profile
            let saved_tabs = a.snapshot_tabs();
            a.active_profile.saved_tabs = saved_tabs;
            a.active_profile.font_size = a.active_font_size();
            a.active_profile.bg_alpha = a.bg_alpha;
            a.active_profile.theme_name = a.theme_name.clone();
            // Update or add to profiles list
            if let Some(p) = a.profiles.iter_mut().find(|p| p.name == name) {
                *p = a.active_profile.clone();
                p.name = String::from(name);
            } else {
                let mut new_p = a.active_profile.clone();
                new_p.name = String::from(name);
                a.profiles.push(new_p);
            }
            save_profiles(&a.profiles);
            buf.current_fg = a.active_profile.color_fg;
            buf.write_str("Profil gespeichert: ");
            buf.write_str(name);
            buf.write_str(&format!(" ({} Tabs, {} Panes)",
                a.active_profile.saved_tabs.len(),
                a.active_profile.saved_tabs.iter().map(|t| count_panes(&t.layout)).sum::<usize>()));
            buf.write_char('\n');
        }
        "new" => {
            if rest.is_empty() {
                buf.current_fg = prof.color_fg;
                buf.write_str("Usage: profile new <name>\n");
                return;
            }
            let a = app();
            if a.profiles.iter().any(|p| p.name == rest) {
                buf.current_fg = a.active_profile.color_fg;
                buf.write_str("Profil existiert bereits: ");
                buf.write_str(rest);
                buf.write_char('\n');
                return;
            }
            let mut new_p = Profile::default_profile();
            new_p.name = String::from(rest);
            new_p.is_default = false;
            // Snapshot current tabs into new profile
            new_p.saved_tabs = a.snapshot_tabs();
            a.profiles.push(new_p.clone());
            a.active_profile = new_p;
            save_profiles(&a.profiles);
            buf.current_fg = a.active_profile.color_fg;
            buf.write_str("Profil erstellt: ");
            buf.write_str(rest);
            buf.write_char('\n');
            redraw();
        }
        "delete" => {
            if rest.is_empty() {
                buf.current_fg = prof.color_fg;
                buf.write_str("Usage: profile delete <name>\n");
                return;
            }
            let a = app();
            if a.profiles.len() <= 1 {
                buf.current_fg = a.active_profile.color_fg;
                buf.write_str("Letztes Profil kann nicht geloescht werden.\n");
                return;
            }
            if let Some(pos) = a.profiles.iter().position(|p| p.name == rest) {
                a.profiles.remove(pos);
                if a.active_profile.name == rest {
                    a.active_profile = a.profiles[0].clone();
                    redraw();
                }
                save_profiles(&a.profiles);
                buf.current_fg = a.active_profile.color_fg;
                buf.write_str("Profil geloescht: ");
                buf.write_str(rest);
                buf.write_char('\n');
            } else {
                buf.current_fg = prof.color_fg;
                buf.write_str("Profil nicht gefunden: ");
                buf.write_str(rest);
                buf.write_char('\n');
            }
        }
        "default" => {
            if rest.is_empty() {
                buf.current_fg = prof.color_fg;
                buf.write_str("Usage: profile default <name>\n");
                return;
            }
            let a = app();
            let mut found = false;
            for p in a.profiles.iter_mut() {
                p.is_default = p.name == rest;
                if p.name == rest { found = true; }
            }
            if found {
                save_profiles(&a.profiles);
                buf.current_fg = a.active_profile.color_fg;
                buf.write_str("Standardprofil: ");
                buf.write_str(rest);
                buf.write_char('\n');
            } else {
                buf.current_fg = prof.color_fg;
                buf.write_str("Profil nicht gefunden: ");
                buf.write_str(rest);
                buf.write_char('\n');
            }
        }
        "set" => {
            let mut kv = rest.splitn(2, ' ');
            let key = kv.next().unwrap_or("");
            let val = kv.next().unwrap_or("").trim();
            if key.is_empty() || val.is_empty() {
                let a = app();
                let p = &a.active_profile;
                buf.current_fg = p.color_title;
                buf.write_str("Aktuelle Einstellungen:\n");
                buf.current_fg = p.color_fg;
                buf.write_str(&format!("  color_bg       {:08X}\n", p.color_bg));
                buf.write_str(&format!("  color_fg       {:08X}\n", p.color_fg));
                buf.write_str(&format!("  color_prompt   {:08X}\n", p.color_prompt));
                buf.write_str(&format!("  color_title    {:08X}\n", p.color_title));
                buf.write_str(&format!("  color_dim      {:08X}\n", p.color_dim));
                buf.write_str(&format!("  color_sel_bg   {:08X}\n", p.color_select_bg));
                buf.write_str(&format!("  color_sel_fg   {:08X}\n", p.color_select_fg));
                buf.write_str(&format!("  win_w          {}\n", p.win_w));
                buf.write_str(&format!("  win_h          {}\n", p.win_h));
                buf.write_str(&format!("  scrollback     {}\n", p.scrollback));
                buf.write_str(&format!("  tabs           {}\n", p.saved_tabs.len()));
                buf.write_str("\n");
                buf.current_fg = p.color_dim;
                buf.write_str("Usage: profile set <key> <value>\n");
                buf.current_fg = p.color_fg;
                return;
            }
            let a = app();
            let p = &mut a.active_profile;
            match key {
                "color_bg" => { if let Some(c) = parse_hex_color(val) { p.color_bg = c; } }
                "color_fg" => { if let Some(c) = parse_hex_color(val) { p.color_fg = c; } }
                "color_prompt" => { if let Some(c) = parse_hex_color(val) { p.color_prompt = c; } }
                "color_title" => { if let Some(c) = parse_hex_color(val) { p.color_title = c; } }
                "color_dim" => { if let Some(c) = parse_hex_color(val) { p.color_dim = c; } }
                "color_sel_bg" => { if let Some(c) = parse_hex_color(val) { p.color_select_bg = c; } }
                "color_sel_fg" => { if let Some(c) = parse_hex_color(val) { p.color_select_fg = c; } }
                "win_w" => { if let Ok(n) = val.parse::<u32>() { p.win_w = n; } }
                "win_h" => { if let Ok(n) = val.parse::<u32>() { p.win_h = n; } }
                "scrollback" => { if let Ok(n) = val.parse::<usize>() { p.scrollback = n; } }
                _ => {
                    buf.current_fg = p.color_fg;
                    buf.write_str("Unbekannte Einstellung: ");
                    buf.write_str(key);
                    buf.write_char('\n');
                    return;
                }
            }
            buf.current_fg = a.active_profile.color_fg;
            buf.write_str("Set ");
            buf.write_str(key);
            buf.write_str(" = ");
            buf.write_str(val);
            buf.write_char('\n');
            redraw();
        }
        _ => {
            buf.current_fg = prof.color_fg;
            buf.write_str("Unbekannter Befehl: ");
            buf.write_str(subcmd);
            buf.write_str("\nUsage: profile <list|use|save|new|delete|default|set>\n");
        }
    }
}

// ─── Rendering (Canvas-based) ────────────────────────────────────────────────

/// Check if a cell is a search match and return (is_match, is_current_match).
fn is_search_match(search: &SearchState, line_idx: usize, col: usize) -> (bool, bool) {
    if !search.active || search.matches.is_empty() { return (false, false); }
    for (i, &(row, start, end)) in search.matches.iter().enumerate() {
        if row == line_idx && col >= start && col < end {
            return (true, i == search.current_match);
        }
    }
    (false, false)
}

/// Detect if a position is part of a URL. Returns the URL bounds if found.
fn find_url_at(line: &[Cell], col: usize) -> Option<(usize, usize)> {
    // Simple URL detection: find http:// or https:// and extend to word boundary
    let line_str: String = line.iter().map(|c| c.ch).collect();
    let bytes = line_str.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    while pos < len {
        let rest = &bytes[pos..];
        let is_url = (rest.len() >= 7 && &rest[..7] == b"http://")
                  || (rest.len() >= 8 && &rest[..8] == b"https://");
        if is_url {
            let start = pos;
            let mut end = pos;
            while end < len && bytes[end] > b' ' && bytes[end] != b'"' && bytes[end] != b'\'' && bytes[end] != b'>' && bytes[end] != b')' {
                end += 1;
            }
            // Strip trailing punctuation
            while end > start && (bytes[end-1] == b'.' || bytes[end-1] == b',' || bytes[end-1] == b';') {
                end -= 1;
            }
            if col >= start && col < end {
                return Some((start, end));
            }
            pos = end;
        } else {
            pos += 1;
        }
    }
    None
}

/// Render a scrollbar on the right edge of a pane.
fn render_scrollbar(pixels: *mut u32, stride: u32, buf_h: u32, buf: &TerminalBuffer, rect: Rect) {
    let total_lines = buf.lines.len();
    if total_lines <= buf.visible_rows {
        // No scrollbar needed — all content fits
        // Just draw a thin track background
        let track_x = (rect.x + rect.w).saturating_sub(SCROLLBAR_WIDTH as u16);
        fill_rect_buf(pixels, stride, buf_h, track_x as i32, rect.y as i32, SCROLLBAR_WIDTH as u16, rect.h, SCROLLBAR_TRACK_COLOR);
        return;
    }

    let track_x = (rect.x + rect.w).saturating_sub(SCROLLBAR_WIDTH as u16);
    let track_h = rect.h as u32;

    // Draw track background
    fill_rect_buf(pixels, stride, buf_h, track_x as i32, rect.y as i32, SCROLLBAR_WIDTH as u16, rect.h, SCROLLBAR_TRACK_COLOR);

    // Calculate thumb size and position
    let thumb_h = ((buf.visible_rows as u32 * track_h) / total_lines as u32).max(SCROLLBAR_MIN_THUMB_H).min(track_h);
    let scrollable = total_lines - buf.visible_rows;
    let thumb_y = if scrollable > 0 {
        (buf.scroll_offset as u32 * (track_h - thumb_h)) / scrollable as u32
    } else {
        0
    };

    // Draw thumb with rounded appearance (2px margin on sides)
    let thumb_x = track_x as i32 + 2;
    let thumb_w = (SCROLLBAR_WIDTH as u16).saturating_sub(4);
    let thumb_py = rect.y as i32 + thumb_y as i32;
    fill_rect_buf(pixels, stride, buf_h, thumb_x, thumb_py, thumb_w, thumb_h as u16, SCROLLBAR_THUMB_COLOR);
}

/// Compute scrollbar geometry for hit-testing: returns (track_x, track_y, track_h, thumb_y, thumb_h).
fn scrollbar_geometry(buf: &TerminalBuffer, rect: Rect) -> (u16, u16, u32, u32, u32) {
    let track_x = (rect.x + rect.w).saturating_sub(SCROLLBAR_WIDTH as u16);
    let track_y = rect.y;
    let track_h = rect.h as u32;
    let total_lines = buf.lines.len();
    if total_lines <= buf.visible_rows {
        return (track_x, track_y, track_h, 0, track_h);
    }
    let thumb_h = ((buf.visible_rows as u32 * track_h) / total_lines as u32).max(SCROLLBAR_MIN_THUMB_H).min(track_h);
    let scrollable = total_lines - buf.visible_rows;
    let thumb_y = if scrollable > 0 {
        (buf.scroll_offset as u32 * (track_h - thumb_h)) / scrollable as u32
    } else {
        0
    };
    (track_x, track_y, track_h, thumb_y, thumb_h)
}

/// Render a single pane's terminal content into a Canvas pixel buffer.
fn render_pane_to_canvas(canvas: &anyui::Canvas, buf: &TerminalBuffer, rect: Rect, sel: Option<&Selection>, is_active: bool, font_size: u16, cell_w: u16, cell_h: u16) {
    let pixels = canvas.get_buffer();
    if pixels.is_null() { return; }
    let stride = canvas.get_stride();
    let buf_h = canvas.get_height();
    let a = app();
    let prof = &a.active_profile;
    let bg = prof.color_bg;
    let sel_bg = prof.color_select_bg;
    let sel_fg = prof.color_select_fg;

    // Clear pane background
    fill_rect_buf(pixels, stride, buf_h, rect.x as i32, rect.y as i32, rect.w, rect.h, bg);

    let start_row = buf.scroll_offset;
    let end_row = (start_row + buf.visible_rows).min(buf.lines.len());
    let sel_range = sel.map(|s| s.ordered());

    for (screen_y, line_idx) in (start_row..end_row).enumerate() {
        let line = &buf.lines[line_idx];
        let py = rect.y as i32 + TEXT_PAD as i32 + (screen_y as i32) * cell_h as i32;

        // Batch characters into runs of the same color+bg for efficient rendering
        // run_len = bytes in run_buf, run_cols = number of terminal columns
        let mut run_start: usize = 0;
        let mut run_fg: u32 = 0;
        let mut run_bg: u32 = 0;
        let mut run_attr: u8 = 0;
        let mut run_buf = [0u8; 512];
        let mut run_len: usize = 0;
        let mut run_cols: usize = 0;

        let mut col = 0usize;
        while col < buf.cols.min(line.len()) {
            let cell = &line[col];

            // Skip continuation cells from wide characters
            if cell.width == 0 {
                col += 1;
                continue;
            }

            let ch = cell.ch;
            let cw = cell.width as usize; // 1 or 2
            let mut fg = cell.fg;
            let mut cell_bg = cell.bg;
            let attr = cell.attr;

            // Apply reverse video attribute
            if attr & ATTR_REVERSE != 0 {
                core::mem::swap(&mut fg, &mut cell_bg);
                if cell_bg == 0 { cell_bg = bg; }
            }

            let selected = if let Some((ref s, ref e)) = sel_range {
                if line_idx > s.row && line_idx < e.row { true }
                else if line_idx == s.row && line_idx == e.row { col >= s.col && col <= e.col }
                else if line_idx == s.row { col >= s.col }
                else if line_idx == e.row { col <= e.col }
                else { false }
            } else { false };

            let (is_match, is_current) = is_search_match(&a.search, line_idx, col);

            if selected || is_match {
                // Flush any pending run
                if run_len > 0 {
                    let rw = (run_cols as u16) * cell_w;
                    if run_bg != 0 {
                        let rpx = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
                        fill_rect_buf(pixels, stride, buf_h, rpx, py, rw, cell_h, run_bg);
                    }
                    let px = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
                    canvas.draw_text(px, py, run_fg, FONT_ID_MONO, font_size, core::str::from_utf8(&run_buf[..run_len]).unwrap_or(""));
                    if run_attr & ATTR_UNDERLINE != 0 {
                        let uy = py + cell_h as i32 - 1;
                        fill_rect_buf(pixels, stride, buf_h, px, uy, rw, 1, run_fg);
                    }
                    if run_attr & ATTR_STRIKETHROUGH != 0 {
                        let sy = py + cell_h as i32 / 2;
                        fill_rect_buf(pixels, stride, buf_h, px, sy, rw, 1, run_fg);
                    }
                    run_len = 0;
                    run_cols = 0;
                }
                let highlight_bg = if selected { sel_bg }
                    else if is_current { COLOR_SEARCH_CURRENT_BG }
                    else { COLOR_SEARCH_BG };
                let highlight_fg = if selected { sel_fg } else { 0xFF000000 };
                let px = rect.x as i32 + TEXT_PAD as i32 + (col as i32) * cell_w as i32;
                let char_px_w = (cw as u16) * cell_w;
                fill_rect_buf(pixels, stride, buf_h, px, py, char_px_w, cell_h, highlight_bg);
                let mut tmp = [0u8; 8];
                let mut tlen = ch.encode_utf8(&mut tmp).len();
                if cell.combining != '\0' {
                    let clen = cell.combining.encode_utf8(&mut tmp[tlen..]).len();
                    tlen += clen;
                }
                canvas.draw_text(px, py, highlight_fg, FONT_ID_MONO, font_size, core::str::from_utf8(&tmp[..tlen]).unwrap_or(""));
            } else {
                let is_url = find_url_at(line, col).is_some() || cell.link_id != 0;
                let display_fg = if is_url { 0xFF5599FF } else { fg };
                let display_bg = if is_url { 0u32 } else { cell_bg };

                // Wide chars and chars with combining marks break runs (need individual draw)
                let is_special = cw == 2 || cell.combining != '\0';

                // Check if we need to break the run (different color, bg, attrs, or special char)
                if run_len > 0 && (display_fg != run_fg || display_bg != run_bg || attr != run_attr || is_special) {
                    // Flush run
                    let rw = (run_cols as u16) * cell_w;
                    if run_bg != 0 {
                        let rpx = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
                        fill_rect_buf(pixels, stride, buf_h, rpx, py, rw, cell_h, run_bg);
                    }
                    let px = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
                    canvas.draw_text(px, py, run_fg, FONT_ID_MONO, font_size, core::str::from_utf8(&run_buf[..run_len]).unwrap_or(""));
                    if run_attr & ATTR_UNDERLINE != 0 {
                        let uy = py + cell_h as i32 - 1;
                        fill_rect_buf(pixels, stride, buf_h, px, uy, rw, 1, run_fg);
                    }
                    if run_attr & ATTR_STRIKETHROUGH != 0 {
                        let sy = py + cell_h as i32 / 2;
                        fill_rect_buf(pixels, stride, buf_h, px, sy, rw, 1, run_fg);
                    }
                    run_len = 0;
                    run_cols = 0;
                }

                if is_special {
                    // Draw wide or combining character individually
                    let px = rect.x as i32 + TEXT_PAD as i32 + (col as i32) * cell_w as i32;
                    let char_px_w = (cw as u16) * cell_w;
                    if display_bg != 0 {
                        fill_rect_buf(pixels, stride, buf_h, px, py, char_px_w, cell_h, display_bg);
                    }
                    let mut tmp = [0u8; 8];
                    let mut tlen = ch.encode_utf8(&mut tmp).len();
                    if cell.combining != '\0' {
                        let clen = cell.combining.encode_utf8(&mut tmp[tlen..]).len();
                        tlen += clen;
                    }
                    canvas.draw_text(px, py, display_fg, FONT_ID_MONO, font_size, core::str::from_utf8(&tmp[..tlen]).unwrap_or(""));
                    if attr & ATTR_UNDERLINE != 0 {
                        let uy = py + cell_h as i32 - 1;
                        fill_rect_buf(pixels, stride, buf_h, px, uy, char_px_w, 1, display_fg);
                    }
                    if attr & ATTR_STRIKETHROUGH != 0 {
                        let sy = py + cell_h as i32 / 2;
                        fill_rect_buf(pixels, stride, buf_h, px, sy, char_px_w, 1, display_fg);
                    }
                } else {
                    if run_len == 0 {
                        run_start = col;
                        run_fg = display_fg;
                        run_bg = display_bg;
                        run_attr = attr;
                    }
                    // Encode character as UTF-8 into run buffer
                    let mut tmp = [0u8; 4];
                    let encoded = ch.encode_utf8(&mut tmp);
                    let elen = encoded.len();
                    if run_len + elen <= run_buf.len() {
                        run_buf[run_len..run_len + elen].copy_from_slice(&tmp[..elen]);
                        run_len += elen;
                        run_cols += 1;
                    }
                }

                // Draw underline for URLs
                if is_url {
                    let px = rect.x as i32 + TEXT_PAD as i32 + (col as i32) * cell_w as i32;
                    let uy = py + cell_h as i32 - 1;
                    let url_w = (cw as u16) * cell_w;
                    fill_rect_buf(pixels, stride, buf_h, px, uy, url_w, 1, 0xFF5599FF);
                }
            }
            col += cw;
        }

        // Flush remaining run
        if run_len > 0 {
            let rw = (run_cols as u16) * cell_w;
            if run_bg != 0 {
                let rpx = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
                fill_rect_buf(pixels, stride, buf_h, rpx, py, rw, cell_h, run_bg);
            }
            let px = rect.x as i32 + TEXT_PAD as i32 + (run_start as i32) * cell_w as i32;
            canvas.draw_text(px, py, run_fg, FONT_ID_MONO, font_size, core::str::from_utf8(&run_buf[..run_len]).unwrap_or(""));
            if run_attr & ATTR_UNDERLINE != 0 {
                let uy = py + cell_h as i32 - 1;
                fill_rect_buf(pixels, stride, buf_h, px, uy, rw, 1, run_fg);
            }
            if run_attr & ATTR_STRIKETHROUGH != 0 {
                let sy = py + cell_h as i32 / 2;
                fill_rect_buf(pixels, stride, buf_h, px, sy, rw, 1, run_fg);
            }
        }

        // For multiline selections: highlight empty area after line end
        if let Some((ref s, ref e)) = sel_range {
            if line_idx > s.row && line_idx < e.row && line.len() < buf.cols {
                let rx = rect.x as i32 + TEXT_PAD as i32 + (line.len() as i32) * cell_w as i32;
                let rw = ((buf.cols - line.len()) as u16) * cell_w;
                fill_rect_buf(pixels, stride, buf_h, rx, py, rw, cell_h, sel_bg);
            }
        }
    }

    // Draw cursor (only for active pane, and only if cursor is visible + blink phase)
    if is_active && buf.cursor_visible && a.cursor_blink_on {
        let cursor_screen_row = buf.cursor_row as i32 - buf.scroll_offset as i32;
        if cursor_screen_row >= 0 && (cursor_screen_row as usize) < buf.visible_rows {
            let cx = rect.x as i32 + TEXT_PAD as i32 + (buf.cursor_col as i32) * cell_w as i32;
            let cy = rect.y as i32 + TEXT_PAD as i32 + cursor_screen_row * cell_h as i32;
            fill_rect_buf(pixels, stride, buf_h, cx, cy, cell_w, cell_h, prof.color_fg);
        }
    }

    // Draw scrollbar on right edge
    render_scrollbar(pixels, stride, buf_h, buf, rect);
}

/// Render pane borders for splits into the canvas.
fn render_pane_borders_to_canvas(canvas: &anyui::Canvas, node: &PaneNode, area: Rect) {
    let pixels = canvas.get_buffer();
    if pixels.is_null() { return; }
    let stride = canvas.get_stride();
    let buf_h = canvas.get_height();

    match node {
        PaneNode::Leaf { .. } => {}
        PaneNode::HSplit { left, right, ratio } => {
            let left_w = ((area.w as f32) * ratio) as u16;
            let border_x = area.x + left_w;
            fill_rect_buf(pixels, stride, buf_h, border_x as i32, area.y as i32, PANE_BORDER, area.h, COLOR_PANE_BORDER);
            let right_w = area.w.saturating_sub(left_w + PANE_BORDER);
            render_pane_borders_to_canvas(canvas, left, Rect { x: area.x, y: area.y, w: left_w, h: area.h });
            render_pane_borders_to_canvas(canvas, right, Rect { x: area.x + left_w + PANE_BORDER, y: area.y, w: right_w, h: area.h });
        }
        PaneNode::VSplit { top, bottom, ratio } => {
            let top_h = ((area.h as f32) * ratio) as u16;
            let border_y = area.y + top_h;
            fill_rect_buf(pixels, stride, buf_h, area.x as i32, border_y as i32, area.w, PANE_BORDER, COLOR_PANE_BORDER);
            let bottom_h = area.h.saturating_sub(top_h + PANE_BORDER);
            render_pane_borders_to_canvas(canvas, top, Rect { x: area.x, y: area.y, w: area.w, h: top_h });
            render_pane_borders_to_canvas(canvas, bottom, Rect { x: area.x, y: area.y + top_h + PANE_BORDER, w: area.w, h: bottom_h });
        }
    }
}

/// Full render: all panes in active tab to the canvas.
fn render_to_canvas(app: &TerminalApp, canvas: &anyui::Canvas) {
    // Clear entire canvas
    canvas.clear(app.active_profile.color_bg);

    let area = app.content_area();
    let tab = &app.tabs[app.active_tab];
    let layout = tab.root.layout(area);

    let pixels = canvas.get_buffer();
    let stride = canvas.get_stride();
    let buf_h = canvas.get_height();

    // Track active pane's font metrics for search/status bars
    let mut active_cell_h: u16 = DEFAULT_CELL_H;
    let mut active_font_size: u16 = DEFAULT_FONT_SIZE;

    for (pane_id, rect) in &layout {
        if let Some(sess) = tab.root.find_session(*pane_id) {
            let is_active = *pane_id == tab.active_pane_id;
            if is_active {
                active_cell_h = sess.cell_h;
                active_font_size = sess.font_size;
            }
            render_pane_to_canvas(canvas, &sess.buf, *rect, sess.selection.as_ref(), is_active, sess.font_size, sess.cell_w, sess.cell_h);

            // Draw active pane border highlight
            if is_active && layout.len() > 1 && !pixels.is_null() {
                fill_rect_buf(pixels, stride, buf_h, rect.x as i32, rect.y as i32, rect.w, 1, COLOR_PANE_ACTIVE_BORDER);
                fill_rect_buf(pixels, stride, buf_h, rect.x as i32, (rect.y + rect.h - 1) as i32, rect.w, 1, COLOR_PANE_ACTIVE_BORDER);
                fill_rect_buf(pixels, stride, buf_h, rect.x as i32, rect.y as i32, 1, rect.h, COLOR_PANE_ACTIVE_BORDER);
                fill_rect_buf(pixels, stride, buf_h, (rect.x + rect.w - 1) as i32, rect.y as i32, 1, rect.h, COLOR_PANE_ACTIVE_BORDER);
            }
        }
    }

    // Pane borders
    render_pane_borders_to_canvas(canvas, &tab.root, area);

    // Draw search bar if active
    if app.search.active && !pixels.is_null() {
        let bar_h = active_cell_h + 4;
        let bar_y = (buf_h as u16).saturating_sub(bar_h);
        fill_rect_buf(pixels, stride, buf_h, 0, bar_y as i32, area.w, bar_h, 0xE0333333);
        let label = format!("Suche: {}  ({}/{})", app.search.query,
            if app.search.matches.is_empty() { 0 } else { app.search.current_match + 1 },
            app.search.matches.len());
        canvas.draw_text(4, bar_y as i32 + 2, 0xFFFFFFFF, FONT_ID_MONO, active_font_size, &label);
    }

    // Draw reverse search indicator if active
    if app.reverse_search.active && !pixels.is_null() {
        let bar_h = active_cell_h + 4;
        let bar_y = (buf_h as u16).saturating_sub(bar_h);
        fill_rect_buf(pixels, stride, buf_h, 0, bar_y as i32, area.w, bar_h, 0xE0333355);
        let result_str = app.reverse_search.result.as_deref().unwrap_or("");
        let label = format!("(reverse-i-search)`{}': {}", app.reverse_search.query, result_str);
        canvas.draw_text(4, bar_y as i32 + 2, 0xFFFFFFFF, FONT_ID_MONO, active_font_size, &label);
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

/// Helper: get the pane rect for a specific pane in the active tab.
fn get_pane_rect(app: &TerminalApp, pane_id: PaneId) -> Rect {
    let area = app.content_area();
    let tab = &app.tabs[app.active_tab];
    let layout = tab.root.layout(area);
    for (id, rect) in layout {
        if id == pane_id { return rect; }
    }
    area
}

/// Poll all sessions in the active tab for foreground process output.
fn poll_fg_processes(app: &mut TerminalApp) {
    let tab = &mut app.tabs[app.active_tab];
    let mut ids = Vec::new();
    tab.root.collect_pane_ids(&mut ids);

    for pane_id in ids {
        let sess = match tab.root.find_session_mut(pane_id) {
            Some(s) => s,
            None => continue,
        };

        if let Some(ref mut fp) = sess.fg_proc {
            let mut read_buf = [0u8; 512];
            if fp.pipe_id != 0 {
                loop {
                    let n = ipc::pipe_read(fp.pipe_id, &mut read_buf);
                    if n == 0 || n == u32::MAX { break; }
                    if let Ok(s) = core::str::from_utf8(&read_buf[..n as usize]) {
                        if let Some(ref mut redir) = fp.redirect {
                            write_redirect(redir, s);
                        } else {
                            sess.buf.current_fg = COLOR_FG;
                            sess.buf.write_str(s);
                        }
                    }
                    sess.dirty = true;
                }
            }

            let status = process::try_waitpid(fp.tid);
            if status == process::STOPPED {
                // Process was stopped externally (SIGTSTP from another source)
                // Move to stopped jobs list
                let tid = fp.tid;
                let pipe_id = fp.pipe_id;
                let stdin_pipe = fp.stdin_pipe;
                let extra_pipes = fp.extra_pipes.clone();
                let cmd = fp.command.clone();
                sess.fg_proc = None;
                sess.buf.write_str("\n");
                let job_id = sess.shell.next_job_id;
                sess.shell.next_job_id += 1;
                sess.shell.bg_jobs.push(BackgroundJob {
                    job_id,
                    tid,
                    command: cmd,
                    finished: false,
                    exit_code: 0,
                    stopped: true,
                    pipe_id,
                    stdin_pipe,
                    extra_pipes,
                });
                sess.buf.current_fg = COLOR_DIM;
                let msg = format!("[{}]  Stopped\n", job_id);
                sess.buf.write_str(&msg);
                sess.show_prompt();
                sess.dirty = true;
                continue;
            }
            if status != process::STILL_RUNNING {
                if fp.pipe_id != 0 {
                    loop {
                        let n = ipc::pipe_read(fp.pipe_id, &mut read_buf);
                        if n == 0 || n == u32::MAX { break; }
                        if let Ok(s) = core::str::from_utf8(&read_buf[..n as usize]) {
                            if let Some(ref mut redir) = fp.redirect {
                                write_redirect(redir, s);
                            } else {
                                sess.buf.current_fg = COLOR_FG;
                                sess.buf.write_str(s);
                            }
                        }
                    }
                }
                let pipe_id = fp.pipe_id;
                let stdin_pipe_id = fp.stdin_pipe;
                let extra_pipes: Vec<u32> = fp.extra_pipes.clone();
                let exit_code = status;
                sess.fg_proc = None;
                if pipe_id != 0 { ipc::pipe_close(pipe_id); }
                if stdin_pipe_id != 0 { ipc::pipe_close(stdin_pipe_id); }
                for &p in &extra_pipes { ipc::pipe_close(p); }

                if exit_code != 0 && exit_code != u32::MAX {
                    sess.buf.current_fg = COLOR_DIM;
                    let msg = format!("Process exited with code {}\n", exit_code);
                    sess.buf.write_str(&msg);
                }
                sess.show_prompt();
                sess.dirty = true;

                // Notify when command finishes (useful when window is in background)
                anyui::show_notification(
                    "Terminal",
                    "Befehl abgeschlossen",
                    None,
                    3000,
                );
            }
        }
    }
}

/// Handle keyboard input for the active session.
fn handle_session_key(sess: &mut Session, key_code: u32, char_val: u32, mods: u32) -> bool {
    // Returns false if the session wants to exit (shell "exit" command)

    // Ctrl+C: cancel foreground process, password prompt, or clear input
    if (mods & MOD_CTRL) != 0 && char_val == 'c' as u32 {
        if let Some(fp) = sess.fg_proc.take() {
            if fp.stdin_pipe != 0 { ipc::pipe_close(fp.stdin_pipe); }
            process::kill(fp.tid);
            if fp.pipe_id != 0 { ipc::pipe_close(fp.pipe_id); }
            for &p in &fp.extra_pipes { ipc::pipe_close(p); }
        }
        sess.su_pending_user = None;
        sess.su_password.clear();
        sess.buf.write_str("^C\n");
        sess.shell.input.clear();
        sess.shell.cursor = 0;
        sess.show_prompt();
        sess.dirty = true;
        return true;
    }

    // Ctrl+Z: suspend foreground process (SIGTSTP)
    if (mods & MOD_CTRL) != 0 && char_val == 'z' as u32 {
        if let Some(fp) = sess.fg_proc.take() {
            // Send SIGTSTP to stop the process
            process::send_signal(fp.tid, process::SIGTSTP);
            sess.buf.write_str("^Z\n");
            // Add to stopped jobs list
            let job_id = sess.shell.next_job_id;
            sess.shell.next_job_id += 1;
            sess.shell.bg_jobs.push(BackgroundJob {
                job_id,
                tid: fp.tid,
                command: fp.command.clone(),
                finished: false,
                exit_code: 0,
                stopped: true,
                pipe_id: fp.pipe_id,
                stdin_pipe: fp.stdin_pipe,
                extra_pipes: fp.extra_pipes,
            });
            let msg = format!("[{}]  Stopped\n", job_id);
            sess.buf.current_fg = COLOR_DIM;
            sess.buf.write_str(&msg);
            sess.show_prompt();
            sess.dirty = true;
        }
        return true;
    }

    if (mods & MOD_CTRL) != 0 && char_val == 'v' as u32 {
        let mut clip_buf = [0u8; 4096];
        let clip_len = anyui::clipboard_get(&mut clip_buf);
        if clip_len > 0 {
            if let Ok(text) = core::str::from_utf8(&clip_buf[..clip_len as usize]) {
                // If foreground process is running, forward paste (with bracketed paste if enabled)
                if let Some(ref fp) = sess.fg_proc {
                    if fp.stdin_pipe != 0 {
                        if sess.buf.bracketed_paste {
                            ipc::pipe_write(fp.stdin_pipe, b"\x1b[200~");
                        }
                        ipc::pipe_write(fp.stdin_pipe, text.as_bytes());
                        if sess.buf.bracketed_paste {
                            ipc::pipe_write(fp.stdin_pipe, b"\x1b[201~");
                        }
                    }
                } else if sess.su_pending_user.is_none() {
                    for c in text.chars() {
                        if c >= ' ' && (c as u32) < 128 { sess.shell.insert_char(c); }
                    }
                    redraw_input_line(&mut sess.buf, &sess.shell);
                    sess.dirty = true;
                }
            }
        }
        return true;
    }

    // Here-document collection mode
    if sess.heredoc.is_some() {
        match key_code {
            KEY_ENTER => {
                let input_line = sess.shell.input.clone();
                sess.buf.write_char('\n');
                sess.shell.input.clear();
                sess.shell.cursor = 0;

                let finished = {
                    let hd = sess.heredoc.as_mut().unwrap();
                    if input_line.trim() == hd.delimiter.as_str() {
                        true
                    } else {
                        hd.lines.push(input_line);
                        false
                    }
                };

                if finished {
                    let hd = sess.heredoc.take().unwrap();
                    let heredoc_text = hd.lines.join("\n");
                    // Execute the command with heredoc as stdin
                    let cmd = hd.command.trim();
                    if !cmd.is_empty() {
                        let mut cmd_parts = cmd.splitn(2, ' ');
                        let prog = cmd_parts.next().unwrap_or("");
                        let args_str = cmd_parts.next().unwrap_or("");
                        let path = if prog.starts_with('/') {
                            String::from(prog)
                        } else {
                            match resolve_from_path(prog) {
                                Some(p) => p,
                                None => format!("/System/bin/{}", prog),
                            }
                        };
                        let full_args = if args_str.is_empty() {
                            String::from(prog)
                        } else {
                            format!("{} {}", prog, args_str)
                        };
                        let pipe_name = format!("term:stdout:heredoc:{}", prog);
                        let pipe_id = ipc::pipe_create(&pipe_name);
                        let stdin_name = format!("term:stdin:heredoc:{}", prog);
                        let stdin_pipe = ipc::pipe_create(&stdin_name);
                        let tid = process::spawn_piped_full(&path, &full_args, pipe_id, stdin_pipe);
                        if tid != u32::MAX {
                            // Write heredoc content to stdin and close
                            ipc::pipe_write(stdin_pipe, heredoc_text.as_bytes());
                            ipc::pipe_write(stdin_pipe, b"\n");
                            ipc::pipe_close(stdin_pipe);
                            sess.fg_proc = Some(ForegroundProcess {
                                tid, pipe_id, stdin_pipe: 0,
                                extra_pipes: Vec::new(), redirect: None,
                                command: String::from(cmd),
                            });
                        } else {
                            ipc::pipe_close(pipe_id);
                            ipc::pipe_close(stdin_pipe);
                            sess.buf.current_fg = COLOR_FG;
                            sess.buf.write_str("Unknown command: ");
                            sess.buf.write_str(prog);
                            sess.buf.write_char('\n');
                            sess.show_prompt();
                        }
                    } else {
                        sess.show_prompt();
                    }
                } else {
                    // Show heredoc prompt (> )
                    sess.buf.current_fg = COLOR_DIM;
                    sess.buf.write_str("> ");
                    sess.buf.current_fg = COLOR_FG;
                }
                sess.dirty = true;
            }
            KEY_BACKSPACE => {
                sess.shell.backspace();
                sess.buf.backspace();
                sess.dirty = true;
            }
            _ => {
                if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                    let c = char_val as u8 as char;
                    if c >= ' ' {
                        sess.shell.insert_char(c);
                        sess.buf.write_char(c);
                        sess.dirty = true;
                    }
                }
            }
        }
        return true;
    }

    if sess.su_pending_user.is_some() {
        match key_code {
            KEY_ENTER => {
                sess.buf.write_char('\n');
                let username = sess.su_pending_user.take().unwrap();
                Shell::do_su(&username, &sess.su_password, &mut sess.buf);
                sess.su_password.clear();
                sess.show_prompt();
                sess.dirty = true;
            }
            KEY_BACKSPACE => {
                if !sess.su_password.is_empty() {
                    sess.su_password.pop();
                    sess.buf.backspace();
                    sess.dirty = true;
                }
            }
            _ => {
                if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                    let c = char_val as u8 as char;
                    if c >= ' ' {
                        sess.su_password.push(c);
                        sess.buf.write_char('*');
                        sess.dirty = true;
                    }
                }
            }
        }
        return true;
    }

    if sess.fg_proc.is_none() {
        match key_code {
            KEY_ENTER => {
                // Check for here-document before submit
                let trimmed_input = sess.shell.input.trim_matches(|c: char| c == ' ').to_string();
                if let Some(heredoc_pos) = trimmed_input.find("<<") {
                    let after = trimmed_input[heredoc_pos + 2..].trim();
                    let delimiter = after.split_whitespace().next().unwrap_or("")
                        .trim_matches('\'').trim_matches('"');
                    if !delimiter.is_empty() {
                        let command = trimmed_input[..heredoc_pos].trim().to_string();
                        sess.buf.write_char('\n');
                        // Show heredoc prompt
                        sess.buf.current_fg = COLOR_DIM;
                        sess.buf.write_str("> ");
                        sess.buf.current_fg = COLOR_FG;
                        sess.heredoc = Some(HereDocState {
                            delimiter: String::from(delimiter),
                            command,
                            lines: Vec::new(),
                        });
                        // Add to history
                        if sess.shell.history.last().map_or(true, |last| *last != trimmed_input) {
                            sess.shell.history.push(trimmed_input);
                        }
                        sess.shell.input.clear();
                        sess.shell.cursor = 0;
                        sess.dirty = true;
                        return true;
                    }
                }
                let (should_continue, new_fg, pending_su) = sess.shell.submit(&mut sess.buf);
                if !should_continue {
                    return false; // exit
                }
                if let Some(fp) = new_fg {
                    sess.fg_proc = Some(fp);
                } else if let Some(user) = pending_su {
                    sess.su_pending_user = Some(user);
                    sess.su_password.clear();
                } else {
                    sess.show_prompt();
                }
                sess.dirty = true;
            }
            KEY_BACKSPACE => {
                if sess.shell.cursor > 0 {
                    sess.shell.backspace();
                    sess.buf.backspace();
                    sess.dirty = true;
                }
            }
            KEY_UP => {
                sess.shell.history_up();
                redraw_input_line(&mut sess.buf, &sess.shell);
                sess.dirty = true;
            }
            KEY_DOWN => {
                sess.shell.history_down();
                redraw_input_line(&mut sess.buf, &sess.shell);
                sess.dirty = true;
            }
            KEY_LEFT => {
                if sess.shell.cursor > 0 {
                    sess.shell.cursor_left();
                    sess.buf.cursor_col -= 1;
                    sess.dirty = true;
                }
            }
            KEY_RIGHT => {
                if sess.shell.cursor < sess.shell.input.len() {
                    sess.shell.cursor_right();
                    sess.buf.cursor_col += 1;
                    sess.dirty = true;
                }
            }
            KEY_HOME => {
                if sess.shell.cursor > 0 {
                    sess.buf.cursor_col -= sess.shell.cursor;
                    sess.shell.cursor_home();
                    sess.dirty = true;
                }
            }
            KEY_END => {
                if sess.shell.cursor < sess.shell.input.len() {
                    sess.buf.cursor_col += sess.shell.input.len() - sess.shell.cursor;
                    sess.shell.cursor_end();
                    sess.dirty = true;
                }
            }
            KEY_DELETE => {
                if sess.shell.cursor < sess.shell.input.len() {
                    sess.shell.delete_at_cursor();
                    let row = sess.buf.cursor_row;
                    let col = sess.buf.cursor_col;
                    if row < sess.buf.lines.len() && col < sess.buf.lines[row].len() {
                        sess.buf.lines[row].remove(col);
                    }
                    sess.dirty = true;
                }
            }
            KEY_TAB => {
                handle_tab(&mut sess.shell, &mut sess.buf);
                sess.dirty = true;
            }
            _ => {
                if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                    let c = char_val as u8 as char;
                    if c >= ' ' {
                        let at_end = sess.shell.cursor >= sess.shell.input.len();
                        sess.shell.insert_char(c);
                        if at_end {
                            sess.buf.write_char(c);
                        } else {
                            sess.buf.ensure_line(sess.buf.cursor_row);
                            let row = sess.buf.cursor_row;
                            let col = sess.buf.cursor_col;
                            let color = sess.buf.current_fg;
                            sess.buf.lines[row].insert(col, Cell { ch: c, fg: color, bg: 0, attr: 0, width: 1, combining: '\0', link_id: 0 });
                            sess.buf.cursor_col += 1;
                        }
                        sess.dirty = true;
                    }
                }
            }
        }
    } else {
        // Foreground process running — forward keyboard input to its stdin pipe
        if let Some(ref fp) = sess.fg_proc {
            if fp.stdin_pipe != 0 {
                match key_code {
                    KEY_ENTER => { ipc::pipe_write(fp.stdin_pipe, b"\n"); }
                    KEY_BACKSPACE => { ipc::pipe_write(fp.stdin_pipe, &[0x7f]); }
                    KEY_TAB => { ipc::pipe_write(fp.stdin_pipe, b"\t"); }
                    KEY_ESCAPE => { ipc::pipe_write(fp.stdin_pipe, b"\x1b"); }
                    KEY_UP => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[A"); }
                    KEY_DOWN => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[B"); }
                    KEY_RIGHT => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[C"); }
                    KEY_LEFT => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[D"); }
                    KEY_HOME => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[H"); }
                    KEY_END => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[F"); }
                    KEY_DELETE => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[3~"); }
                    KEY_PAGE_UP => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[5~"); }
                    KEY_PAGE_DOWN => { ipc::pipe_write(fp.stdin_pipe, b"\x1b[6~"); }
                    _ => {
                        if char_val > 0 && char_val < 128 {
                            let c = char_val as u8;
                            if (mods & MOD_CTRL) != 0 {
                                let ctrl_code = if c >= b'a' && c <= b'z' { c - b'a' + 1 }
                                    else if c >= b'A' && c <= b'Z' { c - b'A' + 1 }
                                    else { 0 };
                                if ctrl_code > 0 { ipc::pipe_write(fp.stdin_pipe, &[ctrl_code]); }
                            } else if c >= b' ' {
                                ipc::pipe_write(fp.stdin_pipe, &[c]);
                            }
                        }
                    }
                    // (Note: local echo is NOT done here — foreground processes
                    //  handle their own echo via stdout pipe, which poll_fg_processes reads.)
                }
            }
        }
    }
    true
}

// ─── Global State (anyui singleton pattern) ──────────────────────────────────

anyos_std::global_app_state!(TerminalApp);
static mut CANVAS: Option<anyui::Canvas> = None;
static mut TAB_BAR: Option<anyui::TabBar> = None;

fn get_canvas() -> anyui::Canvas {
    unsafe { CANVAS.unwrap() }
}

fn get_tab_bar() -> anyui::TabBar {
    unsafe { TAB_BAR.unwrap() }
}

fn redraw() {
    let a = app();
    let canvas = get_canvas();
    render_to_canvas(a, &canvas);
}

/// Perform search in the active session's buffer (separate borrow to avoid conflicts).
fn do_search_in_active_session() {
    let a = app();
    let pane_id = a.tabs[a.active_tab].active_pane_id;
    // Get lines reference from the active session
    let tab = &a.tabs[a.active_tab];
    if let Some(sess) = tab.root.find_session(pane_id) {
        // Inline search to avoid borrow conflict
        let query = a.search.query.clone();
        let a = app();
        a.search.matches.clear();
        a.search.current_match = 0;
        if query.is_empty() { return; }
        let q = query.as_bytes();
        let pane_id = a.tabs[a.active_tab].active_pane_id;
        if let Some(sess) = a.tabs[a.active_tab].root.find_session(pane_id) {
            for (line_idx, line) in sess.buf.lines.iter().enumerate() {
                let line_len = line.len();
                if line_len < q.len() { continue; }
                for col in 0..=line_len - q.len() {
                    let mut found = true;
                    for (i, &qb) in q.iter().enumerate() {
                        let ch = line[col + i].ch as u8;
                        let a = if qb >= b'A' && qb <= b'Z' { qb + 32 } else { qb };
                        let b = if ch >= b'A' && ch <= b'Z' { ch + 32 } else { ch };
                        if a != b { found = false; break; }
                    }
                    if found {
                        a.search.matches.push((line_idx, col, col + q.len()));
                    }
                }
            }
        }
    }
}

/// Perform reverse search in the active session's history.
fn do_reverse_search_in_active_session() {
    let a = app();
    let query = a.reverse_search.query.clone();
    a.reverse_search.result = None;
    a.reverse_search.result_index = None;
    if query.is_empty() { return; }
    let pane_id = a.tabs[a.active_tab].active_pane_id;
    if let Some(sess) = a.tabs[a.active_tab].root.find_session(pane_id) {
        let q_lower: String = query.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
        for (i, entry) in sess.shell.history.iter().enumerate().rev() {
            let entry_lower: String = entry.chars().map(|c| if c >= 'A' && c <= 'Z' { (c as u8 + 32) as char } else { c }).collect();
            if entry_lower.contains(q_lower.as_str()) {
                let a = app();
                a.reverse_search.result = Some(entry.clone());
                a.reverse_search.result_index = Some(i);
                return;
            }
        }
    }
}

fn update_tab_labels() {
    let a = app();
    let mut labels = String::new();
    for (i, tab) in a.tabs.iter().enumerate() {
        if i > 0 { labels.push('|'); }
        labels.push_str(&tab.title);
    }
    let tab_bar = get_tab_bar();
    tab_bar.set_text(&labels);
    tab_bar.set_state(a.active_tab as u32);
}

/// Reconstruct a Window handle from a raw ID.
/// Safe because Window is a #[repr(transparent)] wrapper around a u32.
fn window_from_id(id: u32) -> anyui::Window {
    unsafe { core::mem::transmute(id) }
}

fn destroy_window(id: u32) {
    window_from_id(id).destroy();
}

fn show_rename_tab_dialog(win_id: u32) {
    let a = app();
    let current_title = a.tabs[a.active_tab].title.clone();

    let dlg = anyui::Window::new_with_flags(
        "Tab umbenennen", -1, -1, 320, 120,
        anyui::WIN_FLAG_NOT_RESIZABLE
        | anyui::WIN_FLAG_NO_MINIMIZE
        | anyui::WIN_FLAG_NO_MAXIMIZE,
    );
    dlg.set_modal(&window_from_id(win_id));

    let label = anyui::Label::new("Neuer Tab-Name:");
    label.set_position(10, 10);
    label.set_size(300, 20);
    dlg.add(&label);

    let field = anyui::TextField::new();
    field.set_position(10, 36);
    field.set_size(300, 28);
    field.set_text(&current_title);
    field.select_all();
    dlg.add(&field);

    let btn_ok = anyui::Button::new("OK");
    btn_ok.set_position(140, 76);
    btn_ok.set_size(80, 28);
    dlg.add(&btn_ok);

    let btn_cancel = anyui::Button::new("Abbrechen");
    btn_cancel.set_position(230, 76);
    btn_cancel.set_size(80, 28);
    dlg.add(&btn_cancel);

    let dlg_id = dlg.id();
    let field_id = field.id();

    let apply_rename = move || {
        let mut buf = [0u8; 256];
        let len = anyui::Control::from_id(field_id).get_text(&mut buf);
        if len > 0 && len != u32::MAX {
            if let Ok(name) = core::str::from_utf8(&buf[..len as usize]) {
                let name = name.trim();
                if !name.is_empty() {
                    let a = app();
                    a.tabs[a.active_tab].title = String::from(name);
                    update_tab_labels();
                }
            }
        }
        destroy_window(dlg_id);
    };

    let apply = apply_rename.clone();
    btn_ok.on_click(move |_| { apply(); });

    field.on_submit(move |_| { apply_rename(); });

    btn_cancel.on_click(move |_| {
        destroy_window(dlg_id);
    });

    dlg.on_close(move |_| {
        destroy_window(dlg_id);
    });
}

// ─── Settings Dialog ─────────────────────────────────────────────────────────

/// Helper: read text from a control into a String.
fn read_ctrl_text(id: u32) -> String {
    let mut buf = [0u8; 512];
    let len = anyui::Control::from_id(id).get_text(&mut buf);
    if len > 0 && len != u32::MAX {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return String::from(s.trim());
        }
    }
    String::new()
}

/// Build pipe-separated list of alias names for a DropDown.
fn build_alias_items(aliases: &[(String, String)]) -> String {
    let mut s = String::new();
    for (i, (name, _)) in aliases.iter().enumerate() {
        if i > 0 { s.push('|'); }
        s.push_str(name);
    }
    s
}

/// Build pipe-separated list of environment variables for a DropDown.
fn build_env_items() -> (String, Vec<(String, String)>) {
    let mut buf = [0u8; 4096];
    let n = anyos_std::env::list(&mut buf);
    let mut entries = Vec::new();
    let mut items = String::new();
    if n > 0 && n != u32::MAX {
        let data = &buf[..n as usize];
        for entry in data.split(|&b| b == 0) {
            if entry.is_empty() { continue; }
            if let Ok(s) = core::str::from_utf8(entry) {
                if let Some(eq) = s.find('=') {
                    let key = &s[..eq];
                    let val = &s[eq + 1..];
                    if !items.is_empty() { items.push('|'); }
                    items.push_str(key);
                    entries.push((String::from(key), String::from(val)));
                }
            }
        }
    }
    (items, entries)
}

/// Build pipe-separated profile names for a DropDown.
fn build_profile_items(profiles: &[Profile]) -> String {
    let mut s = String::new();
    for (i, p) in profiles.iter().enumerate() {
        if i > 0 { s.push('|'); }
        s.push_str(&p.name);
        if p.is_default { s.push_str(" *"); }
    }
    s
}

/// Build pipe-separated theme names for a DropDown.
fn build_theme_items(current: &str) -> (String, usize) {
    let mut s = String::new();
    let mut sel = 0;
    for (i, t) in THEMES.iter().enumerate() {
        if i > 0 { s.push('|'); }
        s.push_str(t.name);
        if t.name == current { sel = i; }
    }
    (s, sel)
}

fn open_settings_dialog(win_id: u32) {
    let a = app();
    let dlg_w = 620;
    let dlg_h = 480;

    let dlg = anyui::Window::new_with_flags(
        "Einstellungen", -1, -1, dlg_w, dlg_h,
        anyui::WIN_FLAG_NOT_RESIZABLE
        | anyui::WIN_FLAG_NO_MINIMIZE
        | anyui::WIN_FLAG_NO_MAXIMIZE,
    );
    dlg.set_modal(&window_from_id(win_id));
    let dlg_id = dlg.id();

    // ── Tab selector (top) ──
    let tabs = anyui::SegmentedControl::new("Allgemein|Themes|Profile|Aliase|Umgebung");
    tabs.set_dock(anyui::DOCK_TOP);
    tabs.set_size(dlg_w, 32);
    dlg.add(&tabs);

    // ── Bottom bar with OK button ──
    let bottom_bar = anyui::View::new();
    bottom_bar.set_dock(anyui::DOCK_BOTTOM);
    bottom_bar.set_size(dlg_w, 44);
    bottom_bar.set_color(0xFF252525);

    let btn_ok = anyui::Button::new("OK");
    btn_ok.set_position((dlg_w as i32) - 90, 8);
    btn_ok.set_size(80, 28);
    bottom_bar.add(&btn_ok);

    dlg.add(&bottom_bar);

    // ════════════════════════════════════════════════════════════════════════
    // Panel 1: Allgemein
    // ════════════════════════════════════════════════════════════════════════
    let p_general = anyui::View::new();
    p_general.set_dock(anyui::DOCK_FILL);
    p_general.set_padding(16, 16, 16, 16);

    let mut y = 12;
    let lbl_w = 140u32;
    let field_w = 200u32;

    // Font size
    let lbl_font = anyui::Label::new("Schriftgroesse:");
    lbl_font.set_position(16, y);
    lbl_font.set_size(lbl_w, 24);
    p_general.add(&lbl_font);

    let tf_font = anyui::TextField::new();
    tf_font.set_position(16 + lbl_w as i32, y);
    tf_font.set_size(80, 26);
    tf_font.set_text(&format!("{}", a.active_font_size()));
    p_general.add(&tf_font);

    let lbl_font_hint = anyui::Label::new("(8-32)");
    lbl_font_hint.set_position(16 + lbl_w as i32 + 90, y + 2);
    lbl_font_hint.set_size(60, 20);
    lbl_font_hint.set_text_color(0xFF888888);
    p_general.add(&lbl_font_hint);
    y += 36;

    // Opacity
    let lbl_opacity = anyui::Label::new("Transparenz:");
    lbl_opacity.set_position(16, y);
    lbl_opacity.set_size(lbl_w, 24);
    p_general.add(&lbl_opacity);

    let opacity_pct = (a.bg_alpha as u32 * 100) / 255;
    let sl_opacity = anyui::Slider::new(opacity_pct);
    sl_opacity.set_position(16 + lbl_w as i32, y);
    sl_opacity.set_size(field_w, 26);
    p_general.add(&sl_opacity);

    let lbl_opacity_val = anyui::Label::new(&format!("{}%", opacity_pct));
    lbl_opacity_val.set_position(16 + lbl_w as i32 + field_w as i32 + 8, y + 2);
    lbl_opacity_val.set_size(50, 20);
    p_general.add(&lbl_opacity_val);

    let lbl_opac_id = lbl_opacity_val.id();
    sl_opacity.on_value_changed(move |e| {
        anyui::Control::from_id(lbl_opac_id).set_text(&format!("{}%", e.value));
    });
    y += 36;

    // Scrollback
    let lbl_scroll = anyui::Label::new("Scrollback:");
    lbl_scroll.set_position(16, y);
    lbl_scroll.set_size(lbl_w, 24);
    p_general.add(&lbl_scroll);

    let tf_scroll = anyui::TextField::new();
    tf_scroll.set_position(16 + lbl_w as i32, y);
    tf_scroll.set_size(100, 26);
    tf_scroll.set_text(&format!("{}", a.active_profile.scrollback));
    p_general.add(&tf_scroll);

    let lbl_scroll_hint = anyui::Label::new("Zeilen");
    lbl_scroll_hint.set_position(16 + lbl_w as i32 + 110, y + 2);
    lbl_scroll_hint.set_size(60, 20);
    lbl_scroll_hint.set_text_color(0xFF888888);
    p_general.add(&lbl_scroll_hint);
    y += 36;

    // Cursor blink
    let lbl_blink = anyui::Label::new("Cursor-Blinken:");
    lbl_blink.set_position(16, y);
    lbl_blink.set_size(lbl_w, 24);
    p_general.add(&lbl_blink);

    let tf_blink = anyui::TextField::new();
    tf_blink.set_position(16 + lbl_w as i32, y);
    tf_blink.set_size(80, 26);
    tf_blink.set_text(&format!("{}", a.active_profile.cursor_blink_ms));
    p_general.add(&tf_blink);

    let lbl_blink_hint = anyui::Label::new("ms (0=aus)");
    lbl_blink_hint.set_position(16 + lbl_w as i32 + 90, y + 2);
    lbl_blink_hint.set_size(100, 20);
    lbl_blink_hint.set_text_color(0xFF888888);
    p_general.add(&lbl_blink_hint);
    y += 36;

    // Window size
    let lbl_win = anyui::Label::new("Fenstergroesse:");
    lbl_win.set_position(16, y);
    lbl_win.set_size(lbl_w, 24);
    p_general.add(&lbl_win);

    let tf_win_w = anyui::TextField::new();
    tf_win_w.set_position(16 + lbl_w as i32, y);
    tf_win_w.set_size(80, 26);
    tf_win_w.set_text(&format!("{}", a.active_profile.win_w));
    p_general.add(&tf_win_w);

    let lbl_x = anyui::Label::new("x");
    lbl_x.set_position(16 + lbl_w as i32 + 86, y + 2);
    lbl_x.set_size(16, 20);
    p_general.add(&lbl_x);

    let tf_win_h = anyui::TextField::new();
    tf_win_h.set_position(16 + lbl_w as i32 + 104, y);
    tf_win_h.set_size(80, 26);
    tf_win_h.set_text(&format!("{}", a.active_profile.win_h));
    p_general.add(&tf_win_h);
    y += 46;

    // Color fields
    let color_fields = [
        ("Hintergrund:", a.active_profile.color_bg),
        ("Vordergrund:", a.active_profile.color_fg),
        ("Prompt:", a.active_profile.color_prompt),
        ("Titel:", a.active_profile.color_title),
        ("Auswahl Hgr:", a.active_profile.color_select_bg),
        ("Auswahl Text:", a.active_profile.color_select_fg),
    ];

    let mut color_field_ids: Vec<u32> = Vec::new();
    for (label, color) in &color_fields {
        let lbl = anyui::Label::new(label);
        lbl.set_position(16, y);
        lbl.set_size(lbl_w, 24);
        p_general.add(&lbl);

        let preview = anyui::View::new();
        preview.set_position(16 + lbl_w as i32, y + 2);
        preview.set_size(20, 20);
        preview.set_color(*color);
        p_general.add(&preview);

        let tf = anyui::TextField::new();
        tf.set_position(16 + lbl_w as i32 + 28, y);
        tf.set_size(120, 26);
        tf.set_text(&format!("{:08X}", color));
        p_general.add(&tf);

        color_field_ids.push(tf.id());
        y += 30;
    }

    dlg.add(&p_general);

    // ════════════════════════════════════════════════════════════════════════
    // Panel 2: Themes
    // ════════════════════════════════════════════════════════════════════════
    let p_themes = anyui::View::new();
    p_themes.set_dock(anyui::DOCK_FILL);
    p_themes.set_padding(16, 16, 16, 16);

    let lbl_theme = anyui::Label::new("Theme waehlen:");
    lbl_theme.set_position(16, 12);
    lbl_theme.set_size(200, 24);
    p_themes.add(&lbl_theme);

    let (theme_items, theme_sel) = build_theme_items(&a.theme_name);
    let dd_theme = anyui::DropDown::new(&theme_items);
    dd_theme.set_position(16, 40);
    dd_theme.set_size(250, 28);
    dd_theme.set_selected_index(theme_sel as u32);
    p_themes.add(&dd_theme);

    // Theme preview panel
    let preview_box = anyui::View::new();
    preview_box.set_position(16, 80);
    preview_box.set_size(560, 200);
    let preview_theme = &THEMES[theme_sel];
    preview_box.set_color(preview_theme.bg);
    p_themes.add(&preview_box);

    let preview_lbl1 = anyui::Label::new("user@anyos /> ls -la");
    preview_lbl1.set_position(12, 12);
    preview_lbl1.set_size(500, 20);
    preview_lbl1.set_text_color(preview_theme.prompt);
    preview_box.add(&preview_lbl1);

    let preview_lbl2 = anyui::Label::new("drwxr-xr-x  root  System");
    preview_lbl2.set_position(12, 36);
    preview_lbl2.set_size(500, 20);
    preview_lbl2.set_text_color(preview_theme.fg);
    preview_box.add(&preview_lbl2);

    let preview_lbl3 = anyui::Label::new("drwxr-xr-x  root  Applications");
    preview_lbl3.set_position(12, 56);
    preview_lbl3.set_size(500, 20);
    preview_lbl3.set_text_color(preview_theme.fg);
    preview_box.add(&preview_lbl3);

    let preview_lbl4 = anyui::Label::new("-- Ausgewaehlter Text --");
    preview_lbl4.set_position(12, 86);
    preview_lbl4.set_size(200, 20);
    preview_lbl4.set_text_color(preview_theme.select_fg);
    preview_lbl4.set_color(preview_theme.select_bg);
    preview_box.add(&preview_lbl4);

    let preview_lbl5 = anyui::Label::new("Gedimmter Text");
    preview_lbl5.set_position(12, 116);
    preview_lbl5.set_size(200, 20);
    preview_lbl5.set_text_color(preview_theme.dim);
    preview_box.add(&preview_lbl5);

    let prev_box_id = preview_box.id();
    let prev1_id = preview_lbl1.id();
    let prev2_id = preview_lbl2.id();
    let prev3_id = preview_lbl3.id();
    let prev4_id = preview_lbl4.id();
    let prev5_id = preview_lbl5.id();

    dd_theme.on_selection_changed(move |e| {
        let idx = e.index as usize;
        if idx < THEMES.len() {
            let t = &THEMES[idx];
            anyui::Control::from_id(prev_box_id).set_color(t.bg);
            anyui::Control::from_id(prev1_id).set_text_color(t.prompt);
            anyui::Control::from_id(prev2_id).set_text_color(t.fg);
            anyui::Control::from_id(prev3_id).set_text_color(t.fg);
            anyui::Control::from_id(prev4_id).set_text_color(t.select_fg);
            anyui::Control::from_id(prev4_id).set_color(t.select_bg);
            anyui::Control::from_id(prev5_id).set_text_color(t.dim);
        }
    });

    dlg.add(&p_themes);

    // ════════════════════════════════════════════════════════════════════════
    // Panel 3: Profile
    // ════════════════════════════════════════════════════════════════════════
    let p_profiles = anyui::View::new();
    p_profiles.set_dock(anyui::DOCK_FILL);
    p_profiles.set_padding(16, 16, 16, 16);

    let lbl_prof = anyui::Label::new("Profile verwalten:");
    lbl_prof.set_position(16, 12);
    lbl_prof.set_size(200, 24);
    p_profiles.add(&lbl_prof);

    let prof_items = build_profile_items(&a.profiles);
    let dd_prof = anyui::DropDown::new(&prof_items);
    dd_prof.set_position(16, 40);
    dd_prof.set_size(300, 28);
    // Select active profile
    if let Some(idx) = a.profiles.iter().position(|p| p.name == a.active_profile.name) {
        dd_prof.set_selected_index(idx as u32);
    }
    p_profiles.add(&dd_prof);

    let btn_prof_use = anyui::Button::new("Laden");
    btn_prof_use.set_position(16, 78);
    btn_prof_use.set_size(90, 28);
    p_profiles.add(&btn_prof_use);

    let btn_prof_save = anyui::Button::new("Speichern");
    btn_prof_save.set_position(116, 78);
    btn_prof_save.set_size(90, 28);
    p_profiles.add(&btn_prof_save);

    let btn_prof_default = anyui::Button::new("Standard");
    btn_prof_default.set_position(216, 78);
    btn_prof_default.set_size(90, 28);
    p_profiles.add(&btn_prof_default);

    let btn_prof_delete = anyui::Button::new("Loeschen");
    btn_prof_delete.set_position(316, 78);
    btn_prof_delete.set_size(90, 28);
    p_profiles.add(&btn_prof_delete);

    // New profile
    let lbl_new_prof = anyui::Label::new("Neues Profil:");
    lbl_new_prof.set_position(16, 120);
    lbl_new_prof.set_size(100, 24);
    p_profiles.add(&lbl_new_prof);

    let tf_new_prof = anyui::TextField::new();
    tf_new_prof.set_position(120, 118);
    tf_new_prof.set_size(200, 28);
    tf_new_prof.set_placeholder("Profilname");
    p_profiles.add(&tf_new_prof);

    let btn_prof_new = anyui::Button::new("Erstellen");
    btn_prof_new.set_position(330, 118);
    btn_prof_new.set_size(90, 28);
    p_profiles.add(&btn_prof_new);

    // Status label for profile operations
    let lbl_prof_status = anyui::Label::new("");
    lbl_prof_status.set_position(16, 158);
    lbl_prof_status.set_size(550, 24);
    lbl_prof_status.set_text_color(0xFF88CC88);
    p_profiles.add(&lbl_prof_status);

    let dd_prof_id = dd_prof.id();
    let tf_new_prof_id = tf_new_prof.id();
    let lbl_prof_status_id = lbl_prof_status.id();

    // Profile: Load
    btn_prof_use.on_click(move |_| {
        let a = app();
        let idx = anyui::Control::from_id(dd_prof_id).get_state() as usize;
        if idx < a.profiles.len() {
            let profile = a.profiles[idx].clone();
            a.active_profile = profile.clone();
            a.set_font_size_all(profile.font_size);
            a.bg_alpha = profile.bg_alpha;
            a.theme_name = profile.theme_name.clone();
            if a.bg_alpha < 255 { a.apply_bg_alpha(); }
            if !profile.saved_tabs.is_empty() {
                a.restore_tabs(&profile.saved_tabs);
                a.update_pane_sizes();
                update_tab_labels();
            }
            anyui::Control::from_id(lbl_prof_status_id).set_text(&format!("Profil geladen: {}", profile.name));
            redraw();
        }
    });

    // Profile: Save current
    btn_prof_save.on_click(move |_| {
        let a = app();
        let idx = anyui::Control::from_id(dd_prof_id).get_state() as usize;
        if idx < a.profiles.len() {
            let name = a.profiles[idx].name.clone();
            let saved_tabs = a.snapshot_tabs();
            a.active_profile.saved_tabs = saved_tabs;
            a.active_profile.font_size = a.active_font_size();
            a.active_profile.bg_alpha = a.bg_alpha;
            a.active_profile.theme_name = a.theme_name.clone();
            a.profiles[idx] = a.active_profile.clone();
            a.profiles[idx].name = name.clone();
            save_profiles(&a.profiles);
            anyui::Control::from_id(lbl_prof_status_id).set_text(&format!("Profil gespeichert: {}", name));
        }
    });

    // Profile: Set default
    btn_prof_default.on_click(move |_| {
        let a = app();
        let idx = anyui::Control::from_id(dd_prof_id).get_state() as usize;
        if idx < a.profiles.len() {
            let name = a.profiles[idx].name.clone();
            for p in a.profiles.iter_mut() {
                p.is_default = false;
            }
            a.profiles[idx].is_default = true;
            save_profiles(&a.profiles);
            let items = build_profile_items(&a.profiles);
            anyui::Control::from_id(dd_prof_id).set_text(&items);
            anyui::Control::from_id(lbl_prof_status_id).set_text(&format!("Standardprofil: {}", name));
        }
    });

    // Profile: Delete
    btn_prof_delete.on_click(move |_| {
        let a = app();
        if a.profiles.len() <= 1 {
            anyui::Control::from_id(lbl_prof_status_id).set_text("Letztes Profil kann nicht geloescht werden.");
            return;
        }
        let idx = anyui::Control::from_id(dd_prof_id).get_state() as usize;
        if idx < a.profiles.len() {
            let name = a.profiles[idx].name.clone();
            a.profiles.remove(idx);
            if a.active_profile.name == name {
                a.active_profile = a.profiles[0].clone();
            }
            save_profiles(&a.profiles);
            let items = build_profile_items(&a.profiles);
            anyui::Control::from_id(dd_prof_id).set_text(&items);
            anyui::Control::from_id(dd_prof_id).set_state(0);
            anyui::Control::from_id(lbl_prof_status_id).set_text(&format!("Profil geloescht: {}", name));
            redraw();
        }
    });

    // Profile: New
    btn_prof_new.on_click(move |_| {
        let name = read_ctrl_text(tf_new_prof_id);
        if name.is_empty() {
            anyui::Control::from_id(lbl_prof_status_id).set_text("Bitte einen Namen eingeben.");
            return;
        }
        let a = app();
        if a.profiles.iter().any(|p| p.name == name) {
            anyui::Control::from_id(lbl_prof_status_id).set_text("Profil existiert bereits.");
            return;
        }
        let mut new_p = Profile::default_profile();
        new_p.name = name.clone();
        new_p.is_default = false;
        new_p.saved_tabs = a.snapshot_tabs();
        a.profiles.push(new_p.clone());
        a.active_profile = new_p;
        save_profiles(&a.profiles);
        let items = build_profile_items(&a.profiles);
        anyui::Control::from_id(dd_prof_id).set_text(&items);
        anyui::Control::from_id(dd_prof_id).set_state((a.profiles.len() - 1) as u32);
        anyui::Control::from_id(tf_new_prof_id).set_text("");
        anyui::Control::from_id(lbl_prof_status_id).set_text(&format!("Profil erstellt: {}", name));
        redraw();
    });

    dlg.add(&p_profiles);

    // ════════════════════════════════════════════════════════════════════════
    // Panel 4: Aliase
    // ════════════════════════════════════════════════════════════════════════
    let p_aliases = anyui::View::new();
    p_aliases.set_dock(anyui::DOCK_FILL);
    p_aliases.set_padding(16, 16, 16, 16);

    let lbl_alias = anyui::Label::new("Aliase verwalten:");
    lbl_alias.set_position(16, 12);
    lbl_alias.set_size(200, 24);
    p_aliases.add(&lbl_alias);

    // Get aliases from active session
    let aliases: Vec<(String, String)> = if let Some(sess) = a.active_session() {
        sess.shell.aliases.clone()
    } else {
        Vec::new()
    };

    let alias_items = build_alias_items(&aliases);
    let dd_alias = anyui::DropDown::new(if alias_items.is_empty() { "(keine)" } else { &alias_items });
    dd_alias.set_position(16, 40);
    dd_alias.set_size(300, 28);
    p_aliases.add(&dd_alias);

    let btn_alias_del = anyui::Button::new("Entfernen");
    btn_alias_del.set_position(326, 40);
    btn_alias_del.set_size(90, 28);
    p_aliases.add(&btn_alias_del);

    // Show selected alias value
    let lbl_alias_val = anyui::Label::new("");
    lbl_alias_val.set_position(16, 76);
    lbl_alias_val.set_size(500, 24);
    lbl_alias_val.set_text_color(0xFF88CC88);
    if !aliases.is_empty() {
        lbl_alias_val.set_text(&format!("= {}", aliases[0].1));
    }
    p_aliases.add(&lbl_alias_val);

    let dd_alias_id = dd_alias.id();
    let lbl_alias_val_id = lbl_alias_val.id();

    // Show value when selection changes
    dd_alias.on_selection_changed(move |e| {
        let a = app();
        if let Some(sess) = a.active_session() {
            let idx = e.index as usize;
            if idx < sess.shell.aliases.len() {
                anyui::Control::from_id(lbl_alias_val_id).set_text(&format!("= {}", sess.shell.aliases[idx].1));
            }
        }
    });

    // New alias
    let lbl_new_alias = anyui::Label::new("Neuer Alias:");
    lbl_new_alias.set_position(16, 112);
    lbl_new_alias.set_size(100, 24);
    p_aliases.add(&lbl_new_alias);

    let tf_alias_name = anyui::TextField::new();
    tf_alias_name.set_position(16, 138);
    tf_alias_name.set_size(140, 28);
    tf_alias_name.set_placeholder("Name");
    p_aliases.add(&tf_alias_name);

    let lbl_eq = anyui::Label::new("=");
    lbl_eq.set_position(162, 140);
    lbl_eq.set_size(16, 24);
    p_aliases.add(&lbl_eq);

    let tf_alias_val = anyui::TextField::new();
    tf_alias_val.set_position(180, 138);
    tf_alias_val.set_size(220, 28);
    tf_alias_val.set_placeholder("Befehl");
    p_aliases.add(&tf_alias_val);

    let btn_alias_add = anyui::Button::new("Hinzufuegen");
    btn_alias_add.set_position(410, 138);
    btn_alias_add.set_size(100, 28);
    p_aliases.add(&btn_alias_add);

    let lbl_alias_status = anyui::Label::new("");
    lbl_alias_status.set_position(16, 176);
    lbl_alias_status.set_size(500, 24);
    lbl_alias_status.set_text_color(0xFFCCCC88);
    p_aliases.add(&lbl_alias_status);

    let tf_alias_name_id = tf_alias_name.id();
    let tf_alias_val_id = tf_alias_val.id();
    let lbl_alias_status_id = lbl_alias_status.id();

    // Alias: Add
    btn_alias_add.on_click(move |_| {
        let name = read_ctrl_text(tf_alias_name_id);
        let val = read_ctrl_text(tf_alias_val_id);
        if name.is_empty() || val.is_empty() {
            anyui::Control::from_id(lbl_alias_status_id).set_text("Name und Befehl eingeben.");
            return;
        }
        let a = app();
        if let Some(sess) = a.active_session_mut() {
            if let Some(existing) = sess.shell.aliases.iter_mut().find(|(n, _)| *n == name) {
                existing.1 = val.clone();
            } else {
                sess.shell.aliases.push((name.clone(), val.clone()));
            }
            let items = build_alias_items(&sess.shell.aliases);
            anyui::Control::from_id(dd_alias_id).set_text(if items.is_empty() { "(keine)" } else { &items });
            anyui::Control::from_id(tf_alias_name_id).set_text("");
            anyui::Control::from_id(tf_alias_val_id).set_text("");
            anyui::Control::from_id(lbl_alias_status_id).set_text(&format!("Alias hinzugefuegt: {}={}", name, val));
        }
    });

    // Alias: Remove
    btn_alias_del.on_click(move |_| {
        let a = app();
        if let Some(sess) = a.active_session_mut() {
            let idx = anyui::Control::from_id(dd_alias_id).get_state() as usize;
            if idx < sess.shell.aliases.len() {
                let name = sess.shell.aliases[idx].0.clone();
                sess.shell.aliases.remove(idx);
                let items = build_alias_items(&sess.shell.aliases);
                anyui::Control::from_id(dd_alias_id).set_text(if items.is_empty() { "(keine)" } else { &items });
                anyui::Control::from_id(dd_alias_id).set_state(0);
                anyui::Control::from_id(lbl_alias_val_id).set_text("");
                anyui::Control::from_id(lbl_alias_status_id).set_text(&format!("Alias entfernt: {}", name));
            }
        }
    });

    dlg.add(&p_aliases);

    // ════════════════════════════════════════════════════════════════════════
    // Panel 5: Umgebungsvariablen
    // ════════════════════════════════════════════════════════════════════════
    let p_env = anyui::View::new();
    p_env.set_dock(anyui::DOCK_FILL);
    p_env.set_padding(16, 16, 16, 16);

    let lbl_env = anyui::Label::new("Umgebungsvariablen:");
    lbl_env.set_position(16, 12);
    lbl_env.set_size(200, 24);
    p_env.add(&lbl_env);

    let (env_items, env_entries) = build_env_items();
    let dd_env = anyui::DropDown::new(if env_items.is_empty() { "(keine)" } else { &env_items });
    dd_env.set_position(16, 40);
    dd_env.set_size(300, 28);
    p_env.add(&dd_env);

    let btn_env_del = anyui::Button::new("Entfernen");
    btn_env_del.set_position(326, 40);
    btn_env_del.set_size(90, 28);
    p_env.add(&btn_env_del);

    // Show selected env value
    let lbl_env_val = anyui::Label::new("");
    lbl_env_val.set_position(16, 76);
    lbl_env_val.set_size(550, 24);
    lbl_env_val.set_text_color(0xFF88CC88);
    if !env_entries.is_empty() {
        lbl_env_val.set_text(&format!("= {}", env_entries[0].1));
    }
    p_env.add(&lbl_env_val);

    let dd_env_id = dd_env.id();
    let lbl_env_val_id = lbl_env_val.id();

    dd_env.on_selection_changed(move |e| {
        let idx = e.index as usize;
        let (_, entries) = build_env_items();
        if idx < entries.len() {
            anyui::Control::from_id(lbl_env_val_id).set_text(&format!("= {}", entries[idx].1));
        }
    });

    // New env var
    let lbl_new_env = anyui::Label::new("Neue Variable:");
    lbl_new_env.set_position(16, 112);
    lbl_new_env.set_size(100, 24);
    p_env.add(&lbl_new_env);

    let tf_env_key = anyui::TextField::new();
    tf_env_key.set_position(16, 138);
    tf_env_key.set_size(140, 28);
    tf_env_key.set_placeholder("KEY");
    p_env.add(&tf_env_key);

    let lbl_env_eq = anyui::Label::new("=");
    lbl_env_eq.set_position(162, 140);
    lbl_env_eq.set_size(16, 24);
    p_env.add(&lbl_env_eq);

    let tf_env_val = anyui::TextField::new();
    tf_env_val.set_position(180, 138);
    tf_env_val.set_size(220, 28);
    tf_env_val.set_placeholder("Wert");
    p_env.add(&tf_env_val);

    let btn_env_add = anyui::Button::new("Setzen");
    btn_env_add.set_position(410, 138);
    btn_env_add.set_size(90, 28);
    p_env.add(&btn_env_add);

    let lbl_env_status = anyui::Label::new("");
    lbl_env_status.set_position(16, 176);
    lbl_env_status.set_size(500, 24);
    lbl_env_status.set_text_color(0xFFCCCC88);
    p_env.add(&lbl_env_status);

    let tf_env_key_id = tf_env_key.id();
    let tf_env_val_id = tf_env_val.id();
    let lbl_env_status_id = lbl_env_status.id();

    // Env: Add/Set
    btn_env_add.on_click(move |_| {
        let key = read_ctrl_text(tf_env_key_id);
        let val = read_ctrl_text(tf_env_val_id);
        if key.is_empty() {
            anyui::Control::from_id(lbl_env_status_id).set_text("KEY eingeben.");
            return;
        }
        anyos_std::env::set(&key, &val);
        let (items, _) = build_env_items();
        anyui::Control::from_id(dd_env_id).set_text(if items.is_empty() { "(keine)" } else { &items });
        anyui::Control::from_id(tf_env_key_id).set_text("");
        anyui::Control::from_id(tf_env_val_id).set_text("");
        anyui::Control::from_id(lbl_env_status_id).set_text(&format!("Gesetzt: {}={}", key, val));
    });

    // Env: Remove
    btn_env_del.on_click(move |_| {
        let (_, entries) = build_env_items();
        let idx = anyui::Control::from_id(dd_env_id).get_state() as usize;
        if idx < entries.len() {
            let key = entries[idx].0.clone();
            anyos_std::env::unset(&key);
            let (items, _) = build_env_items();
            anyui::Control::from_id(dd_env_id).set_text(if items.is_empty() { "(keine)" } else { &items });
            anyui::Control::from_id(dd_env_id).set_state(0);
            anyui::Control::from_id(lbl_env_val_id).set_text("");
            anyui::Control::from_id(lbl_env_status_id).set_text(&format!("Entfernt: {}", key));
        }
    });

    dlg.add(&p_env);

    // ── Connect tabs to panels ──
    tabs.connect_panels(&[&p_general, &p_themes, &p_profiles, &p_aliases, &p_env]);

    // ── OK button: apply all General settings and close ──
    let tf_font_id = tf_font.id();
    let sl_opacity_id = sl_opacity.id();
    let tf_scroll_id = tf_scroll.id();
    let tf_blink_id = tf_blink.id();
    let tf_win_w_id = tf_win_w.id();
    let tf_win_h_id = tf_win_h.id();
    let dd_theme_id = dd_theme.id();

    btn_ok.on_click(move |_| {
        let a = app();

        // Apply font size (to all panes)
        let font_str = read_ctrl_text(tf_font_id);
        if let Ok(sz) = font_str.parse::<u16>() {
            a.set_font_size_all(sz);
        }

        // Apply opacity
        let pct = anyui::Control::from_id(sl_opacity_id).get_state();
        let pct = pct.min(100);
        a.bg_alpha = ((pct * 255 + 50) / 100) as u8;
        a.active_profile.bg_alpha = a.bg_alpha;
        a.apply_bg_alpha();

        // Apply scrollback
        let scroll_str = read_ctrl_text(tf_scroll_id);
        if let Ok(n) = scroll_str.parse::<usize>() {
            a.active_profile.scrollback = n;
        }

        // Apply cursor blink
        let blink_str = read_ctrl_text(tf_blink_id);
        if let Ok(ms) = blink_str.parse::<u32>() {
            a.active_profile.cursor_blink_ms = ms;
            a.cursor_blink_elapsed = 0;
            if ms == 0 { a.cursor_blink_on = true; }
        }

        // Apply window size
        let ww_str = read_ctrl_text(tf_win_w_id);
        let wh_str = read_ctrl_text(tf_win_h_id);
        if let Ok(w) = ww_str.parse::<u32>() { a.active_profile.win_w = w; }
        if let Ok(h) = wh_str.parse::<u32>() { a.active_profile.win_h = h; }

        // Apply colors
        let color_setters: [(usize, fn(&mut Profile, u32)); 6] = [
            (0, |p, c| p.color_bg = c),
            (1, |p, c| p.color_fg = c),
            (2, |p, c| p.color_prompt = c),
            (3, |p, c| p.color_title = c),
            (4, |p, c| p.color_select_bg = c),
            (5, |p, c| p.color_select_fg = c),
        ];
        for (i, setter) in &color_setters {
            if *i < color_field_ids.len() {
                let hex = read_ctrl_text(color_field_ids[*i]);
                if let Some(c) = parse_hex_color(&hex) {
                    setter(&mut a.active_profile, c);
                }
            }
        }

        // Apply theme
        let theme_idx = anyui::Control::from_id(dd_theme_id).get_state() as usize;
        if theme_idx < THEMES.len() {
            a.apply_theme(&THEMES[theme_idx]);
            if a.bg_alpha < 255 { a.apply_bg_alpha(); }
        }

        a.update_pane_sizes();
        redraw();
        destroy_window(dlg_id);
    });

    dlg.on_close(move |_| {
        destroy_window(dlg_id);
    });
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    if !anyui::init() {
        anyos_std::println!("terminal: failed to init anyui");
        return;
    }

    // Load profiles and determine default
    let profiles = load_profiles();
    let active_profile = find_default_profile(&profiles);
    let init_w = active_profile.win_w;
    let init_h = active_profile.win_h;
    let init_font_size = active_profile.font_size;
    let init_defaults = SessionDefaults {
        font_size: active_profile.font_size,
        color_prompt: active_profile.color_prompt,
        color_fg: active_profile.color_fg,
    };

    let win = anyui::Window::new("Terminal", -1, -1, init_w, init_h);

    // Tab row: TabBar + "+" button (DOCK_TOP)
    let tab_row = anyui::View::new();
    tab_row.set_dock(anyui::DOCK_TOP);
    tab_row.set_size(init_w, 28);
    tab_row.set_color(0xFF2D2D2D);

    let new_tab_btn = anyui::Button::new("+");
    new_tab_btn.set_dock(anyui::DOCK_RIGHT);
    new_tab_btn.set_size(28, 28);
    new_tab_btn.set_color(0xFF3C3C3C);
    new_tab_btn.set_text_color(0xFFCCCCCC);
    new_tab_btn.set_tooltip("Neuer Tab (Ctrl+Shift+T)");
    tab_row.add(&new_tab_btn);

    let tab_bar = anyui::TabBar::new("Tab 1");
    tab_bar.set_dock(anyui::DOCK_FILL);
    tab_bar.set_size(init_w - 28, 28);
    tab_row.add(&tab_bar);

    win.add(&tab_row);

    // Canvas (DOCK_FILL) for terminal rendering
    let canvas_h = init_h.saturating_sub(28);
    let canvas = anyui::Canvas::new(init_w, canvas_h);
    canvas.set_dock(anyui::DOCK_FILL);
    canvas.set_interactive(true);
    win.add(&canvas);

    // Store widgets for global access
    unsafe {
        CANVAS = Some(canvas);
        TAB_BAR = Some(tab_bar);
    }

    // Load environment
    let aliases = load_dotenv();
    anyos_std::env::set("PWD", "/");
    fs::chdir("/");

    // Compute initial grid size from canvas (using profile font size)
    let (init_cw, init_ch) = cell_dims_for_font(active_profile.font_size);
    let cols = (init_w.saturating_sub(TEXT_PAD as u32 * 2) / init_cw as u32) as usize;
    let rows = (canvas_h.saturating_sub(TEXT_PAD as u32 * 2) / init_ch as u32) as usize;

    anyos_std::env::set("COLUMNS", &format!("{}", cols));
    anyos_std::env::set("LINES", &format!("{}", rows));

    // Initialize app state (needed before show_prompt which calls app())
    let mut term_app = TerminalApp {
        win_id: win.id(),
        canvas_w: init_w,
        canvas_h: canvas_h,
        tabs: Vec::new(),
        active_tab: 0,
        next_pane_id: 2,
        profiles,
        active_profile,
        search: SearchState::new(),
        reverse_search: ReverseSearchState::new(),
        bg_alpha: 255,
        theme_name: String::from("Default"),
        cursor_blink_on: true,
        cursor_blink_elapsed: 0,
        autosave_counter: 0,
    };

    // Apply profile theme/opacity settings (font_size is per-session, applied via Session::new)
    term_app.bg_alpha = term_app.active_profile.bg_alpha;
    term_app.theme_name = term_app.active_profile.theme_name.clone();
    if term_app.bg_alpha < 255 {
        let alpha = term_app.bg_alpha as u32;
        let bg = term_app.active_profile.color_bg;
        term_app.active_profile.color_bg = (alpha << 24) | (bg & 0x00FFFFFF);
    }

    // Try recovery file first (crash recovery with scrollback per pane),
    // then profile saved tabs, then create fresh session.
    let recovery_tabs = load_recovery();
    let has_saved_tabs = recovery_tabs.is_some() || !term_app.active_profile.saved_tabs.is_empty();

    let restore_tabs = if let Some(ref tabs) = recovery_tabs {
        tabs
    } else if !term_app.active_profile.saved_tabs.is_empty() {
        &term_app.active_profile.saved_tabs
    } else {
        &[] as &[SavedTab]
    };

    if !restore_tabs.is_empty() {
        let alias_pairs: Vec<(String, String)> = aliases;
        for st in restore_tabs {
            let mut next_id = term_app.next_pane_id;
            let root = rebuild_pane_tree(&st.layout, &mut next_id, cols.max(1), rows.max(1), &alias_pairs, &init_defaults);
            term_app.next_pane_id = next_id;
            let active_pane = root.first_pane_id();
            term_app.tabs.push(Tab {
                root,
                active_pane_id: active_pane,
                title: st.title.clone(),
            });
        }
    } else {
        // Create default first session
        let first_session = Session::new(cols.max(1), rows.max(1), aliases, init_defaults.font_size, init_defaults.color_prompt, init_defaults.color_fg);
        let first_tab = Tab {
            root: PaneNode::Leaf { id: 1, session: first_session },
            active_pane_id: 1,
            title: String::from("Tab 1"),
        };
        term_app.tabs.push(first_tab);
    }

    unsafe { APP = Some(term_app); }

    // Write welcome message (only for fresh sessions without recovery/saved tabs)
    {
        let a = app();
        let color_title = a.active_profile.color_title;
        let color_dim = a.active_profile.color_dim;
        if !has_saved_tabs {
            if let Some(sess) = a.active_session_mut() {
                sess.buf.current_fg = color_title;
                sess.buf.write_str(".anyOS Terminal v");
                sess.buf.write_str(env!("ANYOS_VERSION"));
                sess.buf.write_str("\n");
                sess.buf.current_fg = color_dim;
                sess.buf.write_str("Type 'help' for available commands.\n\n");
                sess.show_prompt();
            }
        }
    }

    // Update tab bar labels if tabs were restored
    if has_saved_tabs {
        update_tab_labels();
    }

    // Initial render
    render_to_canvas(app(), &canvas);

    // ── Tab bar events ──
    tab_bar.on_active_changed(|e| {
        let a = app();
        let idx = e.index as usize;
        if idx < a.tabs.len() {
            a.active_tab = idx;
            redraw();
        }
    });

    // Double-click on tab bar to rename
    {
        let win_id = win.id();
        tab_bar.on_double_click(move |_| {
            show_rename_tab_dialog(win_id);
        });
    }

    tab_bar.on_tab_close(|e| {
        let a = app();
        let idx = e.index as usize;
        if a.tabs.len() > 1 && idx < a.tabs.len() {
            a.close_tab(idx);
            a.update_pane_sizes();
            update_tab_labels();
            redraw();
        }
    });

    // Right-click context menu on canvas
    let ctx_menu = anyui::ContextMenu::new("Horizontal teilen|Vertikal teilen|Pane schliessen|-|Neuer Tab|Tab schliessen|Tab umbenennen|-|Suchen|Schrift +|Schrift -|-|Einstellungen");
    canvas.set_context_menu(&ctx_menu);
    ctx_menu.on_item_click(|e| {
        let a = app();
        match e.index {
            0 => { // Split horizontal
                a.split_horizontal();
                a.update_pane_sizes();
                redraw();
            }
            1 => { // Split vertical
                a.split_vertical();
                a.update_pane_sizes();
                redraw();
            }
            2 => { // Close pane
                let pane_id = a.tabs[a.active_tab].active_pane_id;
                if !a.close_pane(pane_id) {
                    if a.tabs.len() > 1 {
                        a.close_tab(a.active_tab);
                        update_tab_labels();
                    } else {
                        anyui::quit();
                        return;
                    }
                }
                a.update_pane_sizes();
                redraw();
            }
            4 => { // New tab
                a.new_tab();
                a.update_pane_sizes();
                update_tab_labels();
                redraw();
            }
            5 => { // Close tab
                if a.tabs.len() > 1 {
                    a.close_tab(a.active_tab);
                    a.update_pane_sizes();
                    update_tab_labels();
                    redraw();
                } else {
                    anyui::quit();
                }
            }
            6 => { // Rename tab
                show_rename_tab_dialog(a.win_id);
            }
            8 => { // Search
                a.search.active = true;
                a.search.query.clear();
                a.search.matches.clear();
                redraw();
            }
            9 => { // Font +
                let new_size = a.active_font_size() + 1;
                a.set_font_size(new_size);
                redraw();
            }
            10 => { // Font -
                let new_size = a.active_font_size().saturating_sub(1);
                a.set_font_size(new_size);
                redraw();
            }
            12 => { // Einstellungen
                open_settings_dialog(a.win_id);
            }
            _ => {}
        }
    });

    // "+" button to create new tab
    new_tab_btn.on_click(|_| {
        let a = app();
        a.new_tab();
        a.update_pane_sizes();
        update_tab_labels();
        redraw();
    });

    // ── Canvas mouse events ──
    canvas.on_mouse_down(|x, y, button| {
        let a = app();
        let mods = anyui::get_modifiers();

        // Forward mouse events to child process if mouse tracking is enabled
        if let Some(sess) = a.active_session_mut() {
            if sess.buf.mouse_mode > 0 {
                if let Some(ref fp) = sess.fg_proc {
                    if fp.stdin_pipe != 0 {
                        let cell_x = ((x as u16).saturating_sub(TEXT_PAD) / sess.cell_w) as u32 + 1;
                        let cell_y = ((y as u16).saturating_sub(TEXT_PAD) / sess.cell_h) as u32 + 1;
                        let btn = button.saturating_sub(1); // 0=left, 1=mid, 2=right
                        if sess.buf.mouse_sgr {
                            let seq = format!("\x1b[<{};{};{}M", btn, cell_x, cell_y);
                            ipc::pipe_write(fp.stdin_pipe, seq.as_bytes());
                        } else {
                            let seq = [0x1b, b'[', b'M', btn as u8 + 32, cell_x as u8 + 32, cell_y as u8 + 32];
                            ipc::pipe_write(fp.stdin_pipe, &seq);
                        }
                        return;
                    }
                }
            }
        }

        if button == 1 && (mods & MOD_ALT) != 0 {
            // Alt+click: open file under cursor with 'open' command
            let area = a.content_area();
            let tab = &a.tabs[a.active_tab];
            let active_pane = tab.active_pane_id;
            let layout = tab.root.layout(area);
            let pane_rect = layout.iter().find(|(id, _)| *id == active_pane).map(|(_, r)| *r).unwrap_or(area);
            if let Some(sess) = tab.root.find_session(active_pane) {
                let pos = pixel_to_cell_in_rect(x as u32, y as u32, &sess.buf, pane_rect, sess.cell_w, sess.cell_h);
                let word = word_at_cell(&sess.buf, pos.row, pos.col);
                if !word.is_empty() {
                    // Resolve path relative to cwd
                    let filepath = if word.starts_with('/') {
                        word
                    } else if sess.shell.cwd == "/" {
                        format!("/{}", word)
                    } else {
                        format!("{}/{}", sess.shell.cwd, word)
                    };
                    let args = format!("open {}", filepath);
                    process::spawn("/System/bin/open", &args);
                }
            }
            return;
        }
        // Ctrl+click: open URL under cursor
        if button == 1 && (mods & MOD_CTRL) != 0 {
            let area = a.content_area();
            let tab = &a.tabs[a.active_tab];
            let active_pane = tab.active_pane_id;
            let layout = tab.root.layout(area);
            let pane_rect = layout.iter().find(|(id, _)| *id == active_pane).map(|(_, r)| *r).unwrap_or(area);
            if let Some(sess) = tab.root.find_session(active_pane) {
                let pos = pixel_to_cell_in_rect(x as u32, y as u32, &sess.buf, pane_rect, sess.cell_w, sess.cell_h);
                if pos.row < sess.buf.lines.len() {
                    let line = &sess.buf.lines[pos.row];
                    // Check OSC 8 hyperlink first
                    let mut opened = false;
                    if pos.col < line.len() && line[pos.col].link_id != 0 {
                        if let Some(url) = sess.buf.get_hyperlink(line[pos.col].link_id) {
                            let args = format!("surf {}", url);
                            process::spawn("/Applications/surf", &args);
                            opened = true;
                        }
                    }
                    if !opened {
                        if let Some((start, end)) = find_url_at(line, pos.col) {
                            let url: String = line[start..end].iter().map(|c| c.ch).collect();
                            let args = format!("surf {}", url);
                            process::spawn("/Applications/surf", &args);
                        }
                    }
                }
            }
            return;
        }
        if button == 1 { // Left click
            // Change active pane if needed
            let area = a.content_area();
            let tab = &mut a.tabs[a.active_tab];
            if let Some(clicked_pane) = tab.root.pane_at_point(area, x as u16, y as u16) {
                tab.active_pane_id = clicked_pane;
                let layout = tab.root.layout(area);
                let pane_rect = layout.iter().find(|(id, _)| *id == clicked_pane).map(|(_, r)| *r).unwrap_or(area);

                // Check if click is on scrollbar track
                let scrollbar_x = (pane_rect.x + pane_rect.w).saturating_sub(SCROLLBAR_WIDTH as u16);
                if (x as u16) >= scrollbar_x {
                    // Click on scrollbar — scroll to position
                    if let Some(sess) = tab.root.find_session_mut(clicked_pane) {
                        let total = sess.buf.lines.len();
                        if total > sess.buf.visible_rows {
                            let rel_y = (y as u16).saturating_sub(pane_rect.y) as u32;
                            let track_h = pane_rect.h as u32;
                            let scrollable = total - sess.buf.visible_rows;
                            sess.buf.scroll_offset = ((rel_y as usize * scrollable) / track_h as usize).min(scrollable);
                            sess.scrollbar_dragging = true;
                            sess.dirty = true;
                        }
                    }
                } else {
                    // Start text selection
                    if let Some(sess) = tab.root.find_session_mut(clicked_pane) {
                        sess.scrollbar_dragging = false;
                        let pos = pixel_to_cell_in_rect(x as u32, y as u32, &sess.buf, pane_rect, sess.cell_w, sess.cell_h);
                        sess.selection = Some(Selection {
                            dragging: true,
                            anchor: pos,
                            active: pos,
                        });
                        sess.dirty = true;
                    }
                }
            }
            redraw();
        } else if button == 2 { // Scroll up (button 2 = scroll up in anyui)
            if (mods & MOD_SHIFT) != 0 {
                // Shift+Scroll Up = increase font size
                let new_size = a.active_font_size() + 1;
                a.set_font_size(new_size);
                redraw();
            } else {
                if let Some(sess) = a.active_session_mut() {
                    sess.buf.scroll_up(3);
                    sess.dirty = true;
                }
                redraw();
            }
        } else if button == 3 { // Scroll down (button 3 = scroll down in anyui)
            if (mods & MOD_SHIFT) != 0 {
                // Shift+Scroll Down = decrease font size
                let new_size = a.active_font_size().saturating_sub(1);
                a.set_font_size(new_size);
                redraw();
            } else {
                if let Some(sess) = a.active_session_mut() {
                    sess.buf.scroll_down(3);
                    sess.dirty = true;
                }
                redraw();
            }
        } else if button == 4 { // Middle click paste
            if let Some(sess) = a.active_session_mut() {
                if sess.fg_proc.is_none() && sess.su_pending_user.is_none() {
                    let mut clip_buf = [0u8; 4096];
                    let clip_len = anyui::clipboard_get(&mut clip_buf);
                    if clip_len > 0 {
                        if let Ok(text) = core::str::from_utf8(&clip_buf[..clip_len as usize]) {
                            for c in text.chars() {
                                if c >= ' ' && (c as u32) < 128 { sess.shell.insert_char(c); }
                            }
                            redraw_input_line(&mut sess.buf, &sess.shell);
                            sess.dirty = true;
                        }
                    }
                }
            }
            redraw();
        }
    });

    canvas.on_mouse_move(|x, y| {
        let a = app();
        // Forward mouse move to child process if tracking button/any events
        if let Some(sess) = a.active_session_mut() {
            if sess.buf.mouse_mode >= 1002 {
                if let Some(ref fp) = sess.fg_proc {
                    if fp.stdin_pipe != 0 {
                        let cell_x = ((x as u16).saturating_sub(TEXT_PAD) / sess.cell_w) as u32 + 1;
                        let cell_y = ((y as u16).saturating_sub(TEXT_PAD) / sess.cell_h) as u32 + 1;
                        if sess.buf.mouse_sgr {
                            let seq = format!("\x1b[<35;{};{}M", cell_x, cell_y);
                            ipc::pipe_write(fp.stdin_pipe, seq.as_bytes());
                        } else {
                            let seq = [0x1b, b'[', b'M', 32 + 32, cell_x as u8 + 32, cell_y as u8 + 32];
                            ipc::pipe_write(fp.stdin_pipe, &seq);
                        }
                        return;
                    }
                }
            }
        }
        let active_pane = a.tabs[a.active_tab].active_pane_id;
        let area = a.content_area();
        let tab = &mut a.tabs[a.active_tab];
        let layout = tab.root.layout(area);
        let pane_rect = layout.iter().find(|(id, _)| *id == active_pane).map(|(_, r)| *r).unwrap_or(area);
        if let Some(sess) = tab.root.find_session_mut(active_pane) {
            // Scrollbar drag
            if sess.scrollbar_dragging {
                let total = sess.buf.lines.len();
                if total > sess.buf.visible_rows {
                    let rel_y = (y as u16).saturating_sub(pane_rect.y) as u32;
                    let track_h = pane_rect.h as u32;
                    let scrollable = total - sess.buf.visible_rows;
                    sess.buf.scroll_offset = ((rel_y as usize * scrollable) / track_h as usize).min(scrollable);
                    sess.dirty = true;
                    let canvas = get_canvas();
                    render_to_canvas(app(), &canvas);
                }
                return;
            }
            // Text selection drag
            if let Some(ref mut sel) = sess.selection {
                if sel.dragging {
                    sel.active = pixel_to_cell_in_rect(x as u32, y as u32, &sess.buf, pane_rect, sess.cell_w, sess.cell_h);
                    sess.dirty = true;
                    let canvas = get_canvas();
                    render_to_canvas(app(), &canvas);
                }
            }
        }
    });

    canvas.on_mouse_up(|_x, _y, _button| {
        let a = app();
        // Forward mouse up to child process if mouse tracking enabled
        if let Some(sess) = a.active_session_mut() {
            if sess.buf.mouse_mode > 0 {
                if let Some(ref fp) = sess.fg_proc {
                    if fp.stdin_pipe != 0 {
                        let cell_x = ((_x as u16).saturating_sub(TEXT_PAD) / sess.cell_w) as u32 + 1;
                        let cell_y = ((_y as u16).saturating_sub(TEXT_PAD) / sess.cell_h) as u32 + 1;
                        if sess.buf.mouse_sgr {
                            let btn = _button.saturating_sub(1);
                            let seq = format!("\x1b[<{};{};{}m", btn, cell_x, cell_y);
                            ipc::pipe_write(fp.stdin_pipe, seq.as_bytes());
                        } else {
                            let seq = [0x1b, b'[', b'M', 3 + 32, cell_x as u8 + 32, cell_y as u8 + 32];
                            ipc::pipe_write(fp.stdin_pipe, &seq);
                        }
                        return;
                    }
                }
            }
        }
        if let Some(sess) = a.active_session_mut() {
            sess.scrollbar_dragging = false;
            if let Some(ref mut sel) = sess.selection {
                sel.dragging = false;
                let (s, e) = sel.ordered();
                if s.row == e.row && s.col == e.col {
                    sess.selection = None;
                } else {
                    let text = extract_selected_text(&sess.buf, sel);
                    if !text.is_empty() { anyui::clipboard_set(&text); }
                }
                sess.dirty = true;
            }
        }
        redraw();
    });

    // ── Keyboard events ──
    win.on_key_down(|ke| {
        let a = app();
        let key_code = ke.keycode;
        let char_val = ke.char_code;
        let mods = ke.modifiers;

        // Global shortcuts (Ctrl+Shift combos)
        let ctrl_shift = ke.ctrl() && ke.shift();

        // ── Search mode keyboard handling ──
        if a.search.active {
            match key_code {
                KEY_ESCAPE => {
                    a.search.active = false;
                    a.search.query.clear();
                    a.search.matches.clear();
                    redraw();
                    return;
                }
                KEY_ENTER => {
                    a.search.next_match();
                    // Scroll to current match
                    if !a.search.matches.is_empty() {
                        let (line, _, _) = a.search.matches[a.search.current_match];
                        if let Some(sess) = a.active_session_mut() {
                            if line < sess.buf.scroll_offset || line >= sess.buf.scroll_offset + sess.buf.visible_rows {
                                sess.buf.scroll_offset = line.saturating_sub(sess.buf.visible_rows / 2);
                            }
                        }
                    }
                    redraw();
                    return;
                }
                KEY_BACKSPACE => {
                    a.search.query.pop();
                    do_search_in_active_session();
                    redraw();
                    return;
                }
                _ => {
                    if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                        let c = char_val as u8 as char;
                        if c >= ' ' {
                            a.search.query.push(c);
                            do_search_in_active_session();
                            // Scroll to first match
                            let a = app();
                            if !a.search.matches.is_empty() {
                                let (line, _, _) = a.search.matches[a.search.current_match];
                                if let Some(sess) = a.active_session_mut() {
                                    if line < sess.buf.scroll_offset || line >= sess.buf.scroll_offset + sess.buf.visible_rows {
                                        sess.buf.scroll_offset = line.saturating_sub(sess.buf.visible_rows / 2);
                                    }
                                }
                            }
                        }
                    }
                    // Shift+Enter = previous match
                    if key_code == KEY_ENTER && ke.shift() {
                        let a = app();
                        a.search.prev_match();
                    }
                    redraw();
                    return;
                }
            }
        }

        // ── Reverse search mode (Ctrl+R) keyboard handling ──
        if a.reverse_search.active {
            match key_code {
                KEY_ESCAPE => {
                    a.reverse_search.active = false;
                    a.reverse_search.query.clear();
                    a.reverse_search.result = None;
                    redraw();
                    return;
                }
                KEY_ENTER => {
                    // Accept the result
                    if let Some(result) = a.reverse_search.result.take() {
                        a.reverse_search.active = false;
                        a.reverse_search.query.clear();
                        if let Some(sess) = a.active_session_mut() {
                            sess.shell.input = result;
                            sess.shell.cursor = sess.shell.input.len();
                            redraw_input_line(&mut sess.buf, &sess.shell);
                            sess.dirty = true;
                        }
                    } else {
                        a.reverse_search.active = false;
                        a.reverse_search.query.clear();
                    }
                    redraw();
                    return;
                }
                KEY_BACKSPACE => {
                    a.reverse_search.query.pop();
                    do_reverse_search_in_active_session();
                    redraw();
                    return;
                }
                _ => {
                    if char_val > 0 && char_val < 128 && (mods & MOD_CTRL) == 0 {
                        let c = char_val as u8 as char;
                        if c >= ' ' {
                            a.reverse_search.query.push(c);
                            do_reverse_search_in_active_session();
                        }
                    }
                    redraw();
                    return;
                }
            }
        }

        if ctrl_shift {
            match key_code {
                0x14 => { // Ctrl+Shift+T: New tab
                    a.new_tab();
                    a.update_pane_sizes();
                    update_tab_labels();
                    redraw();
                    return;
                }
                0x11 => { // Ctrl+Shift+W: Close tab
                    if a.tabs.len() > 1 {
                        a.close_tab(a.active_tab);
                        a.update_pane_sizes();
                        update_tab_labels();
                        redraw();
                    } else {
                        anyui::quit();
                    }
                    return;
                }
                0x18 => { // Ctrl+Shift+O: Split horizontal
                    a.split_horizontal();
                    a.update_pane_sizes();
                    redraw();
                    return;
                }
                0x12 => { // Ctrl+Shift+E: Split vertical
                    a.split_vertical();
                    a.update_pane_sizes();
                    redraw();
                    return;
                }
                0x2D => { // Ctrl+Shift+X: Close pane
                    let pane_id = a.tabs[a.active_tab].active_pane_id;
                    if !a.close_pane(pane_id) {
                        if a.tabs.len() > 1 {
                            a.close_tab(a.active_tab);
                            update_tab_labels();
                        } else {
                            anyui::quit();
                            return;
                        }
                    }
                    a.update_pane_sizes();
                    redraw();
                    return;
                }
                0x13 => { // Ctrl+Shift+R: Rename tab
                    show_rename_tab_dialog(a.win_id);
                    return;
                }
                0x21 => { // Ctrl+Shift+F: Search
                    a.search.active = true;
                    a.search.query.clear();
                    a.search.matches.clear();
                    a.search.current_match = 0;
                    redraw();
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+R: Reverse search in history
        if ke.ctrl() && !ke.shift() && char_val == 'r' as u32 {
            a.reverse_search.active = true;
            a.reverse_search.query.clear();
            a.reverse_search.result = None;
            redraw();
            return;
        }

        // Ctrl+,: Settings dialog
        if ke.ctrl() && !ke.shift() && char_val == ',' as u32 {
            open_settings_dialog(a.win_id);
            return;
        }

        // Ctrl+Plus / Ctrl+Minus: Font zoom (per pane)
        if ke.ctrl() && !ke.shift() {
            // Plus key (= on most keyboards, scancode 0x0D)
            if char_val == '+' as u32 || char_val == '=' as u32 || key_code == 0x0D {
                let new_size = a.active_font_size() + 1;
                a.set_font_size(new_size);
                redraw();
                return;
            }
            // Minus key (scancode 0x0C)
            if char_val == '-' as u32 || key_code == 0x0C {
                let new_size = a.active_font_size().saturating_sub(1);
                a.set_font_size(new_size);
                redraw();
                return;
            }
            // Ctrl+0: Reset font size
            if char_val == '0' as u32 {
                a.set_font_size(a.active_profile.font_size);
                redraw();
                return;
            }
        }

        // PageUp/PageDown: scroll in scrollback (when no fg process)
        if key_code == KEY_PAGE_UP && !ke.ctrl() && !ke.shift() {
            if let Some(sess) = a.active_session_mut() {
                if sess.fg_proc.is_none() {
                    let scroll_amount = sess.buf.visible_rows.saturating_sub(1).max(1);
                    sess.buf.scroll_up(scroll_amount);
                    sess.dirty = true;
                    redraw();
                    return;
                }
            }
        }
        if key_code == KEY_PAGE_DOWN && !ke.ctrl() && !ke.shift() {
            if let Some(sess) = a.active_session_mut() {
                if sess.fg_proc.is_none() {
                    let scroll_amount = sess.buf.visible_rows.saturating_sub(1).max(1);
                    sess.buf.scroll_down(scroll_amount);
                    sess.dirty = true;
                    redraw();
                    return;
                }
            }
        }

        // Ctrl+Tab / Ctrl+Shift+Tab: switch tabs
        if ke.ctrl() && key_code == anyui::KEY_TAB {
            if ke.shift() {
                if a.active_tab > 0 { a.active_tab -= 1; }
                else { a.active_tab = a.tabs.len() - 1; }
            } else {
                a.active_tab = (a.active_tab + 1) % a.tabs.len();
            }
            let tab_bar = get_tab_bar();
            tab_bar.set_state(a.active_tab as u32);
            redraw();
            return;
        }

        // Alt+Arrow: switch pane focus
        if ke.alt() {
            match key_code {
                anyui::KEY_LEFT | anyui::KEY_RIGHT | anyui::KEY_UP | anyui::KEY_DOWN => {
                    let tab = &mut a.tabs[a.active_tab];
                    let mut ids = Vec::new();
                    tab.root.collect_pane_ids(&mut ids);
                    if ids.len() > 1 {
                        let current_idx = ids.iter().position(|&id| id == tab.active_pane_id).unwrap_or(0);
                        let new_idx = match key_code {
                            anyui::KEY_RIGHT | anyui::KEY_DOWN => (current_idx + 1) % ids.len(),
                            _ => if current_idx > 0 { current_idx - 1 } else { ids.len() - 1 },
                        };
                        tab.active_pane_id = ids[new_idx];
                        redraw();
                    }
                    return;
                }
                _ => {}
            }
        }

        // Clear selection on keyboard input
        if let Some(sess) = a.active_session_mut() {
            if sess.selection.is_some() {
                sess.selection = None;
                sess.dirty = true;
            }
        }

        // Reset cursor blink on keypress so cursor is always visible while typing
        a.cursor_blink_on = true;
        a.cursor_blink_elapsed = 0;

        // Forward to active session
        let pane_id = a.tabs[a.active_tab].active_pane_id;
        let should_continue = if let Some(sess) = a.tabs[a.active_tab].root.find_session_mut(pane_id) {
            handle_session_key(sess, key_code, char_val, mods)
        } else {
            true
        };

        if !should_continue {
            let pane_id = a.tabs[a.active_tab].active_pane_id;
            if !a.close_pane(pane_id) {
                if a.tabs.len() > 1 {
                    a.close_tab(a.active_tab);
                    update_tab_labels();
                } else {
                    anyui::quit();
                    return;
                }
            }
            a.update_pane_sizes();
        }
        redraw();
    });

    // ── Window events ──
    win.on_resize(|_| {
        let a = app();
        let canvas = get_canvas();
        let stride = canvas.get_stride();
        let h = canvas.get_height();
        if stride > 0 && h > 0 {
            a.canvas_w = stride;
            a.canvas_h = h;
            // Remember window size in profile (canvas + tab row height)
            a.active_profile.win_w = stride;
            a.active_profile.win_h = h + 28; // 28 = tab row height
        }
        a.update_pane_sizes();
        let (new_cols, new_rows) = a.content_cols_rows();
        anyos_std::env::set("COLUMNS", &format!("{}", new_cols));
        anyos_std::env::set("LINES", &format!("{}", new_rows));
        render_to_canvas(a, &canvas);
    });

    win.on_close(|_| {
        let a = app();
        save_recovery(a);
        // Persist active profile (window size, etc.) back to profiles list
        for p in a.profiles.iter_mut() {
            if p.name == a.active_profile.name {
                p.win_w = a.active_profile.win_w;
                p.win_h = a.active_profile.win_h;
                break;
            }
        }
        save_profiles(&a.profiles);
        anyui::quit();
    });

    // ── Timer for polling subprocess pipes + cursor blink ──
    anyui::set_timer(50, || {
        let a = app();

        // Poll foreground processes
        poll_fg_processes(a);

        // Cursor blink: toggle phase when elapsed time exceeds interval
        let blink_ms = a.active_profile.cursor_blink_ms;
        let mut blink_toggled = false;
        if blink_ms > 0 {
            a.cursor_blink_elapsed += 50;
            if a.cursor_blink_elapsed >= blink_ms {
                a.cursor_blink_elapsed = 0;
                a.cursor_blink_on = !a.cursor_blink_on;
                blink_toggled = true;
            }
        } else {
            // Blink disabled — cursor always visible
            if !a.cursor_blink_on {
                a.cursor_blink_on = true;
                blink_toggled = true;
            }
        }

        // Check if any session is dirty
        let tab = &a.tabs[a.active_tab];
        let mut ids = Vec::new();
        tab.root.collect_pane_ids(&mut ids);
        let mut any_dirty = false;
        for pane_id in &ids {
            if let Some(sess) = tab.root.find_session(*pane_id) {
                if sess.dirty { any_dirty = true; break; }
            }
        }

        if any_dirty || blink_toggled {
            // Mark clean
            let tab = &mut app().tabs[app().active_tab];
            let mut ids2 = Vec::new();
            tab.root.collect_pane_ids(&mut ids2);
            for pane_id in ids2 {
                if let Some(sess) = tab.root.find_session_mut(pane_id) {
                    sess.dirty = false;
                }
            }
            redraw();
        }

        // Auto-save scrollback every 30 seconds (600 ticks * 50ms)
        let a = app();
        a.autosave_counter += 1;
        if a.autosave_counter >= 600 {
            a.autosave_counter = 0;
            save_recovery(a);
        }
    });

    anyui::run();
}
