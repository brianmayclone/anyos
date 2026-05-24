// Rule index — O(1) lookup by tag / ID / class
// ---------------------------------------------------------------------------

/// Number of Tag enum variants (used to size the tag bucket array).
const TAG_COUNT: usize = 128; // Tag enum has ~100 variants, 128 is safe

/// Pre-built index for fast rule lookup by the leaf selector's tag, ID, or class.
///
/// Instead of checking all N rules against every DOM node (O(nodes × rules)),
/// we partition rules into buckets so that for a given `<div id="foo" class="bar baz">`
/// we only check rules whose leaf selector requires `div`, `#foo`, `.bar`, or `.baz`
/// — plus the "wildcard" rules that have no tag/id/class restriction.
#[derive(Clone)]
struct RuleIndex {
    /// `by_tag[tag_discriminant]` = rule indices whose leaf selector requires that tag.
    by_tag: [Vec<usize>; TAG_COUNT],
    /// Rules whose leaf selector requires a specific ID.  Key = id string.
    by_id: BTreeMap<String, Vec<usize>>,
    /// Rules whose leaf selector requires a specific class.  Key = class string.
    by_class: BTreeMap<String, Vec<usize>>,
    /// Rules with no tag/id/class restriction (universal, attribute-only, pseudo-only).
    wildcard: Vec<usize>,
    /// Total number of rules (for bitset sizing).
    rule_count: usize,
}

#[derive(Clone)]
pub struct PreparedStylesheets {
    rules: Vec<(Rule, usize)>,
    rule_index: RuleIndex,
    layer_count: usize,
}

impl PreparedStylesheets {
    pub fn prepare(stylesheets: &[&Stylesheet], viewport_width: i32, viewport_height: i32) -> Self {
        let mut global_layer_order: Vec<String> = Vec::new();
        for sheet in stylesheets {
            for layer_name in &sheet.layer_order {
                if !global_layer_order
                    .iter()
                    .any(|existing| existing == layer_name)
                {
                    global_layer_order.push(layer_name.clone());
                }
            }
        }

        let mut rules: Vec<(Rule, usize)> = Vec::new();
        let mut order = 0usize;
        for sheet in stylesheets {
            for rule in &sheet.rules {
                let mut prepared_rule = rule.clone();
                prepared_rule.layer_index = prepared_rule.layer_name.as_ref().and_then(|name| {
                    global_layer_order
                        .iter()
                        .position(|existing| existing == name)
                });
                rules.push((prepared_rule, order));
                order += 1;
            }
            for mr in &sheet.media_rules {
                if crate::css::evaluate_media_query(&mr.query, viewport_width, viewport_height) {
                    for rule in &mr.rules {
                        let mut prepared_rule = rule.clone();
                        prepared_rule.layer_index =
                            prepared_rule.layer_name.as_ref().and_then(|name| {
                                global_layer_order
                                    .iter()
                                    .position(|existing| existing == name)
                            });
                        rules.push((prepared_rule, order));
                        order += 1;
                    }
                }
            }
        }
        let refs: Vec<(&Rule, usize)> = rules.iter().map(|(rule, order)| (rule, *order)).collect();
        let rule_index = RuleIndex::build(&refs);
        Self {
            rules,
            rule_index,
            layer_count: global_layer_order.len(),
        }
    }

    fn as_rule_refs(&self) -> Vec<(&Rule, usize)> {
        self.rules
            .iter()
            .map(|(rule, order)| (rule, *order))
            .collect()
    }
}

impl RuleIndex {
    /// Build the rule index from the collected rules.
    fn build(all_rules: &[(&Rule, usize)]) -> Self {
        const EMPTY_VEC: Vec<usize> = Vec::new();
        let mut idx = RuleIndex {
            by_tag: [EMPTY_VEC; TAG_COUNT],
            by_id: BTreeMap::new(),
            by_class: BTreeMap::new(),
            wildcard: Vec::new(),
            rule_count: all_rules.len(),
        };

        for (rule_idx, (rule, _order)) in all_rules.iter().enumerate() {
            // A rule can have multiple selectors (comma-separated).
            // We must put the rule in every bucket that any of its selectors' leaves require.
            let mut added_to_any = false;

            for sel in &rule.selectors {
                match leaf_simple(sel) {
                    Some(leaf) => {
                        // Index by the most specific leaf attribute (tag > id > class).
                        let mut indexed = false;

                        if let Some(tag) = leaf.tag {
                            let t = tag as usize;
                            if t < TAG_COUNT {
                                idx.by_tag[t].push(rule_idx);
                                indexed = true;
                            }
                        }
                        if let Some(ref id) = leaf.id {
                            push_keyed(&mut idx.by_id, id, rule_idx);
                            indexed = true;
                        }
                        for cls in &leaf.classes {
                            push_keyed(&mut idx.by_class, cls, rule_idx);
                            indexed = true;
                        }

                        if !indexed {
                            // Attribute-only or pseudo-only selector — goes to wildcard.
                            if !idx.wildcard.contains(&rule_idx) {
                                idx.wildcard.push(rule_idx);
                            }
                        }
                        added_to_any = true;
                    }
                    None => {
                        // Universal selector — matches any element.
                        if !idx.wildcard.contains(&rule_idx) {
                            idx.wildcard.push(rule_idx);
                        }
                        added_to_any = true;
                    }
                }
            }

            if !added_to_any {
                idx.wildcard.push(rule_idx);
            }
        }

        idx
    }

