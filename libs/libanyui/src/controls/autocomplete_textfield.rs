//! AutoCompleteTextField — editable text field with autocomplete suggestions popup.
//!
//! Similar to TextField but with an additional suggestions list (pipe-separated).
//! When the user types text with 2+ characters, a popup appears with matching suggestions.
//! Selecting a suggestion replaces the last token (after the last comma for multi-value).

use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};
use crate::control::{KEY_UP, KEY_DOWN, KEY_ENTER, KEY_ESCAPE, KEY_BACKSPACE, KEY_LEFT, KEY_RIGHT, KEY_HOME, KEY_END, MOD_SHIFT, MOD_CTRL};

pub struct AutoCompleteTextField {
    pub(crate) text_base: TextControlBase,
    /// Full list of suggestions (pipe-separated, set by client app)
    pub(crate) suggestions: Vec<u8>,
    /// Cursor position in the text
    pub(crate) cursor_pos: usize,
    /// Horizontal scroll offset for long text
    scroll_x: i32,
    /// Selection anchor (byte offset)
    sel_anchor: usize,
    /// Whether a mouse drag selection is in progress
    dragging: bool,
    /// Placeholder text shown when empty
    pub(crate) placeholder: Vec<u8>,
    /// Set to true when text.len() >= 2; event loop opens popup and clears flag
    pub(crate) suggest: bool,
}

impl AutoCompleteTextField {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            suggestions: Vec::new(),
            cursor_pos: 0,
            scroll_x: 0,
            sel_anchor: 0,
            dragging: false,
            placeholder: Vec::new(),
            suggest: false,
        }
    }

    /// Get the last token (after the last comma), used for filtering
    fn last_token(text: &[u8]) -> &[u8] {
        if let Some(pos) = text.iter().rposition(|&b| b == b',') {
            let rest = &text[pos + 1..];
            // Skip leading whitespace
            let start = rest.iter().position(|&b| b != b' ').unwrap_or(rest.len());
            &rest[start..]
        } else {
            text
        }
    }

    /// Filter suggestions matching the current last token (case-insensitive prefix match)
    pub(crate) fn filtered_items(&self) -> Vec<u8> {
        let query = Self::last_token(&self.text_base.text);

        if self.text_base.text.is_empty() {
            return Vec::new();
        }

        if query.len() < 2 {
            return Vec::new();
        }

        let query_lower = to_lower_u8(query);
        let mut matches = Vec::new();
        let mut count = 0;
        const MAX_SUGGESTIONS: usize = 8;

        // Iterate through pipe-separated suggestions
        let mut start = 0;
        for i in 0..=self.suggestions.len() {
            if i == self.suggestions.len() || self.suggestions[i] == b'|' {
                let item = &self.suggestions[start..i];
                let item_lower = to_lower_u8(item);

                // Case-insensitive prefix match
                if item_lower.starts_with(query_lower.as_slice()) {
                    if !matches.is_empty() {
                        matches.push(b'|');
                    }
                    matches.extend_from_slice(item);
                    count += 1;
                    if count >= MAX_SUGGESTIONS {
                        break;
                    }
                }

                start = i + 1;
            }
        }

        matches
    }

    fn has_selection(&self) -> bool {
        self.sel_anchor != self.cursor_pos
    }

    fn delete_selection(&mut self) {
        if !self.has_selection() {
            return;
        }
        let start = self.sel_anchor.min(self.cursor_pos);
        let end = self.sel_anchor.max(self.cursor_pos);
        self.text_base.text.drain(start..end);
        self.cursor_pos = start;
        self.sel_anchor = start;
    }

    fn ensure_cursor_visible(&mut self) {
        if self.cursor_pos == 0 {
            self.scroll_x = 0;
        }
        // Simple scroll - just keep cursor roughly centered
        if self.cursor_pos > 20 {
            self.scroll_x = (self.cursor_pos as i32 - 10) * 8;
        }
    }

    fn select_all(&mut self) {
        self.sel_anchor = 0;
        self.cursor_pos = self.text_base.text.len();
        self.ensure_cursor_visible();
    }
}

