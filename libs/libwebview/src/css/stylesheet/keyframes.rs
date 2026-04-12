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
        }
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
            }
            break;
        }

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
            p.pos += 1;
        }

        p.skip_whitespace();
        if p.eof() || p.peek() != b'{' {
            while !p.eof() && p.peek() != b'}' {
                p.pos += 1;
            }
            continue;
        }

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
    p.pos += 1;
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
