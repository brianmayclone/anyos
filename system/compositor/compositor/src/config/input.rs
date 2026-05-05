//! Input device settings (mouse, …) backed by confd.
//!
//! Currently only `input/natural_scroll`. The compositor reads this on
//! every wheel event via the `NATURAL_SCROLL` cell so toggles take effect
//! without restarting any process.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::file::{read_i64, register_manifest, write_i64};

// ── natural_scroll (Phase 1: active) ────────────────────────────────────────

/// Cached natural-scroll flag. Refreshed by `refresh_input_settings()`
/// (called periodically from the management loop) and read in the hot
/// `handle_scroll` path. AtomicBool keeps the read lock-free.
static NATURAL_SCROLL: AtomicBool = AtomicBool::new(false);

/// Cached left-handed flag. Read in `handle_mouse_button` to swap
/// primary (LEFT, bit 0) and secondary (RIGHT, bit 2).
static LEFT_HANDED: AtomicBool = AtomicBool::new(false);

/// Cached pointer-speed percentage (50..400, 100 = unchanged). Read in
/// the relative-mouse hot path to scale dx/dy.
static POINTER_SPEED_PCT: AtomicU32 = AtomicU32::new(100);

/// Cached cursor-size index (0=small, 1=medium, 2=large). Read by the
/// cursor-render path to pick scale factor.
static CURSOR_SIZE_IDX: AtomicU32 = AtomicU32::new(1);

pub fn read_natural_scroll() -> bool {
    register_manifest();
    read_i64("input/natural_scroll").map(|v| v != 0).unwrap_or(false)
}

pub fn save_natural_scroll(enabled: bool) {
    register_manifest();
    let _ = write_i64("input/natural_scroll", if enabled { 1 } else { 0 });
    NATURAL_SCROLL.store(enabled, Ordering::Relaxed);
}

/// Hot-path read used by `handle_scroll`. Lock-free.
pub fn natural_scroll_enabled() -> bool {
    NATURAL_SCROLL.load(Ordering::Relaxed)
}

// ── left_handed (Phase 1: active) ───────────────────────────────────────────

pub fn read_left_handed() -> bool {
    register_manifest();
    read_i64("input/left_handed").map(|v| v != 0).unwrap_or(false)
}

pub fn save_left_handed(enabled: bool) {
    register_manifest();
    let _ = write_i64("input/left_handed", if enabled { 1 } else { 0 });
    LEFT_HANDED.store(enabled, Ordering::Relaxed);
}

/// Hot-path read used by the button-dispatch pipeline. Lock-free.
pub fn left_handed_enabled() -> bool {
    LEFT_HANDED.load(Ordering::Relaxed)
}

// ── Phase-2 placeholders (stored, not yet acted on) ─────────────────────────
//
// These four helpers round-trip the value through confd so the
// settings UI works end-to-end today, but the compositor doesn't
// apply them yet. Wiring up the runtime effects is tracked as a
// follow-up: double_click_ms needs a libanyui-side refactor (the
// constant lives per-process in event_loop.rs), pointer_speed needs
// a pass through the absolute-vs-relative mouse pipeline, and
// cursor_size needs the cursor-render path to do software scaling
// since the virtio-gpu HW cursor is fixed at 64x64.

const DOUBLE_CLICK_MIN: i64 = 100;
const DOUBLE_CLICK_MAX: i64 = 1000;
const DOUBLE_CLICK_DEFAULT: i64 = 400;

pub fn read_double_click_ms() -> u32 {
    register_manifest();
    let raw = read_i64("input/double_click_ms").unwrap_or(DOUBLE_CLICK_DEFAULT);
    raw.clamp(DOUBLE_CLICK_MIN, DOUBLE_CLICK_MAX) as u32
}

pub fn save_double_click_ms(value: u32) {
    register_manifest();
    let clamped = (value as i64).clamp(DOUBLE_CLICK_MIN, DOUBLE_CLICK_MAX);
    let _ = write_i64("input/double_click_ms", clamped);
}

