use crate::control::{Control, ControlBase, ControlKind, EventResponse};

pub struct TabBar {
    pub(crate) base: ControlBase,
}

impl TabBar {
    pub fn new(base: ControlBase) -> Self { Self { base } }
}

impl Control for TabBar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TabBar }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, 0xFF2C2C2E);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Compute which tab was clicked based on x position
        // Simple: divide width by number of children (tabs)
        let num_tabs = self.base.children.len() as i32;
        if num_tabs > 0 {
            let tab_width = self.base.w as i32 / num_tabs;
            if tab_width > 0 {
                let tab_idx = (lx / tab_width).max(0).min(num_tabs - 1) as u32;
                self.base.state = tab_idx;
            }
        }
        EventResponse::CHANGED
    }
}
