pub const WIN_W: u32 = 820;
pub const WIN_H: u32 = 540;
pub const SIDEBAR_W: u32 = 230;

pub const DISTRO_NAME: &str = "debian";
pub const IMAGE_REF: &str = "debian-trixie-netboot-amd64";
pub const OWNER: &str = "root";
pub const KERNEL_PROFILE: &str = "linux-x86_64-generic";

pub const CONFIG_PATH: &str = "/System/etc/asl/manager.conf";
pub const DEFAULT_ASL_ROOT: &str = "/System/var/asl";
pub const DEFAULT_DISTROS_DIR: &str = "distros";
pub const DEFAULT_IMAGES_DIR: &str = "images";
pub const DEFAULT_BOOT_DIR: &str = "boot";

pub const DEBIAN_KERNEL_URL: &str =
    "https://deb.debian.org/debian/dists/trixie/main/installer-amd64/current/images/netboot/debian-installer/amd64/linux";
pub const DEBIAN_INITRD_URL: &str =
    "https://deb.debian.org/debian/dists/trixie/main/installer-amd64/current/images/netboot/debian-installer/amd64/initrd.gz";

// URL prefixes are owned by aslmanager_core (shared with tests).
pub use aslmanager_core::{
    DEBIAN_CLOUD_HTTP_URL_PREFIX, DEBIAN_CLOUD_URL_PREFIX, DEBIAN_URL_PREFIX,
};

pub const LINUX_KERNEL_MIN_BYTES: u64 = 2_000_000;
pub const INITRD_MIN_BYTES: u64 = 8_000_000;

/// Hard upper bound the current 32-bit stat can represent without overflow.
/// Used to detect images larger than 4 GiB so we can fail loud rather than
/// silently accept a truncated size value.
pub const STAT_SIZE_OVERFLOW_SENTINEL: u32 = u32::MAX;
