//! CSS selector matching — simplified for querySelector / querySelectorAll.
//!
//! Supports: #id, .class, tag, tag.class, tag#id, [attr], [attr=val],
//! comma-separated selectors, universal selector *.

use crate::dom::{Dom, NodeType, Tag};
use alloc::vec::Vec;

/// Find first element matching a simple CSS selector.
pub fn find_first(dom: &Dom, selector: &str) -> Option<usize> {
    for i in 0..dom.nodes.len() {
        if matches_selector(dom, i, selector) {
            return Some(i);
        }
    }
    None
}

/// Find all elements matching a simple CSS selector.
pub fn find_all(dom: &Dom, selector: &str) -> Vec<usize> {
    let mut results = Vec::new();
    for i in 0..dom.nodes.len() {
        if matches_selector(dom, i, selector) {
            results.push(i);
        }
    }
    results
}

/// Check if a node matches a simple CSS selector.
pub fn matches_selector(dom: &Dom, node_id: usize, selector: &str) -> bool {
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        _ => return false,
    };

    let sel = selector.trim();
    if sel.is_empty() { return false; }

    // Comma-separated: match any.
    if sel.contains(',') {
        return sel.split(',').any(|s| matches_selector(dom, node_id, s.trim()));
    }

    // #id
    if sel.starts_with('#') {
        let target_id = &sel[1..];
        return attrs.iter().any(|a| a.name == "id" && a.value == target_id);
    }

    // .class
    if sel.starts_with('.') {
        let target_class = &sel[1..];
        return attrs.iter().any(|a| {
            a.name == "class" && a.value.split_whitespace().any(|c| c == target_class)
        });
    }

    // [attr] or [attr=val]
    if sel.starts_with('[') && sel.ends_with(']') {
        let inner = &sel[1..sel.len() - 1];
        if let Some(eq_pos) = inner.find('=') {
            let attr_name = inner[..eq_pos].trim();
            let attr_val = inner[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'');
            return attrs.iter().any(|a| a.name == attr_name && a.value == attr_val);
        } else {
            let attr_name = inner.trim();
            return attrs.iter().any(|a| a.name == attr_name);
        }
    }

    // tag.class
    if let Some(dot_pos) = sel.find('.') {
        if dot_pos > 0 {
            let tag_name = &sel[..dot_pos];
            let class_name = &sel[dot_pos + 1..];
            let tag_match = Tag::from_str(tag_name) == *tag;
            let class_match = attrs.iter().any(|a| {
                a.name == "class" && a.value.split_whitespace().any(|c| c == class_name)
            });
            return tag_match && class_match;
        }
    }

    // tag#id
    if let Some(hash_pos) = sel.find('#') {
        if hash_pos > 0 {
            let tag_name = &sel[..hash_pos];
            let id_name = &sel[hash_pos + 1..];
            let tag_match = Tag::from_str(tag_name) == *tag;
            let id_match = attrs.iter().any(|a| a.name == "id" && a.value == id_name);
            return tag_match && id_match;
        }
    }

    // Plain tag name.
    let target_tag = Tag::from_str(sel);
    if target_tag != Tag::Unknown {
        return *tag == target_tag;
    }

    // Universal.
    if sel == "*" { return true; }

    false
}
