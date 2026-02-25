//! Config file parsers and serializers for different formats.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::json::Value;

use crate::types::{ColumnSpec, ConfigFormat};

// ── File I/O helpers ────────────────────────────────────────────────────────

/// Read a config file as text. Returns None if file doesn't exist or is empty.
pub fn load_config_file(path: &str) -> Option<String> {
    let fd = fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    fs::close(fd);
    if data.is_empty() {
        return None;
    }
    core::str::from_utf8(&data).ok().map(String::from)
}

/// Write content to a config file. Creates parent directories if needed.
pub fn save_config_file(path: &str, content: &str) -> bool {
    // Ensure parent directory exists
    if let Some(slash) = path.rfind('/') {
        let parent = &path[..slash];
        if !parent.is_empty() {
            fs::mkdir(parent);
        }
    }
    fs::write_bytes(path, content.as_bytes()).is_ok()
}

// ── Load config into Value based on format ──────────────────────────────────

pub fn load_config(path: &str, format: ConfigFormat, columns: &[ColumnSpec]) -> Value {
    let text = match load_config_file(path) {
        Some(t) => t,
        None => return Value::Null,
    };
    match format {
        ConfigFormat::KeyValue => parse_keyvalue(&text),
        ConfigFormat::Pipe => parse_pipe(&text, columns),
        ConfigFormat::Crontab => parse_crontab(&text),
        ConfigFormat::Json => Value::parse(&text).unwrap_or(Value::Null),
    }
}

/// Serialize values and write to file.
pub fn save_config(path: &str, format: ConfigFormat, values: &Value, columns: &[ColumnSpec]) -> bool {
    let content = match format {
        ConfigFormat::KeyValue => serialize_keyvalue(values),
        ConfigFormat::Pipe => serialize_pipe(values, columns),
        ConfigFormat::Crontab => serialize_crontab(values),
        ConfigFormat::Json => values.to_json_string_pretty(),
    };
    save_config_file(path, &content)
}

// ── Key-Value format (key=value lines, # comments) ─────────────────────────

pub fn parse_keyvalue(text: &str) -> Value {
    let mut obj = Value::new_object();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let val = line[eq_pos + 1..].trim();
            if key.is_empty() {
                continue;
            }
            // Try to parse as bool or number, fallback to string
            if val == "true" {
                obj.set(key, true.into());
            } else if val == "false" {
                obj.set(key, false.into());
            } else if let Some(n) = try_parse_i64(val) {
                obj.set(key, n.into());
            } else {
                obj.set(key, val.into());
            }
        }
    }
    obj
}

pub fn serialize_keyvalue(values: &Value) -> String {
    let mut out = String::new();
    if let Some(obj) = values.as_object() {
        for (key, val) in obj.iter() {
            out.push_str(key);
            out.push('=');
            out.push_str(&value_to_string(val));
            out.push('\n');
        }
    }
    out
}

// ── Pipe format (field1|field2 per line) ────────────────────────────────────

pub fn parse_pipe(text: &str, columns: &[ColumnSpec]) -> Value {
    let mut arr = Value::new_array();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.splitn(columns.len().max(1), '|').collect();
        let mut row = Value::new_object();
        for (i, col) in columns.iter().enumerate() {
            let val = parts.get(i).map(|s| s.trim()).unwrap_or("");
            row.set(&col.key, val.into());
        }
        arr.push(row);
    }
    arr
}

pub fn serialize_pipe(values: &Value, columns: &[ColumnSpec]) -> String {
    let mut out = String::new();
    if let Some(arr) = values.as_array() {
        for row in arr {
            for (i, col) in columns.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                let val = row[col.key.as_str()].as_str().unwrap_or("");
                out.push_str(val);
            }
            out.push('\n');
        }
    }
    out
}

// ── Crontab format (min hour day month weekday command) ─────────────────────

pub fn parse_crontab(text: &str) -> Value {
    let mut arr = Value::new_array();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Extract first 5 space-separated fields, rest is command
        let mut fields: Vec<&str> = Vec::new();
        let mut rest = line;
        for _ in 0..5 {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            let end = rest
                .find(|c: char| c == ' ' || c == '\t')
                .unwrap_or(rest.len());
            fields.push(&rest[..end]);
            rest = &rest[end..];
        }
        if fields.len() < 5 {
            continue;
        }
        let command = rest.trim_start();
        if command.is_empty() {
            continue;
        }

        let mut row = Value::new_object();
        row.set("minute", fields[0].into());
        row.set("hour", fields[1].into());
        row.set("day", fields[2].into());
        row.set("month", fields[3].into());
        row.set("weekday", fields[4].into());
        row.set("command", command.into());
        arr.push(row);
    }
    arr
}

pub fn serialize_crontab(values: &Value) -> String {
    let mut out = String::from("# Crontab — managed by Settings\n");
    if let Some(arr) = values.as_array() {
        for row in arr {
            let min = row["minute"].as_str().unwrap_or("*");
            let hour = row["hour"].as_str().unwrap_or("*");
            let day = row["day"].as_str().unwrap_or("*");
            let month = row["month"].as_str().unwrap_or("*");
            let wday = row["weekday"].as_str().unwrap_or("*");
            let cmd = row["command"].as_str().unwrap_or("");
            out.push_str(&format!("{} {} {} {} {} {}\n", min, hour, day, month, wday, cmd));
        }
    }
    out
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn try_parse_i64(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let (neg, digits) = if s.starts_with('-') {
        (true, &s[1..])
    } else {
        (false, s)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut val: i64 = 0;
    for b in digits.bytes() {
        val = val.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    if neg {
        Some(-val)
    } else {
        Some(val)
    }
}

fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => format!("{}", n),
        Value::Bool(b) => String::from(if *b { "true" } else { "false" }),
        _ => val.to_json_string(),
    }
}