    /// Get candidate rule indices for a node with the given tag, id, and classes.
    /// Returns a deduplicated list of rule indices to check.
    /// Uses a bitset for O(1) deduplication instead of Vec::contains() O(n).
    fn candidates(
        &self,
        tag: Tag,
        id_attr: Option<&str>,
        class_attr: Option<&str>,
        buf: &mut Vec<usize>,
        seen: &mut Vec<u64>,
    ) {
        buf.clear();

        // Reset the bitset (one bit per rule index, packed into u64 words).
        let words_needed = (self.rule_count + 63) / 64;
        seen.clear();
        seen.resize(words_needed, 0u64);

        // Tag bucket.
        let t = tag as usize;
        if t < TAG_COUNT {
            for &ri in &self.by_tag[t] {
                let word = ri / 64;
                let bit = 1u64 << (ri % 64);
                if word < seen.len() && seen[word] & bit == 0 {
                    seen[word] |= bit;
                    buf.push(ri);
                }
            }
        }

        // ID bucket.
        if let Some(id) = id_attr {
            if let Some(indices) = keyed_bucket(&self.by_id, id) {
                for &ri in indices {
                    let word = ri / 64;
                    let bit = 1u64 << (ri % 64);
                    if word < seen.len() && seen[word] & bit == 0 {
                        seen[word] |= bit;
                        buf.push(ri);
                    }
                }
            }
        }

        // Class buckets. Match only the classes present on the node instead of
        // scanning every known class bucket.
        if let Some(classes) = class_attr {
            for cls in classes.split_ascii_whitespace() {
                if let Some(indices) = keyed_bucket(&self.by_class, cls) {
                    for &ri in indices {
                        let word = ri / 64;
                        let bit = 1u64 << (ri % 64);
                        if word < seen.len() && seen[word] & bit == 0 {
                            seen[word] |= bit;
                            buf.push(ri);
                        }
                    }
                }
            }
        }

        // Wildcard rules (always checked).
        for &ri in &self.wildcard {
            let word = ri / 64;
            let bit = 1u64 << (ri % 64);
            if word < seen.len() && seen[word] & bit == 0 {
                seen[word] |= bit;
                buf.push(ri);
            }
        }
    }
}

fn cascade_layer_key(layer_index: Option<usize>, important: bool, layer_count: usize) -> usize {
    if !important {
        layer_index.unwrap_or(layer_count)
    } else if let Some(idx) = layer_index {
        layer_count.saturating_sub(idx)
    } else {
        0
    }
}

fn sort_matches_for_phase(
    matches: &mut Vec<((u32, u32, u32), usize)>,
    all_rules: &[(&Rule, usize)],
    important: bool,
    layer_count: usize,
) {
    matches.sort_by(|a, b| {
        let (rule_a, order_a) = all_rules[a.1];
        let (rule_b, order_b) = all_rules[b.1];
        cascade_layer_key(rule_a.layer_index, important, layer_count)
            .cmp(&cascade_layer_key(
                rule_b.layer_index,
                important,
                layer_count,
            ))
            .then(a.0.cmp(&b.0))
            .then(order_a.cmp(&order_b))
    });
}

fn ensure_pseudo_style<'a>(
    slot: &'a mut Option<ComputedStyle>,
    base: &ComputedStyle,
) -> &'a mut ComputedStyle {
    if slot.is_none() {
        let mut ps = base.clone();
        ps.content = None;
        ps.content_url = None;
        ps.background_color = 0;
        ps.border_width = 0;
        ps.padding_top = 0;
        ps.padding_right = 0;
        ps.padding_bottom = 0;
        ps.padding_left = 0;
        ps.margin_top = 0;
        ps.margin_right = 0;
        ps.margin_bottom = 0;
        ps.margin_left = 0;
        ps.width = None;
        ps.height = None;
        ps.display = Display::Inline;
        *slot = Some(ps);
    }
    slot.as_mut().unwrap()
}

