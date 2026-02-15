use crate::raw::exports;
use crate::nul_copy;

pub fn groupbox(win: u32, x: i32, y: i32, w: u32, h: u32, title: &str) {
    let mut buf = [0u8; 64];
    let len = nul_copy(title, &mut buf);
    (exports().groupbox_render)(win, x, y, w, h, buf.as_ptr(), len);
}
