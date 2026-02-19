#![no_std]
#![no_main]

anyos_std::entry!(main);

use alloc::vec;
use alloc::vec::Vec;
use anyos_std::ui::window;
use anyos_std::process;
use anyos_std::fs;
use uisys_client::*;

const DIALOG_W: u32 = 340;
const DIALOG_H: u32 = 340;
const FIELD_W: u32 = 280;
const FIELD_H: u32 = 28;
const BTN_W: u32 = 280;
const BTN_H: u32 = 34;
const PAD: i32 = 30;

// Error alert dialog dimensions
const ALERT_W: u32 = 300;
const ALERT_H: u32 = 150;
const ALERT_BTN_W: u32 = 80;
const ALERT_BTN_H: u32 = 30;

/// Load a file from disk into a Vec<u8>.
fn read_file(path: &str) -> Option<Vec<u8>> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut content = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        content.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    Some(content)
}

/// Load and decode a PNG image, returning (pixels, width, height).
fn load_png(path: &str) -> Option<(Vec<u32>, u32, u32)> {
    let data = read_file(path)?;
    let info = libimage_client::probe(&data)?;
    let w = info.width;
    let h = info.height;
    let mut pixels = vec![0u32; (w * h) as usize];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    libimage_client::decode(&data, &mut pixels, &mut scratch).ok()?;
    Some((pixels, w, h))
}

/// Show a modal error alert and wait for the user to dismiss it.
fn show_error_alert(_parent_win: u32) {
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
    let alert_id = window::create_ex("Error", ax as u16, ay as u16,
        ALERT_W as u16, ALERT_H as u16, flags);
    if alert_id == u32::MAX {
        return;
    }

    let btn_x = (ALERT_W as i32 - ALERT_BTN_W as i32) / 2;
    let btn_y = (ALERT_H as i32 - ALERT_BTN_H as i32 - 16) as i32;
    let mut ok_btn = UiButton::new(btn_x, btn_y, ALERT_BTN_W, ALERT_BTN_H, ButtonStyle::Primary);

    let mut dirty = true;
    let mut event = [0u32; 5];
    loop {
        if dirty {
            let bg = colors::WINDOW_BG();
            window::fill_rect(alert_id, 0, 0, ALERT_W as u16, ALERT_H as u16, bg);

            alert(alert_id, 0, 0, ALERT_W, ALERT_H - 50, "Login Failed",
                "Invalid username or password.\nPlease try again.");

            ok_btn.render(alert_id, "OK");

            window::present(alert_id);
            dirty = false;
        }

        if window::get_event(alert_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);
            dirty = true;

            if ev.is_key_down() && (ev.key_code() == KEY_ENTER || ev.key_code() == KEY_ESCAPE) {
                break;
            }

            if ok_btn.handle_event(&ev) {
                break;
            }
        }

        process::sleep(16);
    }

    window::destroy(alert_id);
}

/// Pre-scaled logo: (pixels, display_w, display_h).
fn load_and_scale_logo(target_h: u32) -> Option<(Vec<u32>, u32, u32)> {
    let (pixels, img_w, img_h) = load_png("/System/media/anyos_w.png")?;
    if img_h == 0 { return None; }
    let display_w = (img_w as u64 * target_h as u64 / img_h as u64) as u32;
    if display_w == 0 { return None; }
    let mut scaled = vec![0u32; (display_w * target_h) as usize];
    if libimage_client::scale_image(&pixels, img_w, img_h, &mut scaled, display_w, target_h, libimage_client::MODE_SCALE) {
        Some((scaled, display_w, target_h))
    } else {
        Some((pixels, img_w, img_h))
    }
}

