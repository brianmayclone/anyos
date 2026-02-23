#![no_std]
#![no_main]

use alloc::vec::Vec;

anyos_std::entry!(main);

// ── Key definitions ─────────────────────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum Key {
    None,
    Char(u8),
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Up, Down, Left, Right,
    Home, End,
    PageUp, PageDown,
    Ctrl(u8), // 'a'..'z'
}

fn read_key() -> Key {
    let mut buf = [0u8; 8];
    let n = anyos_std::fs::read(0, &mut buf);
    if n == 0 || n == u32::MAX {
        return Key::None;
    }
    let n = n as usize;
    match buf[0] {
        b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7f => Key::Backspace,
        0x1b => {
            if n == 1 { return Key::Escape; }
            if n >= 3 && buf[1] == b'[' {
                match buf[2] {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    b'C' => Key::Right,
                    b'D' => Key::Left,
                    b'H' => Key::Home,
                    b'F' => Key::End,
                    b'3' if n >= 4 && buf[3] == b'~' => Key::Delete,
                    b'5' if n >= 4 && buf[3] == b'~' => Key::PageUp,
                    b'6' if n >= 4 && buf[3] == b'~' => Key::PageDown,
                    _ => Key::None,
                }
            } else if n >= 3 && buf[1] == b'O' {
                match buf[2] {
                    b'P' => Key::Ctrl(b'g'), // F1 → help
                    _ => Key::None,
                }
            } else {
                Key::Escape
            }
        }
        1..=26 => Key::Ctrl(b'a' + buf[0] - 1),
        c if c >= b' ' => Key::Char(c),
        _ => Key::None,
    }
}

// ── Output helpers ──────────────────────────────────────────────────────────

fn out(data: &[u8]) {
    anyos_std::fs::write(1, data);
}

fn out_u32(n: usize) {
    if n == 0 { out(b"0"); return; }
    let mut buf = [0u8; 10];
    let mut val = n;
    let mut len = 0;
    while val > 0 { buf[len] = b'0' + (val % 10) as u8; val /= 10; len += 1; }
    let mut rev = [0u8; 10];
    for i in 0..len { rev[i] = buf[len - 1 - i]; }
    out(&rev[..len]);
}

/// Move cursor to (row, col) — 1-based.
fn move_cursor(row: usize, col: usize) {
    out(b"\x1b[");
    out_u32(row);
    out(b";");
    out_u32(col);
    out(b"H");
}

// ── Editor state ────────────────────────────────────────────────────────────

const ROWS: usize = 24;
const COLS: usize = 80;

struct Editor {
    lines: Vec<Vec<u8>>,
    cx: usize,
    cy: usize,
    row_off: usize,
    col_off: usize,
    filename: Vec<u8>,
    modified: bool,
    cut_buf: Vec<u8>,
    message: Vec<u8>,
    msg_time: u32,
    quit_pending: bool,
    running: bool,
    // Search state
    search_active: bool,
    search_buf: Vec<u8>,
    // Prompt state
    prompt_active: bool,
    prompt_msg: Vec<u8>,
    prompt_buf: Vec<u8>,
    prompt_kind: u8, // 0=none, 1=save-as, 2=search, 3=save-confirm
}

impl Editor {
    fn new() -> Self {
        Editor {
            lines: alloc::vec![Vec::new()],
            cx: 0, cy: 0,
            row_off: 0, col_off: 0,
            filename: Vec::new(),
            modified: false,
            cut_buf: Vec::new(),
            message: Vec::new(),
            msg_time: 0,
            quit_pending: false,
            running: true,
            search_active: false,
            search_buf: Vec::new(),
            prompt_active: false,
            prompt_msg: Vec::new(),
            prompt_buf: Vec::new(),
            prompt_kind: 0,
        }
    }

    fn screen_rows(&self) -> usize {
        if ROWS > 3 { ROWS - 3 } else { 1 } // status + shortcut bars
    }

