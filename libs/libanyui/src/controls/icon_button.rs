use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct IconButton {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl IconButton {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base, pressed: false } }
}

impl Control for IconButton {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::IconButton }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let bg = if self.pressed { tc.control_pressed } else { tc.control_bg };
        crate::draw::fill_rounded_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, crate::theme::BUTTON_CORNER, bg);
        if !self.text_base.text.is_empty() {
            let text_color = if self.text_base.text_style.text_color != 0 { self.text_base.text_style.text_color } else { tc.text };
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, self.text_base.text_style.font_size);
            let tx = x + (self.text_base.base.w as i32 - tw as i32) / 2;
            let ty = y + 4;
            crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, self.text_base.text_style.font_size);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
