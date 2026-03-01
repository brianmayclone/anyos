use crate::control::{Control, ControlBase, ControlKind};

pub struct Card {
    pub(crate) base: ControlBase,
}

impl Card {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Card {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Card }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let corner = crate::theme::card_corner();

        // Bottom shadow line (cheap elevation)
        crate::draw::draw_bottom_shadow(surface, x, y, w, h, corner, crate::theme::darken(tc.card_border, 15));

        // Card body + border
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, tc.card_bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, tc.card_border);

        // Top highlight
        crate::draw::draw_top_highlight(surface, x, y, w, corner, crate::theme::lighten(tc.card_bg, 8));
    }
}
