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
                if let Some(mr) = lower_media_at_rule(
                    node,
                    block,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    media_rules.push(mr);
                }
            }
        }
        "supports" => {
            if let Some(block) = &node.block {
                if let Some(sr) = lower_supports_at_rule(
                    node,
                    block,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
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
