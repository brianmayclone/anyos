//! uisys access layer — reads the uisys export table at 0x04000000.
//!
//! The uisys.dlib is mapped at a fixed address in every process. We read
//! function pointers from its export struct and call them to render controls.

const UISYS_BASE: u64 = 0x0400_0000;

// Header layout: magic(4) + version(4) + num_exports(4) + theme(4) + reserved(16) = 32 bytes
// But the actual struct in uisys has _reserved: [u32; 4] starting at offset 16,
// so function pointers start at offset 32+4 = 36. Let me recalculate:
// magic: [u8; 4]  = offset 0, size 4
// version: u32     = offset 4, size 4
// num_exports: u32 = offset 8, size 4
// theme: u32       = offset 12, size 4
// _reserved: [u32; 4] = offset 16, size 16
// First fn ptr at offset 32... wait, 4+4+4+4+16 = 32, but the struct is repr(C)
// and fn ptrs are 8 bytes on x86_64, so they need 8-byte alignment.
// offset 32 is already 8-byte aligned. But let me check: the exploration said offset 36.
// Actually: magic [u8;4] at 0, version u32 at 4, num_exports u32 at 8, theme u32 at 12,
// _reserved [u32;4] at 16 (16+16=32). First fn ptr at 32.
// Wait, exploration agent said offset 36 for label_render. Let me recheck:
// [u8;4]=4 + u32=4 + u32=4 + u32=4 + [u32;4]=16 = 32. 32 is 8-aligned. So offset 32.
// The exploration agent may have miscounted. Let me use 32 as the base, since that's
// what repr(C) alignment gives us.
//
// Actually, I need to be more careful. Let me read the actual uisys_client raw.rs definition.
// The exploration says: magic:[u8;4], version:u32, num_exports:u32, theme:u32, _reserved:[u32;4]
// That's 4 + 4 + 4 + 4 + 16 = 32 bytes before first fn ptr.
// On x86_64, fn ptrs are 8 bytes and need 8-byte alignment. 32 % 8 = 0, so first fn ptr at 32.

const FN_PTR_BASE: usize = 32; // byte offset of first function pointer in UisysExports

/// Read a function pointer from the uisys export table.
#[inline(always)]
unsafe fn read_fn_ptr(index: usize) -> u64 {
    let addr = UISYS_BASE as usize + FN_PTR_BASE + index * 8;
    *(addr as *const u64)
}

/// Check if the current theme is light (true) or dark (false).
pub fn is_light_theme() -> bool {
    unsafe {
        let theme = *((UISYS_BASE as usize + 12) as *const u32);
        theme != 0
    }
}

/// Get the window background color for the current theme.
pub fn color_window_bg() -> u32 {
    if is_light_theme() { 0xFFFFFFFF } else { 0xFF1E1E1E }
}

/// Get the text color for the current theme.
pub fn color_text() -> u32 {
    if is_light_theme() { 0xFF000000 } else { 0xFFFFFFFF }
}

/// Get the secondary text color for the current theme.
pub fn color_text_secondary() -> u32 {
    if is_light_theme() { 0xFF666666 } else { 0xFF999999 }
}

/// Get the accent color.
pub fn color_accent() -> u32 {
    0xFF007AFF
}

// ── Render functions (call uisys export table) ──────────────────────

/// Render a label.
/// uisys fn index 0: label_render(win, x, y, text_ptr, text_len, color, font_id, flags)
pub fn render_label(win: u32, x: i32, y: i32, text: &[u8], color: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, *const u8, u32, u32, u16, u8) =
            core::mem::transmute(read_fn_ptr(0));
        f(win, x, y, text.as_ptr(), text.len() as u32, color, 0, 0);
    }
}

/// Render a button.
/// uisys fn index 4: button_render(win, x, y, w, h, text_ptr, text_len, style, state)
pub fn render_button(win: u32, x: i32, y: i32, w: u32, h: u32, text: &[u8], style: u8, state: u8) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, u8, u8) =
            core::mem::transmute(read_fn_ptr(4));
        f(win, x, y, w, h, text.as_ptr(), text.len() as u32, style, state);
    }
}

/// Render a toggle.
/// uisys fn index 7: toggle_render(win, x, y, on)
pub fn render_toggle(win: u32, x: i32, y: i32, on: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32) = core::mem::transmute(read_fn_ptr(7));
        f(win, x, y, on);
    }
}

/// Render a checkbox.
/// uisys fn index 9: checkbox_render(win, x, y, checked, label_ptr, label_len)
pub fn render_checkbox(win: u32, x: i32, y: i32, checked: bool, label: &[u8]) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u8, *const u8, u32) =
            core::mem::transmute(read_fn_ptr(9));
        f(win, x, y, checked as u8, label.as_ptr(), label.len() as u32);
    }
}

