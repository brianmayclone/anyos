use crate::control::{prepare_render, Control, ControlBase, ControlKind, TextControlBase};

pub struct Badge {
    pub(crate) text_base: TextControlBase,
}

impl Badge {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for Badge {
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
        ControlKind::Badge
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let tc = crate::theme::colors();
        let bg = if b.color != 0 { b.color } else { tc.badge_red };
        crate::controls::chrome::draw_surface(
            surface,
            ctx.x,
            ctx.y,
            ctx.w,
            ctx.h,
            ctx.h / 2,
            crate::controls::chrome::accent_palette(bg, ctx.hovered, false, ctx.disabled),
        );
    }
}
