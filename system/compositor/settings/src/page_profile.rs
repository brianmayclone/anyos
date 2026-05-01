//! User Profile settings page.
//!
//! Lets the user manage their avatar (image picked via FileDialog, path
//! persisted in confd under `system/profiles/avatars/<username>`) and change
//! their password. The avatar preview mirrors the login screen rendering:
//! a circular badge with the user's initials on a hashed palette colour, or
//! the chosen image cropped to a circle.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use anyos_std::{env, fs, i18n, println, process, users};
use libanyui_client as ui;
use libconf_schema::{manifest, RegistryManifest, RegistryScope, ServiceSchema};
use ui::Widget;

use crate::layout;

// ── Confd schema for profiles (shared with login) ───────────────────────────
//
// User scope so the user owns and can write their own profile data. The login
// screen runs as root and reads each user's User-scope entries directly via
// ConfClient::get_for_user(), avoiding the System-scope write-permission gate.

const PROFILES_MANIFEST: RegistryManifest<'static> =
    manifest("profiles", RegistryScope::User, 1, &[], &[], &[]);

fn profiles_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("settings", &PROFILES_MANIFEST)
}

pub fn read_avatar_path() -> Option<String> {
    let s = profiles_schema().read_string("avatar")?;
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ── Static state for the page (one preview canvas) ──────────────────────────

const AVATAR_SIZE: u32 = 96;
static mut AVATAR_CANVAS: Option<ui::Canvas> = None;
static mut CURRENT_USERNAME: Option<String> = None;
static mut CURRENT_DISPLAYNAME: Option<String> = None;

// ── Public entry point ──────────────────────────────────────────────────────

pub fn build(parent: &ui::ScrollView) -> u32 {
    let _ = profiles_schema().register();

    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(
        &panel,
        i18n::t("Profile"),
        i18n::t("Avatar, account info and password"),
    );

    let (uid, username, displayname, home) = resolve_identity();
    unsafe {
        CURRENT_USERNAME = Some(username.clone());
        CURRENT_DISPLAYNAME = Some(displayname.clone());
    }

    build_avatar_card(&panel);
    build_info_card(&panel, uid, &username, &displayname, &home);
    build_password_card(&panel);

    parent.add(&panel);
    panel.id()
}

// ── Avatar card ─────────────────────────────────────────────────────────────

fn build_avatar_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 124);
    row.set_margin(24, 12, 24, 12);

    // Avatar preview
    let canvas = ui::Canvas::new(AVATAR_SIZE, AVATAR_SIZE);
    canvas.set_position(0, 12);
    canvas.set_size(AVATAR_SIZE, AVATAR_SIZE);
    row.add(&canvas);
    unsafe {
        AVATAR_CANVAS = Some(canvas);
    }
    refresh_avatar();

    // Description
    let title = ui::Label::new(i18n::t("Avatar"));
    title.set_position(120, 12);
    title.set_size(300, 22);
    title.set_font_size(15);
    title.set_text_color(layout::text());
    row.add(&title);

    let hint = ui::Label::new(i18n::t("Shown on the login screen and the deskbar."));
    hint.set_position(120, 36);
    hint.set_size(420, 20);
    hint.set_font_size(12);
    hint.set_text_color(layout::text_dim());
    row.add(&hint);

    // Buttons
    let choose = ui::Button::new(i18n::t("Choose Image..."));
    choose.set_position(120, 70);
    choose.set_size(150, 28);
    choose.on_click(|_| pick_avatar());
    row.add(&choose);

    let reset = ui::Button::new(i18n::t("Reset"));
    reset.set_position(280, 70);
    reset.set_size(90, 28);
    reset.on_click(|_| clear_avatar());
    row.add(&reset);

    card.add(&row);
}

fn pick_avatar() {
    let path = match ui::FileDialog::open_file() {
        Some(p) => p,
        None => return,
    };
    if !is_supported_image(&path) {
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            i18n::t("Please pick a PNG, JPEG or BMP image."),
            Some(i18n::t("OK")),
        );
        return;
    }
    match profiles_schema().write_string("avatar", &path) {
        Ok(_) => println!("settings/profile: avatar set to '{}'", path),
        Err(e) => {
            println!("settings/profile: avatar write FAILED: {:?}", e);
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                i18n::t("Could not save avatar setting."),
                Some(i18n::t("OK")),
            );
            return;
        }
    }
    refresh_avatar();
}

