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

        let _ = tc;
        crate::controls::chrome::draw_card(surface, x, y, w, h, corner, b.hovered);
    }
}
