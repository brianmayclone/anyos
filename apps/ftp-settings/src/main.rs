// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! FTP Settings — anyOS GUI for configuring the ftpd daemon.
//!
//! Reads/writes `/System/etc/ftpd/ftpd.conf` and `/System/etc/ftpd/shares.conf`,
//! then signals the running daemon via the "ftpd" named pipe so it reloads.

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{ipc, println, String, Vec, format, i18n};
use libanyui_client as ui;
use ui::ColumnDef;

// ── Constants ─────────────────────────────────────────────────────────────────

const CONF_PATH: &str = "/System/etc/ftpd/ftpd.conf";
const SHARES_PATH: &str = "/System/etc/ftpd/shares.conf";
const FTPD_PIPE: &str = "ftpd";
const WIN_W: u32 = 520;
const WIN_H: u32 = 640;

const MAX_SHARES: usize = 64;

// ── Config model ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ShareEntry {
    user: String,
    path: String,
    can_read: bool,
    can_write: bool,
}

struct FtpConf {
    enabled: bool,
    port: u16,
    passive_mode: bool,
    passive_port_min: u16,
    passive_port_max: u16,
    allow_anonymous: bool,
    anonymous_root: String,
    max_clients: u16,
    chroot_users: bool,
    shares: Vec<ShareEntry>,
}

impl FtpConf {
    fn default_conf() -> Self {
        let mut cfg = FtpConf {
            enabled: false,
            port: 21,
            passive_mode: true,
            passive_port_min: 50000,
            passive_port_max: 51000,
            allow_anonymous: false,
            anonymous_root: String::from("/users/shared/ftp"),
            max_clients: 10,
            chroot_users: true,
            shares: Vec::new(),
        };
        cfg.shares.push(ShareEntry {
            user: String::from("*"),
            path: String::from("/users/shared/ftp"),
            can_read: true,
            can_write: false,
        });
        cfg
    }
}

// ── Config I/O ────────────────────────────────────────────────────────────────

fn load_conf() -> FtpConf {
    let mut cfg = FtpConf::default_conf();

    if let Ok(content) = anyos_std::fs::read_to_string(CONF_PATH) {
        for line in content.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some(val) = line.strip_prefix("enabled=") {
                cfg.enabled = matches!(val.trim(), "yes" | "true" | "1");
            } else if let Some(val) = line.strip_prefix("port=") {
                if let Ok(p) = val.trim().parse::<u16>() { if p > 0 { cfg.port = p; } }
            } else if let Some(val) = line.strip_prefix("passive_mode=") {
                cfg.passive_mode = matches!(val.trim(), "yes" | "true" | "1");
            } else if let Some(val) = line.strip_prefix("passive_port_min=") {
                if let Ok(p) = val.trim().parse::<u16>() { cfg.passive_port_min = p; }
            } else if let Some(val) = line.strip_prefix("passive_port_max=") {
                if let Ok(p) = val.trim().parse::<u16>() { cfg.passive_port_max = p; }
            } else if let Some(val) = line.strip_prefix("allow_anonymous=") {
                cfg.allow_anonymous = matches!(val.trim(), "yes" | "true" | "1");
            } else if let Some(val) = line.strip_prefix("anonymous_root=") {
                cfg.anonymous_root = String::from(val.trim());
            } else if let Some(val) = line.strip_prefix("max_clients=") {
                if let Ok(v) = val.trim().parse::<u16>() { if v > 0 { cfg.max_clients = v; } }
            } else if let Some(val) = line.strip_prefix("chroot_users=") {
                cfg.chroot_users = matches!(val.trim(), "yes" | "true" | "1");
            }
        }
    }

    cfg.shares.clear();
    if let Ok(content) = anyos_std::fs::read_to_string(SHARES_PATH) {
        for line in content.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let mut parts = line.splitn(3, ':');
            let user = parts.next().unwrap_or("").trim();
            let path = parts.next().unwrap_or("").trim();
            let perms = parts.next().unwrap_or("").trim();
            if user.is_empty() || path.is_empty() { continue; }
            cfg.shares.push(ShareEntry {
                user: String::from(user),
                path: String::from(path),
                can_read: perms.contains('r') || perms.contains('R'),
                can_write: perms.contains('w') || perms.contains('W'),
            });
        }
    }

    cfg
}

