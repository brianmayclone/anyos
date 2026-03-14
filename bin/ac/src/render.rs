//! Full-screen renderer for AC.
//!
//! Renders the two-panel layout, menu bar, command bar (F-key row),
//! and status bar into a single `TermBuf` that is flushed in one shot.
//!
//! Layout (all row/col numbers are 1-based):
//!
//!   Row 1          : Menu bar  (Left | File | Commands | Options | Right)
//!   Row 2 – H-2   : Panels side by side, with a vertical separator
//!   Row H-1        : Command line / mini-shell input
//!   Row H          : Function key labels (F1 Help … F10 Quit)

use crate::term::{self, TermBuf, color};
use crate::panel::Panel;
use crate::fs::{DirEntry, EntryKind, fmt_size};

// ─── Color palette ────────────────────────────────────────────────────────────

/// Dracula/Midnight-Commander inspired palette constants.
mod pal {
    use crate::term::color;

    // Menu bar
    pub const MENU_FG:         u8 = color::BLACK;
    pub const MENU_BG:         u8 = color::BRIGHT_WHITE; // light gray
    pub const MENU_HOT_FG:     u8 = color::BRIGHT_RED;

    // Panel
    pub const PANEL_BORDER_FG: u8 = color::BRIGHT_CYAN;
    pub const PANEL_TITLE_FG:  u8 = color::BRIGHT_WHITE;
    pub const PANEL_BG:        u8 = 4; // blue
    pub const PANEL_FILE_FG:   u8 = color::BRIGHT_WHITE;
    pub const PANEL_DIR_FG:    u8 = color::BRIGHT_CYAN;
    pub const PANEL_LINK_FG:   u8 = color::BRIGHT_MAGENTA;
    pub const PANEL_EXEC_FG:   u8 = color::BRIGHT_GREEN;
    pub const PANEL_SEL_FG:    u8 = color::BRIGHT_YELLOW;
    pub const CURSOR_FG:       u8 = color::BLACK;
    pub const CURSOR_BG:       u8 = color::BRIGHT_CYAN;
    pub const MARKED_FG:       u8 = color::BRIGHT_YELLOW;

    // Status bar
    pub const STATUS_FG:       u8 = color::BLACK;
    pub const STATUS_BG:       u8 = color::BRIGHT_CYAN;

    // Function key bar
    pub const FNBAR_NUM_FG:    u8 = color::BLACK;
    pub const FNBAR_NUM_BG:    u8 = color::BRIGHT_WHITE;
    pub const FNBAR_LBL_FG:    u8 = color::BRIGHT_WHITE;
    pub const FNBAR_LBL_BG:    u8 = 4;
}

use pal::*;

// ─── Function key labels ──────────────────────────────────────────────────────

const FN_LABELS: &[&str] = &[
    "Help", "Menu", "View", "Edit", "Copy",
    "Move", "MkDir","Del ", "PMenu","Quit",
];

// ─── Main render entry point ──────────────────────────────────────────────────

/// Render the entire AC UI into `buf`.
/// `cols`, `rows` — terminal dimensions.
/// `left`, `right` — the two panels.
/// `cmdline` — current command-line buffer.
pub fn render_all(
    buf: &mut TermBuf,
    cols: u32,
    rows: u32,
    left:  &Panel,
    right: &Panel,
    cmdline: &[u8],
    cmdline_len: usize,
) {
    term::clear_screen(buf);
    term::home(buf);

    render_menubar(buf, cols);
    render_panels(buf, cols, rows, left, right);
    render_cmdline(buf, cols, rows, cmdline, cmdline_len);
    render_fnbar(buf, cols, rows);
}

// ─── Menu bar ────────────────────────────────────────────────────────────────