    fn load_file(&mut self, path: &[u8]) {
        self.filename = path.to_vec();
        let path_str = core::str::from_utf8(path).unwrap_or("");
        let fd = anyos_std::fs::open(path_str, 0);
        if fd == u32::MAX {
            // New file — keep empty buffer
            self.set_message(b"[ New File ]");
            return;
        }
        self.lines.clear();
        let mut current_line = Vec::new();
        let mut read_buf = [0u8; 512];
        loop {
            let n = anyos_std::fs::read(fd, &mut read_buf);
            if n == 0 || n == u32::MAX { break; }
            for i in 0..n as usize {
                if read_buf[i] == b'\n' {
                    self.lines.push(core::mem::replace(&mut current_line, Vec::new()));
                } else if read_buf[i] != b'\r' {
                    current_line.push(read_buf[i]);
                }
            }
        }
        self.lines.push(current_line);
        anyos_std::fs::close(fd);
        self.modified = false;
    }

    fn save_file(&mut self) -> bool {
        if self.filename.is_empty() {
            return false;
        }
        let path_str = core::str::from_utf8(&self.filename).unwrap_or("");
        let fd = anyos_std::fs::open(path_str,
            anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC);
        if fd == u32::MAX {
            self.set_message(b"Error: cannot write file");
            return false;
        }
        for (i, line) in self.lines.iter().enumerate() {
            anyos_std::fs::write(fd, line);
            if i + 1 < self.lines.len() {
                anyos_std::fs::write(fd, b"\n");
            }
        }
        anyos_std::fs::close(fd);
        self.modified = false;
        self.set_message(b"Wrote file");
        true
    }

    fn set_message(&mut self, msg: &[u8]) {
        self.message = msg.to_vec();
        self.msg_time = anyos_std::sys::uptime();
    }

    fn current_line_len(&self) -> usize {
        if self.cy < self.lines.len() {
            self.lines[self.cy].len()
        } else { 0 }
    }

    fn clamp_cx(&mut self) {
        let len = self.current_line_len();
        if self.cx > len { self.cx = len; }
    }

    fn scroll(&mut self) {
        let sr = self.screen_rows();
        if self.cy < self.row_off { self.row_off = self.cy; }
        if self.cy >= self.row_off + sr { self.row_off = self.cy - sr + 1; }
        if self.cx < self.col_off { self.col_off = self.cx; }
        if self.cx >= self.col_off + COLS { self.col_off = self.cx - COLS + 1; }
    }

    fn insert_char(&mut self, c: u8) {
        if self.cy >= self.lines.len() {
            self.lines.push(Vec::new());
        }
        if self.cx > self.lines[self.cy].len() {
            self.cx = self.lines[self.cy].len();
        }
        self.lines[self.cy].insert(self.cx, c);
        self.cx += 1;
        self.modified = true;
    }

    fn insert_newline(&mut self) {
        if self.cy >= self.lines.len() {
            self.lines.push(Vec::new());
            self.lines.push(Vec::new());
        } else {
            let rest: Vec<u8> = self.lines[self.cy].split_off(self.cx);
            self.lines.insert(self.cy + 1, rest);
        }
        self.cy += 1;
        self.cx = 0;
        self.modified = true;
    }

    fn delete_char(&mut self) {
        if self.cy >= self.lines.len() { return; }
        if self.cx > 0 {
            self.cx -= 1;
            self.lines[self.cy].remove(self.cx);
            self.modified = true;
        } else if self.cy > 0 {
            let line = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
            self.lines[self.cy].extend_from_slice(&line);
            self.modified = true;
        }
    }

    fn delete_char_forward(&mut self) {
        if self.cy >= self.lines.len() { return; }
        if self.cx < self.lines[self.cy].len() {
            self.lines[self.cy].remove(self.cx);
            self.modified = true;
        } else if self.cy + 1 < self.lines.len() {
            let next = self.lines.remove(self.cy + 1);
            self.lines[self.cy].extend_from_slice(&next);
            self.modified = true;
        }
    }

    fn cut_line(&mut self) {
        if self.cy < self.lines.len() {
            self.cut_buf = self.lines.remove(self.cy).clone();
            if self.lines.is_empty() {
                self.lines.push(Vec::new());
            }
            if self.cy >= self.lines.len() {
                self.cy = self.lines.len() - 1;
            }
            self.clamp_cx();
            self.modified = true;
        }
    }

