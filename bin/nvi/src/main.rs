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
                Key::None
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

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Normal,
    Insert,
    Command,
    Search,
}

struct Editor {
    lines: Vec<Vec<u8>>,
    cx: usize,
    cy: usize,
    row_off: usize,
    col_off: usize,
    filename: Vec<u8>,
    modified: bool,
    mode: Mode,
    // Command-line buffer (for : commands)
    cmd_buf: Vec<u8>,
    // Search
    search_buf: Vec<u8>,
    search_dir: i8, // 1 = forward, -1 = backward
    // Yank / delete buffer
    yank_buf: Vec<Vec<u8>>,
    yank_is_line: bool,
    // Undo (single-level snapshot)
    undo_lines: Vec<Vec<u8>>,
    undo_cx: usize,
    undo_cy: usize,
    // Pending key for multi-key combos (d, g, etc.)
    pending: u8,
    // Status message
    message: Vec<u8>,
    msg_time: u32,
    // Running flag
    running: bool,
}

impl Editor {
    fn new() -> Self {
        Editor {
            lines: alloc::vec![Vec::new()],
            cx: 0, cy: 0,
            row_off: 0, col_off: 0,
            filename: Vec::new(),
            modified: false,
            mode: Mode::Normal,
            cmd_buf: Vec::new(),
            search_buf: Vec::new(),
            search_dir: 1,
            yank_buf: Vec::new(),
            yank_is_line: false,
            undo_lines: Vec::new(),
            undo_cx: 0,
            undo_cy: 0,
            pending: 0,
            message: Vec::new(),
            msg_time: 0,
            running: true,
        }
    }

    fn screen_rows(&self) -> usize {
        if ROWS > 2 { ROWS - 2 } else { 1 } // status line + command/message line
    }

