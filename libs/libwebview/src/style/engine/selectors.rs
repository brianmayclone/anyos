// Selector matching
// ---------------------------------------------------------------------------

/// Check if a CSS selector matches a DOM element node.
fn selector_matches(
    selector: &Selector,
    dom: &Dom,
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    // Skip selectors that target ::before/::after — those are handled separately.
    if selector.pseudo_element().is_some() {
        return false;
    }
    selector_matches_base(selector, dom, node_id, selector_state)
}

/// Match a selector against a node, ignoring the pseudo-element part.
/// Used both for normal matching (via selector_matches) and pseudo-element resolution.
fn selector_matches_base(
    selector: &Selector,
    dom: &Dom,
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    // Bounds check to prevent crashes from corrupted node indices.
    if node_id >= dom.nodes.len() {
        return false;
    }
    match selector {
        Selector::Universal => {
            matches!(dom.nodes[node_id].node_type, NodeType::Element { .. })
        }
        Selector::Simple(simple) => simple_matches(simple, dom, node_id, selector_state),
        Selector::Descendant(ancestor_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id, selector_state) {
                return false;
            }
            let mut cur = dom.nodes[node_id].parent;
            while let Some(pid) = cur {
                if pid >= dom.nodes.len() {
                    break;
                }
                if selector_matches_base(ancestor_sel, dom, pid, selector_state) {
                    return true;
                }
                cur = dom.nodes[pid].parent;
            }
            false
        }
        Selector::Child(parent_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id, selector_state) {
                return false;
            }
            if let Some(pid) = dom.nodes[node_id].parent {
                if pid >= dom.nodes.len() {
                    return false;
                }
                selector_matches_base(parent_sel, dom, pid, selector_state)
            } else {
                false
            }
        }
        Selector::AdjacentSibling(prev_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id, selector_state) {
                return false;
            }
            // Find preceding sibling element
            if let Some(sib) = preceding_element_sibling(dom, node_id) {
                selector_matches_base(prev_sel, dom, sib, selector_state)
            } else {
                false
            }
        }
        Selector::GeneralSibling(prev_sel, leaf) => {
            if !simple_matches(leaf, dom, node_id, selector_state) {
                return false;
            }
            // Check all preceding sibling elements
            let mut sib = preceding_element_sibling(dom, node_id);
            while let Some(sid) = sib {
                if selector_matches_base(prev_sel, dom, sid, selector_state) {
                    return true;
                }
                sib = preceding_element_sibling(dom, sid);
            }
            false
        }
    }
}

/// Find the immediately preceding element sibling of `node_id`.
fn preceding_element_sibling(dom: &Dom, node_id: NodeId) -> Option<NodeId> {
    let parent = dom.nodes[node_id].parent?;
    let children = &dom.nodes[parent].children;
    let pos = children.iter().position(|&c| c == node_id)?;
    // Walk backwards from pos-1 to find first element
    for i in (0..pos).rev() {
        if matches!(dom.nodes[children[i]].node_type, NodeType::Element { .. }) {
            return Some(children[i]);
        }
    }
    Option::None
}

