fn parse_media_query(text: &str) -> MediaQuery {
    let mut conditions = Vec::new();
    let trimmed = text.trim();
    let mut media_type = MediaType::All;
    let mut negated = false;
    for part in split_and(trimmed) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let lower = p.to_ascii_lowercase();
        if lower == "not" {
            negated = true;
            continue;
        }
        if lower == "only" {
            continue;
        }
        if lower == "screen" {
            let mt = MediaType::Screen;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
            continue;
        }
        if lower == "print" {
            let mt = MediaType::Print;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
            continue;
        }
        if lower == "all" {
            let mt = MediaType::All;
            media_type = if negated { MediaType::Not(Box::new(mt)) } else { mt };
            negated = false;
            continue;
        }
        if p.starts_with('(') && p.ends_with(')') {
            let inner = &p[1..p.len() - 1];
            if let Some(cond) = parse_media_condition(inner) {
                conditions.push(cond);
            }
        }
    }
    MediaQuery { conditions, media_type }
}

fn split_and(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    for i in 0..bytes.len() {
        if i + 5 <= bytes.len() {
            let chunk = &bytes[i..i + 5];
            if (chunk[0] == b' ')
                && (chunk[1] | 32 == b'a')
                && (chunk[2] | 32 == b'n')
                && (chunk[3] | 32 == b'd')
                && (chunk[4] == b' ')
            {
                parts.push(&s[start..i]);
                start = i + 5;
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

fn parse_media_condition(inner: &str) -> Option<MediaCondition> {
    let inner = inner.trim();
    if let Some(idx) = inner.find(">=") {
        let feature = inner[..idx].trim().to_ascii_lowercase();
        let val = inner[idx + 2..].trim();
        if feature == "width" {
            if let Some(px) = parse_px_value(val) {
                return Some(MediaCondition::MinWidth(px));
            }
        }
    }
    if let Some(idx) = inner.find("<=") {
        let feature = inner[..idx].trim().to_ascii_lowercase();
        let val = inner[idx + 2..].trim();
        if feature == "width" {
            if let Some(px) = parse_px_value(val) {
                return Some(MediaCondition::MaxWidth(px));
            }
        }
    }
    if !inner.contains(':') {
        let feature = inner.to_ascii_lowercase();
        return match feature.as_str() {
            "color" | "color-index" => Some(MediaCondition::Known(true)),
            "monochrome" => Some(MediaCondition::Known(false)),
            "hover" => Some(MediaCondition::Known(true)),
            _ => Some(MediaCondition::Unsupported),
        };
    }
    let colon = inner.find(':')?;
    let name = inner[..colon].trim().to_ascii_lowercase();
    let value_str = inner[colon + 1..].trim();
    match name.as_str() {
        "min-width" => Some(MediaCondition::MinWidth(parse_px_value(value_str)?)),
        "max-width" => Some(MediaCondition::MaxWidth(parse_px_value(value_str)?)),
        "min-height" => Some(MediaCondition::MinHeight(parse_px_value(value_str)?)),
        "max-height" => Some(MediaCondition::MaxHeight(parse_px_value(value_str)?)),
        "prefers-color-scheme" => Some(MediaCondition::PrefersColorScheme(String::from(value_str.trim()))),
        "hover" => Some(MediaCondition::Known(value_str == "hover")),
        "any-hover" => Some(MediaCondition::Known(value_str == "hover")),
        "pointer" => Some(MediaCondition::Known(value_str == "fine")),
        "any-pointer" => Some(MediaCondition::Known(value_str == "fine")),
        "prefers-reduced-motion" => Some(MediaCondition::Known(value_str == "no-preference")),
        "prefers-contrast" => Some(MediaCondition::Known(value_str == "no-preference")),
        "prefers-reduced-data" | "prefers-reduced-transparency" => Some(MediaCondition::Known(value_str == "no-preference")),
        "color-gamut" => Some(MediaCondition::Known(value_str == "srgb")),
        "resolution" | "min-resolution" | "max-resolution" => Some(MediaCondition::Known(true)),
        "orientation" => Some(MediaCondition::Known(value_str == "landscape")),
        "dynamic-viewport-height" | "environment" => Some(MediaCondition::Unsupported),
        _ => Some(MediaCondition::Unsupported),
    }
}

pub fn evaluate_media_query(query: &MediaQuery, viewport_width: i32, viewport_height: i32) -> bool {
    match &query.media_type {
        MediaType::All => {}
        MediaType::Screen => {}
        MediaType::Print => return false,
        MediaType::Not(inner) => match inner.as_ref() {
            MediaType::Print => {}
            MediaType::Screen => return false,
            MediaType::All => return false,
            MediaType::Not(_) => {}
        },
    }
    for cond in &query.conditions {
        let ok = match cond {
            MediaCondition::MinWidth(w) => viewport_width >= *w,
            MediaCondition::MaxWidth(w) => viewport_width <= *w,
            MediaCondition::MinHeight(h) => viewport_height >= *h,
            MediaCondition::MaxHeight(h) => viewport_height <= *h,
            MediaCondition::PrefersColorScheme(scheme) => scheme == "light",
            MediaCondition::Known(v) => *v,
            MediaCondition::Unsupported => false,
        };
        if !ok {
            return false;
        }
    }
    true
}
