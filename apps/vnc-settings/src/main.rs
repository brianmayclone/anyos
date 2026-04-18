//! VNC Settings — anyOS GUI for configuring the vncd daemon.
//!
//! # Layout
//! ```
//! ┌─ VNC Settings ──────────────────────────────────────────┐
//! │ [Save]                                        (toolbar) │
//! │                                                         │
//! │ ┌─ VNC Server ──────────────────────────────────────┐  │
//! │ │  VNC Access     [●] on                            │  │
//! │ │  Port           [5900          ]                  │  │
//! │ │  VNC Password   [anyos         ]                  │  │
//! │ │  Allow Root     [●] on                            │  │
//! │ └───────────────────────────────────────────────────┘  │
//! │ ┌─ Allowed Users ───────────────────────────────────┐  │
//! │ │  [ + Add User… ]   [ − Remove Selected ]          │  │
//! │ │  ┌──────────────────────────────────────────────┐ │  │
//! │ │  │ alice                                        │ │  │
//! │ │  │ bob                                          │ │  │
//! │ │  └──────────────────────────────────────────────┘ │  │
//! │ └───────────────────────────────────────────────────┘  │
//! │                   [ Apply ]   [ Cancel ]               │
//! │                                             (status)   │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! Adding a user opens a small dialog window (non-modal, Finder-style):
//! ```
//! ┌─ Add Allowed User ──────────────────┐
//! │  Username: [                      ] │
//! │                 [ Cancel ]  [ OK ] │
//! └─────────────────────────────────────┘
//! ```

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{ipc, println, String, Vec, format, i18n};
use anyos_std::users;
use libconf_schema::{default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema};
use libanyui_client as ui;
use ui::ColumnDef;

// ── Constants ─────────────────────────────────────────────────────────────────

const VNCD_PIPE: &str = "vncd";
const WIN_W: u32 = 440;
const WIN_H: u32 = 520;
const VNC_SETTINGS_DIRS: &[&str] = &["config"];
const VNC_SETTINGS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/enabled", false),
    default_int("config/port", 5900),
    default_bool("config/allow_root", false),
    default_string("config/password", "anyos"),
    default_string("config/allowed_users_csv", ""),
];
const VNC_SETTINGS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const VNC_SETTINGS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/vncd",
    RegistryScope::System,
    1,
    VNC_SETTINGS_DIRS,
    VNC_SETTINGS_DEFAULTS,
    VNC_SETTINGS_MIGRATIONS,
);
const VNC_SETTINGS_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("vnc-settings", &VNC_SETTINGS_MANIFEST);

// ── Config model ──────────────────────────────────────────────────────────────

struct VncConf {
    enabled: bool,
    port: u16,
    allow_root: bool,
    password: String,
    allowed_users: Vec<String>,
}

impl VncConf {
    fn default_conf() -> Self {
        VncConf {
            enabled: false,
            port: 5900,
            allow_root: false,
            password: String::from("anyos"),
            allowed_users: Vec::new(),
        }
    }
}

// ── Config I/O ────────────────────────────────────────────────────────────────

fn load_conf() -> VncConf {
    let mut cfg = VncConf::default_conf();
    if let Some(v) = VNC_SETTINGS_SCHEMA.read_bool("config/enabled") {
        cfg.enabled = v;
    }
    if let Some(port) = VNC_SETTINGS_SCHEMA.read_i64("config/port") {
        if port > 0 && port <= u16::MAX as i64 {
            cfg.port = port as u16;
        }
    }
    if let Some(v) = VNC_SETTINGS_SCHEMA.read_bool("config/allow_root") {
        cfg.allow_root = v;
    }
    if let Some(v) = VNC_SETTINGS_SCHEMA.read_string("config/password") {
        cfg.password = v;
    }
    if let Some(v) = VNC_SETTINGS_SCHEMA.read_string("config/allowed_users_csv") {
        for user in v.split(',') {
            let u = user.trim();
            if !u.is_empty() {
                cfg.allowed_users.push(String::from(u));
            }
        }
    }
    cfg
}

