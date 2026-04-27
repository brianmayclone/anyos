fn lower_qualified_rule_ast(node: &CssQualifiedRuleNode, current_layer: Option<&str>) -> Vec<Rule> {
    lower_qualified_rule_ast_inner(node, current_layer, None)
}

fn lower_qualified_rule_ast_inner(
    node: &CssQualifiedRuleNode,
    current_layer: Option<&str>,
    parent_prelude: Option<&str>,
) -> Vec<Rule> {
    let prelude = if let Some(parent) = parent_prelude {
        combine_nested_selector_preludes(parent, &node.prelude)
    } else {
        node.prelude.clone()
    };

    let mut rules = Vec::new();
    let selectors = lower_selector_list_ast(&parse_selector_list_ast(&prelude));
    if selectors.is_empty() {
        return rules;
    }
    let declarations = lower_declaration_list_ast(&parse_declaration_list_ast(&node.block.source));
    if !declarations.is_empty() {
        rules.push(Rule {
            selectors,
            declarations,
            layer_name: current_layer.map(String::from),
            layer_index: None,
            container_query: None,
        });
    }

    for item in &node.block.items {
        if let CssSyntaxNode::QualifiedRule(nested) = item {
            rules.extend(lower_qualified_rule_ast_inner(
                nested,
                current_layer,
                Some(&prelude),
            ));
        }
    }

    rules
}

fn combine_nested_selector_preludes(parent_prelude: &str, nested_prelude: &str) -> String {
    let parents = split_selector_list_text(parent_prelude);
    let nested = split_selector_list_text(nested_prelude);
    let mut out = String::new();

    for parent in &parents {
        for child in &nested {
            let combined = if child.contains('&') {
                replace_nesting_parent(child, parent)
            } else {
                let mut s = String::from(parent.trim());
                if !s.is_empty() && !child.trim().is_empty() {
                    s.push(' ');
                }
                s.push_str(child.trim());
                s
            };
            if !combined.trim().is_empty() {
                if !out.is_empty() {
                    out.push_str(", ");
                }
                out.push_str(combined.trim());
            }
        }
    }

    out
}

fn replace_nesting_parent(selector: &str, parent: &str) -> String {
    let mut out = String::new();
    let bytes = selector.as_bytes();
    let mut i = 0usize;
    let mut string_quote = 0u8;
    while i < bytes.len() {
        let ch = bytes[i];
        if string_quote != 0 {
            out.push(ch as char);
            if ch == b'\\' && i + 1 < bytes.len() {
                i += 1;
                out.push(bytes[i] as char);
            } else if ch == string_quote {
                string_quote = 0;
            }
            i += 1;
            continue;
        }
        match ch {
            b'"' | b'\'' => {
                string_quote = ch;
                out.push(ch as char);
                i += 1;
            }
            b'&' => {
                out.push_str(parent.trim());
                i += 1;
            }
            _ => {
                out.push(ch as char);
                i += 1;
            }
        }
    }
    out
}

fn split_selector_list_text(input: &str) -> Vec<String> {
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
                let item = input[start..i].trim();
                if !item.is_empty() {
                    out.push(String::from(item));
                }
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }

    let item = input[start..].trim();
    if !item.is_empty() {
        out.push(String::from(item));
    }
    out
}

fn lower_selector_list_ast(ast: &[CssSelectorAst]) -> Vec<Selector> {
    let mut out = Vec::new();
    for sel in ast {
        if let Some(lowered) = lower_selector_ast(sel) {
            out.push(lowered);
        }
    }
    out
}

fn lower_selector_ast(ast: &CssSelectorAst) -> Option<Selector> {
    let first = lower_simple_selector_ast(&ast.first)?;
    let mut result = if ast.first.explicit_universal && is_universal(&first) {
        Selector::Universal
    } else {
        Selector::Simple(first)
    };
    for (comb, simple) in &ast.rest {
        let next = lower_simple_selector_ast(simple)?;
        result = match comb {
            CssCombinatorAst::Descendant => Selector::Descendant(Box::new(result), next),
            CssCombinatorAst::Child => Selector::Child(Box::new(result), next),
            CssCombinatorAst::AdjacentSibling => Selector::AdjacentSibling(Box::new(result), next),
            CssCombinatorAst::GeneralSibling => Selector::GeneralSibling(Box::new(result), next),
        };
    }
    Some(result)
}

fn lower_simple_selector_ast(ast: &CssSimpleSelectorAst) -> Option<SimpleSelector> {
    let mut tag = None;
    let mut custom_tag = None;
    if let Some(tag_name) = &ast.tag_name {
        let parsed = Tag::from_str(tag_name);
        if parsed == Tag::Unknown {
            custom_tag = Some(tag_name.to_ascii_lowercase());
        }
        tag = Some(parsed);
    }

    let mut attrs = Vec::new();
    for attr in &ast.attrs {
        attrs.push(AttrSelector {
            name: attr.name.clone(),
            op: match attr.op {
                CssAttrOpAst::Exists => AttrOp::Exists,
                CssAttrOpAst::Exact => AttrOp::Exact,
                CssAttrOpAst::Contains => AttrOp::Contains,
                CssAttrOpAst::Prefix => AttrOp::Prefix,
                CssAttrOpAst::Suffix => AttrOp::Suffix,
                CssAttrOpAst::Substring => AttrOp::Substring,
                CssAttrOpAst::DashMatch => AttrOp::DashMatch,
            },
            value: attr.value.clone(),
        });
    }

    let mut pseudo_classes = Vec::new();
    for pseudo in &ast.pseudo_classes {
        pseudo_classes.push(lower_pseudo_class_ast(pseudo)?);
    }

    let pseudo_element = ast.pseudo_element.map(|pseudo| match pseudo {
        CssPseudoElementAst::Before => PseudoElement::Before,
        CssPseudoElementAst::After => PseudoElement::After,
        CssPseudoElementAst::Unknown => PseudoElement::Unknown,
    });

    Some(SimpleSelector {
        tag,
        custom_tag,
        id: ast.id.clone(),
        classes: ast.classes.clone(),
        attrs,
        pseudo_classes,
        pseudo_element,
    })
}

