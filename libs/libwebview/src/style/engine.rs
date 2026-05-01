//! Style resolution: takes a DOM tree + CSS stylesheets and computes
//! the final `ComputedStyle` for every node.
//!
//! Cascade order: initial values -> UA defaults -> author rules (by
//! specificity) -> inline styles.  Inheritable properties that are not
//! explicitly set by any declaration are inherited from the parent node.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use super::initial::default_style;
use super::lengths::{
    parse_transform_translate_component, resolve_length, set_viewport_size,
};
use super::types::*;
use crate::css::{
    AttrOp, ContainerCondition, ContainerQuery, CssValue, Declaration, Property, PseudoClass,
    PseudoElement, Rule, Selector, SimpleSelector, Stylesheet, Unit,
};
use crate::dom::{Dom, NodeId, NodeType, Tag};

// Bitflags for tracking which inheritable properties were explicitly set.
const SET_COLOR: u32 = 1 << 0;
const SET_FONT_SIZE: u32 = 1 << 1;
const SET_FONT_WEIGHT: u32 = 1 << 2;
const SET_FONT_STYLE: u32 = 1 << 3;
const SET_FONT_FAMILY: u32 = 1 << 4;
const SET_DIRECTION: u32 = 1 << 5;
const SET_TEXT_ALIGN: u32 = 1 << 6;
const SET_LINE_HEIGHT: u32 = 1 << 7;
const SET_WHITE_SPACE: u32 = 1 << 8;
const SET_LIST_STYLE: u32 = 1 << 9;
const SET_TEXT_DECO: u32 = 1 << 10;
const SET_VISIBILITY: u32 = 1 << 11;
const SET_TEXT_TRANSFORM: u32 = 1 << 12;
const SET_LETTER_SPACING: u32 = 1 << 13;
const SET_WORD_SPACING: u32 = 1 << 14;
const SET_WORD_BREAK: u32 = 1 << 15;
const SET_OVERFLOW_WRAP: u32 = 1 << 16;
const SET_LIST_STYLE_POS: u32 = 1 << 17;
const SET_ACCENT_COLOR: u32 = 1 << 18;
const SET_COLOR_SCHEME: u32 = 1 << 19;
const SET_WRITING_MODE: u32 = 1 << 20;

/// User-agent stylesheet: hardcoded browser defaults per HTML tag.
/// Returns the base style AND a bitfield indicating which inheritable
/// properties the UA explicitly sets (so inheritance does not clobber them).
fn ua_style_and_flags(tag: Tag) -> (ComputedStyle, u32) {
    let mut s = default_style();
    let mut flags: u32 = 0;
    match tag {
        Tag::Body => {
            s.margin_top = 8;
            s.margin_right = 8;
            s.margin_bottom = 8;
            s.margin_left = 8;
        }
        Tag::H1 => {
            s.font_size = 32;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 21;
            s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H2 => {
            s.font_size = 24;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 19;
            s.margin_bottom = 19;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H3 => {
            s.font_size = 19;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 18;
            s.margin_bottom = 18;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H4 => {
            s.font_size = 16;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 21;
            s.margin_bottom = 21;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H5 => {
            s.font_size = 13;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 22;
            s.margin_bottom = 22;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::H6 => {
            s.font_size = 11;
            s.font_weight = FontWeight::Bold;
            s.margin_top = 24;
            s.margin_bottom = 24;
            flags |= SET_FONT_SIZE | SET_FONT_WEIGHT;
        }
        Tag::P => {
            s.margin_top = 16;
            s.margin_bottom = 16;
        }
        Tag::A => {
            s.display = Display::Inline;
            s.color = 0xFF007AFF;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_COLOR | SET_TEXT_DECO;
        }
        Tag::Em | Tag::I => {
            s.display = Display::Inline;
            s.font_style = FontStyleVal::Italic;
            flags |= SET_FONT_STYLE;
        }
        Tag::Strong | Tag::B => {
            s.display = Display::Inline;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::U => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_TEXT_DECO;
        }
        Tag::Code => {
            s.display = Display::Inline;
        }
        Tag::Pre => {
            s.white_space = WhiteSpace::Pre;
            flags |= SET_WHITE_SPACE;
        }
        Tag::Blockquote => {
            s.margin_left = 40;
        }
        Tag::Ul => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.padding_left = 40;
            // UA list-style: disc is inherited by <li> children.
            // Setting the flag here prevents <ul> from inheriting list-style from its
            // ancestors; <li> children inherit from <ul> because <li> has no flag.
            s.list_style = ListStyle::Disc;
            flags |= SET_LIST_STYLE;
        }
        Tag::Ol => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.padding_left = 40;
            s.list_style = ListStyle::Decimal;
            flags |= SET_LIST_STYLE;
        }
        Tag::Li => {
            s.display = Display::ListItem;
            // No SET_LIST_STYLE flag: <li> inherits list-style from its parent (<ul>/<ol>).
            // This allows `list-style: none` on the parent to propagate via CSS inheritance.
            s.list_style = ListStyle::Disc; // fallback if orphan (no <ul>/<ol> parent)
        }
        Tag::Hr => {
            s.border_width = 1;
            s.margin_top = 8;
            s.margin_bottom = 8;
        }
        Tag::Img | Tag::Picture | Tag::Br | Tag::Span | Tag::Label => {
            s.display = Display::Inline;
        }
        Tag::Input | Tag::Button | Tag::Select | Tag::Textarea => {
            s.display = Display::Inline;
        }
        Tag::Table => {}
        Tag::Tr => {
            s.display = Display::TableRow;
        }
        Tag::Td => {
            s.display = Display::TableCell;
        }
        Tag::Th => {
            s.display = Display::TableCell;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Head
        | Tag::Title
        | Tag::Meta
        | Tag::Link
        | Tag::Style
        | Tag::Script
        | Tag::Noscript
        | Tag::Template => {
            s.display = Display::None;
        }
        // Inline semantic text elements
        Tag::Small => {
            s.display = Display::Inline;
            s.font_size = 13;
            flags |= SET_FONT_SIZE;
        }
        Tag::S | Tag::Del => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::LineThrough;
            flags |= SET_TEXT_DECO;
        }
        Tag::Ins => {
            s.display = Display::Inline;
            s.text_decoration = TextDeco::Underline;
            flags |= SET_TEXT_DECO;
        }
        Tag::Mark => {
            s.display = Display::Inline;
            s.background_color = 0xFFFFFF00; // yellow highlight
            s.color = 0xFF000000;
            flags |= SET_COLOR;
        }
        Tag::Sub
        | Tag::Sup
        | Tag::Kbd
        | Tag::Samp
        | Tag::Var
        | Tag::Abbr
        | Tag::Cite
        | Tag::Dfn
        | Tag::Q
        | Tag::Time
        | Tag::Bdi
        | Tag::Bdo
        | Tag::Data
        | Tag::Ruby
        | Tag::Rt
        | Tag::Rp
        | Tag::Wbr
        | Tag::Nobr
        | Tag::Tt => {
            s.display = Display::Inline;
        }
        // Definition list
        Tag::Dl => {
            s.margin_top = 16;
            s.margin_bottom = 16;
        }
        Tag::Dt => {
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Dd => {
            s.margin_left = 40;
        }
        // Figure
        Tag::Figure => {
            s.margin_top = 16;
            s.margin_bottom = 16;
            s.margin_left = 40;
            s.margin_right = 40;
        }
        Tag::Figcaption => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        // Details/Summary
        Tag::Details => {}
        Tag::Summary => {
            s.display = Display::Block;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        // Dialog
        Tag::Dialog => {
            s.display = Display::Block;
            s.position = Position::Absolute;
        }
        // Sectioning
        Tag::Aside | Tag::Hgroup | Tag::Address => {}
        // Table extensions
        Tag::Tfoot => {
            s.display = Display::TableRow;
        }
        Tag::Caption => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        // Form elements
        Tag::Fieldset => {
            s.border_width = 1;
            s.padding_top = 8;
            s.padding_right = 12;
            s.padding_bottom = 8;
            s.padding_left = 12;
        }
        Tag::Legend => {
            s.display = Display::Inline;
            s.font_weight = FontWeight::Bold;
            flags |= SET_FONT_WEIGHT;
        }
        Tag::Optgroup => {}
        Tag::Datalist | Tag::Output => {
            s.display = Display::Inline;
        }
        Tag::Progress | Tag::Meter => {
            s.display = Display::Inline;
        }
        // Deprecated
        Tag::Center => {
            s.text_align = TextAlignVal::Center;
            flags |= SET_TEXT_ALIGN;
        }
        Tag::Font => {
            s.display = Display::Inline;
        }
        // Block-level elements that just use defaults.
        Tag::Div
        | Tag::Section
        | Tag::Article
        | Tag::Header
        | Tag::Footer
        | Tag::Nav
        | Tag::Main
        | Tag::Form
        | Tag::Thead
        | Tag::Tbody => {}
        // Custom/unknown elements (Web Components etc.) default to inline per HTML spec.
        // CSS can always override with display:block/flex/grid as needed.
        Tag::Unknown => {
            s.display = Display::Inline;
        }
        _ => {}
    }
    (s, flags)
}

/// Public convenience: returns only the `ComputedStyle` (no flags).
pub fn user_agent_styles(tag: Tag) -> ComputedStyle {
    ua_style_and_flags(tag).0
}

// ---------------------------------------------------------------------------
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
            && !matches!(decl.property, Property::FontSize | Property::CustomProperty(_))
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
        PseudoClass::Hover => selector_state.hovered_node == Some(node_id),
        PseudoClass::Active => selector_state.active_node == Some(node_id),
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
// Resolve styles for entire DOM
// ---------------------------------------------------------------------------

/// Compute the final resolved style for every node in the DOM.
/// Returns a `Vec<ComputedStyle>` indexed by `NodeId`.
pub fn resolve_styles(
    dom: &Dom,
    stylesheets: &[&Stylesheet],
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    let prepared = PreparedStylesheets::prepare(stylesheets, viewport_width, viewport_height);
    resolve_styles_prepared_with_state(
        dom,
        &prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_with_state(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared_with_state(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_impl(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        None,
        selector_state,
    )
}

pub fn resolve_styles_prepared_budgeted(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: usize,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_budgeted_with_state(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        node_budget,
        &SelectorState::default(),
    )
}

pub fn resolve_styles_prepared_budgeted_with_state(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: usize,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    resolve_styles_prepared_impl(
        dom,
        prepared,
        viewport_width,
        viewport_height,
        inline_style_cache,
        Some(node_budget),
        selector_state,
    )
}

fn resolve_styles_prepared_impl(
    dom: &Dom,
    prepared: &PreparedStylesheets,
    viewport_width: i32,
    viewport_height: i32,
    inline_style_cache: &mut Vec<Option<Vec<Declaration>>>,
    node_budget: Option<usize>,
    selector_state: &SelectorState,
) -> (Vec<ComputedStyle>, PseudoStyles) {
    set_viewport_size(viewport_width, viewport_height);

    let count = dom.nodes.len();
    let resolved_count = node_budget.unwrap_or(count).min(count);
    let budgeted = resolved_count < count;
    if inline_style_cache.len() < resolved_count {
        inline_style_cache.resize_with(resolved_count, || None);
    }
    crate::debug_surf!(
        "[style] resolve_styles: {} nodes, {} stylesheets, resolved_count={}, budgeted={}",
        count,
        prepared.rules.len(),
        resolved_count,
        budgeted
    );
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[style]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );

    let mut styles: Vec<ComputedStyle> = Vec::with_capacity(resolved_count.max(1));
    let root_font_size: i32 = 16;

    let all_rules = prepared.as_rule_refs();
    crate::debug_surf!(
        "[style] collected {} applicable rules (once)",
        all_rules.len()
    );

    crate::debug_surf!(
        "[style] rule index: {} wildcard, {} id-buckets, {} class-buckets",
        prepared.rule_index.wildcard.len(),
        prepared.rule_index.by_id.len(),
        prepared.rule_index.by_class.len()
    );

    // Reusable scratch buffers for per-node matching (avoids repeated alloc/free).
    let mut matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(64);
    let mut candidates: Vec<usize> = Vec::with_capacity(128);
    let mut seen_bitset: Vec<u64> = Vec::with_capacity((all_rules.len() + 63) / 64);

    // Separate storage for custom properties (--name: value).
    // Only nodes that DEFINE custom properties have non-empty entries.
    // var() references are resolved on-demand by walking the DOM parent chain,
    // eliminating the per-node clone that caused heap-stack collision on large
    // pages (~54 MiB for chip.de's 6228 nodes).
    let mut custom_props: Vec<Vec<(String, String)>> = vec![Vec::new(); resolved_count];
    let mut set_flags_by_node: Vec<u32> = Vec::with_capacity(resolved_count.max(1));
    let mut active_containers: Vec<ActiveContainer> = Vec::new();

    for id in 0..resolved_count {
        #[cfg(feature = "debug_surf")]
        {
            if id < 5 || id % 1000 == 0 {
                crate::debug_surf!(
                    "[style] node {}/{} RSP=0x{:X} heap=0x{:X}",
                    id,
                    count,
                    crate::debug_rsp(),
                    crate::debug_heap_pos()
                );
            }
        }

        let node = &dom.nodes[id];
        while let Some(container) = active_containers.last() {
            if is_ancestor_or_self(dom, container.node_id, id) {
                break;
            }
            active_containers.pop();
        }
        let parent_fs = node
            .parent
            .map_or(16, |pid| if pid < id { styles[pid].font_size } else { 16 });
        let available_inline_size = active_containers
            .last()
            .map(|container| container.inline_size)
            .unwrap_or(viewport_width);
        let available_block_size = active_containers
            .last()
            .map(|container| container.block_size)
            .unwrap_or(viewport_height);
        let parent_style = node.parent.and_then(|pid| styles.get(pid));

        // Phase 1: Start from UA defaults (elements) or initial values (text).
        let (mut style, mut set_flags) = match &node.node_type {
            NodeType::Element { tag, .. } => ua_style_and_flags(*tag),
            NodeType::Text(_) => {
                let mut s = default_style();
                s.display = Display::Inline;
                (s, 0u32)
            }
        };

        // Seed inheritable runtime-critical values early so relative units and
        // currentColor-dependent properties see the same baseline that later
        // inheritance will preserve.
        if let Some(parent) = parent_style {
            if set_flags & SET_FONT_SIZE == 0 {
                style.font_size = parent.font_size;
            }
            if set_flags & SET_COLOR == 0 {
                style.color = parent.color;
            }
            if set_flags & SET_FONT_WEIGHT == 0 {
                style.font_weight = parent.font_weight;
            }
            if set_flags & SET_FONT_STYLE == 0 {
                style.font_style = parent.font_style;
            }
            if set_flags & SET_DIRECTION == 0 {
                style.direction = parent.direction;
            }
            if set_flags & SET_TEXT_ALIGN == 0 {
                style.text_align = parent.text_align;
            }
            if set_flags & SET_WHITE_SPACE == 0 {
                style.white_space = parent.white_space;
            }
            if set_flags & SET_TEXT_TRANSFORM == 0 {
                style.text_transform = parent.text_transform;
            }
            if set_flags & SET_LETTER_SPACING == 0 {
                style.letter_spacing = parent.letter_spacing;
            }
            if set_flags & SET_WORD_SPACING == 0 {
                style.word_spacing = parent.word_spacing;
            }
            if set_flags & SET_WORD_BREAK == 0 {
                style.word_break = parent.word_break;
            }
            if set_flags & SET_OVERFLOW_WRAP == 0 {
                style.overflow_wrap = parent.overflow_wrap;
            }
        }

        // UA override: [hidden] → display:none (per HTML spec).
        if let NodeType::Element { attrs, .. } = &node.node_type {
            if attrs
                .iter()
                .any(|a| eq_ignore_ascii_case(&a.name, "hidden"))
            {
                style.display = Display::None;
            }
        }

        // Common accessibility utility classes keep text available to screen
        // readers while moving it out of visual layout. Treat them as hidden
        // for the visual renderer so labels and page titles do not leak.
        if element_has_screen_reader_only_class(dom, id) {
            style.display = Display::None;
        }

        // Heuristic for component-driven collapse UIs: collapsed content is
        // usually materialized as a `data-collapse-target` subtree and only
        // becomes visible when an ancestor gains `is-open`.
        if element_has_attr(dom, id, "data-collapse-target")
            && !ancestor_has_attr(dom, id, "is-open")
        {
            style.display = Display::None;
        }

        // UA override: <input type="hidden"> → display:none (per HTML spec).
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            if *tag == Tag::Input {
                if attrs.iter().any(|a| {
                    eq_ignore_ascii_case(&a.name, "type")
                        && eq_ignore_ascii_case(&a.value, "hidden")
                }) {
                    style.display = Display::None;
                }
            }
        }

        // Phase 1b: Presentational hints from HTML attributes (specificity 0,
        // per HTML spec §15.3.3). Applied after UA styles but before author rules.
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            // `align` attribute on div, main, nav, header, footer, section,
            // article, aside, hgroup, address, center, p, h1-h6, blockquote,
            // figure, figcaption, details, summary, dialog, search.
            // Maps to text-align (HTML spec §15.3.3).
            let supports_align = matches!(
                tag,
                Tag::Div
                    | Tag::Main
                    | Tag::Nav
                    | Tag::Header
                    | Tag::Footer
                    | Tag::Section
                    | Tag::Article
                    | Tag::Aside
                    | Tag::Hgroup
                    | Tag::Address
                    | Tag::Center
                    | Tag::P
                    | Tag::H1
                    | Tag::H2
                    | Tag::H3
                    | Tag::H4
                    | Tag::H5
                    | Tag::H6
                    | Tag::Blockquote
                    | Tag::Figure
                    | Tag::Figcaption
                    | Tag::Details
                    | Tag::Summary
                    | Tag::Dialog
                    | Tag::Search
                    | Tag::Td
                    | Tag::Th
            );
            if supports_align {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "align") {
                        let val = a.value.trim();
                        let align = if val.eq_ignore_ascii_case("left") {
                            Some(TextAlignVal::Left)
                        } else if val.eq_ignore_ascii_case("right") {
                            Some(TextAlignVal::Right)
                        } else if val.eq_ignore_ascii_case("center") {
                            Some(TextAlignVal::Center)
                        } else if val.eq_ignore_ascii_case("justify") {
                            Some(TextAlignVal::Justify)
                        } else {
                            None
                        };
                        if let Some(ta) = align {
                            style.text_align = ta;
                            set_flags |= SET_TEXT_ALIGN;
                        }
                        break;
                    }
                }
            }
        }

        // Phase 2 + 3: Apply author rules and inline styles.
        // Custom property declarations are stored in custom_props[id].
        // var() references are resolved by walking the parent chain.
        if matches!(node.node_type, NodeType::Element { .. }) {
            let (ancestors_cp, current_and_rest) = custom_props.split_at_mut(id);
            let node_cp = &mut current_and_rest[0];

            set_flags |= apply_author_rules(
                &mut style,
                parent_style,
                dom,
                id,
                &all_rules,
                prepared.layer_count,
                &prepared.rule_index,
                &mut candidates,
                &mut seen_bitset,
                &mut matches,
                parent_fs,
                root_font_size,
                node_cp,
                ancestors_cp,
                selector_state,
                &active_containers,
            );

            if let NodeType::Element { attrs, .. } = &node.node_type {
                apply_tailwind_display_fallback(&mut style, attrs, viewport_width);
                apply_tailwind_spacing_fallback(&mut style, attrs);
                apply_flexbox_grid_column_fallback(&mut style, attrs, viewport_width);
            }

            // Phase 3: Apply inline styles (highest specificity).
            // Uses a cache to avoid re-parsing style="..." on every relayout.
            if let NodeType::Element { attrs, .. } = &node.node_type {
                for a in attrs {
                    if eq_ignore_ascii_case(&a.name, "style") {
                        // Look up cached declarations for this node, or parse and cache.
                        if inline_style_cache[id].is_none() {
                            inline_style_cache[id] = Some(crate::css::parse_inline_style(&a.value));
                        }
                        let inline_decls = inline_style_cache[id].as_ref().unwrap();
                        apply_custom_props_from_decls(inline_decls, false, node_cp);
                        apply_custom_props_from_decls(inline_decls, true, node_cp);
                        apply_decls_two_pass(
                            &mut style,
                            inline_decls,
                            false,
                            parent_style,
                            dom,
                            id,
                            node_cp,
                            ancestors_cp,
                            parent_fs,
                            root_font_size,
                            &mut set_flags,
                        );
                        apply_decls_two_pass(
                            &mut style,
                            inline_decls,
                            true,
                            parent_style,
                            dom,
                            id,
                            node_cp,
                            ancestors_cp,
                            parent_fs,
                            root_font_size,
                            &mut set_flags,
                        );
                        break;
                    }
                }
            }
        }

        // HTML <dialog> elements are not rendered unless the `open` attribute is
        // present. Many sites keep modal templates in the DOM and rely on this
        // behavior; painting them breaks the page under a full-screen backdrop.
        if let NodeType::Element { tag, attrs, .. } = &node.node_type {
            if *tag == Tag::Dialog
                && attrs
                    .iter()
                    .all(|a| !eq_ignore_ascii_case(&a.name, "open"))
            {
                style.display = Display::None;
            }

            if is_unfocused_skip_link(*tag, attrs, id, selector_state) {
                apply_visually_hidden_style(&mut style);
            } else if selector_state.focused_node != Some(id)
                && attrs.iter().any(|a| {
                    eq_ignore_ascii_case(&a.name, "class")
                        && has_visually_hidden_class(&a.value)
                })
            {
                apply_visually_hidden_style(&mut style);
            }
        }

        fn apply_visually_hidden_style(style: &mut ComputedStyle) {
                style.position = Position::Absolute;
                style.left_offset = Some(-10000);
                style.top = Some(0);
                style.width = Some(1);
                style.height = Some(1);
                style.min_width = 0;
                style.min_height = 0;
                style.padding_top = 0;
                style.padding_right = 0;
                style.padding_bottom = 0;
                style.padding_left = 0;
                style.overflow_x = OverflowVal::Hidden;
                style.overflow_y = OverflowVal::Hidden;
                style.clip_rect = Some([0, 0, 0, 0]);
        }

        // (Phase 3b removed: custom properties are resolved on-demand via
        // parent chain walk, eliminating the per-node clone that caused
        // heap-stack collision on large pages.)

        // Phase 4: Inherit inheritable properties NOT explicitly set.
        if let Some(pid) = node.parent {
            if pid < id {
                inherit_unset(&mut style, &styles[pid], set_flags);
            }
        }

        // Phase 5: Resolve `li` list_style from parent (ol -> decimal).
        if let NodeType::Element { tag: Tag::Li, .. } = &node.node_type {
            if set_flags & SET_LIST_STYLE != 0 && style.list_style == ListStyle::Disc {
                if let Some(pid) = node.parent {
                    if dom.tag(pid) == Some(Tag::Ol) {
                        style.list_style = ListStyle::Decimal;
                    }
                }
            }
        }

        // Phase 6: Resolve auto line_height.
        // Important: an explicitly specified `line-height: 0` is valid CSS and
        // must not be treated as "unset/auto".
        if set_flags & SET_LINE_HEIGHT == 0 && style.line_height == 0 {
            style.line_height = (style.font_size * 6 + 2) / 5;
        }

        set_flags_by_node.push(set_flags);
        styles.push(style);

        if matches!(
            styles[id].container_type,
            ContainerTypeVal::InlineSize | ContainerTypeVal::Size
        ) {
            active_containers.push(ActiveContainer {
                node_id: id,
                names: styles[id].container_names.clone(),
                inline_size: estimated_content_inline_size(&styles[id], available_inline_size),
                block_size: estimated_content_block_size(&styles[id], available_block_size),
            });
        }
    }

    if budgeted {
        let mut hidden = default_style();
        hidden.display = Display::None;
        styles.resize(count, hidden);
        set_flags_by_node.resize(count, 0);
    }

    // JS-created virtual DOM nodes are materialized from React-style createElement
    // trees, where child text nodes can receive real DOM ids before their parent
    // element does.  The main resolver walks numeric node ids for performance, so
    // `pid < id` inheritance is not guaranteed for those nodes.  A cheap DOM-order
    // inheritance pass makes inheritable properties (especially color/font) match
    // the actual tree, while preserving properties explicitly set on the child.
    inherit_unset_in_dom_order(dom, &mut styles, &set_flags_by_node);

    crate::debug_surf!("[style] resolve_styles done: {} styles", styles.len());
    #[cfg(feature = "debug_surf")]
    crate::debug_surf!(
        "[style]   RSP=0x{:X} heap=0x{:X}",
        crate::debug_rsp(),
        crate::debug_heap_pos()
    );

    // ── Phase 7: Resolve ::before/::after pseudo-element styles ──
    let mut pseudo = PseudoStyles::empty(count);
    let mut pseudo_candidates: Vec<usize> = Vec::with_capacity(128);
    let mut pseudo_seen_bitset: Vec<u64> = Vec::with_capacity((all_rules.len() + 63) / 64);
    let mut pseudo_matches: Vec<((u32, u32, u32), usize)> = Vec::with_capacity(32);
    let mut pseudo_active_containers: Vec<ActiveContainer> = Vec::new();
    for id in 0..resolved_count {
        while let Some(container) = pseudo_active_containers.last() {
            if is_ancestor_or_self(dom, container.node_id, id) {
                break;
            }
            pseudo_active_containers.pop();
        }
        let node = &dom.nodes[id];
        let (tag, attrs) = match &node.node_type {
            NodeType::Element { tag, attrs } => (*tag, attrs),
            _ => continue,
        };
        let id_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, "id"))
            .map(|a| a.value.as_str());
        let class_attr = attrs
            .iter()
            .find(|a| eq_ignore_ascii_case(&a.name, "class"))
            .map(|a| a.value.as_str());
        prepared.rule_index.candidates(
            tag,
            id_attr,
            class_attr,
            &mut pseudo_candidates,
            &mut pseudo_seen_bitset,
        );
        pseudo_matches.clear();
        for &rule_idx in pseudo_candidates.iter() {
            let (rule, _order) = all_rules[rule_idx];
            if let Some(ref query) = rule.container_query {
                if !container_query_matches(query, &pseudo_active_containers) {
                    continue;
                }
            }
            for sel in &rule.selectors {
                let pe = sel.pseudo_element();
                if pe.is_none() {
                    continue;
                }
                // Check if the base selector (without pseudo-element) matches this node.
                if !selector_matches_base(sel, dom, id, selector_state) {
                    continue;
                }
                pseudo_matches.push((sel.specificity(), rule_idx));
                break;
            }
        }
        let parent_fs = styles[id].font_size;
        let root_fs = 16;
        let mut before_style: Option<ComputedStyle> = None;
        let mut after_style: Option<ComputedStyle> = None;
        sort_matches_for_phase(&mut pseudo_matches, &all_rules, false, prepared.layer_count);
        apply_pseudo_rule_matches(
            &mut before_style,
            &mut after_style,
            id,
            &styles,
            &pseudo_matches,
            &all_rules,
            false,
            parent_fs,
            root_fs,
        );
        sort_matches_for_phase(&mut pseudo_matches, &all_rules, true, prepared.layer_count);
        apply_pseudo_rule_matches(
            &mut before_style,
            &mut after_style,
            id,
            &styles,
            &pseudo_matches,
            &all_rules,
            true,
            parent_fs,
            root_fs,
        );
        pseudo.before[id] = before_style;
        pseudo.after[id] = after_style;
        // Resolve line_height for pseudo styles.
        if let Some(ref mut ps) = pseudo.before[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            // Only keep if content is set and non-empty.
            if ps.content.is_none() {
                pseudo.before[id] = None;
            }
        }
        if let Some(ref mut ps) = pseudo.after[id] {
            if ps.line_height == 0 {
                ps.line_height = (ps.font_size * 6 + 2) / 5;
            }
            if ps.content.is_none() {
                pseudo.after[id] = None;
            }
        }
        if matches!(
            styles[id].container_type,
            ContainerTypeVal::InlineSize | ContainerTypeVal::Size
        ) {
            let available_inline_size = pseudo_active_containers
                .last()
                .map(|container| container.inline_size)
                .unwrap_or(viewport_width);
            let available_block_size = pseudo_active_containers
                .last()
                .map(|container| container.block_size)
                .unwrap_or(viewport_height);
            pseudo_active_containers.push(ActiveContainer {
                node_id: id,
                names: styles[id].container_names.clone(),
                inline_size: estimated_content_inline_size(&styles[id], available_inline_size),
                block_size: estimated_content_block_size(&styles[id], available_block_size),
            });
        }
    }

    (styles, pseudo)
}

fn apply_author_rules(
    style: &mut ComputedStyle,
    parent_style: Option<&ComputedStyle>,
    dom: &Dom,
    node_id: NodeId,
    all_rules: &[(&Rule, usize)],
    layer_count: usize,
    rule_index: &RuleIndex,
    candidates: &mut Vec<usize>,
    seen_bitset: &mut Vec<u64>,
    matches: &mut Vec<((u32, u32, u32), usize)>,
    parent_fs: i32,
    root_fs: i32,
    node_cp: &mut Vec<(String, String)>,
    ancestors_cp: &[Vec<(String, String)>],
    selector_state: &SelectorState,
    active_containers: &[ActiveContainer],
) -> u32 {
    // Reuse the caller's matches buffer (avoids alloc/free per node).
    matches.clear();

    // Use the rule index to get only candidate rules for this node's tag/id/classes.
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        _ => return 0,
    };
    let id_attr = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "id"))
        .map(|a| a.value.as_str());
    let class_attr = attrs
        .iter()
        .find(|a| eq_ignore_ascii_case(&a.name, "class"))
        .map(|a| a.value.as_str());

    rule_index.candidates(*tag, id_attr, class_attr, candidates, seen_bitset);

    for &idx in candidates.iter() {
        let (rule, _order) = all_rules[idx];
        if let Some(ref query) = rule.container_query {
            if !container_query_matches(query, active_containers) {
                continue;
            }
        }
        for sel in &rule.selectors {
            if selector_matches(sel, dom, node_id, selector_state) {
                matches.push((sel.specificity(), idx));
                break;
            }
        }
    }

    let mut set_flags: u32 = 0;

    // Resolve custom properties before dependent declarations. Utility-first
    // CSS (Tailwind 4 etc.) often emits `background-image: var(...)` before
    // the utility rules that define the gradient stops for the same element.
    sort_matches_for_phase(matches, all_rules, false, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_custom_props_from_decls(&rule.declarations, false, node_cp);
    }
    sort_matches_for_phase(matches, all_rules, true, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_custom_props_from_decls(&rule.declarations, true, node_cp);
    }

    // Phase 1: Apply normal (non-!important) declarations.
    sort_matches_for_phase(matches, all_rules, false, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_decls_two_pass(
            style,
            &rule.declarations,
            false,
            parent_style,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            parent_fs,
            root_fs,
            &mut set_flags,
        );
    }

    // Phase 2: Apply !important declarations (override normal ones).
    sort_matches_for_phase(matches, all_rules, true, layer_count);
    for &(_, idx) in matches.iter() {
        let (rule, _) = all_rules[idx];
        apply_decls_two_pass(
            style,
            &rule.declarations,
            true,
            parent_style,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            parent_fs,
            root_fs,
            &mut set_flags,
        );
    }

    set_flags
}

