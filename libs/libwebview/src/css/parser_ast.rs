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
                value: parse_value_ast(value),
                important,
            });
        }

        if i < bytes.len() && bytes[i] == b';' {
            i += 1;
        }
    }

    decls
}

fn parse_value_ast(input: &str) -> CssValueAst {
    CssValueAst {
        raw: String::from(input.trim()),
        components: parse_value_components_ast(input.trim()),
    }
}

fn parse_value_components_ast(input: &str) -> Vec<CssValueComponentAst> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b',' => {
                out.push(CssValueComponentAst::Comma);
                i += 1;
            }
            b'/' => {
                out.push(CssValueComponentAst::Slash);
                i += 1;
            }
            b'#' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                out.push(CssValueComponentAst::Hash(String::from(&input[start..i])));
            }
            b'"' | b'\'' => {
                let value = read_quoted_string_at(input, &mut i);
                out.push(CssValueComponentAst::String(value));
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                if let Some((component, next_i)) = parse_numeric_component_ast(input, i) {
                    out.push(component);
                    i = next_i;
                } else {
                    out.push(CssValueComponentAst::Delim(bytes[i] as char));
                    i += 1;
                }
            }
            _ if is_ident_start(bytes[i]) || bytes[i] == b'\\' => {
                let start = i;
                let ident = read_ident_at(input, &mut i);
                if i < bytes.len() && bytes[i] == b'(' {
                    let args = parse_function_arguments_ast(input, &mut i);
                    out.push(CssValueComponentAst::Function { name: ident, args });
                } else if !ident.is_empty() {
                    out.push(CssValueComponentAst::Ident(String::from(&input[start..i])));
                }
            }
            other => {
                out.push(CssValueComponentAst::Delim(other as char));
                i += 1;
            }
        }
    }

    out
}

fn parse_numeric_component_ast(input: &str, start: usize) -> Option<(CssValueComponentAst, usize)> {
    let bytes = input.as_bytes();
    let mut i = start;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        i += 1;
    }
    let number_start = start;
    let mut has_digit = false;
    let mut seen_dot = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {
                has_digit = true;
                i += 1;
            }
            b'.' if !seen_dot => {
                seen_dot = true;
                i += 1;
            }
            _ => break,
        }
    }
    if !has_digit {
        return None;
    }

    let mut j = i;
    while j < bytes.len() && (is_ident_start(bytes[j]) || bytes[j].is_ascii_digit() || bytes[j] == b'%') {
        j += 1;
    }

    if j > i {
        Some((CssValueComponentAst::Dimension(String::from(&input[number_start..j])), j))
    } else {
        Some((CssValueComponentAst::Number(String::from(&input[number_start..i])), i))
    }
}

