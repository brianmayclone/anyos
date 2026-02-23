#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::fs;
use anyos_std::format;
use alloc::string::ToString;

anyos_std::entry!(main);

// ─── Key codes ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Key {
    None,
    Char(u8),
    Ctrl(u8),
    Enter,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Escape,
}

// ─── Input reader ────────────────────────────────────────────────────────────

struct InputReader {
    buf: [u8; 32],
    len: usize,
    pos: usize,
}

impl InputReader {
    fn new() -> Self {
        InputReader { buf: [0u8; 32], len: 0, pos: 0 }
    }

    fn read_key(&mut self) -> Key {
        if self.pos >= self.len {
            let n = fs::read(0, &mut self.buf);
            if n == 0 || n == u32::MAX {
                return Key::None;
            }
            self.len = n as usize;
            self.pos = 0;
        }

        let b = self.buf[self.pos];
        self.pos += 1;

        match b {
            0x1b => {
                // Escape sequence
                if self.pos < self.len && self.buf[self.pos] == b'[' {
                    self.pos += 1;
                    return self.parse_csi();
                }
                // Try to read more
                let mut esc_buf = [0u8; 8];
                let n = fs::read(0, &mut esc_buf);
                if n > 0 && n != u32::MAX && esc_buf[0] == b'[' {
                    let remaining = n as usize - 1;
                    if remaining > 0 {
                        for i in 0..remaining {
                            self.buf[i] = esc_buf[1 + i];
                        }
                        self.len = remaining;
                        self.pos = 0;
                    } else {
                        self.len = 0;
                        self.pos = 0;
                    }
                    return self.parse_csi();
                }
                Key::Escape
            }
            0x0a | 0x0d => Key::Enter,
            0x7f | 0x08 => Key::Backspace,
            0x09 => Key::Char(b'\t'),
            1..=26 => Key::Ctrl(b + b'a' - 1),
            0x20..=0x7e => Key::Char(b),
            _ => Key::None,
        }
    }

    fn parse_csi(&mut self) -> Key {
        let b = self.read_csi_byte();
        match b {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            b'H' => Key::Home,
            b'F' => Key::End,
            b'3' => { self.consume_tilde(); Key::Delete }
            b'5' => { self.consume_tilde(); Key::PageUp }
            b'6' => { self.consume_tilde(); Key::PageDown }
            _ => {
                // Consume until we see a letter >= 0x40
                while self.pos < self.len {
                    let c = self.buf[self.pos];
                    self.pos += 1;
                    if c >= 0x40 { break; }
                }
                Key::None
            }
        }
    }

    fn read_csi_byte(&mut self) -> u8 {
        if self.pos < self.len {
            let c = self.buf[self.pos];
            self.pos += 1;
            c
        } else {
            let mut tmp = [0u8; 4];
            let n = fs::read(0, &mut tmp);
            if n > 0 && n != u32::MAX {
                let c = tmp[0];
                let remaining = n as usize - 1;
                if remaining > 0 {
                    for i in 0..remaining.min(self.buf.len()) {
                        self.buf[i] = tmp[1 + i];
                    }
                    self.len = remaining;
                    self.pos = 0;
                }
                c
            } else {
                0
            }
        }
    }

    fn consume_tilde(&mut self) {
        if self.pos < self.len && self.buf[self.pos] == b'~' {
            self.pos += 1;
        } else {
            let mut tmp = [0u8; 1];
            fs::read(0, &mut tmp);
        }
    }
}

// ─── Editor state ────────────────────────────────────────────────────────────

struct Editor {
    lines: Vec<String>,
    cx: usize,          // cursor column in the line
    cy: usize,          // cursor line (0-indexed in file)
    scroll_row: usize,
    scroll_col: usize,
    filename: String,
    modified: bool,
    running: bool,
    message: String,
    message_is_prompt: bool,
    needs_redraw: bool,
    screen_rows: usize,
    screen_cols: usize,
    // Search
    search_query: String,
    // Cut/paste buffer
    cut_buffer: Vec<String>,
    // Prompt state
    prompt_callback: PromptAction,
    prompt_input: String,
    prompt_cursor: usize,
}

#[derive(PartialEq, Clone, Copy)]
enum PromptAction {
    None,
    SaveAs,
    Search,
    GotoLine,
    QuitConfirm,
}

