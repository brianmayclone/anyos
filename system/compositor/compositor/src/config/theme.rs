//! Theme settings in compositor.conf.

use anyos_std::println;

use super::file::read_conf;

pub struct SavedTheme {
    pub mode: alloc::string::String,
    pub style: alloc::string::String,
}

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
    if fs::write_bytes(super::file::CONF_PATH, trimmed.as_bytes()).is_err() {
        println!("compositor: FAILED to save compositor.conf (theme)");
    }
}
