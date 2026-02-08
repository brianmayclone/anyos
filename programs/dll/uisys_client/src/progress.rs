use crate::raw::exports;

pub fn progress(win: u32, x: i32, y: i32, w: u32, h: u32, percent: u32) {
    (exports().progress_render)(win, x, y, w, h, percent, 0);
}

pub fn progress_indeterminate(win: u32, x: i32, y: i32, w: u32, h: u32) {
    (exports().progress_render)(win, x, y, w, h, 0, 1);
}
