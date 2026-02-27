//! Module discovery: built-in registration + JSON descriptor scanning.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs;
use anyos_std::json::Value;

use crate::config_io;
use crate::types::*;

const SETTINGS_DIR: &str = "/System/settings";

// ── Built-in module definitions ─────────────────────────────────────────────

fn builtin_modules() -> Vec<ModuleEntry> {
    let mut m = Vec::new();
    let b = |id: BuiltinId, name: &str, icon: &str, cat: &str, order: i64| ModuleEntry {
        kind: ModuleKind::Builtin(id),
        name: String::from(name),
        icon: String::from(icon),
        category: String::from(cat),
        order,
        panel_id: 0,
        field_ctrl_ids: Vec::new(),
        values: Value::Null,
        dirty: false,
    };
    m.push(b(BuiltinId::Dashboard, "Dashboard", "home", "System", 0));
    m.push(b(BuiltinId::General, "General", "settings", "System", 1));
    m.push(b(BuiltinId::Display, "Display", "device-desktop", "System", 2));
    m.push(b(BuiltinId::Apps, "Apps", "apps", "System", 3));
    m.push(b(BuiltinId::Devices, "Devices", "settings", "System", 4));
    m.push(b(BuiltinId::Network, "Network", "network", "System", 5));
    m.push(b(BuiltinId::Update, "Update", "info-circle", "System", 6));
    m
}

// ── JSON descriptor scanning ────────────────────────────────────────────────

pub fn load_all_modules(uid: u16, username: &str, home: &str) -> Vec<ModuleEntry> {
    let mut modules = builtin_modules();

    // Scan /System/settings/ for .json descriptor files
    let mut dir_buf = [0u8; 64 * 32];
    let count = fs::readdir(SETTINGS_DIR, &mut dir_buf);
    if count != u32::MAX && count > 0 {
        for i in 0..count as usize {
            let entry = &dir_buf[i * 64..(i + 1) * 64];
            let entry_type = entry[0];
            let name_len = entry[1] as usize;
            if entry_type != 0 || name_len == 0 {
                continue;
            }
            let nlen = name_len.min(56);
            let name_bytes = &entry[8..8 + nlen];
            let name = match core::str::from_utf8(name_bytes) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !name.ends_with(".json") {
                continue;
            }

            let path = format!("{}/{}", SETTINGS_DIR, name);
            if let Some(text) = config_io::load_config_file(&path) {
                if let Ok(json) = Value::parse(&text) {
                    if let Some(entry) = parse_descriptor(&json, uid, username, home) {
                        modules.push(entry);
                    }
                }
            }
        }
    }

    // Sort: by category then by order
    sort_modules(&mut modules);
    modules
}

fn parse_descriptor(json: &Value, uid: u16, username: &str, home: &str) -> Option<ModuleEntry> {
    let id = json["id"].as_str()?;
    let name = json["name"].as_str()?;
    let icon = json["icon"].as_str().unwrap_or("settings");
    let category = json["category"].as_str().unwrap_or("Services");
    let order = json["order"].as_i64().unwrap_or(50);
    let description = json["description"].as_str().unwrap_or("");
    let config_path = json["config_path"].as_str().unwrap_or("");
    let on_apply_ipc = json["on_apply_ipc"].as_str().unwrap_or("");

    let config_format = match json["config_format"].as_str().unwrap_or("keyvalue") {
        "keyvalue" => ConfigFormat::KeyValue,
        "pipe" => ConfigFormat::Pipe,
        "crontab" => ConfigFormat::Crontab,
        "json" => ConfigFormat::Json,
        _ => ConfigFormat::KeyValue,
    };

    // Parse fields
    let mut fields = Vec::new();
    if let Some(field_arr) = json["fields"].as_array() {
        for fv in field_arr {
            if let Some(fd) = parse_field(fv) {
                fields.push(fd);
            }
        }
    }

    // Resolve placeholders in config_path
    let resolved_path = resolve_path(config_path, uid, username, home);

    // Get columns for table fields (needed for config loading)
    let columns = first_table_columns(&fields);

    // Load current config values
    let values = config_io::load_config(
        &resolved_path,
        config_format,
        &columns,
    );

    Some(ModuleEntry {
        kind: ModuleKind::External(ModuleDescriptor {
            id: String::from(id),
            name: String::from(name),
            icon: String::from(icon),
            category: String::from(category),
            description: String::from(description),
            config_path: String::from(config_path),
            resolved_path: resolved_path.clone(),
            on_apply_ipc: String::from(on_apply_ipc),
            order,
            config_format,
            fields,
        }),
        name: String::from(name),
        icon: String::from(icon),
        category: String::from(category),
        order,
        panel_id: 0,
        field_ctrl_ids: Vec::new(),
        values,
        dirty: false,
    })
}

