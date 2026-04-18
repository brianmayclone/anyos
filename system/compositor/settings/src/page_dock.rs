//! Dock settings page.
//!
//! Provides controls for dock icon size, magnification toggle, magnified size,
//! and dock position (bottom/left/right). Changes are saved to confd and an
//! IPC signal is sent to the dock to reload.

use alloc::format;
use libconf_schema::{default_bool, default_int, manifest, RegistryScope, ServiceSchema};
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Dock settings config (mirrors dock's settings.rs) ───────────────────────

const POS_BOTTOM: u32 = 0;
const POS_LEFT: u32 = 1;
const POS_RIGHT: u32 = 2;

const DOCK_SETTINGS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/icon_size", 48),
    default_bool("config/magnification", true),
    default_int("config/mag_size", 80),
    default_int("config/position", 0),
    default_bool("config/auto_hide", false),
];
const DOCK_SETTINGS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const DOCK_SETTINGS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "profile/dock_settings",
    RegistryScope::User,
    1,
    &["config"],
    DOCK_SETTINGS_DEFAULTS,
    DOCK_SETTINGS_MIGRATIONS,
);

fn dock_settings_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("settings", &DOCK_SETTINGS_MANIFEST)
}

struct DockSettings {
    icon_size: u32,
    magnification: bool,
    mag_size: u32,
    position: u32,
    auto_hide: bool,
}

impl DockSettings {
    fn default() -> Self {
        Self {
            icon_size: 48,
            magnification: true,
            mag_size: 80,
            position: POS_BOTTOM,
            auto_hide: false,
        }
    }

    fn validate(&mut self) {
        self.icon_size = self.icon_size.clamp(20, 128);
        let min_mag = self.icon_size + 1;
        if min_mag > 128 {
            self.mag_size = 128;
        } else {
            self.mag_size = self.mag_size.clamp(min_mag, 128);
        }
        if self.position > POS_RIGHT {
            self.position = POS_BOTTOM;
        }
    }
}

