use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn button(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, style: ButtonStyle, state: ButtonState) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().button_render)(win, x, y, w, h, buf.as_ptr(), len, style as u8, state as u8);
}

pub fn button_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().button_hit_test)(x, y, w, h, mx, my) != 0
}

pub fn button_measure(text: &str) -> (u32, u32) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    let mut w = 0u32;
    let mut h = 0u32;
    (exports().button_measure)(buf.as_ptr(), len, &mut w, &mut h);
    (w, h)
}

// ── Stateful component ──

pub struct UiButton {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub style: ButtonStyle,
    pressed: bool,
}

impl UiButton {
    pub fn new(x: i32, y: i32, w: u32, h: u32, style: ButtonStyle) -> Self {
        UiButton { x, y, w, h, style, pressed: false }
    }

    pub fn render(&self, win: u32, text: &str) {
        let state = if self.pressed { ButtonState::Pressed } else { ButtonState::Normal };
        button(win, self.x, self.y, self.w, self.h, text, self.style, state);
    }

    /// Returns `true` when the button is clicked (mouse-down inside bounds).
    pub fn handle_event(&mut self, event: &UiEvent) -> bool {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if button_hit_test(self.x, self.y, self.w, self.h, mx, my) {
                self.pressed = true;
                return true;
            }
        }
        if event.is_mouse_up() {
            self.pressed = false;
        }
        false
    }
}
