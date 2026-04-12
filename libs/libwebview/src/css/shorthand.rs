fn is_expandable_shorthand(p: &Property) -> bool {
    matches!(
        p,
        Property::Margin
            | Property::Padding
            | Property::Border
            | Property::BorderTop
            | Property::BorderRight
            | Property::BorderBottom
            | Property::BorderLeft
            | Property::BorderRadius
            | Property::Outline
            | Property::Flex
            | Property::Gap
            | Property::Overflow
            | Property::Background
            | Property::TextDecoration
            | Property::Inset
            | Property::GridTemplate
            | Property::GridTemplateAreas
    )
}

/// Expand a shorthand property into individual declarations.
fn expand_shorthand(property: Property, value_str: &str) -> Vec<Declaration> {
    // If the ENTIRE value is a single var() call, don't expand the shorthand —
    // instead emit a single declaration with the primary property and var() value.
    // The var() will be resolved at style resolution time by apply_author_rules.
    let trimmed_lower = to_ascii_lower(value_str.trim());
    if trimmed_lower.starts_with("var(") && !trimmed_lower.contains(')')
        || (trimmed_lower.starts_with("var(")
            && trimmed_lower.ends_with(')')
            && trimmed_lower.matches(')').count() == 1)
    {
        let primary = match &property {
            Property::Background => Property::BackgroundColor,
            Property::Outline => Property::OutlineColor,
            _ => property.clone(),
        };
        let var_val = parse_var_value(value_str.trim());
        return alloc::vec![Declaration {
            property: primary,
            value: var_val,
            important: false
        }];
    }

    match &property {
        Property::Margin => expand_box_shorthand(
            value_str,
            Property::MarginTop,
            Property::MarginRight,
            Property::MarginBottom,
            Property::MarginLeft,
        ),
        Property::Padding => expand_box_shorthand(
            value_str,
            Property::PaddingTop,
            Property::PaddingRight,
            Property::PaddingBottom,
            Property::PaddingLeft,
        ),
        Property::Border => expand_border_shorthand(value_str),
        Property::Flex => expand_flex_shorthand(value_str),
        Property::Gap => expand_gap_shorthand(value_str),
        Property::Overflow => expand_overflow_shorthand(value_str),
        Property::Background => expand_background_shorthand(value_str),
        Property::BorderTop => expand_border_side_shorthand(
            value_str,
            Property::BorderTopWidth,
            Property::BorderTopStyle,
            Property::BorderTopColor,
        ),
        Property::BorderRight => expand_border_side_shorthand(
            value_str,
            Property::BorderRightWidth,
            Property::BorderRightStyle,
            Property::BorderRightColor,
        ),
        Property::BorderBottom => expand_border_side_shorthand(
            value_str,
            Property::BorderBottomWidth,
            Property::BorderBottomStyle,
            Property::BorderBottomColor,
        ),
        Property::BorderLeft => expand_border_side_shorthand(
            value_str,
            Property::BorderLeftWidth,
            Property::BorderLeftStyle,
            Property::BorderLeftColor,
        ),
        Property::BorderRadius => expand_border_radius_shorthand(value_str),
        Property::Outline => expand_outline_shorthand(value_str),
        Property::TextDecoration => expand_text_decoration_shorthand(value_str),
        Property::Inset => expand_box_shorthand(
            value_str,
            Property::Top,
            Property::Right,
            Property::Bottom,
            Property::Left,
        ),
        Property::GridTemplate => expand_grid_template_shorthand(value_str),
        Property::GridTemplateAreas => expand_grid_template_areas(value_str),
        _ => {
            let value = parse_value(&property, value_str);
            let mut v = Vec::new();
            v.push(Declaration {
                property: property.clone(),
                value,
                important: false,
            });
            v
        }
    }
}

/// Expand margin/padding shorthand: 1 value → all, 2 → TB/LR, 3 → T/LR/B, 4 → T/R/B/L.
fn expand_box_shorthand(
    value_str: &str,
    top: Property,
    right: Property,
    bottom: Property,
    left: Property,
) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let (t, r, b, l) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    let mut v = Vec::with_capacity(4);
    let v_t = parse_value(&top, t);
    v.push(Declaration {
        property: top,
        value: v_t,
        important: false,
    });
    let v_r = parse_value(&right, r);
    v.push(Declaration {
        property: right,
        value: v_r,
        important: false,
    });
    let v_b = parse_value(&bottom, b);
    v.push(Declaration {
        property: bottom,
        value: v_b,
        important: false,
    });
    let v_l = parse_value(&left, l);
    v.push(Declaration {
        property: left,
        value: v_l,
        important: false,
    });
    v
}

