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

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_card(win, x, y, self.base.w, self.base.h);
    }
}
