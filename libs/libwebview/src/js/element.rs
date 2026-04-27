//! Native Element host object — all DOM Element methods.
//!
//! Each method is a native Rust function that reads `vm.current_this`
//! to get the element's `__nodeId`, then accesses the DOM via the bridge.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::value::{JsArray, JsObject};
use libjs::vm::native_fn;
use libjs::JsValue;
use libjs::Vm;

use super::classlist;
use super::selector;
use super::{
    arg_string, dom_property_hook, get_bridge, make_array, read_all_child_node_ids, read_attribute,
    read_child_ids, read_inner_html, read_node_type, read_parent_id, read_tag_name,
    read_text_content, this_node_id, DomMutation,
};

// ═══════════════════════════════════════════════════════════
// Sibling helpers
// ═══════════════════════════════════════════════════════════

/// Convert a `data-*` attribute suffix to camelCase for the `dataset` API.
/// e.g. "foo-bar" → "fooBar", "my-value" → "myValue".
fn data_attr_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for c in s.chars() {
        if c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Compute the previous and next sibling node IDs for the given real DOM node.
/// Returns `(prev_element_id, next_element_id, prev_any_id, next_any_id)`,
/// where `*_element_id` skips text nodes and `*_any_id` includes all node types.
/// Returns `None` when no such sibling exists.
fn compute_sibling_ids(
    vm: &mut Vm,
    node_id: usize,
) -> (Option<usize>, Option<usize>, Option<usize>, Option<usize>) {
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        if let Some(parent_id) = dom.nodes.get(node_id).and_then(|n| n.parent) {
            let siblings = &dom.nodes[parent_id].children;
            if let Some(pos) = siblings.iter().position(|&id| id == node_id) {
                // prev/next for *any* node type
                let prev_any = if pos > 0 {
                    Some(siblings[pos - 1])
                } else {
                    None
                };
                let next_any = if pos + 1 < siblings.len() {
                    Some(siblings[pos + 1])
                } else {
                    None
                };

                // prev/next for element nodes only (nodeType == 1)
                let prev_el = (0..pos)
                    .rev()
                    .find(|&i| {
                        matches!(
                            &dom.nodes[siblings[i]].node_type,
                            crate::dom::NodeType::Element { .. }
                        )
                    })
                    .map(|i| siblings[i]);
                let next_el = (pos + 1..siblings.len())
                    .find(|&i| {
                        matches!(
                            &dom.nodes[siblings[i]].node_type,
                            crate::dom::NodeType::Element { .. }
                        )
                    })
                    .map(|i| siblings[i]);

                return (prev_el, next_el, prev_any, next_any);
            }
        }
    }
    (None, None, None, None)
}

// ═══════════════════════════════════════════════════════════
// Element factory
// ═══════════════════════════════════════════════════════════

/// Create a native Element JsObject for a single DOM node.
///
/// Children are intentionally **not** built eagerly — doing so for a large
/// DOM (e.g. 14 000+ nodes) causes an O(N × properties) allocation storm
/// that exhausts the heap and corrupts the BTreeMap internal tree.
/// Scripts that need child elements should use `querySelector` /
/// `querySelectorAll` / `getElementById`, which create elements on demand.
///
/// Sibling properties (`nextSibling`, `previousSibling`, `nextElementSibling`,
/// Populate an Element/Node prototype object with the standard DOM methods.
///
/// Called once during window initialisation so that `Element.prototype.replaceWith`
/// etc. are visible to polyfill / framework code that feature-detects via the
/// prototype rather than per-instance.  The methods read `vm.current_this`
/// at call time, so they work identically whether found on instance or prototype.
pub fn populate_element_prototype(proto: &JsValue) {
    use libjs::vm::native_fn;
    // Node interface
    proto.set_property(
        String::from("appendChild"),
        native_fn("appendChild", el_append_child),
    );
    proto.set_property(
        String::from("removeChild"),
        native_fn("removeChild", el_remove_child),
    );
    proto.set_property(
        String::from("insertBefore"),
        native_fn("insertBefore", el_insert_before),
    );
    proto.set_property(
        String::from("replaceChild"),
        native_fn("replaceChild", el_replace_child),
    );
    proto.set_property(
        String::from("cloneNode"),
        native_fn("cloneNode", el_clone_node),
    );
    proto.set_property(String::from("contains"), native_fn("contains", el_contains));
    proto.set_property(String::from("remove"), native_fn("remove", el_remove));
    proto.set_property(
        String::from("getRootNode"),
        native_fn("getRootNode", el_get_root_node),
    );
    // Element interface
    proto.set_property(
        String::from("getAttribute"),
        native_fn("getAttribute", el_get_attribute),
    );
    proto.set_property(
        String::from("setAttribute"),
        native_fn("setAttribute", el_set_attribute),
    );
    proto.set_property(
        String::from("removeAttribute"),
        native_fn("removeAttribute", el_remove_attribute),
    );
    proto.set_property(
        String::from("hasAttribute"),
        native_fn("hasAttribute", el_has_attribute),
    );
    proto.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", el_add_event_listener),
    );
    proto.set_property(
        String::from("installListener"),
        native_fn("installListener", el_add_event_listener),
    );
    proto.set_property(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", el_dispatch_event),
    );
    proto.set_property(
        String::from("querySelector"),
        native_fn("querySelector", el_query_selector),
    );
    proto.set_property(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", el_query_selector_all),
    );
    proto.set_property(
        String::from("getElementsByTagName"),
        native_fn("getElementsByTagName", el_get_elements_by_tag_name),
    );
    proto.set_property(
        String::from("getElementsByClassName"),
        native_fn("getElementsByClassName", el_get_elements_by_class_name),
    );
    proto.set_property(
        String::from("getBoundingClientRect"),
        native_fn("getBoundingClientRect", el_get_bounding_rect),
    );
    proto.set_property(
        String::from("getClientRects"),
        native_fn("getClientRects", el_get_client_rects),
    );
    proto.set_property(String::from("matches"), native_fn("matches", el_matches));
    proto.set_property(String::from("closest"), native_fn("closest", el_closest));
    // ParentNode interface
    proto.set_property(String::from("prepend"), native_fn("prepend", el_prepend));
    proto.set_property(String::from("append"), native_fn("append", el_append));
    proto.set_property(
        String::from("replaceChildren"),
        native_fn("replaceChildren", el_replace_children),
    );
    // ChildNode interface
    proto.set_property(String::from("before"), native_fn("before", el_before));
    proto.set_property(String::from("after"), native_fn("after", el_after));
    proto.set_property(
        String::from("replaceWith"),
        native_fn("replaceWith", el_replace_with),
    );
    // insertAdjacent family
    proto.set_property(
        String::from("insertAdjacentHTML"),
        native_fn("insertAdjacentHTML", el_insert_adjacent_html),
    );
    proto.set_property(
        String::from("insertAdjacentElement"),
        native_fn("insertAdjacentElement", el_insert_adjacent_element),
    );
    proto.set_property(
        String::from("insertAdjacentText"),
        native_fn("insertAdjacentText", el_insert_adjacent_text),
    );
    // Scrolling
    proto.set_property(
        String::from("scrollTo"),
        native_fn("scrollTo", el_scroll_to),
    );
    proto.set_property(
        String::from("scrollBy"),
        native_fn("scrollBy", el_scroll_by),
    );
    proto.set_property(String::from("scroll"), native_fn("scroll", el_scroll_to));
    // Misc stubs
    proto.set_property(String::from("focus"), native_fn("focus", el_noop));
    proto.set_property(String::from("blur"), native_fn("blur", el_noop));
    proto.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", el_noop),
    );
    proto.set_property(
        String::from("toString"),
        native_fn("toString", el_to_string),
    );
}

/// `previousElementSibling`) are computed one level deep: the returned sibling
/// objects themselves have `Null` for their own siblings, preventing O(N²)
/// allocation chains for large flat lists. Full sibling traversal loops should
/// use `querySelectorAll` instead.
pub fn make_element(vm: &mut Vm, node_id: i64) -> JsValue {
    make_element_impl(vm, node_id, true)
}

