fn normalize_stylesheet_input(css: &str) -> String {
    let mut text = css.trim();
    if let Some(stripped) = text.strip_prefix("<!--") {
        text = stripped.trim_start();
    }
    if let Some(stripped) = text.strip_suffix("-->") {
        text = stripped.trim_end();
    }
    if let Some(stripped) = text.strip_prefix("<![CDATA[") {
        text = stripped.trim_start();
    }
    if let Some(stripped) = text.strip_suffix("]]>") {
        text = stripped.trim_end();
    }
    String::from(text)
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    crate::debug_surf!("[css] parse_stylesheet: {} bytes", css.len());
    let css_text = normalize_stylesheet_input(css);
    let ast = parse_stylesheet_ast(&css_text);
    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_order = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    let mut anon_layer_counter: u32 = 0;

    lower_ast_items(
        &ast.items,
        &mut layer_stack,
        &mut layer_order,
        &mut anon_layer_counter,
        &mut rules,
        &mut media_rules,
        &mut keyframes,
        &mut imports,
        &mut font_faces,
    );

    crate::debug_surf!(
        "[css] parse_stylesheet done: {} rules, {} @media, {} @keyframes, {} imports",
        rules.len(),
        media_rules.len(),
        keyframes.len(),
        imports.len()
    );
    Stylesheet {
        rules,
        layer_order,
        media_rules,
        keyframes,
        imports,
        font_faces,
    }
}

fn lower_ast_items(
    items: &[CssSyntaxNode],
    layer_stack: &mut Vec<String>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
    rules: &mut Vec<Rule>,
    media_rules: &mut Vec<MediaRule>,
    keyframes: &mut Vec<KeyframeSet>,
    imports: &mut Vec<String>,
    font_faces: &mut Vec<FontFaceRule>,
) {
    for item in items {
        match item {
            CssSyntaxNode::QualifiedRule(node) => {
                if rules.len() >= MAX_CSS_RULES {
                    crate::debug_surf!("[css] RULE LIMIT REACHED: {} rules — stopping", rules.len());
                    break;
                }
                if let Some(rule) = lower_qualified_rule_ast(node, layer_stack.last().map(|s| s.as_str())) {
                    rules.push(rule);
                }
            }
            CssSyntaxNode::AtRule(node) => {
                lower_at_rule_ast(
                    node,
                    layer_stack,
                    layer_order,
                    anon_layer_counter,
                    rules,
                    media_rules,
                    keyframes,
                    imports,
                    font_faces,
                );
            }
        }
    }
}

fn lower_at_rule_ast(
    node: &CssAtRuleNode,
    layer_stack: &mut Vec<String>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
    rules: &mut Vec<Rule>,
    media_rules: &mut Vec<MediaRule>,
    keyframes: &mut Vec<KeyframeSet>,
    imports: &mut Vec<String>,
    font_faces: &mut Vec<FontFaceRule>,
) {
    match node.name.as_str() {
        "import" => {
            if let Some(url) = parse_import_prelude(&node.prelude) {
                imports.push(url);
            }
        }
        "font-face" => {
            if let Some(block) = &node.block {
                if let Some(rule) = parse_font_face_block(&block.source) {
                    font_faces.push(rule);
                }
            }
        }
        "media" => {
            if let Some(block) = &node.block {
                if let Some(mr) = lower_media_at_rule(node, block, layer_stack.last().map(|s| s.as_str()), layer_order, anon_layer_counter) {
                    media_rules.push(mr);
                }
            }
        }
        "supports" => {
            if let Some(block) = &node.block {
                if let Some(sr) = lower_supports_at_rule(node, block, layer_stack.last().map(|s| s.as_str()), layer_order, anon_layer_counter) {
                    rules.extend(sr.rules);
                    media_rules.extend(sr.media_rules);
                }
            }
        }
        "container" => {
            if let Some(block) = &node.block {
                let mut snippet = String::from(node.prelude.trim());
                snippet.push_str(" {");
                snippet.push_str(&block.source);
                snippet.push('}');
                let mut p = Parser::new(&snippet);
                let container_query = parse_container_query_prelude(&mut p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    parse_container_block(
                        &mut p,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                        container_query,
                        rules,
                        media_rules,
                    );
                }
            }
        }
        "layer" => {
            if let Some(block) = &node.block {
                let full_name = resolve_layer_block_name(
                    &node.prelude,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                );
                layer_stack.push(full_name);
                lower_ast_items(
                    &block.items,
                    layer_stack,
                    layer_order,
                    anon_layer_counter,
                    rules,
                    media_rules,
                    keyframes,
                    imports,
                    font_faces,
                );
                layer_stack.pop();
            } else {
                register_layer_statement(
                    &node.prelude,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                );
            }
        }
        "keyframes" | "-webkit-keyframes" => {
            if let Some(block) = &node.block {
                if let Some(kf) = lower_keyframes_at_rule(node, block) {
                    keyframes.push(kf);
                }
            }
        }
        _ => {}
    }
}

