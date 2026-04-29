#![no_std]
#![no_main]

anyos_std::entry!(main);

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{i18n, println, process, sys, users};
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

/// Read the avatar path that the user with `uid` configured in Settings →
/// Profile. Login runs as root so it may read any user's User-scope entries.
fn read_avatar_path(uid: u32) -> Option<String> {
    use libconf::{ConfClient, ConfTarget, ConfValue};
    let mut client = ConfClient::connect("login").ok()?;
    let item = client
        .get_target(ConfTarget::User(uid as u16), "profiles/avatar")
        .ok()?;
    match item.value? {
        ConfValue::String(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

const CURRENT_AVATAR_SIZE: u32 = 72;
const PICKER_AVATAR_SIZE: u32 = 64;
const PASS_H: u32 = 34;

#[derive(Clone, Copy)]
struct LoginPalette {
    text: u32,
    text_soft: u32,
    shadow_soft: u32,
    shadow_near: u32,
    avatar_glow_outer: u32,
    avatar_glow_inner: u32,
    panel: u32,
}

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
static mut DATE_LBL_ID: u32 = 0;
static mut TIME_LBL_ID: u32 = 0;
static mut DATE_SHADOW_SOFT_ID: u32 = 0;
static mut DATE_SHADOW_ID: u32 = 0;
static mut TIME_SHADOW_SOFT_ID: u32 = 0;
static mut TIME_SHADOW_ID: u32 = 0;
static mut NAME_SHADOW_SOFT_ID: u32 = 0;
static mut NAME_SHADOW_ID: u32 = 0;
static mut ERROR_SHADOW_SOFT_ID: u32 = 0;
static mut ERROR_SHADOW_ID: u32 = 0;
static mut ERROR_LBL_ID: u32 = 0;
static mut LOGIN_CONTROLS: Option<Vec<u32>> = None;
static mut PICKER_CONTROLS: Option<Vec<u32>> = None;
static mut AVATAR_GLOW: Option<ui::Canvas> = None;
static mut CENTER_AVATAR_POS: (i32, i32) = (0, 0);
static mut CENTER_GLOW_POS: (i32, i32) = (0, 0);
static mut CENTER_GLOW_SIZE: u32 = 0;
static mut PICKER_AVATARS: Option<Vec<AvatarAnim>> = None;
static mut SELECT_ANIM: Option<SelectAnim> = None;
static mut AVATAR_TIMER_ID: u32 = 0;

#[derive(Clone, Copy)]
struct AvatarAnim {
    glow: ui::Canvas,
    avatar: ui::Canvas,
    avatar_x: i32,
    avatar_y: i32,
    glow_x: i32,
    glow_y: i32,
    avatar_size: u32,
    glow_size: u32,
    pad: u32,
    hover: u16,
    target: u16,
    pressed: bool,
    shown_pressed: bool,
}

#[derive(Clone, Copy)]
struct SelectAnim {
    ticks: u8,
    from_avatar_x: i32,
    from_avatar_y: i32,
    from_glow_x: i32,
    from_glow_y: i32,
    to_avatar_x: i32,
    to_avatar_y: i32,
    to_glow_x: i32,
    to_glow_y: i32,
    returning_to_picker: bool,
}

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

    // Pick last_user from confd. Without a persisted last user, start in picker mode.
    let mut current: usize = 0;
    let mut has_last_user = false;
    if let Some(name) = login_schema().read_string("state/last_user") {
        let name = name.trim();
        if !name.is_empty() {
            if let Some(idx) = list.iter().position(|(_, u, _)| u == name) {
                current = idx;
                has_last_user = true;
            }
        }
    }

    unsafe {
        USERS = Some(list);
        CURRENT_USER = current;
    }

    // ── Full-screen overlay ─────────────────────────────────────────────
    let (sw, sh) = ui::screen_size();
    let flags = ui::WIN_FLAG_BORDERLESS
        | ui::WIN_FLAG_ALWAYS_ON_TOP
        | ui::WIN_FLAG_NOT_RESIZABLE
        | ui::WIN_FLAG_NO_CLOSE
        | ui::WIN_FLAG_NO_MINIMIZE
        | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags(i18n::t("Login"), 0, 0, sw, sh, flags);
    win.set_color(0x00000000);
    ui::set_blur_behind(&win, 18);

    let palette = choose_login_palette(sw, sh, current_uid());
    let mut login_ids: Vec<u32> = Vec::new();
    let mut picker_ids: Vec<u32> = Vec::new();
    let text = palette.text;
    let text_soft = palette.text_soft;
    let shadow_soft = palette.shadow_soft;
    let shadow_near = palette.shadow_near;
    let avatar_glow_outer = palette.avatar_glow_outer;
    let avatar_glow_inner = palette.avatar_glow_inner;
    let panel = palette.panel;

    // ── Large lock-screen clock ─────────────────────────────────────────
    let date_shadow_soft = ui::Label::new("");
    date_shadow_soft.set_font_size((sh / 42).clamp(15, 28));
    date_shadow_soft.set_font(1);
    date_shadow_soft.set_text_color(shadow_soft);
    date_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
    date_shadow_soft.set_position(3, (sh as i32 * 9) / 100 + 4);
    date_shadow_soft.set_size(sw, (sh / 22).max(24));
    win.add(&date_shadow_soft);
    unsafe { DATE_SHADOW_SOFT_ID = date_shadow_soft.id(); }

    let date_shadow = ui::Label::new("");
    date_shadow.set_font_size((sh / 42).clamp(15, 28));
    date_shadow.set_font(1);
    date_shadow.set_text_color(shadow_near);
    date_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
    date_shadow.set_position(1, (sh as i32 * 9) / 100 + 2);
    date_shadow.set_size(sw, (sh / 22).max(24));
    win.add(&date_shadow);
    unsafe { DATE_SHADOW_ID = date_shadow.id(); }

    let date_lbl = ui::Label::new("");
    date_lbl.set_font_size((sh / 42).clamp(15, 28));
    date_lbl.set_font(1);
    date_lbl.set_text_color(text_soft);
    date_lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
    date_lbl.set_position(0, (sh as i32 * 9) / 100);
    date_lbl.set_size(sw, (sh / 22).max(24));
    win.add(&date_lbl);
    unsafe { DATE_LBL_ID = date_lbl.id(); }

    let time_shadow_soft = ui::Label::new("");
    time_shadow_soft.set_font_size((sh / 8).clamp(58, 128));
    time_shadow_soft.set_font(1);
    time_shadow_soft.set_text_color(shadow_soft);
    time_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
    time_shadow_soft.set_position(5, (sh as i32 * 12) / 100 + 8);
    time_shadow_soft.set_size(sw, (sh / 6).max(90));
    win.add(&time_shadow_soft);
    unsafe { TIME_SHADOW_SOFT_ID = time_shadow_soft.id(); }

    let time_shadow = ui::Label::new("");
    time_shadow.set_font_size((sh / 8).clamp(58, 128));
    time_shadow.set_font(1);
    time_shadow.set_text_color(shadow_near);
    time_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
    time_shadow.set_position(2, (sh as i32 * 12) / 100 + 4);
    time_shadow.set_size(sw, (sh / 6).max(90));
    win.add(&time_shadow);
    unsafe { TIME_SHADOW_ID = time_shadow.id(); }

    let time_lbl = ui::Label::new("");
    time_lbl.set_font_size((sh / 8).clamp(58, 128));
    time_lbl.set_font(1);
    time_lbl.set_text_color(text_soft);
    time_lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
    time_lbl.set_position(0, (sh as i32 * 12) / 100);
    time_lbl.set_size(sw, (sh / 6).max(90));
    win.add(&time_lbl);
    unsafe { TIME_LBL_ID = time_lbl.id(); }
    update_clock_labels();
    ui::set_timer(1000, || update_clock_labels());

    // ── Top-right power control ─────────────────────────────────────────
    let top_power = ui::IconButton::new("");
    top_power.set_system_icon("power", IconType::Outline, text, (sh / 45).clamp(18, 28));
    top_power.set_flat_style(true);
    top_power.set_position((sw as i32 * 94) / 100, (sh as i32 * 35) / 1000);
    top_power.set_size((sh / 24).clamp(30, 42), (sh / 24).clamp(30, 42));
    top_power.on_click(|_| process::shutdown());
    win.add(&top_power);

    // ── Selected-user login block ───────────────────────────────────────
    let center_x = sw as i32 / 2;
    let avatar_y = (sh as i32 * 58) / 100;
    let avatar_x = center_x - CURRENT_AVATAR_SIZE as i32 / 2;
    let current_glow_pad = 16;
    let avatar_glow = ui::Canvas::new(CURRENT_AVATAR_SIZE + current_glow_pad * 2, CURRENT_AVATAR_SIZE + current_glow_pad * 2);
    avatar_glow.set_position(avatar_x - current_glow_pad as i32, avatar_y - current_glow_pad as i32);
    avatar_glow.set_size(CURRENT_AVATAR_SIZE + current_glow_pad * 2, CURRENT_AVATAR_SIZE + current_glow_pad * 2);
    avatar_glow.set_visible(has_last_user);
    draw_avatar_ring(
        &avatar_glow,
        CURRENT_AVATAR_SIZE,
        current_glow_pad,
        avatar_glow_outer,
        avatar_glow_inner,
        118,
    );
    win.add(&avatar_glow);
    login_ids.push(avatar_glow.id());
    unsafe {
        AVATAR_GLOW = Some(avatar_glow);
        CENTER_AVATAR_POS = (avatar_x, avatar_y);
        CENTER_GLOW_POS = (avatar_x - current_glow_pad as i32, avatar_y - current_glow_pad as i32);
        CENTER_GLOW_SIZE = CURRENT_AVATAR_SIZE + current_glow_pad * 2;
    }

    let avatar = ui::Canvas::new(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
    avatar.set_position(avatar_x, avatar_y);
    avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
    avatar.set_visible(has_last_user);
    win.add(&avatar);
    login_ids.push(avatar.id());
    unsafe { AVATAR = Some(avatar); }
    update_avatar();

    let name_y = avatar_y + CURRENT_AVATAR_SIZE as i32 + (sh as i32 * 2) / 100;
    let name_shadow_soft = ui::Label::new("");
    name_shadow_soft.set_font_size((sh / 38).clamp(18, 28));
    name_shadow_soft.set_font(1);
    name_shadow_soft.set_text_color(shadow_soft);
    name_shadow_soft.set_position(3, name_y + 4);
    name_shadow_soft.set_size(sw, 34);
    name_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
    name_shadow_soft.set_visible(has_last_user);
    win.add(&name_shadow_soft);
    login_ids.push(name_shadow_soft.id());
    unsafe { NAME_SHADOW_SOFT_ID = name_shadow_soft.id(); }

    let name_shadow = ui::Label::new("");
    name_shadow.set_font_size((sh / 38).clamp(18, 28));
    name_shadow.set_font(1);
    name_shadow.set_text_color(shadow_near);
    name_shadow.set_position(1, name_y + 2);
    name_shadow.set_size(sw, 34);
    name_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
    name_shadow.set_visible(has_last_user);
    win.add(&name_shadow);
    login_ids.push(name_shadow.id());
    unsafe { NAME_SHADOW_ID = name_shadow.id(); }

    let name_lbl = ui::Label::new("");
    name_lbl.set_font_size((sh / 38).clamp(18, 28));
    name_lbl.set_font(1);
    name_lbl.set_text_color(text);
    name_lbl.set_position(0, name_y);
    name_lbl.set_size(sw, 34);
    name_lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
    name_lbl.set_visible(has_last_user);
    win.add(&name_lbl);
    login_ids.push(name_lbl.id());
    unsafe { NAME_LBL_ID = name_lbl.id(); }
    update_name_label();

    let pass_w = (sw / 6).clamp(220, 320);
    let pass_y = name_y + (sh as i32 * 5) / 100;
    let pass_x = center_x - pass_w as i32 / 2;
    let back_size = PASS_H + 8;
    let back_btn = ui::IconButton::new("");
    back_btn.set_system_icon("chevron-left", IconType::Outline, text, 24);
    back_btn.set_flat_style(true);
    back_btn.set_position(pass_x - back_size as i32 - 14, pass_y - 4);
    back_btn.set_size(back_size, back_size);
    back_btn.set_visible(has_last_user);
    win.add(&back_btn);
    login_ids.push(back_btn.id());

    let pass_field = ui::TextField::new();
    pass_field.set_password_mode(true);
    pass_field.set_placeholder(i18n::t("Enter Password"));
    pass_field.set_color(panel);
    pass_field.set_text_color(text);
    pass_field.set_style(ui::STYLE_BORDER, 0x80FFFFFF);
    pass_field.set_style(ui::STYLE_ACCENT, 0xFFFFFFFF);
    pass_field.set_style(ui::STYLE_RADIUS, PASS_H / 2);
    pass_field.set_position(pass_x, pass_y);
    pass_field.set_size(pass_w, PASS_H);
    pass_field.set_visible(has_last_user);
    win.add(&pass_field);
    login_ids.push(pass_field.id());
    unsafe { PASS_ID = pass_field.id(); }
    pass_field.focus();

    let submit_btn = ui::IconButton::new("");
    submit_btn.set_system_icon("arrow-right", IconType::Outline, text, 22);
    submit_btn.set_flat_style(true);
    submit_btn.set_position(pass_x + pass_w as i32 + 14, pass_y - 4);
    submit_btn.set_size(back_size, back_size);
    submit_btn.set_visible(has_last_user);
    win.add(&submit_btn);
    login_ids.push(submit_btn.id());

    let error_text = "";
    let error_color = 0xFFFF453A;
    let error_shadow_soft = ui::Label::new(error_text);
    error_shadow_soft.set_font_size((sh / 58).clamp(12, 17));
    error_shadow_soft.set_font(1);
    error_shadow_soft.set_text_color(0x26000000);
    error_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
    error_shadow_soft.set_position(center_x - pass_w as i32 / 2 - 28, pass_y + PASS_H as i32 + 10);
    error_shadow_soft.set_size(pass_w + 60, 24);
    error_shadow_soft.set_visible(has_last_user);
    win.add(&error_shadow_soft);
    login_ids.push(error_shadow_soft.id());
    unsafe { ERROR_SHADOW_SOFT_ID = error_shadow_soft.id(); }

    let error_shadow = ui::Label::new(error_text);
    error_shadow.set_font_size((sh / 58).clamp(12, 17));
    error_shadow.set_font(1);
    error_shadow.set_text_color(0x40000000);
    error_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
    error_shadow.set_position(center_x - pass_w as i32 / 2 - 29, pass_y + PASS_H as i32 + 8);
    error_shadow.set_size(pass_w + 60, 24);
    error_shadow.set_visible(has_last_user);
    win.add(&error_shadow);
    login_ids.push(error_shadow.id());
    unsafe { ERROR_SHADOW_ID = error_shadow.id(); }

    let error_lbl = ui::Label::new(error_text);
    error_lbl.set_font_size((sh / 58).clamp(12, 17));
    error_lbl.set_font(1);
    error_lbl.set_text_color(error_color);
    error_lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
    error_lbl.set_position(center_x - pass_w as i32 / 2 - 30, pass_y + PASS_H as i32 + 6);
    error_lbl.set_size(pass_w + 60, 24);
    error_lbl.set_visible(has_last_user);
    win.add(&error_lbl);
    login_ids.push(error_lbl.id());
    unsafe { ERROR_LBL_ID = error_lbl.id(); }

    let help = ui::Label::new(i18n::t("Your password is required to log in"));
    let help_shadow_soft = ui::Label::new(i18n::t("Your password is required to log in"));
    help_shadow_soft.set_font_size((sh / 55).clamp(13, 18));
    help_shadow_soft.set_font(1);
    help_shadow_soft.set_text_color(shadow_soft);
    help_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
    help_shadow_soft.set_position(center_x - pass_w as i32 / 2 - 27, pass_y + PASS_H as i32 + 34);
    help_shadow_soft.set_size(pass_w + 60, 48);
    help_shadow_soft.set_visible(has_last_user);
    win.add(&help_shadow_soft);
    login_ids.push(help_shadow_soft.id());

    let help_shadow = ui::Label::new(i18n::t("Your password is required to log in"));
    help_shadow.set_font_size((sh / 55).clamp(13, 18));
    help_shadow.set_font(1);
    help_shadow.set_text_color(shadow_near);
    help_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
    help_shadow.set_position(center_x - pass_w as i32 / 2 - 29, pass_y + PASS_H as i32 + 32);
    help_shadow.set_size(pass_w + 60, 48);
    help_shadow.set_visible(has_last_user);
    win.add(&help_shadow);
    login_ids.push(help_shadow.id());

    help.set_font_size((sh / 55).clamp(13, 18));
    help.set_font(1);
    help.set_text_color(text);
    help.set_text_align(ui::TEXT_ALIGN_CENTER);
    help.set_position(center_x - pass_w as i32 / 2 - 30, pass_y + PASS_H as i32 + 30);
    help.set_size(pass_w + 60, 48);
    help.set_visible(has_last_user);
    win.add(&help);
    login_ids.push(help.id());

    // ── User picker row ─────────────────────────────────────────────────
    let count = user_list().len().max(1) as u32;
    let cell_w = ((PICKER_AVATAR_SIZE + 52).max(sw / 12)).min(150);
    let total_w = (cell_w * count).min(sw.saturating_sub(40));
    let picker_x = sw as i32 / 2 - total_w as i32 / 2;
    let picker_y = (sh as i32 * 52) / 100;
    let mut avatar_anims: Vec<AvatarAnim> = Vec::new();
    for idx in 0..user_list().len() {
        let (uid, username, display) = {
            let users = user_list();
            let (uid, username, display) = &users[idx];
            (*uid, username.clone(), display.clone())
        };
        let cell_x = picker_x + idx as i32 * cell_w as i32;
        let picker_glow_pad = 14;
        let glow_size = PICKER_AVATAR_SIZE + picker_glow_pad * 2;
        let glow_x = cell_x + (cell_w as i32 - PICKER_AVATAR_SIZE as i32) / 2 - picker_glow_pad as i32;
        let glow = ui::Canvas::new(glow_size, glow_size);
        glow.set_position(glow_x, picker_y - picker_glow_pad as i32);
        glow.set_size(glow_size, glow_size);
        glow.set_visible(!has_last_user);
        draw_avatar_ring(
            &glow,
            PICKER_AVATAR_SIZE,
            picker_glow_pad,
            avatar_glow_outer,
            avatar_glow_inner,
            0,
        );
        win.add(&glow);
        picker_ids.push(glow.id());

        let av = ui::Canvas::new(PICKER_AVATAR_SIZE, PICKER_AVATAR_SIZE);
        let av_x = cell_x + (cell_w as i32 - PICKER_AVATAR_SIZE as i32) / 2;
        av.set_position(av_x, picker_y);
        av.set_size(PICKER_AVATAR_SIZE, PICKER_AVATAR_SIZE);
        av.set_interactive(true);
        av.set_visible(!has_last_user);
        draw_avatar_for(&av, PICKER_AVATAR_SIZE, uid, &username, &display);
        av.on_mouse_enter(move |_| set_avatar_hover(idx, true));
        av.on_mouse_leave(move |_| set_avatar_hover(idx, false));
        av.on_mouse_down(move |_, _, _| set_avatar_pressed(idx, true));
        av.on_mouse_up(move |_, _, _| set_avatar_pressed(idx, false));
        av.on_click(move |_| select_user_animated(idx));
        win.add(&av);
        picker_ids.push(av.id());
        avatar_anims.push(AvatarAnim {
            glow,
            avatar: av,
            avatar_x: av_x,
            avatar_y: picker_y,
            glow_x,
            glow_y: picker_y - picker_glow_pad as i32,
            avatar_size: PICKER_AVATAR_SIZE,
            glow_size,
            pad: picker_glow_pad,
            hover: 0,
            target: 0,
            pressed: false,
            shown_pressed: false,
        });

        let label_y = picker_y + PICKER_AVATAR_SIZE as i32 + 18;
        let lbl = ui::Label::new(&display);
        let lbl_shadow_soft = ui::Label::new(&display);
        lbl_shadow_soft.set_font_size((sh / 70).clamp(11, 15));
        lbl_shadow_soft.set_font(1);
        lbl_shadow_soft.set_text_color(shadow_soft);
        lbl_shadow_soft.set_text_align(ui::TEXT_ALIGN_CENTER);
        lbl_shadow_soft.set_position(cell_x + 3, label_y + 4);
        lbl_shadow_soft.set_size(cell_w, 22);
        lbl_shadow_soft.set_visible(!has_last_user);
        win.add(&lbl_shadow_soft);
        picker_ids.push(lbl_shadow_soft.id());

        let lbl_shadow = ui::Label::new(&display);
        lbl_shadow.set_font_size((sh / 70).clamp(11, 15));
        lbl_shadow.set_font(1);
        lbl_shadow.set_text_color(shadow_near);
        lbl_shadow.set_text_align(ui::TEXT_ALIGN_CENTER);
        lbl_shadow.set_position(cell_x + 1, label_y + 2);
        lbl_shadow.set_size(cell_w, 22);
        lbl_shadow.set_visible(!has_last_user);
        win.add(&lbl_shadow);
        picker_ids.push(lbl_shadow.id());

        lbl.set_font_size((sh / 70).clamp(11, 15));
        lbl.set_font(1);
        lbl.set_text_color(text);
        lbl.set_text_align(ui::TEXT_ALIGN_CENTER);
        lbl.set_position(cell_x, label_y);
        lbl.set_size(cell_w, 22);
        lbl.set_visible(!has_last_user);
        win.add(&lbl);
        picker_ids.push(lbl.id());
    }

    let pf_id = pass_field.id();
    submit_btn.on_click(move |_| {
        if attempt_login(pf_id) {
            ui::quit();
        }
    });
    pass_field.on_submit(move |_| {
        if attempt_login(pf_id) {
            ui::quit();
        }
    });
    back_btn.on_click(|_| show_picker_animated());

    unsafe {
        LOGIN_CONTROLS = Some(login_ids);
        PICKER_CONTROLS = Some(picker_ids);
        PICKER_AVATARS = Some(avatar_anims);
        AVATAR_TIMER_ID = ui::set_timer(16, || tick_avatar_animation());
    }
    if has_last_user {
        show_login();
    } else {
        show_picker();
    }

    ui::run();

    unsafe { LOGIN_UID }
}

fn current_username() -> &'static str {
    let users = user_list();
    let idx = unsafe { CURRENT_USER };
    users.get(idx).map(|(_, u, _)| u.as_str()).unwrap_or("")
}

fn current_uid() -> u32 {
    let users = user_list();
    let idx = unsafe { CURRENT_USER };
    users.get(idx).map(|(uid, _, _)| *uid).unwrap_or(0)
}

fn current_displayname() -> &'static str {
    let users = user_list();
    let idx = unsafe { CURRENT_USER };
    users.get(idx).map(|(_, _, d)| d.as_str()).unwrap_or("")
}

fn choose_login_palette(_sw: u32, _sh: u32, _uid: u32) -> LoginPalette {
    LoginPalette {
        text: 0xFFFFFFFF,
        text_soft: 0xF2FFFFFF,
        shadow_soft: 0x32000000,
        shadow_near: 0x70000000,
        avatar_glow_outer: 0x22FFFFFF,
        avatar_glow_inner: 0x24000000,
        panel: 0x66000000,
    }
}

fn update_name_label() {
    let text = current_displayname();
    let id = unsafe { NAME_LBL_ID };
    if id != 0 {
        ui::Control::from_id(id).set_text(text);
    }
    let shadow_id = unsafe { NAME_SHADOW_ID };
    if shadow_id != 0 {
        ui::Control::from_id(shadow_id).set_text(text);
    }
    let shadow_soft_id = unsafe { NAME_SHADOW_SOFT_ID };
    if shadow_soft_id != 0 {
        ui::Control::from_id(shadow_soft_id).set_text(text);
    }
}

fn set_login_error(message: &str) {
    unsafe {
        if ERROR_LBL_ID != 0 {
            ui::Control::from_id(ERROR_LBL_ID).set_text(message);
        }
        if ERROR_SHADOW_ID != 0 {
            ui::Control::from_id(ERROR_SHADOW_ID).set_text(message);
        }
        if ERROR_SHADOW_SOFT_ID != 0 {
            ui::Control::from_id(ERROR_SHADOW_SOFT_ID).set_text(message);
        }
    }
}

fn clear_login_error() {
    set_login_error("");
}

fn set_controls_visible(ids: &Option<Vec<u32>>, visible: bool) {
    if let Some(list) = ids {
        for id in list.iter() {
            ui::Control::from_id(*id).set_visible(visible);
        }
    }
}

fn show_login() {
    clear_login_error();
    unsafe {
        if let Some(glow) = AVATAR_GLOW {
            let (x, y) = CENTER_GLOW_POS;
            glow.set_position(x, y);
            glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
            draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 118);
        }
        set_controls_visible(&LOGIN_CONTROLS, true);
        set_controls_visible(&PICKER_CONTROLS, false);
    }
    ui::Control::from_id(unsafe { PASS_ID }).focus();
}

