use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct StatusIndicator {
    pub(crate) text_base: TextControlBase,
}

impl StatusIndicator {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for StatusIndicator {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::StatusIndicator }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let tc = crate::theme::colors();
        let dot_color = match b.state {
            0 => tc.text_disabled,
            1 => tc.success,
            2 => tc.warning,
            _ => tc.destructive,
        };
        let dot_size = crate::theme::scale(10);
        let dot_radius = crate::theme::scale(5);
        crate::draw::fill_rounded_rect(surface, p.x, p.y + crate::theme::scale_i32(2), dot_size, dot_size, dot_radius, dot_color);
        if !self.text_base.text.is_empty() {
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            let fid = self.text_base.text_style.font_id;
            crate::draw::draw_text_ex(surface, p.x + crate::theme::scale_i32(14), p.y, tc.text, &self.text_base.text, fid, fs);
        }
    }
}
