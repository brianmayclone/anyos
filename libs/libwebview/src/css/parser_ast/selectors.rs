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
                    && (bytes[i] == b' '
                        || bytes[i] == b'\t'
                        || bytes[i] == b'\n'
                        || bytes[i] == b'\r')
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
                    && (bytes[i] == b' '
                        || bytes[i] == b'\t'
                        || bytes[i] == b'\n'
                        || bytes[i] == b'\r')
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
                } else if let Some(pseudo) = parse_pseudo_class_ast_from_name(&lower, input, &mut i)
                {
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
        "nth-child" => Some(CssPseudoClassAst::NthChild(parse_pseudo_nth_arg(
            input, i, 1,
        ))),
        "nth-last-child" => Some(CssPseudoClassAst::NthLastChild(parse_pseudo_nth_arg(
            input, i, 1,
        ))),
        "not" => Some(
            parse_pseudo_selector_list_arg(input, i)
                .map(CssPseudoClassAst::Not)
                .unwrap_or(CssPseudoClassAst::Unsupported),
        ),
        "is" | "matches" | "-webkit-any" | "-moz-any" => Some(
            parse_pseudo_selector_list_arg(input, i)
                .map(CssPseudoClassAst::Is)
                .unwrap_or(CssPseudoClassAst::Unsupported),
        ),
        "where" => Some(
            parse_pseudo_selector_list_arg(input, i)
                .map(CssPseudoClassAst::Where)
                .unwrap_or(CssPseudoClassAst::Unsupported),
        ),
        "has" => Some(
            parse_pseudo_has_arg(input, i)
                .map(|selector| CssPseudoClassAst::Has(Box::new(selector)))
                .unwrap_or(CssPseudoClassAst::Unsupported),
        ),
        "-moz-focusring" => Some(CssPseudoClassAst::FocusVisible),
        _ => {
            if *i < input.len() && input.as_bytes()[*i] == b'(' {
                skip_balanced_parens(input, i);
            }
            Some(CssPseudoClassAst::Unsupported)
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
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
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

fn simple_selector_has_components(selector: &CssSimpleSelectorAst) -> bool {
    selector.explicit_universal
        || selector.tag_name.is_some()
        || selector.id.is_some()
        || !selector.classes.is_empty()
        || !selector.attrs.is_empty()
        || !selector.pseudo_classes.is_empty()
        || selector.pseudo_element.is_some()
}
