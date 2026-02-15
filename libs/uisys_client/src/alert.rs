use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

pub fn alert(win: u32, x: i32, y: i32, w: u32, h: u32, title: &str, message: &str) {
    let mut tbuf = [0u8; 128];
    let tlen = nul_copy(title, &mut tbuf);
    let mut mbuf = [0u8; 256];
    let mlen = nul_copy(message, &mut mbuf);
    (exports().alert_render)(win, x, y, w, h, tbuf.as_ptr(), tlen, mbuf.as_ptr(), mlen);
}

pub fn alert_button(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, style: ButtonStyle, state: ButtonState) {
    let mut buf = [0u8; 64];
    let len = nul_copy(text, &mut buf);
    (exports().alert_render_button)(win, x, y, w, h, buf.as_ptr(), len, style as u8, state as u8);
}