/// Internal factory.  When `include_siblings` is `false` the sibling
/// properties are set to `Null` — used when creating sibling JsObjects to
/// prevent recursive depth growth.
fn make_element_impl(vm: &mut Vm, node_id: i64, include_siblings: bool) -> JsValue {
    // Read properties from DOM or virtual node store.
    let tag_name = read_tag_name(vm, node_id);
    let is_template = tag_name == "TEMPLATE";
    let text = read_text_content(vm, node_id);
    let node_type = read_node_type(vm, node_id);
    let inner_html = read_inner_html(vm, node_id);
    let class_name = match read_attribute(vm, node_id, "class") {
        JsValue::String(s) => s,
        _ => String::new(),
    };

    // Helper to read a string attribute or empty string.
    let attr_or_empty = |vm: &mut Vm, name: &str| -> String {
        match read_attribute(vm, node_id, name) {
            JsValue::String(s) => s,
            _ => String::new(),
        }
    };

    let id_val = attr_or_empty(vm, "id");
    let value_val = attr_or_empty(vm, "value");
    let src_val = attr_or_empty(vm, "src");
    let href_val = attr_or_empty(vm, "href");
    let type_val = attr_or_empty(vm, "type");
    let name_val = attr_or_empty(vm, "name");
    let checked = !matches!(read_attribute(vm, node_id, "checked"), JsValue::Null);
    let disabled = !matches!(read_attribute(vm, node_id, "disabled"), JsValue::Null);

    // Build shallow child collections. When creating a shallow helper object
    // (e.g. for parentNode / sibling references), skip children entirely to
    // avoid recursive parent<->child expansion on large DOM trees.
    let (child_count, child_arr, child_nodes_arr, first_child, last_child) = if include_siblings {
        let element_child_ids = read_child_ids(vm, node_id);
        let all_child_ids = read_all_child_node_ids(vm, node_id);
        let child_count = element_child_ids.len();
        let child_elems: Vec<JsValue> = element_child_ids
            .iter()
            .map(|&id| make_element_impl(vm, id, false))
            .collect();
        let child_nodes: Vec<JsValue> = all_child_ids
            .iter()
            .map(|&id| make_element_impl(vm, id, false))
            .collect();
        let first_child = child_nodes.first().cloned().unwrap_or(JsValue::Null);
        let last_child = child_nodes.last().cloned().unwrap_or(JsValue::Null);
        (
            child_count,
            make_array(child_elems),
            make_array(child_nodes),
            first_child,
            last_child,
        )
    } else {
        (
            0,
            make_array(Vec::new()),
            make_array(Vec::new()),
            JsValue::Null,
            JsValue::Null,
        )
    };
    let parent_id = read_parent_id(vm, node_id);
    let parent_node = if include_siblings && parent_id >= 0 {
        make_element_impl(vm, parent_id, false)
    } else {
        JsValue::Null
    };
    let parent_element = parent_node.clone();

    // Build the element object.
    let mut obj = JsObject::new();
    let prototype_ctor = match node_type as u32 {
        1 => vm.get_global("HTMLElement"),
        _ => vm.get_global("Node"),
    };
    if let JsValue::Function(func) = prototype_ctor {
        obj.prototype = func.borrow().prototype.clone();
    }

    // Identity.
    obj.set(String::from("__nodeId"), JsValue::Number(node_id as f64));

    // Properties.
    obj.set(String::from("nodeType"), JsValue::Number(node_type));
    obj.set(String::from("tagName"), JsValue::String(tag_name.clone()));
    // nodeName: W3C DOM §4.4 — Element→tagName, Text→"#text", Comment→"#comment",
    // Document→"#document", DocumentFragment→"#document-fragment".
    let node_name = match node_type as u32 {
        1 => tag_name.clone(),                    // Element
        3 => String::from("#text"),               // Text
        8 => String::from("#comment"),            // Comment
        9 => String::from("#document"),           // Document
        11 => String::from("#document-fragment"), // DocumentFragment
        _ => tag_name.clone(),
    };
    obj.set(String::from("nodeName"), JsValue::String(node_name));
    obj.set(
        String::from("localName"),
        JsValue::String(tag_name.to_ascii_lowercase()),
    );
    obj.set(String::from("id"), JsValue::String(id_val));
    obj.set(
        String::from("className"),
        JsValue::String(class_name.clone()),
    );
    obj.set(String::from("textContent"), JsValue::String(text.clone()));
    obj.set(String::from("innerText"), JsValue::String(text));
    obj.set(String::from("innerHTML"), JsValue::String(inner_html));
    obj.set(String::from("value"), JsValue::String(value_val));
    obj.set(String::from("src"), JsValue::String(src_val));
    obj.set(String::from("href"), JsValue::String(href_val));
    obj.set(String::from("type"), JsValue::String(type_val.clone()));
    obj.set(String::from("name"), JsValue::String(name_val));
    obj.set(String::from("checked"), JsValue::Bool(checked));
    obj.set(String::from("disabled"), JsValue::Bool(disabled));
    // Additional form properties (HTML §4.10).
    let required = !matches!(read_attribute(vm, node_id, "required"), JsValue::Null);
    let read_only = !matches!(read_attribute(vm, node_id, "readonly"), JsValue::Null);
    let multiple = !matches!(read_attribute(vm, node_id, "multiple"), JsValue::Null);
    obj.set(String::from("required"), JsValue::Bool(required));
    obj.set(String::from("readOnly"), JsValue::Bool(read_only));
    obj.set(String::from("multiple"), JsValue::Bool(multiple));
    obj.set(
        String::from("placeholder"),
        JsValue::String(attr_or_empty(vm, "placeholder")),
    );
    obj.set(
        String::from("min"),
        JsValue::String(attr_or_empty(vm, "min")),
    );
    obj.set(
        String::from("max"),
        JsValue::String(attr_or_empty(vm, "max")),
    );
    obj.set(
        String::from("step"),
        JsValue::String(attr_or_empty(vm, "step")),
    );
    obj.set(
        String::from("pattern"),
        JsValue::String(attr_or_empty(vm, "pattern")),
    );
    obj.set(
        String::from("accept"),
        JsValue::String(attr_or_empty(vm, "accept")),
    );
    obj.set(
        String::from("autocomplete"),
        JsValue::String(attr_or_empty(vm, "autocomplete")),
    );
    // maxLength / minLength — -1 if not set (per HTML spec).
    let maxlength = read_attribute(vm, node_id, "maxlength");
    let maxlength_num = match &maxlength {
        JsValue::String(s) => s.parse::<f64>().unwrap_or(-1.0),
        _ => -1.0,
    };
    obj.set(String::from("maxLength"), JsValue::Number(maxlength_num));
    let minlength = read_attribute(vm, node_id, "minlength");
    let minlength_num = match &minlength {
        JsValue::String(s) => s.parse::<f64>().unwrap_or(-1.0),
        _ => -1.0,
    };
    obj.set(String::from("minLength"), JsValue::Number(minlength_num));
    // selectedIndex for <select> (reflects DOM `selected` attr on <option> children).
    let selected_index = read_attribute(vm, node_id, "selectedIndex");
    if !matches!(selected_index, JsValue::Null) {
        obj.set(String::from("selectedIndex"), selected_index);
    }
    obj.set(
        String::from("namespaceURI"),
        JsValue::String(String::from("http://www.w3.org/1999/xhtml")),
    );
    obj.set(String::from("nodeValue"), JsValue::Null);

    // Sibling references — computed one level deep for real DOM nodes.
    // When `include_siblings` is false (we're already building a sibling object)
    // they stay Null to prevent O(N²) allocation chains on large flat lists.
    let (prev_sib, next_sib, prev_any, next_any) = if include_siblings && node_id >= 0 {
        let (pe, ne, pa, na) = compute_sibling_ids(vm, node_id as usize);
        (
            pe.map(|id| make_element_impl(vm, id as i64, false))
                .unwrap_or(JsValue::Null),
            ne.map(|id| make_element_impl(vm, id as i64, false))
                .unwrap_or(JsValue::Null),
            pa.map(|id| make_element_impl(vm, id as i64, false))
                .unwrap_or(JsValue::Null),
            na.map(|id| make_element_impl(vm, id as i64, false))
                .unwrap_or(JsValue::Null),
        )
    } else {
        (JsValue::Null, JsValue::Null, JsValue::Null, JsValue::Null)
    };

    // Tree references.
    obj.set(String::from("children"), child_arr.clone());
    obj.set(String::from("childNodes"), child_nodes_arr);
    obj.set(
        String::from("childElementCount"),
        JsValue::Number(child_count as f64),
    );
    obj.set(String::from("firstChild"), first_child);
    obj.set(String::from("lastChild"), last_child);
    obj.set(String::from("parentNode"), parent_node);
    obj.set(String::from("parentElement"), parent_element);
    obj.set(String::from("previousSibling"), prev_any);
    obj.set(String::from("nextSibling"), next_any);
    obj.set(String::from("previousElementSibling"), prev_sib);
    obj.set(String::from("nextElementSibling"), next_sib);

    // Style — CSSStyleDeclaration (W3C CSSOM §6.7.2).
    // Properties set on this object trigger SetStyleProperty mutations via set_hook.
    let style_obj = make_css_style_declaration(node_id);
    obj.set(String::from("style"), style_obj);
    // dataset — DOMStringMap from data-* attributes (W3C HTML §3.2.6.1).
    let mut dataset_obj = JsObject::new();
    if node_id >= 0 {
        if let Some(bridge) = get_bridge(vm) {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                if let crate::dom::NodeType::Element { ref attrs, .. } = dom.nodes[nid].node_type {
                    for attr in attrs {
                        if attr.name.starts_with("data-") {
                            let key = data_attr_to_camel(&attr.name[5..]);
                            dataset_obj.set(key, JsValue::String(attr.value.clone()));
                        }
                    }
                }
            }
        }
    }
    obj.set(
        String::from("dataset"),
        JsValue::Object(Rc::new(RefCell::new(dataset_obj))),
    );

    // classList.
    let cl = classlist::make_class_list(node_id, &class_name);
    obj.set(String::from("classList"), cl);

    // ── Native methods ──
    obj.set(
        String::from("getAttribute"),
        native_fn("getAttribute", el_get_attribute),
    );
    obj.set(
        String::from("setAttribute"),
        native_fn("setAttribute", el_set_attribute),
    );
    obj.set(
        String::from("removeAttribute"),
        native_fn("removeAttribute", el_remove_attribute),
    );
    obj.set(
        String::from("hasAttribute"),
        native_fn("hasAttribute", el_has_attribute),
    );
    obj.set(
        String::from("addEventListener"),
        native_fn("addEventListener", el_add_event_listener),
    );
    obj.set(
        String::from("removeEventListener"),
        native_fn("removeEventListener", super::native_remove_event_listener),
    );
    obj.set(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", el_dispatch_event),
    );

    // Query.
    obj.set(
        String::from("querySelector"),
        native_fn("querySelector", el_query_selector),
    );
    obj.set(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", el_query_selector_all),
    );
    obj.set(
        String::from("getElementsByTagName"),
        native_fn("getElementsByTagName", el_get_elements_by_tag_name),
    );
    obj.set(
        String::from("getElementsByClassName"),
        native_fn("getElementsByClassName", el_get_elements_by_class_name),
    );

    // Tree manipulation (Node interface).
    obj.set(
        String::from("appendChild"),
        native_fn("appendChild", el_append_child),
    );
    obj.set(
        String::from("removeChild"),
        native_fn("removeChild", el_remove_child),
    );
    obj.set(
        String::from("insertBefore"),
        native_fn("insertBefore", el_insert_before),
    );
    obj.set(
        String::from("replaceChild"),
        native_fn("replaceChild", el_replace_child),
    );
    obj.set(
        String::from("cloneNode"),
        native_fn("cloneNode", el_clone_node),
    );
    obj.set(String::from("contains"), native_fn("contains", el_contains));
    obj.set(String::from("remove"), native_fn("remove", el_remove));

    // ParentNode interface (W3C DOM §4.2.6).
    obj.set(String::from("prepend"), native_fn("prepend", el_prepend));
    obj.set(String::from("append"), native_fn("append", el_append));
    obj.set(
        String::from("replaceChildren"),
        native_fn("replaceChildren", el_replace_children),
    );

    // ChildNode interface (W3C DOM §4.2.7).
    obj.set(String::from("before"), native_fn("before", el_before));
    obj.set(String::from("after"), native_fn("after", el_after));
    obj.set(
        String::from("replaceWith"),
        native_fn("replaceWith", el_replace_with),
    );

    // insertAdjacentHTML / insertAdjacentElement (W3C DOM Parsing §4).
    obj.set(
        String::from("insertAdjacentHTML"),
        native_fn("insertAdjacentHTML", el_insert_adjacent_html),
    );
    obj.set(
        String::from("insertAdjacentElement"),
        native_fn("insertAdjacentElement", el_insert_adjacent_element),
    );
    obj.set(
        String::from("insertAdjacentText"),
        native_fn("insertAdjacentText", el_insert_adjacent_text),
    );

    // Content setters (since we can't intercept property writes).
    obj.set(
        String::from("setTextContent"),
        native_fn("setTextContent", el_set_text_content),
    );
    obj.set(
        String::from("setInnerHTML"),
        native_fn("setInnerHTML", el_set_inner_html),
    );
    obj.set(
        String::from("setStyle"),
        native_fn("setStyle", el_set_style),
    );

    // Node properties (W3C DOM §4.4).
    obj.set(String::from("isConnected"), JsValue::Bool(node_id >= 0));
    obj.set(
        String::from("getRootNode"),
        native_fn("getRootNode", el_get_root_node),
    );
    // ownerDocument: W3C DOM §4.4 — returns the Document that owns this node.
    // React 19 relies on ownerDocument.defaultView to find the window object.
    obj.set(String::from("ownerDocument"), vm.get_global("document"));

    // outerHTML (W3C DOM Parsing §3).
    obj.set(String::from("outerHTML"), JsValue::String(String::new())); // placeholder, set_hook handles writes

    // Geometry (W3C CSSOM View §6).
    let (offset_w, offset_h) =
        estimate_box_size(vm, node_id, &tag_name, &class_name, child_count, &type_val);
    obj.set(
        String::from("offsetWidth"),
        JsValue::Number(offset_w as f64),
    );
    obj.set(
        String::from("offsetHeight"),
        JsValue::Number(offset_h as f64),
    );
    obj.set(String::from("offsetTop"), JsValue::Number(0.0));
    obj.set(String::from("offsetLeft"), JsValue::Number(0.0));
    obj.set(String::from("offsetParent"), JsValue::Null);
    obj.set(
        String::from("clientWidth"),
        JsValue::Number(offset_w.saturating_sub(2) as f64),
    );
    obj.set(
        String::from("clientHeight"),
        JsValue::Number(offset_h.saturating_sub(2) as f64),
    );
    obj.set(String::from("clientTop"), JsValue::Number(0.0));
    obj.set(String::from("clientLeft"), JsValue::Number(0.0));
    obj.set(
        String::from("scrollWidth"),
        JsValue::Number(offset_w as f64),
    );
    obj.set(
        String::from("scrollHeight"),
        JsValue::Number(offset_h as f64),
    );
    obj.set(String::from("scrollTop"), JsValue::Number(0.0));
    obj.set(String::from("scrollLeft"), JsValue::Number(0.0));

    // Misc.
    obj.set(String::from("matches"), native_fn("matches", el_matches));
    obj.set(String::from("closest"), native_fn("closest", el_closest));
    obj.set(String::from("focus"), native_fn("focus", el_noop));
    obj.set(String::from("blur"), native_fn("blur", el_noop));
    obj.set(String::from("click"), native_fn("click", el_noop));
    obj.set(
        String::from("scrollIntoView"),
        native_fn("scrollIntoView", el_noop),
    );
    obj.set(
        String::from("scrollTo"),
        native_fn("scrollTo", el_scroll_to),
    );
    obj.set(
        String::from("scrollBy"),
        native_fn("scrollBy", el_scroll_by),
    );
    obj.set(
        String::from("getBoundingClientRect"),
        native_fn("getBoundingClientRect", el_get_bounding_rect),
    );
    obj.set(
        String::from("getClientRects"),
        native_fn("getClientRects", el_get_client_rects),
    );
    obj.set(
        String::from("toString"),
        native_fn("toString", el_to_string),
    );

    // Canvas: getContext('2d') returns a CanvasRenderingContext2D stub
    if tag_name == "CANVAS" {
        obj.set(String::from("width"), JsValue::Number(300.0));
        obj.set(String::from("height"), JsValue::Number(150.0));
        obj.set(
            String::from("getContext"),
            native_fn("getContext", el_get_context),
        );
        obj.set(
            String::from("toDataURL"),
            native_fn("toDataURL", |_, _| JsValue::String(String::from("data:,"))),
        );
        obj.set(
            String::from("toBlob"),
            native_fn("toBlob", |_, _| JsValue::Undefined),
        );
    }

    // <form> element: add submit(), reset(), elements, checkValidity().
    if tag_name == "FORM" {
        obj.set(
            String::from("submit"),
            native_fn("submit", |vm, _args| {
                let this = vm.current_this.clone();
                let nid = extract_node_id(&this);
                if nid >= 0 {
                    if let Some(bridge) = get_bridge(vm) {
                        bridge.mutations.push(crate::js::DomMutation::FormSubmit {
                            form_node_id: nid as usize,
                        });
                    }
                }
                JsValue::Undefined
            }),
        );
        obj.set(
            String::from("reset"),
            native_fn("reset", |vm, _args| {
                let this = vm.current_this.clone();
                let nid = extract_node_id(&this);
                if nid >= 0 {
                    if let Some(bridge) = get_bridge(vm) {
                        bridge.mutations.push(crate::js::DomMutation::FormReset {
                            form_node_id: nid as usize,
                        });
                    }
                }
                JsValue::Undefined
            }),
        );
        // checkValidity() — validates all descendant form controls.
        obj.set(
            String::from("checkValidity"),
            native_fn("checkValidity", el_form_check_validity),
        );
        obj.set(
            String::from("reportValidity"),
            native_fn("reportValidity", el_form_check_validity),
        );
    }

    // Form control elements: add checkValidity(), setCustomValidity(), validity.
    if matches!(
        tag_name.as_str(),
        "INPUT" | "SELECT" | "TEXTAREA" | "BUTTON"
    ) {
        obj.set(
            String::from("checkValidity"),
            native_fn("checkValidity", el_check_validity),
        );
        obj.set(
            String::from("reportValidity"),
            native_fn("reportValidity", el_report_validity),
        );
        obj.set(
            String::from("setCustomValidity"),
            native_fn("setCustomValidity", |_, _| JsValue::Undefined),
        );
        // validity — getter that runs real constraint validation.
        obj.set(
            String::from("validity"),
            native_fn("validity", el_get_validity),
        );
        obj.set(
            String::from("validationMessage"),
            JsValue::String(String::new()),
        );
        obj.set(String::from("willValidate"), JsValue::Bool(true));
    }

    // Set property-write interception hook so that assignments like
    // el.textContent = "x" record DOM mutations.
    obj.set_hook = Some(dom_property_hook);
    obj.set_hook_data = node_id as usize as *mut u8;

    let result = JsValue::Object(Rc::new(RefCell::new(obj)));

    // HTML <template> element: expose .content as a DocumentFragment-like
    // object whose children are the template's DOM children.  This allows
    // `template.content.cloneNode(true)` to work as specified in the HTML
    // Living Standard (§4.12.3).
    if is_template && include_siblings {
        let content_obj = make_template_content(vm, node_id);
        if let JsValue::Object(ref obj_rc) = result {
            obj_rc
                .borrow_mut()
                .set(String::from("content"), content_obj);
        }
    }

    result
}

