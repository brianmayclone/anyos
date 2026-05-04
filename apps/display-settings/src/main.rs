//! display-settings — Multi-monitor configuration GUI for anyOS.
//!
//! GNOME-style two-pane layout:
//!
//!   * Top toolbar: title + Erweitern/Spiegeln segmented control
//!     (writes mirror_mode through CMD_SET_GLOBAL_CONFIG).
//!   * Layout preview: schematic showing the relative positions of all
//!     active outputs (read-only for the first cut — drag-arrange is
//!     a follow-up).
//!   * Output list: one row per advertised output, click to drill into
//!     detail.
//!   * Detail panel: enabled toggle, orientation combo, resolution
//!     combo, refresh-rate label, scale segmented (100% / 200%),
//!     fractional-scale toggle. "Apply" pushes a CMD_SET_OUTPUT_CONFIG
//!     into displayd, which persists in confd and re-runs the layout.
//!   * Profiles bar: list of saved monitor setups + Save/Load buttons
//!     so the user can keep "home / office / mobile" presets.

#![no_std]
#![no_main]

use anyos_std::{display, format, String, Vec};
use libanyui_client as ui;
use libanyui_client::Widget;
use libdisplay_client as displayd;

anyos_std::entry!(main);

const WIN_W: u32 = 760;
const WIN_H: u32 = 600;

const COL_BG: u32 = 0xFF1E1E1E;
const COL_PANEL: u32 = 0xFF2A2A2C;
const COL_PANEL_ALT: u32 = 0xFF26262A;
const COL_TEXT: u32 = 0xFFE6E6E6;
const COL_TEXT_DIM: u32 = 0xFF9A9A9A;

struct AppState {
    infos: Vec<display::DisplayInfo>,
    selected_idx: Option<usize>,
    res_options: Vec<(u32, u32)>,
    profile_label: Option<ui::Label>,
}

static mut APP: Option<AppState> = None;
fn app() -> &'static mut AppState {
    unsafe { APP.as_mut().unwrap() }
}

fn isqrt_u64(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

fn pnpid(info: &display::DisplayInfo) -> String {
    let bytes = info.manufacturer.to_le_bytes();
    let mut s = String::new();
    for &b in &bytes[..3] {
        if b.is_ascii_alphabetic() || b.is_ascii_digit() {
            s.push(b as char);
        }
    }
    if s.is_empty() {
        s.push_str("???");
    }
    s
}

fn friendly_name(info: &display::DisplayInfo) -> String {
    let m = pnpid(info);
    if info.is_primary() || info.id == 0 {
        format!("{} (Eingebaute Anzeige)", m)
    } else {
        format!("{} {}\u{0022}", m, diagonal_inches(info))
    }
}

fn diagonal_inches(info: &display::DisplayInfo) -> u32 {
    let (w, h) = info.physical_mm_pair();
    if w == 0 && h == 0 {
        return 0;
    }
    let d_mm = isqrt_u64((w as u64) * (w as u64) + (h as u64) * (h as u64));
    ((d_mm * 100 / 254 + 5) / 10) as u32
}

/// Build the candidate resolution list for the resolution combo. Hard-
/// coded standard list filtered to the output's preferred mode and
/// below — same approach the kernel's COMMON_MODES uses.
fn resolution_options(info: &display::DisplayInfo) -> Vec<(u32, u32)> {
    let cap_w = if info.preferred_w > 0 {
        info.preferred_w
    } else {
        u32::MAX
    };
    let cap_h = if info.preferred_h > 0 {
        info.preferred_h
    } else {
        u32::MAX
    };
    let mut out: Vec<(u32, u32)> = Vec::new();
    let common: &[(u32, u32)] = &[
        (640, 480),
        (800, 600),
        (1024, 768),
        (1280, 720),
        (1280, 800),
        (1280, 1024),
        (1440, 900),
        (1600, 900),
        (1600, 1200),
        (1680, 1050),
        (1920, 1080),
        (1920, 1200),
        (2560, 1440),
        (3440, 1440),
        (3840, 2160),
    ];
    for &(w, h) in common {
        if w <= cap_w && h <= cap_h {
            out.push((w, h));
        }
    }
    if cap_w != u32::MAX && cap_h != u32::MAX && !out.iter().any(|&(w, h)| w == cap_w && h == cap_h)
    {
        out.push((cap_w, cap_h));
    }
    out
}

fn build_combo_items(opts: &[(u32, u32)]) -> String {
    let mut s = String::new();
    for (i, (w, h)) in opts.iter().enumerate() {
        if i > 0 {
            s.push('|');
        }
        s.push_str(&format!("{} \u{00D7} {}", w, h));
    }
    s
}

fn build_orientation_items() -> &'static str {
    "Querformat|Hochformat|Querformat (gespiegelt)|Hochformat (gespiegelt)"
}

