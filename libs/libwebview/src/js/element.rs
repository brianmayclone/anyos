//! Native Element host object — all DOM Element methods.
//!
//! Each method is a native Rust function that reads `vm.current_this`
//! to get the element's `__nodeId`, then accesses the DOM via the bridge.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::{JsObject, JsArray};
use libjs::vm::native_fn;

use super::{
    get_bridge, this_node_id, arg_string, make_array,
    read_attribute, read_text_content, read_tag_name,
    read_child_ids, read_node_type, read_inner_html,
    dom_property_hook, DomMutation,
};
use super::classlist;
use super::selector;

// ═══════════════════════════════════════════════════════════
// Sibling helpers
// ═══════════════════════════════════════════════════════════

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
                let prev_any = if pos > 0 { Some(siblings[pos - 1]) } else { None };
                let next_any = if pos + 1 < siblings.len() { Some(siblings[pos + 1]) } else { None };

                // prev/next for element nodes only (nodeType == 1)
                let prev_el = (0..pos).rev()
                    .find(|&i| matches!(
                        &dom.nodes[siblings[i]].node_type,
                        crate::dom::NodeType::Element { .. }
                    ))
                    .map(|i| siblings[i]);
                let next_el = (pos + 1..siblings.len())
                    .find(|&i| matches!(
                        &dom.nodes[siblings[i]].node_type,
                        crate::dom::NodeType::Element { .. }
                    ))
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

    // Compute the number of direct element children so scripts can query
    // `el.childElementCount` without building child JsObjects.
    let child_count = read_child_ids(vm, node_id).len();

    // Empty child arrays — populated lazily via querySelector/querySelectorAll.
    let child_arr = JsValue::Array(Rc::new(RefCell::new(JsArray::new())));
    let first_child = JsValue::Null;
    let last_child = JsValue::Null;

    // Build the element object.
    let mut obj = JsObject::new();

    // Identity.
    obj.set(String::from("__nodeId"), JsValue::Number(node_id as f64));

    // Properties.
    obj.set(String::from("nodeType"), JsValue::Number(node_type));
    obj.set(String::from("tagName"), JsValue::String(tag_name));
    obj.set(String::from("id"), JsValue::String(id_val));
    obj.set(String::from("className"), JsValue::String(class_name.clone()));
    obj.set(String::from("textContent"), JsValue::String(text.clone()));
    obj.set(String::from("innerText"), JsValue::String(text));
    obj.set(String::from("innerHTML"), JsValue::String(inner_html));
    obj.set(String::from("value"), JsValue::String(value_val));
    obj.set(String::from("src"), JsValue::String(src_val));
    obj.set(String::from("href"), JsValue::String(href_val));
    obj.set(String::from("type"), JsValue::String(type_val));
    obj.set(String::from("name"), JsValue::String(name_val));
    obj.set(String::from("checked"), JsValue::Bool(checked));
    obj.set(String::from("disabled"), JsValue::Bool(disabled));

    // Sibling references — computed one level deep for real DOM nodes.
    // When `include_siblings` is false (we're already building a sibling object)
    // they stay Null to prevent O(N²) allocation chains on large flat lists.
    let (prev_sib, next_sib, prev_any, next_any) = if include_siblings && node_id >= 0 {
        let (pe, ne, pa, na) = compute_sibling_ids(vm, node_id as usize);
        (
            pe.map(|id| make_element_impl(vm, id as i64, false)).unwrap_or(JsValue::Null),
            ne.map(|id| make_element_impl(vm, id as i64, false)).unwrap_or(JsValue::Null),
            pa.map(|id| make_element_impl(vm, id as i64, false)).unwrap_or(JsValue::Null),
            na.map(|id| make_element_impl(vm, id as i64, false)).unwrap_or(JsValue::Null),
        )
    } else {
        (JsValue::Null, JsValue::Null, JsValue::Null, JsValue::Null)
    };

    // Tree references.  children/childNodes are empty — scripts should use
    // querySelector/querySelectorAll to traverse the DOM on demand.
    obj.set(String::from("children"), child_arr.clone());
    obj.set(String::from("childNodes"), child_arr);
    obj.set(String::from("childElementCount"), JsValue::Number(child_count as f64));
    obj.set(String::from("firstChild"), first_child);
    obj.set(String::from("lastChild"), last_child);
    obj.set(String::from("parentNode"), JsValue::Null);
    obj.set(String::from("parentElement"), JsValue::Null);
    obj.set(String::from("previousSibling"), prev_any);
    obj.set(String::from("nextSibling"), next_any);
    obj.set(String::from("previousElementSibling"), prev_sib);
    obj.set(String::from("nextElementSibling"), next_sib);

    // Style — CSSStyleDeclaration (W3C CSSOM §6.7.2).
    // Properties set on this object trigger SetStyleProperty mutations via set_hook.
    let style_obj = make_css_style_declaration(node_id);
    obj.set(String::from("style"), style_obj);
    obj.set(String::from("dataset"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));

    // classList.
    let cl = classlist::make_class_list(node_id, &class_name);
    obj.set(String::from("classList"), cl);

    // ── Native methods ──
    obj.set(String::from("getAttribute"), native_fn("getAttribute", el_get_attribute));
    obj.set(String::from("setAttribute"), native_fn("setAttribute", el_set_attribute));
    obj.set(String::from("removeAttribute"), native_fn("removeAttribute", el_remove_attribute));
    obj.set(String::from("hasAttribute"), native_fn("hasAttribute", el_has_attribute));
    obj.set(String::from("addEventListener"), native_fn("addEventListener", el_add_event_listener));
    obj.set(String::from("removeEventListener"), native_fn("removeEventListener", super::native_remove_event_listener));
    obj.set(String::from("dispatchEvent"), native_fn("dispatchEvent", el_dispatch_event));

    // Query.
    obj.set(String::from("querySelector"), native_fn("querySelector", el_query_selector));
    obj.set(String::from("querySelectorAll"), native_fn("querySelectorAll", el_query_selector_all));
    obj.set(String::from("getElementsByTagName"), native_fn("getElementsByTagName", el_get_elements_by_tag_name));
    obj.set(String::from("getElementsByClassName"), native_fn("getElementsByClassName", el_get_elements_by_class_name));

    // Tree manipulation (Node interface).
    obj.set(String::from("appendChild"), native_fn("appendChild", el_append_child));
    obj.set(String::from("removeChild"), native_fn("removeChild", el_remove_child));
    obj.set(String::from("insertBefore"), native_fn("insertBefore", el_insert_before));
    obj.set(String::from("replaceChild"), native_fn("replaceChild", el_replace_child));
    obj.set(String::from("cloneNode"), native_fn("cloneNode", el_clone_node));
    obj.set(String::from("contains"), native_fn("contains", el_contains));
    obj.set(String::from("remove"), native_fn("remove", el_remove));

    // ParentNode interface (W3C DOM §4.2.6).
    obj.set(String::from("prepend"), native_fn("prepend", el_prepend));
    obj.set(String::from("append"), native_fn("append", el_append));
    obj.set(String::from("replaceChildren"), native_fn("replaceChildren", el_replace_children));

    // ChildNode interface (W3C DOM §4.2.7).
    obj.set(String::from("before"), native_fn("before", el_before));
    obj.set(String::from("after"), native_fn("after", el_after));
    obj.set(String::from("replaceWith"), native_fn("replaceWith", el_replace_with));

    // insertAdjacentHTML / insertAdjacentElement (W3C DOM Parsing §4).
    obj.set(String::from("insertAdjacentHTML"), native_fn("insertAdjacentHTML", el_insert_adjacent_html));
    obj.set(String::from("insertAdjacentElement"), native_fn("insertAdjacentElement", el_insert_adjacent_element));
    obj.set(String::from("insertAdjacentText"), native_fn("insertAdjacentText", el_insert_adjacent_text));

    // Content setters (since we can't intercept property writes).
    obj.set(String::from("setTextContent"), native_fn("setTextContent", el_set_text_content));
    obj.set(String::from("setInnerHTML"), native_fn("setInnerHTML", el_set_inner_html));
    obj.set(String::from("setStyle"), native_fn("setStyle", el_set_style));

    // Node properties (W3C DOM §4.4).
    obj.set(String::from("isConnected"), JsValue::Bool(node_id >= 0));
    obj.set(String::from("getRootNode"), native_fn("getRootNode", el_get_root_node));
    obj.set(String::from("ownerDocument"), JsValue::Null); // set by document setup

    // outerHTML (W3C DOM Parsing §3).
    obj.set(String::from("outerHTML"), JsValue::String(String::new())); // placeholder, set_hook handles writes

    // Geometry (W3C CSSOM View §6).
    obj.set(String::from("offsetWidth"),  JsValue::Number(0.0));
    obj.set(String::from("offsetHeight"), JsValue::Number(0.0));
    obj.set(String::from("offsetTop"),    JsValue::Number(0.0));
    obj.set(String::from("offsetLeft"),   JsValue::Number(0.0));
    obj.set(String::from("offsetParent"), JsValue::Null);
    obj.set(String::from("clientWidth"),  JsValue::Number(0.0));
    obj.set(String::from("clientHeight"), JsValue::Number(0.0));
    obj.set(String::from("clientTop"),    JsValue::Number(0.0));
    obj.set(String::from("clientLeft"),   JsValue::Number(0.0));
    obj.set(String::from("scrollWidth"),  JsValue::Number(0.0));
    obj.set(String::from("scrollHeight"), JsValue::Number(0.0));
    obj.set(String::from("scrollTop"),    JsValue::Number(0.0));
    obj.set(String::from("scrollLeft"),   JsValue::Number(0.0));

    // Misc.
    obj.set(String::from("matches"), native_fn("matches", el_matches));
    obj.set(String::from("closest"), native_fn("closest", el_closest));
    obj.set(String::from("focus"), native_fn("focus", el_noop));
    obj.set(String::from("blur"), native_fn("blur", el_noop));
    obj.set(String::from("click"), native_fn("click", el_noop));
    obj.set(String::from("scrollIntoView"), native_fn("scrollIntoView", el_noop));
    obj.set(String::from("getBoundingClientRect"), native_fn("getBoundingClientRect", el_get_bounding_rect));
    obj.set(String::from("getClientRects"), native_fn("getClientRects", el_get_client_rects));
    obj.set(String::from("toString"), native_fn("toString", el_to_string));

    // Set property-write interception hook so that assignments like
    // el.textContent = "x" record DOM mutations.
    obj.set_hook = Some(dom_property_hook);
    obj.set_hook_data = node_id as usize as *mut u8;

    JsValue::Object(Rc::new(RefCell::new(obj)))
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
                node_id: nid as usize, name: name.clone(), value: value.clone(),
            });
        }
    }

    // Update cached properties on `this`.
    if let JsValue::Object(obj) = &vm.current_this {
        let mut o = obj.borrow_mut();
        if name == "id" { o.set(String::from("id"), JsValue::String(value.clone())); }
        if name == "class" { o.set(String::from("className"), JsValue::String(value.clone())); }
        if name == "value" { o.set(String::from("value"), JsValue::String(value)); }
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
            bridge.mutations.push(DomMutation::RemoveAttribute {
                node_id: nid as usize, name,
            });
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
        Some(JsValue::Bool(b))   => *b,
        Some(JsValue::Object(_)) => args[2].get_property("capture").to_boolean(),
        _                        => false,
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
    if sel.is_empty() { return JsValue::Null; }
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
    if sel.is_empty() { return make_array(Vec::new()); }
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
    if class_name.is_empty() { return make_array(Vec::new()); }
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
        bridge.mutations.push(DomMutation::AppendChild { parent_id, child_id });
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
            arr.borrow_mut().elements.push(child.clone());
        }
        // Update firstChild/lastChild.
        let (first, last) = get_first_last(&children_arr);
        let mut o = obj.borrow_mut();
        o.set(String::from("firstChild"), first);
        o.set(String::from("lastChild"), last);
        o.set(String::from("childNodes"), children_arr);
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
        bridge.mutations.push(DomMutation::RemoveChild { parent_id, child_id });
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
            arr.borrow_mut().elements.retain(|el| extract_node_id(el) != child_id);
        }
        let (first, last) = get_first_last(&children_arr);
        let mut o = obj.borrow_mut();
        o.set(String::from("firstChild"), first);
        o.set(String::from("lastChild"), last);
        o.set(String::from("childNodes"), children_arr);
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
            parent_id, new_child_id: new_id, ref_child_id: ref_id,
        });
    }

    // Insert in JS-side children array.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            let mut a = arr.borrow_mut();
            let idx = a.elements.iter().position(|el| extract_node_id(el) == ref_id);
            if let Some(i) = idx {
                a.elements.insert(i, new_node.clone());
            } else {
                a.elements.push(new_node.clone());
            }
        }
        let (first, last) = get_first_last(&children_arr);
        let mut o = obj.borrow_mut();
        o.set(String::from("firstChild"), first);
        o.set(String::from("lastChild"), last);
        o.set(String::from("childNodes"), children_arr);
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
            parent_id, new_child_id: new_id, old_child_id: old_id,
        });
    }

    // Replace in JS-side children.
    if let JsValue::Object(obj) = &vm.current_this {
        let children_arr = obj.borrow().get("children");
        if let JsValue::Array(arr) = &children_arr {
            let mut a = arr.borrow_mut();
            if let Some(idx) = a.elements.iter().position(|el| extract_node_id(el) == old_id) {
                a.elements[idx] = new_node.clone();
            }
        }
        let (first, last) = get_first_last(&children_arr);
        let mut o = obj.borrow_mut();
        o.set(String::from("firstChild"), first);
        o.set(String::from("lastChild"), last);
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
    if other_id == -9999 || nid < 0 || other_id < 0 { return JsValue::Bool(false); }
    // A node contains itself (per W3C DOM §4.4).
    if nid == other_id { return JsValue::Bool(true); }
    // Walk from other_id up to the root, checking if we reach nid.
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let mut cur = Some(other_id as usize);
        while let Some(id) = cur {
            if id == nid as usize { return JsValue::Bool(true); }
            cur = dom.nodes.get(id).and_then(|n| n.parent);
        }
    }
    JsValue::Bool(false)
}