fn save_conf(cfg: &VncConf) {
    let _ = VNC_SETTINGS_SCHEMA.write_bool("config/enabled", cfg.enabled);
    let _ = VNC_SETTINGS_SCHEMA.write_i64("config/port", cfg.port as i64);
    let _ = VNC_SETTINGS_SCHEMA.write_bool("config/allow_root", cfg.allow_root);
    let _ = VNC_SETTINGS_SCHEMA.write_string("config/password", &cfg.password);
    let mut users_csv = String::new();
    for (i, u) in cfg.allowed_users.iter().enumerate() {
        if i > 0 { users_csv.push(','); }
        users_csv.push_str(u);
    }
    let _ = VNC_SETTINGS_SCHEMA.write_string("config/allowed_users_csv", &users_csv);
}

/// Return `true` if `username` exists in the local user database.
fn user_exists(username: &str) -> bool {
    let mut buf = [0u8; 2048];
    let n = users::listusers(&mut buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    // Format from kernel: "uid:username\n" per line.
    let text = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
    for line in text.split('\n') {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() == 2 && parts[1].trim() == username {
            return true;
        }
    }
    false
}

// ── App state ─────────────────────────────────────────────────────────────────

struct AppState {
    cfg: VncConf,

    // ── UI handles ──
    toggle_enabled: ui::Toggle,
    toggle_root: ui::Toggle,
    port_field: ui::TextField,
    pw_field: ui::TextField,
    user_grid: ui::DataGrid,
    status_label: ui::Label,
    btn_remove: ui::Button,
}

anyos_std::global_app_state!(AppState);

// ── UI helpers ────────────────────────────────────────────────────────────────

/// Rebuild the user DataGrid from the current config.
fn refresh_user_grid() {
    let s = app();
    // Build row data and pass to set_data for a full refresh.
    let rows: Vec<Vec<&str>> = s.cfg.allowed_users.iter()
        .map(|u| { let mut v = Vec::new(); v.push(u.as_str()); v })
        .collect();
    s.user_grid.set_data(&rows);
    s.btn_remove.set_enabled(!s.cfg.allowed_users.is_empty());
}

fn read_form_into_cfg() {
    let s = app();

    s.cfg.enabled = s.toggle_enabled.get_state() != 0;
    s.cfg.allow_root = s.toggle_root.get_state() != 0;

    let mut buf = [0u8; 16];
    let n = s.port_field.get_text(&mut buf);
    if n > 0 && n != u32::MAX {
        if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
            if let Ok(p) = text.trim().parse::<u16>() {
                if p > 0 { s.cfg.port = p; }
            }
        }
    }

    let mut pw_buf = [0u8; 64];
    let pn = s.pw_field.get_text(&mut pw_buf);
    if pn > 0 && pn != u32::MAX {
        if let Ok(text) = core::str::from_utf8(&pw_buf[..pn as usize]) {
            let t = text.trim();
            if !t.is_empty() {
                s.cfg.password = String::from(t);
            }
        }
    }
}

fn apply() {
    read_form_into_cfg();
    save_conf(&app().cfg);

    // Signal running daemon to reload without restarting.
    let pipe = ipc::pipe_open(VNCD_PIPE);
    if pipe != 0 && pipe != u32::MAX {
        ipc::pipe_write(pipe, b"reload\n");
    }

    let s = app();
    let status = if s.cfg.enabled {
        format!("    {} {}", i18n::t("Saved. VNC enabled on port"), s.cfg.port)
    } else {
        format!("    {}", i18n::t("Saved. VNC access disabled."))
    };
    s.status_label.set_text(&status);
}

// ── Add-user dialog (Finder-style non-blocking property window) ───────────────

