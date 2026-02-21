use alloc::vec::Vec;
use crate::control::{Control, ControlBase, TextControlBase, ControlKind, ChildLayout, EventResponse};

pub const HEADER_HEIGHT: u32 = 32;

pub struct Expander {
    pub(crate) text_base: TextControlBase,
}

impl Expander {
    pub fn new(text_base: TextControlBase) -> Self {
        // Default to expanded (state = 1)
        let mut e = Self { text_base };
        e.text_base.base.state = 1;
        e
    }
}

impl Control for Expander {
    fn base(&self) -> &ControlBase { &self.text_base.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.text_base.base }
    fn text_base(&self) -> Option<&TextControlBase> { Some(&self.text_base) }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> { Some(&mut self.text_base) }
    fn kind(&self) -> ControlKind { ControlKind::Expander }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.text_base.base.x;
        let y = ay + self.text_base.base.y;
        let w = self.text_base.base.w;
        let tc = crate::theme::colors();
        let expanded = self.text_base.base.state != 0;

        // Header background
        crate::draw::fill_rect(surface, x, y, w, HEADER_HEIGHT, tc.control_bg);

        // Disclosure triangle
        let tri_x = x + 12;
        let tri_y = y + 10;
        if expanded {
            // ▼ pointing down
            for row in 0..6i32 {
                let half = 5 - row;
                crate::draw::fill_rect(surface, tri_x - half, tri_y + row, (half * 2 + 1) as u32, 1, tc.text);
            }
        } else {
            // ▶ pointing right
            for row in 0..6i32 {
                let half = if row < 3 { row } else { 5 - row };
                crate::draw::fill_rect(surface, tri_x, tri_y + row, (half + 1) as u32 * 2, 1, tc.text);
            }
        }

        // Header text
        let text = &self.text_base.text;
        if !text.is_empty() {
            crate::draw::draw_text(surface, x + 28, y + 8, tc.text, text);
        }

        // Border
        if expanded {
            crate::draw::fill_rect(surface, x, y + HEADER_HEIGHT as i32 - 1, w, 1, tc.card_border);
        } else {
            crate::draw::draw_border(surface, x, y, w, HEADER_HEIGHT, tc.card_border);
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        if ly < HEADER_HEIGHT as i32 {
            let expanded = self.text_base.base.state != 0;
            self.text_base.base.state = if expanded { 0 } else { 1 };
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn layout_children(&self, _controls: &[alloc::boxed::Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        if self.text_base.base.state == 0 {
            // Collapsed: return empty layout (children won't be positioned/rendered)
            return Some(Vec::new());
        }
        // Expanded: use standard dock layout (preserves manual positions).
        // The HEADER_HEIGHT offset is applied in render_tree / hit_test,
        // analogous to ScrollView's scroll_y offset.
        None
    }
}
