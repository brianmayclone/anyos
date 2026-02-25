//! Native document host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::{JsObject, JsArray};
use libjs::vm::native_fn;

use crate::dom::{Dom, NodeType, Tag};

use super::{get_bridge, arg_string, make_array, dom_property_hook, DomMutation, VirtualNode};
use super::element;
use super::selector;

// ═══════════════════════════════════════════════════════════
// URL parsing helper
// ═══════════════════════════════════════════════════════════

/// Parse a URL string into its Location object fields.
/// Returns `(protocol, hostname, port, pathname, search, hash, origin)`.
fn parse_location_fields(url: &str) -> (String, String, String, String, String, String, String) {
    let mut s = url;

    // protocol (scheme + colon, e.g. "https:")
    let (protocol, after_scheme) = if let Some(pos) = s.find("://") {
        let proto = String::from(&s[..pos + 1]); // "http:" or "https:"
        (proto, &s[pos + 3..])
    } else {
        (String::from("http:"), s)
    };
    s = after_scheme;

    // hostname (and optional port after ':')
    let (host_port, path_etc) = if let Some(pos) = s.find('/') {
        (&s[..pos], &s[pos..])
    } else {
        (s, "/")
    };

    let (hostname, port) = if let Some(pos) = host_port.rfind(':') {
        // Only treat as port if it looks numeric
        let maybe_port = &host_port[pos + 1..];
        if maybe_port.bytes().all(|b| b.is_ascii_digit()) {
            (String::from(&host_port[..pos]), String::from(maybe_port))
        } else {
            (String::from(host_port), String::new())
        }
    } else {
        (String::from(host_port), String::new())
    };

    // hash
    let (path_search, hash) = if let Some(pos) = path_etc.find('#') {
        (&path_etc[..pos], String::from(&path_etc[pos..]))
    } else {
        (path_etc, String::new())
    };

    // search (query string)
    let (pathname, search) = if let Some(pos) = path_search.find('?') {
        (String::from(&path_search[..pos]), String::from(&path_search[pos..]))
    } else {
        (String::from(path_search), String::new())
    };

    // origin = protocol + "//" + hostname (+ port if non-standard)
    let mut origin = protocol.clone();
    origin.push_str("//");
    origin.push_str(&hostname);
    if !port.is_empty() {
        let is_default = (protocol == "http:" && port == "80")
            || (protocol == "https:" && port == "443");
        if !is_default {
            origin.push(':');
            origin.push_str(&port);
        }
    }

    (protocol, hostname, port, pathname, search, hash, origin)
}

// ═══════════════════════════════════════════════════════════
// Document cookie write hook
// ═══════════════════════════════════════════════════════════

/// Property-write hook installed on the document JsObject.
/// Intercepts `document.cookie = "name=value"` writes and records them as
/// `DomMutation::SetCookie` so the host application (e.g. surf) can update
/// its cookie jar.
fn doc_property_hook(_data: *mut u8, key: &str, value: &libjs::JsValue) {
    if key != "cookie" { return; }
    let mutations = unsafe {
        if super::MUTATION_TARGET.is_null() { return; }
        &mut *super::MUTATION_TARGET
    };
    mutations.push(DomMutation::SetCookie { value: value.to_js_string() });
}

