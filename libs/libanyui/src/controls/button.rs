use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Button {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl Button {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base, pressed: false } }
}

/// Darken a color by subtracting `amount` from each RGB channel.
fn darken(color: u32, amount: u32) -> u32 {
    let a = color & 0xFF000000;
    let r = ((color >> 16) & 0xFF).saturating_sub(amount);
    let g = ((color >> 8) & 0xFF).saturating_sub(amount);
    let b = (color & 0xFF).saturating_sub(amount);
    a | (r << 16) | (g << 8) | b
}

impl Control for Button {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Button }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let custom = self.text_base.base.color;
        let bg = if self.pressed {
            if custom != 0 { darken(custom, 30) } else { tc.control_pressed }
        } else if custom != 0 {
            custom
        } else {
            tc.control_bg
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, crate::theme::BUTTON_CORNER, bg);
        let text_color = if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text
        };
        let font_size = self.text_base.text_style.font_size;
        let (tw, _th) = crate::draw::text_size_at(&self.text_base.text, font_size);
        let tx = x + (w as i32 - tw as i32) / 2;
        let ty = y + (h as i32 - font_size as i32) / 2;
        crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
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
