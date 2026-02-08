use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

pub fn splitview(win: u32, x: i32, y: i32, w: u32, h: u32, split_x: u32) {
    (exports().splitview_render)(win, x, y, w, h, split_x);
}

pub fn splitview_divider_hit(x: i32, y: i32, w: u32, h: u32, split_x: u32, mx: i32, my: i32) -> bool {
    (exports().splitview_hit_test_divider)(x, y, w, h, split_x, mx, my) != 0
}

pub fn splitview_clamp(w: u32, min_left: u32, min_right: u32, split_x: u32) -> u32 {
    (exports().splitview_clamp)(w, min_left, min_right, split_x)
}

// ── Stateful component ──

pub struct UiSplitView {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub split_x: u32,
    pub min_left: u32,
    pub min_right: u32,
    dragging: bool,
}

impl UiSplitView {
    pub fn new(x: i32, y: i32, w: u32, h: u32, split_x: u32) -> Self {
        UiSplitView { x, y, w, h, split_x, min_left: 100, min_right: 100, dragging: false }
    }

    pub fn render(&self, win: u32) {
        splitview(win, self.x, self.y, self.w, self.h, self.split_x);
    }

    /// Returns `Some(new_split_x)` when divider is dragged.
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<u32> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if splitview_divider_hit(self.x, self.y, self.w, self.h, self.split_x, mx, my) {
                self.dragging = true;
            }
        }
        if event.is_mouse_up() {
            self.dragging = false;
        }
        if self.dragging && event.event_type == EVENT_MOUSE_MOVE {
            let (mx, _) = event.mouse_pos();
            let new_x = (mx - self.x).max(0) as u32;
            let clamped = splitview_clamp(self.w, self.min_left, self.min_right, new_x);
            if clamped != self.split_x {
                self.split_x = clamped;
                return Some(clamped);
            }
        }
        None
    }
}
