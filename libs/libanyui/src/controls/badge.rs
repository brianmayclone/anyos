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
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let bg = if self.text_base.base.color != 0 { self.text_base.base.color } else { tc.badge_red };
        crate::draw::fill_rounded_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, self.text_base.base.h / 2, bg);
    }
}
