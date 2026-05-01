pub fn parse_value(property: &Property, value_str: &str) -> CssValue {
    let s = value_str.trim();
    if s.is_empty() {
        return CssValue::None;
    }

    let lower = to_ascii_lower(s);
    match lower.as_str() {
        "auto" => return CssValue::Auto,
        "none" => return CssValue::None,
        "inherit" => return CssValue::Inherit,
        "transparent" => return CssValue::Color(0x00000000),
        _ => {}
    }

    if lower.starts_with("var(") {
        return parse_var_value(property, s);
    }
    if lower.contains("var(") {
        return CssValue::Keyword(String::from(s));
    }
    if lower.starts_with("calc(") {
        return parse_calc_value(s);
    }
    if lower.starts_with("min(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Min);
    }
    if lower.starts_with("max(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Max);
    }
    if lower.starts_with("clamp(") {
        return parse_min_max_clamp_value(s, CssMathFunc::Clamp);
    }

    if lower == "currentcolor" {
        return CssValue::CurrentColor;
    }

    if is_color_property(property) {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    if s.starts_with('#') || lower.starts_with("rgb") {
        if let Some(c) = try_parse_color(s) {
            return CssValue::Color(c);
        }
    }

    if is_color_property(property) {
        if let Some(c) = named_color(&lower) {
            return CssValue::Color(c);
        }
    }

    if let Some(v) = try_parse_dimension(s) {
        return v;
    }

    let is_case_sensitive = matches!(
        property,
        Property::GridColumn
            | Property::GridColumnStart
            | Property::GridColumnEnd
            | Property::GridRow
            | Property::GridRowStart
            | Property::GridRowEnd
            | Property::GridArea
            | Property::GridTemplateAreas
            | Property::FontFamily
            | Property::Content
            | Property::Background
            | Property::BackgroundImage
            | Property::MaskImage
            | Property::Filter
            | Property::BackdropFilter
            | Property::ClipPath
            | Property::Cursor
    );
    if is_case_sensitive {
        CssValue::Keyword(String::from(s))
    } else {
        CssValue::Keyword(lower)
    }
}

fn parse_var_value(property: &Property, s: &str) -> CssValue {
    let inner = s.trim();
    let inner = if inner.starts_with("var(") || inner.starts_with("VAR(") {
        &inner[4..]
    } else {
        inner
    };
    let inner = inner.trim_end_matches(')').trim();

    if let Some(comma) = inner.find(',') {
        let name = inner[..comma].trim();
        let fallback_str = inner[comma + 1..].trim();
        let fallback = if fallback_str.is_empty() {
            None
        } else {
            Some(Box::new(parse_value(property, fallback_str)))
        };
        CssValue::Var(String::from(name), fallback)
    } else {
        CssValue::Var(String::from(inner), None)
    }
}

fn is_color_property(p: &Property) -> bool {
    matches!(
        p,
        Property::Color
            | Property::BackgroundColor
            | Property::Background
            | Property::BorderColor
            | Property::BorderTopColor
            | Property::BorderRightColor
            | Property::BorderBottomColor
            | Property::BorderLeftColor
            | Property::OutlineColor
            | Property::TextDecorationColor
            | Property::AccentColor
    )
}
