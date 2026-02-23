use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct GroupBox {
    pub(crate) text_base: TextControlBase,
}

impl GroupBox {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for GroupBox {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::GroupBox }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;
        let tc = crate::theme::colors();
        let corner = crate::theme::CARD_CORNER;

        // Bottom shadow line
        crate::draw::draw_bottom_shadow(surface, x, y + 8, w, h - 8, corner, crate::theme::darken(tc.card_border, 10));

        // Group border
        crate::draw::draw_rounded_border(surface, x, y + 8, w, h - 8, corner, tc.card_border);

        // Title label (overlaps top border)
        if !self.text_base.text.is_empty() {
            let (tw, _) = crate::draw::text_size(&self.text_base.text);
            crate::draw::fill_rect(surface, x + 8, y, tw + 8, 16, tc.window_bg);
            crate::draw::draw_text(surface, x + 12, y, tc.text_secondary, &self.text_base.text);
        }
    }
}
