//! Shared confd-backed compositor config helpers.

use alloc::format;
use alloc::string::String;

use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

const DEFAULT_LOGIN_PROGRAMS: &str = "/System/inputmon\n";
const DEFAULT_AUTOSTART_PROGRAMS: &str = "/System/notifyd\n/System/netmon\n/System/audiomon\n";
const DEFAULT_SHORTCUTS: &str = "";

const COMPOSITOR_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("login/programs_blob", DEFAULT_LOGIN_PROGRAMS),
    default_string("autostart/programs_blob", DEFAULT_AUTOSTART_PROGRAMS),
    default_string("shortcuts/mappings_blob", DEFAULT_SHORTCUTS),
    default_int("display/font_smoothing", 1),
    default_int("display/scale", 100),
    default_bool("display/scale_auto", true),
    default_int("resolution/width", 0),
    default_int("resolution/height", 0),
    default_string("theme/mode", "dark"),
    default_string("theme/style", ""),
    // Mouse wheel direction. 0 = traditional (wheel up scrolls content
    // up = standard PC behaviour, factory default), 1 = natural
    // scrolling (wheel up scrolls content down, like a touchpad).
    // Applied in `handle_scroll` (sign flip on dz).
    default_int("input/natural_scroll", 0),
    // Left-handed mouse: swap primary (LEFT) and secondary (RIGHT)
    // buttons in the compositor's button pipeline. 0 = right-handed
    // (default), 1 = left-handed.
    default_int("input/left_handed", 0),
    // Double-click threshold in milliseconds. Standard: 400.
    // Backend wiring lives in libanyui (per-process); for now the
    // value is stored in confd and the UI reflects it, the runtime
    // application is a Phase-2 follow-up.
    default_int("input/double_click_ms", 400),
    // Pointer speed in percent (50 = half, 100 = unchanged, 200 =
    // double). Backend application is a Phase-2 follow-up — for
    // now the value just round-trips through the UI.
    default_int("input/pointer_speed", 100),
    // Cursor size selector: 0 = small, 1 = medium (default), 2 = large.
    // Backend cursor-render scaling is a Phase-2 follow-up.
    default_int("input/cursor_size", 1),
];
const COMPOSITOR_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const COMPOSITOR_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "system/compositor",
    RegistryScope::System,
    1,
    &[
        "login",
        "autostart",
        "shortcuts",
        "display",
        "resolution",
        "theme",
        "input",
    ],
    COMPOSITOR_DEFAULTS,
    COMPOSITOR_MIGRATIONS,
);

fn schema() -> ServiceSchema<'static> {
    ServiceSchema::new("compositor", &COMPOSITOR_MANIFEST)
}

pub(super) fn register_manifest() {
    let _ = schema().register();
}

pub(super) fn read_string(rel_path: &str) -> Option<String> {
    register_manifest();
    schema().read_string(rel_path)
}

pub(super) fn write_string(rel_path: &str, value: &str) -> bool {
    register_manifest();
    schema().write_string(rel_path, value).is_ok()
}

pub(super) fn read_i64(rel_path: &str) -> Option<i64> {
    register_manifest();
    schema().read_i64(rel_path)
}

pub(super) fn write_i64(rel_path: &str, value: i64) -> bool {
    register_manifest();
    schema().write_i64(rel_path, value).is_ok()
}

pub(super) fn read_bool(rel_path: &str) -> Option<bool> {
    register_manifest();
    schema().read_bool(rel_path)
}

pub(super) fn write_bool(rel_path: &str, value: bool) -> bool {
    register_manifest();
    schema().write_bool(rel_path, value).is_ok()
}

#[allow(dead_code)]
pub(super) fn read_conf() -> Option<String> {
    register_manifest();
    let login =
        read_string("login/programs_blob").unwrap_or_else(|| String::from(DEFAULT_LOGIN_PROGRAMS));
    let autostart = read_string("autostart/programs_blob")
        .unwrap_or_else(|| String::from(DEFAULT_AUTOSTART_PROGRAMS));
    let shortcuts = read_string("shortcuts/mappings_blob").unwrap_or_default();
    let font_smoothing = read_i64("display/font_smoothing").unwrap_or(1);
    let scale = read_i64("display/scale").unwrap_or(100);
    let scale_auto = read_bool("display/scale_auto").unwrap_or(true);
    let width = read_i64("resolution/width").unwrap_or(0);
    let height = read_i64("resolution/height").unwrap_or(0);
    let mode = read_string("theme/mode").unwrap_or_else(|| String::from("dark"));
    let style = read_string("theme/style").unwrap_or_default();

    Some(format!(
        "[login]\n{}\n\n[autostart]\n{}\n\n[shortcuts]\n{}\n\n[display]\nfont_smoothing={}\nscale={}\nscale_auto={}\n\n[resolution]\nwidth={}\nheight={}\n\n[theme]\nmode={}\nstyle={}\n",
        login.trim_end(),
        autostart.trim_end(),
        shortcuts.trim_end(),
        font_smoothing,
        scale,
        scale_auto,
        width,
        height,
        mode,
        style,
    ))
}
