//! Terminal abstraction for AC — wraps ANSI escape sequences and
//! the `con_set_mode` / `con_get_size` syscalls.
//!
//! All rendering in AC goes through this module so that the rest of
//! the code never has to emit raw escape sequences directly.

use anyos_std::sys::{CON_MODE_HIDE_CURSOR, CON_MODE_NO_AUTOSCROLL};

// ─── Colors ──────────────────────────────────────────────────────────────────

/// Standard 8-bit terminal color indices (ANSI 256-color palette).
#[allow(dead_code)]
pub mod color {
    // Basic ANSI foreground colors (used as fg/bg in SGR)
    pub const BLACK:          u8 = 0;
    pub const RED:            u8 = 1;
    pub const GREEN:          u8 = 2;
    pub const YELLOW:         u8 = 3;
    pub const BLUE:           u8 = 4;
    pub const MAGENTA:        u8 = 5;
    pub const CYAN:           u8 = 6;
    pub const WHITE:          u8 = 7;
    pub const BRIGHT_BLACK:   u8 = 8;
    pub const BRIGHT_RED:     u8 = 9;
    pub const BRIGHT_GREEN:   u8 = 10;
    pub const BRIGHT_YELLOW:  u8 = 11;
    pub const BRIGHT_BLUE:    u8 = 12;
    pub const BRIGHT_MAGENTA: u8 = 13;
    pub const BRIGHT_CYAN:    u8 = 14;
    pub const BRIGHT_WHITE:   u8 = 15;
}

// ─── Output buffer ───────────────────────────────────────────────────────────

/// Maximum size of the render output buffer.
/// AC renders a full frame into this buffer and flushes it in one syscall.
const BUF_SIZE: usize = 65536;

pub struct TermBuf {
    data: [u8; BUF_SIZE],
    len:  usize,
}

impl TermBuf {
    pub const fn new() -> Self {
        Self { data: [0u8; BUF_SIZE], len: 0 }
    }

    /// Append raw bytes.
    pub fn push_bytes(&mut self, b: &[u8]) {
        let space = BUF_SIZE.saturating_sub(self.len);
        let n = b.len().min(space);
        self.data[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
    }

    /// Append a str slice.
    #[inline]
    pub fn push_str(&mut self, s: &str) {
        self.push_bytes(s.as_bytes());
    }

    /// Append a single ASCII byte.
    #[inline]
    pub fn push_byte(&mut self, b: u8) {
        if self.len < BUF_SIZE {
            self.data[self.len] = b;
            self.len += 1;
        }
    }

    /// Append a decimal u32 without heap allocation.
    pub fn push_u32(&mut self, mut n: u32) {
        if n == 0 { self.push_byte(b'0'); return; }
        let mut tmp = [0u8; 10];
        let mut i = 10usize;
        while n > 0 {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        self.push_bytes(&tmp[i..]);
    }

    /// Flush the buffer to the kernel text console in one syscall, then reset.
    pub fn flush(&mut self) {
        if self.len == 0 { return; }
        if let Ok(s) = core::str::from_utf8(&self.data[..self.len]) {
            anyos_std::sys::con_write(s);
        }
        self.len = 0;
    }

    pub fn is_empty(&self) -> bool { self.len == 0 }
}

// ─── Terminal control ─────────────────────────────────────────────────────────

/// Enter fullscreen TUI mode: hide cursor, disable auto-scroll.
pub fn enter_tui_mode() {
    anyos_std::sys::con_set_mode(CON_MODE_HIDE_CURSOR | CON_MODE_NO_AUTOSCROLL);
}

/// Leave fullscreen TUI mode: show cursor, re-enable auto-scroll.
pub fn leave_tui_mode() {
    anyos_std::sys::con_set_mode(0);
}

/// Read terminal dimensions from the kernel.
/// Falls back to 80×24 if not initialised.
pub fn get_size() -> (u32, u32) {
    let (cols, rows) = anyos_std::sys::con_get_size();
    let cols = if cols > 0 { cols } else { 80 };
    let rows = if rows > 0 { rows } else { 24 };
    (cols, rows)
}

// ─── Escape-sequence helpers (write into TermBuf) ────────────────────────────

/// Move cursor to 1-based (col, row).
pub fn goto(buf: &mut TermBuf, col: u32, row: u32) {
    buf.push_str("\x1B[");
    buf.push_u32(row);
    buf.push_byte(b';');
    buf.push_u32(col);
    buf.push_byte(b'H');
}

/// Erase from cursor to end of line.
pub fn erase_to_eol(buf: &mut TermBuf) {
    buf.push_str("\x1B[K");
}

/// Erase the entire screen (cursor stays).
pub fn clear_screen(buf: &mut TermBuf) {
    buf.push_str("\x1B[2J");
}

/// Move cursor to top-left (1,1).
pub fn home(buf: &mut TermBuf) {
    buf.push_str("\x1B[H");
}

/// Reset all SGR attributes.
pub fn reset(buf: &mut TermBuf) {
    buf.push_str("\x1B[0m");
}

/// Set bold.
pub fn bold(buf: &mut TermBuf) {
    buf.push_str("\x1B[1m");
}

/// Set foreground color using 256-color index.
pub fn fg256(buf: &mut TermBuf, idx: u8) {
    buf.push_str("\x1B[38;5;");
    buf.push_u32(idx as u32);
    buf.push_byte(b'm');
}

/// Set background color using 256-color index.
pub fn bg256(buf: &mut TermBuf, idx: u8) {
    buf.push_str("\x1B[48;5;");
    buf.push_u32(idx as u32);
    buf.push_byte(b'm');
}

/// Set 16-color foreground (0–15).
pub fn fg16(buf: &mut TermBuf, idx: u8) {
    if idx < 8 {
        buf.push_str("\x1B[");
        buf.push_u32(30 + idx as u32);
        buf.push_byte(b'm');
    } else {
        buf.push_str("\x1B[");
        buf.push_u32(90 + (idx - 8) as u32);
        buf.push_byte(b'm');
    }
}

/// Set 16-color background (0–15).
pub fn bg16(buf: &mut TermBuf, idx: u8) {
    if idx < 8 {
        buf.push_str("\x1B[");
        buf.push_u32(40 + idx as u32);
        buf.push_byte(b'm');
    } else {
        buf.push_str("\x1B[");
        buf.push_u32(100 + (idx - 8) as u32);
        buf.push_byte(b'm');
    }
}

/// Set reverse video (swap fg/bg).
pub fn reverse(buf: &mut TermBuf) {
    buf.push_str("\x1B[7m");
}

/// Write a fixed-width string field, padding with spaces or truncating.
/// Does not emit escape sequences — purely text.
pub fn write_fixed(buf: &mut TermBuf, s: &str, width: usize) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(width);
    buf.push_bytes(&bytes[..len]);
    for _ in len..width {
        buf.push_byte(b' ');
    }
}

/// Write `count` spaces.
pub fn spaces(buf: &mut TermBuf, count: usize) {
    for _ in 0..count { buf.push_byte(b' '); }
}

/// Draw a horizontal line of `ch` repeated `width` times.
pub fn hline(buf: &mut TermBuf, ch: u8, width: usize) {
    for _ in 0..width { buf.push_byte(ch); }
}