fn select_combo_index(opts: &[(u32, u32)], target: (u32, u32)) -> Option<u32> {
    opts.iter()
        .position(|&p| p == target)
        .map(|i| i as u32)
}

fn refresh_outputs() {
    app().infos = display::list(16);
}

fn primary_id() -> u32 {
    app().infos
        .iter()
        .find(|i| i.is_primary())
        .map(|i| i.id)
        .unwrap_or(0)
}

/// Render a small textual layout preview into the supplied label.
/// A Canvas-based drag-arrange comes with phase 9c-extended; for the
/// first cut we show a one-liner like
/// `[1] 1920×1080 Eingebaute · [2] 2560×1440 Eizo*`.
fn render_layout_label(label: &ui::Label) {
    let mut s = String::new();
    for (i, info) in app().infos.iter().enumerate() {
        if !info.is_connected() {
            continue;
        }
        let (w, h) = if info.current_w > 0 {
            (info.current_w, info.current_h)
        } else {
            (info.preferred_w, info.preferred_h)
        };
        if w == 0 || h == 0 {
            continue;
        }
        if !s.is_empty() {
            s.push_str("    ");
        }
        let primary_marker = if info.is_primary() || info.id == 0 {
            "*"
        } else {
            ""
        };
        s.push_str(&format!(
            "[{}] {}\u{00D7}{} {}{}",
            i + 1,
            w,
            h,
            pnpid(info),
            primary_marker
        ));
    }
    if s.is_empty() {
        s.push_str("Keine angeschlossenen Bildschirme.");
    }
    label.set_text(&s);
}

fn commit_output(idx: usize, cfg: displayd::OutputConfig) {
    let _ = idx;
    if let Some(client) = displayd::DisplaydClient::connect() {
        let _ = client.set_output_config(&cfg);
        client.disconnect();
    }
    refresh_outputs();
}

fn save_profile_with_name(name: &str) {
    if let Some(client) = displayd::DisplaydClient::connect() {
        let _ = client.save_profile(name);
        client.disconnect();
        if let Some(lbl) = app().profile_label.as_ref() {
            lbl.set_text(&format!("Profil: {}", name));
        }
    }
}

