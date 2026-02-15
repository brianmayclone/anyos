use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

pub fn status_indicator(win: u32, x: i32, y: i32, kind: StatusKind, text: &str) {
    status_indicator_sized(win, x, y, kind, text, FontSize::Normal);
}

pub fn status_indicator_sized(win: u32, x: i32, y: i32, kind: StatusKind, text: &str, font_size: FontSize) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().status_render)(win, x, y, kind as u8, buf.as_ptr(), len, font_size as u16);
}
