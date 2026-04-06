use crate::control::{prepare_render, Control, ControlBase, ControlKind};

pub struct Divider {
    pub(crate) base: ControlBase,
}

impl Divider {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for Divider {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Divider
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let ctx = prepare_render(b, ax, ay);
        let tc = crate::theme::colors();
        if b.h <= 1 {
            crate::draw::fill_rect(
                surface,
                ctx.x,
                ctx.y,
                ctx.w,
                1,
                crate::theme::darken(tc.separator, 10),
            );
            crate::draw::fill_rect(
                surface,
                ctx.x,
                ctx.y + 1,
                ctx.w,
                1,
                crate::theme::with_alpha(0xFFFFFFFF, 18),
            );
        } else {
            crate::draw::fill_rect(
                surface,
                ctx.x,
                ctx.y,
                1,
                ctx.h,
                crate::theme::darken(tc.separator, 10),
            );
            crate::draw::fill_rect(
                surface,
                ctx.x + 1,
                ctx.y,
                1,
                ctx.h,
                crate::theme::with_alpha(0xFFFFFFFF, 18),
            );
        }
    }
}
