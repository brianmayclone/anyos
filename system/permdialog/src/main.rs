#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::ipc;
use anyos_std::permissions;
use anyos_std::process;
use anyos_std::sys;
use anyos_std::ui::window;
use uisys_client::*;

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
        cleanup(lock_pipe, u32::MAX, u32::MAX, 1);
    }

    let (app_id, app_name, declared_caps) = match parse_args(args_str) {
        Some(parsed) => parsed,
        None => cleanup(lock_pipe, u32::MAX, u32::MAX, 1),
    };

    // Determine which permission groups the app actually requests
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
        cleanup(lock_pipe, u32::MAX, u32::MAX, 0);
    }

    // ── Create fullscreen dim overlay ──
    anyos_std::println!("[permdialog] creating windows");
    let (sw, sh) = window::screen_size();

    let dim_flags = window::WIN_FLAG_BORDERLESS
        | window::WIN_FLAG_SHADOW
        | window::WIN_FLAG_ALWAYS_ON_TOP
        | window::WIN_FLAG_NOT_RESIZABLE
        | window::WIN_FLAG_NO_CLOSE
        | window::WIN_FLAG_NO_MINIMIZE
        | window::WIN_FLAG_NO_MAXIMIZE
        | window::WIN_FLAG_NO_MOVE;

    let dim_win = window::create_ex("", 0, 0, sw as u16, sh as u16, dim_flags);
    if dim_win == u32::MAX {
        cleanup(lock_pipe, u32::MAX, u32::MAX, 1);
    }
    window::fill_rect(dim_win, 0, 0, sw as u16, sh as u16, 0x80000000);
    window::present(dim_win);

    // ── Create the actual dialog ──
    let header_h: i32 = 80;
    let list_h = num_active as i32 * ROW_H;
    let footer_h: i32 = BTN_H as i32 + BTN_PAD * 2 + 8;
    let dialog_h = (header_h + list_h + footer_h) as u32;

    let dx = ((sw as i32 - DIALOG_W as i32) / 2).max(0);
    let dy = ((sh as i32 - dialog_h as i32) / 2).max(0);

    let dialog_flags = window::WIN_FLAG_BORDERLESS
        | window::WIN_FLAG_SHADOW
        | window::WIN_FLAG_NOT_RESIZABLE
        | window::WIN_FLAG_ALWAYS_ON_TOP
        | window::WIN_FLAG_NO_CLOSE
        | window::WIN_FLAG_NO_MINIMIZE
        | window::WIN_FLAG_NO_MAXIMIZE
        | window::WIN_FLAG_NO_MOVE;

    let win = window::create_ex(
        "", dx as u16, dy as u16, DIALOG_W as u16, dialog_h as u16, dialog_flags,
    );
    if win == u32::MAX {
        cleanup(lock_pipe, dim_win, u32::MAX, 1);
    }

    anyos_std::println!("[permdialog] dim_win={} dialog_win={}", dim_win, win);

    // ── Build UI elements ──
    let cb_x = PAD;
    let mut checkboxes: [UiCheckbox; 6] = [
        UiCheckbox::new(0, 0, true),
        UiCheckbox::new(0, 0, true),
        UiCheckbox::new(0, 0, true),
        UiCheckbox::new(0, 0, true),
        UiCheckbox::new(0, 0, true),
        UiCheckbox::new(0, 0, true),
    ];
    for i in 0..num_active {
        let y = header_h + (i as i32 * ROW_H) + 12;
        checkboxes[i] = UiCheckbox::new(cb_x, y, active_groups[i].1);
    }

    let total_btn_w = BTN_W * 2 + BTN_PAD as u32;
    let btn_base_x = (DIALOG_W as i32 - total_btn_w as i32) / 2;
    let btn_y = dialog_h as i32 - BTN_H as i32 - BTN_PAD;

    let mut deny_btn = UiButton::new(btn_base_x, btn_y, BTN_W, BTN_H, ButtonStyle::Default);
    let mut allow_btn = UiButton::new(
        btn_base_x + BTN_W as i32 + BTN_PAD, btn_y,
        BTN_W, BTN_H, ButtonStyle::Primary,
    );

    anyos_std::println!("[permdialog] entering event loop");

    // ── Event loop ──
    let mut dirty = true;
    let mut event = [0u32; 5];

    loop {
        let t0 = sys::uptime_ms();
        if dirty {
            render(
                win, DIALOG_W, dialog_h, app_name,
                &active_groups, num_active, &checkboxes,
                &deny_btn, &allow_btn,
            );
            dirty = false;
        }

        while window::get_event(win, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);
            dirty = true;

            if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                cleanup(lock_pipe, dim_win, win, 1);
            }

            for i in 0..num_active {
                if let Some(new_state) = checkboxes[i].handle_event(&ev) {
                    active_groups[i].1 = new_state;
                }
            }

            if deny_btn.handle_event(&ev) {
                cleanup(lock_pipe, dim_win, win, 1);
            }

            if allow_btn.handle_event(&ev) {
                let mut granted: u32 = 0;
                for i in 0..num_active {
                    if active_groups[i].1 {
                        granted |= PERM_GROUPS[active_groups[i].0].mask;
                    }
                }
                if permissions::perm_store(app_id, granted, 0) {
                    cleanup(lock_pipe, dim_win, win, 0);
                } else {
                    // Store failed — exit with error so spawn() doesn't retry blindly
                    cleanup(lock_pipe, dim_win, win, 2);
                }
            }
        }

        // Drain events from dim overlay (prevents event queue buildup)
        let mut dim_ev = [0u32; 5];
        while window::get_event(dim_win, &mut dim_ev) == 1 {}

        let elapsed = sys::uptime_ms().wrapping_sub(t0);
        if elapsed < 16 { process::sleep(16 - elapsed); }
    }
}

/// Parse "\x1F"-delimited args into (app_id, app_name, declared_caps).
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