fn save_conf(cfg: &FtpConf) -> bool {
    let _ = anyos_std::fs::mkdir("/System/etc/ftpd");

    let mut main_out = String::new();
    main_out.push_str("# anyOS FTP Server Configuration\n");
    main_out.push_str(if cfg.enabled { "enabled=yes\n" } else { "enabled=no\n" });
    main_out.push_str(&format!("port={}\n", cfg.port));
    main_out.push_str(if cfg.passive_mode { "passive_mode=yes\n" } else { "passive_mode=no\n" });
    main_out.push_str(&format!("passive_port_min={}\n", cfg.passive_port_min));
    main_out.push_str(&format!("passive_port_max={}\n", cfg.passive_port_max));
    main_out.push_str(if cfg.allow_anonymous { "allow_anonymous=yes\n" } else { "allow_anonymous=no\n" });
    main_out.push_str(&format!("anonymous_root={}\n", cfg.anonymous_root));
    main_out.push_str(&format!("max_clients={}\n", cfg.max_clients));
    main_out.push_str(if cfg.chroot_users { "chroot_users=yes\n" } else { "chroot_users=no\n" });

    let ok_main = anyos_std::fs::write_bytes(CONF_PATH, main_out.as_bytes()).is_ok();

    let mut shares_out = String::new();
    shares_out.push_str("# anyOS FTP Shares Configuration\n");
    shares_out.push_str("# Format: username:path:permissions  (r=read, w=write, *=all users)\n");
    for share in &cfg.shares {
        shares_out.push_str(&share.user);
        shares_out.push(':');
        shares_out.push_str(&share.path);
        shares_out.push(':');
        if share.can_read { shares_out.push('r'); }
        if share.can_write { shares_out.push('w'); }
        shares_out.push('\n');
    }

    let ok_shares = anyos_std::fs::write_bytes(SHARES_PATH, shares_out.as_bytes()).is_ok();
    ok_main && ok_shares
}

// ── App state ─────────────────────────────────────────────────────────────────

struct AppState {
    cfg: FtpConf,
    toggle_enabled: ui::Toggle,
    port_field: ui::TextField,
    toggle_passive: ui::Toggle,
    pasv_min_field: ui::TextField,
    pasv_max_field: ui::TextField,
    toggle_anon: ui::Toggle,
    anon_root_field: ui::TextField,
    max_clients_field: ui::TextField,
    toggle_chroot: ui::Toggle,
    shares_grid: ui::DataGrid,
    btn_edit_share: ui::Button,
    btn_remove_share: ui::Button,
    status_label: ui::Label,
}

anyos_std::global_app_state!(AppState);

// ── Helpers ───────────────────────────────────────────────────────────────────

fn perm_str(can_read: bool, can_write: bool) -> &'static str {
    match (can_read, can_write) {
        (true, true)  => "rw",
        (true, false) => "r",
        (false, true) => "w",
        _             => "-",
    }
}

fn refresh_shares_grid() {
    let s = app();
    let rows: Vec<Vec<&str>> = s.cfg.shares.iter()
        .map(|sh| {
            let mut v: Vec<&str> = Vec::new();
            v.push(sh.user.as_str());
            v.push(sh.path.as_str());
            v.push(perm_str(sh.can_read, sh.can_write));
            v
        })
        .collect();
    s.shares_grid.set_data(&rows);
    let has = !s.cfg.shares.is_empty();
    s.btn_edit_share.set_enabled(has);
    s.btn_remove_share.set_enabled(has);
}

