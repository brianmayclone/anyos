use crate::raw::exports;

/// Render an RGBA image. `pixels` is a slice of u32 ARGB values, `img_w` x `img_h`.
pub fn imageview(win: u32, x: i32, y: i32, w: u32, h: u32, pixels: &[u32], img_w: u32, img_h: u32) {
    (exports().imageview_render)(win, x, y, w, h, pixels.as_ptr(), img_w, img_h);
}
