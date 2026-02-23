use alloc::string::String;
use alloc::format;
use anyos_std::json::{Value, Number};

use crate::util::path;

/// Application settings, loaded from and saved to JSON.
pub struct Config {
    // Display settings
    pub font_size: u32,
    pub font_id: u32,
    pub tab_width: u32,
    pub show_line_numbers: bool,
    pub sidebar_width: u32,
    pub output_height: u32,
    // Path settings (auto-discovered on first launch)
    pub settings_path: String,
    pub syntax_dir: String,
    pub plugin_dir: String,
    pub temp_dir: String,
    pub make_path: String,
    pub cc_path: String,
    pub git_path: String,
}

const DEFAULT_SETTINGS_PATH: &str = "/Users/settings/anycode.json";
const FONT_MONO: u32 = 4;

impl Config {
    /// Load settings from disk, or return defaults with auto-discovery.
    pub fn load() -> Self {
        let defaults = Self::defaults();
        let data = match anyos_std::fs::read_to_string(DEFAULT_SETTINGS_PATH) {
            Ok(s) => s,
            Err(_) => {
                let mut cfg = defaults;
                cfg.auto_discover();
                cfg.save();
                return cfg;
            }
        };
        let val = match Value::parse(&data) {
            Ok(v) => v,
            Err(_) => {
                let mut cfg = defaults;
                cfg.auto_discover();
                cfg.save();
                return cfg;
            }
        };
        let mut cfg = Self {
            font_size: json_u32(&val, "font_size", defaults.font_size),
            font_id: json_u32(&val, "font_id", defaults.font_id),
            tab_width: json_u32(&val, "tab_width", defaults.tab_width),
            show_line_numbers: json_bool(&val, "show_line_numbers", defaults.show_line_numbers),
            sidebar_width: json_u32(&val, "sidebar_width", defaults.sidebar_width),
            output_height: json_u32(&val, "output_height", defaults.output_height),
            settings_path: json_str(&val, "settings_path", DEFAULT_SETTINGS_PATH),
            // Always derive syntax_dir from current bundle (path changes between installs)
            syntax_dir: defaults.syntax_dir,
            plugin_dir: json_str(&val, "plugin_dir", &defaults.plugin_dir),
            temp_dir: json_str(&val, "temp_dir", &defaults.temp_dir),
            make_path: json_str(&val, "make_path", ""),
            cc_path: json_str(&val, "cc_path", ""),
            git_path: json_str(&val, "git_path", ""),
        };
        // Re-discover any empty tool paths
        if cfg.make_path.is_empty() || cfg.cc_path.is_empty() || cfg.git_path.is_empty() {
            cfg.auto_discover();
            cfg.save();
        }
        cfg
    }

    /// Save settings to disk.
    pub fn save(&self) {
        let mut obj = Value::new_object();
        obj.set("font_size", Value::Number(Number::Int(self.font_size as i64)));
        obj.set("font_id", Value::Number(Number::Int(self.font_id as i64)));
        obj.set("tab_width", Value::Number(Number::Int(self.tab_width as i64)));
        obj.set("show_line_numbers", Value::Bool(self.show_line_numbers));
        obj.set("sidebar_width", Value::Number(Number::Int(self.sidebar_width as i64)));
        obj.set("output_height", Value::Number(Number::Int(self.output_height as i64)));
        obj.set("settings_path", Value::String(self.settings_path.clone()));
        obj.set("syntax_dir", Value::String(self.syntax_dir.clone()));
        obj.set("plugin_dir", Value::String(self.plugin_dir.clone()));
        obj.set("temp_dir", Value::String(self.temp_dir.clone()));
        obj.set("make_path", Value::String(self.make_path.clone()));
        obj.set("cc_path", Value::String(self.cc_path.clone()));
        obj.set("git_path", Value::String(self.git_path.clone()));
        let json = obj.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(DEFAULT_SETTINGS_PATH, json.as_bytes());
    }

    pub fn defaults() -> Self {
        // Syntax files ship inside the .app bundle — use CWD (= bundle dir)
        let syntax_dir = bundle_path("syntax");

        Self {
            font_size: 13,
            font_id: FONT_MONO,
            tab_width: 4,
            show_line_numbers: true,
            sidebar_width: 28,
            output_height: 25,
            settings_path: String::from(DEFAULT_SETTINGS_PATH),
            syntax_dir,
            plugin_dir: String::from("/Libraries/anycode/plugins"),
            temp_dir: String::from("/tmp"),
            make_path: String::new(),
            cc_path: String::new(),
            git_path: String::new(),
        }
    }

    /// Auto-discover paths for tools via PATH environment variable.
    pub fn auto_discover(&mut self) {
        if self.make_path.is_empty() {
            self.make_path = find_in_path("make");
        }
        if self.cc_path.is_empty() {
            self.cc_path = find_in_path("cc");
        }
        if self.git_path.is_empty() {
            self.git_path = find_in_path("git");
        }
    }

    /// Check whether git was discovered.
    pub fn has_git(&self) -> bool {
        !self.git_path.is_empty()
    }
}

/// Get the app bundle directory (CWD at startup, set by kernel from Info.conf working_dir=bundle).
fn bundle_dir() -> String {
    let mut buf = [0u8; 256];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len > 0 && len < 256 {
        if let Ok(s) = core::str::from_utf8(&buf[..len as usize]) {
            return String::from(s);
        }
    }
    String::from("/Applications/anyOS Code.app")
}

/// Build a path relative to the app bundle directory.
pub fn bundle_path(relative: &str) -> String {
    let base = bundle_dir();
    if base.ends_with('/') {
        format!("{}{}", base, relative)
    } else {
        format!("{}/{}", base, relative)
    }
}

/// Well-known system directories to search for binaries.
const SYSTEM_DIRS: &[&str] = &["/bin", "/System/bin", "/usr/bin"];

/// Public wrapper for find_in_path (used by build rules).
pub fn find_tool(name: &str) -> String {
    find_in_path(name)
}

/// Search for a binary by name using the PATH environment variable
/// and well-known system directories.
fn find_in_path(name: &str) -> String {
    // First: search PATH
    let mut path_buf = [0u8; 256];
    let len = anyos_std::env::get("PATH", &mut path_buf);
    if len != u32::MAX && (len as usize) < path_buf.len() {
        if let Ok(path_str) = core::str::from_utf8(&path_buf[..len as usize]) {
            for dir in path_str.split(':') {
                let dir = dir.trim();
                if dir.is_empty() {
                    continue;
                }
                let candidate = format!("{}/{}", dir, name);
                if path::exists(&candidate) {
                    return candidate;
                }
            }
        }
    }
    // Fallback: check well-known system directories
    for dir in SYSTEM_DIRS {
        let candidate = format!("{}/{}", dir, name);
        if path::exists(&candidate) {
            return candidate;
        }
    }
    String::new()
}

fn json_u32(val: &Value, key: &str, default: u32) -> u32 {
    match &val[key] {
        Value::Number(Number::Int(n)) => *n as u32,
        Value::Number(Number::Float(f)) => *f as u32,
        _ => default,
    }
}

fn json_bool(val: &Value, key: &str, default: bool) -> bool {
    match &val[key] {
        Value::Bool(b) => *b,
        _ => default,
    }
}

fn json_str(val: &Value, key: &str, default: &str) -> String {
    match &val[key] {
        Value::String(s) => s.clone(),
        _ => String::from(default),
    }
}
