//! All bcedit subcommand implementations.
//!
//! Read-only commands (`list`, `list_flags`, `show`) take `&Config`.
//! Mutating commands return `bool` — true means the config was modified
//! and should be written back to disk.

use crate::config::{
    count_entries, find_key_in, find_section, first_section, is_section_header, line_value,
    make_header, make_kv, section_end, section_name, Config, MAX_LINE, MAX_LINES,
};

// ─── Read-only commands ───────────────────────────────────────────────────────

/// Compact or verbose entry listing.
pub fn list(cfg: &Config, verbose: bool) {
    let first = first_section(cfg);
    let global_end = if first == MAX_LINES { cfg.count } else { first };

    let mut has_globals = false;
    for i in 0..global_end {
        let s = cfg.lines[i].as_str();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        if !has_globals {
            anyos_std::println!("Global flags:");
            has_globals = true;
        }
        anyos_std::println!("  {}", s);
    }
    if has_globals {
        anyos_std::println!();
    }

    let mut entry_idx = 0u32;
    let mut i = 0;
    while i < cfg.count {
        let s = cfg.lines[i].as_str();
        if is_section_header(s) {
            anyos_std::println!("[{}] {}", entry_idx, section_name(s));
            if verbose {
                let end = section_end(cfg, i);
                for j in i + 1..end {
                    let line = cfg.lines[j].as_str();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    anyos_std::println!("      {}", line);
                }
            }
            entry_idx += 1;
        }
        i += 1;
    }
    if entry_idx == 0 {
        anyos_std::println!("(no boot entries)");
    }
}

/// Show all keys of a single named entry.
pub fn show(cfg: &Config, name: &str) {
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return;
    }
    anyos_std::println!("[{}]", name);
    let end = section_end(cfg, sec);
    for i in sec + 1..end {
        let s = cfg.lines[i].as_str();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        anyos_std::println!("  {}", s);
    }
}

/// List global flags (lines before the first section).
pub fn list_flags(cfg: &Config) {
    let first = first_section(cfg);
    let end = if first == MAX_LINES { cfg.count } else { first };
    let mut found = false;
    for i in 0..end {
        let s = cfg.lines[i].as_str();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        anyos_std::println!("{}", s);
        found = true;
    }
    if !found {
        anyos_std::println!("(no global flags)");
    }
}

// ─── Global flag commands ─────────────────────────────────────────────────────

pub fn set_flag(cfg: &mut Config, key: &str, value: &str) -> bool {
    // Validate numeric flags
    if key == "timeout" || key == "default" {
        if !is_non_negative_integer(value) {
            anyos_std::println!(
                "bcedit: '{}' must be a non-negative integer, got '{}'",
                key,
                value
            );
            return false;
        }
        if key == "default" {
            let idx = parse_usize(value).unwrap_or(usize::MAX);
            let total = count_entries(cfg);
            if idx >= total {
                anyos_std::println!(
                    "  WARNING: default={} is out of range ({} entr{} exist). Setting anyway.",
                    idx,
                    total,
                    if total == 1 { "y" } else { "ies" }
                );
            }
        }
    }

    let first = first_section(cfg);
    let end = if first == MAX_LINES { cfg.count } else { first };
    let pos = find_key_in(cfg, 0, end, key);

    let mut kv = [0u8; MAX_LINE];
    let total = make_kv(key, value, &mut kv);
    if total == 0 {
        anyos_std::println!("bcedit: value too long");
        return false;
    }
    let kv_str = core::str::from_utf8(&kv[..total]).unwrap_or("");

    if pos < MAX_LINES {
        cfg.lines[pos] = crate::config::Line::from_str(kv_str);
        anyos_std::println!("Updated: {}", kv_str);
    } else {
        let insert_at = if first == MAX_LINES { cfg.count } else { first };
        cfg.insert(insert_at, kv_str);
        // Keep a blank line between globals and first section
        if first < MAX_LINES {
            let blank_pos = insert_at + 1;
            if blank_pos < cfg.count && !cfg.lines[blank_pos].as_str().is_empty() {
                cfg.insert(blank_pos, "");
            }
        }
        anyos_std::println!("Added: {}", kv_str);
    }
    true
}

