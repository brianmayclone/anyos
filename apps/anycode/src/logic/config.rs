use anyos_std::json::{Value, Number};

/// Application settings, loaded from and saved to JSON.
pub struct Config {
    pub font_size: u32,
    pub font_id: u32,
    pub tab_width: u32,
    pub show_line_numbers: bool,
    pub sidebar_width: u32,
    pub output_height: u32,
}

const SETTINGS_PATH: &str = "/Users/settings/anycode.json";
const FONT_MONO: u32 = 4;

impl Config {
    /// Load settings from disk, or return defaults.
    pub fn load() -> Self {
        let defaults = Self::defaults();
        let data = match anyos_std::fs::read_to_string(SETTINGS_PATH) {
            Ok(s) => s,
            Err(_) => return defaults,
        };
        let val = match Value::parse(&data) {
            Ok(v) => v,
            Err(_) => return defaults,
        };
        Self {
            font_size: json_u32(&val, "font_size", defaults.font_size),
            font_id: json_u32(&val, "font_id", defaults.font_id),
            tab_width: json_u32(&val, "tab_width", defaults.tab_width),
            show_line_numbers: json_bool(&val, "show_line_numbers", defaults.show_line_numbers),
            sidebar_width: json_u32(&val, "sidebar_width", defaults.sidebar_width),
            output_height: json_u32(&val, "output_height", defaults.output_height),
        }
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
        let json = obj.to_json_string_pretty();
        let _ = anyos_std::fs::write_bytes(SETTINGS_PATH, json.as_bytes());
    }

    pub fn defaults() -> Self {
        Self {
            font_size: 13,
            font_id: FONT_MONO,
            tab_width: 4,
            show_line_numbers: true,
            sidebar_width: 22,
            output_height: 25,
        }
    }
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
