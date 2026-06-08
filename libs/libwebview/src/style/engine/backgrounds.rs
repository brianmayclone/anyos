// Background image parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

fn extract_css_url_function(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let mut i = start + 4;
    let bytes = s.as_bytes();
    let mut quote: Option<u8> = None;
    let mut depth: i32 = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                let inner = s[start + 4..i].trim();
                return Some(String::from(inner.trim_matches('"').trim_matches('\'')));
            }
        }
        i += 1;
    }
    None
}

/// Parse `background-image` value: `url(...)`, `image-set(...)`, or CSS gradients.
fn parse_background_image_val(s: &str) -> Option<BackgroundImageVal> {
    let trimmed = s.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "none" {
        return Some(BackgroundImageVal::None);
    }
    if lower.starts_with("url(")
        || lower.starts_with("image-set(")
        || lower.starts_with("-webkit-image-set(")
    {
        return extract_css_url_function(trimmed).map(BackgroundImageVal::Url);
    }
    if lower.starts_with("linear-gradient(") && lower.ends_with(')') {
        let inner = &lower["linear-gradient(".len()..lower.len() - 1];
        return parse_linear_gradient(inner);
    }
    if lower.starts_with("radial-gradient(") && lower.ends_with(')') {
        let inner = &lower["radial-gradient(".len()..lower.len() - 1];
        return parse_radial_gradient(inner);
    }
    if lower.starts_with("conic-gradient(") && lower.ends_with(')') {
        let inner = &lower["conic-gradient(".len()..lower.len() - 1];
        return parse_conic_gradient(inner);
    }
    None
}

/// Parse the interior of `linear-gradient(...)`.
fn parse_linear_gradient(inner: &str) -> Option<BackgroundImageVal> {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.is_empty() {
        return None;
    }

    let mut angle_deg: i32 = 180; // default top-to-bottom
    let mut stops = Vec::new();
    let mut start_idx = 0;

    // Check if first part is an angle or direction
    let first = parts[0].trim();
    let first_direction = first
        .split_once(" in ")
        .map(|(dir, _)| dir.trim())
        .unwrap_or(first);
    if let Some(a) = parse_gradient_angle(first_direction) {
        angle_deg = a;
        start_idx = 1;
    } else if first_direction.starts_with("to ") {
        angle_deg = match first_direction {
            "to top" => 0,
            "to right" => 90,
            "to bottom" => 180,
            "to left" => 270,
            "to top right" | "to right top" => 45,
            "to bottom right" | "to right bottom" => 135,
            "to bottom left" | "to left bottom" => 225,
            "to top left" | "to left top" => 315,
            _ => return None,
        };
        start_idx = 1;
    } else if looks_like_invalid_gradient_angle(first_direction) {
        return None;
    }

    for i in start_idx..parts.len() {
        let part = parts[i].trim();
        let (color_str, position_str) = split_gradient_stop(part);
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))?;
        let position = if let Some(pos) = position_str {
            parse_gradient_position(pos)
        } else {
            -1 // auto
        };
        stops.push(GradientStop { color, position });
    }

    // Auto-distribute positions for stops with position == -1
    if !stops.is_empty() {
        let len = stops.len();
        if stops[0].position < 0 {
            stops[0].position = 0;
        }
        if len > 1 && stops[len - 1].position < 0 {
            stops[len - 1].position = 10000;
        }
        // Interpolate auto positions
        let mut i = 1;
        while i < len - 1 {
            if stops[i].position < 0 {
                // Find next non-auto
                let mut j = i + 1;
                while j < len && stops[j].position < 0 {
                    j += 1;
                }
                if j < len {
                    let start_pos = stops[i - 1].position;
                    let end_pos = stops[j].position;
                    let span = j - i + 1;
                    for k in i..j {
                        stops[k].position =
                            start_pos + (end_pos - start_pos) * (k - i + 1) as i32 / span as i32;
                    }
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
    }

    Some(BackgroundImageVal::LinearGradient { angle_deg, stops })
}

fn parse_radial_gradient(inner: &str) -> Option<BackgroundImageVal> {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.len() < 2 {
        return None;
    }

    let mut center_x = 5000;
    let mut center_y = 5000;
    let mut start_idx = 0;
    let first = parts[0].trim();
    if !looks_like_color_stop(first) {
        if let Some((cx, cy)) = parse_radial_position(first) {
            center_x = cx;
            center_y = cy;
        }
        start_idx = 1;
    }

    let mut stops = Vec::new();
    for part in parts.iter().skip(start_idx) {
        let part = part.trim();
        let (color_str, position_str) = split_gradient_stop(part);
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))?;
        let position = if let Some(pos) = position_str {
            parse_gradient_position(pos)
        } else {
            -1
        };
        stops.push(GradientStop { color, position });
    }
    distribute_gradient_positions(&mut stops);
    Some(BackgroundImageVal::RadialGradient {
        center_x,
        center_y,
        stops,
    })
}