fn parse_function_arguments_ast(input: &str, i: &mut usize) -> Vec<CssValueAst> {
    let bytes = input.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'(' {
        return Vec::new();
    }
    *i += 1;
    let arg_start = *i;
    let mut start = arg_start;
    let mut depth = 0i32;
    let mut string_quote = 0u8;
    let mut args = Vec::new();

    while *i < bytes.len() {
        let ch = bytes[*i];
        if string_quote != 0 {
            if ch == b'\\' {
                *i = (*i + 2).min(bytes.len());
                continue;
            }
            *i += 1;
            if ch == string_quote {
                string_quote = 0;
            }
            continue;
        }

        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                *i += 1;
            }
            b'(' => {
                depth += 1;
                *i += 1;
            }
            b')' if depth == 0 => {
                let arg = input[start..*i].trim();
                if !arg.is_empty() {
                    args.push(parse_value_ast(arg));
                }
                *i += 1;
                break;
            }
            b')' => {
                depth -= 1;
                *i += 1;
            }
            b',' if depth == 0 => {
                let arg = input[start..*i].trim();
                if !arg.is_empty() {
                    args.push(parse_value_ast(arg));
                }
                *i += 1;
                start = *i;
            }
            _ => *i += 1,
        }
    }

    args
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
    let mut parts: Vec<(Option<CssCombinatorAst>, CssSimpleSelectorAst)> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;
    let mut string_quote = 0u8;
    let mut pending_space = false;
    let mut pending_combinator = None;

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
                if let Some(simple) = parse_simple_selector_ast(raw) {
                    let combinator = if parts.is_empty() {
                        None
                    } else if let Some(comb) = pending_combinator.take() {
                        Some(comb)
                    } else if pending_space {
                        Some(CssCombinatorAst::Descendant)
                    } else {
                        None
                    };
                    parts.push((combinator, simple));
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
                pending_combinator = Some(comb);
            }
            b' ' | b'\t' | b'\n' | b'\r' if paren_depth == 0 && bracket_depth == 0 => {
                let raw = input[start..i].trim();
                if let Some(simple) = parse_simple_selector_ast(raw) {
                    let combinator = if parts.is_empty() {
                        None
                    } else if let Some(comb) = pending_combinator.take() {
                        Some(comb)
                    } else {
                        Some(CssCombinatorAst::Descendant)
                    };
                    parts.push((combinator, simple));
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
    if let Some(simple) = parse_simple_selector_ast(raw) {
        let combinator = if parts.is_empty() {
            None
        } else if let Some(comb) = pending_combinator.take() {
            Some(comb)
        } else if pending_space {
            Some(CssCombinatorAst::Descendant)
        } else {
            None
        };

        if parts.is_empty() {
            parts.push((None, simple));
        } else {
            parts.push((combinator, simple));
        }
    }

    let mut iter = parts.into_iter();
    let (_, first) = iter.next()?;
    let mut rest = Vec::new();
    for (comb, raw) in iter {
        rest.push((comb.unwrap_or(CssCombinatorAst::Descendant), raw));
    }
    Some(CssSelectorAst { first, rest })
}

fn parse_simple_selector_ast(input: &str) -> Option<CssSimpleSelectorAst> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut explicit_universal = false;
    let mut tag_name = None;
    let mut id = None;
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudo_classes = Vec::new();
    let mut pseudo_element = None;

    if bytes[i] == b'*' {
        explicit_universal = true;
        i += 1;
    } else if is_ident_start(bytes[i]) || bytes[i] == b'\\' {
        tag_name = Some(read_ident_at(input, &mut i));
    }

    while i < bytes.len() {
        match bytes[i] {
            b'#' => {
                i += 1;
                let ident = read_ident_at(input, &mut i);
                if !ident.is_empty() {
                    id = Some(ident);
                }
            }
            b'.' => {
                i += 1;
                let ident = read_ident_at(input, &mut i);
                if !ident.is_empty() {
                    classes.push(ident);
                }
            }
            b'[' => {
                if let Some(attr) = parse_attr_selector_ast(input, &mut i) {
                    attrs.push(attr);
                }
            }
            b':' if i + 1 < bytes.len() && bytes[i + 1] == b':' => {
                i += 2;
                let name = read_ident_at(input, &mut i);
                let lower = name.to_ascii_lowercase();
                pseudo_element = Some(match lower.as_str() {
                    "before" => CssPseudoElementAst::Before,
                    "after" => CssPseudoElementAst::After,
                    _ => {
                        if i < bytes.len() && bytes[i] == b'(' {
                            skip_balanced_parens(input, &mut i);
                        }
                        CssPseudoElementAst::Unknown
                    }
                });
            }
            b':' => {
                i += 1;
                let name = read_ident_at(input, &mut i);
                let lower = name.to_ascii_lowercase();
                if lower == "before" {
                    pseudo_element = Some(CssPseudoElementAst::Before);
                } else if lower == "after" {
                    pseudo_element = Some(CssPseudoElementAst::After);
                } else if let Some(pseudo) = parse_pseudo_class_ast_from_name(&lower, input, &mut i) {
                    pseudo_classes.push(pseudo);
                }
            }
            _ => break,
        }
    }

    let selector = CssSimpleSelectorAst {
        explicit_universal,
        tag_name,
        id,
        classes,
        attrs,
        pseudo_classes,
        pseudo_element,
    };

    if simple_selector_has_components(&selector) {
        Some(selector)
    } else {
        None
    }
}

fn parse_attr_selector_ast(input: &str, i: &mut usize) -> Option<CssAttrSelectorAst> {
    let bytes = input.as_bytes();
    *i += 1;
    skip_inline_selector_whitespace(input, i);
    let name = read_ident_at(input, i);
    if name.is_empty() {
        while *i < bytes.len() && bytes[*i] != b']' {
            *i += 1;
        }
        if *i < bytes.len() && bytes[*i] == b']' {
            *i += 1;
        }
        return None;
    }

    skip_inline_selector_whitespace(input, i);
    if *i < bytes.len() && bytes[*i] == b']' {
        *i += 1;
        return Some(CssAttrSelectorAst {
            name,
            op: CssAttrOpAst::Exists,
            value: None,
        });
    }

    let op = if starts_with_at(bytes, *i, b"~=") {
        *i += 2;
        CssAttrOpAst::Contains
    } else if starts_with_at(bytes, *i, b"^=") {
        *i += 2;
        CssAttrOpAst::Prefix
    } else if starts_with_at(bytes, *i, b"$=") {
        *i += 2;
        CssAttrOpAst::Suffix
    } else if starts_with_at(bytes, *i, b"*=") {
        *i += 2;
        CssAttrOpAst::Substring
    } else if starts_with_at(bytes, *i, b"|=") {
        *i += 2;
        CssAttrOpAst::DashMatch
    } else if *i < bytes.len() && bytes[*i] == b'=' {
        *i += 1;
        CssAttrOpAst::Exact
    } else {
        while *i < bytes.len() && bytes[*i] != b']' {
            *i += 1;
        }
        if *i < bytes.len() && bytes[*i] == b']' {
            *i += 1;
        }
        return None;
    };

    skip_inline_selector_whitespace(input, i);
    let value = if *i < bytes.len() && (bytes[*i] == b'"' || bytes[*i] == b'\'') {
        read_quoted_string_at(input, i)
    } else {
        read_ident_at(input, i)
    };

    skip_inline_selector_whitespace(input, i);
    if *i < bytes.len() && bytes[*i] == b']' {
        *i += 1;
    }

    Some(CssAttrSelectorAst {
        name,
        op,
        value: Some(value),
    })
}

