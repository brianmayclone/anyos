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
