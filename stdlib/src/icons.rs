//! Icon and mimetype lookup utilities.
//!
//! Provides path resolution for app icons and file type icons,
//! shared between the dock, finder, and other GUI programs.

use alloc::string::String;
use alloc::vec::Vec;
use crate::fs;

/// Base directory for app icons (.ico files).
pub const APP_ICONS_DIR: &str = "/system/media/icons/apps";

/// Default app icon (fallback when no app-specific icon exists).
pub const DEFAULT_APP_ICON: &str = "/system/media/icons/apps/default.ico";

/// Default file icon (fallback when no mimetype-specific icon exists).
pub const DEFAULT_FILE_ICON: &str = "/system/media/icons/default.ico";

/// Folder icon path.
pub const FOLDER_ICON: &str = "/system/media/icons/folder.ico";

/// Mimetype configuration file path.
const MIMETYPES_CONF: &str = "/system/mimetypes.conf";

/// Derive the app icon path for a binary.
///
/// Given a binary path like `/system/finder`, extracts the basename
/// and returns `/system/media/icons/apps/finder.ico`.
/// Falls back to `DEFAULT_APP_ICON` if the specific icon file does not exist.
pub fn app_icon_path(bin_path: &str) -> String {
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
    let mut stat_buf = [0u32; 2];
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
    /// Load the mimetype database from /system/mimetypes.conf.
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