fn clear_avatar() {
    match profiles_schema().write_string("avatar", "") {
        Ok(_) => println!("settings/profile: avatar cleared"),
        Err(e) => println!("settings/profile: avatar clear FAILED: {:?}", e),
    }
    refresh_avatar();
}

fn is_supported_image(path: &str) -> bool {
    let lower: String = path.chars().map(|c| c.to_ascii_lowercase()).collect();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".bmp")
}

// ── Info card ───────────────────────────────────────────────────────────────

fn build_info_card(panel: &ui::View, uid: u16, username: &str, displayname: &str, home: &str) {
    let card = layout::build_auto_card(panel);
    layout::build_info_row(&card, i18n::t("Display Name"), displayname, true);
    layout::build_separator(&card);
    layout::build_info_row(&card, i18n::t("Username"), username, false);
    layout::build_separator(&card);
    layout::build_info_row(&card, i18n::t("UID"), &format!("{}", uid), false);
    layout::build_separator(&card);
    layout::build_info_row(&card, i18n::t("Home"), home, false);
}

// ── Password card ───────────────────────────────────────────────────────────

fn build_password_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);
    let row = layout::build_setting_row(&card, i18n::t("Password"), true);
    let val = ui::Label::new("••••••••");
    val.set_position(200, 12);
    val.set_size(200, 20);
    val.set_text_color(layout::text_dim());
    val.set_font_size(13);
    row.add(&val);

    let btn = ui::Button::new(i18n::t("Change..."));
    btn.set_position(420, 8);
    btn.set_size(110, 28);
    btn.on_click(|_| open_password_dialog());
    row.add(&btn);
}

fn open_password_dialog() {
    let win = ui::Window::new(i18n::t("Change Password"), -1, -1, 380, 240);

    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(layout::bg());

    let pad: i32 = 20;
    let field_w: u32 = 340;
    let field_h: u32 = 28;
    let mut y: i32 = 16;

    let mk_label = |text: &str, top: i32| {
        let l = ui::Label::new(text);
        l.set_position(pad, top);
        l.set_size(field_w, 18);
        l.set_font_size(12);
        l.set_text_color(layout::text());
        l
    };

    let l_old = mk_label(i18n::t("Current Password"), y);
    root.add(&l_old);
    y += 20;
    let f_old = ui::TextField::new();
    f_old.set_password_mode(true);
    f_old.set_position(pad, y);
    f_old.set_size(field_w, field_h);
    root.add(&f_old);
    y += field_h as i32 + 10;

    let l_new = mk_label(i18n::t("New Password"), y);
    root.add(&l_new);
    y += 20;
    let f_new = ui::TextField::new();
    f_new.set_password_mode(true);
    f_new.set_position(pad, y);
    f_new.set_size(field_w, field_h);
    root.add(&f_new);
    y += field_h as i32 + 10;

    let l_confirm = mk_label(i18n::t("Confirm New Password"), y);
    root.add(&l_confirm);
    y += 20;
    let f_confirm = ui::TextField::new();
    f_confirm.set_password_mode(true);
    f_confirm.set_position(pad, y);
    f_confirm.set_size(field_w, field_h);
    root.add(&f_confirm);

    let cancel = ui::Button::new(i18n::t("Cancel"));
    cancel.set_position(pad, 200);
    cancel.set_size(90, 28);
    let win_id_c = win.id();
    cancel.on_click(move |_| {
        ui::Control::from_id(win_id_c).remove();
    });
    root.add(&cancel);

    let ok = ui::Button::new(i18n::t("Change"));
    ok.set_position(380 - pad - 110, 200);
    ok.set_size(110, 28);
    let old_id = f_old.id();
    let new_id = f_new.id();
    confirm_id_setup(&ok, old_id, new_id, f_confirm.id(), win.id());
    root.add(&ok);

    win.add(&root);
    f_old.focus();
}

