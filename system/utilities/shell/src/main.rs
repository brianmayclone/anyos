#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;
use anyos_std::process;
use anyos_std::ipc;
use anyos_std::ui::window;
use libshellcommon;

anyos_std::entry!(main);

// ─── Constants ───────────────────────────────────────────────────────────────

const TEXT_PAD: u16 = 4;
const MAX_SCROLLBACK: usize = 1000;

// Font sizes available for cycling
const FONT_SIZES: &[u16] = &[10, 12, 14, 16, 18, 20];
const DEFAULT_FONT_SIZE_IDX: usize = 2; // 14px

// Colors (ARGB) — default ANSI palette
const COLOR_BG: u32 = 0xFF1E1E28;
const COLOR_FG: u32 = 0xFFCCCCCC;
const COLOR_DIM: u32 = 0xFF969696;

// ANSI standard colors (normal)
const ANSI_COLORS: [u32; 8] = [
    0xFF000000, // 0 black
    0xFFCC0000, // 1 red
    0xFF00CC00, // 2 green
    0xFFCCCC00, // 3 yellow
    0xFF5577FF, // 4 blue
    0xFFCC00CC, // 5 magenta
    0xFF00CCCC, // 6 cyan
    0xFFCCCCCC, // 7 white
];

// ANSI bright colors
const ANSI_BRIGHT: [u32; 8] = [
    0xFF555555, // 0 bright black (gray)
    0xFFFF5555, // 1 bright red
    0xFF55FF55, // 2 bright green
    0xFFFFFF55, // 3 bright yellow
    0xFF5577FF, // 4 bright blue
    0xFFFF55FF, // 5 bright magenta
    0xFF55FFFF, // 6 bright cyan
    0xFFFFFFFF, // 7 bright white
];

// Key codes (must match desktop.rs encode_key)
const KEY_ENTER: u32 = 0x100;
const KEY_BACKSPACE: u32 = 0x101;
const KEY_TAB: u32 = 0x102;
const KEY_ESCAPE: u32 = 0x103;
const KEY_UP: u32 = 0x105;
const KEY_DOWN: u32 = 0x106;
const KEY_LEFT: u32 = 0x107;
const KEY_RIGHT: u32 = 0x108;
const KEY_DELETE: u32 = 0x120;
const KEY_HOME: u32 = 0x121;
const KEY_END: u32 = 0x122;
const KEY_PAGE_UP: u32 = 0x123;
const KEY_PAGE_DOWN: u32 = 0x124;

// Event types
const EVENT_KEY_DOWN: u32 = 1;
const EVENT_RESIZE: u32 = 3;
const EVENT_MOUSE_SCROLL: u32 = 7;
const EVENT_WINDOW_CLOSE: u32 = 8;

// Modifier flags
const MOD_CTRL: u32 = 2;

// ─── Terminal Cell ───────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Cell {
    ch: char,
    fg: u32,
    bg: u32,
}

impl Cell {
    fn blank() -> Self {
        Cell { ch: ' ', fg: COLOR_FG, bg: 0 }
    }
}

// ─── ANSI Parser State ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum AnsiState {
    Normal,
    Escape,    // saw \x1b
    Csi,       // saw \x1b[
}

struct AnsiParser {
    state: AnsiState,
    params: [u16; 16],
    param_count: usize,
    current_param: u16,
    has_digit: bool,
}

impl AnsiParser {
    fn new() -> Self {
        AnsiParser {
            state: AnsiState::Normal,
            params: [0; 16],
            param_count: 0,
            current_param: 0,
            has_digit: false,
        }
    }

    fn reset(&mut self) {
        self.state = AnsiState::Normal;
        self.param_count = 0;
        self.current_param = 0;
        self.has_digit = false;
    }
}

// ─── Terminal Buffer ─────────────────────────────────────────────────────────

