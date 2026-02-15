use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

/// Render a tab bar with multiple tabs. `items` are the tab labels.
pub fn tabbar(win: u32, x: i32, y: i32, w: u32, h: u32, items: &[&str], selected: usize) {
    let mut buf = [0u8; 512];
    let mut offsets = [0u32; 16];
    let mut pos = 0usize;
    for (i, item) in items.iter().enumerate().take(16) {
        offsets[i] = pos as u32;
        let n = nul_copy(item, &mut buf[pos..]) as usize;
        pos += n + 1; // include NUL
    }
    (exports().tabbar_render)(win, x, y, w, h, buf.as_ptr(), pos as u32, offsets.as_ptr(), selected as u32);
}

pub fn tabbar_hit_test(x: i32, y: i32, w: u32, h: u32, num_tabs: usize, mx: i32, my: i32) -> Option<usize> {
    let idx = (exports().tabbar_hit_test)(x, y, w, h, num_tabs as u32, mx, my);
    if idx < num_tabs as u32 { Some(idx as usize) } else { None }
}

// ── Stateful component ──

pub struct UiTabBar {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub selected: usize,
}

impl UiTabBar {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiTabBar { x, y, w, h, selected: 0 }
    }

    pub fn render(&self, win: u32, items: &[&str]) {
        tabbar(win, self.x, self.y, self.w, self.h, items, self.selected);
    }

    /// Returns `Some(new_index)` when a different tab is selected.
    pub fn handle_event(&mut self, event: &UiEvent, num_tabs: usize) -> Option<usize> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if let Some(idx) = tabbar_hit_test(self.x, self.y, self.w, self.h, num_tabs, mx, my) {
                if idx != self.selected {
                    self.selected = idx;
                    return Some(idx);
                }
            }
        }
        None
    }
}