fn apply_tailwind_display_fallback(
    style: &mut ComputedStyle,
    attrs: &[crate::dom::Attr],
    viewport_width: i32,
) {
    let Some(class_attr) = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "class")) else {
        return;
    };

    let mut base_display = None;
    let mut sm_display = None;
    let mut md_display = None;
    let mut lg_display = None;
    let mut xl_display = None;
    let mut xxl_display = None;
    let mut max_sm_display = None;
    let mut max_md_display = None;
    let mut max_lg_display = None;
    let mut max_xl_display = None;

    for class_name in class_attr.value.split_ascii_whitespace() {
        let mut matched_prefix = None;
        let mut utility = class_name;
        if let Some((prefix, rest)) = class_name.split_once(':') {
            matched_prefix = Some(prefix);
            utility = rest.rsplit(':').next().unwrap_or(rest);
        }
        let Some(display) = tailwind_display_utility(utility) else {
            continue;
        };
        match matched_prefix {
            Some("sm") => sm_display = Some(display),
            Some("md") => md_display = Some(display),
            Some("lg") => lg_display = Some(display),
            Some("xl") => xl_display = Some(display),
            Some("2xl") => xxl_display = Some(display),
            Some("max-sm") => max_sm_display = Some(display),
            Some("max-md") => max_md_display = Some(display),
            Some("max-lg") => max_lg_display = Some(display),
            Some("max-xl") => max_xl_display = Some(display),
            None => base_display = Some(display),
            _ => {}
        }
    }

    let mut display = base_display;
    if viewport_width >= 640 {
        display = sm_display.or(display);
    } else {
        display = max_sm_display.or(display);
    }
    if viewport_width >= 768 {
        display = md_display.or(display);
    } else {
        display = max_md_display.or(display);
    }
    if viewport_width >= 1024 {
        display = lg_display.or(display);
    } else {
        display = max_lg_display.or(display);
    }
    if viewport_width >= 1280 {
        display = xl_display.or(display);
    } else {
        display = max_xl_display.or(display);
    }
    if viewport_width >= 1536 {
        display = xxl_display.or(display);
    }
    if let Some(display) = display {
        style.display = display;
    }
}

fn tailwind_display_utility(class_name: &str) -> Option<Display> {
    match class_name {
        "hidden" => Some(Display::None),
        "block" => Some(Display::Block),
        "inline" => Some(Display::Inline),
        "inline-block" => Some(Display::InlineBlock),
        "flex" => Some(Display::Flex),
        "inline-flex" => Some(Display::InlineFlex),
        "grid" => Some(Display::Grid),
        "inline-grid" => Some(Display::InlineGrid),
        "flow-root" => Some(Display::FlowRoot),
        "contents" => Some(Display::Contents),
        _ => None,
    }
}

fn apply_tailwind_spacing_fallback(style: &mut ComputedStyle, attrs: &[crate::dom::Attr]) {
    let Some(class_attr) = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "class")) else {
        return;
    };

    for class_name in class_attr.value.split_ascii_whitespace() {
        let utility = class_name.rsplit(':').next().unwrap_or(class_name);

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-")) {
            style.row_gap = px;
            style.column_gap = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-x-")) {
            style.column_gap = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("gap-y-")) {
            style.row_gap = px;
            continue;
        }

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("p-")) {
            style.padding_top = px;
            style.padding_right = px;
            style.padding_bottom = px;
            style.padding_left = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("px-")) {
            style.padding_left = px;
            style.padding_right = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("py-")) {
            style.padding_top = px;
            style.padding_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pt-")) {
            style.padding_top = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pr-")) {
            style.padding_right = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pb-")) {
            style.padding_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("pl-")) {
            style.padding_left = px;
            continue;
        }

        if utility == "mx-auto" {
            style.margin_left_auto = true;
            style.margin_right_auto = true;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("m-")) {
            style.margin_top = px;
            style.margin_right = px;
            style.margin_bottom = px;
            style.margin_left = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mx-")) {
            style.margin_left = px;
            style.margin_right = px;
            style.margin_left_auto = false;
            style.margin_right_auto = false;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("my-")) {
            style.margin_top = px;
            style.margin_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mt-")) {
            style.margin_top = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mr-")) {
            style.margin_right = px;
            style.margin_right_auto = false;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("mb-")) {
            style.margin_bottom = px;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("ml-")) {
            style.margin_left = px;
            style.margin_left_auto = false;
            continue;
        }

        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("w-")) {
            style.width = Some(px);
            style.width_pct = None;
            style.width_calc = None;
            continue;
        }
        if let Some(px) = tailwind_spacing_value(utility.strip_prefix("h-")) {
            style.height = Some(px);
            style.height_pct = None;
            style.height_calc = None;
            continue;
        }
        if let Some(px) = tailwind_max_width_value(utility) {
            style.max_width = Some(px);
            style.max_width_calc = None;
        }
    }
}

fn tailwind_spacing_value(rest: Option<&str>) -> Option<i32> {
    let rest = rest?;
    match rest {
        "px" => return Some(1),
        "0" => return Some(0),
        "0.5" => return Some(2),
        "1" => return Some(4),
        "1.5" => return Some(6),
        "2" => return Some(8),
        "2.5" => return Some(10),
        "3" => return Some(12),
        "3.5" => return Some(14),
        "4" => return Some(16),
        "5" => return Some(20),
        "6" => return Some(24),
        "7" => return Some(28),
        "8" => return Some(32),
        "9" => return Some(36),
        "10" => return Some(40),
        "11" => return Some(44),
        "12" => return Some(48),
        "14" => return Some(56),
        "16" => return Some(64),
        "20" => return Some(80),
        "24" => return Some(96),
        "28" => return Some(112),
        "32" => return Some(128),
        _ => {}
    }
    None
}

fn tailwind_max_width_value(class_name: &str) -> Option<i32> {
    match class_name {
        "max-w-xs" => Some(320),
        "max-w-sm" => Some(384),
        "max-w-md" => Some(448),
        "max-w-lg" => Some(512),
        "max-w-xl" => Some(576),
        "max-w-2xl" => Some(672),
        "max-w-3xl" => Some(768),
        "max-w-4xl" => Some(896),
        "max-w-5xl" => Some(1024),
        "max-w-6xl" => Some(1152),
        "max-w-7xl" => Some(1280),
        _ => None,
    }
}

fn apply_flexbox_grid_column_fallback(
    style: &mut ComputedStyle,
    attrs: &[crate::dom::Attr],
    viewport_width: i32,
) {
    let Some(class_attr) = attrs.iter().find(|a| eq_ignore_ascii_case(&a.name, "class")) else {
        return;
    };

    let mut base: Option<i32> = None;
    let mut sm: Option<i32> = None;
    let mut md: Option<i32> = None;
    let mut lg: Option<i32> = None;
    let mut xl: Option<i32> = None;

    for class_name in class_attr.value.split_ascii_whitespace() {
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-") {
            base = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-sm-") {
            sm = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-md-") {
            md = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-lg-") {
            lg = Some(cols);
            continue;
        }
        if let Some(cols) = parse_fb_col_class(class_name, "fb-col-xl-") {
            xl = Some(cols);
        }
    }

    let mut cols = base;
    if viewport_width >= 660 {
        cols = sm.or(md).or(cols);
    }
    if viewport_width >= 1000 {
        cols = lg.or(cols);
    }
    if viewport_width >= 1200 {
        cols = xl.or(cols);
    }

    let Some(cols) = cols else {
        return;
    };
    let pct = ((cols.clamp(1, 12) as i64 * 10000 + 6) / 12) as i32;
    style.width = None;
    style.width_calc = None;
    style.width_pct = Some(pct);
}

fn parse_fb_col_class(class_name: &str, prefix: &str) -> Option<i32> {
    let rest = class_name.strip_prefix(prefix)?;
    if rest.is_empty() || rest.as_bytes().iter().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    let cols = rest.parse::<i32>().ok()?;
    if (1..=12).contains(&cols) {
        Some(cols)
    } else {
        None
    }
}

/// Store a custom property in a node's custom property list.
fn store_custom_prop(cp: &mut Vec<(String, String)>, name: &str, val: &str) {
    if let Some(existing) = cp.iter_mut().find(|(k, _)| k == name) {
        existing.1.clear();
        existing.1.push_str(val);
    } else {
        cp.push((String::from(name), String::from(val)));
    }
}

fn custom_property_value_to_string(value: &CssValue) -> Option<String> {
    match value {
        CssValue::Keyword(s) => Some(s.clone()),
        CssValue::Color(c) => {
            let a = (c >> 24) & 0xff;
            let r = (c >> 16) & 0xff;
            let g = (c >> 8) & 0xff;
            let b = c & 0xff;
            if a == 255 {
                Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
            } else {
                Some(format!("#{:02x}{:02x}{:02x}{:02x}", r, g, b, a))
            }
        }
        CssValue::Length(v, unit) => Some(format!("{}{}", v, unit_suffix(*unit))),
        CssValue::Percentage(v) => Some(format!("{}%", v / 100)),
        CssValue::Number(v) => Some(v.to_string()),
        CssValue::Auto => Some(String::from("auto")),
        CssValue::None => Some(String::from("none")),
        CssValue::Inherit => Some(String::from("inherit")),
        CssValue::CurrentColor => Some(String::from("currentColor")),
        CssValue::Calc(px, pct) => {
            if *pct == 0 {
                Some(format!("{}px", px))
            } else {
                Some(format!("calc({}px + {}%)", px, pct / 100))
            }
        }
        CssValue::Var(name, fallback) => {
            let mut out = String::from("var(");
            out.push_str(name);
            if let Some(fallback) = fallback {
                if let Some(fallback_text) = custom_property_value_to_string(fallback) {
                    out.push_str(", ");
                    out.push_str(&fallback_text);
                }
            }
            out.push(')');
            Some(out)
        }
    }
}

fn unit_suffix(unit: Unit) -> &'static str {
    match unit {
        Unit::Px => "px",
        Unit::Em => "em",
        Unit::Rem => "rem",
        Unit::In => "in",
        Unit::Cm => "cm",
        Unit::Mm => "mm",
        Unit::Pt => "pt",
        Unit::Pc => "pc",
        Unit::Q => "q",
        Unit::Percent => "%",
        Unit::Fr => "fr",
        Unit::Vw => "vw",
        Unit::Vh => "vh",
        Unit::Vmin => "vmin",
        Unit::Vmax => "vmax",
    }
}

/// Look up a custom property by walking the DOM parent chain.
///
/// Checks the current node's own custom properties first, then walks up
/// the ancestor chain. Returns the raw value string if found.
fn lookup_custom_property<'a>(
    name: &str,
    node_cp: &'a [(String, String)],
    dom: &Dom,
    node_id: NodeId,
    ancestors_cp: &'a [Vec<(String, String)>],
) -> Option<&'a str> {
    // Check this node's own custom properties first.
    if let Some((_, val)) = node_cp.iter().find(|(k, _)| k == name) {
        return Some(val.as_str());
    }
    // Walk up the parent chain.
    let mut cur = dom.nodes[node_id].parent;
    while let Some(pid) = cur {
        if pid < ancestors_cp.len() {
            if let Some((_, val)) = ancestors_cp[pid].iter().find(|(k, _)| k == name) {
                return Some(val.as_str());
            }
        }
        cur = dom.nodes.get(pid).and_then(|node| node.parent);
    }
    fallback_custom_property(name)
}

fn custom_property_is_unset_keyword(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.eq_ignore_ascii_case("initial")
        || trimmed.eq_ignore_ascii_case("unset")
        || trimmed.eq_ignore_ascii_case("revert")
        || trimmed.eq_ignore_ascii_case("revert-layer")
}

fn fallback_custom_property(name: &str) -> Option<&'static str> {
    match name {
        "--base" | "--text-black" | "--surface-black" | "--global-black" => Some("#333333"),
        "--black" => Some("#000000"),
        "--white" | "--global-white" | "--background-primary" | "--text-on-color-white" => {
            Some("#ffffff")
        }
        "--background" | "--background-light" | "--surface-gray-10" => Some("#eeeeef"),
        "--surface-gray-20" | "--border-bg" | "--border-medium" => Some("#e0e0e0"),
        "--surface-gray-30" | "--text-muted" => Some("#666666"),
        "--surface-brand" | "--text-brand" | "--border-brand" | "--primary" => Some("#ee001c"),
        "--text-link" => Some("#156fbc"),
        "--font-weight-bold" => Some("700"),
        "--font-weight-bolder" => Some("800"),
        "--font-weight-black" => Some("900"),
        "--font-letter-spacing" | "--font-letter-spacing-p" | "--font-letter-spacing-caps" => {
            Some("0")
        }
        "--font-family-headline" | "--font-family-inter-tight" | "--website-font"
        | "--website-paragraph" => Some("Arial"),
        "--text-xxs" | "--unified-text-xxs" => Some(".555rem"),
        "--text-xs" | "--unified-text-xs" => Some(".666rem"),
        "--text-sm" | "--unified-text-sm" => Some(".777rem"),
        "--text-md" | "--unified-text-md" => Some(".888rem"),
        "--text-base" | "--unified-text-base" => Some("1rem"),
        "--text-lg" | "--unified-text-lg" => Some("1.125rem"),
        "--text-xl" | "--unified-text-xl" => Some("1.222rem"),
        "--text-xxl" | "--unified-text-xxl" => Some("1.5rem"),
        "--headline-xxs" | "--unified-headline-xxs" => Some(".888rem"),
        "--headline-xs" | "--unified-headline-xs" => Some("1rem"),
        "--headline-sm" | "--unified-headline-sm" | "--headline-lg-mobile" => Some("1.222rem"),
        "--headline-md" | "--unified-headline-md" => Some("1.555rem"),
        "--headline-lg" | "--unified-headline-lg" => Some("1.777rem"),
        "--headline-xl" | "--unified-headline-xl" => Some("2rem"),
        "--headline-xxl" | "--unified-headline-xxl" => Some("3rem"),
        "--line-height-default" | "--line-height-text-xs" | "--line-height-text-sm"
        | "--line-height-text-md" | "--line-height-text-lg" | "--line-height-text-xl"
        | "--line-height-text-xxl" | "--txt-line-height-xs" | "--txt-line-height-sm"
        | "--txt-line-height-md" | "--txt-line-height-lg" | "--txt-line-height-xl"
        | "--unified-line-height-base" | "--unified-line-height-text-xxs"
        | "--unified-line-height-text-xs" | "--unified-line-height-text-sm"
        | "--unified-line-height-text-md" | "--unified-line-height-text-lg"
        | "--unified-line-height-text-xl" | "--unified-line-height-text-xxl" => Some("1.3"),
        "--line-height-hl-xxs" | "--line-height-hl-xs" | "--line-height-hl-sm"
        | "--line-height-hl-md" | "--line-height-hl-lg" | "--line-height-hl-xl"
        | "--line-height-hl-xxl" | "--unified-line-height-hl-xxs"
        | "--unified-line-height-hl-xs" | "--unified-line-height-hl-sm"
        | "--unified-line-height-hl-md" | "--unified-line-height-hl-lg"
        | "--unified-line-height-hl-xl" | "--unified-line-height-hl-xxl" => Some("1.2"),
        "--spacing-xxs" => Some(".125rem"),
        "--spacing-xs" => Some(".25rem"),
        "--spacing-sm" => Some(".5rem"),
        "--spacing-md" => Some("1rem"),
        "--spacing-lg" => Some("1.5rem"),
        "--spacing-xl" => Some("2rem"),
        "--spacing-xxl" => Some("4rem"),
        "--spacing" => Some(".25rem"),
        "--container-xs" => Some("20rem"),
        "--container-sm" => Some("24rem"),
        "--container-md" => Some("28rem"),
        "--container-lg" => Some("32rem"),
        "--container-xl" => Some("36rem"),
        "--container-2xl" => Some("42rem"),
        "--container-3xl" => Some("48rem"),
        "--container-4xl" => Some("56rem"),
        "--container-5xl" => Some("64rem"),
        "--container-6xl" => Some("72rem"),
        "--container-7xl" => Some("80rem"),
        "--scaling-factor-xxxs" => Some("27/40"),
        "--scaling-factor-xxs" => Some("3/4"),
        "--scaling-factor-xs" => Some("27/32"),
        "--scaling-factor-sm" => Some("1"),
        "--scaling-factor-base" => Some("9/8"),
        "--scaling-factor-md" => Some("27/20"),
        "--scaling-factor-lg" => Some("27/16"),
        "--scaling-factor-xl" => Some("2/1"),
        "--scaling-factor-xxl" => Some("9/4"),
        "--scaling-factor-xxxl" => Some("27/8"),
        "--baseline-down-04" => Some("calc(1rem * 27/40)"),
        "--baseline-down-03" => Some("calc(1rem * 3/4)"),
        "--baseline-down-02" => Some("calc(1rem * 27/32)"),
        "--baseline-down-01" => Some("1rem"),
        "--baseline" => Some("calc(1rem * 9/8)"),
        "--baseline-up-01" => Some("calc(1rem * 27/20)"),
        "--baseline-up-02" => Some("calc(1rem * 27/16)"),
        "--baseline-up-03" => Some("calc(1rem * 2)"),
        "--baseline-up-04" => Some("calc(1rem * 9/4)"),
        "--baseline-up-05" => Some("calc(1rem * 27/8)"),
        "--grid-spacing" | "--container-spacing" => Some("20px"),
        "--column-gap" => Some("1.25rem"),
        "--article-content-width" => Some("800px"),
        "--full-content-width" | "--container-width" => Some("956px"),
        "--border-sm" => Some("1px"),
        _ => None,
    }
}

