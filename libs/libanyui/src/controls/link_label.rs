use crate::control::{Control, ControlBase, ControlKind, EventResponse, TextControlBase};

pub struct LinkLabel {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl LinkLabel {
    pub fn new(text_base: TextControlBase) -> Self {
        Self {
            text_base,
            pressed: false,
        }
    }
}

impl Control for LinkLabel {
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
        ControlKind::LinkLabel
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let fid = self.text_base.text_style.font_id;
        let (tw, th) = crate::draw::measure_text_ex(&self.text_base.text, fid, font_size);

        let radius = if b.style.radius != 0 {
            crate::theme::scale(b.style.radius)
        } else {
            0
        }
        .min(h.saturating_sub(2) / 2);
        if b.color != 0 {
            crate::draw::fill_rounded_rect(surface, x, y, w, h, radius, b.color);
        } else if b.hovered && b.style.hover_bg != 0 && !b.disabled {
            crate::draw::fill_rounded_rect(surface, x, y, w, h, radius, b.style.hover_bg);
        }

        let text_color = if b.disabled {
            tc.text_disabled
        } else if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else if self.pressed {
            tc.accent_hover
        } else {
            tc.accent
        };
        let pad_left = crate::theme::scale_i32(b.padding.left);
        let tx = x + pad_left;
        let ty = y + ((h as i32 - th as i32) / 2).max(0);

        crate::draw::draw_text_ex(
            surface,
            tx,
            ty,
            text_color,
            &self.text_base.text,
            fid,
            font_size,
        );

        if b.hovered && !b.disabled && b.style.hover_bg == 0 {
            let underline_y = ty + th as i32 + crate::theme::scale_i32(1);
            let line_h = crate::theme::scale(1).max(1);
            crate::draw::fill_rect(surface, tx, underline_y, tw, line_h, text_color);
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = true;
        self.text_base.base.mark_dirty();
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = false;
        self.text_base.base.mark_dirty();
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