/// Create the native `document` host object.
///
/// * `url`     — the current page URL (used to populate `document.location`).
/// * `cookies` — the `Cookie` header value for this domain, used to populate
///               `document.cookie`.  Writes to `document.cookie` are recorded
///               as `DomMutation::SetCookie` mutations.
pub fn make_document(vm: &mut Vm, dom: &Dom, url: &str, cookies: &str) -> JsValue {
    let body_id = dom.find_body().unwrap_or(0);
    let head_id: usize = dom.nodes.iter().enumerate()
        .find(|(_, n)| matches!(&n.node_type, NodeType::Element { tag: Tag::Head, .. }))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let title = dom.find_title().unwrap_or_else(|| String::from(""));

    let doc_el = element::make_element(vm, 0);
    let body_el = element::make_element(vm, body_id as i64);
    let head_el = element::make_element(vm, head_id as i64);

    // Parse URL into location fields.
    let href = String::from(url);
    let (protocol, hostname, port, pathname, search, hash, origin) =
        parse_location_fields(url);

    let mut obj = JsObject::new();

    // Properties.
    obj.set(String::from("title"), JsValue::String(title));
    obj.set(String::from("documentElement"), doc_el);
    obj.set(String::from("body"), body_el);
    obj.set(String::from("head"), head_el);
    // cookie — readable; writes are intercepted by doc_property_hook
    obj.set(String::from("cookie"), JsValue::String(String::from(cookies)));
    obj.set(String::from("readyState"), JsValue::String(String::from("complete")));
    obj.set(String::from("referrer"), JsValue::String(String::new()));
    obj.set(String::from("domain"), JsValue::String(hostname.clone()));
    obj.set(String::from("URL"), JsValue::String(href.clone()));
    obj.set(String::from("characterSet"), JsValue::String(String::from("UTF-8")));
    obj.set(String::from("contentType"), JsValue::String(String::from("text/html")));
    obj.set(String::from("compatMode"), JsValue::String(String::from("CSS1Compat")));
    obj.set(String::from("defaultView"), JsValue::Null);

    // location sub-object — all fields populated from the current URL.
    let loc = JsValue::new_object();
    loc.set_property(String::from("href"), JsValue::String(href));
    loc.set_property(String::from("hostname"), JsValue::String(hostname));
    loc.set_property(String::from("port"), JsValue::String(port));
    loc.set_property(String::from("pathname"), JsValue::String(pathname));
    loc.set_property(String::from("protocol"), JsValue::String(protocol));
    loc.set_property(String::from("search"), JsValue::String(search));
    loc.set_property(String::from("hash"), JsValue::String(hash));
    loc.set_property(String::from("origin"), JsValue::String(origin));
    loc.set_property(String::from("assign"), native_fn("assign", |_,_| JsValue::Undefined));
    loc.set_property(String::from("replace"), native_fn("replace", |_,_| JsValue::Undefined));
    loc.set_property(String::from("reload"), native_fn("reload", |_,_| JsValue::Undefined));
    obj.set(String::from("location"), loc);

    // implementation sub-object.
    let impl_obj = JsValue::new_object();
    impl_obj.set_property(String::from("hasFeature"), native_fn("hasFeature", |_,_| JsValue::Bool(true)));
    obj.set(String::from("implementation"), impl_obj);

    // ── Native methods ──
    obj.set(String::from("getElementById"), native_fn("getElementById", doc_get_element_by_id));
    obj.set(String::from("getElementsByTagName"), native_fn("getElementsByTagName", doc_get_elements_by_tag_name));
    obj.set(String::from("getElementsByClassName"), native_fn("getElementsByClassName", doc_get_elements_by_class_name));
    obj.set(String::from("querySelector"), native_fn("querySelector", doc_query_selector));
    obj.set(String::from("querySelectorAll"), native_fn("querySelectorAll", doc_query_selector_all));
    obj.set(String::from("createElement"), native_fn("createElement", doc_create_element));
    obj.set(String::from("createTextNode"), native_fn("createTextNode", doc_create_text_node));
    obj.set(String::from("createDocumentFragment"), native_fn("createDocumentFragment", doc_create_document_fragment));
    obj.set(String::from("createComment"), native_fn("createComment", doc_create_comment));
    obj.set(String::from("createEvent"), native_fn("createEvent", doc_create_event));
    obj.set(String::from("addEventListener"), native_fn("addEventListener", doc_add_event_listener));
    obj.set(String::from("removeEventListener"), native_fn("removeEventListener", doc_noop));

    // Install property-write hook to intercept `document.cookie = "..."`.
    obj.set_hook = Some(doc_property_hook);
    obj.set_hook_data = core::ptr::null_mut();

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// Document methods
// ═══════════════════════════════════════════════════════════

fn doc_get_element_by_id(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = arg_string(args, 0);
    if id.is_empty() { return JsValue::Null; }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { attrs, .. } = &node.node_type {
                if attrs.iter().any(|a| a.name == "id" && a.value == id) {
                    return element::make_element(vm, i as i64);
                }
            }
        }
    }
    JsValue::Null
}

fn doc_get_elements_by_tag_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag_name = arg_string(args, 0).to_ascii_uppercase();
    let mut ids = Vec::new();
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let target = Tag::from_str(&tag_name);
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag, .. } = &node.node_type {
                if *tag == target || tag_name == "*" {
                    ids.push(i as i64);
                }
            }
        }
    }
    let results: Vec<JsValue> = ids.iter().map(|&id| element::make_element(vm, id)).collect();
    make_array(results)
}

fn doc_get_elements_by_class_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class_name = arg_string(args, 0);
    if class_name.is_empty() { return make_array(Vec::new()); }
    let mut ids = Vec::new();
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { attrs, .. } = &node.node_type {
                if attrs.iter().any(|a| a.name == "class" && a.value.split_whitespace().any(|c| c == class_name)) {
                    ids.push(i as i64);
                }
            }
        }
    }
    let results: Vec<JsValue> = ids.iter().map(|&id| element::make_element(vm, id)).collect();
    make_array(results)
}