const GUTTER_WIDTH: usize = 0; // nano doesn't show line numbers by default
const STATUS_LINES: usize = 2; // status bar + help line

impl Editor {
    fn new() -> Self {
        let (rows, cols) = get_terminal_size();
        Editor {
            lines: alloc::vec![String::new()],
            cx: 0,
            cy: 0,
            scroll_row: 0,
            scroll_col: 0,
            filename: String::new(),
            modified: false,
            running: true,
            message: String::new(),
            message_is_prompt: false,
            needs_redraw: true,
            screen_rows: rows.saturating_sub(STATUS_LINES),
            screen_cols: cols,
            search_query: String::new(),
            cut_buffer: Vec::new(),
            prompt_callback: PromptAction::None,
            prompt_input: String::new(),
            prompt_cursor: 0,
        }
    }

    fn edit_rows(&self) -> usize {
        self.screen_rows
    }

    fn load_file(&mut self, path: &str) {
        self.filename = String::from(path);
        let fd = fs::open(path, 0);
        if fd == u32::MAX {
            // New file
            self.set_message(&format!("[ New File: {} ]", path));
            return;
        }
        let mut data = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            data.extend_from_slice(&buf[..n as usize]);
        }
        fs::close(fd);

        self.lines.clear();
        if let Ok(text) = core::str::from_utf8(&data) {
            for line in text.split('\n') {
                // Strip \r
                let line = if line.ends_with('\r') { &line[..line.len()-1] } else { line };
                self.lines.push(String::from(line));
            }
        }
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        // Remove trailing empty line if file ended with \n
        if self.lines.len() > 1 && self.lines.last().map_or(false, |l| l.is_empty()) {
            self.lines.pop();
        }
        self.modified = false;
        self.set_message(&format!("Read {} lines", self.lines.len()));
    }

    fn save_file(&mut self) -> bool {
        if self.filename.is_empty() {
            self.start_prompt("File Name to Write: ", PromptAction::SaveAs);
            return false;
        }
        let mut content = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 { content.push('\n'); }
            content.push_str(line);
        }
        content.push('\n');
        if fs::write_bytes(&self.filename, content.as_bytes()).is_err() {
            self.set_message(&format!("Error writing {}", self.filename));
            return false;
        }
        self.modified = false;
        self.set_message(&format!("Wrote {} lines to {}", self.lines.len(), self.filename));
        true
    }

    fn set_message(&mut self, msg: &str) {
        self.message = String::from(msg);
        self.message_is_prompt = false;
        self.needs_redraw = true;
    }

    fn start_prompt(&mut self, msg: &str, action: PromptAction) {
        self.message = String::from(msg);
        self.message_is_prompt = true;
        self.prompt_callback = action;
        self.prompt_input.clear();
        self.prompt_cursor = 0;
        self.needs_redraw = true;
    }

    fn finish_prompt(&mut self) {
        let action = self.prompt_callback;
        let input = self.prompt_input.clone();
        self.prompt_callback = PromptAction::None;
        self.message_is_prompt = false;
        self.message.clear();

        match action {
            PromptAction::SaveAs => {
                if !input.is_empty() {
                    self.filename = input;
                    self.save_file();
                } else {
                    self.set_message("Cancelled");
                }
            }
            PromptAction::Search => {
                if !input.is_empty() {
                    self.search_query = input;
                }
                self.search_forward();
            }
            PromptAction::GotoLine => {
                if let Some(num) = parse_int(&input) {
                    let line = (num as usize).saturating_sub(1).min(self.lines.len().saturating_sub(1));
                    self.cy = line;
                    self.cx = 0;
                    self.ensure_cursor_visible();
                    self.set_message(&format!("Line {}", line + 1));
                }
            }
            PromptAction::QuitConfirm => {
                let lower = input.to_lowercase();
                if lower == "y" || lower == "yes" {
                    self.save_file();
                    self.running = false;
                } else if lower == "n" || lower == "no" {
                    self.running = false;
                } else {
                    self.set_message("Cancelled");
                }
            }
            PromptAction::None => {}
        }
        self.needs_redraw = true;
    }

    fn cancel_prompt(&mut self) {
        self.prompt_callback = PromptAction::None;
        self.message_is_prompt = false;
        self.message.clear();
        self.needs_redraw = true;
    }

    // ─── Key handling ────────────────────────────────────────────────────────

    fn handle_key(&mut self, key: Key) {
        // If in prompt mode, handle prompt input
        if self.message_is_prompt {
            match key {
                Key::Enter => self.finish_prompt(),
                Key::Ctrl(b'c') | Key::Escape => self.cancel_prompt(),
                Key::Backspace => {
                    if self.prompt_cursor > 0 {
                        self.prompt_cursor -= 1;
                        self.prompt_input.remove(self.prompt_cursor);
                        self.needs_redraw = true;
                    }
                }
                Key::Char(c) => {
                    self.prompt_input.insert(self.prompt_cursor, c as char);
                    self.prompt_cursor += 1;
                    self.needs_redraw = true;
                }
                _ => {}
            }
            return;
        }

        match key {
            // ── Ctrl commands (nano-style) ──
            Key::Ctrl(b'x') => self.cmd_exit(),
            Key::Ctrl(b'o') => { self.save_file(); }
            Key::Ctrl(b's') => { self.save_file(); }
            Key::Ctrl(b'w') => self.start_prompt("Search: ", PromptAction::Search),
            Key::Ctrl(b'k') => self.cmd_cut_line(),
            Key::Ctrl(b'u') => self.cmd_paste(),
            Key::Ctrl(b'g') => self.cmd_help(),
            Key::Ctrl(b'_') | Key::Ctrl(b't') => self.start_prompt("Enter line number: ", PromptAction::GotoLine),
            Key::Ctrl(b'c') => {
                // Show current position
                let msg = format!("line {}/{}, col {}/{}", self.cy + 1, self.lines.len(), self.cx + 1,
                    self.lines.get(self.cy).map_or(0, |l| l.len()));
                self.set_message(&msg);
            }
            Key::Ctrl(b'a') => self.cx = 0,
            Key::Ctrl(b'e') => {
                self.cx = self.lines.get(self.cy).map_or(0, |l| l.len());
            }

            // ── Navigation ──
            Key::Up => self.move_up(),
            Key::Down => self.move_down(),
            Key::Left => self.move_left(),
            Key::Right => self.move_right(),
            Key::Home => self.cx = 0,
            Key::End => { self.cx = self.lines.get(self.cy).map_or(0, |l| l.len()); }
            Key::PageUp => {
                for _ in 0..self.edit_rows() {
                    self.move_up();
                }
            }
            Key::PageDown => {
                for _ in 0..self.edit_rows() {
                    self.move_down();
                }
            }

            // ── Editing ──
            Key::Enter => self.insert_newline(),
            Key::Backspace => self.delete_back(),
            Key::Delete => self.delete_forward(),
            Key::Char(b'\t') => {
                // Insert spaces for tab
                for _ in 0..4 {
                    self.insert_char(' ');
                }
            }
            Key::Char(c) => self.insert_char(c as char),

            _ => {}
        }

        self.ensure_cursor_visible();
        self.needs_redraw = true;
    }

    fn move_up(&mut self) {
        if self.cy > 0 {
            self.cy -= 1;
            self.clamp_cx();
        }
    }

    fn move_down(&mut self) {
        if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.clamp_cx();
        }
    }

    fn move_left(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
        } else if self.cy > 0 {
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
        }
    }

    fn move_right(&mut self) {
        let line_len = self.lines.get(self.cy).map_or(0, |l| l.len());
        if self.cx < line_len {
            self.cx += 1;
        } else if self.cy + 1 < self.lines.len() {
            self.cy += 1;
            self.cx = 0;
        }
    }

    fn clamp_cx(&mut self) {
        let line_len = self.lines.get(self.cy).map_or(0, |l| l.len());
        if self.cx > line_len {
            self.cx = line_len;
        }
    }

    fn insert_char(&mut self, c: char) {
        if self.cy >= self.lines.len() {
            self.lines.push(String::new());
        }
        let line = &mut self.lines[self.cy];
        if self.cx >= line.len() {
            line.push(c);
        } else {
            line.insert(self.cx, c);
        }
        self.cx += 1;
        self.modified = true;
    }

    fn insert_newline(&mut self) {
        if self.cy >= self.lines.len() {
            self.lines.push(String::new());
            self.cy += 1;
            self.cx = 0;
        } else {
            let line = &self.lines[self.cy];
            let rest = String::from(&line[self.cx..]);
            self.lines[self.cy] = String::from(&self.lines[self.cy][..self.cx]);
            self.cy += 1;
            self.lines.insert(self.cy, rest);
            self.cx = 0;
        }
        self.modified = true;
    }

    fn delete_back(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
            self.lines[self.cy].remove(self.cx);
            self.modified = true;
        } else if self.cy > 0 {
            // Merge with previous line
            let current_line = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
            self.lines[self.cy].push_str(&current_line);
            self.modified = true;
        }
    }

    fn delete_forward(&mut self) {
        let line_len = self.lines.get(self.cy).map_or(0, |l| l.len());
        if self.cx < line_len {
            self.lines[self.cy].remove(self.cx);
            self.modified = true;
        } else if self.cy + 1 < self.lines.len() {
            // Merge with next line
            let next_line = self.lines.remove(self.cy + 1);
            self.lines[self.cy].push_str(&next_line);
            self.modified = true;
        }
    }

    fn cmd_cut_line(&mut self) {
        if self.cy < self.lines.len() {
            let line = self.lines.remove(self.cy);
            self.cut_buffer.push(line);
            if self.lines.is_empty() {
                self.lines.push(String::new());
            }
            if self.cy >= self.lines.len() {
                self.cy = self.lines.len() - 1;
            }
            self.clamp_cx();
            self.modified = true;
            self.set_message("Cut 1 line");
        }
    }

    fn cmd_paste(&mut self) {
        if self.cut_buffer.is_empty() {
            self.set_message("Cut buffer is empty");
            return;
        }
        let buf = self.cut_buffer.clone();
        for (i, line) in buf.iter().enumerate() {
            self.lines.insert(self.cy + 1 + i, line.clone());
        }
        self.cy += buf.len();
        self.clamp_cx();
        self.modified = true;
        self.set_message(&format!("Pasted {} line(s)", buf.len()));
    }

    fn cmd_exit(&mut self) {
        if self.modified {
            self.start_prompt("Save modified buffer? (Y/N) ", PromptAction::QuitConfirm);
        } else {
            self.running = false;
        }
    }

    fn cmd_help(&mut self) {
        self.set_message("^O Save  ^X Exit  ^W Search  ^K Cut  ^U Paste  ^G Help  ^_ GoTo");
    }

    fn search_forward(&mut self) {
        if self.search_query.is_empty() {
            self.set_message("No search query");
            return;
        }
        let start_line = self.cy;
        let start_col = self.cx + 1; // start after current position
        // Search from current position to end, then wrap
        for offset in 0..self.lines.len() {
            let line_idx = (start_line + offset) % self.lines.len();
            let line = &self.lines[line_idx];
            let search_from = if offset == 0 { start_col } else { 0 };
            if search_from < line.len() {
                if let Some(pos) = line[search_from..].find(&*self.search_query) {
                    self.cy = line_idx;
                    self.cx = search_from + pos;
                    self.ensure_cursor_visible();
                    self.set_message(&format!("Found at line {}", line_idx + 1));
                    return;
                }
            }
        }
        self.set_message("Not found");
    }

    fn ensure_cursor_visible(&mut self) {
        if self.cy < self.scroll_row {
            self.scroll_row = self.cy;
        }
        if self.cy >= self.scroll_row + self.edit_rows() {
            self.scroll_row = self.cy - self.edit_rows() + 1;
        }
    }

    // ─── Rendering ───────────────────────────────────────────────────────────

    fn render(&mut self) {
        if !self.needs_redraw { return; }
        self.needs_redraw = false;

        // Hide cursor during render
        anyos_std::print!("\x1B[?25l");

        // Render text lines
        for screen_row in 0..self.edit_rows() {
            anyos_std::print!("\x1B[{};1H", screen_row + 1);
            let file_row = self.scroll_row + screen_row;
            if file_row < self.lines.len() {
                let line = &self.lines[file_row];
                let display_len = line.len().min(self.screen_cols);
                // Render visible portion
                let start = self.scroll_col.min(line.len());
                let end = (start + self.screen_cols).min(line.len());
                if start < line.len() {
                    anyos_std::print!("{}", &line[start..end]);
                }
            }
            // Clear rest of line
            anyos_std::print!("\x1B[K");
        }

        // Status bar (inverted colors)
        self.render_status_bar();

        // Help/message line
        self.render_help_line();

        // Position cursor
        let screen_y = self.cy.saturating_sub(self.scroll_row) + 1;
        let screen_x = self.cx.saturating_sub(self.scroll_col) + 1;
        anyos_std::print!("\x1B[{};{}H", screen_y, screen_x);

        // Show cursor
        anyos_std::print!("\x1B[?25h");
    }

    fn render_status_bar(&self) {
        let status_row = self.edit_rows() + 1;
        anyos_std::print!("\x1B[{};1H", status_row);
        anyos_std::print!("\x1B[7m"); // Inverse video

        let fname = if self.filename.is_empty() { "New Buffer" } else { &self.filename };
        let modified_indicator = if self.modified { " [Modified]" } else { "" };
        let left = format!(" nano  {}{}", fname, modified_indicator);
        let right = format!("Ln {}, Col {} ", self.cy + 1, self.cx + 1);

        let mut bar = left.clone();
        let fill = self.screen_cols.saturating_sub(bar.len() + right.len());
        for _ in 0..fill {
            bar.push(' ');
        }
        bar.push_str(&right);
        // Truncate to screen width
        if bar.len() > self.screen_cols {
            bar.truncate(self.screen_cols);
        }
        anyos_std::print!("{}", bar);
        anyos_std::print!("\x1B[0m"); // Reset
    }

    fn render_help_line(&self) {
        let help_row = self.edit_rows() + 2;
        anyos_std::print!("\x1B[{};1H", help_row);

        if self.message_is_prompt {
            anyos_std::print!("{}{}", self.message, self.prompt_input);
        } else if !self.message.is_empty() {
            anyos_std::print!("{}", self.message);
        } else {
            // Default help hints
            anyos_std::print!("\x1B[7m^X\x1B[0m Exit  \x1B[7m^O\x1B[0m Save  \x1B[7m^W\x1B[0m Search  \x1B[7m^K\x1B[0m Cut  \x1B[7m^U\x1B[0m Paste  \x1B[7m^G\x1B[0m Help");
        }
        anyos_std::print!("\x1B[K"); // Clear rest
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn get_terminal_size() -> (usize, usize) {
    // Try LINES/COLUMNS env vars
    let mut buf = [0u8; 16];
    let rows = {
        let n = anyos_std::env::get("LINES", &mut buf);
        if n != u32::MAX {
            parse_int(core::str::from_utf8(&buf[..n as usize]).unwrap_or("24")).unwrap_or(24) as usize
        } else {
            24
        }
    };
    let cols = {
        let n = anyos_std::env::get("COLUMNS", &mut buf);
        if n != u32::MAX {
            parse_int(core::str::from_utf8(&buf[..n as usize]).unwrap_or("80")).unwrap_or(80) as usize
        } else {
            80
        }
    };
    (rows, cols)
}

fn parse_int(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let mut result = 0u32;
    for c in s.chars() {
        if c >= '0' && c <= '9' {
            result = result * 10 + (c as u32 - '0' as u32);
        } else {
            break;
        }
    }
    Some(result)
}

// Extension trait for lowercase
trait ToLowercase {
    fn to_lowercase(&self) -> String;
}

impl ToLowercase for String {
    fn to_lowercase(&self) -> String {
        let mut result = String::new();
        for c in self.chars() {
            if c >= 'A' && c <= 'Z' {
                result.push((c as u8 + 32) as char);
            } else {
                result.push(c);
            }
        }
        result
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut editor = Editor::new();
    let mut input = InputReader::new();

    // Parse arguments
    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);
    let arg = args_str.trim();
    if !arg.is_empty() {
        editor.load_file(arg);
    }

    // Clear screen
    anyos_std::print!("\x1B[2J");

    // Show initial help
    if editor.filename.is_empty() {
        editor.set_message("nano: New Buffer. ^O to save, ^X to exit.");
    }

    // Main loop
    while editor.running {
        editor.render();

        let key = input.read_key();
        if key == Key::None {
            anyos_std::process::sleep(10);
            continue;
        }

        editor.handle_key(key);
    }

    // Cleanup: clear screen
    anyos_std::print!("\x1B[2J\x1B[H");
}
