use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn radio(win: u32, x: i32, y: i32, selected: bool, text: &str) {
    let mut buf = [0u8; 128];
    let len = nul_copy(text, &mut buf);
    (exports().radio_render)(win, x, y, selected as u32, buf.as_ptr(), len);
}

pub fn radio_hit_test(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().radio_hit_test)(x, y, mx, my) != 0
}

// ── Stateful component ──

pub struct UiRadioGroup {
    pub x: i32,
    pub y: i32,
    pub spacing: i32,
    pub selected: usize,
}

impl UiRadioGroup {
    pub fn new(x: i32, y: i32, spacing: i32) -> Self {
        UiRadioGroup { x, y, spacing, selected: 0 }
    }

    pub fn render(&self, win: u32, items: &[&str]) {
        for (i, name) in items.iter().enumerate() {
            let iy = self.y + i as i32 * self.spacing;
            radio(win, self.x, iy, i == self.selected, name);
        }
    }

    /// Returns `Some(new_index)` when a different radio is selected.
    pub fn handle_event(&mut self, event: &UiEvent, num_items: usize) -> Option<usize> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            for i in 0..num_items {
                let iy = self.y + i as i32 * self.spacing;
                if radio_hit_test(self.x, iy, mx, my) && self.selected != i {
                    self.selected = i;
                    return Some(i);
                }
            }
        }
        None
    }
}
