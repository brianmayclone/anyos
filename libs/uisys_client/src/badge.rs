use crate::raw::exports;

pub fn badge(win: u32, x: i32, y: i32, count: u32) {
    (exports().badge_render)(win, x, y, count, 0);
}

pub fn badge_dot(win: u32, x: i32, y: i32) {
    (exports().badge_render)(win, x, y, 0, 1);
}
