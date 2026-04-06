use crate::control::{prepare_render, Control, ControlBase, ControlKind, TextControlBase};

pub struct Alert {
    pub(crate) text_base: TextControlBase,
}

impl Alert {
    pub fn new(text_base: TextControlBase) -> Self {
        Self { text_base }
    }
}

impl Control for Alert {
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
        ControlKind::Alert
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let ctx = prepare_render(b, ax, ay);
        let (x, y, w, h) = (ctx.x, ctx.y, ctx.w, ctx.h);
        let tc = crate::theme::colors();

        // Dark overlay behind the alert
        crate::draw::fill_rect(surface, x, y, w, h, 0xCC000000);

        let card_w = w.min(crate::theme::scale(320));
        let card_h = h.min(crate::theme::scale(180));
        let cx = x + (w as i32 - card_w as i32) / 2;
        let cy = y + (h as i32 - card_h as i32) / 2;
        let corner = crate::theme::alert_corner();

        // SDF shadow (Alert is rare and small — SDF cost acceptable)
        crate::draw::draw_shadow_rounded_rect(
            surface,
            cx,
            cy,
            card_w,
            card_h,
            corner as i32,
            0,
            crate::theme::popup_shadow_offset_y(),
            crate::theme::popup_shadow_spread(),
            crate::theme::POPUP_SHADOW_ALPHA,
        );

        crate::controls::chrome::draw_surface(
            surface,
            cx,
            cy,
            card_w,
            card_h,
            corner,
            crate::controls::chrome::card_palette(false),
        );

        if !self.text_base.text.is_empty() {
            let fs = crate::draw::scale_font(crate::theme::FONT_SIZE_LARGE);
            let text_pad = crate::theme::scale_i32(20);
            crate::draw::draw_text_sized(
                surface,
                cx + text_pad,
                cy + text_pad,
                tc.text,
                &self.text_base.text,
                fs,
            );
        }
    }
}
