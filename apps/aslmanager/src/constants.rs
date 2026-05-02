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
pub const DEBIAN_URL_PREFIX: &str = "https://deb.debian.org/debian/";
pub const DEBIAN_CLOUD_URL_PREFIX: &str = "https://cloud.debian.org/images/cloud/";
pub const DEBIAN_CLOUD_HTTP_URL_PREFIX: &str = "http://cloud.debian.org/images/cloud/";

pub const RAW_DISK_MIN_BYTES: u32 = 500_000_000;