fn show_picker() {
    clear_login_error();
    unsafe {
        SELECT_ANIM = None;
        if let Some(avatar) = AVATAR {
            let (x, y) = CENTER_AVATAR_POS;
            avatar.set_position(x, y);
            avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
            update_avatar();
        }
        if let Some(glow) = AVATAR_GLOW {
            let (x, y) = CENTER_GLOW_POS;
            glow.set_position(x, y);
            glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
            draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 0);
        }
        set_controls_visible(&LOGIN_CONTROLS, false);
        set_controls_visible(&PICKER_CONTROLS, true);
    }
}

fn show_picker_animated() {
    clear_login_error();
    unsafe {
        let idx = CURRENT_USER;
        let Some(list) = PICKER_AVATARS.as_ref() else {
            show_picker();
            return;
        };
        let Some(anim) = list.get(idx) else {
            show_picker();
            return;
        };
        set_controls_visible(&PICKER_CONTROLS, true);
        for picker in list.iter() {
            picker.glow.set_visible(false);
            picker.avatar.set_visible(false);
        }
        if let Some(avatar) = AVATAR {
            let (x, y) = CENTER_AVATAR_POS;
            avatar.set_position(x, y);
            avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
            update_avatar();
        }
        if let Some(glow) = AVATAR_GLOW {
            let (x, y) = CENTER_GLOW_POS;
            glow.set_position(x, y);
            glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
            draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 120);
        }
        let to_avatar_x = anim.avatar_x - ((CURRENT_AVATAR_SIZE - anim.avatar_size) / 2) as i32;
        let to_avatar_y = anim.avatar_y - ((CURRENT_AVATAR_SIZE - anim.avatar_size) / 2) as i32;
        let to_glow_x = anim.glow_x - ((CENTER_GLOW_SIZE - anim.glow_size) / 2) as i32;
        let to_glow_y = anim.glow_y - ((CENTER_GLOW_SIZE - anim.glow_size) / 2) as i32;
        let (from_avatar_x, from_avatar_y) = CENTER_AVATAR_POS;
        let (from_glow_x, from_glow_y) = CENTER_GLOW_POS;
        SELECT_ANIM = Some(SelectAnim {
            ticks: 0,
            from_avatar_x,
            from_avatar_y,
            from_glow_x,
            from_glow_y,
            to_avatar_x,
            to_avatar_y,
            to_glow_x,
            to_glow_y,
            returning_to_picker: true,
        });
    }
}

