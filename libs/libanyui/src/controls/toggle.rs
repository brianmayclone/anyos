use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};

pub struct Toggle {
    pub(crate) text_base: TextControlBase,
}

impl Toggle {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for Toggle {
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
        ControlKind::Toggle
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y) = (ctx.x, ctx.y);
        let tc = crate::theme::colors();
        let on = b.state != 0;

        // Track (theme values are already logical — scale them)
        let tw = crate::theme::scale(crate::theme::toggle_width());
        let th = crate::theme::scale(crate::theme::toggle_height());
        let track_palette = if on {
            crate::controls::chrome::accent_palette(tc.toggle_on, ctx.hovered, false, ctx.disabled)
        } else {
            crate::controls::chrome::field_palette(
                tc.toggle_off,
                ctx.hovered,
                ctx.focused,
                ctx.disabled,
            )
        };
        if ctx.focused && !ctx.disabled {
            crate::controls::chrome::draw_focus(surface, x, y, tw, th, th / 2, track_palette);
        }
        crate::controls::chrome::draw_surface(surface, x, y, tw, th, th / 2, track_palette);

        let thumb_sz = crate::theme::scale(crate::theme::toggle_thumb_size());
        let inset = crate::theme::scale_i32(2);
        let thumb_x = if on {
            x + (tw - thumb_sz) as i32 - inset
        } else {
            x + inset
        };
        let thumb_y = y + inset;
        crate::controls::chrome::draw_surface(
            surface,
            thumb_x,
            thumb_y,
            thumb_sz,
            thumb_sz,
            thumb_sz / 2,
            crate::controls::chrome::neutral_palette(ctx.hovered, false, ctx.disabled),
        );
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
