use crate::raw::exports;
use crate::nul_copy;
use crate::types::*;

// ── Raw rendering functions ──

/// Render a multi-line text area.
/// `flags`: bit 0 = show line numbers.
pub fn textarea(win: u32, x: i32, y: i32, w: u32, h: u32, text: &str, cursor: u32, scroll: u32, flags: u32) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (exports().textarea_render)(win, x, y, w, h, buf.as_ptr(), len, cursor, scroll, flags, 0);
}

pub fn textarea_hit_test(x: i32, y: i32, w: u32, h: u32, mx: i32, my: i32) -> bool {
    (exports().textarea_hit_test)(x, y, w, h, mx, my) != 0
}
