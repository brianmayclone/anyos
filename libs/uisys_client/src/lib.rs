//! uisys client — type-safe Rust wrappers for the uisys.dlib shared library.
//!
//! Programs depend on this crate to call DLL functions through the export table
//! at the fixed virtual address 0x04000000.
//!
//! Each component lives in its own module with:
//! - Raw rendering functions (thin wrappers around DLL exports)
//! - Stateful `Ui*` struct with `render()` and `handle_event()` where applicable

#![no_std]

pub mod raw;
pub mod types;

// Individual component modules
pub mod label;
pub mod button;
pub mod toggle;
pub mod checkbox;
pub mod radio;
pub mod slider;
pub mod stepper;
pub mod textfield;
pub mod searchfield;
pub mod sidebar;
pub mod navbar;
pub mod toolbar;
pub mod tabbar;
pub mod segmented;
pub mod splitview;
pub mod contextmenu;
pub mod scrollview;
pub mod tableview;
pub mod card;
pub mod divider;
pub mod groupbox;
pub mod alert;
pub mod tooltip;
pub mod badge;
pub mod tag;
pub mod status;
pub mod colorwell;
pub mod progress;
pub mod imageview;
pub mod iconbutton;
pub mod controls;

// Re-export everything so `use uisys_client::*` works
pub use types::*;
pub use label::*;
pub use button::*;
pub use toggle::*;
pub use checkbox::*;
pub use radio::*;
pub use slider::*;
pub use stepper::*;
pub use textfield::*;
pub use searchfield::*;
pub use sidebar::*;
pub use navbar::*;
pub use toolbar::*;
pub use tabbar::*;
pub use segmented::*;
pub use splitview::*;
pub use contextmenu::*;
pub use scrollview::*;
pub use tableview::*;
pub use card::*;
pub use divider::*;
pub use groupbox::*;
pub use alert::*;
pub use tooltip::*;
pub use badge::*;
pub use tag::*;
pub use status::*;
pub use colorwell::*;
pub use progress::*;
pub use imageview::*;
pub use iconbutton::*;
pub use controls::*;

/// Copy a &str into a NUL-terminated buffer. Returns the string length (excl. NUL).
pub(crate) fn nul_copy(s: &str, buf: &mut [u8]) -> u32 {
    let bytes = s.as_bytes();
    let len = bytes.len().min(buf.len() - 1);
    buf[..len].copy_from_slice(&bytes[..len]);
    buf[len] = 0;
    len as u32
}

// --- v2 API ---

/// Query whether GPU acceleration (VMware SVGA II) is available.
pub fn gpu_has_accel() -> bool {
    (raw::exports().gpu_has_accel)() != 0
}

/// Draw a filled rounded rectangle with kernel-side anti-aliasing.
pub fn fill_rounded_rect_aa(win: u32, x: i32, y: i32, w: u32, h: u32, r: u32, color: u32) {
    (raw::exports().fill_rounded_rect_aa)(win, x, y, w, h, r, color);
}

/// Draw text with an explicit font ID and size.
pub fn draw_text_with_font(win: u32, x: i32, y: i32, color: u32, size: u32, font_id: u16, text: &str) {
    let mut buf = [0u8; 256];
    let len = nul_copy(text, &mut buf);
    (raw::exports().draw_text_with_font)(win, x, y, color, size, font_id, buf.as_ptr(), len);
}

/// Measure text extent using a specific font.
/// Returns (width, height) in pixels.
pub fn font_measure(font_id: u16, size: u16, text: &str) -> (u32, u32) {
    let bytes = text.as_bytes();
    let mut w = 0u32;
    let mut h = 0u32;
    (raw::exports().font_measure)(font_id as u32, size, bytes.as_ptr(), bytes.len() as u32, &mut w, &mut h);
    (w, h)
}

// --- Shadow API ---

/// Surface descriptor for raw pixel buffer shadow drawing.
/// Must match the layout expected by uisys DLL (pointer, width, height).
#[repr(C)]
pub struct ShadowSurface {
    pub pixels: *mut u32,
    pub width: u32,
    pub height: u32,
}

/// Draw a soft shadow for a rectangle onto a raw pixel buffer.
pub fn draw_shadow_rect_buf(
    pixels: &mut [u32], fb_w: u32, fb_h: u32,
    x: i32, y: i32, w: u32, h: u32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    let mut surface = ShadowSurface { pixels: pixels.as_mut_ptr(), width: fb_w, height: fb_h };
    let win = &mut surface as *mut ShadowSurface as u32;
    (raw::exports().draw_shadow_rect)(win, x, y, w, h, offset_x, offset_y, spread, alpha);
}

/// Draw a soft shadow for a rounded rectangle onto a raw pixel buffer.
pub fn draw_shadow_rounded_rect_buf(
    pixels: &mut [u32], fb_w: u32, fb_h: u32,
    x: i32, y: i32, w: u32, h: u32, r: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    let mut surface = ShadowSurface { pixels: pixels.as_mut_ptr(), width: fb_w, height: fb_h };
    let win = &mut surface as *mut ShadowSurface as u32;
    (raw::exports().draw_shadow_rounded_rect)(win, x, y, w, h, r, offset_x, offset_y, spread, alpha);
}

/// Draw a soft shadow for an oval/ellipse onto a raw pixel buffer.
pub fn draw_shadow_oval_buf(
    pixels: &mut [u32], fb_w: u32, fb_h: u32,
    cx: i32, cy: i32, rx: i32, ry: i32,
    offset_x: i32, offset_y: i32, spread: i32, alpha: u32,
) {
    let mut surface = ShadowSurface { pixels: pixels.as_mut_ptr(), width: fb_w, height: fb_h };
    let win = &mut surface as *mut ShadowSurface as u32;
    (raw::exports().draw_shadow_oval)(win, cx, cy, rx, ry, offset_x, offset_y, spread, alpha);
}

// --- Blur API ---

/// Apply a box blur to a rectangular region of a raw pixel buffer.
/// `radius` = blur kernel half-size. `passes` = number of blur iterations (3 ≈ Gaussian).
pub fn blur_rect_buf(
    pixels: &mut [u32], fb_w: u32, fb_h: u32,
    x: i32, y: i32, w: u32, h: u32,
    radius: u32, passes: u32,
) {
    let mut surface = ShadowSurface { pixels: pixels.as_mut_ptr(), width: fb_w, height: fb_h };
    let win = &mut surface as *mut ShadowSurface as u32;
    (raw::exports().blur_rect)(win, x, y, w, h, radius, passes);
}

/// Apply a box blur to a rounded-rect region of a raw pixel buffer.
/// Pixels outside the rounded rect are not modified.
pub fn blur_rounded_rect_buf(
    pixels: &mut [u32], fb_w: u32, fb_h: u32,
    x: i32, y: i32, w: u32, h: u32, r: i32,
    radius: u32, passes: u32,
) {
    let mut surface = ShadowSurface { pixels: pixels.as_mut_ptr(), width: fb_w, height: fb_h };
    let win = &mut surface as *mut ShadowSurface as u32;
    (raw::exports().blur_rounded_rect)(win, x, y, w, h, r, radius, passes);
}