fn render_menubar(buf: &mut TermBuf, cols: u32) {
    term::goto(buf, 1, 1);
    term::fg16(buf, MENU_FG);
    term::bg16(buf, MENU_BG);
    term::spaces(buf, cols as usize);
    term::goto(buf, 1, 1);

    // Menu items with highlighted hotkey (first letter)
    let items: &[(&str, &str)] = &[
        ("L", "eft"),
        (" ", ""),
        ("F", "ile"),
        (" ", ""),
        ("C", "ommands"),
        (" ", ""),
        ("O", "ptions"),
        (" ", ""),
        ("R", "ight"),
    ];
    for (hot, rest) in items {
        term::fg16(buf, MENU_HOT_FG);
        term::bg16(buf, MENU_BG);
        buf.push_bytes(hot.as_bytes());
        term::fg16(buf, MENU_FG);
        buf.push_bytes(rest.as_bytes());
        buf.push_byte(b' ');
    }
    term::reset(buf);
}

// ─── Panel rendering ─────────────────────────────────────────────────────────

/// Panel content occupies rows 2 .. rows-2 (1-based).
fn panel_rows(rows: u32) -> u32 {
    rows.saturating_sub(3) // menu + cmdline + fnbar
}

fn render_panels(buf: &mut TermBuf, cols: u32, rows: u32, left: &Panel, right: &Panel) {
    let p_rows = panel_rows(rows);
    if p_rows < 3 { return; }

    // Divide horizontally: left takes floor(cols/2), right takes ceil(cols/2)
    let left_w  = cols / 2;
    let right_w = cols - left_w;

    render_panel(buf, 1,          2, left_w,  p_rows, left,  left.focused);
    render_panel(buf, left_w + 1, 2, right_w, p_rows, right, right.focused);
}