// ═══════════════════════════════════════════════════════════
// HTML <template> element — DocumentFragment .content
// ═══════════════════════════════════════════════════════════

/// Build a DocumentFragment-like JsObject for `<template>.content`.
///
/// The fragment exposes `children`, `childNodes`, `firstChild`, `lastChild`,
/// `querySelector`, `querySelectorAll`, `getElementById`, and `cloneNode`.
fn make_template_content(vm: &mut Vm, template_node_id: i64) -> JsValue {
    let mut frag = JsObject::new();
    frag.set(String::from("nodeType"), JsValue::Number(11.0)); // DOCUMENT_FRAGMENT_NODE
    frag.set(
        String::from("nodeName"),
        JsValue::String(String::from("#document-fragment")),
    );
    frag.set(
        String::from("__templateNodeId"),
        JsValue::Number(template_node_id as f64),
    );

    // Build children array from the template's DOM children (elements only,
    // matching `read_child_ids` semantics).
    let child_ids = read_child_ids(vm, template_node_id);
    let children: Vec<JsValue> = child_ids
        .iter()
        .map(|&id| make_element_impl(vm, id, false))
        .collect();
    let child_count = children.len();

    let first = children.first().cloned().unwrap_or(JsValue::Null);
    let last = children.last().cloned().unwrap_or(JsValue::Null);

    frag.set(String::from("children"), make_array(children.clone()));
    frag.set(String::from("childNodes"), make_array(children));
    frag.set(
        String::from("childElementCount"),
        JsValue::Number(child_count as f64),
    );
    frag.set(String::from("firstChild"), first.clone());
    frag.set(String::from("lastChild"), last);
    frag.set(String::from("firstElementChild"), first);

    // Query methods delegate to the template node's subtree.
    frag.set(
        String::from("querySelector"),
        native_fn("querySelector", el_query_selector),
    );
    frag.set(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", el_query_selector_all),
    );
    frag.set(
        String::from("getElementById"),
        native_fn("getElementById", frag_get_element_by_id),
    );

    // cloneNode(deep) — the critical method for template instantiation.
    frag.set(
        String::from("cloneNode"),
        native_fn("cloneNode", frag_clone_node),
    );

    // appendChild — so scripts can append to the fragment before inserting.
    frag.set(
        String::from("appendChild"),
        native_fn("appendChild", el_append_child),
    );

    // Store the template node ID so cloneNode can find the children.
    frag.set_hook_data = template_node_id as usize as *mut u8;

    JsValue::Object(Rc::new(RefCell::new(frag)))
}

/// getElementById scoped to the full DOM (template children are real DOM nodes).
fn frag_get_element_by_id(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = arg_string(args, 0);
    if id.is_empty() {
        return JsValue::Null;
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for i in 0..dom.nodes.len() {
            if let crate::dom::NodeType::Element { ref attrs, .. } = dom.nodes[i].node_type {
                for attr in attrs {
                    if attr.name == "id" && attr.value == id {
                        return make_element(vm, i as i64);
                    }
                }
            }
        }
    }
    JsValue::Null
}

/// Deep-clone the template content fragment.  Creates new virtual nodes
/// for each child subtree so the caller gets independent DOM nodes.
fn frag_clone_node(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let deep = args.first().map(|v| v.to_boolean()).unwrap_or(false);

    // Get the template node ID from `this.__templateNodeId`.
    let template_id = if let JsValue::Object(ref obj) = vm.current_this {
        let o = obj.borrow();
        match o.get("__templateNodeId") {
            JsValue::Number(n) => n as i64,
            _ => return JsValue::Null,
        }
    } else {
        return JsValue::Null;
    };

    if !deep {
        // Shallow clone: return empty fragment.
        let mut frag = JsObject::new();
        frag.set(String::from("nodeType"), JsValue::Number(11.0));
        frag.set(
            String::from("nodeName"),
            JsValue::String(String::from("#document-fragment")),
        );
        frag.set(String::from("children"), make_array(Vec::new()));
        frag.set(String::from("childNodes"), make_array(Vec::new()));
        frag.set(String::from("childElementCount"), JsValue::Number(0.0));
        frag.set(String::from("firstChild"), JsValue::Null);
        frag.set(String::from("lastChild"), JsValue::Null);
        frag.set(String::from("firstElementChild"), JsValue::Null);
        return JsValue::Object(Rc::new(RefCell::new(frag)));
    }

    // Deep clone: read ALL children of the template node (including text
    // nodes) and recursively clone each as a new virtual node.
    let all_child_ids = read_all_child_ids(vm, template_id);
    let mut cloned_children = Vec::new();
    for &child_id in &all_child_ids {
        let cloned = deep_clone_node(vm, child_id);
        cloned_children.push(cloned);
    }

    let child_count = cloned_children.len();
    let first = cloned_children.first().cloned().unwrap_or(JsValue::Null);
    let last = cloned_children.last().cloned().unwrap_or(JsValue::Null);

    let mut frag = JsObject::new();
    frag.set(String::from("nodeType"), JsValue::Number(11.0));
    frag.set(
        String::from("nodeName"),
        JsValue::String(String::from("#document-fragment")),
    );
    frag.set(
        String::from("children"),
        make_array(cloned_children.clone()),
    );
    frag.set(String::from("childNodes"), make_array(cloned_children));
    frag.set(
        String::from("childElementCount"),
        JsValue::Number(child_count as f64),
    );
    frag.set(String::from("firstChild"), first.clone());
    frag.set(String::from("lastChild"), last);
    frag.set(String::from("firstElementChild"), first);
    frag.set(
        String::from("querySelector"),
        native_fn("querySelector", el_query_selector),
    );
    frag.set(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", el_query_selector_all),
    );
    frag.set(
        String::from("getElementById"),
        native_fn("getElementById", frag_get_element_by_id),
    );
    frag.set(
        String::from("cloneNode"),
        native_fn("cloneNode", frag_clone_node),
    );
    frag.set(
        String::from("appendChild"),
        native_fn("appendChild", el_append_child),
    );

    JsValue::Object(Rc::new(RefCell::new(frag)))
}

/// Read ALL child node IDs of a real DOM node (elements + text nodes).
/// Unlike `read_child_ids` which filters to elements only, this returns
/// every child so that deep cloning preserves text nodes.
fn read_all_child_ids(vm: &mut Vm, node_id: i64) -> Vec<i64> {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return dom.nodes[nid]
                    .children
                    .iter()
                    .map(|&cid| cid as i64)
                    .collect();
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            return vn.child_ids.clone();
        }
    }
    Vec::new()
}

