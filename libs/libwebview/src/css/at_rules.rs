fn register_layer_name(full_name: &str, layer_order: &mut Vec<String>) {
    if full_name.is_empty() {
        return;
    }
    if !layer_order.iter().any(|name| name == full_name) {
        layer_order.push(String::from(full_name));
    }
}

fn qualify_layer_name(name: &str, parent: Option<&str>) -> String {
    let name = name.trim();
    if name.is_empty() {
        return String::new();
    }
    if let Some(parent_name) = parent {
        let mut full = String::from(parent_name);
        full.push('.');
        full.push_str(name);
        full
    } else {
        String::from(name)
    }
}

fn register_layer_statement(name_text: &str, parent: Option<&str>, layer_order: &mut Vec<String>) {
    for raw_name in name_text.split(',') {
        let full_name = qualify_layer_name(raw_name.trim(), parent);
        register_layer_name(&full_name, layer_order);
    }
}

fn resolve_layer_block_name(
    name_text: &str,
    parent: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> String {
    let trimmed = name_text.trim();
    if trimmed.is_empty() {
        *anon_layer_counter += 1;
        let mut full = String::from("__anon_layer_");
        full.push_str(&anon_layer_counter.to_string());
        register_layer_name(&full, layer_order);
        return full;
    }

    let full_name = qualify_layer_name(trimmed, parent);
    register_layer_name(&full_name, layer_order);
    full_name
}

fn parse_container_query_prelude(p: &mut Parser) -> Option<ContainerQuery> {
    p.skip_whitespace();
    let start = p.pos;
    while !p.eof() && p.peek() != b'{' {
        p.pos += 1;
    }
    let text = core::str::from_utf8(&p.input[start..p.pos]).unwrap_or("").trim();
    parse_container_query_text(text)
}

fn parse_container_query_text(text: &str) -> Option<ContainerQuery> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first_paren = trimmed.find('(')?;
    let name_text = trimmed[..first_paren].trim();
    let name = if name_text.is_empty() {
        None
    } else {
        Some(String::from(name_text.split_whitespace().next().unwrap_or(name_text)))
    };
    let mut conditions = Vec::new();
    let bytes = trimmed.as_bytes();
    let mut i = first_paren;
    while i < bytes.len() {
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        let inner_start = i + 1;
        let mut depth = 1i32;
        i += 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            break;
        }
        let inner = trimmed[inner_start..i - 1].trim();
        if let Some(cond) = parse_container_condition(inner) {
            conditions.push(cond);
        }
    }
    if conditions.is_empty() {
        None
    } else {
        Some(ContainerQuery { name, conditions })
    }
}

fn parse_container_condition(text: &str) -> Option<ContainerCondition> {
    let lower = text.trim().to_ascii_lowercase();
    let (name, value) = lower.split_once(':')?;
    let px = parse_px_value(value.trim())?;
    match name.trim() {
        "min-width" => Some(ContainerCondition::MinWidth(px)),
        "max-width" => Some(ContainerCondition::MaxWidth(px)),
        "width" => Some(ContainerCondition::Width(px)),
        "min-height" => Some(ContainerCondition::MinHeight(px)),
        "max-height" => Some(ContainerCondition::MaxHeight(px)),
        "height" => Some(ContainerCondition::Height(px)),
        "min-inline-size" | "min-inline" => Some(ContainerCondition::MinInlineSize(px)),
        "max-inline-size" | "max-inline" => Some(ContainerCondition::MaxInlineSize(px)),
        "inline-size" | "inline" => Some(ContainerCondition::InlineSize(px)),
        "min-block-size" | "min-block" => Some(ContainerCondition::MinBlockSize(px)),
        "max-block-size" | "max-block" => Some(ContainerCondition::MaxBlockSize(px)),
        "block-size" | "block" => Some(ContainerCondition::BlockSize(px)),
        _ => None,
    }
}

fn apply_container_query_to_rule(rule: &mut Rule, query: &ContainerQuery) {
    match &mut rule.container_query {
        Some(existing) => {
            if existing.name.is_none() {
                existing.name = query.name.clone();
            }
            for cond in &query.conditions {
                existing.conditions.push(cond.clone());
            }
        }
        None => rule.container_query = Some(query.clone()),
    }
}

fn apply_container_query_to_media_rule(media_rule: &mut MediaRule, query: &ContainerQuery) {
    for rule in &mut media_rule.rules {
        apply_container_query_to_rule(rule, query);
    }
}

