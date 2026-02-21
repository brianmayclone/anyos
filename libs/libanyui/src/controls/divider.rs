use crate::control::{Control, ControlBase, ControlKind};

pub struct Divider {
    pub(crate) base: ControlBase,
}

impl Divider {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Divider {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Divider }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        if self.base.h <= 1 {
            crate::draw::fill_rect(surface, x, y, self.base.w, 1, tc.separator);
        } else {
            crate::draw::fill_rect(surface, x, y, 1, self.base.h, tc.separator);
        }
    }
}
