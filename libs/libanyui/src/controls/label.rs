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
        let w = self.text_base.base.w;
        let h = self.text_base.base.h;

        // Background (only if set_color() was called with a non-zero color)
        if self.text_base.base.color != 0 {
            crate::draw::fill_rect(surface, bx, by, w, h, self.text_base.base.color);
        }

        // Text color: set_text_color() > theme default
        let text_color = if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            crate::theme::colors().text
        };
        let fs = self.text_base.text_style.font_size;
        let align = self.text_base.base.state; // 0=left, 1=center, 2=right
        let pad = &self.text_base.base.padding;

        // Handle multiline text (split on '\n')
        let text = &self.text_base.text;
        let text_x = bx + pad.left;
        let text_w = w as i32 - pad.left - pad.right;
        let mut line_y = by + pad.top;
        let line_h = fs as i32 + 2;
        let mut start = 0;
        loop {
            let end = text[start..].iter().position(|&b| b == b'\n').map(|p| start + p).unwrap_or(text.len());
            let line = &text[start..end];

            let tx = if align == 1 {
                // Center
                let (tw, _) = crate::draw::text_size_at(line, fs);
                text_x + (text_w - tw as i32) / 2
            } else if align == 2 {
                // Right
                let (tw, _) = crate::draw::text_size_at(line, fs);
                text_x + text_w - tw as i32
            } else {
                text_x
            };

            crate::draw::draw_text_sized(surface, tx, line_y, text_color, line, fs);
            line_y += line_h;

            if end >= text.len() { break; }
            start = end + 1;
        }
    }
}
