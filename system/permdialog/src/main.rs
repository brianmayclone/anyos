#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::ipc;
use anyos_std::ui::window;
use anyos_std::process;
use anyos_std::permissions;
use uisys_client::*;

// ── Capability bit definitions (must match kernel) ──────────────────────────

const CAP_FILESYSTEM: u32  = 1 << 0;
const CAP_NETWORK: u32     = 1 << 1;
const CAP_AUDIO: u32       = 1 << 2;
const CAP_DISPLAY: u32     = 1 << 3;
const CAP_DEVICE: u32      = 1 << 4;
const CAP_PROCESS: u32     = 1 << 5;
const CAP_SYSTEM: u32      = 1 << 6;
const CAP_COMPOSITOR: u32  = 1 << 12;

// ── Permission groups (user-friendly, grouped) ─────────────────────────────

struct PermGroup {
    mask: u32,
    name: &'static str,
    desc: &'static str,
}

const PERM_GROUPS: &[PermGroup] = &[
    PermGroup {
        mask: CAP_FILESYSTEM,
        name: "Files & Storage",
        desc: "Read and write your files",
    },
    PermGroup {
        mask: CAP_NETWORK,
        name: "Internet & Network",
        desc: "Send and receive data",
    },
    PermGroup {
        mask: CAP_AUDIO,
        name: "Audio",
        desc: "Play sounds and music",
    },
    PermGroup {
        mask: CAP_DISPLAY | CAP_COMPOSITOR,
        name: "Display",
        desc: "Control display and windows",
    },
    PermGroup {
        mask: CAP_DEVICE,
        name: "Devices",
        desc: "Access hardware devices",
    },
    PermGroup {
        mask: CAP_PROCESS | CAP_SYSTEM,
        name: "System",
        desc: "Manage processes and settings",
    },
];

// ── Layout constants ────────────────────────────────────────────────────────

const DIALOG_W: u32 = 380;
const PAD: i32 = 24;
const ROW_H: i32 = 48;
const BTN_W: u32 = 130;
const BTN_H: u32 = 34;
const BTN_PAD: i32 = 12;

/// Named pipe used as a singleton lock.
const LOCK_PIPE: &str = "_permdialog_lock";

fn main() {
    anyos_std::println!("[permdialog] start");

    // ── Singleton check: only one PermissionDialog at a time ────────────
    let existing = ipc::pipe_open(LOCK_PIPE);
    if existing != 0 {
        // Another instance is already running — exit immediately
        process::exit(1);
    }
    let lock_pipe = ipc::pipe_create(LOCK_PIPE);
    if lock_pipe == 0 || lock_pipe == u32::MAX {
        process::exit(1);
    }

    // ── Parse args: "app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path" ────
    // Use getargs() directly — the args are structured data with \x1F separators,
    // NOT traditional "program_name arg1 arg2" format, so args() would fail to
    // find a space and return "".
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

    // Split by \x1F separator
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
        cleanup(lock_pipe, u32::MAX, u32::MAX, 1);
    }

    let app_id = &args_str[offsets[0].0..offsets[0].1];
    let app_name = &args_str[offsets[1].0..offsets[1].1];
    let caps_hex = &args_str[offsets[2].0..offsets[2].1];
    let declared_caps = parse_hex(caps_hex);

    // Determine which groups are relevant (app actually requests them)
    let mut active_groups: [(usize, bool); 6] = [(0, false); 6];
    let mut num_active = 0usize;
    for (i, group) in PERM_GROUPS.iter().enumerate() {
        if declared_caps & group.mask != 0 {
            active_groups[num_active] = (i, true);
            num_active += 1;
        }
    }

    anyos_std::println!("[permdialog] app_id='{}' app_name='{}' caps=0x{} num_active={}", app_id, app_name, caps_hex, num_active);

    if num_active == 0 {
        permissions::perm_store(app_id, 0, 0);
        cleanup(lock_pipe, u32::MAX, u32::MAX, 0);
    }

    // ── Create fullscreen dim overlay ───────────────────────────────────
    anyos_std::println!("[permdialog] creating windows");
    let (sw, sh) = window::screen_size();

    // Borderless + SHADOW makes the layer non-opaque → alpha blending works
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
    // Fill with semi-transparent black (alpha ~50%)
    window::fill_rect(dim_win, 0, 0, sw as u16, sh as u16, 0x80000000);
    window::present(dim_win);

    // ── Create the actual dialog ────────────────────────────────────────
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

    let win = window::create_ex("", dx as u16, dy as u16,
        DIALOG_W as u16, dialog_h as u16, dialog_flags);
    if win == u32::MAX {
        cleanup(lock_pipe, dim_win, u32::MAX, 1);
    }

    anyos_std::println!("[permdialog] dim_win={} dialog_win={}", dim_win, win);

    // ── Build UI elements ───────────────────────────────────────────────
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
    let mut allow_btn = UiButton::new(btn_base_x + BTN_W as i32 + BTN_PAD, btn_y,
        BTN_W, BTN_H, ButtonStyle::Primary);

    anyos_std::println!("[permdialog] entering event loop");

    // ── Event loop ──────────────────────────────────────────────────────
    let mut dirty = true;
    let mut event = [0u32; 5];

    loop {
        if dirty {
            render(win, DIALOG_W, dialog_h, app_name,
                &active_groups, num_active, &checkboxes,
                &deny_btn, &allow_btn);
            dirty = false;
        }

        if window::get_event(win, &mut event) == 1 {
            let ev = UiEvent::from_raw(&event);
            dirty = true;

            // ESC = cancel (no file stored, dialog will show again next time)
            if ev.is_key_down() && ev.key_code() == KEY_ESCAPE {
                cleanup(lock_pipe, dim_win, win, 1);
            }

            // Handle checkboxes
            for i in 0..num_active {
                if let Some(new_state) = checkboxes[i].handle_event(&ev) {
                    active_groups[i].1 = new_state;
                }
            }

            // Don't Allow — no file stored, app won't start, dialog appears again next time
            if deny_btn.handle_event(&ev) {
                cleanup(lock_pipe, dim_win, win, 1);
            }

            // Allow Selected
            if allow_btn.handle_event(&ev) {
                let mut granted: u32 = 0;
                for i in 0..num_active {
                    if active_groups[i].1 {
                        granted |= PERM_GROUPS[active_groups[i].0].mask;
                    }
                }
                permissions::perm_store(app_id, granted, 0);
                cleanup(lock_pipe, dim_win, win, 0);
            }
        }

        // Also drain events from dim overlay (prevents event queue buildup)
        let mut dim_ev = [0u32; 5];
        while window::get_event(dim_win, &mut dim_ev) == 1 {}

        process::sleep(16);
    }
}

