use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Button {
    pub(crate) base: ControlBase,
    pressed: bool,
}

impl Button {
    pub fn new(base: ControlBase) -> Self { Self { base, pressed: false } }
}

impl Control for Button {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Button }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let pressed_flag: u8 = if self.pressed { 1 } else { 0 };
        crate::uisys::render_button(win, x, y, self.base.w, self.base.h, &self.base.text, pressed_flag, 0);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_mouse_down(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = true;
        EventResponse::CONSUMED
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        self.pressed = false;
        EventResponse::CONSUMED
    }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
