//! Resolution persistence in compositor.conf.

use anyos_std::println;

use super::file::read_conf;

pub struct SavedResolution {
    pub width: u32,
    pub height: u32,
}

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

pub fn save_resolution(width: u32, height: u32) {
    use anyos_std::fs;

    let old_text = read_conf().unwrap_or_default();
    let mut result = alloc::string::String::with_capacity(old_text.len() + 64);
    let mut wrote_resolution = false;
    let mut in_resolution = false;
    let mut skip_resolution_keys = false;

    for line in old_text.split('\n') {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            if in_resolution {
                in_resolution = false;
                skip_resolution_keys = false;
            }
            if trimmed == "[resolution]" {
                result.push_str("[resolution]\n");
                result.push_str(&alloc::format!("width={}\n", width));
                result.push_str(&alloc::format!("height={}\n", height));
                result.push('\n');
                wrote_resolution = true;
                in_resolution = true;
                skip_resolution_keys = true;
                continue;
            }
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if skip_resolution_keys {
            if trimmed.starts_with("width=") || trimmed.starts_with("height=") || trimmed.is_empty() {
                continue;
            }
            skip_resolution_keys = false;
            in_resolution = false;
        }

        result.push_str(line);
        result.push('\n');
    }

    if !wrote_resolution {
        let mut final_text = alloc::string::String::with_capacity(result.len() + 64);
        final_text.push_str("[resolution]\n");
        final_text.push_str(&alloc::format!("width={}\n", width));
        final_text.push_str(&alloc::format!("height={}\n", height));
        final_text.push('\n');
        final_text.push_str(&result);
        result = final_text;
    }

    let trimmed = result.trim_end();
    if fs::write_bytes(super::file::CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf");
    }
}