pub fn del_flag(cfg: &mut Config, key: &str) -> bool {
    let first = first_section(cfg);
    let end = if first == MAX_LINES { cfg.count } else { first };
    let pos = find_key_in(cfg, 0, end, key);
    if pos == MAX_LINES {
        anyos_std::println!("bcedit: global flag '{}' not found", key);
        return false;
    }
    cfg.remove_line(pos);
    anyos_std::println!("Removed global flag: {}", key);
    if key == "default" || key == "timeout" {
        anyos_std::println!(
            "  NOTE: '{}' is a required flag. Run 'bcedit check' to validate.",
            key
        );
    }
    true
}

// ─── Entry management ─────────────────────────────────────────────────────────

pub fn add(cfg: &mut Config, name: &str) -> bool {
    if name.contains('[') || name.contains(']') {
        anyos_std::println!("bcedit: entry name must not contain '[' or ']'");
        return false;
    }
    if find_section(cfg, name) < MAX_LINES {
        anyos_std::println!(
            "bcedit: entry '{}' already exists — use 'rename' or choose a different name",
            name
        );
        return false;
    }
    let mut hdr = [0u8; MAX_LINE];
    let hdr_len = make_header(name, &mut hdr);
    let hdr_str = core::str::from_utf8(&hdr[..hdr_len]).unwrap_or("");

    if cfg.count > 0 {
        cfg.push("");
    }
    cfg.push(hdr_str);
    cfg.push("kernel=0");
    cfg.push("description=");
    anyos_std::println!("Added entry: {}", name);
    anyos_std::println!(
        "  Tip: use 'bcedit set {} params <value>' to set boot parameters.",
        name
    );
    true
}

pub fn remove(cfg: &mut Config, name: &str) -> bool {
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return false;
    }

    // Refuse to remove the last entry
    let total = count_entries(cfg);
    if total == 1 {
        anyos_std::println!("bcedit: REFUSED — '{}' is the only boot entry.", name);
        anyos_std::println!("  Removing it would make the system unbootable.");
        anyos_std::println!(
            "  Use 'bcedit init' to restore defaults, or 'bcedit add' to create another entry first."
        );
        return false;
    }

    // Warn if this is the current default
    warn_if_default(cfg, sec);

    let end = section_end(cfg, sec);
    let start = if sec > 0 && cfg.lines[sec - 1].as_str().is_empty() {
        sec - 1
    } else {
        sec
    };
    let remove_count = end - start;
    for _ in 0..remove_count {
        cfg.remove_line(start);
    }
    anyos_std::println!("Removed entry: {}", name);
    true
}

pub fn rename(cfg: &mut Config, name: &str, newname: &str) -> bool {
    if newname.contains('[') || newname.contains(']') {
        anyos_std::println!("bcedit: entry name must not contain '[' or ']'");
        return false;
    }
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return false;
    }
    if find_section(cfg, newname) < MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' already exists", newname);
        return false;
    }
    let mut hdr = [0u8; MAX_LINE];
    let hdr_len = make_header(newname, &mut hdr);
    cfg.lines[sec] =
        crate::config::Line::from_str(core::str::from_utf8(&hdr[..hdr_len]).unwrap_or(""));
    anyos_std::println!("Renamed '{}' -> '{}'", name, newname);
    true
}

