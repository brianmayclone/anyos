use crate::control::{prepare_render, Control, ControlBase, ControlKind};

pub struct Card {
    pub(crate) base: ControlBase,
}

impl Card {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for Card {
    fn base(&self) -> &ControlBase {
        &self.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.base
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Card
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let ctx = prepare_render(b, ax, ay);
        let corner = crate::theme::card_corner();

        crate::controls::chrome::draw_card(
            surface,
            ctx.x,
            ctx.y,
            ctx.w,
            ctx.h,
            corner,
            ctx.hovered,
        );
    }
}