fn main() -> u32 {
    // Load and pre-scale logo to 48px height
    let logo = load_and_scale_logo(48);

    // Get screen size to center the window
    let (sw, sh) = window::screen_size();
    let wx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let wy = ((sh as i32 - DIALOG_H as i32) / 2).max(0);

    let flags = window::WIN_FLAG_BORDERLESS
        | window::WIN_FLAG_SHADOW
        | window::WIN_FLAG_NOT_RESIZABLE
        | window::WIN_FLAG_NO_CLOSE
        | window::WIN_FLAG_NO_MINIMIZE
        | window::WIN_FLAG_NO_MAXIMIZE;
    let win_id = window::create_ex("Login", wx as u16, wy as u16,
        DIALOG_W as u16, DIALOG_H as u16, flags);
    if win_id == u32::MAX {
        return 0;
    }

    // Measure welcome text for manual centering
    let welcome_text = "Welcome to .anyOS";
    let (welcome_tw, _) = label_measure(welcome_text, FontSize::Title);

    // Layout: compute Y positions
    let mut y_cursor: i32 = 30;

    // Logo
    let logo_display_h: u32 = if logo.is_some() { 48 } else { 0 };
    let logo_y = y_cursor;
    if logo.is_some() {
        y_cursor += logo_display_h as i32 + 40; // 40px gap after logo
    }

    // "Welcome to .anyOS" centered
    let welcome_x = ((DIALOG_W as i32 - welcome_tw as i32) / 2).max(0);
    let welcome_y = y_cursor;
    y_cursor += 46; // title height + gap

    // Username label + field
    let user_label_y = y_cursor;
    y_cursor += 16;
    let user_field_y = y_cursor;
    y_cursor += FIELD_H as i32 + 12;

    // Password label + field
    let pass_label_y = y_cursor;
    y_cursor += 16;
    let pass_field_y = y_cursor;
    y_cursor += FIELD_H as i32 + 16;

    // Login button (full width)
    let btn_y = y_cursor;

    let field_x = PAD;
    let mut username_field = UiTextField::new(field_x, user_field_y, FIELD_W, FIELD_H);
    username_field.focused = true;

    let mut password_field = UiTextField::new(field_x, pass_field_y, FIELD_W, FIELD_H);
    password_field.password = true;

    let mut login_btn = UiButton::new(PAD, btn_y, BTN_W, BTN_H, ButtonStyle::Primary);

    let mut focused_field: u8 = 0; // 0=username, 1=password
    let mut dirty = true;

    let mut event = [0u32; 5];

    loop {
        // ── Render (only when dirty) ──
        if dirty {
            let bg = colors::WINDOW_BG();
            window::fill_rect(win_id, 0, 0, DIALOG_W as u16, DIALOG_H as u16, bg);

            // Logo (centered, alpha-blended)
            if let Some((ref pixels, disp_w, disp_h)) = logo {
                let logo_x = ((DIALOG_W as i32 - disp_w as i32) / 2) as i16;
                window::blit_alpha(win_id, logo_x, logo_y as i16, disp_w as u16, disp_h as u16, pixels);
            }

            // Welcome text (manually centered)
            label(win_id, welcome_x, welcome_y, welcome_text, colors::TEXT(), FontSize::Title, TextAlign::Left);

            // Username label + field
            label(win_id, field_x, user_label_y, "Username", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
            username_field.render(win_id, "Enter username");

            // Password label + field
            label(win_id, field_x, pass_label_y, "Password", colors::TEXT_SECONDARY(), FontSize::Small, TextAlign::Left);
            password_field.render(win_id, "Enter password");

            // Login button (disabled if username is empty)
            let username_empty = username_field.text().is_empty();
            if username_empty {
                button(win_id, login_btn.x, login_btn.y, login_btn.w, login_btn.h,
                    "Log In", ButtonStyle::Primary, ButtonState::Disabled);
            } else {
                login_btn.render(win_id, "Log In");
            }

            window::present(win_id);
            dirty = false;
        }

        // ── Events ──
        if window::get_event(win_id, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);
            dirty = true; // Any event triggers a re-render

            if event[0] == EVENT_WINDOW_CLOSE {
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
                if !username_field.text().is_empty() {
                    let uid = try_login(&username_field, &password_field);
                    if uid != u32::MAX {
                        window::destroy(win_id);
                        return uid;
                    } else {
                        show_error_alert(win_id);
                        dirty = true;
                    }
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

            // Button click (only if username is not empty)
            if !username_field.text().is_empty() {
                if login_btn.handle_event(&ev) {
                    let uid = try_login(&username_field, &password_field);
                    if uid != u32::MAX {
                        window::destroy(win_id);
                        return uid;
                    } else {
                        show_error_alert(win_id);
                        dirty = true;
                    }
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
        process::getuid() as u32
    } else {
        u32::MAX
    }
}
