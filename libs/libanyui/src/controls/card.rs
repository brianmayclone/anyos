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
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.base.w, self.base.h, crate::theme::CARD_CORNER, tc.card_bg);
        crate::draw::draw_rounded_border(surface, x, y, self.base.w, self.base.h, crate::theme::CARD_CORNER, tc.card_border);
    }
}
