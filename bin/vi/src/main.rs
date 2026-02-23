#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::vec;
use anyos_std::format;

anyos_std::entry!(main);

// ─── Constants / Defaults ────────────────────────────────────────────────────

const DEFAULT_ROWS: usize = 24;
const DEFAULT_COLS: usize = 80;
const GUTTER_WIDTH: usize = 5; // "1234 " — 4 digits + space

fn get_term_size() -> (usize, usize) {
    let mut cols_buf = [0u8; 16];
    let mut rows_buf = [0u8; 16];
    let cols_len = anyos_std::env::get("COLUMNS", &mut cols_buf);
    let rows_len = anyos_std::env::get("LINES", &mut rows_buf);
    let cols = if cols_len != u32::MAX && cols_len > 0 {
        parse_usize(&cols_buf[..cols_len as usize]).unwrap_or(DEFAULT_COLS)
    } else {
        DEFAULT_COLS
    };
    let rows = if rows_len != u32::MAX && rows_len > 0 {
        parse_usize(&rows_buf[..rows_len as usize]).unwrap_or(DEFAULT_ROWS)
    } else {
        DEFAULT_ROWS
    };
    (cols.max(20), rows.max(5))
}

fn parse_usize(s: &[u8]) -> Option<usize> {
    let mut val: usize = 0;
    for &b in s {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as usize;
        } else {
            break;
        }
    }
    if val > 0 { Some(val) } else { None }
}

// ─── Key Representation ─────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Key {
    Char(u8),
    Ctrl(u8),
    Enter,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    None,
}

// ─── Editor Mode ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Insert,
    Command,
    Search,
    Replace, // single-char replace (r)
}

// ─── Undo Actions ───────────────────────────────────────────────────────────

#[derive(Clone)]
enum UndoAction {
    InsertChar { row: usize, col: usize, ch: u8 },
    DeleteChar { row: usize, col: usize, ch: u8 },
    InsertLine { row: usize, content: String },
    DeleteLine { row: usize, content: String },
    SplitLine { row: usize, col: usize },
    JoinLine { row: usize, trailing: String },
    ReplaceLine { row: usize, old: String },
    Batch { actions: Vec<UndoAction> },
}

// ─── Editor State ───────────────────────────────────────────────────────────

struct Editor {
    lines: Vec<String>,
    cx: usize,
    cy: usize,
    scroll_row: usize,
    mode: Mode,
    filename: Option<String>,
    modified: bool,
    running: bool,

    // Dynamic terminal dimensions
    screen_rows: usize,
    screen_cols: usize,
    edit_rows: usize, // screen_rows - 2 (status + cmd)

    cmd_buf: String,
    search_pattern: String,
    search_forward: bool,
    message: String,
    message_is_error: bool,

    undo_stack: Vec<UndoAction>,

    clipboard: Vec<String>,
    clipboard_line_mode: bool,

    count: usize,
    pending_op: Option<u8>, // 'd', 'y', 'c' for operator-pending
    last_insert_text: Vec<u8>,
    needs_redraw: bool,
}

impl Editor {
    fn new() -> Self {
        let (cols, rows) = get_term_size();
        Editor {
            lines: vec![String::new()],
            cx: 0,
            cy: 0,
            scroll_row: 0,
            mode: Mode::Normal,
            filename: None,
            modified: false,
            running: true,
            screen_rows: rows,
            screen_cols: cols,
            edit_rows: rows.saturating_sub(2),
            cmd_buf: String::new(),
            search_pattern: String::new(),
            search_forward: true,
            message: String::new(),
            message_is_error: false,
            undo_stack: Vec::new(),
            clipboard: Vec::new(),
            clipboard_line_mode: false,
            count: 0,
            pending_op: None,
            last_insert_text: Vec::new(),
            needs_redraw: true,
        }
    }

    // ─── Layout Helpers ──────────────────────────────────────────────────

    fn text_cols(&self) -> usize {
        self.screen_cols.saturating_sub(GUTTER_WIDTH)
    }

    fn status_row(&self) -> usize {
        self.screen_rows.saturating_sub(2)
    }

    fn cmd_row(&self) -> usize {
        self.screen_rows.saturating_sub(1)
    }

    // ─── File Operations ────────────────────────────────────────────────

    fn load_file(&mut self, path: &str) {
        match anyos_std::fs::read_to_string(path) {
            Ok(content) => {
                self.lines.clear();
                if content.is_empty() {
                    self.lines.push(String::new());
                } else {
                    for line in content.split('\n') {
                        self.lines.push(String::from(line));
                    }
                    // If the file ended with \n, the split produces a trailing empty
                    // string — that's the "line after the last newline". Remove it
                    // so we don't show a phantom blank line at the bottom.
                    if self.lines.len() > 1 && self.lines.last().map_or(false, |l| l.is_empty()) {
                        self.lines.pop();
                    }
                }
                self.filename = Some(String::from(path));
                self.modified = false;
                self.cx = 0;
                self.cy = 0;
                self.scroll_row = 0;
                self.set_message(&format!("\"{}\" {}L", path, self.lines.len()));
            }
            Err(_) => {
                // New file
                self.lines = vec![String::new()];
                self.filename = Some(String::from(path));
                self.modified = false;
                self.set_message(&format!("\"{}\" [New File]", path));
            }
        }
    }

    fn save_file(&mut self, path: Option<&str>) -> bool {
        let path = match path {
            Some(p) => {
                self.filename = Some(String::from(p));
                p
            }
            None => match &self.filename {
                Some(f) => f.as_str(),
                None => {
                    self.set_error("No file name");
                    return false;
                }
            },
        };

        let mut content = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            content.push_str(line);
            if i < self.lines.len() - 1 {
                content.push('\n');
            }
        }

