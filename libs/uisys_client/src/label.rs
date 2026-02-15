use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

pub fn label(win: u32, x: i32, y: i32, text: &str, color: u32, font_size: FontSize, align: TextAlign) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().label_render)(win, x, y, buf.as_ptr(), len, color, font_size as u16, align as u8);
}

pub fn label_measure(text: &str, font_size: FontSize) -> (u32, u32) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    let mut w = 0u32;
    let mut h = 0u32;
    (exports().label_measure)(buf.as_ptr(), len, font_size as u16, &mut w, &mut h);
    (w, h)
}

pub fn label_ellipsis(win: u32, x: i32, y: i32, text: &str, color: u32, font_size: FontSize, max_w: u32) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().label_render_ellipsis)(win, x, y, buf.as_ptr(), len, color, font_size as u16, max_w);
}

pub fn label_multiline(win: u32, x: i32, y: i32, text: &str, color: u32, font_size: FontSize, max_w: u32, spacing: u32) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().label_render_multiline)(win, x, y, buf.as_ptr(), len, color, font_size as u16, max_w, spacing);
}
