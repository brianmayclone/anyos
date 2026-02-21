//! TableLayout — grid layout container with rows and columns.

use alloc::boxed::Box;
use crate::control::{Control, ControlBase, ControlKind, EventResponse, find_idx};

pub struct TableLayout {
    pub(crate) base: ControlBase,
    pub columns: u32,
    pub row_height: u32,
}

impl TableLayout {
    pub fn new(base: ControlBase) -> Self {
        Self { base, columns: 2, row_height: 32 }
    }
}

impl Control for TableLayout {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::TableLayout }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        if self.base.color != 0 {
            crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, self.base.color);
        }
    }

    fn layout_children(&self, controls: &mut [Box<dyn Control>]) -> bool {
        if self.columns == 0 { return true; }

        let pad = &self.base.padding;
        let avail_w = self.base.w as i32 - pad.left - pad.right;
        let col_w = avail_w / self.columns as i32;

        let children = self.base.children.clone();
        let mut col = 0u32;
        let mut row = 0u32;

        for &child_id in &children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            let m = controls[ci].base().margin;
            let x = pad.left + (col as i32) * col_w + m.left;
            let y = pad.top + (row as i32) * self.row_height as i32 + m.top;
            let w = (col_w - m.left - m.right).max(0) as u32;
            let h = (self.row_height as i32 - m.top - m.bottom).max(0) as u32;

            controls[ci].set_position(x, y);
            controls[ci].set_size(w, h);

            col += 1;
            if col >= self.columns {
                col = 0;
                row += 1;
            }
        }
        true
    }
}
