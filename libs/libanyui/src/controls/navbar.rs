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
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let tc = crate::theme::colors();
        crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, tc.toolbar_bg);
        if !self.text_base.text.is_empty() {
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            crate::draw::draw_text_sized(surface, p.x + crate::theme::scale_i32(12), p.y + crate::theme::scale_i32(8), tc.text, &self.text_base.text, fs);
        }
    }
}
