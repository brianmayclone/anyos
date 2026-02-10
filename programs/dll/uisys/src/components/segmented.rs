//! SegmentedControl — horizontal segment picker (tab-like selector).

use crate::draw;
use crate::theme;

const CORNER: u32 = 6;

/// Render a segmented control.
///
/// labels: pointer to a concatenated byte buffer of all segment label strings (each null-terminated).
/// num_segments: number of segments.
/// label_offsets: pointer to an array of `num_segments` u32 offsets into `labels` where each
///                segment's null-terminated string begins.
/// selected: index of the currently selected segment (0-based).
pub extern "C" fn segmented_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    labels: *const u8, num_segments: u32,
    label_offsets: *const u32, selected: u32,
) {
    if num_segments == 0 { return; }

    // Outer rounded background
    draw::fill_rounded_rect(win, x, y, w, h, CORNER, theme::CONTROL_PRESSED);

    let seg_w = w / num_segments;

    for i in 0..num_segments {
        let seg_x = x + (i * seg_w) as i32;
        let is_selected = i == selected;

        if is_selected {
            // Selected segment: lighter background
            let sel_corner = if i == 0 || i == num_segments - 1 { CORNER } else { 4 };
            draw::fill_rounded_rect(win, seg_x + 2, y + 2, seg_w - 4, h - 4, sel_corner, theme::CONTROL_BG);
        }

        // Draw label text centered in segment
        if !labels.is_null() && !label_offsets.is_null() {
            let offset = unsafe { *label_offsets.add(i as usize) } as usize;
            let label_ptr = unsafe { labels.add(offset) };

            // Find label length (up to NUL)
            let mut len = 0u32;
            unsafe {
                while *label_ptr.add(len as usize) != 0 && len < 64 {
                    len += 1;
                }
            }

            if len > 0 {
                let label_slice = unsafe { core::slice::from_raw_parts(label_ptr, len as usize) };
                let (tw, th) = draw::text_size(label_slice);
                let text_x = seg_x + (seg_w as i32 - tw as i32) / 2;
                let text_y = y + (h as i32 - th as i32) / 2;
                let fg = if is_selected { theme::TEXT } else { theme::TEXT_SECONDARY };
                draw::draw_text(win, text_x, text_y, fg, label_slice);
            }
        }
    }
}

/// Hit test: returns the segment index (0-based) if (mx,my) is inside,
/// or 0xFFFFFFFF if outside.
pub extern "C" fn segmented_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    num_segments: u32,
    mx: i32, my: i32,
) -> u32 {
    if num_segments == 0 { return 0xFFFFFFFF; }
    if mx < x || mx >= x + w as i32 || my < y || my >= y + h as i32 {
        return 0xFFFFFFFF;
    }
    let seg_w = w / num_segments;
    let offset = (mx - x) as u32;
    let idx = offset / seg_w;
    if idx >= num_segments { num_segments - 1 } else { idx }
}
