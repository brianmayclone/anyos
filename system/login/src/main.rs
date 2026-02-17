#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::ui::window;
use anyos_std::process;
use uisys_client::*;

const DIALOG_W: u32 = 340;
const DIALOG_H: u32 = 280;
const FIELD_W: u32 = 280;
const FIELD_H: u32 = 28;
const BTN_W: u32 = 120;
const BTN_H: u32 = 32;
const PAD: i32 = 30;

fn main() -> u32 {
    // Get screen size to center the window
    let (sw, sh) = window::screen_size();
    let wx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let wy = ((sh as i32 - DIALOG_H as i32) / 2).max(0);

    let win_id = window::create("Login", wx as u16, wy as u16, DIALOG_W as u16, DIALOG_H as u16);
    if win_id == u32::MAX {
        return 0; // Fallback: continue as root
    }

    let field_x = PAD;
    let mut username_field = UiTextField::new(field_x, 70, FIELD_W, FIELD_H);
    username_field.focused = true;

    let mut password_field = UiTextField::new(field_x, 140, FIELD_W, FIELD_H);
    password_field.password = true;

    let btn_x = (DIALOG_W as i32 - BTN_W as i32) / 2;
    let mut login_btn = UiButton::new(btn_x, 200, BTN_W, BTN_H, ButtonStyle::Primary);

    let mut error_msg = false;
    let mut focused_field: u8 = 0; // 0=username, 1=password

    let mut event = [0u32; 5];

    loop {
        // ── Render ──
        let bg = colors::WINDOW_BG();
        window::fill_rect(win_id, 0, 0, DIALOG_W as u16, DIALOG_H as u16, bg);

        // Title
        label(win_id, PAD, 20, "Welcome to anyOS", colors::TEXT(), FontSize::Large, TextAlign::Left);

        // Username label + field
        label(win_id, field_x, 55, "Username", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        username_field.render(win_id, "Enter username");

        // Password label + field
        label(win_id, field_x, 125, "Password", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
        password_field.render(win_id, "Enter password");

        // Login button
        login_btn.render(win_id, "Log In");

        // Error message
        if error_msg {
            label(win_id, PAD, 245, "Invalid credentials. Try again.", 0xFFFF4444, FontSize::Small, TextAlign::Left);
        }

        window::present(win_id);

        // ── Events ──
        if window::get_event(win_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);

            if event[0] == EVENT_WINDOW_CLOSE {
                // Don't allow closing the login window
                continue;
            }

            // Tab switches focus between fields
            if ev.is_key_down() && ev.key_code() == KEY_TAB {
                if focused_field == 0 {
                    focused_field = 1;
                    username_field.focused = false;
                    password_field.focused = true;
                } else {
                    focused_field = 0;
                    username_field.focused = true;
                    password_field.focused = false;
                }
                continue;
            }

            // Enter submits login
            if ev.is_key_down() && ev.key_code() == KEY_ENTER {
                let uid = try_login(&username_field, &password_field);
                if uid != u32::MAX {
                    window::destroy(win_id);
                    return uid;
                } else {
                    error_msg = true;
                }
                continue;
            }

            // Mouse click to focus fields
            if ev.is_mouse_down() {
                let (mx, my) = ev.mouse_pos();
                if textfield_hit_test(username_field.x, username_field.y, username_field.w, username_field.h, mx, my) {
                    focused_field = 0;
                    username_field.focused = true;
                    password_field.focused = false;
                } else if textfield_hit_test(password_field.x, password_field.y, password_field.w, password_field.h, mx, my) {
                    focused_field = 1;
                    username_field.focused = false;
                    password_field.focused = true;
                }
            }

            // Button click
            if login_btn.handle_event(&ev) {
                let uid = try_login(&username_field, &password_field);
                if uid != u32::MAX {
                    window::destroy(win_id);
                    return uid;
                } else {
                    error_msg = true;
                }
            }

            // Forward events to focused field
            if focused_field == 0 {
                username_field.handle_event(&ev);
            } else {
                password_field.handle_event(&ev);
            }
        }

        process::sleep(16);
    }
}

/// Try to authenticate. Returns the uid on success, or u32::MAX on failure.
fn try_login(username_field: &UiTextField, password_field: &UiTextField) -> u32 {
    let username = username_field.text();
    let password = password_field.text();

    if username.is_empty() {
        return u32::MAX;
    }

    if process::authenticate(username, password) {
        // Authentication succeeded — return the uid
        process::getuid() as u32
    } else {
        u32::MAX
    }
}
