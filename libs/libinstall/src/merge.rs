use alloc::string::{String, ToString};
use alloc::vec::Vec;
use anyos_std::format;
use anyos_std::fs;
use anyos_std::json::Value;

use crate::transaction::UpgradeTransaction;
use crate::util::is_kv_config_path;

pub fn merge_config_file(src: &str, dst: &str, tx: &mut UpgradeTransaction) -> Result<(), String> {
    if dst.ends_with(".json") {
        merge_json_file(src, dst, tx)
    } else if is_kv_config_path(dst) {
        merge_kv_config_file(src, dst, tx)
    } else {
        tx.replace_file_from_path(src, dst)
    }
}

fn merge_json_file(src: &str, dst: &str, tx: &mut UpgradeTransaction) -> Result<(), String> {
    let src_text = fs::read_to_string(src).map_err(|_| format!("failed to read {}", src))?;
    let dst_text = fs::read_to_string(dst).map_err(|_| format!("failed to read {}", dst))?;
    let src_val = Value::parse(&src_text).map_err(|_| format!("invalid json {}", src))?;
    let dst_val = Value::parse(&dst_text).map_err(|_| format!("invalid json {}", dst))?;
    let merged = merge_json_values(&src_val, &dst_val);
    tx.replace_file_with_bytes(dst, merged.to_json_string_pretty().as_bytes())
}

fn merge_json_values(defaults: &Value, existing: &Value) -> Value {
    match (defaults, existing) {
        (Value::Object(src), Value::Object(dst)) => {
            let mut merged = Value::new_object();
            for (key, src_val) in src.iter() {
                let next = match dst.get(key) {
                    Some(dst_val) => merge_json_values(src_val, dst_val),
                    None => src_val.clone(),
                };
                merged.set(key, next);
            }
            for (key, dst_val) in dst.iter() {
                if src.get(key).is_none() {
                    merged.set(key, dst_val.clone());
                }
            }
            merged
        }
        (_, Value::Array(_)) => existing.clone(),
        (_, _) => existing.clone(),
    }
}

fn merge_kv_config_file(src: &str, dst: &str, tx: &mut UpgradeTransaction) -> Result<(), String> {
    let src_text = fs::read_to_string(src).map_err(|_| format!("failed to read {}", src))?;
    let dst_text = fs::read_to_string(dst).map_err(|_| format!("failed to read {}", dst))?;
    let src_items = parse_kv_config(&src_text);
    let dst_items = parse_kv_config(&dst_text);
    let mut merged = Vec::new();

    for src_item in &src_items {
        if src_item.kind == ConfigLineKind::Pair {
            if let Some(existing) = dst_items.iter().find(|item| {
                item.kind == ConfigLineKind::Pair
                    && item.section == src_item.section
                    && item.key == src_item.key
            }) {
                merged.push(existing.clone());
            } else {
                merged.push(src_item.clone());
            }
        } else {
            merged.push(src_item.clone());
        }
    }

    for dst_item in &dst_items {
        if dst_item.kind == ConfigLineKind::Pair
            && !merged.iter().any(|item| {
                item.kind == ConfigLineKind::Pair
                    && item.section == dst_item.section
                    && item.key == dst_item.key
            })
        {
            merged.push(dst_item.clone());
        }
    }

    let rendered = render_kv_config(&merged);
    tx.replace_file_with_bytes(dst, rendered.as_bytes())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigLineKind {
    Section,
    Pair,
    Other,
}

#[derive(Clone)]
struct ConfigLine {
    kind: ConfigLineKind,
    section: String,
    key: String,
    value: String,
    raw: String,
}

fn parse_kv_config(text: &str) -> Vec<ConfigLine> {
    let mut section = String::new();
    let mut items = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            section = line[1..line.len() - 1].to_string();
            items.push(ConfigLine {
                kind: ConfigLineKind::Section,
                section: section.clone(),
                key: String::new(),
                value: String::new(),
                raw: raw_line.to_string(),
            });
            continue;
        }

        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            items.push(ConfigLine {
                kind: ConfigLineKind::Other,
                section: section.clone(),
                key: String::new(),
                value: String::new(),
                raw: raw_line.to_string(),
            });
            continue;
        }

        if let Some(eq) = line.find('=') {
            items.push(ConfigLine {
                kind: ConfigLineKind::Pair,
                section: section.clone(),
                key: line[..eq].trim().to_string(),
                value: line[eq + 1..].trim().to_string(),
                raw: raw_line.to_string(),
            });
        } else {
            items.push(ConfigLine {
                kind: ConfigLineKind::Other,
                section: section.clone(),
                key: String::new(),
                value: String::new(),
                raw: raw_line.to_string(),
            });
        }
    }

    items
}

fn render_kv_config(items: &[ConfigLine]) -> String {
    let mut out = String::new();
    for item in items {
        match item.kind {
            ConfigLineKind::Section | ConfigLineKind::Other => out.push_str(&item.raw),
            ConfigLineKind::Pair => {
                if !item.key.is_empty() {
                    out.push_str(&item.key);
                    out.push('=');
                    out.push_str(&item.value);
                }
            }
        }
        out.push('\n');
    }
    out
}
