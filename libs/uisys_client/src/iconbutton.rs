use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

/// Render a circular/square icon button. `shape`: 0=circle, 1=square.
pub fn iconbutton(win: u32, x: i32, y: i32, size: u32, shape: u8, color: u32) {
    (exports().iconbutton_render)(win, x, y, size, shape, color);
}

pub fn iconbutton_hit_test(x: i32, y: i32, size: u32, mx: i32, my: i32) -> bool {
    (exports().iconbutton_hit_test)(x, y, size, mx, my) != 0
}

// ── Stateful component ──

pub struct UiIconButton {
    pub x: i32,
    pub y: i32,
    pub size: u32,
    pub shape: u8,
    pub color: u32,
    pub enabled: bool,
}

impl UiIconButton {
    pub fn new(x: i32, y: i32, size: u32, color: u32) -> Self {
        UiIconButton { x, y, size, shape: 0, color, enabled: true }
    }

    pub fn render(&self, win: u32) {
        iconbutton(win, self.x, self.y, self.size, self.shape, self.color);
    }

    /// Returns `true` if clicked and enabled.
    pub fn handle_event(&self, event: &UiEvent) -> bool {
        if !self.enabled { return false; }
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            return iconbutton_hit_test(self.x, self.y, self.size, mx, my);
        }
        false
    }
}
