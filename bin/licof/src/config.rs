use alloc::string::String;
use alloc::vec::Vec;
use libconf_schema::{
    default_int, default_string, manifest, migration_string, RegistryScope, ServiceSchema,
};

const DEFAULT_ROOT: &str = "/System/var/licof";
const DEFAULT_ROOTFS: &str = "/System/var/licof/rootfs";
const DEFAULT_CACHE: &str = "/System/var/licof/cache/bookworm";
const DEFAULT_DB: &str = "/System/var/licof/db/bookworm";
const DEFAULT_INSTALLED_DB: &str = "/System/var/licof/db/bookworm/installed";
const DEFAULT_APT_BASE: &str = "http://deb.debian.org/debian";
const DEFAULT_APT_DIST: &str = "bookworm";
const DEFAULT_APT_COMPONENT: &str = "main";
const DEFAULT_APT_ARCH: &str = "amd64";
const DEFAULT_WGET: &str = "/System/bin/wget";
const DEFAULT_DOWNLOAD_ATTEMPTS: i64 = 4;
const DEFAULT_INDEX_REQUIRED_PACKAGES: &str =
    "apt,base-files,base-passwd,bash,coreutils,dash,debian-archive-keyring,libc6,libgcc-s1,libpam-runtime,libstdc++6,login,passwd,zlib1g";
const DEFAULT_BOOTSTRAP_SEED: &str =
    "base-files,base-passwd,libc6,libgcc-s1,libstdc++6,zlib1g,apt,debian-archive-keyring,dash,bash,coreutils,libpam-runtime,login,passwd,mc,procps,htop,gcc,make";

const LICOF_DIRS: &[&str] = &["paths", "apt", "bootstrap", "tools"];
const LICOF_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("paths/root", DEFAULT_ROOT),
    default_string("paths/rootfs", DEFAULT_ROOTFS),
    default_string("paths/cache", DEFAULT_CACHE),
    default_string("paths/db", DEFAULT_DB),
    default_string("paths/installed_db", DEFAULT_INSTALLED_DB),
    default_string("apt/base_url", DEFAULT_APT_BASE),
    default_string("apt/suite", DEFAULT_APT_DIST),
    default_string("apt/component", DEFAULT_APT_COMPONENT),
    default_string("apt/arch", DEFAULT_APT_ARCH),
    default_string(
        "apt/index_required_packages_csv",
        DEFAULT_INDEX_REQUIRED_PACKAGES,
    ),
    default_int("apt/download_attempts", DEFAULT_DOWNLOAD_ATTEMPTS),
    default_string("bootstrap/packages_csv", DEFAULT_BOOTSTRAP_SEED),
    default_string("tools/wget", DEFAULT_WGET),
];
const LICOF_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[
    migration_string(2, "paths/cache", DEFAULT_CACHE),
    migration_string(2, "paths/db", DEFAULT_DB),
    migration_string(2, "paths/installed_db", DEFAULT_INSTALLED_DB),
    migration_string(2, "apt/base_url", DEFAULT_APT_BASE),
    migration_string(2, "apt/suite", DEFAULT_APT_DIST),
    migration_string(
        2,
        "apt/index_required_packages_csv",
        DEFAULT_INDEX_REQUIRED_PACKAGES,
    ),
    migration_string(2, "bootstrap/packages_csv", DEFAULT_BOOTSTRAP_SEED),
    migration_string(
        3,
        "apt/index_required_packages_csv",
        DEFAULT_INDEX_REQUIRED_PACKAGES,
    ),
    migration_string(3, "bootstrap/packages_csv", DEFAULT_BOOTSTRAP_SEED),
    migration_string(4, "bootstrap/packages_csv", DEFAULT_BOOTSTRAP_SEED),
];
const LICOF_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/licof",
    RegistryScope::System,
    4,
    LICOF_DIRS,
    LICOF_DEFAULTS,
    LICOF_MIGRATIONS,
);
const LICOF_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("licof", &LICOF_MANIFEST);

#[derive(Clone)]
pub struct LicoConfig {
    pub root: String,
    pub rootfs: String,
    pub cache: String,
    pub db: String,
    pub installed_db: String,
    pub apt_base: String,
    pub apt_dist: String,
    pub apt_component: String,
    pub apt_arch: String,
    pub wget: String,
    pub download_attempts: u8,
    pub index_required_packages: Vec<String>,
    pub bootstrap_seed: Vec<String>,
}