struct TerminalBuffer {
    lines: Vec<Vec<Cell>>,
    cols: usize,
    visible_rows: usize,
    cursor_row: usize,
    cursor_col: usize,
    scroll_offset: usize,
    // Current drawing attributes
    fg_color: u32,
    bg_color: u32,
    bold: bool,
    // ANSI parser
    ansi: AnsiParser,
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
            fg_color: COLOR_FG,
            bg_color: 0,
            bold: false,
            ansi: AnsiParser::new(),
        }
    }

    fn ensure_line(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push(Vec::new());
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(Vec::new());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    fn scroll_down(&mut self, n: usize) {
        let max = self.lines.len().saturating_sub(self.visible_rows);
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    fn scroll_to_bottom(&mut self) {
        if self.lines.len() > self.visible_rows {
            self.scroll_offset = self.lines.len() - self.visible_rows;
        } else {
            self.scroll_offset = 0;
        }
    }

    /// Feed a byte stream (UTF-8) through the ANSI parser into the buffer.
    fn feed(&mut self, data: &[u8]) {
        for &b in data {
            match self.ansi.state {
                AnsiState::Normal => {
                    if b == 0x1b {
                        self.ansi.state = AnsiState::Escape;
                    } else if b == b'\n' {
                        self.newline();
                    } else if b == b'\r' {
                        self.cursor_col = 0;
                    } else if b == 8 || b == 0x7f {
                        // Backspace
                        if self.cursor_col > 0 {
                            self.cursor_col -= 1;
                            self.ensure_line(self.cursor_row);
                            if self.cursor_col < self.lines[self.cursor_row].len() {
                                self.lines[self.cursor_row][self.cursor_col] = Cell::blank();
                            }
                        }
                    } else if b == b'\t' {
                        // Tab: advance to next 8-column boundary
                        let next = (self.cursor_col + 8) & !7;
                        let next = next.min(self.cols.saturating_sub(1));
                        while self.cursor_col < next {
                            self.put_char(' ');
                        }
                    } else if b >= 0x20 {
                        // Printable ASCII (we ignore multi-byte UTF-8 for now)
                        self.put_char(b as char);
                    }
                }
                AnsiState::Escape => {
                    if b == b'[' {
                        self.ansi.state = AnsiState::Csi;
                        self.ansi.params = [0; 16];
                        self.ansi.param_count = 0;
                        self.ansi.current_param = 0;
                        self.ansi.has_digit = false;
                    } else if b == b'c' {
                        // ESC c = full reset
                        self.clear();
                        self.fg_color = COLOR_FG;
                        self.bg_color = 0;
                        self.bold = false;
                        self.ansi.reset();
                    } else {
                        // Unknown escape, ignore
                        self.ansi.reset();
                    }
                }
                AnsiState::Csi => {
                    if b >= b'0' && b <= b'9' {
                        self.ansi.current_param = self.ansi.current_param * 10 + (b - b'0') as u16;
                        self.ansi.has_digit = true;
                    } else if b == b';' {
                        if self.ansi.param_count < 16 {
                            self.ansi.params[self.ansi.param_count] = self.ansi.current_param;
                            self.ansi.param_count += 1;
                        }
                        self.ansi.current_param = 0;
                        self.ansi.has_digit = false;
                    } else {
                        // Final byte — push last param
                        if self.ansi.has_digit || self.ansi.param_count > 0 {
                            if self.ansi.param_count < 16 {
                                self.ansi.params[self.ansi.param_count] = self.ansi.current_param;
                                self.ansi.param_count += 1;
                            }
                        }
                        self.handle_csi(b);
                        self.ansi.reset();
                    }
                }
            }
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.newline();
        }
        self.ensure_line(self.cursor_row);
        let row = &mut self.lines[self.cursor_row];
        while row.len() <= self.cursor_col {
            row.push(Cell::blank());
        }
        row[self.cursor_col] = Cell {
            ch,
            fg: self.fg_color,
            bg: self.bg_color,
        };
        self.cursor_col += 1;
    }

    fn newline(&mut self) {
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.ensure_line(self.cursor_row);
        // Auto-scroll
        if self.cursor_row >= self.scroll_offset + self.visible_rows {
            self.scroll_offset = self.cursor_row - self.visible_rows + 1;
        }
        // Trim scrollback
        if self.lines.len() > MAX_SCROLLBACK {
            let excess = self.lines.len() - MAX_SCROLLBACK;
            self.lines.drain(0..excess);
            self.cursor_row = self.cursor_row.saturating_sub(excess);
            self.scroll_offset = self.scroll_offset.saturating_sub(excess);
        }
    }

    fn handle_csi(&mut self, cmd: u8) {
        let p = &self.ansi.params;
        let n = self.ansi.param_count;

        match cmd {
            b'm' => self.handle_sgr(),
            b'A' => {
                // Cursor Up
                let count = if n > 0 && p[0] > 0 { p[0] as usize } else { 1 };
                self.cursor_row = self.cursor_row.saturating_sub(count);
            }
            b'B' => {
                // Cursor Down
                let count = if n > 0 && p[0] > 0 { p[0] as usize } else { 1 };
                self.cursor_row += count;
                self.ensure_line(self.cursor_row);
            }
            b'C' => {
                // Cursor Forward
                let count = if n > 0 && p[0] > 0 { p[0] as usize } else { 1 };
                self.cursor_col = (self.cursor_col + count).min(self.cols.saturating_sub(1));
            }
            b'D' => {
                // Cursor Back
                let count = if n > 0 && p[0] > 0 { p[0] as usize } else { 1 };
                self.cursor_col = self.cursor_col.saturating_sub(count);
            }
            b'H' | b'f' => {
                // Cursor Position
                let row = if n > 0 && p[0] > 0 { p[0] as usize - 1 } else { 0 };
                let col = if n > 1 && p[1] > 0 { p[1] as usize - 1 } else { 0 };
                self.cursor_row = self.scroll_offset + row;
                self.cursor_col = col.min(self.cols.saturating_sub(1));
                self.ensure_line(self.cursor_row);
            }
            b'J' => {
                // Erase in Display
                let mode = if n > 0 { p[0] } else { 0 };
                match mode {
                    0 => {
                        // Clear from cursor to end
                        self.ensure_line(self.cursor_row);
                        self.lines[self.cursor_row].truncate(self.cursor_col);
                        let next = self.cursor_row + 1;
                        if next < self.lines.len() {
                            self.lines.truncate(next);
                        }
                    }
                    1 => {
                        // Clear from start to cursor
                        for r in 0..self.cursor_row {
                            if r < self.lines.len() {
                                self.lines[r].clear();
                            }
                        }
                        self.ensure_line(self.cursor_row);
                        for c in 0..self.cursor_col.min(self.lines[self.cursor_row].len()) {
                            self.lines[self.cursor_row][c] = Cell::blank();
                        }
                    }
                    2 | 3 => {
                        // Clear entire screen
                        self.clear();
                    }
                    _ => {}
                }
            }
            b'K' => {
                // Erase in Line
                let mode = if n > 0 { p[0] } else { 0 };
                self.ensure_line(self.cursor_row);
                let row = &mut self.lines[self.cursor_row];
                match mode {
                    0 => row.truncate(self.cursor_col),
                    1 => {
                        for c in 0..self.cursor_col.min(row.len()) {
                            row[c] = Cell::blank();
                        }
                    }
                    2 => row.clear(),
                    _ => {}
                }
            }
            _ => {} // Ignore unsupported CSI sequences
        }
    }

    fn handle_sgr(&mut self) {
        let n = self.ansi.param_count;
        if n == 0 {
            // ESC[m = reset
            self.fg_color = COLOR_FG;
            self.bg_color = 0;
            self.bold = false;
            return;
        }

        let mut i = 0;
        while i < n {
            let code = self.ansi.params[i];
            match code {
                0 => {
                    self.fg_color = COLOR_FG;
                    self.bg_color = 0;
                    self.bold = false;
                }
                1 => self.bold = true,
                2 => self.bold = false, // dim
                22 => self.bold = false,
                30..=37 => {
                    let idx = (code - 30) as usize;
                    self.fg_color = if self.bold { ANSI_BRIGHT[idx] } else { ANSI_COLORS[idx] };
                }
                38 => {
                    // Extended foreground: 38;5;N (256-color) or 38;2;R;G;B (truecolor)
                    if i + 1 < n && self.ansi.params[i + 1] == 5 && i + 2 < n {
                        self.fg_color = color_256(self.ansi.params[i + 2]);
                        i += 2;
                    } else if i + 1 < n && self.ansi.params[i + 1] == 2 && i + 4 < n {
                        let r = self.ansi.params[i + 2] as u32;
                        let g = self.ansi.params[i + 3] as u32;
                        let b = self.ansi.params[i + 4] as u32;
                        self.fg_color = 0xFF000000 | (r << 16) | (g << 8) | b;
                        i += 4;
                    }
                }
                39 => self.fg_color = COLOR_FG,
                40..=47 => {
                    let idx = (code - 40) as usize;
                    self.bg_color = ANSI_COLORS[idx];
                }
                48 => {
                    // Extended background
                    if i + 1 < n && self.ansi.params[i + 1] == 5 && i + 2 < n {
                        self.bg_color = color_256(self.ansi.params[i + 2]);
                        i += 2;
                    } else if i + 1 < n && self.ansi.params[i + 1] == 2 && i + 4 < n {
                        let r = self.ansi.params[i + 2] as u32;
                        let g = self.ansi.params[i + 3] as u32;
                        let b = self.ansi.params[i + 4] as u32;
                        self.bg_color = 0xFF000000 | (r << 16) | (g << 8) | b;
                        i += 4;
                    }
                }
                49 => self.bg_color = 0,
                90..=97 => {
                    let idx = (code - 90) as usize;
                    self.fg_color = ANSI_BRIGHT[idx];
                }
                100..=107 => {
                    let idx = (code - 100) as usize;
                    self.bg_color = ANSI_BRIGHT[idx];
                }
                _ => {} // Ignore unknown SGR codes
            }
            i += 1;
        }
    }
}

/// Convert 256-color index to ARGB.
fn color_256(idx: u16) -> u32 {
    let i = idx as usize;
    if i < 8 {
        ANSI_COLORS[i]
    } else if i < 16 {
        ANSI_BRIGHT[i - 8]
    } else if i < 232 {
        // 6x6x6 color cube
        let n = i - 16;
        let b = (n % 6) as u32;
        let g = ((n / 6) % 6) as u32;
        let r = (n / 36) as u32;
        let scale = |v: u32| if v == 0 { 0 } else { 55 + v * 40 };
        0xFF000000 | (scale(r) << 16) | (scale(g) << 8) | scale(b)
    } else {
        // Grayscale ramp (232..=255)
        let v = (8 + (i - 232) * 10) as u32;
        0xFF000000 | (v << 16) | (v << 8) | v
    }
}

// ─── Font metrics helper ─────────────────────────────────────────────────────

struct FontMetrics {
    font_id: u16,
    size: u16,
    cell_w: u16,
    cell_h: u16,
}

impl FontMetrics {
    fn measure(font_id: u16, size: u16) -> Self {
        // Measure a representative character to get cell dimensions
        let (w, h) = window::font_measure(font_id, size, "M");
        // Use the measured width as cell width, height + 2 for line spacing
        let cell_w = if w > 0 { w as u16 } else { size / 2 + 1 };
        let cell_h = if h > 0 { h as u16 + 2 } else { size + 4 };
        FontMetrics { font_id, size, cell_w, cell_h }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render(win_id: u32, buf: &TerminalBuffer, font: &FontMetrics, win_w: u32, win_h: u32) {
    // Clear background
    window::fill_rect(win_id, 0, 0, win_w as u16, win_h as u16, COLOR_BG);

    let start = buf.scroll_offset;
    let end = (start + buf.visible_rows).min(buf.lines.len());

    // Build runs of same-colored text per line for efficient rendering
    for screen_row in 0..(end - start) {
        let line_idx = start + screen_row;
        let line = &buf.lines[line_idx];
        let py = TEXT_PAD as i32 + (screen_row as i32) * (font.cell_h as i32);

        if line.is_empty() {
            continue;
        }

        // Draw background cells that have a non-zero bg
        for (col, cell) in line.iter().enumerate() {
            if cell.bg != 0 {
                let px = TEXT_PAD as i16 + (col as i16) * (font.cell_w as i16);
                window::fill_rect(win_id, px, py as i16, font.cell_w, font.cell_h, cell.bg);
            }
        }

        // Batch text by foreground color for efficient rendering
        let mut run_start = 0usize;
        let mut run_color = if !line.is_empty() { line[0].fg } else { COLOR_FG };
        let mut text_buf = String::new();

        for (col, cell) in line.iter().enumerate() {
            if cell.fg != run_color && !text_buf.is_empty() {
                let px = TEXT_PAD as i16 + (run_start as i16) * (font.cell_w as i16);
                window::draw_text_ex(win_id, px, py as i16, run_color, font.font_id, font.size, &text_buf);
                text_buf.clear();
                run_start = col;
                run_color = cell.fg;
            }
            if text_buf.is_empty() {
                run_start = col;
                run_color = cell.fg;
            }
            text_buf.push(cell.ch);
        }
        if !text_buf.is_empty() {
            let px = TEXT_PAD as i16 + (run_start as i16) * (font.cell_w as i16);
            window::draw_text_ex(win_id, px, py as i16, run_color, font.font_id, font.size, &text_buf);
        }
    }

    // Draw cursor (block)
    let cursor_screen_row = buf.cursor_row as i32 - buf.scroll_offset as i32;
    if cursor_screen_row >= 0 && (cursor_screen_row as usize) < buf.visible_rows {
        let cx = TEXT_PAD + (buf.cursor_col as u16) * font.cell_w;
        let cy = TEXT_PAD + (cursor_screen_row as u16) * font.cell_h;
        window::fill_rect(win_id, cx as i16, cy as i16, font.cell_w, font.cell_h, 0xFFCCCCCC);
    }

    window::present(win_id);
}

// ─── ShellOutput adapter for TerminalBuffer ──────────────────────────────────

struct BufOutput<'a> {
    buf: &'a mut TerminalBuffer,
}

impl<'a> libshellcommon::interpreter::ShellOutput for BufOutput<'a> {
    fn write_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.write_char(ch);
        }
    }

    fn write_char(&mut self, ch: char) {
        if ch == '\n' {
            self.buf.newline();
        } else {
            self.buf.put_char(ch);
        }
    }
}

