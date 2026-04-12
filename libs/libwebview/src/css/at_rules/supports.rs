fn parse_supports_rule(
    p: &mut Parser,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<SupportsResult> {
    p.skip_whitespace();

    let cond_start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let condition = core::str::from_utf8(&p.input[cond_start..p.pos])
        .unwrap_or("")
        .trim();

    if p.eof() {
        return None;
    }
    p.pos += 1;

    let mut inner_rules = Vec::new();
    let mut inner_media = Vec::new();
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
            if kw_lower == "media" {
                if let Some(mr) = parse_media_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    inner_media.push(mr);
                }
            } else if kw_lower == "supports" {
                if let Some(sr) = parse_supports_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    for rule in sr.rules {
                        inner_rules.push(rule);
                    }
                    for mr in sr.media_rules {
                        inner_media.push(mr);
                    }
                }
            } else if kw_lower == "container" {
                let container_query = parse_container_query_prelude(p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    parse_container_block(
                        p,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                        container_query,
                        &mut inner_rules,
                        &mut inner_media,
                    );
                }
            } else if kw_lower == "layer" {
                p.skip_whitespace();
                let name_start = p.pos;
                while !p.eof() && p.peek() != b'{' && p.peek() != b';' {
                    p.pos += 1;
                }
                let name_text = core::str::from_utf8(&p.input[name_start..p.pos])
                    .unwrap_or("")
                    .trim();
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

    if evaluate_supports_condition(condition) {
        Some(SupportsResult {
            rules: inner_rules,
            media_rules: inner_media,
        })
    } else {
        None
    }
}

fn evaluate_supports_condition(cond: &str) -> bool {
    evaluate_supports_depth(cond, 0)
}

fn evaluate_supports_depth(cond: &str, depth: u32) -> bool {
    if depth > 10 {
        return false;
    }
    let cond = cond.trim();
    if cond.is_empty() {
        return false;
    }

    let cond = if cond.starts_with('(') && cond.ends_with(')') {
        let mut d = 0i32;
        let mut matches_outer = true;
        for (i, ch) in cond.chars().enumerate() {
            if ch == '(' {
                d += 1;
            } else if ch == ')' {
                d -= 1;
            }
            if d == 0 && i < cond.len() - 1 {
                matches_outer = false;
                break;
            }
        }
        if matches_outer {
            &cond[1..cond.len() - 1]
        } else {
            cond
        }
    } else {
        cond
    };

    if cond.starts_with("not ") || cond.starts_with("not(") {
        let rest = cond[3..].trim();
        return !evaluate_supports_depth(rest, depth + 1);
    }

    if let Some(pos) = find_top_level(cond, " and ") {
        let left = &cond[..pos];
        let right = &cond[pos + 5..];
        return evaluate_supports_depth(left, depth + 1)
            && evaluate_supports_depth(right, depth + 1);
    }

    if let Some(pos) = find_top_level(cond, " or ") {
        let left = &cond[..pos];
        let right = &cond[pos + 4..];
        return evaluate_supports_depth(left, depth + 1)
            || evaluate_supports_depth(right, depth + 1);
    }

    let inner = cond.trim();
    if let Some(colon) = inner.find(':') {
        return parse_property(inner[..colon].trim()).is_some();
    }

    true
}

fn find_top_level(s: &str, needle: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let needle_bytes = needle.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && i + needle_bytes.len() <= bytes.len() {
            if &bytes[i..i + needle_bytes.len()] == needle_bytes {
                return Some(i);
            }
        }
    }
    None
}