/// Recursively deep-clone a DOM node (real or virtual) as a new virtual node.
/// Returns a JsValue element/text-node that can be appended to other elements.
fn deep_clone_node(vm: &mut Vm, node_id: i64) -> JsValue {
    if node_id < 0 {
        // Virtual node — clone its tag/attrs/text.
        if let Some(bridge) = get_bridge(vm) {
            if let Some(vn) = bridge.get_virtual(node_id) {
                let tag = vn.tag.clone();
                let attrs = vn.attrs.clone();
                let text_content = vn.text_content.clone();
                let child_ids = vn.child_ids.clone();

                let new_id = bridge.alloc_virtual_id();
                bridge.virtual_nodes.push(super::VirtualNode {
                    id: new_id,
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                    text_content,
                    child_ids: Vec::new(),
                    parent_id: None,
                });
                bridge.mutations.push(DomMutation::CreateElement {
                    virtual_id: new_id,
                    tag,
                });
                for (name, value) in &attrs {
                    bridge.mutations.push(DomMutation::SetAttribute {
                        node_id: new_id,
                        name: name.clone(),
                        value: value.clone(),
                    });
                }

                // Recursively clone children.
                for &cid in &child_ids {
                    let child_val = deep_clone_node(vm, cid);
                    let child_nid = extract_node_id(&child_val);
                    if child_nid != -9999 {
                        if let Some(b) = get_bridge(vm) {
                            b.mutations.push(DomMutation::AppendChild {
                                parent_id: new_id,
                                child_id: child_nid,
                            });
                            if let Some(vn2) = b.get_virtual_mut(new_id) {
                                vn2.child_ids.push(child_nid);
                            }
                        }
                    }
                }

                return make_element(vm, new_id);
            }
        }
        return JsValue::Null;
    }

    // Real DOM node — read its type and clone accordingly.
    let node_info = if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let nid = node_id as usize;
        if nid < dom.nodes.len() {
            match &dom.nodes[nid].node_type {
                crate::dom::NodeType::Element { tag, attrs } => {
                    let tag_str = String::from(tag.tag_name());
                    let attr_pairs: Vec<(String, String)> = attrs
                        .iter()
                        .map(|a| (a.name.clone(), a.value.clone()))
                        .collect();
                    let child_ids: Vec<i64> = dom.nodes[nid]
                        .children
                        .iter()
                        .map(|&cid| cid as i64)
                        .collect();
                    Some((tag_str, attr_pairs, None, child_ids))
                }
                crate::dom::NodeType::Text(text) => Some((
                    String::from("#text"),
                    Vec::new(),
                    Some(text.clone()),
                    Vec::new(),
                )),
            }
        } else {
            None
        }
    } else {
        None
    };

    if let Some((tag, attrs, text_opt, child_ids)) = node_info {
        if let Some(text) = text_opt {
            // Clone as virtual text node.
            if let Some(bridge) = get_bridge(vm) {
                let new_id = bridge.alloc_virtual_id();
                bridge.virtual_nodes.push(super::VirtualNode {
                    id: new_id,
                    tag: String::from("#text"),
                    attrs: Vec::new(),
                    text_content: text.clone(),
                    child_ids: Vec::new(),
                    parent_id: None,
                });
                bridge.mutations.push(DomMutation::CreateElement {
                    virtual_id: new_id,
                    tag: String::from("#text"),
                });

                let mut obj = JsObject::new();
                obj.set(String::from("__nodeId"), JsValue::Number(new_id as f64));
                obj.set(String::from("nodeType"), JsValue::Number(3.0));
                obj.set(
                    String::from("nodeName"),
                    JsValue::String(String::from("#text")),
                );
                obj.set(String::from("textContent"), JsValue::String(text));
                obj.set_hook = Some(dom_property_hook);
                obj.set_hook_data = new_id as usize as *mut u8;
                return JsValue::Object(Rc::new(RefCell::new(obj)));
            }
        } else {
            // Clone as virtual element.
            if let Some(bridge) = get_bridge(vm) {
                let new_id = bridge.alloc_virtual_id();
                bridge.virtual_nodes.push(super::VirtualNode {
                    id: new_id,
                    tag: tag.clone(),
                    attrs: attrs.clone(),
                    text_content: String::new(),
                    child_ids: Vec::new(),
                    parent_id: None,
                });
                bridge.mutations.push(DomMutation::CreateElement {
                    virtual_id: new_id,
                    tag,
                });
                for (name, value) in &attrs {
                    bridge.mutations.push(DomMutation::SetAttribute {
                        node_id: new_id,
                        name: name.clone(),
                        value: value.clone(),
                    });
                }
            }

            // Recursively clone children.
            let new_id = if let Some(bridge) = get_bridge(vm) {
                // The virtual node we just pushed is the last one; get its ID.
                bridge.virtual_nodes.last().map(|v| v.id).unwrap_or(-9999)
            } else {
                -9999
            };

            for &cid in &child_ids {
                let child_val = deep_clone_node(vm, cid);
                let child_nid = extract_node_id(&child_val);
                if child_nid != -9999 {
                    if let Some(b) = get_bridge(vm) {
                        b.mutations.push(DomMutation::AppendChild {
                            parent_id: new_id,
                            child_id: child_nid,
                        });
                        if let Some(vn) = b.get_virtual_mut(new_id) {
                            vn.child_ids.push(child_nid);
                        }
                    }
                }
            }

            return make_element(vm, new_id);
        }
    }

    JsValue::Null
}

// ═══════════════════════════════════════════════════════════
// Element native methods
// ═══════════════════════════════════════════════════════════

fn el_get_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let name = arg_string(args, 0);
    read_attribute(vm, nid, &name)
}

fn el_set_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let name = arg_string(args, 0);
    let value = arg_string(args, 1);

    // Update virtual node if applicable.
    if let Some(bridge) = get_bridge(vm) {
        if nid < 0 {
            if let Some(vn) = bridge.get_virtual_mut(nid) {
                // Update or insert attribute.
                if let Some(attr) = vn.attrs.iter_mut().find(|(k, _)| k == &name) {
                    attr.1 = value.clone();
                } else {
                    vn.attrs.push((name.clone(), value.clone()));
                }
            }
        }
        if nid >= 0 {
            bridge.mutations.push(DomMutation::SetAttribute {
                node_id: nid,
                name: name.clone(),
                value: value.clone(),
            });
        }
    }

    // Update cached properties on `this`.
    if let JsValue::Object(obj) = &vm.current_this {
        let mut o = obj.borrow_mut();
        if name == "id" {
            o.set(String::from("id"), JsValue::String(value.clone()));
        }
        if name == "class" {
            o.set(String::from("className"), JsValue::String(value.clone()));
        }
        if name == "value" {
            o.set(String::from("value"), JsValue::String(value));
        }
    }
    JsValue::Undefined
}

fn el_remove_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let name = arg_string(args, 0);

    if let Some(bridge) = get_bridge(vm) {
        if nid < 0 {
            if let Some(vn) = bridge.get_virtual_mut(nid) {
                vn.attrs.retain(|(k, _)| k != &name);
            }
        }
        if nid >= 0 {
            bridge
                .mutations
                .push(DomMutation::RemoveAttribute { node_id: nid, name });
        }
    }
    JsValue::Undefined
}

fn el_has_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let name = arg_string(args, 0);
    let val = read_attribute(vm, nid, &name);
    JsValue::Bool(!matches!(val, JsValue::Null))
}

fn el_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let event = arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    // Third argument: boolean or EventListenerOptions { capture: bool }.
    // Per W3C DOM §2.9 — defaults to false (bubble phase).
    let capture = match args.get(2) {
        Some(JsValue::Bool(b)) => *b,
        Some(JsValue::Object(_)) => args[2].get_property("capture").to_boolean(),
        _ => false,
    };
    if let Some(bridge) = get_bridge(vm) {
        bridge.event_listeners.push(super::EventListener {
            node_id: if nid >= 0 { nid as usize } else { usize::MAX },
            event,
            callback,
            capture,
        });
    }
    JsValue::Undefined
}

fn el_dispatch_event(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(true)
}

// ── Query methods ──

fn el_query_selector(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() {
        return JsValue::Null;
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        if let Some(id) = selector::find_first(dom, &sel) {
            return make_element(vm, id as i64);
        }
    }
    JsValue::Null
}

fn el_query_selector_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() {
        return make_array(Vec::new());
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let ids = selector::find_all(dom, &sel);
        let elements: Vec<JsValue> = ids.iter().map(|&id| make_element(vm, id as i64)).collect();
        return make_array(elements);
    }
    make_array(Vec::new())
}

fn el_get_elements_by_tag_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag_name = arg_string(args, 0).to_ascii_uppercase();
    let mut ids = Vec::new();
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let target = crate::dom::Tag::from_str(&tag_name);
        for (i, node) in dom.nodes.iter().enumerate() {
            if let crate::dom::NodeType::Element { tag, .. } = &node.node_type {
                if *tag == target || tag_name == "*" {
                    ids.push(i as i64);
                }
            }
        }
    }
    let results: Vec<JsValue> = ids.iter().map(|&id| make_element(vm, id)).collect();
    make_array(results)
}

fn el_get_elements_by_class_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class_name = arg_string(args, 0);
    if class_name.is_empty() {
        return make_array(Vec::new());
    }
    let mut ids = Vec::new();
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let crate::dom::NodeType::Element { attrs, .. } = &node.node_type {
                for a in attrs {
                    if a.name == "class" && a.value.split_whitespace().any(|c| c == class_name) {
                        ids.push(i as i64);
                        break;
                    }
                }
            }
        }
    }
    let results: Vec<JsValue> = ids.iter().map(|&id| make_element(vm, id)).collect();
    make_array(results)
}

// ── Tree manipulation ──

fn el_append_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    let child_id = extract_node_id(&child);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::AppendChild {
            parent_id,
            child_id,
        });
        if parent_id < 0 {
            if let Some(vn) = bridge.get_virtual_mut(parent_id) {
                vn.child_ids.push(child_id);
            }
        }
    }

    // Update JS-side tree on `this`.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            arr.borrow_mut().push(child.clone());
        }
        refresh_element_children_metadata(&vm.current_this);
    }

    // Set child.parentNode = this.
    if let JsValue::Object(cobj) = &child {
        let this_clone = vm.current_this.clone();
        let mut c = cobj.borrow_mut();
        c.set(String::from("parentNode"), this_clone.clone());
        c.set(String::from("parentElement"), this_clone);
    }

    child
}

fn el_remove_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    let child_id = extract_node_id(&child);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::RemoveChild {
            parent_id,
            child_id,
        });
        if parent_id < 0 {
            if let Some(vn) = bridge.get_virtual_mut(parent_id) {
                vn.child_ids.retain(|&id| id != child_id);
            }
        }
    }

    // Remove from JS-side children array.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            arr.borrow_mut()
                .elements
                .retain(|_k, el| extract_node_id(el) != child_id);
        }
        refresh_element_children_metadata(&vm.current_this);
    }

    // Clear child.parentNode.
    if let JsValue::Object(cobj) = &child {
        let mut c = cobj.borrow_mut();
        c.set(String::from("parentNode"), JsValue::Null);
        c.set(String::from("parentElement"), JsValue::Null);
    }

    child
}

fn el_insert_before(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let new_node = args.first().cloned().unwrap_or(JsValue::Null);
    let ref_node = args.get(1).cloned().unwrap_or(JsValue::Null);
    let new_id = extract_node_id(&new_node);
    let ref_id = extract_node_id(&ref_node);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::InsertBefore {
            parent_id,
            new_child_id: new_id,
            ref_child_id: ref_id,
        });
    }

    // Insert in JS-side children array.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            let mut a = arr.borrow_mut();
            let idx = a
                .elements
                .iter()
                .find(|(_k, el)| extract_node_id(el) == ref_id)
                .map(|(k, _)| *k);
            if let Some(i) = idx {
                a.insert_and_shift(i, new_node.clone());
            } else {
                a.push(new_node.clone());
            }
        }
        refresh_element_children_metadata(&vm.current_this);
    }

    // Set parentNode.
    if let JsValue::Object(nobj) = &new_node {
        let this_clone = vm.current_this.clone();
        let mut n = nobj.borrow_mut();
        n.set(String::from("parentNode"), this_clone.clone());
        n.set(String::from("parentElement"), this_clone);
    }

    new_node
}