pub fn duplicate(cfg: &mut Config, name: &str, newname: &str) -> bool {
    if newname.contains('[') || newname.contains(']') {
        anyos_std::println!("bcedit: entry name must not contain '[' or ']'");
        return false;
    }
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return false;
    }
    if find_section(cfg, newname) < MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' already exists", newname);
        return false;
    }
    let end = section_end(cfg, sec);

    // Collect lines to copy (up to 32 lines per entry is generous)
    let mut copy_buf: [crate::config::Line; 32] = [crate::config::Line::empty(); 32];
    let mut copy_count = 0usize;

    // New header
    let mut hdr = [0u8; MAX_LINE];
    let hdr_len = make_header(newname, &mut hdr);
    copy_buf[copy_count] =
        crate::config::Line::from_str(core::str::from_utf8(&hdr[..hdr_len]).unwrap_or(""));
    copy_count += 1;

    for i in sec + 1..end {
        if copy_count >= 32 {
            break;
        }
        copy_buf[copy_count] = cfg.lines[i];
        copy_count += 1;
    }

    cfg.push("");
    for i in 0..copy_count {
        cfg.push(copy_buf[i].as_str());
    }
    anyos_std::println!("Duplicated '{}' as '{}'", name, newname);
    true
}

// ─── Key-level commands ───────────────────────────────────────────────────────

pub fn set(cfg: &mut Config, name: &str, key: &str, value: &str) -> bool {
    if key.contains('=') {
        anyos_std::println!("bcedit: key must not contain '='");
        return false;
    }
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return false;
    }
    let end = section_end(cfg, sec);

    let mut kv = [0u8; MAX_LINE];
    let total = make_kv(key, value, &mut kv);
    if total == 0 {
        anyos_std::println!("bcedit: value too long");
        return false;
    }
    let kv_str = core::str::from_utf8(&kv[..total]).unwrap_or("");

    // Warn on unusual parameter combos
    if key == "params" && value.contains("nogui") && value.contains("verbose") {
        anyos_std::println!("  NOTE: combining 'nogui' and 'verbose' is valid but unusual.");
    }

    let pos = find_key_in(cfg, sec + 1, end, key);
    if pos < MAX_LINES {
        cfg.lines[pos] = crate::config::Line::from_str(kv_str);
        anyos_std::println!("Updated [{}]: {}", name, kv_str);
    } else {
        cfg.insert(end, kv_str);
        anyos_std::println!("Added to [{}]: {}", name, kv_str);
    }
    true
}

pub fn del(cfg: &mut Config, name: &str, key: &str) -> bool {
    let sec = find_section(cfg, name);
    if sec == MAX_LINES {
        anyos_std::println!("bcedit: entry '{}' not found", name);
        return false;
    }
    let end = section_end(cfg, sec);
    let pos = find_key_in(cfg, sec + 1, end, key);
    if pos == MAX_LINES {
        anyos_std::println!("bcedit: key '{}' not found in entry '{}'", key, name);
        return false;
    }
    if key == "kernel" {
        anyos_std::println!(
            "  WARNING: removing 'kernel' from '{}' will make this entry unbootable.",
            name
        );
    }
    cfg.remove_line(pos);
    anyos_std::println!("Removed [{}] {}", name, key);
    true
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Warn if `sec` is the current default entry.
fn warn_if_default(cfg: &Config, sec: usize) {
    let first = first_section(cfg);
    let global_end = if first == MAX_LINES { cfg.count } else { first };
    let di = find_key_in(cfg, 0, global_end, "default");
    if di == MAX_LINES {
        return;
    }

    // Determine 0-based index of this section
    let mut entry_idx = 0usize;
    let mut i = 0;
    while i < cfg.count {
        if is_section_header(cfg.lines[i].as_str()) {
            if i == sec {
                break;
            }
            entry_idx += 1;
        }
        i += 1;
    }

    let default_val = line_value(cfg.lines[di].as_str());
    if let Some(default_idx) = parse_usize(default_val) {
        if entry_idx == default_idx {
            anyos_std::println!(
                "  WARNING: '{}' is the current default entry (default={}).",
                section_name(cfg.lines[sec].as_str()),
                default_idx
            );
            anyos_std::println!("  Update with 'bcedit set-flag default <index>' after removal.");
        }
    }
}

fn is_non_negative_integer(s: &str) -> bool {
    !s.is_empty() && s.as_bytes().iter().all(|b| b.is_ascii_digit())
}

fn parse_usize(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (*b - b'0') as usize;
    }
    Some(n)
}
