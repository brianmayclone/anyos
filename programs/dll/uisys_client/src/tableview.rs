use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn tableview_bg(win: u32, x: i32, y: i32, w: u32, h: u32, row_h: u32, num_rows: u32, scroll: u32, selected: u32) {
    (exports().tableview_render)(win, x, y, w, h, row_h, num_rows, scroll, selected);
}

pub fn tableview_row(win: u32, x: i32, y: i32, w: u32, row_h: u32, text: &str, index: u32, selected: bool) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().tableview_render_row)(win, x, y, w, row_h, buf.as_ptr(), len, index, selected as u32);
}

pub fn tableview_header(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().tableview_render_header)(win, x, y, w, h, buf.as_ptr(), len);
}

pub fn tableview_hit_row(y: i32, row_h: u32, num_rows: u32, my: i32) -> Option<usize> {
    let idx = (exports().tableview_hit_test_row)(y, row_h, num_rows, my);
    if idx < num_rows { Some(idx as usize) } else { None }
}

// ── Stateful component ──

pub struct UiTableView {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub row_h: u32,
    pub selected: Option<usize>,
    pub scroll: u32,
}

impl UiTableView {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        UiTableView { x, y, w, h, row_h: 28, selected: None, scroll: 0 }
    }

    pub fn render_bg(&self, win: u32, num_rows: usize) {
        let sel = self.selected.map(|s| s as u32).unwrap_or(u32::MAX);
        tableview_bg(win, self.x, self.y, self.w, self.h, self.row_h, num_rows as u32, self.scroll, sel);
    }

    pub fn render_row(&self, win: u32, index: usize, text: &str) {
        let ry = self.y + index as i32 * self.row_h as i32 - self.scroll as i32;
        let sel = self.selected == Some(index);
        tableview_row(win, self.x, ry, self.w, self.row_h, text, index as u32, sel);
    }

    /// Returns `Some(row_index)` when a row is clicked.
    pub fn handle_event(&mut self, event: &UiEvent, num_rows: usize) -> Option<usize> {
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            if mx >= self.x && mx < self.x + self.w as i32 {
                if let Some(idx) = tableview_hit_row(self.y, self.row_h, num_rows as u32, my) {
                    self.selected = Some(idx);
                    return Some(idx);
                }
            }
        }
        None
    }
}
