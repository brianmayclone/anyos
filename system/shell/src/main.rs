#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;
use anyos_std::process;
use anyos_std::ipc;
use anyos_std::fs;
use anyos_std::ui::window;
use alloc::string::ToString;

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

// ─── Shell Process ───────────────────────────────────────────────────────────

struct ShellProcess {
    tid: u32,
    stdout_pipe: u32,
    stdin_pipe: u32,
}

impl ShellProcess {
    fn spawn() -> Option<Self> {
        let stdout_name = format!("shell:stdout:{}", process::getpid());
        let stdin_name = format!("shell:stdin:{}", process::getpid());
        let stdout_pipe = ipc::pipe_create(&stdout_name);
        let stdin_pipe = ipc::pipe_create(&stdin_name);

        let tid = process::spawn_piped_full("/System/bin/sh", "sh -i", stdout_pipe, stdin_pipe);
        if tid == u32::MAX {
            ipc::pipe_close(stdout_pipe);
            ipc::pipe_close(stdin_pipe);
            None
        } else {
            Some(ShellProcess { tid, stdout_pipe, stdin_pipe })
        }
    }

    fn write(&self, data: &[u8]) {
        ipc::pipe_write(self.stdin_pipe, data);
    }

    fn read(&self, buf: &mut [u8]) -> u32 {
        ipc::pipe_read(self.stdout_pipe, buf)
    }

    fn is_alive(&self) -> bool {
        process::try_waitpid(self.tid) == process::STILL_RUNNING
    }

    fn kill(&self) {
        process::kill(self.tid);
    }

    fn close_pipes(&self) {
        ipc::pipe_close(self.stdout_pipe);
        ipc::pipe_close(self.stdin_pipe);
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

// ─── Environment Setup ───────────────────────────────────────────────────────

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

fn source_env_file(path: &str, depth: u32) {
    if depth > 4 {
        return;
    }
    let mut data = [0u8; 2048];
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
        if line.starts_with("source ") {
            let target = line[7..].trim();
            if !target.is_empty() {
                source_env_file(target, depth + 1);
            }
            continue;
        }
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

fn load_dotenv() {
    source_env_file("/System/env", 0);
    let uid = process::getuid();
    let mut name_buf = [0u8; 32];
    let nlen = process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..nlen as usize]) {
            if username != "root" {
                let user_env = format!("/Users/{}/env", username);
                source_env_file(&user_env, 0);
                let home = format!("/Users/{}", username);
                anyos_std::env::set("HOME", &home);
                anyos_std::env::set("USER", username);
            }
        }
    }
}

// ─── Tab Completion ──────────────────────────────────────────────────────

const BUILTIN_COMMANDS: &[&str] = &[
    ".", "alias", "bg", "break", "cd", "command", "continue",
    "echo", "eval", "exec", "exit", "export", "false",
    "fg", "getopts", "hash", "jobs", "kill", "local",
    "printf", "pwd", "read", "readonly", "return", "set",
    "shift", "source", "test", "times", "trap", "true", "type",
    "ulimit", "umask", "unalias", "unset", "wait",
];

fn find_file_completions(partial: &str) -> Vec<String> {
    let (dir_path, prefix) = match partial.rfind('/') {
        Some(pos) => {
            let d = if pos == 0 { "/" } else { &partial[..pos] };
            (d, &partial[pos + 1..])
        }
        None => (".", partial),
    };

    let mut entry_buf = [0u8; 64 * 64];
    let count = fs::readdir(dir_path, &mut entry_buf);
    if count == u32::MAX {
        return Vec::new();
    }

    let mut results = Vec::new();
    for i in 0..count as usize {
        let base = i * 64;
        let entry_type = entry_buf[base];
        let name_len = (entry_buf[base + 1] as usize).min(56);
        if let Ok(name) = core::str::from_utf8(&entry_buf[base + 8..base + 8 + name_len]) {
            if name == "." || name == ".." { continue; }
            if name.starts_with(prefix) {
                let mut s = String::from(name);
                if entry_type == 1 { s.push('/'); }
                results.push(s);
            }
        }
    }
    results
}

fn find_command_completions(partial: &str) -> Vec<String> {
    let mut results = Vec::new();

    // Builtins
    for &cmd in BUILTIN_COMMANDS {
        if cmd.starts_with(partial) {
            results.push(String::from(cmd));
        }
    }

    // Scan PATH directories
    let mut path_buf = [0u8; 512];
    let path_len = anyos_std::env::get("PATH", &mut path_buf);
    if path_len != u32::MAX && (path_len as usize) <= path_buf.len() {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..path_len as usize]) {
            for dir in path_str.split(':') {
                if dir.is_empty() { continue; }
                let mut entry_buf = [0u8; 64 * 64];
                let count = fs::readdir(dir, &mut entry_buf);
                if count == u32::MAX { continue; }
                for i in 0..count as usize {
                    let base = i * 64;
                    let entry_type = entry_buf[base];
                    let name_len = (entry_buf[base + 1] as usize).min(56);
                    if entry_type != 0 { continue; } // Only regular files
                    if let Ok(name) = core::str::from_utf8(&entry_buf[base + 8..base + 8 + name_len]) {
                        if name.starts_with(partial) && !results.iter().any(|r: &String| r.as_str() == name) {
                            results.push(String::from(name));
                        }
                    }
                }
            }
        }
    }

    results
}

fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() { return String::new(); }
    let first = strings[0].as_bytes();
    let mut len = first.len();
    for s in &strings[1..] {
        let b = s.as_bytes();
        len = len.min(b.len());
        for i in 0..len {
            if first[i] != b[i] {
                len = i;
                break;
            }
        }
    }
    if let Ok(s) = core::str::from_utf8(&first[..len]) {
        String::from(s)
    } else {
        String::new()
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // Load environment from /System/env (same as Terminal)
    load_dotenv();

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

    // Spawn shell process
    let shell_proc = match ShellProcess::spawn() {
        Some(sp) => sp,
        None => {
            buf.fg_color = 0xFFFF5555;
            buf.feed(b"Error: Could not start /System/bin/sh\n");
            buf.feed(b"Make sure dash is installed.\n");
            render(win_id, &buf, &font, win_w, win_h);
            // Wait for close
            let mut event = [0u32; 5];
            loop {
                let got = window::get_event(win_id, &mut event);
                if got == 1 && event[0] == EVENT_WINDOW_CLOSE {
                    window::destroy(win_id);
                    return;
                }
                process::sleep(50);
            }
        }
    };

    // Initial render
    render(win_id, &buf, &font, win_w, win_h);

    let mut dirty = false;
    let mut event = [0u32; 5];
    let mut shell_exited = false;

    // Tab completion state
    let mut input_line: Vec<u8> = Vec::new();
    let mut tab_count: u32 = 0;

    loop {
        // Poll shell stdout for output
        if !shell_exited {
            let mut read_buf = [0u8; 1024];
            let mut got_output = false;
            loop {
                let n = shell_proc.read(&mut read_buf);
                if n == 0 || n == u32::MAX {
                    break;
                }
                buf.feed(&read_buf[..n as usize]);
                got_output = true;
            }
            if got_output {
                dirty = true;
            }

            // Check if shell exited
            if !shell_proc.is_alive() {
                // Drain remaining output
                loop {
                    let n = shell_proc.read(&mut read_buf);
                    if n == 0 || n == u32::MAX { break; }
                    buf.feed(&read_buf[..n as usize]);
                }
                shell_proc.close_pipes();
                shell_exited = true;

                buf.fg_color = COLOR_DIM;
                buf.feed(b"\n[Shell exited]\n");
                dirty = true;
            }
        }

        // Poll window events
        let got = window::get_event(win_id, &mut event);
        if got == 1 {
            match event[0] {
                EVENT_WINDOW_CLOSE => {
                    if !shell_exited {
                        shell_proc.kill();
                        shell_proc.close_pipes();
                    }
                    window::destroy(win_id);
                    return;
                }
                _ if event[0] == window::EVENT_MENU_ITEM => {
                    match event[2] {
                        1 => {
                            // Clear
                            buf.clear();
                            dirty = true;
                        }
                        2 => {
                            // Close
                            if !shell_exited {
                                shell_proc.kill();
                                shell_proc.close_pipes();
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

                    // Ctrl+Plus/Minus: font size (local only)
                    if (mods & MOD_CTRL) != 0 && char_val == '+' as u32 {
                        // Ctrl+Plus: increase font
                        if font_size_idx + 1 < FONT_SIZES.len() {
                            font_size_idx += 1;
                            font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                            buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                            buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                            dirty = true;
                        }
                    } else if (mods & MOD_CTRL) != 0 && char_val == '-' as u32 {
                        // Ctrl+Minus: decrease font
                        if font_size_idx > 0 {
                            font_size_idx -= 1;
                            font = FontMetrics::measure(mono_font_id, FONT_SIZES[font_size_idx]);
                            buf.cols = (win_w.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_w as u32).max(1) as usize;
                            buf.visible_rows = (win_h.saturating_sub(TEXT_PAD as u32 * 2) / font.cell_h as u32).max(1) as usize;
                            dirty = true;
                        }
                    } else if !shell_exited {
                        // Forward keystrokes to child process
                        match key_code {
                            KEY_ENTER => {
                                shell_proc.write(b"\n");
                                buf.feed(b"\n");
                                input_line.clear();
                                tab_count = 0;
                            }
                            KEY_BACKSPACE => {
                                shell_proc.write(&[0x7f]);
                                // Erase last character on screen
                                if buf.cursor_col > 0 {
                                    buf.cursor_col -= 1;
                                    buf.ensure_line(buf.cursor_row);
                                    if buf.cursor_col < buf.lines[buf.cursor_row].len() {
                                        buf.lines[buf.cursor_row].truncate(buf.cursor_col);
                                    }
                                }
                                input_line.pop();
                                tab_count = 0;
                            }
                            KEY_TAB => {
                                // Tab completion
                                let word_start = input_line.iter().rposition(|&b| b == b' ')
                                    .map(|p| p + 1).unwrap_or(0);
                                let partial = &input_line[word_start..];

                                if partial.is_empty() {
                                    // No partial word, just insert tab
                                    shell_proc.write(b"\t");
                                    buf.feed(b"\t");
                                } else {
                                    let is_first_word = word_start == 0;
                                    let partial_str = core::str::from_utf8(partial).unwrap_or("");

                                    let matches = if is_first_word && !partial_str.contains('/') {
                                        find_command_completions(partial_str)
                                    } else {
                                        find_file_completions(partial_str)
                                    };

                                    tab_count += 1;

                                    if matches.len() == 1 {
                                        // Single match: complete it
                                        let m = &matches[0];
                                        let file_prefix_len = match partial_str.rfind('/') {
                                            Some(pos) => partial_str.len() - pos - 1,
                                            None => partial_str.len(),
                                        };
                                        let insert = &m[file_prefix_len..];
                                        if !insert.is_empty() {
                                            shell_proc.write(insert.as_bytes());
                                            buf.feed(insert.as_bytes());
                                            input_line.extend_from_slice(insert.as_bytes());
                                        }
                                        // Add trailing space if not a directory
                                        if !m.ends_with('/') {
                                            shell_proc.write(b" ");
                                            buf.feed(b" ");
                                            input_line.push(b' ');
                                        }
                                        tab_count = 0;
                                    } else if matches.len() > 1 {
                                        // Multiple matches: insert common prefix
                                        let lcp = longest_common_prefix(&matches);
                                        let file_prefix_len = match partial_str.rfind('/') {
                                            Some(pos) => partial_str.len() - pos - 1,
                                            None => partial_str.len(),
                                        };
                                        if lcp.len() > file_prefix_len {
                                            let insert = &lcp[file_prefix_len..];
                                            shell_proc.write(insert.as_bytes());
                                            buf.feed(insert.as_bytes());
                                            input_line.extend_from_slice(insert.as_bytes());
                                        }
                                        // On double-tab, show all matches
                                        if tab_count >= 2 {
                                            buf.feed(b"\r\n");
                                            let show_count = matches.len().min(30);
                                            for i in 0..show_count {
                                                buf.feed(matches[i].as_bytes());
                                                buf.feed(b"  ");
                                            }
                                            if matches.len() > 30 {
                                                let msg = format!("... ({} total)", matches.len());
                                                buf.feed(msg.as_bytes());
                                            }
                                            buf.feed(b"\r\n$ ");
                                            buf.feed(&input_line);
                                            tab_count = 0;
                                        }
                                    }
                                    // No matches: do nothing
                                }
                            }
                            KEY_ESCAPE => shell_proc.write(b"\x1b"),
                            KEY_UP => shell_proc.write(b"\x1b[A"),
                            KEY_DOWN => shell_proc.write(b"\x1b[B"),
                            KEY_RIGHT => shell_proc.write(b"\x1b[C"),
                            KEY_LEFT => shell_proc.write(b"\x1b[D"),
                            KEY_HOME => shell_proc.write(b"\x1b[H"),
                            KEY_END => shell_proc.write(b"\x1b[F"),
                            KEY_DELETE => shell_proc.write(b"\x1b[3~"),
                            KEY_PAGE_UP => shell_proc.write(b"\x1b[5~"),
                            KEY_PAGE_DOWN => shell_proc.write(b"\x1b[6~"),
                            _ => {
                                if char_val > 0 && char_val < 128 {
                                    let c = char_val as u8;
                                    if (mods & MOD_CTRL) != 0 {
                                        // Forward Ctrl combinations as control codes
                                        // Ctrl+A=1, Ctrl+B=2, ..., Ctrl+Z=26
                                        let ctrl_code = if c >= b'a' && c <= b'z' {
                                            c - b'a' + 1
                                        } else if c >= b'A' && c <= b'Z' {
                                            c - b'A' + 1
                                        } else {
                                            0
                                        };
                                        if ctrl_code > 0 {
                                            shell_proc.write(&[ctrl_code]);
                                            // Ctrl+C or Ctrl+U: clear input line
                                            if ctrl_code == 3 || ctrl_code == 21 {
                                                input_line.clear();
                                            }
                                        }
                                    } else if c >= b' ' {
                                        shell_proc.write(&[c]);
                                        // Local echo
                                        buf.feed(&[c]);
                                        input_line.push(c);
                                    }
                                }
                                tab_count = 0;
                            }
                        }
                        // Scroll to bottom on any key input
                        buf.scroll_to_bottom();
                        dirty = true;
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