fn el_replace_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let new_node = args.first().cloned().unwrap_or(JsValue::Null);
    let old_node = args.get(1).cloned().unwrap_or(JsValue::Null);
    let new_id = extract_node_id(&new_node);
    let old_id = extract_node_id(&old_node);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::ReplaceChild {
            parent_id,
            new_child_id: new_id,
            old_child_id: old_id,
        });
    }

    // Replace in JS-side children.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            let mut a = arr.borrow_mut();
            if let Some(idx) = a
                .elements
                .iter()
                .find(|(_k, el)| extract_node_id(el) == old_id)
                .map(|(k, _)| *k)
            {
                a.elements.insert(idx, new_node.clone());
            }
        }
        refresh_element_children_metadata(&vm.current_this);
    }

    old_node
}

fn el_clone_node(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    make_element(vm, nid)
}

fn el_contains(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let other = args.first().cloned().unwrap_or(JsValue::Null);
    let other_id = extract_node_id(&other);
    if other_id == -9999 || nid < 0 || other_id < 0 {
        return JsValue::Bool(false);
    }
    // A node contains itself (per W3C DOM §4.4).
    if nid == other_id {
        return JsValue::Bool(true);
    }
    // Walk from other_id up to the root, checking if we reach nid.
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let mut cur = Some(other_id as usize);
        while let Some(id) = cur {
            if id == nid as usize {
                return JsValue::Bool(true);
            }
            cur = dom.nodes.get(id).and_then(|n| n.parent);
        }
    }
    JsValue::Bool(false)
}

fn el_remove(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        bridge
            .mutations
            .push(DomMutation::RemoveNode { node_id: nid });
    }
    JsValue::Undefined
}

// ── Content setters ──

fn el_set_text_content(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let text = arg_string(args, 0);

    if let Some(bridge) = get_bridge(vm) {
        if nid < 0 {
            if let Some(vn) = bridge.get_virtual_mut(nid) {
                vn.text_content = text.clone();
            }
        }
        if nid >= 0 {
            bridge.mutations.push(DomMutation::SetTextContent {
                node_id: nid,
                text: text.clone(),
            });
        }
    }

    if let JsValue::Object(obj) = &vm.current_this {
        let mut o = obj.borrow_mut();
        o.set(String::from("textContent"), JsValue::String(text.clone()));
        o.set(String::from("innerText"), JsValue::String(text));
    }
    JsValue::Undefined
}

fn el_set_inner_html(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let html = arg_string(args, 0);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetInnerHTML {
            node_id: nid,
            html: html.clone(),
        });
    }

    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut()
            .set(String::from("innerHTML"), JsValue::String(html));
    }
    JsValue::Undefined
}

fn el_set_style(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let prop = arg_string(args, 0);
    let val = arg_string(args, 1);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetStyleProperty {
            node_id: nid,
            property: prop.clone(),
            value: val.clone(),
        });
    }

    // Update style object on this.
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(sp) = o.properties.get("style") {
            sp.value.set_property(prop, JsValue::String(val));
        }
    }
    JsValue::Undefined
}

// ── Misc ──

fn el_get_bounding_rect(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let rect = JsValue::new_object();
    let (top, left, width, height) = if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        let top = match o.get("offsetTop") {
            JsValue::Number(n) => n,
            _ => 0.0,
        };
        let left = match o.get("offsetLeft") {
            JsValue::Number(n) => n,
            _ => 0.0,
        };
        let width = match o.get("offsetWidth") {
            JsValue::Number(n) => n,
            _ => 0.0,
        };
        let height = match o.get("offsetHeight") {
            JsValue::Number(n) => n,
            _ => 0.0,
        };
        (top, left, width, height)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    rect.set_property(String::from("top"), JsValue::Number(top));
    rect.set_property(String::from("left"), JsValue::Number(left));
    rect.set_property(String::from("bottom"), JsValue::Number(top + height));
    rect.set_property(String::from("right"), JsValue::Number(left + width));
    rect.set_property(String::from("width"), JsValue::Number(width));
    rect.set_property(String::from("height"), JsValue::Number(height));
    rect.set_property(String::from("x"), JsValue::Number(left));
    rect.set_property(String::from("y"), JsValue::Number(top));
    rect
}

fn el_get_client_rects(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_array(Vec::new())
}

fn el_to_string(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("[object HTMLElement]"))
}

// ── ParentNode: prepend / append / replaceChildren (W3C DOM §4.2.6) ──

fn el_prepend(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let parent_js = vm.current_this.clone();
    for arg in args {
        let child_id = extract_node_id(arg);
        if let Some(bridge) = get_bridge(vm) {
            // InsertBefore the first child — we use the first child as ref.
            // If no first child, this degrades to AppendChild.
            let first_child_id = if parent_id >= 0 {
                bridge
                    .dom()
                    .nodes
                    .get(parent_id as usize)
                    .and_then(|n| n.children.first().copied())
                    .map(|id| id as i64)
            } else {
                bridge
                    .get_virtual(parent_id)
                    .and_then(|vn| vn.child_ids.first().copied())
            };
            if let Some(ref_id) = first_child_id {
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id,
                    new_child_id: child_id,
                    ref_child_id: ref_id,
                });
            } else {
                bridge.mutations.push(DomMutation::AppendChild {
                    parent_id,
                    child_id,
                });
            }
        }
        js_prepend_child(&parent_js, arg.clone());
    }
    JsValue::Undefined
}

fn el_append(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    let parent_js = vm.current_this.clone();
    for arg in args {
        let child_id = extract_node_id(arg);
        if let Some(bridge) = get_bridge(vm) {
            bridge.mutations.push(DomMutation::AppendChild {
                parent_id,
                child_id,
            });
            if parent_id < 0 {
                if let Some(vn) = bridge.get_virtual_mut(parent_id) {
                    vn.child_ids.push(child_id);
                }
            }
        }
        js_append_child(&parent_js, arg.clone());
    }
    JsValue::Undefined
}

fn el_replace_children(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        // Remove all existing children.
        let child_ids: Vec<i64> = if parent_id >= 0 {
            bridge
                .dom()
                .nodes
                .get(parent_id as usize)
                .map(|n| n.children.iter().map(|&id| id as i64).collect())
                .unwrap_or_default()
        } else {
            bridge
                .get_virtual(parent_id)
                .map(|vn| vn.child_ids.clone())
                .unwrap_or_default()
        };
        for cid in &child_ids {
            bridge.mutations.push(DomMutation::RemoveChild {
                parent_id,
                child_id: *cid,
            });
        }
        // Append new children.
        for arg in args {
            let child_id = extract_node_id(arg);
            bridge.mutations.push(DomMutation::AppendChild {
                parent_id,
                child_id,
            });
        }
    }
    js_replace_children(&vm.current_this, args);
    JsValue::Undefined
}

// ── ChildNode: before / after / replaceWith (W3C DOM §4.2.7) ──

fn el_before(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let this_js = vm.current_this.clone();
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge
                .dom()
                .nodes
                .get(nid as usize)
                .and_then(|n| n.parent)
                .map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };
        if let Some(pid) = parent_id {
            for arg in args {
                let child_id = extract_node_id(arg);
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id: pid,
                    new_child_id: child_id,
                    ref_child_id: nid,
                });
            }
        }
    }
    let parent_js = this_js.get_property("parentNode");
    if !parent_js.is_null() && !parent_js.is_undefined() {
        for arg in args {
            js_insert_before(&parent_js, arg.clone(), &this_js);
        }
    }
    JsValue::Undefined
}

fn el_after(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let this_js = vm.current_this.clone();
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge
                .dom()
                .nodes
                .get(nid as usize)
                .and_then(|n| n.parent)
                .map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };
        if let Some(pid) = parent_id {
            // Find next sibling for InsertBefore, or AppendChild if last.
            let next_sib_id = if nid >= 0 {
                let dom = bridge.dom();
                if let Some(parent) = dom.nodes.get(pid as usize) {
                    let pos = parent.children.iter().position(|&c| c == nid as usize);
                    pos.and_then(|p| parent.children.get(p + 1).map(|&c| c as i64))
                } else {
                    None
                }
            } else {
                None
            };

            for arg in args {
                let child_id = extract_node_id(arg);
                if let Some(ref_id) = next_sib_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid,
                        new_child_id: child_id,
                        ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild {
                        parent_id: pid,
                        child_id,
                    });
                }
            }
        }
    }
    let parent_js = this_js.get_property("parentNode");
    if !parent_js.is_null() && !parent_js.is_undefined() {
        let next_js = this_js.get_property("nextSibling");
        for arg in args {
            if next_js.is_null() || next_js.is_undefined() {
                js_append_child(&parent_js, arg.clone());
            } else {
                js_insert_before(&parent_js, arg.clone(), &next_js);
            }
        }
    }
    JsValue::Undefined
}

fn el_replace_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let this_js = vm.current_this.clone();
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge
                .dom()
                .nodes
                .get(nid as usize)
                .and_then(|n| n.parent)
                .map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };
        if let Some(pid) = parent_id {
            // Insert each new node before this, then remove this.
            for arg in args {
                let child_id = extract_node_id(arg);
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id: pid,
                    new_child_id: child_id,
                    ref_child_id: nid,
                });
            }
            bridge.mutations.push(DomMutation::RemoveChild {
                parent_id: pid,
                child_id: nid,
            });
        }
    }
    let parent_js = this_js.get_property("parentNode");
    if !parent_js.is_null() && !parent_js.is_undefined() {
        for arg in args {
            js_insert_before(&parent_js, arg.clone(), &this_js);
        }
        js_remove_child(&parent_js, extract_node_id(&this_js));
        clear_js_parent_links(&this_js);
    }
    JsValue::Undefined
}

// ── insertAdjacentHTML / Element / Text (W3C DOM Parsing §4) ──

