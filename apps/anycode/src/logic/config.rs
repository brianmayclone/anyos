use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::{Number, Value};
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};

use crate::util::path;

/// Application settings, loaded from and saved to JSON.
pub struct Config {
    // Display settings
    pub font_size: u32,
    pub line_height: u32,
    pub font_id: u32,
    pub tab_width: u32,
    pub show_line_numbers: bool,
    pub auto_save: bool,
    pub reopen_last_project: bool,
    pub terminal_font_size: u32,
    pub sidebar_width: u32,
    pub output_height: u32,
    // Path settings (auto-discovered on first launch)
    pub settings_path: String,
    pub syntax_dir: String,
    pub plugin_dir: String,
    pub temp_dir: String,
    pub crust_path: String,
    pub ccargo_path: String,
    pub make_path: String,
    pub cc_path: String,
    pub cxx_path: String,
    pub git_path: String,
    pub last_project: String,
    pub recent_projects: Vec<String>,
    pub session_project: String,
    pub session_files: Vec<String>,
    pub session_active_file: String,
}

const DEFAULT_SETTINGS_PATH: &str = "/Users/settings/anycode.json";
const FONT_MONO: u32 = 4;
const ANYCODE_DIRS: &[&str] = &["config"];
const ANYCODE_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] =
    &[default_string("config/settings_json", "")];
const ANYCODE_MIGRATIONS: &[libconf_schema::MigrationStep<'static>] = &[];
const ANYCODE_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/anycode",
    RegistryScope::User,
    1,
    ANYCODE_DIRS,
    ANYCODE_DEFAULTS,
    ANYCODE_MIGRATIONS,
);
const ANYCODE_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("anycode", &ANYCODE_MANIFEST);

