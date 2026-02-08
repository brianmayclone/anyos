use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

pub fn stepper(win: u32, x: i32, y: i32, val: i32, min: i32, max: i32) {
    (exports().stepper_render)(win, x, y, val, min, max);
}

pub fn stepper_hit_plus(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().stepper_hit_test_plus)(x, y, mx, my) != 0
}

pub fn stepper_hit_minus(x: i32, y: i32, mx: i32, my: i32) -> bool {
    (exports().stepper_hit_test_minus)(x, y, mx, my) != 0
}

// ── Stateful component ──

pub struct UiStepper {
    pub x: i32,
    pub y: i32,
    pub value: i32,
    pub min: i32,
    pub max: i32,
}

impl UiStepper {
    pub fn new(x: i32, y: i32, value: i32, min: i32, max: i32) -> Self {
        UiStepper { x, y, value, min, max }
    }

    pub fn render(&self, win: u32) {
        stepper(win, self.x, self.y, self.value, self.min, self.max);
    }

    /// Returns `Some(new_value)` when +/- is clicked.
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<i32> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if stepper_hit_plus(self.x, self.y, mx, my) && self.value < self.max {
                self.value += 1;
                return Some(self.value);
            }
            if stepper_hit_minus(self.x, self.y, mx, my) && self.value > self.min {
                self.value -= 1;
                return Some(self.value);
            }
        }
        None
    }
}
