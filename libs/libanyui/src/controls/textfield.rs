use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct TextField {
    pub(crate) base: ControlBase,
    pub(crate) cursor_pos: usize,
    pub(crate) focused: bool,
}

impl TextField {
    pub fn new(base: ControlBase) -> Self {
        Self { base, cursor_pos: 0, focused: false }
    }
}

impl Control for TextField {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TextField }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let focus_flag = if self.focused { 1u32 } else { 0 };
        crate::uisys::render_textfield(win, x, y, self.base.w, self.base.h, &self.base.text, focus_flag, self.cursor_pos as u32);
    }

    fn is_interactive(&self) -> bool { true }
    fn accepts_focus(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.cursor_pos = self.base.text.len();
        EventResponse::CONSUMED
    }

    fn handle_key_down(&mut self, keycode: u32, char_code: u32) -> EventResponse {
        if char_code >= 0x20 && char_code < 0x7F {
            let ch = char_code as u8;
            if self.cursor_pos > self.base.text.len() {
                self.cursor_pos = self.base.text.len();
            }
            self.base.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
            EventResponse::CHANGED
        } else if keycode == 0x0E || char_code == 0x08 {
            if self.cursor_pos > 0 && !self.base.text.is_empty() {
                self.cursor_pos -= 1;
                self.base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == 0x53 || char_code == 0x7F {
            if self.cursor_pos < self.base.text.len() {
                self.base.text.remove(self.cursor_pos);
                EventResponse::CHANGED
            } else {
                EventResponse::CONSUMED
            }
        } else if keycode == 0x4B {
            if self.cursor_pos > 0 { self.cursor_pos -= 1; }
            EventResponse::CONSUMED
        } else if keycode == 0x4D {
            if self.cursor_pos < self.base.text.len() { self.cursor_pos += 1; }
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_focus(&mut self) {
        self.focused = true;
        self.cursor_pos = self.base.text.len();
    }

    fn handle_blur(&mut self) {
        self.focused = false;
    }
}
