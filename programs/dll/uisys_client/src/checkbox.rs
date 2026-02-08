use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn checkbox(win: u32, x: i32, y: i32, state: CheckboxState, text: &str) {
    let mut buf = [0u8; 128];
    let len = nul_copy(text, &mut buf);
    (exports().checkbox_render)(win, x, y, state as u8, buf.as_ptr(), len);
}

pub fn checkbox_hit_test(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().checkbox_hit_test)(x, y, mx, my) != 0
}

// ── Stateful component ──

pub struct UiCheckbox {
    pub x: i32,
    pub y: i32,
    pub checked: bool,
}

impl UiCheckbox {
    pub fn new(x: i32, y: i32, checked: bool) -> Self {
        UiCheckbox { x, y, checked }
    }

    pub fn render(&self, win: u32, text: &str) {
        let state = if self.checked { CheckboxState::Checked } else { CheckboxState::Unchecked };
        checkbox(win, self.x, self.y, state, text);
    }

    /// Returns `Some(new_state)` when the checkbox is clicked.
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<bool> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if checkbox_hit_test(self.x, self.y, mx, my) {
                self.checked = !self.checked;
                return Some(self.checked);
            }
        }
        None
    }
}