        // Ensure trailing newline for POSIX compliance
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }

        // Need to clone path before mutable borrow
        let path_owned = String::from(path);
        match anyos_std::fs::write_bytes(&path_owned, content.as_bytes()) {
            Ok(_) => {
                self.modified = false;
                let n = self.lines.len();
                self.set_message(&format!("\"{}\" {}L written", path_owned, n));
                true
            }
            Err(_) => {
                self.set_error(&format!("Can't write \"{}\"", path_owned));
                false
            }
        }
    }

    // ─── Message Helpers ────────────────────────────────────────────────

    fn set_message(&mut self, msg: &str) {
        self.message = String::from(msg);
        self.message_is_error = false;
    }

    fn set_error(&mut self, msg: &str) {
        self.message = String::from(msg);
        self.message_is_error = true;
    }

    // ─── Cursor Helpers ─────────────────────────────────────────────────

    fn line_len(&self, row: usize) -> usize {
        if row < self.lines.len() {
            self.lines[row].len()
        } else {
            0
        }
    }

    fn clamp_cx(&mut self) {
        let max = if self.mode == Mode::Insert {
            self.line_len(self.cy)
        } else {
            let len = self.line_len(self.cy);
            if len > 0 { len - 1 } else { 0 }
        };
        if self.cx > max {
            self.cx = max;
        }
    }

    fn ensure_cursor_visible(&mut self) {
        if self.cy < self.scroll_row {
            self.scroll_row = self.cy;
        }
        if self.cy >= self.scroll_row + self.edit_rows {
            self.scroll_row = self.cy - self.edit_rows + 1;
        }
    }

    // ─── Undo ───────────────────────────────────────────────────────────

    fn push_undo(&mut self, action: UndoAction) {
        self.undo_stack.push(action);
        // Limit undo stack
        if self.undo_stack.len() > 1000 {
            self.undo_stack.remove(0);
        }
    }

    fn undo(&mut self) {
        let action = match self.undo_stack.pop() {
            Some(a) => a,
            None => {
                self.set_message("Already at oldest change");
                return;
            }
        };
        self.apply_undo(action);
    }

    fn apply_undo(&mut self, action: UndoAction) {
        match action {
            UndoAction::InsertChar { row, col, .. } => {
                // Undo an insert = delete
                if row < self.lines.len() && col < self.lines[row].len() {
                    self.lines[row].remove(col);
                }
                self.cy = row;
                self.cx = col;
            }
            UndoAction::DeleteChar { row, col, ch } => {
                // Undo a delete = re-insert
                if row < self.lines.len() {
                    if col <= self.lines[row].len() {
                        self.lines[row].insert(col, ch as char);
                    }
                }
                self.cy = row;
                self.cx = col;
            }
            UndoAction::InsertLine { row, .. } => {
                // Undo line insertion = delete it
                if row < self.lines.len() {
                    self.lines.remove(row);
                }
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.cy = if row > 0 { row - 1 } else { 0 };
                self.clamp_cx();
            }
            UndoAction::DeleteLine { row, content } => {
                // Undo line deletion = re-insert
                if row <= self.lines.len() {
                    self.lines.insert(row, content);
                }
                self.cy = row;
                self.cx = 0;
            }
            UndoAction::SplitLine { row, col } => {
                // Undo split = join
                if row + 1 < self.lines.len() {
                    let next = self.lines.remove(row + 1);
                    self.lines[row].push_str(&next);
                }
                self.cy = row;
                self.cx = col;
            }
            UndoAction::JoinLine { row, trailing } => {
                // Undo join = re-split
                if row < self.lines.len() {
                    let line = &self.lines[row];
                    let split_at = line.len() - trailing.len();
                    let rest = String::from(&self.lines[row][split_at..]);
                    self.lines[row].truncate(split_at);
                    self.lines.insert(row + 1, rest);
                }
                self.cy = row;
                self.clamp_cx();
            }
            UndoAction::ReplaceLine { row, old } => {
                if row < self.lines.len() {
                    self.lines[row] = old;
                }
                self.cy = row;
                self.clamp_cx();
            }
            UndoAction::Batch { actions } => {
                // Undo batch in reverse order
                for a in actions.into_iter().rev() {
                    self.apply_undo(a);
                }
            }
        }
        self.modified = true;
        self.needs_redraw = true;
    }

    // ─── Word Motion ────────────────────────────────────────────────────

    fn is_word_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    fn word_forward(&self) -> (usize, usize) {
        let mut row = self.cy;
        let mut col = self.cx;
        if row >= self.lines.len() { return (row, col); }
        let bytes = self.lines[row].as_bytes();

        // Skip current word chars
        while col < bytes.len() && Self::is_word_char(bytes[col]) {
            col += 1;
        }
        // Skip non-word chars (spaces, punctuation)
        loop {
            let bytes = self.lines[row].as_bytes();
            while col < bytes.len() && !Self::is_word_char(bytes[col]) {
                col += 1;
            }
            if col < bytes.len() || row + 1 >= self.lines.len() {
                break;
            }
            row += 1;
            col = 0;
        }
        (row, col)
    }

    fn word_backward(&self) -> (usize, usize) {
        let mut row = self.cy;
        let mut col = self.cx;

        // If at start of line, go to end of previous line
        if col == 0 {
            if row > 0 {
                row -= 1;
                col = self.lines[row].len();
                if col > 0 { col -= 1; }
            }
            return (row, col);
        }
        if col > 0 { col -= 1; }

        let bytes = self.lines[row].as_bytes();
        // Skip spaces/non-word backward
        while col > 0 && !Self::is_word_char(bytes[col]) {
            col -= 1;
        }
        // Skip word chars backward
        while col > 0 && Self::is_word_char(bytes[col - 1]) {
            col -= 1;
        }
        (row, col)
    }

    fn word_end(&self) -> (usize, usize) {
        let mut row = self.cy;
        let mut col = self.cx;
        if row >= self.lines.len() { return (row, col); }

        if col + 1 < self.lines[row].len() {
            col += 1;
        } else if row + 1 < self.lines.len() {
            row += 1;
            col = 0;
        }

        let bytes = self.lines[row].as_bytes();
        // Skip non-word chars
        while col < bytes.len() && !Self::is_word_char(bytes[col]) {
            col += 1;
        }
        // Skip word chars
        while col + 1 < bytes.len() && Self::is_word_char(bytes[col + 1]) {
            col += 1;
        }
        (row, col)
    }

    // ─── Search ─────────────────────────────────────────────────────────

    fn search_next(&mut self, forward: bool) {
        if self.search_pattern.is_empty() {
            self.set_error("No previous search pattern");
            return;
        }
        let pat = self.search_pattern.clone();
        let total = self.lines.len();
        if total == 0 { return; }

        let start_row = self.cy;
        let start_col = self.cx + 1; // start after cursor

        if forward {
            // Search forward from current position
            for i in 0..total {
                let row = (start_row + i) % total;
                let line = &self.lines[row];
                let from = if i == 0 { start_col } else { 0 };
                if from < line.len() {
                    if let Some(pos) = line[from..].find(pat.as_str()) {
                        self.cy = row;
                        self.cx = from + pos;
                        self.ensure_cursor_visible();
                        self.needs_redraw = true;
                        return;
                    }
                }
            }
        } else {
            // Search backward
            for i in 0..total {
                let row = (start_row + total - i) % total;
                let line = &self.lines[row];
                let search_in = if i == 0 && self.cx > 0 {
                    &line[..self.cx]
                } else {
                    line.as_str()
                };
                if let Some(pos) = search_in.rfind(pat.as_str()) {
                    self.cy = row;
                    self.cx = pos;
                    self.ensure_cursor_visible();
                    self.needs_redraw = true;
                    return;
                }
            }
        }
        self.set_error(&format!("Pattern not found: {}", pat));
    }

    // ─── Insert Operations ──────────────────────────────────────────────

    fn insert_char(&mut self, ch: u8) {
        if self.cy >= self.lines.len() {
            self.lines.push(String::new());
        }
        if self.cx > self.lines[self.cy].len() {
            self.cx = self.lines[self.cy].len();
        }
        self.lines[self.cy].insert(self.cx, ch as char);
        self.push_undo(UndoAction::InsertChar { row: self.cy, col: self.cx, ch });
        self.cx += 1;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn insert_newline(&mut self) {
        if self.cy >= self.lines.len() {
            self.lines.push(String::new());
        }
        let col = self.cx.min(self.lines[self.cy].len());
        let rest = String::from(&self.lines[self.cy][col..]);
        self.lines[self.cy].truncate(col);
        self.lines.insert(self.cy + 1, rest);
        self.push_undo(UndoAction::SplitLine { row: self.cy, col });
        self.cy += 1;
        self.cx = 0;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn backspace(&mut self) {
        if self.cx > 0 {
            self.cx -= 1;
            let ch = self.lines[self.cy].as_bytes()[self.cx];
            self.lines[self.cy].remove(self.cx);
            self.push_undo(UndoAction::DeleteChar { row: self.cy, col: self.cx, ch });
            self.modified = true;
            self.needs_redraw = true;
        } else if self.cy > 0 {
            // Join with previous line
            let current = self.lines.remove(self.cy);
            self.cy -= 1;
            self.cx = self.lines[self.cy].len();
            self.push_undo(UndoAction::JoinLine { row: self.cy, trailing: current.clone() });
            self.lines[self.cy].push_str(&current);
            self.modified = true;
            self.needs_redraw = true;
        }
    }

    fn delete_at_cursor(&mut self) {
        if self.cy < self.lines.len() && self.cx < self.lines[self.cy].len() {
            let ch = self.lines[self.cy].as_bytes()[self.cx];
            self.lines[self.cy].remove(self.cx);
            self.push_undo(UndoAction::DeleteChar { row: self.cy, col: self.cx, ch });
            self.modified = true;
            self.needs_redraw = true;
        }
    }

    // ─── Normal Mode Edit Operations ────────────────────────────────────

    fn delete_char_forward(&mut self) {
        if self.cy < self.lines.len() && self.cx < self.lines[self.cy].len() {
            let ch = self.lines[self.cy].as_bytes()[self.cx];
            self.lines[self.cy].remove(self.cx);
            self.push_undo(UndoAction::DeleteChar { row: self.cy, col: self.cx, ch });
            self.clamp_cx();
            self.modified = true;
            self.needs_redraw = true;
        }
    }

    fn delete_line(&mut self, count: usize) {
        let mut batch = Vec::new();
        let mut yanked = Vec::new();
        let n = count.min(self.lines.len().saturating_sub(self.cy));
        for _ in 0..n {
            if self.cy < self.lines.len() {
                let line = self.lines.remove(self.cy);
                batch.push(UndoAction::DeleteLine { row: self.cy, content: line.clone() });
                yanked.push(line);
            }
        }
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.cy >= self.lines.len() {
            self.cy = self.lines.len() - 1;
        }
        self.clamp_cx();
        self.push_undo(UndoAction::Batch { actions: batch });
        self.clipboard = yanked;
        self.clipboard_line_mode = true;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn delete_to_eol(&mut self) {
        if self.cy < self.lines.len() {
            let old = self.lines[self.cy].clone();
            self.lines[self.cy].truncate(self.cx);
            self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
            self.clamp_cx();
            self.modified = true;
            self.needs_redraw = true;
        }
    }

    fn delete_word(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let (wr, wc) = self.word_forward();
        if wr == self.cy {
            // Delete from cx to wc
            let old = self.lines[self.cy].clone();
            let yanked = String::from(&self.lines[self.cy][self.cx..wc]);
            let mut new_line = String::from(&self.lines[self.cy][..self.cx]);
            new_line.push_str(&self.lines[self.cy][wc..]);
            self.lines[self.cy] = new_line;
            self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
            self.clipboard = vec![yanked];
            self.clipboard_line_mode = false;
        } else {
            // Word spans multiple lines, just delete to end of current line
            self.delete_to_eol();
        }
        self.clamp_cx();
        self.modified = true;
        self.needs_redraw = true;
    }

    fn yank_line(&mut self, count: usize) {
        let n = count.min(self.lines.len().saturating_sub(self.cy));
        self.clipboard.clear();
        for i in 0..n {
            if self.cy + i < self.lines.len() {
                self.clipboard.push(self.lines[self.cy + i].clone());
            }
        }
        self.clipboard_line_mode = true;
        self.set_message(&format!("{} line(s) yanked", n));
    }

    fn yank_word(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let (wr, wc) = self.word_forward();
        if wr == self.cy && wc > self.cx {
            let yanked = String::from(&self.lines[self.cy][self.cx..wc]);
            self.clipboard = vec![yanked];
            self.clipboard_line_mode = false;
        }
    }

    fn paste_after(&mut self) {
        if self.clipboard.is_empty() { return; }
        if self.clipboard_line_mode {
            let row = self.cy + 1;
            let mut batch = Vec::new();
            for (i, line) in self.clipboard.clone().iter().enumerate() {
                self.lines.insert(row + i, line.clone());
                batch.push(UndoAction::InsertLine { row: row + i, content: line.clone() });
            }
            self.push_undo(UndoAction::Batch { actions: batch });
            self.cy = row;
            self.cx = 0;
        } else {
            // Inline paste after cursor
            let text = self.clipboard[0].clone();
            let old = self.lines[self.cy].clone();
            let insert_at = (self.cx + 1).min(self.lines[self.cy].len());
            self.lines[self.cy].insert_str(insert_at, &text);
            self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
            self.cx = insert_at + text.len().saturating_sub(1);
        }
        self.modified = true;
        self.needs_redraw = true;
    }

    fn paste_before(&mut self) {
        if self.clipboard.is_empty() { return; }
        if self.clipboard_line_mode {
            let row = self.cy;
            let mut batch = Vec::new();
            for (i, line) in self.clipboard.clone().iter().enumerate() {
                self.lines.insert(row + i, line.clone());
                batch.push(UndoAction::InsertLine { row: row + i, content: line.clone() });
            }
            self.push_undo(UndoAction::Batch { actions: batch });
            self.cy = row;
            self.cx = 0;
        } else {
            let text = self.clipboard[0].clone();
            let old = self.lines[self.cy].clone();
            let insert_at = self.cx.min(self.lines[self.cy].len());
            self.lines[self.cy].insert_str(insert_at, &text);
            self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
            self.cx = insert_at + text.len().saturating_sub(1);
        }
        self.modified = true;
        self.needs_redraw = true;
    }

    fn join_lines(&mut self) {
        if self.cy + 1 >= self.lines.len() { return; }
        let next = self.lines.remove(self.cy + 1);
        let join_col = self.lines[self.cy].len();
        // Add a space between if current doesn't end with space
        if !self.lines[self.cy].is_empty() && !next.is_empty() {
            let trimmed = next.trim_start();
            self.lines[self.cy].push(' ');
            self.push_undo(UndoAction::JoinLine { row: self.cy, trailing: format!(" {}", trimmed) });
            self.lines[self.cy].push_str(trimmed);
        } else {
            self.push_undo(UndoAction::JoinLine { row: self.cy, trailing: next.clone() });
            self.lines[self.cy].push_str(&next);
        }
        self.cx = join_col;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn toggle_case(&mut self) {
        if self.cy >= self.lines.len() { return; }
        if self.cx >= self.lines[self.cy].len() { return; }
        let old = self.lines[self.cy].clone();
        let b = self.lines[self.cy].as_bytes()[self.cx];
        let new_b = if b.is_ascii_lowercase() {
            b.to_ascii_uppercase()
        } else if b.is_ascii_uppercase() {
            b.to_ascii_lowercase()
        } else {
            b
        };
        // Safety: we're replacing a single ASCII byte with another ASCII byte
        unsafe {
            self.lines[self.cy].as_bytes_mut()[self.cx] = new_b;
        }
        self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
        self.cx += 1;
        self.clamp_cx();
        self.modified = true;
        self.needs_redraw = true;
    }

    fn replace_char(&mut self, ch: u8) {
        if self.cy >= self.lines.len() { return; }
        if self.cx >= self.lines[self.cy].len() { return; }
        let old = self.lines[self.cy].clone();
        unsafe {
            self.lines[self.cy].as_bytes_mut()[self.cx] = ch;
        }
        self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
        self.modified = true;
        self.needs_redraw = true;
    }

    fn open_line_below(&mut self) {
        let row = self.cy + 1;
        self.lines.insert(row, String::new());
        self.push_undo(UndoAction::InsertLine { row, content: String::new() });
        self.cy = row;
        self.cx = 0;
        self.mode = Mode::Insert;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn open_line_above(&mut self) {
        let row = self.cy;
        self.lines.insert(row, String::new());
        self.push_undo(UndoAction::InsertLine { row, content: String::new() });
        self.cy = row;
        self.cx = 0;
        self.mode = Mode::Insert;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn change_line(&mut self) {
        if self.cy >= self.lines.len() { return; }
        let old = self.lines[self.cy].clone();
        self.lines[self.cy] = String::new();
        self.push_undo(UndoAction::ReplaceLine { row: self.cy, old });
        self.cx = 0;
        self.mode = Mode::Insert;
        self.modified = true;
        self.needs_redraw = true;
    }

    fn change_to_eol(&mut self) {
        self.delete_to_eol();
        self.mode = Mode::Insert;
    }

    fn change_word(&mut self) {
        self.delete_word();
        self.mode = Mode::Insert;
    }

    // ─── Command Mode ───────────────────────────────────────────────────

    fn execute_command(&mut self) {
        let cmd = self.cmd_buf.clone();
        self.cmd_buf.clear();

        let cmd = cmd.trim();
        if cmd.is_empty() { return; }

        if cmd == "q" {
            if self.modified {
                self.set_error("No write since last change (add ! to override)");
            } else {
                self.running = false;
            }
        } else if cmd == "q!" {
            self.running = false;
        } else if cmd == "w" {
            self.save_file(None);
        } else if cmd.starts_with("w ") {
            let path = cmd[2..].trim();
            if !path.is_empty() {
                self.save_file(Some(path));
            }
        } else if cmd == "wq" || cmd == "x" {
            if self.save_file(None) {
                self.running = false;
            }
        } else if cmd.starts_with("e ") {
            let path = cmd[2..].trim();
            if !path.is_empty() {
                self.load_file(path);
                self.undo_stack.clear();
            }
        } else if cmd.starts_with('%') && cmd.contains("s/") {
            self.substitute_command(cmd);
        } else if let Ok(line_num) = cmd.parse::<usize>() {
            // Go to line
            if line_num > 0 && line_num <= self.lines.len() {
                self.cy = line_num - 1;
                self.cx = 0;
                self.ensure_cursor_visible();
                self.needs_redraw = true;
            } else {
                self.set_error("Invalid line number");
            }
        } else {
            self.set_error(&format!("Not a command: {}", cmd));
        }
    }

    fn substitute_command(&mut self, cmd: &str) {
        // Parse :%s/old/new/g
        let rest = if cmd.starts_with("%s/") {
            &cmd[3..]
        } else if cmd.starts_with("%s!") {
            // Support alternate delimiter
            &cmd[3..]
        } else {
            self.set_error("Invalid substitute command");
            return;
        };

        let delim = '/';
        let parts: Vec<&str> = rest.splitn(3, delim).collect();
        if parts.len() < 2 {
            self.set_error("Invalid substitute command");
            return;
        }

        let old_pat = parts[0];
        let new_pat = parts[1];
        let _flags = if parts.len() > 1 { parts.get(2).unwrap_or(&"") } else { &"" };

        if old_pat.is_empty() {
            self.set_error("Empty search pattern");
            return;
        }

        let mut total = 0u32;
        let mut batch = Vec::new();
        for i in 0..self.lines.len() {
            if self.lines[i].contains(old_pat) {
                let old = self.lines[i].clone();
                let new_line = self.lines[i].replace(old_pat, new_pat);
                // Count occurrences
                let count = old.matches(old_pat).count() as u32;
                total += count;
                self.lines[i] = new_line;
                batch.push(UndoAction::ReplaceLine { row: i, old });
            }
        }
        if !batch.is_empty() {
            self.push_undo(UndoAction::Batch { actions: batch });
            self.modified = true;
            self.needs_redraw = true;
        }
        self.set_message(&format!("{} substitution(s)", total));
    }

    // ─── Rendering ──────────────────────────────────────────────────────

    fn render(&mut self) {
        if !self.needs_redraw { return; }
        self.needs_redraw = false;

        // Render text lines using absolute cursor positioning (no \n to avoid scroll)
        for screen_row in 0..self.edit_rows {
            // Move to row (1-indexed), column 1
            anyos_std::print!("\x1B[{};1H", screen_row + 1);
            let file_row = self.scroll_row + screen_row;
            if file_row < self.lines.len() {
                // Line number (1-indexed)
                let num = file_row + 1;
                anyos_std::print!("\x1B[33m{:>4} \x1B[0m", num);

                // Line content (truncate to fit)
                let line = &self.lines[file_row];
                let display_len = line.len().min(self.text_cols());
                anyos_std::print!("{}", &line[..display_len]);
            } else {
                // Past end of file — show tilde
                anyos_std::print!("\x1B[34m~\x1B[0m");
            }
            // Clear rest of line
            anyos_std::print!("\x1B[K");
        }

        // Status bar (row 23, 1-indexed)
        anyos_std::print!("\x1B[{};1H", self.status_row() + 1);
        self.render_status_bar();

        // Command/message line (row 24, 1-indexed)
        anyos_std::print!("\x1B[{};1H", self.cmd_row() + 1);
        self.render_command_line();

        // Position cursor at edit position
        let screen_y = self.cy.saturating_sub(self.scroll_row) + 1; // 1-indexed
        let screen_x = GUTTER_WIDTH + self.cx + 1; // 1-indexed
        anyos_std::print!("\x1B[{};{}H", screen_y, screen_x);
    }

    fn render_status_bar(&self) {
        let fname = match &self.filename {
            Some(f) => f.as_str(),
            None => "[No Name]",
        };
        let modified_str = if self.modified { "[+]" } else { "" };
        let mode_str = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Search => "SEARCH",
            Mode::Replace => "REPLACE",
        };
        let pos = format!("Ln {}, Col {}", self.cy + 1, self.cx + 1);

        // Use bright white for status bar (no reverse video — terminal doesn't support bg colors)
        anyos_std::print!("\x1B[97m");

        let left = format!(" {} {} | {} ", fname, modified_str, mode_str);
        let right = format!(" {} ", pos);

        let pad = if self.screen_cols > left.len() + right.len() {
            self.screen_cols - left.len() - right.len()
        } else {
            0
        };

        anyos_std::print!("{}", left);
        for _ in 0..pad {
            anyos_std::print!("-");
        }
        anyos_std::print!("{}", right);
        anyos_std::print!("\x1B[0m\x1B[K");
    }

    fn render_command_line(&self) {
        match self.mode {
            Mode::Command => {
                anyos_std::print!(":{}", self.cmd_buf);
            }
            Mode::Search => {
                let ch = if self.search_forward { '/' } else { '?' };
                anyos_std::print!("{}{}", ch, self.cmd_buf);
            }
            _ => {
                if self.message_is_error {
                    anyos_std::print!("\x1B[31m{}\x1B[0m", self.message);
                } else {
                    anyos_std::print!("{}", self.message);
                }
            }
        }
        anyos_std::print!("\x1B[K");
    }

    // ─── Key Handling ───────────────────────────────────────────────────

    fn handle_key(&mut self, key: Key) {
        // Clear message on any keypress (unless in command/search mode)
        if self.mode != Mode::Command && self.mode != Mode::Search {
            if !self.message.is_empty() {
                self.message.clear();
                self.needs_redraw = true;
            }
        }

        match self.mode {
            Mode::Normal => self.handle_normal(key),
            Mode::Insert => self.handle_insert(key),
            Mode::Command => self.handle_command(key),
            Mode::Search => self.handle_search(key),
            Mode::Replace => self.handle_replace(key),
        }

        self.ensure_cursor_visible();
    }

    fn handle_normal(&mut self, key: Key) {
        match key {
            // ── Count prefix ──
            Key::Char(b'1'..=b'9') if self.pending_op.is_none() && self.count == 0 => {
                self.count = (key_byte(key) - b'0') as usize;
                return;
            }
            Key::Char(b'0'..=b'9') if self.count > 0 => {
                self.count = self.count * 10 + (key_byte(key) - b'0') as usize;
                return;
            }

            // ── Movement ──
            Key::Char(b'h') | Key::Left => {
                let n = self.get_count();
                for _ in 0..n {
                    if self.cx > 0 { self.cx -= 1; }
                }
                self.needs_redraw = true;
            }
            Key::Char(b'l') | Key::Right => {
                let n = self.get_count();
                for _ in 0..n {
                    let max = if self.line_len(self.cy) > 0 { self.line_len(self.cy) - 1 } else { 0 };
                    if self.cx < max { self.cx += 1; }
                }
                self.needs_redraw = true;
            }
            Key::Char(b'j') | Key::Down => {
                let n = self.get_count();
                for _ in 0..n {
                    if self.cy + 1 < self.lines.len() { self.cy += 1; }
                }
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Char(b'k') | Key::Up => {
                let n = self.get_count();
                for _ in 0..n {
                    if self.cy > 0 { self.cy -= 1; }
                }
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Char(b'w') => {
                if let Some(op) = self.pending_op {
                    match op {
                        b'd' => { self.delete_word(); }
                        b'c' => { self.change_word(); }
                        b'y' => { self.yank_word(); }
                        _ => {}
                    }
                    self.pending_op = None;
                    self.count = 0;
                } else {
                    let n = self.get_count();
                    for _ in 0..n {
                        let (r, c) = self.word_forward();
                        self.cy = r;
                        self.cx = c;
                    }
                    self.needs_redraw = true;
                }
            }
            Key::Char(b'b') => {
                let n = self.get_count();
                for _ in 0..n {
                    let (r, c) = self.word_backward();
                    self.cy = r;
                    self.cx = c;
                }
                self.needs_redraw = true;
            }
            Key::Char(b'e') => {
                let n = self.get_count();
                for _ in 0..n {
                    let (r, c) = self.word_end();
                    self.cy = r;
                    self.cx = c;
                }
                self.needs_redraw = true;
            }
            Key::Char(b'0') | Key::Home => {
                self.cx = 0;
                self.needs_redraw = true;
            }
            Key::Char(b'$') | Key::End => {
                let len = self.line_len(self.cy);
                self.cx = if len > 0 { len - 1 } else { 0 };
                self.needs_redraw = true;
            }
            Key::Char(b'^') => {
                // First non-blank
                if self.cy < self.lines.len() {
                    let bytes = self.lines[self.cy].as_bytes();
                    self.cx = 0;
                    while self.cx < bytes.len() && (bytes[self.cx] == b' ' || bytes[self.cx] == b'\t') {
                        self.cx += 1;
                    }
                }
                self.needs_redraw = true;
            }
            Key::Char(b'G') => {
                let n = self.count;
                self.count = 0;
                if n > 0 {
                    self.cy = (n - 1).min(self.lines.len().saturating_sub(1));
                } else {
                    self.cy = self.lines.len().saturating_sub(1);
                }
                self.cx = 0;
                self.needs_redraw = true;
            }
            Key::Char(b'g') => {
                if self.pending_op == Some(b'g') {
                    // gg = go to top
                    self.pending_op = None;
                    let n = self.count;
                    self.count = 0;
                    if n > 0 {
                        self.cy = (n - 1).min(self.lines.len().saturating_sub(1));
                    } else {
                        self.cy = 0;
                    }
                    self.cx = 0;
                    self.needs_redraw = true;
                } else {
                    self.pending_op = Some(b'g');
                    return;
                }
            }
            Key::Char(b'H') => {
                self.cy = self.scroll_row;
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Char(b'M') => {
                let mid = self.edit_rows / 2;
                self.cy = (self.scroll_row + mid).min(self.lines.len().saturating_sub(1));
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Char(b'L') => {
                let bottom = self.scroll_row + self.edit_rows - 1;
                self.cy = bottom.min(self.lines.len().saturating_sub(1));
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Ctrl(b'f') | Key::PageDown => {
                let n = self.get_count().max(1);
                for _ in 0..n {
                    self.cy = (self.cy + self.edit_rows).min(self.lines.len().saturating_sub(1));
                    self.scroll_row = (self.scroll_row + self.edit_rows)
                        .min(self.lines.len().saturating_sub(self.edit_rows));
                }
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Ctrl(b'b') | Key::PageUp => {
                let n = self.get_count().max(1);
                for _ in 0..n {
                    self.cy = self.cy.saturating_sub(self.edit_rows);
                    self.scroll_row = self.scroll_row.saturating_sub(self.edit_rows);
                }
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Ctrl(b'd') => {
                let half = self.edit_rows / 2;
                self.cy = (self.cy + half).min(self.lines.len().saturating_sub(1));
                self.clamp_cx();
                self.needs_redraw = true;
            }
            Key::Ctrl(b'u') => {
                let half = self.edit_rows / 2;
                self.cy = self.cy.saturating_sub(half);
                self.clamp_cx();
                self.needs_redraw = true;
            }

            // ── Insert mode entry ──
            Key::Char(b'i') => {
                self.mode = Mode::Insert;
                self.needs_redraw = true;
            }
            Key::Char(b'I') => {
                // Insert at first non-blank
                if self.cy < self.lines.len() {
                    let bytes = self.lines[self.cy].as_bytes();
                    self.cx = 0;
                    while self.cx < bytes.len() && (bytes[self.cx] == b' ' || bytes[self.cx] == b'\t') {
                        self.cx += 1;
                    }
                }
                self.mode = Mode::Insert;
                self.needs_redraw = true;
            }
            Key::Char(b'a') => {
                if self.cx < self.line_len(self.cy) {
                    self.cx += 1;
                }
                self.mode = Mode::Insert;
                self.needs_redraw = true;
            }
            Key::Char(b'A') => {
                self.cx = self.line_len(self.cy);
                self.mode = Mode::Insert;
                self.needs_redraw = true;
            }
            Key::Char(b'o') => {
                self.open_line_below();
            }
            Key::Char(b'O') => {
                self.open_line_above();
            }

            // ── Editing ──
            Key::Char(b'x') => {
                let n = self.get_count();
                for _ in 0..n {
                    self.delete_char_forward();
                }
            }
            Key::Char(b'd') => {
                if self.pending_op == Some(b'd') {
                    // dd = delete line
                    self.pending_op = None;
                    let n = self.get_count().max(1);
                    self.delete_line(n);
                } else {
                    self.pending_op = Some(b'd');
                    return;
                }
            }
            Key::Char(b'D') => {
                self.delete_to_eol();
                self.count = 0;
            }
            Key::Char(b'y') => {
                if self.pending_op == Some(b'y') {
                    // yy = yank line
                    self.pending_op = None;
                    let n = self.get_count().max(1);
                    self.yank_line(n);
                } else {
                    self.pending_op = Some(b'y');
                    return;
                }
            }
            Key::Char(b'c') => {
                if self.pending_op == Some(b'c') {
                    // cc = change line
                    self.pending_op = None;
                    self.change_line();
                } else {
                    self.pending_op = Some(b'c');
                    return;
                }
            }
            Key::Char(b'C') => {
                self.count = 0;
                self.change_to_eol();
            }
            Key::Char(b'p') => {
                self.paste_after();
                self.count = 0;
            }
            Key::Char(b'P') => {
                self.paste_before();
                self.count = 0;
            }
            Key::Char(b'u') => {
                self.undo();
                self.count = 0;
            }
            Key::Char(b'J') => {
                self.join_lines();
                self.count = 0;
            }
            Key::Char(b'r') => {
                self.mode = Mode::Replace;
                return;
            }
            Key::Char(b'~') => {
                self.toggle_case();
                self.count = 0;
            }

            // ── Search ──
            Key::Char(b'/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.cmd_buf.clear();
                self.needs_redraw = true;
            }
            Key::Char(b'?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.cmd_buf.clear();
                self.needs_redraw = true;
            }
            Key::Char(b'n') => {
                self.search_next(self.search_forward);
            }
            Key::Char(b'N') => {
                self.search_next(!self.search_forward);
            }

            // ── Command mode ──
            Key::Char(b':') => {
                self.mode = Mode::Command;
                self.cmd_buf.clear();
                self.needs_redraw = true;
            }

            // ── ZZ / ZQ ──
            Key::Char(b'Z') => {
                if self.pending_op == Some(b'Z') {
                    // ZZ = save and quit
                    self.pending_op = None;
                    if self.modified {
                        if self.save_file(None) {
                            self.running = false;
                        }
                    } else {
                        self.running = false;
                    }
                } else if self.pending_op == Some(b'Q') {
                    // We don't get here via normal flow; see ZQ below
                    self.pending_op = None;
                } else {
                    self.pending_op = Some(b'Z');
                    return;
                }
            }
            Key::Char(b'Q') if self.pending_op == Some(b'Z') => {
                // ZQ = quit without saving
                self.pending_op = None;
                self.running = false;
            }

            Key::Escape => {
                self.pending_op = None;
                self.count = 0;
            }

            _ => {
                // Unknown key — clear pending
                self.pending_op = None;
                self.count = 0;
            }
        }

        // Clear pending operator if we processed a complete command
        // (operators that need to persist handle this by returning early)
        if self.pending_op.is_some() && !matches!(key, Key::Char(b'g')) {
            // Already handled or invalid
        }
    }

    fn handle_insert(&mut self, key: Key) {
        match key {
            Key::Escape => {
                // Move cursor back one (standard vi behavior)
                if self.cx > 0 { self.cx -= 1; }
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
            Key::Enter => {
                self.insert_newline();
            }
            Key::Backspace => {
                self.backspace();
            }
            Key::Delete => {
                self.delete_at_cursor();
            }
            Key::Up => {
                if self.cy > 0 { self.cy -= 1; self.clamp_cx(); }
                self.needs_redraw = true;
            }
            Key::Down => {
                if self.cy + 1 < self.lines.len() { self.cy += 1; self.clamp_cx(); }
                self.needs_redraw = true;
            }
            Key::Left => {
                if self.cx > 0 { self.cx -= 1; }
                self.needs_redraw = true;
            }
            Key::Right => {
                if self.cx < self.line_len(self.cy) { self.cx += 1; }
                self.needs_redraw = true;
            }
            Key::Home => {
                self.cx = 0;
                self.needs_redraw = true;
            }
            Key::End => {
                self.cx = self.line_len(self.cy);
                self.needs_redraw = true;
            }
            Key::Char(ch) => {
                if ch >= 0x20 && ch < 0x7f {
                    self.insert_char(ch);
                }
            }
            Key::Ctrl(b'h') => {
                // Ctrl+H = backspace alias
                self.backspace();
            }
            _ => {}
        }
    }

    fn handle_command(&mut self, key: Key) {
        match key {
            Key::Escape => {
                self.cmd_buf.clear();
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
            Key::Enter => {
                self.mode = Mode::Normal;
                self.execute_command();
                self.needs_redraw = true;
            }
            Key::Backspace => {
                if self.cmd_buf.is_empty() {
                    self.mode = Mode::Normal;
                } else {
                    self.cmd_buf.pop();
                }
                self.needs_redraw = true;
            }
            Key::Char(ch) => {
                if ch >= 0x20 && ch < 0x7f {
                    self.cmd_buf.push(ch as char);
                    self.needs_redraw = true;
                }
            }
            _ => {}
        }
    }

    fn handle_search(&mut self, key: Key) {
        match key {
            Key::Escape => {
                self.cmd_buf.clear();
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
            Key::Enter => {
                self.search_pattern = self.cmd_buf.clone();
                self.cmd_buf.clear();
                self.mode = Mode::Normal;
                self.search_next(self.search_forward);
                self.needs_redraw = true;
            }
            Key::Backspace => {
                if self.cmd_buf.is_empty() {
                    self.mode = Mode::Normal;
                } else {
                    self.cmd_buf.pop();
                }
                self.needs_redraw = true;
            }
            Key::Char(ch) => {
                if ch >= 0x20 && ch < 0x7f {
                    self.cmd_buf.push(ch as char);
                    self.needs_redraw = true;
                }
            }
            _ => {}
        }
    }

    fn handle_replace(&mut self, key: Key) {
        match key {
            Key::Escape => {
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
            Key::Char(ch) => {
                if ch >= 0x20 && ch < 0x7f {
                    self.replace_char(ch);
                }
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
            _ => {
                self.mode = Mode::Normal;
                self.needs_redraw = true;
            }
        }
    }

    fn get_count(&mut self) -> usize {
        let c = if self.count == 0 { 1 } else { self.count };
        self.count = 0;
        c
    }
}

// ─── Helper to extract byte from Key::Char ──────────────────────────────────

fn key_byte(key: Key) -> u8 {
    match key {
        Key::Char(b) => b,
        _ => 0,
    }
}

// ─── Input Reader ───────────────────────────────────────────────────────────

struct InputReader {
    buf: [u8; 32],
    len: usize,
    pos: usize,
}

impl InputReader {
    fn new() -> Self {
        InputReader {
            buf: [0u8; 32],
            len: 0,
            pos: 0,
        }
    }

    fn read_key(&mut self) -> Key {
        // Refill buffer if exhausted
        if self.pos >= self.len {
            let n = anyos_std::fs::read(0, &mut self.buf);
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
                if self.pos < self.len {
                    let next = self.buf[self.pos];
                    if next == b'[' {
                        self.pos += 1;
                        return self.parse_csi();
                    }
                    // Bare escape
                    return Key::Escape;
                }
                // Try to read more bytes for escape sequence
                let mut esc_buf = [0u8; 8];
                let n = anyos_std::fs::read(0, &mut esc_buf);
                if n > 0 && n != u32::MAX && esc_buf[0] == b'[' {
                    // Store remaining bytes
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
            0x7f => Key::Backspace,
            0x08 => Key::Backspace,
            0x09 => Key::Char(b'\t'),
            1..=26 => {
                // Ctrl+A through Ctrl+Z
                Key::Ctrl(b + b'a' - 1)
            }
            0x20..=0x7e => Key::Char(b),
            _ => Key::None,
        }
    }

    fn parse_csi(&mut self) -> Key {
        // Read the CSI parameters
        let mut param: u32 = 0;
        let mut has_param = false;

        loop {
            let b = if self.pos < self.len {
                let c = self.buf[self.pos];
                self.pos += 1;
                c
            } else {
                // Try reading more
                let mut tmp = [0u8; 4];
                let n = anyos_std::fs::read(0, &mut tmp);
                if n > 0 && n != u32::MAX {
                    let c = tmp[0];
                    // Store remaining
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
                    return Key::Escape;
                }
            };

            match b {
                b'A' => return Key::Up,
                b'B' => return Key::Down,
                b'C' => return Key::Right,
                b'D' => return Key::Left,
                b'H' => return Key::Home,
                b'F' => return Key::End,
                b'5' => {
                    // Page Up: \x1b[5~
                    self.consume_tilde();
                    return Key::PageUp;
                }
                b'6' => {
                    // Page Down: \x1b[6~
                    self.consume_tilde();
                    return Key::PageDown;
                }
                b'3' => {
                    // Delete: \x1b[3~
                    self.consume_tilde();
                    return Key::Delete;
                }
                b'~' => {
                    // End of parameterized sequence
                    return Key::None;
                }
                b'0'..=b'9' => {
                    param = param * 10 + (b - b'0') as u32;
                    has_param = true;
                }
                b';' => {
                    // Multi-param — consume rest until letter
                    param = 0;
                    has_param = false;
                }
                _ if b >= 0x40 => {
                    // Final byte of unknown sequence
                    return Key::None;
                }
                _ => {
                    // Keep reading
                }
            }
        }
    }

    fn consume_tilde(&mut self) {
        if self.pos < self.len && self.buf[self.pos] == b'~' {
            self.pos += 1;
        } else {
            let mut tmp = [0u8; 1];
            anyos_std::fs::read(0, &mut tmp);
        }
    }
}

// ─── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let mut editor = Editor::new();
    let mut input = InputReader::new();

    // Parse arguments
    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);

    // Open file if argument provided
    let arg = args_str.trim();
    if !arg.is_empty() {
        editor.load_file(arg);
    } else {
        editor.set_message("vi - type :q to quit, i to insert");
    }

    // Clear screen
    anyos_std::print!("\x1B[2J");

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

    // Cleanup: clear screen and move cursor home
    anyos_std::print!("\x1B[2J\x1B[H");
}