fn confirm_id_setup(ok: &ui::Button, old_id: u32, new_id: u32, confirm_id: u32, win_id: u32) {
    ok.on_click(move |_| {
        let mut buf = [0u8; 128];
        let len = ui::Control::from_id(old_id).get_text(&mut buf);
        let old_pw = core::str::from_utf8(&buf[..len as usize])
            .unwrap_or("")
            .to_string();

        let mut buf2 = [0u8; 128];
        let len2 = ui::Control::from_id(new_id).get_text(&mut buf2);
        let new_pw = core::str::from_utf8(&buf2[..len2 as usize])
            .unwrap_or("")
            .to_string();

        let mut buf3 = [0u8; 128];
        let len3 = ui::Control::from_id(confirm_id).get_text(&mut buf3);
        let confirm_pw = core::str::from_utf8(&buf3[..len3 as usize])
            .unwrap_or("")
            .to_string();

        if new_pw.is_empty() {
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                i18n::t("New password cannot be empty."),
                Some(i18n::t("OK")),
            );
            return;
        }
        if new_pw != confirm_pw {
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                i18n::t("New password and confirmation do not match."),
                Some(i18n::t("OK")),
            );
            return;
        }

        let username = unsafe { CURRENT_USERNAME.clone() }.unwrap_or_default();
        let rc = users::chpasswd(&username, &old_pw, &new_pw);
        if rc == 0 {
            ui::MessageBox::show(
                ui::MessageBoxType::Info,
                i18n::t("Password changed."),
                Some(i18n::t("OK")),
            );
            ui::Control::from_id(win_id).remove();
        } else {
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                i18n::t("Could not change password.\nCheck the current password and try again."),
                Some(i18n::t("OK")),
            );
        }
    });
}

// ── Avatar rendering ────────────────────────────────────────────────────────

const AVATAR_PALETTE: [u32; 8] = [
    0xFF4E79A7, 0xFFF28E2B, 0xFFE15759, 0xFF76B7B2, 0xFF59A14F, 0xFFB07AA1, 0xFF9C755F, 0xFF5470C6,
];

fn avatar_color(seed: &str) -> u32 {
    let mut h: u32 = 0x811C9DC5;
    for b in seed.bytes() {
        h = h.wrapping_mul(0x01000193) ^ b as u32;
    }
    AVATAR_PALETTE[(h as usize) % AVATAR_PALETTE.len()]
}

