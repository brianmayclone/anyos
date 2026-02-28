//! Compositor configuration — INI-style `/System/compositor/compositor.conf`.
//!
//! Supports section groups:
//!   `[resolution]` — saved display resolution (width, height)
//!   `[login]`      — programs to launch before the login screen (e.g. inputmon)
//!   `[autostart]`  — programs to launch after compositor + dock are ready
//!
//! Example:
//! ```text
//! [resolution]
//! width=1280
//! height=1024
//!
//! [login]
//! /System/inputmon
//!
//! [autostart]
//! /System/netmon
//! /System/audiomon
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

/// Launch all programs listed in the `[login]` section.
///
/// These are services needed before the login screen (e.g. inputmon for keyboard layout).
pub fn launch_login_services() {
    let text = match read_conf() {
        Some(t) => t,
        None => return,
    };

    let mut in_login = false;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_login = line == "[login]";
            continue;
        }
        if !in_login {
            continue;
        }
        let tid = process::spawn(line, "");
        if tid != 0 {
            println!("compositor: [login] launched '{}' (TID={})", line, tid);
        } else {
            println!("compositor: [login] FAILED to launch '{}'", line);
        }
    }
}

/// Launch all programs listed in the `[autostart]` section.
///
/// Returns a `Vec<u32>` of successfully spawned TIDs (used for logout cleanup).
pub fn launch_autostart() -> alloc::vec::Vec<u32> {
    let text = match read_conf() {
        Some(t) => t,
        None => {
            println!("compositor: no compositor.conf found");
            return alloc::vec::Vec::new();
        }
    };

    let mut in_autostart = false;
    let mut tids = alloc::vec::Vec::new();

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
        if tid != 0 && tid != u32::MAX {
            println!("compositor: launched '{}' (TID={})", line, tid);
            tids.push(tid);
        } else {
            println!("compositor: FAILED to launch '{}'", line);
        }
    }
    tids
}

// ── Font Smoothing ───────────────────────────────────────────────────────────

/// Read the `[display]` section for the `font_smoothing` key.
///
/// Returns the saved mode (0/1/2), or `None` if not present.
pub fn read_font_smoothing() -> Option<u32> {
    let text = read_conf()?;
    let mut in_display = false;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_display = line == "[display]";
            continue;
        }
        if !in_display {
            continue;
        }
        if let Some(val) = line.strip_prefix("font_smoothing=") {
            return val.trim().parse::<u32>().ok();
        }
    }
    None
}

/// Save font smoothing mode to the `[display]` section of compositor.conf.
///
/// Preserves all other sections. If no `[display]` section exists it is appended.
pub fn save_font_smoothing(mode: u32) {
    use anyos_std::fs;

    let old_text = read_conf().unwrap_or_default();
    let mut result = alloc::string::String::with_capacity(old_text.len() + 64);
    let mut wrote_display = false;
    let mut in_display = false;
    let mut skip_display_keys = false;

    for line in old_text.split('\n') {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            if in_display {
                in_display = false;
                skip_display_keys = false;
            }
            if trimmed == "[display]" {
                result.push_str("[display]\n");
                result.push_str(&alloc::format!("font_smoothing={}\n", mode));
                result.push('\n');
                wrote_display = true;
                in_display = true;
                skip_display_keys = true;
                continue;
            }
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if skip_display_keys {
            if trimmed.starts_with("font_smoothing=") || trimmed.is_empty() {
                continue;
            }
            skip_display_keys = false;
            in_display = false;
        }

        result.push_str(line);
        result.push('\n');
    }

    if !wrote_display {
        result.push_str("\n[display]\n");
        result.push_str(&alloc::format!("font_smoothing={}\n", mode));
    }

    let trimmed = result.trim_end();
    if fs::write_bytes(CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (font_smoothing)");
    }
}

// ── DPI Scale Factor ────────────────────────────────────────────────────────

