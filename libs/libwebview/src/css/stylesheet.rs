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
    let mut p = Parser::new(&css_text);
    let mut rules = Vec::new();
    let mut media_rules = Vec::new();
    let mut keyframes = Vec::new();
    let mut imports = Vec::new();
    let mut font_faces = Vec::new();
    let mut layer_order = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    let mut anon_layer_counter: u32 = 0;

    loop {
        p.skip_whitespace();
        if p.eof() {
            break;
        }

        if p.peek() == b'}' {
            p.pos += 1;
            if !layer_stack.is_empty() {
                layer_stack.pop();
            }
            continue;
        }

        // At-rules
        if p.peek() == b'@' {
            p.pos += 1;
            let keyword = p.read_ident();
            let kw_lower = keyword.to_ascii_lowercase();

            if kw_lower == "import" {
                // Parse @import url("...") or @import "..."
                p.skip_whitespace();
                let url = if p.starts_with(b"url(") {
                    p.pos += 4;
                    let q = if p.peek() == b'"' || p.peek() == b'\'' {
                        p.advance()
                    } else {
                        0
                    };
                    let start = p.pos;
                    while !p.eof() && p.peek() != b')' && (q == 0 || p.peek() != q) {
                        p.pos += 1;
                    }
                    let url = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
                    if q != 0 && p.peek() == q {
                        p.pos += 1;
                    }
                    if p.peek() == b')' {
                        p.pos += 1;
                    }
                    url
                } else if p.peek() == b'"' || p.peek() == b'\'' {
                    let q = p.advance();
                    let start = p.pos;
                    while !p.eof() && p.peek() != q {
                        p.pos += 1;
                    }
                    let url = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
                    if p.peek() == q {
                        p.pos += 1;
                    }
                    url
                } else {
                    String::new()
                };
                // Skip to semicolon
                while !p.eof() && p.peek() != b';' {
                    p.pos += 1;
                }
                if p.peek() == b';' {
                    p.pos += 1;
                }
                if !url.is_empty() {
                    imports.push(url);
                }
                continue;
            }

            if kw_lower == "font-face" {
                // Parse @font-face { font-family: ...; src: url(...); ... }
                p.skip_whitespace();
                if p.peek() == b'{' {
                    p.pos += 1;
                    let mut family = String::new();
                    let mut src_url = String::new();
                    let mut weight = 400u32;
                    let mut italic = false;
                    let mut display = FontDisplay::Auto;
                    // Parse declarations until '}'
                    while !p.eof() && p.peek() != b'}' {
                        p.skip_whitespace();
                        if p.peek() == b'}' {
                            break;
                        }
                        let prop_name = p.read_ident();
                        p.skip_whitespace();
                        if p.peek() == b':' {
                            p.pos += 1;
                        }
                        p.skip_whitespace();
                        // Read value until ';' or '}'
                        let val_start = p.pos;
                        while !p.eof() && p.peek() != b';' && p.peek() != b'}' {
                            p.pos += 1;
                        }
                        let val = String::from_utf8_lossy(&p.input[val_start..p.pos]).into_owned();
                        if p.peek() == b';' {
                            p.pos += 1;
                        }
                        let prop_lower = prop_name.to_ascii_lowercase();
                        match prop_lower.as_str() {
                            "font-family" => {
                                family = val.trim().trim_matches('"').trim_matches('\'').into();
                            }
                            "src" => {
                                // Parse comma-separated url() entries with optional format() hints.
                                // Prefer non-WOFF2 sources (TTF, OTF, WOFF) since WOFF2 (Brotli)
                                // is not yet supported.  Fall back to WOFF2 if nothing else.
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
                                    // Check for format('woff2') hint after the url()
                                    let rest = &after[url_end..];
                                    let is_woff2 = rest.contains("format('woff2')")
                                        || rest.contains("format(\"woff2\")")
                                        || url.ends_with(".woff2");
                                    if best_url.is_empty() || (best_is_woff2 && !is_woff2) {
                                        best_url = String::from(url);
                                        best_is_woff2 = is_woff2;
                                    }
                                    // Advance past this url() entry
                                    let consumed = url_start + 4 + url_end;
                                    if consumed >= search.len() {
                                        break;
                                    }
                                    search = &search[consumed..];
                                    // Skip to next comma-separated entry
                                    if let Some(comma) = search.find(',') {
                                        search = &search[comma + 1..];
                                    } else {
                                        break;
                                    }
                                }
                                src_url = best_url;
                            }
                            "font-weight" => {
                                let v = val.trim();
                                weight = match v {
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
                            "font-style" => {
                                italic = val.trim() == "italic";
                            }
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
                    if p.peek() == b'}' {
                        p.pos += 1;
                    }
                    if !family.is_empty() && !src_url.is_empty() {
                        font_faces.push(FontFaceRule {
                            family,
                            src_url,
                            weight,
                            italic,
                            display,
                        });
                    }
                }
                continue;
            }

            if kw_lower == "media" {
                // Parse @media query and inner rules.
                if let Some(mr) = parse_media_rule(
                    &mut p,
                    layer_stack.last().map(|s| s.as_str()),
                    &mut layer_order,
                    &mut anon_layer_counter,
                ) {
                    media_rules.push(mr);
                }
                continue;
            }

            if kw_lower == "keyframes" || kw_lower == "-webkit-keyframes" {
                if let Some(kf) = parse_keyframes(&mut p) {
                    keyframes.push(kf);
                }
                continue;
            }

            // @supports — evaluate the condition and include rules if supported.
            if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(
                    &mut p,
                    layer_stack.last().map(|s| s.as_str()),
                    &mut layer_order,
                    &mut anon_layer_counter,
                ) {
                    // @supports rules whose condition evaluates to true have their
                    // inner rules and media rules merged into the main lists.
                    for rule in sr.rules {
                        rules.push(rule);
                    }
                    for mr in sr.media_rules {
                        media_rules.push(mr);
                    }
                }
                continue;
            }

            if kw_lower == "container" {
                let container_query = parse_container_query_prelude(&mut p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    parse_container_block(
                        &mut p,
                        layer_stack.last().map(|s| s.as_str()),
                        &mut layer_order,
                        &mut anon_layer_counter,
                        container_query,
                        &mut rules,
                        &mut media_rules,
                    );
                }
                continue;
            }

            if kw_lower == "layer" {
                p.skip_whitespace();
                let name_start = p.pos;
                while !p.eof() && p.peek() != b'{' && p.peek() != b';' {
                    p.pos += 1;
                }
                let name_text = core::str::from_utf8(&p.input[name_start..p.pos])
                    .unwrap_or("")
                    .trim();
                if p.peek() == b';' {
                    register_layer_statement(name_text, layer_stack.last().map(|s| s.as_str()), &mut layer_order);
                    p.pos += 1;
                } else if p.peek() == b'{' {
                    let full_name = resolve_layer_block_name(
                        name_text,
                        layer_stack.last().map(|s| s.as_str()),
                        &mut layer_order,
                        &mut anon_layer_counter,
                    );
                    p.pos += 1;
                    layer_stack.push(full_name);
                }
                continue;
            }

            // Skip other at-rules.
            loop {
                p.skip_whitespace();
                if p.eof() {
                    break;
                }
                if p.peek() == b'{' {
                    p.skip_block();
                    break;
                }
                if p.peek() == b';' {
                    p.pos += 1;
                    break;
                }
                p.pos += 1;
            }
            continue;
        }

        // Skip stray closing braces
        if p.peek() == b'}' {
            p.pos += 1;
            continue;
        }

        // Safety: stop parsing if we hit limits
        if rules.len() >= MAX_CSS_RULES {
            crate::debug_surf!("[css] RULE LIMIT REACHED: {} rules — stopping", rules.len());
            break;
        }

        // Parse rule: selectors { declarations }
        if let Some(rule) = parse_rule(&mut p, layer_stack.last().map(|s| s.as_str())) {
            rules.push(rule);
        }
    }

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
