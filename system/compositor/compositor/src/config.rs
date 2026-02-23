//! Compositor configuration — INI-style `/System/compositor/compositor.conf`.
//!
//! Supports section groups:
//!   `[resolution]` — saved display resolution (width, height)
//!   `[autostart]`  — programs to launch after compositor + dock are ready
//!
//! Example:
//! ```text
//! [resolution]
//! width=1280
//! height=1024
//!
//! [autostart]
//! /System/netmon
//! /System/audiomon
//! /System/inputmon
//! ```

use anyos_std::println;
use anyos_std::process;

const CONF_PATH: &str = "/System/compositor/compositor.conf";

/// Saved resolution from `[resolution]` section, if any.
pub struct SavedResolution {
    pub width: u32,
    pub height: u32,
}

/// Read the config file and return its raw text content.
fn read_conf() -> Option<alloc::string::String> {
    use anyos_std::fs;

    let fd = fs::open(CONF_PATH, 0);
    if fd == u32::MAX {
        return None;
    }

    let mut buf = [0u8; 2048];
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);

    if n == 0 {
        return None;
    }

    match core::str::from_utf8(&buf[..n]) {
        Ok(s) => Some(alloc::string::String::from(s)),
        Err(_) => None,
    }
}

/// Parse the `[resolution]` section from the config text.
///
/// Returns `Some(SavedResolution)` if both `width` and `height` keys are present
/// and valid. Enforces minimum 1024x768.
pub fn read_resolution() -> Option<SavedResolution> {
    let text = read_conf()?;
    let mut in_resolution = false;
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_resolution = line == "[resolution]";
            continue;
        }
        if !in_resolution {
            continue;
        }
        if let Some(val) = line.strip_prefix("width=") {
            width = val.trim().parse::<u32>().ok();
        } else if let Some(val) = line.strip_prefix("height=") {
            height = val.trim().parse::<u32>().ok();
        }
    }

    match (width, height) {
        (Some(w), Some(h)) if w >= 1024 && h >= 768 => Some(SavedResolution { width: w, height: h }),
        _ => None,
    }
}

/// Launch all programs listed in the `[autostart]` section.
pub fn launch_autostart() {
    let text = match read_conf() {
        Some(t) => t,
        None => {
            println!("compositor: no compositor.conf found");
            return;
        }
    };

    let mut in_autostart = false;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_autostart = line == "[autostart]";
            continue;
        }
        if !in_autostart {
            continue;
        }
        // Each non-empty line in [autostart] is a program path
        let tid = process::spawn(line, "");
        if tid != 0 {
            println!("compositor: launched '{}' (TID={})", line, tid);
        } else {
            println!("compositor: FAILED to launch '{}'", line);
        }
    }
}

/// Save the given resolution to the `[resolution]` section of compositor.conf.
///
/// Preserves all other sections and comments. If no `[resolution]` section exists,
/// it is prepended. If it exists, the width/height values are updated in-place.
pub fn save_resolution(width: u32, height: u32) {
    use anyos_std::fs;

    let old_text = read_conf().unwrap_or_default();

    // Rebuild the file: replace or insert [resolution] section
    let mut result = alloc::string::String::with_capacity(old_text.len() + 64);
    let mut wrote_resolution = false;
    let mut in_resolution = false;
    let mut skip_resolution_keys = false;

    for line in old_text.split('\n') {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            // Entering a new section
            if in_resolution {
                // We were in [resolution] and now leaving — already wrote our values
                in_resolution = false;
                skip_resolution_keys = false;
            }
            if trimmed == "[resolution]" {
                // Write our updated resolution section
                result.push_str("[resolution]\n");
                result.push_str(&alloc::format!("width={}\n", width));
                result.push_str(&alloc::format!("height={}\n", height));
                result.push('\n');
                wrote_resolution = true;
                in_resolution = true;
                skip_resolution_keys = true;
                continue;
            }
            // Other section header — pass through
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if skip_resolution_keys {
            // Skip old width=/height= lines in the [resolution] section
            if trimmed.starts_with("width=") || trimmed.starts_with("height=") {
                continue;
            }
            if trimmed.is_empty() {
                // Skip blank line after resolution keys (we already added one)
                continue;
            }
            // Non-resolution content inside [resolution] section — stop skipping
            skip_resolution_keys = false;
            in_resolution = false;
        }

        result.push_str(line);
        result.push('\n');
    }

    // If no [resolution] section existed, prepend it
    if !wrote_resolution {
        let mut final_text = alloc::string::String::with_capacity(result.len() + 64);
        final_text.push_str("[resolution]\n");
        final_text.push_str(&alloc::format!("width={}\n", width));
        final_text.push_str(&alloc::format!("height={}\n", height));
        final_text.push('\n');
        final_text.push_str(&result);
        result = final_text;
    }

    // Remove trailing whitespace
    let trimmed = result.trim_end();

    // Write the updated config back
    if fs::write_bytes(CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf");
    }
}