fn el_insert_adjacent_html(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let position = arg_string(args, 0).to_ascii_lowercase();
    let html = arg_string(args, 1);
    if let Some(bridge) = get_bridge(vm) {
        // Create a virtual fragment to hold the parsed HTML.
        let frag_id = bridge.alloc_virtual_id();
        bridge.mutations.push(DomMutation::CreateElement {
            virtual_id: frag_id,
            tag: String::from("div"),
        });
        bridge.mutations.push(DomMutation::SetInnerHTML {
            node_id: frag_id,
            html,
        });

        let parent_id = if nid >= 0 {
            bridge
                .dom()
                .nodes
                .get(nid as usize)
                .and_then(|n| n.parent)
                .map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };

        match position.as_str() {
            "beforebegin" => {
                // Before the element itself.
                if let Some(pid) = parent_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid,
                        new_child_id: frag_id,
                        ref_child_id: nid,
                    });
                }
            }
            "afterbegin" => {
                // Inside the element, before its first child.
                let first_child_id = if nid >= 0 {
                    bridge
                        .dom()
                        .nodes
                        .get(nid as usize)
                        .and_then(|n| n.children.first().copied())
                        .map(|id| id as i64)
                } else {
                    None
                };
                if let Some(ref_id) = first_child_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: nid,
                        new_child_id: frag_id,
                        ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild {
                        parent_id: nid,
                        child_id: frag_id,
                    });
                }
            }
            "beforeend" => {
                // Inside the element, after its last child.
                bridge.mutations.push(DomMutation::AppendChild {
                    parent_id: nid,
                    child_id: frag_id,
                });
            }
            "afterend" => {
                // After the element itself.
                if let Some(pid) = parent_id {
                    let next_sib_id = if nid >= 0 {
                        let dom = bridge.dom();
                        if let Some(parent) = dom.nodes.get(pid as usize) {
                            let pos = parent.children.iter().position(|&c| c == nid as usize);
                            pos.and_then(|p| parent.children.get(p + 1).map(|&c| c as i64))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some(ref_id) = next_sib_id {
                        bridge.mutations.push(DomMutation::InsertBefore {
                            parent_id: pid,
                            new_child_id: frag_id,
                            ref_child_id: ref_id,
                        });
                    } else {
                        bridge.mutations.push(DomMutation::AppendChild {
                            parent_id: pid,
                            child_id: frag_id,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    JsValue::Undefined
}

fn el_insert_adjacent_element(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let position = arg_string(args, 0).to_ascii_lowercase();
    let element = args.get(1).cloned().unwrap_or(JsValue::Null);
    let child_id = extract_node_id(&element);
    if child_id == -9999 {
        return JsValue::Null;
    }

    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge
                .dom()
                .nodes
                .get(nid as usize)
                .and_then(|n| n.parent)
                .map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };

        match position.as_str() {
            "beforebegin" => {
                if let Some(pid) = parent_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid,
                        new_child_id: child_id,
                        ref_child_id: nid,
                    });
                }
            }
            "afterbegin" => {
                let first = if nid >= 0 {
                    bridge
                        .dom()
                        .nodes
                        .get(nid as usize)
                        .and_then(|n| n.children.first().copied())
                        .map(|id| id as i64)
                } else {
                    None
                };
                if let Some(ref_id) = first {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: nid,
                        new_child_id: child_id,
                        ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild {
                        parent_id: nid,
                        child_id,
                    });
                }
            }
            "beforeend" => {
                bridge.mutations.push(DomMutation::AppendChild {
                    parent_id: nid,
                    child_id,
                });
            }
            "afterend" => {
                if let Some(pid) = parent_id {
                    let next = if nid >= 0 {
                        let dom = bridge.dom();
                        dom.nodes.get(pid as usize).and_then(|p| {
                            let pos = p.children.iter().position(|&c| c == nid as usize);
                            pos.and_then(|i| p.children.get(i + 1).map(|&c| c as i64))
                        })
                    } else {
                        None
                    };
                    if let Some(ref_id) = next {
                        bridge.mutations.push(DomMutation::InsertBefore {
                            parent_id: pid,
                            new_child_id: child_id,
                            ref_child_id: ref_id,
                        });
                    } else {
                        bridge.mutations.push(DomMutation::AppendChild {
                            parent_id: pid,
                            child_id,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    element
}

fn el_insert_adjacent_text(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Create a text node and use insertAdjacentElement logic.
    let position = arg_string(args, 0);
    let text = arg_string(args, 1);
    if let Some(bridge) = get_bridge(vm) {
        let text_id = bridge.alloc_virtual_id();
        bridge.virtual_nodes.push(super::VirtualNode {
            id: text_id,
            tag: String::from("#text"),
            attrs: Vec::new(),
            text_content: text,
            child_ids: Vec::new(),
            parent_id: None,
        });
        bridge.mutations.push(DomMutation::CreateElement {
            virtual_id: text_id,
            tag: String::from("#text"),
        });
        // Re-call with the created text node.
        let text_el = make_element(vm, text_id);
        let pos_val = JsValue::String(position);
        return el_insert_adjacent_element(vm, &[pos_val, text_el]);
    }
    JsValue::Undefined
}

// ── matches / closest (W3C DOM §4.4) ──

fn el_matches(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 {
        return JsValue::Bool(false);
    }
    let sel = arg_string(args, 0);
    if sel.is_empty() {
        return JsValue::Bool(false);
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let ids = selector::find_all(dom, &sel);
        return JsValue::Bool(ids.contains(&(nid as usize)));
    }
    JsValue::Bool(false)
}

fn el_closest(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 {
        return JsValue::Null;
    }
    let sel = arg_string(args, 0);
    if sel.is_empty() {
        return JsValue::Null;
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let matching_ids = selector::find_all(dom, &sel);
        // Walk from this node upward through ancestors.
        let mut cur = Some(nid as usize);
        while let Some(id) = cur {
            if matching_ids.contains(&id) {
                return make_element(vm, id as i64);
            }
            cur = dom.nodes.get(id).and_then(|n| n.parent);
        }
    }
    JsValue::Null
}

// ── getRootNode (W3C DOM §4.4.3) ──

fn el_get_root_node(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 {
        return vm.current_this.clone();
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let mut cur = nid as usize;
        while let Some(parent) = dom.nodes.get(cur).and_then(|n| n.parent) {
            cur = parent;
        }
        return make_element(vm, cur as i64);
    }
    vm.current_this.clone()
}

fn el_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

/// `form.checkValidity()` — validates all descendant form controls.
fn el_form_check_validity(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let form_nid = this_node_id(vm);
    if form_nid < 0 {
        return JsValue::Bool(true);
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        // Walk all nodes in the DOM and validate those that are children of this form.
        for i in 0..dom.nodes.len() {
            let tag = dom.tag(i);
            if !matches!(
                tag,
                Some(crate::dom::Tag::Input)
                    | Some(crate::dom::Tag::Select)
                    | Some(crate::dom::Tag::Textarea)
            ) {
                continue;
            }
            // Check if this node is a descendant of the form.
            let mut cur = dom.nodes[i].parent;
            let mut is_child = false;
            while let Some(pid) = cur {
                if pid == form_nid as usize {
                    is_child = true;
                    break;
                }
                cur = dom.nodes[pid].parent;
            }
            if is_child {
                let r = crate::dom::validate_form_control(dom, i);
                if !r.is_valid() {
                    return JsValue::Bool(false);
                }
            }
        }
        JsValue::Bool(true)
    } else {
        JsValue::Bool(true)
    }
}

/// `element.reportValidity()` — validates and returns result.
/// Per W3C, this reports validation errors to the user. In our implementation
/// it runs the same checks as checkValidity() — the browser UI layer (Surf)
/// can show error bubbles based on the result.
fn el_report_validity(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    el_check_validity(vm, _args)
}

/// `element.checkValidity()` — real constraint validation.
fn el_check_validity(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 {
        return JsValue::Bool(true);
    }
    if let Some(bridge) = get_bridge(vm) {
        let r = crate::dom::validate_form_control(bridge.dom(), nid as usize);
        JsValue::Bool(r.is_valid())
    } else {
        JsValue::Bool(true)
    }
}

/// `element.validity` — returns a ValidityState object with real constraint check results.
fn el_get_validity(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let r = if nid >= 0 {
        if let Some(bridge) = get_bridge(vm) {
            crate::dom::validate_form_control(bridge.dom(), nid as usize)
        } else {
            crate::dom::ValidationResult::default()
        }
    } else {
        crate::dom::ValidationResult::default()
    };
    let mut obj = JsObject::new();
    obj.set(String::from("valid"), JsValue::Bool(r.is_valid()));
    obj.set(String::from("valueMissing"), JsValue::Bool(r.value_missing));
    obj.set(String::from("typeMismatch"), JsValue::Bool(r.type_mismatch));
    obj.set(
        String::from("patternMismatch"),
        JsValue::Bool(r.pattern_mismatch),
    );
    obj.set(String::from("tooLong"), JsValue::Bool(r.too_long));
    obj.set(String::from("tooShort"), JsValue::Bool(r.too_short));
    obj.set(
        String::from("rangeUnderflow"),
        JsValue::Bool(r.range_underflow),
    );
    obj.set(
        String::from("rangeOverflow"),
        JsValue::Bool(r.range_overflow),
    );
    obj.set(String::from("stepMismatch"), JsValue::Bool(r.step_mismatch));
    obj.set(String::from("badInput"), JsValue::Bool(r.bad_input));
    obj.set(String::from("customError"), JsValue::Bool(false));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `element.scrollTo(x, y)` or `element.scrollTo({ top, left })`.
/// Sets both scrollTop and scrollLeft on the element via DomMutations.
fn el_scroll_to(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 {
        return JsValue::Undefined;
    }
    let (sx, sy, smooth) = parse_scroll_args(args);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetScrollTop {
            node_id: nid as usize,
            value: sy.max(0.0) as i32,
            smooth,
        });
        bridge.mutations.push(DomMutation::SetScrollLeft {
            node_id: nid as usize,
            value: sx.max(0.0) as i32,
            smooth,
        });
    }
    JsValue::Undefined
}

/// `element.scrollBy(x, y)` or `element.scrollBy({ top, left })`.
/// Adjusts scrollTop/scrollLeft relative to current values.
/// Since we don't have the current scroll position in JS, we approximate
/// by emitting an additive mutation.  The host will clamp to valid range.
fn el_scroll_by(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // scrollBy is equivalent to scrollTo with the deltas added to current.
    // Since the JS property objects report 0 for scrollTop/scrollLeft,
    // we read the current values from the property and add.
    let nid = this_node_id(vm);
    if nid < 0 {
        return JsValue::Undefined;
    }
    let (dx, dy, smooth) = parse_scroll_args(args);

    // Read current scrollTop/scrollLeft from the JS object.
    let (cur_top, cur_left) = {
        let this = vm.current_this.clone();
        let st = match &this {
            JsValue::Object(o) => {
                let obj = o.borrow();
                match obj.get("scrollTop") {
                    JsValue::Number(n) => n,
                    _ => 0.0,
                }
            }
            _ => 0.0,
        };
        let sl = match &this {
            JsValue::Object(o) => {
                let obj = o.borrow();
                match obj.get("scrollLeft") {
                    JsValue::Number(n) => n,
                    _ => 0.0,
                }
            }
            _ => 0.0,
        };
        (st, sl)
    };

    let new_top = ((cur_top + dy) as i32).max(0);
    let new_left = ((cur_left + dx) as i32).max(0);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetScrollTop {
            node_id: nid as usize,
            value: new_top,
            smooth,
        });
        bridge.mutations.push(DomMutation::SetScrollLeft {
            node_id: nid as usize,
            value: new_left,
            smooth,
        });
    }
    JsValue::Undefined
}

/// Parse scrollTo/scrollBy arguments: either `(x, y)` or `({ top, left, behavior })`.
/// Returns `(left, top, Some(true)=smooth, Some(false)=auto, None=use CSS behavior)`.
fn parse_scroll_args(args: &[JsValue]) -> (f64, f64, Option<bool>) {
    if args.is_empty() {
        return (0.0, 0.0, None);
    }
    match &args[0] {
        JsValue::Object(o) => {
            let obj = o.borrow();
            let top = match obj.get("top") {
                JsValue::Number(n) => n,
                _ => 0.0,
            };
            let left = match obj.get("left") {
                JsValue::Number(n) => n,
                _ => 0.0,
            };
            let smooth = match obj.get("behavior") {
                JsValue::String(s) if s.eq_ignore_ascii_case("smooth") => Some(true),
                JsValue::String(s) if s.eq_ignore_ascii_case("auto") => Some(false),
                _ => None,
            };
            (left, top, smooth)
        }
        JsValue::Number(x) => {
            let y = if args.len() > 1 {
                match &args[1] {
                    JsValue::Number(n) => *n,
                    _ => 0.0,
                }
            } else {
                0.0
            };
            (*x, y, None)
        }
        _ => (0.0, 0.0, None),
    }
}

// ═══════════════════════════════════════════════════════════
// CSSStyleDeclaration (W3C CSSOM §6.7.2)
// ═══════════════════════════════════════════════════════════

/// Create a CSSStyleDeclaration object for an element.
///
/// This object acts like `element.style` with:
/// - `setProperty(name, value)` / `getPropertyValue(name)` / `removeProperty(name)`
/// - `cssText` getter
/// - Direct property assignment (`style.color = "red"`) via set_hook
fn make_css_style_declaration(node_id: i64) -> JsValue {
    let mut sobj = JsObject::new();

    // `__nodeId` so native methods can find the element.
    sobj.set(String::from("__nodeId"), JsValue::Number(node_id as f64));

    // setProperty(propertyName, value [, priority])
    sobj.set(
        String::from("setProperty"),
        native_fn("setProperty", style_set_property),
    );
    // getPropertyValue(propertyName)
    sobj.set(
        String::from("getPropertyValue"),
        native_fn("getPropertyValue", style_get_property_value),
    );
    // removeProperty(propertyName) — returns old value
    sobj.set(
        String::from("removeProperty"),
        native_fn("removeProperty", style_remove_property),
    );
    // getPropertyPriority(propertyName)
    sobj.set(
        String::from("getPropertyPriority"),
        native_fn("getPropertyPriority", |_, _| JsValue::String(String::new())),
    );
    // cssText
    sobj.set(String::from("cssText"), JsValue::String(String::new()));
    // length
    sobj.set(String::from("length"), JsValue::Number(0.0));
    // item(index)
    sobj.set(
        String::from("item"),
        native_fn("item", |_, _| JsValue::String(String::new())),
    );

    // set_hook: intercepts `style.color = "red"` and generates SetStyleProperty mutations.
    sobj.set_hook = Some(style_property_hook);
    sobj.set_hook_data = node_id as usize as *mut u8;

    JsValue::Object(Rc::new(RefCell::new(sobj)))
}

/// setProperty(name, value) — sets a CSS property and emits a SetStyleProperty mutation.
fn style_set_property(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = css_prop_from_camel(&arg_string(args, 0));
    let val = arg_string(args, 1);

    // Update on the style object itself.
    let camel = camel_case(&prop);
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(camel, JsValue::String(val.clone()));
    }

    // Emit mutation.
    let nid = super::this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetStyleProperty {
            node_id: nid,
            property: prop,
            value: val,
        });
    }
    JsValue::Undefined
}

/// getPropertyValue(name) — returns the current value of a CSS property.
fn style_get_property_value(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = arg_string(args, 0);
    let camel = camel_case(&prop);
    if let JsValue::Object(obj) = &vm.current_this {
        let val = obj.borrow().get(&camel);
        if !val.is_undefined() && !val.is_null() {
            return JsValue::String(val.to_js_string());
        }
    }
    JsValue::String(String::new())
}

/// removeProperty(name) — removes a CSS property, returns old value.
fn style_remove_property(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = css_prop_from_camel(&arg_string(args, 0));
    let camel = camel_case(&prop);
    let old = if let JsValue::Object(obj) = &vm.current_this {
        let old_val = obj.borrow().get(&camel);
        obj.borrow_mut().delete(&camel);
        old_val
    } else {
        JsValue::String(String::new())
    };

    let nid = super::this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetStyleProperty {
            node_id: nid,
            property: prop,
            value: String::new(),
        });
    }
    old
}

