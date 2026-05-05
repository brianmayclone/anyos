// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Settings page: Mouse — pointer behaviour and ergonomics.
//!
//! Reads/writes `system/compositor/input/*` via confd. The compositor
//! re-reads these on every management tick (~500 ms) so changes apply
//! without a restart.
//!
//! ## Phase 1 (active runtime effect)
//! - `natural_scroll`     — wheel direction inversion
//! - `left_handed`        — primary/secondary button swap
//!
//! ## Phase 2 (stored only — UI works, runtime wiring pending)
//! - `double_click_ms`    — needs libanyui per-process refresh
//! - `pointer_speed`      — needs absolute/relative pipeline audit
//! - `cursor_size`        — needs cursor-render software scaling
//!
//! Phase-2 fields still round-trip cleanly through this page so the
//! user-visible UX is complete; only the live effect is missing.

use alloc::format;
use alloc::string::String;
use anyos_std::i18n;
use libanyui_client as ui;
use libconf_schema::{
    default_int, manifest, RegistryScope, ServiceSchema,
};
use ui::Widget;

use crate::layout;

// ── Confd schema ────────────────────────────────────────────────────────────
//
// Mirror of the keys registered by the compositor itself. We list them
// here too so the settings app can read/write before the compositor
// has had a chance to register (e.g. first boot ordering).

const INPUT_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("input/natural_scroll", 0),
    default_int("input/left_handed", 0),
    default_int("input/double_click_ms", 400),
    default_int("input/pointer_speed", 100),
    default_int("input/cursor_size", 1),
];
const INPUT_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const INPUT_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "system/compositor",
    RegistryScope::System,
    1,
    &["input"],
    INPUT_DEFAULTS,
    INPUT_MIGRATIONS,
);

fn input_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("settings", &INPUT_MANIFEST)
}

fn read_bool(rel_path: &str) -> bool {
    let _ = input_schema().register();
    input_schema().read_i64(rel_path).map(|v| v != 0).unwrap_or(false)
}

fn write_bool(rel_path: &str, value: bool) {
    let _ = input_schema().register();
    let _ = input_schema().write_i64(rel_path, if value { 1 } else { 0 });
}

fn read_int(rel_path: &str, default: i64) -> i64 {
    let _ = input_schema().register();
    input_schema().read_i64(rel_path).unwrap_or(default)
}

fn write_int(rel_path: &str, value: i64) {
    let _ = input_schema().register();
    let _ = input_schema().write_i64(rel_path, value);
}

// ── Phase-2 control ranges ──────────────────────────────────────────────────

const DOUBLE_CLICK_MIN: i64 = 100;
const DOUBLE_CLICK_MAX: i64 = 1000;
const DOUBLE_CLICK_STEP: i64 = 50;

const POINTER_SPEED_MIN: i64 = 25;
const POINTER_SPEED_MAX: i64 = 400;
const POINTER_SPEED_STEP: i64 = 25;

// Cursor sizes are an enum (small/medium/large) rather than a slider so
// the dropdown matches what the cursor-render path will eventually
// support. Index 0/1/2 in confd.
const CURSOR_SIZE_LABELS: &[&str] = &["Small", "Medium", "Large"];

// ── Live label state ────────────────────────────────────────────────────────
//
// Plus/minus buttons need to repaint the numeric value next to them.
// Stash the label control IDs after they're built so the click handlers
// can update them in place.

static mut DOUBLE_CLICK_LABEL_ID: u32 = 0;
static mut POINTER_SPEED_LABEL_ID: u32 = 0;
static mut CURSOR_SIZE_LABEL_ID: u32 = 0;

fn update_label(id: u32, text: &str) {
    if id != 0 {
        let ctrl = ui::Control::from_id(id);
        ctrl.set_text(text);
    }
}

// ── Build ───────────────────────────────────────────────────────────────────

