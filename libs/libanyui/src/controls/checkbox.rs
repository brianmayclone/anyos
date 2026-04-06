use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};

pub struct Checkbox {
    pub(crate) text_base: TextControlBase,
}

impl Checkbox {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for Checkbox {
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
        ControlKind::Checkbox
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y) = (ctx.x, ctx.y);
        let tc = crate::theme::colors();
        let checked = b.state != 0;
        let sz = crate::theme::scale(crate::theme::checkbox_size());
        let corner = crate::theme::scale(5);

        let palette = if checked {
            crate::controls::chrome::accent_palette(tc.accent, ctx.hovered, false, ctx.disabled)
        } else {
            crate::controls::chrome::field_palette(0, ctx.hovered, ctx.focused, ctx.disabled)
        };

        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, sz, sz, corner, palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, sz, sz, corner, palette);

        // Checkmark — scaled diagonal lines forming a check shape
        if checked {
            let cm = tc.check_mark;
            let s = |v: i32| crate::theme::scale_i32(v);
            let ps = crate::theme::scale(2);
            // Short leg (bottom-left to center-bottom)
            crate::draw::fill_rect(surface, x + s(4), y + s(8), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(5), y + s(9), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(6), y + s(10), ps, ps, cm);
            // Long leg (center-bottom to top-right)
            crate::draw::fill_rect(surface, x + s(7), y + s(9), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(8), y + s(8), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(9), y + s(7), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(10), y + s(6), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(11), y + s(5), ps, ps, cm);
            crate::draw::fill_rect(surface, x + s(12), y + s(4), ps, ps, cm);
        }

        // Label text
        let text_color = if ctx.disabled {
            tc.text_disabled
        } else {
            tc.text
        };
        if !self.text_base.text.is_empty() {
            let font_size = crate::draw::scale_font(self.text_base.text_style.font_size);
            crate::draw::draw_text_sized(
                surface,
                x + sz as i32 + crate::theme::scale_i32(6),
                y + crate::theme::scale_i32(2),
                text_color,
                &self.text_base.text,
                font_size,
            );
        }
    }

    fn is_interactive(&self) -> bool {
        !self.text_base.base.disabled
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }

    fn handle_key_down(&mut self, keycode: u32, _char_code: u32, _modifiers: u32) -> EventResponse {
        if keycode == crate::control::KEY_SPACE || keycode == crate::control::KEY_ENTER {
            self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }
}
