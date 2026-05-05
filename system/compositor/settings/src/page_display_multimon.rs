//! Multi-monitor settings block for the Display page.
//!
//! Shows mirror-mode toggle (Erweitern/Spiegeln), a drag-arrange canvas,
//! and one detail card per connected output (resolution, orientation,
//! scale, fractional, enabled). Replaces the standalone display-settings
//! app — the same logic lives here in the unified Settings UI.
//!
//! State that must persist across the closures (outputs vector, drag
//! offsets, canvas cache) lives in module-level statics following the
//! existing WP_TIMER pattern.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::display;
use anyos_std::i18n;
use libanyui_client as ui;
use libdisplay_client as displayd;
use ui::Widget;

use crate::layout;

// ── Module state ────────────────────────────────────────────────────────────

struct State {
    infos: Vec<display::DisplayInfo>,
    layout_x: Vec<i32>,
    initial_layout_x: Vec<i32>,
    dragging: Option<(usize, i32)>,
    canvas_id: u32,
    canvas_w: u32,
    canvas_h: u32,
    canvas_scale_num: i32,
    canvas_off_x: i32,
    canvas_off_y: i32,
    selected_idx: Option<usize>,
    global_mode_id: u32,
    apply_btn_id: u32,
    status_label_id: u32,
    output_uis: Vec<OutputUi>,
    pending_confirm: Option<PendingConfirm>,
}