/// Hook for direct property assignment on style objects (`style.color = "red"`).
/// Converts camelCase to kebab-case and emits a SetStyleProperty mutation.
fn style_property_hook(data: *mut u8, key: &str, value: &JsValue) {
    // Skip internal properties.
    match key {
        "__nodeId"
        | "setProperty"
        | "getPropertyValue"
        | "removeProperty"
        | "getPropertyPriority"
        | "cssText"
        | "length"
        | "item" => return,
        _ => {}
    }
    let node_id = data as usize as i64;
    let css_prop = css_prop_from_camel(key);
    let val_str = value.to_js_string();

    // For virtual nodes: store as "style" attribute on VirtualNode
    if node_id < 0 {
        let vnodes = unsafe {
            if super::VIRTUAL_NODES_TARGET.is_null() {
                return;
            }
            &mut *super::VIRTUAL_NODES_TARGET
        };
        if let Some(vn) = vnodes.iter_mut().find(|v| v.id == node_id) {
            // Append to existing style attribute
            if let Some(attr) = vn.attrs.iter_mut().find(|(k, _)| k == "style") {
                if attr.1.is_empty() {
                    attr.1 = alloc::format!("{}: {}", css_prop, val_str);
                } else {
                    attr.1 = alloc::format!("{}; {}: {}", attr.1, css_prop, val_str);
                }
            } else {
                vn.attrs.push((
                    String::from("style"),
                    alloc::format!("{}: {}", css_prop, val_str),
                ));
            }
        }
        return;
    }

    let mutations = unsafe {
        if super::MUTATION_TARGET.is_null() {
            return;
        }
        &mut *super::MUTATION_TARGET
    };
    mutations.push(DomMutation::SetStyleProperty {
        node_id,
        property: css_prop,
        value: val_str,
    });
}

/// Convert camelCase CSS property name to kebab-case.
/// e.g. `backgroundColor` → `background-color`, `cssFloat` → `float`
fn css_prop_from_camel(name: &str) -> String {
    if name == "cssFloat" {
        return String::from("float");
    }
    if name.contains('-') {
        return String::from(name);
    } // already kebab
    let mut out = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Convert kebab-case CSS property to camelCase.
/// e.g. `background-color` → `backgroundColor`, `float` → `cssFloat`
fn camel_case(name: &str) -> String {
    if name == "float" {
        return String::from("cssFloat");
    }
    if !name.contains('-') {
        return String::from(name);
    }
    let mut out = String::with_capacity(name.len());
    let mut capitalize_next = false;
    for ch in name.chars() {
        if ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            out.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

/// Extract `__nodeId` from a JsValue element object (public for sibling modules).
pub fn extract_node_id_pub(val: &JsValue) -> i64 {
    extract_node_id(val)
}

/// Extract __nodeId from a JsValue (element object).
fn extract_node_id(val: &JsValue) -> i64 {
    if let JsValue::Object(obj) = val {
        if let Some(prop) = obj.borrow().properties.get("__nodeId") {
            return prop.value.to_number() as i64;
        }
    }
    -9999
}

/// Extract first and last child from a children JsValue (array).
fn get_first_last(children: &JsValue) -> (JsValue, JsValue) {
    if let JsValue::Array(arr) = children {
        let elems = &arr.borrow().elements;
        if !elems.is_empty() {
            let first = elems.values().next().cloned().unwrap_or(JsValue::Null);
            let last = elems.values().next_back().cloned().unwrap_or(JsValue::Null);
            return (first, last);
        }
    }
    (JsValue::Null, JsValue::Null)
}

pub(super) fn refresh_element_children_metadata(parent: &JsValue) {
    let children = parent.get_property("children");
    let ordered_children = match &children {
        JsValue::Array(arr) => arr.borrow().values_vec(),
        _ => Vec::new(),
    };

    let (first, last) = get_first_last(&children);
    if let JsValue::Object(obj) = parent {
        let mut o = obj.borrow_mut();
        o.set(String::from("firstChild"), first);
        o.set(String::from("lastChild"), last);
        o.set(String::from("childNodes"), children.clone());
        let child_element_count = ordered_children
            .iter()
            .filter(|child| child.get_property("nodeType").to_number() == 1.0)
            .count() as f64;
        o.set(
            String::from("childElementCount"),
            JsValue::Number(child_element_count),
        );
    }

    for (idx, child) in ordered_children.iter().enumerate() {
        if let JsValue::Object(cobj) = child {
            let prev = if idx > 0 {
                ordered_children[idx - 1].clone()
            } else {
                JsValue::Null
            };
            let next = if idx + 1 < ordered_children.len() {
                ordered_children[idx + 1].clone()
            } else {
                JsValue::Null
            };
            let mut c = cobj.borrow_mut();
            c.set(String::from("parentNode"), parent.clone());
            c.set(String::from("parentElement"), parent.clone());
            c.set(String::from("previousSibling"), prev.clone());
            c.set(String::from("nextSibling"), next.clone());
            c.set(String::from("previousElementSibling"), prev);
            c.set(String::from("nextElementSibling"), next);
        }
    }
}

pub(super) fn clear_js_parent_links(child: &JsValue) {
    if let JsValue::Object(cobj) = child {
        let mut c = cobj.borrow_mut();
        c.set(String::from("parentNode"), JsValue::Null);
        c.set(String::from("parentElement"), JsValue::Null);
        c.set(String::from("previousSibling"), JsValue::Null);
        c.set(String::from("nextSibling"), JsValue::Null);
        c.set(String::from("previousElementSibling"), JsValue::Null);
        c.set(String::from("nextElementSibling"), JsValue::Null);
    }
}

pub(super) fn js_remove_child(parent: &JsValue, child_id: i64) {
    if child_id == -9999 {
        return;
    }
    let children = parent.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut()
            .elements
            .retain(|_k, el| extract_node_id(el) != child_id);
        refresh_element_children_metadata(parent);
    }
}

fn js_detach_from_old_parent(child: &JsValue) {
    let old_parent = child.get_property("parentNode");
    let child_id = extract_node_id(child);
    if !old_parent.is_null() && !old_parent.is_undefined() {
        js_remove_child(&old_parent, child_id);
    }
    clear_js_parent_links(child);
}

fn js_append_child(parent: &JsValue, child: JsValue) {
    js_detach_from_old_parent(&child);
    let children = parent.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut().push(child);
        refresh_element_children_metadata(parent);
    }
}

fn js_prepend_child(parent: &JsValue, child: JsValue) {
    js_detach_from_old_parent(&child);
    let children = parent.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut().insert_and_shift(0, child);
        refresh_element_children_metadata(parent);
    }
}

fn js_insert_before(parent: &JsValue, child: JsValue, ref_child: &JsValue) {
    js_detach_from_old_parent(&child);
    let ref_id = extract_node_id(ref_child);
    let children = parent.get_property("children");
    if let JsValue::Array(arr) = &children {
        let mut arr_mut = arr.borrow_mut();
        if ref_id == -9999 {
            arr_mut.push(child);
        } else if let Some(idx) = arr_mut
            .elements
            .iter()
            .find(|(_k, el)| extract_node_id(el) == ref_id)
            .map(|(k, _)| *k)
        {
            arr_mut.insert_and_shift(idx, child);
        } else {
            arr_mut.push(child);
        }
        refresh_element_children_metadata(parent);
    }
}

fn js_replace_children(parent: &JsValue, new_children: &[JsValue]) {
    let old_children = match parent.get_property("children") {
        JsValue::Array(arr) => arr.borrow().values_vec(),
        _ => Vec::new(),
    };
    for child in &old_children {
        clear_js_parent_links(child);
    }
    let children = parent.get_property("children");
    if let JsValue::Array(arr) = &children {
        let mut arr_mut = arr.borrow_mut();
        arr_mut.elements.clear();
        arr_mut.length = 0;
    }
    refresh_element_children_metadata(parent);
    for child in new_children {
        js_append_child(parent, child.clone());
    }
}

fn parse_dimension_attr(vm: &mut Vm, node_id: i64, name: &str) -> Option<u32> {
    match read_attribute(vm, node_id, name) {
        JsValue::String(s) => parse_dimension_str(&s),
        JsValue::Number(n) if n > 0.0 => Some(n as u32),
        _ => None,
    }
}

