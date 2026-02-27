use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct NavigationBar {
    pub(crate) text_base: TextControlBase,
}

impl NavigationBar {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for NavigationBar {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::NavigationBar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, tc.toolbar_bg);
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, x + 12, y + 8, tc.text, &self.text_base.text, self.text_base.text_style.font_size);
        }
    }
}
