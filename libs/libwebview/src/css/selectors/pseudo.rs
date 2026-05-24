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
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Not(selectors))
            } else {
                Some(PseudoClass::Unsupported)
            }
        }
        "is" | "matches" | "-webkit-any" | "-moz-any" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Is(selectors))
            } else {
                Some(PseudoClass::Unsupported)
            }
        }
        "where" => {
            if p.peek() == b'(' {
                let selectors = parse_selector_list_in_parens(p);
                Some(PseudoClass::Where(selectors))
            } else {
                Some(PseudoClass::Unsupported)
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
                Some(PseudoClass::Unsupported)
            }
        }
        "-moz-focusring" => Some(PseudoClass::FocusVisible),
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
            Some(PseudoClass::Unsupported)
        }
    }
}

fn parse_selector_list_in_parens(p: &mut Parser) -> Vec<SimpleSelector> {
    let mut selectors = Vec::new();
    if p.peek() != b'(' {
        return selectors;
    }
    p.pos += 1;
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
        if p.pos == before {
            p.pos += 1;
        }
        if selectors.len() > 1000 {
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
