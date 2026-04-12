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

fn expand_shorthand(property: Property, value_str: &str) -> Vec<Declaration> {
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
            let value = parse_property_value_ast(&property, value_str);
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
    let v_t = parse_property_value_ast(&top, t);
    v.push(Declaration { property: top, value: v_t, important: false });
    let v_r = parse_property_value_ast(&right, r);
    v.push(Declaration { property: right, value: v_r, important: false });
    let v_b = parse_property_value_ast(&bottom, b);
    v.push(Declaration { property: bottom, value: v_b, important: false });
    let v_l = parse_property_value_ast(&left, l);
    v.push(Declaration { property: left, value: v_l, important: false });
    v
}

fn reassemble_var_parts(parts: &[&str]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let lower = parts[i].to_ascii_lowercase();
        if lower.starts_with("var(") && !lower.ends_with(')') {
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