fn lower_pseudo_class_ast(ast: &CssPseudoClassAst) -> Option<PseudoClass> {
    Some(match ast {
        CssPseudoClassAst::Hover => PseudoClass::Hover,
        CssPseudoClassAst::Active => PseudoClass::Active,
        CssPseudoClassAst::Focus => PseudoClass::Focus,
        CssPseudoClassAst::Visited => PseudoClass::Visited,
        CssPseudoClassAst::FirstChild => PseudoClass::FirstChild,
        CssPseudoClassAst::LastChild => PseudoClass::LastChild,
        CssPseudoClassAst::NthChild(n) => PseudoClass::NthChild(*n),
        CssPseudoClassAst::NthLastChild(n) => PseudoClass::NthLastChild(*n),
        CssPseudoClassAst::FirstOfType => PseudoClass::FirstOfType,
        CssPseudoClassAst::LastOfType => PseudoClass::LastOfType,
        CssPseudoClassAst::Not(selectors) => PseudoClass::Not(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Is(selectors) => PseudoClass::Is(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Where(selectors) => PseudoClass::Where(lower_simple_selector_list_ast(selectors)?),
        CssPseudoClassAst::Has(selector) => PseudoClass::Has(Box::new(lower_simple_selector_ast(selector)?)),
        CssPseudoClassAst::Empty => PseudoClass::Empty,
        CssPseudoClassAst::Checked => PseudoClass::Checked,
        CssPseudoClassAst::Disabled => PseudoClass::Disabled,
        CssPseudoClassAst::Enabled => PseudoClass::Enabled,
        CssPseudoClassAst::Root => PseudoClass::Root,
        CssPseudoClassAst::FocusVisible => PseudoClass::FocusVisible,
        CssPseudoClassAst::FocusWithin => PseudoClass::FocusWithin,
        CssPseudoClassAst::PlaceholderShown => PseudoClass::PlaceholderShown,
        CssPseudoClassAst::Required => PseudoClass::Required,
        CssPseudoClassAst::Optional => PseudoClass::Optional,
        CssPseudoClassAst::ReadOnly => PseudoClass::ReadOnly,
        CssPseudoClassAst::ReadWrite => PseudoClass::ReadWrite,
        CssPseudoClassAst::Valid => PseudoClass::Valid,
        CssPseudoClassAst::Invalid => PseudoClass::Invalid,
        CssPseudoClassAst::InRange => PseudoClass::InRange,
        CssPseudoClassAst::OutOfRange => PseudoClass::OutOfRange,
        CssPseudoClassAst::Default => PseudoClass::Default,
        CssPseudoClassAst::Indeterminate => PseudoClass::Indeterminate,
    })
}

fn lower_simple_selector_list_ast(ast: &[CssSimpleSelectorAst]) -> Option<Vec<SimpleSelector>> {
    let mut out = Vec::with_capacity(ast.len());
    for selector in ast {
        out.push(lower_simple_selector_ast(selector)?);
    }
    Some(out)
}

#[cfg(test)]
mod stylesheet_selector_tests {
    use super::*;

    #[test]
    fn nested_selectors_expand_with_parent_reference() {
        let sheet = parse_stylesheet(
            ".card, .panel { color: black; &:hover, &.active { color: red; } & > .title { font-weight: 700; } }",
        );

        assert_eq!(sheet.rules.len(), 3);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(sheet.rules[1].selectors.len(), 4);
        assert_eq!(sheet.rules[2].selectors.len(), 2);
        assert!(sheet.rules[1]
            .declarations
            .iter()
            .any(|d| d.property == Property::Color));
        assert!(sheet.rules[2]
            .declarations
            .iter()
            .any(|d| d.property == Property::FontWeight));
    }

    #[test]
    fn selector_list_split_ignores_commas_in_arguments_and_attributes() {
        let selectors = split_selector_list_text(r#".a:is(.b, .c), [data-x="a,b"], .d"#);
        assert_eq!(selectors.len(), 3);
        assert_eq!(selectors[0], ".a:is(.b, .c)");
        assert_eq!(selectors[1], r#"[data-x="a,b"]"#);
        assert_eq!(selectors[2], ".d");
    }

    #[test]
    fn media_and_supports_blocks_use_ast_lowering_for_nested_rules() {
        let sheet = parse_stylesheet(
            "@media (min-width: 20px) { .card { color: black; &:hover { color: red; } } } @supports (display: grid) { .grid { & > .item { display: block; } } }",
        );

        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.media_rules.len(), 1);
        assert_eq!(sheet.media_rules[0].rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert!(sheet.rules[0]
            .declarations
            .iter()
            .any(|d| d.property == Property::Display));
    }
}
