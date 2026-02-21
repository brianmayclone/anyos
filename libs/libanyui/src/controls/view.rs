use crate::control::{Control, ControlBase, ControlKind};

pub struct View {
    pub(crate) base: ControlBase,
}

impl View {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for View {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::View }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        if self.base.color != 0 {
            let x = ax + self.base.x;
            let y = ay + self.base.y;
            crate::draw::fill_rect(surface, x, y, self.base.w, self.base.h, self.base.color);
        }
    }
}
