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
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let dot_color = match self.text_base.base.state {
            0 => tc.text_disabled,
            1 => tc.success,
            2 => tc.warning,
            _ => tc.destructive,
        };
        crate::draw::fill_rounded_rect(surface, x, y + 2, 10, 10, 5, dot_color);
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text(surface, x + 14, y, tc.text, &self.text_base.text);
        }
    }
}
