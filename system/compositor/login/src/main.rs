#![no_std]
#![no_main]

anyos_std::entry!(main);

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{i18n, println, process, users};
use libanyui_client as ui;
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};
use ui::{IconType, Widget};

const LOGIN_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_string("state/last_user", "")];
const LOGIN_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const LOGIN_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "system/login",
    RegistryScope::System,
    1,
    &["state"],
    LOGIN_DEFAULTS,
    LOGIN_MIGRATIONS,
);

fn login_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("login", &LOGIN_MANIFEST)
}

const DIALOG_W: u32 = 380;
const DIALOG_H: u32 = 470;
const AVATAR_SIZE: u32 = 96;
const PASS_W: u32 = 240;
const PASS_H: u32 = 32;
const SWITCH_BTN: u32 = 28;
const POWER_ICON: u32 = 44;
const POWER_LBL_W: u32 = 96;

/// Set by the login callback on success; read after ui::run() returns.
static mut LOGIN_UID: u32 = u32::MAX;

/// Available users (uid, username, fullname). Includes root.
static mut USERS: Option<Vec<(u32, String, String)>> = None;
/// Index of the currently selected user in USERS.
static mut CURRENT_USER: usize = 0;
/// Avatar canvas (updated when user switches). Canvas is Copy.
static mut AVATAR: Option<ui::Canvas> = None;
/// Username label control (updated when user switches).
static mut NAME_LBL_ID: u32 = 0;
/// Password field control.
static mut PASS_ID: u32 = 0;

fn user_list() -> &'static mut Vec<(u32, String, String)> {
    unsafe { USERS.as_mut().unwrap() }
}

fn main() -> u32 {
    match login_schema().register() {
        Ok(_) => println!("login: schema register OK"),
        Err(err) => println!("login: schema register FAILED: {:?}", err),
    }
    if !ui::init() {
        return u32::MAX;
    }
    i18n::init();

    // ── Load users (uid:username:fullname per line) ─────────────────────
    let mut list: Vec<(u32, String, String)> = Vec::new();
    let mut ubuf = [0u8; 1024];
    let n = users::listusers(&mut ubuf);
    if n > 0 {
        if let Ok(s) = core::str::from_utf8(&ubuf[..n as usize]) {
            for line in s.split('\n') {
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(3, ':');
                let uid_str = parts.next().unwrap_or("");
                let name = parts.next().unwrap_or("");
                let full = parts.next().unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let uid: u32 = uid_str.parse().unwrap_or(0);
                let display = if full.is_empty() { name } else { full };
                list.push((uid, name.to_string(), display.to_string()));
            }
        }
    }
    if list.is_empty() {
        // Fallback so we can still try to authenticate manually if needed.
        list.push((1000, "user".to_string(), "User".to_string()));
    }

    // Pick last_user from confd; fall back to first entry.
    let mut current: usize = 0;
    if let Some(name) = login_schema().read_string("state/last_user") {
        let name = name.trim();
        if !name.is_empty() {
            if let Some(idx) = list.iter().position(|(_, u, _)| u == name) {
                current = idx;
            }
        }
    }

    unsafe {
        USERS = Some(list);
        CURRENT_USER = current;
    }

    // ── Window ──────────────────────────────────────────────────────────
    let (sw, sh) = ui::screen_size();
    let wx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let wy = ((sh as i32 - DIALOG_H as i32) / 2).max(0);

    let flags = ui::WIN_FLAG_BORDERLESS
        | ui::WIN_FLAG_SHADOW
        | ui::WIN_FLAG_NOT_RESIZABLE
        | ui::WIN_FLAG_NO_CLOSE
        | ui::WIN_FLAG_NO_MINIMIZE
        | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags(i18n::t("Login"), wx, wy, DIALOG_W, DIALOG_H, flags);
    let tc = ui::theme::colors();
    win.set_color(tc.window_bg);

    // ── Avatar (centered, circular) ─────────────────────────────────────
    let avatar_x = (DIALOG_W as i32 - AVATAR_SIZE as i32) / 2;
    let avatar_y: i32 = 36;
    let avatar = ui::Canvas::new(AVATAR_SIZE, AVATAR_SIZE);
    avatar.set_position(avatar_x, avatar_y);
    avatar.set_size(AVATAR_SIZE, AVATAR_SIZE);
    win.add(&avatar);
    unsafe { AVATAR = Some(avatar); }
    update_avatar();

    // ── User name row: [<] DisplayName ──────────────────────────────────
    let name_y = avatar_y + AVATAR_SIZE as i32 + 14;
    let name_lbl = ui::Label::new("");
    name_lbl.set_font_size(20);
    name_lbl.set_position(0, name_y);
    name_lbl.set_size(DIALOG_W, 28);
    name_lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
    win.add(&name_lbl);
    unsafe { NAME_LBL_ID = name_lbl.id(); }
    update_name_label();

    let switch_btn = ui::IconButton::new("");
    switch_btn.set_system_icon("chevron-left", IconType::Outline, tc.text, 18);
    switch_btn.set_flat_style(true);
    let switch_x: i32 = 16;
    let switch_y = name_y + (28 - SWITCH_BTN as i32) / 2;
    switch_btn.set_position(switch_x, switch_y);
    switch_btn.set_size(SWITCH_BTN, SWITCH_BTN);
    if user_list().len() <= 1 {
        switch_btn.set_visible(false);
    }
    win.add(&switch_btn);

    // ── Password field (centered, no switcher) ──────────────────────────
    let pass_y = name_y + 46;
    let pass_x = (DIALOG_W as i32 - PASS_W as i32) / 2;
    let pass_field = ui::TextField::new();
    pass_field.set_password_mode(true);
    pass_field.set_placeholder(i18n::t("Enter Password"));
    pass_field.set_position(pass_x, pass_y);
    pass_field.set_size(PASS_W, PASS_H);
    win.add(&pass_field);
    unsafe { PASS_ID = pass_field.id(); }
    pass_field.focus();

    // ── Bottom row: Sleep / Restart / Shut Down ─────────────────────────
    let bottom_y = (DIALOG_H as i32) - 110;
    let icon_size: u32 = 28;
    let triple_w: u32 = POWER_LBL_W * 3;
    let triple_x = (DIALOG_W as i32 - triple_w as i32) / 2;

    let entries: [(&str, &str, fn()); 3] = [
        ("moon", "Sleep", || {
            println!("login: sleep requested (no-op — not supported)");
        }),
        ("rotate-clockwise", "Restart", || { process::reboot(); }),
        ("power", "Shut Down", || { process::shutdown(); }),
    ];
    for (i, (icon, label, action)) in entries.iter().enumerate() {
        let cell_x = triple_x + (i as i32) * POWER_LBL_W as i32;
        let icon_x = cell_x + (POWER_LBL_W as i32 - POWER_ICON as i32) / 2;
        let btn = ui::IconButton::new("");
        btn.set_system_icon(icon, IconType::Outline, tc.text, icon_size);
        btn.set_flat_style(true);
        btn.set_position(icon_x, bottom_y);
        btn.set_size(POWER_ICON, POWER_ICON);
        let action = *action;
        btn.on_click(move |_| action());
        win.add(&btn);

        let lbl = ui::Label::new(i18n::t(label));
        lbl.set_font_size(11);
        lbl.set_position(cell_x, bottom_y + POWER_ICON as i32 + 6);
        lbl.set_size(POWER_LBL_W, 16);
        lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
        win.add(&lbl);
    }

    // Switch user: cycle to next entry.
    switch_btn.on_click(|_| {
        let users = user_list();
        if users.len() <= 1 {
            return;
        }
        unsafe {
            CURRENT_USER = (CURRENT_USER + 1) % users.len();
        }
        update_avatar();
        update_name_label();
        // Clear password field on switch.
        let pf_id = unsafe { PASS_ID };
        ui::Control::from_id(pf_id).set_text("");
        ui::Control::from_id(pf_id).focus();
    });

    let pf_id = pass_field.id();
    pass_field.on_submit(move |_| {
        if attempt_login(pf_id) {
            ui::quit();
        }
    });

    ui::run();

    unsafe { LOGIN_UID }
}