// ─── AppBuiltins for Shell app ───────────────────────────────────────────────

struct ShellAppBuiltins {
    clear_flag: bool,
}

impl ShellAppBuiltins {
    fn new() -> Self {
        ShellAppBuiltins { clear_flag: false }
    }
}

impl libshellcommon::interpreter::AppBuiltins for ShellAppBuiltins {
    fn handle(&mut self, cmd: &str, _args: &str, _shell: &mut libshellcommon::interpreter::ShellState, out: &mut dyn libshellcommon::interpreter::ShellOutput) -> bool {
        match cmd {
            "help" => {
                out.write_line("anyOS Shell");
                out.write_line("Built-in commands:");
                out.write_line("  alias, bg, cd, clear, echo, eval, exit, export, fg");
                out.write_line("  help, jobs, pwd, reboot, set, shutdown, source, su");
                out.write_line("  uname, unalias, unset");
                true
            }
            _ => false,
        }
    }

    fn on_cd(&mut self, _new_cwd: &str) {
        // Nothing extra needed
    }

    fn clear_screen(&mut self) {
        self.clear_flag = true;
    }
}

// ─── Foreground process state ────────────────────────────────────────────────

struct ForegroundProcess {
    tid: u32,
    stdout_pipe: u32,
    stdin_pipe: u32,
    command: String,
    extra_pipes: Vec<u32>,
}