    fn paste_line(&mut self) {
        if self.cut_buf.is_empty() { return; }
        self.lines.insert(self.cy + 1, self.cut_buf.clone());
        self.cy += 1;
        self.cx = 0;
        self.modified = true;
    }

    fn search_forward(&mut self) {
        if self.search_buf.is_empty() { return; }
        let start_y = self.cy;
        let start_x = self.cx + 1;
        for dy in 0..self.lines.len() {
            let y = (start_y + dy) % self.lines.len();
            let sx = if dy == 0 { start_x } else { 0 };
            let line = &self.lines[y];
            if line.len() >= self.search_buf.len() + sx {
                for x in sx..=line.len().saturating_sub(self.search_buf.len()) {
                    if &line[x..x + self.search_buf.len()] == self.search_buf.as_slice() {
                        self.cy = y;
                        self.cx = x;
                        return;
                    }
                }
            }
        }
        self.set_message(b"Not found");
    }

    // ── Rendering ───────────────────────────────────────────────────────────

    fn render(&mut self) {
        self.scroll();

        out(b"\x1b[?25l"); // hide cursor
        out(b"\x1b[H");    // top-left

        // Title bar (reverse video)
        out(b"\x1b[7m");
        let title_start = b"  nano  ";
        out(title_start);
        if self.filename.is_empty() {
            out(b"[No Name]");
        } else {
            let show = if self.filename.len() > 50 { &self.filename[self.filename.len()-50..] } else { &self.filename };
            out(show);
        }
        if self.modified { out(b" [Modified]"); }
        // Pad to full width
        let used = title_start.len() + if self.filename.is_empty() { 9 } else { self.filename.len().min(50) }
            + if self.modified { 11 } else { 0 };
        for _ in used..COLS { out(b" "); }
        out(b"\x1b[0m\r\n");

        // Text area
        let sr = self.screen_rows();
        for i in 0..sr {
            let file_row = self.row_off + i;
            if file_row < self.lines.len() {
                let line = &self.lines[file_row];
                if self.col_off < line.len() {
                    let end = (self.col_off + COLS).min(line.len());
                    out(&line[self.col_off..end]);
                }
            } else {
                out(b"~");
            }
            out(b"\x1b[K\r\n"); // clear to EOL
        }

        // Message / prompt line
        if self.prompt_active {
            out(&self.prompt_msg);
            out(&self.prompt_buf);
            out(b"\x1b[K");
        } else if !self.message.is_empty() {
            let elapsed = anyos_std::sys::uptime().wrapping_sub(self.msg_time);
            let hz = anyos_std::sys::tick_hz();
            if hz > 0 && elapsed < hz * 5 {
                let show = if self.message.len() > COLS { &self.message[..COLS] } else { &self.message };
                out(show);
            } else {
                self.message.clear();
            }
            out(b"\x1b[K");
        } else {
            out(b"\x1b[K");
        }
        out(b"\r\n");

        // Shortcut bars (2 lines, reverse video)
        out(b"\x1b[7m");
        out(b"^G Help  ^O Write  ^W Search ^K Cut   ^U Paste  ^X Exit ");
        let shortcut_len = 57;
        for _ in shortcut_len..COLS { out(b" "); }
        out(b"\x1b[0m\r\n");
        out(b"\x1b[7m");
        out(b"^\\  Replace               ^J Justify               ^T Spell");
        let shortcut2_len = 60;
        for _ in shortcut2_len..COLS { out(b" "); }
        out(b"\x1b[0m");

        // Position cursor
        let cursor_row = 2 + (self.cy - self.row_off); // 1 for title bar, 1-based
        let cursor_col = 1 + (self.cx - self.col_off);
        if self.prompt_active {
            move_cursor(sr + 2, self.prompt_msg.len() + self.prompt_buf.len() + 1);
        } else {
            move_cursor(cursor_row, cursor_col);
        }
        out(b"\x1b[?25h"); // show cursor
    }

    // ── Key processing ──────────────────────────────────────────────────────