fn compute_initials(display: &str, fallback: &str) -> String {
    let src = if display.trim().is_empty() {
        fallback
    } else {
        display
    };
    let mut out = String::new();
    let mut prev_space = true;
    for c in src.chars() {
        if c.is_whitespace() {
            prev_space = true;
            continue;
        }
        if prev_space {
            for u in c.to_uppercase() {
                out.push(u);
            }
            if out.chars().count() >= 2 {
                break;
            }
            prev_space = false;
        }
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

fn refresh_avatar() {
    let canvas = match unsafe { AVATAR_CANVAS } {
        Some(c) => c,
        None => return,
    };
    canvas.clear(layout::card_bg());

    let username = unsafe { CURRENT_USERNAME.clone() }.unwrap_or_default();
    let displayname = unsafe { CURRENT_DISPLAYNAME.clone() }.unwrap_or_default();

    // Try the user-chosen image first.
    match read_avatar_path() {
        Some(path) => match load_image(&path) {
            Some((pixels, w, h)) => {
                println!(
                    "settings/profile: rendering avatar from '{}' ({}x{})",
                    path, w, h
                );
                blit_circular_buffer(&canvas, &pixels, w, h, layout::card_bg());
                return;
            }
            None => {
                println!(
                    "settings/profile: avatar path '{}' could not be decoded",
                    path
                );
            }
        },
        None => {}
    }

    // Fallback: initials on a hashed colour.
    let half = (AVATAR_SIZE as i32) / 2;
    canvas.fill_circle(half, half, half - 1, avatar_color(&username));

    let initials = compute_initials(&displayname, &username);
    let font_size: u16 = 40;
    let glyph_w = (font_size as i32) * 6 / 10;
    let text_w = glyph_w * (initials.chars().count() as i32);
    canvas.draw_text(
        half - text_w / 2,
        half - (font_size as i32) / 2 - 2,
        0xFFFFFFFF,
        1,
        font_size,
        &initials,
    );
}

fn load_image(path: &str) -> Option<(Vec<u32>, u32, u32)> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        println!("settings/profile: load_image: fs::open('{}') failed", path);
        return None;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    println!(
        "settings/profile: load_image: read {} bytes from '{}'",
        data.len(),
        path
    );
    if data.len() >= 16 {
        let h = &data[..16];
        println!(
            "settings/profile: load_image: first 16 bytes = {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
            h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7],
            h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]
        );
    }
    let info = match libimage_client::probe(&data) {
        Some(i) => i,
        None => {
            println!("settings/profile: load_image: probe failed (unrecognized format)");
            return None;
        }
    };
    println!(
        "settings/profile: load_image: probe ok format={} {}x{} scratch={}",
        info.format, info.width, info.height, info.scratch_needed
    );
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 {
        return None;
    }
    let mut pixels = vec![0u32; (w as usize).saturating_mul(h as usize)];
    let mut scratch = vec![0u8; info.scratch_needed as usize];
    if let Err(e) = libimage_client::decode(&data, &mut pixels, &mut scratch) {
        println!("settings/profile: load_image: decode failed: {:?}", e);
        return None;
    }
    // Center-crop to square then scale to AVATAR_SIZE.
    let side = w.min(h);
    let ox = (w - side) / 2;
    let oy = (h - side) / 2;
    let mut cropped = vec![0u32; (side as usize) * (side as usize)];
    for y in 0..side {
        let src_off = ((oy + y) * w + ox) as usize;
        let dst_off = (y * side) as usize;
        cropped[dst_off..dst_off + side as usize]
            .copy_from_slice(&pixels[src_off..src_off + side as usize]);
    }
    let mut scaled = vec![0u32; (AVATAR_SIZE as usize) * (AVATAR_SIZE as usize)];
    if libimage_client::scale_image(
        &cropped,
        side,
        side,
        &mut scaled,
        AVATAR_SIZE,
        AVATAR_SIZE,
        libimage_client::MODE_SCALE,
    ) {
        Some((scaled, AVATAR_SIZE, AVATAR_SIZE))
    } else {
        Some((cropped, side, side))
    }
}

fn blit_circular_buffer(canvas: &ui::Canvas, pixels: &[u32], w: u32, h: u32, bg: u32) {
    let size = AVATAR_SIZE as usize;
    let mut buf = vec![bg; size * size];
    let cx = (AVATAR_SIZE as i32) / 2;
    let cy = (AVATAR_SIZE as i32) / 2;
    let r = cx - 1;
    let r2 = r * r;
    let lim_w = w.min(AVATAR_SIZE);
    let lim_h = h.min(AVATAR_SIZE);
    for y in 0..lim_h {
        for x in 0..lim_w {
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            if dx * dx + dy * dy <= r2 {
                let p = pixels[(y * w + x) as usize];
                buf[(y as usize) * size + x as usize] = p | 0xFF000000;
            }
        }
    }
    canvas.copy_pixels_from(&buf);
}

// ── User identity resolution (uid, username, displayname, home) ─────────────

fn resolve_identity() -> (u16, String, String, String) {
    let uid = process::getuid();

    let mut name_buf = [0u8; 64];
    let nlen = process::getusername(uid, &mut name_buf);
    let username = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize])
            .unwrap_or("root")
            .to_string()
    } else {
        String::from("root")
    };

    // Look up the fullname via listusers().
    let mut displayname = username.clone();
    let mut ubuf = [0u8; 1024];
    let n = users::listusers(&mut ubuf);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&ubuf[..n as usize]) {
            for line in s.split('\n') {
                let mut parts = line.splitn(3, ':');
                let _uid_str = parts.next().unwrap_or("");
                let name = parts.next().unwrap_or("");
                let full = parts.next().unwrap_or("");
                if name == username && !full.is_empty() {
                    displayname = full.to_string();
                    break;
                }
            }
        }
    }

    let mut home_buf = [0u8; 256];
    let hlen = env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&home_buf[..hlen as usize])
            .unwrap_or("/tmp")
            .to_string()
    } else {
        format!("/Users/{}", username)
    };

    (uid, username, displayname, home)
}
