use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct TableView {
    pub(crate) base: ControlBase,
    pub(crate) scroll_y: i32,
    pub(crate) row_height: u32,
}

impl TableView {
    pub fn new(base: ControlBase) -> Self {
        Self { base, scroll_y: 0, row_height: 24 }
    }
}

impl Control for TableView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TableView }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = self.base();
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, crate::theme::colors().window_bg);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Select row based on y position
        if self.row_height > 0 {
            let row = ((ly + self.scroll_y) as u32) / self.row_height;
            self.base.state = row;
        }
        EventResponse::CHANGED
    }

    fn handle_scroll(&mut self, delta: i32) -> EventResponse {
        self.scroll_y = (self.scroll_y + delta * 16).max(0);
        EventResponse::CONSUMED
    }
}