fn parse_pseudo_class_ast_from_name(
    lower: &str,
    input: &str,
    i: &mut usize,
) -> Option<CssPseudoClassAst> {
    match lower {
        "hover" => Some(CssPseudoClassAst::Hover),
        "active" => Some(CssPseudoClassAst::Active),
        "focus" => Some(CssPseudoClassAst::Focus),
        "visited" => Some(CssPseudoClassAst::Visited),
        "first-child" => Some(CssPseudoClassAst::FirstChild),
        "last-child" => Some(CssPseudoClassAst::LastChild),
        "first-of-type" => Some(CssPseudoClassAst::FirstOfType),
        "last-of-type" => Some(CssPseudoClassAst::LastOfType),
        "empty" => Some(CssPseudoClassAst::Empty),
        "checked" => Some(CssPseudoClassAst::Checked),
        "disabled" => Some(CssPseudoClassAst::Disabled),
        "enabled" => Some(CssPseudoClassAst::Enabled),
        "root" => Some(CssPseudoClassAst::Root),
        "focus-visible" => Some(CssPseudoClassAst::FocusVisible),
        "focus-within" => Some(CssPseudoClassAst::FocusWithin),
        "placeholder-shown" => Some(CssPseudoClassAst::PlaceholderShown),
        "required" => Some(CssPseudoClassAst::Required),
        "optional" => Some(CssPseudoClassAst::Optional),
        "read-only" => Some(CssPseudoClassAst::ReadOnly),
        "read-write" => Some(CssPseudoClassAst::ReadWrite),
        "valid" => Some(CssPseudoClassAst::Valid),
        "invalid" => Some(CssPseudoClassAst::Invalid),
        "in-range" => Some(CssPseudoClassAst::InRange),
        "out-of-range" => Some(CssPseudoClassAst::OutOfRange),
        "default" => Some(CssPseudoClassAst::Default),
        "indeterminate" => Some(CssPseudoClassAst::Indeterminate),
        "nth-child" => Some(CssPseudoClassAst::NthChild(parse_pseudo_nth_arg(input, i, 1))),
        "nth-last-child" => Some(CssPseudoClassAst::NthLastChild(parse_pseudo_nth_arg(input, i, 1))),
        "not" => parse_pseudo_selector_list_arg(input, i).map(CssPseudoClassAst::Not),
        "is" | "matches" | "-webkit-any" | "-moz-any" => {
            parse_pseudo_selector_list_arg(input, i).map(CssPseudoClassAst::Is)
        }
        "where" => parse_pseudo_selector_list_arg(input, i).map(CssPseudoClassAst::Where),
        "has" => parse_pseudo_has_arg(input, i).map(|selector| CssPseudoClassAst::Has(Box::new(selector))),
        _ => {
            if *i < input.len() && input.as_bytes()[*i] == b'(' {
                skip_balanced_parens(input, i);
            }
            None
        }
    }
}

fn parse_pseudo_selector_list_arg(input: &str, i: &mut usize) -> Option<Vec<CssSimpleSelectorAst>> {
    if *i >= input.len() || input.as_bytes()[*i] != b'(' {
        return None;
    }
    let start = *i + 1;
    skip_balanced_parens(input, i);
    let end = i.saturating_sub(1);
    let list = parse_simple_selector_list_ast(&input[start..end]);
    if list.is_empty() { None } else { Some(list) }
}

fn parse_pseudo_has_arg(input: &str, i: &mut usize) -> Option<CssSimpleSelectorAst> {
    if *i >= input.len() || input.as_bytes()[*i] != b'(' {
        return None;
    }
    let start = *i + 1;
    skip_balanced_parens(input, i);
    let end = i.saturating_sub(1);
    parse_simple_selector_ast(input[start..end].trim())
}