impl Config {
    /// Load settings from disk, or return defaults with auto-discovery.
    pub fn load() -> Self {
        let _ = ANYCODE_SCHEMA.register();
        let defaults = Self::defaults();
        let data = match ANYCODE_SCHEMA.read_string("config/settings_json") {
            Some(s) if !s.is_empty() => s,
            _ => {
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
            line_height: json_u32(&val, "line_height", defaults.line_height),
            font_id: json_u32(&val, "font_id", defaults.font_id),
            tab_width: json_u32(&val, "tab_width", defaults.tab_width),
            show_line_numbers: json_bool(&val, "show_line_numbers", defaults.show_line_numbers),
            auto_save: json_bool(&val, "auto_save", defaults.auto_save),
            reopen_last_project: json_bool(
                &val,
                "reopen_last_project",
                defaults.reopen_last_project,
            ),
            terminal_font_size: json_u32(&val, "terminal_font_size", defaults.terminal_font_size),
            sidebar_width: json_u32(&val, "sidebar_width", defaults.sidebar_width),
            output_height: json_u32(&val, "output_height", defaults.output_height),
            settings_path: json_str(&val, "settings_path", DEFAULT_SETTINGS_PATH),
            // Always derive syntax_dir from current bundle (path changes between installs)
            syntax_dir: defaults.syntax_dir,
            plugin_dir: json_str(&val, "plugin_dir", &defaults.plugin_dir),
            temp_dir: json_str(&val, "temp_dir", &defaults.temp_dir),
            crust_path: json_str(&val, "crust_path", ""),
            ccargo_path: json_str(&val, "ccargo_path", ""),
            make_path: json_str(&val, "make_path", ""),
            cc_path: json_str(&val, "cc_path", ""),
            cxx_path: json_str(&val, "cxx_path", ""),
            git_path: json_str(&val, "git_path", ""),
            last_project: json_str(&val, "last_project", ""),
            recent_projects: json_str_array(&val, "recent_projects"),
            session_project: json_str(&val, "session_project", ""),
            session_files: json_str_array(&val, "session_files"),
            session_active_file: json_str(&val, "session_active_file", ""),
        };
        // Re-discover any empty tool paths
        if cfg.crust_path.is_empty()
            || cfg.ccargo_path.is_empty()
            || cfg.make_path.is_empty()
            || cfg.cc_path.is_empty()
            || cfg.cxx_path.is_empty()
            || cfg.git_path.is_empty()
        {
            cfg.auto_discover();
            cfg.save();
        }
        cfg
    }

    /// Save settings to disk.
    pub fn save(&self) {
        let mut obj = Value::new_object();
        obj.set(
            "font_size",
            Value::Number(Number::Int(self.font_size as i64)),
        );
        obj.set(
            "line_height",
            Value::Number(Number::Int(self.line_height as i64)),
        );
        obj.set("font_id", Value::Number(Number::Int(self.font_id as i64)));
        obj.set(
            "tab_width",
            Value::Number(Number::Int(self.tab_width as i64)),
        );
        obj.set("show_line_numbers", Value::Bool(self.show_line_numbers));
        obj.set("auto_save", Value::Bool(self.auto_save));
        obj.set("reopen_last_project", Value::Bool(self.reopen_last_project));
        obj.set(
            "terminal_font_size",
            Value::Number(Number::Int(self.terminal_font_size as i64)),
        );
        obj.set(
            "sidebar_width",
            Value::Number(Number::Int(self.sidebar_width as i64)),
        );
        obj.set(
            "output_height",
            Value::Number(Number::Int(self.output_height as i64)),
        );
        obj.set("settings_path", Value::String(self.settings_path.clone()));
        obj.set("syntax_dir", Value::String(self.syntax_dir.clone()));
        obj.set("plugin_dir", Value::String(self.plugin_dir.clone()));
        obj.set("temp_dir", Value::String(self.temp_dir.clone()));
        obj.set("crust_path", Value::String(self.crust_path.clone()));
        obj.set("ccargo_path", Value::String(self.ccargo_path.clone()));
        obj.set("make_path", Value::String(self.make_path.clone()));
        obj.set("cc_path", Value::String(self.cc_path.clone()));
        obj.set("cxx_path", Value::String(self.cxx_path.clone()));
        obj.set("git_path", Value::String(self.git_path.clone()));
        obj.set("last_project", Value::String(self.last_project.clone()));
        obj.set("recent_projects", json_string_array(&self.recent_projects));
        obj.set(
            "session_project",
            Value::String(self.session_project.clone()),
        );
        obj.set("session_files", json_string_array(&self.session_files));
        obj.set(
            "session_active_file",
            Value::String(self.session_active_file.clone()),
        );
        let json = obj.to_json_string_pretty();
        let _ = ANYCODE_SCHEMA.write_string("config/settings_json", &json);
    }

    pub fn defaults() -> Self {
        // Syntax files ship inside the .app bundle — use CWD (= bundle dir)
        let syntax_dir = bundle_path("syntax");

        Self {
            font_size: 13,
            line_height: 20,
            font_id: FONT_MONO,
            tab_width: 4,
            show_line_numbers: true,
            auto_save: false,
            reopen_last_project: true,
            terminal_font_size: 12,
            sidebar_width: 28,
            output_height: 25,
            settings_path: String::from(DEFAULT_SETTINGS_PATH),
            syntax_dir,
            plugin_dir: String::from("/Libraries/anycode/plugins"),
            temp_dir: String::from("/tmp"),
            crust_path: String::new(),
            ccargo_path: String::new(),
            make_path: String::new(),
            cc_path: String::new(),
            cxx_path: String::new(),
            git_path: String::new(),
            last_project: String::new(),
            recent_projects: Vec::new(),
            session_project: String::new(),
            session_files: Vec::new(),
            session_active_file: String::new(),
        }
    }

    /// Auto-discover paths for tools via PATH environment variable.
    pub fn auto_discover(&mut self) {
        if self.crust_path.is_empty() {
            self.crust_path = find_first_in_path(&["crust", "rustc", "anyrc"]);
        }
        if self.ccargo_path.is_empty() {
            self.ccargo_path = find_first_in_path(&["ccargo", "cargo", "acargo"]);
        }
        if self.make_path.is_empty() {
            self.make_path = find_in_path("make");
        }
        if self.cc_path.is_empty() {
            self.cc_path = find_first_in_path(&["cc", "gcc", "clang"]);
        }
        if self.cxx_path.is_empty() {
            self.cxx_path = find_first_in_path(&["c++", "g++", "clang++"]);
        }
        if self.git_path.is_empty() {
            self.git_path = find_first_in_path(&["agit", "git", "cgit"]);
        }
    }

    /// Check whether git was discovered.
    pub fn has_git(&self) -> bool {
        !self.git_path.is_empty()
    }

    pub fn push_recent_project(&mut self, project: &str) {
        self.recent_projects.retain(|p| p != project);
        self.recent_projects.insert(0, String::from(project));
        if self.recent_projects.len() > 10 {
            self.recent_projects.truncate(10);
        }
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

fn find_first_in_path(names: &[&str]) -> String {
    for name in names {
        let path = find_in_path(name);
        if !path.is_empty() {
            return path;
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

fn json_str_array(val: &Value, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = val[key].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push(String::from(s));
            }
        }
    }
    out
}

fn json_string_array(values: &[String]) -> Value {
    let mut arr = Value::new_array();
    for value in values {
        arr.push(Value::String(value.clone()));
    }
    arr
}
