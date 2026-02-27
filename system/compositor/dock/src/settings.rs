//! Dock settings: size, magnification, position.
//!
//! Persisted as key=value pairs. Path: `~/.dock_settings.conf` for normal
//! users, `/System/dock/dock_settings.conf` as fallback (e.g. root).

use alloc::format;
use alloc::string::String;
use alloc::vec;

use anyos_std::fs;

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
}

impl DockSettings {
    /// Default dock settings.
    pub fn default() -> Self {
        Self {
            icon_size: 48,
            magnification: true,
            mag_size: 80,
            position: POS_BOTTOM,
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

/// System fallback path for dock settings.
const SYSTEM_SETTINGS_PATH: &str = "/System/dock/dock_settings.conf";

/// Resolve the dock settings file path.
/// Tries `/Users/<username>/.dock_settings.conf` first,
/// falls back to `/System/dock/dock_settings.conf` (e.g. for root).
pub fn settings_path() -> String {
    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 64];
    let len = anyos_std::process::getusername(uid, &mut name_buf);
    if len != u32::MAX && len > 0 {
        if let Ok(username) = core::str::from_utf8(&name_buf[..len as usize]) {
            let dir = format!("/Users/{}", username);
            let mut stat_buf = [0u32; 7];
            if fs::stat(&dir, &mut stat_buf) == 0 {
                return format!("/Users/{}/.dock_settings.conf", username);
            }
        }
    }
    let mut home_buf = [0u8; 256];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    if hlen != u32::MAX && hlen > 0 {
        if let Ok(home) = core::str::from_utf8(&home_buf[..hlen as usize]) {
            let mut stat_buf = [0u32; 7];
            if fs::stat(home, &mut stat_buf) == 0 {
                return format!("{}/.dock_settings.conf", home);
            }
        }
    }
    String::from(SYSTEM_SETTINGS_PATH)
}

/// Load dock settings from the settings file. Returns defaults on failure.
pub fn load_dock_settings() -> DockSettings {
    let path = settings_path();

    let mut stat_buf = [0u32; 7];
    if fs::stat(&path, &mut stat_buf) != 0 {
        return DockSettings::default();
    }
    let file_size = stat_buf[1] as usize;
    if file_size == 0 || file_size > 1024 {
        return DockSettings::default();
    }

    let fd = fs::open(&path, 0);
    if fd == u32::MAX {
        return DockSettings::default();
    }

    let mut data = vec![0u8; file_size];
    let n = fs::read(fd, &mut data) as usize;
    fs::close(fd);

    if n == 0 {
        return DockSettings::default();
    }

    let text = match core::str::from_utf8(&data[..n]) {
        Ok(s) => s,
        Err(_) => return DockSettings::default(),
    };

    let mut s = DockSettings::default();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "icon_size" => {
                    if let Some(v) = parse_u32(val) { s.icon_size = v; }
                }
                "magnification" => {
                    s.magnification = val == "1" || val == "true";
                }
                "mag_size" => {
                    if let Some(v) = parse_u32(val) { s.mag_size = v; }
                }
                "position" => {
                    if let Some(v) = parse_u32(val) { s.position = v; }
                }
                _ => {}
            }
        }
    }

    s.validate();
    s
}

/// Save dock settings to the settings file.
pub fn save_dock_settings(s: &DockSettings) {
    let path = settings_path();

    let content = format!(
        "icon_size={}\nmagnification={}\nmag_size={}\nposition={}\n",
        s.icon_size,
        if s.magnification { 1 } else { 0 },
        s.mag_size,
        s.position,
    );

    let _ = fs::write_bytes(&path, content.as_bytes());
}

/// Simple u32 parser (no_std).
fn parse_u32(s: &str) -> Option<u32> {
    let mut result: u32 = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    if s.is_empty() { None } else { Some(result) }
}