fn read_form_into_cfg() {
    let s = app();
    s.cfg.enabled       = s.toggle_enabled.get_state() != 0;
    s.cfg.passive_mode  = s.toggle_passive.get_state() != 0;
    s.cfg.allow_anonymous = s.toggle_anon.get_state() != 0;
    s.cfg.chroot_users  = s.toggle_chroot.get_state() != 0;

    let mut buf = [0u8; 16];

    let n = s.port_field.get_text(&mut buf);
    if n > 0 && n != u32::MAX {
        if let Ok(t) = core::str::from_utf8(&buf[..n as usize]) {
            if let Ok(p) = t.trim().parse::<u16>() { if p > 0 { s.cfg.port = p; } }
        }
    }

    let n = s.pasv_min_field.get_text(&mut buf);
    if n > 0 && n != u32::MAX {
        if let Ok(t) = core::str::from_utf8(&buf[..n as usize]) {
            if let Ok(p) = t.trim().parse::<u16>() { s.cfg.passive_port_min = p; }
        }
    }

    let n = s.pasv_max_field.get_text(&mut buf);
    if n > 0 && n != u32::MAX {
        if let Ok(t) = core::str::from_utf8(&buf[..n as usize]) {
            if let Ok(p) = t.trim().parse::<u16>() { s.cfg.passive_port_max = p; }
        }
    }

    let n = s.max_clients_field.get_text(&mut buf);
    if n > 0 && n != u32::MAX {
        if let Ok(t) = core::str::from_utf8(&buf[..n as usize]) {
            if let Ok(p) = t.trim().parse::<u16>() { if p > 0 { s.cfg.max_clients = p; } }
        }
    }

    let mut path_buf = [0u8; 256];
    let n = s.anon_root_field.get_text(&mut path_buf);
    if n > 0 && n != u32::MAX {
        if let Ok(t) = core::str::from_utf8(&path_buf[..n as usize]) {
            let t = t.trim();
            if !t.is_empty() { s.cfg.anonymous_root = String::from(t); }
        }
    }
}

fn apply() {
    read_form_into_cfg();
    let ok = save_conf(&app().cfg);

    let pipe = ipc::pipe_open(FTPD_PIPE);
    if pipe != 0 && pipe != u32::MAX {
        ipc::pipe_write(pipe, b"reload\n");
    }

    let s = app();
    if ok {
        let status = if s.cfg.enabled {
            format!("    {} {}", i18n::t("Saved. FTP server enabled on port"), s.cfg.port)
        } else {
            format!("    {}", i18n::t("Saved. FTP server disabled."))
        };
        s.status_label.set_text(&status);
    } else {
        s.status_label.set_text(&format!("    {}", i18n::t("Error: could not save config.")));
    }
}

// ── Add/Edit Share Dialog ─────────────────────────────────────────────────────