/// Reassemble var() calls that were split across whitespace-delimited parts.
/// E.g. ["1px", "solid", "var(", "--color-grey", ")"] → ["1px", "solid", "var( --color-grey )"]
fn reassemble_var_parts(parts: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let lower = parts[i].to_ascii_lowercase();
        if lower.starts_with("var(") && !lower.ends_with(')') {
            // Collect parts until we find one ending with ')'
            let mut combined = String::from(parts[i]);
            i += 1;
            while i < parts.len() {
                combined.push(' ');
                combined.push_str(parts[i]);
                if parts[i].ends_with(')') {
                    i += 1;
                    break;
                }
                i += 1;
            }
            result.push(combined);
        } else {
            result.push(String::from(parts[i]));
            i += 1;
        }
    }
    result
}

/// Expand `border: <width> <style> <color>` shorthand.
/// Sets both the unified properties AND per-side properties (like litehtml).
fn expand_border_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    // Reassemble var() calls that span multiple whitespace-split parts.
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    let mut width_val: Option<CssValue> = None;
    let mut style_val: Option<CssValue> = None;
    let mut color_val: Option<CssValue> = None;
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(
            lower.as_str(),
            "solid"
                | "dashed"
                | "dotted"
                | "double"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
                | "hidden"
        ) {
            style_val = Some(CssValue::Keyword(lower));
        } else if lower.starts_with("var(") {
            // var() reference — store as Var for later resolution.
            color_val = Some(parse_var_value(part));
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
    // Emit unified properties
    if let Some(ref sv) = style_val {
        decls.push(Declaration {
            property: Property::BorderStyle,
            value: sv.clone(),
            important: false,
        });
    }
    if let Some(ref cv) = color_val {
        decls.push(Declaration {
            property: Property::BorderColor,
            value: cv.clone(),
            important: false,
        });
    }
    if let Some(ref wv) = width_val {
        decls.push(Declaration {
            property: Property::BorderWidth,
            value: wv.clone(),
            important: false,
        });
    }
    // Emit per-side properties for consistent per-side override support
    for side_w in &[
        Property::BorderTopWidth,
        Property::BorderRightWidth,
        Property::BorderBottomWidth,
        Property::BorderLeftWidth,
    ] {
        if let Some(ref wv) = width_val {
            decls.push(Declaration {
                property: side_w.clone(),
                value: wv.clone(),
                important: false,
            });
        }
    }
    for side_s in &[
        Property::BorderTopStyle,
        Property::BorderRightStyle,
        Property::BorderBottomStyle,
        Property::BorderLeftStyle,
    ] {
        if let Some(ref sv) = style_val {
            decls.push(Declaration {
                property: side_s.clone(),
                value: sv.clone(),
                important: false,
            });
        }
    }
    for side_c in &[
        Property::BorderTopColor,
        Property::BorderRightColor,
        Property::BorderBottomColor,
        Property::BorderLeftColor,
    ] {
        if let Some(ref cv) = color_val {
            decls.push(Declaration {
                property: side_c.clone(),
                value: cv.clone(),
                important: false,
            });
        }
    }
    decls
}

/// Expand `flex: <grow> [<shrink>] [<basis>]` shorthand.
fn expand_flex_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();

    match lower.as_str() {
        "none" => {
            decls.push(Declaration {
                property: Property::FlexGrow,
                value: CssValue::Number(0),
                important: false,
            });
            decls.push(Declaration {
                property: Property::FlexShrink,
                value: CssValue::Number(0),
                important: false,
            });
            decls.push(Declaration {
                property: Property::FlexBasis,
                value: CssValue::Auto,
                important: false,
            });
            return decls;
        }
        "auto" => {
            decls.push(Declaration {
                property: Property::FlexGrow,
                value: CssValue::Number(100),
                important: false,
            });
            decls.push(Declaration {
                property: Property::FlexShrink,
                value: CssValue::Number(100),
                important: false,
            });
            decls.push(Declaration {
                property: Property::FlexBasis,
                value: CssValue::Auto,
                important: false,
            });
            return decls;
        }
        _ => {}
    }

    let parts: Vec<&str> = value_str.split_whitespace().collect();
    if parts.is_empty() {
        return decls;
    }

    decls.push(Declaration {
        property: Property::FlexGrow,
        value: parse_value(&Property::FlexGrow, parts[0]),
        important: false,
    });

    // CSS spec: `flex: <number>` is shorthand for `flex: <number> 1 0`.
    // If only one value, set shrink=1 and basis=0 (not auto).
    if parts.len() == 1 {
        decls.push(Declaration {
            property: Property::FlexShrink,
            value: CssValue::Number(100),
            important: false,
        });
        decls.push(Declaration {
            property: Property::FlexBasis,
            value: CssValue::Length(0, Unit::Px),
            important: false,
        });
        return decls;
    }

    if parts.len() >= 2 {
        if let Some(dim) = try_parse_dimension(parts[1]) {
            if matches!(dim, CssValue::Length(_, _) | CssValue::Percentage(_)) {
                decls.push(Declaration {
                    property: Property::FlexShrink,
                    value: CssValue::Number(100),
                    important: false,
                });
                decls.push(Declaration {
                    property: Property::FlexBasis,
                    value: dim,
                    important: false,
                });
            } else {
                decls.push(Declaration {
                    property: Property::FlexShrink,
                    value: dim,
                    important: false,
                });
            }
        } else {
            decls.push(Declaration {
                property: Property::FlexShrink,
                value: parse_value(&Property::FlexShrink, parts[1]),
                important: false,
            });
        }
    }

    if parts.len() >= 3 {
        decls.push(Declaration {
            property: Property::FlexBasis,
            value: parse_value(&Property::FlexBasis, parts[2]),
            important: false,
        });
    }

    decls
}

