//! TableLayout — grid layout container with rows and columns.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, ChildLayout, find_idx};

pub struct TableLayout {
    pub(crate) base: ControlBase,
    pub columns: u32,
    pub row_height: u32,
    /// Optional per-column pixel widths. When non-empty the first N-1 entries
    /// are used verbatim; the last column receives the remaining available
    /// width. Empty means equal distribution (legacy behaviour).
    pub col_widths: Vec<u32>,
}

impl TableLayout {
    pub fn new(base: ControlBase) -> Self {
        Self { base, columns: 2, row_height: 32, col_widths: Vec::new() }
    }
}

impl Control for TableLayout {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TableLayout }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        if self.base.color != 0 {
            let b = self.base();
            let p = crate::draw::scale_bounds(ax, ay, b.x, b.y, b.w, b.h);
            crate::draw::fill_rect(surface, p.x, p.y, p.w, p.h, b.color);
        }
    }

    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        if self.columns == 0 { return Some(Vec::new()); }

        let pad = &self.base.padding;
        let avail_w = self.base.w as i32 - pad.left - pad.right;

        // Build per-column x-offsets and widths.
        // If col_widths is set: use verbatim for the first N-1 columns;
        // last column gets the remaining width. Falls back to equal split.
        let col_xs: Vec<i32>;
        let col_ws: Vec<i32>;
        if !self.col_widths.is_empty() {
            let mut xs = Vec::new();
            let mut ws = Vec::new();
            let mut x = pad.left;
            for c in 0..self.columns as usize {
                xs.push(x);
                let w = if c + 1 < self.col_widths.len() {
                    self.col_widths[c] as i32
                } else if c < self.col_widths.len() {
                    // Last entry in col_widths — remaining width.
                    let used: i32 = self.col_widths[..c].iter().map(|&v| v as i32).sum();
                    (avail_w - used).max(0)
                } else {
                    // More columns than widths — give zero (hidden).
                    0
                };
                ws.push(w);
                x += w;
            }
            col_xs = xs;
            col_ws = ws;
        } else {
            let cw = avail_w / self.columns as i32;
            col_xs = (0..self.columns as i32).map(|c| pad.left + c * cw).collect();
            col_ws = (0..self.columns as usize).map(|_| cw).collect();
        }

        let mut result = Vec::new();
        let children = &self.base.children;
        let mut col = 0usize;
        let mut row = 0u32;

        for &child_id in children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            let m = controls[ci].base().margin;
            let x = col_xs[col] + m.left;
            let y = pad.top + (row as i32) * self.row_height as i32 + m.top;
            let w = (col_ws[col] - m.left - m.right).max(0) as u32;
            let h = (self.row_height as i32 - m.top - m.bottom).max(0) as u32;

            result.push(ChildLayout { id: child_id, x, y, w: Some(w), h: Some(h) });

            col += 1;
            if col >= self.columns as usize {
                col = 0;
                row += 1;
            }
        }
        Some(result)
    }
}
