fn parse_rule(p: &mut Parser, current_layer: Option<&str>) -> Option<Rule> {
    let selectors = parse_selector_list(p);
    if selectors.is_empty() {
        return Option::None;
    }

    p.skip_whitespace();
    if p.peek() != b'{' {
        // Malformed — skip to next brace or EOF
        while !p.eof() && p.peek() != b'{' && p.peek() != b'}' {
            p.pos += 1;
        }
        if p.peek() == b'{' {
            p.skip_block();
        }
        return Option::None;
    }
    p.pos += 1; // consume '{'

    let declarations = parse_declarations(p);

    // consume '}'
    p.skip_whitespace();
    if p.peek() == b'}' {
        p.pos += 1;
    }

    Some(Rule {
        selectors,
        declarations,
        layer_name: current_layer.map(String::from),
        layer_index: None,
        container_query: None,
    })
}

fn parse_selector_list(p: &mut Parser) -> Vec<Selector> {
    let mut selectors = Vec::new();

    loop {
        p.skip_whitespace();
        if p.eof() || p.peek() == b'{' {
            break;
        }

        let sel = parse_selector(p);
        selectors.push(sel);

        p.skip_whitespace();
        if p.peek() == b',' {
            p.pos += 1;
        } else {
            break;
        }
    }

    selectors
}

fn parse_selector(p: &mut Parser) -> Selector {
    p.skip_whitespace();

    let first = parse_simple_selector(p);
    let mut result = if is_universal(&first) {
        Selector::Universal
    } else {
        Selector::Simple(first)
    };

    loop {
        let had_space = skip_spaces_only(p);
        if p.eof() || p.peek() == b'{' || p.peek() == b',' {
            break;
        }
        // Check for explicit combinators: > + ~
        let combinator = if p.peek() == b'>' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'>')
        } else if p.peek() == b'+' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'+')
        } else if p.peek() == b'~' {
            p.pos += 1;
            skip_spaces_only(p);
            Some(b'~')
        } else if had_space {
            Some(b' ')
        } else {
            None
        };
        match combinator {
            Some(b'>') => {
                let next = parse_simple_selector(p);
                result = Selector::Child(Box::new(result), next);
            }
            Some(b'+') => {
                let next = parse_simple_selector(p);
                result = Selector::AdjacentSibling(Box::new(result), next);
            }
            Some(b'~') => {
                let next = parse_simple_selector(p);
                result = Selector::GeneralSibling(Box::new(result), next);
            }
            Some(b' ') => {
                let next = parse_simple_selector(p);
                result = Selector::Descendant(Box::new(result), next);
            }
            _ => break,
        }
    }

    result
}

/// Skip spaces/tabs only (not newlines treated as whitespace in selectors,
/// but we do skip them). Returns true if any whitespace was consumed.
fn skip_spaces_only(p: &mut Parser) -> bool {
    let start = p.pos;
    while !p.eof() {
        let ch = p.peek();
        if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
            p.pos += 1;
        } else if p.starts_with(b"/*") {
            p.skip_comment();
        } else {
            break;
        }
    }
    p.pos > start
}

fn is_universal(s: &SimpleSelector) -> bool {
    s.tag.is_none()
        && s.id.is_none()
        && s.classes.is_empty()
        && s.attrs.is_empty()
        && s.pseudo_classes.is_empty()
        && s.pseudo_element.is_none()
}