fn parse_conic_gradient(inner: &str) -> Option<BackgroundImageVal> {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.len() < 2 {
        return None;
    }

    let mut from_deg = 0;
    let mut center_x = 5000;
    let mut center_y = 5000;
    let mut start_idx = 0;
    let first = parts[0].trim();
    if !looks_like_color_stop(first) {
        let (from, cx, cy) = parse_conic_prelude(first);
        from_deg = from;
        center_x = cx;
        center_y = cy;
        start_idx = 1;
    }

    let mut stops = Vec::new();
    for part in parts.iter().skip(start_idx) {
        let part = part.trim();
        let (color_str, position_str) = split_gradient_stop(part);
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))?;
        let position = if let Some(pos) = position_str {
            parse_conic_gradient_position(pos)
        } else {
            -1
        };
        stops.push(GradientStop { color, position });
    }
    distribute_gradient_positions(&mut stops);
    Some(BackgroundImageVal::ConicGradient {
        from_deg,
        center_x,
        center_y,
        stops,
    })
}

fn parse_conic_prelude(s: &str) -> (i32, i32, i32) {
    let mut from_deg = 0;
    let mut center_x = 5000;
    let mut center_y = 5000;
    let lower = s.replace('_', " ").to_ascii_lowercase();
    let mut before_at = lower.as_str();
    let mut after_at = "";
    if let Some((before, after)) = lower.split_once(" at ") {
        before_at = before.trim();
        after_at = after.trim();
    }

    if let Some(rest) = before_at.strip_prefix("from ") {
        if let Some(angle) = parse_gradient_angle(rest.trim()) {
            from_deg = angle;
        }
    }
    if !after_at.is_empty() {
        if let Some((cx, cy)) = parse_radial_position(after_at) {
            center_x = cx;
            center_y = cy;
        }
    }
    (from_deg, center_x, center_y)
}

fn looks_like_color_stop(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with('#')
        || lower.starts_with("rgb(")
        || lower.starts_with("rgba(")
        || lower.starts_with("hsl(")
        || lower.starts_with("hsla(")
        || lower == "transparent"
        || crate::css::named_color_pub(&lower).is_some()
}

fn parse_radial_position(s: &str) -> Option<(i32, i32)> {
    let lower = s.replace('_', " ");
    let lower = lower.to_ascii_lowercase();
    let after_at = lower
        .split_once(" at ")
        .map(|(_, pos)| pos.trim())
        .unwrap_or_else(|| lower.trim());
    if after_at.is_empty() || after_at == "circle" || after_at == "ellipse" {
        return Some((5000, 5000));
    }

    let mut cx = 5000;
    let mut cy = 5000;
    let mut saw_pos = false;
    for token in after_at.split_whitespace() {
        match token {
            "left" => {
                cx = 0;
                saw_pos = true;
            }
            "right" => {
                cx = 10000;
                saw_pos = true;
            }
            "top" => {
                cy = 0;
                saw_pos = true;
            }
            "bottom" => {
                cy = 10000;
                saw_pos = true;
            }
            "center" => saw_pos = true,
            _ if token.ends_with('%') => {
                let pct = parse_i32_prefix(&token[..token.len().saturating_sub(1)]).unwrap_or(50);
                if cx == 5000 {
                    cx = (pct * 100).clamp(0, 10000);
                } else {
                    cy = (pct * 100).clamp(0, 10000);
                }
                saw_pos = true;
            }
            _ => {}
        }
    }
    if saw_pos {
        Some((cx, cy))
    } else {
        Some((5000, 5000))
    }
}

