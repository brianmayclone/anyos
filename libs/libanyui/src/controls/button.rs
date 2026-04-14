use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};

pub struct Button {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl Button {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            pressed: false,
        }
    }

    fn current_size(&self) -> (u32, u32) {
        let b = &self.text_base.base;
        let w = if b.auto_size {
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            let (tw, _th) = crate::draw::text_size_at(&self.text_base.text, font_size);
            (tw + 34).min(0xFFFF) as u32
        } else {
            b.w
        };
        (w, b.h)
    }
}

impl Control for Button {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Button
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let (button_w, button_h) = self.current_size();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, button_w, button_h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);

        let tc = crate::theme::colors();
        let corner = crate::theme::button_corner().min(h.saturating_sub(2) / 2);
        let palette = if b.color != 0 {
            crate::controls::chrome::accent_palette(b.color, b.hovered, self.pressed, b.disabled)
        } else {
            crate::controls::chrome::neutral_palette(b.hovered, self.pressed, b.disabled)
        };

        if b.focused && !b.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, w, h, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, w, h, corner, palette);

        let content_offset_y = if self.pressed && !b.disabled { 1 } else { 0 };
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let (tw, th) = crate::draw::text_size_at(&self.text_base.text, font_size);
        let tx = x + ((w as i32 - tw as i32) / 2);
        let ty = y + ((h as i32 - th as i32) / 2) + content_offset_y - 1;
        let text_color = if b.disabled {
            tc.text_disabled
        } else if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else if b.color != 0 {
            0xFFFFFFFF
        } else {
            tc.text
        };
        crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

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

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        if keycode == crate::control::KEY_SPACE || keycode == crate::control::KEY_ENTER {
            EventResponse::CLICK
        } else {
            EventResponse::IGNORED
        }
    }
}
