use crate::control::{prepare_render, Control, ControlBase, ControlKind};

pub struct ProgressBar {
    pub(crate) base: ControlBase,
}

impl ProgressBar {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for ProgressBar {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::ProgressBar
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();
        let r = h / 2;

        crate::controls::chrome::draw_surface(
            surface,
            x,
            y,
            w,
            h,
            r,
            crate::controls::chrome::field_palette(0, ctx.hovered, false, ctx.disabled),
        );

        let val = b.state.min(100);
        let fill_w = (w.saturating_sub(4) as u64 * val as u64 / 100) as u32;
        if fill_w > 0 && h > 4 {
            crate::controls::chrome::draw_surface(
                surface,
                x + 2,
                y + 2,
                fill_w,
                h - 4,
                r.saturating_sub(2),
                crate::controls::chrome::accent_palette(
                    tc.accent,
                    ctx.hovered,
                    false,
                    ctx.disabled,
                ),
            );
        }
    }
}
