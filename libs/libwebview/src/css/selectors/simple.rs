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
        if parsed == Tag::Unknown {
            custom_tag = Some(name.to_ascii_lowercase());
        }
        tag = Some(parsed);
    }

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
            p.pos += 1;
            let name = p.read_ident();
            let lower = to_ascii_lower(&name);
            match lower.as_str() {
                "before" => pseudo_element = Some(PseudoElement::Before),
                "after" => pseudo_element = Some(PseudoElement::After),
                _ => {
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
    p.pos += 1;
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
