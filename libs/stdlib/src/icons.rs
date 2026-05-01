//! Icon and mimetype lookup utilities.
//!
//! Provides path resolution for app icons and file type icons,
//! shared between the dock, finder, and other GUI programs.

use crate::fs;
use alloc::string::String;
use alloc::vec::Vec;

/// Base directory for app icons (.ico files).
pub const APP_ICONS_DIR: &str = "/System/media/icons/apps";

/// Default app icon (fallback when no app-specific icon exists).
pub const DEFAULT_APP_ICON: &str = "/System/media/icons/apps/default.ico";

/// Default file icon (fallback when no mimetype-specific icon exists).
pub const DEFAULT_FILE_ICON: &str = "/System/media/icons/default.ico";

/// Folder icon path.
pub const FOLDER_ICON: &str = "/System/media/icons/folder.ico";
const DIR_ENTRY_SIZE: usize = 64;
const DIR_NAME_OFFSET: usize = 8;
const DIR_NAME_MAX: usize = 56;

const MIMETYPES_DEFAULTS: &str = "\
# anyOS mimetype associations\n\
# Format: extension|application_path|icon_path (icon_path is optional)\n\
txt|/Applications/Notepad.app|/System/media/icons/text.ico\n\
conf|/Applications/Notepad.app|/System/media/icons/config.ico\n\
log|/Applications/Notepad.app|/System/media/icons/text.ico\n\
md|/Applications/Markdown Viewer.app|/System/media/icons/text.ico\n\
ini|/Applications/Notepad.app|/System/media/icons/config.ico\n\
c|/Applications/anyOS Code.app|/System/media/icons/code.ico\n\
cpp|/Applications/anyOS Code.app|/System/media/icons/code.ico\n\
rs|/Applications/anyOS Code.app|/System/media/icons/code.ico\n\
py|/Applications/anyOS Code.app|/System/media/icons/code.ico\n\
js|/Applications/anyOS Code.app|/System/media/icons/code.ico\n\
mjv|/Applications/Video Player.app|/System/media/icons/video.ico\n\
png|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
jpg|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
jpeg|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
bmp|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
gif|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
ico|/Applications/Image Viewer.app|/System/media/icons/image.ico\n\
dlib||/System/media/icons/dll.ico\n\
ttf||/System/media/icons/font.ico\n";

/// Path for user mimetype overrides (JSON).
const USER_MIMETYPES_PATH: &str = "/System/user_mimetypes.json";

/// Check if a path refers to a .app bundle (directory ending in `.app`).
pub fn is_app_bundle(path: &str) -> bool {
    path.ends_with(".app")
}

fn collect_app_bundles_from_dir(dir: &str, out: &mut Vec<String>) {
    let mut buf = [0u8; DIR_ENTRY_SIZE * 128];
    let count = fs::readdir(dir, &mut buf);
    if count == 0 || count == u32::MAX {
        return;
    }

    for i in 0..count as usize {
        let base = i * DIR_ENTRY_SIZE;
        if base + DIR_ENTRY_SIZE > buf.len() || buf[base] != 1 {
            continue;
        }

        let name_len = (buf[base + 1] as usize).min(DIR_NAME_MAX);
        if name_len == 0 {
            continue;
        }
        let Ok(entry_name) =
            core::str::from_utf8(&buf[base + DIR_NAME_OFFSET..base + DIR_NAME_OFFSET + name_len])
        else {
            continue;
        };

        let mut path = String::from(dir);
        if !dir.ends_with('/') {
            path.push('/');
        }
        path.push_str(entry_name);

        if is_app_bundle(entry_name) {
            out.push(path);
        } else {
            collect_app_bundles_from_dir(&path, out);
        }
    }
}

/// Enumerate `.app` bundles under `/Applications`, including nested folders
/// such as `/Applications/Management`.
pub fn collect_app_bundles() -> Vec<String> {
    let mut bundles = Vec::new();
    collect_app_bundles_from_dir("/Applications", &mut bundles);
    bundles
}

