use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

pub fn scrollbar(win: u32, x: i32, y: i32, w: u32, h: u32, content_h: u32, scroll: u32) {
    (exports().scrollview_render_scrollbar)(win, x, y, w, h, content_h, scroll);
}

pub fn scrollbar_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().scrollview_hit_test_scrollbar)(x, y, w, h, mx, my) != 0
}

/// Returns (thumb_y, thumb_h) for the scrollbar thumb position.
pub fn scrollbar_thumb_pos(h: u32, content_h: u32, scroll: u32) -> (u32, u32) {
    let packed = (exports().scrollview_thumb_pos)(h, content_h, scroll);
    let thumb_y = packed as u32;
    let thumb_h = (packed >> 32) as u32;
    (thumb_y, thumb_h)
}

// ── Stateful component ──

pub struct UiScrollbar {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub content_h: u32,
    pub scroll: u32,
    dragging: bool,
    drag_start_y: i32,
    drag_start_scroll: u32,
}

impl UiScrollbar {
    pub fn new(x: i32, y: i32, w: u32, h: u32, content_h: u32) -> Self {
        UiScrollbar { x, y, w, h, content_h, scroll: 0, dragging: false, drag_start_y: 0, drag_start_scroll: 0 }
    }

    pub fn render(&self, win: u32) {
        scrollbar(win, self.x, self.y, self.w, self.h, self.content_h, self.scroll);
    }

    pub fn max_scroll(&self) -> u32 {
        self.content_h.saturating_sub(self.h)
    }

    /// Returns `Some(new_scroll)` when scroll position changes.
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<u32> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if scrollbar_hit_test(self.x, self.y, self.w, self.h, mx, my) {
                self.dragging = true;
                self.drag_start_y = my;
                self.drag_start_scroll = self.scroll;
            }
        }
        if event.is_mouse_up() {
            self.dragging = false;
        }
        if self.dragging && event.event_type == EVENT_MOUSE_MOVE {
            let (_, my) = event.mouse_pos();
            let max_scroll = self.max_scroll();
            if max_scroll > 0 && self.h > 0 {
                let delta_y = my - self.drag_start_y;
                let scale = max_scroll as i64 * 1000 / self.h as i64;
                let new_scroll = (self.drag_start_scroll as i64 + delta_y as i64 * scale / 1000)
                    .max(0).min(max_scroll as i64) as u32;
                if new_scroll != self.scroll {
                    self.scroll = new_scroll;
                    return Some(new_scroll);
                }
            }
        }
        None
    }
}