/// Resolve var() references by walking the DOM parent chain.
fn resolve_var_in_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if matches!(decl.value, CssValue::Var(_, _)) {
        if let Some(value) = resolve_css_var_value(
            &decl.value,
            &decl.property,
            dom,
            node_id,
            node_cp,
            ancestors_cp,
            0,
        ) {
            return Declaration {
                property: decl.property.clone(),
                value,
                important: decl.important,
            };
        }
    }
    decl.clone()
}

fn resolve_css_var_value(
    value: &CssValue,
    property: &Property,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
    depth: usize,
) -> Option<CssValue> {
    if depth > 8 {
        return None;
    }
    match value {
        CssValue::Var(name, fallback) => {
            if let Some(val) = lookup_custom_property(name, node_cp, dom, node_id, ancestors_cp) {
                if custom_property_is_unset_keyword(val) {
                    return if let Some(fb) = fallback {
                        resolve_css_var_value(
                            fb,
                            property,
                            dom,
                            node_id,
                            node_cp,
                            ancestors_cp,
                            depth + 1,
                        )
                    } else {
                        None
                    };
                }
                let resolved_val = if val.contains("var(") {
                    resolve_nested_vars(val, dom, node_id, node_cp, ancestors_cp)
                } else {
                    String::from(val)
                };
                let parsed = crate::css::parse_value(property, &resolved_val);
                if matches!(parsed, CssValue::Var(_, _)) {
                    resolve_css_var_value(
                        &parsed,
                        property,
                        dom,
                        node_id,
                        node_cp,
                        ancestors_cp,
                        depth + 1,
                    )
                } else {
                    Some(parsed)
                }
            } else if let Some(fb) = fallback {
                if matches!(**fb, CssValue::Var(_, _)) {
                    resolve_css_var_value(
                        fb,
                        property,
                        dom,
                        node_id,
                        node_cp,
                        ancestors_cp,
                        depth + 1,
                    )
                } else {
                    Some((**fb).clone())
                }
            } else {
                None
            }
        }
        other => Some(other.clone()),
    }
}

/// Check if a declaration has nested var() inside a function value (e.g. rgb(R G B/var(--x,1))).
fn has_nested_var(decl: &Declaration) -> bool {
    if let CssValue::Keyword(ref s) = decl.value {
        s.contains("var(")
    } else {
        false
    }
}

