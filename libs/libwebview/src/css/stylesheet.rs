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
    let mut selector_parser = Parser::new(&node.prelude);
    let selectors = parse_selector_list(&mut selector_parser);
    if selectors.is_empty() {
        return None;
    }
    let mut decl_parser = Parser::new(&node.block.source);
    let declarations = parse_declarations(&mut decl_parser);
    Some(Rule {
        selectors,
        declarations,
        layer_name: current_layer.map(String::from),
        layer_index: None,
        container_query: None,
    })
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
    let mut p = Parser::new(block);
    let mut family = String::new();
    let mut src_url = String::new();
    let mut weight = 400u32;
    let mut italic = false;
    let mut display = FontDisplay::Auto;

    while !p.eof() {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'}' {
            break;
        }
        let prop_name = p.read_ident();
        if prop_name.is_empty() {
            p.pos += 1;
            continue;
        }
        p.skip_whitespace();
        if p.peek() == b':' {
            p.pos += 1;
        }
        p.skip_whitespace();
        let val_start = p.pos;
        while !p.eof() && p.peek() != b';' && p.peek() != b'}' {
            p.pos += 1;
        }
        let val = String::from_utf8_lossy(&p.input[val_start..p.pos]).into_owned();
        if p.peek() == b';' {
            p.pos += 1;
        }
        match prop_name.to_ascii_lowercase().as_str() {
            "font-family" => {
                family = val.trim().trim_matches('"').trim_matches('\'').into();
            }
            "src" => {
                let v = val.trim();
                let mut best_url = String::new();
                let mut best_is_woff2 = true;
                let mut search = v;
                while let Some(url_start) = search.find("url(") {
                    let after = &search[url_start + 4..];
                    let url_end = after.find(')').unwrap_or(after.len());
                    let url = after[..url_end]
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'');
                    let rest = &after[url_end..];
                    let is_woff2 = rest.contains("format('woff2')")
                        || rest.contains("format(\"woff2\")")
                        || url.ends_with(".woff2");
                    if best_url.is_empty() || (best_is_woff2 && !is_woff2) {
                        best_url = String::from(url);
                        best_is_woff2 = is_woff2;
                    }
                    let consumed = url_start + 4 + url_end;
                    if consumed >= search.len() {
                        break;
                    }
                    search = &search[consumed..];
                    if let Some(comma) = search.find(',') {
                        search = &search[comma + 1..];
                    } else {
                        break;
                    }
                }
                src_url = best_url;
            }
            "font-weight" => {
                weight = match val.trim() {
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
            "font-style" => italic = val.trim() == "italic",
            "font-display" => {
                display = match val.trim() {
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
