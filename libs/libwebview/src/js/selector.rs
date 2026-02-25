//! CSS selector matching for querySelector / querySelectorAll.
//!
//! Supports compound selectors with combinators:
//!   - Descendant (space): `div p`
//!   - Child (`>`): `div > p`
//!   - Adjacent sibling (`+`): `h1 + p`
//!   - General sibling (`~`): `h1 ~ p`
//!
//! Simple selectors: `#id`, `.class`, `tag`, `tag.class`, `tag#id`,
//! `[attr]`, `[attr=val]`, `*`, and comma-separated groups.

use crate::dom::{Dom, NodeType, Tag};
use alloc::vec::Vec;

/// Combinator between two compound selectors.
#[derive(Clone, Copy, PartialEq)]
enum Combinator {
    /// Descendant (whitespace): ancestor … descendant.
    Descendant,
    /// Child (`>`): parent > child.
    Child,
    /// Adjacent sibling (`+`): prev + next.
    Adjacent,
    /// General sibling (`~`): prev ~ later.
    General,
}

/// A single segment in a parsed selector chain.
/// The chain is stored left-to-right: `div > p .cls` → [(None,"div"), (Child,"p"), (Desc,".cls")]
struct SelectorPart<'a> {
    /// Combinator connecting this part to the *previous* part (None for the leftmost).
    combinator: Option<Combinator>,
    /// The compound selector string (e.g. "div", ".class", "#id", "tag.class").
    compound: &'a str,
}

/// Find the first element matching a CSS selector.
pub fn find_first(dom: &Dom, selector: &str) -> Option<usize> {
    let sel = selector.trim();
    if sel.is_empty() { return None; }

    // Comma-separated: try each group.
    if sel.contains(',') {
        for group in sel.split(',') {
            if let Some(id) = find_first(dom, group.trim()) {
                return Some(id);
            }
        }
        return None;
    }

    let parts = parse_selector(sel);
    if parts.is_empty() { return None; }

    for i in 0..dom.nodes.len() {
        if matches_parsed(dom, i, &parts) {
            return Some(i);
        }
    }
    None
}

/// Find all elements matching a CSS selector.
pub fn find_all(dom: &Dom, selector: &str) -> Vec<usize> {
    let sel = selector.trim();
    let mut results = Vec::new();
    if sel.is_empty() { return results; }

    // Comma-separated: union of each group.
    if sel.contains(',') {
        for group in sel.split(',') {
            let group = group.trim();
            if group.is_empty() { continue; }
            let parts = parse_selector(group);
            if parts.is_empty() { continue; }
            for i in 0..dom.nodes.len() {
                if matches_parsed(dom, i, &parts) && !results.contains(&i) {
                    results.push(i);
                }
            }
        }
        return results;
    }

    let parts = parse_selector(sel);
    if parts.is_empty() { return results; }

    for i in 0..dom.nodes.len() {
        if matches_parsed(dom, i, &parts) {
            results.push(i);
        }
    }
    results
}

/// Check if a node matches a CSS selector string (public API kept for compat).
pub fn matches_selector(dom: &Dom, node_id: usize, selector: &str) -> bool {
    let sel = selector.trim();
    if sel.is_empty() { return false; }

    // Comma-separated: match any group.
    if sel.contains(',') {
        return sel.split(',').any(|s| matches_selector(dom, node_id, s.trim()));
    }

    let parts = parse_selector(sel);
    if parts.is_empty() { return false; }
    matches_parsed(dom, node_id, &parts)
}

// ---------------------------------------------------------------------------
// Selector parser
// ---------------------------------------------------------------------------