fn load_settings() -> DockSettings {
    let _ = dock_settings_schema().register();
    let mut confd = DockSettings::default();
    let mut found = false;
    if let Some(v) = dock_settings_schema().read_i64("config/icon_size") {
        confd.icon_size = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_bool("config/magnification") {
        confd.magnification = v;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_i64("config/mag_size") {
        confd.mag_size = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_i64("config/position") {
        confd.position = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_bool("config/auto_hide") {
        confd.auto_hide = v;
        found = true;
    }
    confd.validate();
    confd
}

fn save_settings(s: &DockSettings) {
    let _ = dock_settings_schema().register();
    let _ = dock_settings_schema().write_i64("config/icon_size", s.icon_size as i64);
    let _ = dock_settings_schema().write_bool("config/magnification", s.magnification);
    let _ = dock_settings_schema().write_i64("config/mag_size", s.mag_size as i64);
    let _ = dock_settings_schema().write_i64("config/position", s.position as i64);
    let _ = dock_settings_schema().write_bool("config/auto_hide", s.auto_hide);
}

/// Send an IPC reload-settings signal to the dock process.
fn notify_dock() {
    let chan = anyos_std::ipc::evt_chan_create("dock");
    let cmd: [u32; 5] = [2, 0, 0, 0, 0]; // 2 = reload settings
    anyos_std::ipc::evt_chan_emit(chan, &cmd);
}

// ── Slider ↔ value mapping ─────────────────────────────────────────────────

/// Map slider value (0–100) to icon size (20–128).
fn slider_to_icon_size(v: u32) -> u32 {
    20 + v * 108 / 100
}

/// Map icon size (20–128) to slider value (0–100).
fn icon_size_to_slider(size: u32) -> u32 {
    if size <= 20 { return 0; }
    if size >= 128 { return 100; }
    (size - 20) * 100 / 108
}

/// Map slider value (0–100) to mag size (min_mag–128).
fn slider_to_mag_size(v: u32, icon_size: u32) -> u32 {
    let min_mag = (icon_size + 1).min(128);
    let range = 128u32.saturating_sub(min_mag);
    min_mag + v * range / 100
}

/// Map mag size to slider value (0–100).
fn mag_size_to_slider(mag_size: u32, icon_size: u32) -> u32 {
    let min_mag = (icon_size + 1).min(128);
    if mag_size <= min_mag { return 0; }
    if mag_size >= 128 { return 100; }
    let range = 128u32.saturating_sub(min_mag);
    if range == 0 { return 0; }
    (mag_size - min_mag) * 100 / range
}

// ── Page builder ───────────────────────────────────────────────────────────

/// Build the Dock settings panel inside `parent`. Returns the panel View ID.
pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, "Dock", "Customize dock appearance and behavior");

    let settings = load_settings();
    build_size_card(&panel, &settings);
    build_position_card(&panel, &settings);
    build_behavior_card(&panel, &settings);

    parent.add(&panel);
    panel.id()
}

// ── Size & magnification card ──────────────────────────────────────────────

fn build_size_card(panel: &ui::View, settings: &DockSettings) {
    let card = layout::build_auto_card(panel);

    // ── Size slider row ──
    let size_row = layout::build_setting_row(&card, "Size", true);

    let size_val_lbl = ui::Label::new(&format!("{}px", settings.icon_size));
    size_val_lbl.set_position(400, 12);
    size_val_lbl.set_size(60, 20);
    size_val_lbl.set_text_color(layout::text_dim());
    size_val_lbl.set_font_size(13);
    size_row.add(&size_val_lbl);

    let size_slider = ui::Slider::new(icon_size_to_slider(settings.icon_size));
    size_slider.set_position(200, 10);
    size_slider.set_size(190, 24);
    size_row.add(&size_slider);

    let size_lbl_id = size_val_lbl.id();

    layout::build_separator(&card);

    // ── Magnification toggle row ──
    let mag_row = layout::build_setting_row(&card, "Magnification", false);
    let mag_toggle = layout::add_toggle_to_row(&mag_row, settings.magnification);

    layout::build_separator(&card);

    // ── Magnified size slider row ──
    let zoom_row = layout::build_setting_row(&card, "Max Zoom Size", false);

    let zoom_val_lbl = ui::Label::new(&format!("{}px", settings.mag_size));
    zoom_val_lbl.set_position(400, 12);
    zoom_val_lbl.set_size(60, 20);
    zoom_val_lbl.set_text_color(layout::text_dim());
    zoom_val_lbl.set_font_size(13);
    zoom_row.add(&zoom_val_lbl);

    let zoom_slider = ui::Slider::new(mag_size_to_slider(settings.mag_size, settings.icon_size));
    zoom_slider.set_position(200, 10);
    zoom_slider.set_size(190, 24);
    zoom_row.add(&zoom_slider);

    let zoom_lbl_id = zoom_val_lbl.id();
    let zoom_slider_id = zoom_slider.id();

    // ── Size slider change handler ──
    size_slider.on_value_changed(move |e| {
        let icon_size = slider_to_icon_size(e.value);
        let mut s = load_settings();
        s.icon_size = icon_size;
        // Re-clamp mag_size if it fell below icon_size + 1
        if s.mag_size <= icon_size {
            s.mag_size = (icon_size + 1).min(128);
        }
        s.validate();
        save_settings(&s);

        // Update size label
        let lbl = ui::Control::from_id(size_lbl_id);
        lbl.set_text(&format!("{}px", s.icon_size));

        // Update zoom slider position + label (range changed)
        let zoom_ctrl = ui::Control::from_id(zoom_slider_id);
        zoom_ctrl.set_state(mag_size_to_slider(s.mag_size, s.icon_size));
        let zoom_lbl = ui::Control::from_id(zoom_lbl_id);
        zoom_lbl.set_text(&format!("{}px", s.mag_size));

        notify_dock();
    });

    // ── Magnification toggle change handler ──
    mag_toggle.on_checked_changed(move |e| {
        let mut s = load_settings();
        s.magnification = e.checked;
        save_settings(&s);
        notify_dock();
    });

    // ── Zoom size slider change handler ──
    zoom_slider.on_value_changed(move |e| {
        let s_cur = load_settings();
        let mag_size = slider_to_mag_size(e.value, s_cur.icon_size);
        let mut s = s_cur;
        s.mag_size = mag_size;
        s.validate();
        save_settings(&s);

        let lbl = ui::Control::from_id(zoom_lbl_id);
        lbl.set_text(&format!("{}px", s.mag_size));

        notify_dock();
    });
}

// ── Position card ──────────────────────────────────────────────────────────

fn build_position_card(panel: &ui::View, settings: &DockSettings) {
    let card = layout::build_auto_card(panel);

    let row = ui::View::new();
    row.set_dock(ui::DOCK_TOP);
    row.set_size(552, 52);
    row.set_margin(24, 8, 24, 8);

    let lbl = ui::Label::new("Position on screen");
    lbl.set_position(0, 16);
    lbl.set_size(180, 20);
    lbl.set_text_color(layout::text());
    lbl.set_font_size(13);
    row.add(&lbl);

    let seg = ui::SegmentedControl::new("Bottom|Left|Right");
    seg.set_position(200, 10);
    seg.set_size(240, 32);
    seg.set_state(settings.position);
    row.add(&seg);

    seg.on_active_changed(move |e| {
        let mut s = load_settings();
        s.position = e.index;
        s.validate();
        save_settings(&s);
        notify_dock();
    });

    card.add(&row);
}

// ── Behavior card ──────────────────────────────────────────────────────────

fn build_behavior_card(panel: &ui::View, settings: &DockSettings) {
    let card = layout::build_auto_card(panel);

    let row = layout::build_setting_row(&card, "Auto-hide", true);

    let hint = ui::Label::new("Blendet das Dock aus und zeigt es am Bildschirmrand wieder an.");
    hint.set_position(200, 12);
    hint.set_size(250, 20);
    hint.set_text_color(layout::text_dim());
    hint.set_font_size(12);
    row.add(&hint);

    let toggle = layout::add_toggle_to_row(&row, settings.auto_hide);
    toggle.on_checked_changed(move |e| {
        let mut s = load_settings();
        s.auto_hide = e.checked;
        save_settings(&s);
        notify_dock();
    });
}