fn parse_i32_prefix(s: &str) -> Option<i32> {
    let mut sign = 1;
    let mut value = 0i32;
    let mut saw_digit = false;
    for (idx, ch) in s.trim().chars().enumerate() {
        if idx == 0 && ch == '-' {
            sign = -1;
            continue;
        }
        if let Some(digit) = ch.to_digit(10) {
            saw_digit = true;
            value = value.saturating_mul(10).saturating_add(digit as i32);
        } else {
            break;
        }
    }
    saw_digit.then_some(value.saturating_mul(sign))
}

fn distribute_gradient_positions(stops: &mut [GradientStop]) {
    if stops.is_empty() {
        return;
    }
    let len = stops.len();
    if stops[0].position < 0 {
        stops[0].position = 0;
    }
    if len > 1 && stops[len - 1].position < 0 {
        stops[len - 1].position = 10000;
    }
    let mut i = 1;
    while i < len.saturating_sub(1) {
        if stops[i].position < 0 {
            let mut j = i + 1;
            while j < len && stops[j].position < 0 {
                j += 1;
            }
            if j < len {
                let start_pos = stops[i - 1].position;
                let end_pos = stops[j].position;
                let span = j - i + 1;
                for k in i..j {
                    stops[k].position =
                        start_pos + (end_pos - start_pos) * (k - i + 1) as i32 / span as i32;
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

fn split_gradient_stop(part: &str) -> (&str, Option<&str>) {
    let s = part.trim();
    if s.is_empty() {
        return (s, None);
    }
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut depth = 0u32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b.is_ascii_whitespace() && depth == 0 {
            let color = s[..i].trim();
            let rest = s[i..].trim();
            return (color, if rest.is_empty() { None } else { Some(rest) });
        }
        i += 1;
    }
    (s, None)
}

fn parse_gradient_angle(s: &str) -> Option<i32> {
    if s.ends_with("deg") {
        return s
            .trim_end_matches("deg")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|a| a as i32);
    }
    if s.ends_with("grad") {
        return s
            .trim_end_matches("grad")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|g| (g * 0.9) as i32);
    }
    if s.ends_with("rad") {
        return s
            .trim_end_matches("rad")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|r| (r * 180.0 / core::f32::consts::PI) as i32);
    }
    if s.ends_with("turn") {
        return s
            .trim_end_matches("turn")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|t| (t * 360.0) as i32);
    }
    None
}

fn looks_like_invalid_gradient_angle(s: &str) -> bool {
    let trimmed = s.trim();
    let starts_numeric = trimmed
        .as_bytes()
        .first()
        .map(|b| b.is_ascii_digit() || *b == b'+' || *b == b'-' || *b == b'.')
        .unwrap_or(false);
    starts_numeric && trimmed.as_bytes().iter().any(|b| b.is_ascii_alphabetic())
}

fn parse_gradient_position(s: &str) -> i32 {
    if s.ends_with('%') {
        if let Ok(v) = s.trim_end_matches('%').parse::<f32>() {
            return (v * 100.0) as i32;
        }
    }
    -1
}

fn parse_conic_gradient_position(s: &str) -> i32 {
    let pos = s.split_whitespace().next().unwrap_or(s).trim();
    if let Some(angle) = parse_gradient_angle(pos) {
        let mut normalized = angle % 360;
        if normalized < 0 {
            normalized += 360;
        }
        return ((normalized as i64 * 10000) / 360) as i32;
    }
    parse_gradient_position(pos)
}