fn simple_matches(
    sel: &SimpleSelector,
    dom: &Dom,
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    if node_id >= dom.nodes.len() {
        return false;
    }
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        NodeType::Text(_) => return false,
    };

    // Tag check.
    if let Some(sel_tag) = sel.tag {
        if sel_tag != *tag {
            return false;
        }
        // For custom/unknown elements, additionally verify that the raw tag name
        // matches. This prevents e.g. `a-analytics { }` from matching all unknown
        // elements — it should only match `<a-analytics>` nodes.
        if sel_tag == Tag::Unknown {
            if let Some(ref custom) = sel.custom_tag {
                let node_custom = attrs
                    .iter()
                    .find(|a| a.name == "\x00")
                    .map(|a| a.value.as_str());
                if node_custom != Some(custom.as_str()) {
                    return false;
                }
            }
        }
    }

    // ID check.
    if let Some(ref sel_id) = sel.id {
        let node_id_attr = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "id"));
        match node_id_attr {
            Some(a) if eq_ignore_ascii_case(&a.value, sel_id) => {}
            _ => return false,
        }
    }

    // Class check: every selector class must be present on the node.
    if !sel.classes.is_empty() {
        let class_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, "class"));
        let class_str = match class_attr {
            Some(a) => &a.value,
            Option::None => return false,
        };
        for sc in &sel.classes {
            if !has_class(class_str, sc) {
                return false;
            }
        }
    }

    // Attribute selector check.
    for attr_sel in &sel.attrs {
        let node_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, &attr_sel.name));
        match attr_sel.op {
            AttrOp::Exists => {
                if node_attr.is_none() {
                    return false;
                }
            }
            AttrOp::Exact => match (node_attr, &attr_sel.value) {
                (Some(a), Some(v)) if eq_ignore_ascii_case(&a.value, v) => {}
                _ => return false,
            },
            AttrOp::Contains => {
                // [attr~=val]: word in space-separated list
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) if has_class(&a.value, v) => {}
                    _ => return false,
                }
            }
            AttrOp::Prefix => match (node_attr, &attr_sel.value) {
                (Some(a), Some(v)) => {
                    if !starts_with_ignore_case(&a.value, v) {
                        return false;
                    }
                }
                _ => return false,
            },
            AttrOp::Suffix => match (node_attr, &attr_sel.value) {
                (Some(a), Some(v)) => {
                    if !ends_with_ignore_case(&a.value, v) {
                        return false;
                    }
                }
                _ => return false,
            },
            AttrOp::Substring => match (node_attr, &attr_sel.value) {
                (Some(a), Some(v)) => {
                    if !contains_ignore_case(&a.value, v) {
                        return false;
                    }
                }
                _ => return false,
            },
            AttrOp::DashMatch => {
                // [attr|=val]: exact or starts with val-
                match (node_attr, &attr_sel.value) {
                    (Some(a), Some(v)) => {
                        if !eq_ignore_ascii_case(&a.value, v)
                            && !starts_with_ignore_case(&a.value, &{
                                let mut s = v.clone();
                                s.push('-');
                                s
                            })
                        {
                            return false;
                        }
                    }
                    _ => return false,
                }
            }
        }
    }

    // Pseudo-class check.
    for pc in &sel.pseudo_classes {
        if !pseudo_class_matches(pc, dom, node_id, selector_state) {
            return false;
        }
    }

    true
}

