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
        let bx = ax + self.text_base.base.x;
        let by = ay + self.text_base.base.y;
        let color = if self.text_base.base.color != 0 {
            self.text_base.base.color
        } else {
            crate::theme::colors().text
        };
        let fs = self.text_base.text_style.font_size;
        let align = self.text_base.base.state; // 0=left, 1=center, 2=right

        // Handle multiline text (split on '\n')
        let text = &self.text_base.text;
        let mut line_y = by;
        let line_h = fs as i32 + 2;
        let mut start = 0;
        loop {
            let end = text[start..].iter().position(|&b| b == b'\n').map(|p| start + p).unwrap_or(text.len());
            let line = &text[start..end];

            let tx = if align == 1 {
                // Center
                let (tw, _) = crate::draw::text_size_at(line, fs);
                bx + (self.text_base.base.w as i32 - tw as i32) / 2
            } else if align == 2 {
                // Right
                let (tw, _) = crate::draw::text_size_at(line, fs);
                bx + self.text_base.base.w as i32 - tw as i32
            } else {
                bx
            };

            crate::draw::draw_text_sized(surface, tx, line_y, color, line, fs);
            line_y += line_h;

            if end >= text.len() { break; }
            start = end + 1;
        }
    }
}