fn current_username() -> &'static str {
    let users = user_list();
    let idx = unsafe { CURRENT_USER };
    users.get(idx).map(|(_, u, _)| u.as_str()).unwrap_or("")
}

fn current_displayname() -> &'static str {
    let users = user_list();
    let idx = unsafe { CURRENT_USER };
    users.get(idx).map(|(_, _, d)| d.as_str()).unwrap_or("")
}

fn update_name_label() {
    let id = unsafe { NAME_LBL_ID };
    if id == 0 {
        return;
    }
    ui::Control::from_id(id).set_text(current_displayname());
}

/// Pleasant palette for initials backgrounds (avoids reds/yellows for readability).
const AVATAR_PALETTE: [u32; 8] = [
    0xFF4E79A7, 0xFFF28E2B, 0xFFE15759, 0xFF76B7B2,
    0xFF59A14F, 0xFFB07AA1, 0xFF9C755F, 0xFF5470C6,
];

fn avatar_color(seed: &str) -> u32 {
    let mut h: u32 = 0x811C9DC5;
    for b in seed.bytes() {
        h = h.wrapping_mul(0x01000193) ^ b as u32;
    }
    AVATAR_PALETTE[(h as usize) % AVATAR_PALETTE.len()]
}

fn compute_initials(display: &str, fallback: &str) -> String {
    let src = if display.trim().is_empty() { fallback } else { display };
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

fn update_avatar() {
    let canvas = match unsafe { AVATAR } {
        Some(v) => v,
        None => return,
    };
    let tc = ui::theme::colors();
    let username = current_username();
    let displayname = current_displayname();

    // Background matches window so circle edges anti-alias correctly.
    canvas.clear(tc.window_bg);

    let half = (AVATAR_SIZE as i32) / 2;
    let radius = half - 1;
    let bg = avatar_color(username);
    canvas.fill_circle(half, half, radius, bg);

    let initials = compute_initials(displayname, username);
    let font_size: u16 = 40;
    // Approximate text width — bold uppercase letters average ~0.6 * font size.
    let glyph_w = (font_size as i32) * 6 / 10;
    let text_w = glyph_w * (initials.chars().count() as i32);
    let tx = half - text_w / 2;
    let ty = half - (font_size as i32) / 2 - 2;
    canvas.draw_text(tx, ty, 0xFFFFFFFF, 1, font_size, &initials);
}

fn attempt_login(pf_id: u32) -> bool {
    let username = current_username();
    if username.is_empty() {
        return false;
    }

    let mut pbuf = [0u8; 128];
    let plen = ui::Control::from_id(pf_id).get_text(&mut pbuf);
    let password = core::str::from_utf8(&pbuf[..plen as usize]).unwrap_or("");

    if process::authenticate(username, password) {
        // Persist last successful user for next boot.
        let _ = login_schema().write_string("state/last_user", username);
        unsafe { LOGIN_UID = process::getuid() as u32; }
        true
    } else {
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            i18n::t("Invalid password.\nPlease try again."),
            Some(i18n::t("OK")),
        );
        ui::Control::from_id(pf_id).set_text("");
        ui::Control::from_id(pf_id).focus();
        false
    }
}