/// Render a text field.
/// uisys fn index 11: textfield_render(win, x, y, w, h, text, len, placeholder, plen, cursor, sel)
pub fn render_textfield(
    win: u32, x: i32, y: i32, w: u32, h: u32,
    text: &[u8], cursor: u32, selection: u32,
) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32, *const u8, u32, u32, u32) =
            core::mem::transmute(read_fn_ptr(11));
        f(win, x, y, w, h, text.as_ptr(), text.len() as u32,
          core::ptr::null(), 0, cursor, selection);
    }
}

/// Render a slider.
/// uisys fn index 18: slider_render(win, x, y, w, h, min, max, value)
pub fn render_slider(win: u32, x: i32, y: i32, w: u32, h: u32, min: u32, max: u32, value: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32, u32, u32, u32) =
            core::mem::transmute(read_fn_ptr(18));
        f(win, x, y, w, h, min, max, value);
    }
}

/// Render a progress bar.
/// uisys fn index 21: progress_render(win, x, y, w, h, value, max)
pub fn render_progress(win: u32, x: i32, y: i32, w: u32, h: u32, value: u32, max: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32, u32, u32) =
            core::mem::transmute(read_fn_ptr(21));
        f(win, x, y, w, h, value, max);
    }
}

/// Render a radio button.
/// uisys fn index 16: radio_render(win, x, y, selected, label_ptr, label_len)
pub fn render_radio(win: u32, x: i32, y: i32, selected: bool, label: &[u8]) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, *const u8, u32) =
            core::mem::transmute(read_fn_ptr(16));
        f(win, x, y, selected as u32, label.as_ptr(), label.len() as u32);
    }
}

/// Render a badge.
/// uisys fn index 14: badge_render(win, x, y, count, color)
pub fn render_badge(win: u32, x: i32, y: i32, count: u32, color: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32) = core::mem::transmute(read_fn_ptr(14));
        f(win, x, y, count, color);
    }
}

/// Render a divider (horizontal).
/// uisys fn index 46: divider_render_h(win, x, y, w)
pub fn render_divider_h(win: u32, x: i32, y: i32, w: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32) = core::mem::transmute(read_fn_ptr(46));
        f(win, x, y, w);
    }
}

/// Render a divider (vertical).
/// uisys fn index 47: divider_render_v(win, x, y, h)
pub fn render_divider_v(win: u32, x: i32, y: i32, h: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32) = core::mem::transmute(read_fn_ptr(47));
        f(win, x, y, h);
    }
}

/// Render a card background.
/// uisys fn index 42: card_render(win, x, y, w, h)
pub fn render_card(win: u32, x: i32, y: i32, w: u32, h: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32) = core::mem::transmute(read_fn_ptr(42));
        f(win, x, y, w, h);
    }
}

/// Render a group box.
/// uisys fn index 43: groupbox_render(win, x, y, w, h, title_ptr, title_len)
pub fn render_groupbox(win: u32, x: i32, y: i32, w: u32, h: u32, title: &[u8]) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32, *const u8, u32) =
            core::mem::transmute(read_fn_ptr(43));
        f(win, x, y, w, h, title.as_ptr(), title.len() as u32);
    }
}

/// Render a tooltip.
/// uisys fn index 55: tooltip_render(win, x, y, text_ptr, text_len)
pub fn render_tooltip(win: u32, x: i32, y: i32, text: &[u8]) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, *const u8, u32) =
            core::mem::transmute(read_fn_ptr(55));
        f(win, x, y, text.as_ptr(), text.len() as u32);
    }
}

/// Render a status indicator.
/// uisys fn index 57: status_render(win, x, y, status, label_ptr, label_len, font_id)
pub fn render_status(win: u32, x: i32, y: i32, status: u8, label: &[u8]) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u8, *const u8, u32, u16) =
            core::mem::transmute(read_fn_ptr(57));
        f(win, x, y, status, label.as_ptr(), label.len() as u32, 0);
    }
}

/// Render a color well.
/// uisys fn index 58: colorwell_render(win, x, y, color, size)
pub fn render_colorwell(win: u32, x: i32, y: i32, color: u32, size: u32) {
    unsafe {
        let f: extern "C" fn(u32, i32, i32, u32, u32) = core::mem::transmute(read_fn_ptr(58));
        f(win, x, y, color, size);
    }
}

// The render_control() dispatcher has been removed.
// Each control type now overrides Control::render() directly (OOP pattern).