/// Expand `gap: <row> [<column>]` shorthand.
fn expand_gap_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration {
        property: Property::RowGap,
        value: parse_value(&Property::RowGap, parts[0]),
        important: false,
    });
    let col = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration {
        property: Property::ColumnGap,
        value: parse_value(&Property::ColumnGap, col),
        important: false,
    });
    decls
}

/// Expand `overflow: <x> [<y>]` shorthand.
fn expand_overflow_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration {
        property: Property::OverflowX,
        value: parse_value(&Property::OverflowX, parts[0]),
        important: false,
    });
    let y = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration {
        property: Property::OverflowY,
        value: parse_value(&Property::OverflowY, y),
        important: false,
    });
    decls
}

/// Expand `background` shorthand — extract color and ignore image/repeat/position.
fn expand_background_shorthand(value_str: &str) -> Vec<Declaration> {
    let s = value_str.trim();
    let lower = to_ascii_lower(s);

    // Handle simple keywords.
    if lower == "none" || lower == "transparent" {
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Color(0x00000000),
            important: false,
        });
        return v;
    }
    if lower == "inherit" {
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Inherit,
            important: false,
        });
        return v;
    }

    // Handle var() — store as Var for later resolution by apply_author_rules.
    if lower.starts_with("var(") {
        let var_val = parse_var_value(s);
        let mut v = Vec::new();
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: var_val,
            important: false,
        });
        return v;
    }

    // Scan tokens for a color value; skip url(...), gradient functions, and keywords
    // like no-repeat, center, cover, etc.
    let mut found_color: Option<u32> = None;
    let mut found_var: Option<CssValue> = None;
    let raw_parts: Vec<&str> = split_background_tokens(s);
    let parts = reassemble_var_parts(&raw_parts.iter().map(|s| *s).collect::<Vec<&str>>());
    for part in &parts {
        let pl = to_ascii_lower(part);
        // Handle var() reference within background shorthand.
        if pl.starts_with("var(") {
            found_var = Some(parse_var_value(part));
            continue;
        }
        // Skip url(...) and gradient functions.
        if pl.starts_with("url(")
            || pl.starts_with("linear-gradient(")
            || pl.starts_with("radial-gradient(")
            || pl.starts_with("conic-gradient(")
            || pl.starts_with("repeating-")
        {
            continue;
        }
        // Skip layout/repeat keywords.
        if matches!(
            pl.as_str(),
            "no-repeat"
                | "repeat"
                | "repeat-x"
                | "repeat-y"
                | "center"
                | "left"
                | "right"
                | "top"
                | "bottom"
                | "cover"
                | "contain"
                | "fixed"
                | "scroll"
                | "local"
                | "border-box"
                | "padding-box"
                | "content-box"
        ) {
            continue;
        }
        // Skip if it looks like a size (e.g., 100%, 50px, 0).
        if pl.ends_with('%')
            || pl.ends_with("px")
            || pl.ends_with("em")
            || pl.ends_with("rem")
            || pl.ends_with("vw")
            || pl.ends_with("vh")
        {
            continue;
        }
        // Try parsing as a color.
        if pl == "transparent" {
            found_color = Some(0x00000000);
            continue;
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
    if let Some(c) = found_color {
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: CssValue::Color(c),
            important: false,
        });
    } else if let Some(var_val) = found_var {
        v.push(Declaration {
            property: Property::BackgroundColor,
            value: var_val,
            important: false,
        });
    }
    v
}

