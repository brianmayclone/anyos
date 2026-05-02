pub const WIN_W: u32 = 820;
pub const WIN_H: u32 = 540;
pub const SIDEBAR_W: u32 = 230;

pub const DISTRO_NAME: &str = "debian";
pub const IMAGE_REF: &str = "debian-stable-amd64-netboot";
pub const OWNER: &str = "root";

pub const CONFIG_PATH: &str = "/System/etc/asl/manager.conf";
pub const DEFAULT_ASL_ROOT: &str = "/System/var/asl";
pub const DEFAULT_DISTROS_DIR: &str = "distros";
pub const DEFAULT_BOOT_DIR: &str = "boot";

pub const DEBIAN_KERNEL_URL: &str =
    "https://deb.debian.org/debian/dists/stable/main/installer-amd64/current/images/netboot/debian-installer/amd64/linux";
pub const DEBIAN_INITRD_URL: &str =
    "https://deb.debian.org/debian/dists/stable/main/installer-amd64/current/images/netboot/debian-installer/amd64/initrd.gz";
pub const DEBIAN_URL_PREFIX: &str = "https://deb.debian.org/debian/";

pub const KERNEL_MIN_BYTES: u32 = 1_000_000;
pub const INITRD_MIN_BYTES: u32 = 4_000_000;
