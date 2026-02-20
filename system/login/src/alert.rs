//! Modal error alert dialog.

use anyos_std::process;
use anyos_std::ui::window;
use uisys_client::*;

const ALERT_W: u32 = 300;
const ALERT_H: u32 = 150;
const ALERT_BTN_W: u32 = 80;
const ALERT_BTN_H: u32 = 30;

/// Show a modal error alert and wait for the user to dismiss it.
pub fn show_error_alert(_parent_win: u32) {
    let (sw, sh) = window::screen_size();
    let ax = ((sw as i32 - ALERT_W as i32) / 2).max(0);
    let ay = ((sh as i32 - ALERT_H as i32) / 2).max(0);

    let flags = window::WIN_FLAG_BORDERLESS
        | window::WIN_FLAG_SHADOW
        | window::WIN_FLAG_NOT_RESIZABLE
        | window::WIN_FLAG_ALWAYS_ON_TOP
        | window::WIN_FLAG_NO_CLOSE
        | window::WIN_FLAG_NO_MINIMIZE
        | window::WIN_FLAG_NO_MAXIMIZE;
    let alert_id = window::create_ex(
        "Error", ax as u16, ay as u16, ALERT_W as u16, ALERT_H as u16, flags,
    );
    if alert_id == u32::MAX {
        return;
    }

    let btn_x = (ALERT_W as i32 - ALERT_BTN_W as i32) / 2;
    let btn_y = ALERT_H as i32 - ALERT_BTN_H as i32 - 16;
    let mut ok_btn = UiButton::new(btn_x, btn_y, ALERT_BTN_W, ALERT_BTN_H, ButtonStyle::Primary);

    let mut dirty = true;
    let mut event = [0u32; 5];
    loop {
        if dirty {
            let bg = colors::WINDOW_BG();
            window::fill_rect(alert_id, 0, 0, ALERT_W as u16, ALERT_H as u16, bg);

            alert(
                alert_id, 0, 0, ALERT_W, ALERT_H - 50,
                "Login Failed", "Invalid username or password.\nPlease try again.",
            );

            ok_btn.render(alert_id, "OK");
            window::present(alert_id);
            dirty = false;
        }

        while window::get_event(alert_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);
            dirty = true;

            if ev.is_key_down() && (ev.key_code() == KEY_ENTER || ev.key_code() == KEY_ESCAPE) {
                window::destroy(alert_id);
                return;
            }

            if ok_btn.handle_event(&ev) {
                window::destroy(alert_id);
                return;
            }
        }

        process::sleep(16);
    }
}