/// Split a `background` shorthand value into tokens, respecting parentheses.
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

/// Expand `border-top/right/bottom/left: <width> <style> <color>` per-side shorthand.
fn expand_border_side_shorthand(
    value_str: &str,
    width_prop: Property,
    style_prop: Property,
    color_prop: Property,
) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let raw_parts: Vec<&str> = value_str.split_whitespace().collect();
    let parts = reassemble_var_parts(&raw_parts);
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(
            lower.as_str(),
            "solid"
                | "dashed"
                | "dotted"
                | "double"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
                | "hidden"
        ) {
            decls.push(Declaration {
                property: style_prop.clone(),
                value: CssValue::Keyword(lower),
                important: false,
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: style_prop.clone(),
                value: CssValue::None,
                important: false,
            });
            decls.push(Declaration {
                property: width_prop.clone(),
                value: CssValue::Length(0, Unit::Px),
                important: false,
            });
        } else if lower.starts_with("var(") {
            decls.push(Declaration {
                property: color_prop.clone(),
                value: parse_var_value(part),
                important: false,
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: color_prop.clone(),
                value: CssValue::Color(c),
                important: false,
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: color_prop.clone(),
                value: CssValue::Color(c),
                important: false,
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: width_prop.clone(),
                value: dim,
                important: false,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: width_prop.clone(),
                value: CssValue::Keyword(lower),
                important: false,
            });
        }
    }
    decls
}

/// Expand `border-radius: <tl> [<tr>] [<br>] [<bl>]` shorthand.
fn expand_border_radius_shorthand(value_str: &str) -> Vec<Declaration> {
    // Ignore elliptical syntax (/) for simplicity — only use the first set.
    let s = if let Some(pos) = value_str.find('/') {
        &value_str[..pos]
    } else {
        value_str
    };
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let (tl, tr, br, bl) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    let mut v = Vec::with_capacity(4);
    v.push(Declaration {
        property: Property::BorderTopLeftRadius,
        value: parse_value(&Property::BorderTopLeftRadius, tl),
        important: false,
    });
    v.push(Declaration {
        property: Property::BorderTopRightRadius,
        value: parse_value(&Property::BorderTopRightRadius, tr),
        important: false,
    });
    v.push(Declaration {
        property: Property::BorderBottomRightRadius,
        value: parse_value(&Property::BorderBottomRightRadius, br),
        important: false,
    });
    v.push(Declaration {
        property: Property::BorderBottomLeftRadius,
        value: parse_value(&Property::BorderBottomLeftRadius, bl),
        important: false,
    });
    v
}

/// Expand `outline: <width> <style> <color>` shorthand.
fn expand_outline_shorthand(value_str: &str) -> Vec<Declaration> {
    let mut decls = Vec::new();
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    for part in &parts {
        let lower = to_ascii_lower(part);
        if matches!(
            lower.as_str(),
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
        ) {
            decls.push(Declaration {
                property: Property::OutlineStyle,
                value: CssValue::Keyword(lower),
                important: false,
            });
        } else if lower == "none" {
            decls.push(Declaration {
                property: Property::OutlineStyle,
                value: CssValue::None,
                important: false,
            });
        } else if let Some(c) = try_parse_color(part) {
            decls.push(Declaration {
                property: Property::OutlineColor,
                value: CssValue::Color(c),
                important: false,
            });
        } else if let Some(c) = named_color(&lower) {
            decls.push(Declaration {
                property: Property::OutlineColor,
                value: CssValue::Color(c),
                important: false,
            });
        } else if let Some(dim) = try_parse_dimension(part) {
            decls.push(Declaration {
                property: Property::OutlineWidth,
                value: dim,
                important: false,
            });
        } else if matches!(lower.as_str(), "thin" | "medium" | "thick") {
            decls.push(Declaration {
                property: Property::OutlineWidth,
                value: CssValue::Keyword(lower),
                important: false,
            });
        }
    }
    decls
}

/// Expand `text-decoration: <line> [<style>] [<color>]` shorthand (CSS3).
/// We keep it simple: extract underline/line-through/none and store as keyword.
fn expand_text_decoration_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();
    // Extract the line value (underline, line-through, overline, none)
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
    decls.push(Declaration {
        property: Property::TextDecoration,
        value: CssValue::Keyword(String::from(line_kw)),
        important: false,
    });
    decls
}
