//! Toolbar — horizontal bar that lays out children left-to-right with spacing.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, ChildLayout, find_idx};

pub struct Toolbar {
    pub(crate) base: ControlBase,
    /// Horizontal gap between children (pixels). Default: 4.
    pub spacing: u32,
}

impl Toolbar {
    pub fn new(base: ControlBase) -> Self {
        Self { base, spacing: 4 }
    }
}

impl Control for Toolbar {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::Toolbar }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let w = self.base.w;
        let h = self.base.h;

        // Background
        crate::draw::fill_rect(surface, x, y, w, h, 0xFF2C2C2E);

        // 1px bottom border (separator color from theme)
        let sep_color = crate::theme::colors().separator;
        crate::draw::fill_rect(surface, x, y + h as i32 - 1, w, 1, sep_color);
    }

    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        let pad = &self.base.padding;
        let mut x_offset = pad.left;
        let inner_h = self.base.h as i32 - pad.top - pad.bottom;
        let mut result = Vec::new();

        for &child_id in &self.base.children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            result.push(ChildLayout {
                id: child_id,
                x: x_offset,
                y: pad.top,
                w: None,
                h: if inner_h > 0 { Some(inner_h as u32) } else { None },
            });

            x_offset += controls[ci].base().w as i32 + self.spacing as i32;
        }

        Some(result)
    }
}
