use crate::control::{
    ChildLayout, Control, ControlBase, ControlKind, EventResponse, TextControlBase,
};
use alloc::vec::Vec;

pub const HEADER_HEIGHT: u32 = 32;

pub struct Expander {
    pub(crate) text_base: TextControlBase,
    /// Height when expanded (saved on collapse, restored on expand).
    expanded_height: u32,
}

impl Expander {
    pub fn new(text_base: TextControlBase) -> Self {
        // Default to expanded (state = 1)
        let h = text_base.base.h;
        let mut e = Self {
            text_base,
            expanded_height: h,
        };
        e.text_base.base.state = 1;
        e
    }
}

impl Control for Expander {
    fn base(&self) -> &ControlBase {
        &self.text_base.base
    }
    fn base_mut(&mut self) -> &mut ControlBase {
        &mut self.text_base.base
    }
    fn text_base(&self) -> Option<&TextControlBase> {
        Some(&self.text_base)
    }
    fn text_base_mut(&mut self) -> Option<&mut TextControlBase> {
        Some(&mut self.text_base)
    }
    fn kind(&self) -> ControlKind {
        ControlKind::Expander
    }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let b = &self.text_base.base;
        let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
        let (x, y, w, h) = (p.x, p.y, p.w, p.h);
        let tc = crate::theme::colors();
        let expanded = b.state != 0;
        let hdr_h = crate::theme::scale(HEADER_HEIGHT);
        let corner = crate::theme::scale(8);

        let header_palette = crate::controls::chrome::neutral_palette(b.hovered, false, b.disabled);
        crate::controls::chrome::draw_surface(surface, x, y, w, hdr_h, corner, header_palette);

        if expanded && h > hdr_h + 4 {
            let body_y = y + hdr_h as i32 + crate::theme::scale_i32(6);
            let body_h = h - hdr_h - crate::theme::scale(6);
            crate::controls::chrome::draw_card(surface, x, body_y, w, body_h, corner, false);
        }

        // Disclosure triangle (scaled)
        let tri_x = x + crate::theme::scale_i32(12);
        let tri_y = y + crate::theme::scale_i32(10);
        let tri_rows = crate::theme::scale_i32(6);
        if expanded {
            // Pointing down
            for row in 0..tri_rows {
                let half = tri_rows - 1 - row;
                crate::draw::fill_rect(
                    surface,
                    tri_x - half,
                    tri_y + row,
                    (half * 2 + 1) as u32,
                    1,
                    tc.text,
                );
            }
        } else {
            // Pointing right
            let half_max = tri_rows / 2;
            for row in 0..tri_rows {
                let half = if row < half_max {
                    row
                } else {
                    tri_rows - 1 - row
                };
                crate::draw::fill_rect(
                    surface,
                    tri_x,
                    tri_y + row,
                    (half + 1) as u32 * 2,
                    1,
                    tc.text,
                );
            }
        }

        // Header text
        let text = &self.text_base.text;
        if !text.is_empty() {
            let fs = crate::draw::scale_font(self.text_base.text_style.font_size);
            crate::draw::draw_text_sized(
                surface,
                x + crate::theme::scale_i32(30),
                y + crate::theme::scale_i32(8),
                tc.text,
                text,
                fs,
            );
        }
    }

    fn set_size(&mut self, w: u32, h: u32) {
        let b = &mut self.text_base.base;
        if b.w != w || b.h != h {
            b.w = w;
            b.h = h;
            b.mark_dirty();
            // Track expanded height when in expanded state
            if b.state != 0 {
                self.expanded_height = h;
            }
        }
    }

    fn is_interactive(&self) -> bool {
        true
    }

    fn handle_click(&mut self, _lx: i32, ly: i32, _button: u32) -> EventResponse {
        if ly < HEADER_HEIGHT as i32 {
            let expanded = self.text_base.base.state != 0;
            if expanded {
                // Collapsing: save current height, shrink to header only
                self.expanded_height = self.text_base.base.h;
                self.text_base.base.h = HEADER_HEIGHT;
                self.text_base.base.state = 0;
            } else {
                // Expanding: restore saved height
                self.text_base.base.h = self.expanded_height;
                self.text_base.base.state = 1;
            }
            self.text_base.base.mark_dirty();
            crate::mark_needs_layout();
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn layout_children(
        &self,
        _controls: &[alloc::boxed::Box<dyn Control>],
    ) -> Option<Vec<ChildLayout>> {
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