fn parse_dimension_str(s: &str) -> Option<u32> {
    let trimmed = s.trim().trim_end_matches("px").trim();
    trimmed.parse::<u32>().ok().filter(|v| *v > 0)
}

fn estimate_box_size(
    vm: &mut Vm,
    node_id: i64,
    tag_name: &str,
    class_name: &str,
    child_count: usize,
    type_val: &str,
) -> (u32, u32) {
    let width = parse_dimension_attr(vm, node_id, "width").unwrap_or_else(|| {
        if matches!(tag_name, "IMG" | "SVG" | "CANVAS") {
            64
        } else if tag_name == "INPUT" && matches!(type_val, "checkbox" | "radio") {
            16
        } else if child_count > 0 {
            (child_count as u32).saturating_mul(120).clamp(24, 640)
        } else {
            let text_len = read_text_content(vm, node_id).chars().count() as u32;
            (text_len.saturating_mul(8) + 16).clamp(8, 480)
        }
    });
    let height = parse_dimension_attr(vm, node_id, "height").unwrap_or_else(|| {
        if matches!(tag_name, "IMG" | "SVG" | "CANVAS") {
            32
        } else if tag_name == "INPUT" && matches!(type_val, "checkbox" | "radio") {
            16
        } else if class_name.contains("hidden") {
            0
        } else if child_count > 0 {
            (child_count as u32).saturating_mul(18).clamp(18, 320)
        } else {
            18
        }
    });
    (width.max(1), height.max(1))
}

// ═══════════════════════════════════════════════════════════
// Canvas 2D API
// ═══════════════════════════════════════════════════════════

/// `canvas.getContext('2d')` — returns a CanvasRenderingContext2D object.
fn el_get_context(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let context_type = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    if context_type != "2d" {
        return JsValue::Null; // Only 2D context supported
    }
    let canvas = vm.current_this.clone();
    make_canvas_2d_context(canvas)
}

/// Create a CanvasRenderingContext2D object with all W3C Canvas 2D API methods.
fn make_canvas_2d_context(canvas: JsValue) -> JsValue {
    let ctx = JsValue::new_object();

    // Back-reference to the canvas element
    ctx.set_property(String::from("canvas"), canvas);

    // ── State ──
    ctx.set_property(
        String::from("fillStyle"),
        JsValue::String(String::from("#000000")),
    );
    ctx.set_property(
        String::from("strokeStyle"),
        JsValue::String(String::from("#000000")),
    );
    ctx.set_property(String::from("lineWidth"), JsValue::Number(1.0));
    ctx.set_property(
        String::from("lineCap"),
        JsValue::String(String::from("butt")),
    );
    ctx.set_property(
        String::from("lineJoin"),
        JsValue::String(String::from("miter")),
    );
    ctx.set_property(String::from("miterLimit"), JsValue::Number(10.0));
    ctx.set_property(String::from("lineDashOffset"), JsValue::Number(0.0));
    ctx.set_property(
        String::from("font"),
        JsValue::String(String::from("10px sans-serif")),
    );
    ctx.set_property(
        String::from("textAlign"),
        JsValue::String(String::from("start")),
    );
    ctx.set_property(
        String::from("textBaseline"),
        JsValue::String(String::from("alphabetic")),
    );
    ctx.set_property(
        String::from("direction"),
        JsValue::String(String::from("ltr")),
    );
    ctx.set_property(String::from("globalAlpha"), JsValue::Number(1.0));
    ctx.set_property(
        String::from("globalCompositeOperation"),
        JsValue::String(String::from("source-over")),
    );
    ctx.set_property(String::from("imageSmoothingEnabled"), JsValue::Bool(true));
    ctx.set_property(String::from("shadowBlur"), JsValue::Number(0.0));
    ctx.set_property(
        String::from("shadowColor"),
        JsValue::String(String::from("rgba(0,0,0,0)")),
    );
    ctx.set_property(String::from("shadowOffsetX"), JsValue::Number(0.0));
    ctx.set_property(String::from("shadowOffsetY"), JsValue::Number(0.0));
    ctx.set_property(
        String::from("filter"),
        JsValue::String(String::from("none")),
    );

    // ── Drawing methods (stubs that record operations) ──
    let noop = native_fn("noop", |_, _| JsValue::Undefined);

    // Rectangles
    ctx.set_property(String::from("fillRect"), native_fn("fillRect", ctx_noop));
    ctx.set_property(
        String::from("strokeRect"),
        native_fn("strokeRect", ctx_noop),
    );
    ctx.set_property(String::from("clearRect"), native_fn("clearRect", ctx_noop));

    // Paths
    ctx.set_property(String::from("beginPath"), native_fn("beginPath", ctx_noop));
    ctx.set_property(String::from("closePath"), native_fn("closePath", ctx_noop));
    ctx.set_property(String::from("moveTo"), native_fn("moveTo", ctx_noop));
    ctx.set_property(String::from("lineTo"), native_fn("lineTo", ctx_noop));
    ctx.set_property(
        String::from("bezierCurveTo"),
        native_fn("bezierCurveTo", ctx_noop),
    );
    ctx.set_property(
        String::from("quadraticCurveTo"),
        native_fn("quadraticCurveTo", ctx_noop),
    );
    ctx.set_property(String::from("arc"), native_fn("arc", ctx_noop));
    ctx.set_property(String::from("arcTo"), native_fn("arcTo", ctx_noop));
    ctx.set_property(String::from("ellipse"), native_fn("ellipse", ctx_noop));
    ctx.set_property(String::from("rect"), native_fn("rect", ctx_noop));
    ctx.set_property(String::from("roundRect"), native_fn("roundRect", ctx_noop));
    ctx.set_property(String::from("fill"), native_fn("fill", ctx_noop));
    ctx.set_property(String::from("stroke"), native_fn("stroke", ctx_noop));
    ctx.set_property(String::from("clip"), native_fn("clip", ctx_noop));
    ctx.set_property(
        String::from("isPointInPath"),
        native_fn("isPointInPath", |_, _| JsValue::Bool(false)),
    );
    ctx.set_property(
        String::from("isPointInStroke"),
        native_fn("isPointInStroke", |_, _| JsValue::Bool(false)),
    );

    // Text
    ctx.set_property(String::from("fillText"), native_fn("fillText", ctx_noop));
    ctx.set_property(
        String::from("strokeText"),
        native_fn("strokeText", ctx_noop),
    );
    ctx.set_property(
        String::from("measureText"),
        native_fn("measureText", ctx_measure_text),
    );

    // Drawing images
    ctx.set_property(String::from("drawImage"), native_fn("drawImage", ctx_noop));

    // Pixel manipulation
    ctx.set_property(
        String::from("createImageData"),
        native_fn("createImageData", ctx_create_image_data),
    );
    ctx.set_property(
        String::from("getImageData"),
        native_fn("getImageData", ctx_get_image_data),
    );
    ctx.set_property(
        String::from("putImageData"),
        native_fn("putImageData", ctx_noop),
    );

    // Transforms
    ctx.set_property(String::from("save"), native_fn("save", ctx_noop));
    ctx.set_property(String::from("restore"), native_fn("restore", ctx_noop));
    ctx.set_property(String::from("translate"), native_fn("translate", ctx_noop));
    ctx.set_property(String::from("rotate"), native_fn("rotate", ctx_noop));
    ctx.set_property(String::from("scale"), native_fn("scale", ctx_noop));
    ctx.set_property(String::from("transform"), native_fn("transform", ctx_noop));
    ctx.set_property(
        String::from("setTransform"),
        native_fn("setTransform", ctx_noop),
    );
    ctx.set_property(
        String::from("getTransform"),
        native_fn("getTransform", ctx_get_transform),
    );
    ctx.set_property(
        String::from("resetTransform"),
        native_fn("resetTransform", ctx_noop),
    );

    // Gradients & Patterns
    ctx.set_property(
        String::from("createLinearGradient"),
        native_fn("createLinearGradient", ctx_create_gradient),
    );
    ctx.set_property(
        String::from("createRadialGradient"),
        native_fn("createRadialGradient", ctx_create_gradient),
    );
    ctx.set_property(
        String::from("createConicGradient"),
        native_fn("createConicGradient", ctx_create_gradient),
    );
    ctx.set_property(
        String::from("createPattern"),
        native_fn("createPattern", ctx_create_gradient),
    );

    // Line styles
    ctx.set_property(
        String::from("setLineDash"),
        native_fn("setLineDash", ctx_noop),
    );
    ctx.set_property(
        String::from("getLineDash"),
        native_fn("getLineDash", |_, _| JsValue::new_array(Vec::new())),
    );

    // Compositing — already set as properties above

    ctx
}

fn ctx_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn ctx_measure_text(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let text = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let width = text.len() as f64 * 7.0; // approximate 7px per char
    let metrics = JsValue::new_object();
    metrics.set_property(String::from("width"), JsValue::Number(width));
    metrics.set_property(String::from("actualBoundingBoxLeft"), JsValue::Number(0.0));
    metrics.set_property(
        String::from("actualBoundingBoxRight"),
        JsValue::Number(width),
    );
    metrics.set_property(
        String::from("actualBoundingBoxAscent"),
        JsValue::Number(10.0),
    );
    metrics.set_property(
        String::from("actualBoundingBoxDescent"),
        JsValue::Number(2.0),
    );
    metrics.set_property(String::from("fontBoundingBoxAscent"), JsValue::Number(10.0));
    metrics.set_property(String::from("fontBoundingBoxDescent"), JsValue::Number(2.0));
    metrics.set_property(String::from("emHeightAscent"), JsValue::Number(10.0));
    metrics.set_property(String::from("emHeightDescent"), JsValue::Number(2.0));
    metrics
}

fn ctx_create_image_data(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let w = args.first().map(|v| v.to_number() as usize).unwrap_or(1);
    let h = args.get(1).map(|v| v.to_number() as usize).unwrap_or(1);
    let len = w * h * 4;
    let data = JsValue::new_array((0..len).map(|_| JsValue::Number(0.0)).collect());
    let img = JsValue::new_object();
    img.set_property(String::from("width"), JsValue::Number(w as f64));
    img.set_property(String::from("height"), JsValue::Number(h as f64));
    img.set_property(String::from("data"), data);
    img
}

fn ctx_get_image_data(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let x = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    let y = args.get(1).map(|v| v.to_number() as usize).unwrap_or(0);
    let w = args.get(2).map(|v| v.to_number() as usize).unwrap_or(1);
    let h = args.get(3).map(|v| v.to_number() as usize).unwrap_or(1);
    ctx_create_image_data(_vm, &[JsValue::Number(w as f64), JsValue::Number(h as f64)])
}

fn ctx_get_transform(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let m = JsValue::new_object();
    m.set_property(String::from("a"), JsValue::Number(1.0));
    m.set_property(String::from("b"), JsValue::Number(0.0));
    m.set_property(String::from("c"), JsValue::Number(0.0));
    m.set_property(String::from("d"), JsValue::Number(1.0));
    m.set_property(String::from("e"), JsValue::Number(0.0));
    m.set_property(String::from("f"), JsValue::Number(0.0));
    m.set_property(String::from("is2D"), JsValue::Bool(true));
    m.set_property(String::from("isIdentity"), JsValue::Bool(true));
    m
}

fn ctx_create_gradient(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let grad = JsValue::new_object();
    grad.set_property(
        String::from("addColorStop"),
        native_fn("addColorStop", ctx_noop),
    );
    grad
}
