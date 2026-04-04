use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Button {
    pub(crate) text_base: TextControlBase,
    pressed: bool,
}

impl Button {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base, pressed: false } }
}

impl Control for Button {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Button }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;

        // Calculate actual button width: if auto_size is set, measure text and add padding
        let button_w = if b.auto_size {
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            let (tw, _th) = crate::draw::text_size_at(&self.text_base.text, font_size);
            // Button padding: 12px left + 12px right (24px total)
            (tw + 24).min(0xFFFF) as u32
        } else {
            b.w
        };

        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, button_w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let disabled = b.disabled;
        let hovered = b.hovered;
        let focused = b.focused;
        let custom = b.color;
        let corner = crate::theme::button_corner();

        // Background color: pressed > hovered > normal, with custom color support
        let bg = if disabled {
            if custom != 0 { crate::theme::darken(custom, 20) } else { tc.control_bg }
        } else if self.pressed {
            if custom != 0 { crate::theme::darken(custom, 30) } else { tc.control_pressed }
        } else if hovered {
            if custom != 0 { crate::theme::lighten(custom, 12) } else { tc.control_hover }
        } else if custom != 0 {
            custom
        } else {
            tc.control_bg
        };

        // Border: draw a filled rounded rect in border color, then the body 1px inset
        let border_color = if disabled {
            crate::theme::darken(bg, 10)
        } else {
            crate::theme::darken(bg, 25)
        };
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, border_color);

        // Main button body (2px inset for visible border)
        if w > 4 && h > 4 {
            let inner_corner = if corner > 2 { corner - 2 } else { 0 };
            crate::draw::fill_rounded_rect(surface, x + 2, y + 2, w - 4, h - 4, inner_corner, bg);
        }

        // Focus ring
        if focused && !disabled {
            crate::draw::draw_focus_ring(surface, x, y, w, h, corner, tc.accent);
        }

        // Text color
        let text_color = if disabled {
            tc.text_disabled
        } else if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            tc.text
        };
        let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
        let (tw, _th) = crate::draw::text_size_at(&self.text_base.text, font_size);
        let tx = x + (w as i32 - tw as i32) / 2;
        let ty = y + (h as i32 - font_size as i32) / 2;
        crate::draw::draw_text_sized(surface, tx, ty, text_color, &self.text_base.text, font_size);
    }

    fn is_interactive(&self) -> bool { !self.text_base.base.disabled }

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