/// Find an app bundle by its folder stem (case-insensitive).
pub fn find_app_bundle_by_stem(name: &str) -> Option<String> {
    let name_lower = name.to_ascii_lowercase();
    for bundle_path in collect_app_bundles() {
        let folder = bundle_path
            .rsplit('/')
            .next()
            .unwrap_or(bundle_path.as_str());
        let Some(stem) = folder.strip_suffix(".app") else {
            continue;
        };
        if stem.to_ascii_lowercase() == name_lower {
            return Some(bundle_path);
        }
    }
    None
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
        let mut stat_buf = [0u32; 7];
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
        let mut stat_buf = [0u32; 7];
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
    let mut stat_buf = [0u32; 7];
    if fs::stat(&path, &mut stat_buf) == 0 {
        path
    } else {
        String::from(DEFAULT_APP_ICON)
    }
}

/// A parsed mimetype association entry.
#[derive(Clone)]
pub struct MimeEntry {
    pub ext: String,
    pub app: String,
    pub icon_path: String,
}

/// A user override: extension -> preferred application path.
#[derive(Clone)]
pub struct MimeOverride {
    pub ext: String,
    pub app: String,
}

/// A collection of mimetype associations loaded from mimetypes.conf,
/// with optional user overrides from user_mimetypes.json.
#[derive(Clone)]
pub struct MimeDb {
    entries: Vec<MimeEntry>,
    overrides: Vec<MimeOverride>,
}

static mut MIME_DB_CACHE: Option<MimeDb> = None;

impl MimeDb {
    /// Load the mimetype database from /System/mimetypes.conf
    /// and user overrides from /System/user_mimetypes.json.
    pub fn load() -> Self {
        unsafe {
            if let Some(cache) = MIME_DB_CACHE.as_ref() {
                return cache.clone();
            }
        }

        let db = Self {
            entries: load_mimetypes_inner(),
            overrides: load_user_overrides(),
        };

        unsafe {
            MIME_DB_CACHE = Some(db.clone());
        }

        db
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
    /// User overrides take priority over system defaults.
    pub fn app_for_ext(&self, ext: &str) -> Option<&str> {
        // Check user overrides first
        if let Some(ovr) = self.overrides.iter().find(|o| o.ext == ext) {
            if !ovr.app.is_empty() {
                return Some(&ovr.app);
            }
        }
        // Fall back to system default
        match self.lookup(ext) {
            Some(entry) if !entry.app.is_empty() => Some(&entry.app),
            _ => None,
        }
    }

    /// Set a user override for an extension. Persists to disk.
    pub fn set_user_default(&mut self, ext: &str, app_path: &str) {
        if let Some(ovr) = self.overrides.iter_mut().find(|o| o.ext == ext) {
            ovr.app = String::from(app_path);
        } else {
            self.overrides.push(MimeOverride {
                ext: String::from(ext),
                app: String::from(app_path),
            });
        }
        save_user_overrides(&self.overrides);
        unsafe {
            MIME_DB_CACHE = Some(self.clone());
        }
    }
}

fn load_mimetypes_inner() -> Vec<MimeEntry> {
    let text = String::from(MIMETYPES_DEFAULTS);

    let mut entries = Vec::new();
    for line in text.as_str().split('\n') {
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

fn load_user_overrides() -> Vec<MimeOverride> {
    let fd = fs::open(USER_MIMETYPES_PATH, 0);
    if fd == u32::MAX {
        return Vec::new();
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
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
    let json = match crate::json::Value::parse(text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut overrides = Vec::new();
    if let Some(arr) = json["overrides"].as_array() {
        for item in arr {
            let ext = item["ext"].as_str().unwrap_or("");
            let app = item["app"].as_str().unwrap_or("");
            if !ext.is_empty() {
                overrides.push(MimeOverride {
                    ext: String::from(ext),
                    app: String::from(app),
                });
            }
        }
    }
    overrides
}

fn save_user_overrides(overrides: &[MimeOverride]) {
    use crate::json::Value;

    let mut root = Value::new_object();
    let mut arr = Value::new_array();
    for ovr in overrides {
        let mut obj = Value::new_object();
        obj.set("ext", Value::from(ovr.ext.as_str()));
        obj.set("app", Value::from(ovr.app.as_str()));
        arr.push(obj);
    }
    root.set("overrides", arr);

    let json_str = root.to_json_string_pretty();
    let _ = fs::write_bytes(USER_MIMETYPES_PATH, json_str.as_bytes());
}
