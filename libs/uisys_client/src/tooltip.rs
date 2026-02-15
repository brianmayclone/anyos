use crate::raw::exports;
use crate::nul_copy;

pub fn tooltip(win: u32, x: i32, y: i32, text: &str) {
    let mut buf = [0u8; 128];
    let len = nul_copy(text, &mut buf);
    (exports().tooltip_render)(win, x, y, buf.as_ptr(), len);
}
