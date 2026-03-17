//! Command history: load from and append to `~/.history`.
//!
//! Entries are stored one per line, oldest first.  The file is capped at
//! [`HISTORY_MAX`] lines; older entries are discarded when the limit is
//! exceeded.

use anyos_std::{env, fs};
use anyos_std::String;
use anyos_std::Vec;
use anyos_std::format;

/// Maximum number of history entries kept on disk.
pub const HISTORY_MAX: usize = 200;

/// Path relative to `$HOME` where the history file lives.
const HISTORY_FILE: &str = "/.history";

// ─── Internal helper ─────────────────────────────────────────────────────────

/// Resolve the absolute path to the history file using `$HOME`.
/// Returns `None` when `HOME` is not set or empty.
fn history_path() -> Option<String> {
    let mut home_buf = [0u8; 128];
    let hlen = env::get("HOME", &mut home_buf);
    if hlen == u32::MAX || hlen == 0 { return None; }
    let home = core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/");
    Some(format!("{}{}", home, HISTORY_FILE))
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Load history from `~/.history`.  Returns entries in chronological order
/// (oldest first).  Returns an empty Vec when the file does not exist or
/// `$HOME` is unset.
pub fn history_load() -> Vec<String> {
    let path = match history_path() { Some(p) => p, None => return Vec::new() };
    match fs::read_to_string(&path) {
        Ok(s) => s.split('\n').filter(|l| !l.is_empty()).map(String::from).collect(),
        Err(_) => Vec::new(),
    }
}

/// Append `entry` to `~/.history`, truncating the file to the last
/// [`HISTORY_MAX`] lines.  Empty entries are silently ignored.
pub fn history_append(entry: &str) {
    if entry.is_empty() { return; }
    let path = match history_path() { Some(p) => p, None => return };

    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<&str> = existing.split('\n').filter(|l| !l.is_empty()).collect();
    lines.push(entry);
    if lines.len() > HISTORY_MAX {
        lines = lines[lines.len() - HISTORY_MAX..].to_vec();
    }

    let mut out = String::new();
    for l in &lines { out.push_str(l); out.push('\n'); }
    let _ = fs::write_bytes(&path, out.as_bytes());
}