fn parse_simple_selector(p: &mut Parser) -> SimpleSelector {
    let mut tag = Option::None;
    let mut custom_tag: Option<String> = None;
    let mut id = Option::None;
    let mut classes = Vec::new();
    let mut attrs = Vec::new();
    let mut pseudo_classes = Vec::new();
    let mut pseudo_element = Option::None;

    if p.peek() == b'*' {
        p.pos += 1;
    } else if p.peek().is_ascii_alphabetic() {
        let name = p.read_ident();
        let parsed = Tag::from_str(&name);
        // For custom/unknown elements (e.g. "a-analytics"), store the raw name
        // so we can distinguish between different unknown element types.
        if parsed == Tag::Unknown {
            custom_tag = Some(name.to_ascii_lowercase());
        }
        tag = Some(parsed);
    }

    // Parse chained #id, .class, [attr], :pseudo, ::pseudo-element (no spaces between them)
    loop {
        if p.peek() == b'#' {
            p.pos += 1;
            id = Some(p.read_ident());
        } else if p.peek() == b'.' {
            p.pos += 1;
            classes.push(p.read_ident());
        } else if p.peek() == b'[' {
            if let Some(attr) = parse_attr_selector(p) {
                attrs.push(attr);
            }
        } else if p.starts_with(b"::") {
            p.pos += 2;
            let name = p.read_ident();
            let lower = to_ascii_lower(&name);
            match lower.as_str() {
                "before" => pseudo_element = Some(PseudoElement::Before),
                "after" => pseudo_element = Some(PseudoElement::After),
                _ => {
                    // Unknown pseudo-element — mark as never-matches
                    pseudo_element = Some(PseudoElement::Unknown);
                    if p.peek() == b'(' {
                        let mut depth: u32 = 1;
                        p.pos += 1;
                        while !p.eof() && depth > 0 {
                            if p.peek() == b'(' {
                                depth += 1;
                            }
                            if p.peek() == b')' {
                                depth -= 1;
                            }
                            p.pos += 1;
                        }
                    }
                }
            }
        } else if p.peek() == b':' {
            // Single colon — could also be legacy :before/:after syntax
            p.pos += 1;
            let name = p.read_ident();
            let lower = to_ascii_lower(&name);
            match lower.as_str() {
                "before" => pseudo_element = Some(PseudoElement::Before),
                "after" => pseudo_element = Some(PseudoElement::After),
                _ => {
                    // Re-parse as pseudo-class by feeding the already-read name
                    if let Some(pc) = parse_pseudo_class_from_name(&lower, p) {
                        pseudo_classes.push(pc);
                    }
                }
            }
        } else {
            break;
        }
    }

    SimpleSelector {
        tag,
        custom_tag,
        id,
        classes,
        attrs,
        pseudo_classes,
        pseudo_element,
    }
}

fn parse_attr_selector(p: &mut Parser) -> Option<AttrSelector> {
    p.pos += 1; // skip '['
    skip_spaces_only(p);
    let name = p.read_ident();
    if name.is_empty() {
        while !p.eof() && p.peek() != b']' {
            p.pos += 1;
        }
        if p.peek() == b']' {
            p.pos += 1;
        }
        return Option::None;
    }
    skip_spaces_only(p);
    if p.peek() == b']' {
        p.pos += 1;
        return Some(AttrSelector {
            name,
            op: AttrOp::Exists,
            value: Option::None,
        });
    }

    let op = if p.starts_with(b"~=") {
        p.pos += 2;
        AttrOp::Contains
    } else if p.starts_with(b"^=") {
        p.pos += 2;
        AttrOp::Prefix
    } else if p.starts_with(b"$=") {
        p.pos += 2;
        AttrOp::Suffix
    } else if p.starts_with(b"*=") {
        p.pos += 2;
        AttrOp::Substring
    } else if p.starts_with(b"|=") {
        p.pos += 2;
        AttrOp::DashMatch
    } else if p.peek() == b'=' {
        p.pos += 1;
        AttrOp::Exact
    } else {
        while !p.eof() && p.peek() != b']' {
            p.pos += 1;
        }
        if p.peek() == b']' {
            p.pos += 1;
        }
        return Option::None;
    };

    skip_spaces_only(p);
    let value = if p.peek() == b'"' || p.peek() == b'\'' {
        let quote = p.advance();
        let start = p.pos;
        while !p.eof() && p.peek() != quote {
            p.pos += 1;
        }
        let val = String::from_utf8_lossy(&p.input[start..p.pos]).into_owned();
        if p.peek() == quote {
            p.pos += 1;
        }
        val
    } else {
        p.read_ident()
    };

    skip_spaces_only(p);
    if p.peek() == b']' {
        p.pos += 1;
    }
    Some(AttrSelector {
        name,
        op,
        value: Some(value),
    })
}

fn parse_pseudo_class(p: &mut Parser) -> Option<PseudoClass> {
    let name = p.read_ident();
    let lower = to_ascii_lower(&name);
    parse_pseudo_class_from_name(&lower, p)
}