fn pseudo_class_matches(
    pc: &PseudoClass,
    dom: &Dom,
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    match pc {
        PseudoClass::Root => {
            // Root is the <html> element (no parent or parent is document root)
            dom.nodes[node_id].parent.is_none() || dom.nodes[node_id].parent == Some(0)
        }
        PseudoClass::FirstChild => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                children
                    .iter()
                    .find(|&&c| matches!(dom.nodes[c].node_type, NodeType::Element { .. }))
                    == Some(&node_id)
            } else {
                false
            }
        }
        PseudoClass::LastChild => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                children
                    .iter()
                    .rev()
                    .find(|&&c| matches!(dom.nodes[c].node_type, NodeType::Element { .. }))
                    == Some(&node_id)
            } else {
                false
            }
        }
        PseudoClass::NthChild(n) => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                let mut count = 0i32;
                for &c in children {
                    if matches!(dom.nodes[c].node_type, NodeType::Element { .. }) {
                        count += 1;
                        if c == node_id {
                            return count == *n;
                        }
                    }
                }
            }
            false
        }
        PseudoClass::NthLastChild(n) => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let children = &dom.nodes[pid].children;
                let mut count = 0i32;
                for &c in children.iter().rev() {
                    if matches!(dom.nodes[c].node_type, NodeType::Element { .. }) {
                        count += 1;
                        if c == node_id {
                            return count == *n;
                        }
                    }
                }
            }
            false
        }
        PseudoClass::FirstOfType => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let my_tag = dom.tag(node_id);
                let children = &dom.nodes[pid].children;
                for &c in children {
                    if dom.tag(c) == my_tag {
                        return c == node_id;
                    }
                }
            }
            false
        }
        PseudoClass::LastOfType => {
            if let Some(pid) = dom.nodes[node_id].parent {
                let my_tag = dom.tag(node_id);
                let children = &dom.nodes[pid].children;
                for &c in children.iter().rev() {
                    if dom.tag(c) == my_tag {
                        return c == node_id;
                    }
                }
            }
            false
        }
        PseudoClass::Empty => dom.nodes[node_id].children.is_empty(),
        PseudoClass::Not(selectors) => {
            // :not(a, b, c) — matches if NONE of the listed selectors match.
            !selectors
                .iter()
                .any(|sel| simple_matches(sel, dom, node_id, selector_state))
        }
        PseudoClass::Checked | PseudoClass::Disabled | PseudoClass::Enabled => {
            // Check for corresponding HTML attributes
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                match pc {
                    PseudoClass::Checked => attrs
                        .iter()
                        .any(|a| eq_ignore_ascii_case(&a.name, "checked")),
                    PseudoClass::Disabled => attrs
                        .iter()
                        .any(|a| eq_ignore_ascii_case(&a.name, "disabled")),
                    PseudoClass::Enabled => !attrs
                        .iter()
                        .any(|a| eq_ignore_ascii_case(&a.name, "disabled")),
                    _ => false,
                }
            } else {
                false
            }
        }
        // :is() — matches if any selector in the list matches.
        PseudoClass::Is(selectors) | PseudoClass::Where(selectors) => selectors
            .iter()
            .any(|sel| simple_selector_matches(sel, dom, node_id, selector_state)),
        // :has() — matches if any descendant matches the inner selector.
        PseudoClass::Has(sel) => subtree_has_match(dom, node_id, sel, selector_state),
        PseudoClass::FocusVisible => selector_state.focus_visible_node == Some(node_id),
        PseudoClass::FocusWithin => {
            selector_state.focused_node == Some(node_id)
                || has_descendant_node(dom, node_id, selector_state.focused_node)
        }
        // :placeholder-shown — check if input has no value.
        PseudoClass::PlaceholderShown => {
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                let has_value = attrs
                    .iter()
                    .any(|a| eq_ignore_ascii_case(&a.name, "value") && !a.value.is_empty());
                !has_value
            } else {
                false
            }
        }
        PseudoClass::Hover => {
            selector_state.hovered_node == Some(node_id)
                || has_descendant_node(dom, node_id, selector_state.hovered_node)
        }
        PseudoClass::Active => {
            selector_state.active_node == Some(node_id)
                || has_descendant_node(dom, node_id, selector_state.active_node)
        }
        PseudoClass::Focus => selector_state.focused_node == Some(node_id),
        // Browsers restrict :visited matching for privacy; until we have a
        // history model, treat it as not matched.
        PseudoClass::Visited => false,
        // ── Form validation pseudo-classes (HTML §4.10.21) ──────────────────
        PseudoClass::Required => {
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                attrs
                    .iter()
                    .any(|a| eq_ignore_ascii_case(&a.name, "required"))
            } else {
                false
            }
        }
        PseudoClass::Optional => {
            let tag = dom.tag(node_id);
            let is_form_el = matches!(
                tag,
                Some(Tag::Input) | Some(Tag::Select) | Some(Tag::Textarea)
            );
            if !is_form_el {
                return false;
            }
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                !attrs
                    .iter()
                    .any(|a| eq_ignore_ascii_case(&a.name, "required"))
            } else {
                false
            }
        }
        PseudoClass::ReadOnly => {
            if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                let tag = dom.tag(node_id);
                if matches!(tag, Some(Tag::Input) | Some(Tag::Textarea)) {
                    attrs
                        .iter()
                        .any(|a| eq_ignore_ascii_case(&a.name, "readonly"))
                        || attrs
                            .iter()
                            .any(|a| eq_ignore_ascii_case(&a.name, "disabled"))
                } else {
                    // Non-editable elements are :read-only by default.
                    true
                }
            } else {
                true
            }
        }
        PseudoClass::ReadWrite => {
            let tag = dom.tag(node_id);
            if matches!(tag, Some(Tag::Input) | Some(Tag::Textarea)) {
                if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                    !attrs.iter().any(|a| {
                        eq_ignore_ascii_case(&a.name, "readonly")
                            || eq_ignore_ascii_case(&a.name, "disabled")
                    })
                } else {
                    false
                }
            } else {
                false
            }
        }
        PseudoClass::Valid => {
            let tag = dom.tag(node_id);
            if !matches!(
                tag,
                Some(Tag::Input) | Some(Tag::Select) | Some(Tag::Textarea)
            ) {
                return false;
            }
            crate::dom::validate_form_control(dom, node_id).is_valid()
        }
        PseudoClass::Invalid => {
            let tag = dom.tag(node_id);
            if !matches!(
                tag,
                Some(Tag::Input) | Some(Tag::Select) | Some(Tag::Textarea)
            ) {
                return false;
            }
            !crate::dom::validate_form_control(dom, node_id).is_valid()
        }
        PseudoClass::InRange => {
            let tag = dom.tag(node_id);
            if tag != Some(Tag::Input) {
                return false;
            }
            let r = crate::dom::validate_form_control(dom, node_id);
            // :in-range matches if neither underflow nor overflow.
            !r.range_underflow && !r.range_overflow
        }
        PseudoClass::OutOfRange => {
            let tag = dom.tag(node_id);
            if tag != Some(Tag::Input) {
                return false;
            }
            let r = crate::dom::validate_form_control(dom, node_id);
            r.range_underflow || r.range_overflow
        }
        PseudoClass::Default => {
            // :default matches the default submit button or initially-checked checkbox/radio.
            let tag = dom.tag(node_id);
            if tag == Some(Tag::Input) {
                if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                    let is_submit = attrs.iter().any(|a| {
                        eq_ignore_ascii_case(&a.name, "type")
                            && eq_ignore_ascii_case(&a.value, "submit")
                    });
                    let is_checked_default = attrs
                        .iter()
                        .any(|a| eq_ignore_ascii_case(&a.name, "checked"));
                    is_submit || is_checked_default
                } else {
                    false
                }
            } else if tag == Some(Tag::Button) {
                // First submit button is :default.
                true
            } else {
                false
            }
        }
        PseudoClass::Indeterminate => {
            let tag = dom.tag(node_id);
            if tag == Some(Tag::Progress) {
                // <progress> without value attribute is indeterminate.
                if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                    !attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, "value"))
                } else {
                    false
                }
            } else if tag == Some(Tag::Input) {
                if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                    let input_type = attrs
                        .iter()
                        .find(|a| eq_ignore_ascii_case(&a.name, "type"))
                        .map(|a| a.value.as_str())
                        .unwrap_or("text");
                    if input_type == "checkbox" {
                        // Checkbox: indeterminate if has `indeterminate` attribute (set via JS).
                        attrs
                            .iter()
                            .any(|a| eq_ignore_ascii_case(&a.name, "indeterminate"))
                    } else if input_type == "radio" {
                        // Radio: indeterminate if no radio in the same name group is checked.
                        let name = attrs
                            .iter()
                            .find(|a| eq_ignore_ascii_case(&a.name, "name"))
                            .map(|a| a.value.as_str())
                            .unwrap_or("");
                        if name.is_empty() {
                            // No name group — check just this one.
                            !attrs
                                .iter()
                                .any(|a| eq_ignore_ascii_case(&a.name, "checked"))
                        } else {
                            // Check all radios with same name in the form.
                            !radio_group_has_checked(dom, node_id, name)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
    }
}

/// Check if a SimpleSelector matches a node (used for :is/:where/:has).
fn simple_selector_matches(
    sel: &SimpleSelector,
    dom: &Dom,
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    if let Some(tag) = sel.tag {
        if dom.tag(node_id) != Some(tag) {
            return false;
        }
        if tag == Tag::Unknown {
            if let Some(ref custom) = sel.custom_tag {
                if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
                    let node_custom = attrs
                        .iter()
                        .find(|a| a.name == "\x00")
                        .map(|a| a.value.as_str());
                    if node_custom != Some(custom.as_str()) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
    }
    if let Some(ref id) = sel.id {
        if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
            let has_id = attrs
                .iter()
                .any(|a| eq_ignore_ascii_case(&a.name, "id") && eq_ignore_ascii_case(&a.value, id));
            if !has_id {
                return false;
            }
        } else {
            return false;
        }
    }
    for cls in &sel.classes {
        if let NodeType::Element { attrs, .. } = &dom.nodes[node_id].node_type {
            let class_attr = attrs
                .iter()
                .find(|a| eq_ignore_ascii_case(&a.name, "class"));
            let has_class = class_attr.map_or(false, |a| {
                a.value
                    .split_whitespace()
                    .any(|c| eq_ignore_ascii_case(c, cls))
            });
            if !has_class {
                return false;
            }
        } else {
            return false;
        }
    }
    for pc in &sel.pseudo_classes {
        if !pseudo_class_matches(pc, dom, node_id, selector_state) {
            return false;
        }
    }
    true
}

/// Check if any radio button with the given name in the form is checked.
fn radio_group_has_checked(dom: &Dom, radio_node: NodeId, name: &str) -> bool {
    // Find the parent <form> (or document root).
    let mut form_root = 0;
    let mut cur = dom.nodes[radio_node].parent;
    while let Some(pid) = cur {
        if dom.tag(pid) == Some(Tag::Form) {
            form_root = pid;
            break;
        }
        cur = dom.nodes[pid].parent;
    }
    // Walk all descendants of form_root looking for radio inputs with matching name.
    let mut stack = dom.nodes[form_root].children.clone();
    while let Some(nid) = stack.pop() {
        if dom.tag(nid) == Some(Tag::Input) {
            if let NodeType::Element { ref attrs, .. } = dom.nodes[nid].node_type {
                let is_radio = attrs.iter().any(|a| {
                    eq_ignore_ascii_case(&a.name, "type") && eq_ignore_ascii_case(&a.value, "radio")
                });
                if is_radio {
                    let n = attrs
                        .iter()
                        .find(|a| eq_ignore_ascii_case(&a.name, "name"))
                        .map(|a| a.value.as_str())
                        .unwrap_or("");
                    if n == name {
                        if attrs
                            .iter()
                            .any(|a| eq_ignore_ascii_case(&a.name, "checked"))
                        {
                            return true;
                        }
                    }
                }
            }
        }
        for &cid in dom.nodes[nid].children.iter().rev() {
            stack.push(cid);
        }
    }
    false
}

fn subtree_has_match(
    dom: &Dom,
    root_id: NodeId,
    sel: &SimpleSelector,
    selector_state: &SelectorState,
) -> bool {
    let children = &dom.nodes[root_id].children;
    for &child in children {
        if matches!(dom.nodes[child].node_type, NodeType::Element { .. })
            && simple_selector_matches(sel, dom, child, selector_state)
        {
            return true;
        }
        if subtree_has_match(dom, child, sel, selector_state) {
            return true;
        }
    }
    false
}

fn has_descendant_node(dom: &Dom, root_id: NodeId, target: Option<NodeId>) -> bool {
    let Some(target_id) = target else {
        return false;
    };
    let children = &dom.nodes[root_id].children;
    for &child in children {
        if child == target_id || has_descendant_node(dom, child, Some(target_id)) {
            return true;
        }
    }
    false
}

fn element_has_attr(dom: &Dom, node_id: NodeId, name: &str) -> bool {
    let Some(node) = dom.nodes.get(node_id) else {
        return false;
    };
    let NodeType::Element { attrs, .. } = &node.node_type else {
        return false;
    };
    attrs.iter().any(|a| eq_ignore_ascii_case(&a.name, name))
}

fn element_has_screen_reader_only_class(dom: &Dom, node_id: NodeId) -> bool {
    let Some(class_attr) = dom.attr(node_id, "class") else {
        return false;
    };
    for class_name in [
        "sr-only",
        "visually-hidden",
        "screen-reader-only",
        "screenreader-only",
        "u-hidden-visually",
        "a11y-hidden",
    ] {
        if has_class(class_attr, class_name) {
            return true;
        }
    }
    false
}

fn ancestor_has_attr(dom: &Dom, node_id: NodeId, name: &str) -> bool {
    let mut cur = dom.nodes.get(node_id).and_then(|n| n.parent);
    while let Some(pid) = cur {
        if element_has_attr(dom, pid, name) {
            return true;
        }
        cur = dom.nodes.get(pid).and_then(|n| n.parent);
    }
    false
}

fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .get(..needle.len())
        .is_some_and(|prefix| eq_ignore_ascii_case(prefix, needle))
}

fn ends_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .get(haystack.len() - needle.len()..)
        .is_some_and(|suffix| eq_ignore_ascii_case(suffix, needle))
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    for (i, _) in haystack.char_indices() {
        if i + needle.len() > haystack.len() {
            break;
        }
        if haystack
            .get(i..i + needle.len())
            .is_some_and(|part| eq_ignore_ascii_case(part, needle))
        {
            return true;
        }
    }
    false
}

