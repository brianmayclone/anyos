//! Settings page: Sound — output device, volume, audio status.
//!
//! macOS-style Sound settings panel with:
//! - Output device info
//! - Volume control (+/- buttons with percentage)
//! - Mute toggle
//! - Playback status

use alloc::format;
use alloc::string::String;
use anyos_std::audio;
use anyos_std::i18n;
use libanyui_client as ui;
use ui::Widget;

use crate::layout;

// ── State ───────────────────────────────────────────────────────────────────

static mut VOLUME_LABEL_ID: u32 = 0;

// ── Build ───────────────────────────────────────────────────────────────────

pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(&panel, i18n::t("Sound"), i18n::t("Output, volume and audio devices"));

    // ── Output Device card ──────────────────────────────────────────────
    let device_card = layout::build_auto_card(&panel);

    let device_name = if audio::audio_is_available() {
        "Built-in Audio"
    } else {
        "No Audio Device"
    };
    layout::build_info_row(&device_card, i18n::t("Output Device"), device_name, true);
    layout::build_info_row(&device_card, i18n::t("Sample Rate"), "48000 Hz", false);

    let playing = audio::audio_is_playing();
    layout::build_info_row(&device_card, i18n::t("Status"),
        if playing { "Playing" } else { "Idle" }, false);

    panel.add(&device_card);

    // ── Volume Control card ─────────────────────────────────────────────
    let vol_card = layout::build_auto_card(&panel);

    let current_vol = if audio::audio_is_available() { audio::audio_get_volume() } else { 0 };

    // Volume row: label, [-] button, percentage, [+] button
    let vol_row = ui::View::new();
    vol_row.set_dock(ui::DOCK_TOP);
    vol_row.set_size(560, 36);
    vol_row.set_color(0x00000000);
    vol_row.set_margin(16, 12, 16, 4);

    let vol_text = ui::Label::new(i18n::t("Volume"));
    vol_text.set_dock(ui::DOCK_LEFT);
    vol_text.set_size(120, 36);
    vol_text.set_font_size(13);
    vol_text.set_text_color(layout::text());
    vol_row.add(&vol_text);

    let btn_minus = ui::IconButton::new("-");
    btn_minus.set_dock(ui::DOCK_LEFT);
    btn_minus.set_size(36, 30);
    btn_minus.on_click(|_| {
        let vol = audio::audio_get_volume();
        let new_vol = vol.saturating_sub(10);
        audio::audio_set_volume(new_vol);
        update_volume_label(new_vol);
    });
    vol_row.add(&btn_minus);

    let vol_pct = ui::Label::new(&format!("{}%", current_vol));
    vol_pct.set_dock(ui::DOCK_LEFT);
    vol_pct.set_size(60, 36);
    vol_pct.set_font_size(14);
    vol_pct.set_text_color(layout::text());
    vol_pct.set_padding(8, 0, 8, 0);
    unsafe { VOLUME_LABEL_ID = vol_pct.id(); }
    vol_row.add(&vol_pct);

    let btn_plus = ui::IconButton::new("+");
    btn_plus.set_dock(ui::DOCK_LEFT);
    btn_plus.set_size(36, 30);
    btn_plus.on_click(|_| {
        let vol = audio::audio_get_volume();
        let new_vol = (vol + 10).min(100);
        audio::audio_set_volume(new_vol);
        update_volume_label(new_vol);
    });
    vol_row.add(&btn_plus);

    vol_card.add(&vol_row);

    // Mute row
    let mute_row = ui::View::new();
    mute_row.set_dock(ui::DOCK_TOP);
    mute_row.set_size(560, 34);
    mute_row.set_color(0x00000000);
    mute_row.set_margin(16, 4, 16, 12);

    let mute_text = ui::Label::new(i18n::t("Mute"));
    mute_text.set_dock(ui::DOCK_LEFT);
    mute_text.set_size(120, 34);
    mute_text.set_font_size(13);
    mute_text.set_text_color(layout::text());
    mute_row.add(&mute_text);

    let mute_btn = ui::IconButton::new(if current_vol == 0 { "Unmute" } else { "Mute" });
    mute_btn.set_dock(ui::DOCK_LEFT);
    mute_btn.set_size(72, 28);
    mute_btn.on_click(|_| {
        let vol = audio::audio_get_volume();
        if vol > 0 {
            audio::audio_set_volume(0);
            update_volume_label(0);
        } else {
            audio::audio_set_volume(50);
            update_volume_label(50);
        }
    });
    mute_row.add(&mute_btn);

    vol_card.add(&mute_row);
    panel.add(&vol_card);

    // ── Alert Sounds card ───────────────────────────────────────────────
    let alert_card = layout::build_auto_card(&panel);
    layout::build_info_row(&alert_card, i18n::t("Alert Sound"), "Default", true);
    layout::build_info_row(&alert_card, i18n::t("Play alerts through"), "Selected output device", false);
    panel.add(&alert_card);

    parent.add(&panel);
    panel.id()
}

fn update_volume_label(vol: u8) {
    let id = unsafe { VOLUME_LABEL_ID };
    if id != 0 {
        let ctrl = ui::Control::from_id(id);
        ctrl.set_text(&format!("{}%", vol));
    }
}