/// Resolve nested var() references within a value string, e.g.
/// "rgb(31 30 28/var(--tw-bg-opacity,1))" → "rgb(31 30 28/1)"
fn resolve_nested_vars(
    value: &str,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> String {
    let mut result = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"var(" {
            // Find matching closing paren, respecting nesting
            let start = i + 4;
            let mut depth: u32 = 1;
            let mut end = start;
            while end < bytes.len() && depth > 0 {
                if bytes[end] == b'(' {
                    depth += 1;
                }
                if bytes[end] == b')' {
                    depth -= 1;
                }
                if depth > 0 {
                    end += 1;
                }
            }
            let inner = &value[start..end]; // content between var( and )
                                            // Split on first comma for fallback
            let (var_name, fallback) = if let Some(comma) = inner.find(',') {
                (inner[..comma].trim(), Some(inner[comma + 1..].trim()))
            } else {
                (inner.trim(), None)
            };
            // Look up the variable
            if let Some(val) = lookup_custom_property(var_name, node_cp, dom, node_id, ancestors_cp)
            {
                let use_fallback = custom_property_is_unset_keyword(val);
                if use_fallback {
                    if let Some(fb) = fallback {
                        let resolved_fb = resolve_nested_vars(fb, dom, node_id, node_cp, ancestors_cp);
                        result.push_str(&resolved_fb);
                    } else {
                        let stop = (end + 1).min(bytes.len());
                        result.push_str(&value[i..stop]);
                    }
                } else {
                    let resolved_val = if val.contains("var(") {
                        resolve_nested_vars(val, dom, node_id, node_cp, ancestors_cp)
                    } else {
                        String::from(val)
                    };
                    result.push_str(&resolved_val);
                }
            } else if let Some(fb) = fallback {
                // Recursively resolve vars in fallback too
                let resolved_fb = resolve_nested_vars(fb, dom, node_id, node_cp, ancestors_cp);
                result.push_str(&resolved_fb);
            } else {
                // No value, no fallback — keep original
                let stop = (end + 1).min(bytes.len());
                result.push_str(&value[i..stop]);
            }
            i = (end + 1).min(bytes.len()); // skip past closing )
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}

/// Resolve a declaration that has nested var() in its Keyword value.
fn resolve_nested_var_decl(
    decl: &Declaration,
    dom: &Dom,
    node_id: NodeId,
    node_cp: &[(String, String)],
    ancestors_cp: &[Vec<(String, String)>],
) -> Declaration {
    if let CssValue::Keyword(ref s) = decl.value {
        let resolved_str = resolve_nested_vars(s, dom, node_id, node_cp, ancestors_cp);
        let resolved = crate::css::parse_value(&decl.property, &resolved_str);
        Declaration {
            property: decl.property.clone(),
            value: resolved,
            important: decl.important,
        }
    } else {
        decl.clone()
    }
}

// ---------------------------------------------------------------------------
// Inheritance (only unset inheritable properties)
// ---------------------------------------------------------------------------

fn inherit_unset(child: &mut ComputedStyle, parent: &ComputedStyle, set: u32) {
    if set & SET_COLOR == 0 {
        child.color = parent.color;
    }
    if set & SET_FONT_SIZE == 0 {
        child.font_size = parent.font_size;
    }
    if set & SET_FONT_WEIGHT == 0 {
        child.font_weight = parent.font_weight;
    }
    if set & SET_FONT_STYLE == 0 {
        child.font_style = parent.font_style;
    }
    if set & SET_FONT_FAMILY == 0 {
        child.font_family = parent.font_family.clone();
    }
    if set & SET_DIRECTION == 0 {
        child.direction = parent.direction;
    }
    if set & SET_WRITING_MODE == 0 {
        child.writing_mode = parent.writing_mode;
    }
    if set & SET_TEXT_ALIGN == 0 {
        child.text_align = parent.text_align;
    }
    if set & SET_LINE_HEIGHT == 0 {
        child.line_height = parent.line_height;
    }
    if set & SET_WHITE_SPACE == 0 {
        child.white_space = parent.white_space;
    }
    if set & SET_LIST_STYLE == 0 {
        child.list_style = parent.list_style;
    }
    if set & SET_LIST_STYLE_POS == 0 {
        child.list_style_position = parent.list_style_position;
    }
    if set & SET_TEXT_DECO == 0 {
        child.text_decoration = parent.text_decoration;
    }
    if set & SET_VISIBILITY == 0 {
        child.visibility = parent.visibility;
    }
    if set & SET_TEXT_TRANSFORM == 0 {
        child.text_transform = parent.text_transform;
    }
    if set & SET_LETTER_SPACING == 0 {
        child.letter_spacing = parent.letter_spacing;
    }
    if set & SET_WORD_SPACING == 0 {
        child.word_spacing = parent.word_spacing;
    }
    if set & SET_WORD_BREAK == 0 {
        child.word_break = parent.word_break;
    }
    if set & SET_OVERFLOW_WRAP == 0 {
        child.overflow_wrap = parent.overflow_wrap;
    }
    if set & SET_ACCENT_COLOR == 0 {
        child.accent_color = parent.accent_color;
    }
    if set & SET_COLOR_SCHEME == 0 {
        child.color_scheme = parent.color_scheme;
    }
}

fn inherit_unset_in_dom_order(dom: &Dom, styles: &mut [ComputedStyle], set_flags: &[u32]) {
    fn walk(dom: &Dom, styles: &mut [ComputedStyle], set_flags: &[u32], node_id: usize) {
        if node_id >= styles.len() {
            return;
        }
        let parent_style = dom.nodes[node_id]
            .parent
            .and_then(|pid| styles.get(pid))
            .cloned();
        if let Some(parent) = parent_style {
            let flags = set_flags.get(node_id).copied().unwrap_or(0);
            inherit_unset(&mut styles[node_id], &parent, flags);
            if flags & SET_LINE_HEIGHT == 0 {
                styles[node_id].line_height = (styles[node_id].font_size * 6 + 2) / 5;
            }
        }

        let children = dom.nodes[node_id].children.clone();
        for child_id in children {
            walk(dom, styles, set_flags, child_id);
        }
    }

    for (node_id, node) in dom.nodes.iter().enumerate() {
        if node.parent.is_none() {
            walk(dom, styles, set_flags, node_id);
        }
    }
}

/// Map a CSS property to the inheritable-set bitflag (0 if not inheritable).
fn decl_set_flag(prop: &Property) -> u32 {
    match prop {
        Property::Color => SET_COLOR,
        Property::AccentColor => SET_ACCENT_COLOR,
        Property::ColorScheme => SET_COLOR_SCHEME,
        Property::FontSize => SET_FONT_SIZE,
        Property::FontWeight => SET_FONT_WEIGHT,
        Property::FontStyle => SET_FONT_STYLE,
        Property::FontFamily => SET_FONT_FAMILY,
        Property::Direction => SET_DIRECTION,
        Property::WritingMode => SET_WRITING_MODE,
        Property::TextAlign => SET_TEXT_ALIGN,
        Property::LineHeight => SET_LINE_HEIGHT,
        Property::WhiteSpace => SET_WHITE_SPACE,
        Property::ListStyleType => SET_LIST_STYLE,
        Property::ListStylePosition => SET_LIST_STYLE_POS,
        Property::TextDecoration => SET_TEXT_DECO,
        Property::Visibility => SET_VISIBILITY,
        Property::TextTransform => SET_TEXT_TRANSFORM,
        Property::LetterSpacing => SET_LETTER_SPACING,
        Property::WordSpacing => SET_WORD_SPACING,
        Property::WordBreak => SET_WORD_BREAK,
        Property::OverflowWrap => SET_OVERFLOW_WRAP,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Declaration application
// ---------------------------------------------------------------------------

fn apply_inset_side(
    value: &CssValue,
    offset: &mut Option<i32>,
    calc: &mut Option<(i32, i32)>,
    parent_fs: i32,
    root_fs: i32,
) {
    if matches!(value, CssValue::Auto) {
        *offset = None;
        *calc = None;
    } else if let CssValue::Calc(px, pct) = value {
        *offset = if *pct == 0 { Some(px / 100) } else { None };
        *calc = Some((*px, *pct));
    } else if let CssValue::Percentage(v) = value {
        *offset = None;
        *calc = Some((0, *v));
    } else if let Some(px) = resolve_length(value, parent_fs, root_fs) {
        *offset = Some(px);
        *calc = None;
    }
}

/// Resolve a CSS length value to pixels.
///
/// `CssValue::Length` stores fixed-point * 100: "16px" -> Length(1600, Px),
/// "1.5em" -> Length(150, Em), "2rem" -> Length(200, Rem).
///
/// Conversion formulas (v = stored value):
///   Px:  pixels = v / 100
///   Em:  pixels = v * parent_fs / 100
///   Rem: pixels = v * root_fs / 100
///   Pt:  pixels = v * 4 / 300   (1pt ~= 1.333px)
pub fn apply_declaration(
    style: &mut ComputedStyle,
    decl: &Declaration,
    parent_style: Option<&ComputedStyle>,
    parent_fs: i32,
    root_fs: i32,
) {
    match decl.property {
        Property::Display => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.display = parent.display;
                }
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.display = match kw.as_str() {
                    "block" => Display::Block,
                    "inline" => Display::Inline,
                    "inline-block" => Display::InlineBlock,
                    "list-item" => Display::ListItem,
                    "table-row" => Display::TableRow,
                    "table-cell" => Display::TableCell,
                    "flex" => Display::Flex,
                    "inline-flex" => Display::InlineFlex,
                    "grid" => Display::Grid,
                    "inline-grid" => Display::InlineGrid,
                    "flow-root" => Display::FlowRoot,
                    "none" => Display::None,
                    "contents" => Display::Contents,
                    _ => style.display,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.display = Display::None;
            }
        }
        Property::Color => {
            match decl.value {
                CssValue::Color(c) => {
                    style.color = c;
                }
                CssValue::CurrentColor => {}
                CssValue::Inherit => {
                    if let Some(parent) = parent_style {
                        style.color = parent.color;
                    }
                }
                _ => {}
            }
        }
        Property::BackgroundColor | Property::Background => match decl.value {
            CssValue::Color(c) => {
                style.background_color = c;
                style.background_color_is_current = false;
            }
            CssValue::None => {
                style.background_color = 0x00000000;
                style.background_color_is_current = false;
            }
            CssValue::CurrentColor => {
                style.background_color_is_current = true;
                style.background_color = style.color;
            }
            CssValue::Inherit => {
                if let Some(parent) = parent_style {
                    style.background_color = parent.background_color;
                    style.background_color_is_current = parent.background_color_is_current;
                }
            }
            _ => {}
        },
        Property::AccentColor => match decl.value {
            CssValue::Color(c) => {
                style.accent_color = c;
            }
            CssValue::CurrentColor => {
                style.accent_color = style.color;
            }
            CssValue::Auto | CssValue::None => {
                style.accent_color = 0;
            }
            _ => {}
        },
        Property::FontSize => {
            if let CssValue::Percentage(v) = decl.value {
                let px = (parent_fs as i64 * v as i64 / 10000) as i32;
                if px > 0 {
                    style.font_size = px;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                if px > 0 {
                    style.font_size = px;
                }
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_size = match kw.as_str() {
                    "xx-small" => 9,
                    "x-small" => 10,
                    "small" => 13,
                    "medium" => 16,
                    "large" => 18,
                    "x-large" => 24,
                    "xx-large" => 32,
                    "smaller" => (parent_fs * 5 + 3) / 6, // ~0.833x
                    "larger" => (parent_fs * 6 + 2) / 5,  // ~1.2x
                    _ => style.font_size,
                };
            }
        }
        Property::FontWeight => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_weight = match kw.as_str() {
                    "bold" | "bolder" => FontWeight::Bold,
                    "normal" | "lighter" => FontWeight::Normal,
                    _ => style.font_weight,
                };
            }
            if let CssValue::Number(v) = decl.value {
                style.font_weight = if v / 100 >= 700 {
                    FontWeight::Bold
                } else {
                    FontWeight::Normal
                };
            }
        }
        Property::FontStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_style = match kw.as_str() {
                    "italic" | "oblique" => FontStyleVal::Italic,
                    _ => FontStyleVal::Normal,
                };
            }
        }
        Property::Direction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.direction = match kw.as_str() {
                    "rtl" => Direction::Rtl,
                    _ => Direction::Ltr,
                };
            }
        }
        Property::WritingMode => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.writing_mode = match kw.as_str() {
                    "vertical-lr" => WritingMode::VerticalLr,
                    "vertical-rl" => WritingMode::VerticalRl,
                    "sideways-lr" => WritingMode::SidewaysLr,
                    "sideways-rl" => WritingMode::SidewaysRl,
                    _ => WritingMode::HorizontalTb,
                };
            }
        }
        Property::TextAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_align = match kw.as_str() {
                    "center" => TextAlignVal::Center,
                    "right" => TextAlignVal::Right,
                    "end" => {
                        if style.direction == Direction::Rtl {
                            TextAlignVal::Left
                        } else {
                            TextAlignVal::Right
                        }
                    }
                    "justify" => TextAlignVal::Justify,
                    "start" | "match-parent" => {
                        if style.direction == Direction::Rtl {
                            TextAlignVal::Right
                        } else {
                            TextAlignVal::Left
                        }
                    }
                    _ => TextAlignVal::Left,
                };
            } else if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.text_align = parent.text_align;
                }
            }
        }
        Property::TextDecoration => {
            match decl.value {
                CssValue::Keyword(ref kw) => {
                    style.text_decoration = match kw.as_str() {
                        "underline" => TextDeco::Underline,
                        "line-through" => TextDeco::LineThrough,
                        "overline" => TextDeco::Overline,
                        "none" => TextDeco::None,
                        _ => style.text_decoration,
                    };
                }
                CssValue::None => {
                    style.text_decoration = TextDeco::None;
                }
                CssValue::Inherit => {
                    if let Some(parent) = parent_style {
                        style.text_decoration = parent.text_decoration;
                    }
                }
                _ => {}
            }
        }
        Property::LineHeight => {
            // line-height: <number> means multiple of font_size (not pixels).
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100, e.g. "1.5" -> 150
                style.line_height = (style.font_size * v) / 100;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.line_height = px;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.line_height = (style.font_size * 6 + 2) / 5;
                }
            } else if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.line_height = parent.line_height;
                }
            }
        }
        Property::Width => {
            // Clear all width variants first.
            style.width_max_content = false;
            style.width_min_content = false;
            style.width_fit_content = false;
            match decl.value {
                CssValue::Auto => {
                    style.width = Option::None;
                    style.width_pct = Option::None;
                    style.width_calc = Option::None;
                }
                CssValue::Percentage(v) => {
                    style.width_pct = Some(v);
                    style.width = Option::None;
                    style.width_calc = Option::None;
                }
                CssValue::Calc(px, pct) => {
                    style.width_calc = Some((px, pct));
                    style.width = Option::None;
                    style.width_pct = Option::None;
                }
                CssValue::Keyword(ref kw) => match kw.as_str() {
                    "max-content" | "-webkit-max-content" | "-moz-max-content" => {
                        style.width_max_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    "min-content" | "-webkit-min-content" | "-moz-min-content" => {
                        style.width_min_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    "fit-content" | "-webkit-fit-content" | "-moz-fit-content" => {
                        style.width_fit_content = true;
                        style.width = Option::None;
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                    _ => {
                        if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                            style.width = Some(px);
                            style.width_pct = Option::None;
                            style.width_calc = Option::None;
                        }
                    }
                },
                _ => {
                    if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                        style.width = Some(px);
                        style.width_pct = Option::None;
                        style.width_calc = Option::None;
                    }
                }
            }
        }
        Property::Height => match decl.value {
            CssValue::Auto => {
                style.height = Option::None;
                style.height_pct = Option::None;
                style.height_calc = Option::None;
            }
            CssValue::Percentage(v) => {
                style.height_pct = Some(v);
                style.height = Option::None;
                style.height_calc = Option::None;
            }
            CssValue::Calc(px, pct) => {
                style.height_calc = Some((px, pct));
                style.height = Option::None;
                style.height_pct = Option::None;
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, style.font_size, root_fs) {
                    style.height = Some(px);
                    style.height_pct = Option::None;
                    style.height_calc = Option::None;
                }
            }
        },
        Property::MaxWidth => {
            match decl.value {
                CssValue::None => {
                    style.max_width = Option::None;
                    style.max_width_calc = Option::None;
                }
                CssValue::Percentage(v) => {
                    // Store percentage as negative marker; layout resolves against container.
                    style.max_width = Some(-(v.max(1)));
                    style.max_width_calc = Option::None;
                }
                CssValue::Calc(px, pct) => {
                    style.max_width = Option::None;
                    style.max_width_calc = Some((px, pct));
                }
                _ => {
                    if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                        style.max_width = Some(px);
                        style.max_width_calc = Option::None;
                    }
                }
            }
        }
        Property::MinWidth => {
            if let CssValue::Percentage(v) = decl.value {
                style.min_width = -(v.max(1));
                style.min_width_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.min_width = 0;
                style.min_width_calc = Some((px, pct));
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_width = px;
                style.min_width_calc = Option::None;
            }
        }
        Property::MaxHeight => match decl.value {
            CssValue::None => {
                style.max_height = Option::None;
                style.max_height_calc = Option::None;
            }
            CssValue::Calc(px, pct) => {
                style.max_height = Option::None;
                style.max_height_calc = Some((px, pct));
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                    style.max_height = Some(px);
                    style.max_height_calc = Option::None;
                }
            }
        },
        Property::MinHeight => {
            if let CssValue::Calc(px, pct) = decl.value {
                style.min_height = 0;
                style.min_height_calc = Some((px, pct));
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.min_height = px;
                style.min_height_calc = Option::None;
            }
        }
        // Margin properties — track `auto` for centering.
        Property::Margin => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_left_auto = true;
                style.margin_bottom_auto = true;
                style.margin_right_auto = true;
                style.margin_top_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_left_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_top = 0;
                style.margin_right = 0;
                style.margin_bottom = 0;
                style.margin_left = 0;
                style.margin_top_calc = calc;
                style.margin_right_calc = calc;
                style.margin_bottom_calc = calc;
                style.margin_left_calc = calc;
                style.margin_top_auto = false;
                style.margin_left_auto = false;
                style.margin_bottom_auto = false;
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_right = px;
                style.margin_bottom = px;
                style.margin_left = px;
                style.margin_top_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_left_calc = Option::None;
                style.margin_top_auto = false;
                style.margin_left_auto = false;
                style.margin_bottom_auto = false;
                style.margin_right_auto = false;
            }
        }
        Property::MarginTop => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_top_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_top = 0;
                style.margin_top_calc = Some((px, pct));
                style.margin_top_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_top_calc = Option::None;
                style.margin_top_auto = false;
            }
        }
        Property::MarginRight => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_right_auto = true;
                style.margin_right_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_right = 0;
                style.margin_right_calc = Some((px, pct));
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_right = px;
                style.margin_right_calc = Option::None;
                style.margin_right_auto = false;
            }
        }
        Property::MarginBottom => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_bottom_auto = true;
                style.margin_bottom_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_bottom = 0;
                style.margin_bottom_calc = Some((px, pct));
                style.margin_bottom_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_bottom = px;
                style.margin_bottom_calc = Option::None;
                style.margin_bottom_auto = false;
            }
        }
        Property::MarginLeft => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
                style.margin_left_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                style.margin_left = 0;
                style.margin_left_calc = Some((px, pct));
                style.margin_left_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
                style.margin_left_calc = Option::None;
                style.margin_left_auto = false;
            }
        }
        // Shorthand padding.
        Property::Padding => {
            if let CssValue::Keyword(ref value) = decl.value {
                apply_padding_shorthand(style, value, parent_fs, root_fs);
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_top_pct = Some(v);
                style.padding_right_pct = Some(v);
                style.padding_bottom_pct = Some(v);
                style.padding_left_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_right = px;
                style.padding_bottom = px;
                style.padding_left = px;
                style.padding_top_pct = None;
                style.padding_right_pct = None;
                style.padding_bottom_pct = None;
                style.padding_left_pct = None;
            }
        }
        Property::PaddingTop => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_top = parent.padding_top;
                    style.padding_top_pct = parent.padding_top_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_top_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_top_pct = None;
            }
        }
        Property::PaddingRight => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_right = parent.padding_right;
                    style.padding_right_pct = parent.padding_right_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_right_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_right = px;
                style.padding_right_pct = None;
            }
        }
        Property::PaddingBottom => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_bottom = parent.padding_bottom;
                    style.padding_bottom_pct = parent.padding_bottom_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_bottom_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_bottom = px;
                style.padding_bottom_pct = None;
            }
        }
        Property::PaddingLeft => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.padding_left = parent.padding_left;
                    style.padding_left_pct = parent.padding_left_pct;
                }
            } else if let CssValue::Percentage(v) = decl.value {
                style.padding_left_pct = Some(v);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_left = px;
                style.padding_left_pct = None;
            }
        }
        Property::BorderWidth => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px;
                style.border_right.width = px;
                style.border_bottom.width = px;
                style.border_left.width = px;
            }
            if let CssValue::Keyword(ref kw) = decl.value {
                let w = match kw.as_str() {
                    "thin" => 1,
                    "medium" => 3,
                    "thick" => 5,
                    _ => style.border_width,
                };
                style.border_width = w;
                style.border_top.width = w;
                style.border_right.width = w;
                style.border_bottom.width = w;
                style.border_left.width = w;
            }
        }
        Property::BorderColor => {
            let c = match decl.value {
                CssValue::Color(c) => Some(c),
                CssValue::CurrentColor => Some(if style.color != 0 {
                    style.color
                } else {
                    0xFF000000
                }),
                _ => None,
            };
            if let Some(c) = c {
                style.border_color = c;
                style.border_top.color = c;
                style.border_right.color = c;
                style.border_bottom.color = c;
                style.border_left.color = c;
            }
        }
        Property::BorderStyle => {
            let sv = resolve_border_style_val(&decl.value);
            style.border_top.style = sv;
            style.border_right.style = sv;
            style.border_bottom.style = sv;
            style.border_left.style = sv;
        }
        Property::BorderRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_radius = px;
                style.border_top_left_radius = px;
                style.border_top_right_radius = px;
                style.border_bottom_right_radius = px;
                style.border_bottom_left_radius = px;
            }
        }
        // Shorthand border: just pick up width and color from the value.
        Property::Border
        | Property::BorderTop
        | Property::BorderRight
        | Property::BorderBottom
        | Property::BorderLeft => {
            if let CssValue::Color(c) = decl.value {
                style.border_color = c;
                style.border_top.color = c;
                style.border_right.color = c;
                style.border_bottom.color = c;
                style.border_left.color = c;
            }
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_width = px;
                style.border_top.width = px;
                style.border_right.width = px;
                style.border_bottom.width = px;
                style.border_left.width = px;
            }
        }
        Property::ListStyleType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style = match kw.as_str() {
                    "disc" => ListStyle::Disc,
                    "circle" => ListStyle::Circle,
                    "square" => ListStyle::Square,
                    "decimal" | "decimal-leading-zero" => ListStyle::Decimal,
                    "none" => ListStyle::None,
                    "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                    "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
                    "lower-roman" => ListStyle::LowerRoman,
                    "upper-roman" => ListStyle::UpperRoman,
                    _ => style.list_style,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.list_style = ListStyle::None;
            }
        }
        Property::ListStylePosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.list_style_position = match kw.as_str() {
                    "inside" => ListStylePosition::Inside,
                    _ => ListStylePosition::Outside,
                };
            }
        }
        Property::WhiteSpace => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.white_space = match kw.as_str() {
                    "pre" => WhiteSpace::Pre,
                    "nowrap" => WhiteSpace::Nowrap,
                    "pre-wrap" => WhiteSpace::PreWrap,
                    _ => WhiteSpace::Normal,
                };
            }
        }
        Property::Position => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.position = match kw.as_str() {
                    "static" => Position::Static,
                    "relative" => Position::Relative,
                    "absolute" => Position::Absolute,
                    "fixed" => Position::Fixed,
                    "sticky" => Position::Sticky,
                    _ => style.position,
                };
            }
        }
        Property::Top => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.top = parent.top;
                    style.top_calc = parent.top_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.top,
                    &mut style.top_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Right => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.right_offset = parent.right_offset;
                    style.right_calc = parent.right_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.right_offset,
                    &mut style.right_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Bottom => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.bottom_offset = parent.bottom_offset;
                    style.bottom_calc = parent.bottom_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.bottom_offset,
                    &mut style.bottom_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::Left => {
            if matches!(decl.value, CssValue::Inherit) {
                if let Some(parent) = parent_style {
                    style.left_offset = parent.left_offset;
                    style.left_calc = parent.left_calc;
                }
            } else {
                apply_inset_side(
                    &decl.value,
                    &mut style.left_offset,
                    &mut style.left_calc,
                    parent_fs,
                    root_fs,
                );
            }
        }
        Property::ZIndex => match decl.value {
            CssValue::Number(v) => {
                style.z_index = v / 100;
                style.z_index_auto = false;
            }
            CssValue::Auto | CssValue::Inherit => {
                style.z_index = 0;
                style.z_index_auto = true;
            }
            _ => {
                if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                    style.z_index = px;
                    style.z_index_auto = false;
                }
            }
        },
        Property::FlexDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_direction = match kw.as_str() {
                    "row" => FlexDirection::Row,
                    "row-reverse" => FlexDirection::RowReverse,
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    _ => style.flex_direction,
                };
            }
        }
        Property::FlexWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.flex_wrap = match kw.as_str() {
                    "nowrap" => FlexWrap::Nowrap,
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => style.flex_wrap,
                };
            }
        }
        Property::FlexFlow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                for part in kw.split_whitespace() {
                    match part {
                        "row" => style.flex_direction = FlexDirection::Row,
                        "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
                        "column" => style.flex_direction = FlexDirection::Column,
                        "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
                        "nowrap" => style.flex_wrap = FlexWrap::Nowrap,
                        "wrap" => style.flex_wrap = FlexWrap::Wrap,
                        "wrap-reverse" => style.flex_wrap = FlexWrap::WrapReverse,
                        _ => {}
                    }
                }
            }
        }
        Property::JustifyContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_content = match kw.as_str() {
                    "flex-start" | "start" | "left" => JustifyContent::FlexStart,
                    "flex-end" | "end" | "right" => JustifyContent::FlexEnd,
                    "center" => JustifyContent::Center,
                    "space-between" => JustifyContent::SpaceBetween,
                    "space-around" => JustifyContent::SpaceAround,
                    "space-evenly" => JustifyContent::SpaceEvenly,
                    _ => style.justify_content,
                };
            }
        }
        Property::AlignItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.align_items = match kw.as_str() {
                    "flex-start" | "start" => AlignItems::FlexStart,
                    "flex-end" | "end" => AlignItems::FlexEnd,
                    "center" => AlignItems::Center,
                    "stretch" => AlignItems::Stretch,
                    "baseline" => AlignItems::Baseline,
                    _ => style.align_items,
                };
            }
        }
        Property::AlignSelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_self_alignment_kw(kw) {
                    style.align_self = v;
                    style.align_self_is_normal = kw.trim() == "normal";
                }
            }
        }
        Property::JustifySelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_self_alignment_kw(kw) {
                    style.justify_self = v;
                    style.justify_self_is_normal = kw.trim() == "normal";
                    style.justify_self_inline = parse_inline_axis_alignment_kw(kw);
                }
            }
        }
        Property::PlaceItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_items_value(kw);
                style.align_items = align;
                style.justify_items = justify;
                style.justify_items_specified = true;
                style.justify_items_inline = parse_place_items_inline_value(kw).1;
            }
        }
        Property::PlaceSelf => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_self_value(kw);
                style.align_self = align;
                style.justify_self = justify;
                style.align_self_is_normal = kw.split_whitespace().next() == Some("normal");
                style.justify_self_is_normal = kw.split_whitespace().nth(1) == Some("normal")
                    || (kw.split_whitespace().nth(1).is_none()
                        && kw.split_whitespace().next() == Some("normal"));
                style.justify_self_inline = parse_place_self_inline_value(kw).1;
            }
        }
        Property::PlaceContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (align, justify) = parse_place_content_value(kw);
                style.align_content = align;
                style.align_content_is_normal = kw.split_whitespace().next() == Some("normal");
                style.justify_content = justify;
            }
        }
        Property::FlexGrow => {
            if let CssValue::Number(v) = decl.value {
                style.flex_grow = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_grow = px * 100;
            }
        }
        Property::FlexShrink => {
            if let CssValue::Number(v) = decl.value {
                style.flex_shrink = v;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_shrink = px * 100;
            }
        }
        Property::FlexBasis => {
            if matches!(decl.value, CssValue::Auto) {
                style.flex_basis = Option::None;
                style.flex_basis_pct = Option::None;
            } else if let CssValue::Length(v, Unit::Percent) = &decl.value {
                // Percentage flex-basis: resolved at layout time against container main size.
                // Stored as percent × 100 (e.g. 100% → 10000), matching width_pct convention.
                style.flex_basis_pct = Some(*v);
                style.flex_basis = Option::None;
            } else if let CssValue::Percentage(v) = &decl.value {
                // Percentage(v) is also stored as percent × 100, just like Length(_, Percent).
                style.flex_basis_pct = Some(*v);
                style.flex_basis = Option::None;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.flex_basis = Some(px);
                style.flex_basis_pct = Option::None;
            }
        }
        Property::RowGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.row_gap = px;
            }
        }
        Property::ColumnGap => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.column_gap = px;
            }
        }
        Property::Order => {
            if let CssValue::Number(v) = decl.value {
                style.order = v / 100;
            }
        }
        Property::BoxSizing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.box_sizing = match kw.as_str() {
                    "border-box" => BoxSizing::BorderBox,
                    "content-box" => BoxSizing::ContentBox,
                    _ => style.box_sizing,
                };
            }
        }
        Property::Float => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.float = match kw.as_str() {
                    "left" => FloatVal::Left,
                    "right" => FloatVal::Right,
                    "none" => FloatVal::None,
                    _ => style.float,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.float = FloatVal::None;
            }
        }
        Property::Clear => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.clear = match kw.as_str() {
                    "left" => ClearVal::Left,
                    "right" => ClearVal::Right,
                    "both" => ClearVal::Both,
                    "none" => ClearVal::None,
                    _ => style.clear,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.clear = ClearVal::None;
            }
        }
        Property::Opacity => {
            if let CssValue::Number(v) = decl.value {
                // v is fixed-point * 100: "0.5" → 50, "1" → 100
                style.opacity = ((v * 255) / 100).max(0).min(255);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.opacity = (px * 255).max(0).min(255);
            }
        }
        Property::Visibility => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.visibility = match kw.as_str() {
                    "visible" => Visibility::Visible,
                    "hidden" => Visibility::Hidden,
                    "collapse" => Visibility::Collapse,
                    _ => style.visibility,
                };
            }
        }
        Property::TextTransform => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_transform = match kw.as_str() {
                    "uppercase" => TextTransform::Uppercase,
                    "lowercase" => TextTransform::Lowercase,
                    "capitalize" => TextTransform::Capitalize,
                    "none" => TextTransform::None,
                    _ => style.text_transform,
                };
            }
            if matches!(decl.value, CssValue::None) {
                style.text_transform = TextTransform::None;
            }
        }
        Property::OverflowX => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_x = parse_overflow_keyword(kw);
            }
        }
        Property::OverflowY => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_y = parse_overflow_keyword(kw);
            }
        }
        // Transitions
        Property::Transition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.transitions = parse_transition_shorthand(kw);
            }
        }
        Property::TransitionProperty => {
            // Set property names on existing TransitionDef entries, or create one.
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style
                    .transitions
                    .resize_with(names.len().max(style.transitions.len()), || TransitionDef {
                        property: String::new(),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                    });
                for (i, name) in names.iter().enumerate() {
                    if i < style.transitions.len() {
                        style.transitions[i].property = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::TransitionDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: ms,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.duration_ms = ms;
                    }
                }
            }
        }
        Property::TransitionTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: 0,
                        timing: tf,
                        delay_ms: 0,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.timing = tf;
                    }
                }
            }
        }
        Property::TransitionDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.transitions.is_empty() {
                    style.transitions.push(TransitionDef {
                        property: String::from("all"),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: ms,
                    });
                } else {
                    for t in &mut style.transitions {
                        t.delay_ms = ms;
                    }
                }
            }
        }
        // Animations
        Property::Animation => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.animations = parse_animation_shorthand(kw);
            }
        }
        Property::AnimationName => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let names: Vec<&str> = kw.split(',').map(|s| s.trim()).collect();
                style
                    .animations
                    .resize_with(names.len().max(style.animations.len()), || AnimationDef {
                        name: String::new(),
                        duration_ms: 0,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                        iteration_count: 1,
                        alternate: false,
                    });
                for (i, name) in names.iter().enumerate() {
                    if i < style.animations.len() {
                        style.animations[i].name = name.to_ascii_lowercase();
                    }
                }
            }
        }
        Property::AnimationDuration => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                if style.animations.is_empty() {
                    style.animations.push(AnimationDef {
                        name: String::new(),
                        duration_ms: ms,
                        timing: TimingFunction::Ease,
                        delay_ms: 0,
                        iteration_count: 1,
                        alternate: false,
                    });
                } else {
                    for a in &mut style.animations {
                        a.duration_ms = ms;
                    }
                }
            }
        }
        Property::AnimationTimingFunction => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let tf = parse_timing_function(kw);
                for a in &mut style.animations {
                    a.timing = tf;
                }
            }
        }
        Property::AnimationDelay => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let ms = parse_time_ms(kw);
                for a in &mut style.animations {
                    a.delay_ms = ms;
                }
            }
        }
        Property::AnimationIterationCount => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let count = if kw == "infinite" {
                    0
                } else {
                    kw.parse::<u32>().unwrap_or(1)
                };
                for a in &mut style.animations {
                    a.iteration_count = count;
                }
            } else if let CssValue::Number(v) = decl.value {
                let count = (v / 100) as u32;
                for a in &mut style.animations {
                    a.iteration_count = count;
                }
            }
        }
        Property::AnimationDirection => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let alt = kw == "alternate" || kw == "alternate-reverse";
                for a in &mut style.animations {
                    a.alternate = alt;
                }
            }
        }
        Property::AnimationFillMode | Property::AnimationPlayState => {}
        Property::TextIndent => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_indent = px;
            }
        }
        Property::VerticalAlign => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.vertical_align = match kw.as_str() {
                    "baseline" => VerticalAlign::Baseline,
                    "top" => VerticalAlign::Top,
                    "middle" => VerticalAlign::Middle,
                    "bottom" => VerticalAlign::Bottom,
                    "text-top" => VerticalAlign::TextTop,
                    "text-bottom" => VerticalAlign::TextBottom,
                    "sub" => VerticalAlign::Sub,
                    "super" => VerticalAlign::Super,
                    _ => style.vertical_align,
                };
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.vertical_align = VerticalAlign::Length(px);
            }
        }
        Property::FontFamily => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_family = Some(kw.clone());
            }
        }
        Property::LetterSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.letter_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.letter_spacing = px;
            }
        }
        Property::WordSpacing => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "normal" {
                    style.word_spacing = 0;
                }
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.word_spacing = px;
            }
        }
        Property::WordBreak => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.word_break = match kw.as_str() {
                    "break-all" => WordBreak::BreakAll,
                    "keep-all" => WordBreak::KeepAll,
                    _ => WordBreak::Normal,
                };
            }
        }
        Property::OverflowWrap => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.overflow_wrap = match kw.as_str() {
                    "break-word" => OverflowWrapVal::BreakWord,
                    "anywhere" => OverflowWrapVal::Anywhere,
                    _ => OverflowWrapVal::Normal,
                };
            }
        }
        Property::TextOverflow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_overflow = match kw.as_str() {
                    "ellipsis" => TextOverflowVal::Ellipsis,
                    _ => TextOverflowVal::Clip,
                };
            }
        }
        // Per-side border widths
        Property::BorderTopWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.border_top.width);
            style.border_width = style.border_top.width; // sync unified
        }
        Property::BorderRightWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_right.width,
            );
        }
        Property::BorderBottomWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_bottom.width,
            );
        }
        Property::BorderLeftWidth => {
            resolve_border_width(
                &decl.value,
                parent_fs,
                root_fs,
                &mut style.border_left.width,
            );
        }
        // Per-side border colors
        Property::BorderTopColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_top.color = c;
            }
        }
        Property::BorderRightColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_right.color = c;
            }
        }
        Property::BorderBottomColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_bottom.color = c;
            }
        }
        Property::BorderLeftColor => {
            if let CssValue::Color(c) = decl.value {
                style.border_left.color = c;
            }
        }
        // Per-side border styles
        Property::BorderTopStyle => {
            style.border_top.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderRightStyle => {
            style.border_right.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderBottomStyle => {
            style.border_bottom.style = resolve_border_style_val(&decl.value);
        }
        Property::BorderLeftStyle => {
            style.border_left.style = resolve_border_style_val(&decl.value);
        }
        // Per-corner border radius
        Property::BorderTopLeftRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_top_left_radius = px;
            }
        }
        Property::BorderTopRightRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_top_right_radius = px;
            }
        }
        Property::BorderBottomRightRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_bottom_right_radius = px;
            }
        }
        Property::BorderBottomLeftRadius => {
            if let Some(px) = resolve_border_radius(&decl.value, parent_fs, root_fs) {
                style.border_bottom_left_radius = px;
            }
        }
        // Outline
        Property::OutlineWidth => {
            resolve_border_width(&decl.value, parent_fs, root_fs, &mut style.outline_width);
        }
        Property::OutlineColor => {
            if let CssValue::Color(c) = decl.value {
                style.outline_color = c;
            }
        }
        Property::OutlineStyle => {
            style.outline_style = resolve_border_style_val(&decl.value);
        }
        Property::OutlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.outline_offset = px;
            }
        }
        // Shadows
        Property::BoxShadow => {
            if matches!(decl.value, CssValue::None) {
                style.box_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.box_shadows = parse_box_shadows(kw, parent_fs, root_fs);
            }
        }
        Property::TextShadow => {
            if matches!(decl.value, CssValue::None) {
                style.text_shadows.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.text_shadows = parse_text_shadows(kw, parent_fs, root_fs);
            }
        }
        // Background extensions
        Property::BackgroundImage => {
            if matches!(decl.value, CssValue::None) {
                style.background_image = BackgroundImageVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(parsed) = parse_background_image_val(kw) {
                    style.background_image = parsed;
                }
            }
        }
        Property::BackgroundSize => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_size = match kw.as_str() {
                    "cover" => BackgroundSizeVal::Cover,
                    "contain" => BackgroundSizeVal::Contain,
                    "auto" => BackgroundSizeVal::Auto,
                    _ => {
                        // Try "Wpx Hpx" or "W% H%"
                        let parts: Vec<&str> = kw.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            let h = parse_bg_size_dim(parts[1], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, h)
                        } else if parts.len() == 1 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, -1)
                        } else {
                            BackgroundSizeVal::Auto
                        }
                    }
                };
            }
            if matches!(decl.value, CssValue::Auto) {
                style.background_size = BackgroundSizeVal::Auto;
            }
        }
        Property::BackgroundRepeat => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_repeat = match kw.as_str() {
                    "repeat-x" => BackgroundRepeatVal::RepeatX,
                    "repeat-y" => BackgroundRepeatVal::RepeatY,
                    "no-repeat" => BackgroundRepeatVal::NoRepeat,
                    _ => BackgroundRepeatVal::Repeat,
                };
            }
        }
        Property::BackgroundClip => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.background_clip = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::BackgroundPosition => {
            // Simplified: just parse keywords or lengths
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if !parts.is_empty() {
                    style.background_position_x =
                        parse_bg_position_part(parts[0], parent_fs, root_fs);
                }
                if parts.len() >= 2 {
                    style.background_position_y =
                        parse_bg_position_part(parts[1], parent_fs, root_fs);
                } else if parts.len() == 1 {
                    // CSS Backgrounds: one-value background-position means
                    // horizontal position plus vertical center.
                    style.background_position_y = 5000;
                }
            }
        }
        // Content
        Property::Content => {
            if matches!(decl.value, CssValue::None) {
                style.content = Option::None;
                style.content_url = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                // Use the full content value parser for proper multi-value handling.
                let (text, url) = parse_content_value(kw.as_str());
                style.content = text;
                style.content_url = url;
            }
        }
        Property::ObjectFit => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.object_fit = match kw.as_str() {
                    "fill" => ObjectFit::Fill,
                    "contain" => ObjectFit::Contain,
                    "cover" => ObjectFit::Cover,
                    "none" => ObjectFit::None,
                    "scale-down" => ObjectFit::ScaleDown,
                    _ => style.object_fit,
                };
            }
        }
        Property::ObjectPosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 5000, true, 5000, true);
                style.object_position_x = x;
                style.object_position_x_is_percent = x_is_percent;
                style.object_position_y = y;
                style.object_position_y_is_percent = y_is_percent;
            }
        }
        Property::Transform => {
            // Parse transform functions: translate(x,y), translateX(x), translateY(y)
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_tx = 0;
                style.transform_ty = 0;
                style.transform_tx_pct = 0;
                style.transform_ty_pct = 0;
                style.transform_sx = 1000;
                style.transform_sy = 1000;
                style.transform_rotate = 0;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                let s = kw.as_str();
                let mut tx = 0i32;
                let mut ty = 0i32;
                let mut tx_pct = 0i32;
                let mut ty_pct = 0i32;
                if !s.contains('(') {
                    let parts: Vec<&str> = s.split_whitespace().collect();
                    let looks_like_translate = parts.iter().any(|part| {
                        let p = part.trim();
                        p == "0"
                            || p.ends_with('%')
                            || p.ends_with("px")
                            || p.ends_with("em")
                            || p.ends_with("rem")
                    });
                    if looks_like_translate {
                        if let Some(x) = parts.first() {
                            let (px, pct) = parse_transform_translate_component(x, parent_fs);
                            tx = px;
                            tx_pct = pct;
                        }
                        if let Some(y) = parts.get(1) {
                            let (px, pct) = parse_transform_translate_component(y, parent_fs);
                            ty = px;
                            ty_pct = pct;
                        }
                        style.transform_tx = tx;
                        style.transform_ty = ty;
                        style.transform_tx_pct = tx_pct;
                        style.transform_ty_pct = ty_pct;
                        return;
                    }
                }
                let mut pos = 0usize;
                let bytes = s.as_bytes();
                while pos < bytes.len() {
                    // Skip whitespace
                    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                        pos += 1;
                    }
                    if pos >= bytes.len() {
                        break;
                    }
                    // Read function name
                    let name_start = pos;
                    while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' {
                        pos += 1;
                    }
                    let fname = core::str::from_utf8(&bytes[name_start..pos]).unwrap_or("");
                    if pos < bytes.len() && bytes[pos] == b'(' {
                        pos += 1; // skip '('
                                  // Read args until ')'
                        let args_start = pos;
                        while pos < bytes.len() && bytes[pos] != b')' {
                            pos += 1;
                        }
                        let args = core::str::from_utf8(&bytes[args_start..pos]).unwrap_or("");
                        if pos < bytes.len() {
                            pos += 1;
                        } // skip ')'
                        match fname {
                            "translateX" | "translatex" => {
                                let (px, pct) =
                                    parse_transform_translate_component(args.trim(), parent_fs);
                                tx += px;
                                tx_pct += pct;
                            }
                            "translateY" | "translatey" => {
                                let (px, pct) =
                                    parse_transform_translate_component(args.trim(), parent_fs);
                                ty += px;
                                ty_pct += pct;
                            }
                            "translate" => {
                                let parts: Vec<&str> = if args.contains(',') {
                                    args.split(',').collect()
                                } else {
                                    args.split_whitespace().collect()
                                };
                                if !parts.is_empty() {
                                    let (px, pct) = parse_transform_translate_component(
                                        parts[0].trim(),
                                        parent_fs,
                                    );
                                    tx += px;
                                    tx_pct += pct;
                                }
                                if parts.len() > 1 {
                                    let (px, pct) = parse_transform_translate_component(
                                        parts[1].trim(),
                                        parent_fs,
                                    );
                                    ty += px;
                                    ty_pct += pct;
                                }
                            }
                            "scale" => {
                                // scale(sx) or scale(sx, sy)
                                let parts: Vec<&str> = args.split(',').collect();
                                if let Some(sx_str) = parts.first() {
                                    if let Ok(sx) = sx_str.trim().parse::<f32>() {
                                        style.transform_sx = (sx * 1000.0) as i32;
                                        style.transform_sy = if let Some(sy_str) = parts.get(1) {
                                            if let Ok(sy) = sy_str.trim().parse::<f32>() {
                                                (sy * 1000.0) as i32
                                            } else {
                                                style.transform_sx
                                            }
                                        } else {
                                            style.transform_sx
                                        };
                                    }
                                }
                            }
                            "scaleX" | "scalex" => {
                                if let Ok(sx) = args.trim().parse::<f32>() {
                                    style.transform_sx = (sx * 1000.0) as i32;
                                }
                            }
                            "scaleY" | "scaley" => {
                                if let Ok(sy) = args.trim().parse::<f32>() {
                                    style.transform_sy = (sy * 1000.0) as i32;
                                }
                            }
                            "rotate" => {
                                let s = args.trim();
                                let deg = if s.ends_with("deg") {
                                    s.trim_end_matches("deg").parse::<f32>().ok()
                                } else if s.ends_with("rad") {
                                    s.trim_end_matches("rad")
                                        .parse::<f32>()
                                        .ok()
                                        .map(|r| r * 180.0 / 3.14159265)
                                } else if s.ends_with("turn") {
                                    s.trim_end_matches("turn")
                                        .parse::<f32>()
                                        .ok()
                                        .map(|t| t * 360.0)
                                } else {
                                    s.parse::<f32>().ok()
                                };
                                if let Some(d) = deg {
                                    style.transform_rotate = (d * 100.0) as i32;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        break;
                    }
                }
                style.transform_tx = tx;
                style.transform_ty = ty;
                style.transform_tx_pct = tx_pct;
                style.transform_ty_pct = ty_pct;
            }
        }
        Property::Translate => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_tx = 0;
                style.transform_ty = 0;
                style.transform_tx_pct = 0;
                style.transform_ty_pct = 0;
            } else if let Some((tx, ty, tx_pct, ty_pct)) =
                parse_individual_translate(&decl.value, parent_fs, root_fs)
            {
                style.transform_tx = tx;
                style.transform_ty = ty;
                style.transform_tx_pct = tx_pct;
                style.transform_ty_pct = ty_pct;
            }
        }
        Property::Scale => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_sx = 1000;
                style.transform_sy = 1000;
            } else if let Some((sx, sy)) = parse_individual_scale(&decl.value) {
                style.transform_sx = sx;
                style.transform_sy = sy;
            }
        }
        Property::Rotate => {
            if matches!(decl.value, CssValue::None)
                || matches!(decl.value, CssValue::Keyword(ref k) if k == "none")
            {
                style.transform_rotate = 0;
            } else if let Some(deg100) = parse_individual_rotate(&decl.value) {
                style.transform_rotate = deg100;
            }
        }
        Property::TransformOrigin => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 5000, true, 5000, true);
                style.transform_origin_x = x;
                style.transform_origin_x_is_percent = x_is_percent;
                style.transform_origin_y = y;
                style.transform_origin_y_is_percent = y_is_percent;
            }
        }
        Property::AlignContent => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(v) = parse_align_content_kw(kw) {
                    style.align_content = v;
                    style.align_content_is_normal = kw.trim() == "normal";
                }
            }
        }
        // Properties we parse but do not yet resolve:
        Property::BorderCollapse => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.border_collapse = kw == "collapse";
            }
        }
        Property::BorderSpacing => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.border_spacing_x = px;
                style.border_spacing_y = px;
            } else if let CssValue::Keyword(ref raw) = decl.value {
                let mut parts = raw.split_ascii_whitespace();
                if let Some(first) = parts.next() {
                    if let Some(px) = resolve_length(
                        &crate::css::parse_value(&Property::BorderSpacing, first),
                        parent_fs,
                        root_fs,
                    ) {
                        style.border_spacing_x = px;
                        style.border_spacing_y = parts
                            .next()
                            .and_then(|second| {
                                resolve_length(
                                    &crate::css::parse_value(&Property::BorderSpacing, second),
                                    parent_fs,
                                    root_fs,
                                )
                            })
                            .unwrap_or(px);
                    }
                }
            }
        }
        Property::TableLayout => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.table_layout_fixed = kw == "fixed";
            }
        }
        // Filter
        Property::Filter => {
            if matches!(decl.value, CssValue::None) {
                style.filter = FilterVal::none();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.filter = parse_filter_value(kw, parent_fs, root_fs);
            }
        }
        // Aspect ratio
        Property::AspectRatio => {
            if let CssValue::Keyword(ref kw) = decl.value {
                if kw == "auto" {
                    style.aspect_ratio = 0;
                } else if let Some(pos) = kw.find('/') {
                    // "16 / 9" or "auto 16/9" or "16/9 auto" format
                    // Strip optional "auto" keyword (CSS Sizing §5.1.2).
                    let w_str = kw[..pos]
                        .trim()
                        .trim_start_matches("auto")
                        .trim_end_matches("auto")
                        .trim();
                    let h_str = kw[pos + 1..]
                        .trim()
                        .trim_start_matches("auto")
                        .trim_end_matches("auto")
                        .trim();
                    if let (Some(w), Some(h)) =
                        (try_parse_simple_float(w_str), try_parse_simple_float(h_str))
                    {
                        if h > 0 {
                            style.aspect_ratio = w * 100 / h;
                        }
                    }
                } else if let Some(v) =
                    try_parse_simple_float(kw.trim().trim_start_matches("auto").trim())
                {
                    style.aspect_ratio = v;
                }
            } else if let CssValue::Number(v) = decl.value {
                style.aspect_ratio = v;
            }
        }
        // Text decoration sub-properties
        Property::TextDecorationColor => {
            if let CssValue::Color(c) = decl.value {
                style.text_decoration_color = c;
            }
        }
        Property::TextDecorationStyle => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.text_decoration_style = match kw.as_str() {
                    "solid" => TextDecorationStyle::Solid,
                    "double" => TextDecorationStyle::Double,
                    "dotted" => TextDecorationStyle::Dotted,
                    "dashed" => TextDecorationStyle::Dashed,
                    "wavy" => TextDecorationStyle::Wavy,
                    _ => style.text_decoration_style,
                };
            }
        }
        Property::TextDecorationThickness => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_decoration_thickness = px;
            }
        }
        Property::ColorScheme => {
            if matches!(decl.value, CssValue::Auto) {
                style.color_scheme = ColorSchemeVal::Auto;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                let mut resolved = ColorSchemeVal::Auto;
                for part in kw.split_whitespace() {
                    match part {
                        "dark" => {
                            resolved = ColorSchemeVal::Dark;
                            break;
                        }
                        "light" => {
                            resolved = ColorSchemeVal::Light;
                            break;
                        }
                        "normal" => {
                            resolved = ColorSchemeVal::Auto;
                            break;
                        }
                        "only" => {
                            continue;
                        }
                        _ => {}
                    }
                }
                style.color_scheme = resolved;
            }
        }
        Property::ContainerType => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.container_type = if kw.contains("inline-size") {
                    ContainerTypeVal::InlineSize
                } else if kw.contains("size") {
                    ContainerTypeVal::Size
                } else {
                    ContainerTypeVal::Normal
                };
            } else if matches!(decl.value, CssValue::None) {
                style.container_type = ContainerTypeVal::Normal;
            }
        }
        Property::ContainerName => {
            if matches!(decl.value, CssValue::None) {
                style.container_names.clear();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.container_names = kw
                    .split_whitespace()
                    .filter(|part| !part.is_empty() && *part != "none")
                    .map(String::from)
                    .collect();
            }
        }
        Property::TextUnderlineOffset => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.text_underline_offset = px;
            }
        }
        Property::ScrollBehavior => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.scroll_behavior = if kw.eq_ignore_ascii_case("smooth") {
                    ScrollBehaviorVal::Smooth
                } else {
                    ScrollBehaviorVal::Auto
                };
            }
        }
        // Font variant
        Property::FontVariant => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.font_variant = match kw.as_str() {
                    "small-caps" => FontVariantVal::SmallCaps,
                    _ => FontVariantVal::Normal,
                };
            }
        }
        // Tab size
        Property::TabSize => {
            if let CssValue::Number(v) = decl.value {
                style.tab_size = (v / 100).max(1);
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.tab_size = px.max(1);
            }
        }
        // Clip path
        Property::ClipPath => {
            if matches!(decl.value, CssValue::None) {
                style.clip_path = ClipPathVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_path = parse_clip_path_value(kw, parent_fs, root_fs);
            }
        }
        Property::Clip => {
            // `clip: rect(top, right, bottom, left)` for absolutely-positioned elements.
            // `clip: auto` clears the clip rect.
            if matches!(decl.value, CssValue::Auto) || matches!(decl.value, CssValue::None) {
                style.clip_rect = Option::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.clip_rect = parse_clip_rect(kw, parent_fs, root_fs);
            }
        }
        // CSS counters
        Property::CounterReset => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_reset = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_reset = Option::None;
            }
        }
        Property::CounterIncrement => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.counter_increment = Some(kw.clone());
            } else if matches!(decl.value, CssValue::None) {
                style.counter_increment = Option::None;
            }
        }
        // Inset shorthand is expanded before reaching here.
        Property::Inset => {
            apply_inset_side(
                &decl.value,
                &mut style.top,
                &mut style.top_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.right_offset,
                &mut style.right_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.bottom_offset,
                &mut style.bottom_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.left_offset,
                &mut style.left_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::InsetInline => {
            apply_inset_side(
                &decl.value,
                &mut style.left_offset,
                &mut style.left_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.right_offset,
                &mut style.right_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::InsetBlock => {
            apply_inset_side(
                &decl.value,
                &mut style.top,
                &mut style.top_calc,
                parent_fs,
                root_fs,
            );
            apply_inset_side(
                &decl.value,
                &mut style.bottom_offset,
                &mut style.bottom_calc,
                parent_fs,
                root_fs,
            );
        }
        Property::Overflow => {
            // `overflow` shorthand: one or two keywords.
            // One value → both axes. Two values → overflow-x overflow-y.
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.split_whitespace().collect();
                if parts.len() == 1 {
                    let v = parse_overflow_keyword(parts[0]);
                    style.overflow_x = v;
                    style.overflow_y = v;
                } else if parts.len() >= 2 {
                    style.overflow_x = parse_overflow_keyword(parts[0]);
                    style.overflow_y = parse_overflow_keyword(parts[1]);
                }
            }
        }
        Property::BorderStyle | Property::Flex | Property::Cursor | Property::Outline => {}
        Property::Gap => {
            // gap: <row-gap> <column-gap>?
            // Single value → both row and column gap
            // Two values → row then column
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.row_gap = px;
                style.column_gap = px;
            } else if let CssValue::Keyword(ref s) = decl.value {
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Some(v1) = crate::css::try_parse_dimension_pub(parts[0]) {
                        if let Some(rg) = resolve_length(&v1, parent_fs, root_fs) {
                            style.row_gap = rg;
                        }
                    }
                    if let Some(v2) = crate::css::try_parse_dimension_pub(parts[1]) {
                        if let Some(cg) = resolve_length(&v2, parent_fs, root_fs) {
                            style.column_gap = cg;
                        }
                    }
                } else if parts.len() == 1 {
                    if let Some(v) = crate::css::try_parse_dimension_pub(parts[0]) {
                        if let Some(g) = resolve_length(&v, parent_fs, root_fs) {
                            style.row_gap = g;
                            style.column_gap = g;
                        }
                    }
                }
            }
        }
        // Grid container properties
        Property::GridTemplateColumns => {
            style.grid_template_columns = decode_track_list(&decl.value);
        }
        Property::GridTemplateRows => {
            style.grid_template_rows = decode_track_list(&decl.value);
        }
        Property::GridTemplateAreas => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_template_areas = parse_grid_template_areas_value(kw);
            }
        }
        // GridTemplate shorthand is expanded before reaching here.
        Property::GridTemplate => {}
        Property::GridAutoColumns => {
            style.grid_auto_columns = decode_single_track(&decl.value);
        }
        Property::GridAutoRows => {
            style.grid_auto_rows = decode_single_track(&decl.value);
        }
        Property::GridAutoFlow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_auto_flow_column = kw.contains("column");
            }
        }
        Property::JustifyItems => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.justify_items = parse_align_items_kw(kw);
                style.justify_items_specified = true;
                style.justify_items_inline = parse_inline_axis_alignment_kw(kw);
            }
        }
        // Grid item placement
        Property::GridColumn => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_column_start = start;
                style.grid_column_end = end;
            }
        }
        Property::GridColumnStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_start = GridLine::Index(n);
            }
        }
        Property::GridColumnEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_column_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_column_end = GridLine::Index(n);
            }
        }
        Property::GridRow => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (start, end) = parse_grid_line_pair(kw);
                style.grid_row_start = start;
                style.grid_row_end = end;
            }
        }
        Property::GridRowStart => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_start = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_start = GridLine::Index(n);
            }
        }
        Property::GridRowEnd => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.grid_row_end = parse_grid_line(kw);
            } else if let Some(n) = try_integer(&decl.value) {
                style.grid_row_end = GridLine::Index(n);
            }
        }
        Property::GridArea => {
            // CSS Grid §8.2: `grid-area: row-start / col-start / row-end / col-end`
            // If fewer than 4 values:
            //   1 value:  all four set to that value
            //   2 values: row-end = row-start, col-end = col-start
            //   3 values: col-end = col-start
            if let CssValue::Keyword(ref kw) = decl.value {
                let parts: Vec<&str> = kw.splitn(4, '/').collect();
                let trimmed: Vec<&str> = parts.iter().map(|s| s.trim()).collect();
                let n = trimmed.len();
                let row_s = parse_grid_line(trimmed[0]);
                let col_s = if n >= 2 {
                    parse_grid_line(trimmed[1])
                } else {
                    row_s.clone()
                };
                let row_e = if n >= 3 {
                    parse_grid_line(trimmed[2])
                } else {
                    row_s.clone()
                };
                let col_e = if n >= 4 {
                    parse_grid_line(trimmed[3])
                } else {
                    col_s.clone()
                };
                style.grid_row_start = row_s;
                style.grid_column_start = col_s;
                style.grid_row_end = row_e;
                style.grid_column_end = col_e;
            }
        }
        Property::CustomProperty(_) => {
            // Custom properties stored separately in resolve_styles; no-op here.
        }
        Property::MaskImage => {
            if matches!(decl.value, CssValue::None) {
                style.mask_image = BackgroundImageVal::None;
            } else if let CssValue::Keyword(ref kw) = decl.value {
                if let Some(parsed) = parse_background_image_val(kw) {
                    style.mask_image = parsed;
                }
            }
        }
        Property::MaskSize => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_size = match kw.as_str() {
                    "cover" => BackgroundSizeVal::Cover,
                    "contain" => BackgroundSizeVal::Contain,
                    "auto" => BackgroundSizeVal::Auto,
                    _ => {
                        let parts: Vec<&str> = kw.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            let h = parse_bg_size_dim(parts[1], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, h)
                        } else if parts.len() == 1 {
                            let w = parse_bg_size_dim(parts[0], parent_fs, root_fs);
                            BackgroundSizeVal::Explicit(w, -1)
                        } else {
                            BackgroundSizeVal::Auto
                        }
                    }
                };
            }
            if matches!(decl.value, CssValue::Auto) {
                style.mask_size = BackgroundSizeVal::Auto;
            }
        }
        Property::MaskRepeat => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_repeat = match kw.as_str() {
                    "repeat-x" => BackgroundRepeatVal::RepeatX,
                    "repeat-y" => BackgroundRepeatVal::RepeatY,
                    "no-repeat" => BackgroundRepeatVal::NoRepeat,
                    _ => BackgroundRepeatVal::Repeat,
                };
            }
        }
        Property::MaskClip => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_clip = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::MaskOrigin => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.mask_origin = match kw.as_str() {
                    "padding-box" => BackgroundClipVal::PaddingBox,
                    "content-box" => BackgroundClipVal::ContentBox,
                    "text" => BackgroundClipVal::Text,
                    _ => BackgroundClipVal::BorderBox,
                };
            }
        }
        Property::MaskPosition => {
            if let CssValue::Keyword(ref kw) = decl.value {
                let (x, x_is_percent, y, y_is_percent) =
                    parse_position_pair(kw, parent_fs, root_fs, 0, true, 0, true);
                style.mask_position_x = x;
                style.mask_position_x_is_percent = x_is_percent;
                style.mask_position_y = y;
                style.mask_position_y_is_percent = y_is_percent;
            }
        }
        Property::PointerEvents => {
            if let CssValue::Keyword(ref kw) = decl.value {
                match kw.as_str() {
                    "none" => style.pointer_events = PointerEventsVal::None,
                    _ => style.pointer_events = PointerEventsVal::Auto,
                }
            }
        }
        Property::UserSelect => {
            if let CssValue::Keyword(ref kw) = decl.value {
                match kw.as_str() {
                    "none" => style.user_select = UserSelectVal::None,
                    "text" => style.user_select = UserSelectVal::Text,
                    "all" => style.user_select = UserSelectVal::All,
                    _ => style.user_select = UserSelectVal::Auto,
                }
            }
        }
        Property::BackdropFilter => {
            if matches!(decl.value, CssValue::None) {
                style.backdrop_filter = FilterVal::none();
            } else if let CssValue::Keyword(ref kw) = decl.value {
                style.backdrop_filter = parse_filter_value(kw, parent_fs, root_fs);
            }
        }
        Property::Appearance => {
            if let CssValue::Keyword(ref kw) = decl.value {
                style.appearance = if kw.eq_ignore_ascii_case("none") {
                    AppearanceVal::None
                } else {
                    AppearanceVal::Auto
                };
            } else if matches!(decl.value, CssValue::None) {
                style.appearance = AppearanceVal::None;
            }
        }
        // CSS Logical Properties — expand to physical sides (LTR assumption)
        Property::PaddingInline => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_left = px;
                style.padding_right = px;
            }
        }
        Property::PaddingBlock => {
            if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.padding_top = px;
                style.padding_bottom = px;
            }
        }
        Property::MarginInline => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_left_auto = true;
                style.margin_right_auto = true;
                style.margin_left_calc = Option::None;
                style.margin_right_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_left = 0;
                style.margin_right = 0;
                style.margin_left_calc = calc;
                style.margin_right_calc = calc;
                style.margin_left_auto = false;
                style.margin_right_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_left = px;
                style.margin_right = px;
                style.margin_left_calc = Option::None;
                style.margin_right_calc = Option::None;
                style.margin_left_auto = false;
                style.margin_right_auto = false;
            }
        }
        Property::MarginBlock => {
            if matches!(decl.value, CssValue::Auto) {
                style.margin_top_auto = true;
                style.margin_bottom_auto = true;
                style.margin_top_calc = Option::None;
                style.margin_bottom_calc = Option::None;
            } else if let CssValue::Calc(px, pct) = decl.value {
                let calc = Some((px, pct));
                style.margin_top = 0;
                style.margin_bottom = 0;
                style.margin_top_calc = calc;
                style.margin_bottom_calc = calc;
                style.margin_top_auto = false;
                style.margin_bottom_auto = false;
            } else if let Some(px) = resolve_length(&decl.value, parent_fs, root_fs) {
                style.margin_top = px;
                style.margin_bottom = px;
                style.margin_top_calc = Option::None;
                style.margin_bottom_calc = Option::None;
                style.margin_top_auto = false;
                style.margin_bottom_auto = false;
            }
        }
        // Parsed but not visually applied (accepted to prevent "unknown property" skips)
        Property::Resize => {}
    }
}