/// Check if `class_str` (space-separated class list) contains `needle`
/// (case-insensitive).
fn has_class(class_str: &str, needle: &str) -> bool {
    for tok in class_str.split(|c: char| c == ' ' || c == '\t' || c == '\n') {
        if eq_ignore_ascii_case(tok, needle) {
            return true;
        }
    }
    false
}

fn has_skip_link_class(class_str: &str) -> bool {
    for tok in class_str.split(|c: char| c == ' ' || c == '\t' || c == '\n') {
        if eq_ignore_ascii_case(tok, "skip-link")
            || eq_ignore_ascii_case(tok, "skiplink")
            || eq_ignore_ascii_case(tok, "skip-to-content")
        {
            return true;
        }
    }
    false
}

fn has_visually_hidden_class(class_str: &str) -> bool {
    for tok in class_str.split_ascii_whitespace() {
        if eq_ignore_ascii_case(tok, "sr-only")
            || eq_ignore_ascii_case(tok, "visually-hidden")
            || eq_ignore_ascii_case(tok, "screen-reader-text")
            || eq_ignore_ascii_case(tok, "u-hidden-visually")
        {
            return true;
        }
    }
    false
}

fn is_unfocused_skip_link(
    tag: Tag,
    attrs: &[crate::dom::Attr],
    node_id: NodeId,
    selector_state: &SelectorState,
) -> bool {
    if tag != Tag::A || selector_state.focused_node == Some(node_id) {
        return false;
    }

    let mut has_skip_class = false;
    let mut has_fragment_href = false;
    for attr in attrs {
        if eq_ignore_ascii_case(&attr.name, "class") && has_skip_link_class(&attr.value) {
            has_skip_class = true;
        } else if eq_ignore_ascii_case(&attr.name, "href")
            && attr.value.trim_start().starts_with('#')
        {
            has_fragment_href = true;
        }
    }

    has_skip_class && has_fragment_href
}

