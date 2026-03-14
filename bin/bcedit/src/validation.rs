//! Config validation — `bcedit check`
//!
//! Checks for:
//!   - Missing or invalid global flags (timeout, default)
//!   - default index out of range
//!   - No boot entries at all
//!   - Duplicate section names
//!   - Duplicate keys within a section
//!   - Missing required keys per entry (kernel)
//!   - Missing recommended keys per entry (description)

use crate::config::{
    Config, MAX_LINES,
    count_entries, find_key_in, first_section, is_section_header,
    line_value, section_end, section_name,
};

pub fn check(cfg: &Config) {
    let mut issues = 0usize;
    let mut notes  = 0usize;

    // ── global flags ──────────────────────────────────────────────────────────
    let first = first_section(cfg);
    let global_end = if first == MAX_LINES { cfg.count } else { first };

    issues += check_numeric_flag(cfg, global_end, "timeout",
        "the boot menu will not auto-select an entry");
    issues += check_numeric_flag(cfg, global_end, "default",
        "the boot loader will not know which entry to boot");

    // 'default' index in range?
    let di = find_key_in(cfg, 0, global_end, "default");
    if di < MAX_LINES {
        let v = line_value(cfg.lines[di].as_str());
        if let Some(idx) = parse_usize(v) {
            let total = count_entries(cfg);
            if idx >= total {
                anyos_std::println!(
                    "  ERROR: default={} is out of range — only {} entr{} defined",
                    idx, total, if total == 1 { "y" } else { "ies" }
                );
                issues += 1;
            }
        }
    }

    // ── entry count ───────────────────────────────────────────────────────────
    let entry_count = count_entries(cfg);
    if entry_count == 0 {
        anyos_std::println!("  ERROR: no boot entries defined — system will not boot!");
        issues += 1;
    }

    // ── duplicate section names ───────────────────────────────────────────────
    issues += warn_duplicate_sections(cfg);

    // ── per-entry checks ──────────────────────────────────────────────────────
    let mut i = 0;
    while i < cfg.count {
        let s = cfg.lines[i].as_str();
        if is_section_header(s) {
            let name = section_name(s);
            let end  = section_end(cfg, i);

            // required: kernel
            if find_key_in(cfg, i + 1, end, "kernel") == MAX_LINES {
                anyos_std::println!("  ERROR: entry '{}' has no 'kernel' key", name);
                issues += 1;
            }

            // recommended: description
            if find_key_in(cfg, i + 1, end, "description") == MAX_LINES {
                anyos_std::println!("  NOTE: entry '{}' has no 'description'", name);
                notes += 1;
            }

            // duplicate keys within entry
            issues += warn_duplicate_keys(cfg, name, i + 1, end);
        }
        i += 1;
    }

    // ── summary ───────────────────────────────────────────────────────────────
    anyos_std::println!("");
    if issues == 0 && notes == 0 {
        anyos_std::println!("OK — config is valid ({} entr{}).",
            entry_count, if entry_count == 1 { "y" } else { "ies" });
    } else {
        if issues > 0 {
            anyos_std::println!("{} error(s) found — fix before rebooting.", issues);
        }
        if notes > 0 {
            anyos_std::println!("{} note(s) (non-critical).", notes);
        }
    }
}

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Check that a named global flag exists and holds a non-negative integer.
/// Returns 1 if an issue was found, 0 otherwise.
fn check_numeric_flag(cfg: &Config, global_end: usize, key: &str, missing_hint: &str) -> usize {
    let pos = find_key_in(cfg, 0, global_end, key);
    if pos == MAX_LINES {
        anyos_std::println!("  WARNING: '{}' flag is missing ({})", key, missing_hint);
        return 1;
    }
    let v = line_value(cfg.lines[pos].as_str());
    let mut ok = !v.is_empty();
    for b in v.as_bytes() { if !b.is_ascii_digit() { ok = false; } }
    if !ok {
        anyos_std::println!("  ERROR: '{}' value '{}' is not a valid non-negative integer", key, v);
        return 1;
    }
    0
}

/// Warn about duplicate `[section]` names. Returns count of duplicates found.
fn warn_duplicate_sections(cfg: &Config) -> usize {
    let mut dups = 0usize;
    let mut i = 0;
    while i < cfg.count {
        let s = cfg.lines[i].as_str();
        if is_section_header(s) {
            let name = section_name(s);
            let mut j = i + 1;
            while j < cfg.count {
                let t = cfg.lines[j].as_str();
                if is_section_header(t) && section_name(t) == name {
                    anyos_std::println!("  WARNING: duplicate entry name '{}'", name);
                    dups += 1;
                }
                j += 1;
            }
        }
        i += 1;
    }
    dups
}

/// Warn about duplicate keys within a single entry. Returns count found.
fn warn_duplicate_keys(cfg: &Config, entry_name: &str, start: usize, end: usize) -> usize {
    let mut dups = 0usize;
    let mut j = start;
    while j < end {
        let line_j = cfg.lines[j].as_str();
        if line_j.is_empty() || line_j.starts_with('#') { j += 1; continue; }
        if let Some(eq) = line_j.find('=') {
            let key = &line_j[..eq];
            let mut k = j + 1;
            while k < end {
                let line_k = cfg.lines[k].as_str();
                if line_k.len() > key.len()
                    && line_k.as_bytes()[key.len()] == b'='
                    && &line_k[..key.len()] == key
                {
                    anyos_std::println!(
                        "  WARNING: duplicate key '{}' in entry '{}'", key, entry_name
                    );
                    dups += 1;
                }
                k += 1;
            }
        }
        j += 1;
    }
    dups
}

/// Parse a decimal string to usize. Returns None on invalid input.
fn parse_usize(s: &str) -> Option<usize> {
    if s.is_empty() { return None; }
    let mut n = 0usize;
    for b in s.as_bytes() {
        if !b.is_ascii_digit() { return None; }
        n = n * 10 + (*b - b'0') as usize;
    }
    Some(n)
}