fn select_user_animated(idx: usize) {
    if idx >= user_list().len() {
        return;
    }
    unsafe { CURRENT_USER = idx; }
    update_avatar();
    update_name_label();
    ui::Control::from_id(unsafe { PASS_ID }).set_text("");
    clear_login_error();
    unsafe {
        set_controls_visible(&LOGIN_CONTROLS, true);
        set_controls_visible(&PICKER_CONTROLS, false);
        if let Some(list) = PICKER_AVATARS.as_ref() {
            if let Some(anim) = list.get(idx) {
                let from_avatar_x = anim.avatar_x - ((CURRENT_AVATAR_SIZE - anim.avatar_size) / 2) as i32;
                let from_avatar_y = anim.avatar_y - ((CURRENT_AVATAR_SIZE - anim.avatar_size) / 2) as i32;
                let from_glow_x = anim.glow_x - ((CENTER_GLOW_SIZE - anim.glow_size) / 2) as i32;
                let from_glow_y = anim.glow_y - ((CENTER_GLOW_SIZE - anim.glow_size) / 2) as i32;
                if let Some(avatar) = AVATAR {
                    avatar.set_position(from_avatar_x, from_avatar_y);
                    avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
                    update_avatar();
                }
                if let Some(glow) = AVATAR_GLOW {
                    glow.set_position(from_glow_x, from_glow_y);
                    glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
                    draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 180);
                }
                let (to_avatar_x, to_avatar_y) = CENTER_AVATAR_POS;
                let (to_glow_x, to_glow_y) = CENTER_GLOW_POS;
                SELECT_ANIM = Some(SelectAnim {
                    ticks: 0,
                    from_avatar_x,
                    from_avatar_y,
                    from_glow_x,
                    from_glow_y,
                    to_avatar_x,
                    to_avatar_y,
                    to_glow_x,
                    to_glow_y,
                    returning_to_picker: false,
                });
                return;
            }
        }
    }
    show_login();
}

