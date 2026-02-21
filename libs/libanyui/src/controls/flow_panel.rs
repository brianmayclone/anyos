//! FlowPanel — layout container that arranges children horizontally with line wrapping.

use alloc::boxed::Box;
use crate::control::{Control, ControlBase, ControlKind, EventResponse, find_idx};

pub struct FlowPanel {
    pub(crate) base: ControlBase,
}

impl FlowPanel {
    pub fn new(base: ControlBase) -> Self {
        Self { base }
    }
}

impl Control for FlowPanel {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::FlowPanel }

    fn render(&self, win: u32, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        if self.base.color != 0 {
            crate::syscall::win_fill_rect(win, x, y, self.base.w, self.base.h, self.base.color);
        }
    }

    fn layout_children(&self, controls: &mut [Box<dyn Control>]) -> bool {
        let pad = &self.base.padding;
        let max_x = self.base.w as i32 - pad.right;
        let mut cursor_x = pad.left;
        let mut cursor_y = pad.top;
        let mut row_height: i32 = 0;

        let children = self.base.children.clone();
        for &child_id in &children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            let m = controls[ci].base().margin;
            let cw = controls[ci].base().w as i32 + m.left + m.right;
            let ch = controls[ci].base().h as i32 + m.top + m.bottom;

            // Wrap to next line if this child won't fit
            if cursor_x + cw > max_x && cursor_x > pad.left {
                cursor_x = pad.left;
                cursor_y += row_height;
                row_height = 0;
            }

            controls[ci].set_position(cursor_x + m.left, cursor_y + m.top);
            cursor_x += cw;
            if ch > row_height {
                row_height = ch;
            }
        }
        true
    }
}