fn lower_qualified_rule_ast(node: &CssQualifiedRuleNode, current_layer: Option<&str>) -> Option<Rule> {
    let selectors = lower_selector_list_ast(&parse_selector_list_ast(&node.prelude));
    if selectors.is_empty() {
        return None;
    }
    let declarations = lower_declaration_list_ast(&parse_declaration_list_ast(&node.block.source));
    Some(Rule {
        selectors,
        declarations,
        layer_name: current_layer.map(String::from),
        layer_index: None,
        container_query: None,
    })
}

fn lower_selector_list_ast(ast: &[CssSelectorAst]) -> Vec<Selector> {
    let mut out = Vec::new();
    for sel in ast {
        if let Some(lowered) = lower_selector_ast(sel) {
            out.push(lowered);
        }
    }
    out
}

fn lower_selector_ast(ast: &CssSelectorAst) -> Option<Selector> {
    let first = lower_simple_selector_ast(&ast.first)?;
    let mut result = if ast.first.explicit_universal && is_universal(&first) {
        Selector::Universal
    } else {
        Selector::Simple(first)
    };
    for (comb, simple) in &ast.rest {
        let next = lower_simple_selector_ast(simple)?;
        result = match comb {
            CssCombinatorAst::Descendant => Selector::Descendant(Box::new(result), next),
            CssCombinatorAst::Child => Selector::Child(Box::new(result), next),
            CssCombinatorAst::AdjacentSibling => Selector::AdjacentSibling(Box::new(result), next),
            CssCombinatorAst::GeneralSibling => Selector::GeneralSibling(Box::new(result), next),
        };
    }
    Some(result)
}

fn lower_simple_selector_ast(ast: &CssSimpleSelectorAst) -> Option<SimpleSelector> {
    let mut tag = None;
    let mut custom_tag = None;
    if let Some(tag_name) = &ast.tag_name {
        let parsed = Tag::from_str(tag_name);
        if parsed == Tag::Unknown {
            custom_tag = Some(tag_name.to_ascii_lowercase());
        }
        tag = Some(parsed);
    }

    let mut attrs = Vec::new();
    for attr in &ast.attrs {
        attrs.push(AttrSelector {
            name: attr.name.clone(),
            op: match attr.op {
                CssAttrOpAst::Exists => AttrOp::Exists,
                CssAttrOpAst::Exact => AttrOp::Exact,
                CssAttrOpAst::Contains => AttrOp::Contains,
                CssAttrOpAst::Prefix => AttrOp::Prefix,
                CssAttrOpAst::Suffix => AttrOp::Suffix,
                CssAttrOpAst::Substring => AttrOp::Substring,
                CssAttrOpAst::DashMatch => AttrOp::DashMatch,
            },
            value: attr.value.clone(),
        });
    }

    let mut pseudo_classes = Vec::new();
    for pseudo in &ast.pseudo_classes {
        pseudo_classes.push(lower_pseudo_class_ast(pseudo)?);
    }

    let pseudo_element = ast.pseudo_element.map(|pseudo| match pseudo {
        CssPseudoElementAst::Before => PseudoElement::Before,
        CssPseudoElementAst::After => PseudoElement::After,
        CssPseudoElementAst::Unknown => PseudoElement::Unknown,
    });

    Some(SimpleSelector {
        tag,
        custom_tag,
        id: ast.id.clone(),
        classes: ast.classes.clone(),
        attrs,
        pseudo_classes,
        pseudo_element,
    })
}

