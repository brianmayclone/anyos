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
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);

        // Background (only if set_color() was called with a non-zero color)
        if b.color != 0 {
            crate::draw::fill_rect(surface, x, y, w, h, b.color);
        }

        // Text color: set_text_color() > theme default
        let text_color = if self.text_base.text_style.text_color != 0 {
            self.text_base.text_style.text_color
        } else {
            crate::theme::colors().text
        };
        let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
        let fid = self.text_base.text_style.font_id;
        let align = b.state; // 0=left, 1=center, 2=right
        let pad_left = crate::theme::scale_i32(b.padding.left);
        let pad_right = crate::theme::scale_i32(b.padding.right);
        let pad_top = crate::theme::scale_i32(b.padding.top);

        // Handle multiline text (split on '\n')
        let text = &self.text_base.text;
        let text_x = x + pad_left;
        let text_w = w as i32 - pad_left - pad_right;
        let mut line_y = y + pad_top;
        let line_h = fs as i32 + crate::theme::scale_i32(2);
        let mut start = 0;
        loop {
            let end = text[start..].iter().position(|&b| b == b'\n').map(|p| start + p).unwrap_or(text.len());
            let line = &text[start..end];

            let tx = if align == 1 {
                // Center
                let (tw, _) = crate::draw::measure_text_ex(line, fid, fs);
                text_x + (text_w - tw as i32) / 2
            } else if align == 2 {
                // Right
                let (tw, _) = crate::draw::measure_text_ex(line, fid, fs);
                text_x + text_w - tw as i32
            } else {
                text_x
            };

            crate::draw::draw_text_ex(surface, tx, line_y, text_color, line, fid, fs);
            line_y += line_h;

            if end >= text.len() { break; }
            start = end + 1;
        }
    }
}