#[derive(Clone)]
struct ActiveContainer {
    node_id: NodeId,
    names: Vec<String>,
    inline_size: i32,
    block_size: i32,
}

fn is_ancestor_or_self(dom: &Dom, ancestor: NodeId, node_id: NodeId) -> bool {
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        if id == ancestor {
            return true;
        }
        cur = dom.nodes.get(id).and_then(|n| n.parent);
    }
    false
}

fn estimated_content_inline_size(style: &ComputedStyle, available_inline_size: i32) -> i32 {
    let border_pad = style.padding_left
        + style.padding_right
        + style.border_left.width
        + style.border_right.width;
    let margin = style.margin_left.max(0) + style.margin_right.max(0);
    let base = if let Some(width) = style.width {
        width
    } else if let Some(pct) = style.width_pct {
        (available_inline_size.max(0) * pct) / 10000
    } else if let Some((px, pct)) = style.width_calc {
        px / 100 + (available_inline_size.max(0) * pct) / 10000
    } else {
        (available_inline_size - margin).max(0)
    };
    if style.box_sizing == BoxSizing::BorderBox {
        (base - border_pad).max(0)
    } else {
        base.max(0)
    }
}

fn estimated_content_block_size(style: &ComputedStyle, available_block_size: i32) -> i32 {
    let border_pad = style.padding_top
        + style.padding_bottom
        + style.border_top.width
        + style.border_bottom.width;
    let base = if let Some(height) = style.height {
        height
    } else if let Some(pct) = style.height_pct {
        (available_block_size.max(0) * pct) / 10000
    } else if let Some((px, pct)) = style.height_calc {
        px / 100 + (available_block_size.max(0) * pct) / 10000
    } else {
        available_block_size.max(0)
    };
    if style.box_sizing == BoxSizing::BorderBox {
        (base - border_pad).max(0)
    } else {
        base.max(0)
    }
}

fn container_query_matches(query: &ContainerQuery, containers: &[ActiveContainer]) -> bool {
    let container = containers.iter().rev().find(|container| {
        if let Some(ref wanted_name) = query.name {
            container
                .names
                .iter()
                .any(|name| eq_ignore_ascii_case(name, wanted_name))
        } else {
            true
        }
    });
    let Some(container) = container else {
        return false;
    };
    query.conditions.iter().all(|cond| match cond {
        ContainerCondition::MinWidth(v) | ContainerCondition::MinInlineSize(v) => {
            container.inline_size >= *v
        }
        ContainerCondition::MaxWidth(v) | ContainerCondition::MaxInlineSize(v) => {
            container.inline_size <= *v
        }
        ContainerCondition::Width(v) | ContainerCondition::InlineSize(v) => {
            container.inline_size == *v
        }
        ContainerCondition::MinHeight(v) | ContainerCondition::MinBlockSize(v) => {
            container.block_size >= *v
        }
        ContainerCondition::MaxHeight(v) | ContainerCondition::MaxBlockSize(v) => {
            container.block_size <= *v
        }
        ContainerCondition::Height(v) | ContainerCondition::BlockSize(v) => {
            container.block_size == *v
        }
    })
}

// ---------------------------------------------------------------------------
