use crate::control::{Control, ControlBase, ControlKind};

pub struct Tooltip {
    pub(crate) base: ControlBase,
}

impl Tooltip {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Tooltip {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Tooltip }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_tooltip(win, x, y, &self.base.text);
    }
}