fn parse_pseudo_class_from_name(lower: &str, p: &mut Parser) -> Option<PseudoClass> {
    match lower {
        "hover" => Some(PseudoClass::Hover),
        "active" => Some(PseudoClass::Active),
        "focus" => Some(PseudoClass::Focus),
        "visited" => Some(PseudoClass::Visited),
        "first-child" => Some(PseudoClass::FirstChild),
        "last-child" => Some(PseudoClass::LastChild),
        "first-of-type" => Some(PseudoClass::FirstOfType),
        "last-of-type" => Some(PseudoClass::LastOfType),
        "empty" => Some(PseudoClass::Empty),
        "checked" => Some(PseudoClass::Checked),
        "disabled" => Some(PseudoClass::Disabled),
        "enabled" => Some(PseudoClass::Enabled),
        "root" => Some(PseudoClass::Root),
        "nth-child" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let n = parse_nth_arg(p);
                skip_spaces_only(p);
                if p.peek() == b')' {
                    p.pos += 1;
                }
                Some(PseudoClass::NthChild(n))
            } else {
                Some(PseudoClass::NthChild(1))
            }
        }
        "nth-last-child" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let n = parse_nth_arg(p);
                skip_spaces_only(p);
                if p.peek() == b')' {
                    p.pos += 1;
                }
                Some(PseudoClass::NthLastChild(n))
            } else {
                Some(PseudoClass::NthLastChild(1))
            }
        }
        "not" => {
            if p.peek() == b'(' {
                // Use parse_selector_list_in_parens so :not(.a, .b) works.
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Not(selectors))
            } else {
                Option::None
            }
        }
        "is" | "matches" | "-webkit-any" | "-moz-any" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Is(selectors))
            } else {
                Option::None
            }
        }
        "where" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Where(selectors))
            } else {
                Option::None
            }
        }
        "has" => {
            if p.peek() == b'(' {
                p.pos += 1;
                skip_spaces_only(p);
                let inner = parse_simple_selector(p);
                skip_spaces_only(p);
                if p.peek() == b')' {
                    p.pos += 1;
                }
                Some(PseudoClass::Has(Box::new(inner)))
            } else {
                Option::None
            }
        }
        "focus-visible" => Some(PseudoClass::FocusVisible),
        "focus-within" => Some(PseudoClass::FocusWithin),
        "placeholder-shown" => Some(PseudoClass::PlaceholderShown),
        "required" => Some(PseudoClass::Required),
        "optional" => Some(PseudoClass::Optional),
        "read-only" => Some(PseudoClass::ReadOnly),
        "read-write" => Some(PseudoClass::ReadWrite),
        "valid" => Some(PseudoClass::Valid),
        "invalid" => Some(PseudoClass::Invalid),
        "in-range" => Some(PseudoClass::InRange),
        "out-of-range" => Some(PseudoClass::OutOfRange),
        "default" => Some(PseudoClass::Default),
        "indeterminate" => Some(PseudoClass::Indeterminate),
        _ => {
            // Skip unknown pseudo-class arguments
            if p.peek() == b'(' {
                let mut depth: u32 = 1;
                p.pos += 1;
                while !p.eof() && depth > 0 {
                    match p.advance() {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                }
            }
            Option::None
        }
    }
}

/// Parse a comma-separated list of simple selectors inside parentheses: `(sel1, sel2, ...)`
fn parse_selector_list_in_parens(p: &mut Parser) -> Vec<SimpleSelector> {
    let mut selectors = Vec::new();
    if p.peek() != b'(' {
        return selectors;
    }
    p.pos += 1; // consume '('
    loop {
        skip_spaces_only(p);
        if p.eof() || p.peek() == b')' {
            if p.peek() == b')' {
                p.pos += 1;
            }
            break;
        }
        let before = p.pos;
        let sel = parse_simple_selector(p);
        selectors.push(sel);
        skip_spaces_only(p);
        if p.peek() == b',' {
            p.pos += 1;
        }
        // Safety: if parser didn't advance, skip one char to prevent infinite loop
        if p.pos == before {
            p.pos += 1;
        }
        // Safety: cap selector list size
        if selectors.len() > 1000 {
            // Skip to closing paren
            let mut depth = 1i32;
            while !p.eof() && depth > 0 {
                match p.input[p.pos] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                p.pos += 1;
            }
            break;
        }
    }
    selectors
}

fn parse_nth_arg(p: &mut Parser) -> i32 {
    let start = p.pos;
    while !p.eof() && p.peek() != b')' {
        p.pos += 1;
    }
    let arg = core::str::from_utf8(&p.input[start..p.pos]).unwrap_or("");
    let arg = arg.trim();
    match arg {
        "odd" => 1,
        "even" => 2,
        _ => parse_int(arg).unwrap_or(1),
    }
}

// ---------------------------------------------------------------------------
// Declaration parser
// ---------------------------------------------------------------------------
