//! confd schema for the multi-monitor display configuration.
//!
//! Lives under `system/services/displayd/`. Two layers:
//!
//! `config/global/...`
//!   * `mirror_mode` (bool): when true, every output mirrors the
//!     primary instead of forming an extended desktop.
//!   * `primary_edid_hash` (string, hex): EDID-hash of the output
//!     that owns the menu bar / dock. Empty string falls back to
//!     output id 0.
//!
//! `config/output/<edid_hash>/...`   (one section per physically
//!                                    seen monitor, keyed by EDID
//!                                    hash so a re-plugged monitor
//!                                    keeps its settings)
//!   * `enabled`         (bool):    show this output at all
//!   * `orientation`     (int):     0 = landscape, 1 = portrait,
//!                                  2 = landscape-flipped, 3 = portrait-flipped
//!   * `mode_w`          (int):     pixel width
//!   * `mode_h`          (int):     pixel height
//!   * `mode_refresh_mhz`(int):     refresh in millihertz (60000 = 60 Hz)
//!   * `scale_percent`   (int):     100 = 1.0x, 200 = 2.0x, etc.
//!   * `fractional_scale`(bool):    enable non-integer scale steps
//!                                  (125, 150, 175). Off by default
//!                                  because it costs sharpness, same
//!                                  caveat GNOME shows the user.
//!   * `virtual_x`       (int):     position in virtual desktop
//!   * `virtual_y`       (int):
//!   * `mirror_of`       (string):  EDID hash of the source output
//!                                  if this output mirrors another
//!                                  one; empty for own framebuffer.
//!   * `friendly_name`   (string):  display-friendly name shown in
//!                                  the GUI (e.g. "Eingebaute Anzeige",
//!                                  "Eizo Nanao 27\"") — falls back
//!                                  to manufacturer + connector id.

use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};

/// Top-level globals that don't depend on a specific monitor.
///
/// Profiles ("home", "office", "mobile") let users save the layout for
/// a specific monitor combination and have displayd auto-pick the
/// matching one whenever the connected EDID set changes (hot-plug,
/// resume from suspend with a new dock, …).
///
/// `config/profiles/<name>/edids`            (string, comma-separated
///                                            EDID hex hashes)
/// `config/profiles/<name>/output/<edid>/...` (same per-output keys as
///                                            the live `config/output/`)
/// `config/active_profile`                   (string, current profile;
///                                            empty = no profile active)
const DISPLAYD_DIRS: &[&str] = &[
    "config",
    "config/global",
    "config/output",
    "config/profiles",
];

const DISPLAYD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    // mirror vs. extended desktop
    default_bool("config/global/mirror_mode", false),
    // empty = "use output id 0 as primary"
    default_string("config/global/primary_edid_hash", ""),
    // Optional layout-policy hint for displayd's default-layout
    // synthesis: one of "right_of_primary" / "below_primary" /
    // "above_primary" / "left_of_primary". Used only when an
    // unknown EDID hash shows up (no per-output entry yet).
    default_string("config/global/default_attach_side", "right_of_primary"),
    // Currently-active profile name (empty = no profile applied,
    // running off the bare config/output/* values).
    default_string("config/active_profile", ""),
];

const DISPLAYD_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];

const DISPLAYD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/displayd",
    RegistryScope::System,
    1,
    DISPLAYD_DIRS,
    DISPLAYD_DEFAULTS,
    DISPLAYD_MIGRATIONS,
);

pub(crate) const DISPLAYD_SCHEMA: ServiceSchema<'static> =
    ServiceSchema::new("displayd", &DISPLAYD_MANIFEST);

/// Build the relative path under `config/output/<edid_hash>/<key>`
/// for a per-output value. Caller passes the EDID hash as a hex
/// string (we always render it as 16 lower-case hex digits to keep
/// the path namespace stable).
pub(crate) fn output_key(edid_hex: &str, key: &str) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(20 + edid_hex.len() + key.len());
    s.push_str("config/output/");
    s.push_str(edid_hex);
    s.push('/');
    s.push_str(key);
    s
}

/// Render a u64 EDID hash to the canonical 16-hex-digit form used in
/// confd paths. Lower-case, no `0x` prefix, zero-padded.
pub(crate) fn edid_hex(hash: u64) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(16);
    let chars = b"0123456789abcdef";
    for shift in (0..64).step_by(4).rev() {
        s.push(chars[((hash >> shift) & 0xF) as usize] as char);
    }
    s
}

/// Build the relative path under `config/profiles/<name>/<key>` for a
/// per-profile global value (e.g. `edids`, `friendly_name`).
pub(crate) fn profile_key(name: &str, key: &str) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(20 + name.len() + key.len());
    s.push_str("config/profiles/");
    s.push_str(name);
    s.push('/');
    s.push_str(key);
    s
}

/// Build the relative path for a per-output value inside a profile —
/// `config/profiles/<name>/output/<edid_hex>/<key>`. Same shape as
/// the live `output_key` so the same OutputConfig blob can be written
/// into either namespace.
pub(crate) fn profile_output_key(
    profile: &str,
    edid_hex: &str,
    key: &str,
) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(28 + profile.len() + edid_hex.len() + key.len());
    s.push_str("config/profiles/");
    s.push_str(profile);
    s.push_str("/output/");
    s.push_str(edid_hex);
    s.push('/');
    s.push_str(key);
    s
}
