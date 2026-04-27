fn expand_border_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    let mut width_val: Option<CssValue> = None;
    let mut style_val: Option<CssValue> = None;
    let mut color_val: Option<CssValue> = None;
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            style_val = Some(CssValue::Keyword(lower));
        } else if lower == "currentcolor" {
            color_val = Some(CssValue::CurrentColor);
        } else if lower.starts_with("var(") {
            color_val = Some(parse_var_value(&Property::BorderColor, part));
        } else if let Some(c) = try_parse_color(part) {
            color_val = Some(CssValue::Color(c));
        } else if let Some(c) = named_color(&lower) {
            color_val = Some(CssValue::Color(c));
        } else if let Some(dim) = try_parse_dimension(part) {
            width_val = Some(dim);
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            width_val = Some(CssValue::Keyword(lower));
        } else if lower == "none" {
            style_val = Some(CssValue::None);
            width_val = Some(CssValue::Length(0, Unit::Px));
        }
    }
    if let Some(ref sv) = style_val {
        decls.push(Declaration { property: Property::BorderStyle, value: sv.clone(), important: false });
    }
    if let Some(ref cv) = color_val {
        decls.push(Declaration { property: Property::BorderColor, value: cv.clone(), important: false });
    }
    if let Some(ref wv) = width_val {
        decls.push(Declaration { property: Property::BorderWidth, value: wv.clone(), important: false });
    }
    for side_w in &[Property::BorderTopWidth, Property::BorderRightWidth, Property::BorderBottomWidth, Property::BorderLeftWidth] {
        if let Some(ref wv) = width_val {
            decls.push(Declaration { property: side_w.clone(), value: wv.clone(), important: false });
        }
    }
    for side_s in &[Property::BorderTopStyle, Property::BorderRightStyle, Property::BorderBottomStyle, Property::BorderLeftStyle] {
        if let Some(ref sv) = style_val {
            decls.push(Declaration { property: side_s.clone(), value: sv.clone(), important: false });
        }
    }
    for side_c in &[Property::BorderTopColor, Property::BorderRightColor, Property::BorderBottomColor, Property::BorderLeftColor] {
        if let Some(ref cv) = color_val {
            decls.push(Declaration { property: side_c.clone(), value: cv.clone(), important: false });
        }
    }
    decls
}

