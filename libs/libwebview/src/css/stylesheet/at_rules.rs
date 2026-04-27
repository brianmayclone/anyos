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
                if let Some(cr) = lower_container_at_rule(
                    node,
                    block,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    rules.extend(cr.rules);
                    media_rules.extend(cr.media_rules);
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
    let query = parse_media_query(&node.prelude);
    let mut rules = Vec::new();
    let mut nested_media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_stack = Vec::new();
    if let Some(layer) = current_layer {
        layer_stack.push(String::from(layer));
    }

    lower_ast_items(
        &block.items,
        &mut layer_stack,
        layer_order,
        anon_layer_counter,
        &mut rules,
        &mut nested_media_rules,
        &mut keyframes,
        &mut imports,
        &mut font_faces,
    );

    Some(MediaRule { query, rules })
}

fn lower_supports_at_rule(
    node: &CssAtRuleNode,
    block: &CssBlockNode,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<SupportsResult> {
    if !evaluate_supports_condition(&node.prelude) {
        return None;
    }

    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_stack = Vec::new();
    if let Some(layer) = current_layer {
        layer_stack.push(String::from(layer));
    }

    lower_ast_items(
        &block.items,
        &mut layer_stack,
        layer_order,
        anon_layer_counter,
        &mut rules,
        &mut media_rules,
        &mut keyframes,
        &mut imports,
        &mut font_faces,
    );

    Some(SupportsResult { rules, media_rules })
}

fn lower_container_at_rule(
    node: &CssAtRuleNode,
    block: &CssBlockNode,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<SupportsResult> {
    let container_query = parse_container_query_text(&node.prelude);

    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_stack = Vec::new();
    if let Some(layer) = current_layer {
        layer_stack.push(String::from(layer));
    }

    lower_ast_items(
        &block.items,
        &mut layer_stack,
        layer_order,
        anon_layer_counter,
        &mut rules,
        &mut media_rules,
        &mut keyframes,
        &mut imports,
        &mut font_faces,
    );

    if let Some(query) = container_query {
        for rule in &mut rules {
            apply_container_query_to_rule(rule, &query);
        }
        for media_rule in &mut media_rules {
            apply_container_query_to_media_rule(media_rule, &query);
        }
    }

    Some(SupportsResult { rules, media_rules })
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

fn split_font_src_candidates(raw: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut start = 0usize;
    let mut depth = 0u32;
    let mut quote = None::<u8>;
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' => quote = Some(b),
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let part = raw[start..i].trim();
                if !part.is_empty() {
                    candidates.push(part);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let part = raw[start..].trim();
    if !part.is_empty() {
        candidates.push(part);
    }
    candidates
}

fn extract_font_src_url(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let mut i = start + 4;
    let bytes = raw.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let quote = if i < bytes.len() && (bytes[i] == b'\'' || bytes[i] == b'"') {
        let q = bytes[i];
        i += 1;
        Some(q)
    } else {
        None
    };
    let value_start = i;
    while i < bytes.len() {
        if let Some(q) = quote {
            if bytes[i] == q {
                break;
            }
        } else if bytes[i] == b')' || bytes[i].is_ascii_whitespace() {
            break;
        }
        i += 1;
    }
    let url = raw[value_start..i].trim();
    if url.is_empty() {
        None
    } else {
        Some(String::from(url))
    }
}

fn font_src_score(candidate: &str, url: &str) -> u8 {
    let lower = candidate.to_ascii_lowercase();
    let url_lower = url.to_ascii_lowercase();
    let has_format = lower.contains("format(");
    let is_woff2 = lower.contains("format('woff2')")
        || lower.contains("format(\"woff2\")")
        || lower.contains("format(woff2)")
        || url_lower.ends_with(".woff2");
    let is_truetype = lower.contains("format('truetype')")
        || lower.contains("format(\"truetype\")")
        || lower.contains("format(truetype)")
        || lower.contains("format('opentype')")
        || lower.contains("format(\"opentype\")")
        || lower.contains("format(opentype)")
        || url_lower.ends_with(".ttf")
        || url_lower.ends_with(".otf");
    let is_woff = lower.contains("format('woff')")
        || lower.contains("format(\"woff\")")
        || lower.contains("format(woff)")
        || url_lower.ends_with(".woff");

    if is_woff2 {
        4
    } else if is_truetype {
        3
    } else if !has_format && !is_woff {
        2
    } else if is_woff {
        1
    } else {
        0
    }
}

fn select_font_src_url(raw: &str) -> String {
    let mut best_url = String::new();
    let mut best_score = 0u8;
    for candidate in split_font_src_candidates(raw) {
        let Some(url) = extract_font_src_url(candidate) else {
            continue;
        };
        let score = font_src_score(candidate, &url);
        if best_url.is_empty() || score > best_score {
            best_score = score;
            best_url = url;
        }
    }
    best_url
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
                src_url = select_font_src_url(&decl.value.raw);
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
