use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct Badge {
    pub(crate) text_base: TextControlBase,
}

impl Badge {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Badge {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Badge }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let tc = crate::theme::colors();
        let bg = if b.color != 0 { b.color } else { tc.badge_red };
        crate::draw::fill_rounded_rect(surface, p.x, p.y, p.w, p.h, p.h / 2, bg);
    }
}