const POINTER_SPEED_MIN: i64 = 25;
const POINTER_SPEED_MAX: i64 = 400;
const POINTER_SPEED_DEFAULT: i64 = 100;

pub fn read_pointer_speed() -> u32 {
    register_manifest();
    let raw = read_i64("input/pointer_speed").unwrap_or(POINTER_SPEED_DEFAULT);
    raw.clamp(POINTER_SPEED_MIN, POINTER_SPEED_MAX) as u32
}

pub fn save_pointer_speed(percent: u32) {
    register_manifest();
    let clamped = (percent as i64).clamp(POINTER_SPEED_MIN, POINTER_SPEED_MAX);
    let _ = write_i64("input/pointer_speed", clamped);
    POINTER_SPEED_PCT.store(clamped as u32, Ordering::Relaxed);
}

/// Hot-path read used by the relative-mouse scaling helper. Lock-free.
pub fn pointer_speed_percent() -> u32 {
    POINTER_SPEED_PCT.load(Ordering::Relaxed)
}

const CURSOR_SIZE_MIN: i64 = 0;
const CURSOR_SIZE_MAX: i64 = 2;

pub fn read_cursor_size() -> u32 {
    register_manifest();
    let raw = read_i64("input/cursor_size").unwrap_or(1);
    raw.clamp(CURSOR_SIZE_MIN, CURSOR_SIZE_MAX) as u32
}

pub fn save_cursor_size(idx: u32) {
    register_manifest();
    let clamped = (idx as i64).clamp(CURSOR_SIZE_MIN, CURSOR_SIZE_MAX);
    let _ = write_i64("input/cursor_size", clamped);
    CURSOR_SIZE_IDX.store(clamped as u32, Ordering::Relaxed);
}

/// Hot-path read used by the cursor-render path. Lock-free.
/// Returns the configured size index: 0 = small, 1 = medium, 2 = large.
pub fn cursor_size_index() -> u32 {
    CURSOR_SIZE_IDX.load(Ordering::Relaxed)
}

/// Cursor scale factor as a percentage. 0 = 75%, 1 = 100%, 2 = 150%.
pub fn cursor_scale_percent() -> u32 {
    match cursor_size_index() {
        0 => 75,
        2 => 150,
        _ => 100,
    }
}

// ── Bulk refresh ────────────────────────────────────────────────────────────

/// Re-read all input settings from confd into the cached cells.
/// Called on startup and from the periodic management tick so
/// external changes (`confctl set …` or the settings UI) propagate
/// without a compositor restart.
pub fn refresh_input_settings() {
    NATURAL_SCROLL.store(read_natural_scroll(), Ordering::Relaxed);
    LEFT_HANDED.store(read_left_handed(), Ordering::Relaxed);
    POINTER_SPEED_PCT.store(read_pointer_speed(), Ordering::Relaxed);
    CURSOR_SIZE_IDX.store(read_cursor_size(), Ordering::Relaxed);
}

/// Same as `refresh_input_settings` but reports whether the cursor
/// size index actually changed. The management loop uses this to
/// decide whether to re-define the HW cursor (which is expensive
/// enough — GPU command, scaled-pixel realloc — that we don't want
/// to redo it every 500 ms tick).
pub fn refresh_input_settings_check_cursor() -> bool {
    NATURAL_SCROLL.store(read_natural_scroll(), Ordering::Relaxed);
    LEFT_HANDED.store(read_left_handed(), Ordering::Relaxed);
    POINTER_SPEED_PCT.store(read_pointer_speed(), Ordering::Relaxed);
    let new_idx = read_cursor_size();
    let old_idx = CURSOR_SIZE_IDX.swap(new_idx, Ordering::Relaxed);
    old_idx != new_idx
}

/// Backwards-compat alias — initial code path called this. Kept so
/// existing callers in bootstrap/management don't break.
pub fn refresh_natural_scroll() {
    refresh_input_settings();
}