fn el_remove(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::RemoveNode { node_id: nid });
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
                node_id: nid as usize, text: text.clone(),
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
        bridge.mutations.push(DomMutation::SetInnerHTML { node_id: nid, html: html.clone() });
    }

    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from("innerHTML"), JsValue::String(html));
    }
    JsValue::Undefined
}

fn el_set_style(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    let prop = arg_string(args, 0);
    let val = arg_string(args, 1);

    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetStyleProperty {
            node_id: nid, property: prop.clone(), value: val.clone(),
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

fn el_get_bounding_rect(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let rect = JsValue::new_object();
    for key in &["top", "left", "bottom", "right", "width", "height", "x", "y"] {
        rect.set_property(String::from(*key), JsValue::Number(0.0));
    }
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
    for arg in args {
        let child_id = extract_node_id(arg);
        if let Some(bridge) = get_bridge(vm) {
            // InsertBefore the first child — we use the first child as ref.
            // If no first child, this degrades to AppendChild.
            let first_child_id = if parent_id >= 0 {
                bridge.dom().nodes.get(parent_id as usize)
                    .and_then(|n| n.children.first().copied())
                    .map(|id| id as i64)
            } else {
                bridge.get_virtual(parent_id).and_then(|vn| vn.child_ids.first().copied())
            };
            if let Some(ref_id) = first_child_id {
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id, new_child_id: child_id, ref_child_id: ref_id,
                });
            } else {
                bridge.mutations.push(DomMutation::AppendChild { parent_id, child_id });
            }
        }
    }
    JsValue::Undefined
}

