use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct SplitView {
    pub(crate) base: ControlBase,
    pub(crate) divider_pos: i32,
    dragging: bool,
}

impl SplitView {
    pub fn new(base: ControlBase) -> Self {
        let default_pos = (base.w / 3) as i32;
        Self { base, divider_pos: default_pos, dragging: false }
    }
}

impl Control for SplitView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::SplitView }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rect(surface, x + self.divider_pos, y, 1, self.base.h, tc.separator);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Check if click is near the divider (within 4px)
        if (lx - self.divider_pos).abs() <= 4 {
            self.dragging = true;
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_move(&mut self, lx: i32, _ly: i32) -> EventResponse {
        if self.dragging {
            let min = 50;
            let max = (self.base.w as i32) - 50;
            self.divider_pos = lx.max(min).min(max);
            self.base.state = self.divider_pos as u32;
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if self.dragging {
            self.dragging = false;
            EventResponse::CHANGED
        } else {
            EventResponse::CONSUMED
        }
    }
}
