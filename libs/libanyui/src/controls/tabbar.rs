use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct TabBar {
    pub(crate) text_base: TextControlBase,
}

impl TabBar {
    pub fn new(text_base: TextControlBase) -> Self { Self { text_base } }
}

impl Control for TabBar {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&crate::control::TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut crate::control::TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::TabBar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        crate::draw::fill_rect(surface, x, y, self.text_base.base.w, self.text_base.base.h, 0xFF2C2C2E);
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, lx: i32, _ly: i32, _button: u32) -> EventResponse {
        // Compute which tab was clicked based on x position
        // Simple: divide width by number of children (tabs)
        let num_tabs = self.text_base.base.children.len() as i32;
        if num_tabs > 0 {
            let tab_width = self.text_base.base.w as i32 / num_tabs;
            if tab_width > 0 {
                let tab_idx = (lx / tab_width).max(0).min(num_tabs - 1) as u32;
                self.text_base.base.state = tab_idx;
            }
        }
        EventResponse::CHANGED
    }
}
