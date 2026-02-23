use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct Alert {
    pub(crate) text_base: TextControlBase,
}

impl Alert {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Alert {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Alert }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();

        // Dark overlay behind the alert
        crate::draw::fill_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, 0xCC000000);

        let card_w = self.text_base.base.w.min(320);
        let card_h = self.text_base.base.h.min(180);
        let cx = x + (self.text_base.base.w as i32 - card_w as i32) / 2;
        let cy = y + (self.text_base.base.h as i32 - card_h as i32) / 2;
        let corner = crate::theme::ALERT_CORNER;

        // SDF shadow (Alert is rare and small — SDF cost acceptable)
        crate::draw::draw_shadow_rounded_rect(
            surface, cx, cy, card_w, card_h, corner as i32,
            0, crate::theme::POPUP_SHADOW_OFFSET_Y,
            crate::theme::POPUP_SHADOW_SPREAD,
            crate::theme::POPUP_SHADOW_ALPHA,
        );

        // Card body + border
        crate::draw::fill_rounded_rect(surface, cx, cy, card_w, card_h, corner, tc.card_bg);
        crate::draw::draw_rounded_border(surface, cx, cy, card_w, card_h, corner, tc.card_border);

        // Top highlight for depth
        crate::draw::draw_top_highlight(surface, cx, cy, card_w, corner, crate::theme::lighten(tc.card_bg, 10));

        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, cx + 20, cy + 20, tc.text, &self.text_base.text, crate::theme::FONT_SIZE_LARGE);
        }
    }
}
