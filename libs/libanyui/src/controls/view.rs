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
            let b = self.base();
            let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
            crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, b.color);
        }
    }
}
