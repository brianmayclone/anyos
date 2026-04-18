//! Icon and mimetype lookup utilities.
//!
//! Provides path resolution for app icons and file type icons,
//! shared between the dock, finder, and other GUI programs.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::fs;
use crate::ipc;
use crate::process;

/// Base directory for app icons (.ico files).
pub const APP_ICONS_DIR: &str = "/System/media/icons/apps";

/// Default app icon (fallback when no app-specific icon exists).
pub const DEFAULT_APP_ICON: &str = "/System/media/icons/apps/default.ico";

/// Default file icon (fallback when no mimetype-specific icon exists).
pub const DEFAULT_FILE_ICON: &str = "/System/media/icons/default.ico";

/// Folder icon path.
pub const FOLDER_ICON: &str = "/System/media/icons/folder.ico";

/// Registry path containing system mimetype associations.
const MIMETYPES_CONF_PATH: &str = "system/mimetypes/config/associations_blob";
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
const CONFD_PIPE_NAME: &str = "confd";
const CONFD_READ_RETRIES: usize = 40;
const CONFD_READ_CHUNK_SIZE: usize = 512;

/// Path for user mimetype overrides (JSON).
const USER_MIMETYPES_PATH: &str = "/System/user_mimetypes.json";

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
pub struct MimeEntry {
    pub ext: String,
    pub app: String,
    pub icon_path: String,
}

/// A user override: extension -> preferred application path.
pub struct MimeOverride {
    pub ext: String,
    pub app: String,
}

/// A collection of mimetype associations loaded from mimetypes.conf,
/// with optional user overrides from user_mimetypes.json.
pub struct MimeDb {
    entries: Vec<MimeEntry>,
    overrides: Vec<MimeOverride>,
}

impl MimeDb {
    /// Load the mimetype database from /System/mimetypes.conf
    /// and user overrides from /System/user_mimetypes.json.
    pub fn load() -> Self {
        Self {
            entries: load_mimetypes_inner(),
            overrides: load_user_overrides(),
        }
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
    }
}

fn load_mimetypes_inner() -> Vec<MimeEntry> {
    let text = load_system_mimetypes_text();

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

fn load_system_mimetypes_text() -> &'static str {
    let dynamic = read_confd_string(MIMETYPES_CONF_PATH);
    if let Some(text) = dynamic {
        return Box::leak(text.into_boxed_str());
    }
    MIMETYPES_DEFAULTS
}

fn read_confd_string(path: &str) -> Option<String> {
    let req_pipe = ipc::pipe_open(CONFD_PIPE_NAME);
    if req_pipe == 0 {
        return None;
    }

    let tid = libsyscall::get_tid();
    let reply_name = alloc::format!("confd-{}", tid);
    let reply_pipe = ipc::pipe_create(&reply_name);
    if reply_pipe == 0 {
        return None;
    }

    let mut command = alloc::format!("{}\tHELLO icons\n", tid);
    if ipc::pipe_write(req_pipe, command.as_bytes()) == 0 {
        ipc::pipe_close(reply_pipe);
        return None;
    }
    if read_reply_line(reply_pipe).is_none() {
        ipc::pipe_close(reply_pipe);
        return None;
    }

    command = alloc::format!("{}\tGET system {}\n", tid, path);
    if ipc::pipe_write(req_pipe, command.as_bytes()) == 0 {
        ipc::pipe_close(reply_pipe);
        return None;
    }

    let line = read_reply_line(reply_pipe);
    ipc::pipe_close(reply_pipe);
    parse_item_string(&line?)
}

fn read_reply_line(reply_pipe: u32) -> Option<String> {
    let mut data = Vec::new();
    let mut chunk = [0u8; CONFD_READ_CHUNK_SIZE];

    for _ in 0..CONFD_READ_RETRIES {
        let n = ipc::pipe_read(reply_pipe, &mut chunk);
        if n == u32::MAX {
            return None;
        }
        if n > 0 {
            data.extend_from_slice(&chunk[..n as usize]);
            if let Some(newline) = data.iter().position(|&b| b == b'\n') {
                return String::from_utf8(data[..newline].to_vec()).ok();
            }
        }
        process::sleep(10);
    }

    None
}

fn parse_item_string(line: &str) -> Option<String> {
    if line.starts_with("ERR ") {
        return None;
    }

    let mut parts = line.splitn(8, ' ');
    if parts.next()? != "ITEM" {
        return None;
    }
    let _scope = parts.next()?;
    let _path = parts.next()?;
    let kind = parts.next()?;
    let value_type = parts.next()?;
    let value_text = parts.next()?;
    let _version = parts.next()?;
    let _updated_at = parts.next()?;

    if kind != "value" || value_type != "string" {
        return None;
    }

    decode_conf_value(value_text)
}

fn decode_conf_value(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = decode_hex(bytes[i + 1])?;
            let lo = decode_hex(bytes[i + 2])?;
            out.push((hi << 4 | lo) as char);
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(out)
}

fn decode_hex(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
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
        if n == 0 || n == u32::MAX { break; }
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
