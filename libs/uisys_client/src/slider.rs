use crate::raw::exports;
use crate::types::*;

// ── Raw rendering functions ──

pub fn slider(win: u32, x: i32, y: i32, w: u32, min: u32, max: u32, val: u32, h: u32) {
    (exports().slider_render)(win, x, y, w, min, max, val, h);
}

pub fn slider_value_from_x(x: i32, w: u32, min: u32, max: u32, mx: i32) -> u32 {
    (exports().slider_value_from_x)(x, w, min, max, mx)
}

// ── Stateful component ──

pub struct UiSlider {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub min: u32,
    pub max: u32,
    pub value: u32,
    dragging: bool,
}

impl UiSlider {
    pub fn new(x: i32, y: i32, w: u32, min: u32, max: u32, value: u32) -> Self {
        UiSlider { x, y, w, h: 28, min, max, value, dragging: false }
    }

    pub fn render(&self, win: u32) {
        slider(win, self.x, self.y, self.w, self.min, self.max, self.value, self.h);
    }

    /// Returns `Some(new_value)` when the slider value changes (click or drag).
    pub fn handle_event(&mut self, event: &UiEvent) -> Option<u32> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if mx >= self.x && mx < self.x + self.w as i32
                && my >= self.y && my < self.y + self.h as i32
            {
                self.dragging = true;
                let new_val = slider_value_from_x(self.x, self.w, self.min, self.max, mx);
                if new_val != self.value {
                    self.value = new_val;
                    return Some(new_val);
                }
            }
        }
        if event.is_mouse_up() {
            self.dragging = false;
        }
        if self.dragging && event.event_type == EVENT_MOUSE_MOVE {
            let (mx, _) = event.mouse_pos();
            let new_val = slider_value_from_x(self.x, self.w, self.min, self.max, mx);
            if new_val != self.value {
                self.value = new_val;
                return Some(new_val);
            }
        }
        None
    }
}