// ---------------------------------------------------------------------------
// Grid helpers
// ---------------------------------------------------------------------------

/// Decode a `CssValue` into a list of `GridTrackSize` (for `grid-template-*`).
///
/// Single-token values such as `CssValue::Length(100, Unit::Fr)` are wrapped in
/// a one-element Vec; multi-token values arrive as `CssValue::Keyword`.
fn decode_track_list(val: &CssValue) -> Vec<GridTrackSize> {
    match val {
        CssValue::Keyword(kw) => parse_track_list(kw),
        CssValue::Auto => vec![GridTrackSize::Auto],
        CssValue::Length(v, Unit::Fr) => vec![GridTrackSize::Fr(*v)],
        CssValue::Length(v, Unit::Px) => vec![GridTrackSize::Px(v / 100)],
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => {
            vec![GridTrackSize::Percent(*v)]
        }
        _ => Vec::new(),
    }
}

/// Decode a `CssValue` into a single `GridTrackSize` (for `grid-auto-*`).
fn decode_single_track(val: &CssValue) -> GridTrackSize {
    match val {
        CssValue::Keyword(kw) => parse_single_track(kw),
        CssValue::Auto => GridTrackSize::Auto,
        CssValue::Length(v, Unit::Fr) => GridTrackSize::Fr(*v),
        CssValue::Length(v, Unit::Px) => GridTrackSize::Px(v / 100),
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => GridTrackSize::Percent(*v),
        _ => GridTrackSize::Auto,
    }
}

