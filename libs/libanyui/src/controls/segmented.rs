use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct SegmentedControl {
    pub(crate) text_base: TextControlBase,
}

impl SegmentedControl {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for SegmentedControl {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::SegmentedControl }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, crate::theme::BUTTON_CORNER, tc.control_bg);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Select segment based on click position
        let num_segments = self.text_base.base.children.len().max(1) as i32;
        let seg_width = self.text_base.base.w as i32 / num_segments;
        if seg_width > 0 {
            let seg_idx = (lx / seg_width).max(0).min(num_segments - 1) as u32;
            self.text_base.base.state = seg_idx;
        }
        EventResponse::CHANGED
    }
}
