use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct Tooltip {
    pub(crate) text_base: TextControlBase,
}

impl Tooltip {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Tooltip {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Tooltip }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let corner = crate::theme::TOOLTIP_CORNER;

        // SDF shadow (Tooltip is rare — only one visible at a time)
        crate::draw::draw_shadow_rounded_rect(
            surface, x, y, w, h, corner as i32,
            0, 2, 6, 40,
        );

        // Body + border
        crate::draw::fill_rounded_rect(surface, x, y, w, h, corner, tc.sidebar_bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, corner, tc.card_border);

        if !self.text_base.text.is_empty() {
            crate::draw::draw_text(surface, x + 8, y + 4, tc.text, &self.text_base.text);
        }
    }
}
