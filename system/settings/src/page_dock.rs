//! Dock settings page.
//!
//! Provides controls for dock icon size, magnification toggle, magnified size,
//! and dock position (bottom/left/right). Changes are saved to the dock
//! settings file and an IPC signal is sent to the dock to reload.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── Dock settings config (mirrors dock's settings.rs) ───────────────────────

const POS_BOTTOM: u32 = 0;
const POS_LEFT: u32 = 1;
const POS_RIGHT: u32 = 2;

/// System fallback path for dock settings.
const SYSTEM_SETTINGS_PATH: &str = "/System/dock/dock_settings.conf";

struct DockSettings {
    icon_size: u32,
    magnification: bool,
    mag_size: u32,
    position: u32,
}

impl DockSettings {
    fn default() -> Self {
        Self { icon_size: 48, magnification: true, mag_size: 80, position: POS_BOTTOM }
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

/// Resolve dock settings path — user home first, then `/System/dock/` fallback.
fn settings_path() -> String {
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 64];
    let len = anyos_std::process::getusername(uid, &mut name_buf);
    if len != u32::MAX && len > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..len as usize]) {
            let dir = format!("/Users/{}", username);
            let mut stat_buf = [0u32; 7];
            if anyos_std::fs::stat(&dir, &mut stat_buf) == 0 {
                return format!("/Users/{}/.dock_settings.conf", username);
            }
        }
    }
    let mut home_buf = [0u8; 256];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    if hlen != u32::MAX && hlen > 0 {
        if let Ok(home) = core::str::from_utf8(&home_buf[..hlen as usize]) {
            let mut stat_buf = [0u32; 7];
            if anyos_std::fs::stat(home, &mut stat_buf) == 0 {
                return format!("{}/.dock_settings.conf", home);
            }
        }
    }
    String::from(SYSTEM_SETTINGS_PATH)
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    if s.is_empty() { None } else { Some(result) }
}

fn load_settings() -> DockSettings {
    let path = settings_path();

    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(&path, &mut stat_buf) != 0 {
        return DockSettings::default();
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 1024 {
        return DockSettings::default();
    }

    let fd = anyos_std::fs::open(&path, 0);
    if fd == u32::MAX {
        return DockSettings::default();
    }

    let mut data = vec![0u8; file_size];
    let n = anyos_std::fs::read(fd, &mut data) as usize;
    anyos_std::fs::close(fd);

    if n == 0 {
        return DockSettings::default();
    }

    let text = match core::str::from_utf8(&data[..n]) {
        Ok(s) => s,
        Err(_) => return DockSettings::default(),
    };

    let mut s = DockSettings::default();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "icon_size" => { if let Some(v) = parse_u32(val) { s.icon_size = v; } }
                "magnification" => { s.magnification = val == "1" || val == "true"; }
                "mag_size" => { if let Some(v) = parse_u32(val) { s.mag_size = v; } }
                "position" => { if let Some(v) = parse_u32(val) { s.position = v; } }
                _ => {}
            }
        }
    }
    s.validate();
    s
}

fn save_settings(s: &DockSettings) {
    let path = settings_path();
    let content = format!(
        "icon_size={}\nmagnification={}\nmag_size={}\nposition={}\n",
        s.icon_size,
        if s.magnification { 1 } else { 0 },
        s.mag_size,
        s.position,
    );
    let _ = anyos_std::fs::write_bytes(&path, content.as_bytes());
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
