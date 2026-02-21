use crate::control::{Control, ControlBase, TextControlBase, ControlKind, EventResponse};

pub struct ContextMenu {
    pub(crate) text_base: TextControlBase,
}

impl ContextMenu {
    pub fn new(text_base: TextControlBase) -> Self {
        let mut cm = Self { text_base };
        // Start hidden
        cm.text_base.base.visible = false;
        cm
    }
}

impl Control for ContextMenu {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::ContextMenu }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let tc = crate::theme::colors();

        // Count items from pipe-separated text
        let items: alloc::vec::Vec<&[u8]> = self.text_base.text.split(|&b| b == b'|').collect();
        let item_count = items.len().max(1);
        let h = (item_count as u32) * 28 + 8; // 28px per item + 8px padding

        // Background + border
        crate::draw::fill_rounded_rect(surface, x, y, w, h, 6, tc.control_bg);
        crate::draw::draw_rounded_border(surface, x, y, w, h, 6, tc.card_border);

        // Render each item
        let selected = self.text_base.base.state;
        for (i, item_text) in items.iter().enumerate() {
            let iy = y + 4 + (i as i32) * 28;

            // Highlight selected item
            if i as u32 == selected {
                crate::draw::fill_rect(surface, x + 4, iy, w - 8, 28, tc.accent);
            }

            // Item text
            if !item_text.is_empty() {
                let text_color = if i as u32 == selected { 0xFFFFFFFF } else { tc.text };
                crate::draw::draw_text(surface, x + 12, iy + 6, text_color, item_text);
            }
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        let item_idx = ((ly - 4) / 28).max(0) as u32;
        self.text_base.base.state = item_idx;
        // Hide after selection
        self.text_base.base.visible = false;
        EventResponse::CLICK
    }

    fn handle_blur(&mut self) {
        // Hide context menu when focus leaves
        self.text_base.base.visible = false;
        self.text_base.base.dirty = true;
    }

    fn accepts_focus(&self) -> bool { true }
}