/// Parse a CSS track-list string such as `"100px 1fr auto"` or
/// `"repeat(3, 1fr)"` into a `Vec<GridTrackSize>`.
fn parse_track_list(s: &str) -> Vec<GridTrackSize> {
    let mut tracks = Vec::new();
    let s = s.trim();

    // Handle repeat(count, size) — supports numeric counts and auto-fill/auto-fit.
    if s.starts_with("repeat(") {
        let inner = s.trim_start_matches("repeat(").trim_end_matches(')');
        let mut parts = inner.splitn(2, ',');
        let count_str = parts.next().unwrap_or("1").trim();
        let size_str = parts.next().unwrap_or("auto").trim();

        // Handle auto-fill / auto-fit keywords.
        if count_str == "auto-fill" || count_str == "auto-fit" {
            let min_px = parse_minmax_min(size_str);
            let track = if count_str == "auto-fill" {
                GridTrackSize::AutoFill { min_px }
            } else {
                GridTrackSize::AutoFit { min_px }
            };
            tracks.push(track);
            return tracks;
        }

        // Numeric repeat count.
        let count: usize = count_str.parse().unwrap_or(1).max(1);
        let track = parse_single_track(size_str);
        for _ in 0..count {
            tracks.push(track.clone());
        }
        return tracks;
    }

    // Space-separated list of track sizes (respecting parentheses).
    let tokens = split_whitespace_respecting_parens(s);
    for token in &tokens {
        tracks.push(parse_single_track(token));
    }
    tracks
}

/// Split a string on whitespace, but keep parenthesized groups together.
/// E.g. "12.25rem minmax(0, 1fr)" → ["12.25rem", "minmax(0, 1fr)"]
fn split_whitespace_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;
    // Skip leading whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                // Skip whitespace.
                while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                    i += 1;
                }
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

fn apply_padding_shorthand(
    style: &mut ComputedStyle,
    value: &str,
    parent_fs: i32,
    root_fs: i32,
) {
    let parts = split_whitespace_respecting_parens(value);
    if parts.is_empty() {
        return;
    }
    let (top, right, bottom, left) = match parts.len() {
        1 => (parts[0], parts[0], parts[0], parts[0]),
        2 => (parts[0], parts[1], parts[0], parts[1]),
        3 => (parts[0], parts[1], parts[2], parts[1]),
        _ => (parts[0], parts[1], parts[2], parts[3]),
    };
    apply_padding_side(
        &mut style.padding_top,
        &mut style.padding_top_pct,
        top,
        &Property::PaddingTop,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_right,
        &mut style.padding_right_pct,
        right,
        &Property::PaddingRight,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_bottom,
        &mut style.padding_bottom_pct,
        bottom,
        &Property::PaddingBottom,
        parent_fs,
        root_fs,
    );
    apply_padding_side(
        &mut style.padding_left,
        &mut style.padding_left_pct,
        left,
        &Property::PaddingLeft,
        parent_fs,
        root_fs,
    );
}

fn apply_padding_side(
    px_slot: &mut i32,
    pct_slot: &mut Option<i32>,
    value: &str,
    property: &Property,
    parent_fs: i32,
    root_fs: i32,
) {
    let parsed = crate::css::parse_value(property, value);
    if let CssValue::Percentage(v) = parsed {
        *pct_slot = Some(v);
    } else if let Some(px) = resolve_length(&parsed, parent_fs, root_fs) {
        *px_slot = px;
        *pct_slot = None;
    }
}

/// Extract the minimum pixel value from `minmax(300px, 1fr)` or similar.
/// Falls back to 0 if the syntax is not recognized.
fn parse_minmax_min(s: &str) -> i32 {
    let s = s.trim();
    if s.starts_with("minmax(") {
        let inner = s.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, _max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            if let Some(px_val) = min_str.strip_suffix("px") {
                return px_val.trim().parse::<f32>().unwrap_or(0.0) as i32;
            }
            if let Some(pct_val) = min_str.strip_suffix('%') {
                // Store percentage as negative to distinguish from px.
                return -(pct_val.trim().parse::<f32>().unwrap_or(0.0) as i32);
            }
        }
    }
    // Not minmax(), try as a plain track size.
    match parse_single_track(s) {
        GridTrackSize::Px(px) => px,
        _ => 0,
    }
}

/// Parse a single track size token (`"100px"`, `"1fr"`, `"50%"`, `"auto"`,
/// `"minmax(200px, 1fr)"`).
pub(crate) fn parse_single_track(token: &str) -> GridTrackSize {
    let token = token.trim();
    if token.eq_ignore_ascii_case("subgrid") {
        return GridTrackSize::Subgrid;
    }
    if token == "auto" || token.is_empty() {
        return GridTrackSize::Auto;
    }
    // Handle minmax(min, max).
    if token.starts_with("minmax(") {
        let inner = token.trim_start_matches("minmax(").trim_end_matches(')');
        if let Some((min_str, max_str)) = inner.split_once(',') {
            let min_str = min_str.trim();
            let max_str = max_str.trim();
            // Parse min component → pixel value (0 for min-content/auto).
            let min_px = if min_str == "0" {
                0
            } else if min_str == "min-content" || min_str == "max-content" || min_str == "auto" {
                0
            } else if let Some(v) = min_str.strip_suffix("px") {
                v.parse::<f32>().map(|f| f as i32).unwrap_or(0)
            } else if let Some(v) = min_str.strip_suffix("rem") {
                v.parse::<f32>().map(|f| (f * 16.0) as i32).unwrap_or(0)
            } else {
                0
            };
            // Parse max component.
            if let Some(fr_v) = max_str.strip_suffix("fr") {
                let fr = fr_v
                    .parse::<f32>()
                    .map(|f| (f * 100.0) as i32)
                    .unwrap_or(100);
                return GridTrackSize::Minmax {
                    min_px,
                    max_px: fr,
                    max_is_fr: true,
                };
            }
            // Non-fr max: treat as a track size with a minimum floor.
            let max_track = parse_single_track(max_str);
            return match max_track {
                GridTrackSize::Px(px) => GridTrackSize::Minmax {
                    min_px,
                    max_px: px,
                    max_is_fr: false,
                },
                GridTrackSize::Auto | GridTrackSize::MaxContent => GridTrackSize::Minmax {
                    min_px,
                    max_px: -1,
                    max_is_fr: false,
                },
                other => other,
            };
        }
        return GridTrackSize::Auto;
    }
    if let Some(fr_val) = token.strip_suffix("fr") {
        if let Ok(v) = fr_val.parse::<f32>() {
            return GridTrackSize::Fr((v * 100.0) as i32);
        }
    }
    if let Some(pct_val) = token.strip_suffix('%') {
        if let Ok(v) = pct_val.parse::<f32>() {
            return GridTrackSize::Percent((v * 100.0) as i32);
        }
    }
    if let Some(px_val) = token.strip_suffix("px") {
        if let Ok(v) = px_val.parse::<f32>() {
            return GridTrackSize::Px(v as i32);
        }
    }
    if let Some(rem_val) = token.strip_suffix("rem") {
        if let Ok(v) = rem_val.parse::<f32>() {
            // 1rem = 16px (root font-size default).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    if let Some(em_val) = token.strip_suffix("em") {
        if let Ok(v) = em_val.parse::<f32>() {
            // 1em ≈ 16px (approximation — grid tracks don't have font context).
            return GridTrackSize::Px((v * 16.0) as i32);
        }
    }
    // Handle fit-content(value): min(max-content, max(min-content, value))
    // Approximated as Minmax { min_px: 0, max_px: value, max_is_fr: false }.
    if token.starts_with("fit-content(") && token.ends_with(')') {
        let inner = &token["fit-content(".len()..token.len() - 1];
        let max_px = if let Some(v) = inner.trim().strip_suffix("px") {
            v.parse::<f32>().unwrap_or(0.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("rem") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else if let Some(v) = inner.trim().strip_suffix("em") {
            (v.parse::<f32>().unwrap_or(0.0) * 16.0) as i32
        } else {
            0
        };
        return GridTrackSize::Minmax {
            min_px: 0,
            max_px,
            max_is_fr: false,
        };
    }
    // Handle min-content / max-content keywords.
    if token == "min-content" {
        return GridTrackSize::MinContent;
    }
    if token == "max-content" {
        return GridTrackSize::MaxContent;
    }
    GridTrackSize::Auto
}

/// Parse a single `GridLine` from a string token (`"auto"`, `"2"`, `"span 3"`, `"areaName"`).
fn parse_grid_line(s: &str) -> GridLine {
    let s = s.trim();
    if s.is_empty() || s == "auto" {
        return GridLine::Auto;
    }
    if let Some(rest) = s.strip_prefix("span ") {
        if let Ok(n) = rest.trim().parse::<i32>() {
            return GridLine::Span(n.max(1));
        }
    }
    if let Ok(n) = s.parse::<i32>() {
        return GridLine::Index(n);
    }
    // Named grid area — store the name for resolution at layout time.
    if s.chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return GridLine::Named(String::from(s));
    }
    GridLine::Auto
}

/// Parse `"start / end"` shorthand into a pair of `GridLine` values.
fn parse_grid_line_pair(s: &str) -> (GridLine, GridLine) {
    let mut it = s.splitn(2, '/');
    let start = parse_grid_line(it.next().unwrap_or("auto"));
    let end = parse_grid_line(it.next().unwrap_or("auto"));
    (start, end)
}

/// Extract an integer from a `CssValue::Number` (fixed-point ×100).
fn try_integer(val: &CssValue) -> Option<i32> {
    if let CssValue::Number(v) = val {
        return Some(v / 100);
    }
    None
}

/// Parse an `align-items` / `justify-items` keyword into `AlignItems`.
fn parse_align_items_kw(kw: &str) -> AlignItems {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "self-start" | "left" => AlignItems::FlexStart,
        "flex-end" | "end" | "self-end" | "right" | "last baseline" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" | "first baseline" => AlignItems::Baseline,
        _ => AlignItems::Stretch,
    }
}

fn parse_inline_axis_alignment_kw(kw: &str) -> Option<InlineAxisAlignment> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "self-start" => Some(InlineAxisAlignment::Start),
        "flex-end" | "end" | "self-end" => Some(InlineAxisAlignment::End),
        "left" | "legacy" => Some(InlineAxisAlignment::Left),
        "right" => Some(InlineAxisAlignment::Right),
        "center" | "anchor-center" => Some(InlineAxisAlignment::Center),
        "stretch" | "normal" => Some(InlineAxisAlignment::Stretch),
        "baseline" | "first baseline" => Some(InlineAxisAlignment::FirstBaseline),
        "last baseline" => Some(InlineAxisAlignment::LastBaseline),
        _ => None,
    }
}

fn parse_self_alignment_kw(kw: &str) -> Option<Option<AlignItems>> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "auto" => Some(None),
        "flex-start" | "start" | "self-start" | "left" => Some(Some(AlignItems::FlexStart)),
        "flex-end" | "end" | "self-end" | "right" | "last baseline" => {
            Some(Some(AlignItems::FlexEnd))
        }
        "center" | "anchor-center" => Some(Some(AlignItems::Center)),
        "stretch" | "normal" => Some(Some(AlignItems::Stretch)),
        "baseline" | "first baseline" => Some(Some(AlignItems::Baseline)),
        "legacy" => Some(Some(AlignItems::FlexStart)),
        _ => None,
    }
}

fn parse_place_items_inline_value(
    kw: &str,
) -> (Option<InlineAxisAlignment>, Option<InlineAxisAlignment>) {
    let mut it = kw.split_whitespace();
    let first = it.next();
    let second = it.next();
    let align = first.and_then(parse_inline_axis_alignment_kw);
    let justify = second
        .and_then(parse_inline_axis_alignment_kw)
        .or_else(|| first.and_then(parse_inline_axis_alignment_kw));
    (align, justify)
}

fn parse_place_self_inline_value(
    kw: &str,
) -> (Option<InlineAxisAlignment>, Option<InlineAxisAlignment>) {
    parse_place_items_inline_value(kw)
}

fn parse_align_content_kw(kw: &str) -> Option<AlignContent> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "baseline" | "first baseline" => Some(AlignContent::FlexStart),
        "flex-end" | "end" | "last baseline" => Some(AlignContent::FlexEnd),
        "center" | "anchor-center" => Some(AlignContent::Center),
        "space-between" => Some(AlignContent::SpaceBetween),
        "space-around" => Some(AlignContent::SpaceAround),
        "space-evenly" => Some(AlignContent::SpaceEvenly),
        "stretch" | "normal" => Some(AlignContent::Stretch),
        _ => None,
    }
}

fn parse_justify_content_kw(kw: &str) -> Option<JustifyContent> {
    let kw = kw.trim();
    let kw = kw
        .strip_prefix("safe ")
        .or_else(|| kw.strip_prefix("unsafe "))
        .unwrap_or(kw)
        .trim();
    match kw {
        "flex-start" | "start" | "left" => Some(JustifyContent::FlexStart),
        "flex-end" | "end" | "right" => Some(JustifyContent::FlexEnd),
        "center" | "anchor-center" => Some(JustifyContent::Center),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" => Some(JustifyContent::SpaceAround),
        "space-evenly" => Some(JustifyContent::SpaceEvenly),
        _ => None,
    }
}

fn parse_place_items_value(kw: &str) -> (AlignItems, AlignItems) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("stretch");
    let second = parts.next().unwrap_or(first);
    (parse_align_items_kw(first), parse_align_items_kw(second))
}

fn parse_place_self_value(kw: &str) -> (Option<AlignItems>, Option<AlignItems>) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("auto");
    let second = parts.next().unwrap_or(first);
    (
        parse_self_alignment_kw(first).unwrap_or(None),
        parse_self_alignment_kw(second).unwrap_or(None),
    )
}

fn parse_place_content_value(kw: &str) -> (AlignContent, JustifyContent) {
    let mut parts = kw.split_whitespace();
    let first = parts.next().unwrap_or("stretch");
    let second = parts.next().unwrap_or(first);
    (
        parse_align_content_kw(first).unwrap_or(AlignContent::Stretch),
        parse_justify_content_kw(second).unwrap_or(JustifyContent::FlexStart),
    )
}

fn parse_overflow_keyword(kw: &str) -> OverflowVal {
    match kw {
        "visible" => OverflowVal::Visible,
        "hidden" => OverflowVal::Hidden,
        "scroll" => OverflowVal::Scroll,
        "auto" => OverflowVal::Auto,
        _ => OverflowVal::Visible,
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Transition / Animation helpers
// ---------------------------------------------------------------------------

/// Parse a CSS timing-function keyword.
pub(crate) fn parse_timing_function(s: &str) -> TimingFunction {
    match s.trim() {
        "linear" => TimingFunction::Linear,
        "ease-in" => TimingFunction::EaseIn,
        "ease-out" => TimingFunction::EaseOut,
        "ease-in-out" => TimingFunction::EaseInOut,
        "step-start" => TimingFunction::StepStart,
        "step-end" => TimingFunction::StepEnd,
        _ => TimingFunction::Ease,
    }
}

/// Apply a timing function: maps progress `t ∈ [0,1]` to `[0,1]`.
/// Input and output are multiplied by 1000 (fixed-point) to avoid floats.
pub(crate) fn apply_timing(timing: TimingFunction, t: i32) -> i32 {
    // t is in [0, 1000].
    match timing {
        TimingFunction::Linear => t,
        TimingFunction::StepStart => {
            if t > 0 {
                1000
            } else {
                0
            }
        }
        TimingFunction::StepEnd => {
            if t >= 1000 {
                1000
            } else {
                0
            }
        }
        // Cubic bezier approximations (sufficient for browser rendering).
        TimingFunction::EaseIn => {
            // cubic-bezier(0.42, 0, 1, 1) ≈ t³
            let f = t as i64;
            ((f * f * f) / (1_000_000)) as i32
        }
        TimingFunction::EaseOut => {
            // cubic-bezier(0, 0, 0.58, 1) ≈ 1 - (1-t)³
            let inv = (1000 - t) as i64;
            (1000 - (inv * inv * inv / 1_000_000)) as i32
        }
        // Ease and EaseInOut use the same cheap approximation: smoothstep.
        TimingFunction::Ease | TimingFunction::EaseInOut => {
            // smoothstep: 3t² - 2t³
            let f = t as i64;
            ((3 * f * f - 2 * f * f * f / 1000) / 1000) as i32
        }
    }
}

/// Parse a CSS time value (`"0.3s"`, `"300ms"`) to milliseconds.
fn parse_time_ms(s: &str) -> u32 {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("ms") {
        return v.trim().parse::<f32>().map(|f| f as u32).unwrap_or(0);
    }
    if let Some(v) = s.strip_suffix('s') {
        return v
            .trim()
            .parse::<f32>()
            .map(|f| (f * 1000.0) as u32)
            .unwrap_or(0);
    }
    // Pure number — assume seconds if ≤ 10, milliseconds otherwise.
    if let Ok(v) = s.parse::<f32>() {
        return if v <= 10.0 {
            (v * 1000.0) as u32
        } else {
            v as u32
        };
    }
    0
}

/// Parse a `transition` shorthand: `property duration timing delay`.
///
/// Comma-separated layers are each parsed into a `TransitionDef`.
fn parse_transition_shorthand(s: &str) -> Vec<TransitionDef> {
    let mut defs = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let mut def = TransitionDef {
            property: String::from("all"),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 {
                    def.duration_ms = ms;
                } else {
                    def.delay_ms = ms;
                }
                time_count += 1;
            } else if matches!(
                *tok,
                "linear"
                    | "ease"
                    | "ease-in"
                    | "ease-out"
                    | "ease-in-out"
                    | "step-start"
                    | "step-end"
            ) {
                def.timing = parse_timing_function(tok);
            } else if *tok != "none" {
                def.property = tok.to_ascii_lowercase();
            }
        }
        defs.push(def);
    }
    defs
}

/// Parse an `animation` shorthand: `name duration timing delay iterations direction fill-mode`.
///
/// Comma-separated layers each become an `AnimationDef`.
fn parse_animation_shorthand(s: &str) -> Vec<AnimationDef> {
    let mut defs = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let tokens: Vec<&str> = layer.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let mut def = AnimationDef {
            name: String::new(),
            duration_ms: 0,
            timing: TimingFunction::Ease,
            delay_ms: 0,
            iteration_count: 1,
            alternate: false,
        };
        let mut time_count = 0u32;
        for tok in &tokens {
            if tok.ends_with("ms") || tok.ends_with('s') {
                let ms = parse_time_ms(tok);
                if time_count == 0 {
                    def.duration_ms = ms;
                } else {
                    def.delay_ms = ms;
                }
                time_count += 1;
            } else if matches!(
                *tok,
                "linear"
                    | "ease"
                    | "ease-in"
                    | "ease-out"
                    | "ease-in-out"
                    | "step-start"
                    | "step-end"
            ) {
                def.timing = parse_timing_function(tok);
            } else if *tok == "infinite" {
                def.iteration_count = 0;
            } else if *tok == "alternate" || *tok == "alternate-reverse" {
                def.alternate = true;
            } else if matches!(
                *tok,
                "none"
                    | "normal"
                    | "reverse"
                    | "both"
                    | "forwards"
                    | "backwards"
                    | "running"
                    | "paused"
            ) {
                // Ignore direction/fill-mode/play-state keywords — not yet tracked.
            } else if let Ok(n) = tok.parse::<u32>() {
                def.iteration_count = n;
            } else if !tok.is_empty() && def.name.is_empty() {
                def.name = tok.to_ascii_lowercase();
            }
        }
        if !def.name.is_empty() {
            defs.push(def);
        }
    }
    defs
}

fn eq_ignore_ascii_case(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    for i in 0..ab.len() {
        let ca = if ab[i] >= b'A' && ab[i] <= b'Z' {
            ab[i] + 32
        } else {
            ab[i]
        };
        let cb = if bb[i] >= b'A' && bb[i] <= b'Z' {
            bb[i] + 32
        } else {
            bb[i]
        };
        if ca != cb {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Border helpers
// ---------------------------------------------------------------------------

fn resolve_border_width(val: &CssValue, parent_fs: i32, root_fs: i32, out: &mut i32) {
    if let Some(px) = resolve_length(val, parent_fs, root_fs) {
        *out = px;
    }
    if let CssValue::Keyword(ref kw) = *val {
        *out = match kw.as_str() {
            "thin" => 1,
            "medium" => 3,
            "thick" => 5,
            _ => *out,
        };
    }
}

fn resolve_border_style_val(val: &CssValue) -> BorderStyleVal {
    if matches!(*val, CssValue::None) {
        return BorderStyleVal::None;
    }
    if let CssValue::Keyword(ref kw) = *val {
        match kw.as_str() {
            "solid" => BorderStyleVal::Solid,
            "dashed" => BorderStyleVal::Dashed,
            "dotted" => BorderStyleVal::Dotted,
            "double" => BorderStyleVal::Double,
            "groove" => BorderStyleVal::Groove,
            "ridge" => BorderStyleVal::Ridge,
            "inset" => BorderStyleVal::Inset,
            "outset" => BorderStyleVal::Outset,
            "hidden" => BorderStyleVal::Hidden,
            "none" => BorderStyleVal::None,
            _ => BorderStyleVal::None,
        }
    } else {
        BorderStyleVal::None
    }
}

// ---------------------------------------------------------------------------
// Shadow parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse `box-shadow` value: `offset-x offset-y [blur [spread]] color [inset], ...`
fn parse_box_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<BoxShadowVal> {
    let mut shadows = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" {
            continue;
        }
        let mut inset = false;
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        let mut unresolved_var = false;
        // Tokenize respecting parentheses (for rgb()/rgba())
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if lower == "inset" {
                inset = true;
            } else if lower.contains("var(") {
                unresolved_var = true;
            } else if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if unresolved_var {
            continue;
        }
        if lengths.len() >= 2 {
            shadows.push(BoxShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                spread: if lengths.len() >= 4 { lengths[3] } else { 0 },
                color,
                inset,
            });
        }
    }
    shadows
}

/// Parse `text-shadow` value: `offset-x offset-y [blur] color, ...`
fn parse_text_shadows(s: &str, parent_fs: i32, root_fs: i32) -> Vec<TextShadowVal> {
    let mut shadows = Vec::new();
    for layer in split_comma_respecting_parens(s) {
        let layer = layer.trim();
        if layer.is_empty() || layer == "none" {
            continue;
        }
        let mut lengths: Vec<i32> = Vec::new();
        let mut color: u32 = 0xFF000000;
        let mut unresolved_var = false;
        let tokens = tokenize_respecting_parens(layer);
        for tok in &tokens {
            let lower = tok.to_ascii_lowercase();
            if lower.contains("var(") {
                unresolved_var = true;
            } else if let Some(c) = crate::css::try_parse_color_pub(tok) {
                color = c;
            } else if let Some(c) = crate::css::named_color_pub(&lower) {
                color = c;
            } else if let Some(dim) = crate::css::try_parse_dimension_pub(tok) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    lengths.push(px);
                }
            }
        }
        if unresolved_var {
            continue;
        }
        if lengths.len() >= 2 {
            shadows.push(TextShadowVal {
                offset_x: lengths[0],
                offset_y: lengths[1],
                blur: if lengths.len() >= 3 { lengths[2] } else { 0 },
                color,
            });
        }
    }
    shadows
}

/// Tokenize a CSS value string, keeping parenthesized groups (like `rgb(...)`) as one token.
fn tokenize_respecting_parens(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b' ' | b'\t' if depth == 0 => {
                if start < i {
                    tokens.push(&s[start..i]);
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        tokens.push(&s[start..]);
    }
    tokens
}

// ---------------------------------------------------------------------------
// Background image parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

fn extract_css_url_function(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("url(")?;
    let mut i = start + 4;
    let bytes = s.as_bytes();
    let mut quote: Option<u8> = None;
    let mut depth: i32 = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                let inner = s[start + 4..i].trim();
                return Some(String::from(inner.trim_matches('"').trim_matches('\'')));
            }
        }
        i += 1;
    }
    None
}