fn doc_query_selector(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() { return JsValue::Null; }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        if let Some(id) = selector::find_first(dom, &sel) {
            return element::make_element(vm, id as i64);
        }
    }
    JsValue::Null
}

fn doc_query_selector_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() { return make_array(Vec::new()); }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let ids = selector::find_all(dom, &sel);
        let elems: Vec<JsValue> = ids.iter().map(|&id| element::make_element(vm, id as i64)).collect();
        return make_array(elems);
    }
    make_array(Vec::new())
}

fn doc_create_element(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag = arg_string(args, 0).to_ascii_uppercase();
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        let id = bridge.alloc_virtual_id();
        bridge.mutations.push(DomMutation::CreateElement { virtual_id: id, tag: tag.clone() });
        bridge.virtual_nodes.push(VirtualNode {
            id,
            tag: tag.clone(),
            attrs: Vec::new(),
            text_content: String::new(),
            child_ids: Vec::new(),
            parent_id: None,
        });
        id
    } else {
        -1
    };
    element::make_element(vm, virtual_id)
}

fn doc_create_text_node(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let text = arg_string(args, 0);
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        bridge.alloc_virtual_id()
    } else {
        -9999
    };
    let mut obj = JsObject::new();
    obj.set(String::from("__nodeId"), JsValue::Number(virtual_id as f64));
    obj.set(String::from("nodeType"), JsValue::Number(3.0));
    obj.set(String::from("textContent"), JsValue::String(text.clone()));
    obj.set(String::from("innerText"), JsValue::String(text));
    obj.set_hook = Some(dom_property_hook);
    obj.set_hook_data = virtual_id as usize as *mut u8;
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_document_fragment(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("nodeType"), JsValue::Number(11.0));
    obj.set(String::from("children"), JsValue::Array(Rc::new(RefCell::new(JsArray::new()))));
    obj.set(String::from("childNodes"), JsValue::Array(Rc::new(RefCell::new(JsArray::new()))));
    obj.set(String::from("appendChild"), native_fn("appendChild", frag_append_child));
    obj.set(String::from("removeChild"), native_fn("removeChild", frag_remove_child));
    obj.set(String::from("querySelector"), native_fn("querySelector", |_,_| JsValue::Null));
    obj.set(String::from("querySelectorAll"), native_fn("querySelectorAll", |_,_| make_array(Vec::new())));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_comment(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let text = arg_string(args, 0);
    let mut obj = JsObject::new();
    obj.set(String::from("nodeType"), JsValue::Number(8.0));
    obj.set(String::from("textContent"), JsValue::String(text));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"), JsValue::String(typ));
    evt.set_property(String::from("target"), JsValue::Null);
    evt.set_property(String::from("preventDefault"), native_fn("preventDefault", doc_noop));
    evt.set_property(String::from("stopPropagation"), native_fn("stopPropagation", doc_noop));
    evt
}

fn doc_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);

    // For DOMContentLoaded/load, fire immediately since doc is already loaded.
    if event == "DOMContentLoaded" || event == "load" || event == "readystatechange" {
        if let JsValue::Function(_) = &callback {
            vm.call_value(&callback, &[], JsValue::Undefined);
        }
        return JsValue::Undefined;
    }

    // Store for other events (node_id 0 = document root).
    if let Some(bridge) = get_bridge(vm) {
        bridge.event_listeners.push(super::EventListener {
            node_id: 0,
            event,
            callback,
        });
    }
    JsValue::Undefined
}

fn doc_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }

// ── DocumentFragment helpers ──

fn frag_append_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("children") {
            if let JsValue::Array(arr) = &p.value {
                arr.borrow_mut().elements.push(child.clone());
            }
        }
    }
    child
}

fn frag_remove_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("children") {
            if let JsValue::Array(arr) = &p.value {
                let child_id = if let JsValue::Object(cobj) = &child {
                    cobj.borrow().get("__nodeId").to_number() as i64
                } else { -9999 };
                arr.borrow_mut().elements.retain(|el| {
                    if let JsValue::Object(eobj) = el {
                        eobj.borrow().get("__nodeId").to_number() as i64 != child_id
                    } else { true }
                });
            }
        }
    }
    child
}

/// Image constructor: `new Image()` → `document.createElement('img')`.
pub fn native_image_ctor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    doc_create_element(vm, &[JsValue::String(String::from("img"))])
}
