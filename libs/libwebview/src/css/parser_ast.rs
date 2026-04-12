fn parse_stylesheet_ast(input: &str) -> CssStylesheetAst {
    let tokens = lex_css(input);
    let mut cursor = 0usize;
    CssStylesheetAst {
        items: parse_rule_list_ast(input, &tokens, &mut cursor, false),
    }
}

fn parse_rule_list_ast(
    input: &str,
    tokens: &[CssToken],
    cursor: &mut usize,
    stop_at_close_brace: bool,
) -> Vec<CssSyntaxNode> {
    let mut items = Vec::new();

    while *cursor < tokens.len() {
        match tokens[*cursor].kind {
            CssTokenKind::Whitespace | CssTokenKind::Comment => {
                *cursor += 1;
            }
            CssTokenKind::CloseBrace if stop_at_close_brace => {
                break;
            }
            CssTokenKind::CloseBrace => {
                *cursor += 1;
            }
            CssTokenKind::AtKeyword => {
                if let Some(node) = parse_at_rule_ast(input, tokens, cursor) {
                    items.push(CssSyntaxNode::AtRule(node));
                }
            }
            _ => {
                if let Some(node) = parse_qualified_rule_ast(input, tokens, cursor) {
                    items.push(CssSyntaxNode::QualifiedRule(node));
                }
            }
        }
    }

    items
}

fn parse_at_rule_ast(
    input: &str,
    tokens: &[CssToken],
    cursor: &mut usize,
) -> Option<CssAtRuleNode> {
    if *cursor >= tokens.len() || tokens[*cursor].kind != CssTokenKind::AtKeyword {
        return None;
    }

    let name = input[tokens[*cursor].start + 1..tokens[*cursor].end].trim().to_ascii_lowercase();
    *cursor += 1;

    let prelude_start = skip_ignorable_tokens(tokens, *cursor);
    let mut prelude_end = prelude_start;
    let mut paren_depth = 0i32;

    while *cursor < tokens.len() {
        match tokens[*cursor].kind {
            CssTokenKind::OpenParen => {
                paren_depth += 1;
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
            CssTokenKind::CloseParen => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
            CssTokenKind::Semicolon if paren_depth == 0 => {
                let prelude = slice_trimmed(input, prelude_start, prelude_end);
                *cursor += 1;
                return Some(CssAtRuleNode {
                    name,
                    prelude,
                    block: None,
                });
            }
            CssTokenKind::OpenBrace if paren_depth == 0 => {
                let prelude = slice_trimmed(input, prelude_start, prelude_end);
                let block = parse_block_node_ast(input, tokens, cursor)?;
                return Some(CssAtRuleNode {
                    name,
                    prelude,
                    block: Some(block),
                });
            }
            _ => {
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
        }
    }

    Some(CssAtRuleNode {
        name,
        prelude: slice_trimmed(input, prelude_start, prelude_end),
        block: None,
    })
}

fn parse_qualified_rule_ast(
    input: &str,
    tokens: &[CssToken],
    cursor: &mut usize,
) -> Option<CssQualifiedRuleNode> {
    let prelude_start = skip_ignorable_tokens(tokens, *cursor);
    let mut prelude_end = prelude_start;
    let mut paren_depth = 0i32;

    while *cursor < tokens.len() {
        match tokens[*cursor].kind {
            CssTokenKind::OpenParen => {
                paren_depth += 1;
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
            CssTokenKind::CloseParen => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
            CssTokenKind::OpenBrace if paren_depth == 0 => {
                let prelude = slice_trimmed(input, prelude_start, prelude_end);
                let block = parse_block_node_ast(input, tokens, cursor)?;
                return Some(CssQualifiedRuleNode { prelude, block });
            }
            CssTokenKind::Semicolon if paren_depth == 0 => {
                *cursor += 1;
                return None;
            }
            CssTokenKind::CloseBrace if paren_depth == 0 => return None,
            _ => {
                prelude_end = tokens[*cursor].end;
                *cursor += 1;
            }
        }
    }

    None
}

fn parse_block_node_ast(
    input: &str,
    tokens: &[CssToken],
    cursor: &mut usize,
) -> Option<CssBlockNode> {
    if *cursor >= tokens.len() || tokens[*cursor].kind != CssTokenKind::OpenBrace {
        return None;
    }

    let open_end = tokens[*cursor].end;
    *cursor += 1;
    let inner_start = open_end;
    let items = parse_rule_list_ast(input, tokens, cursor, true);
    let close_start = if *cursor < tokens.len() && tokens[*cursor].kind == CssTokenKind::CloseBrace {
        let start = tokens[*cursor].start;
        *cursor += 1;
        start
    } else {
        input.len()
    };

    Some(CssBlockNode {
        source: slice_trimmed(input, inner_start, close_start),
        items,
    })
}

fn skip_ignorable_tokens(tokens: &[CssToken], mut cursor: usize) -> usize {
    while cursor < tokens.len() {
        match tokens[cursor].kind {
            CssTokenKind::Whitespace | CssTokenKind::Comment => cursor += 1,
            _ => break,
        }
    }
    if cursor < tokens.len() {
        tokens[cursor].start
    } else {
        0
    }
}

fn slice_trimmed(input: &str, start: usize, end: usize) -> String {
    if start >= end || start >= input.len() {
        return String::new();
    }
    String::from(input[start..end.min(input.len())].trim())
}

fn parse_declaration_list_ast(input: &str) -> Vec<CssDeclarationAst> {
    let mut decls = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }

        let name_start = i;
        let mut colon = None;
        let mut paren_depth = 0i32;
        let mut string_quote = 0u8;
        while i < bytes.len() {
            let ch = bytes[i];
            if string_quote != 0 {
                if ch == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
                if ch == string_quote {
                    string_quote = 0;
                }
                continue;
            }
            match ch {
                b'"' | b'\'' => {
                    string_quote = ch;
                    i += 1;
                }
                b'(' => {
                    paren_depth += 1;
                    i += 1;
                }
                b')' => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    i += 1;
                }
                b':' if paren_depth == 0 => {
                    colon = Some(i);
                    i += 1;
                    break;
                }
                b';' | b'{' | b'}' if paren_depth == 0 => {
                    break;
                }
                _ => i += 1,
            }
        }

        let Some(colon_pos) = colon else {
            while i < bytes.len() && bytes[i] != b';' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b';' {
                i += 1;
            }
            continue;
        };

        let name = input[name_start..colon_pos].trim();
        let value_start = i;
        paren_depth = 0;
        string_quote = 0;
        while i < bytes.len() {
            let ch = bytes[i];
            if string_quote != 0 {
                if ch == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
                if ch == string_quote {
                    string_quote = 0;
                }
                continue;
            }
            match ch {
                b'"' | b'\'' => {
                    string_quote = ch;
                    i += 1;
                }
                b'(' => {
                    paren_depth += 1;
                    i += 1;
                }
                b')' => {
                    if paren_depth > 0 {
                        paren_depth -= 1;
                    }
                    i += 1;
                }
                b';' if paren_depth == 0 => break,
                b'{' | b'}' if paren_depth == 0 => break,
                _ => i += 1,
            }
        }

        let raw_value = input[value_start..i].trim();
        let (value, important) = strip_important(raw_value);
        if !name.is_empty() && !value.is_empty() {
            decls.push(CssDeclarationAst {
                name: String::from(name),
                value: String::from(value),
                important,
            });
        }

        if i < bytes.len() && bytes[i] == b';' {
            i += 1;
        }
    }

    decls
}