fn expand_background_shorthand(value_str: &str) -> Vec<Declaration> {
    let s = value_str.trim();
    let lower = to_ascii_lower(s);
    if lower == "none" || lower == "transparent" {
        let mut v = Vec::new();
        v.push(Declaration { property: Property::BackgroundColor, value: CssValue::Color(0x00000000), important: false });
        return v;
    }
    if lower == "inherit" {
        let mut v = Vec::new();
        v.push(Declaration { property: Property::BackgroundColor, value: CssValue::Inherit, important: false });
        return v;
    }
    if lower.starts_with("var(") {
        let var_val = parse_var_value(&Property::BackgroundColor, s);
        let mut v = Vec::new();
        v.push(Declaration { property: Property::BackgroundColor, value: var_val, important: false });
        return v;
    }
    let mut found_color: Option<u32> = None;
    let mut found_var: Option<CssValue> = None;
    let mut found_image: Option<String> = None;
    let mut found_repeat: Option<String> = None;
    let mut position_parts: Vec<String> = Vec::new();
    let raw_parts: Vec<&str> = split_background_tokens(s);
    let parts = reassemble_var_parts(&raw_parts.iter().map(|s| *s).collect::<Vec<&str>>());
    for part in &parts {
        let pl = to_ascii_lower(part);
        if pl.starts_with("var(") {
            found_var = Some(parse_var_value(&Property::BackgroundColor, part));
            continue;
        }
        if pl.starts_with("url(")
            || pl.starts_with("linear-gradient(")
            || pl.starts_with("radial-gradient(")
            || pl.starts_with("conic-gradient(")
            || pl.starts_with("repeating-")
        {
            found_image = Some((*part).to_string());
            continue;
        }
        if matches!(pl.as_str(), "no-repeat" | "repeat" | "repeat-x" | "repeat-y") {
            found_repeat = Some(pl);
            continue;
        }
        if matches!(pl.as_str(), "center" | "left" | "right" | "top" | "bottom")
            || pl.ends_with('%')
            || pl.ends_with("px")
            || pl.ends_with("em")
            || pl.ends_with("rem")
            || pl.ends_with("vw")
            || pl.ends_with("vh")
        {
            if position_parts.len() < 2 {
                position_parts.push((*part).to_string());
            }
            continue;
        }
        if matches!(pl.as_str(), "cover" | "contain" | "fixed" | "scroll" | "local" | "border-box" | "padding-box" | "content-box") {
            continue;
        }
        if pl == "transparent" {
            found_color = Some(0x00000000);
            continue;
        }
        if pl == "currentcolor" {
            let mut v = Vec::new();
            v.push(Declaration { property: Property::BackgroundColor, value: CssValue::CurrentColor, important: false });
            return v;
        }
        if let Some(c) = try_parse_color(part) {
            found_color = Some(c);
            continue;
        }
        if let Some(c) = named_color(&pl) {
            found_color = Some(c);
            continue;
        }
    }
    let mut v = Vec::new();
    if let Some(image) = found_image {
        v.push(Declaration { property: Property::BackgroundImage, value: CssValue::Keyword(image), important: false });
    }
    if let Some(repeat) = found_repeat {
        v.push(Declaration { property: Property::BackgroundRepeat, value: CssValue::Keyword(repeat), important: false });
    }
    if !position_parts.is_empty() {
        v.push(Declaration {
            property: Property::BackgroundPosition,
            value: CssValue::Keyword(position_parts.join(" ")),
            important: false,
        });
    }
    if let Some(c) = found_color {
        v.push(Declaration { property: Property::BackgroundColor, value: CssValue::Color(c), important: false });
    } else if let Some(var_val) = found_var {
        v.push(Declaration { property: Property::BackgroundColor, value: var_val, important: false });
    }
    v
}

fn split_background_tokens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            b',' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

fn expand_border_side_shorthand(value_str: &str, width_prop: Property, style_prop: Property, color_prop: Property) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" | "hidden") {
            decls.push(Declaration { property: style_prop.clone(), value: CssValue::Keyword(lower), important: false });
        } else if lower == "none" {
            decls.push(Declaration { property: style_prop.clone(), value: CssValue::None, important: false });
            decls.push(Declaration { property: width_prop.clone(), value: CssValue::Length(0, Unit::Px), important: false });
        } else if lower == "currentcolor" {
            decls.push(Declaration { property: color_prop.clone(), value: CssValue::CurrentColor, important: false });
        } else if lower.starts_with("var(") {
            decls.push(Declaration { property: color_prop.clone(), value: parse_var_value(&color_prop, part), important: false });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration { property: color_prop.clone(), value: CssValue::Color(c), important: false });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration { property: color_prop.clone(), value: CssValue::Color(c), important: false });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration { property: width_prop.clone(), value: dim, important: false });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration { property: width_prop.clone(), value: CssValue::Keyword(lower), important: false });
        }
    }
    decls
}

fn expand_outline_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(lower.as_str(), "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset") {
            decls.push(Declaration { property: Property::OutlineStyle, value: CssValue::Keyword(lower), important: false });
        } else if lower == "none" {
            decls.push(Declaration { property: Property::OutlineStyle, value: CssValue::None, important: false });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration { property: Property::OutlineColor, value: CssValue::Color(c), important: false });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration { property: Property::OutlineColor, value: CssValue::Color(c), important: false });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration { property: Property::OutlineWidth, value: dim, important: false });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration { property: Property::OutlineWidth, value: CssValue::Keyword(lower), important: false });
        }
    }
    decls
}

fn expand_text_decoration_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();
    let line_kw = if lower.contains("underline") {
        "underline"
    } else if lower.contains("line-through") {
        "line-through"
    } else if lower.contains("overline") {
        "overline"
    } else if lower.contains("none") {
        "none"
    } else {
        "none"
    };
    decls.push(Declaration { property: Property::TextDecoration, value: CssValue::Keyword(String::from(line_kw)), important: false });
    decls
}
