use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Checkbox {
    pub(crate) base: ControlBase,
}

impl Checkbox {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Checkbox {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Checkbox }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::uisys::render_checkbox(win, x, y, self.base.state != 0, &self.base.text);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Toggle checked state
        self.base.state = if self.base.state != 0 { 0 } else { 1 };
        EventResponse::CHANGED
    }
}