fn set_avatar_hover(idx: usize, hovering: bool) {
    unsafe {
        if let Some(list) = PICKER_AVATARS.as_mut() {
            if let Some(anim) = list.get_mut(idx) {
                anim.target = if hovering { 255 } else { 0 };
                if !hovering {
                    anim.pressed = false;
                }
            }
        }
    }
}

fn set_avatar_pressed(idx: usize, pressed: bool) {
    unsafe {
        if let Some(list) = PICKER_AVATARS.as_mut() {
            if let Some(anim) = list.get_mut(idx) {
                anim.pressed = pressed;
                anim.target = if pressed { 255 } else { anim.target };
            }
        }
    }
}

fn tick_avatar_animation() {
    unsafe {
        if let Some(list) = PICKER_AVATARS.as_mut() {
            for anim in list.iter_mut() {
                let target = anim.target;
                let old_hover = anim.hover;
                if anim.hover < target {
                    anim.hover = (anim.hover + 26).min(target);
                } else if anim.hover > target {
                    anim.hover = anim.hover.saturating_sub(22).max(target);
                }
                if old_hover == anim.hover && anim.shown_pressed == anim.pressed {
                    continue;
                }
                anim.shown_pressed = anim.pressed;
                let inset = if anim.pressed { 2 } else { 0 };
                anim.avatar.set_position(anim.avatar_x + inset, anim.avatar_y + inset);
                anim.avatar.set_size(anim.avatar_size, anim.avatar_size);
                draw_avatar_ring(
                    &anim.glow,
                    anim.avatar_size,
                    anim.pad,
                    0x22FFFFFF,
                    0x24000000,
                    anim.hover,
                );
            }
        }

        if let Some(sel) = SELECT_ANIM.as_mut() {
            sel.ticks = sel.ticks.saturating_add(1);
            let t = sel.ticks.min(14) as i32;
            let eased = t * t * (42 - 2 * t) / (14 * 14);
            let av_x = lerp_i32(sel.from_avatar_x, sel.to_avatar_x, eased, 14);
            let av_y = lerp_i32(sel.from_avatar_y, sel.to_avatar_y, eased, 14);
            let gl_x = lerp_i32(sel.from_glow_x, sel.to_glow_x, eased, 14);
            let gl_y = lerp_i32(sel.from_glow_y, sel.to_glow_y, eased, 14);
            if let Some(avatar) = AVATAR {
                avatar.set_position(av_x, av_y);
                avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
            }
            if let Some(glow) = AVATAR_GLOW {
                glow.set_position(gl_x, gl_y);
                glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
                draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 160);
            }
            if sel.ticks >= 14 {
                let returning_to_picker = sel.returning_to_picker;
                if let Some(avatar) = AVATAR {
                    avatar.set_position(sel.to_avatar_x, sel.to_avatar_y);
                    avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
                    update_avatar();
                }
                if let Some(glow) = AVATAR_GLOW {
                    glow.set_position(sel.to_glow_x, sel.to_glow_y);
                    glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
                    let final_glow = if returning_to_picker { 0 } else { 118 };
                    draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, final_glow);
                }
                SELECT_ANIM = None;
                if returning_to_picker {
                    if let Some(list) = PICKER_AVATARS.as_mut() {
                        for anim in list.iter_mut() {
                            anim.hover = 0;
                            anim.target = 0;
                            anim.pressed = false;
                            anim.shown_pressed = false;
                            anim.avatar.set_position(anim.avatar_x, anim.avatar_y);
                            anim.avatar.set_size(anim.avatar_size, anim.avatar_size);
                            anim.avatar.set_visible(true);
                            anim.glow.set_position(anim.glow_x, anim.glow_y);
                            anim.glow.set_size(anim.glow_size, anim.glow_size);
                            draw_avatar_ring(&anim.glow, anim.avatar_size, anim.pad, 0x22FFFFFF, 0x24000000, 0);
                            anim.glow.set_visible(true);
                        }
                    }
                    set_controls_visible(&LOGIN_CONTROLS, false);
                    set_controls_visible(&PICKER_CONTROLS, true);
                } else {
                    let (to_ax, to_ay) = CENTER_AVATAR_POS;
                    let (to_gx, to_gy) = CENTER_GLOW_POS;
                    if let Some(avatar) = AVATAR {
                        avatar.set_position(to_ax, to_ay);
                        avatar.set_size(CURRENT_AVATAR_SIZE, CURRENT_AVATAR_SIZE);
                        update_avatar();
                    }
                    if let Some(glow) = AVATAR_GLOW {
                        glow.set_position(to_gx, to_gy);
                        glow.set_size(CENTER_GLOW_SIZE, CENTER_GLOW_SIZE);
                        draw_avatar_ring(&glow, CURRENT_AVATAR_SIZE, 16, 0x22FFFFFF, 0x24000000, 118);
                    }
                    ui::Control::from_id(PASS_ID).focus();
                }
            }
        }
    }
}