fn el_append(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    for arg in args {
        let child_id = extract_node_id(arg);
        if let Some(bridge) = get_bridge(vm) {
            bridge.mutations.push(DomMutation::AppendChild { parent_id, child_id });
            if parent_id < 0 {
                if let Some(vn) = bridge.get_virtual_mut(parent_id) {
                    vn.child_ids.push(child_id);
                }
            }
        }
    }
    JsValue::Undefined
}

fn el_replace_children(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parent_id = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        // Remove all existing children.
        let child_ids: Vec<i64> = if parent_id >= 0 {
            bridge.dom().nodes.get(parent_id as usize)
                .map(|n| n.children.iter().map(|&id| id as i64).collect())
                .unwrap_or_default()
        } else {
            bridge.get_virtual(parent_id)
                .map(|vn| vn.child_ids.clone())
                .unwrap_or_default()
        };
        for cid in &child_ids {
            bridge.mutations.push(DomMutation::RemoveChild { parent_id, child_id: *cid });
        }
        // Append new children.
        for arg in args {
            let child_id = extract_node_id(arg);
            bridge.mutations.push(DomMutation::AppendChild { parent_id, child_id });
        }
    }
    JsValue::Undefined
}

// ── ChildNode: before / after / replaceWith (W3C DOM §4.2.7) ──

