use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct TextField {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
    pub(crate) password_mode: bool,
    pub(crate) placeholder: Vec<u8>,

    /// Optional prefix icon code (rendered at left edge).
    pub(crate) prefix_icon: Option<u32>,
    /// Optional postfix icon code (rendered at right edge).
    pub(crate) postfix_icon: Option<u32>,
    /// Width reserved for prefix area in pixels.
    pub(crate) prefix_width: u32,
    /// Width reserved for postfix area in pixels.
    pub(crate) postfix_width: u32,

    /// Horizontal scroll offset (pixels) for long text.
    scroll_x: i32,
    /// Selection anchor (byte offset). If != cursor_pos, text is selected.
    sel_anchor: usize,
    /// Whether a mouse drag selection is in progress.
    dragging: bool,
}

impl TextField {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            cursor_pos: 0,
            focused: false,
            password_mode: false,
            placeholder: Vec::new(),
            prefix_icon: None,
            postfix_icon: None,
            prefix_width: 28,
            postfix_width: 28,
            scroll_x: 0,
            sel_anchor: 0,
            dragging: false,
        }
    }

    /// Left edge of the text area (after prefix).
    fn text_area_left(&self) -> i32 {
        if self.prefix_icon.is_some() { self.prefix_width as i32 } else { 8 }
    }

    /// Right edge of the text area (before postfix), relative to control width.
    fn text_area_right(&self) -> i32 {
        let w = self.text_base.base.w as i32;
        if self.postfix_icon.is_some() { w - self.postfix_width as i32 } else { w - 8 }
    }

    /// Visible text width in pixels.
    fn text_area_width(&self) -> i32 {
        self.text_area_right() - self.text_area_left()
    }

    /// Get the display text (asterisks for password mode).
    fn display_text(&self) -> Vec<u8> {
        if self.password_mode {
            let n = self.text_base.text.len();
            let mut dots = Vec::with_capacity(n);
            for _ in 0..n { dots.push(b'*'); }
            dots
        } else {
            self.text_base.text.clone()
        }
    }

    /// Returns (sel_start, sel_end) sorted.
    fn selection_range(&self) -> (usize, usize) {
        if self.cursor_pos <= self.sel_anchor {
            (self.cursor_pos, self.sel_anchor)
        } else {
            (self.sel_anchor, self.cursor_pos)
        }
    }

    fn has_selection(&self) -> bool {
        self.cursor_pos != self.sel_anchor
    }

    /// Delete selected text and collapse cursor.
    fn delete_selection(&mut self) {
        if !self.has_selection() { return; }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        self.text_base.text.drain(start..end);
        self.cursor_pos = start;
        self.sel_anchor = start;
    }

    /// Get selected text as bytes.
    fn selected_bytes(&self) -> &[u8] {
        if !self.has_selection() { return &[]; }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        &self.text_base.text[start..end]
    }

    /// Ensure cursor is visible by adjusting scroll_x.
    fn ensure_cursor_visible(&mut self) {
        let fs = self.text_base.text_style.font_size;
        let display = self.display_text();
        let cursor = self.cursor_pos.min(display.len());
        let cursor_px = crate::draw::text_width_n_at(&display, cursor, fs) as i32;
        let area_w = self.text_area_width();

        // Scroll right if cursor is past the visible area.
        if cursor_px - self.scroll_x > area_w {
            self.scroll_x = cursor_px - area_w;
        }
        // Scroll left if cursor is before the visible area.
        if cursor_px - self.scroll_x < 0 {
            self.scroll_x = cursor_px;
        }
        // Don't scroll past zero.
        if self.scroll_x < 0 { self.scroll_x = 0; }
    }

    /// Convert a local x coordinate (relative to control) to a byte position.
    fn x_to_pos(&self, local_x: i32) -> usize {
        let fs = self.text_base.text_style.font_size;
        let display = self.display_text();
        let text_local_x = local_x - self.text_area_left() + self.scroll_x;
        crate::draw::text_hit_test(&display, text_local_x, fs)
    }

    /// Find the start of the previous word boundary.
    fn word_left(&self, pos: usize) -> usize {
        if pos == 0 { return 0; }
        let text = &self.text_base.text;
        let mut i = pos - 1;
        // Skip whitespace/punctuation.
        while i > 0 && !is_word_char(text[i]) { i -= 1; }
        // Skip word characters.
        while i > 0 && is_word_char(text[i - 1]) { i -= 1; }
        i
    }

    /// Find the end of the next word boundary.
    fn word_right(&self, pos: usize) -> usize {
        let text = &self.text_base.text;
        let len = text.len();
        if pos >= len { return len; }
        let mut i = pos;
        // Skip word characters.
        while i < len && is_word_char(text[i]) { i += 1; }
        // Skip whitespace/punctuation.
        while i < len && !is_word_char(text[i]) { i += 1; }
        i
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Control for TextField {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::TextField }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
        let corner = crate::theme::INPUT_CORNER;

        // Background: use custom color if set, otherwise theme color.
        let custom = self.text_base.base.color;
        let bg = if custom != 0 {
            if disabled { crate::theme::darken(custom, 10) } else { custom }
        } else {
            if disabled { crate::theme::darken(tc.input_bg, 10) } else { tc.input_bg }
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);

        // Border: focus (accent) > hover (lighter) > normal
        let border_color = if self.focused {
            tc.input_focus
        } else if hovered && !disabled {
            tc.accent
        } else {
            tc.input_border
        };
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, border_color);

        // Focus ring (2px glow around the field)
        if self.focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }

        // Prefix icon placeholder
        if let Some(_icon) = self.prefix_icon {
            crate::draw::fill_rounded_rect(surface, x + 8, y + (h as i32 - 12) / 2, 12, 12, 6, tc.text_secondary);
        }

        // Postfix icon placeholder
        if let Some(_icon) = self.postfix_icon {
            let px = x + w as i32 - self.postfix_width as i32 + 8;
            crate::draw::fill_rounded_rect(surface, px, y + (h as i32 - 12) / 2, 12, 12, 6, tc.text_secondary);
        }

        // Text area — clip to the text region to prevent overflow.
        let text_left = self.text_area_left();
        let text_right = self.text_area_right();
        let area_w = (text_right - text_left).max(0) as u32;
        let clipped = surface.with_clip(x + text_left, y, area_w, h);

        let text_color = if disabled {
            tc.text_disabled
        } else {
            self.text_base.effective_text_color()
        };
        let font_size = self.text_base.text_style.font_size;
        let text_y = y + 6;
        let text_x = x + text_left - self.scroll_x;

        if self.text_base.text.is_empty() && !self.placeholder.is_empty() {
            crate::draw::draw_text_sized(&clipped, x + text_left, text_y, tc.text_secondary, &self.placeholder, font_size);
        } else {
            let display = self.display_text();

            // Draw selection highlight.
            if self.has_selection() && self.focused {
                let (sel_start, sel_end) = self.selection_range();
                let start_px = crate::draw::text_width_n_at(&display, sel_start.min(display.len()), font_size) as i32;
                let end_px = crate::draw::text_width_n_at(&display, sel_end.min(display.len()), font_size) as i32;
                let sel_x = text_x + start_px;
                let sel_w = (end_px - start_px).max(0) as u32;
                crate::draw::fill_rect(&clipped, sel_x, y + 3, sel_w, h - 6, tc.accent & 0x60FFFFFF);
            }

            // Draw text.
            crate::draw::draw_text_sized(&clipped, text_x, text_y, text_color, &display, font_size);

            // Cursor.
            if self.focused {
                let cursor = self.cursor_pos.min(display.len());
                let cursor_px = crate::draw::text_width_n_at(&display, cursor, font_size) as i32;
                let cx = text_x + cursor_px;
                crate::draw::fill_rect(&clipped, cx, y + 4, 2, h - 8, tc.accent);
            }
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }
    fn accepts_focus(&self) -> bool { !self.text_base.base.disabled }

    fn handle_mouse_down(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let pos = self.x_to_pos(lx);
        self.cursor_pos = pos;
        self.sel_anchor = pos;
        self.dragging = true;
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_mouse_move(&mut self, lx: i32, _ly: i32) -> EventResponse {
        if !self.dragging { return EventResponse::IGNORED; }
        let pos = self.x_to_pos(lx);
        self.cursor_pos = pos;
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.dragging = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Click-to-position is handled in handle_mouse_down.
        EventResponse::CONSUMED
    }

    fn handle_double_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Select the word under cursor.
        let pos = self.x_to_pos(lx);
        let text = &self.text_base.text;
        if text.is_empty() { return EventResponse::CONSUMED; }
        let pos = pos.min(text.len().saturating_sub(1));
        // Find word boundaries.
        let mut start = pos;
        while start > 0 && is_word_char(text[start - 1]) { start -= 1; }
        let mut end = pos;
        while end < text.len() && is_word_char(text[end]) { end += 1; }
        // If we clicked on a non-word char, select that single char.
        if start == end && pos < text.len() {
            end = pos + 1;
            start = pos;
        }
        self.sel_anchor = start;
        self.cursor_pos = end;
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_triple_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Select all text.
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        use crate::control::*;
        let shift = modifiers & MOD_SHIFT != 0;
        let ctrl = modifiers & MOD_CTRL != 0;

        // Ctrl+A: select all.
        if ctrl && (char_code == b'a' as u32 || char_code == b'A' as u32) {
            self.sel_anchor = 0;
            self.cursor_pos = self.text_base.text.len();
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }

        // Ctrl+C: copy selection to clipboard.
        if ctrl && (char_code == b'c' as u32 || char_code == b'C' as u32) {
            if self.has_selection() {
                let bytes = self.selected_bytes().to_vec();
                crate::compositor::clipboard_set(&bytes);
            }
            return EventResponse::CONSUMED;
        }

        // Ctrl+X: cut selection.
        if ctrl && (char_code == b'x' as u32 || char_code == b'X' as u32) {
            if self.has_selection() {
                let bytes = self.selected_bytes().to_vec();
                crate::compositor::clipboard_set(&bytes);
                self.delete_selection();
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }

        // Ctrl+V: paste from clipboard.
        if ctrl && (char_code == b'v' as u32 || char_code == b'V' as u32) {
            if let Some(clip) = crate::compositor::clipboard_get() {
                // Filter to printable ASCII.
                let filtered: Vec<u8> = clip.into_iter().filter(|&b| b >= 0x20 && b < 0x7F).collect();
                if !filtered.is_empty() {
                    self.delete_selection();
                    let pos = self.cursor_pos.min(self.text_base.text.len());
                    for (i, &b) in filtered.iter().enumerate() {
                        self.text_base.text.insert(pos + i, b);
                    }
                    self.cursor_pos = pos + filtered.len();
                    self.sel_anchor = self.cursor_pos;
                    self.ensure_cursor_visible();
                    return EventResponse::CHANGED;
                }
            }
            return EventResponse::CONSUMED;
        }

        // Printable character input.
        if char_code >= 0x20 && char_code < 0x7F && !ctrl {
            let ch = char_code as u8;
            self.delete_selection();
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, ch);
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            return EventResponse::CHANGED;
        }

        if keycode == KEY_BACKSPACE {
            if self.has_selection() {
                self.delete_selection();
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                self.sel_anchor = self.cursor_pos;
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_DELETE {
            if self.has_selection() {
                self.delete_selection();
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            if self.cursor_pos < self.text_base.text.len() {
                self.text_base.text.remove(self.cursor_pos);
                self.sel_anchor = self.cursor_pos;
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_LEFT {
            if ctrl {
                // Word left.
                self.cursor_pos = self.word_left(self.cursor_pos);
            } else if !shift && self.has_selection() {
                // Collapse selection to left edge.
                let (start, _) = self.selection_range();
                self.cursor_pos = start;
            } else if self.cursor_pos > 0 {
                self.cursor_pos -= 1;
            }
            if !shift { self.sel_anchor = self.cursor_pos; }
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_RIGHT {
            if ctrl {
                // Word right.
                self.cursor_pos = self.word_right(self.cursor_pos);
            } else if !shift && self.has_selection() {
                // Collapse selection to right edge.
                let (_, end) = self.selection_range();
                self.cursor_pos = end;
            } else if self.cursor_pos < self.text_base.text.len() {
                self.cursor_pos += 1;
            }
            if !shift { self.sel_anchor = self.cursor_pos; }
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_HOME {
            self.cursor_pos = 0;
            if !shift { self.sel_anchor = 0; }
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_END {
            self.cursor_pos = self.text_base.text.len();
            if !shift { self.sel_anchor = self.cursor_pos; }
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_ENTER {
            return EventResponse::SUBMIT;
        }

        EventResponse::IGNORED
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.text_base.base.focused = true;
        self.text_base.base.mark_dirty();
        // Select all on focus (standard OS behavior).
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.text_base.base.focused = false;
        self.dragging = false;
        // Collapse selection on blur.
        self.sel_anchor = self.cursor_pos;
        self.text_base.base.mark_dirty();
    }
}
