//! Generic modal dialogs — Error, Warning, Info, Success, Confirm.
//!
//! Creates a temporary centered window, runs its own event loop, and returns
//! the index of the clicked button (0-based). Returns `u32::MAX` if the user
//! presses Escape or closes the dialog.
//!
//! # Usage
//! ```rust,ignore
//! use anyos_std::ui::dialog;
//!
//! // Simple error message
//! dialog::show_error(parent_win, "Error", "File not found.");
//!
//! // Confirmation dialog (returns 0 for Cancel, 1 for OK)
//! let choice = dialog::show_confirm(parent_win, "Delete?", "Are you sure?");
//! ```

use alloc::string::String;
use alloc::vec::Vec;
use crate::ui::window;
use crate::process;

/// Dialog type determines the icon color and symbol.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DialogType {
    Info,
    Warning,
    Error,
    Success,
    Question,
}

// ── Colors ──────────────────────────────────────────────────────────────────

const COLOR_BG: u32 = 0xFF2A2A2A;
const COLOR_TEXT: u32 = 0xFFE6E6E6;
const COLOR_TEXT_SECONDARY: u32 = 0xFF969696;
const COLOR_BTN_BG: u32 = 0xFF3C3C3C;
const COLOR_BTN_PRIMARY: u32 = 0xFF007AFF;
const COLOR_BTN_TEXT: u32 = 0xFFFFFFFF;
const COLOR_ICON_INFO: u32 = 0xFF007AFF;
const COLOR_ICON_WARNING: u32 = 0xFFFFD60A;
const COLOR_ICON_ERROR: u32 = 0xFFFF3B30;
const COLOR_ICON_SUCCESS: u32 = 0xFF30D158;

// ── Layout ──────────────────────────────────────────────────────────────────

const DIALOG_W: u16 = 340;
const DIALOG_H: u16 = 180;
const ICON_SIZE: u16 = 28;
const ICON_X: i16 = 20;
const ICON_Y: i16 = 24;
const TEXT_X: i16 = 60;
const TITLE_Y: i16 = 24;
const MSG_Y: i16 = 50;
const MSG_MAX_W: u16 = 260;
const BTN_H: u16 = 28;
const BTN_PAD: i16 = 12;
const BTN_Y: i16 = (DIALOG_H as i16) - BTN_H as i16 - 16;
const BTN_MIN_W: u16 = 72;

/// Show a modal dialog and return the index of the clicked button.
///
/// `_parent` is reserved for future use (centering relative to parent).
/// `buttons` is a slice of button labels, rendered left-to-right. The last
/// button is styled as the primary (blue) action.
///
/// Returns the 0-based button index, or `u32::MAX` on Escape/close.
pub fn show(
    _parent: u32,
    dtype: DialogType,
    title: &str,
    message: &str,
    buttons: &[&str],
) -> u32 {
    if buttons.is_empty() {
        return u32::MAX;
    }

    // Center the dialog on screen
    let (sw, sh) = window::screen_size();
    let dx = ((sw as i32 - DIALOG_W as i32) / 2).max(0) as u16;
    let dy = ((sh as i32 - DIALOG_H as i32) / 2).max(0) as u16;

    let win = window::create_ex("", dx, dy, DIALOG_W, DIALOG_H,
        window::WIN_FLAG_NOT_RESIZABLE);
    if win == u32::MAX {
        return u32::MAX;
    }

    // Pre-compute button positions (right-aligned)
    let btn_positions = compute_button_positions(buttons);

    // Initial render
    render_dialog(win, dtype, title, message, buttons, &btn_positions);
    window::present(win);

    // Event loop
    let mut event = [0u32; 5];
    let result;
    loop {
        if window::get_event(win, &mut event) == 1 {
            match event[0] {
                window::EVENT_KEY_DOWN => {
                    let key = event[1];
                    if key == 0x103 {
                        // Escape
                        result = u32::MAX;
                        break;
                    }
                    if key == 0x100 {
                        // Enter → activate last (primary) button
                        result = (buttons.len() - 1) as u32;
                        break;
                    }
                }
                window::EVENT_MOUSE_DOWN => {
                    let mx = event[1] as i16;
                    let my = event[2] as i16;
                    if let Some(idx) = hit_test_buttons(mx, my, &btn_positions) {
                        result = idx as u32;
                        break;
                    }
                }
                window::EVENT_WINDOW_CLOSE => {
                    result = u32::MAX;
                    break;
                }
                _ => {}
            }
        }
        process::yield_cpu();
    }

    window::destroy(win);
    result
}

// ── Convenience functions ───────────────────────────────────────────────────

pub fn show_error(_parent: u32, title: &str, msg: &str) -> u32 {
    show(_parent, DialogType::Error, title, msg, &["OK"])
}

pub fn show_warning(_parent: u32, title: &str, msg: &str) -> u32 {
    show(_parent, DialogType::Warning, title, msg, &["OK"])
}

pub fn show_info(_parent: u32, title: &str, msg: &str) -> u32 {
    show(_parent, DialogType::Info, title, msg, &["OK"])
}

