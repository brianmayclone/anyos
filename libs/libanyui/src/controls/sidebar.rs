use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct Sidebar {
    pub(crate) base: ControlBase,
}

impl Sidebar {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for Sidebar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Sidebar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::draw::fill_rect(surface, x, y, self.base.w, self.base.h, crate::theme::colors().sidebar_bg);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Select item based on y position (32px per item)
        let item_idx = (ly / 32).max(0) as u32;
        self.base.state = item_idx;
        EventResponse::CHANGED
    }
}
