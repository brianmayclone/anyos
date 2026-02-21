use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct RadioButton {
    pub(crate) text_base: TextControlBase,
}

impl RadioButton {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for RadioButton {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::RadioButton }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let selected = self.text_base.base.state != 0;
        let border_color = if selected { tc.accent } else { tc.input_border };
        crate::draw::fill_rounded_rect(surface, x, y, crate::theme::RADIO_SIZE, crate::theme::RADIO_SIZE, crate::theme::RADIO_SIZE / 2, tc.control_bg);
        crate::draw::draw_rounded_border(surface, x, y, crate::theme::RADIO_SIZE, crate::theme::RADIO_SIZE, crate::theme::RADIO_SIZE / 2, border_color);
        if selected {
            crate::draw::fill_rounded_rect(surface, x + 5, y + 5, 8, 8, 4, tc.accent);
        }
        if !self.text_base.text.is_empty() {
            crate::draw::draw_text_sized(surface, x + crate::theme::RADIO_SIZE as i32 + 6, y + 2, tc.text, &self.text_base.text, self.text_base.text_style.font_size);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.text_base.base.state = 1;
        EventResponse::CHANGED
    }
}