/// Open a small dialog window to add a new allowed user.
///
/// The dialog is non-blocking: it registers its button handlers and returns
/// immediately. The user interacts with it as part of the normal event loop,
/// exactly like Finder property windows.
fn show_add_user_dialog() {
    let dlg = ui::Window::new_with_flags(
        i18n::t("Add Allowed User"), -1, -1, 340, 120,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_NO_MINIMIZE | ui::WIN_FLAG_NO_MAXIMIZE,
    );

    let lbl = ui::Label::new(i18n::t("Username:"));
    lbl.set_position(12, 18);
    lbl.set_size(80, 22);
    dlg.add(&lbl);

    let input = ui::TextField::new();
    input.set_position(96, 16);
    input.set_size(228, 26);
    dlg.add(&input);

    let btn_cancel = ui::Button::new(i18n::t("Cancel"));
    btn_cancel.set_position(156, 58);
    btn_cancel.set_size(80, 28);
    dlg.add(&btn_cancel);

    let btn_ok = ui::Button::new("OK");
    btn_ok.set_position(244, 58);
    btn_ok.set_size(80, 28);
    dlg.add(&btn_ok);

    // OK: validate, add to list, close dialog.
    {
        let input_ref = input.clone();
        let dlg_ok = dlg.clone();
        btn_ok.on_click(move |_| {
            let mut buf = [0u8; 64];
            let n = input_ref.get_text(&mut buf);
            if n == 0 || n == u32::MAX {
                return;
            }
            let name = match core::str::from_utf8(&buf[..n as usize]) {
                Ok(t) => t.trim(),
                Err(_) => return,
            };
            if name.is_empty() {
                return;
            }

            dlg_ok.destroy();

            if !user_exists(name) {
                app().status_label.set_text(&format!("    {}", i18n::t("Error: user does not exist locally.")));
                return;
            }
            let s = app();
            if s.cfg.allowed_users.iter().any(|u| u.as_str() == name) {
                s.status_label.set_text(&format!("    {}", i18n::t("User already in list.")));
                return;
            }
            s.cfg.allowed_users.push(String::from(name));
            s.status_label.set_text(&format!("    {}", i18n::t("User added.")));
            refresh_user_grid();
        });
    }

    // Cancel: just close.
    {
        let dlg_cancel = dlg.clone();
        btn_cancel.on_click(move |_| {
            dlg_cancel.destroy();
        });
    }

    // Return immediately — dialog lives in the main event loop.
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    if !ui::init() {
        println!("[VNC Settings] Failed to init libanyui");
        return;
    }
    i18n::init();
    let _ = VNC_SETTINGS_SCHEMA.register();

    let cfg = load_conf();

    // ── Window ───────────────────────────────────────────────────────────────
    let win = ui::Window::new(i18n::t("VNC Settings"), -1, -1, WIN_W, WIN_H);
    let tc = ui::theme::colors();

    // ── Content scroll area ───────────────────────────────────────────────────
    let scroll = ui::ScrollView::new();
    scroll.set_dock(ui::DOCK_FILL);
    win.add(&scroll);

    // ── Status bar — added last so it is never occluded by other controls ────
    // Use a View as the coloured container so we can position the label with
    // explicit 4 px top offset (no set_padding API on Label).
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 26);
    status_bar.set_color(tc.accent);
    win.add(&status_bar);

    let status_label = ui::Label::new(&format!("    {}", i18n::t("VNC Settings")));
    status_label.set_position(0, 4);
    status_label.set_size(WIN_W, 18);
    status_label.set_color(tc.accent);
    status_label.set_text_color(0xFFFFFFFF);
    status_label.set_font_size(11);
    status_bar.add(&status_label);

    // ════════════════════════════════════════════════════════════════
    //  "VNC Server" section — 3-column TableLayout:
    //  col 0 = label (right-aligned), col 1 = 10 px spacer, col 2 = control
    //  Total inner width = WIN_W-40 = 400 px → columns [130, 10, 260]
    //  We nest a fixed-size View in col 1 to enforce the 10 px width.
    // ════════════════════════════════════════════════════════════════
    let grp_server = ui::GroupBox::new(i18n::t("VNC Server"));
    grp_server.set_position(12, 8);
    grp_server.set_size(WIN_W - 24, 185);
    scroll.add(&grp_server);

    let tl = ui::TableLayout::new(3);
    tl.set_position(8, 22);
    tl.set_size(WIN_W - 40, 155);
    tl.set_row_height(34);
    // col 0 = 130 px labels, col 1 = 10 px gap, col 2 = 260 px controls
    tl.set_column_widths(&[130, 10, WIN_W - 40 - 130 - 10]);
    grp_server.add(&tl);

    // Helper macro: add one spacer view in the middle column.
    // (A Label with no text — the layout engine sizes the cell, the view
    //  sits invisibly in between labels and controls.)
    macro_rules! spacer {
        () => {{
            let s = ui::Label::new("");
            tl.add(&s);
        }};
    }

    // Row 0: VNC Access
    let lbl_enabled = ui::Label::new(i18n::t("VNC Access"));
    lbl_enabled.set_text_align(ui::TEXT_ALIGN_RIGHT);
    tl.add(&lbl_enabled);
    spacer!();
    let toggle_enabled = ui::Toggle::new(cfg.enabled);
    tl.add(&toggle_enabled);

    // Row 1: Port
    let lbl_port = ui::Label::new(i18n::t("Port"));
    lbl_port.set_text_align(ui::TEXT_ALIGN_RIGHT);
    tl.add(&lbl_port);
    spacer!();
    let port_field = ui::TextField::new();
    port_field.set_text(&format!("{}", cfg.port));
    tl.add(&port_field);

    // Row 2: VNC Password
    let lbl_pw = ui::Label::new(i18n::t("VNC Password"));
    lbl_pw.set_text_align(ui::TEXT_ALIGN_RIGHT);
    tl.add(&lbl_pw);
    spacer!();
    let pw_field = ui::TextField::new();
    pw_field.set_text(&cfg.password);
    tl.add(&pw_field);

    // Row 3: Allow Root
    let lbl_root = ui::Label::new(i18n::t("Allow Root"));
    lbl_root.set_text_align(ui::TEXT_ALIGN_RIGHT);
    tl.add(&lbl_root);
    spacer!();
    let toggle_root = ui::Toggle::new(cfg.allow_root);
    tl.add(&toggle_root);

    // ════════════════════════════════════════════════════════════════
    //  "Allowed Users" section
    // ════════════════════════════════════════════════════════════════
    let grp_users = ui::GroupBox::new(i18n::t("Allowed Users (must exist locally)"));
    grp_users.set_position(12, 200);
    grp_users.set_size(WIN_W - 24, 234);
    scroll.add(&grp_users);

    let btn_add = ui::Button::new(i18n::t("+ Add User..."));
    btn_add.set_position(8, 24);
    btn_add.set_size(110, 26);
    grp_users.add(&btn_add);

    let btn_remove = ui::Button::new(i18n::t("Remove Selected"));
    btn_remove.set_position(126, 24);
    btn_remove.set_size(150, 26);
    btn_remove.set_enabled(!cfg.allowed_users.is_empty());
    grp_users.add(&btn_remove);

    let user_grid = ui::DataGrid::new(WIN_W - 40, 168);
    user_grid.set_position(8, 58);
    user_grid.set_columns(&[ColumnDef::new(i18n::t("Username")).width(WIN_W - 40)]);
    user_grid.set_row_height(22);
    user_grid.set_selection_mode(ui::SELECTION_SINGLE);
    grp_users.add(&user_grid);

    // ════════════════════════════════════════════════════════════════
    //  Bottom action buttons — positioned manually (8 px gap like
    //  the Add/Remove buttons above)
    // ════════════════════════════════════════════════════════════════
    let btn_cancel = ui::Button::new(i18n::t("Cancel"));
    btn_cancel.set_position(12, 442);
    btn_cancel.set_size(90, 28);
    scroll.add(&btn_cancel);

    let btn_apply = ui::Button::new(i18n::t("Apply"));
    btn_apply.set_position(12 + 90 + 8, 442);   // 8 px gap after Cancel
    btn_apply.set_size(90, 28);
    scroll.add(&btn_apply);

    // ── Initialize AppState ───────────────────────────────────────────────────
    unsafe {
        APP = Some(AppState {
            cfg,
            toggle_enabled,
            toggle_root,
            port_field,
            pw_field,
            user_grid,
            status_label,
            btn_remove,
        });
    }

    refresh_user_grid();

    // ── Event handlers ────────────────────────────────────────────────────────

    btn_apply.on_click(|_| apply());

    btn_cancel.on_click(|_| {
        anyos_std::process::exit(0);
    });

    btn_add.on_click(|_| {
        show_add_user_dialog();
    });

    btn_remove.on_click(|_| {
        let s = app();
        let sel = s.user_grid.selected_row();
        if sel != u32::MAX && (sel as usize) < s.cfg.allowed_users.len() {
            s.cfg.allowed_users.remove(sel as usize);
            s.status_label.set_text(&format!("    {}", i18n::t("User removed.")));
            refresh_user_grid();
        }
    });

    ui::run();
}
