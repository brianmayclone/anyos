//! TextEditor — code editor control with syntax highlighting, line numbers,
//! auto-indent, and smooth scrolling.

use alloc::vec;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, EventResponse};

// ── Selection ────────────────────────────────────────────────────────

struct Selection {
    start_row: usize,
    start_col: usize,
    end_row: usize,
    end_col: usize,
}

impl Selection {
    /// Return (start_row, start_col, end_row, end_col) in reading order.
    fn ordered(&self) -> (usize, usize, usize, usize) {
        if self.start_row < self.end_row
            || (self.start_row == self.end_row && self.start_col <= self.end_col)
        {
            (self.start_row, self.start_col, self.end_row, self.end_col)
        } else {
            (self.end_row, self.end_col, self.start_row, self.start_col)
        }
    }

    fn is_empty(&self) -> bool {
        self.start_row == self.end_row && self.start_col == self.end_col
    }
}

// ── Color span for syntax-highlighted text ───────────────────────────

struct ColorSpan {
    start: usize,
    end: usize,
    color: u32,
}

// ── Syntax definition ────────────────────────────────────────────────

pub(crate) struct SyntaxDef {
    keywords: Vec<Vec<u8>>,
    types: Vec<Vec<u8>>,
    builtins: Vec<Vec<u8>>,
    line_comment: Vec<u8>,
    block_comment_start: Vec<u8>,
    block_comment_end: Vec<u8>,
    string_delimiters: Vec<u8>,
    char_delimiter: u8,
    keyword_color: u32,
    type_color: u32,
    builtin_color: u32,
    string_color: u32,
    comment_color: u32,
    number_color: u32,
    operator_color: u32,
}

impl SyntaxDef {
    pub fn parse(data: &[u8]) -> Option<SyntaxDef> {
        let mut syn = SyntaxDef {
            keywords: Vec::new(),
            types: Vec::new(),
            builtins: Vec::new(),
            line_comment: Vec::new(),
            block_comment_start: Vec::new(),
            block_comment_end: Vec::new(),
            string_delimiters: Vec::new(),
            char_delimiter: b'\'',
            keyword_color: 0xFFFF6B6B,
            type_color: 0xFF4ECDC4,
            builtin_color: 0xFFDCDCAA,
            string_color: 0xFFE2B93D,
            comment_color: 0xFF6A737D,
            number_color: 0xFF9B59B6,
            operator_color: 0xFF56B6C2,
        };

        // Split data into lines
        let mut start = 0;
        let len = data.len();
        while start <= len {
            let end = {
                let mut e = start;
                while e < len && data[e] != b'\n' {
                    e += 1;
                }
                e
            };
            let line = &data[start..end];
            // Find the '=' separator
            if let Some(eq_pos) = line.iter().position(|&b| b == b'=') {
                let key = &line[..eq_pos];
                let val = &line[eq_pos + 1..];
                if key == b"keywords" {
                    syn.keywords = split_csv(val);
                } else if key == b"types" {
                    syn.types = split_csv(val);
                } else if key == b"builtins" {
                    syn.builtins = split_csv(val);
                } else if key == b"line_comment" {
                    syn.line_comment = val.to_vec();
                } else if key == b"block_comment_start" {
                    syn.block_comment_start = val.to_vec();
                } else if key == b"block_comment_end" {
                    syn.block_comment_end = val.to_vec();
                } else if key == b"string_delimiters" {
                    syn.string_delimiters = val.to_vec();
                } else if key == b"char_delimiter" {
                    if !val.is_empty() {
                        syn.char_delimiter = val[0];
                    }
                } else if key == b"keyword_color" {
                    if let Some(c) = parse_hex_color(val) { syn.keyword_color = c; }
                } else if key == b"type_color" {
                    if let Some(c) = parse_hex_color(val) { syn.type_color = c; }
                } else if key == b"builtin_color" {
                    if let Some(c) = parse_hex_color(val) { syn.builtin_color = c; }
                } else if key == b"string_color" {
                    if let Some(c) = parse_hex_color(val) { syn.string_color = c; }
                } else if key == b"comment_color" {
                    if let Some(c) = parse_hex_color(val) { syn.comment_color = c; }
                } else if key == b"number_color" {
                    if let Some(c) = parse_hex_color(val) { syn.number_color = c; }
                } else if key == b"operator_color" {
                    if let Some(c) = parse_hex_color(val) { syn.operator_color = c; }
                }
            }
            start = end + 1;
        }

        Some(syn)
    }
}

