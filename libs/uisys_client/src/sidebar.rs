use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn sidebar_bg(win: u32, x: i32, y: i32, w: u32, h: u32) {
    (exports().sidebar_render_bg)(win, x, y, w, h);
}

pub fn sidebar_item(win: u32, x: i32, y: i32, w: u32, text: &str, selected: bool) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().sidebar_render_item)(win, x, y, w, buf.as_ptr(), len, selected as u32);
}

pub fn sidebar_header(win: u32, x: i32, y: i32, w: u32, text: &str) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().sidebar_render_header)(win, x, y, w, buf.as_ptr(), len);
}

// ── Stateful component ──

pub struct UiSidebar {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub selected: usize,
    pub header_h: u32,
    pub item_h: u32,
}

impl UiSidebar {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiSidebar { x, y, w, h, selected: 0, header_h: 28, item_h: 32 }
    }

    pub fn render(&self, win: u32, header: &str, items: &[&str]) {
        sidebar_bg(win, self.x, self.y, self.w, self.h);
        sidebar_header(win, self.x, self.y, self.w, header);

        let y0 = self.y + self.header_h as i32;
        for (i, name) in items.iter().enumerate() {
            let iy = y0 + i as i32 * self.item_h as i32;
            sidebar_item(win, self.x, iy, self.w, name, i == self.selected);
        }
    }

    /// Returns `Some(new_index)` when the user clicks a different item.
    pub fn handle_event(&mut self, event: &UiEvent, num_items: usize) -> Option<usize> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if mx < self.x || mx >= self.x + self.w as i32 {
                return None;
            }
            let y0 = self.y + self.header_h as i32;
            for i in 0..num_items {
                let iy = y0 + i as i32 * self.item_h as i32;
                if my >= iy && my < iy + self.item_h as i32 {
                    if self.selected != i {
                        self.selected = i;
                        return Some(i);
                    }
                    return None;
                }
            }
        }
        None
    }
}