#[derive(Clone)]
struct OutputUi {
    idx: usize,
    enabled_id: u32,
    res_combo_id: u32,
    orient_combo_id: u32,
    scale_seg_id: u32,
    frac_toggle_id: u32,
    mirror_combo_id: u32,
    resolution_opts: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct PendingConfirm {
    timer_id: u32,
    win_id: u32,
    label_id: u32,
    seconds_left: u32,
    previous_outputs: Vec<displayd::OutputConfig>,
    previous_global: displayd::GlobalConfig,
}

static mut STATE: Option<State> = None;
fn st() -> &'static mut State {
    unsafe { STATE.as_mut().unwrap() }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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

fn diagonal_inches(info: &display::DisplayInfo) -> u32 {
    let (w, h) = info.physical_mm_pair();
    if w == 0 && h == 0 {
        return 0;
    }
    let d_mm = isqrt_u64((w as u64) * (w as u64) + (h as u64) * (h as u64));
    ((d_mm * 100 / 254 + 5) / 10) as u32
}

fn friendly_name(info: &display::DisplayInfo) -> String {
    let m = pnpid(info);
    if info.is_primary() || info.id == 0 {
        format!("{} ({})", m, i18n::t("Built-in display"))
    } else {
        let inches = diagonal_inches(info);
        if inches > 0 {
            format!("{} {}\u{0022}", m, inches)
        } else {
            format!("{} #{}", m, info.id)
        }
    }
}

fn effective_size(info: &display::DisplayInfo) -> (u32, u32) {
    let w = if info.current_w > 0 {
        info.current_w
    } else {
        info.preferred_w
    };
    let h = if info.current_h > 0 {
        info.current_h
    } else {
        info.preferred_h
    };
    (w, h)
}

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

fn select_combo_index(opts: &[(u32, u32)], target: (u32, u32)) -> Option<u32> {
    opts.iter().position(|&p| p == target).map(|i| i as u32)
}

fn init_layout_x() {
    let mut next_x: i32 = 0;
    let mut xs: Vec<i32> = Vec::with_capacity(st().infos.len());
    for info in st().infos.iter() {
        if !info.is_connected() {
            xs.push(next_x);
            continue;
        }
        let (w, _h) = effective_size(info);
        xs.push(next_x);
        next_x += w as i32;
    }
    st().layout_x = xs;
    st().initial_layout_x = st().layout_x.clone();
}

fn active_global_mode_is_mirror() -> bool {
    st().infos
        .iter()
        .any(|info| info.is_connected() && info.is_mirror())
}

fn current_global_config() -> displayd::GlobalConfig {
    let mut g = displayd::GlobalConfig::default();
    g.mirror_mode = if active_global_mode_is_mirror() { 1 } else { 0 };
    if let Some(primary) = st().infos.iter().find(|info| info.is_connected() && info.is_primary())
    {
        g.primary_edid_hash = primary.edid_hash;
    } else if let Some(first) = st().infos.iter().find(|info| info.is_connected()) {
        g.primary_edid_hash = first.edid_hash;
    }
    g
}

fn output_config_from_info(idx: usize, info: &display::DisplayInfo) -> displayd::OutputConfig {
    let mut cfg = displayd::OutputConfig::default();
    cfg.edid_hash = info.edid_hash;
    cfg.enabled = if info.is_connected() { 1 } else { 0 };
    cfg.orientation = 0;
    let (w, h) = effective_size(info);
    cfg.mode_w = w;
    cfg.mode_h = h;
    cfg.mode_refresh_mhz = if info.refresh_mhz > 0 {
        info.refresh_mhz
    } else {
        60_000
    };
    cfg.scale_percent = 100;
    cfg.fractional_scale = 0;
    cfg.virtual_x = st().initial_layout_x.get(idx).copied().unwrap_or(0);
    cfg.virtual_y = 0;
    cfg.mirror_of_hash = if info.mirror_of == display::LayoutEntry::NO_MIRROR {
        0
    } else {
        st().infos
            .iter()
            .find(|target| target.id == info.mirror_of)
            .map(|target| target.edid_hash)
            .unwrap_or(0)
    };
    cfg
}

fn capture_current_configs() -> Vec<displayd::OutputConfig> {
    let mut out = Vec::new();
    let n = st().infos.len();
    for idx in 0..n {
        let info = st().infos[idx];
        if info.edid_hash != 0 {
            out.push(output_config_from_info(idx, &info));
        }
    }
    out
}

fn mirror_target_hash(ui: &OutputUi, state: u32) -> u64 {
    if state == 0 {
        return 0;
    }
    let mut nth = 1u32;
    for (j, info) in st().infos.iter().enumerate() {
        if j == ui.idx || !info.is_connected() {
            continue;
        }
        if nth == state {
            return info.edid_hash;
        }
        nth += 1;
    }
    0
}

fn output_config_from_controls(ui_state: &OutputUi) -> displayd::OutputConfig {
    let info = st().infos[ui_state.idx];
    let res_idx = ui::Control::from_id(ui_state.res_combo_id).get_state() as usize;
    let (w, h) = ui_state
        .resolution_opts
        .get(res_idx)
        .copied()
        .unwrap_or_else(|| effective_size(&info));
    let mut cfg = displayd::OutputConfig::default();
    cfg.edid_hash = info.edid_hash;
    cfg.enabled = if ui::Control::from_id(ui_state.enabled_id).get_state() != 0 {
        1
    } else {
        0
    };
    cfg.orientation = ui::Control::from_id(ui_state.orient_combo_id).get_state();
    cfg.mode_w = w;
    cfg.mode_h = h;
    cfg.mode_refresh_mhz = if info.refresh_mhz > 0 {
        info.refresh_mhz
    } else {
        60_000
    };
    cfg.scale_percent = match ui::Control::from_id(ui_state.scale_seg_id).get_state() {
        1 => 200,
        _ => 100,
    };
    cfg.fractional_scale = if ui::Control::from_id(ui_state.frac_toggle_id).get_state() != 0 {
        1
    } else {
        0
    };
    cfg.virtual_x = st().layout_x.get(ui_state.idx).copied().unwrap_or(0);
    cfg.virtual_y = 0;
    cfg.mirror_of_hash =
        mirror_target_hash(ui_state, ui::Control::from_id(ui_state.mirror_combo_id).get_state());
    cfg
}

fn set_status(text: &str) {
    let id = st().status_label_id;
    if id != 0 {
        ui::Control::from_id(id).set_text(text);
    }
}

fn apply_configs(outputs: &[displayd::OutputConfig], global: &displayd::GlobalConfig) -> bool {
    let Some(client) = displayd::DisplaydClient::connect() else {
        return false;
    };
    let mut ok = true;
    for cfg in outputs {
        if client.set_output_config(cfg).unwrap_or(u32::MAX) != 0 {
            ok = false;
        }
    }
    if client.set_global_config(global).unwrap_or(u32::MAX) != 0 {
        ok = false;
    }
    client.disconnect();
    ok
}

fn rollback_pending() {
    let Some(pending) = st().pending_confirm.take() else {
        return;
    };
    ui::kill_timer(pending.timer_id);
    let _ = apply_configs(&pending.previous_outputs, &pending.previous_global);
    ui::Window::from_id(pending.win_id).destroy();
    set_status(i18n::t("Display changes were reverted."));
}

fn confirm_pending() {
    let Some(pending) = st().pending_confirm.take() else {
        return;
    };
    ui::kill_timer(pending.timer_id);
    ui::Window::from_id(pending.win_id).destroy();
    set_status(i18n::t("Display changes applied."));
}

extern "C" fn confirm_revert_clicked(_control_id: u32, _event_type: u32, _userdata: u64) {
    rollback_pending();
}

extern "C" fn confirm_keep_clicked(_control_id: u32, _event_type: u32, _userdata: u64) {
    confirm_pending();
}

fn show_confirm_window(previous_outputs: Vec<displayd::OutputConfig>, previous_global: displayd::GlobalConfig) {
    if st().pending_confirm.is_some() {
        rollback_pending();
    }
    let win = ui::Window::new_with_flags(
        i18n::t("Keep display settings?"),
        -1,
        -1,
        360,
        150,
        ui::WIN_FLAG_NOT_RESIZABLE | ui::WIN_FLAG_ALWAYS_ON_TOP,
    );
    win.set_color(layout::card_bg());
    let title = ui::Label::new(i18n::t("Keep these display settings?"));
    title.set_position(18, 16);
    title.set_size(320, 22);
    title.set_font_size(15);
    title.set_text_color(layout::text());
    win.add(&title);

    let msg = ui::Label::new(i18n::t("Reverting automatically in 10 seconds."));
    msg.set_position(18, 46);
    msg.set_size(320, 22);
    msg.set_text_color(layout::text_dim());
    win.add(&msg);
    let label_id = msg.id();

    let revert = ui::Button::new(i18n::t("Revert"));
    revert.set_position(158, 102);
    revert.set_size(86, 30);
    win.add(&revert);
    let keep = ui::Button::new(i18n::t("Keep"));
    keep.set_position(254, 102);
    keep.set_size(86, 30);
    win.add(&keep);

    ui::Control::from_id(revert.id()).on_click_raw(confirm_revert_clicked, 0);
    ui::Control::from_id(keep.id()).on_click_raw(confirm_keep_clicked, 0);
    ui::Control::from_id(win.id()).on_event_raw(ui::EVENT_CLOSE, confirm_revert_clicked, 0);

    let timer_id = ui::set_timer(1000, || {
        let Some(pending) = st().pending_confirm.as_mut() else {
            return;
        };
        if pending.seconds_left == 0 {
            rollback_pending();
            return;
        }
        pending.seconds_left -= 1;
        if pending.seconds_left == 0 {
            rollback_pending();
        } else {
            ui::Control::from_id(pending.label_id).set_text(&format!(
                "{} {} {}",
                i18n::t("Reverting automatically in"),
                pending.seconds_left,
                i18n::t("seconds.")
            ));
        }
    });

    st().pending_confirm = Some(PendingConfirm {
        timer_id,
        win_id: win.id(),
        label_id,
        seconds_left: 10,
        previous_outputs,
        previous_global,
    });
}

fn apply_pending_settings() {
    let previous_outputs = capture_current_configs();
    let previous_global = current_global_config();
    let mut outputs = Vec::new();
    for ui_state in st().output_uis.clone() {
        outputs.push(output_config_from_controls(&ui_state));
    }
    let mut global = displayd::GlobalConfig::default();
    global.mirror_mode = if st().global_mode_id != 0
        && ui::Control::from_id(st().global_mode_id).get_state() == 1
    {
        1
    } else {
        0
    };
    if let Some(primary) = st().infos.iter().find(|info| info.is_connected() && info.is_primary())
    {
        global.primary_edid_hash = primary.edid_hash;
    } else if let Some(first) = st().infos.iter().find(|info| info.is_connected()) {
        global.primary_edid_hash = first.edid_hash;
    }
    if apply_configs(&outputs, &global) {
        set_status(i18n::t("Confirm the display change."));
        show_confirm_window(previous_outputs, previous_global);
    } else {
        set_status(i18n::t("Could not apply display settings."));
    }
}

// ── Canvas drawing ──────────────────────────────────────────────────────────

fn recalc_canvas_scale(cw: i32, ch: i32) {
    let mut total_w: i32 = 0;
    let mut max_h: i32 = 1;
    for (i, info) in st().infos.iter().enumerate() {
        if !info.is_connected() {
            continue;
        }
        let (w, h) = effective_size(info);
        if w == 0 || h == 0 {
            continue;
        }
        let right = st().layout_x[i] + w as i32;
        total_w = total_w.max(right);
        max_h = max_h.max(h as i32);
    }
    if total_w <= 0 {
        st().canvas_scale_num = 1;
        st().canvas_off_x = 0;
        st().canvas_off_y = 0;
        return;
    }
    let margin = 16;
    let avail_w = (cw - 2 * margin).max(1);
    let avail_h = (ch - 2 * margin).max(1);
    let scale_w = (avail_w * 1000) / total_w;
    let scale_h = (avail_h * 1000) / max_h;
    let scale = scale_w.min(scale_h).max(1);
    let render_w = total_w * scale / 1000;
    let render_h = max_h * scale / 1000;
    st().canvas_scale_num = scale;
    st().canvas_off_x = margin + (avail_w - render_w) / 2;
    st().canvas_off_y = margin + (avail_h - render_h) / 2;
}

fn render_canvas(canvas: &ui::Canvas) {
    let tc = ui::theme::colors();
    canvas.clear(tc.card_bg);
    let cw = canvas.get_stride() as i32;
    let ch = canvas.get_height() as i32;
    recalc_canvas_scale(cw, ch);
    let scale = st().canvas_scale_num;
    let off_x = st().canvas_off_x;
    let off_y = st().canvas_off_y;
    for (i, info) in st().infos.iter().enumerate() {
        if !info.is_connected() {
            continue;
        }
        let (w, h) = effective_size(info);
        if w == 0 || h == 0 {
            continue;
        }
        let x = off_x + st().layout_x[i] * scale / 1000;
        let y = off_y;
        let rw = (w as i32 * scale / 1000).max(20) as u32;
        let rh = (h as i32 * scale / 1000).max(15) as u32;
        let bg = if Some(i) == st().selected_idx {
            tc.accent
        } else if st().dragging.map(|(d, _)| d) == Some(i) {
            tc.accent_hover
        } else if info.is_primary() || info.id == 0 {
            tc.selection
        } else {
            tc.placeholder_bg
        };
        canvas.fill_rect(x, y, rw, rh, bg);
        canvas.draw_rect(x, y, rw, rh, tc.card_border, 1);
        let label = format!("{}", i + 1);
        let (lw, lh) = anyos_std::ui::window::font_measure(0, 18, &label);
        let lx = x + (rw as i32 - lw as i32) / 2;
        let ly = y + (rh as i32 - lh as i32) / 2;
        canvas.draw_text(lx, ly, 0xFFFFFFFF, 0, 18, &label);
    }
}

fn canvas_to_virtual_x(cx: i32) -> i32 {
    let scale = st().canvas_scale_num.max(1);
    ((cx - st().canvas_off_x) * 1000 + scale / 2) / scale
}

fn output_at_canvas(cx: i32, cy: i32) -> Option<usize> {
    let scale = st().canvas_scale_num.max(1);
    for (i, info) in st().infos.iter().enumerate() {
        if !info.is_connected() {
            continue;
        }
        let (w, h) = effective_size(info);
        if w == 0 || h == 0 {
            continue;
        }
        let x = st().canvas_off_x + st().layout_x[i] * scale / 1000;
        let y = st().canvas_off_y;
        let rw = (w as i32 * scale / 1000).max(20);
        let rh = (h as i32 * scale / 1000).max(15);
        if cx >= x && cx < x + rw && cy >= y && cy < y + rh {
            return Some(i);
        }
    }
    None
}

// ── Build ───────────────────────────────────────────────────────────────────

/// Build the multi-monitor section. Inserts mode card, drag-arrange
/// canvas (when >1 connected output), and one detail card per
/// connected output. Returns silently when no displays are advertised.
pub(crate) fn build(panel: &ui::View) {
    let infos = display::list(16);
    if infos.is_empty() {
        return;
    }

    unsafe {
        STATE = Some(State {
            infos,
            layout_x: Vec::new(),
            initial_layout_x: Vec::new(),
            dragging: None,
            canvas_id: 0,
            canvas_w: 0,
            canvas_h: 0,
            canvas_scale_num: 1,
            canvas_off_x: 0,
            canvas_off_y: 0,
            selected_idx: None,
            global_mode_id: 0,
            apply_btn_id: 0,
            status_label_id: 0,
            output_uis: Vec::new(),
            pending_confirm: None,
        });
    }
    init_layout_x();

    let connected: usize = st().infos.iter().filter(|i| i.is_connected()).count();

    // ── Mode card (Erweitern/Spiegeln) ──────────────────────────────────
    if connected > 1 {
        let mode_card = layout::build_auto_card(panel);
        let mode_row = layout::build_setting_row(&mode_card, i18n::t("Display mode"), true);
        let mode_seg =
            ui::SegmentedControl::new(&format!("{}|{}", i18n::t("Extend"), i18n::t("Mirror")));
        mode_seg.set_position(200, 8);
        mode_seg.set_size(220, 28);
        mode_seg.set_state(if active_global_mode_is_mirror() { 1 } else { 0 });
        st().global_mode_id = mode_seg.id();
        mode_row.add(&mode_seg);
    }

    // ── Drag-arrange canvas ─────────────────────────────────────────────
    if connected > 1 {
        let arr_card = layout::build_auto_card(panel);
        let hdr = layout::build_setting_row(&arr_card, i18n::t("Arrangement"), true);
        let hint = ui::Label::new(i18n::t("Drag to rearrange"));
        hint.set_position(200, 12);
        hint.set_size(340, 20);
        hint.set_font_size(11);
        hint.set_text_color(layout::text_dim());
        hdr.add(&hint);

        const CW: u32 = 552;
        const CH: u32 = 200;
        let canvas = ui::Canvas::new(CW, CH);
        canvas.set_dock(ui::DOCK_TOP);
        canvas.set_size(CW, CH);
        canvas.set_margin(24, 4, 24, 12);
        canvas.set_interactive(true);
        arr_card.add(&canvas);
        st().canvas_id = canvas.id();
        st().canvas_w = CW;
        st().canvas_h = CH;
        render_canvas(&canvas);

        let canvas_for_down = canvas.clone();
        canvas.on_mouse_down(move |cx, cy, _btn| {
            if let Some(idx) = output_at_canvas(cx, cy) {
                let virt_click = canvas_to_virtual_x(cx);
                let offset = virt_click - st().layout_x[idx];
                st().dragging = Some((idx, offset));
                st().selected_idx = Some(idx);
                render_canvas(&canvas_for_down);
            }
        });
        let canvas_for_move = canvas.clone();
        canvas.on_mouse_move(move |cx, _cy| {
            if let Some((idx, offset)) = st().dragging {
                let virt_x = canvas_to_virtual_x(cx) - offset;
                st().layout_x[idx] = virt_x;
                render_canvas(&canvas_for_move);
            }
        });
        let canvas_for_up = canvas.clone();
        canvas.on_mouse_up(move |_cx, _cy, _btn| {
            if let Some((idx, _)) = st().dragging.take() {
                const SNAP: i32 = 32;
                let my_w = effective_size(&st().infos[idx]).0 as i32;
                let my_left = st().layout_x[idx];
                let my_right = my_left + my_w;
                for (j, info) in st().infos.iter().enumerate() {
                    if j == idx || !info.is_connected() {
                        continue;
                    }
                    let other_left = st().layout_x[j];
                    let other_right = other_left + effective_size(info).0 as i32;
                    if (my_left - other_right).abs() < SNAP {
                        st().layout_x[idx] = other_right;
                        break;
                    }
                    if (my_right - other_left).abs() < SNAP {
                        st().layout_x[idx] = other_left - my_w;
                        break;
                    }
                }
                render_canvas(&canvas_for_up);
            }
        });
    }

    // ── Per-output detail cards ─────────────────────────────────────────
    let n = st().infos.len();
    for idx in 0..n {
        if !st().infos[idx].is_connected() {
            continue;
        }
        build_output_card(panel, idx);
    }

    let apply_card = layout::build_auto_card(panel);
    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 52);
    row.set_margin(24, 8, 24, 8);
    let status = ui::Label::new(i18n::t("Changes are applied together."));
    status.set_position(0, 16);
    status.set_size(360, 20);
    status.set_text_color(layout::text_dim());
    status.set_font_size(12);
    row.add(&status);
    st().status_label_id = status.id();
    let apply_btn = ui::Button::new(i18n::t("Apply"));
    apply_btn.set_position(424, 10);
    apply_btn.set_size(120, 30);
    apply_btn.on_click(|_| apply_pending_settings());
    row.add(&apply_btn);
    st().apply_btn_id = apply_btn.id();
    apply_card.add(&row);
}