fn main() {
    if !ui::init() {
        return;
    }
    unsafe {
        APP = Some(AppState {
            infos: Vec::new(),
            selected_idx: None,
            res_options: Vec::new(),
            profile_label: None,
        });
    }
    refresh_outputs();
    if !app().infos.is_empty() {
        app().selected_idx = Some(0);
    }

    let win = ui::Window::new("Anzeigegeräte", -1, -1, WIN_W, WIN_H);

    // ── Toolbar: Erweitern / Spiegeln ───────────────────────────
    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, 40);
    toolbar.set_color(0xFF252526);
    toolbar.set_padding(8, 8, 8, 8);
    let mode_seg = ui::SegmentedControl::new("Erweitern|Spiegeln");
    mode_seg.set_size(220, 26);
    toolbar.add(&mode_seg);
    win.add(&toolbar);

    // ── Status bar: profile + apply hint ─────────────────────────
    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, 28);
    status_bar.set_color(0xFF252526);
    let profile_label = ui::Label::new("Profil: (kein aktives Profil)");
    profile_label.set_position(12, 6);
    profile_label.set_size(WIN_W - 200, 18);
    profile_label.set_text_color(COL_TEXT_DIM);
    status_bar.add(&profile_label);
    let save_btn = ui::Button::new("Aktuelles Layout speichern …");
    save_btn.set_position((WIN_W - 200) as i32, 2);
    save_btn.set_size(190, 22);
    status_bar.add(&save_btn);
    win.add(&status_bar);
    app().profile_label = Some(profile_label);

    // ── Layout preview (text for now; canvas drag-arrange follows) ──
    let preview_panel = ui::View::new();
    preview_panel.set_dock(ui::DOCK_TOP);
    preview_panel.set_size(WIN_W, 56);
    preview_panel.set_color(COL_PANEL_ALT);
    preview_panel.set_padding(16, 12, 16, 12);
    let preview = ui::Label::new("");
    preview.set_position(0, 0);
    preview.set_size(WIN_W - 32, 28);
    preview.set_text_color(COL_TEXT_DIM);
    preview_panel.add(&preview);
    win.add(&preview_panel);
    render_layout_label(&preview);

    // ── Split: list (left) + detail (right) ──────────────────────
    let split = ui::View::new();
    split.set_dock(ui::DOCK_FILL);
    split.set_color(COL_BG);
    win.add(&split);

    let list_panel = ui::View::new();
    list_panel.set_dock(ui::DOCK_LEFT);
    list_panel.set_size(280, 0);
    list_panel.set_color(COL_PANEL_ALT);
    list_panel.set_padding(8, 8, 8, 8);
    split.add(&list_panel);

    let detail_panel = ui::View::new();
    detail_panel.set_dock(ui::DOCK_FILL);
    detail_panel.set_color(COL_PANEL);
    detail_panel.set_padding(16, 16, 16, 16);
    split.add(&detail_panel);

    // ── Build list rows ────────────────────────────────────────
    for (idx, info) in app().infos.iter().enumerate() {
        let row = ui::Button::new(&friendly_name(info));
        row.set_dock(ui::DOCK_TOP);
        row.set_size(264, 36);
        list_panel.add(&row);
        let i = idx;
        row.on_click(move |_| {
            app().selected_idx = Some(i);
            // Selection is now `i`; the detail widgets stay mounted
            // and just re-read their state on the next Apply. A
            // smoother flow (live update of combo selections) would
            // require holding all widget handles in AppState — saved
            // for a follow-up.
        });
    }

    // ── Detail widgets (built once, refilled per selection) ────
    let enabled_lbl = ui::Label::new("Eingeschaltet");
    enabled_lbl.set_position(0, 0);
    enabled_lbl.set_size(280, 22);
    enabled_lbl.set_text_color(COL_TEXT);
    detail_panel.add(&enabled_lbl);
    let enabled_toggle = ui::Toggle::new(true);
    enabled_toggle.set_position((WIN_W as i32 - 280 - 80) as i32, 0);
    enabled_toggle.set_size(60, 26);
    detail_panel.add(&enabled_toggle);

    let res_lbl = ui::Label::new("Auflösung");
    res_lbl.set_position(0, 44);
    res_lbl.set_size(280, 22);
    res_lbl.set_text_color(COL_TEXT);
    detail_panel.add(&res_lbl);
    let res_combo = ui::ComboBox::new();
    res_combo.set_position(0, 70);
    res_combo.set_size(WIN_W - 280 - 32, 28);
    detail_panel.add(&res_combo);

    let orient_lbl = ui::Label::new("Ausrichtung");
    orient_lbl.set_position(0, 110);
    orient_lbl.set_size(280, 22);
    orient_lbl.set_text_color(COL_TEXT);
    detail_panel.add(&orient_lbl);
    let orient_combo = ui::ComboBox::new();
    orient_combo.set_position(0, 136);
    orient_combo.set_size(WIN_W - 280 - 32, 28);
    orient_combo.set_items(build_orientation_items());
    detail_panel.add(&orient_combo);

    let scale_lbl = ui::Label::new("Skalierung");
    scale_lbl.set_position(0, 176);
    scale_lbl.set_size(280, 22);
    scale_lbl.set_text_color(COL_TEXT);
    detail_panel.add(&scale_lbl);
    let scale_seg = ui::SegmentedControl::new("100 %|200 %");
    scale_seg.set_position(0, 200);
    scale_seg.set_size(180, 26);
    detail_panel.add(&scale_seg);

    let frac_lbl = ui::Label::new("Fraktionale Skalierung");
    frac_lbl.set_position(0, 240);
    frac_lbl.set_size(280, 22);
    frac_lbl.set_text_color(COL_TEXT);
    detail_panel.add(&frac_lbl);
    let frac_toggle = ui::Toggle::new(false);
    frac_toggle.set_position((WIN_W as i32 - 280 - 80) as i32, 240);
    frac_toggle.set_size(60, 26);
    detail_panel.add(&frac_toggle);

    let apply_btn = ui::Button::new("Anwenden");
    apply_btn.set_position(0, 290);
    apply_btn.set_size(140, 32);
    detail_panel.add(&apply_btn);

    // ── Wiring ─────────────────────────────────────────────────
    if let Some(idx) = app().selected_idx {
        app().res_options = resolution_options(&app().infos[idx]);
        res_combo.set_items(&build_combo_items(&app().res_options));
        let cur = (app().infos[idx].current_w, app().infos[idx].current_h);
        if let Some(i) = select_combo_index(&app().res_options, cur) {
            res_combo.set_selected_index(Some(i));
        }
    }

    mode_seg.on_active_changed(|ev| {
        let mut g = displayd::GlobalConfig::default();
        g.mirror_mode = if ev.index == 1 { 1 } else { 0 };
        if let Some(client) = displayd::DisplaydClient::connect() {
            let _ = client.set_global_config(&g);
            client.disconnect();
        }
        refresh_outputs();
    });

    let enabled_toggle_id = enabled_toggle.id();
    let frac_toggle_id = frac_toggle.id();
    let scale_seg_id = scale_seg.id();
    apply_btn.on_click(move |_| {
        let Some(idx) = app().selected_idx else { return; };
        let info = app().infos[idx];
        let res_idx = res_combo.selected_index().unwrap_or(0) as usize;
        let (w, h) = app().res_options.get(res_idx).copied().unwrap_or((info.current_w, info.current_h));
        let mut cfg = displayd::OutputConfig::default();
        cfg.edid_hash = info.edid_hash;
        // Toggle state: 0 = off, 1 = on. Read via Control::get_state.
        cfg.enabled = if ui::Control::from_id(enabled_toggle_id).get_state() != 0 { 1 } else { 0 };
        cfg.orientation = orient_combo.selected_index().unwrap_or(0);
        cfg.mode_w = w;
        cfg.mode_h = h;
        cfg.mode_refresh_mhz = if info.refresh_mhz > 0 { info.refresh_mhz } else { 60_000 };
        // SegmentedControl active index lives in the Control's state.
        cfg.scale_percent = match ui::Control::from_id(scale_seg_id).get_state() {
            1 => 200,
            _ => 100,
        };
        cfg.fractional_scale = if ui::Control::from_id(frac_toggle_id).get_state() != 0 { 1 } else { 0 };
        commit_output(idx, cfg);
    });

    save_btn.on_click(|_| {
        // Minimal save: prompt with a fixed name "current". A proper
        // text-input dialog comes with Phase 9c-extended.
        save_profile_with_name("current");
    });

    win.on_close(|_| ui::quit());
    ui::run();
}