/// Parse `background-image` value: `url(...)`, `image-set(...)`, or `linear-gradient(...)`.
fn parse_background_image_val(s: &str) -> Option<BackgroundImageVal> {
    let trimmed = s.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "none" {
        return Some(BackgroundImageVal::None);
    }
    if lower.starts_with("url(")
        || lower.starts_with("image-set(")
        || lower.starts_with("-webkit-image-set(")
    {
        return extract_css_url_function(trimmed).map(BackgroundImageVal::Url);
    }
    if lower.starts_with("linear-gradient(") {
        let inner = lower
            .trim_start_matches("linear-gradient(")
            .trim_end_matches(')');
        return parse_linear_gradient(inner);
    }
    if lower.starts_with("radial-gradient(") {
        let inner = lower
            .trim_start_matches("radial-gradient(")
            .trim_end_matches(')');
        return parse_radial_gradient(inner);
    }
    None
}

/// Parse the interior of `linear-gradient(...)`.
fn parse_linear_gradient(inner: &str) -> Option<BackgroundImageVal> {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.is_empty() {
        return None;
    }

    let mut angle_deg: i32 = 180; // default top-to-bottom
    let mut stops = Vec::new();
    let mut start_idx = 0;

    // Check if first part is an angle or direction
    let first = parts[0].trim();
    let first_direction = first.split_once(" in ").map(|(dir, _)| dir.trim()).unwrap_or(first);
    if let Some(a) = parse_gradient_angle(first_direction) {
        angle_deg = a;
        start_idx = 1;
    } else if first_direction.starts_with("to ") {
        angle_deg = match first_direction {
            "to top" => 0,
            "to right" => 90,
            "to bottom" => 180,
            "to left" => 270,
            "to top right" | "to right top" => 45,
            "to bottom right" | "to right bottom" => 135,
            "to bottom left" | "to left bottom" => 225,
            "to top left" | "to left top" => 315,
            _ => return None,
        };
        start_idx = 1;
    } else if looks_like_invalid_gradient_angle(first_direction) {
        return None;
    }

    for i in start_idx..parts.len() {
        let part = parts[i].trim();
        let (color_str, position_str) = split_gradient_stop(part);
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))?;
        let position = if let Some(pos) = position_str {
            parse_gradient_position(pos)
        } else {
            -1 // auto
        };
        stops.push(GradientStop { color, position });
    }

    // Auto-distribute positions for stops with position == -1
    if !stops.is_empty() {
        let len = stops.len();
        if stops[0].position < 0 {
            stops[0].position = 0;
        }
        if len > 1 && stops[len - 1].position < 0 {
            stops[len - 1].position = 10000;
        }
        // Interpolate auto positions
        let mut i = 1;
        while i < len - 1 {
            if stops[i].position < 0 {
                // Find next non-auto
                let mut j = i + 1;
                while j < len && stops[j].position < 0 {
                    j += 1;
                }
                if j < len {
                    let start_pos = stops[i - 1].position;
                    let end_pos = stops[j].position;
                    let span = j - i + 1;
                    for k in i..j {
                        stops[k].position =
                            start_pos + (end_pos - start_pos) * (k - i + 1) as i32 / span as i32;
                    }
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
    }

    Some(BackgroundImageVal::LinearGradient { angle_deg, stops })
}

fn parse_radial_gradient(inner: &str) -> Option<BackgroundImageVal> {
    let parts: Vec<&str> = split_comma_respecting_parens(inner);
    if parts.len() < 2 {
        return None;
    }

    let mut center_x = 5000;
    let mut center_y = 5000;
    let mut start_idx = 0;
    let first = parts[0].trim();
    if !looks_like_color_stop(first) {
        if let Some((cx, cy)) = parse_radial_position(first) {
            center_x = cx;
            center_y = cy;
        }
        start_idx = 1;
    }

    let mut stops = Vec::new();
    for part in parts.iter().skip(start_idx) {
        let part = part.trim();
        let (color_str, position_str) = split_gradient_stop(part);
        let color = crate::css::try_parse_color_pub(color_str)
            .or_else(|| crate::css::named_color_pub(&color_str.to_ascii_lowercase()))?;
        let position = if let Some(pos) = position_str {
            parse_gradient_position(pos)
        } else {
            -1
        };
        stops.push(GradientStop { color, position });
    }
    distribute_gradient_positions(&mut stops);
    Some(BackgroundImageVal::RadialGradient {
        center_x,
        center_y,
        stops,
    })
}

fn looks_like_color_stop(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with('#')
        || lower.starts_with("rgb(")
        || lower.starts_with("rgba(")
        || lower.starts_with("hsl(")
        || lower.starts_with("hsla(")
        || lower == "transparent"
        || crate::css::named_color_pub(&lower).is_some()
}

fn parse_radial_position(s: &str) -> Option<(i32, i32)> {
    let lower = s.replace('_', " ");
    let lower = lower.to_ascii_lowercase();
    let after_at = lower
        .split_once(" at ")
        .map(|(_, pos)| pos.trim())
        .unwrap_or_else(|| lower.trim());
    if after_at.is_empty() || after_at == "circle" || after_at == "ellipse" {
        return Some((5000, 5000));
    }

    let mut cx = 5000;
    let mut cy = 5000;
    let mut saw_pos = false;
    for token in after_at.split_whitespace() {
        match token {
            "left" => {
                cx = 0;
                saw_pos = true;
            }
            "right" => {
                cx = 10000;
                saw_pos = true;
            }
            "top" => {
                cy = 0;
                saw_pos = true;
            }
            "bottom" => {
                cy = 10000;
                saw_pos = true;
            }
            "center" => saw_pos = true,
            _ if token.ends_with('%') => {
                let pct = parse_i32_prefix(&token[..token.len().saturating_sub(1)]).unwrap_or(50);
                if cx == 5000 {
                    cx = (pct * 100).clamp(0, 10000);
                } else {
                    cy = (pct * 100).clamp(0, 10000);
                }
                saw_pos = true;
            }
            _ => {}
        }
    }
    if saw_pos {
        Some((cx, cy))
    } else {
        Some((5000, 5000))
    }
}

fn parse_i32_prefix(s: &str) -> Option<i32> {
    let mut sign = 1;
    let mut value = 0i32;
    let mut saw_digit = false;
    for (idx, ch) in s.trim().chars().enumerate() {
        if idx == 0 && ch == '-' {
            sign = -1;
            continue;
        }
        if let Some(digit) = ch.to_digit(10) {
            saw_digit = true;
            value = value.saturating_mul(10).saturating_add(digit as i32);
        } else {
            break;
        }
    }
    saw_digit.then_some(value.saturating_mul(sign))
}

fn distribute_gradient_positions(stops: &mut [GradientStop]) {
    if stops.is_empty() {
        return;
    }
    let len = stops.len();
    if stops[0].position < 0 {
        stops[0].position = 0;
    }
    if len > 1 && stops[len - 1].position < 0 {
        stops[len - 1].position = 10000;
    }
    let mut i = 1;
    while i < len.saturating_sub(1) {
        if stops[i].position < 0 {
            let mut j = i + 1;
            while j < len && stops[j].position < 0 {
                j += 1;
            }
            if j < len {
                let start_pos = stops[i - 1].position;
                let end_pos = stops[j].position;
                let span = j - i + 1;
                for k in i..j {
                    stops[k].position =
                        start_pos + (end_pos - start_pos) * (k - i + 1) as i32 / span as i32;
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
}

fn split_gradient_stop(part: &str) -> (&str, Option<&str>) {
    let s = part.trim();
    if s.is_empty() {
        return (s, None);
    }
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut depth = 0u32;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b.is_ascii_whitespace() && depth == 0 {
            let color = s[..i].trim();
            let rest = s[i..].trim();
            return (color, if rest.is_empty() { None } else { Some(rest) });
        }
        i += 1;
    }
    (s, None)
}

fn parse_gradient_angle(s: &str) -> Option<i32> {
    if s.ends_with("deg") {
        return s
            .trim_end_matches("deg")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|a| a as i32);
    }
    if s.ends_with("grad") {
        return s
            .trim_end_matches("grad")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|g| (g * 0.9) as i32);
    }
    if s.ends_with("rad") {
        return s
            .trim_end_matches("rad")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|r| (r * 180.0 / core::f32::consts::PI) as i32);
    }
    if s.ends_with("turn") {
        return s
            .trim_end_matches("turn")
            .trim()
            .parse::<f32>()
            .ok()
            .map(|t| (t * 360.0) as i32);
    }
    None
}

fn looks_like_invalid_gradient_angle(s: &str) -> bool {
    let trimmed = s.trim();
    let starts_numeric = trimmed
        .as_bytes()
        .first()
        .map(|b| b.is_ascii_digit() || *b == b'+' || *b == b'-' || *b == b'.')
        .unwrap_or(false);
    starts_numeric && trimmed.as_bytes().iter().any(|b| b.is_ascii_alphabetic())
}

fn parse_gradient_position(s: &str) -> i32 {
    if s.ends_with('%') {
        if let Ok(v) = s.trim_end_matches('%').parse::<f32>() {
            return (v * 100.0) as i32;
        }
    }
    -1
}

fn split_comma_respecting_parens(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut depth: u32 = 0;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn split_transform_component_list(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' | b' ' | b'\t' | b'\n' | b'\r' if depth == 0 => {
                if start < i {
                    let part = s[start..i].trim();
                    if !part.is_empty() {
                        parts.push(part);
                    }
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        let part = s[start..].trim();
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts
}

fn parse_individual_translate(
    value: &CssValue,
    parent_fs: i32,
    root_fs: i32,
) -> Option<(i32, i32, i32, i32)> {
    match value {
        CssValue::Length(_, _)
        | CssValue::Percentage(_)
        | CssValue::Calc(_, _)
        | CssValue::Number(_) => {
            let (tx, tx_pct) = translate_component_from_value(value, parent_fs, root_fs)?;
            Some((tx, 0, tx_pct, 0))
        }
        CssValue::Keyword(s) => {
            let parts = split_transform_component_list(s);
            if parts.is_empty() {
                return None;
            }
            let (tx, tx_pct) = translate_component_from_str(parts[0], parent_fs, root_fs)?;
            let (ty, ty_pct) = if let Some(y) = parts.get(1) {
                translate_component_from_str(y, parent_fs, root_fs)?
            } else {
                (0, 0)
            };
            Some((tx, ty, tx_pct, ty_pct))
        }
        _ => None,
    }
}

fn translate_component_from_value(
    value: &CssValue,
    parent_fs: i32,
    root_fs: i32,
) -> Option<(i32, i32)> {
    match value {
        CssValue::Length(_, _) => resolve_length(value, parent_fs, root_fs).map(|px| (px, 0)),
        CssValue::Percentage(pct) => Some((0, *pct)),
        CssValue::Calc(px, pct) => Some((px / 100, *pct)),
        CssValue::Number(n) => Some((n / 100, 0)),
        _ => None,
    }
}

fn translate_component_from_str(s: &str, parent_fs: i32, root_fs: i32) -> Option<(i32, i32)> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.starts_with("calc(")
        || lower.starts_with("min(")
        || lower.starts_with("max(")
        || lower.starts_with("clamp(")
    {
        let parsed = crate::css::parse_value(&Property::Width, s);
        translate_component_from_value(&parsed, parent_fs, root_fs)
    } else {
        Some(parse_transform_translate_component(s, parent_fs))
    }
}

fn parse_individual_scale(value: &CssValue) -> Option<(i32, i32)> {
    match value {
        CssValue::Number(n) => {
            let scale = *n * 10;
            Some((scale, scale))
        }
        CssValue::Percentage(p) => {
            let scale = *p / 10;
            Some((scale, scale))
        }
        CssValue::Keyword(s) => {
            let parts = split_transform_component_list(s);
            if parts.is_empty() {
                return None;
            }
            let sx = parse_scale_component(parts[0])?;
            let sy = if let Some(y) = parts.get(1) {
                parse_scale_component(y)?
            } else {
                sx
            };
            Some((sx, sy))
        }
        _ => None,
    }
}

fn parse_scale_component(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix('%') {
        parse_decimal_fixed100(num).map(|v| v / 10)
    } else {
        parse_decimal_fixed100(s).map(|v| v * 10)
    }
}

fn parse_individual_rotate(value: &CssValue) -> Option<i32> {
    match value {
        CssValue::Number(n) => Some(*n),
        CssValue::Keyword(s) => parse_angle_deg100(s),
        _ => None,
    }
}

fn parse_angle_deg100(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix("deg") {
        parse_decimal_fixed100(num)
    } else if let Some(num) = s.strip_suffix("turn") {
        parse_decimal_fixed100(num).map(|v| v * 360)
    } else if let Some(num) = s.strip_suffix("rad") {
        parse_decimal_fixed100(num).map(|v| (v as i64 * 18000 / 314) as i32)
    } else {
        parse_decimal_fixed100(s)
    }
}

fn parse_decimal_fixed100(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let neg = s.starts_with('-');
    let s = if neg || s.starts_with('+') {
        &s[1..]
    } else {
        s
    };
    let mut int_part = 0i32;
    let mut frac = 0i32;
    let mut in_frac = false;
    let mut frac_mul = 10;
    let mut saw_digit = false;
    for &b in s.as_bytes() {
        if b == b'.' && !in_frac {
            in_frac = true;
            continue;
        }
        if !b.is_ascii_digit() {
            return None;
        }
        saw_digit = true;
        if in_frac {
            if frac_mul <= 100 {
                frac += (b - b'0') as i32 * (100 / frac_mul);
                frac_mul *= 10;
            }
        } else {
            int_part = int_part.saturating_mul(10) + (b - b'0') as i32;
        }
    }
    if !saw_digit {
        return None;
    }
    let value = int_part.saturating_mul(100).saturating_add(frac);
    Some(if neg { -value } else { value })
}

#[cfg(test)]
mod declaration_tests {
    use super::*;

    #[test]
    fn inline_style_applies_max_width_calc() {
        let decls = crate::css::parse_inline_style("max-width: calc(50% - 3px)");
        assert_eq!(decls.len(), 1);
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.max_width, None);
        assert_eq!(style.max_width_calc, Some((-300, 5000)));
    }

    #[test]
    fn calc_with_nested_var_is_resolved_after_custom_property_lookup() {
        let decls = crate::css::parse_inline_style("width: calc(956px + 2 * var(--container-spacing))");
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0].value, CssValue::Keyword(_)));

        let resolved = crate::css::parse_value(
            &Property::Width,
            "calc(956px + 2 * 20px)",
        );
        assert!(matches!(resolved, CssValue::Length(996, Unit::Px)));
    }

    #[test]
    fn calc_preserves_nested_function_parentheses() {
        let resolved = crate::css::parse_value(
            &Property::Width,
            "calc(100% - 0px - env(safe-area-inset-left) - env(safe-area-inset-right))",
        );
        assert!(matches!(resolved, CssValue::Percentage(10000)));
    }

    #[test]
    fn logical_inset_properties_expand_to_physical_offsets() {
        let decls = crate::css::parse_inline_style(
            "inset-inline: calc(.25rem * 6); inset-block: 50%; inset-inline-start: 3px",
        );
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }

        assert_eq!(style.left_offset, Some(3));
        assert_eq!(style.right_offset, Some(24));
        assert_eq!(style.top, None);
        assert_eq!(style.top_calc, Some((0, 5000)));
        assert_eq!(style.bottom_offset, None);
        assert_eq!(style.bottom_calc, Some((0, 5000)));
    }

    #[test]
    fn transform_translate_percent_uses_fixed_percent_units() {
        let decls = crate::css::parse_inline_style("transform: translateX(-50%) translateY(25%)");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx_pct, -5000);
        assert_eq!(style.transform_ty_pct, 2500);
    }

    #[test]
    fn individual_translate_percent_uses_fixed_percent_units() {
        let decls = crate::css::parse_inline_style("translate: -50% 25%");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx_pct, -5000);
        assert_eq!(style.transform_ty_pct, 2500);
    }

    #[test]
    fn individual_transform_properties_apply_translate_scale_and_rotate() {
        let decls = crate::css::parse_inline_style(
            "translate: calc(1rem * -2) 50%; scale: 1.05 95%; rotate: 0.25turn",
        );
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.transform_tx, -32);
        assert_eq!(style.transform_ty_pct, 5000);
        assert_eq!(style.transform_sx, 1050);
        assert_eq!(style.transform_sy, 950);
        assert_eq!(style.transform_rotate, 9000);
    }

    #[test]
    fn border_radius_accepts_percentage_for_avatar_circles() {
        let decls = crate::css::parse_inline_style("border-radius: 50%");
        let mut style = default_style();
        for decl in &decls {
            apply_declaration(&mut style, decl, None, 16, 16);
        }
        assert_eq!(style.border_top_left_radius, -5000);
        assert_eq!(style.border_top_right_radius, -5000);
        assert_eq!(style.border_bottom_right_radius, -5000);
        assert_eq!(style.border_bottom_left_radius, -5000);
    }

    #[test]
    fn invalid_gradient_angle_is_rejected() {
        assert!(parse_background_image_val("linear-gradient(90degree, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(100gradian, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(1.57radian, red, red)").is_none());
        assert!(parse_background_image_val("linear-gradient(0.25turns, red, red)").is_none());
    }

    #[test]
    fn linear_gradient_accepts_space_separated_function_colors() {
        let parsed = parse_background_image_val(
            "linear-gradient(to right, oklch(0.65 0.2 280), rgb(59 130 246 / 1))",
        );
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::LinearGradient { ref stops, .. }) if stops.len() == 2
        ));
    }

    #[test]
    fn linear_gradient_accepts_modern_color_interpolation_space() {
        let parsed = parse_background_image_val(
            "linear-gradient(to right in oklab, #863bff 0%, #47bfff 100%)",
        );
        match parsed {
            Some(BackgroundImageVal::LinearGradient { angle_deg, ref stops }) => {
                assert_eq!(angle_deg, 90);
                assert_eq!(stops.len(), 2);
                assert_eq!(stops[0].color, 0xFF863BFF);
                assert_eq!(stops[1].color, 0xFF47BFFF);
            }
            _ => panic!("expected modern color-space gradient"),
        }
    }

    #[test]
    fn background_url_preserves_asset_path_case() {
        let parsed = parse_background_image_val("url('/Images/HeroLarge.PNG')");
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/Images/HeroLarge.PNG"
        ));
    }

    #[test]
    fn background_image_set_uses_first_url_candidate() {
        let parsed =
            parse_background_image_val("image-set(url('/hero.avif') 1x, url('/hero@2x.avif') 2x)");
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/hero.avif"
        ));

        let parsed = parse_background_image_val(
            "-webkit-image-set(url(\"/Promo/Hero.JPG\") 1x, url(\"/Promo/Hero_2x.JPG\") 2x)",
        );
        assert!(matches!(
            parsed,
            Some(BackgroundImageVal::Url(ref src)) if src == "/Promo/Hero.JPG"
        ));
    }
}

fn resolve_border_radius(value: &CssValue, parent_fs: i32, root_fs: i32) -> Option<i32> {
    match value {
        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => Some(-(*v).max(0)),
        _ => resolve_length(value, parent_fs, root_fs),
    }
}

fn parse_bg_size_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    let s = s.trim();
    if s == "auto" {
        return -1;
    }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    -1
}

fn parse_bg_position_part(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    match s {
        "left" | "top" => 0,
        "center" => 5000,            // 50% * 100
        "right" | "bottom" => 10000, // 100% * 100
        _ => {
            if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    return px;
                }
            }
            0
        }
    }
}

fn parse_position_component(
    s: &str,
    parent_fs: i32,
    root_fs: i32,
    default_value: i32,
    default_is_percent: bool,
) -> (i32, bool) {
    let s = s.trim();
    match s {
        "left" | "top" => (0, true),
        "center" => (5000, true),
        "right" | "bottom" => (10000, true),
        _ => {
            if let Some(stripped) = s.strip_suffix('%') {
                if let Ok(v) = stripped.trim().parse::<i32>() {
                    return (v * 100, true);
                }
            }
            if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
                if matches!(
                    dim,
                    CssValue::Length(_, Unit::Percent) | CssValue::Percentage(_)
                ) {
                    match dim {
                        CssValue::Length(v, Unit::Percent) | CssValue::Percentage(v) => {
                            return (v, true);
                        }
                        _ => {}
                    }
                }
                if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                    return (px, false);
                }
            }
            (default_value, default_is_percent)
        }
    }
}

fn parse_position_pair(
    s: &str,
    parent_fs: i32,
    root_fs: i32,
    default_x: i32,
    default_x_is_percent: bool,
    default_y: i32,
    default_y_is_percent: bool,
) -> (i32, bool, i32, bool) {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return (
            default_x,
            default_x_is_percent,
            default_y,
            default_y_is_percent,
        );
    }

    if parts.len() == 1 {
        let part = parts[0];
        if matches!(part, "top" | "bottom") {
            let (y, y_is_percent) =
                parse_position_component(part, parent_fs, root_fs, default_y, default_y_is_percent);
            return (default_x, default_x_is_percent, y, y_is_percent);
        }
        let (x, x_is_percent) =
            parse_position_component(part, parent_fs, root_fs, default_x, default_x_is_percent);
        return (x, x_is_percent, default_y, default_y_is_percent);
    }

    let (x, x_is_percent) = parse_position_component(
        parts[0],
        parent_fs,
        root_fs,
        default_x,
        default_x_is_percent,
    );
    let (y, y_is_percent) = parse_position_component(
        parts[1],
        parent_fs,
        root_fs,
        default_y,
        default_y_is_percent,
    );
    (x, x_is_percent, y, y_is_percent)
}

// ---------------------------------------------------------------------------
// Filter parsing (litehtml-inspired)
// ---------------------------------------------------------------------------

/// Parse a CSS `filter` value like `blur(5px) grayscale(50%) brightness(120%)`.
fn parse_filter_value(s: &str, parent_fs: i32, root_fs: i32) -> FilterVal {
    let mut f = FilterVal::none();
    let s = s.trim();
    if s == "none" {
        return f;
    }

    // Tokenize function calls like "blur(5px)" "grayscale(50%)"
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Read function name
        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b'(' && bytes[pos] != b' ' {
            pos += 1;
        }
        let name = &s[name_start..pos];
        if pos >= bytes.len() || bytes[pos] != b'(' {
            break;
        }
        pos += 1; // skip '('

        // Read argument until ')'
        let arg_start = pos;
        while pos < bytes.len() && bytes[pos] != b')' {
            pos += 1;
        }
        let arg = &s[arg_start..pos];
        if pos < bytes.len() {
            pos += 1;
        } // skip ')'

        let arg = arg.trim();
        match name {
            "blur" => {
                if let Some(dim) = crate::css::try_parse_dimension_pub(arg) {
                    if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
                        f.blur_px = px.max(0);
                    }
                }
            }
            "brightness" => {
                f.brightness = parse_filter_pct(arg);
            }
            "contrast" => {
                f.contrast = parse_filter_pct(arg);
            }
            "grayscale" => {
                f.grayscale = parse_filter_pct(arg);
            }
            "saturate" => {
                f.saturate = parse_filter_pct(arg);
            }
            "sepia" => {
                f.sepia = parse_filter_pct(arg);
            }
            "opacity" => {
                f.opacity = parse_filter_pct(arg);
            }
            "invert" => {
                f.invert = parse_filter_pct(arg);
            }
            "hue-rotate" => {
                let deg_str = arg.trim_end_matches("deg").trim();
                if let Ok(v) = deg_str.parse::<i32>() {
                    f.hue_rotate = v;
                }
            }
            _ => {} // drop-shadow, url() — not supported
        }
    }
    f
}

