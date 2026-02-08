use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn toolbar(win: u32, x: i32, y: i32, w: u32, h: u32) {
    (exports().toolbar_render)(win, x, y, w, h);
}

pub fn toolbar_button(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, state: ButtonState) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().toolbar_render_button)(win, x, y, w, h, buf.as_ptr(), len, state as u8);
}

// ── Stateful component ──

pub struct UiToolbarButton {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub enabled: bool,
}

impl UiToolbarButton {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiToolbarButton { x, y, w, h, enabled: true }
    }

    pub fn render(&self, win: u32, text: &str) {
        let state = if self.enabled { ButtonState::Normal } else { ButtonState::Disabled };
        toolbar_button(win, self.x, self.y, self.w, self.h, text, state);
    }

    /// Returns `true` if clicked and enabled.
    pub fn handle_event(&self, event: &UiEvent) -> bool {
        if !self.enabled { return false; }
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if mx >= self.x && mx < self.x + self.w as i32
                && my >= self.y && my < self.y + self.h as i32
            {
                return true;
            }
        }
        false
    }
}
