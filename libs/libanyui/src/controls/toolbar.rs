use crate::control::{Control, ControlBase, ControlKind};

pub struct Toolbar {
    pub(crate) base: ControlBase,
}

impl Toolbar {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Toolbar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Toolbar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::draw::fill_rect(surface, x, y, self.base.w, self.base.h, 0xFF2C2C2E);
    }
}
