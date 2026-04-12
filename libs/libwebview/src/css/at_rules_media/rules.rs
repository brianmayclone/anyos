fn parse_media_rule(
    p: &mut Parser,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<MediaRule> {
    p.skip_whitespace();
    let query_start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let query_text = core::str::from_utf8(&p.input[query_start..p.pos]).unwrap_or("");
    let query = parse_media_query(query_text);
    if p.eof() {
        return None;
    }
    p.pos += 1;
    let mut inner_rules = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    if let Some(layer) = current_layer {
        layer_stack.push(String::from(layer));
    }
    let base_layer_depth = layer_stack.len();
    loop {
        p.skip_whitespace();
        if p.eof() {
            break;
        }
        if p.peek() == b'}' {
            p.pos += 1;
            if layer_stack.len() > base_layer_depth {
                layer_stack.pop();
                continue;
            }
            break;
        }
        if p.peek() == b'@' {
            p.pos += 1;
            let kw = p.read_ident();
            let kw_lower = {
                let mut buf = [0u8; 32];
                let len = kw.len().min(32);
                for (i, &b) in kw.as_bytes()[..len].iter().enumerate() {
                    buf[i] = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
                }
                String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
            };
            if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    for rule in sr.rules {
                        inner_rules.push(rule);
                    }
                }
            } else if kw_lower == "container" {
                let container_query = parse_container_query_prelude(p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    let mut nested_media_rules = Vec::new();
                    parse_container_block(
                        p,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                        container_query,
                        &mut inner_rules,
                        &mut nested_media_rules,
                    );
                    for nested in nested_media_rules {
                        inner_rules.extend(nested.rules);
                    }
                }
            } else if kw_lower == "layer" {
                p.skip_whitespace();
                let name_start = p.pos;
                while !p.eof() && p.peek() != b'{' && p.peek() != b';' {
                    p.pos += 1;
                }
                let name_text = core::str::from_utf8(&p.input[name_start..p.pos]).unwrap_or("").trim();
                if p.peek() == b';' {
                    register_layer_statement(
                        name_text,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                    );
                    p.pos += 1;
                } else if p.peek() == b'{' {
                    let full_name = resolve_layer_block_name(
                        name_text,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                    );
                    p.pos += 1;
                    layer_stack.push(full_name);
                }
            } else {
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
            }
            continue;
        }
        if let Some(rule) = parse_rule(p, layer_stack.last().map(|s| s.as_str())) {
            inner_rules.push(rule);
        }
    }

    Some(MediaRule { query, rules: inner_rules })
}
