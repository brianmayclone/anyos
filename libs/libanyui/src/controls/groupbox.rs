use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct GroupBox {
    pub(crate) text_base: TextControlBase,
}

impl GroupBox {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for GroupBox {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::GroupBox }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::card_corner();
        let inset = crate::theme::scale_i32(8);
        let inset_u = crate::theme::scale(8);

        // Bottom shadow line
        let border_h = if h > inset_u { h - inset_u } else { 1 };
        crate::draw::draw_bottom_shadow(surface, x, y + inset, w, border_h, corner, crate::theme::darken(tc.card_border, 10));

        // Group border
        crate::draw::draw_rounded_border(surface, x, y + inset, w, border_h, corner, tc.card_border);

        // Title label (overlaps top border)
        if !self.text_base.text.is_empty() {
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            let (tw, _) = crate::draw::text_size_at(&self.text_base.text, fs);
            let label_h = crate::theme::scale(16);
            crate::draw::fill_rect(surface, x + inset, y, tw + crate::theme::scale(8), label_h, tc.window_bg);
            crate::draw::draw_text_sized(surface, x + crate::theme::scale_i32(12), y, tc.text_secondary, &self.text_base.text, fs);
        }
    }
}