    fn process_key(&mut self, key: Key) {
        // Handle prompt mode
        if self.prompt_active {
            match key {
                Key::Enter => {
                    let kind = self.prompt_kind;
                    self.prompt_active = false;
                    match kind {
                        1 => {
                            // Save-as: filename entered
                            self.filename = self.prompt_buf.clone();
                            self.save_file();
                        }
                        2 => {
                            // Search
                            self.search_buf = self.prompt_buf.clone();
                            self.search_forward();
                        }
                        _ => {}
                    }
                    self.prompt_buf.clear();
                    self.prompt_msg.clear();
                    self.prompt_kind = 0;
                }
                Key::Escape | Key::Ctrl(b'c') => {
                    self.prompt_active = false;
                    self.prompt_buf.clear();
                    self.prompt_msg.clear();
                    self.prompt_kind = 0;
                    self.set_message(b"Cancelled");
                }
                Key::Backspace => {
                    self.prompt_buf.pop();
                }
                Key::Char(c) => {
                    self.prompt_buf.push(c);
                }
                _ => {}
            }
            return;
        }

        self.quit_pending = false;

        match key {
            Key::None => {}
            Key::Up => { if self.cy > 0 { self.cy -= 1; self.clamp_cx(); } }
            Key::Down => { if self.cy + 1 < self.lines.len() { self.cy += 1; self.clamp_cx(); } }
            Key::Left => {
                if self.cx > 0 { self.cx -= 1; }
                else if self.cy > 0 { self.cy -= 1; self.cx = self.current_line_len(); }
            }
            Key::Right => {
                if self.cx < self.current_line_len() { self.cx += 1; }
                else if self.cy + 1 < self.lines.len() { self.cy += 1; self.cx = 0; }
            }
            Key::Home => { self.cx = 0; }
            Key::End => { self.cx = self.current_line_len(); }
            Key::PageUp => {
                let sr = self.screen_rows();
                self.cy = self.cy.saturating_sub(sr);
                self.clamp_cx();
            }
            Key::PageDown => {
                let sr = self.screen_rows();
                self.cy = (self.cy + sr).min(self.lines.len().saturating_sub(1));
                self.clamp_cx();
            }
            Key::Enter => { self.insert_newline(); }
            Key::Tab => { self.insert_char(b' '); self.insert_char(b' '); self.insert_char(b' '); self.insert_char(b' '); }
            Key::Backspace => { self.delete_char(); }
            Key::Delete => { self.delete_char_forward(); }
            Key::Char(c) => { self.insert_char(c); }
            Key::Ctrl(b'x') => {
                // Exit
                if self.modified && !self.quit_pending {
                    self.set_message(b"Modified buffer! Press Ctrl+X again to exit without saving");
                    self.quit_pending = true;
                    return; // Don't clear quit_pending below
                }
                self.running = false;
            }
            Key::Ctrl(b'o') => {
                // Write Out
                if self.filename.is_empty() {
                    self.prompt_active = true;
                    self.prompt_msg = b"File Name to Write: ".to_vec();
                    self.prompt_buf.clear();
                    self.prompt_kind = 1;
                } else {
                    self.save_file();
                }
            }
            Key::Ctrl(b'k') => { self.cut_line(); }
            Key::Ctrl(b'u') => { self.paste_line(); }
            Key::Ctrl(b'w') => {
                // Search
                self.prompt_active = true;
                self.prompt_msg = b"Search: ".to_vec();
                self.prompt_buf.clear();
                self.prompt_kind = 2;
            }
            Key::Ctrl(b'g') => {
                self.set_message(b"nano: ^O Save  ^X Exit  ^K Cut  ^U Paste  ^W Search  Arrows move");
            }
            _ => {}
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let mut editor = Editor::new();

    if args.pos_count > 0 {
        editor.load_file(args.positional[0].as_bytes());
    }

    // Clear screen
    out(b"\x1b[2J");

    while editor.running {
        editor.render();
        let key = read_key();
        if key != Key::None {
            editor.process_key(key);
        } else {
            anyos_std::process::sleep(10);
        }
    }

    // Cleanup: clear screen and move cursor to top
    out(b"\x1b[2J\x1b[H");
}
