use crate::control::{Control, ControlBase, ControlKind};

pub struct Badge {
    pub(crate) base: ControlBase,
}

impl Badge {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Badge {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Badge }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_badge(win, x, y, self.base.state, self.base.color);
    }
}
