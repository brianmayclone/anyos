//! CPU power management settings.

use alloc::format;
use alloc::string::String;
use anyos_std::{i18n, sys};
use libanyui_client as ui;
use ui::Widget;

use libconf_schema::{default_int, manifest, RegistryScope, ServiceSchema};

use crate::layout;

const POWER_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "profile/power",
    RegistryScope::System,
    1,
    &["config", "scheduler"],
    &[
        default_int("config/profile", 1),
        default_int("scheduler/placement", 1),
        default_int("scheduler/efficiency_bias", 50),
    ],
    &[],
);
const POWER_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("settings", &POWER_MANIFEST);

const PROFILE_ITEMS: &str = "Power Saver|Balanced|Performance";
const PLACEMENT_ITEMS: &str = "Consolidate idle cores|Balanced|Spread work";

pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(
        &panel,
        i18n::t("Power"),
        i18n::t("CPU frequency policy and scheduler energy model"),
    );

    let _ = POWER_SCHEMA.register();
    let profile = read_profile();
    let _ = sys::set_cpu_power_profile(profile);

    build_status_card(&panel);
    build_profile_card(&panel, profile);
    build_scheduler_card(&panel);

    parent.add(&panel);
    panel.id()
}

fn read_profile() -> u32 {
    POWER_SCHEMA
        .read_i64("config/profile")
        .unwrap_or(1)
        .clamp(0, 2) as u32
}

fn read_placement() -> u32 {
    POWER_SCHEMA
        .read_i64("scheduler/placement")
        .unwrap_or(1)
        .clamp(0, 2) as u32
}

fn read_efficiency_bias() -> u32 {
    POWER_SCHEMA
        .read_i64("scheduler/efficiency_bias")
        .unwrap_or(50)
        .clamp(0, 100) as u32
}

fn build_status_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);
    let info = sys::cpu_power_info().unwrap_or_default();

    layout::build_info_row(&card, i18n::t("Driver"), driver_label(info.driver), true);
    layout::build_separator(&card);
    layout::build_info_row(
        &card,
        i18n::t("Profile"),
        profile_label(info.profile),
        false,
    );
    layout::build_separator(&card);
    layout::build_info_row(
        &card,
        i18n::t("Current Frequency"),
        &mhz_label(info.current_mhz),
        false,
    );
    layout::build_separator(&card);
    layout::build_info_row(
        &card,
        i18n::t("Maximum Frequency"),
        &mhz_label(info.max_mhz),
        false,
    );
    layout::build_separator(&card);
    layout::build_info_row(
        &card,
        i18n::t("Features"),
        &feature_label(info.features),
        false,
    );
}

fn build_profile_card(panel: &ui::View, profile: u32) {
    let card = layout::build_auto_card(panel);
    let row = layout::build_setting_row(&card, i18n::t("Energy Profile"), true);

    let dd = ui::DropDown::new(PROFILE_ITEMS);
    dd.set_position(250, 8);
    dd.set_size(220, 28);
    dd.set_selected_index(profile);
    dd.on_selection_changed(|e| {
        let profile = e.index.min(2);
        let _ = POWER_SCHEMA.write_i64("config/profile", profile as i64);
        let _ = sys::set_cpu_power_profile(profile);
    });
    row.add(&dd);
}

fn build_scheduler_card(panel: &ui::View) {
    let card = layout::build_auto_card(panel);

    let placement_row = layout::build_setting_row(&card, i18n::t("Task Placement"), true);
    let placement = read_placement();
    let dd = ui::DropDown::new(PLACEMENT_ITEMS);
    dd.set_position(250, 8);
    dd.set_size(220, 28);
    dd.set_selected_index(placement);
    dd.on_selection_changed(|e| {
        let placement = e.index.min(2);
        let _ = POWER_SCHEMA.write_i64("scheduler/placement", placement as i64);
    });
    placement_row.add(&dd);

    layout::build_separator(&card);

    let bias_row = layout::build_setting_row(&card, i18n::t("Efficiency Bias"), false);
    let bias = read_efficiency_bias();
    let val = ui::Label::new(&format!("{}%", bias));
    val.set_position(250, 12);
    val.set_size(60, 20);
    val.set_text_color(layout::text_dim());
    val.set_font_size(13);
    bias_row.add(&val);
    let val_id = val.id();

    let slider = ui::Slider::new(bias);
    slider.set_position(320, 10);
    slider.set_size(150, 24);
    slider.on_value_changed(move |e| {
        let value = e.value.min(100);
        let _ = POWER_SCHEMA.write_i64("scheduler/efficiency_bias", value as i64);
        ui::Control::from_id(val_id).set_text(&format!("{}%", value));
    });
    bias_row.add(&slider);
}

fn profile_label(profile: u32) -> &'static str {
    match profile {
        0 => "Power Saver",
        2 => "Performance",
        _ => "Balanced",
    }
}

fn driver_label(driver: u32) -> &'static str {
    match driver {
        1 => "Intel HWP",
        2 => "Intel P-state",
        3 => "AMD P-state",
        4 => "KVM host CPU",
        _ => "Unavailable",
    }
}

fn mhz_label(value: u32) -> String {
    if value == 0 {
        String::from("Unknown")
    } else {
        format!("{} MHz", value)
    }
}

fn feature_label(features: u32) -> String {
    let mut out = String::new();
    push_feature(&mut out, features & 1 != 0, "HWP");
    push_feature(&mut out, features & 2 != 0, "Turbo");
    push_feature(&mut out, features & 4 != 0, "APERF");
    push_feature(&mut out, features & 8 != 0, "Hypervisor");
    push_feature(&mut out, features & 16 != 0, "Active control");
    if out.is_empty() {
        String::from("None")
    } else {
        out
    }
}

fn push_feature(out: &mut String, enabled: bool, name: &str) {
    if !enabled {
        return;
    }
    if !out.is_empty() {
        out.push_str(", ");
    }
    out.push_str(name);
}
