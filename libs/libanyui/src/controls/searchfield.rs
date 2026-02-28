use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct SearchField {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
    /// Horizontal scroll offset (pixels) for long text.
    scroll_x: i32,
    /// Selection anchor (byte offset). If != cursor_pos, text is selected.
    sel_anchor: usize,
    /// Whether a mouse drag selection is in progress.
    dragging: bool,
}

impl SearchField {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            cursor_pos: 0,
            focused: false,
            scroll_x: 0,
            sel_anchor: 0,
            dragging: false,
        }
    }

    const TEXT_LEFT: i32 = 26; // After search icon.
    const TEXT_PAD_RIGHT: i32 = 8;

    fn text_area_width(&self) -> i32 {
        (self.text_base.base.w as i32 - Self::TEXT_LEFT - Self::TEXT_PAD_RIGHT).max(0)
    }

    fn selection_range(&self) -> (usize, usize) {
        if self.cursor_pos <= self.sel_anchor {
            (self.cursor_pos, self.sel_anchor)
        } else {
            (self.sel_anchor, self.cursor_pos)
        }
    }

    fn has_selection(&self) -> bool { self.cursor_pos != self.sel_anchor }

    fn delete_selection(&mut self) {
        if !self.has_selection() { return; }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        self.text_base.text.drain(start..end);
        self.cursor_pos = start;
        self.sel_anchor = start;
    }

    fn selected_bytes(&self) -> &[u8] {
        if !self.has_selection() { return &[]; }
        let (start, end) = self.selection_range();
        let end = end.min(self.text_base.text.len());
        let start = start.min(end);
        &self.text_base.text[start..end]
    }

    fn ensure_cursor_visible(&mut self) {
        let fs = self.text_base.text_style.font_size;
        let cursor = self.cursor_pos.min(self.text_base.text.len());
        let cursor_px = crate::draw::text_width_n_at(&self.text_base.text, cursor, fs) as i32;
        let area_w = self.text_area_width();
        if cursor_px - self.scroll_x > area_w { self.scroll_x = cursor_px - area_w; }
        if cursor_px - self.scroll_x < 0 { self.scroll_x = cursor_px; }
        if self.scroll_x < 0 { self.scroll_x = 0; }
    }

    fn x_to_pos(&self, local_x: i32) -> usize {
        let fs = self.text_base.text_style.font_size;
        let text_local_x = local_x - Self::TEXT_LEFT + self.scroll_x;
        crate::draw::text_hit_test(&self.text_base.text, text_local_x, fs)
    }

    fn word_left(&self, pos: usize) -> usize {
        if pos == 0 { return 0; }
        let text = &self.text_base.text;
        let mut i = pos - 1;
        while i > 0 && !is_word_char(text[i]) { i -= 1; }
        while i > 0 && is_word_char(text[i - 1]) { i -= 1; }
        i
    }

    fn word_right(&self, pos: usize) -> usize {
        let text = &self.text_base.text;
        let len = text.len();
        if pos >= len { return len; }
        let mut i = pos;
        while i < len && is_word_char(text[i]) { i += 1; }
        while i < len && !is_word_char(text[i]) { i += 1; }
        i
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Control for SearchField {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::SearchField }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let hovered = b.hovered;
        let corner = h / 2; // Full round ends (pill shape)

        // Background
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, tc.input_bg);

        // Border: focus > hover > normal
        let border_color = if self.focused {
            tc.input_focus
        } else if hovered && !disabled {
            tc.accent
        } else {
            tc.input_border
        };
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, border_color);

        // Focus ring
        if self.focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }

        // Search icon placeholder (small circle + line = magnifying glass)
        let icon_sz = crate::theme::scale(10);
        let icon_r = crate::theme::scale(5);
        let inner_sz = crate::theme::scale(4);
        let inner_r = crate::theme::scale(2);
        let inner_off = crate::theme::scale_i32(3);
        let icon_x = x + crate::theme::scale_i32(10);
        let icon_y = y + (h as i32 - icon_sz as i32) / 2;
        crate::draw::fill_rounded_rect(surface, icon_x, icon_y, icon_sz, icon_sz, icon_r, tc.text_secondary);
        crate::draw::fill_rounded_rect(surface, icon_x + inner_off, icon_y + inner_off, inner_sz, inner_sz, inner_r, tc.input_bg);

        // Text area — clipped (physical coordinates).
        let text_left = crate::theme::scale_i32(Self::TEXT_LEFT);
        let text_pad_right = crate::theme::scale_i32(Self::TEXT_PAD_RIGHT);
        let area_w = (w as i32 - text_left - text_pad_right).max(0) as u32;
        let clipped = surface.with_clip(x + text_left, y, area_w, h);

        let text_color = if disabled { tc.text_disabled } else if self.text_base.text_style.text_color != 0 { self.text_base.text_style.text_color } else { tc.text };
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let scaled_scroll_x = crate::theme::scale_i32(self.scroll_x);
        let text_x = x + text_left - scaled_scroll_x;
        let text_y = y + crate::theme::scale_i32(6);

        if self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(&clipped, x + text_left, text_y, tc.text_secondary, b"Search", font_size);
        } else {
            // Selection highlight.
            if self.has_selection() && self.focused {
                let (sel_start, sel_end) = self.selection_range();
                let text = &self.text_base.text;
                let start_px = crate::draw::text_width_n_at(text, sel_start.min(text.len()), font_size) as i32;
                let end_px = crate::draw::text_width_n_at(text, sel_end.min(text.len()), font_size) as i32;
                let sel_pad = crate::theme::scale_i32(3);
                let sel_h = if h > (sel_pad as u32 * 2) { h - sel_pad as u32 * 2 } else { 1 };
                crate::draw::fill_rect(&clipped, text_x + start_px, y + sel_pad, (end_px - start_px).max(0) as u32, sel_h, tc.accent & 0x60FFFFFF);
            }

            crate::draw::draw_text_sized(&clipped, text_x, text_y, text_color, &self.text_base.text, font_size);

            if self.focused {
                let cursor = self.cursor_pos.min(self.text_base.text.len());
                let cursor_px = crate::draw::text_width_n_at(&self.text_base.text, cursor, font_size) as i32;
                let cursor_pad = crate::theme::scale_i32(4);
                let cursor_w = crate::theme::scale(2);
                let cursor_h = if h > (cursor_pad as u32 * 2) { h - cursor_pad as u32 * 2 } else { 1 };
                crate::draw::fill_rect(&clipped, text_x + cursor_px, y + cursor_pad, cursor_w, cursor_h, tc.accent);
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
        self.cursor_pos = self.x_to_pos(lx);
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.dragging = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CONSUMED
    }

    fn handle_double_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let pos = self.x_to_pos(lx);
        let text = &self.text_base.text;
        if text.is_empty() { return EventResponse::CONSUMED; }
        let pos = pos.min(text.len().saturating_sub(1));
        let mut start = pos;
        while start > 0 && is_word_char(text[start - 1]) { start -= 1; }
        let mut end = pos;
        while end < text.len() && is_word_char(text[end]) { end += 1; }
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
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        use crate::control::*;
        let shift = modifiers & MOD_SHIFT != 0;
        let ctrl = modifiers & MOD_CTRL != 0;

        if ctrl && (char_code == b'a' as u32 || char_code == b'A' as u32) {
            self.sel_anchor = 0;
            self.cursor_pos = self.text_base.text.len();
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }
        if ctrl && (char_code == b'c' as u32 || char_code == b'C' as u32) {
            if self.has_selection() {
                crate::compositor::clipboard_set(self.selected_bytes());
            }
            return EventResponse::CONSUMED;
        }
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
        if ctrl && (char_code == b'v' as u32 || char_code == b'V' as u32) {
            if let Some(clip) = crate::compositor::clipboard_get() {
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

        if keycode == KEY_ENTER { return EventResponse::SUBMIT; }

        if char_code >= 0x20 && char_code < 0x7F && !ctrl {
            self.delete_selection();
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, char_code as u8);
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            return EventResponse::CHANGED;
        }

        if keycode == KEY_BACKSPACE {
            if self.has_selection() { self.delete_selection(); self.ensure_cursor_visible(); return EventResponse::CHANGED; }
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
            if self.has_selection() { self.delete_selection(); self.ensure_cursor_visible(); return EventResponse::CHANGED; }
            if self.cursor_pos < self.text_base.text.len() {
                self.text_base.text.remove(self.cursor_pos);
                self.sel_anchor = self.cursor_pos;
                self.ensure_cursor_visible();
                return EventResponse::CHANGED;
            }
            return EventResponse::CONSUMED;
        }

        if keycode == KEY_LEFT {
            if ctrl { self.cursor_pos = self.word_left(self.cursor_pos); }
            else if !shift && self.has_selection() { let (s, _) = self.selection_range(); self.cursor_pos = s; }
            else if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            if !shift { self.sel_anchor = self.cursor_pos; }
            self.ensure_cursor_visible();
            return EventResponse::CONSUMED;
        }
        if keycode == KEY_RIGHT {
            if ctrl { self.cursor_pos = self.word_right(self.cursor_pos); }
            else if !shift && self.has_selection() { let (_, e) = self.selection_range(); self.cursor_pos = e; }
            else if self.cursor_pos < self.text_base.text.len() { self.cursor_pos += 1; }
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

        EventResponse::IGNORED
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.text_base.base.focused = true;
        self.text_base.base.mark_dirty();
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.text_base.base.focused = false;
        self.dragging = false;
        self.sel_anchor = self.cursor_pos;
        self.text_base.base.mark_dirty();
    }
}
