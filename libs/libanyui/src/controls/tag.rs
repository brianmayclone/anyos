use crate::control::{
    prepare_render, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};

pub struct Tag {
    pub(crate) text_base: TextControlBase,
}

impl Tag {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for Tag {
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
        ControlKind::Tag
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let bg = if b.color != 0 {
            b.color
        } else {
            crate::theme::colors().accent
        };
        crate::controls::chrome::draw_surface(
            surface,
            ctx.x,
            ctx.y,
            ctx.w,
            ctx.h,
            ctx.h / 2,
            crate::controls::chrome::accent_palette(bg, ctx.hovered, false, ctx.disabled),
        );
        if !self.text_base.text.is_empty() {
            let text_color = if self.text_base.text_style.text_color != 0 {
                self.text_base.text_style.text_color
            } else {
                0xFFFFFFFF
            };
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            let fid = self.text_base.text_style.font_id;
            crate::draw::draw_text_ex(
                surface,
                ctx.x + crate::theme::scale_i32(8),
                ctx.y + crate::theme::scale_i32(4),
                text_color,
                &self.text_base.text,
                fid,
                fs,
            );
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
