use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct SearchField {
    pub(crate) text_base: TextControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
}

impl SearchField {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base, cursor_pos: 0, focused: false }
    }
}

impl Control for SearchField {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::SearchField }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let disabled = self.text_base.base.disabled;
        let hovered = self.text_base.base.hovered;
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
        let icon_x = x + 10;
        let icon_y = y + (h as i32 - 12) / 2;
        crate::draw::fill_rounded_rect(surface, icon_x, icon_y, 10, 10, 5, tc.text_secondary);
        crate::draw::fill_rounded_rect(surface, icon_x + 3, icon_y + 3, 4, 4, 2, tc.input_bg);

        // Text
        let text_color = if disabled { tc.text_disabled } else if self.text_base.text_style.text_color != 0 { self.text_base.text_style.text_color } else { tc.text };
        let text_x = x + 26;
        if self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, text_x, y + 6, tc.text_secondary, b"Search", self.text_base.text_style.font_size);
        } else {
            crate::draw::draw_text_sized(surface, text_x, y + 6, text_color, &self.text_base.text, self.text_base.text_style.font_size);
        }

        // Cursor
        if self.focused {
            let cursor_text = self.cursor_pos.min(self.text_base.text.len());
            let cursor_x_offset = crate::draw::text_width_n(&self.text_base.text, cursor_text) as i32;
            crate::draw::fill_rect(surface, text_x + cursor_x_offset, y + 4, 2, h - 8, tc.accent);
        }
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }
    fn accepts_focus(&self) -> bool { !self.text_base.base.disabled }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.cursor_pos = self.text_base.text.len();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32, _modifiers: u32) -> EventResponse {
        use crate::control::*;
        if keycode == KEY_ENTER {
            return EventResponse::SUBMIT;
        }
        if char_code >= 0x20 && char_code < 0x7F {
            let ch = char_code as u8;
            if self.cursor_pos > self.text_base.text.len() {
                self.cursor_pos = self.text_base.text.len();
            }
            self.text_base.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if keycode == KEY_BACKSPACE {
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == KEY_LEFT {
            if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            EventResponse::CONSUMED
        } else if keycode == KEY_RIGHT {
            if self.cursor_pos < self.text_base.text.len() { self.cursor_pos += 1; }
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.text_base.base.focused = true;
        self.text_base.base.mark_dirty();
        self.cursor_pos = self.text_base.text.len();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
        self.text_base.base.focused = false;
        self.text_base.base.mark_dirty();
    }
}