/// Parse a filter function argument as percentage (100% = 10000).
fn parse_filter_pct(s: &str) -> i32 {
    let s = s.trim();
    if s.ends_with('%') {
        let num = &s[..s.len() - 1];
        if let Ok(v) = num.parse::<i32>() {
            return v * 100;
        }
    }
    // Try as decimal (0.5 = 5000, 1.0 = 10000)
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let CssValue::Number(v) = dim {
            return v * 100; // v is already *100
        }
    }
    10000
}

/// Parse a simple float/int string to fixed-point * 100 (returns Option).
fn try_parse_simple_float(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        match dim {
            CssValue::Number(v) => return Some(v),
            CssValue::Length(v, _) => return Some(v),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Clip-path parsing
// ---------------------------------------------------------------------------

/// Parse `clip-path: circle(...)` or `clip-path: inset(...)`.
/// Parse `clip: rect(top, right, bottom, left)` into [top, right, bottom, left] in px.
/// Also accepts space-separated values (legacy syntax).
fn parse_clip_rect(s: &str, parent_fs: i32, root_fs: i32) -> Option<[i32; 4]> {
    let s = s.trim();
    // Must start with "rect("
    let inner = s.strip_prefix("rect(")?.trim_end_matches(')').trim();
    // Values can be comma- or space-separated.
    let parts: Vec<&str> = if inner.contains(',') {
        inner.split(',').map(|p| p.trim()).collect()
    } else {
        inner.split_whitespace().collect()
    };
    if parts.len() < 4 {
        return None;
    }
    let mut vals = [0i32; 4];
    for (i, p) in parts[..4].iter().enumerate() {
        vals[i] = if *p == "auto" {
            0
        } else {
            let cv = crate::css::parse_value(&crate::css::Property::Top, p);
            resolve_length(&cv, parent_fs, root_fs).unwrap_or(0)
        };
    }
    Some(vals)
}

/// Parse a CSS `content` property value.
///
/// Handles:
/// - Quoted strings: `"text"` or `'text'`
/// - `none` / `normal` → (None, None)
/// - `counter(name)` / `counter(name, style)` → encoded as `\x01COUNTER:name\x01` in text
/// - `counters(name, sep)` → encoded as `\x01COUNTER:name\x01`
/// - `url("...")` → (Some(""), Some(url))
/// - Multi-value: `"(" counter(n) ")"` → concatenated result
/// - Icon/unicode: `"\e900"` → kept as-is (Unicode escape)
///
/// Returns `(text_content, url_content)`.
pub(crate) fn parse_content_value(raw: &str) -> (Option<String>, Option<String>) {
    let s = raw.trim();
    if s.is_empty() {
        return (None, None);
    }

    let lower = s.to_ascii_lowercase();
    if lower == "none" || lower == "normal" || lower == "no-open-quote" || lower == "no-close-quote"
    {
        return (None, None);
    }

    // Pure url(...) without any surrounding text
    if lower.starts_with("url(") && !lower.contains('"') && !lower.contains('\'')
        || lower.starts_with("url(\"")
        || lower.starts_with("url('")
    {
        // Check if the whole value is url(...)
        let trimmed = s.trim_end_matches(')').trim();
        if trimmed.starts_with("url(") || trimmed.to_ascii_lowercase().starts_with("url(") {
            let url = extract_css_url(s);
            return (Some(String::new()), Some(url));
        }
    }

    // Multi-value parser: iterate over tokens
    let mut result = String::new();
    let mut url_found: Option<String> = None;
    let bytes = s.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        if bytes[pos] == b'"' || bytes[pos] == b'\'' {
            // Quoted string: collect content between quotes
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote {
                pos += 1;
            }
            let text = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            // Unescape CSS unicode escapes like \e900
            result.push_str(&unescape_css_string(text));
            if pos < bytes.len() {
                pos += 1;
            } // skip closing quote
        } else if rest_starts_with_ci(bytes, pos, b"counter(") {
            pos += 8;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"counters(") {
            pos += 9;
            let (name, new_pos) = read_counter_name(bytes, pos);
            pos = new_pos;
            result.push('\x01');
            result.push_str("COUNTER:");
            result.push_str(&name);
            result.push('\x01');
        } else if rest_starts_with_ci(bytes, pos, b"url(") {
            // url(...) inside multi-value content
            pos += 4;
            // Skip past closing paren
            let mut depth = 1usize;
            let url_start = pos;
            while pos < bytes.len() && depth > 0 {
                if bytes[pos] == b'(' {
                    depth += 1;
                } else if bytes[pos] == b')' {
                    depth -= 1;
                }
                if depth > 0 {
                    pos += 1;
                }
            }
            let url_raw = core::str::from_utf8(&bytes[url_start..pos]).unwrap_or("");
            let url = url_raw.trim().trim_matches('"').trim_matches('\'');
            url_found = Some(String::from(url));
            if pos < bytes.len() {
                pos += 1;
            }
        } else if rest_starts_with_ci(bytes, pos, b"open-quote") {
            result.push('\u{201C}');
            pos += 10;
        } else if rest_starts_with_ci(bytes, pos, b"close-quote") {
            result.push('\u{201D}');
            pos += 11;
        } else if rest_starts_with_ci(bytes, pos, b"attr(") {
            // attr(name) — skip for now
            pos += 5;
            while pos < bytes.len() && bytes[pos] != b')' {
                pos += 1;
            }
            if pos < bytes.len() {
                pos += 1;
            }
        } else {
            // Unknown token — skip to next whitespace or quote
            while pos < bytes.len()
                && bytes[pos] != b' '
                && bytes[pos] != b'\t'
                && bytes[pos] != b'"'
                && bytes[pos] != b'\''
            {
                pos += 1;
            }
        }
    }

    if result.is_empty() && url_found.is_none() {
        // Nothing useful parsed — treat the raw value as a plain text string
        // (handles icon font chars stored as unquoted keywords)
        let stripped = s.trim_matches('"').trim_matches('\'');
        if stripped == "none" || stripped == "normal" {
            return (None, None);
        }
        if stripped.is_empty() {
            return (Some(String::new()), None);
        }
        return (Some(String::from(stripped)), None);
    }

    let text = if result.is_empty() {
        Some(String::new())
    } else {
        Some(result)
    };
    (text, url_found)
}

/// Unescape CSS string escapes: `\e900` → U+E900, `\n` → newline, etc.
fn unescape_css_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 1;
            // Hex escape: up to 6 hex digits
            if bytes[i].is_ascii_hexdigit() {
                let start = i;
                let mut hex_end = i;
                while hex_end < bytes.len()
                    && hex_end - start < 6
                    && bytes[hex_end].is_ascii_hexdigit()
                {
                    hex_end += 1;
                }
                let hex_str = core::str::from_utf8(&bytes[start..hex_end]).unwrap_or("0");
                if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                }
                i = hex_end;
                // Skip optional single whitespace after hex escape
                if i < bytes.len()
                    && (bytes[i] == b' '
                        || bytes[i] == b'\n'
                        || bytes[i] == b'\r'
                        || bytes[i] == b'\t')
                {
                    i += 1;
                }
            } else {
                // Simple escape: \n, \t, \", \\, etc.
                let c = match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'r' => '\r',
                    b => b as char,
                };
                out.push(c);
                i += 1;
            }
        } else {
            // Pass through non-escape bytes as UTF-8.
            // Collect a run of non-backslash bytes and decode them.
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            if let Ok(s) = core::str::from_utf8(&bytes[start..i]) {
                out.push_str(s);
            } else {
                // Fallback: push individual ASCII chars
                for b in &bytes[start..i] {
                    if *b < 128 {
                        out.push(*b as char);
                    }
                }
            }
        }
    }
    out
}

/// Check if `bytes[pos..]` starts with `prefix` (case-insensitive ASCII).
fn rest_starts_with_ci(bytes: &[u8], pos: usize, prefix: &[u8]) -> bool {
    if pos + prefix.len() > bytes.len() {
        return false;
    }
    for (i, &pb) in prefix.iter().enumerate() {
        let b = bytes[pos + i];
        let bl = if b >= b'A' && b <= b'Z' { b + 32 } else { b };
        let pl = if pb >= b'A' && pb <= b'Z' {
            pb + 32
        } else {
            pb
        };
        if bl != pl {
            return false;
        }
    }
    true
}

/// Read a counter name from bytes starting at `pos` (inside counter(...) after the `(`).
/// Returns (name, new_pos) where new_pos is after the closing `)`.
fn read_counter_name(bytes: &[u8], mut pos: usize) -> (String, usize) {
    // Skip whitespace
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    let start = pos;
    // Read until comma or closing paren
    while pos < bytes.len() && bytes[pos] != b',' && bytes[pos] != b')' {
        pos += 1;
    }
    let name = core::str::from_utf8(&bytes[start..pos])
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Skip past closing paren (and anything between comma and paren)
    let mut depth = 1i32;
    while pos < bytes.len() && depth > 0 {
        if bytes[pos] == b'(' {
            depth += 1;
        } else if bytes[pos] == b')' {
            depth -= 1;
        }
        pos += 1;
    }
    (name, pos)
}

/// Extract the URL from `url("...")` or `url(...)`.
fn extract_css_url(s: &str) -> String {
    let s = s.trim();
    let inner = if let Some(rest) = s.strip_prefix("url(") {
        rest.trim_end_matches(')').trim()
    } else if let Some(rest) = s.to_ascii_lowercase().strip_prefix("url(").map(|_| &s[4..]) {
        rest.trim_end_matches(')').trim()
    } else {
        s
    };
    String::from(inner.trim_matches('"').trim_matches('\''))
}

fn parse_clip_path_value(s: &str, parent_fs: i32, root_fs: i32) -> ClipPathVal {
    let s = s.trim();
    if s == "none" {
        return ClipPathVal::None;
    }

    if s.starts_with("circle(") {
        let inner = s.trim_start_matches("circle(").trim_end_matches(')').trim();
        // "50px at 100px 100px" or "50%" or "50px"
        let parts: Vec<&str> = inner.split_whitespace().collect();
        let radius = if !parts.is_empty() {
            resolve_clip_dim(parts[0], parent_fs, root_fs)
        } else {
            50
        };
        let (cx, cy) = if parts.len() >= 4 && parts[1] == "at" {
            (
                resolve_clip_dim(parts[2], parent_fs, root_fs),
                resolve_clip_dim(parts[3], parent_fs, root_fs),
            )
        } else {
            (50, 50)
        }; // default: center (percentage-like)
        return ClipPathVal::Circle { radius, cx, cy };
    }

    if s.starts_with("inset(") {
        let inner = s.trim_start_matches("inset(").trim_end_matches(')').trim();
        // Split on "round" for optional border-radius
        let (dims_str, radius) = if let Some(round_pos) = inner.find("round") {
            let r_str = inner[round_pos + 5..].trim();
            let r = resolve_clip_dim(r_str, parent_fs, root_fs);
            (&inner[..round_pos], r)
        } else {
            (inner, 0)
        };
        let parts: Vec<&str> = dims_str.split_whitespace().collect();
        let (t, r, b, l) = match parts.len() {
            1 => {
                let v = resolve_clip_dim(parts[0], parent_fs, root_fs);
                (v, v, v, v)
            }
            2 => {
                let tb = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                (tb, lr, tb, lr)
            }
            3 => {
                let t = resolve_clip_dim(parts[0], parent_fs, root_fs);
                let lr = resolve_clip_dim(parts[1], parent_fs, root_fs);
                let b = resolve_clip_dim(parts[2], parent_fs, root_fs);
                (t, lr, b, lr)
            }
            _ => (
                resolve_clip_dim(parts[0], parent_fs, root_fs),
                resolve_clip_dim(parts[1], parent_fs, root_fs),
                resolve_clip_dim(parts[2], parent_fs, root_fs),
                resolve_clip_dim(parts[3], parent_fs, root_fs),
            ),
        };
        return ClipPathVal::Inset {
            top: t,
            right: r,
            bottom: b,
            left: l,
            radius,
        };
    }

    ClipPathVal::None
}

fn resolve_clip_dim(s: &str, parent_fs: i32, root_fs: i32) -> i32 {
    if let Some(dim) = crate::css::try_parse_dimension_pub(s) {
        if let Some(px) = resolve_length(&dim, parent_fs, root_fs) {
            return px;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Grid template areas parsing
// ---------------------------------------------------------------------------

/// Parse `grid-template-areas` value into named grid areas.
/// Example: `'header header' 'sidebar content' 'footer footer'`
/// Returns a list of GridArea with 1-based line numbers.
fn parse_grid_template_areas_value(s: &str) -> Vec<GridArea> {
    let mut areas: Vec<GridArea> = Vec::new();
    let mut row: i32 = 1;

    // Extract each quoted row string.
    let mut pos = 0;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        // Find start of quoted string.
        if bytes[pos] == b'\'' || bytes[pos] == b'"' {
            let quote = bytes[pos];
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos] != quote {
                pos += 1;
            }
            let row_str = core::str::from_utf8(&bytes[start..pos]).unwrap_or("");
            if pos < bytes.len() {
                pos += 1;
            } // skip closing quote

            // Parse cells in this row.
            let cells: Vec<&str> = row_str.split_whitespace().collect();
            for (col_idx, &name) in cells.iter().enumerate() {
                if name == "." {
                    continue;
                } // empty cell
                let col = col_idx as i32 + 1; // 1-based

                // Check if this area already exists — extend it.
                if let Some(existing) = areas.iter_mut().find(|a| a.name == name) {
                    // Extend the area to cover this cell.
                    if row + 1 > existing.row_end {
                        existing.row_end = row + 1;
                    }
                    if col + 1 > existing.col_end {
                        existing.col_end = col + 1;
                    }
                    if row < existing.row_start {
                        existing.row_start = row;
                    }
                    if col < existing.col_start {
                        existing.col_start = col;
                    }
                } else {
                    areas.push(GridArea {
                        name: String::from(name),
                        row_start: row,
                        col_start: col,
                        row_end: row + 1,
                        col_end: col + 1,
                    });
                }
            }
            row += 1;
        } else {
            pos += 1;
        }
    }
    areas
}

#[cfg(test)]
mod layout_regression_tests {
    use super::*;

    #[test]
    fn resolves_negative_margins_for_replaced_elements() {
        let dom = crate::html::parse(r#"<img id="t1" src="x">"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            img { margin: 10px; }
            #t1 {
                padding-left: 20px;
                margin-left: -10px;
                padding-bottom: 20px;
                margin-bottom: -10px;
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let img_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Img, .. }))
            .expect("img node");
        let style = &styles[img_id];

        assert_eq!(style.margin_top, 10);
        assert_eq!(style.margin_right, 10);
        assert_eq!(style.margin_bottom, -10);
        assert_eq!(style.margin_left, -10);
        assert_eq!(style.padding_left, 20);
        assert_eq!(style.padding_bottom, 20);
    }

    #[test]
    fn calc_division_by_negative_number_after_var_resolution() {
        let dom = crate::html::parse(r#"<section id="home"></section>"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            :root {
                --viewport-width: 800px;
                --viewport-height: 600px;
                --padding-width: 15px;
                --border-width: 6px;
            }
            section {
                width: var(--viewport-width);
                height: var(--viewport-height);
                margin-top: calc(var(--viewport-height) / -2 - var(--padding-width) - var(--border-width));
                margin-left: calc(var(--viewport-width) / -2 - var(--padding-width) - var(--border-width));
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 900, &mut inline_style_cache);
        let section_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Section, .. }))
            .expect("section node");
        let style = &styles[section_id];

        assert_eq!(style.width, Some(800));
        assert_eq!(style.height, Some(600));
        assert_eq!(style.margin_top, -321);
        assert_eq!(style.margin_left, -421);
    }

    #[test]
    fn picture_defaults_to_inline_like_replaced_media_container() {
        let dom = crate::html::parse(r#"<picture><img src="x"></picture>"#);
        let stylesheet = crate::css::parse_stylesheet("");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let picture_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Picture, .. }))
            .expect("picture node");

        assert!(matches!(styles[picture_id].display, Display::Inline));
    }

    #[test]
    fn early_inheritance_seed_preserves_explicit_ua_styles() {
        let dom = crate::html::parse(r#"<center><div id="child">x</div></center><h1>Title</h1>"#);
        let stylesheet = crate::css::parse_stylesheet("center { background: #eee; }");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);

        let child_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { tag: Tag::Div, attrs }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "child")
                )
            })
            .expect("child div");
        let h1_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::H1, .. }))
            .expect("h1");

        assert!(matches!(styles[child_id].text_align, TextAlignVal::Center));
        assert_eq!(styles[h1_id].font_size, 32);
        assert!(matches!(styles[h1_id].font_weight, FontWeight::Bold));
    }

    #[test]
    fn dialog_without_open_is_not_rendered() {
        let dom = crate::html::parse(r#"<dialog class="modal-dialog">Modal</dialog>"#);
        let stylesheet = crate::css::parse_stylesheet("dialog { display: block; }");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 800, 600, &mut inline_style_cache);
        let dialog_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::Dialog, .. }))
            .expect("dialog node");

        assert!(matches!(styles[dialog_id].display, Display::None));
    }

    #[test]
    fn skip_link_is_offscreen_until_focused() {
        let dom =
            crate::html::parse(r##"<a class="skip-link" href="#main">Weiter zum Hauptinhalt</a>"##);
        let stylesheet = crate::css::parse_stylesheet(
            ".skip-link { display: flex; position: absolute; padding: 24px; }",
        );
        let prepared = PreparedStylesheets::prepare(&[&stylesheet], 800, 600);
        let link_id = dom
            .nodes
            .iter()
            .position(|node| matches!(node.node_type, NodeType::Element { tag: Tag::A, .. }))
            .expect("skip link node");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles_prepared_with_state(
            &dom,
            &prepared,
            800,
            600,
            &mut inline_style_cache,
            &SelectorState::default(),
        );

        assert_eq!(styles[link_id].left_offset, Some(-10000));
        assert_eq!(styles[link_id].width, Some(1));
        assert_eq!(styles[link_id].height, Some(1));

        let mut focused = SelectorState::default();
        focused.focused_node = Some(link_id);
        inline_style_cache.clear();
        let (focused_styles, _) = resolve_styles_prepared_with_state(
            &dom,
            &prepared,
            800,
            600,
            &mut inline_style_cache,
            &focused,
        );

        assert_ne!(focused_styles[link_id].left_offset, Some(-10000));
        assert_eq!(focused_styles[link_id].padding_left, 24);
    }

    #[test]
    fn responsive_escaped_tailwind_classes_override_base_display() {
        let dom = crate::html::parse(
            r#"<div id="md" class="hidden md:flex"></div><div id="xl" class="hidden xl:inline"></div>"#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            .hidden { display: none; }
            @media (min-width: 768px) { .md\:flex { display: flex; } }
            @media (min-width: 1280px) { .xl\:inline { display: inline; } }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let md_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "md")
                )
            })
            .expect("md node");
        let xl_id = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "xl")
                )
            })
            .expect("xl node");

        assert!(matches!(styles[md_id].display, Display::Flex));
        assert!(matches!(styles[xl_id].display, Display::Inline));
    }

    #[test]
    fn descendant_display_rule_requires_matching_ancestor() {
        let dom = crate::html::parse(
            r#"
            <div>
                <div id="closed" class="oMByyf"></div>
            </div>
            <div class="KWUYAe">
                <div id="open" class="oMByyf"></div>
            </div>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            ".oMByyf { display: none; } .KWUYAe .oMByyf { display: block; }",
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let find = |id_value: &str| {
            dom.nodes
                .iter()
                .position(|node| {
                    matches!(
                        &node.node_type,
                        NodeType::Element { attrs, .. }
                            if attrs.iter().any(|a| a.name == "id" && a.value == id_value)
                    )
                })
                .expect("node")
        };

        assert!(matches!(styles[find("closed")].display, Display::None));
        assert!(matches!(styles[find("open")].display, Display::Block));
    }

    #[test]
    fn body_attribute_rule_custom_properties_inherit_to_children() {
        let dom = crate::html::parse(
            r#"
            <body data-color-brand="bild">
                <nav><span id="nav-text">STARTSEITE</span></nav>
            </body>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            body[data-color-brand=bild] {
                --navi-font: Gotham XNarrow, Arial Narrow, sans-serif;
            }
            nav span { font-family: var(--navi-font); }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let nav_text = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "nav-text")
                )
            })
            .expect("nav text");

        assert_eq!(
            styles[nav_text].font_family.as_deref(),
            Some("Gotham XNarrow, Arial Narrow, sans-serif")
        );
    }

    #[test]
    fn custom_element_property_var_resolves_for_overflow_x() {
        let dom = crate::html::parse(
            r#"
            <a-scroll-container>
                <div id="scroller" class="scroll-container"></div>
            </a-scroll-container>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            a-scroll-container {
                --ho-scroll-container-overflow-x: scroll;
            }
            a-scroll-container .scroll-container {
                overflow-x: var(--ho-scroll-container-overflow-x);
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let scroller = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "scroller")
                )
            })
            .expect("scroller node");

        assert!(matches!(styles[scroller].overflow_x, OverflowVal::Scroll));
    }

    #[test]
    fn layered_root_custom_properties_resolve_for_tailwind_utilities() {
        let dom = crate::html::parse(
            r#"
            <main id="app" class="min-h-screen bg-surface-950 text-white">
                CoreVM
            </main>
            "#,
        );
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            @layer theme {
                :root, :host {
                    --color-white: #fff;
                    --color-surface-950: #020617;
                }
            }
            @layer utilities {
                .min-h-screen { min-height: 100vh; }
                .bg-surface-950 { background-color: var(--color-surface-950); }
                .text-white { color: var(--color-white); }
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1280, 900, &mut inline_style_cache);
        let app = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "app")
                )
            })
            .expect("app node");

        assert_eq!(styles[app].color, 0xFFFFFFFF);
        assert_eq!(styles[app].background_color, 0xFF020617);
        assert_eq!(styles[app].min_height, 900);
    }

    #[test]
    fn custom_property_chain_resolves_nested_var_inside_calc() {
        let dom = crate::html::parse(r#"<svg id="icon" class="icon"></svg>"#);
        let stylesheet = crate::css::parse_stylesheet(
            r#"
            :root {
                --font-size-base: 1rem;
                --scaling-factor-xxxs: 27/40;
                --baseline-down-04: calc(var(--font-size-base) * var(--scaling-factor-xxxs));
                --text-xxs: var(--baseline-down-04);
            }
            .icon {
                width: var(--text-xxs);
                height: var(--text-xxs);
            }
            "#,
        );
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1280, 900, &mut inline_style_cache);
        let icon = dom
            .nodes
            .iter()
            .position(|node| {
                matches!(
                    &node.node_type,
                    NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "icon")
                )
            })
            .expect("icon node");

        assert_eq!(styles[icon].width, Some(10));
        assert_eq!(styles[icon].height, Some(10));
    }

    #[test]
    fn tailwind_display_fallback_handles_missing_responsive_rules() {
        let dom = crate::html::parse(
            r#"<div id="mobile" class="flex md:hidden"></div><div id="desktop" class="hidden xl:inline"></div>"#,
        );
        let stylesheet = crate::css::parse_stylesheet("");
        let mut inline_style_cache = Vec::new();
        let (styles, _) = resolve_styles(&dom, &[&stylesheet], 1365, 700, &mut inline_style_cache);
        let find = |id_value: &str| {
            dom.nodes
                .iter()
                .position(|node| {
                    matches!(
                        &node.node_type,
                        NodeType::Element { attrs, .. }
                            if attrs.iter().any(|a| a.name == "id" && a.value == id_value)
                    )
                })
                .expect("node")
        };

        assert!(matches!(styles[find("mobile")].display, Display::None));
        assert!(matches!(styles[find("desktop")].display, Display::Inline));
    }
}
