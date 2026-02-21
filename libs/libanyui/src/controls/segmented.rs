use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct SegmentedControl {
    pub(crate) base: ControlBase,
}

impl SegmentedControl {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for SegmentedControl {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::SegmentedControl }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, 0xFF3A3A3C);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Select segment based on click position
        let num_segments = self.base.children.len().max(1) as i32;
        let seg_width = self.base.w as i32 / num_segments;
        if seg_width > 0 {
            let seg_idx = (lx / seg_width).max(0).min(num_segments - 1) as u32;
            self.base.state = seg_idx;
        }
        EventResponse::CHANGED
    }
}
