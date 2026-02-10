//! TabBar — bottom tab navigation with multiple selectable tabs.

use crate::draw;
use crate::theme;

/// Render a tab bar with labeled tabs.
/// `labels` points to a concatenated, null-terminated string buffer where each
/// tab label is placed consecutively.
/// `label_offsets` is an array of `num_tabs` byte offsets into `labels` for each tab.
/// `selected` is the 0-based index of the active tab.
pub extern "C" fn tabbar_render(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    labels: *const u8, num_tabs: u32,
    label_offsets: *const u32, selected: u32,
) {
    if num_tabs == 0 {
        return;
    }

    // Background
    draw::fill_rect(win, x, y, w, h, theme::SIDEBAR_BG);

    // Top separator
    draw::fill_rect(win, x, y, w, 1, theme::SEPARATOR);

    let tab_w = w / num_tabs;

    for i in 0..num_tabs {
        let tab_x = x + (i * tab_w) as i32;
        let is_selected = i == selected;

        // Active indicator (small bar at top of selected tab)
        if is_selected {
            draw::fill_rect(win, tab_x, y, tab_w, 2, theme::ACCENT);
        }

        // Tab label
        if !labels.is_null() && !label_offsets.is_null() {
            let offset = unsafe { *label_offsets.add(i as usize) } as usize;
            let label_ptr = unsafe { labels.add(offset) };

            // Compute label length (scan for NUL)
            let mut label_len = 0u32;
            unsafe {
                while *label_ptr.add(label_len as usize) != 0 {
                    label_len += 1;
                }
            }

            if label_len > 0 {
                let label_slice = unsafe {
                    core::slice::from_raw_parts(label_ptr, label_len as usize)
                };
                let (tw, th) = draw::text_size(label_slice);
                let text_x = tab_x + (tab_w as i32 - tw as i32) / 2;
                let text_y = y + (h as i32 - th as i32) / 2;
                let fg = if is_selected { theme::ACCENT } else { theme::TEXT_SECONDARY };
                draw::draw_text(win, text_x, text_y, fg, label_slice);
            }
        }
    }
}

/// Hit test: return the 0-based tab index at mouse position, or 0xFFFFFFFF if none.
pub extern "C" fn tabbar_hit_test(
    x: i32, y: i32, w: u32, h: u32,
    num_tabs: u32, mx: i32, my: i32,
) -> u32 {
    if num_tabs == 0 {
        return 0xFFFFFFFF;
    }
    if mx < x || mx >= x + w as i32 || my < y || my >= y + h as i32 {
        return 0xFFFFFFFF;
    }
    let relative_x = (mx - x) as u32;
    let tab_w = w / num_tabs;
    let index = relative_x / tab_w;
    if index >= num_tabs {
        num_tabs - 1
    } else {
        index
    }
}