    fn load_file(&mut self, path: &[u8]) {
        self.filename = path.to_vec();
        let path_str = core::str::from_utf8(path).unwrap_or("");
        let fd = anyos_std::fs::open(path_str, 0);
        if fd == u32::MAX {
            self.set_message(b"[New File]");
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
        let line_count = self.lines.len();
        let mut msg = Vec::new();
        msg.extend_from_slice(b"\"");
        msg.extend_from_slice(path);
        msg.extend_from_slice(b"\" ");
        append_usize(&mut msg, line_count);
        msg.extend_from_slice(b" lines");
        self.set_message(&msg);
    }

    fn save_file(&mut self) -> bool {
        if self.filename.is_empty() {
            self.set_message(b"No file name");
            return false;
        }
        let path_str = core::str::from_utf8(&self.filename).unwrap_or("");
        let fd = anyos_std::fs::open(path_str,
            anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC);
        if fd == u32::MAX {
            self.set_message(b"Error: cannot write file");
            return false;
        }
        let mut bytes_written: usize = 0;
        for (i, line) in self.lines.iter().enumerate() {
            anyos_std::fs::write(fd, line);
            bytes_written += line.len();
            if i + 1 < self.lines.len() {
                anyos_std::fs::write(fd, b"\n");
                bytes_written += 1;
            }
        }
        anyos_std::fs::close(fd);
        self.modified = false;
        let mut msg = Vec::new();
        msg.extend_from_slice(b"\"");
        msg.extend_from_slice(&self.filename);
        msg.extend_from_slice(b"\" ");
        append_usize(&mut msg, self.lines.len());
        msg.extend_from_slice(b"L, ");
        append_usize(&mut msg, bytes_written);
        msg.extend_from_slice(b"B written");
        self.set_message(&msg);
        true
    }

    fn set_message(&mut self, msg: &[u8]) {
        self.message = msg.to_vec();
        self.msg_time = anyos_std::sys::uptime();
    }

    fn save_undo(&mut self) {
        self.undo_lines = self.lines.clone();
        self.undo_cx = self.cx;
        self.undo_cy = self.cy;
    }

    fn restore_undo(&mut self) {
        if self.undo_lines.is_empty() { return; }
        let old_lines = core::mem::replace(&mut self.undo_lines, self.lines.clone());
        let old_cx = self.undo_cx;
        let old_cy = self.undo_cy;
        self.undo_cx = self.cx;
        self.undo_cy = self.cy;
        self.lines = old_lines;
        self.cx = old_cx;
        self.cy = old_cy;
        self.clamp_cursor();
        self.modified = true;
    }

    fn current_line_len(&self) -> usize {
        if self.cy < self.lines.len() {
            self.lines[self.cy].len()
        } else { 0 }
    }

    fn clamp_cx_normal(&mut self) {
        let len = self.current_line_len();
        if len == 0 {
            self.cx = 0;
        } else if self.cx >= len {
            self.cx = len - 1;
        }
    }

    fn clamp_cx_insert(&mut self) {
        let len = self.current_line_len();
        if self.cx > len { self.cx = len; }
    }

    fn clamp_cursor(&mut self) {
        if self.cy >= self.lines.len() {
            self.cy = if self.lines.is_empty() { 0 } else { self.lines.len() - 1 };
        }
        if self.mode == Mode::Insert {
            self.clamp_cx_insert();
        } else {
            self.clamp_cx_normal();
        }
    }

    fn scroll(&mut self) {
        let sr = self.screen_rows();
        if self.cy < self.row_off { self.row_off = self.cy; }
        if self.cy >= self.row_off + sr { self.row_off = self.cy - sr + 1; }
        if self.cx < self.col_off { self.col_off = self.cx; }
        if self.cx >= self.col_off + COLS { self.col_off = self.cx - COLS + 1; }
    }

    // ── Text manipulation ───────────────────────────────────────────────────

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

    fn delete_char_back(&mut self) {
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

    fn delete_char_at_cursor(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let len = self.lines[self.cy].len();
        if len > 0 && self.cx < len {
            self.lines[self.cy].remove(self.cx);
            self.modified = true;
            if self.cx >= self.lines[self.cy].len() && self.cx > 0 {
                self.cx -= 1;
            }
        }
    }

    fn delete_line(&mut self) {
        if self.cy < self.lines.len() {
            self.yank_buf.clear();
            self.yank_buf.push(self.lines[self.cy].clone());
            self.yank_is_line = true;
            self.lines.remove(self.cy);
            if self.lines.is_empty() {
                self.lines.push(Vec::new());
            }
            if self.cy >= self.lines.len() {
                self.cy = self.lines.len() - 1;
            }
            self.clamp_cx_normal();
            self.modified = true;
        }
    }

    fn delete_to_end_of_line(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let len = self.lines[self.cy].len();
        if self.cx < len {
            self.yank_buf.clear();
            self.yank_buf.push(self.lines[self.cy][self.cx..].to_vec());
            self.yank_is_line = false;
            self.lines[self.cy].truncate(self.cx);
            self.modified = true;
            if self.cx > 0 { self.cx -= 1; }
        }
    }

    fn delete_word(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let line = &self.lines[self.cy];
        let start = self.cx;
        let mut end = start;
        let len = line.len();
        if end >= len { return; }

        // Skip current word characters
        if is_word_char(line[end]) {
            while end < len && is_word_char(line[end]) { end += 1; }
        } else {
            while end < len && !is_word_char(line[end]) && line[end] != b' ' { end += 1; }
        }
        // Skip trailing whitespace
        while end < len && line[end] == b' ' { end += 1; }

        self.yank_buf.clear();
        self.yank_buf.push(self.lines[self.cy][start..end].to_vec());
        self.yank_is_line = false;
        self.lines[self.cy].drain(start..end);
        self.modified = true;
        self.clamp_cx_normal();
    }

    fn yank_line(&mut self) {
        if self.cy < self.lines.len() {
            self.yank_buf.clear();
            self.yank_buf.push(self.lines[self.cy].clone());
            self.yank_is_line = true;
            self.set_message(b"1 line yanked");
        }
    }

    fn paste_after(&mut self) {
        if self.yank_buf.is_empty() { return; }
        self.save_undo();
        if self.yank_is_line {
            // Insert line(s) below
            for (i, line) in self.yank_buf.clone().iter().enumerate() {
                self.lines.insert(self.cy + 1 + i, line.clone());
            }
            self.cy += 1;
            self.cx = 0;
        } else {
            // Insert text after cursor
            if let Some(text) = self.yank_buf.first().cloned() {
                if self.cy < self.lines.len() {
                    let pos = (self.cx + 1).min(self.lines[self.cy].len());
                    for (i, &c) in text.iter().enumerate() {
                        self.lines[self.cy].insert(pos + i, c);
                    }
                    self.cx = pos + text.len().saturating_sub(1);
                }
            }
        }
        self.modified = true;
    }

    fn paste_before(&mut self) {
        if self.yank_buf.is_empty() { return; }
        self.save_undo();
        if self.yank_is_line {
            for (i, line) in self.yank_buf.clone().iter().enumerate() {
                self.lines.insert(self.cy + i, line.clone());
            }
            self.cx = 0;
        } else {
            if let Some(text) = self.yank_buf.first().cloned() {
                if self.cy < self.lines.len() {
                    for (i, &c) in text.iter().enumerate() {
                        self.lines[self.cy].insert(self.cx + i, c);
                    }
                }
            }
        }
        self.modified = true;
    }

    // ── Movement ────────────────────────────────────────────────────────────

    fn move_word_forward(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let line = &self.lines[self.cy];
        let len = line.len();
        if len == 0 {
            // Move to next line
            if self.cy + 1 < self.lines.len() {
                self.cy += 1;
                self.cx = 0;
            }
            return;
        }
        let mut x = self.cx;
        if x >= len {
            if self.cy + 1 < self.lines.len() {
                self.cy += 1;
                self.cx = 0;
            }
            return;
        }
        // Skip current word
        if is_word_char(line[x]) {
            while x < len && is_word_char(line[x]) { x += 1; }
        } else {
            while x < len && !is_word_char(line[x]) && line[x] != b' ' { x += 1; }
        }
        // Skip whitespace
        while x < len && line[x] == b' ' { x += 1; }
        if x >= len {
            if self.cy + 1 < self.lines.len() {
                self.cy += 1;
                self.cx = 0;
                return;
            }
        }
        self.cx = x;
    }

    fn move_word_backward(&mut self) {
        if self.cy >= self.lines.len() { return; }
        if self.cx == 0 {
            if self.cy > 0 {
                self.cy -= 1;
                let len = self.current_line_len();
                self.cx = if len > 0 { len - 1 } else { 0 };
            }
            return;
        }
        let line = &self.lines[self.cy];
        let mut x = self.cx;
        // Skip whitespace backward
        while x > 0 && line[x - 1] == b' ' { x -= 1; }
        // Skip word backward
        if x > 0 && is_word_char(line[x - 1]) {
            while x > 0 && is_word_char(line[x - 1]) { x -= 1; }
        } else {
            while x > 0 && !is_word_char(line[x - 1]) && line[x - 1] != b' ' { x -= 1; }
        }
        self.cx = x;
    }

    // ── Search ──────────────────────────────────────────────────────────────

    fn search_next(&mut self) {
        if self.search_buf.is_empty() { return; }
        let total = self.lines.len();
        if total == 0 { return; }

        let start_y = self.cy;
        let start_x = if self.search_dir > 0 { self.cx + 1 } else { self.cx };

        if self.search_dir > 0 {
            // Forward search
            for dy in 0..total {
                let y = (start_y + dy) % total;
                let sx = if dy == 0 { start_x } else { 0 };
                let line = &self.lines[y];
                if line.len() >= self.search_buf.len() {
                    let end = line.len() - self.search_buf.len() + 1;
                    if sx < end {
                        for x in sx..end {
                            if &line[x..x + self.search_buf.len()] == self.search_buf.as_slice() {
                                self.cy = y;
                                self.cx = x;
                                return;
                            }
                        }
                    }
                }
            }
        } else {
            // Backward search
            for dy in 0..total {
                let y = (start_y + total - dy) % total;
                let line = &self.lines[y];
                let max_x = if dy == 0 {
                    if start_x > 0 { start_x - 1 } else { continue; }
                } else {
                    if line.len() >= self.search_buf.len() { line.len() - self.search_buf.len() } else { continue; }
                };
                if line.len() >= self.search_buf.len() {
                    let mut x = max_x.min(line.len() - self.search_buf.len());
                    loop {
                        if &line[x..x + self.search_buf.len()] == self.search_buf.as_slice() {
                            self.cy = y;
                            self.cx = x;
                            return;
                        }
                        if x == 0 { break; }
                        x -= 1;
                    }
                }
            }
        }
        self.set_message(b"Pattern not found");
    }

    // ── Rendering ───────────────────────────────────────────────────────────

    fn render(&mut self) {
        self.scroll();

        out(b"\x1b[?25l"); // hide cursor
        out(b"\x1b[H");    // top-left

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

        // Status line (reverse video)
        out(b"\x1b[7m");
        // Left side: filename + modified + line count
        if self.filename.is_empty() {
            out(b"[No Name]");
        } else {
            let show = if self.filename.len() > 30 { &self.filename[self.filename.len()-30..] } else { &self.filename };
            out(show);
        }
        if self.modified { out(b" [+]"); }
        let left_used = if self.filename.is_empty() { 9 } else { self.filename.len().min(30) }
            + if self.modified { 4 } else { 0 };

        // Right side: mode + line/total
        let mut right_buf = [0u8; 64];
        let mut rlen = 0;
        // Mode indicator
        let mode_str = match self.mode {
            Mode::Normal => b"NORMAL" as &[u8],
            Mode::Insert => b"INSERT",
            Mode::Command => b"COMMAND",
            Mode::Search => b"SEARCH",
        };
        for &b in mode_str {
            if rlen < 64 { right_buf[rlen] = b; rlen += 1; }
        }
        if rlen < 64 { right_buf[rlen] = b' '; rlen += 1; }
        // line/total
        let cy_str = usize_to_buf(self.cy + 1);
        for &b in &cy_str.0[..cy_str.1] {
            if rlen < 64 { right_buf[rlen] = b; rlen += 1; }
        }
        if rlen < 64 { right_buf[rlen] = b'/'; rlen += 1; }
        let total_str = usize_to_buf(self.lines.len());
        for &b in &total_str.0[..total_str.1] {
            if rlen < 64 { right_buf[rlen] = b; rlen += 1; }
        }

        // Pad between left and right
        let total_used = left_used + rlen;
        if total_used < COLS {
            for _ in 0..(COLS - total_used) { out(b" "); }
        }
        out(&right_buf[..rlen]);
        out(b"\x1b[0m\r\n");

        // Command/message line
        match self.mode {
            Mode::Command => {
                out(b":");
                out(&self.cmd_buf);
                out(b"\x1b[K");
            }
            Mode::Search => {
                if self.search_dir > 0 {
                    out(b"/");
                } else {
                    out(b"?");
                }
                out(&self.search_buf);
                out(b"\x1b[K");
            }
            _ => {
                if !self.message.is_empty() {
                    let elapsed = anyos_std::sys::uptime().wrapping_sub(self.msg_time);
                    let hz = anyos_std::sys::tick_hz();
                    if hz > 0 && elapsed < hz * 5 {
                        let show = if self.message.len() > COLS { &self.message[..COLS] } else { &self.message };
                        out(show);
                    } else {
                        self.message.clear();
                    }
                }
                out(b"\x1b[K");
            }
        }

        // Position cursor
        match self.mode {
            Mode::Command => {
                move_cursor(ROWS, 2 + self.cmd_buf.len());
            }
            Mode::Search => {
                move_cursor(ROWS, 2 + self.search_buf.len());
            }
            _ => {
                let cursor_row = 1 + (self.cy - self.row_off);
                let cursor_col = 1 + self.cx.saturating_sub(self.col_off);
                move_cursor(cursor_row, cursor_col);
            }
        }
        out(b"\x1b[?25h"); // show cursor
    }

    // ── Key processing ──────────────────────────────────────────────────────

    fn process_key(&mut self, key: Key) {
        match self.mode {
            Mode::Normal => self.process_normal(key),
            Mode::Insert => self.process_insert(key),
            Mode::Command => self.process_command(key),
            Mode::Search => self.process_search(key),
        }
    }

    fn process_normal(&mut self, key: Key) {
        // Handle pending multi-key commands
        if self.pending == b'd' {
            self.pending = 0;
            match key {
                Key::Char(b'd') => {
                    self.save_undo();
                    self.delete_line();
                    return;
                }
                Key::Char(b'w') => {
                    self.save_undo();
                    self.delete_word();
                    return;
                }
                _ => { return; }
            }
        }
        if self.pending == b'g' {
            self.pending = 0;
            match key {
                Key::Char(b'g') => {
                    // Go to first line
                    self.cy = 0;
                    self.cx = 0;
                    return;
                }
                _ => { return; }
            }
        }

        match key {
            Key::None => {}
            // Movement
            Key::Char(b'h') | Key::Left => {
                if self.cx > 0 { self.cx -= 1; }
            }
            Key::Char(b'j') | Key::Down => {
                if self.cy + 1 < self.lines.len() {
                    self.cy += 1;
                    self.clamp_cx_normal();
                }
            }
            Key::Char(b'k') | Key::Up => {
                if self.cy > 0 {
                    self.cy -= 1;
                    self.clamp_cx_normal();
                }
            }
            Key::Char(b'l') | Key::Right => {
                let len = self.current_line_len();
                if len > 0 && self.cx + 1 < len { self.cx += 1; }
            }
            Key::Char(b'0') | Key::Home => { self.cx = 0; }
            Key::Char(b'$') | Key::End => {
                let len = self.current_line_len();
                self.cx = if len > 0 { len - 1 } else { 0 };
            }
            Key::Char(b'^') => {
                // First non-whitespace
                if self.cy < self.lines.len() {
                    let line = &self.lines[self.cy];
                    self.cx = 0;
                    while self.cx < line.len() && line[self.cx] == b' ' { self.cx += 1; }
                    if self.cx >= line.len() && self.cx > 0 { self.cx -= 1; }
                }
            }
            Key::Char(b'w') => { self.move_word_forward(); self.clamp_cx_normal(); }
            Key::Char(b'b') => { self.move_word_backward(); }
            Key::Char(b'G') => {
                // Go to last line
                self.cy = if self.lines.is_empty() { 0 } else { self.lines.len() - 1 };
                self.cx = 0;
            }
            Key::Char(b'g') => { self.pending = b'g'; }
            Key::PageUp | Key::Ctrl(b'b') => {
                let sr = self.screen_rows();
                self.cy = self.cy.saturating_sub(sr);
                self.clamp_cx_normal();
            }
            Key::PageDown | Key::Ctrl(b'f') => {
                let sr = self.screen_rows();
                self.cy = (self.cy + sr).min(self.lines.len().saturating_sub(1));
                self.clamp_cx_normal();
            }

            // Insert mode entries
            Key::Char(b'i') => {
                self.mode = Mode::Insert;
            }
            Key::Char(b'a') => {
                self.mode = Mode::Insert;
                let len = self.current_line_len();
                if len > 0 { self.cx = (self.cx + 1).min(len); }
            }
            Key::Char(b'A') => {
                self.mode = Mode::Insert;
                self.cx = self.current_line_len();
            }
            Key::Char(b'I') => {
                self.mode = Mode::Insert;
                // Move to first non-whitespace
                if self.cy < self.lines.len() {
                    let line = &self.lines[self.cy];
                    self.cx = 0;
                    while self.cx < line.len() && line[self.cx] == b' ' { self.cx += 1; }
                }
            }
            Key::Char(b'o') => {
                self.save_undo();
                self.lines.insert(self.cy + 1, Vec::new());
                self.cy += 1;
                self.cx = 0;
                self.modified = true;
                self.mode = Mode::Insert;
            }
            Key::Char(b'O') => {
                self.save_undo();
                self.lines.insert(self.cy, Vec::new());
                self.cx = 0;
                self.modified = true;
                self.mode = Mode::Insert;
            }

            // Deletion
            Key::Char(b'x') => {
                self.save_undo();
                if self.cy < self.lines.len() && !self.lines[self.cy].is_empty() {
                    let c = self.lines[self.cy][self.cx];
                    self.yank_buf.clear();
                    self.yank_buf.push(alloc::vec![c]);
                    self.yank_is_line = false;
                }
                self.delete_char_at_cursor();
            }
            Key::Char(b'X') => {
                self.save_undo();
                if self.cx > 0 {
                    self.cx -= 1;
                    self.delete_char_at_cursor();
                }
            }
            Key::Char(b'd') => { self.pending = b'd'; }
            Key::Char(b'D') => {
                self.save_undo();
                self.delete_to_end_of_line();
            }

            // Change
            Key::Char(b'r') => {
                // Replace single char - wait for next key
                // We'll set pending to 'r'
                self.pending = b'r';
            }

            // Yank / Paste
            Key::Char(b'y') => {
                // yy = yank line (simplified: just y yanks current line)
                self.yank_line();
            }
            Key::Char(b'p') => {
                self.paste_after();
            }
            Key::Char(b'P') => {
                self.paste_before();
            }

            // Undo
            Key::Char(b'u') => {
                self.restore_undo();
            }

            // Search
            Key::Char(b'/') => {
                self.mode = Mode::Search;
                self.search_dir = 1;
                self.search_buf.clear();
            }
            Key::Char(b'?') => {
                self.mode = Mode::Search;
                self.search_dir = -1;
                self.search_buf.clear();
            }
            Key::Char(b'n') => {
                self.search_next();
            }
            Key::Char(b'N') => {
                self.search_dir = -self.search_dir;
                self.search_next();
                self.search_dir = -self.search_dir;
            }

            // Command mode
            Key::Char(b':') => {
                self.mode = Mode::Command;
                self.cmd_buf.clear();
            }

            // Join lines (J)
            Key::Char(b'J') => {
                if self.cy + 1 < self.lines.len() {
                    self.save_undo();
                    let next = self.lines.remove(self.cy + 1);
                    let old_len = self.lines[self.cy].len();
                    if !self.lines[self.cy].is_empty() && !next.is_empty() {
                        self.lines[self.cy].push(b' ');
                    }
                    self.lines[self.cy].extend_from_slice(&next);
                    self.cx = old_len;
                    self.modified = true;
                }
            }

            _ => {}
        }

        // Handle replace pending
        if self.pending == b'r' {
            if let Key::Char(c) = key {
                if c != b'r' {
                    self.pending = 0;
                    if self.cy < self.lines.len() && self.cx < self.lines[self.cy].len() {
                        self.save_undo();
                        self.lines[self.cy][self.cx] = c;
                        self.modified = true;
                    }
                }
            }
        }
    }

    fn process_insert(&mut self, key: Key) {
        match key {
            Key::None => {}
            Key::Escape => {
                self.mode = Mode::Normal;
                // Move cursor back one (vi convention)
                if self.cx > 0 { self.cx -= 1; }
                self.clamp_cx_normal();
            }
            Key::Enter => {
                self.save_undo();
                self.insert_newline();
            }
            Key::Backspace => { self.delete_char_back(); }
            Key::Delete => {
                if self.cy < self.lines.len() && self.cx < self.lines[self.cy].len() {
                    self.lines[self.cy].remove(self.cx);
                    self.modified = true;
                }
            }
            Key::Tab => {
                for _ in 0..4 { self.insert_char(b' '); }
            }
            Key::Up => {
                if self.cy > 0 { self.cy -= 1; self.clamp_cx_insert(); }
            }
            Key::Down => {
                if self.cy + 1 < self.lines.len() { self.cy += 1; self.clamp_cx_insert(); }
            }
            Key::Left => {
                if self.cx > 0 { self.cx -= 1; }
            }
            Key::Right => {
                if self.cx < self.current_line_len() { self.cx += 1; }
            }
            Key::Home => { self.cx = 0; }
            Key::End => { self.cx = self.current_line_len(); }
            Key::PageUp => {
                let sr = self.screen_rows();
                self.cy = self.cy.saturating_sub(sr);
                self.clamp_cx_insert();
            }
            Key::PageDown => {
                let sr = self.screen_rows();
                self.cy = (self.cy + sr).min(self.lines.len().saturating_sub(1));
                self.clamp_cx_insert();
            }
            Key::Char(c) => { self.insert_char(c); }
            Key::Ctrl(b'c') => {
                self.mode = Mode::Normal;
                if self.cx > 0 { self.cx -= 1; }
                self.clamp_cx_normal();
            }
            _ => {}
        }
    }

    fn process_command(&mut self, key: Key) {
        match key {
            Key::Enter => {
                let cmd = self.cmd_buf.clone();
                self.mode = Mode::Normal;
                self.cmd_buf.clear();
                self.execute_command(&cmd);
            }
            Key::Escape => {
                self.mode = Mode::Normal;
                self.cmd_buf.clear();
            }
            Key::Backspace => {
                if self.cmd_buf.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => {
                self.cmd_buf.push(c);
            }
            _ => {}
        }
    }

    fn process_search(&mut self, key: Key) {
        match key {
            Key::Enter => {
                self.mode = Mode::Normal;
                self.search_next();
            }
            Key::Escape => {
                self.mode = Mode::Normal;
                self.search_buf.clear();
            }
            Key::Backspace => {
                if self.search_buf.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => {
                self.search_buf.push(c);
            }
            _ => {}
        }
    }

    fn execute_command(&mut self, cmd: &[u8]) {
        let cmd = trim(cmd);
        if cmd.is_empty() { return; }

        if cmd == b"q" {
            if self.modified {
                self.set_message(b"No write since last change (add ! to override)");
            } else {
                self.running = false;
            }
        } else if cmd == b"q!" {
            self.running = false;
        } else if cmd == b"w" {
            self.save_file();
        } else if cmd.len() > 2 && cmd[0] == b'w' && cmd[1] == b' ' {
            // :w filename
            self.filename = cmd[2..].to_vec();
            self.save_file();
        } else if cmd == b"wq" || cmd == b"x" {
            if self.save_file() {
                self.running = false;
            }
        } else if cmd == b"wq!" {
            self.save_file();
            self.running = false;
        } else if is_all_digits(cmd) {
            // :NUMBER — go to line
            let n = parse_usize(cmd);
            if n > 0 {
                self.cy = (n - 1).min(self.lines.len().saturating_sub(1));
                self.cx = 0;
            }
        } else {
            let mut msg = Vec::new();
            msg.extend_from_slice(b"Not an editor command: ");
            msg.extend_from_slice(cmd);
            self.set_message(&msg);
        }
    }
}

// ── Utility functions ───────────────────────────────────────────────────────

fn is_word_char(c: u8) -> bool {
    (c >= b'a' && c <= b'z') || (c >= b'A' && c <= b'Z') || (c >= b'0' && c <= b'9') || c == b'_'
}

fn trim(s: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < s.len() && s[start] == b' ' { start += 1; }
    let mut end = s.len();
    while end > start && s[end - 1] == b' ' { end -= 1; }
    &s[start..end]
}

fn is_all_digits(s: &[u8]) -> bool {
    !s.is_empty() && s.iter().all(|&c| c >= b'0' && c <= b'9')
}

fn parse_usize(s: &[u8]) -> usize {
    let mut n: usize = 0;
    for &c in s {
        if c >= b'0' && c <= b'9' {
            n = n.saturating_mul(10).saturating_add((c - b'0') as usize);
        }
    }
    n
}

fn usize_to_buf(n: usize) -> ([u8; 10], usize) {
    let mut buf = [0u8; 10];
    if n == 0 { buf[0] = b'0'; return (buf, 1); }
    let mut val = n;
    let mut len = 0;
    while val > 0 { buf[len] = b'0' + (val % 10) as u8; val /= 10; len += 1; }
    // Reverse
    let mut i = 0;
    let mut j = len - 1;
    while i < j { buf.swap(i, j); i += 1; j -= 1; }
    (buf, len)
}

fn append_usize(v: &mut Vec<u8>, n: usize) {
    let (buf, len) = usize_to_buf(n);
    v.extend_from_slice(&buf[..len]);
}

// ── Main ────────────────────────────────────────────────────────────────────

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
