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

    let name = input[tokens[*cursor].start + 1..tokens[*cursor].end]
        .trim()
        .to_ascii_lowercase();
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