impl Control for AutoCompleteTextField {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }

    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }

    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.text_base)
    }

    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.text_base)
    }

    fn kind(&self) -> ControlKind {
        ControlKind::AutoCompleteTextField
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let focused = b.focused;
        let hovered = b.hovered;
        let corner = crate::theme::scale(6);

        // ── Background ──────────────────────────────────────────────
        let bg = if disabled {
            crate::theme::darken(tc.input_bg, 10)
        } else if hovered {
            tc.control_hover
        } else {
            tc.input_bg
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, tc.input_border);

        // ── Text content ────────────────────────────────────────────
        let logical_fs = if self.text_base.text_style.font_size > 0 {
            self.text_base.text_style.font_size
        } else {
            13
        };
        let font_size = crate::draw::scale_font(logical_fs);
        let text_color = if disabled { tc.text_disabled } else { tc.text };
        let text_x = x + crate::theme::scale_i32(10);
        let ty = y + (h as i32 - font_size as i32) / 2;

        let display = &self.text_base.text;
        if display.is_empty() && !self.placeholder.is_empty() {
            crate::draw::draw_text_sized(surface, text_x, ty, tc.text_secondary, &self.placeholder, font_size);
        } else {
            crate::draw::draw_text_sized(surface, text_x - crate::theme::scale_i32(self.scroll_x), ty, text_color, display, font_size);
        }

        // ── Cursor (when focused) ────────────────────────────────────
        if focused && !disabled {
            // Simple cursor: just a vertical line
            let cursor_offset = (self.cursor_pos as i32 * 8) - self.scroll_x;
            let cursor_x = text_x + crate::theme::scale_i32(cursor_offset);
            crate::draw::fill_rect(surface, cursor_x, ty, 2, font_size as u32, text_color);
        }

        // ── Focus ring ──────────────────────────────────────────────
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if !self.is_interactive() {
            return EventResponse::IGNORED;
        }
        self.text_base.base.focused = true;
        self.base_mut().mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, modifiers: u32) -> EventResponse {
        if !self.is_interactive() {
            return EventResponse::IGNORED;
        }

        let shift = modifiers & MOD_SHIFT != 0;
        let ctrl = modifiers & MOD_CTRL != 0;

        // Ctrl+A: select all
        if ctrl && (char_code == b'a' as u32 || char_code == b'A' as u32) {
            self.select_all();
            self.text_base.base.mark_dirty();
            return EventResponse::CONSUMED;
        }

        // Ctrl+C: copy selection
        if ctrl && (char_code == b'c' as u32 || char_code == b'C' as u32) {
            if self.has_selection() {
                let start = self.sel_anchor.min(self.cursor_pos);
                let end = self.sel_anchor.max(self.cursor_pos);
                let bytes = self.text_base.text[start..end].to_vec();
                crate::compositor::clipboard_set(&bytes);
            }
            return EventResponse::CONSUMED;
        }

        // Ctrl+V: paste
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
                    self.suggest = self.text_base.text.len() >= 2;
                    self.text_base.base.mark_dirty();
                    return EventResponse::CHANGED;
                }
            }
            return EventResponse::CONSUMED;
        }

        // Printable character input
        if char_code >= 0x20 && char_code < 0x7F && !ctrl {
            let ch = char_code as u8;
            self.delete_selection();
            let pos = self.cursor_pos.min(self.text_base.text.len());
            self.text_base.text.insert(pos, ch);
            self.cursor_pos = pos + 1;
            self.sel_anchor = self.cursor_pos;
            self.ensure_cursor_visible();
            self.suggest = self.text_base.text.len() >= 2;
            self.text_base.base.mark_dirty();
            return EventResponse::CHANGED;
        }

        // Navigation & editing keys
        match keycode {
            KEY_LEFT => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    if !shift {
                        self.sel_anchor = self.cursor_pos;
                    }
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_RIGHT => {
                if self.cursor_pos < self.text_base.text.len() {
                    self.cursor_pos += 1;
                    if !shift {
                        self.sel_anchor = self.cursor_pos;
                    }
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_HOME => {
                self.cursor_pos = 0;
                if !shift {
                    self.sel_anchor = 0;
                }
                self.scroll_x = 0;
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_END => {
                self.cursor_pos = self.text_base.text.len();
                if !shift {
                    self.sel_anchor = self.cursor_pos;
                }
                self.ensure_cursor_visible();
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_BACKSPACE => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.text_base.text.remove(self.cursor_pos);
                    self.sel_anchor = self.cursor_pos;
                }
                self.ensure_cursor_visible();
                self.suggest = self.text_base.text.len() >= 2;
                self.text_base.base.mark_dirty();
                if self.has_selection() || self.cursor_pos > 0 {
                    EventResponse::CHANGED
                } else {
                    EventResponse::CONSUMED
                }
            }
            KEY_ESCAPE => {
                self.suggest = false;
                self.text_base.base.mark_dirty();
                EventResponse::CONSUMED
            }
            KEY_ENTER => {
                EventResponse::SUBMIT
            }
            _ => EventResponse::IGNORED,
        }
    }

    fn handle_blur(&mut self) {
        self.base_mut().focused = false;
        self.suggest = false;
        self.base_mut().mark_dirty();
    }
}

/// Helper: convert bytes to lowercase
fn to_lower_u8(s: &[u8]) -> Vec<u8> {
    let mut r = Vec::with_capacity(s.len());
    for &b in s {
        if b >= b'A' && b <= b'Z' {
            r.push(b + 32);
        } else {
            r.push(b);
        }
    }
    r
}
