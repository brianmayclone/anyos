use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn tag(win: u32, x: i32, y: i32, text: &str, bg: u32, fg: u32, show_close: bool) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().tag_render)(win, x, y, buf.as_ptr(), len, bg, fg, show_close as u32);
}

pub fn tag_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().tag_hit_test)(x, y, w, h, mx, my) != 0
}

pub fn tag_close_hit_test(x: i32, y: i32, w: u32, mx: i32, my: i32) -> bool {
    (exports().tag_close_hit_test)(x, y, w, mx, my) != 0
}

// ── Stateful component ──

/// Tag/chip component with optional close button.
pub struct UiTag {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub bg: u32,
    pub fg: u32,
    pub show_close: bool,
}

impl UiTag {
    pub fn new(x: i32, y: i32, bg: u32, fg: u32, show_close: bool) -> Self {
        UiTag { x, y, w: 80, h: 24, bg, fg, show_close }
    }

    pub fn render(&self, win: u32, text: &str) {
        tag(win, self.x, self.y, text, self.bg, self.fg, self.show_close);
    }

    /// Returns `true` when the close button is clicked (if visible).
    pub fn handle_close(&self, event: &UiEvent) -> bool {
        if !self.show_close { return false; }
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            return tag_close_hit_test(self.x, self.y, self.w, mx, my);
        }
        false
    }

    /// Returns `true` when the tag body is clicked.
    pub fn handle_event(&self, event: &UiEvent) -> bool {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            return tag_hit_test(self.x, self.y, self.w, self.h, mx, my);
        }
        false
    }
}
