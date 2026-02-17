//! Icon and mimetype lookup utilities.
//!
//! Provides path resolution for app icons and file type icons,
//! shared between the dock, finder, and other GUI programs.

use alloc::string::String;
use alloc::vec::Vec;
use crate::fs;

/// Base directory for app icons (.ico files).
pub const APP_ICONS_DIR: &str = "/System/media/icons/apps";

/// Default app icon (fallback when no app-specific icon exists).
pub const DEFAULT_APP_ICON: &str = "/System/media/icons/apps/default.ico";

/// Default file icon (fallback when no mimetype-specific icon exists).
pub const DEFAULT_FILE_ICON: &str = "/System/media/icons/default.ico";

/// Folder icon path.
pub const FOLDER_ICON: &str = "/System/media/icons/folder.ico";

/// Mimetype configuration file path.
const MIMETYPES_CONF: &str = "/System/mimetypes.conf";

/// Check if a path refers to a .app bundle (directory ending in `.app`).
pub fn is_app_bundle(path: &str) -> bool {
    path.ends_with(".app")
}

/// Read the display name from a .app bundle's Info.conf.
/// Falls back to the folder name minus ".app" if Info.conf is missing or has no `name=` key.
pub fn app_bundle_name(bundle_path: &str) -> String {
    // Try reading Info.conf
    let mut conf_path = String::from(bundle_path);
    conf_path.push_str("/Info.conf");

    let fd = fs::open(&conf_path, 0);
    if fd != u32::MAX {
        let mut buf = [0u8; 512];
        let n = fs::read(fd, &mut buf);
        fs::close(fd);
        if n > 0 && n != u32::MAX {
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                for line in text.split('\n') {
                    let line = line.trim();
                    if let Some(rest) = line.strip_prefix("name=") {
                        if !rest.is_empty() {
                            return String::from(rest);
                        }
                    }
                }
            }
        }
    }

    // Fallback: derive from folder name
    let folder = bundle_path.rsplit('/').next().unwrap_or(bundle_path);
    if let Some(name) = folder.strip_suffix(".app") {
        String::from(name)
    } else {
        String::from(folder)
    }
}

/// Derive the app icon path for a binary or .app bundle path.
///
/// For .app bundles: checks for `Icon.ico` inside the bundle directory.
/// For regular binaries: returns `/System/media/icons/apps/{basename}.ico`.
/// Falls back to `DEFAULT_APP_ICON` if no icon file exists.
pub fn app_icon_path(bin_path: &str) -> String {
    // .app bundle: look for Icon.ico inside the bundle
    if bin_path.ends_with(".app") {
        let mut path = String::from(bin_path);
        path.push_str("/Icon.ico");
        let mut stat_buf = [0u32; 6];
        if fs::stat(&path, &mut stat_buf) == 0 {
            return path;
        }
        return String::from(DEFAULT_APP_ICON);
    }

    // Path inside a .app bundle (e.g. "/Applications/Calc.app/Calc")
    if let Some(pos) = bin_path.find(".app/") {
        let bundle_dir = &bin_path[..pos + 4];
        let mut path = String::from(bundle_dir);
        path.push_str("/Icon.ico");
        let mut stat_buf = [0u32; 6];
        if fs::stat(&path, &mut stat_buf) == 0 {
            return path;
        }
        return String::from(DEFAULT_APP_ICON);
    }

    // Regular binary: /System/media/icons/apps/{basename}.ico
    let basename = match bin_path.rfind('/') {
        Some(pos) if pos + 1 < bin_path.len() => &bin_path[pos + 1..],
        _ => bin_path,
    };

    if basename.is_empty() {
        return String::from(DEFAULT_APP_ICON);
    }

    let mut path = String::from(APP_ICONS_DIR);
    path.push('/');
    path.push_str(basename);
    path.push_str(".ico");

    // Check if the file exists
    let mut stat_buf = [0u32; 6];
    if fs::stat(&path, &mut stat_buf) == 0 {
        path
    } else {
        String::from(DEFAULT_APP_ICON)
    }
}

/// A parsed mimetype association entry.
pub struct MimeEntry {
    pub ext: String,
    pub app: String,
    pub icon_path: String,
}

/// A collection of mimetype associations loaded from mimetypes.conf.
pub struct MimeDb {
    entries: Vec<MimeEntry>,
}

impl MimeDb {
    /// Load the mimetype database from /System/mimetypes.conf.
    pub fn load() -> Self {
        Self { entries: load_mimetypes_inner() }
    }

    /// Look up a mimetype entry by file extension (e.g. "txt", "png").
    pub fn lookup(&self, ext: &str) -> Option<&MimeEntry> {
        self.entries.iter().find(|e| e.ext == ext)
    }

    /// Look up the icon path for a file extension.
    /// Returns the mimetype icon path if found, otherwise `DEFAULT_FILE_ICON`.
    pub fn icon_for_ext(&self, ext: &str) -> &str {
        match self.lookup(ext) {
            Some(entry) if !entry.icon_path.is_empty() => &entry.icon_path,
            _ => DEFAULT_FILE_ICON,
        }
    }

    /// Look up the application path for a file extension.
    pub fn app_for_ext(&self, ext: &str) -> Option<&str> {
        match self.lookup(ext) {
            Some(entry) if !entry.app.is_empty() => Some(&entry.app),
            _ => None,
        }
    }
}

fn load_mimetypes_inner() -> Vec<MimeEntry> {
    let fd = fs::open(MIMETYPES_CONF, 0);
    if fd == u32::MAX {
        return Vec::new();
    }

    let mut data = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);

    let text = match core::str::from_utf8(&data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(sep) = line.find('|') {
            let ext = line[..sep].trim();
            let rest = &line[sep + 1..];
            let (app, icon_path) = if let Some(sep2) = rest.find('|') {
                (rest[..sep2].trim(), rest[sep2 + 1..].trim())
            } else {
                (rest.trim(), "")
            };
            if !ext.is_empty() {
                entries.push(MimeEntry {
                    ext: String::from(ext),
                    app: String::from(app),
                    icon_path: String::from(icon_path),
                });
            }
        }
    }
    entries
}
