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
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let tc = crate::theme::colors();
        if b.h <= 1 {
            crate::draw::fill_rect(surface, p.x, p.y, p.w, 1, crate::theme::darken(tc.separator, 10));
            crate::draw::fill_rect(surface, p.x, p.y + 1, p.w, 1, crate::theme::with_alpha(0xFFFFFFFF, 18));
        } else {
            crate::draw::fill_rect(surface, p.x, p.y, 1, p.h, crate::theme::darken(tc.separator, 10));
            crate::draw::fill_rect(surface, p.x + 1, p.y, 1, p.h, crate::theme::with_alpha(0xFFFFFFFF, 18));
        }
    }
}
