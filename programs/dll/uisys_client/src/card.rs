use crate::raw::exports;

pub fn card(win: u32, x: i32, y: i32, w: u32, h: u32) {
    (exports().card_render)(win, x, y, w, h);
}
