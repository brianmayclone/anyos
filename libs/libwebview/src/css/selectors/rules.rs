fn parse_rule(p: &mut Parser, current_layer: Option<&str>) -> Option<Rule> {
    let selectors = parse_selector_list(p);
    if selectors.is_empty() {
        return Option::None;
    }

    p.skip_whitespace();
    if p.peek() != b'{' {
        while !p.eof() && p.peek() != b'{' && p.peek() != b'}' {
            p.pos += 1;
        }
        if p.peek() == b'{' {
            p.skip_block();
        }
        return Option::None;
    }
    p.pos += 1;

    let declarations = parse_declarations(p);

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