/// Destroy windows, release lock pipe, and exit.
fn cleanup(lock_pipe: u32, dim_win: u32, win: u32, code: u32) -> ! {
    if win != u32::MAX {
        window::destroy(win);
    }
    if dim_win != u32::MAX {
        window::destroy(dim_win);
    }
    if lock_pipe != 0 && lock_pipe != u32::MAX {
        ipc::pipe_close(lock_pipe);
    }
    process::exit(code);
}

fn render(
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
    anyos_std::println!("[permdialog] render: getting colors");
    let bg = colors::WINDOW_BG();
    let text_color = colors::TEXT();
    let secondary = colors::TEXT_SECONDARY();

    // Background
    anyos_std::println!("[permdialog] render: fill_rect bg");
    window::fill_rect(win, 0, 0, w as u16, h as u16, bg);

    // Title: "AppName" wants access to:
    let mut title_buf = [0u8; 128];
    let title_len = format_title(app_name, &mut title_buf);
    let title = core::str::from_utf8(&title_buf[..title_len]).unwrap_or("App wants access to:");

    anyos_std::println!("[permdialog] render: calling label()");
    label(win, PAD, 20, title, text_color, FontSize::Normal, TextAlign::Left);
    label(win, PAD, 46, "This app requests the following permissions:",
        secondary, FontSize::Small, TextAlign::Left);

    // Divider below header
    divider_h(win, PAD, 74, w - PAD as u32 * 2);

    // Permission rows
    let header_h: i32 = 80;
    for i in 0..num_active {
        let group = &PERM_GROUPS[active_groups[i].0];
        let y = header_h + (i as i32 * ROW_H);

        checkboxes[i].render(win, group.name);
        label(win, PAD + 30, y + 28, group.desc, secondary,
            FontSize::Small, TextAlign::Left);

        if i + 1 < num_active {
            divider_h(win, PAD, y + ROW_H - 2, w - PAD as u32 * 2);
        }
    }

    // Divider above buttons
    let btn_div_y = h as i32 - BTN_H as i32 - BTN_PAD * 2 - 12;
    divider_h(win, PAD, btn_div_y, w - PAD as u32 * 2);

    // Buttons
    deny_btn.render(win, "Don't Allow");
    allow_btn.render(win, "Allow");

    window::present(win);
}

fn format_title(app_name: &str, buf: &mut [u8]) -> usize {
    let mut pos = 0usize;
    let max = buf.len() - 1;

    // Opening double quote (UTF-8: E2 80 9C)
    if pos + 2 < max { buf[pos] = 0xE2; buf[pos+1] = 0x80; buf[pos+2] = 0x9C; pos += 3; }

    for &b in app_name.as_bytes() {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    // Closing double quote (UTF-8: E2 80 9D)
    if pos + 2 < max { buf[pos] = 0xE2; buf[pos+1] = 0x80; buf[pos+2] = 0x9D; pos += 3; }

    for &b in b" wants access to:" {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    pos
}

fn parse_hex(s: &str) -> u32 {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => continue,
        };
        val = val.wrapping_shl(4) | digit as u32;
    }
    val
}
