use crate::raw::exports;

pub fn divider_h(win: u32, x: i32, y: i32, w: u32) {
    (exports().divider_render_h)(win, x, y, w);
}

pub fn divider_v(win: u32, x: i32, y: i32, h: u32) {
    (exports().divider_render_v)(win, x, y, h);
}