fn parse_selector_list_ast(input: &str) -> Vec<CssSelectorAst> {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut string_quote = 0u8;

    while i < bytes.len() {
        let ch = bytes[i];
        if string_quote != 0 {
            if ch == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            i += 1;
            if ch == string_quote {
                string_quote = 0;
            }
            continue;
        }
        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                i += 1;
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                i += 1;
            }
            b'[' => {
                bracket_depth += 1;
                i += 1;
            }
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
                i += 1;
            }
            b',' if paren_depth == 0 && bracket_depth == 0 => {
                if let Some(sel) = parse_selector_ast(input[start..i].trim()) {
                    out.push(sel);
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }

    if start < bytes.len() {
        if let Some(sel) = parse_selector_ast(input[start..].trim()) {
            out.push(sel);
        }
    }

    out
}

fn parse_selector_ast(input: &str) -> Option<CssSelectorAst> {
    if input.is_empty() {
        return None;
    }
    let bytes = input.as_bytes();
    let mut parts: Vec<(Option<CssCombinatorAst>, String)> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut string_quote = 0u8;
    let mut pending_space = false;

    while i < bytes.len() {
        let ch = bytes[i];
        if string_quote != 0 {
            if ch == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            i += 1;
            if ch == string_quote {
                string_quote = 0;
            }
            continue;
        }

        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                i += 1;
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                i += 1;
            }
            b'[' => {
                bracket_depth += 1;
                i += 1;
            }
            b']' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
                i += 1;
            }
            b'>' | b'+' | b'~' if paren_depth == 0 && bracket_depth == 0 => {
                let raw = input[start..i].trim();
                if !raw.is_empty() {
                    let combinator = if parts.is_empty() {
                        None
                    } else if pending_space {
                        Some(CssCombinatorAst::Descendant)
                    } else {
                        None
                    };
                    parts.push((combinator, String::from(raw)));
                }
                let comb = match ch {
                    b'>' => CssCombinatorAst::Child,
                    b'+' => CssCombinatorAst::AdjacentSibling,
                    _ => CssCombinatorAst::GeneralSibling,
                };
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
                {
                    i += 1;
                }
                start = i;
                pending_space = false;
                if !parts.is_empty() {
                    parts.push((Some(comb), String::new()));
                }
            }
            b' ' | b'\t' | b'\n' | b'\r' if paren_depth == 0 && bracket_depth == 0 => {
                let raw = input[start..i].trim();
                if !raw.is_empty() {
                    let combinator = if parts.is_empty() { None } else { Some(CssCombinatorAst::Descendant) };
                    parts.push((combinator, String::from(raw)));
                }
                i += 1;
                while i < bytes.len()
                    && (bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\n' || bytes[i] == b'\r')
                {
                    i += 1;
                }
                start = i;
                pending_space = true;
            }
            _ => i += 1,
        }
    }

    let raw = input[start..].trim();
    if !raw.is_empty() {
        let combinator = if parts.is_empty() {
            None
        } else if let Some((Some(comb), last)) = parts.last() {
            if last.is_empty() { Some(comb.clone()) } else { Some(CssCombinatorAst::Descendant) }
        } else if pending_space {
            Some(CssCombinatorAst::Descendant)
        } else {
            None
        };

        if let Some((_, last)) = parts.last_mut() {
            if last.is_empty() {
                *last = String::from(raw);
            } else {
                parts.push((combinator, String::from(raw)));
            }
        } else {
            parts.push((None, String::from(raw)));
        }
    }

    let mut iter = parts.into_iter();
    let (_, first) = iter.next()?;
    let mut rest = Vec::new();
    for (comb, raw) in iter {
        if raw.is_empty() {
            continue;
        }
        rest.push((comb.unwrap_or(CssCombinatorAst::Descendant), raw));
    }
    Some(CssSelectorAst { first, rest })
}
