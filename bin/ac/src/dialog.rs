//! Dialog and popup system for AC.
//!
//! Provides modal dialogs that are drawn over the main panels:
//! - Confirmation dialog (Yes/No)
//! - Single-line text input dialog
//! - Error/info message box
//! - Progress bar (for copy/delete operations)
//! - Sort menu
//! - Help screen

use crate::term::{self, TermBuf, color};
use crate::input::{self, Key};

// ─── Color scheme for dialogs ─────────────────────────────────────────────────

/// Dialog border / title color.
const DLG_FG:     u8 = color::BRIGHT_WHITE;
const DLG_BG:     u8 = color::BLUE;
const DLG_BTN_FG: u8 = color::BLACK;
const DLG_BTN_BG: u8 = color::BRIGHT_WHITE;
const DLG_ERR_BG: u8 = color::RED;

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Draw a centered box with a title.
/// Returns (box_col, box_row, box_width, box_height) — inner content area.
fn draw_box(buf: &mut TermBuf, cols: u32, rows: u32, width: u32, height: u32, title: &str, err: bool) -> (u32, u32) {
    let col = (cols.saturating_sub(width)) / 2 + 1;
    let row = (rows.saturating_sub(height)) / 2 + 1;

    let bg = if err { DLG_ERR_BG } else { DLG_BG };

    // Draw rows
    for r in 0..height {
        term::goto(buf, col, row + r);
        term::fg16(buf, DLG_FG);
        term::bg16(buf, bg);
        if r == 0 {
            // Top border
            buf.push_str("+");
            for _ in 0..width.saturating_sub(2) { buf.push_str("="); }
            buf.push_str("+");
        } else if r + 1 == height {
            // Bottom border
            buf.push_str("+");
            for _ in 0..width.saturating_sub(2) { buf.push_str("="); }
            buf.push_str("+");
        } else {
            // Side borders + fill
            buf.push_str("|");
            term::spaces(buf, width.saturating_sub(2) as usize);
            buf.push_str("|");
        }
    }

    // Title (centered on top border)
    if !title.is_empty() {
        let max_title = width.saturating_sub(4) as usize;
        let tlen = title.len().min(max_title);
        let tstart = col + (width.saturating_sub(tlen as u32 + 2)) / 2;
        term::goto(buf, tstart, row);
        term::fg16(buf, DLG_FG);
        term::bg16(buf, bg);
        buf.push_str("[ ");
        buf.push_bytes(&title.as_bytes()[..tlen]);
        buf.push_str(" ]");
    }

    term::reset(buf);
    (col + 1, row + 1) // inner area starts here
}

/// Draw a button label centered in a field of `width` chars.
fn draw_button(buf: &mut TermBuf, col: u32, row: u32, label: &str, selected: bool) {
    term::goto(buf, col, row);
    if selected {
        term::bg16(buf, DLG_BTN_BG);
        term::fg16(buf, DLG_BTN_FG);
        term::bold(buf);
    } else {
        term::fg16(buf, color::BRIGHT_WHITE);
    }
    buf.push_str("[ ");
    buf.push_bytes(label.as_bytes());
    buf.push_str(" ]");
    term::reset(buf);
}

// ─── Confirmation dialog ──────────────────────────────────────────────────────

/// Show a Yes/No confirmation dialog. Returns `true` if user chose Yes.
pub fn confirm(buf: &mut TermBuf, cols: u32, rows: u32, title: &str, message: &str) -> bool {
    // Box: width = max(message+4, 30), height = 7
    let w = (message.len() as u32 + 6).max(32).min(cols.saturating_sub(4));
    let h = 7u32;
    let (ic, ir) = draw_box(buf, cols, rows, w, h, title, false);

    // Message line
    term::goto(buf, ic + 1, ir + 1);
    term::fg16(buf, color::BRIGHT_WHITE);
    term::bg16(buf, DLG_BG);
    let mlen = message.len().min((w - 4) as usize);
    buf.push_bytes(&message.as_bytes()[..mlen]);

    buf.flush();

    // Button loop
    let mut yes = true;
    loop {
        // Redraw buttons
        let btn_row = ir + 3;
        let btn_col_yes = ic + (w / 2).saturating_sub(10);
        let btn_col_no  = btn_col_yes + 12;
        draw_button(buf, btn_col_yes, btn_row, " Yes ", yes);
        draw_button(buf, btn_col_no,  btn_row, " No  ", !yes);
        buf.flush();

        match input::read_key() {
            Key::Left | Key::Right | Key::Tab => yes = !yes,
            Key::Char(b'y') | Key::Char(b'Y') => { return true; }
            Key::Char(b'n') | Key::Char(b'N') => { return false; }
            Key::Enter  => return yes,
            Key::Escape | Key::Char(b'q') => return false,
            _ => {}
        }
    }
}

// ─── Input dialog ─────────────────────────────────────────────────────────────

