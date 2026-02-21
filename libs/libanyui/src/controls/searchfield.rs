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
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, crate::theme::INPUT_CORNER, tc.input_bg);
        crate::draw::draw_rounded_border(surface, x, y, self.text_base.base.w, self.text_base.base.h, crate::theme::INPUT_CORNER, tc.input_border);
        let text_color = if self.text_base.text_style.text_color != 0 { self.text_base.text_style.text_color } else { tc.text };
        crate::draw::draw_text_sized(surface, x + 8, y + 6, text_color, &self.text_base.text, self.text_base.text_style.font_size);
    }

    fn is_interactive(&self) -> bool { true }
    fn accepts_focus(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.cursor_pos = self.text_base.text.len();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse {
        if char_code >= 0x20 && char_code < 0x7F {
            let ch = char_code as u8;
            if self.cursor_pos > self.text_base.text.len() {
                self.cursor_pos = self.text_base.text.len();
            }
            self.text_base.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if keycode == 0x0E || char_code == 0x08 {
            if self.cursor_pos > 0 && !self.text_base.text.is_empty() {
                self.cursor_pos -= 1;
                self.text_base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == 0x4B {
            if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            EventResponse::CONSUMED
        } else if keycode == 0x4D {
            if self.cursor_pos < self.text_base.text.len() { self.cursor_pos += 1; }
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.cursor_pos = self.text_base.text.len();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
    }
}
