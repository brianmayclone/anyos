//! Dock settings: size, magnification, position.
use libconf_schema::{default_bool, default_int, manifest, RegistryScope, ServiceSchema};

/// Dock position constants.
pub const POS_BOTTOM: u32 = 0;
pub const POS_LEFT: u32 = 1;
pub const POS_RIGHT: u32 = 2;

/// Dock appearance and behavior settings.
pub struct DockSettings {
    /// Icon size in pixels (20..=128).
    pub icon_size: u32,
    /// Whether the magnification (zoom) effect is enabled.
    pub magnification: bool,
    /// Maximum magnified icon size in pixels (must be > icon_size, max 128).
    pub mag_size: u32,
    /// Dock position: 0=bottom, 1=left, 2=right.
    pub position: u32,
    /// Whether the dock should slide out of view until the mouse reaches the edge zone.
    pub auto_hide: bool,
}

impl DockSettings {
    /// Default dock settings.
    pub fn default() -> Self {
        Self {
            icon_size: 48,
            magnification: true,
            mag_size: 80,
            position: POS_BOTTOM,
            auto_hide: false,
        }
    }

    /// Clamp and validate all fields.
    pub fn validate(&mut self) {
        self.icon_size = self.icon_size.clamp(20, 128);
        let min_mag = self.icon_size + 1;
        if min_mag > 128 {
            // icon_size is already 128; magnification has no effect
            self.mag_size = 128;
        } else {
            self.mag_size = self.mag_size.clamp(min_mag, 128);
        }
        if self.position > POS_RIGHT {
            self.position = POS_BOTTOM;
        }
    }
}

const DOCK_SETTINGS_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/icon_size", 48),
    default_bool("config/magnification", true),
    default_int("config/mag_size", 80),
    default_int("config/position", 0),
    default_bool("config/auto_hide", false),
];
const DOCK_SETTINGS_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const DOCK_SETTINGS_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "profile/dock_settings",
    RegistryScope::User,
    1,
    &["config"],
    DOCK_SETTINGS_DEFAULTS,
    DOCK_SETTINGS_MIGRATIONS,
);

fn dock_settings_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("dock", &DOCK_SETTINGS_MANIFEST)
}

/// Load dock settings from the settings file. Returns defaults on failure.
pub fn load_dock_settings() -> DockSettings {
    let _ = dock_settings_schema().register();
    let mut confd = DockSettings::default();
    let mut found = false;
    if let Some(v) = dock_settings_schema().read_i64("config/icon_size") {
        confd.icon_size = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_bool("config/magnification") {
        confd.magnification = v;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_i64("config/mag_size") {
        confd.mag_size = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_i64("config/position") {
        confd.position = v.max(0) as u32;
        found = true;
    }
    if let Some(v) = dock_settings_schema().read_bool("config/auto_hide") {
        confd.auto_hide = v;
        found = true;
    }
    confd.validate();
    confd
}

/// Save dock settings to the settings file.
pub fn save_dock_settings(s: &DockSettings) {
    let _ = dock_settings_schema().register();
    let _ = dock_settings_schema().write_i64("config/icon_size", s.icon_size as i64);
    let _ = dock_settings_schema().write_bool("config/magnification", s.magnification);
    let _ = dock_settings_schema().write_i64("config/mag_size", s.mag_size as i64);
    let _ = dock_settings_schema().write_i64("config/position", s.position as i64);
    let _ = dock_settings_schema().write_bool("config/auto_hide", s.auto_hide);
}
