use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Slider {
    pub(crate) base: ControlBase,
    dragging: bool,
}

impl Slider {
    pub fn new(base: ControlBase) -> Self { Self { base, dragging: false } }

    fn value_from_x(&self, local_x: i32) -> u32 {
        let w = self.base.w as i32;
        if w <= 0 { return 0; }
        let clamped = local_x.max(0).min(w);
        ((clamped as u32) * 100) / (w as u32)
    }
}

impl Control for Slider {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Slider }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        let track_y = y + (self.base.h as i32 - 4) / 2;
        crate::draw::fill_rounded_rect(surface, x, track_y, self.base.w, 4, 2, tc.control_bg);
        let val = self.base.state.min(100);
        let fill_w = (self.base.w as u64 * val as u64 / 100) as u32;
        if fill_w > 0 {
            crate::draw::fill_rounded_rect(surface, x, track_y, fill_w, 4, 2, tc.accent);
        }
        let thumb_x = x + fill_w as i32 - 9;
        let thumb_y = y + (self.base.h as i32 - 18) / 2;
        crate::draw::fill_rounded_rect(surface, thumb_x, thumb_y, 18, 18, 9, tc.toggle_thumb);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.dragging = true;
        self.base.state = self.value_from_x(lx);
        EventResponse::CHANGED
    }

    fn handle_mouse_move(&mut self, lx: i32, _ly: i32) -> EventResponse {
        if self.dragging {
            self.base.state = self.value_from_x(lx);
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_up(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if self.dragging {
            self.dragging = false;
            self.base.state = self.value_from_x(lx);
            EventResponse::CHANGED
        } else {
            EventResponse::CONSUMED
        }
    }
}
