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
    DomMutation,
};
use super::classlist;
use super::selector;

// ═══════════════════════════════════════════════════════════
// Element factory
// ═══════════════════════════════════════════════════════════

/// Create a fully-populated native Element JsObject.
/// This is the equivalent of what real browsers do when exposing a DOM
/// node to JavaScript — a host object with native method bindings.
pub fn make_element(vm: &mut Vm, node_id: i64) -> JsValue {
    // Read properties from DOM or virtual node store.
    let tag_name = read_tag_name(vm, node_id);
    let text = read_text_content(vm, node_id);
    let node_type = read_node_type(vm, node_id);
    let inner_html = read_inner_html(vm, node_id);
    let class_name = match read_attribute(vm, node_id, "class") {
        JsValue::String(s) => s,
        _ => String::new(),
    };
    let _is_virtual = node_id < 0;

    // Build children array (recursive).
    let child_ids = read_child_ids(vm, node_id);
    let mut children = Vec::new();
    for &cid in &child_ids {
        children.push(make_element(vm, cid));
    }
    let child_arr = JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(children.clone()))));

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

    let first_child = if children.is_empty() { JsValue::Null } else { children[0].clone() };
    let last_child = if children.is_empty() { JsValue::Null } else { children.last().unwrap().clone() };

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

    // Tree references.
    obj.set(String::from("children"), child_arr.clone());
    obj.set(String::from("childNodes"), child_arr);
    obj.set(String::from("firstChild"), first_child);
    obj.set(String::from("lastChild"), last_child);
    obj.set(String::from("parentNode"), JsValue::Null);
    obj.set(String::from("parentElement"), JsValue::Null);
    obj.set(String::from("nextSibling"), JsValue::Null);
    obj.set(String::from("previousSibling"), JsValue::Null);

    // Style and dataset.
    obj.set(String::from("style"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));
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
    obj.set(String::from("removeEventListener"), native_fn("removeEventListener", el_noop));
    obj.set(String::from("dispatchEvent"), native_fn("dispatchEvent", el_dispatch_event));

    // Query.
    obj.set(String::from("querySelector"), native_fn("querySelector", el_query_selector));
    obj.set(String::from("querySelectorAll"), native_fn("querySelectorAll", el_query_selector_all));
    obj.set(String::from("getElementsByTagName"), native_fn("getElementsByTagName", el_get_elements_by_tag_name));
    obj.set(String::from("getElementsByClassName"), native_fn("getElementsByClassName", el_get_elements_by_class_name));

    // Tree manipulation.
    obj.set(String::from("appendChild"), native_fn("appendChild", el_append_child));
    obj.set(String::from("removeChild"), native_fn("removeChild", el_remove_child));
    obj.set(String::from("insertBefore"), native_fn("insertBefore", el_insert_before));
    obj.set(String::from("replaceChild"), native_fn("replaceChild", el_replace_child));
    obj.set(String::from("cloneNode"), native_fn("cloneNode", el_clone_node));
    obj.set(String::from("contains"), native_fn("contains", el_contains));
    obj.set(String::from("remove"), native_fn("remove", el_remove));

    // Content setters (since we can't intercept property writes).
    obj.set(String::from("setTextContent"), native_fn("setTextContent", el_set_text_content));
    obj.set(String::from("setInnerHTML"), native_fn("setInnerHTML", el_set_inner_html));
    obj.set(String::from("setStyle"), native_fn("setStyle", el_set_style));

    // Misc.
    obj.set(String::from("matches"), native_fn("matches", el_noop_false));
    obj.set(String::from("closest"), native_fn("closest", el_noop_null));
    obj.set(String::from("focus"), native_fn("focus", el_noop));
    obj.set(String::from("blur"), native_fn("blur", el_noop));
    obj.set(String::from("click"), native_fn("click", el_noop));
    obj.set(String::from("getBoundingClientRect"), native_fn("getBoundingClientRect", el_get_bounding_rect));
    obj.set(String::from("getClientRects"), native_fn("getClientRects", el_get_client_rects));
    obj.set(String::from("toString"), native_fn("toString", el_to_string));

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
    if let Some(bridge) = get_bridge(vm) {
        let index = bridge.event_listeners.len();
        bridge.event_listeners.push(super::EventListener {
            node_id: if nid >= 0 { nid as usize } else { usize::MAX },
            event,
            callback_index: index,
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

fn el_contains(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Simplified: check JS children array recursively.
    let other = args.first().cloned().unwrap_or(JsValue::Null);
    let other_id = extract_node_id(&other);
    if other_id == -9999 { return JsValue::Bool(false); }
    // A full implementation would walk the subtree; for now return false.
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

fn el_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }
fn el_noop_false(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Bool(false) }
fn el_noop_null(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Null }

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

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
