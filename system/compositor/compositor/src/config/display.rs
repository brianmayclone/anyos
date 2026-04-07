//! Display settings in compositor.conf.

use anyos_std::println;

use super::file::read_conf;

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
    if fs::write_bytes(super::file::CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (font_smoothing)");
    }
}

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
                if (100..=300).contains(&v) {
                    return Some(v);
                }
            }
        }
    }
    None
}

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
            result.push_str(&alloc::format!("scale={}\n", rounded));
            wrote_scale_in_display = true;
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    if in_display && !wrote_scale_in_display {
        result.push_str(&alloc::format!("scale={}\n", rounded));
    }

    if !wrote_display {
        result.push_str("\n[display]\nscale=");
        result.push_str(&alloc::format!("{}\n", rounded));
    }

    let trimmed = result.trim_end();
    if fs::write_bytes(super::file::CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (scale)");
    }
}
