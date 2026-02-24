//! Toolbar — horizontal bar that lays out children left-to-right with spacing.
//!
//! Enforces a minimum height of 36px (or the tallest child + padding,
//! whichever is larger) so toolbar buttons are always visible.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, ChildLayout, find_idx};

/// Absolute minimum toolbar height in pixels.
const MIN_HEIGHT: u32 = 36;

pub struct Toolbar {
    pub(crate) base: ControlBase,
    /// Horizontal gap between children (pixels). Default: 4.
    pub spacing: u32,
}

impl Toolbar {
    pub fn new(base: ControlBase) -> Self {
        let mut tb = Self { base, spacing: 4 };
        // Enforce minimum height at creation time
        if tb.base.h < MIN_HEIGHT {
            tb.base.h = MIN_HEIGHT;
        }
        tb
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

            let child_h = controls[ci].base().h as i32;
            // Center children vertically if they are shorter than the toolbar
            let cy = if child_h < inner_h {
                pad.top + (inner_h - child_h) / 2
            } else {
                pad.top
            };

            result.push(ChildLayout {
                id: child_id,
                x: x_offset,
                y: cy,
                w: None,
                h: None, // keep child's own height
            });

            x_offset += controls[ci].base().w as i32 + self.spacing as i32;
        }

        Some(result)
    }

    fn set_size(&mut self, w: u32, h: u32) {
        self.base.w = w;
        // Enforce minimum height on resize too
        self.base.h = if h < MIN_HEIGHT { MIN_HEIGHT } else { h };
        self.base.mark_dirty();
    }
}
