use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct ContextMenu {
    pub(crate) base: ControlBase,
}

impl ContextMenu {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for ContextMenu {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::ContextMenu }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        crate::draw::fill_rounded_rect(surface, x, y, self.base.w, self.base.h, 6, tc.control_bg);
        crate::draw::draw_rounded_border(surface, x, y, self.base.w, self.base.h, 6, tc.card_border);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Select menu item (28px per item)
        let item_idx = (ly / 28).max(0) as u32;
        self.base.state = item_idx;
        EventResponse::CLICK
    }
}
