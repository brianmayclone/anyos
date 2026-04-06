use crate::control::{prepare_render, Control, ControlBase, ControlKind, TextControlBase};

pub struct NavigationBar {
    pub(crate) text_base: TextControlBase,
}

impl NavigationBar {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for NavigationBar {
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
        ControlKind::NavigationBar
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let tc = crate::theme::colors();
        crate::draw::fill_rect(surface, ctx.x, ctx.y, ctx.w, ctx.h, tc.toolbar_bg);
        if !self.text_base.text.is_empty() {
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            crate::draw::draw_text_sized(
                surface,
                ctx.x + crate::theme::scale_i32(12),
                ctx.y + crate::theme::scale_i32(8),
                tc.text,
                &self.text_base.text,
                fs,
            );
        }
    }
}