// ── TextEditor ───────────────────────────────────────────────────────

pub struct TextEditor {
    pub(crate) base: ControlBase,
    lines: Vec<Vec<u8>>,
    pub(crate) cursor_row: usize,
    pub(crate) cursor_col: usize,
    scroll_y: i32,
    scroll_x: i32,
    focused: bool,
    selection: Option<Selection>,
    syntax: Option<SyntaxDef>,
    pub(crate) show_line_numbers: bool,
    gutter_width: u32,
    pub(crate) line_height: u32,
    pub(crate) tab_width: u32,
    pub(crate) font_id: u16,
    pub(crate) font_size: u16,
    pub(crate) char_width: u32,
}

impl TextEditor {
    pub fn new(base: ControlBase) -> Self {
        let (cw, _) = crate::draw::measure_text_ex(b"M", 4, 13);
        let char_width = if cw > 0 { cw } else { 8 };
        Self {
            base,
            lines: vec![Vec::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_y: 0,
            scroll_x: 0,
            focused: false,
            selection: None,
            syntax: None,
            show_line_numbers: true,
            gutter_width: 40,
            line_height: 20,
            tab_width: 4,
            font_id: 4,
            font_size: 13,
            char_width,
        }
    }

    pub fn set_text(&mut self, text: &[u8]) {
        self.lines.clear();
        let mut line = Vec::new();
        for &b in text {
            if b == b'\n' {
                self.lines.push(line);
                line = Vec::new();
            } else if b != b'\r' {
                line.push(b);
            }
        }
        self.lines.push(line);
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_y = 0;
        self.scroll_x = 0;
        self.selection = None;
        self.update_gutter_width();
        self.base.mark_dirty();
    }

    pub fn get_text(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push(b'\n');
            }
            out.extend_from_slice(line);
        }
        out
    }

    pub fn set_syntax(&mut self, data: &[u8]) {
        crate::log!("[SYNTAX-SERVER] set_syntax called with {} bytes", data.len());
        self.syntax = SyntaxDef::parse(data);
        if let Some(ref syn) = self.syntax {
            crate::log!("[SYNTAX-SERVER] parsed OK: {} keywords, {} types, {} builtins",
                syn.keywords.len(), syn.types.len(), syn.builtins.len());
        } else {
            crate::log!("[SYNTAX-SERVER] parse returned None");
        }
        self.base.mark_dirty();
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.lines.len().saturating_sub(1));
        self.cursor_col = col.min(self.lines[self.cursor_row].len());
        self.ensure_cursor_visible();
        self.base.mark_dirty();
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn insert_text_at_cursor(&mut self, text: &[u8]) {
        for &b in text {
            if b == b'\n' {
                let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
                self.cursor_row += 1;
                self.lines.insert(self.cursor_row, rest);
                self.cursor_col = 0;
            } else {
                self.lines[self.cursor_row].insert(self.cursor_col, b);
                self.cursor_col += 1;
            }
        }
        self.update_gutter_width();
        self.ensure_cursor_visible();
        self.base.mark_dirty();
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn update_gutter_width(&mut self) {
        if !self.show_line_numbers {
            self.gutter_width = 0;
            return;
        }
        let digits = if self.lines.len() < 10 {
            1
        } else if self.lines.len() < 100 {
            2
        } else if self.lines.len() < 1000 {
            3
        } else if self.lines.len() < 10000 {
            4
        } else {
            5
        };
        self.gutter_width = (digits + 1) as u32 * self.char_width + 8;
    }

    fn ensure_cursor_visible(&mut self) {
        let cursor_y = (self.cursor_row as i32) * self.line_height as i32;
        let visible_h = self.base.h as i32 - 2;
        if cursor_y < self.scroll_y {
            self.scroll_y = cursor_y;
        }
        if cursor_y + self.line_height as i32 > self.scroll_y + visible_h {
            self.scroll_y = cursor_y + self.line_height as i32 - visible_h;
        }
        let cursor_x = (self.cursor_col as i32) * self.char_width as i32;
        let text_area_w = self.base.w as i32 - self.gutter_width as i32 - 10;
        if cursor_x < self.scroll_x {
            self.scroll_x = cursor_x;
        }
        if cursor_x + self.char_width as i32 > self.scroll_x + text_area_w {
            self.scroll_x = cursor_x + self.char_width as i32 - text_area_w;
        }
        self.scroll_y = self.scroll_y.max(0);
        self.scroll_x = self.scroll_x.max(0);
    }

    fn content_height(&self) -> i32 {
        (self.lines.len() as i32) * self.line_height as i32
    }

    pub fn clamp_cursor(&mut self) {
        if self.cursor_row >= self.lines.len() {
            self.cursor_row = self.lines.len().saturating_sub(1);
        }
        if self.cursor_col > self.lines[self.cursor_row].len() {
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    /// Convert local pixel coordinates to (row, col) in the buffer.
    fn pixel_to_cursor(&self, lx: i32, ly: i32) -> (usize, usize) {
        let row = ((ly - 1 + self.scroll_y) / self.line_height as i32).max(0) as usize;
        let row = row.min(self.lines.len().saturating_sub(1));
        let text_lx = lx - self.gutter_width as i32 - 1 + self.scroll_x;
        let col = (text_lx / self.char_width as i32).max(0) as usize;
        let col = col.min(self.lines[row].len());
        (row, col)
    }

    /// Extract selected text as bytes. Returns None if no selection.
    pub fn extract_selected_text(&self) -> Option<Vec<u8>> {
        let sel = self.selection.as_ref()?;
        if sel.is_empty() {
            return None;
        }
        let (sr, sc, er, ec) = sel.ordered();
        let mut out = Vec::new();
        for row in sr..=er {
            if row >= self.lines.len() {
                break;
            }
            let line = &self.lines[row];
            let c0 = if row == sr { sc.min(line.len()) } else { 0 };
            let c1 = if row == er { ec.min(line.len()) } else { line.len() };
            if c0 <= c1 {
                out.extend_from_slice(&line[c0..c1]);
            }
            if row < er {
                out.push(b'\n');
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// Select all text.
    pub fn select_all(&mut self) {
        let last_row = self.lines.len().saturating_sub(1);
        let last_col = self.lines[last_row].len();
        self.selection = Some(Selection {
            start_row: 0,
            start_col: 0,
            end_row: last_row,
            end_col: last_col,
        });
        self.cursor_row = last_row;
        self.cursor_col = last_col;
    }

    /// Delete the selected text and place cursor at the start of the selection.
    pub fn delete_selection(&mut self) -> bool {
        let sel = match self.selection.take() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };
        let (sr, sc, er, ec) = sel.ordered();
        if sr == er {
            // Single line deletion
            if sr < self.lines.len() {
                let end = ec.min(self.lines[sr].len());
                let start = sc.min(end);
                self.lines[sr].drain(start..end);
            }
        } else {
            // Multi-line deletion
            let end_col = ec.min(self.lines.get(er).map_or(0, |l| l.len()));
            // Keep tail of last line
            let tail: Vec<u8> = if er < self.lines.len() {
                self.lines[er][end_col..].to_vec()
            } else {
                Vec::new()
            };
            // Remove lines from sr+1 to er (inclusive)
            let remove_end = (er + 1).min(self.lines.len());
            if sr + 1 <= remove_end {
                self.lines.drain(sr + 1..remove_end);
            }
            // Truncate first line at sc and append tail
            if sr < self.lines.len() {
                let start = sc.min(self.lines[sr].len());
                self.lines[sr].truncate(start);
                self.lines[sr].extend_from_slice(&tail);
            }
        }
        self.cursor_row = sr.min(self.lines.len().saturating_sub(1));
        self.cursor_col = sc.min(self.lines[self.cursor_row].len());
        self.update_gutter_width();
        self.ensure_cursor_visible();
        self.base.mark_dirty();
        true
    }
}

// ── Control trait ────────────────────────────────────────────────────

impl Control for TextEditor {
    fn base(&self) -> &ControlBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }

    fn kind(&self) -> ControlKind {
        ControlKind::TextEditor
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn accepts_focus(&self) -> bool {
        true
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let w = self.base.w;
        let h = self.base.h;
        let tc = crate::theme::colors();

        // Background
        crate::draw::fill_rect(surface, x, y, w, h, tc.editor_bg);

        // Clipped surface for content
        let clipped = surface.with_clip(x + 1, y + 1, w.saturating_sub(2), h.saturating_sub(2));

        let visible_start = (self.scroll_y / self.line_height as i32).max(0) as usize;
        let visible_end = ((self.scroll_y + h as i32) / self.line_height as i32 + 1)
            .min(self.lines.len() as i32) as usize;

        let text_x_base = x + 1 + self.gutter_width as i32;

        // Track block comment state: pre-scan lines before visible_start
        let mut in_block_comment = false;
        if self.syntax.is_some() {
            for i in 0..visible_start {
                if let Some(ref syn) = self.syntax {
                    let (_, still_in) = tokenize_line(&self.lines[i], in_block_comment, syn);
                    in_block_comment = still_in;
                }
            }
        }

        for row in visible_start..visible_end {
            let row_y = y + 1 + (row as i32) * self.line_height as i32 - self.scroll_y;

            // Current line highlight
            if row == self.cursor_row && self.focused {
                crate::draw::fill_rect(
                    &clipped,
                    x + 1 + self.gutter_width as i32,
                    row_y,
                    w.saturating_sub(2).saturating_sub(self.gutter_width),
                    self.line_height,
                    tc.editor_line_hl,
                );
            }

            // Selection highlight
            if let Some(ref sel) = self.selection {
                if !sel.is_empty() {
                    let (sr, sc, er, ec) = sel.ordered();
                    if row >= sr && row <= er {
                        let line_len = self.lines[row].len();
                        let sel_start = if row == sr { sc.min(line_len) } else { 0 };
                        let sel_end = if row == er { ec.min(line_len) } else { line_len };
                        if sel_start < sel_end || (row > sr && row < er) {
                            let sx = text_x_base + (sel_start as i32) * self.char_width as i32 - self.scroll_x;
                            let sel_chars = if sel_end > sel_start { sel_end - sel_start } else { 0 };
                            // For middle lines of multiline selection, extend to edge
                            let sw = if row > sr && row < er && sel_chars == 0 {
                                w.saturating_sub(self.gutter_width).saturating_sub(2)
                            } else {
                                (sel_chars as u32) * self.char_width
                            };
                            if sw > 0 {
                                crate::draw::fill_rect(
                                    &clipped,
                                    sx,
                                    row_y,
                                    sw,
                                    self.line_height,
                                    tc.editor_selection,
                                );
                            }
                        }
                    }
                }
            }

            // Line number (gutter)
            if self.show_line_numbers {
                let mut num_buf = [0u8; 8];
                let num_len = format_line_number(row + 1, &mut num_buf);
                let num_text = &num_buf[..num_len];
                let (nw, _) = crate::draw::measure_text_ex(num_text, self.font_id, self.font_size);
                let gutter_text_x = x + 1 + self.gutter_width as i32 - nw as i32 - 8;
                let line_num_color = if row == self.cursor_row {
                    tc.text_secondary
                } else {
                    tc.text_disabled
                };
                crate::draw::draw_text_ex(
                    &clipped,
                    gutter_text_x,
                    row_y + 2,
                    line_num_color,
                    num_text,
                    self.font_id,
                    self.font_size,
                );
            }

            // Text content
            let line = &self.lines[row];
            if !line.is_empty() {
                if let Some(ref syn) = self.syntax {
                    let (spans, still_in) = tokenize_line(line, in_block_comment, syn);
                    in_block_comment = still_in;
                    for span in &spans {
                        let text_slice = &line[span.start..span.end];
                        let span_x = text_x_base + (span.start as i32) * self.char_width as i32
                            - self.scroll_x;
                        crate::draw::draw_text_ex(
                            &clipped,
                            span_x,
                            row_y + 2,
                            span.color,
                            text_slice,
                            self.font_id,
                            self.font_size,
                        );
                    }
                } else {
                    let text_x = text_x_base - self.scroll_x;
                    crate::draw::draw_text_ex(
                        &clipped,
                        text_x,
                        row_y + 2,
                        tc.text,
                        line,
                        self.font_id,
                        self.font_size,
                    );
                }
            } else if let Some(ref syn) = self.syntax {
                let (_, still_in) = tokenize_line(line, in_block_comment, syn);
                in_block_comment = still_in;
            }

            // Cursor
            if row == self.cursor_row && self.focused {
                let cursor_x = text_x_base + (self.cursor_col as i32) * self.char_width as i32
                    - self.scroll_x;
                crate::draw::fill_rect(
                    &clipped,
                    cursor_x,
                    row_y + 1,
                    2,
                    self.line_height.saturating_sub(2),
                    tc.accent,
                );
            }
        }

        // Gutter separator
        if self.show_line_numbers && self.gutter_width > 0 {
            crate::draw::fill_rect(
                &clipped,
                x + self.gutter_width as i32,
                y + 1,
                1,
                h.saturating_sub(2),
                tc.separator,
            );
        }

        // Border
        let border_color = if self.focused { tc.input_focus } else { tc.input_border };
        crate::draw::draw_border(surface, x, y, w, h, border_color);

        // Vertical scrollbar
        let content_h = self.content_height();
        let visible_h = h as i32 - 2;
        if content_h > visible_h && visible_h > 0 {
            let track_x = x + w as i32 - 9;
            let track_h = h.saturating_sub(2);
            crate::draw::fill_rect(surface, track_x, y + 1, 8, track_h, tc.scrollbar_track);
            let max_scroll = (content_h - visible_h).max(1) as u32;
            let thumb_h = ((visible_h as u32 * track_h) / content_h as u32).max(20);
            let thumb_y = y + 1
                + (self.scroll_y as u32 * (track_h.saturating_sub(thumb_h)) / max_scroll) as i32;
            crate::draw::fill_rect(surface, track_x + 1, thumb_y, 6, thumb_h, tc.scrollbar);
        }
    }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, button: u32) -> EventResponse {
        if button & 1 != 0 {
            // Left button: start selection
            let (row, col) = self.pixel_to_cursor(lx, ly);
            self.cursor_row = row;
            self.cursor_col = col;
            self.selection = Some(Selection {
                start_row: row,
                start_col: col,
                end_row: row,
                end_col: col,
            });
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        if button & 4 != 0 {
            // Middle button: paste clipboard
            if let Some(data) = crate::compositor::clipboard_get() {
                self.delete_selection();
                self.clamp_cursor();
                self.insert_text_at_cursor(&data);
                return EventResponse::CHANGED;
            }
        }
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        if self.selection.is_some() {
            let (row, col) = self.pixel_to_cursor(lx, ly);
            if let Some(ref mut sel) = self.selection {
                sel.end_row = row;
                sel.end_col = col;
            }
            self.cursor_row = row;
            self.cursor_col = col;
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        EventResponse::IGNORED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if let Some(ref sel) = self.selection {
            if sel.is_empty() {
                // Single click, no drag — just position cursor
                self.selection = None;
            } else {
                // Copy selected text to clipboard
                if let Some(text) = self.extract_selected_text() {
                    crate::compositor::clipboard_set(&text);
                }
            }
            self.base.mark_dirty();
        }
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Selection is already handled by mouse_down/move/up
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        use crate::control::*;
        let has_ctrl = (modifiers & MOD_CTRL) != 0;
        let has_shift = (modifiers & MOD_SHIFT) != 0;

        // ── Ctrl shortcuts ──
        if has_ctrl {
            // Ctrl+C: copy
            if char_code == b'c' as u32 || char_code == b'C' as u32 {
                if let Some(text) = self.extract_selected_text() {
                    crate::compositor::clipboard_set(&text);
                } else {
                }
                return EventResponse::CONSUMED;
            }
            // Ctrl+X: cut
            if char_code == b'x' as u32 || char_code == b'X' as u32 {
                if let Some(text) = self.extract_selected_text() {
                    crate::compositor::clipboard_set(&text);
                    self.delete_selection();
                }
                return EventResponse::CHANGED;
            }
            // Ctrl+V: paste
            if char_code == b'v' as u32 || char_code == b'V' as u32 {
                if let Some(data) = crate::compositor::clipboard_get() {
                    self.delete_selection();
                    self.clamp_cursor();
                    self.insert_text_at_cursor(&data);
                }
                return EventResponse::CHANGED;
            }
            // Ctrl+A: select all
            if char_code == b'a' as u32 || char_code == b'A' as u32 {
                let last_row = self.lines.len().saturating_sub(1);
                let last_col = self.lines[last_row].len();
                self.selection = Some(Selection {
                    start_row: 0,
                    start_col: 0,
                    end_row: last_row,
                    end_col: last_col,
                });
                self.cursor_row = last_row;
                self.cursor_col = last_col;
                self.base.mark_dirty();
                return EventResponse::CONSUMED;
            }
            // Don't process Ctrl+key as printable
            return EventResponse::IGNORED;
        }

        // ── Arrow keys with Shift: extend selection ──
        if has_shift && matches!(keycode, KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN | KEY_HOME | KEY_END) {
            // Start selection at current cursor if none exists
            if self.selection.is_none() {
                self.selection = Some(Selection {
                    start_row: self.cursor_row,
                    start_col: self.cursor_col,
                    end_row: self.cursor_row,
                    end_col: self.cursor_col,
                });
            }
            // Move cursor
            match keycode {
                KEY_LEFT => {
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                    } else if self.cursor_row > 0 {
                        self.cursor_row -= 1;
                        self.cursor_col = self.lines[self.cursor_row].len();
                    }
                }
                KEY_RIGHT => {
                    if self.cursor_col < self.lines[self.cursor_row].len() {
                        self.cursor_col += 1;
                    } else if self.cursor_row + 1 < self.lines.len() {
                        self.cursor_row += 1;
                        self.cursor_col = 0;
                    }
                }
                KEY_UP => {
                    if self.cursor_row > 0 {
                        self.cursor_row -= 1;
                        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                    }
                }
                KEY_DOWN => {
                    if self.cursor_row + 1 < self.lines.len() {
                        self.cursor_row += 1;
                        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
                    }
                }
                KEY_HOME => { self.cursor_col = 0; }
                KEY_END => { self.cursor_col = self.lines[self.cursor_row].len(); }
                _ => {}
            }
            // Update selection endpoint
            if let Some(ref mut sel) = self.selection {
                sel.end_row = self.cursor_row;
                sel.end_col = self.cursor_col;
                if sel.is_empty() {
                    self.selection = None;
                }
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }

        // ── Backspace / Delete with selection: delete selection ──
        if keycode == KEY_BACKSPACE || keycode == KEY_DELETE {
            if self.selection.as_ref().map_or(false, |s| !s.is_empty()) {
                self.delete_selection();
                return EventResponse::CHANGED;
            }
        }

        // ── Clear selection on arrow keys without Shift ──
        if matches!(keycode, KEY_LEFT | KEY_RIGHT | KEY_UP | KEY_DOWN | KEY_HOME | KEY_END
                    | KEY_PAGE_UP | KEY_PAGE_DOWN) {
            self.selection = None;
        }

        // ── Delete selection before inserting text ──
        if char_code >= 0x20 && char_code < 0x7F {
            self.delete_selection();
        }
        if keycode == KEY_ENTER || keycode == KEY_TAB {
            self.delete_selection();
        }

        // Printable ASCII
        if char_code >= 0x20 && char_code < 0x7F {
            self.clamp_cursor();
            self.lines[self.cursor_row].insert(self.cursor_col, char_code as u8);
            self.cursor_col += 1;
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CHANGED;
        }
        // Enter
        if keycode == KEY_ENTER {
            self.clamp_cursor();
            let indent = self.lines[self.cursor_row]
                .iter()
                .take_while(|&&b| b == b' ')
                .count();
            let rest = self.lines[self.cursor_row].split_off(self.cursor_col);
            self.cursor_row += 1;
            let mut new_line = Vec::new();
            for _ in 0..indent {
                new_line.push(b' ');
            }
            new_line.extend_from_slice(&rest);
            self.cursor_col = indent;
            self.lines.insert(self.cursor_row, new_line);
            self.update_gutter_width();
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CHANGED;
        }
        // Backspace
        if keycode == KEY_BACKSPACE {
            self.clamp_cursor();
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
                self.lines[self.cursor_row].remove(self.cursor_col);
            } else if self.cursor_row > 0 {
                let current_line = self.lines.remove(self.cursor_row);
                self.cursor_row -= 1;
                self.cursor_col = self.lines[self.cursor_row].len();
                self.lines[self.cursor_row].extend_from_slice(&current_line);
                self.update_gutter_width();
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CHANGED;
        }
        // Delete
        if keycode == KEY_DELETE {
            self.clamp_cursor();
            if self.cursor_col < self.lines[self.cursor_row].len() {
                self.lines[self.cursor_row].remove(self.cursor_col);
            } else if self.cursor_row + 1 < self.lines.len() {
                let next_line = self.lines.remove(self.cursor_row + 1);
                self.lines[self.cursor_row].extend_from_slice(&next_line);
                self.update_gutter_width();
            }
            self.base.mark_dirty();
            return EventResponse::CHANGED;
        }
        // Tab
        if keycode == KEY_TAB {
            self.clamp_cursor();
            for _ in 0..self.tab_width {
                self.lines[self.cursor_row].insert(self.cursor_col, b' ');
                self.cursor_col += 1;
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CHANGED;
        }
        // Left arrow
        if keycode == KEY_LEFT {
            if self.cursor_col > 0 {
                self.cursor_col -= 1;
            } else if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.lines[self.cursor_row].len();
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Right arrow
        if keycode == KEY_RIGHT {
            if self.cursor_col < self.lines[self.cursor_row].len() {
                self.cursor_col += 1;
            } else if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = 0;
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Up arrow
        if keycode == KEY_UP {
            if self.cursor_row > 0 {
                self.cursor_row -= 1;
                self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Down arrow
        if keycode == KEY_DOWN {
            if self.cursor_row + 1 < self.lines.len() {
                self.cursor_row += 1;
                self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            }
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Home
        if keycode == KEY_HOME {
            self.cursor_col = 0;
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // End
        if keycode == KEY_END {
            self.cursor_col = self.lines[self.cursor_row].len();
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Page Up
        if keycode == KEY_PAGE_UP {
            self.selection = None;
            let page = (self.base.h / self.line_height).max(1) as usize;
            self.cursor_row = self.cursor_row.saturating_sub(page);
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        // Page Down
        if keycode == KEY_PAGE_DOWN {
            self.selection = None;
            let page = (self.base.h / self.line_height).max(1) as usize;
            self.cursor_row = (self.cursor_row + page).min(self.lines.len().saturating_sub(1));
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
            self.ensure_cursor_visible();
            self.base.mark_dirty();
            return EventResponse::CONSUMED;
        }
        EventResponse::IGNORED
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        let max_scroll = (self.content_height() - (self.base.h as i32 - 2)).max(0);
        self.scroll_y =
            (self.scroll_y - delta * self.line_height as i32).clamp(0, max_scroll);
        self.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.base.mark_dirty();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.base.mark_dirty();
    }
}

// ── Tokenizer ────────────────────────────────────────────────────────

fn tokenize_line(line: &[u8], in_block_comment: bool, syn: &SyntaxDef) -> (Vec<ColorSpan>, bool) {
    let mut spans = Vec::new();
    let mut i = 0;
    let mut in_comment = in_block_comment;
    let default_color = crate::theme::colors().text;

    while i < line.len() {
        // Block comment continuation
        if in_comment {
            let start = i;
            if let Some(pos) = find_subsequence(&line[i..], &syn.block_comment_end) {
                i += pos + syn.block_comment_end.len();
                spans.push(ColorSpan { start, end: i, color: syn.comment_color });
                in_comment = false;
            } else {
                spans.push(ColorSpan { start, end: line.len(), color: syn.comment_color });
                i = line.len();
            }
            continue;
        }

        // Block comment start
        if !syn.block_comment_start.is_empty() && starts_with_at(line, i, &syn.block_comment_start)
        {
            let start = i;
            i += syn.block_comment_start.len();
            if let Some(pos) = find_subsequence(&line[i..], &syn.block_comment_end) {
                i += pos + syn.block_comment_end.len();
                spans.push(ColorSpan { start, end: i, color: syn.comment_color });
            } else {
                spans.push(ColorSpan { start, end: line.len(), color: syn.comment_color });
                i = line.len();
                in_comment = true;
            }
            continue;
        }

        // Line comment
        if !syn.line_comment.is_empty() && starts_with_at(line, i, &syn.line_comment) {
            spans.push(ColorSpan { start: i, end: line.len(), color: syn.comment_color });
            i = line.len();
            continue;
        }

        // String literal
        if syn.string_delimiters.contains(&line[i]) {
            let delim = line[i];
            let start = i;
            i += 1;
            while i < line.len() {
                if line[i] == b'\\' && i + 1 < line.len() {
                    i += 2;
                } else if line[i] == delim {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            spans.push(ColorSpan { start, end: i, color: syn.string_color });
            continue;
        }

        // Char literal
        if line[i] == syn.char_delimiter {
            let start = i;
            i += 1;
            while i < line.len() {
                if line[i] == b'\\' && i + 1 < line.len() {
                    i += 2;
                } else if line[i] == syn.char_delimiter {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            spans.push(ColorSpan { start, end: i, color: syn.string_color });
            continue;
        }

        // Number
        if line[i].is_ascii_digit()
            || (line[i] == b'.' && i + 1 < line.len() && line[i + 1].is_ascii_digit())
        {
            let start = i;
            if line[i] == b'0'
                && i + 1 < line.len()
                && (line[i + 1] == b'x' || line[i + 1] == b'X')
            {
                i += 2;
                while i < line.len() && (line[i].is_ascii_hexdigit() || line[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < line.len()
                    && (line[i].is_ascii_digit() || line[i] == b'.' || line[i] == b'_')
                {
                    i += 1;
                }
            }
            // Type suffix (u32, i64, f64, etc.)
            if i < line.len() && (line[i] == b'u' || line[i] == b'i' || line[i] == b'f') {
                i += 1;
                while i < line.len() && line[i].is_ascii_digit() {
                    i += 1;
                }
            }
            spans.push(ColorSpan { start, end: i, color: syn.number_color });
            continue;
        }

        // Identifier (keyword, type, builtin, or default)
        if line[i].is_ascii_alphabetic() || line[i] == b'_' {
            let start = i;
            while i < line.len() && (line[i].is_ascii_alphanumeric() || line[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let color = if syn.keywords.iter().any(|k| k.as_slice() == word) {
                syn.keyword_color
            } else if syn.types.iter().any(|t| t.as_slice() == word) {
                syn.type_color
            } else if syn.builtins.iter().any(|b| b.as_slice() == word) {
                syn.builtin_color
            } else {
                default_color
            };
            spans.push(ColorSpan { start, end: i, color });
            continue;
        }

        // Operator
        if is_operator(line[i]) {
            spans.push(ColorSpan { start: i, end: i + 1, color: syn.operator_color });
            i += 1;
            continue;
        }

        // Default (whitespace and other)
        let start = i;
        while i < line.len()
            && !line[i].is_ascii_alphanumeric()
            && line[i] != b'_'
            && !is_operator(line[i])
            && !syn.string_delimiters.contains(&line[i])
            && line[i] != syn.char_delimiter
        {
            i += 1;
        }
        if start < i {
            spans.push(ColorSpan { start, end: i, color: default_color });
        }
    }

    (spans, in_comment)
}

// ── Helpers ──────────────────────────────────────────────────────────

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn starts_with_at(data: &[u8], offset: usize, prefix: &[u8]) -> bool {
    if offset + prefix.len() > data.len() {
        return false;
    }
    &data[offset..offset + prefix.len()] == prefix
}

fn is_operator(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'='
            | b'<'
            | b'>'
            | b'!'
            | b'&'
            | b'|'
            | b'^'
            | b'~'
            | b':'
            | b';'
            | b','
            | b'.'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'['
            | b']'
            | b'@'
            | b'#'
            | b'?'
    )
}

fn format_line_number(n: usize, buf: &mut [u8; 8]) -> usize {
    if n == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut v = n;
    let mut len = 0;
    while v > 0 && len < 8 {
        buf[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    buf[..len].reverse();
    len
}

fn split_csv(data: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut start = 0;
    for i in 0..data.len() {
        if data[i] == b',' {
            if i > start {
                result.push(data[start..i].to_vec());
            }
            start = i + 1;
        }
    }
    if start < data.len() {
        result.push(data[start..].to_vec());
    }
    result
}

fn parse_hex_color(s: &[u8]) -> Option<u32> {
    // Expect "0xNNNNNNNN" or "0XNNNNNNNN"
    if s.len() < 3 {
        return None;
    }
    let start = if s[0] == b'0' && (s[1] == b'x' || s[1] == b'X') {
        2
    } else {
        0
    };
    let mut val = 0u32;
    for &b in &s[start..] {
        let digit = if b >= b'0' && b <= b'9' {
            b - b'0'
        } else if b >= b'a' && b <= b'f' {
            b - b'a' + 10
        } else if b >= b'A' && b <= b'F' {
            b - b'A' + 10
        } else {
            return None;
        };
        val = val * 16 + digit as u32;
    }
    Some(val)
}