impl LicoConfig {
    pub fn defaults() -> Self {
        Self {
            root: String::from(DEFAULT_ROOT),
            rootfs: String::from(DEFAULT_ROOTFS),
            cache: String::from(DEFAULT_CACHE),
            db: String::from(DEFAULT_DB),
            installed_db: String::from(DEFAULT_INSTALLED_DB),
            apt_base: String::from(DEFAULT_APT_BASE),
            apt_dist: String::from(DEFAULT_APT_DIST),
            apt_component: String::from(DEFAULT_APT_COMPONENT),
            apt_arch: String::from(DEFAULT_APT_ARCH),
            wget: String::from(DEFAULT_WGET),
            download_attempts: DEFAULT_DOWNLOAD_ATTEMPTS as u8,
            index_required_packages: parse_csv(DEFAULT_INDEX_REQUIRED_PACKAGES),
            bootstrap_seed: parse_csv(DEFAULT_BOOTSTRAP_SEED),
        }
    }

    pub fn load() -> Self {
        let mut cfg = Self::defaults();
        let _ = LICOF_SCHEMA.register();
        cfg.read_confd_strings();
        cfg.read_confd_numbers();
        cfg.normalize();
        cfg
    }

    pub fn package_index_gz(&self) -> String {
        alloc::format!(
            "{}/debian-{}-{}-Packages.gz",
            self.cache,
            self.apt_dist,
            self.apt_arch
        )
    }

    pub fn package_index_txt(&self) -> String {
        alloc::format!(
            "{}/debian-{}-{}-Packages",
            self.cache,
            self.apt_dist,
            self.apt_arch
        )
    }

    pub fn package_index_url(&self) -> String {
        alloc::format!(
            "{}/dists/{}/{}/binary-{}/Packages.gz",
            self.apt_base,
            self.apt_dist,
            self.apt_component,
            self.apt_arch
        )
    }

    fn read_confd_strings(&mut self) {
        read_confd_string("paths/root", &mut self.root);
        read_confd_string("paths/rootfs", &mut self.rootfs);
        read_confd_string("paths/cache", &mut self.cache);
        read_confd_string("paths/db", &mut self.db);
        read_confd_string("paths/installed_db", &mut self.installed_db);
        read_confd_string("apt/base_url", &mut self.apt_base);
        read_confd_string("apt/suite", &mut self.apt_dist);
        read_confd_string("apt/component", &mut self.apt_component);
        read_confd_string("apt/arch", &mut self.apt_arch);
        read_confd_string("tools/wget", &mut self.wget);
        if let Some(v) = LICOF_SCHEMA.read_string("apt/index_required_packages_csv") {
            let parsed = parse_csv(&v);
            if !parsed.is_empty() {
                self.index_required_packages = parsed;
            }
        }
        if let Some(v) = LICOF_SCHEMA.read_string("bootstrap/packages_csv") {
            let parsed = parse_csv(&v);
            if !parsed.is_empty() {
                self.bootstrap_seed = parsed;
            }
        }
    }

    fn read_confd_numbers(&mut self) {
        if let Some(v) = LICOF_SCHEMA.read_i64("apt/download_attempts") {
            if v > 0 && v <= u8::MAX as i64 {
                self.download_attempts = v as u8;
            }
        }
    }

    fn normalize(&mut self) {
        trim_trailing_slashes(&mut self.root);
        trim_trailing_slashes(&mut self.rootfs);
        trim_trailing_slashes(&mut self.cache);
        trim_trailing_slashes(&mut self.db);
        trim_trailing_slashes(&mut self.installed_db);
        trim_trailing_slashes(&mut self.apt_base);
        if self.download_attempts == 0 {
            self.download_attempts = DEFAULT_DOWNLOAD_ATTEMPTS as u8;
        }
    }
}

fn read_confd_string(path: &str, target: &mut String) {
    if let Some(value) = LICOF_SCHEMA.read_string(path) {
        if !value.trim().is_empty() {
            *target = value;
        }
    }
}

fn parse_csv(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    for part in raw.split(',') {
        let value = part.trim();
        if !value.is_empty() {
            values.push(String::from(value));
        }
    }
    values
}

fn trim_trailing_slashes(value: &mut String) {
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
}