fn split_comma_respecting_parens(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn split_transform_component_list(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' | b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => {
                if start < i {
                    let part = s[start..i].trim();
                    if !part.is_empty() {
                        parts.push(part);
                    }
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        let part = s[start..].trim();
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts
}

fn parse_individual_translate(
    value: &CssValue,
    parent_fs: i32,
    root_fs: i32,
) -> Option<(i32, i32, i32, i32)> {
    match value {
        CssValue::Length(_, _)
        | CssValue::Percentage(_)
        | CssValue::Calc(_, _)
        | CssValue::Number(_) => {
            let (tx, tx_pct) = translate_component_from_value(value, parent_fs, root_fs)?;
            Some((tx, 0, tx_pct, 0))
        }
        CssValue::Keyword(s) => {
            let parts = split_transform_component_list(s);
            if parts.is_empty() {
                return None;
            }
            let (tx, tx_pct) = translate_component_from_str(parts[0], parent_fs, root_fs)?;
            let (ty, ty_pct) = if let Some(y) = parts.get(1) {
                translate_component_from_str(y, parent_fs, root_fs)?
            } else {
                (0, 0)
            };
            Some((tx, ty, tx_pct, ty_pct))
        }
        _ => None,
    }
}

fn translate_component_from_value(
    value: &CssValue,
    parent_fs: i32,
    root_fs: i32,
) -> Option<(i32, i32)> {
    match value {
        CssValue::Length(_, _) => resolve_length(value, parent_fs, root_fs).map(|px| (px, 0)),
        CssValue::Percentage(pct) => Some((0, *pct)),
        CssValue::Calc(px, pct) => Some((px / 100, *pct)),
        CssValue::Number(n) => Some((n / 100, 0)),
        _ => None,
    }
}

fn translate_component_from_str(s: &str, parent_fs: i32, root_fs: i32) -> Option<(i32, i32)> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.starts_with("calc(")
        || lower.starts_with("min(")
        || lower.starts_with("max(")
        || lower.starts_with("clamp(")
    {
        let parsed = crate::css::parse_value(&Property::Width, s);
        translate_component_from_value(&parsed, parent_fs, root_fs)
    } else {
        Some(parse_transform_translate_component(s, parent_fs))
    }
}

fn parse_individual_scale(value: &CssValue) -> Option<(i32, i32)> {
    match value {
        CssValue::Number(n) => {
            let scale = *n * 10;
            Some((scale, scale))
        }
        CssValue::Percentage(p) => {
            let scale = *p / 10;
            Some((scale, scale))
        }
        CssValue::Keyword(s) => {
            let parts = split_transform_component_list(s);
            if parts.is_empty() {
                return None;
            }
            let sx = parse_scale_component(parts[0])?;
            let sy = if let Some(y) = parts.get(1) {
                parse_scale_component(y)?
            } else {
                sx
            };
            Some((sx, sy))
        }
        _ => None,
    }
}

fn parse_scale_component(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        parse_decimal_fixed100(num).map(|v| v / 10)
    } else {
        parse_decimal_fixed100(s).map(|v| v * 10)
    }
}

fn parse_individual_rotate(value: &CssValue) -> Option<i32> {
    match value {
        CssValue::Number(n) => Some(*n),
        CssValue::Keyword(s) => parse_angle_deg100(s),
        _ => None,
    }
}

fn parse_angle_deg100(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("deg") {
        parse_decimal_fixed100(num)
    } else if let Some(num) = s.strip_suffix("turn") {
        parse_decimal_fixed100(num).map(|v| v * 360)
    } else if let Some(num) = s.strip_suffix("rad") {
        parse_decimal_fixed100(num).map(|v| (v as i64 * 18000 / 314) as i32)
    } else {
        parse_decimal_fixed100(s)
    }
}

fn parse_decimal_fixed100(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let neg = s.starts_with('-');
    let s = if neg || s.starts_with('+') {
        &s[1..]
    } else {
        s
    };
    let mut int_part = 0i32;
    let mut frac = 0i32;
    let mut in_frac = false;
    let mut frac_mul = 10;
    let mut saw_digit = false;
    for &b in s.as_bytes() {
        if b == b'.' && !in_frac {
            in_frac = true;
            continue;
        }
        if !b.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        if in_frac {
            if frac_mul <= 100 {
                frac += (b - b'0') as i32 * (100 / frac_mul);
                frac_mul *= 10;
            }
        } else {
            int_part = int_part.saturating_mul(10) + (b - b'0') as i32;
        }
    }
    if !saw_digit {
        return None;
    }
    let value = int_part.saturating_mul(100).saturating_add(frac);
    Some(if neg { -value } else { value })
}

