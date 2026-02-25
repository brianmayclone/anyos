#![no_std]
#![no_main]

anyos_std::entry!(main);

use alloc::boxed::Box;
use anyos_std::ipc;
use anyos_std::permissions;
use anyos_std::process;
use libanyui_client as ui;
use ui::Widget;

mod render;
mod perms;

use perms::{parse_hex, PERM_GROUPS};
use render::*;

/// Named pipe used as a singleton lock.
const LOCK_PIPE: &str = "_permdialog_lock";

fn main() {
    anyos_std::println!("[permdialog] start");

    // ── Singleton check: only one PermissionDialog at a time ──
    let existing = ipc::pipe_open(LOCK_PIPE);
    if existing != 0 {
        process::exit(1);
    }
    let lock_pipe = ipc::pipe_create(LOCK_PIPE);
    if lock_pipe == 0 || lock_pipe == u32::MAX {
        process::exit(1);
    }

    // ── Parse args: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path" ──
    let mut args_buf = [0u8; 256];
    let args_len = process::getargs(&mut args_buf);
    let args_str = if args_len > 0 {
        core::str::from_utf8(&args_buf[..args_len]).unwrap_or("")
    } else {
        ""
    };

    anyos_std::println!("[permdialog] args_len={}, args='{}'", args_len, args_str);

    if args_str.is_empty() {
        anyos_std::println!("[permdialog] empty args, exiting");
        cleanup(lock_pipe, 1);
    }

    let (app_id, app_name, declared_caps) = match parse_args(args_str) {
        Some(parsed) => parsed,
        None => cleanup(lock_pipe, 1),
    };

    // ── Determine which permission groups the app actually requests ──
    let mut active_groups: [(usize, bool); 6] = [(0, false); 6];
    let mut num_active = 0usize;
    for (i, group) in PERM_GROUPS.iter().enumerate() {
        if declared_caps & group.mask != 0 {
            active_groups[num_active] = (i, true);
            num_active += 1;
        }
    }

    anyos_std::println!(
        "[permdialog] app_id='{}' app_name='{}' caps=0x{:x} num_active={}",
        app_id, app_name, declared_caps, num_active
    );

    if num_active == 0 {
        permissions::perm_store(app_id, 0, 0);
        cleanup(lock_pipe, 0);
    }

    // ── Initialize libanyui ──
    anyos_std::println!("[permdialog] init libanyui");
    if !ui::init() {
        anyos_std::println!("[permdialog] libanyui init failed");
        cleanup(lock_pipe, 1);
    }

    // ── Compute dialog dimensions (same formula as before) ──
    let header_h: i32 = 80;
    let list_h = num_active as i32 * ROW_H;
    let footer_h: i32 = BTN_H as i32 + BTN_PAD * 2 + 8;
    let dialog_h = (header_h + list_h + footer_h) as u32;

    let (sw, sh) = ui::screen_size();
    let dx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let dy = ((sh as i32 - dialog_h as i32) / 2).max(0);

    // ── Create the dialog window (not resizable, not closable) ──
    let flags = ui::WIN_FLAG_NOT_RESIZABLE
        | ui::WIN_FLAG_NO_CLOSE
        | ui::WIN_FLAG_NO_MINIMIZE
        | ui::WIN_FLAG_NO_MAXIMIZE;
    let win = ui::Window::new_with_flags("Permissions", dx, dy, DIALOG_W, dialog_h, flags);

    anyos_std::println!("[permdialog] dialog win={}", win.id());
    if win.id() == u32::MAX {
        cleanup(lock_pipe, 1);
    }

    let tc = ui::theme::colors();
    let content_w = (DIALOG_W as i32 - PAD * 2) as u32; // 332

    // ── Title: "AppName" wants access to: ──
    let mut title_buf = [0u8; 128];
    let title_len = format_title(app_name, &mut title_buf);
    let title_str = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("App wants access to:");

    let title_lbl = ui::Label::new(title_str);
    title_lbl.set_position(PAD, 20);
    title_lbl.set_size(content_w, 22);
    title_lbl.set_font_size(13);
    title_lbl.set_text_color(tc.text);
    win.add(&title_lbl);

    // ── Subtitle ──
    let sub_lbl = ui::Label::new("This app requests the following permissions:");
    sub_lbl.set_position(PAD, 46);
    sub_lbl.set_size(content_w, 16);
    sub_lbl.set_font_size(11);
    sub_lbl.set_text_color(tc.text_secondary);
    win.add(&sub_lbl);

    // ── Header separator ──
    let hdiv = ui::Divider::new();
    hdiv.set_position(PAD, 74);
    hdiv.set_size(content_w, 1);
    win.add(&hdiv);

    // ── Permission rows ──
    let mut cb_ids = [0u32; 6];

    for i in 0..num_active {
        let group = &PERM_GROUPS[active_groups[i].0];
        let row_y = header_h + (i as i32 * ROW_H);

        // Checkbox with group name — checked by default
        let cb = ui::Checkbox::new(group.name);
        cb.set_position(PAD, row_y + 8);
        cb.set_size(content_w, 20);
        cb.set_state(1);
        cb_ids[i] = cb.id();
        win.add(&cb);

        // Description label
        let desc = ui::Label::new(group.desc);
        desc.set_position(PAD + 30, row_y + 28);
        desc.set_size(content_w - 30, 14);
        desc.set_font_size(11);
        desc.set_text_color(tc.text_secondary);
        win.add(&desc);

        // Row separator (between groups, not after the last one)
        if i + 1 < num_active {
            let rdiv = ui::Divider::new();
            rdiv.set_position(PAD, row_y + ROW_H - 2);
            rdiv.set_size(content_w, 1);
            win.add(&rdiv);
        }
    }

    // ── Footer separator ──
    let btn_div_y = dialog_h as i32 - BTN_H as i32 - BTN_PAD * 2 - 12;
    let fdiv = ui::Divider::new();
    fdiv.set_position(PAD, btn_div_y);
    fdiv.set_size(content_w, 1);
    win.add(&fdiv);

    // ── Buttons: "Don't Allow" (left) and "Allow" (right, accent color) ──
    let total_btn_w = BTN_W * 2 + BTN_PAD as u32;
    let btn_base_x = (DIALOG_W as i32 - total_btn_w as i32) / 2;
    let btn_y = dialog_h as i32 - BTN_H as i32 - BTN_PAD;

    let deny = ui::Button::new("Don't Allow");
    deny.set_position(btn_base_x, btn_y);
    deny.set_size(BTN_W, BTN_H);
    win.add(&deny);

    let allow = ui::Button::new("Allow");
    allow.set_position(btn_base_x + BTN_W as i32 + BTN_PAD, btn_y);
    allow.set_size(BTN_W, BTN_H);
    allow.set_color(tc.accent);
    win.add(&allow);

    // ── Callbacks ──

    // Leak app_id into a 'static str so it can be captured by the Allow closure.
    // Safe: permdialog is a short-lived process; the allocation lives until process exit.
    let app_id_bytes: &'static [u8] = Box::leak(app_id.as_bytes().to_vec().into_boxed_slice());
    let app_id_static: &'static str = core::str::from_utf8(app_id_bytes).unwrap_or("");

    // Deny — close lock and exit with code 1
    deny.on_click(move |_| {
        ipc::pipe_close(lock_pipe);
        process::exit(1);
    });

    // Allow — collect checked permissions, store, exit with 0 (or 2 on store failure)
    let active_groups_snap = active_groups;
    let cb_ids_snap = cb_ids;
    allow.on_click(move |_| {
        let mut granted: u32 = 0;
        for i in 0..num_active {
            if ui::Control::from_id(cb_ids_snap[i]).get_state() != 0 {
                granted |= PERM_GROUPS[active_groups_snap[i].0].mask;
            }
        }
        ipc::pipe_close(lock_pipe);
        if permissions::perm_store(app_id_static, granted, 0) {
            process::exit(0);
        } else {
            process::exit(2);
        }
    });

    // Escape key — treated as deny
    win.on_key_down(move |ke| {
        if ke.keycode == ui::KEY_ESCAPE {
            ipc::pipe_close(lock_pipe);
            process::exit(1);
        }
    });

    anyos_std::println!("[permdialog] entering event loop");
    ui::run();

    // Unreachable: WIN_FLAG_NO_CLOSE means run() only exits via process::exit() in a callback
    cleanup(lock_pipe, 1);
}

/// Parse `"\x1F"`-delimited args into `(app_id, app_name, declared_caps)`.
fn parse_args(args_str: &str) -> Option<(&str, &str, u32)> {
    let mut offsets: [(usize, usize); 4] = [(0, 0); 4];
    let mut part_count = 0usize;
    let mut start = 0usize;
    let bytes = args_str.as_bytes();

    for i in 0..bytes.len() {
        if bytes[i] == 0x1F && part_count < 4 {
            offsets[part_count] = (start, i);
            part_count += 1;
            start = i + 1;
        }
    }
    if part_count < 4 && start <= bytes.len() {
        offsets[part_count] = (start, bytes.len());
        part_count += 1;
    }

    if part_count < 3 {
        return None;
    }

    let app_id = &args_str[offsets[0].0..offsets[0].1];
    let app_name = &args_str[offsets[1].0..offsets[1].1];
    let caps_hex = &args_str[offsets[2].0..offsets[2].1];
    let declared_caps = parse_hex(caps_hex);

    Some((app_id, app_name, declared_caps))
}