const INPUT_BUF_SIZE: usize = 256;

/// Show a single-line text input dialog.
/// `default` is pre-filled into the input field.
/// Returns `Some(string)` if user pressed Enter, `None` if Escape.
pub fn input_line(
    buf: &mut TermBuf,
    cols: u32,
    rows: u32,
    title: &str,
    prompt: &str,
    default: &str,
) -> Option<[u8; INPUT_BUF_SIZE]> {
    let w = (prompt.len() as u32 + 20).max(50).min(cols.saturating_sub(4));
    let h = 6u32;
    let (ic, ir) = draw_box(buf, cols, rows, w, h, title, false);

    // Prompt
    term::goto(buf, ic + 1, ir + 1);
    term::fg16(buf, color::BRIGHT_WHITE);
    term::bg16(buf, DLG_BG);
    let plen = prompt.len().min((w - 4) as usize);
    buf.push_bytes(&prompt.as_bytes()[..plen]);
    buf.flush();

    // Input field
    let field_col = ic + 1;
    let field_row = ir + 2;
    let field_w   = (w.saturating_sub(4)) as usize;

    let mut ibuf = [0u8; INPUT_BUF_SIZE];
    let def = default.as_bytes();
    let n = def.len().min(INPUT_BUF_SIZE - 1);
    ibuf[..n].copy_from_slice(&def[..n]);
    let mut ilen = n;
    let mut cursor_pos = ilen;

    loop {
        // Draw input field
        term::goto(buf, field_col, field_row);
        term::fg16(buf, color::BLACK);
        term::bg16(buf, color::BRIGHT_WHITE);
        // Show the visible portion of the input
        let start = if cursor_pos > field_w { cursor_pos - field_w } else { 0 };
        let visible = &ibuf[start..ilen];
        let vlen = visible.len().min(field_w);
        buf.push_bytes(&visible[..vlen]);
        term::spaces(buf, field_w.saturating_sub(vlen));
        term::reset(buf);
        buf.flush();

        match input::read_key() {
            Key::Escape => return None,
            Key::Enter  => {
                if ilen == 0 { return None; }
                return Some(ibuf);
            }
            Key::Char(c) if c >= 32 && c < 127 => {
                if ilen < INPUT_BUF_SIZE - 1 {
                    // Insert at cursor_pos
                    if cursor_pos < ilen {
                        ibuf.copy_within(cursor_pos..ilen, cursor_pos + 1);
                    }
                    ibuf[cursor_pos] = c;
                    ilen += 1;
                    cursor_pos += 1;
                }
            }
            Key::Backspace => {
                if cursor_pos > 0 {
                    ibuf.copy_within(cursor_pos..ilen, cursor_pos - 1);
                    ilen -= 1;
                    cursor_pos -= 1;
                }
            }
            Key::Delete => {
                if cursor_pos < ilen {
                    ibuf.copy_within(cursor_pos + 1..ilen, cursor_pos);
                    ilen -= 1;
                }
            }
            Key::Left  => { if cursor_pos > 0 { cursor_pos -= 1; } }
            Key::Right => { if cursor_pos < ilen { cursor_pos += 1; } }
            Key::Home  => { cursor_pos = 0; }
            Key::End   => { cursor_pos = ilen; }
            _ => {}
        }
    }
}

/// Convenience: return the result as a &str slice (up to null or end of buffer).
pub fn input_buf_as_str(buf: &[u8; INPUT_BUF_SIZE]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(INPUT_BUF_SIZE);
    core::str::from_utf8(&buf[..end]).unwrap_or("")
}

// ─── Message / error box ─────────────────────────────────────────────────────

/// Show an informational message box. User dismisses with Enter or Escape.
pub fn message(buf: &mut TermBuf, cols: u32, rows: u32, title: &str, msg: &str, err: bool) {
    let w = (msg.len() as u32 + 6).max(30).min(cols.saturating_sub(4));
    let h = 6u32;
    let (ic, ir) = draw_box(buf, cols, rows, w, h, title, err);

    term::goto(buf, ic + 1, ir + 1);
    let bg = if err { DLG_ERR_BG } else { DLG_BG };
    term::fg16(buf, color::BRIGHT_WHITE);
    term::bg16(buf, bg);
    let mlen = msg.len().min((w - 4) as usize);
    buf.push_bytes(&msg.as_bytes()[..mlen]);

    // OK button
    let btn_col = ic + (w.saturating_sub(8)) / 2;
    draw_button(buf, btn_col, ir + 3, " OK  ", true);
    buf.flush();

    loop {
        match input::read_key() {
            Key::Enter | Key::Escape | Key::Char(b'q') => break,
            _ => {}
        }
    }
}

// ─── Progress display ─────────────────────────────────────────────────────────

