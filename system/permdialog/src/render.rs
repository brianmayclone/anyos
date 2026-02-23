//! Dialog rendering, layout constants, and cleanup.

use anyos_std::ipc;
use anyos_std::process;
use anyos_std::ui::window;
use uisys_client::*;

use crate::perms::PERM_GROUPS;

// ── Layout constants ──

pub const DIALOG_W: u32 = 380;
pub const PAD: i32 = 24;
pub const ROW_H: i32 = 48;
pub const BTN_W: u32 = 130;
pub const BTN_H: u32 = 34;
pub const BTN_PAD: i32 = 12;

/// Destroy window, release lock pipe, and exit.
pub fn cleanup(lock_pipe: u32, win: u32, code: u32) -> ! {
    if win != u32::MAX {
        window::destroy(win);
    }
    if lock_pipe != 0 && lock_pipe != u32::MAX {
        ipc::pipe_close(lock_pipe);
    }
    process::exit(code);
}

pub fn render(
    win: u32,
    w: u32,
    h: u32,
    app_name: &str,
    active_groups: &[(usize, bool); 6],
    num_active: usize,
    checkboxes: &[UiCheckbox; 6],
    deny_btn: &UiButton,
    allow_btn: &UiButton,
) {
    let bg = colors::WINDOW_BG();
    let text_color = colors::TEXT();
    let secondary = colors::TEXT_SECONDARY();

    window::fill_rect(win, 0, 0, w as u16, h as u16, bg);

    let mut title_buf = [0u8; 128];
    let title_len = format_title(app_name, &mut title_buf);
    let title = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("App wants access to:");

    label(win, PAD, 20, title, text_color, FontSize::Normal, TextAlign::Left);
    label(
        win, PAD, 46,
        "This app requests the following permissions:",
        secondary, FontSize::Small, TextAlign::Left,
    );

    divider_h(win, PAD, 74, w - PAD as u32 * 2);

    let header_h: i32 = 80;
    for i in 0..num_active {
        let group = &PERM_GROUPS[active_groups[i].0];
        let y = header_h + (i as i32 * ROW_H);

        checkboxes[i].render(win, group.name);
        label(win, PAD + 30, y + 28, group.desc, secondary, FontSize::Small, TextAlign::Left);

        if i + 1 < num_active {
            divider_h(win, PAD, y + ROW_H - 2, w - PAD as u32 * 2);
        }
    }

    let btn_div_y = h as i32 - BTN_H as i32 - BTN_PAD * 2 - 12;
    divider_h(win, PAD, btn_div_y, w - PAD as u32 * 2);

    deny_btn.render(win, "Don't Allow");
    allow_btn.render(win, "Allow");

    window::present(win);
}

fn format_title(app_name: &str, buf: &mut [u8]) -> usize {
    let mut pos = 0usize;
    let max = buf.len().saturating_sub(1);

    // Opening double quote (UTF-8: E2 80 9C)
    if pos + 2 < max {
        buf[pos] = 0xE2;
        buf[pos + 1] = 0x80;
        buf[pos + 2] = 0x9C;
        pos += 3;
    }

    for &b in app_name.as_bytes() {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    // Closing double quote (UTF-8: E2 80 9D)
    if pos + 2 < max {
        buf[pos] = 0xE2;
        buf[pos + 1] = 0x80;
        buf[pos + 2] = 0x9D;
        pos += 3;
    }

    for &b in b" wants access to:" {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    pos
}
