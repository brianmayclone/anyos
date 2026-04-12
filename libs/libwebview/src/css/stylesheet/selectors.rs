fn lower_qualified_rule_ast(node: &CssQualifiedRuleNode, current_layer: Option<&str>) -> Option<Rule> {
    let selectors = lower_selector_list_ast(&parse_selector_list_ast(&node.prelude));
    if selectors.is_empty() {
        return None;
    }
    let declarations = lower_declaration_list_ast(&parse_declaration_list_ast(&node.block.source));
    Some(Rule {
        selectors,
        declarations,
        layer_name: current_layer.map(String::from),
        layer_index: None,
        container_query: None,
    })
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
