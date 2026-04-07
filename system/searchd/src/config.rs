//! Configuration parser for `/System/etc/searchd.conf`.
//!
//! Format:
//! ```ini
//! [Main]
//! idleTimeout=10000     # Milliseconds before start indexing
//! maxEntries=1000000    # Maximum index entries
//!
//! [Folders]
//! excludes=/tmp,/proc   # Comma-separated folders to exclude
//! ```

use alloc::string::String;
use alloc::vec::Vec;

const CONFIG_PATH: &str = "/System/etc/searchd.conf";

/// Parsed configuration.
pub struct Config {
    /// Milliseconds of idle time before starting an index pass.
    pub idle_timeout_ms: u32,
    /// Maximum number of entries in the files table.
    pub max_entries: u32,
    /// Folders to exclude from indexing.
    pub excludes: Vec<String>,
}

impl Config {
    /// Default configuration used when no config file exists.
    pub fn defaults() -> Config {
        Config {
            idle_timeout_ms: 10000,
            max_entries: 1000000,
            excludes: Vec::new(),
        }
    }

    /// Load configuration from `/System/etc/searchd.conf`.
    /// Falls back to defaults for any missing values.
    pub fn load() -> Config {
        let mut cfg = Config::defaults();

        let fd = anyos_std::fs::open(CONFIG_PATH, 0);
        if fd == u32::MAX {
            anyos_std::println!("searchd: no config at {}, using defaults", CONFIG_PATH);
            return cfg;
        }

        let mut buf = [0u8; 2048];
        let n = anyos_std::fs::read(fd, &mut buf);
        anyos_std::fs::close(fd);

        if n == 0 || n == u32::MAX {
            return cfg;
        }

        let text = match core::str::from_utf8(&buf[..n as usize]) {
            Ok(s) => s,
            Err(_) => return cfg,
        };

        let mut section = "";

        for line in text.split('\n') {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            // Section header
            if line.starts_with('[') {
                if let Some(end) = line.find(']') {
                    section = &line[1..end];
                }
                continue;
            }

            // Key=Value (strip inline comments)
            let line_no_comment = if let Some(hash) = line.find('#') {
                line[..hash].trim()
            } else {
                line
            };

            if let Some(eq) = line_no_comment.find('=') {
                let key = line_no_comment[..eq].trim();
                let value = line_no_comment[eq + 1..].trim();

                match (section_lower(section), key_lower(key)) {
                    (Section::Main, Key::IdleTimeout) => {
                        if let Some(v) = parse_u32(value) {
                            cfg.idle_timeout_ms = v;
                        }
                    }
                    (Section::Main, Key::MaxEntries) => {
                        if let Some(v) = parse_u32(value) {
                            cfg.max_entries = v;
                        }
                    }
                    (Section::Folders, Key::Excludes) => {
                        cfg.excludes.clear();
                        for part in value.split(',') {
                            let trimmed = part.trim();
                            if !trimmed.is_empty() {
                                cfg.excludes.push(String::from(trimmed));
                            }
                        }
                    }
                    _ => {
                        // Unknown key — ignore
                    }
                }
            }
        }

        anyos_std::println!(
            "searchd: config loaded (idle={}ms, max={}, excludes={})",
            cfg.idle_timeout_ms,
            cfg.max_entries,
            cfg.excludes.len()
        );

        cfg
    }
}

// ── Internal helpers ────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Section { Main, Folders, Unknown }

#[derive(PartialEq)]
enum Key { IdleTimeout, MaxEntries, Excludes, Unknown }

fn section_lower(s: &str) -> Section {
    let bytes = s.as_bytes();
    if bytes.len() == 4 && eq_ignore_case(bytes, b"main") {
        Section::Main
    } else if bytes.len() == 7 && eq_ignore_case(bytes, b"folders") {
        Section::Folders
    } else {
        Section::Unknown
    }
}

fn key_lower(s: &str) -> Key {
    let bytes = s.as_bytes();
    if eq_ignore_case(bytes, b"idletimeout") {
        Key::IdleTimeout
    } else if eq_ignore_case(bytes, b"maxentries") {
        Key::MaxEntries
    } else if eq_ignore_case(bytes, b"excludes") {
        Key::Excludes
    } else {
        Key::Unknown
    }
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    for i in 0..a.len() {
        let mut ca = a[i];
        let mut cb = b[i];
        if ca >= b'A' && ca <= b'Z' { ca += 32; }
        if cb >= b'A' && cb <= b'Z' { cb += 32; }
        if ca != cb { return false; }
    }
    true
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    let mut found = false;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
            found = true;
        } else {
            break;
        }
    }
    if found { Some(val) } else { None }
}
