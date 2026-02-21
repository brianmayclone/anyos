use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct Toggle {
    pub(crate) text_base: TextControlBase,
}

impl Toggle {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for Toggle {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Toggle }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        let on = self.text_base.base.state != 0;
        let track_color = if on { tc.toggle_on } else { tc.toggle_off };
        crate::draw::fill_rounded_rect(surface, x, y, crate::theme::TOGGLE_WIDTH, crate::theme::TOGGLE_HEIGHT, crate::theme::TOGGLE_HEIGHT / 2, track_color);
        let thumb_x = if on { x + (crate::theme::TOGGLE_WIDTH - crate::theme::TOGGLE_THUMB_SIZE - 2) as i32 } else { x + 2 };
        let thumb_y = y + 2;
        crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y, crate::theme::TOGGLE_THUMB_SIZE, crate::theme::TOGGLE_THUMB_SIZE, crate::theme::TOGGLE_THUMB_SIZE / 2, tc.toggle_thumb);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Flip the on/off state
        self.text_base.base.state = if self.text_base.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }
}
