use alloc::string::String;
use libconf_schema::{default_string, manifest, migration_string, RegistryScope, ServiceSchema};

const DEFAULT_ROOT: &str = "/System/var/wxe";
const DEFAULT_DRIVE_C: &str = "/System/var/wxe/drive_c";
const DEFAULT_CACHE: &str = "/System/var/wxe/cache";
const DEFAULT_DB: &str = "/System/var/wxe/db";
const DEFAULT_PROFILE: &str = "win10_19041";
const DEFAULT_DRIVE_Z: &str = "";
const DEFAULT_SHELL_DRIVE: &str = "C:";
const DEFAULT_SHELL_CWD: &str = "\\Users\\Default";
const DEFAULT_COMSPEC: &str = "C:\\Windows\\System32\\cmd.exe";

const WXE_DIRS: &[&str] = &["paths", "profile", "drives", "shell"];
const WXE_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("paths/root", DEFAULT_ROOT),
    default_string("paths/drive_c", DEFAULT_DRIVE_C),
    default_string("paths/cache", DEFAULT_CACHE),
    default_string("paths/db", DEFAULT_DB),
    default_string("profile/nt", DEFAULT_PROFILE),
    default_string("drives/c", DEFAULT_DRIVE_C),
    default_string("drives/z", DEFAULT_DRIVE_Z),
    default_string("shell/default_drive", DEFAULT_SHELL_DRIVE),
    default_string("shell/default_cwd", DEFAULT_SHELL_CWD),
    default_string("shell/comspec", DEFAULT_COMSPEC),
];
const WXE_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[
    migration_string(1, "paths/root", DEFAULT_ROOT),
    migration_string(1, "paths/drive_c", DEFAULT_DRIVE_C),
    migration_string(1, "paths/cache", DEFAULT_CACHE),
    migration_string(1, "paths/db", DEFAULT_DB),
    migration_string(1, "profile/nt", DEFAULT_PROFILE),
    migration_string(1, "drives/c", DEFAULT_DRIVE_C),
    migration_string(1, "drives/z", DEFAULT_DRIVE_Z),
    migration_string(1, "shell/default_drive", DEFAULT_SHELL_DRIVE),
    migration_string(1, "shell/default_cwd", DEFAULT_SHELL_CWD),
    migration_string(1, "shell/comspec", DEFAULT_COMSPEC),
];
const WXE_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/wxe",
    RegistryScope::System,
    1,
    WXE_DIRS,
    WXE_DEFAULTS,
    WXE_MIGRATIONS,
);
const WXE_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("wxe", &WXE_MANIFEST);

#[derive(Clone)]
pub struct WxeConfig {
    pub root: String,
    pub drive_c: String,
    pub cache: String,
    pub db: String,
    pub nt_profile: String,
    pub drive_z: String,
    pub default_drive: String,
    pub default_cwd: String,
    pub comspec: String,
}

impl WxeConfig {
    pub fn defaults() -> Self {
        Self {
            root: String::from(DEFAULT_ROOT),
            drive_c: String::from(DEFAULT_DRIVE_C),
            cache: String::from(DEFAULT_CACHE),
            db: String::from(DEFAULT_DB),
            nt_profile: String::from(DEFAULT_PROFILE),
            drive_z: String::from(DEFAULT_DRIVE_Z),
            default_drive: String::from(DEFAULT_SHELL_DRIVE),
            default_cwd: String::from(DEFAULT_SHELL_CWD),
            comspec: String::from(DEFAULT_COMSPEC),
        }
    }

    pub fn load() -> Self {
        let mut cfg = Self::defaults();
        let _ = WXE_SCHEMA.register();
        read_confd_string("paths/root", &mut cfg.root);
        read_confd_string("paths/drive_c", &mut cfg.drive_c);
        read_confd_string("paths/cache", &mut cfg.cache);
        read_confd_string("paths/db", &mut cfg.db);
        read_confd_string("profile/nt", &mut cfg.nt_profile);
        read_confd_string("drives/c", &mut cfg.drive_c);
        read_confd_string("drives/z", &mut cfg.drive_z);
        read_confd_string("shell/default_drive", &mut cfg.default_drive);
        read_confd_string("shell/default_cwd", &mut cfg.default_cwd);
        read_confd_string("shell/comspec", &mut cfg.comspec);
        trim_trailing_slashes(&mut cfg.root);
        trim_trailing_slashes(&mut cfg.drive_c);
        trim_trailing_slashes(&mut cfg.cache);
        trim_trailing_slashes(&mut cfg.db);
        trim_trailing_slashes(&mut cfg.drive_z);
        cfg
    }

    pub fn system32(&self) -> String {
        alloc::format!("{}/Windows/System32", self.drive_c)
    }
}

fn read_confd_string(path: &str, target: &mut String) {
    if let Some(value) = WXE_SCHEMA.read_string(path) {
        if !value.trim().is_empty() {
            *target = value;
        }
    }
}

fn trim_trailing_slashes(value: &mut String) {
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
}
