use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn contextmenu_bg(win: u32, x: i32, y: i32, w: u32, h: u32) {
    (exports().contextmenu_render_bg)(win, x, y, w, h);
}

pub fn contextmenu_item(win: u32, x: i32, y: i32, w: u32, text: &str, highlighted: bool) {
    let mut buf = [0u8; 128];
    let len = nul_copy(text, &mut buf);
    (exports().contextmenu_render_item)(win, x, y, w, buf.as_ptr(), len, highlighted as u32);
}

pub fn contextmenu_separator(win: u32, x: i32, y: i32, w: u32) {
    (exports().contextmenu_render_separator)(win, x, y, w);
}

// ── Stateful component ──

pub struct UiContextMenu {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub item_h: u32,
    pub visible: bool,
}

impl UiContextMenu {
    pub fn new(w: u32) -> Self {
        UiContextMenu { x: 0, y: 0, w, item_h: 28, visible: false }
    }

    pub fn show(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
        self.visible = true;
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn render(&self, win: u32, items: &[&str]) {
        if !self.visible { return; }
        let h = items.len() as u32 * self.item_h;
        contextmenu_bg(win, self.x, self.y, self.w, h);
        for (i, item) in items.iter().enumerate() {
            let iy = self.y + i as i32 * self.item_h as i32;
            contextmenu_item(win, self.x, iy, self.w, item, false);
        }
    }

    /// Returns `Some(index)` when an item is clicked. Hides the menu.
    /// Returns `None` and hides the menu if the user clicks outside.
    pub fn handle_event(&mut self, event: &UiEvent, num_items: usize) -> Option<usize> {
        if !self.visible { return None; }
        if event.is_mouse_down() {
            let (mx, my) = event.mouse_pos();
            let total_h = num_items as i32 * self.item_h as i32;
            if mx >= self.x && mx < self.x + self.w as i32
                && my >= self.y && my < self.y + total_h
            {
                let idx = ((my - self.y) as u32 / self.item_h) as usize;
                self.visible = false;
                return Some(idx);
            }
            // Click outside — close
            self.visible = false;
        }
        None
    }
}