fn lower_pseudo_class_ast(ast: &CssPseudoClassAst) -> Option<PseudoClass> {
    Some(match ast {
        CssPseudoClassAst::Hover => PseudoClass::Hover,
        CssPseudoClassAst::Active => PseudoClass::Active,
        CssPseudoClassAst::Focus => PseudoClass::Focus,
        CssPseudoClassAst::Visited => PseudoClass::Visited,
        CssPseudoClassAst::FirstChild => PseudoClass::FirstChild,
        CssPseudoClassAst::LastChild => PseudoClass::LastChild,
        CssPseudoClassAst::NthChild(n) => PseudoClass::NthChild(*n),
        CssPseudoClassAst::NthLastChild(n) => PseudoClass::NthLastChild(*n),
        CssPseudoClassAst::FirstOfType => PseudoClass::FirstOfType,
        CssPseudoClassAst::LastOfType => PseudoClass::LastOfType,
        CssPseudoClassAst::Not(selectors) => PseudoClass::Not(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Is(selectors) => PseudoClass::Is(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Where(selectors) => PseudoClass::Where(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Has(selector) => PseudoClass::Has(Box::new(lower_simple_selector_ast(selector)?)),
        CssPseudoClassAst::Empty => PseudoClass::Empty,
        CssPseudoClassAst::Checked => PseudoClass::Checked,
        CssPseudoClassAst::Disabled => PseudoClass::Disabled,
        CssPseudoClassAst::Enabled => PseudoClass::Enabled,
        CssPseudoClassAst::Root => PseudoClass::Root,
        CssPseudoClassAst::FocusVisible => PseudoClass::FocusVisible,
        CssPseudoClassAst::FocusWithin => PseudoClass::FocusWithin,
        CssPseudoClassAst::PlaceholderShown => PseudoClass::PlaceholderShown,
        CssPseudoClassAst::Required => PseudoClass::Required,
        CssPseudoClassAst::Optional => PseudoClass::Optional,
        CssPseudoClassAst::ReadOnly => PseudoClass::ReadOnly,
        CssPseudoClassAst::ReadWrite => PseudoClass::ReadWrite,
        CssPseudoClassAst::Valid => PseudoClass::Valid,
        CssPseudoClassAst::Invalid => PseudoClass::Invalid,
        CssPseudoClassAst::InRange => PseudoClass::InRange,
        CssPseudoClassAst::OutOfRange => PseudoClass::OutOfRange,
        CssPseudoClassAst::Default => PseudoClass::Default,
        CssPseudoClassAst::Indeterminate => PseudoClass::Indeterminate,
    })
}

fn lower_simple_selector_list_ast(ast: &[CssSimpleSelectorAst]) -> Option<Vec<SimpleSelector>> {
    let mut out = Vec::with_capacity(ast.len());
    for selector in ast {
        out.push(lower_simple_selector_ast(selector)?);
    }
    Some(out)
}

fn lower_media_at_rule(
    node: &CssAtRuleNode,
    block: &CssBlockNode,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<MediaRule> {
    let mut snippet = String::from(node.prelude.trim());
    snippet.push_str(" {");
    snippet.push_str(&block.source);
    snippet.push('}');
    let mut p = Parser::new(&snippet);
    parse_media_rule(&mut p, current_layer, layer_order, anon_layer_counter)
}

fn lower_supports_at_rule(
    node: &CssAtRuleNode,
    block: &CssBlockNode,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<SupportsResult> {
    let mut snippet = String::from(node.prelude.trim());
    snippet.push_str(" {");
    snippet.push_str(&block.source);
    snippet.push('}');
    let mut p = Parser::new(&snippet);
    parse_supports_rule(&mut p, current_layer, layer_order, anon_layer_counter)
}

fn lower_keyframes_at_rule(node: &CssAtRuleNode, block: &CssBlockNode) -> Option<KeyframeSet> {
    let mut snippet = String::from(node.prelude.trim());
    snippet.push_str(" {");
    snippet.push_str(&block.source);
    snippet.push('}');
    let mut p = Parser::new(&snippet);
    parse_keyframes(&mut p)
}

fn parse_import_prelude(prelude: &str) -> Option<String> {
    let s = prelude.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix("url(") {
        let url = rest
            .trim_end_matches(')')
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if !url.is_empty() {
            return Some(String::from(url));
        }
    }
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let url = s[1..s.len().saturating_sub(1)].trim();
        if !url.is_empty() {
            return Some(String::from(url));
        }
    }
    None
}

fn parse_font_face_block(block: &str) -> Option<FontFaceRule> {
    let mut family = String::new();
    let mut src_url = String::new();
    let mut weight = 400u32;
    let mut italic = false;
    let mut display = FontDisplay::Auto;

    for decl in parse_declaration_list_ast(block) {
        match decl.name.to_ascii_lowercase().as_str() {
            "font-family" => {
                family = decl
                    .value
                    .raw
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .into();
            }
            "src" => {
                let mut best_url = String::new();
                let mut best_is_woff2 = true;
                for component in &decl.value.components {
                    if let CssValueComponentAst::Function { name, args } = component {
                        if !name.eq_ignore_ascii_case("url") || args.is_empty() {
                            continue;
                        }
                        let url = args[0]
                            .raw
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'');
                        let is_woff2 = decl.value.raw.contains("format('woff2')")
                            || decl.value.raw.contains("format(\"woff2\")")
                            || url.ends_with(".woff2");
                        if best_url.is_empty() || (best_is_woff2 && !is_woff2) {
                            best_url = String::from(url);
                            best_is_woff2 = is_woff2;
                        }
                    }
                }
                src_url = best_url;
            }
            "font-weight" => {
                weight = match decl.value.raw.trim() {
                    "bold" | "700" => 700,
                    "normal" | "400" => 400,
                    "100" => 100,
                    "200" => 200,
                    "300" => 300,
                    "500" => 500,
                    "600" => 600,
                    "800" => 800,
                    "900" => 900,
                    _ => 400,
                };
            }
            "font-style" => italic = decl.value.raw.trim() == "italic",
            "font-display" => {
                display = match decl.value.raw.trim() {
                    "block" => FontDisplay::Block,
                    "swap" => FontDisplay::Swap,
                    "fallback" => FontDisplay::Fallback,
                    "optional" => FontDisplay::Optional,
                    _ => FontDisplay::Auto,
                };
            }
            _ => {}
        }
    }

    if family.is_empty() || src_url.is_empty() {
        None
    } else {
        Some(FontFaceRule {
            family,
            src_url,
            weight,
            italic,
            display,
        })
    }
}

fn lower_declaration_list_ast(ast: &[CssDeclarationAst]) -> Vec<Declaration> {
    let mut decls = Vec::new();
    for decl in ast {
        if decl.name.starts_with("--") {
            decls.push(Declaration {
                property: Property::CustomProperty(String::from(&decl.name)),
                value: lower_custom_property_value_ast(&decl.value),
                important: decl.important,
            });
            continue;
        }

        if decl.name.eq_ignore_ascii_case("font") {
            let mut expanded = expand_font_shorthand(&decl.value.raw);
            if decl.important {
                for d in &mut expanded {
                    d.important = true;
                }
            }
            decls.extend(expanded);
            continue;
        }

        let Some(property) = parse_property(&decl.name) else {
            continue;
        };

        if is_expandable_shorthand(&property) {
            let mut expanded = expand_shorthand(property, &decl.value.raw);
            if decl.important {
                for d in &mut expanded {
                    d.important = true;
                }
            }
            decls.extend(expanded);
        } else {
            let value = lower_property_value_ast(&property, &decl.value);
            decls.push(Declaration {
                property,
                value,
                important: decl.important,
            });
        }
    }
    decls
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
            CssValueComponentAst::Comma | CssValueComponentAst::Slash | CssValueComponentAst::Delim(_) => {}
        }
    }

    parse_value(property, &value.raw)
}

fn lower_custom_property_value_ast(value: &CssValueAst) -> CssValue {
    CssValue::Keyword(value.raw.clone())
}

/// Parse a @media rule: query { rules }.
fn parse_keyframes(p: &mut Parser) -> Option<KeyframeSet> {
    p.skip_whitespace();

    // Read animation name (may be quoted or an ident).
    let name = if p.peek() == b'"' || p.peek() == b'\'' {
        p.pos += 1; // skip opening quote
        let start = p.pos;
        let q = p.input[p.pos - 1];
        while p.pos < p.input.len() && p.input[p.pos] != q {
            p.pos += 1;
        }
        let name = core::str::from_utf8(&p.input[start..p.pos])
            .unwrap_or("")
            .to_ascii_lowercase();
        if !p.eof() {
            p.pos += 1;
        } // skip closing quote
        name
    } else {
        p.read_ident().to_ascii_lowercase()
    };

    if name.is_empty() {
        p.skip_block();
        return None;
    }

    p.skip_whitespace();
    if p.eof() || p.peek() != b'{' {
        return None;
    }
    p.pos += 1; // consume '{'

    let mut stops = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'}' {
            if !p.eof() {
                p.pos += 1;
            } // consume '}'
            break;
        }

        // Read keyframe selectors: `from`, `to`, `50%` separated by commas.
        let mut offsets: Vec<i32> = Vec::new();
        loop {
            p.skip_whitespace();
            let token_start = p.pos;
            while p.pos < p.input.len()
                && p.input[p.pos] != b','
                && p.input[p.pos] != b'{'
                && p.input[p.pos] != b'}'
            {
                p.pos += 1;
            }
            let token = core::str::from_utf8(&p.input[token_start..p.pos])
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !token.is_empty() {
                let offset = if token == "from" {
                    0
                } else if token == "to" {
                    100
                } else if let Some(pct_str) = token.strip_suffix('%') {
                    pct_str.trim().parse::<f32>().map(|v| v as i32).unwrap_or(0)
                } else {
                    0
                };
                offsets.push(offset);
            }
            p.skip_whitespace();
            if p.eof() || p.peek() != b',' {
                break;
            }
            p.pos += 1; // consume ','
        }

        p.skip_whitespace();
        if p.eof() || p.peek() != b'{' {
            while !p.eof() && p.peek() != b'}' {
                p.pos += 1;
            }
            continue;
        }
        // Parse the declarations block for this stop.
        let decls = parse_declarations_block(p);

        for offset in offsets {
            stops.push(KeyframeStop {
                offset,
                declarations: decls.clone(),
            });
        }
    }

    stops.sort_by_key(|s| s.offset);
    Some(KeyframeSet { name, stops })
}

/// Parse a `{ declaration; ... }` block and return the declarations.
/// Expects the opening `{` to be the next character; consumes through the matching `}`.
fn parse_declarations_block(p: &mut Parser) -> Vec<Declaration> {
    if p.eof() || p.peek() != b'{' {
        return Vec::new();
    }
    p.pos += 1; // consume '{'
    let start = p.pos;
    let mut depth = 1u32;
    while p.pos < p.input.len() {
        match p.input[p.pos] {
            b'{' => {
                depth += 1;
                p.pos += 1;
            }
            b'}' => {
                depth -= 1;
                p.pos += 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {
                p.pos += 1;
            }
        }
    }
    let block_text = core::str::from_utf8(&p.input[start..p.pos.saturating_sub(1)]).unwrap_or("");
    let mut inner = Parser::new(block_text);
    parse_declarations(&mut inner)
}