pub fn show_confirm(_parent: u32, title: &str, msg: &str) -> u32 {
    show(_parent, DialogType::Question, title, msg, &["Cancel", "OK"])
}

pub fn show_success(_parent: u32, title: &str, msg: &str) -> u32 {
    show(_parent, DialogType::Success, title, msg, &["OK"])
}

// ── Internal rendering ──────────────────────────────────────────────────────

struct BtnRect {
    x: i16,
    y: i16,
    w: u16,
}

fn compute_button_positions(buttons: &[&str]) -> Vec<BtnRect> {
    let mut rects = Vec::with_capacity(buttons.len());
    // Measure all buttons from right to left
    let mut right_edge = DIALOG_W as i16 - 16;
    for &label in buttons.iter().rev() {
        let (tw, _) = window::font_measure(0, 13, label);
        let bw = (tw as u16 + 24).max(BTN_MIN_W);
        let bx = right_edge - bw as i16;
        rects.push(BtnRect { x: bx, y: BTN_Y, w: bw });
        right_edge = bx - BTN_PAD;
    }
    rects.reverse();
    rects
}

fn render_dialog(
    win: u32,
    dtype: DialogType,
    title: &str,
    message: &str,
    buttons: &[&str],
    btn_positions: &[BtnRect],
) {
    // Background
    window::fill_rect(win, 0, 0, DIALOG_W, DIALOG_H, COLOR_BG);

    // Icon circle
    let icon_color = match dtype {
        DialogType::Info | DialogType::Question => COLOR_ICON_INFO,
        DialogType::Warning => COLOR_ICON_WARNING,
        DialogType::Error => COLOR_ICON_ERROR,
        DialogType::Success => COLOR_ICON_SUCCESS,
    };
    let icon_cx = ICON_X + ICON_SIZE as i16 / 2;
    let icon_cy = ICON_Y + ICON_SIZE as i16 / 2;
    window::fill_rounded_rect(win, ICON_X, ICON_Y, ICON_SIZE, ICON_SIZE, ICON_SIZE / 2, icon_color);

    // Icon symbol (centered in the circle)
    let symbol = match dtype {
        DialogType::Info => "i",
        DialogType::Warning => "!",
        DialogType::Error => "X",
        DialogType::Success => "v",
        DialogType::Question => "?",
    };
    let (sw, sh) = window::font_measure(0, 13, symbol);
    let sym_x = icon_cx - sw as i16 / 2;
    let sym_y = icon_cy - sh as i16 / 2;
    window::draw_text_ex(win, sym_x, sym_y, COLOR_BTN_TEXT, 0, 13, symbol);

    // Title text
    window::draw_text_ex(win, TEXT_X, TITLE_Y, COLOR_TEXT, 0, 15, title);

    // Message text (simple word-wrap)
    render_wrapped_text(win, TEXT_X, MSG_Y, MSG_MAX_W, COLOR_TEXT_SECONDARY, message);

    // Buttons
    for (i, (&label, rect)) in buttons.iter().zip(btn_positions.iter()).enumerate() {
        let is_primary = i == buttons.len() - 1;
        let bg = if is_primary { COLOR_BTN_PRIMARY } else { COLOR_BTN_BG };
        window::fill_rounded_rect(win, rect.x, rect.y, rect.w, BTN_H, 6, bg);

        // Center label
        let (tw, th) = window::font_measure(0, 13, label);
        let tx = rect.x + (rect.w as i16 - tw as i16) / 2;
        let ty = rect.y + (BTN_H as i16 - th as i16) / 2;
        window::draw_text_ex(win, tx, ty, COLOR_BTN_TEXT, 0, 13, label);
    }
}

fn render_wrapped_text(win: u32, x: i16, y: i16, max_w: u16, color: u32, text: &str) {
    let line_h: i16 = 16;
    let mut cy = y;
    let mut line_start = 0;
    let bytes = text.as_bytes();

    while line_start < bytes.len() {
        // Find the longest substring that fits in max_w
        let mut end = bytes.len().min(line_start + 80); // safety cap
        let mut last_space = None;

        for i in line_start..end {
            if bytes[i] == b' ' {
                last_space = Some(i);
            }
            let slice = &text[line_start..=i];
            let (tw, _) = window::font_measure(0, 13, slice);
            if tw > max_w as u32 {
                // Exceeded width — break at last space or here
                end = last_space.unwrap_or(i);
                break;
            }
        }

        if end <= line_start {
            end = line_start + 1; // force progress
        }

        let line = &text[line_start..end];
        window::draw_text_ex(win, x, cy, color, 0, 13, line);
        cy += line_h;
        line_start = end;
        // Skip leading space
        if line_start < bytes.len() && bytes[line_start] == b' ' {
            line_start += 1;
        }
    }
}

fn hit_test_buttons(mx: i16, my: i16, positions: &[BtnRect]) -> Option<usize> {
    for (i, rect) in positions.iter().enumerate() {
        if mx >= rect.x && mx < rect.x + rect.w as i16
            && my >= rect.y && my < rect.y + BTN_H as i16
        {
            return Some(i);
        }
    }
    None
}