fn render_panel(
    buf:     &mut TermBuf,
    col:     u32,
    row:     u32,
    width:   u32,
    height:  u32,
    panel:   &Panel,
    focused: bool,
) {
    if width < 5 || height < 3 { return; }

    let border_fg = if focused { PANEL_BORDER_FG } else { color::CYAN };

    // ── Top border ────────────────────────────────────────────────────────────
    term::goto(buf, col, row);
    term::fg16(buf, border_fg);
    term::bg16(buf, PANEL_BG);
    buf.push_str("┌");
    for _ in 0..width.saturating_sub(2) { buf.push_str("─"); }
    buf.push_str("┐");

    // Title (path) centered on top border
    let path = panel.path();
    let max_title = width.saturating_sub(4) as usize;
    let tlen = path.len().min(max_title);
    // Show the last `max_title` chars of the path
    let tstart_byte = path.len().saturating_sub(tlen);
    let title_str = &path[tstart_byte..];
    let twidth = title_str.len();
    let title_col = col + (width.saturating_sub(twidth as u32 + 2)) / 2;
    term::goto(buf, title_col, row);
    term::fg16(buf, PANEL_TITLE_FG);
    term::bg16(buf, PANEL_BG);
    if focused { term::bold(buf); }
    buf.push_str("[ ");
    buf.push_bytes(title_str.as_bytes());
    buf.push_str(" ]");
    term::reset(buf);

    // ── Column header ─────────────────────────────────────────────────────────
    let content_row = row + 1;
    let content_col = col + 1;
    let content_w   = width.saturating_sub(2) as usize;

    term::goto(buf, content_col, content_row);
    term::fg16(buf, color::BRIGHT_YELLOW);
    term::bg16(buf, PANEL_BG);
    // Layout: "Name" left-padded, "Size" right-padded (7 chars), "Type" (4 chars)
    let name_w = content_w.saturating_sub(13); // 7 (size) + 1 (space) + 4 (type) + 1
    term::write_fixed(buf, "Name", name_w);
    buf.push_str("   Size  Ext");
    term::erase_to_eol(buf);
    term::reset(buf);

    // ── File list ─────────────────────────────────────────────────────────────
    let list_row     = content_row + 1;
    let list_height  = height.saturating_sub(4); // top + header + bottom + status
    let visible_rows = list_height as usize;

    // Adjust panel scroll for current visible_rows
    // (we can't call panel.ensure_visible_with() here since panel is &Panel)
    let scroll = {
        let mut sc = panel.scroll;
        if panel.cursor < sc { sc = panel.cursor; }
        if panel.entry_count > 0 && panel.cursor >= sc + visible_rows {
            sc = panel.cursor + 1 - visible_rows;
        }
        sc
    };

    for vi in 0..visible_rows {
        let ei = scroll + vi;
        let list_y = list_row + vi as u32;

        term::goto(buf, content_col, list_y);
        term::bg16(buf, PANEL_BG);

        if ei >= panel.entry_count {
            // Empty row
            term::spaces(buf, content_w);
            continue;
        }

        let entry    = &panel.entries[ei];
        let is_cur   = ei == panel.cursor;
        let is_marked = panel.marked[ei];

        // Choose colors
        if is_cur && focused {
            term::fg16(buf, CURSOR_FG);
            term::bg16(buf, CURSOR_BG);
        } else if is_marked {
            term::fg16(buf, MARKED_FG);
            term::bg16(buf, PANEL_BG);
        } else {
            let fg = entry_fg(entry);
            term::fg16(buf, fg);
            term::bg16(buf, PANEL_BG);
        }

        render_entry_row(buf, entry, content_w, is_marked, is_cur && focused);
    }
    term::reset(buf);

    // ── Bottom border ─────────────────────────────────────────────────────────
    let bottom_row = row + height - 1;
    term::goto(buf, col, bottom_row);
    term::fg16(buf, border_fg);
    term::bg16(buf, PANEL_BG);
    buf.push_str("└");
    for _ in 0..width.saturating_sub(2) { buf.push_str("─"); }
    buf.push_str("┘");

    // ── Status line inside bottom border ─────────────────────────────────────
    let status_row = bottom_row - 1;
    term::goto(buf, content_col, status_row);
    term::fg16(buf, STATUS_FG);
    term::bg16(buf, STATUS_BG);
    let mc = panel.marked_count();
    if mc > 0 {
        // Show marked count + total size of marked files
        let mut sbuf = [0u8; 16];
        let mut total_marked: u64 = 0;
        for i in 0..panel.entry_count {
            if panel.marked[i] { total_marked += panel.entries[i].size; }
        }
        let size_s = fmt_size(&mut sbuf, total_marked);
        // Build status string
        let status = "Marked:";
        buf.push_bytes(status.as_bytes());
        // print mc as number
        let mut tmp = [0u8; 8];
        let mc_s = u32_to_str(&mut tmp, mc as u32);
        buf.push_bytes(mc_s);
        buf.push_str(" (");
        buf.push_bytes(size_s.as_bytes());
        buf.push_byte(b')');
    } else if let Some(e) = panel.current() {
        // Show current file info
        let mut sbuf = [0u8; 16];
        let size_s = if e.is_dir() { "DIR" } else { fmt_size(&mut sbuf, e.size) };
        let avail = content_w.saturating_sub(2);
        let name = e.name_str();
        let nlen = name.len().min(avail.saturating_sub(size_s.len() + 1));
        buf.push_bytes(&name.as_bytes()[..nlen]);
        term::spaces(buf, avail.saturating_sub(nlen + size_s.len()));
        buf.push_bytes(size_s.as_bytes());
    } else {
        buf.push_str("Empty");
    }
    term::erase_to_eol(buf);
    term::reset(buf);
}

fn entry_fg(e: &DirEntry) -> u8 {
    match e.kind {
        EntryKind::Dir     => PANEL_DIR_FG,
        EntryKind::Symlink => PANEL_LINK_FG,
        EntryKind::File    => PANEL_FILE_FG,
        EntryKind::Unknown => color::WHITE,
    }
}

