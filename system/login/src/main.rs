#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{process, fs, users};
use libanyui_client as ui;
use ui::Widget;

const LAST_USER_PATH: &str = "/System/etc/last_user";

mod assets;

const DIALOG_W: u32 = 340;
const DIALOG_H: u32 = 340;
const FIELD_W: u32 = 280;
const FIELD_H: u32 = 28;
const BTN_W: u32 = 280;
const BTN_H: u32 = 34;
const PAD: i32 = 30;

/// Set by the login callback on success; read after ui::run() returns.
static mut LOGIN_UID: u32 = u32::MAX;

fn main() -> u32 {
    if !ui::init() {
        return 0;
    }

    let (sw, sh) = ui::screen_size();
    let wx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let wy = ((sh as i32 - DIALOG_H as i32) / 2).max(0);

    let flags = ui::WIN_FLAG_BORDERLESS
        | ui::WIN_FLAG_SHADOW
        | ui::WIN_FLAG_NOT_RESIZABLE
        | ui::WIN_FLAG_NO_CLOSE
        | ui::WIN_FLAG_NO_MINIMIZE
        | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags("Login", wx, wy, DIALOG_W, DIALOG_H, flags);

    // Semi-transparent background for blur-behind effect
    win.set_color(0xD8F0F0F0);
    ui::set_blur_behind(&win, 8);

    // ── Logo ──
    let mut y_cursor: i32 = 30;
    if let Some((pixels, dw, dh)) = assets::load_and_scale_logo(48) {
        let logo = ui::ImageView::new(dw, dh);
        logo.set_pixels(&pixels, dw, dh);
        logo.set_position(((DIALOG_W as i32 - dw as i32) / 2).max(0), y_cursor);
        win.add(&logo);
        y_cursor += dh as i32 + 40;
    }

    // ── Welcome label ──
    let welcome = ui::Label::new("Welcome to .anyOS");
    welcome.set_font_size(20);
    welcome.set_position(PAD, y_cursor);
    welcome.set_size(FIELD_W, 30);
    welcome.set_text_align(ui::TEXT_ALIGN_CENTER);
    win.add(&welcome);
    y_cursor += 46;

    // ── Username ──
    let user_lbl = ui::Label::new("Username");
    user_lbl.set_font_size(11);
    user_lbl.set_position(PAD, y_cursor);
    win.add(&user_lbl);
    y_cursor += 16;

    let user_field = ui::TextField::new();
    user_field.set_placeholder("Enter username");
    user_field.set_position(PAD, y_cursor);
    user_field.set_size(FIELD_W, FIELD_H);
    win.add(&user_field);
    y_cursor += FIELD_H as i32 + 12;

    // ── Password ──
    let pass_lbl = ui::Label::new("Password");
    pass_lbl.set_font_size(11);
    pass_lbl.set_position(PAD, y_cursor);
    win.add(&pass_lbl);
    y_cursor += 16;

    let pass_field = ui::TextField::new();
    pass_field.set_password_mode(true);
    pass_field.set_placeholder("Enter password");
    pass_field.set_position(PAD, y_cursor);
    pass_field.set_size(FIELD_W, FIELD_H);
    win.add(&pass_field);
    y_cursor += FIELD_H as i32 + 16;

    // ── Login button ──
    let login_btn = ui::Button::new("Log In");
    login_btn.set_position(PAD, y_cursor);
    login_btn.set_size(BTN_W, BTN_H);
    login_btn.set_state(0); // disabled initially (username empty)
    win.add(&login_btn);

    // Pre-fill last username if available
    let mut has_last_user = false;
    {
        let fd = fs::open(LAST_USER_PATH, 0);
        if fd != u32::MAX {
            let mut buf = [0u8; 128];
            let n = fs::read(fd, &mut buf);
            fs::close(fd);
            if n > 0 {
                if let Ok(name) = core::str::from_utf8(&buf[..n as usize]) {
                    let name = name.trim();
                    if !name.is_empty() {
                        user_field.set_text(name);
                        login_btn.set_state(1); // enable button
                        has_last_user = true;
                    }
                }
            }
        }
    }

    // Fallback: if exactly one non-root user exists, suggest them.
    // listusers() returns "uid:username:fullname\n" per user.
    if !has_last_user {
        let mut ubuf = [0u8; 512];
        let n = users::listusers(&mut ubuf);
        if n > 0 {
            if let Ok(list) = core::str::from_utf8(&ubuf[..n as usize]) {
                let mut sole_name = [0u8; 32];
                let mut sole_len = 0usize;
                let mut count = 0u32;
                for line in list.split('\n') {
                    // Parse "uid:username:fullname"
                    let mut parts = line.splitn(3, ':');
                    let uid_str = match parts.next() { Some(s) => s, None => continue };
                    let name = match parts.next() { Some(s) => s, None => continue };
                    if uid_str != "0" && !name.is_empty() {
                        count += 1;
                        if count == 1 {
                            let cpy = name.len().min(32);
                            sole_name[..cpy].copy_from_slice(&name.as_bytes()[..cpy]);
                            sole_len = cpy;
                        }
                    }
                }
                if count == 1 && sole_len > 0 {
                    if let Ok(name) = core::str::from_utf8(&sole_name[..sole_len]) {
                        user_field.set_text(name);
                        login_btn.set_state(1);
                        has_last_user = true;
                    }
                }
            }
        }
    }

    // Focus password field if username is pre-filled, otherwise username field
    if has_last_user {
        pass_field.focus();
    } else {
        user_field.focus();
    }

    // ── Callbacks ──

    // Enable/disable button when username text changes
    let btn_id = login_btn.id();
    let uf_id = user_field.id();
    user_field.on_text_changed(move |_| {
        let mut buf = [0u8; 128];
        let len = ui::Control::from_id(uf_id).get_text(&mut buf);
        ui::Control::from_id(btn_id).set_state(if len > 0 { 1 } else { 0 });
    });

    // Button click → attempt login
    let uf_id = user_field.id();
    let pf_id = pass_field.id();
    login_btn.on_click(move |_| {
        if attempt_login(uf_id, pf_id) {
            ui::quit();
        }
    });

        // Enter on password field → attempt login
    let uf_id2 = user_field.id();
    let pf_id3 = pass_field.id();

    // Enter on username field → move focus to password
    let pf_id2 = pass_field.id();
    user_field.on_submit(move |_| {
        if attempt_login(uf_id2, pf_id3) {
            ui::quit();
        } else {
            ui::Control::from_id(pf_id2).focus();
        }
    });


    pass_field.on_submit(move |_| {
        if attempt_login(uf_id2, pf_id3) {
            ui::quit();
        }
    });

    // Blocks until quit() is called
    ui::run();

    unsafe { LOGIN_UID }
}

fn attempt_login(uf_id: u32, pf_id: u32) -> bool {
    let mut ubuf = [0u8; 128];
    let ulen = ui::Control::from_id(uf_id).get_text(&mut ubuf);
    if ulen == 0 {
        return false;
    }
    let username = core::str::from_utf8(&ubuf[..ulen as usize]).unwrap_or("");

    let mut pbuf = [0u8; 128];
    let plen = ui::Control::from_id(pf_id).get_text(&mut pbuf);
    let password = core::str::from_utf8(&pbuf[..plen as usize]).unwrap_or("");

    if process::authenticate(username, password) {
        unsafe { LOGIN_UID = process::getuid() as u32; }
        // Remember last username for next login
        let _ = fs::write_bytes(LAST_USER_PATH, username.as_bytes());
        true
    } else {
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "Invalid username or password.\nPlease try again.",
            Some("OK"),
        );
        false
    }
}
