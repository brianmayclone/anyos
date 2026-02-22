use crate::control::{Control, ControlBase, ControlKind, EventResponse, ChildLayout, Orientation};
use alloc::boxed::Box;
use alloc::vec::Vec;

pub struct SplitView {
    pub(crate) base: ControlBase,
    pub(crate) divider_pos: i32,
    /// Split ratio in percent (0-100). Default 30 (30% first pane, 70% second pane).
    pub(crate) split_ratio: u32,
    /// Minimum split ratio in percent. Default 10.
    pub(crate) min_ratio: u32,
    /// Maximum split ratio in percent. Default 90.
    pub(crate) max_ratio: u32,
    pub(crate) orientation: Orientation,
    dragging: bool,
}

impl SplitView {
    pub fn new(base: ControlBase) -> Self {
        let default_ratio = 30u32;
        let default_pos = (base.w * default_ratio / 100) as i32;
        Self {
            base,
            divider_pos: default_pos,
            split_ratio: default_ratio,
            min_ratio: 10,
            max_ratio: 90,
            orientation: Orientation::Horizontal,
            dragging: false,
        }
    }

    fn total_extent(&self) -> u32 {
        match self.orientation {
            Orientation::Horizontal => self.base.w,
            Orientation::Vertical => self.base.h,
        }
    }

    fn min_pos(&self) -> i32 {
        (self.total_extent() as u64 * self.min_ratio as u64 / 100) as i32
    }

    fn max_pos(&self) -> i32 {
        (self.total_extent() as u64 * self.max_ratio as u64 / 100) as i32
    }

    /// Recalculate divider_pos from split_ratio when size changes.
    pub fn sync_divider(&mut self) {
        self.divider_pos = (self.total_extent() as u64 * self.split_ratio as u64 / 100) as i32;
    }
}

impl Control for SplitView {
    fn base(&self) -> &ControlBase { &self.base }
    fn base_mut(&mut self) -> &mut ControlBase { &mut self.base }
    fn kind(&self) -> ControlKind { ControlKind::SplitView }

    fn render(&self, surface: &crate::draw::Surface, ax: i32, ay: i32) {
        let x = ax + self.base.x;
        let y = ay + self.base.y;
        let tc = crate::theme::colors();
        let total = self.total_extent();
        let div = (total as u64 * self.split_ratio as u64 / 100) as i32;
        match self.orientation {
            Orientation::Horizontal => {
                // Vertical divider line (left-right split)
                crate::draw::fill_rect(surface, x + div, y, 1, self.base.h, tc.separator);
            }
            Orientation::Vertical => {
                // Horizontal divider line (top-bottom split)
                crate::draw::fill_rect(surface, x, y + div, self.base.w, 1, tc.separator);
            }
        }
    }

    fn is_interactive(&self) -> bool { true }

    fn layout_children(&self, _controls: &[Box<dyn Control>]) -> Option<Vec<ChildLayout>> {
        let children = &self.base.children;
        if children.is_empty() {
            return Some(Vec::new());
        }

        let mut layouts = Vec::new();
        // Recompute divider from ratio + current extent (handles resize)
        let total = self.total_extent();
        let div = (total as u64 * self.split_ratio as u64 / 100) as i32;

        match self.orientation {
            Orientation::Horizontal => {
                let w = self.base.w as i32;
                let h = self.base.h;
                // First child gets left pane
                if children.len() >= 1 {
                    layouts.push(ChildLayout {
                        id: children[0],
                        x: 0,
                        y: 0,
                        w: Some(div.max(0) as u32),
                        h: Some(h),
                    });
                }
                // Second child gets right pane
                if children.len() >= 2 {
                    let right_x = div + 1; // 1px divider
                    let right_w = (w - right_x).max(0) as u32;
                    layouts.push(ChildLayout {
                        id: children[1],
                        x: right_x,
                        y: 0,
                        w: Some(right_w),
                        h: Some(h),
                    });
                }
            }
            Orientation::Vertical => {
                let w = self.base.w;
                let h = self.base.h as i32;
                // First child gets top pane
                if children.len() >= 1 {
                    layouts.push(ChildLayout {
                        id: children[0],
                        x: 0,
                        y: 0,
                        w: Some(w),
                        h: Some(div.max(0) as u32),
                    });
                }
                // Second child gets bottom pane
                if children.len() >= 2 {
                    let bottom_y = div + 1; // 1px divider
                    let bottom_h = (h - bottom_y).max(0) as u32;
                    layouts.push(ChildLayout {
                        id: children[1],
                        x: 0,
                        y: bottom_y,
                        w: Some(w),
                        h: Some(bottom_h),
                    });
                }
            }
        }

        Some(layouts)
    }

    fn handle_mouse_down(&mut self, lx: i32, ly: i32, _button: u32) -> EventResponse {
        // Check if click is near the divider (within 4px)
        let pos = match self.orientation {
            Orientation::Horizontal => lx,
            Orientation::Vertical => ly,
        };
        if (pos - self.divider_pos).abs() <= 4 {
            self.dragging = true;
            EventResponse::CONSUMED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_move(&mut self, lx: i32, ly: i32) -> EventResponse {
        if self.dragging {
            let pos = match self.orientation {
                Orientation::Horizontal => lx,
                Orientation::Vertical => ly,
            };
            let min = self.min_pos();
            let max = self.max_pos();
            self.divider_pos = pos.max(min).min(max);
            let total = self.total_extent();
            if total > 0 {
                self.split_ratio = (self.divider_pos as u32 * 100) / total;
            }
            self.base.state = self.split_ratio;
            self.base.dirty = true;
            EventResponse::CHANGED
        } else {
            EventResponse::IGNORED
        }
    }

    fn handle_mouse_up(&mut self, _lx: i32, _ly: i32, _button: u32) -> EventResponse {
        if self.dragging {
            self.dragging = false;
            EventResponse::CHANGED
        } else {
            EventResponse::CONSUMED
        }
    }
}