/// Read the `[display]` section for the `scale` key.
///
/// Returns the saved scale percentage (100–300), or `None` if not present.
pub fn read_scale_factor() -> Option<u32> {
    let text = read_conf()?;
    let mut in_display = false;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_display = line == "[display]";
            continue;
        }
        if !in_display {
            continue;
        }
        if let Some(val) = line.strip_prefix("scale=") {
            if let Ok(v) = val.trim().parse::<u32>() {
                if v >= 100 && v <= 300 {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Save DPI scale factor to the `[display]` section of compositor.conf.
///
/// Preserves all other keys in the section. Rounds to nearest multiple of 25.
pub fn save_scale_factor(percent: u32) {
    use anyos_std::fs;

    let clamped = percent.max(100).min(300);
    let rounded = ((clamped + 12) / 25) * 25;

    let old_text = read_conf().unwrap_or_default();
    let mut result = alloc::string::String::with_capacity(old_text.len() + 64);
    let mut wrote_display = false;
    let mut in_display = false;
    let mut wrote_scale_in_display = false;

    for line in old_text.split('\n') {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            if in_display && !wrote_scale_in_display {
                // [display] section existed but had no scale= key — append before leaving
                result.push_str(&alloc::format!("scale={}\n", rounded));
            }
            in_display = false;
            wrote_scale_in_display = false;

            if trimmed == "[display]" {
                in_display = true;
                wrote_display = true;
            }
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_display && trimmed.starts_with("scale=") {
            // Replace existing scale= line
            result.push_str(&alloc::format!("scale={}\n", rounded));
            wrote_scale_in_display = true;
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    // If we were still in [display] at EOF and didn't write scale
    if in_display && !wrote_scale_in_display {
        result.push_str(&alloc::format!("scale={}\n", rounded));
    }

    if !wrote_display {
        result.push_str("\n[display]\nscale=");
        result.push_str(&alloc::format!("{}\n", rounded));
    }

    let trimmed = result.trim_end();
    if fs::write_bytes(CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (scale)");
    }
}

/// Saved theme preference from `[theme]` section.
pub struct SavedTheme {
    /// `"dark"` or `"light"`.
    pub mode: alloc::string::String,
    /// Accent style name (e.g. `"blue"`, `"purple"`). Empty if unset.
    pub style: alloc::string::String,
}

/// Read the `[theme]` section from compositor.conf.
///
/// Returns `Some(SavedTheme)` if at least a `mode=` key is present.
pub fn read_theme() -> Option<SavedTheme> {
    let text = read_conf()?;
    let mut in_theme = false;
    let mut mode: Option<alloc::string::String> = None;
    let mut style = alloc::string::String::new();

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_theme = line == "[theme]";
            continue;
        }
        if !in_theme {
            continue;
        }
        if let Some(val) = line.strip_prefix("mode=") {
            mode = Some(alloc::string::String::from(val.trim()));
        } else if let Some(val) = line.strip_prefix("style=") {
            style = alloc::string::String::from(val.trim());
        }
    }

    mode.map(|m| SavedTheme { mode: m, style })
}

/// Save theme preference to the `[theme]` section of compositor.conf.
///
/// Preserves all other sections. If no `[theme]` section exists it is appended.
pub fn save_theme(mode: &str, style: &str) {
    use anyos_std::fs;

    let old_text = read_conf().unwrap_or_default();
    let mut result = alloc::string::String::with_capacity(old_text.len() + 64);
    let mut wrote_theme = false;
    let mut in_theme = false;
    let mut skip_theme_keys = false;

    for line in old_text.split('\n') {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            if in_theme {
                in_theme = false;
                skip_theme_keys = false;
            }
            if trimmed == "[theme]" {
                result.push_str("[theme]\n");
                result.push_str(&alloc::format!("mode={}\n", mode));
                if !style.is_empty() {
                    result.push_str(&alloc::format!("style={}\n", style));
                }
                result.push('\n');
                wrote_theme = true;
                in_theme = true;
                skip_theme_keys = true;
                continue;
            }
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if skip_theme_keys {
            if trimmed.starts_with("mode=") || trimmed.starts_with("style=") || trimmed.is_empty() {
                continue;
            }
            skip_theme_keys = false;
            in_theme = false;
        }

        result.push_str(line);
        result.push('\n');
    }

    if !wrote_theme {
        result.push_str("\n[theme]\n");
        result.push_str(&alloc::format!("mode={}\n", mode));
        if !style.is_empty() {
            result.push_str(&alloc::format!("style={}\n", style));
        }
    }

    let trimmed = result.trim_end();
    if fs::write_bytes(CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (theme)");
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