/// edit_idx: None = add new share, Some(i) = edit share at index i
fn show_share_dialog(edit_idx: Option<usize>) {
    let title = if edit_idx.is_some() { i18n::t("Edit Share") } else { i18n::t("Add Share") };
    let dlg = ui::Window::new_with_flags(
        title, -1, -1, 400, 180,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE,
    );

    // Pre-fill values from existing share when editing
    let (init_user, init_path, init_read, init_write) = if let Some(idx) = edit_idx {
        let s = app();
        if idx < s.cfg.shares.len() {
            let sh = &s.cfg.shares[idx];
            (sh.user.clone(), sh.path.clone(), sh.can_read, sh.can_write)
        } else {
            (String::from("*"), String::new(), true, false)
        }
    } else {
        (String::from("*"), String::new(), true, false)
    };

    let lbl_user = ui::Label::new(i18n::t("User (* = all):"));
    lbl_user.set_position(12, 16);
    lbl_user.set_size(110, 22);
    dlg.add(&lbl_user);

    let user_field = ui::TextField::new();
    user_field.set_position(126, 14);
    user_field.set_size(258, 26);
    user_field.set_text(&init_user);
    dlg.add(&user_field);

    let lbl_path = ui::Label::new(i18n::t("Path:"));
    lbl_path.set_position(12, 50);
    lbl_path.set_size(110, 22);
    dlg.add(&lbl_path);

    let path_field = ui::TextField::new();
    path_field.set_position(126, 48);
    path_field.set_size(258, 26);
    path_field.set_text(&init_path);
    dlg.add(&path_field);

    let lbl_perms = ui::Label::new(i18n::t("Access:"));
    lbl_perms.set_position(12, 86);
    lbl_perms.set_size(110, 22);
    dlg.add(&lbl_perms);

    let chk_read = ui::Checkbox::new(i18n::t("Read"));
    chk_read.set_position(126, 84);
    chk_read.set_size(80, 22);
    chk_read.set_state(if init_read { 1 } else { 0 });
    dlg.add(&chk_read);

    let chk_write = ui::Checkbox::new(i18n::t("Write"));
    chk_write.set_position(214, 84);
    chk_write.set_size(80, 22);
    chk_write.set_state(if init_write { 1 } else { 0 });
    dlg.add(&chk_write);

    let btn_cancel = ui::Button::new(i18n::t("Cancel"));
    btn_cancel.set_position(196, 134);
    btn_cancel.set_size(88, 28);
    dlg.add(&btn_cancel);

    let btn_ok = ui::Button::new("OK");
    btn_ok.set_position(292, 134);
    btn_ok.set_size(88, 28);
    dlg.add(&btn_ok);

    {
        let user_ref = user_field.clone();
        let path_ref = path_field.clone();
        let chk_r = chk_read.clone();
        let chk_w = chk_write.clone();
        let dlg_ok = dlg.clone();
        btn_ok.on_click(move |_| {
            let mut ubuf = [0u8; 64];
            let un = user_ref.get_text(&mut ubuf);
            let mut pbuf = [0u8; 256];
            let pn = path_ref.get_text(&mut pbuf);

            if un == 0 || un == u32::MAX || pn == 0 || pn == u32::MAX { return; }
            let user_str = match core::str::from_utf8(&ubuf[..un as usize]) {
                Ok(s) => s.trim(), Err(_) => return,
            };
            let path_str = match core::str::from_utf8(&pbuf[..pn as usize]) {
                Ok(s) => s.trim(), Err(_) => return,
            };
            if user_str.is_empty() || path_str.is_empty() { return; }

            // Read checkbox states BEFORE destroying the dialog
            let can_read  = chk_r.get_state() != 0;
            let can_write = chk_w.get_state() != 0;

            let user_owned = String::from(user_str);
            let path_owned = String::from(path_str);

            dlg_ok.destroy();

            let s = app();
            if let Some(idx) = edit_idx {
                // Edit existing share
                if idx < s.cfg.shares.len() {
                    s.cfg.shares[idx] = ShareEntry {
                        user: user_owned,
                        path: path_owned,
                        can_read,
                        can_write,
                    };
                    s.status_label.set_text(&format!("    {}", i18n::t("Share updated.")));
                }
            } else {
                // Add new share
                if s.cfg.shares.len() >= MAX_SHARES {
                    s.status_label.set_text(&format!("    {}", i18n::t("Error: maximum number of shares reached.")));
                    return;
                }
                s.cfg.shares.push(ShareEntry {
                    user: user_owned,
                    path: path_owned,
                    can_read,
                    can_write,
                });
                s.status_label.set_text(&format!("    {}", i18n::t("Share added.")));
            }
            refresh_shares_grid();
        });
    }

    {
        let dlg_cancel = dlg.clone();
        btn_cancel.on_click(move |_| { dlg_cancel.destroy(); });
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        println!("[FTP Settings] Failed to init libanyui");
        return;
    }
    i18n::init();

    let cfg = load_conf();

    let win = ui::Window::new(i18n::t("FTP Settings"), -1, -1, WIN_W, WIN_H);
    let tc = ui::theme::colors();

    // ── Status bar (DOCK_BOTTOM, add before DOCK_FILL) ────────────────────────
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 26);
    status_bar.set_color(tc.accent);
    win.add(&status_bar);

    let status_label = ui::Label::new(&format!("    {}", i18n::t("FTP Settings")));
    status_label.set_position(0, 4);
    status_label.set_size(WIN_W, 18);
    status_label.set_color(tc.accent);
    status_label.set_text_color(0xFFFFFFFF);
    status_label.set_font_size(11);
    status_bar.add(&status_label);

    // ── Scroll area (DOCK_FILL, add last) ─────────────────────────────────────
    let scroll = ui::ScrollView::new();
    scroll.set_dock(ui::DOCK_FILL);
    win.add(&scroll);

    // ── Outer StackPanel — stacks all sections vertically ─────────────────────
    // Width = WIN_W, height = tall enough for all content.
    let stack = ui::StackPanel::vertical();
    stack.set_position(0, 0);
    stack.set_size(WIN_W, 900);
    stack.set_padding(12, 8, 12, 8);
    scroll.add(&stack);

    // ── Sizes ─────────────────────────────────────────────────────────────────
    // Inner width available inside the stack (WIN_W - left_pad - right_pad)
    let inner_w = WIN_W - 24;
    let col_lbl: u32 = 150;
    let col_gap: u32 = 10;
    let col_ctrl = inner_w - 24 - col_lbl - col_gap; // 24 = GroupBox inner padding

    // ── FTP Server GroupBox ───────────────────────────────────────────────────
    // 9 rows × 34 px + title 24 px + padding 8 px = 338 px
    let grp_server_h: u32 = 24 + 8 + 9 * 34;
    let grp_server = ui::GroupBox::new(i18n::t("FTP Server"));
    grp_server.set_size(inner_w, grp_server_h);
    grp_server.set_margin(0, 0, 0, 10);
    stack.add(&grp_server);

    let tl = ui::TableLayout::new(3);
    tl.set_position(8, 24);
    tl.set_size(inner_w - 16, grp_server_h - 32);
    tl.set_row_height(34);
    tl.set_column_widths(&[col_lbl, col_gap, col_ctrl]);
    grp_server.add(&tl);

    macro_rules! lbl_right {
        ($text:expr) => {{
            let l = ui::Label::new($text);
            l.set_text_align(ui::TEXT_ALIGN_RIGHT);
            tl.add(&l);
        }};
    }
    macro_rules! spacer {
        () => {{ tl.add(&ui::Label::new("")); }};
    }

    // Row 0: FTP Server enabled
    lbl_right!(i18n::t("FTP Server"));
    spacer!();
    let toggle_enabled = ui::Toggle::new(cfg.enabled);
    tl.add(&toggle_enabled);

    // Row 1: Port
    lbl_right!(i18n::t("Port"));
    spacer!();
    let port_field = ui::TextField::new();
    port_field.set_text(&format!("{}", cfg.port));
    tl.add(&port_field);

    // Row 2: Passive Mode
    lbl_right!(i18n::t("Passive Mode"));
    spacer!();
    let toggle_passive = ui::Toggle::new(cfg.passive_mode);
    tl.add(&toggle_passive);

    // Row 3: Passive Port Min
    lbl_right!(i18n::t("Passive Port Min"));
    spacer!();
    let pasv_min_field = ui::TextField::new();
    pasv_min_field.set_text(&format!("{}", cfg.passive_port_min));
    tl.add(&pasv_min_field);

    // Row 4: Passive Port Max
    lbl_right!(i18n::t("Passive Port Max"));
    spacer!();
    let pasv_max_field = ui::TextField::new();
    pasv_max_field.set_text(&format!("{}", cfg.passive_port_max));
    tl.add(&pasv_max_field);

    // Row 5: Allow Anonymous
    lbl_right!(i18n::t("Allow Anonymous"));
    spacer!();
    let toggle_anon = ui::Toggle::new(cfg.allow_anonymous);
    tl.add(&toggle_anon);

    // Row 6: Anonymous Root
    lbl_right!(i18n::t("Anonymous Root"));
    spacer!();
    let anon_root_field = ui::TextField::new();
    anon_root_field.set_text(&cfg.anonymous_root);
    tl.add(&anon_root_field);

    // Row 7: Max Clients
    lbl_right!(i18n::t("Max Clients"));
    spacer!();
    let max_clients_field = ui::TextField::new();
    max_clients_field.set_text(&format!("{}", cfg.max_clients));
    tl.add(&max_clients_field);

    // Row 8: Chroot / Restrict to Share
    lbl_right!(i18n::t("Restrict to Share"));
    spacer!();
    let toggle_chroot = ui::Toggle::new(cfg.chroot_users);
    tl.add(&toggle_chroot);

    // ── Shared Directories GroupBox ───────────────────────────────────────────
    // toolbar row 30 px + grid 180 px + title 24 px + padding 8 px = 242 px
    let grp_shares_h: u32 = 24 + 8 + 36 + 180;
    let grp_shares = ui::GroupBox::new(i18n::t("Shared Directories"));
    grp_shares.set_size(inner_w, grp_shares_h);
    grp_shares.set_margin(0, 0, 0, 10);
    stack.add(&grp_shares);

    let btn_add_share = ui::Button::new(i18n::t("+ Add Share"));
    btn_add_share.set_position(8, 28);
    btn_add_share.set_size(110, 26);
    grp_shares.add(&btn_add_share);

    let btn_edit_share = ui::Button::new(i18n::t("Edit Selected"));
    btn_edit_share.set_position(126, 28);
    btn_edit_share.set_size(120, 26);
    btn_edit_share.set_enabled(!cfg.shares.is_empty());
    grp_shares.add(&btn_edit_share);

    let btn_remove_share = ui::Button::new(i18n::t("Remove Selected"));
    btn_remove_share.set_position(254, 28);
    btn_remove_share.set_size(140, 26);
    btn_remove_share.set_enabled(!cfg.shares.is_empty());
    grp_shares.add(&btn_remove_share);

    let col_user_w = 110u32;
    let col_path_w = inner_w - 16 - col_user_w - 64;
    let shares_grid = ui::DataGrid::new(inner_w - 16, 178);
    shares_grid.set_position(8, 62);
    shares_grid.set_columns(&[
        ColumnDef::new(i18n::t("User")).width(col_user_w),
        ColumnDef::new(i18n::t("Path")).width(col_path_w),
        ColumnDef::new(i18n::t("Perms")).width(60),
    ]);
    shares_grid.set_row_height(22);
    shares_grid.set_selection_mode(ui::SELECTION_SINGLE);
    grp_shares.add(&shares_grid);

    // ── Bottom action buttons ─────────────────────────────────────────────────
    let btn_row = ui::StackPanel::horizontal();
    btn_row.set_size(inner_w, 36);
    btn_row.set_padding(0, 4, 0, 4);
    btn_row.set_margin(0, 0, 0, 4);
    stack.add(&btn_row);

    let btn_apply = ui::Button::new(i18n::t("Apply"));
    btn_apply.set_size(90, 28);
    btn_row.add(&btn_apply);

    let btn_gap = ui::Label::new("");
    btn_gap.set_size(8, 28);
    btn_row.add(&btn_gap);

    let btn_cancel = ui::Button::new(i18n::t("Cancel"));
    btn_cancel.set_size(90, 28);
    btn_row.add(&btn_cancel);

    // ── Initialize AppState ───────────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            cfg,
            toggle_enabled,
            port_field,
            toggle_passive,
            pasv_min_field,
            pasv_max_field,
            toggle_anon,
            anon_root_field,
            max_clients_field,
            toggle_chroot,
            shares_grid,
            btn_edit_share,
            btn_remove_share,
            status_label,
        });
    }

    refresh_shares_grid();

    // ── Event handlers ────────────────────────────────────────────────────────
    btn_apply.on_click(|_| apply());

    btn_cancel.on_click(|_| {
        anyos_std::process::exit(0);
    });

    btn_add_share.on_click(|_| {
        show_share_dialog(None);
    });

    btn_edit_share.on_click(|_| {
        let sel = app().shares_grid.selected_row();
        if sel != u32::MAX && (sel as usize) < app().cfg.shares.len() {
            show_share_dialog(Some(sel as usize));
        }
    });

    // Doppelklick oder Enter auf Grid-Zeile öffnet Edit-Dialog
    app().shares_grid.on_submit(|e| {
        let idx = e.index as usize;
        if idx < app().cfg.shares.len() {
            show_share_dialog(Some(idx));
        }
    });

    btn_remove_share.on_click(|_| {
        let s = app();
        let sel = s.shares_grid.selected_row();
        if sel != u32::MAX && (sel as usize) < s.cfg.shares.len() {
            s.cfg.shares.remove(sel as usize);
            s.status_label.set_text(&format!("    {}", i18n::t("Share removed.")));
            refresh_shares_grid();
        }
    });

    ui::run();
}
