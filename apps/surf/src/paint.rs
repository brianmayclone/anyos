//! Painter for the Surf web browser.
//!
//! Walks the layout tree produced by `layout.rs` and renders each box to a
//! raw ARGB pixel buffer.  Uses uisys font rendering for text and provides
//! software fallbacks for rectangles, lines, and image blitting.

use alloc::string::String;
use alloc::vec::Vec;

use crate::layout::{BoxType, Edges, LayoutBox};
use crate::style::{TextAlignVal, TextDeco};

// ---------------------------------------------------------------------------
// Image cache
// ---------------------------------------------------------------------------

pub struct ImageCache {
    pub entries: Vec<ImageEntry>,
}

pub struct ImageEntry {
    pub src: String,
    pub pixels: Vec<u32>,
    pub width: u32,
    pub height: u32,
}

impl ImageCache {
    pub fn new() -> Self {
        ImageCache { entries: Vec::new() }
    }

    pub fn get(&self, src: &str) -> Option<&ImageEntry> {
        self.entries.iter().find(|e| e.src == src)
    }
}

// ---------------------------------------------------------------------------
// WinSurface shim for uisys draw_text_with_font
// ---------------------------------------------------------------------------

/// Must match the DLL's internal WinSurface layout so that
/// `draw_text_with_font(win, ...)` can dereference `win` as a pointer.
#[repr(C)]
struct WinSurface {
    pixels: *mut u32,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// Main paint entry point
// ---------------------------------------------------------------------------

/// Render the layout tree to a pixel buffer.
///
/// `pixels` is a row-major ARGB buffer of size `width * height`.
/// `scroll_y` is the vertical scroll offset (positive = scrolled down).
/// `images` provides decoded image data for `<img>` elements.
pub fn paint(
    root: &LayoutBox,
    pixels: &mut [u32],
    width: u32,
    height: u32,
    scroll_y: i32,
    images: &ImageCache,
) {
    // Clear to white.
    let len = (width * height) as usize;
    for i in 0..len.min(pixels.len()) {
        pixels[i] = 0xFFFFFFFF;
    }

    paint_box(root, pixels, width, height, scroll_y, 0, 0, images);
}

/// Recursively paint a layout box and its children.
fn paint_box(
    bx: &LayoutBox,
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    scroll_y: i32,
    parent_x: i32,
    parent_y: i32,
    images: &ImageCache,
) {
    let abs_x = parent_x + bx.x;
    let abs_y = parent_y + bx.y - scroll_y;

    // Cull boxes entirely outside the viewport.
    if abs_y + bx.height < 0 || abs_y > buf_h as i32 {
        return;
    }

    // 1. Background.
    if bx.bg_color != 0 {
        if bx.border_radius > 0 {
            fill_rounded_rect(
                pixels, buf_w, buf_h,
                abs_x, abs_y, bx.width, bx.height,
                bx.border_radius, bx.bg_color,
            );
        } else {
            fill_rect(pixels, buf_w, buf_h, abs_x, abs_y, bx.width, bx.height, bx.bg_color);
        }
    }

    // 2. Border.
    if bx.border_width > 0 && bx.border_color != 0 {
        draw_border(
            pixels, buf_w, buf_h,
            abs_x, abs_y, bx.width, bx.height,
            bx.border_width, bx.border_color,
        );
    }

    // 3. Horizontal rule.
    if bx.is_hr {
        let hr_y = abs_y + bx.padding.top;
        draw_line_h(pixels, buf_w, buf_h, abs_x + 4, hr_y, bx.width - 8, 0xFFCCCCCC);
    }

    // 4. List marker.
    if let Some(ref marker) = bx.list_marker {
        let marker_w = measure_text_width(marker, bx.font_size, bx.bold);
        let mx = abs_x - marker_w - 2;
        let my = abs_y + bx.padding.top;
        draw_text_to_buffer(
            pixels, buf_w, buf_h, mx, my,
            marker, bx.font_size, bx.bold, bx.color,
        );
    }

    // 5. Text.
    if let Some(ref text) = bx.text {
        if !text.is_empty() {
            let tx = abs_x;
            let ty = abs_y;
            draw_text_to_buffer(pixels, buf_w, buf_h, tx, ty, text, bx.font_size, bx.bold, bx.color);

            // Text decorations.
            let (tw, th) = measure_text_dims(text, bx.font_size, bx.bold);
            match bx.text_decoration {
                TextDeco::Underline => {
                    let uy = ty + th + 1;
                    draw_line_h(pixels, buf_w, buf_h, tx, uy, tw, bx.color);
                }
                TextDeco::LineThrough => {
                    let sy = ty + th / 2;
                    draw_line_h(pixels, buf_w, buf_h, tx, sy, tw, bx.color);
                }
                TextDeco::None => {}
            }
        }
    }

    // 6. Images.
    if let Some(ref src) = bx.image_src {
        if let Some(entry) = images.get(src) {
            let iw = bx.image_width.unwrap_or(entry.width as i32);
            let ih = bx.image_height.unwrap_or(entry.height as i32);
            let ix = abs_x + bx.padding.left;
            let iy = abs_y + bx.padding.top;
            if entry.width == iw as u32 && entry.height == ih as u32 {
                blit_image(
                    pixels, buf_w, buf_h,
                    ix, iy,
                    &entry.pixels, entry.width, entry.height,
                );
            } else {
                // Simple nearest-neighbour scaling.
                blit_image_scaled(
                    pixels, buf_w, buf_h,
                    ix, iy, iw, ih,
                    &entry.pixels, entry.width, entry.height,
                );
            }
        } else {
            // Placeholder box for images that haven't loaded.
            fill_rect(
                pixels, buf_w, buf_h,
                abs_x + bx.padding.left,
                abs_y + bx.padding.top,
                bx.image_width.unwrap_or(64),
                bx.image_height.unwrap_or(64),
                0xFFE0E0E0,
            );
        }
    }

    // 7. Children.
    for child in &bx.children {
        paint_box(child, pixels, buf_w, buf_h, scroll_y, abs_x, abs_y + scroll_y, images);
    }
}

// ---------------------------------------------------------------------------
// Hit testing
// ---------------------------------------------------------------------------

/// Walk the layout tree and return the URL of the deepest link at `(x, y)`.
/// Coordinates are in document space; `scroll_y` is added to `y`.
pub fn hit_test(root: &LayoutBox, x: i32, y: i32, scroll_y: i32) -> Option<String> {
    hit_test_inner(root, x, y + scroll_y, 0, 0)
}

fn hit_test_inner(
    bx: &LayoutBox,
    x: i32,
    y: i32,
    parent_x: i32,
    parent_y: i32,
) -> Option<String> {
    let abs_x = parent_x + bx.x;
    let abs_y = parent_y + bx.y;

    // Check children first (deepest match wins).
    for child in bx.children.iter().rev() {
        if let Some(url) = hit_test_inner(child, x, y, abs_x, abs_y) {
            return Some(url);
        }
    }

    // Check this box.
    if x >= abs_x && x < abs_x + bx.width && y >= abs_y && y < abs_y + bx.height {
        if let Some(ref url) = bx.link_url {
            return Some(url.clone());
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Document height
// ---------------------------------------------------------------------------

/// Return the total document height for scrollbar calculation.
pub fn total_height(root: &LayoutBox) -> i32 {
    let bottom = root.y + root.height;
    let mut max = bottom;
    for child in &root.children {
        let ch = child_total_height(child, root.y);
        if ch > max {
            max = ch;
        }
    }
    max
}

fn child_total_height(bx: &LayoutBox, parent_y: i32) -> i32 {
    let abs_y = parent_y + bx.y;
    let mut max = abs_y + bx.height;
    for child in &bx.children {
        let ch = child_total_height(child, abs_y);
        if ch > max {
            max = ch;
        }
    }
    max
}

// ---------------------------------------------------------------------------
// Text rendering
// ---------------------------------------------------------------------------

/// Render text into a raw pixel buffer using the uisys font rendering DLL.
pub fn draw_text_to_buffer(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    text: &str,
    font_size: i32,
    bold: bool,
    color: u32,
) {
    if text.is_empty() || x >= buf_w as i32 || y >= buf_h as i32 {
        return;
    }

    let font_id: u16 = if bold { 1 } else { 0 };

    // Construct a WinSurface pointing at our pixel buffer so the DLL can
    // render directly into it.
    let surface = WinSurface {
        pixels: pixels.as_mut_ptr(),
        width: buf_w,
        height: buf_h,
    };
    let win = &surface as *const WinSurface as u32;
    uisys_client::draw_text_with_font(win, x, y, color, font_size as u32, font_id, text);
}

/// Measure text dimensions.
fn measure_text_dims(text: &str, font_size: i32, bold: bool) -> (i32, i32) {
    let font_id: u16 = if bold { 1 } else { 0 };
    let (w, h) = uisys_client::font_measure(font_id, font_size as u16, text);
    (w as i32, h as i32)
}

/// Measure text width only.
fn measure_text_width(text: &str, font_size: i32, bold: bool) -> i32 {
    let (w, _) = measure_text_dims(text, font_size, bold);
    w
}

// ---------------------------------------------------------------------------
// Software drawing primitives
// ---------------------------------------------------------------------------

/// Fill a rectangle with clipping and alpha blending for semi-transparent
/// colors (alpha < 0xFF).
fn fill_rect(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
) {
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x + w) as u32).min(buf_w);
    let y1 = ((y + h) as u32).min(buf_h);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let alpha = (color >> 24) & 0xFF;

    if alpha >= 255 {
        // Opaque fast path.
        for row in y0..y1 {
            let off = (row * buf_w) as usize;
            for col in x0..x1 {
                pixels[off + col as usize] = color;
            }
        }
    } else if alpha > 0 {
        // Alpha blending.
        let sa = alpha;
        let sr = (color >> 16) & 0xFF;
        let sg = (color >> 8) & 0xFF;
        let sb = color & 0xFF;
        let inv_a = 255 - sa;

        for row in y0..y1 {
            let off = (row * buf_w) as usize;
            for col in x0..x1 {
                let dst = pixels[off + col as usize];
                let dr = (dst >> 16) & 0xFF;
                let dg = (dst >> 8) & 0xFF;
                let db = dst & 0xFF;
                let r = (sr * sa + dr * inv_a) / 255;
                let g = (sg * sa + dg * inv_a) / 255;
                let b = (sb * sa + db * inv_a) / 255;
                pixels[off + col as usize] = 0xFF000000 | (r << 16) | (g << 8) | b;
            }
        }
    }
}

/// Draw a horizontal line, 1 pixel tall.
fn draw_line_h(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    color: u32,
) {
    fill_rect(pixels, buf_w, buf_h, x, y, w, 1, color);
}

/// Draw a rectangular border (outline only).
fn draw_border(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    bw: i32,
    color: u32,
) {
    // Top edge.
    fill_rect(pixels, buf_w, buf_h, x, y, w, bw, color);
    // Bottom edge.
    fill_rect(pixels, buf_w, buf_h, x, y + h - bw, w, bw, color);
    // Left edge.
    fill_rect(pixels, buf_w, buf_h, x, y + bw, bw, h - bw * 2, color);
    // Right edge.
    fill_rect(pixels, buf_w, buf_h, x + w - bw, y + bw, bw, h - bw * 2, color);
}

/// Fill a rounded rectangle using a simple circle-test at each corner.
fn fill_rounded_rect(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    color: u32,
) {
    let r = r.min(w / 2).min(h / 2).max(0);
    if r == 0 {
        fill_rect(pixels, buf_w, buf_h, x, y, w, h, color);
        return;
    }

    // For each row, determine the fill extent accounting for rounded corners.
    for dy in 0..h {
        let (x0, x1);
        if dy < r {
            // Top corners.
            let ry = r - dy - 1;
            let rx = corner_inset(r, ry);
            x0 = x + rx;
            x1 = x + w - rx;
        } else if dy >= h - r {
            // Bottom corners.
            let ry = dy - (h - r);
            let rx = corner_inset(r, ry);
            x0 = x + rx;
            x1 = x + w - rx;
        } else {
            x0 = x;
            x1 = x + w;
        }
        fill_rect(pixels, buf_w, buf_h, x0, y + dy, x1 - x0, 1, color);
    }
}

/// Compute the horizontal inset for a corner of radius `r` at vertical
/// offset `ry` from the corner centre. Uses integer arithmetic only.
fn corner_inset(r: i32, ry: i32) -> i32 {
    // isqrt(r*r - ry*ry) subtracted from r
    let rr = r * r;
    let yy = ry * ry;
    if yy >= rr {
        return r;
    }
    let diff = rr - yy;
    let s = isqrt(diff as u32) as i32;
    r - s
}

/// Integer square root (no FPU).
fn isqrt(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Blit image pixels into the buffer with clipping.
fn blit_image(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    for sy in 0..src_h as i32 {
        let dy = y + sy;
        if dy < 0 || dy >= buf_h as i32 {
            continue;
        }
        for sx in 0..src_w as i32 {
            let dx = x + sx;
            if dx < 0 || dx >= buf_w as i32 {
                continue;
            }
            let src_idx = (sy as u32 * src_w + sx as u32) as usize;
            let dst_idx = (dy as u32 * buf_w + dx as u32) as usize;
            if src_idx < src.len() && dst_idx < pixels.len() {
                let sc = src[src_idx];
                let sa = (sc >> 24) & 0xFF;
                if sa >= 255 {
                    pixels[dst_idx] = sc;
                } else if sa > 0 {
                    let dst = pixels[dst_idx];
                    let inv = 255 - sa;
                    let r = (((sc >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv) / 255;
                    let g = (((sc >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv) / 255;
                    let b = ((sc & 0xFF) * sa + (dst & 0xFF) * inv) / 255;
                    pixels[dst_idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}

/// Blit with nearest-neighbour scaling.
fn blit_image_scaled(
    pixels: &mut [u32],
    buf_w: u32,
    buf_h: u32,
    x: i32,
    y: i32,
    dst_w: i32,
    dst_h: i32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) {
    if dst_w <= 0 || dst_h <= 0 || src_w == 0 || src_h == 0 {
        return;
    }
    for dy in 0..dst_h {
        let py = y + dy;
        if py < 0 || py >= buf_h as i32 {
            continue;
        }
        let sy = ((dy as u32) * src_h / dst_h as u32).min(src_h - 1);
        for dx in 0..dst_w {
            let px = x + dx;
            if px < 0 || px >= buf_w as i32 {
                continue;
            }
            let sx = ((dx as u32) * src_w / dst_w as u32).min(src_w - 1);
            let src_idx = (sy * src_w + sx) as usize;
            let dst_idx = (py as u32 * buf_w + px as u32) as usize;
            if src_idx < src.len() && dst_idx < pixels.len() {
                let sc = src[src_idx];
                let sa = (sc >> 24) & 0xFF;
                if sa >= 255 {
                    pixels[dst_idx] = sc;
                } else if sa > 0 {
                    let dst = pixels[dst_idx];
                    let inv = 255 - sa;
                    let r = (((sc >> 16) & 0xFF) * sa + ((dst >> 16) & 0xFF) * inv) / 255;
                    let g = (((sc >> 8) & 0xFF) * sa + ((dst >> 8) & 0xFF) * inv) / 255;
                    let b = ((sc & 0xFF) * sa + (dst & 0xFF) * inv) / 255;
                    pixels[dst_idx] = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
        }
    }
}