fn lerp_i32(a: i32, b: i32, t: i32, denom: i32) -> i32 {
    a + ((b - a) * t) / denom
}

fn update_clock_labels() {
    let mut b = [0u8; 8];
    sys::time(&mut b);
    let year = (b[0] as u32) | ((b[1] as u32) << 8);
    let month = b[2].max(1).min(12) as u32;
    let day = b[3].max(1).min(31) as u32;
    let hour = b[4];
    let minute = b[5];
    let time = format!("{:02}:{:02}", hour, minute);
    let date = format!(
        "{}, {} {}",
        weekday_name(year, month, day),
        month_name(month),
        day
    );
    unsafe {
        if TIME_LBL_ID != 0 {
            ui::Control::from_id(TIME_LBL_ID).set_text(&time);
        }
        if TIME_SHADOW_ID != 0 {
            ui::Control::from_id(TIME_SHADOW_ID).set_text(&time);
        }
        if TIME_SHADOW_SOFT_ID != 0 {
            ui::Control::from_id(TIME_SHADOW_SOFT_ID).set_text(&time);
        }
        if DATE_LBL_ID != 0 {
            ui::Control::from_id(DATE_LBL_ID).set_text(&date);
        }
        if DATE_SHADOW_ID != 0 {
            ui::Control::from_id(DATE_SHADOW_ID).set_text(&date);
        }
        if DATE_SHADOW_SOFT_ID != 0 {
            ui::Control::from_id(DATE_SHADOW_SOFT_ID).set_text(&date);
        }
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

fn weekday_name(year: u32, month: u32, day: u32) -> &'static str {
    let (y, m) = if month < 3 { (year - 1, month + 12) } else { (year, month) };
    let k = y % 100;
    let j = y / 100;
    let h = (day + (13 * (m + 1)) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    match h {
        0 => "Saturday",
        1 => "Sunday",
        2 => "Monday",
        3 => "Tuesday",
        4 => "Wednesday",
        5 => "Thursday",
        _ => "Friday",
    }
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
    let username = current_username();
    let displayname = current_displayname();

    draw_avatar_for(&canvas, CURRENT_AVATAR_SIZE, current_uid(), username, displayname);
}

fn draw_avatar_for(canvas: &ui::Canvas, size: u32, uid: u32, username: &str, displayname: &str) {
    canvas.clear(0);

    if let Some(path) = read_avatar_path(uid) {
        if let Some((pixels, w, h)) = load_avatar_image(&path, size) {
            blit_circular(canvas, &pixels, w, h, size);
            return;
        }
    }

    let half = (size as i32) / 2;
    let radius = half - 1;
    let bg = avatar_color(username);
    canvas.fill_circle(half, half, radius, bg);

    let initials = compute_initials(displayname, username);
    let font_size: u16 = (size * 5 / 12).clamp(24, 42) as u16;
    let glyph_w = (font_size as i32) * 6 / 10;
    let text_w = glyph_w * (initials.chars().count() as i32);
    let tx = half - text_w / 2;
    let ty = half - (font_size as i32) / 2 - 2;
    canvas.draw_text(tx, ty, 0xFFFFFFFF, 1, font_size, &initials);
}

fn draw_avatar_ring(canvas: &ui::Canvas, avatar_size: u32, pad: u32, outer: u32, inner: u32, glow: u16) {
    let size = avatar_size + pad * 2;
    let center = (size / 2) as i32;
    let avatar_r = (avatar_size as i32) / 2;
    let ring_inner = avatar_r + 2;
    let ring_outer = ring_inner + 3;
    let halo_outer = avatar_r + pad as i32 - 5;
    let glow_i = glow as i32;
    canvas.clear(0);
    if glow == 0 {
        return;
    }
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let dx = x - center;
            let dy = y - center;
            let d2 = dx * dx + dy * dy;
            let mut px = 0;

            if d2 >= ring_inner * ring_inner && d2 <= ring_outer * ring_outer {
                let alpha = 126 + (glow_i * 86) / 255;
                px = rgba(alpha as u32, 255, 255, 255);
            } else if d2 > ring_outer * ring_outer && d2 <= (ring_outer + 2) * (ring_outer + 2) {
                let alpha = 36 + (glow_i * 76) / 255;
                px = rgba(alpha as u32, 0, 0, 0);
            } else if glow > 0 && d2 > (ring_outer + 2) * (ring_outer + 2) && d2 <= halo_outer * halo_outer {
                let dist = isqrt(d2 as u32) as i32;
                let span = (halo_outer - ring_outer - 2).max(1);
                let falloff = ((halo_outer - dist).max(0) * 255) / span;
                let alpha = (glow_i * falloff * 58) / (255 * 255);
                if alpha > 0 {
                    let r = ((outer >> 16) & 0xFF) as u32;
                    let g = ((outer >> 8) & 0xFF) as u32;
                    let b = (outer & 0xFF) as u32;
                    px = rgba(alpha as u32, r, g, b);
                }
            } else if d2 >= (avatar_r - 1) * (avatar_r - 1) && d2 < ring_inner * ring_inner {
                let alpha = ((inner >> 24) & 0xFF).max(22);
                px = rgba(alpha, 0, 0, 0);
            }

            if px != 0 {
                canvas.set_pixel(x, y, px);
            }
        }
    }
}

