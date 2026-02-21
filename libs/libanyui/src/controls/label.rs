use crate::control::{Control, ControlBase, TextControlBase, ControlKind};

pub struct Label {
    pub(crate) text_base: TextControlBase,
}

impl Label {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Label {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Label }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let color = if self.text_base.base.color != 0 {
            self.text_base.base.color
        } else {
            crate::theme::colors().text
        };
        crate::draw::draw_text_sized(surface, x, y, color, &self.text_base.text, self.text_base.text_style.font_size);
    }
}
