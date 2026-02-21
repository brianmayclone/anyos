use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Stepper {
    pub(crate) base: ControlBase,
}

impl Stepper {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Stepper {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Stepper }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, 0xFF3A3A3C);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        let half = self.base.w as i32 / 2;
        if lx < half {
            if self.base.state > 0 { self.base.state -= 1; }
        } else {
            self.base.state += 1;
        }
        EventResponse::CHANGED
    }
}
