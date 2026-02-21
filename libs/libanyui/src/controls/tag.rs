use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Tag {
    pub(crate) base: ControlBase,
}

impl Tag {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Tag {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Tag }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let bg = if self.base.color != 0 { self.base.color } else { 0xFF3A3A3C };
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, bg);
        if !self.base.text.is_empty() {
            crate::uisys::render_label(win, x + 8, y + 4, &self.base.text, 0xFFFFFFFF);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        EventResponse::CLICK
    }
}