fn el_before(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge.dom().nodes.get(nid as usize).and_then(|n| n.parent).map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };
        if let Some(pid) = parent_id {
            for arg in args {
                let child_id = extract_node_id(arg);
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id: pid, new_child_id: child_id, ref_child_id: nid,
                });
            }
        }
    }
    JsValue::Undefined
}

fn el_after(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge.dom().nodes.get(nid as usize).and_then(|n| n.parent).map(|p| p as i64)
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
                } else { None }
            } else { None };

            for arg in args {
                let child_id = extract_node_id(arg);
                if let Some(ref_id) = next_sib_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid, new_child_id: child_id, ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild { parent_id: pid, child_id });
                }
            }
        }
    }
    JsValue::Undefined
}

fn el_replace_with(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge.dom().nodes.get(nid as usize).and_then(|n| n.parent).map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };
        if let Some(pid) = parent_id {
            // Insert each new node before this, then remove this.
            for arg in args {
                let child_id = extract_node_id(arg);
                bridge.mutations.push(DomMutation::InsertBefore {
                    parent_id: pid, new_child_id: child_id, ref_child_id: nid,
                });
            }
            bridge.mutations.push(DomMutation::RemoveChild { parent_id: pid, child_id: nid });
        }
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
        bridge.mutations.push(DomMutation::CreateElement { virtual_id: frag_id, tag: String::from("div") });
        bridge.mutations.push(DomMutation::SetInnerHTML { node_id: frag_id, html });

        let parent_id = if nid >= 0 {
            bridge.dom().nodes.get(nid as usize).and_then(|n| n.parent).map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };

        match position.as_str() {
            "beforebegin" => {
                // Before the element itself.
                if let Some(pid) = parent_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid, new_child_id: frag_id, ref_child_id: nid,
                    });
                }
            }
            "afterbegin" => {
                // Inside the element, before its first child.
                let first_child_id = if nid >= 0 {
                    bridge.dom().nodes.get(nid as usize)
                        .and_then(|n| n.children.first().copied())
                        .map(|id| id as i64)
                } else { None };
                if let Some(ref_id) = first_child_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: nid, new_child_id: frag_id, ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild { parent_id: nid, child_id: frag_id });
                }
            }
            "beforeend" => {
                // Inside the element, after its last child.
                bridge.mutations.push(DomMutation::AppendChild { parent_id: nid, child_id: frag_id });
            }
            "afterend" => {
                // After the element itself.
                if let Some(pid) = parent_id {
                    let next_sib_id = if nid >= 0 {
                        let dom = bridge.dom();
                        if let Some(parent) = dom.nodes.get(pid as usize) {
                            let pos = parent.children.iter().position(|&c| c == nid as usize);
                            pos.and_then(|p| parent.children.get(p + 1).map(|&c| c as i64))
                        } else { None }
                    } else { None };
                    if let Some(ref_id) = next_sib_id {
                        bridge.mutations.push(DomMutation::InsertBefore {
                            parent_id: pid, new_child_id: frag_id, ref_child_id: ref_id,
                        });
                    } else {
                        bridge.mutations.push(DomMutation::AppendChild { parent_id: pid, child_id: frag_id });
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
    if child_id == -9999 { return JsValue::Null; }

    if let Some(bridge) = get_bridge(vm) {
        let parent_id = if nid >= 0 {
            bridge.dom().nodes.get(nid as usize).and_then(|n| n.parent).map(|p| p as i64)
        } else {
            bridge.get_virtual(nid).and_then(|vn| vn.parent_id)
        };

        match position.as_str() {
            "beforebegin" => {
                if let Some(pid) = parent_id {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: pid, new_child_id: child_id, ref_child_id: nid,
                    });
                }
            }
            "afterbegin" => {
                let first = if nid >= 0 {
                    bridge.dom().nodes.get(nid as usize)
                        .and_then(|n| n.children.first().copied())
                        .map(|id| id as i64)
                } else { None };
                if let Some(ref_id) = first {
                    bridge.mutations.push(DomMutation::InsertBefore {
                        parent_id: nid, new_child_id: child_id, ref_child_id: ref_id,
                    });
                } else {
                    bridge.mutations.push(DomMutation::AppendChild { parent_id: nid, child_id });
                }
            }
            "beforeend" => {
                bridge.mutations.push(DomMutation::AppendChild { parent_id: nid, child_id });
            }
            "afterend" => {
                if let Some(pid) = parent_id {
                    let next = if nid >= 0 {
                        let dom = bridge.dom();
                        dom.nodes.get(pid as usize).and_then(|p| {
                            let pos = p.children.iter().position(|&c| c == nid as usize);
                            pos.and_then(|i| p.children.get(i + 1).map(|&c| c as i64))
                        })
                    } else { None };
                    if let Some(ref_id) = next {
                        bridge.mutations.push(DomMutation::InsertBefore {
                            parent_id: pid, new_child_id: child_id, ref_child_id: ref_id,
                        });
                    } else {
                        bridge.mutations.push(DomMutation::AppendChild { parent_id: pid, child_id });
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
        bridge.mutations.push(DomMutation::CreateElement { virtual_id: text_id, tag: String::from("#text") });
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
    if nid < 0 { return JsValue::Bool(false); }
    let sel = arg_string(args, 0);
    if sel.is_empty() { return JsValue::Bool(false); }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let ids = selector::find_all(dom, &sel);
        return JsValue::Bool(ids.contains(&(nid as usize)));
    }
    JsValue::Bool(false)
}

fn el_closest(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let nid = this_node_id(vm);
    if nid < 0 { return JsValue::Null; }
    let sel = arg_string(args, 0);
    if sel.is_empty() { return JsValue::Null; }
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
    if nid < 0 { return vm.current_this.clone(); }
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

fn el_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }

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
    sobj.set(String::from("setProperty"), native_fn("setProperty", style_set_property));
    // getPropertyValue(propertyName)
    sobj.set(String::from("getPropertyValue"), native_fn("getPropertyValue", style_get_property_value));
    // removeProperty(propertyName) — returns old value
    sobj.set(String::from("removeProperty"), native_fn("removeProperty", style_remove_property));
    // getPropertyPriority(propertyName)
    sobj.set(String::from("getPropertyPriority"), native_fn("getPropertyPriority", |_, _| JsValue::String(String::new())));
    // cssText
    sobj.set(String::from("cssText"), JsValue::String(String::new()));
    // length
    sobj.set(String::from("length"), JsValue::Number(0.0));
    // item(index)
    sobj.set(String::from("item"), native_fn("item", |_, _| JsValue::String(String::new())));

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
            node_id: nid, property: prop, value: val,
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
            node_id: nid, property: prop, value: String::new(),
        });
    }
    old
}