fn parse_field(fv: &Value) -> Option<FieldDef> {
    let key = fv["key"].as_str()?;
    let label = fv["label"].as_str().unwrap_or(key);
    let type_str = fv["type"].as_str().unwrap_or("text");
    let default_str = fv["default"].as_str().unwrap_or("");

    let field_type = match type_str {
        "text" => FieldType::Text,
        "number" => FieldType::Number {
            min: fv["min"].as_i64().unwrap_or(i64::MIN),
            max: fv["max"].as_i64().unwrap_or(i64::MAX),
        },
        "bool" => FieldType::Bool,
        "choice" => {
            let mut options = Vec::new();
            if let Some(arr) = fv["options"].as_array() {
                for opt in arr {
                    if let Some(s) = opt.as_str() {
                        options.push(String::from(s));
                    }
                }
            }
            FieldType::Choice { options }
        }
        "path" => FieldType::Path {
            browse_folder: fv["browse"].as_str() == Some("folder"),
        },
        "table" => {
            let mut columns = Vec::new();
            if let Some(arr) = fv["columns"].as_array() {
                for cv in arr {
                    let ck = cv["key"].as_str().unwrap_or("");
                    let cl = cv["label"].as_str().unwrap_or(ck);
                    let cw = cv["width"].as_i64().unwrap_or(100) as u32;
                    let cp = cv["placeholder"].as_str().unwrap_or("");
                    columns.push(ColumnSpec {
                        key: String::from(ck),
                        label: String::from(cl),
                        placeholder: String::from(cp),
                        width: cw,
                    });
                }
            }
            FieldType::Table { columns }
        }
        _ => FieldType::Text,
    };

    Some(FieldDef {
        key: String::from(key),
        label: String::from(label),
        field_type,
        default_str: String::from(default_str),
    })
}

fn resolve_path(template: &str, uid: u16, username: &str, home: &str) -> String {
    let mut result = String::from(template);
    // Replace {uid}
    if result.contains("{uid}") {
        let uid_str = format!("{}", uid);
        result = result.replace("{uid}", &uid_str);
    }
    // Replace {username}
    if result.contains("{username}") {
        result = result.replace("{username}", username);
    }
    // Replace {home}
    if result.contains("{home}") {
        result = result.replace("{home}", home);
    }
    result
}

/// Get columns from the first table field (used for config loading).
fn first_table_columns(fields: &[FieldDef]) -> Vec<ColumnSpec> {
    for f in fields {
        if let FieldType::Table { columns } = &f.field_type {
            return columns
                .iter()
                .map(|c| ColumnSpec {
                    key: c.key.clone(),
                    label: c.label.clone(),
                    placeholder: c.placeholder.clone(),
                    width: c.width,
                })
                .collect();
        }
    }
    Vec::new()
}

/// Simple insertion sort by (category, order).
fn sort_modules(modules: &mut Vec<ModuleEntry>) {
    let n = modules.len();
    for i in 1..n {
        let mut j = i;
        while j > 0 && cmp_modules(&modules[j - 1], &modules[j]) == core::cmp::Ordering::Greater {
            modules.swap(j - 1, j);
            j -= 1;
        }
    }
}

fn cmp_modules(a: &ModuleEntry, b: &ModuleEntry) -> core::cmp::Ordering {
    let cat = a.category.as_str().cmp(&b.category.as_str());
    if cat != core::cmp::Ordering::Equal {
        return cat;
    }
    a.order.cmp(&b.order)
}

/// Resolve the current user's identity for path placeholders.
pub fn resolve_user() -> (u16, String, String) {
    let uid = anyos_std::process::getuid();

    let mut name_buf = [0u8; 64];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    let username = if nlen != u32::MAX && nlen > 0 {
        core::str::from_utf8(&name_buf[..nlen as usize])
            .unwrap_or("root")
            .into()
    } else {
        String::from("root")
    };

    let mut home_buf = [0u8; 256];
    let hlen = anyos_std::env::get("HOME", &mut home_buf);
    let home = if hlen != u32::MAX && hlen > 0 {
        core::str::from_utf8(&home_buf[..hlen as usize])
            .unwrap_or("/tmp")
            .into()
    } else {
        format!("/Users/{}", username)
    };

    (uid, username, home)
}