fn parse_container_block(
    p: &mut Parser,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
    container_query: Option<ContainerQuery>,
    out_rules: &mut Vec<Rule>,
    out_media_rules: &mut Vec<MediaRule>,
) {
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
            let kw_lower = kw.to_ascii_lowercase();
            if kw_lower == "media" {
                if let Some(mut mr) = parse_media_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    if let Some(ref query) = container_query {
                        apply_container_query_to_media_rule(&mut mr, query);
                    }
                    out_media_rules.push(mr);
                }
            } else if kw_lower == "supports" {
                if let Some(mut sr) = parse_supports_rule(
                    p,
                    layer_stack.last().map(|s| s.as_str()),
                    layer_order,
                    anon_layer_counter,
                ) {
                    if let Some(ref query) = container_query {
                        for rule in &mut sr.rules {
                            apply_container_query_to_rule(rule, query);
                        }
                        for media_rule in &mut sr.media_rules {
                            apply_container_query_to_media_rule(media_rule, query);
                        }
                    }
                    out_rules.extend(sr.rules);
                    out_media_rules.extend(sr.media_rules);
                }
            } else if kw_lower == "container" {
                let nested_query = parse_container_query_prelude(p);
                if p.peek() == b'{' {
                    p.pos += 1;
                    parse_container_block(
                        p,
                        layer_stack.last().map(|s| s.as_str()),
                        layer_order,
                        anon_layer_counter,
                        nested_query.or_else(|| container_query.clone()),
                        out_rules,
                        out_media_rules,
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
        if let Some(mut rule) = parse_rule(p, layer_stack.last().map(|s| s.as_str())) {
            if let Some(ref query) = container_query {
                apply_container_query_to_rule(&mut rule, query);
            }
            out_rules.push(rule);
        }
    }
}

fn parse_supports_rule(
    p: &mut Parser,
    current_layer: Option<&str>,
    layer_order: &mut Vec<String>,
    anon_layer_counter: &mut u32,
) -> Option<SupportsResult> {
    p.skip_whitespace();

    // Read everything until '{' as the supports condition.
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
    p.pos += 1; // consume '{'

    // Parse inner rules (including nested @media).
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
                // Skip other nested at-rules.
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

    // Evaluate the supports condition.
    if evaluate_supports_condition(condition) {
        Some(SupportsResult {
            rules: inner_rules,
            media_rules: inner_media,
        })
    } else {
        None // condition not supported — discard rules
    }
}

/// Evaluate a simple @supports condition.
/// Supports: `(property: value)`, `not (...)`, `(...) and (...)`, `(...) or (...)`.
fn evaluate_supports_condition(cond: &str) -> bool {
    evaluate_supports_depth(cond, 0)
}

fn evaluate_supports_depth(cond: &str, depth: u32) -> bool {
    if depth > 10 {
        return false;
    } // prevent infinite recursion
    let cond = cond.trim();
    if cond.is_empty() {
        return false;
    }

    // Strip exactly one matching pair of outer parens if present
    let cond = if cond.starts_with('(') && cond.ends_with(')') {
        // Verify the closing ) matches the opening (
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

    // Handle `not (...)`
    if cond.starts_with("not ") || cond.starts_with("not(") {
        let rest = cond[3..].trim();
        return !evaluate_supports_depth(rest, depth + 1);
    }

    // Handle `(...) and (...)` — split at top-level " and "
    if let Some(pos) = find_top_level(cond, " and ") {
        let left = &cond[..pos];
        let right = &cond[pos + 5..];
        return evaluate_supports_depth(left, depth + 1)
            && evaluate_supports_depth(right, depth + 1);
    }

    // Handle `(...) or (...)` — split at top-level " or "
    if let Some(pos) = find_top_level(cond, " or ") {
        let left = &cond[..pos];
        let right = &cond[pos + 4..];
        return evaluate_supports_depth(left, depth + 1)
            || evaluate_supports_depth(right, depth + 1);
    }

    // Simple `property: value` — check if property is known.
    let inner = cond.trim();
    if let Some(colon) = inner.find(':') {
        let prop_name = inner[..colon]
            .trim()
            .trim_start_matches('-')
            .trim_start_matches("webkit-")
            .trim_start_matches("moz-");
        return parse_property(inner[..colon].trim()).is_some();
    }

    // Unknown condition — be conservative, assume supported.
    true
}

/// Find the position of `needle` at the top level of parentheses (depth 0).
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