/// Draw a progress bar overlay (non-interactive — caller controls loop).
pub fn progress(buf: &mut TermBuf, cols: u32, rows: u32, title: &str, msg: &str, pct: u32) {
    let w = 50u32.min(cols.saturating_sub(4));
    let h = 6u32;
    let (ic, ir) = draw_box(buf, cols, rows, w, h, title, false);

    // Message
    term::goto(buf, ic + 1, ir + 1);
    term::fg16(buf, color::BRIGHT_WHITE);
    term::bg16(buf, DLG_BG);
    let mlen = msg.len().min((w - 4) as usize);
    buf.push_bytes(&msg.as_bytes()[..mlen]);
    term::erase_to_eol(buf);

    // Bar
    let bar_w = (w.saturating_sub(4)) as usize;
    let filled = (pct.min(100) as usize * bar_w / 100).min(bar_w);
    term::goto(buf, ic + 1, ir + 2);
    term::bg16(buf, color::BRIGHT_BLUE);
    term::fg16(buf, color::BRIGHT_WHITE);
    for _ in 0..filled { buf.push_byte(b' '); }
    term::bg16(buf, DLG_BG);
    for _ in filled..bar_w { buf.push_byte(b' '); }
    term::reset(buf);
    buf.flush();
}

// ─── Help screen ─────────────────────────────────────────────────────────────

const HELP_LINES: &[&str] = &[
    "  F1  Help (this screen)",
    "  F2  User menu",
    "  F3  View file",
    "  F4  Edit file (vi)",
    "  F5  Copy",
    "  F6  Move / Rename",
    "  F7  Make directory",
    "  F8  Delete",
    "  F9  Menu bar",
    "  F10 Quit",
    "",
    "  Tab       Switch panels",
    "  Ins       Mark/unmark file",
    "  Enter     Open dir / execute",
    "  Backspace Go to parent dir",
    "  Ctrl+R    Reload panel",
    "  Ctrl+S    Sort menu",
    "  Ctrl+H    Toggle hidden files",
    "  Ctrl+Q    Quit",
];

pub fn show_help(buf: &mut TermBuf, cols: u32, rows: u32) {
    let h = (HELP_LINES.len() as u32 + 4).min(rows.saturating_sub(2));
    let w = 50u32.min(cols.saturating_sub(4));
    let (ic, ir) = draw_box(buf, cols, rows, w, h, " AC Help ", false);

    for (i, line) in HELP_LINES.iter().enumerate() {
        if i as u32 >= h.saturating_sub(3) { break; }
        term::goto(buf, ic, ir + i as u32);
        term::fg16(buf, color::BRIGHT_WHITE);
        term::bg16(buf, DLG_BG);
        let llen = line.len().min((w - 2) as usize);
        buf.push_bytes(&line.as_bytes()[..llen]);
        term::erase_to_eol(buf);
    }

    let btn_col = ic + (w.saturating_sub(8)) / 2;
    draw_button(buf, btn_col, ir + h - 2, " OK  ", true);
    term::reset(buf);
    buf.flush();

    loop {
        match input::read_key() {
            Key::Enter | Key::Escape | Key::F1 | Key::Char(b'q') => break,
            _ => {}
        }
    }
}

// ─── Sort menu ───────────────────────────────────────────────────────────────

use crate::fs::SortMode;

pub fn sort_menu(buf: &mut TermBuf, cols: u32, rows: u32) -> Option<(SortMode, bool)> {
    let options: &[(&str, SortMode, bool)] = &[
        ("Name            (ascending)",  SortMode::Name, false),
        ("Name            (descending)", SortMode::Name, true),
        ("Size            (ascending)",  SortMode::Size, false),
        ("Size            (descending)", SortMode::Size, true),
        ("Type/Extension  (ascending)",  SortMode::Kind, false),
        ("Type/Extension  (descending)", SortMode::Kind, true),
    ];

    let w = 42u32.min(cols.saturating_sub(4));
    let h = options.len() as u32 + 4;
    let (ic, ir) = draw_box(buf, cols, rows, w, h, " Sort Order ", false);

    let mut sel = 0usize;
    loop {
        for (i, (label, _, _)) in options.iter().enumerate() {
            term::goto(buf, ic, ir + i as u32);
            if i == sel {
                term::bg16(buf, color::BRIGHT_BLUE);
                term::fg16(buf, color::BRIGHT_WHITE);
                term::bold(buf);
            } else {
                term::bg16(buf, DLG_BG);
                term::fg16(buf, color::BRIGHT_WHITE);
            }
            term::write_fixed(buf, label, (w - 2) as usize);
            term::reset(buf);
        }
        buf.flush();

        match input::read_key() {
            Key::Up   => { if sel > 0 { sel -= 1; } }
            Key::Down => { if sel + 1 < options.len() { sel += 1; } }
            Key::Enter => {
                let (_, mode, rev) = options[sel];
                return Some((mode, rev));
            }
            Key::Escape | Key::Char(b'q') => return None,
            _ => {}
        }
    }
}