fn build_output_card(panel: &ui::View, idx: usize) {
    let info = st().infos[idx];
    let card = layout::build_auto_card(panel);

    // Header row: name + enabled toggle
    let hdr = layout::build_setting_row(&card, &friendly_name(&info), true);
    let enabled_toggle = ui::Toggle::new(info.is_connected());
    enabled_toggle.set_position(480, 8);
    enabled_toggle.set_size(60, 26);
    hdr.add(&enabled_toggle);
    let enabled_id = enabled_toggle.id();

    layout::build_separator(&card);

    // Resolution combo
    let res_row = layout::build_setting_row(&card, i18n::t("Resolution"), false);
    let opts = resolution_options(&info);
    let res_combo = ui::ComboBox::new();
    res_combo.set_position(200, 8);
    res_combo.set_size(280, 28);
    res_combo.set_items(&build_combo_items(&opts));
    let cur = (info.current_w, info.current_h);
    if let Some(i) = select_combo_index(&opts, cur) {
        res_combo.set_selected_index(Some(i));
    }
    res_row.add(&res_combo);
    let res_combo_id = res_combo.id();

    layout::build_separator(&card);

    // Orientation combo
    let orient_row = layout::build_setting_row(&card, i18n::t("Orientation"), false);
    let orient_combo = ui::ComboBox::new();
    orient_combo.set_position(200, 8);
    orient_combo.set_size(280, 28);
    orient_combo.set_items(&format!(
        "{}|{}|{}|{}",
        i18n::t("Landscape"),
        i18n::t("Portrait"),
        i18n::t("Landscape (flipped)"),
        i18n::t("Portrait (flipped)")
    ));
    orient_combo.set_selected_index(Some(0));
    orient_row.add(&orient_combo);
    let orient_combo_id = orient_combo.id();

    layout::build_separator(&card);

    // Scale segmented
    let scale_row = layout::build_setting_row(&card, i18n::t("Scale"), false);
    let scale_seg = ui::SegmentedControl::new("100 %|200 %");
    scale_seg.set_position(200, 8);
    scale_seg.set_size(180, 28);
    scale_row.add(&scale_seg);
    let scale_seg_id = scale_seg.id();

    layout::build_separator(&card);

    // Fractional scale toggle
    let frac_row = layout::build_setting_row(&card, i18n::t("Fractional scaling"), false);
    let frac_toggle = ui::Toggle::new(false);
    frac_toggle.set_position(480, 8);
    frac_toggle.set_size(60, 26);
    frac_row.add(&frac_toggle);
    let frac_toggle_id = frac_toggle.id();

    layout::build_separator(&card);

    // Per-output mirror target. "Extend" leaves this output independent;
    // choosing another display persists mirror_of for mixed 3-monitor setups.
    let mirror_row = layout::build_setting_row(&card, i18n::t("Use as"), false);
    let mut mirror_items = String::from(i18n::t("Extend"));
    let mut selected_mirror = 0u32;
    let mut mirror_idx = 1u32;
    for (j, other) in st().infos.iter().enumerate() {
        if j != idx && other.is_connected() {
            mirror_items.push('|');
            mirror_items.push_str(&format!("{} {}", i18n::t("Mirror Display"), j + 1));
            if info.mirror_of != display::LayoutEntry::NO_MIRROR && info.mirror_of == other.id {
                selected_mirror = mirror_idx;
            }
            mirror_idx += 1;
        }
    }
    let mirror_combo = ui::ComboBox::new();
    mirror_combo.set_position(200, 8);
    mirror_combo.set_size(280, 28);
    mirror_combo.set_items(&mirror_items);
    mirror_combo.set_selected_index(Some(selected_mirror));
    mirror_row.add(&mirror_combo);
    let mirror_combo_id = mirror_combo.id();

    st().output_uis.push(OutputUi {
        idx,
        enabled_id,
        res_combo_id,
        orient_combo_id,
        scale_seg_id,
        frac_toggle_id,
        mirror_combo_id,
        resolution_opts: opts,
    });
}