fn rgba(a: u32, r: u32, g: u32, b: u32) -> u32 {
    ((a.min(255)) << 24) | ((r.min(255)) << 16) | ((g.min(255)) << 8) | b.min(255)
}

fn isqrt(n: u32) -> u32 {
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn load_image_pixels(path: &str, max_pixels: usize) -> Option<(Vec<u32>, u32, u32)> {
    use anyos_std::fs;
    let fd = fs::open(path, 0);
    if fd == u32::MAX { return None; }
    let mut data: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    let info = libimage_client::probe(&data)?;
    let w = info.width;
    let h = info.height;
    if w == 0 || h == 0 { return None; }
    let pixel_count = (w as usize).saturating_mul(h as usize);
    if pixel_count == 0 || pixel_count > max_pixels {
        return None;
    }
    let mut pixels = alloc::vec![0u32; pixel_count];
    let mut scratch = alloc::vec![0u8; info.scratch_needed as usize];
    libimage_client::decode(&data, &mut pixels, &mut scratch).ok()?;
    Some((pixels, w, h))
}

fn load_avatar_image(path: &str, target_size: u32) -> Option<(Vec<u32>, u32, u32)> {
    let (pixels, w, h) = load_image_pixels(path, 4 * 1024 * 1024)?;
    let side = w.min(h);
    let ox = (w - side) / 2;
    let oy = (h - side) / 2;
    let mut cropped = alloc::vec![0u32; (side as usize) * (side as usize)];
    for y in 0..side {
        let src_off = ((oy + y) * w + ox) as usize;
        let dst_off = (y * side) as usize;
        cropped[dst_off..dst_off + side as usize]
            .copy_from_slice(&pixels[src_off..src_off + side as usize]);
    }
    let mut scaled = alloc::vec![0u32; (target_size as usize) * (target_size as usize)];
    if libimage_client::scale_image(
        &cropped, side, side,
        &mut scaled, target_size, target_size,
        libimage_client::MODE_SCALE,
    ) {
        Some((scaled, target_size, target_size))
    } else {
        Some((cropped, side, side))
    }
}

fn blit_circular(canvas: &ui::Canvas, pixels: &[u32], w: u32, h: u32, size: u32) {
    let cx = (size as i32) / 2;
    let cy = (size as i32) / 2;
    let r = cx - 1;
    let r2 = r * r;
    for y in 0..h.min(size) {
        for x in 0..w.min(size) {
            let dx = x as i32 - cx;
            let dy = y as i32 - cy;
            if dx * dx + dy * dy <= r2 {
                let p = pixels[(y * w + x) as usize];
                canvas.set_pixel(x as i32, y as i32, p | 0xFF000000);
            }
        }
    }
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
        set_login_error(i18n::t("Invalid password. Please try again."));
        ui::Control::from_id(pf_id).set_text("");
        ui::Control::from_id(pf_id).focus();
        false
    }
}