#[cfg(test)]
mod declaration_tests {
    use super::*;

    #[test]
    fn inline_style_applies_max_width_calc() {
        let decls = crate::css::parse_inline_style("max-width: calc(50% - 3px)");
        assert_eq!(decls.len(), 1);
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.max_width, None);
        assert_eq!(style.max_width_calc, Some((-300, 5000)));
    }

    #[test]
    fn calc_with_nested_var_is_resolved_after_custom_property_lookup() {
        let decls =
            crate::css::parse_inline_style("width: calc(956px + 2 * var(--container-spacing))");
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0].value, CssValue::Keyword(_)));

        let resolved = crate::css::parse_value(&Property::Width, "calc(956px + 2 * 20px)");
        assert!(matches!(resolved, CssValue::Length(99_600, Unit::Px)));
    }

    #[test]
    fn calc_preserves_nested_function_parentheses() {
        let resolved = crate::css::parse_value(
            &Property::Width,
            "calc(100% - 0px - env(safe-area-inset-left) - env(safe-area-inset-right))",
        );
        assert!(matches!(resolved, CssValue::Percentage(10000)));
    }

    #[test]
    fn logical_inset_properties_expand_to_physical_offsets() {
        let decls = crate::css::parse_inline_style(
            "inset-inline: calc(.25rem * 6); inset-block: 50%; inset-inline-start: 3px",
        );
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }

        assert_eq!(style.left_offset, Some(3));
        assert_eq!(style.right_offset, Some(24));
        assert_eq!(style.top, None);
        assert_eq!(style.top_calc, Some((0, 5000)));
        assert_eq!(style.bottom_offset, None);
        assert_eq!(style.bottom_calc, Some((0, 5000)));
    }

    #[test]
    fn transform_translate_percent_uses_fixed_percent_units() {
        let decls = crate::css::parse_inline_style("transform: translateX(-50%) translateY(25%)");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx_pct, -5000);
        assert_eq!(style.transform_ty_pct, 2500);
    }

    #[test]
    fn individual_translate_percent_uses_fixed_percent_units() {
        let decls = crate::css::parse_inline_style("translate: -50% 25%");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx_pct, -5000);
        assert_eq!(style.transform_ty_pct, 2500);
    }

    #[test]
    fn individual_transform_properties_apply_translate_scale_and_rotate() {
        let decls = crate::css::parse_inline_style(
            "translate: calc(1rem * -2) 50%; scale: 1.05 95%; rotate: 0.25turn",
        );
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx, -32);
        assert_eq!(style.transform_ty_pct, 5000);
        assert_eq!(style.transform_sx, 1050);
        assert_eq!(style.transform_sy, 950);
        assert_eq!(style.transform_rotate, 9000);
    }

    #[test]
    fn border_radius_accepts_percentage_for_avatar_circles() {
        let decls = crate::css::parse_inline_style("border-radius: 50%");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.border_top_left_radius, -5000);
        assert_eq!(style.border_top_right_radius, -5000);
        assert_eq!(style.border_bottom_right_radius, -5000);
        assert_eq!(style.border_bottom_left_radius, -5000);
    }

    #[test]
    fn invalid_gradient_angle_is_rejected() {
        assert!(parse_background_image_val("linear-gradient(90degree, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(100gradian, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(1.57radian, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(0.25turns, red, red)").is_none());
    }

    #[test]
    fn linear_gradient_accepts_space_separated_function_colors() {
        let parsed = parse_background_image_val(
            "linear-gradient(to right, oklch(0.65 0.2 280), rgb(59 130 246 / 1))",
        );
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::LinearGradient { ref stops, .. }) if stops.len() == 2
        ));
    }

    #[test]
    fn linear_gradient_accepts_modern_color_interpolation_space() {
        let parsed = parse_background_image_val(
            "linear-gradient(to right in oklab, #863bff 0%, #47bfff 100%)",
        );
        match parsed {
            Some(BackgroundImageVal::LinearGradient {
                angle_deg,
                ref stops,
            }) => {
                assert_eq!(angle_deg, 90);
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].color, 0xFF863BFF);
                assert_eq!(stops[1].color, 0xFF47BFFF);
            }
            _ => panic!("expected modern color-space gradient"),
        }
    }

    #[test]
    fn background_url_preserves_asset_path_case() {
        let parsed = parse_background_image_val("url('/Images/HeroLarge.PNG')");
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/Images/HeroLarge.PNG"
        ));
    }

    #[test]
    fn background_image_set_uses_first_url_candidate() {
        let parsed =
            parse_background_image_val("image-set(url('/hero.avif') 1x, url('/hero@2x.avif') 2x)");
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/hero.avif"
        ));

        let parsed = parse_background_image_val(
            "-webkit-image-set(url(\"/Promo/Hero.JPG\") 1x, url(\"/Promo/Hero_2x.JPG\") 2x)",
        );
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/Promo/Hero.JPG"
        ));
    }
}