impl ForegroundProcess {
    fn close_pipes(&self) {
        if self.stdout_pipe != 0 {
            ipc::pipe_close(self.stdout_pipe);
        }
        if self.stdin_pipe != 0 {
            ipc::pipe_close(self.stdin_pipe);
        }
        for &p in &self.extra_pipes {
            ipc::pipe_close(p);
        }
    }
}

// ─── Builtin commands list for tab completion ────────────────────────────────

const SHELL_BUILTINS: &[&str] = &[
    "alias", "bg", "cd", "clear", "echo", "eval", "exit", "export",
    "fg", "help", "jobs", "pwd", "reboot", "set", "shutdown", "source",
    "su", "uname", "unalias", "unset",
];

// ─── Helper: draw prompt + current input on terminal buffer ──────────────────

/// Write a str to the terminal buffer character by character (Unicode-safe).
fn buf_write_str(buf: &mut TerminalBuffer, s: &str) {
    for ch in s.chars() {
        if ch == '\n' {
            buf.newline();
        } else {
            buf.put_char(ch);
        }
    }
}

fn redraw_prompt_line(buf: &mut TerminalBuffer, shell: &libshellcommon::interpreter::ShellState) {
    // Erase current line content
    buf.ensure_line(buf.cursor_row);
    buf.lines[buf.cursor_row].clear();
    buf.cursor_col = 0;

    // Draw prompt
    let prompt = shell.prompt();
    buf.fg_color = COLOR_DIM;
    buf_write_str(buf, &prompt);

    // Draw input text
    buf.fg_color = COLOR_FG;
    buf_write_str(buf, &shell.line.text);

    // Position cursor at correct location (prompt len + cursor char pos)
    let prompt_chars = prompt.chars().count();
    let cursor_chars = shell.line.cursor;
    buf.cursor_col = prompt_chars + cursor_chars;
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // Load environment from /System/env and collect aliases
    let alias_defs = libshellcommon::load_dotenv();

    // Create window
    let win_id = window::create("Shell", 80, 80, 720, 450);
    if win_id == u32::MAX {
        anyos_std::println!("shell: failed to create window");
        return;
    }

    // Menu bar
    let mut mb = window::MenuBarBuilder::new()
        .menu("Shell")
            .item(1, "Clear", 0)
            .separator()
            .item(2, "Close", 0)
        .end_menu()
        .menu("View")
            .item(10, "Increase Font Size", 0)
            .item(11, "Decrease Font Size", 0)
            .item(12, "Reset Font Size", 0)
        .end_menu();
    let data = mb.build();
    window::set_menu(win_id, data);

    // Load monospace font
    let mono_font_id = window::font_load("/System/fonts/andale-mono.ttf")
        .unwrap_or(0) as u16; // fallback to system font 0

    let mut font_size_idx = DEFAULT_FONT_SIZE_IDX;
    let mut font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);

    let (mut win_w, mut win_h) = window::get_size(win_id).unwrap_or((720, 450));
    let cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32) as usize;
    let rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32) as usize;

    let mut buf = TerminalBuffer::new(cols.max(1), rows.max(1));

    // Create shell interpreter state
    let mut shell = libshellcommon::interpreter::ShellState::new();

    // Load aliases from dotenv into shell state
    for ad in &alias_defs {
        shell.aliases.push((String::from(ad.name.as_str()), String::from(ad.value.as_str())));
    }

    // Set initial cwd
    let mut cwd_buf = [0u8; 256];
    let cwd_len = anyos_std::env::get("PWD", &mut cwd_buf);
    if cwd_len != u32::MAX && cwd_len > 0 {
        if let Ok(cwd_str) = core::str::from_utf8(&cwd_buf[..cwd_len as usize]) {
            shell.cwd = String::from(cwd_str);
        }
    }

    let mut app_builtins = ShellAppBuiltins::new();

    // Foreground process tracking
    let mut fg_proc: Option<ForegroundProcess> = None;

    // Display initial prompt
    let prompt = shell.prompt();
    buf.fg_color = COLOR_DIM;
    buf.feed(prompt.as_bytes());
    buf.fg_color = COLOR_FG;

    // Initial render
    render(win_id, &buf, &font, win_w, win_h);

    let mut dirty = false;
    let mut event = [0u32; 5];

    // Password prompt state (for su command)
    let mut password_mode: Option<String> = None; // Some(username) when prompting
    let mut password_buf = String::new();

    loop {
        // Poll foreground process stdout if one is running
        if let Some(ref fg) = fg_proc {
            let mut read_buf = [0u8; 1024];
            let mut got_output = false;
            loop {
                let n = ipc::pipe_read(fg.stdout_pipe, &mut read_buf);
                if n == 0 || n == u32::MAX {
                    break;
                }
                buf.feed(&read_buf[..n as usize]);
                got_output = true;
            }
            if got_output {
                dirty = true;
            }

            // Check if foreground process exited
            let status = process::try_waitpid(fg.tid);
            if status != process::STILL_RUNNING {
                // Drain remaining output
                loop {
                    let n = ipc::pipe_read(fg.stdout_pipe, &mut read_buf);
                    if n == 0 || n == u32::MAX { break; }
                    buf.feed(&read_buf[..n as usize]);
                }

                let was_stopped = status == process::STOPPED;
                let finished_fg = fg_proc.take().unwrap();

                if was_stopped {
                    // Ctrl+Z: add to background jobs as stopped
                    shell.add_stopped_job(
                        finished_fg.tid,
                        &finished_fg.command,
                        finished_fg.stdout_pipe,
                        finished_fg.stdin_pipe,
                        finished_fg.extra_pipes,
                    );
                    let job_id = shell.next_job_id - 1;
                    let msg = format!("\n[{}]+  Stopped  {}\n", job_id, finished_fg.command);
                    buf.feed(msg.as_bytes());
                } else {
                    finished_fg.close_pipes();
                }

                // Resume pending command chain (&&, ||, ;) if present
                if !shell.pending_chain.is_empty() {
                    shell.chain_last_success = status == 0;
                    let chain_actions = {
                        let mut out = BufOutput { buf: &mut buf };
                        shell.resume_pending_chain(&mut out, &mut app_builtins)
                    };
                    let mut chain_needs_fg = false;
                    for action in chain_actions {
                        match action {
                            libshellcommon::interpreter::ShellAction::Done => {}
                            libshellcommon::interpreter::ShellAction::RunForeground { path, args, command, redirect, input_data } => {
                                let pipe_name = format!("shell:fg:{}:{}", process::getpid(), shell.pipe_counter);
                                shell.pipe_counter += 1;
                                let stdout_pipe = ipc::pipe_create(&pipe_name);
                                let stdin_name = format!("shell:fgin:{}:{}", process::getpid(), shell.pipe_counter);
                                shell.pipe_counter += 1;
                                let stdin_pipe = ipc::pipe_create(&stdin_name);
                                if let Some(ref input) = input_data {
                                    ipc::pipe_write(stdin_pipe, input.as_bytes());
                                }
                                let tid = process::spawn_piped_full(&path, &args, stdout_pipe, stdin_pipe);
                                if tid != u32::MAX {
                                    if let Some(redir) = redirect {
                                        process::waitpid(tid);
                                        let mut out_buf2 = [0u8; 4096];
                                        let mut captured = String::new();
                                        loop {
                                            let n = ipc::pipe_read(stdout_pipe, &mut out_buf2);
                                            if n == 0 || n == u32::MAX { break; }
                                            if let Ok(s) = core::str::from_utf8(&out_buf2[..n as usize]) {
                                                captured.push_str(s);
                                            }
                                        }
                                        ipc::pipe_close(stdout_pipe);
                                        ipc::pipe_close(stdin_pipe);
                                        let mut redir = redir;
                                        anyos_std::shell::write_redirect(&mut redir, &captured);
                                    } else {
                                        fg_proc = Some(ForegroundProcess {
                                            tid, stdout_pipe, stdin_pipe,
                                            command: String::from(command.as_str()),
                                            extra_pipes: Vec::new(),
                                        });
                                        chain_needs_fg = true;
                                    }
                                } else {
                                    let msg = format!("shell: {}: command not found\n", command);
                                    buf.feed(msg.as_bytes());
                                    ipc::pipe_close(stdout_pipe);
                                    ipc::pipe_close(stdin_pipe);
                                }
                            }
                            libshellcommon::interpreter::ShellAction::RunPipeline { line, redirect } => {
                                if let Some(result) = anyos_std::shell::run_pipeline(&line, &shell.cwd, &mut shell.pipe_counter) {
                                    fg_proc = Some(ForegroundProcess {
                                        tid: result.last_tid,
                                        stdout_pipe: result.display_pipe,
                                        stdin_pipe: 0,
                                        command: String::from(line.as_str()),
                                        extra_pipes: result.extra_pipes,
                                    });
                                    chain_needs_fg = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    if chain_needs_fg {
                        dirty = true;
                        // Don't show prompt — wait for the next fg process
                    } else {
                        redraw_prompt_line(&mut buf, &shell);
                        dirty = true;
                    }
                } else {
                    // Show prompt again
                    redraw_prompt_line(&mut buf, &shell);
                    dirty = true;
                }
            }
        }

        // Poll window events
        let got = window::get_event(win_id, &mut event);
        if got == 1 {
            match event[0] {
                EVENT_WINDOW_CLOSE => {
                    if let Some(ref fg) = fg_proc {
                        process::kill(fg.tid);
                        fg.close_pipes();
                    }
                    window::destroy(win_id);
                    return;
                }
                _ if event[0] == window::EVENT_MENU_ITEM => {
                    match event[2] {
                        1 => {
                            // Clear
                            buf.clear();
                            redraw_prompt_line(&mut buf, &shell);
                            dirty = true;
                        }
                        2 => {
                            // Close
                            if let Some(ref fg) = fg_proc {
                                process::kill(fg.tid);
                                fg.close_pipes();
                            }
                            window::destroy(win_id);
                            return;
                        }
                        10 => {
                            // Increase font size
                            if font_size_idx + 1 < FONT_SIZES.len() {
                                font_size_idx += 1;
                                font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                                buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                                buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                                dirty = true;
                            }
                        }
                        11 => {
                            // Decrease font size
                            if font_size_idx > 0 {
                                font_size_idx -= 1;
                                font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                                buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                                buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                                dirty = true;
                            }
                        }
                        12 => {
                            // Reset font size
                            font_size_idx = DEFAULT_FONT_SIZE_IDX;
                            font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                            buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                            buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                            dirty = true;
                        }
                        _ => {}
                    }
                }
                EVENT_RESIZE => {
                    win_w = event[1];
                    win_h = event[2];
                    buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                    buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                    dirty = true;
                }
                EVENT_MOUSE_SCROLL => {
                    let dz = event[1] as i32;
                    if dz < 0 {
                        buf.scroll_up(3);
                    } else if dz > 0 {
                        buf.scroll_down(3);
                    }
                    dirty = true;
                }
                EVENT_KEY_DOWN => {
                    let key_code = event[1];
                    let char_val = event[2];
                    let mods = event[3];

                    // Ctrl+Plus/Minus: font size (local only, always active)
                    if (mods & MOD_CTRL) != 0 && char_val == '+' as u32 {
                        if font_size_idx + 1 < FONT_SIZES.len() {
                            font_size_idx += 1;
                            font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                            buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                            buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                            dirty = true;
                        }
                    } else if (mods & MOD_CTRL) != 0 && char_val == '-' as u32 {
                        if font_size_idx > 0 {
                            font_size_idx -= 1;
                            font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                            buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                            buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                            dirty = true;
                        }
                    } else if fg_proc.is_some() {
                        // Foreground process is running — forward input to its stdin pipe
                        let fg = fg_proc.as_ref().unwrap();
                        match key_code {
                            KEY_ENTER => { ipc::pipe_write(fg.stdin_pipe, b"\n"); }
                            KEY_BACKSPACE => { ipc::pipe_write(fg.stdin_pipe, &[0x7f]); }
                            KEY_ESCAPE => { ipc::pipe_write(fg.stdin_pipe, b"\x1b"); }
                            KEY_UP => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[A"); }
                            KEY_DOWN => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[B"); }
                            KEY_RIGHT => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[C"); }
                            KEY_LEFT => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[D"); }
                            KEY_HOME => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[H"); }
                            KEY_END => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[F"); }
                            KEY_DELETE => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[3~"); }
                            KEY_PAGE_UP => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[5~"); }
                            KEY_PAGE_DOWN => { ipc::pipe_write(fg.stdin_pipe, b"\x1b[6~"); }
                            _ => {
                                if char_val > 0 {
                                    if (mods & MOD_CTRL) != 0 && char_val < 128 {
                                        let c = char_val as u8;
                                        let ctrl_code = if c >= b'a' && c <= b'z' {
                                            c - b'a' + 1
                                        } else if c >= b'A' && c <= b'Z' {
                                            c - b'A' + 1
                                        } else {
                                            0
                                        };
                                        if ctrl_code > 0 {
                                            ipc::pipe_write(fg.stdin_pipe, &[ctrl_code]);
                                            // Ctrl+C: also kill the process
                                            if ctrl_code == 3 {
                                                process::kill(fg.tid);
                                            }
                                            // Ctrl+Z: send SIGTSTP
                                            if ctrl_code == 26 {
                                                process::send_signal(fg.tid, process::SIGTSTP);
                                            }
                                        }
                                    } else if let Some(ch) = char::from_u32(char_val) {
                                        if !ch.is_control() {
                                            let mut utf8_buf = [0u8; 4];
                                            let encoded = ch.encode_utf8(&mut utf8_buf);
                                            ipc::pipe_write(fg.stdin_pipe, encoded.as_bytes());
                                        }
                                    }
                                }
                            }
                        }
                        buf.scroll_to_bottom();
                        dirty = true;
                    } else if let Some(ref username) = password_mode.clone() {
                        // Password input mode (for su command)
                        match key_code {
                            KEY_ENTER => {
                                buf.newline();
                                {
                                    let mut out = BufOutput { buf: &mut buf };
                                    libshellcommon::interpreter::ShellState::do_su(username, &password_buf, &mut out);
                                }
                                password_buf.clear();
                                password_mode = None;
                                // Show prompt
                                buf.newline();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            KEY_BACKSPACE => {
                                if !password_buf.is_empty() {
                                    password_buf.pop();
                                }
                            }
                            _ => {
                                if char_val > 0 {
                                    if let Some(ch) = char::from_u32(char_val) {
                                        if !ch.is_control() {
                                            password_buf.push(ch);
                                        }
                                    }
                                }
                            }
                        }
                        buf.scroll_to_bottom();
                        dirty = true;
                    } else {
                        // No foreground process — handle input locally with ShellState
                        match key_code {
                            KEY_ENTER => {
                                // Display the current text on the line before executing
                                let actions = {
                                    let mut out = BufOutput { buf: &mut buf };
                                    shell.execute(&mut out, &mut app_builtins)
                                };

                                // Handle clear_screen flag from builtins
                                if app_builtins.clear_flag {
                                    buf.clear();
                                    app_builtins.clear_flag = false;
                                }

                                // Process actions
                                let mut show_prompt = true;
                                for action in actions {
                                    match action {
                                        libshellcommon::interpreter::ShellAction::Done => {}
                                        libshellcommon::interpreter::ShellAction::Exit => {
                                            window::destroy(win_id);
                                            return;
                                        }
                                        libshellcommon::interpreter::ShellAction::Reboot => {
                                            process::reboot();
                                        }
                                        libshellcommon::interpreter::ShellAction::Shutdown => {
                                            process::shutdown();
                                        }
                                        libshellcommon::interpreter::ShellAction::PromptPassword { username } => {
                                            password_mode = Some(username);
                                            show_prompt = false;
                                        }
                                        libshellcommon::interpreter::ShellAction::RunForeground { path, args, command, redirect, input_data } => {
                                            show_prompt = false;
                                            // Already-running process (from fg command) — path is empty
                                            if path.is_empty() {
                                                // fg resumed a background job: wait for it
                                                // The bg job's pipes were transferred; nothing more to set up here
                                            } else {
                                                // Spawn new foreground process with pipes
                                                let pipe_name = format!("shell:fg:{}:{}", process::getpid(), shell.pipe_counter);
                                                shell.pipe_counter += 1;
                                                let stdout_pipe = ipc::pipe_create(&pipe_name);
                                                let stdin_name = format!("shell:fgin:{}:{}", process::getpid(), shell.pipe_counter);
                                                shell.pipe_counter += 1;
                                                let stdin_pipe = ipc::pipe_create(&stdin_name);

                                                // Write input data to stdin pipe if present
                                                if let Some(ref input) = input_data {
                                                    ipc::pipe_write(stdin_pipe, input.as_bytes());
                                                }

                                                let tid = if path.starts_with("/Applications/") {
                                                    ipc::pipe_close(stdout_pipe);
                                                    ipc::pipe_close(stdin_pipe);
                                                    process::launch_app(&path, &args)
                                                } else {
                                                    process::spawn_piped_full(&path, &args, stdout_pipe, stdin_pipe)
                                                };
                                                if tid == u32::MAX {
                                                    let msg = format!("shell: {}: command not found\n", command);
                                                    buf.feed(msg.as_bytes());
                                                    if !path.starts_with("/Applications/") {
                                                        ipc::pipe_close(stdout_pipe);
                                                        ipc::pipe_close(stdin_pipe);
                                                    }
                                                    show_prompt = true;
                                                } else if path.starts_with("/Applications/") {
                                                    // GUI app launched via launch_app — fire and forget,
                                                    // pipes are already closed; return prompt immediately.
                                                    show_prompt = true;
                                                } else {
                                                    // Handle output redirect: if redirect is set,
                                                    // we need to capture output and write to file
                                                    // For now, treat as normal pipe output
                                                    if let Some(redir) = redirect {
                                                        // Wait for process, capture output, write redirect
                                                        process::waitpid(tid);
                                                        let mut out_buf = [0u8; 4096];
                                                        let mut captured = String::new();
                                                        loop {
                                                            let n = ipc::pipe_read(stdout_pipe, &mut out_buf);
                                                            if n == 0 || n == u32::MAX { break; }
                                                            if let Ok(s) = core::str::from_utf8(&out_buf[..n as usize]) {
                                                                captured.push_str(s);
                                                            }
                                                        }
                                                        ipc::pipe_close(stdout_pipe);
                                                        ipc::pipe_close(stdin_pipe);
                                                        let mut redir = redir;
                                                        anyos_std::shell::write_redirect(&mut redir, &captured);
                                                        show_prompt = true;
                                                    } else {
                                                        fg_proc = Some(ForegroundProcess {
                                                            tid,
                                                            stdout_pipe,
                                                            stdin_pipe,
                                                            command: String::from(command.as_str()),
                                                            extra_pipes: Vec::new(),
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                        libshellcommon::interpreter::ShellAction::RunBackground { path, args, command } => {
                                            let pipe_name = format!("shell:bg:{}:{}", process::getpid(), shell.pipe_counter);
                                            shell.pipe_counter += 1;
                                            let stdout_pipe = ipc::pipe_create(&pipe_name);
                                            let stdin_name = format!("shell:bgin:{}:{}", process::getpid(), shell.pipe_counter);
                                            shell.pipe_counter += 1;
                                            let stdin_pipe = ipc::pipe_create(&stdin_name);

                                            let tid = if path.starts_with("/Applications/") {
                                                ipc::pipe_close(stdout_pipe);
                                                ipc::pipe_close(stdin_pipe);
                                                process::launch_app(&path, &args)
                                            } else {
                                                process::spawn_piped_full(&path, &args, stdout_pipe, stdin_pipe)
                                            };
                                            if tid == u32::MAX {
                                                let msg = format!("shell: {}: command not found\n", command);
                                                buf.feed(msg.as_bytes());
                                                if !path.starts_with("/Applications/") {
                                                    ipc::pipe_close(stdout_pipe);
                                                    ipc::pipe_close(stdin_pipe);
                                                }
                                            } else {
                                                shell.add_bg_job(tid, &command);
                                                let job_id = shell.next_job_id - 1;
                                                let msg = format!("[{}] {}\n", job_id, tid);
                                                buf.feed(msg.as_bytes());
                                            }
                                        }
                                        libshellcommon::interpreter::ShellAction::RunPipeline { line, redirect } => {
                                            show_prompt = false;
                                            if let Some(result) = anyos_std::shell::run_pipeline(&line, &shell.cwd, &mut shell.pipe_counter) {
                                                let tid = result.last_tid;
                                                if let Some(redir) = redirect {
                                                    // Capture pipeline output to redirect
                                                    process::waitpid(tid);
                                                    let mut out_buf = [0u8; 4096];
                                                    let mut captured = String::new();
                                                    loop {
                                                        let n = ipc::pipe_read(result.display_pipe, &mut out_buf);
                                                        if n == 0 || n == u32::MAX { break; }
                                                        if let Ok(s) = core::str::from_utf8(&out_buf[..n as usize]) {
                                                            captured.push_str(s);
                                                        }
                                                    }
                                                    ipc::pipe_close(result.display_pipe);
                                                    for &p in &result.extra_pipes {
                                                        ipc::pipe_close(p);
                                                    }
                                                    let mut redir = redir;
                                                    anyos_std::shell::write_redirect(&mut redir, &captured);
                                                    show_prompt = true;
                                                } else {
                                                    fg_proc = Some(ForegroundProcess {
                                                        tid,
                                                        stdout_pipe: result.display_pipe,
                                                        stdin_pipe: 0,
                                                        command: String::from(line.as_str()),
                                                        extra_pipes: result.extra_pipes,
                                                    });
                                                }
                                            } else {
                                                buf.feed(b"shell: pipeline failed\n");
                                                show_prompt = true;
                                            }
                                        }
                                    }
                                }

                                if show_prompt && fg_proc.is_none() && password_mode.is_none() {
                                    redraw_prompt_line(&mut buf, &shell);
                                }
                                buf.scroll_to_bottom();
                                dirty = true;
                            }
                            KEY_BACKSPACE => {
                                shell.backspace();
                                redraw_prompt_line(&mut buf, &shell);
                                buf.scroll_to_bottom();
                                dirty = true;
                            }
                            KEY_TAB => {
                                let before = shell.line.before_cursor();
                                let before_owned = String::from(before);
                                match libshellcommon::complete(&before_owned, &shell.cwd, SHELL_BUILTINS) {
                                    libshellcommon::CompletionResult::None => {
                                        // No matches — do nothing
                                    }
                                    libshellcommon::CompletionResult::Single(remaining, is_dir) => {
                                        if !remaining.is_empty() {
                                            shell.line.insert_str(&remaining);
                                        }
                                        if !is_dir {
                                            shell.line.insert_char(' ');
                                        }
                                        redraw_prompt_line(&mut buf, &shell);
                                    }
                                    libshellcommon::CompletionResult::Multiple(to_insert, completions) => {
                                        if !to_insert.is_empty() {
                                            shell.line.insert_str(&to_insert);
                                        }
                                        // Show all matches
                                        buf.newline();
                                        for m in &completions {
                                            buf.feed(m.as_bytes());
                                            buf.feed(b"  ");
                                        }
                                        if completions.len() > 200 {
                                            let msg = format!("... ({} total)", completions.len());
                                            buf.feed(msg.as_bytes());
                                        }
                                        buf.newline();
                                        redraw_prompt_line(&mut buf, &shell);
                                    }
                                }
                                buf.scroll_to_bottom();
                                dirty = true;
                            }
                            KEY_UP => {
                                shell.history_up();
                                redraw_prompt_line(&mut buf, &shell);
                                buf.scroll_to_bottom();
                                dirty = true;
                            }
                            KEY_DOWN => {
                                shell.history_down();
                                redraw_prompt_line(&mut buf, &shell);
                                buf.scroll_to_bottom();
                                dirty = true;
                            }
                            KEY_LEFT => {
                                shell.cursor_left();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            KEY_RIGHT => {
                                shell.cursor_right();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            KEY_HOME => {
                                shell.cursor_home();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            KEY_END => {
                                shell.cursor_end();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            KEY_DELETE => {
                                shell.delete_at_cursor();
                                redraw_prompt_line(&mut buf, &shell);
                                dirty = true;
                            }
                            _ => {
                                if char_val > 0 {
                                    if (mods & MOD_CTRL) != 0 && char_val < 128 {
                                        let c = char_val as u8;
                                        let ctrl_code = if c >= b'a' && c <= b'z' {
                                            c - b'a' + 1
                                        } else if c >= b'A' && c <= b'Z' {
                                            c - b'A' + 1
                                        } else {
                                            0
                                        };
                                        if ctrl_code == 3 {
                                            // Ctrl+C: cancel current input
                                            buf.feed(b"^C");
                                            buf.newline();
                                            shell.line.clear();
                                            shell.history.reset_index();
                                            redraw_prompt_line(&mut buf, &shell);
                                            dirty = true;
                                        } else if ctrl_code == 12 {
                                            // Ctrl+L: clear screen
                                            buf.clear();
                                            redraw_prompt_line(&mut buf, &shell);
                                            dirty = true;
                                        } else if ctrl_code == 21 {
                                            // Ctrl+U: clear line
                                            shell.line.clear();
                                            redraw_prompt_line(&mut buf, &shell);
                                            dirty = true;
                                        }
                                    } else if let Some(ch) = char::from_u32(char_val) {
                                        if !ch.is_control() {
                                            shell.insert_char(ch);
                                            redraw_prompt_line(&mut buf, &shell);
                                            buf.scroll_to_bottom();
                                            dirty = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        } else {
            // No event
            if dirty {
                render(win_id, &buf, &font, win_w, win_h);
                dirty = false;
            }
            process::sleep(8); // ~125 Hz poll
        }
    }
}