/// Hook for direct property assignment on style objects (`style.color = "red"`).
/// Converts camelCase to kebab-case and emits a SetStyleProperty mutation.
fn style_property_hook(data: *mut u8, key: &str, value: &JsValue) {
    // Skip internal properties.
    match key {
        "__nodeId" | "setProperty" | "getPropertyValue" | "removeProperty"
        | "getPropertyPriority" | "cssText" | "length" | "item" => return,
        _ => {}
    }
    let mutations = unsafe {
        if super::MUTATION_TARGET.is_null() { return; }
        &mut *super::MUTATION_TARGET
    };
    let node_id = data as usize as i64;
    let css_prop = css_prop_from_camel(key);
    mutations.push(DomMutation::SetStyleProperty {
        node_id, property: css_prop, value: value.to_js_string(),
    });
}

/// Convert camelCase CSS property name to kebab-case.
/// e.g. `backgroundColor` → `background-color`, `cssFloat` → `float`
fn css_prop_from_camel(name: &str) -> String {
    if name == "cssFloat" { return String::from("float"); }
    if name.contains('-') { return String::from(name); } // already kebab
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
    if name == "float" { return String::from("cssFloat"); }
    if !name.contains('-') { return String::from(name); }
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
pub fn extract_node_id_pub(val: &JsValue) -> i64 { extract_node_id(val) }

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
            return (elems[0].clone(), elems[elems.len() - 1].clone());
        }
    }
    (JsValue::Null, JsValue::Null)
}
