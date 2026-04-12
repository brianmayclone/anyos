fn expand_flex_shorthand(value_str: &str) -> Vec<Declaration> {
    let lower = to_ascii_lower(value_str);
    let mut decls = Vec::new();
    match lower.as_str() {
        "none" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(0), important: false });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(0), important: false });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto, important: false });
            return decls;
        }
        "auto" => {
            decls.push(Declaration { property: Property::FlexGrow, value: CssValue::Number(100), important: false });
            decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
            decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Auto, important: false });
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
        value: parse_property_value_ast(&Property::FlexGrow, parts[0]),
        important: false,
    });
    if parts.len() == 1 {
        decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
        decls.push(Declaration { property: Property::FlexBasis, value: CssValue::Length(0, Unit::Px), important: false });
        return decls;
    }
    if parts.len() >= 2 {
        if let Some(dim) = try_parse_dimension(parts[1]) {
            if matches!(dim, CssValue::Length(_, _) | CssValue::Percentage(_)) {
                decls.push(Declaration { property: Property::FlexShrink, value: CssValue::Number(100), important: false });
                decls.push(Declaration { property: Property::FlexBasis, value: dim, important: false });
            } else {
                decls.push(Declaration { property: Property::FlexShrink, value: dim, important: false });
            }
        } else {
            decls.push(Declaration {
                property: Property::FlexShrink,
                value: parse_property_value_ast(&Property::FlexShrink, parts[1]),
                important: false,
            });
        }
    }
    if parts.len() >= 3 {
        decls.push(Declaration {
            property: Property::FlexBasis,
            value: parse_property_value_ast(&Property::FlexBasis, parts[2]),
            important: false,
        });
    }
    decls
}

fn expand_gap_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration {
        property: Property::RowGap,
        value: parse_property_value_ast(&Property::RowGap, parts[0]),
        important: false,
    });
    let col = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration {
        property: Property::ColumnGap,
        value: parse_property_value_ast(&Property::ColumnGap, col),
        important: false,
    });
    decls
}

fn expand_overflow_shorthand(value_str: &str) -> Vec<Declaration> {
    let parts: Vec<&str> = value_str.split_whitespace().collect();
    let mut decls = Vec::new();
    if parts.is_empty() {
        return decls;
    }
    decls.push(Declaration {
        property: Property::OverflowX,
        value: parse_property_value_ast(&Property::OverflowX, parts[0]),
        important: false,
    });
    let y = if parts.len() >= 2 { parts[1] } else { parts[0] };
    decls.push(Declaration {
        property: Property::OverflowY,
        value: parse_property_value_ast(&Property::OverflowY, y),
        important: false,
    });
    decls
}

fn expand_border_radius_shorthand(value_str: &str) -> Vec<Declaration> {
    let s = if let Some(pos) = value_str.find('/') { &value_str[..pos] } else { value_str };
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
    v.push(Declaration { property: Property::BorderTopLeftRadius, value: parse_property_value_ast(&Property::BorderTopLeftRadius, tl), important: false });
    v.push(Declaration { property: Property::BorderTopRightRadius, value: parse_property_value_ast(&Property::BorderTopRightRadius, tr), important: false });
    v.push(Declaration { property: Property::BorderBottomRightRadius, value: parse_property_value_ast(&Property::BorderBottomRightRadius, br), important: false });
    v.push(Declaration { property: Property::BorderBottomLeftRadius, value: parse_property_value_ast(&Property::BorderBottomLeftRadius, bl), important: false });
    v
}
