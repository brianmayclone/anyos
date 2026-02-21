//! StackPanel — layout container that stacks children vertically or horizontally.

use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::control::{Control, ControlBase, ControlKind, ChildLayout, Orientation, find_idx};

pub struct StackPanel {
    pub(crate) base: ControlBase,
    pub orientation: Orientation,
}

impl StackPanel {
    pub fn new(base: ControlBase) -> Self {
        Self { base, orientation: Orientation::Vertical }
    }
}

impl Control for StackPanel {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::StackPanel }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        // StackPanel is transparent — only renders its background if color is set
        if self.base.color != 0 {
            crate::draw::fill_rect(surface, x, y, self.base.w, self.base.h, self.base.color);
        }
    }

    fn layout_children(&self, controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        let pad = &self.base.padding;
        let mut cursor_x = pad.left;
        let mut cursor_y = pad.top;
        let mut result = Vec::new();

        let children = &self.base.children;
        for &child_id in children {
            let ci = match find_idx(controls, child_id) {
                Some(i) => i,
                None => continue,
            };
            if !controls[ci].base().visible {
                continue;
            }

            let m = controls[ci].base().margin;

            match self.orientation {
                Orientation::Vertical => {
                    result.push(ChildLayout { id: child_id, x: cursor_x + m.left, y: cursor_y + m.top, w: None, h: None });
                    cursor_y += controls[ci].base().h as i32 + m.top + m.bottom;
                }
                Orientation::Horizontal => {
                    result.push(ChildLayout { id: child_id, x: cursor_x + m.left, y: cursor_y + m.top, w: None, h: None });
                    cursor_x += controls[ci].base().w as i32 + m.left + m.right;
                }
            }
        }
        Some(result)
    }
}
