use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

pub fn toggle(win: u32, x: i32, y: i32, on: bool) {
    (exports().toggle_render)(win, x, y, on as u32);
}

pub fn toggle_hit_test(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().toggle_hit_test)(x, y, mx, my) != 0
}

// ── Stateful component ──

pub struct UiToggle {
    pub x: i32,
    pub y: i32,
    pub on: bool,
}

impl UiToggle {
    pub fn new(x: i32, y: i32, on: bool) -> Self {
        UiToggle { x, y, on }
    }

    pub fn render(&self, win: u32) {
        toggle(win, self.x, self.y, self.on);
    }

    /// Returns `Some(new_state)` when toggled by a click.
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<bool> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if toggle_hit_test(self.x, self.y, mx, my) {
                self.on = !self.on;
                return Some(self.on);
            }
        }
        None
    }
}