fn parse_simple_selector_list_ast(input: &str) -> Vec<CssSimpleSelectorAst> {
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
                if let Some(selector) = parse_simple_selector_ast(input[start..i].trim()) {
                    out.push(selector);
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }

    if start < bytes.len() {
        if let Some(selector) = parse_simple_selector_ast(input[start..].trim()) {
            out.push(selector);
        }
    }

    out
}

fn parse_pseudo_nth_arg(input: &str, i: &mut usize, fallback: i32) -> i32 {
    if *i >= input.len() || input.as_bytes()[*i] != b'(' {
        return fallback;
    }
    let start = *i + 1;
    skip_balanced_parens(input, i);
    let end = i.saturating_sub(1);
    match input[start..end].trim() {
        "odd" => 1,
        "even" => 2,
        raw => parse_int(raw).unwrap_or(fallback),
    }
}

fn skip_inline_selector_whitespace(input: &str, i: &mut usize) {
    let bytes = input.as_bytes();
    while *i < bytes.len() {
        match bytes[*i] {
            b' ' | b'\t' | b'\n' | b'\r' => *i += 1,
            b'/' if starts_with_at(bytes, *i, b"/*") => {
                *i += 2;
                while *i + 1 < bytes.len() {
                    if bytes[*i] == b'*' && bytes[*i + 1] == b'/' {
                        *i += 2;
                        break;
                    }
                    *i += 1;
                }
            }
            _ => break,
        }
    }
}

fn read_ident_at(input: &str, i: &mut usize) -> String {
    let bytes = input.as_bytes();
    let mut result = String::new();
    while *i < bytes.len() {
        let ch = bytes[*i];
        if ch == b'\\' && *i + 1 < bytes.len() {
            *i += 1;
            let escaped = bytes[*i];
            if escaped.is_ascii_hexdigit() {
                let hex_start = *i;
                let mut count = 0usize;
                while *i < bytes.len() && bytes[*i].is_ascii_hexdigit() && count < 6 {
                    *i += 1;
                    count += 1;
                }
                if *i < bytes.len() && bytes[*i] == b' ' {
                    *i += 1;
                }
                if let Ok(s) = core::str::from_utf8(&bytes[hex_start..hex_start + count]) {
                    if let Ok(cp) = u32::from_str_radix(s, 16) {
                        if let Some(c) = char::from_u32(cp) {
                            result.push(c);
                            continue;
                        }
                    }
                }
            } else {
                result.push(escaped as char);
                *i += 1;
            }
        } else if ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_' {
            result.push(ch as char);
            *i += 1;
        } else {
            break;
        }
    }
    result
}

fn read_quoted_string_at(input: &str, i: &mut usize) -> String {
    let bytes = input.as_bytes();
    if *i >= bytes.len() {
        return String::new();
    }
    let quote = bytes[*i];
    *i += 1;
    let start = *i;
    while *i < bytes.len() {
        if bytes[*i] == b'\\' {
            *i = (*i + 2).min(bytes.len());
            continue;
        }
        if bytes[*i] == quote {
            let value = String::from(&input[start..*i]);
            *i += 1;
            return value;
        }
        *i += 1;
    }
    String::from(&input[start..bytes.len()])
}

fn skip_balanced_parens(input: &str, i: &mut usize) {
    let bytes = input.as_bytes();
    if *i >= bytes.len() || bytes[*i] != b'(' {
        return;
    }
    let mut depth = 0u32;
    let mut string_quote = 0u8;
    while *i < bytes.len() {
        let ch = bytes[*i];
        if string_quote != 0 {
            if ch == b'\\' {
                *i = (*i + 2).min(bytes.len());
                continue;
            }
            *i += 1;
            if ch == string_quote {
                string_quote = 0;
            }
            continue;
        }
        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                *i += 1;
            }
            b'(' => {
                depth += 1;
                *i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                *i += 1;
                if depth == 0 {
                    break;
                }
            }
            _ => *i += 1,
        }
    }
}

fn starts_with_at(bytes: &[u8], index: usize, prefix: &[u8]) -> bool {
    index + prefix.len() <= bytes.len() && &bytes[index..index + prefix.len()] == prefix
}

fn is_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_' || ch == b'-'
}

fn simple_selector_has_components(selector: &CssSimpleSelectorAst) -> bool {
    selector.explicit_universal
        || selector.tag_name.is_some()
        || selector.id.is_some()
        || !selector.classes.is_empty()
        || !selector.attrs.is_empty()
        || !selector.pseudo_classes.is_empty()
        || selector.pseudo_element.is_some()
}