fn render_entry_row(buf: &mut TermBuf, entry: &DirEntry, width: usize, marked: bool, _cursor: bool) {
    // Layout: [mark] name ... size ext
    // size column: 7 chars right-aligned, ext: 4 chars
    let name_w = width.saturating_sub(13);

    // Mark prefix
    if marked {
        buf.push_byte(b'*');
    } else {
        buf.push_byte(b' ');
    }

    // Name
    let name = entry.name_str();
    let is_dir = entry.is_dir();
    let display_name = name;
    let dlen = display_name.len().min(name_w.saturating_sub(1));
    buf.push_bytes(&display_name.as_bytes()[..dlen]);

    // Suffix for directories
    if is_dir && dlen + 1 < name_w {
        buf.push_byte(b'/');
        term::spaces(buf, name_w.saturating_sub(dlen + 1));
    } else {
        term::spaces(buf, name_w.saturating_sub(dlen));
    }

    // Size field (7 chars)
    if entry.is_dir() {
        if entry.is_dotdot() || entry.is_dot() {
            buf.push_str("   UP--");
        } else {
            buf.push_str("   DIR ");
        }
    } else {
        let mut sbuf = [0u8; 16];
        let size_s = fmt_size(&mut sbuf, entry.size);
        let sl = size_s.len().min(6);
        for _ in sl..6 { buf.push_byte(b' '); }
        buf.push_bytes(&size_s.as_bytes()[..sl]);
        buf.push_byte(b' ');
    }

    // Extension (up to 4 chars)
    let ext = get_ext(entry.name_str());
    let elen = ext.len().min(4);
    buf.push_bytes(&ext.as_bytes()[..elen]);
    for _ in elen..4 { buf.push_byte(b' '); }
}

fn get_ext(name: &str) -> &str {
    if let Some(dot) = name.rfind('.') {
        if dot + 1 < name.len() && dot > 0 {
            return &name[dot + 1..];
        }
    }
    ""
}

// ─── Command line ─────────────────────────────────────────────────────────────

fn render_cmdline(buf: &mut TermBuf, cols: u32, rows: u32, cmdline: &[u8], cmdline_len: usize) {
    let row = rows - 1;
    term::goto(buf, 1, row);
    term::fg16(buf, color::BRIGHT_WHITE);
    term::bg16(buf, 4); // dark blue bg
    buf.push_str("ac:$ ");
    let vis_w = (cols as usize).saturating_sub(6);
    let start = if cmdline_len > vis_w { cmdline_len - vis_w } else { 0 };
    let shown = cmdline_len.saturating_sub(start);
    if shown > 0 {
        buf.push_bytes(&cmdline[start..start + shown]);
    }
    term::erase_to_eol(buf);
    term::reset(buf);
}

// ─── Function key bar ─────────────────────────────────────────────────────────

fn render_fnbar(buf: &mut TermBuf, cols: u32, rows: u32) {
    term::goto(buf, 1, rows);
    let cell_w = (cols as usize) / FN_LABELS.len();

    for (i, label) in FN_LABELS.iter().enumerate() {
        let num = i + 1;
        // Number in bright bg
        term::fg16(buf, FNBAR_NUM_FG);
        term::bg16(buf, FNBAR_NUM_BG);
        term::bold(buf);
        buf.push_u32(num as u32);
        // Label
        term::fg16(buf, FNBAR_LBL_FG);
        term::bg16(buf, FNBAR_LBL_BG);
        let llen = label.len().min(cell_w.saturating_sub(2));
        buf.push_bytes(&label.as_bytes()[..llen]);
        // Pad to cell width
        let used = 1 + if num >= 10 { 2 } else { 1 } + llen;
        term::spaces(buf, cell_w.saturating_sub(used));
        term::reset(buf);
    }
    term::erase_to_eol(buf);
    term::reset(buf);
}

// ─── Util ────────────────────────────────────────────────────────────────────

fn u32_to_str<'a>(buf: &'a mut [u8; 8], mut n: u32) -> &'a [u8] {
    if n == 0 { buf[0] = b'0'; return &buf[..1]; }
    let mut i = 8usize;
    while n > 0 { i -= 1; buf[i] = b'0' + (n % 10) as u8; n /= 10; }
    &buf[i..]
}