fn resolve_border_radius(value: &CssValue, parent_fs: i32, root_fs: i32) -> Option<i32> {
    match value {
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => Some(-(*v).max(0)),
        _ => resolve_length(value, parent_fs, root_fs),
    }
}

fn parse_bg_size_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    let s = s.trim();
    if s == "auto" {
        return -1;
    }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    -1
}

fn parse_bg_position_part(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    match s {
        "left" | "top" => 0,
        "center" => 5000,            // 50% * 100
        "right" | "bottom" => 10000, // 100% * 100
        _ => {
            if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    return px;
                }
            }
            0
        }
    }
}

fn parse_position_component(
    s: &str,
    parent_fs: i32,
    root_fs: i32,
    default_value: i32,
    default_is_percent: bool,
) -> (i32, bool) {
    let s = s.trim();
    match s {
        "left" | "top" => (0, true),
        "center" => (5000, true),
        "right" | "bottom" => (10000, true),
        _ => {
            if let Some(stripped) = s.strip_suffix('%') {
                if let Ok(v) = stripped.trim().parse::<i32>() {
                    return (v * 100, true);
                }
            }
            if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
                if matches!(
                    dim,
                    CssValue::Length(_, Unit::Percent) | CssValue::Percentage(_)
                ) {
                    match dim {
                        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => {
                            return (v, true);
                        }
                        _ => {}
                    }
                }
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    return (px, false);
                }
            }
            (default_value, default_is_percent)
        }
    }
}

fn parse_position_pair(
    s: &str,
    parent_fs: i32,
    root_fs: i32,
    default_x: i32,
    default_x_is_percent: bool,
    default_y: i32,
    default_y_is_percent: bool,
) -> (i32, bool, i32, bool) {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return (
            default_x,
            default_x_is_percent,
            default_y,
            default_y_is_percent,
        );
    }

    if parts.len() == 1 {
        let part = parts[0];
        if matches!(part, "top" | "bottom") {
            let (y, y_is_percent) =
                parse_position_component(part, parent_fs, root_fs, default_y, default_y_is_percent);
            return (default_x, default_x_is_percent, y, y_is_percent);
        }
        let (x, x_is_percent) =
            parse_position_component(part, parent_fs, root_fs, default_x, default_x_is_percent);
        return (x, x_is_percent, default_y, default_y_is_percent);
    }

    let (x, x_is_percent) = parse_position_component(
        parts[0],
        parent_fs,
        root_fs,
        default_x,
        default_x_is_percent,
    );
    let (y, y_is_percent) = parse_position_component(
        parts[1],
        parent_fs,
        root_fs,
        default_y,
        default_y_is_percent,
    );
    (x, x_is_percent, y, y_is_percent)
}

// ---------------------------------------------------------------------------
