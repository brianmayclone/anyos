pub const WIN_W: u32 = 820;
pub const WIN_H: u32 = 540;
pub const SIDEBAR_W: u32 = 230;

pub const DISTRO_NAME: &str = "debian";
pub const IMAGE_REF: &str = "debian-13-nocloud-amd64-raw";
pub const OWNER: &str = "root";
pub const KERNEL_PROFILE: &str = "seabios-x86_64";

pub const CONFIG_PATH: &str = "/System/etc/asl/manager.conf";
pub const DEFAULT_ASL_ROOT: &str = "/System/var/asl";
pub const DEFAULT_DISTROS_DIR: &str = "distros";
pub const DEFAULT_IMAGES_DIR: &str = "images";

pub const DEBIAN_RAW_URL: &str =
    "https://cloud.debian.org/images/cloud/trixie/latest/debian-13-nocloud-amd64.raw";

/// SHA-512 hash of the Debian 13 nocloud raw image (ADR-0011 stage 2).
///
/// Pinned 2026-05-07 from the official `SHA512SUMS` at
/// `https://cloud.debian.org/images/cloud/trixie/latest/SHA512SUMS`.
///
/// **Update procedure:** when Debian rotates the `latest` symlink to a new
/// build, fetch the new `SHA512SUMS`, locate the line ending in
/// `debian-13-nocloud-amd64.raw`, and replace this constant with the new
/// 128-character hex digest. The `WARN: ... ADR-0011 stage 2` log line
/// shows actual vs. expected digest if a mismatch occurs — useful when
/// diagnosing a stale pin.
pub const DEBIAN_RAW_SHA512_HEX: &str =
    "eaf1401a2ddea2f8fdf5ce8c6263fbca144b4de3b0eea88a8875a3664438bce3f7298a3c7c3b11b94411d29f10ac61706530d278532fb8d6c269eab8d74fa017";

// URL prefixes are owned by aslmanager_core (shared with tests).
pub use aslmanager_core::{
    DEBIAN_CLOUD_HTTP_URL_PREFIX, DEBIAN_CLOUD_URL_PREFIX, DEBIAN_URL_PREFIX,
};

/// Minimum acceptable size for the Debian raw disk in bytes.
///
/// Defined as `u64` so this constant remains correct once anyOS exposes a
/// 64-bit stat. Today `anyos_std::fs::stat` only returns 32-bit sizes (see
/// `libs/stdlib/src/fs.rs::stat`), so the size check below is implicitly
/// capped at ~4 GiB. The current Debian 13 nocloud image is ~3 GiB, so this
/// is sufficient — but the cap is documented so it's not silently exceeded.
pub const RAW_DISK_MIN_BYTES: u64 = 500_000_000;

/// Hard upper bound the current 32-bit stat can represent without overflow.
/// Used to detect images larger than 4 GiB so we can fail loud rather than
/// silently accept a truncated size value.
pub const STAT_SIZE_OVERFLOW_SENTINEL: u32 = u32::MAX;