fn apply_pseudo_rule_matches(
    before_style: &mut Option<ComputedStyle>,
    after_style: &mut Option<ComputedStyle>,
    node_id: usize,
    styles: &[ComputedStyle],
    matches: &[((u32, u32, u32), usize)],
    all_rules: &[(&Rule, usize)],
    important: bool,
    parent_fs: i32,
    root_fs: i32,
) {
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        for sel in &rule.selectors {
            let ps = match sel.pseudo_element() {
                Some(PseudoElement::Before) => ensure_pseudo_style(before_style, &styles[node_id]),
                Some(PseudoElement::After) => ensure_pseudo_style(after_style, &styles[node_id]),
                _ => continue,
            };
            for decl in &rule.declarations {
                if decl.important == important {
                    apply_declaration(ps, decl, Some(&styles[node_id]), parent_fs, root_fs);
                }
            }
        }
    }
}

fn apply_decl_with_vars(
    style: &mut ComputedStyle,
    decl: &Declaration,
    parent_style: Option<&ComputedStyle>,
    dom: &Dom,
    node_id: usize,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
    parent_fs: i32,
    root_fs: i32,
    set_flags: &mut u32,
) {
    if let Property::CustomProperty(ref name) = decl.property {
        if let Some(val) = custom_property_value_to_string(&decl.value) {
            store_custom_prop(node_cp, name, &val);
        }
    } else if let CssValue::Var(_, _) = &decl.value {
        let resolved = resolve_var_in_decl(decl, dom, node_id, node_cp, ancestors_cp);
        *set_flags |= decl_set_flag(&resolved.property);
        apply_declaration(style, &resolved, parent_style, parent_fs, root_fs);
    } else if has_nested_var(decl) {
        let resolved = resolve_nested_var_decl(decl, dom, node_id, node_cp, ancestors_cp);
        *set_flags |= decl_set_flag(&resolved.property);
        apply_declaration(style, &resolved, parent_style, parent_fs, root_fs);
    } else {
        *set_flags |= decl_set_flag(&decl.property);
        apply_declaration(style, decl, parent_style, parent_fs, root_fs);
    }
}

fn apply_custom_props_from_decls(
    declarations: &[Declaration],
    important: bool,
    node_cp: &mut Vec<(String, String)>,
) {
    for decl in declarations {
        if decl.important != important {
            continue;
        }
        if let Property::CustomProperty(ref name) = decl.property {
            if let Some(val) = custom_property_value_to_string(&decl.value) {
                store_custom_prop(node_cp, name, &val);
            }
        }
    }
}

fn apply_decls_two_pass(
    style: &mut ComputedStyle,
    declarations: &[Declaration],
    important: bool,
    parent_style: Option<&ComputedStyle>,
    dom: &Dom,
    node_id: usize,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
    parent_fs: i32,
    root_fs: i32,
    set_flags: &mut u32,
) {
    apply_custom_props_from_decls(declarations, important, node_cp);

    for decl in declarations {
        if decl.important == important
            && matches!(decl.property, Property::FontSize)
            && !matches!(decl.property, Property::CustomProperty(_))
        {
            apply_decl_with_vars(
                style,
                decl,
                parent_style,
                dom,
                node_id,
                node_cp,
                ancestors_cp,
                parent_fs,
                root_fs,
                set_flags,
            );
        }
    }
    for decl in declarations {
        if decl.important == important
            && !matches!(
                decl.property,
                Property::FontSize | Property::CustomProperty(_)
            )
        {
            apply_decl_with_vars(
                style,
                decl,
                parent_style,
                dom,
                node_id,
                node_cp,
                ancestors_cp,
                parent_fs,
                root_fs,
                set_flags,
            );
        }
    }
}

/// Extract the leaf (rightmost) SimpleSelector from a combinator chain.
/// Returns None for Universal selectors.
fn leaf_simple(sel: &Selector) -> Option<&SimpleSelector> {
    match sel {
        Selector::Simple(s) => Some(s),
        Selector::Descendant(_, leaf)
        | Selector::Child(_, leaf)
        | Selector::AdjacentSibling(_, leaf)
        | Selector::GeneralSibling(_, leaf) => Some(leaf),
        Selector::Universal => None,
    }
}

/// Push `value` into the keyed bucket list.
fn push_keyed(buckets: &mut BTreeMap<String, Vec<usize>>, key: &str, value: usize) {
    buckets
        .entry(key.to_ascii_lowercase())
        .or_default()
        .push(value);
}

#[inline]
fn keyed_bucket<'a>(
    buckets: &'a BTreeMap<String, Vec<usize>>,
    key: &str,
) -> Option<&'a Vec<usize>> {
    let lower = key.to_ascii_lowercase();
    buckets.get(lower.as_str())
}

// ---------------------------------------------------------------------------