pub fn build(parent: &ui::ScrollView) -> u32 {
    let panel = ui::View::new();
    panel.set_dock(ui::DOCK_TOP);
    panel.set_auto_size(true);
    panel.set_color(layout::bg());

    layout::build_page_header(
        &panel,
        i18n::t("Mouse"),
        i18n::t("Pointer, scrolling and click behaviour"),
    );

    // ── Card 1: Wheel + buttons (Phase 1, live) ─────────────────────────
    let buttons_card = layout::build_auto_card(&panel);

    let natural_now = read_bool("input/natural_scroll");
    let nat_row = layout::build_setting_row(
        &buttons_card,
        i18n::t("Natural scrolling"),
        true,
    );
    let nat_toggle = layout::add_toggle_to_row(&nat_row, natural_now);
    nat_toggle.on_checked_changed(|e| {
        write_bool("input/natural_scroll", e.checked);
    });

    let left_now = read_bool("input/left_handed");
    let left_row = layout::build_setting_row(
        &buttons_card,
        i18n::t("Left-handed (swap buttons)"),
        false,
    );
    let left_toggle = layout::add_toggle_to_row(&left_row, left_now);
    left_toggle.on_checked_changed(|e| {
        write_bool("input/left_handed", e.checked);
    });

    panel.add(&buttons_card);

    // ── Card 2: Double-click (Phase 2) ──────────────────────────────────
    let dbl_card = layout::build_auto_card(&panel);
    let dbl_now = read_int("input/double_click_ms", 400)
        .clamp(DOUBLE_CLICK_MIN, DOUBLE_CLICK_MAX);

    let dbl_row =
        layout::build_setting_row(&dbl_card, i18n::t("Double-click speed"), true);
    build_int_stepper(
        &dbl_row,
        "input/double_click_ms",
        dbl_now,
        DOUBLE_CLICK_MIN,
        DOUBLE_CLICK_MAX,
        DOUBLE_CLICK_STEP,
        |v| format!("{} ms", v),
        unsafe { &raw mut DOUBLE_CLICK_LABEL_ID },
    );

    panel.add(&dbl_card);

    // ── Card 3: Pointer speed (Phase 2) ─────────────────────────────────
    let speed_card = layout::build_auto_card(&panel);
    let speed_now =
        read_int("input/pointer_speed", 100).clamp(POINTER_SPEED_MIN, POINTER_SPEED_MAX);

    let speed_row =
        layout::build_setting_row(&speed_card, i18n::t("Pointer speed"), true);
    build_int_stepper(
        &speed_row,
        "input/pointer_speed",
        speed_now,
        POINTER_SPEED_MIN,
        POINTER_SPEED_MAX,
        POINTER_SPEED_STEP,
        |v| format!("{} %", v),
        unsafe { &raw mut POINTER_SPEED_LABEL_ID },
    );

    panel.add(&speed_card);

    // ── Card 4: Cursor size (Phase 2) ───────────────────────────────────
    let cursor_card = layout::build_auto_card(&panel);
    let cursor_now =
        read_int("input/cursor_size", 1).clamp(0, (CURSOR_SIZE_LABELS.len() - 1) as i64);

    let cursor_row =
        layout::build_setting_row(&cursor_card, i18n::t("Cursor size"), true);
    build_int_stepper(
        &cursor_row,
        "input/cursor_size",
        cursor_now,
        0,
        (CURSOR_SIZE_LABELS.len() - 1) as i64,
        1,
        |v| String::from(i18n::t(CURSOR_SIZE_LABELS[v as usize])),
        unsafe { &raw mut CURSOR_SIZE_LABEL_ID },
    );

    panel.add(&cursor_card);

    parent.add(&panel);
    panel.id()
}

/// Build a [-] [value] [+] stepper control inside an existing setting
/// row. Persists each change to confd immediately so the compositor's
/// 500 ms refresh tick picks it up. The label is identified by
/// `label_slot` so the click handlers can repaint it in place.
fn build_int_stepper<F>(
    row: &ui::View,
    confd_key: &'static str,
    initial: i64,
    min: i64,
    max: i64,
    step: i64,
    formatter: F,
    label_slot: *mut u32,
) where
    F: Fn(i64) -> String + Copy + 'static,
{
    let btn_minus = ui::IconButton::new("-");
    btn_minus.set_position(330, 6);
    btn_minus.set_size(36, 30);
    let key_minus = confd_key;
    btn_minus.on_click(move |_| {
        let cur = read_int(key_minus, initial);
        let new_val = (cur - step).max(min);
        if new_val != cur {
            write_int(key_minus, new_val);
            let id = unsafe { *label_slot };
            update_label(id, &formatter(new_val));
        }
    });
    row.add(&btn_minus);

    let value_label = ui::Label::new(&formatter(initial));
    value_label.set_position(372, 12);
    value_label.set_size(80, 20);
    value_label.set_text_color(layout::text());
    value_label.set_font_size(13);
    unsafe {
        *label_slot = value_label.id();
    }
    row.add(&value_label);

    let btn_plus = ui::IconButton::new("+");
    btn_plus.set_position(456, 6);
    btn_plus.set_size(36, 30);
    let key_plus = confd_key;
    btn_plus.on_click(move |_| {
        let cur = read_int(key_plus, initial);
        let new_val = (cur + step).min(max);
        if new_val != cur {
            write_int(key_plus, new_val);
            let id = unsafe { *label_slot };
            update_label(id, &formatter(new_val));
        }
    });
    row.add(&btn_plus);
}
