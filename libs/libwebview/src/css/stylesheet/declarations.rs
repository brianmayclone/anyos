fn lower_declaration_list_ast(ast: &[CssDeclarationAst]) -> Vec<Declaration> {
    let mut decls = Vec::new();
    for decl in ast {
        decls.extend(lower_declaration_ast(decl));
    }
    decls
}

fn lower_declaration_ast(decl: &CssDeclarationAst) -> Vec<Declaration> {
    if decl.name.starts_with("--") {
        let mut decls = Vec::new();
        decls.push(Declaration {
            property: Property::CustomProperty(String::from(&decl.name)),
            value: lower_custom_property_value_ast(&decl.value),
            important: decl.important,
        });
        return decls;
    }

    if decl.name.eq_ignore_ascii_case("font") {
        let mut expanded = expand_font_shorthand(&decl.value.raw);
        if decl.important {
            for d in &mut expanded {
                d.important = true;
            }
        }
        return expanded;
    }

    let Some(property) = parse_property(&decl.name) else {
        return Vec::new();
    };

    if is_expandable_shorthand(&property) {
        let mut expanded = expand_shorthand(property, &decl.value.raw);
        if decl.important {
            for d in &mut expanded {
                d.important = true;
            }
        }
        expanded
    } else {
        let value = lower_property_value_ast(&property, &decl.value);
        let mut decls = Vec::new();
        decls.push(Declaration {
            property,
            value,
            important: decl.important,
        });
        decls
    }
}

fn parse_property_value_ast(property: &Property, value_str: &str) -> CssValue {
    let value_ast = parse_value_ast(value_str);
    lower_property_value_ast(property, &value_ast)
}

fn lower_property_value_ast(property: &Property, value: &CssValueAst) -> CssValue {
    if value.components.len() == 1 {
        match &value.components[0] {
            CssValueComponentAst::Ident(ident) => {
                let lower = to_ascii_lower(ident.trim());
                match lower.as_str() {
                    "auto" => return CssValue::Auto,
                    "none" => return CssValue::None,
                    "inherit" => return CssValue::Inherit,
                    "currentcolor" => return CssValue::CurrentColor,
                    "transparent" => return CssValue::Color(0x00000000),
                    _ => {
                        if is_color_property(property) {
                            if let Some(color) = named_color(&lower) {
                                return CssValue::Color(color);
                            }
                        }
                    }
                }
            }
            CssValueComponentAst::Hash(hash) => {
                if let Some(color) = try_parse_color(hash) {
                    return CssValue::Color(color);
                }
            }
            CssValueComponentAst::Number(number) | CssValueComponentAst::Dimension(number) => {
                if let Some(value) = try_parse_dimension(number) {
                    return value;
                }
            }
            CssValueComponentAst::Function { name, args } => {
                let lower = to_ascii_lower(name);
                match lower.as_str() {
                    "var" => {
                        if let Some(var_name) = args.first() {
                            let fallback = args
                                .get(1)
                                .map(|fallback| Box::new(lower_property_value_ast(property, fallback)));
                            return CssValue::Var(var_name.raw.trim().into(), fallback);
                        }
                    }
                    "calc" => {
                        return parse_calc_value(&value.raw);
                    }
                    "min" => return parse_min_max_clamp_value(&value.raw, CssMathFunc::Min),
                    "max" => return parse_min_max_clamp_value(&value.raw, CssMathFunc::Max),
                    "clamp" => return parse_min_max_clamp_value(&value.raw, CssMathFunc::Clamp),
                    "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab"
                    | "oklch" | "color" | "color-mix" | "light-dark" => {
                        if let Some(color) = try_parse_color(&value.raw) {
                            return CssValue::Color(color);
                        }
                    }
                    "url" => {
                        return CssValue::Keyword(value.raw.clone());
                    }
                    _ => {}
                }
            }
            CssValueComponentAst::String(text) => {
                return CssValue::Keyword(text.clone());
            }
            CssValueComponentAst::Comma
            | CssValueComponentAst::Slash
            | CssValueComponentAst::Delim(_) => {}
        }
    }

    parse_value(property, &value.raw)
}

fn lower_custom_property_value_ast(value: &CssValueAst) -> CssValue {
    CssValue::Keyword(value.raw.clone())
}
