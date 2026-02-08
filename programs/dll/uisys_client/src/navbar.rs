use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn navbar(win: u32, x: i32, y: i32, w: u32, title: &str, show_back: bool) {
    let mut buf = [0u8; 128];
    let len = nul_copy(title, &mut buf);
    (exports().navbar_render)(win, x, y, w, buf.as_ptr(), len, show_back as u32);
}

pub fn navbar_back_hit(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().navbar_hit_test_back)(x, y, mx, my) != 0
}

// ── Stateful component ──

pub struct UiNavbar {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub show_back: bool,
}

impl UiNavbar {
    pub fn new(x: i32, y: i32, w: u32, show_back: bool) -> Self {
        UiNavbar { x, y, w, show_back }
    }

    pub fn render(&self, win: u32, title: &str) {
        navbar(win, self.x, self.y, self.w, title, self.show_back);
    }

    /// Returns `true` when the back button is clicked.
    pub fn handle_event(&self, event: &UiEvent) -> bool {
        if self.show_back && event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            return navbar_back_hit(self.x, self.y, mx, my);
        }
        false
    }
}