/// Parse a selector string (without commas) into a chain of parts.
/// Returns parts in left-to-right order.
fn parse_selector(sel: &str) -> Vec<SelectorPart<'_>> {
    let mut parts: Vec<SelectorPart<'_>> = Vec::new();
    let bytes = sel.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Skip leading whitespace.
    while i < len && bytes[i] == b' ' { i += 1; }

    loop {
        if i >= len { break; }

        // Read compound selector (until whitespace or combinator char).
        let start = i;
        while i < len {
            let b = bytes[i];
            // Stop at whitespace or explicit combinator characters.
            if b == b' ' || b == b'>' || b == b'+' || b == b'~' {
                // But not inside [...] attribute selectors.
                if in_brackets(bytes, start, i) {
                    i += 1;
                    continue;
                }
                break;
            }
            i += 1;
        }

        if i > start {
            let compound = &sel[start..i];
            let combinator = if parts.is_empty() { None } else { Some(Combinator::Descendant) };
            // The combinator will be overridden below if an explicit combinator is found.
            parts.push(SelectorPart { combinator, compound });
        }

        // Skip whitespace and look for combinator.
        let mut found_combinator: Option<Combinator> = None;
        while i < len {
            match bytes[i] {
                b' ' => { i += 1; }
                b'>' => { found_combinator = Some(Combinator::Child); i += 1; }
                b'+' => { found_combinator = Some(Combinator::Adjacent); i += 1; }
                b'~' => { found_combinator = Some(Combinator::General); i += 1; }
                _ => break,
            }
        }
        // Skip trailing whitespace after combinator.
        while i < len && bytes[i] == b' ' { i += 1; }

        // If we found an explicit combinator, record it on the NEXT part.
        if let Some(comb) = found_combinator {
            if i >= len { break; } // trailing combinator, ignore

            // Read next compound.
            let start2 = i;
            while i < len {
                let b = bytes[i];
                if b == b' ' || b == b'>' || b == b'+' || b == b'~' {
                    if in_brackets(bytes, start2, i) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            if i > start2 {
                parts.push(SelectorPart { combinator: Some(comb), compound: &sel[start2..i] });
            }
        }
    }

    parts
}

/// Check if position `pos` is inside a `[...]` block starting from `start`.
fn in_brackets(bytes: &[u8], start: usize, pos: usize) -> bool {
    let mut depth = 0i32;
    for j in start..pos {
        if bytes[j] == b'[' { depth += 1; }
        else if bytes[j] == b']' { depth -= 1; }
    }
    depth > 0
}

// ---------------------------------------------------------------------------
// Matching engine (right-to-left)
// ---------------------------------------------------------------------------

/// Match a node against a parsed selector chain.
fn matches_parsed(dom: &Dom, node_id: usize, parts: &[SelectorPart<'_>]) -> bool {
    if parts.is_empty() { return false; }

    // The rightmost part must match the candidate node itself.
    let last = parts.len() - 1;
    if !matches_compound(dom, node_id, parts[last].compound) {
        return false;
    }

    // Walk left through the chain, verifying each combinator.
    let mut current = node_id;
    let mut idx = last;

    while idx > 0 {
        idx -= 1;
        let part = &parts[idx];
        let comb = part.combinator.unwrap_or(Combinator::Descendant);

        match comb {
            Combinator::Descendant => {
                // Walk up ancestors until one matches.
                let mut found = false;
                let mut ancestor = dom.nodes[current].parent;
                while let Some(anc_id) = ancestor {
                    if matches_compound(dom, anc_id, part.compound) {
                        current = anc_id;
                        found = true;
                        break;
                    }
                    ancestor = dom.nodes[anc_id].parent;
                }
                if !found { return false; }
            }
            Combinator::Child => {
                // Direct parent must match.
                match dom.nodes[current].parent {
                    Some(pid) if matches_compound(dom, pid, part.compound) => {
                        current = pid;
                    }
                    _ => return false,
                }
            }
            Combinator::Adjacent => {
                // Previous element sibling must match.
                match prev_element_sibling(dom, current) {
                    Some(sib) if matches_compound(dom, sib, part.compound) => {
                        current = sib;
                    }
                    _ => return false,
                }
            }
            Combinator::General => {
                // Any earlier element sibling must match.
                let mut found = false;
                let mut sib = prev_element_sibling(dom, current);
                while let Some(sid) = sib {
                    if matches_compound(dom, sid, part.compound) {
                        current = sid;
                        found = true;
                        break;
                    }
                    sib = prev_element_sibling(dom, sid);
                }
                if !found { return false; }
            }
        }
    }

    true
}

/// Find the previous element sibling of a node (skipping text nodes).
fn prev_element_sibling(dom: &Dom, node_id: usize) -> Option<usize> {
    let parent_id = dom.nodes[node_id].parent?;
    let siblings = &dom.nodes[parent_id].children;
    let pos = siblings.iter().position(|&c| c == node_id)?;
    // Walk backwards from pos-1 to find an element.
    let mut j = pos;
    while j > 0 {
        j -= 1;
        if matches!(&dom.nodes[siblings[j]].node_type, NodeType::Element { .. }) {
            return Some(siblings[j]);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Compound selector matching (handles tag, #id, .class, [attr], *)
// ---------------------------------------------------------------------------

/// Match a single compound selector against a node.
/// A compound selector is e.g. `div.cls#id[attr=val]`.
fn matches_compound(dom: &Dom, node_id: usize, compound: &str) -> bool {
    if node_id >= dom.nodes.len() { return false; }

    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        _ => return false,
    };

    // Universal selector matches everything.
    if compound == "*" { return true; }

    // Tokenize compound into sub-parts: tag, .class, #id, [attr].
    let parts = tokenize_compound(compound);
    if parts.is_empty() { return false; }

    for part in &parts {
        match part {
            CompoundPart::Tag(name) => {
                let t = Tag::from_str(name);
                if t == Tag::Unknown || *tag != t {
                    return false;
                }
            }
            CompoundPart::Id(id_val) => {
                if !attrs.iter().any(|a| a.name == "id" && a.value == *id_val) {
                    return false;
                }
            }
            CompoundPart::Class(cls) => {
                if !attrs.iter().any(|a| {
                    a.name == "class" && a.value.split_whitespace().any(|c| c == *cls)
                }) {
                    return false;
                }
            }
            CompoundPart::Attr(name, val) => {
                match val {
                    Some(v) => {
                        if !attrs.iter().any(|a| a.name == *name && a.value == *v) {
                            return false;
                        }
                    }
                    None => {
                        if !attrs.iter().any(|a| a.name == *name) {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
}

/// Sub-parts of a compound selector.
enum CompoundPart<'a> {
    Tag(&'a str),
    Id(&'a str),
    Class(&'a str),
    Attr(&'a str, Option<&'a str>),
}

/// Tokenize a compound selector like `div.cls#id[href]` into parts.
fn tokenize_compound(s: &str) -> Vec<CompoundPart<'_>> {
    let mut parts = Vec::new();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    // Leading tag name (if doesn't start with #, ., [, or *).
    if i < len && bytes[i] != b'#' && bytes[i] != b'.' && bytes[i] != b'[' && bytes[i] != b'*' {
        let start = i;
        while i < len && bytes[i] != b'#' && bytes[i] != b'.' && bytes[i] != b'[' {
            i += 1;
        }
        parts.push(CompoundPart::Tag(&s[start..i]));
    } else if i < len && bytes[i] == b'*' {
        // Universal in compound — skip it (matches any tag).
        i += 1;
    }

    while i < len {
        match bytes[i] {
            b'#' => {
                i += 1;
                let start = i;
                while i < len && bytes[i] != b'#' && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                if i > start {
                    parts.push(CompoundPart::Id(&s[start..i]));
                }
            }
            b'.' => {
                i += 1;
                let start = i;
                while i < len && bytes[i] != b'#' && bytes[i] != b'.' && bytes[i] != b'[' {
                    i += 1;
                }
                if i > start {
                    parts.push(CompoundPart::Class(&s[start..i]));
                }
            }
            b'[' => {
                i += 1;
                let start = i;
                while i < len && bytes[i] != b']' {
                    i += 1;
                }
                let inner = &s[start..i];
                if i < len { i += 1; } // skip ']'
                if let Some(eq_pos) = inner.find('=') {
                    let name = inner[..eq_pos].trim();
                    let val = inner[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'');
                    parts.push(CompoundPart::Attr(name, Some(val)));
                } else {
                    parts.push(CompoundPart::Attr(inner.trim(), None));
                }
            }
            _ => {
                i += 1; // skip unexpected characters
            }
        }
    }

    parts
}
