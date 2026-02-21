use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Checkbox {
    pub(crate) text_base: TextControlBase,
}

impl Checkbox {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Checkbox {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Checkbox }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let checked = self.text_base.base.state != 0;
        let bg = if checked { tc.accent } else { tc.control_bg };
        crate::draw::fill_rounded_rect(surface, x, y, crate::theme::CHECKBOX_SIZE, crate::theme::CHECKBOX_SIZE, 4, bg);
        if !checked {
            crate::draw::draw_rounded_border(surface, x, y, crate::theme::CHECKBOX_SIZE, crate::theme::CHECKBOX_SIZE, 4, tc.input_border);
        }
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, x + crate::theme::CHECKBOX_SIZE as i32 + 6, y + 2, tc.text, &self.text_base.text, self.text_base.text_style.font_size);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Toggle checked state
        self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }
}
