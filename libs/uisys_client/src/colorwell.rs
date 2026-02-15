use crate::raw::exports;

pub fn colorwell(win: u32, x: i32, y: i32, size: u32, color: u32) {
    (exports().colorwell_render)(win, x, y, size, color);
}
