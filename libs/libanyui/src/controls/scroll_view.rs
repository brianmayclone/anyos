use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct ScrollView {
    pub(crate) base: ControlBase,
    pub(crate) scroll_y: i32,
}

impl ScrollView {
    pub fn new(base: ControlBase) -> Self { Self { base, scroll_y: 0 } }
}

impl Control for ScrollView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ScrollView }

    fn render(&self, _win: u32, _ax: i32, _ay: i32) {
        // Transparent container — children rendered by tree walker.
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        self.scroll_y = (self.scroll_y + delta * 16).max(0);
        self.base.state = self.scroll_y as u32;
        EventResponse::CHANGED
    }
}
