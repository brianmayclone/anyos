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

/// CRC-64 (ECMA polynomial) — kernel uses the same constant for EDID
/// hashes (kernel/src/drivers/gpu/output.rs::edid_hash). Inlined here
/// because displayd has no direct dep on the kernel crate.
pub(crate) fn crc64_ecma(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    const POLY: u64 = 0x42F0_E1EB_A9EA_3693;
    let mut crc: u64 = 0;
    for &b in bytes {
        crc ^= (b as u64) << 56;
        for _ in 0..8 {
            if crc & (1u64 << 63) != 0 {
                crc = (crc << 1) ^ POLY;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

/// Top-level globals that don't depend on a specific monitor.
///
/// Setups (auto-keyed). The display layout is keyed by a deterministic
/// hash of the currently connected EDID set, so plugging the same set
/// of monitors anywhere always restores the same layout — no manual
/// "save profile" step.
///
/// `config/setups/<setup_hash>/edids`            (comma-separated
///                                                 EDID hex list, used
///                                                 for verification)
/// `config/setups/<setup_hash>/friendly_name`    (optional, e.g. "home")
/// `config/setups/<setup_hash>/output/<edid>/...` (per-output config —
///                                                 same keys as the
///                                                 live `config/output/`)
/// `config/active_setup`                          (current setup hash,
///                                                 empty = no setup
///                                                 hash matched)
const DISPLAYD_DIRS: &[&str] = &[
    "config",
    "config/global",
    "config/output",
    "config/setups",
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
    // Currently-active setup hash (empty = no matching setup loaded
    // yet; the next CMD_SET_OUTPUT_CONFIG creates one). The hash
    // itself is canonical: 16 hex chars derived from the sorted EDID
    // hashes of the currently connected outputs.
    default_string("config/active_setup", ""),
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

/// Path for a setup-global value: `config/setups/<setup_hash>/<key>`.
/// Used for `edids` and `friendly_name`.
pub(crate) fn setup_key(setup_hash: &str, key: &str) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(20 + setup_hash.len() + key.len());
    s.push_str("config/setups/");
    s.push_str(setup_hash);
    s.push('/');
    s.push_str(key);
    s
}

/// Path for a per-output value inside a setup —
/// `config/setups/<setup_hash>/output/<edid_hex>/<key>`. Same shape as
/// the live `output_key` so the same OutputConfig blob writes into
/// either namespace.
pub(crate) fn setup_output_key(
    setup_hash: &str,
    edid_hex: &str,
    key: &str,
) -> alloc::string::String {
    use alloc::string::String;
    let mut s = String::with_capacity(28 + setup_hash.len() + edid_hex.len() + key.len());
    s.push_str("config/setups/");
    s.push_str(setup_hash);
    s.push_str("/output/");
    s.push_str(edid_hex);
    s.push('/');
    s.push_str(key);
    s
}

/// Compute a canonical setup hash from a slice of connected EDID hashes.
///
/// Sort the EDID hashes lower-to-upper (set semantics — order doesn't
/// matter), concatenate as 16-hex strings, then run the same CRC-64
/// (ECMA polynomial) we already use for individual EDIDs. The result
/// is rendered as 16 lower-case hex chars and used as the setup key.
///
/// Properties this gives us, by construction:
///
///   * Same set of connected monitors → same setup hash, regardless
///     of plug order or scanout id assignment. Plugging the laptop +
///     the Eizo at home produces the same hash as plugging them in a
///     different order at the office, so the layout is shared.
///   * Different set → different hash → independent layout entry.
///   * Hot-plugging one extra monitor produces a fresh hash — the
///     previous setup's layout stays intact under its own key, the
///     new combination gets its own slot.
pub(crate) fn compute_setup_hash(edid_hashes: &[u64]) -> alloc::string::String {
    use alloc::string::String;
    if edid_hashes.is_empty() {
        return String::new();
    }
    let mut sorted: alloc::vec::Vec<u64> = edid_hashes.into();
    sorted.sort();
    // Concatenate bytes (big-endian, deterministic) and CRC-64 them.
    let mut buf: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(sorted.len() * 8);
    for h in &sorted {
        buf.extend_from_slice(&h.to_be_bytes());
    }
    let crc = crc64_ecma(&buf);
    let mut s = String::with_capacity(16);
    let chars = b"0123456789abcdef";
    for shift in (0..64).step_by(4).rev() {
        s.push(chars[((crc >> shift) & 0xF) as usize] as char);
    }
    s
}
