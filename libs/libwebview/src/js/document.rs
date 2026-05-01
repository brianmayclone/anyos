//! Native document host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::value::{JsArray, JsObject, Property};
use libjs::vm::native_fn;
use libjs::JsValue;
use libjs::Vm;

use crate::dom::{Dom, NodeType, Tag};

use super::element;
use super::element::refresh_element_children_metadata;
use super::selector;
use super::{
    arg_string, dom_property_hook, get_bridge, make_array, DomMutation, PendingNavigationRequest,
    VirtualNode,
};

// ═══════════════════════════════════════════════════════════
// URL parsing helper
// ═══════════════════════════════════════════════════════════

/// Parse a URL string into its Location object fields.
/// Returns `(protocol, hostname, host, port, pathname, search, hash, origin)`.
fn parse_location_fields(
    url: &str,
) -> (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
) {
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
        (
            String::from(&path_search[..pos]),
            String::from(&path_search[pos..]),
        )
    } else {
        (String::from(path_search), String::new())
    };

    let host = if port.is_empty() {
        hostname.clone()
    } else {
        alloc::format!("{}:{}", hostname, port)
    };

    // origin = protocol + "//" + hostname (+ port if non-standard)
    let mut origin = protocol.clone();
    origin.push_str("//");
    origin.push_str(&hostname);
    if !port.is_empty() {
        let is_default =
            (protocol == "http:" && port == "80") || (protocol == "https:" && port == "443");
        if !is_default {
            origin.push(':');
            origin.push_str(&port);
        }
    }

    (
        protocol, hostname, host, port, pathname, search, hash, origin,
    )
}

// ═══════════════════════════════════════════════════════════
// Document cookie write hook
// ═══════════════════════════════════════════════════════════

/// Property-write hook installed on the document JsObject.
/// Intercepts `document.cookie = "name=value"` writes and records them as
/// `DomMutation::SetCookie` so the host application (e.g. surf) can update
/// its cookie jar.
fn doc_property_hook(_data: *mut u8, key: &str, value: &libjs::JsValue) {
    if key != "cookie" {
        return;
    }
    let mutations = unsafe {
        if super::MUTATION_TARGET.is_null() {
            return;
        }
        &mut *super::MUTATION_TARGET
    };
    mutations.push(DomMutation::SetCookie {
        value: value.to_js_string(),
    });
}

fn queue_navigation(vm: &mut Vm, url: String, replace: bool) -> JsValue {
    if url.is_empty() || url == "undefined" {
        return JsValue::Undefined;
    }
    if let Some(bridge) = get_bridge(vm) {
        bridge
            .pending_navigation_requests
            .push(PendingNavigationRequest { url, replace });
    }
    JsValue::Undefined
}

fn location_assign(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    queue_navigation(vm, arg_string(args, 0), false)
}

fn location_replace(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    queue_navigation(vm, arg_string(args, 0), true)
}

fn location_reload(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let href = vm
        .current_this
        .get_property("href")
        .to_js_string();
    queue_navigation(vm, href, true)
}

fn location_property_hook(_data: *mut u8, key: &str, value: &libjs::JsValue) {
    if key != "href" {
        return;
    }
    let pending = unsafe {
        if super::NAVIGATION_TARGET.is_null() {
            return;
        }
        &mut *super::NAVIGATION_TARGET
    };
    let url = value.to_js_string();
    if !url.is_empty() && url != "undefined" {
        pending.push(PendingNavigationRequest {
            url,
            replace: false,
        });
    }
}

/// Create the native `document` host object.
///
/// * `url`     — the current page URL (used to populate `document.location`).
/// * `cookies` — the `Cookie` header value for this domain, used to populate
///               `document.cookie`.  Writes to `document.cookie` are recorded
///               as `DomMutation::SetCookie` mutations.
pub fn make_document(vm: &mut Vm, dom: &Dom, url: &str, cookies: &str) -> JsValue {
    let title = dom.find_title().unwrap_or_else(|| String::from(""));

    // Parse URL into location fields.
    let href = String::from(url);
    let (protocol, hostname, host, port, pathname, search, hash, origin) =
        parse_location_fields(url);

    let mut obj = JsObject::new();
    if let JsValue::Function(func) = vm.get_global("Document") {
        obj.prototype = func.borrow().prototype.clone();
    }

    // Properties.
    obj.set(String::from("title"), JsValue::String(title));
    obj.properties.insert(
        String::from("documentElement"),
        Property::accessor(
            Some(native_fn("get documentElement", doc_get_document_element)),
            None,
        ),
    );
    obj.properties.insert(
        String::from("body"),
        Property::accessor(Some(native_fn("get body", doc_get_body)), None),
    );
    obj.properties.insert(
        String::from("head"),
        Property::accessor(Some(native_fn("get head", doc_get_head)), None),
    );
    // cookie — readable; writes are intercepted by doc_property_hook
    obj.set(
        String::from("cookie"),
        JsValue::String(String::from(cookies)),
    );
    obj.set(
        String::from("readyState"),
        JsValue::String(String::from("complete")),
    );
    obj.set(String::from("referrer"), JsValue::String(String::new()));
    obj.set(String::from("domain"), JsValue::String(hostname.clone()));
    obj.set(String::from("URL"), JsValue::String(href.clone()));
    obj.set(
        String::from("characterSet"),
        JsValue::String(String::from("UTF-8")),
    );
    obj.set(
        String::from("contentType"),
        JsValue::String(String::from("text/html")),
    );
    obj.set(
        String::from("compatMode"),
        JsValue::String(String::from("CSS1Compat")),
    );
    obj.set(String::from("defaultView"), JsValue::Null);

    // location sub-object — all fields populated from the current URL.
    let loc = JsValue::new_object();
    loc.set_property(String::from("href"), JsValue::String(href));
    loc.set_property(String::from("hostname"), JsValue::String(hostname));
    loc.set_property(String::from("host"), JsValue::String(host));
    loc.set_property(String::from("port"), JsValue::String(port));
    loc.set_property(String::from("pathname"), JsValue::String(pathname));
    loc.set_property(String::from("protocol"), JsValue::String(protocol));
    loc.set_property(String::from("search"), JsValue::String(search));
    loc.set_property(String::from("hash"), JsValue::String(hash));
    loc.set_property(String::from("origin"), JsValue::String(origin));
    loc.set_property(
        String::from("assign"),
        native_fn("assign", location_assign),
    );
    loc.set_property(
        String::from("replace"),
        native_fn("replace", location_replace),
    );
    loc.set_property(
        String::from("reload"),
        native_fn("reload", location_reload),
    );
    if let JsValue::Object(o) = &loc {
        let mut borrowed = o.borrow_mut();
        borrowed.set_hook = Some(location_property_hook);
        borrowed.set_hook_data = core::ptr::null_mut();
    }
    obj.set(String::from("location"), loc);

    // implementation sub-object.
    let impl_obj = JsValue::new_object();
    impl_obj.set_property(
        String::from("hasFeature"),
        native_fn("hasFeature", |_, _| JsValue::Bool(true)),
    );
    impl_obj.set_property(
        String::from("createHTMLDocument"),
        native_fn("createHTMLDocument", |vm, _| vm.get_global("document")),
    );
    obj.set(String::from("implementation"), impl_obj);

    // ── Native methods ──
    obj.set(
        String::from("getElementById"),
        native_fn("getElementById", doc_get_element_by_id),
    );
    obj.set(
        String::from("getElementsByTagName"),
        native_fn("getElementsByTagName", doc_get_elements_by_tag_name),
    );
    obj.set(
        String::from("getElementsByClassName"),
        native_fn("getElementsByClassName", doc_get_elements_by_class_name),
    );
    obj.set(
        String::from("querySelector"),
        native_fn("querySelector", doc_query_selector),
    );
    obj.set(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", doc_query_selector_all),
    );
    obj.set(
        String::from("createElement"),
        native_fn("createElement", doc_create_element),
    );
    obj.set(
        String::from("createElementNS"),
        native_fn("createElementNS", doc_create_element_ns),
    );
    obj.set(
        String::from("createTextNode"),
        native_fn("createTextNode", doc_create_text_node),
    );
    obj.set(
        String::from("createDocumentFragment"),
        native_fn("createDocumentFragment", doc_create_document_fragment),
    );
    obj.set(
        String::from("createComment"),
        native_fn("createComment", doc_create_comment),
    );
    obj.set(
        String::from("createEvent"),
        native_fn("createEvent", doc_create_event),
    );
    obj.set(
        String::from("addEventListener"),
        native_fn("addEventListener", doc_add_event_listener),
    );
    obj.set(
        String::from("installListener"),
        native_fn("installListener", doc_add_event_listener),
    );
    obj.set(
        String::from("removeEventListener"),
        native_fn("removeEventListener", super::native_remove_event_listener),
    );
    obj.set(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", |_, _| JsValue::Bool(true)),
    );

    // W3C DOM: activeElement defaults to <body>.
    obj.properties.insert(
        String::from("activeElement"),
        Property::accessor(
            Some(native_fn("get activeElement", doc_get_active_element)),
            None,
        ),
    );
    // createTreeWalker / createRange stubs (used by React hydration).
    obj.set(
        String::from("createTreeWalker"),
        native_fn("createTreeWalker", doc_create_tree_walker),
    );
    obj.set(
        String::from("createRange"),
        native_fn("createRange", doc_create_range),
    );
    // hidden (used by some React checks).
    obj.set(String::from("hidden"), JsValue::Bool(false));
    obj.set(
        String::from("visibilityState"),
        JsValue::String(String::from("visible")),
    );

    // Install property-write hook to intercept `document.cookie = "..."`.
    obj.set_hook = Some(doc_property_hook);
    obj.set_hook_data = core::ptr::null_mut();

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// Document methods
// ═══════════════════════════════════════════════════════════

fn doc_get_document_element(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if matches!(&node.node_type, NodeType::Element { tag: Tag::Html, .. }) {
                return element::make_element(vm, i as i64);
            }
        }
        if !dom.nodes.is_empty() {
            return element::make_element(vm, 0);
        }
    }
    JsValue::Null
}

fn doc_get_body(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        if let Some(body_id) = dom.find_body() {
            return element::make_element(vm, body_id as i64);
        }
    }
    JsValue::Null
}

fn doc_get_head(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if matches!(&node.node_type, NodeType::Element { tag: Tag::Head, .. }) {
                return element::make_element(vm, i as i64);
            }
        }
    }
    JsValue::Null
}

fn doc_get_active_element(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    doc_get_body(vm, &[])
}

fn doc_get_element_by_id(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = arg_string(args, 0);
    if id.is_empty() {
        return JsValue::Null;
    }
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
    let results: Vec<JsValue> = ids
        .iter()
        .map(|&id| element::make_element(vm, id))
        .collect();
    make_array(results)
}

fn doc_get_elements_by_class_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class_name = arg_string(args, 0);
    if class_name.is_empty() {
        return make_array(Vec::new());
    }
    let mut ids = Vec::new();
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { attrs, .. } = &node.node_type {
                if attrs.iter().any(|a| {
                    a.name == "class" && a.value.split_whitespace().any(|c| c == class_name)
                }) {
                    ids.push(i as i64);
                }
            }
        }
    }
    let results: Vec<JsValue> = ids
        .iter()
        .map(|&id| element::make_element(vm, id))
        .collect();
    make_array(results)
}

fn doc_query_selector(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() {
        return JsValue::Null;
    }
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
    if sel.is_empty() {
        return make_array(Vec::new());
    }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let ids = selector::find_all(dom, &sel);
        let elems: Vec<JsValue> = ids
            .iter()
            .map(|&id| element::make_element(vm, id as i64))
            .collect();
        return make_array(elems);
    }
    make_array(Vec::new())
}

fn doc_create_element(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag = arg_string(args, 0).to_ascii_uppercase();
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        let id = bridge.alloc_virtual_id();
        bridge.mutations.push(DomMutation::CreateElement {
            virtual_id: id,
            tag: tag.clone(),
        });
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

/// `document.createElementNS(namespaceURI, qualifiedName)` — W3C DOM §4.5.
/// Creates an element with the given namespace.  We treat all namespaces
/// identically (HTML) but set `namespaceURI` on the resulting element.
fn doc_create_element_ns(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let ns = arg_string(args, 0);
    let tag = arg_string(args, 1).to_ascii_uppercase();
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        let id = bridge.alloc_virtual_id();
        bridge.mutations.push(DomMutation::CreateElement {
            virtual_id: id,
            tag: tag.clone(),
        });
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
    let el = element::make_element(vm, virtual_id);
    // Override namespace with the requested one.
    if !ns.is_empty() {
        el.set_property(String::from("namespaceURI"), JsValue::String(ns));
    }
    el
}

fn doc_create_text_node(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let text = arg_string(args, 0);
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        let id = bridge.alloc_virtual_id();
        bridge.mutations.push(DomMutation::CreateTextNode {
            virtual_id: id,
            text: text.clone(),
        });
        id
    } else {
        -9999
    };
    let mut obj = JsObject::new();
    if let JsValue::Function(func) = vm.get_global("Text") {
        obj.prototype = func.borrow().prototype.clone();
    }
    obj.set(String::from("__nodeId"), JsValue::Number(virtual_id as f64));
    obj.set(String::from("nodeType"), JsValue::Number(3.0));
    obj.set(
        String::from("nodeName"),
        JsValue::String(String::from("#text")),
    );
    obj.set(String::from("textContent"), JsValue::String(text.clone()));
    obj.set(String::from("nodeValue"), JsValue::String(text.clone()));
    obj.set(String::from("data"), JsValue::String(text.clone()));
    obj.set(String::from("innerText"), JsValue::String(text));
    obj.set(String::from("parentNode"), JsValue::Null);
    obj.set(String::from("nextSibling"), JsValue::Null);
    obj.set(String::from("previousSibling"), JsValue::Null);
    obj.set_hook = Some(dom_property_hook);
    obj.set_hook_data = virtual_id as usize as *mut u8;
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_document_fragment(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    if let JsValue::Function(func) = vm.get_global("DocumentFragment") {
        obj.prototype = func.borrow().prototype.clone();
    }
    obj.set(String::from("nodeType"), JsValue::Number(11.0));
    obj.set(
        String::from("nodeName"),
        JsValue::String(String::from("#document-fragment")),
    );
    obj.set(
        String::from("children"),
        JsValue::Array(Rc::new(RefCell::new(JsArray::new()))),
    );
    obj.set(
        String::from("childNodes"),
        JsValue::Array(Rc::new(RefCell::new(JsArray::new()))),
    );
    obj.set(String::from("firstChild"), JsValue::Null);
    obj.set(String::from("lastChild"), JsValue::Null);
    obj.set(String::from("childElementCount"), JsValue::Number(0.0));
    obj.set(String::from("textContent"), JsValue::String(String::new()));
    obj.set(
        String::from("appendChild"),
        native_fn("appendChild", frag_append_child),
    );
    obj.set(
        String::from("removeChild"),
        native_fn("removeChild", frag_remove_child),
    );
    obj.set(
        String::from("insertBefore"),
        native_fn("insertBefore", frag_insert_before),
    );
    obj.set(
        String::from("cloneNode"),
        native_fn("cloneNode", frag_clone_node),
    );
    obj.set(
        String::from("querySelector"),
        native_fn("querySelector", frag_query_selector),
    );
    obj.set(
        String::from("querySelectorAll"),
        native_fn("querySelectorAll", frag_query_selector_all),
    );
    obj.set(
        String::from("getElementById"),
        native_fn("getElementById", frag_get_element_by_id),
    );
    obj.set(
        String::from("append"),
        native_fn("append", frag_append_child),
    );
    obj.set(String::from("prepend"), native_fn("prepend", frag_prepend));
    obj.set(
        String::from("replaceChildren"),
        native_fn("replaceChildren", frag_replace_children),
    );
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_comment(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let text = arg_string(args, 0);
    let virtual_id = if let Some(bridge) = get_bridge(vm) {
        let id = bridge.alloc_virtual_id();
        // Comment nodes are invisible but serve as DOM anchors for React
        bridge.mutations.push(DomMutation::CreateTextNode {
            virtual_id: id,
            text: String::new(),
        });
        id
    } else {
        -9999
    };
    let mut obj = JsObject::new();
    if let JsValue::Function(func) = vm.get_global("Comment") {
        obj.prototype = func.borrow().prototype.clone();
    }
    obj.set(String::from("__nodeId"), JsValue::Number(virtual_id as f64));
    obj.set(String::from("nodeType"), JsValue::Number(8.0));
    obj.set(
        String::from("nodeName"),
        JsValue::String(String::from("#comment")),
    );
    obj.set(String::from("textContent"), JsValue::String(text.clone()));
    obj.set(String::from("data"), JsValue::String(text));
    obj.set(String::from("parentNode"), JsValue::Null);
    obj.set(String::from("nextSibling"), JsValue::Null);
    obj.set(String::from("previousSibling"), JsValue::Null);
    obj.set_hook = Some(dom_property_hook);
    obj.set_hook_data = virtual_id as usize as *mut u8;
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn doc_create_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"), JsValue::String(typ));
    evt.set_property(String::from("bubbles"), JsValue::Bool(false));
    evt.set_property(String::from("cancelable"), JsValue::Bool(false));
    evt.set_property(String::from("composed"), JsValue::Bool(false));
    evt.set_property(String::from("isTrusted"), JsValue::Bool(false));
    evt.set_property(String::from("defaultPrevented"), JsValue::Bool(false));
    evt.set_property(String::from("target"), JsValue::Null);
    evt.set_property(String::from("currentTarget"), JsValue::Null);
    evt.set_property(String::from("eventPhase"), JsValue::Number(0.0));
    evt.set_property(String::from("timeStamp"), JsValue::Number(0.0));
    evt.set_property(
        String::from("preventDefault"),
        native_fn("preventDefault", |vm, _| {
            vm.current_this
                .set_property(String::from("defaultPrevented"), JsValue::Bool(true));
            JsValue::Undefined
        }),
    );
    evt.set_property(
        String::from("stopPropagation"),
        native_fn("stopPropagation", doc_noop),
    );
    evt.set_property(
        String::from("stopImmediatePropagation"),
        native_fn("stopImmediatePropagation", doc_noop),
    );
    evt.set_property(
        String::from("composedPath"),
        native_fn("composedPath", |_, _| make_array(Vec::new())),
    );
    evt.set_property(
        String::from("initEvent"),
        native_fn("initEvent", |vm, args| {
            let typ = arg_string(args, 0);
            let bubbles = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
            let cancelable = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            vm.current_this
                .set_property(String::from("type"), JsValue::String(typ));
            vm.current_this
                .set_property(String::from("bubbles"), JsValue::Bool(bubbles));
            vm.current_this
                .set_property(String::from("cancelable"), JsValue::Bool(cancelable));
            JsValue::Undefined
        }),
    );
    evt.set_property(
        String::from("initCustomEvent"),
        native_fn("initCustomEvent", |vm, args| {
            let typ = arg_string(args, 0);
            let bubbles = args.get(1).map(|v| v.to_boolean()).unwrap_or(false);
            let cancelable = args.get(2).map(|v| v.to_boolean()).unwrap_or(false);
            let detail = args.get(3).cloned().unwrap_or(JsValue::Null);
            vm.current_this
                .set_property(String::from("type"), JsValue::String(typ));
            vm.current_this
                .set_property(String::from("bubbles"), JsValue::Bool(bubbles));
            vm.current_this
                .set_property(String::from("cancelable"), JsValue::Bool(cancelable));
            vm.current_this.set_property(String::from("detail"), detail);
            JsValue::Undefined
        }),
    );
    evt
}

fn doc_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let capture = match args.get(2) {
        Some(JsValue::Bool(b)) => *b,
        Some(JsValue::Object(_)) => args[2].get_property("capture").to_boolean(),
        _ => false,
    };

    // For DOMContentLoaded/load, fire immediately since doc is already loaded.
    if event == "DOMContentLoaded" || event == "load" || event == "readystatechange" {
        let document = vm.get_global("document");
        super::call_event_listener(vm, &callback, &JsValue::Undefined, &document);
        return JsValue::Undefined;
    }

    // Store for other events (node_id 0 = document root).
    if let Some(bridge) = get_bridge(vm) {
        bridge.event_listeners.push(super::EventListener {
            node_id: 0,
            event,
            callback,
            capture,
        });
    }
    JsValue::Undefined
}

fn doc_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

/// createTreeWalker (W3C DOM §5.2) — traverses the DOM tree depth-first.
///
/// Supports `whatToShow` filter:
/// - `NodeFilter.SHOW_ALL`     (0xFFFFFFFF)
/// - `NodeFilter.SHOW_ELEMENT` (0x1)
/// - `NodeFilter.SHOW_TEXT`    (0x4)
///
/// `nextNode()` does a depth-first pre-order traversal starting from root.
fn doc_create_tree_walker(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let root = args.first().cloned().unwrap_or(JsValue::Null);
    let what_to_show = args
        .get(1)
        .map(|v| v.to_number() as u32)
        .unwrap_or(0xFFFFFFFF);
    let root_id = super::element::extract_node_id_pub(&root);

    // Pre-build a flat list of all matching descendant node_ids in document order.
    let mut node_ids: Vec<i64> = Vec::new();
    if root_id >= 0 {
        if let Some(bridge) = get_bridge(vm) {
            let dom = bridge.dom();
            collect_tree_nodes(dom, root_id as usize, what_to_show, &mut node_ids);
        }
    }

    // Store as a JS array of numbers for the walker methods to iterate.
    let ids_arr: Vec<JsValue> = node_ids
        .iter()
        .map(|&id| JsValue::Number(id as f64))
        .collect();
    let walker = JsValue::new_object();
    walker.set_property(String::from("root"), root.clone());
    walker.set_property(String::from("currentNode"), root);
    walker.set_property(
        String::from("whatToShow"),
        JsValue::Number(what_to_show as f64),
    );
    walker.set_property(String::from("__ids"), JsValue::new_array(ids_arr));
    walker.set_property(String::from("__pos"), JsValue::Number(-1.0));

    walker.set_property(
        String::from("nextNode"),
        native_fn("nextNode", |vm, _| {
            let pos = vm.current_this.get_property("__pos").to_number() as i64;
            let ids = vm.current_this.get_property("__ids");
            let next_pos = pos + 1;
            if let JsValue::Array(arr) = &ids {
                let a = arr.borrow();
                if (next_pos as usize) < a.len() {
                    vm.current_this
                        .set_property(String::from("__pos"), JsValue::Number(next_pos as f64));
                    let nid = a.elements[&(next_pos as usize)].to_number() as i64;
                    let el = super::element::make_element(vm, nid);
                    vm.current_this
                        .set_property(String::from("currentNode"), el.clone());
                    return el;
                }
            }
            JsValue::Null
        }),
    );

    walker.set_property(
        String::from("previousNode"),
        native_fn("previousNode", |vm, _| {
            let pos = vm.current_this.get_property("__pos").to_number() as i64;
            let ids = vm.current_this.get_property("__ids");
            let prev_pos = pos - 1;
            if prev_pos >= 0 {
                if let JsValue::Array(arr) = &ids {
                    let a = arr.borrow();
                    if (prev_pos as usize) < a.len() {
                        vm.current_this
                            .set_property(String::from("__pos"), JsValue::Number(prev_pos as f64));
                        let nid = a.elements[&(prev_pos as usize)].to_number() as i64;
                        let el = super::element::make_element(vm, nid);
                        vm.current_this
                            .set_property(String::from("currentNode"), el.clone());
                        return el;
                    }
                }
            }
            JsValue::Null
        }),
    );

    walker.set_property(
        String::from("firstChild"),
        native_fn("firstChild", |_, _| JsValue::Null),
    );
    walker.set_property(
        String::from("lastChild"),
        native_fn("lastChild", |_, _| JsValue::Null),
    );
    walker.set_property(
        String::from("parentNode"),
        native_fn("parentNode", |_, _| JsValue::Null),
    );

    walker
}

/// Collect all node IDs matching `what_to_show` in document order (pre-order DFS).
fn collect_tree_nodes(
    dom: &crate::dom::Dom,
    node_id: usize,
    what_to_show: u32,
    out: &mut Vec<i64>,
) {
    if let Some(node) = dom.nodes.get(node_id) {
        let include = match &node.node_type {
            crate::dom::NodeType::Element { .. } => (what_to_show & 0x1) != 0,
            crate::dom::NodeType::Text(_) => (what_to_show & 0x4) != 0,
            _ => (what_to_show & 0xFFFFFFFF) == 0xFFFFFFFF,
        };
        if include {
            out.push(node_id as i64);
        }
        for &child_id in &node.children {
            collect_tree_nodes(dom, child_id, what_to_show, out);
        }
    }
}

/// createRange stub — returns a minimal Range object.
fn doc_create_range(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let range = JsValue::new_object();
    range.set_property(String::from("startContainer"), JsValue::Null);
    range.set_property(String::from("startOffset"), JsValue::Number(0.0));
    range.set_property(String::from("endContainer"), JsValue::Null);
    range.set_property(String::from("endOffset"), JsValue::Number(0.0));
    range.set_property(String::from("collapsed"), JsValue::Bool(true));
    range.set_property(String::from("setStart"), native_fn("setStart", doc_noop));
    range.set_property(String::from("setEnd"), native_fn("setEnd", doc_noop));
    range.set_property(
        String::from("selectNode"),
        native_fn("selectNode", |vm, args| {
            let node = args.first().cloned().unwrap_or(JsValue::Null);
            if let JsValue::Object(cur) = &vm.current_this {
                let mut r = cur.borrow_mut();
                r.set(String::from("startContainer"), node.clone());
                r.set(String::from("endContainer"), node);
                r.set(String::from("collapsed"), JsValue::Bool(false));
            }
            JsValue::Undefined
        }),
    );
    range.set_property(
        String::from("selectNodeContents"),
        native_fn("selectNodeContents", |vm, args| {
            let node = args.first().cloned().unwrap_or(JsValue::Null);
            if let JsValue::Object(cur) = &vm.current_this {
                let mut r = cur.borrow_mut();
                r.set(String::from("startContainer"), node.clone());
                r.set(String::from("endContainer"), node);
                r.set(String::from("collapsed"), JsValue::Bool(false));
            }
            JsValue::Undefined
        }),
    );
    range.set_property(String::from("collapse"), native_fn("collapse", doc_noop));
    range.set_property(
        String::from("cloneRange"),
        native_fn("cloneRange", doc_create_range),
    );
    range.set_property(
        String::from("getBoundingClientRect"),
        native_fn("getBoundingClientRect", |vm, _| {
            let start = vm.current_this.get_property("startContainer");
            let target = if start.is_null() || start.is_undefined() {
                vm.current_this.get_property("endContainer")
            } else {
                start
            };
            let rect_fn = target.get_property("getBoundingClientRect");
            if rect_fn.is_function() {
                vm.call_value(&rect_fn, &[], target);
                return vm.stack.pop().unwrap_or(JsValue::new_object());
            }
            let rect = JsValue::new_object();
            for k in &[
                "top", "left", "bottom", "right", "width", "height", "x", "y",
            ] {
                rect.set_property(String::from(*k), JsValue::Number(0.0));
            }
            rect
        }),
    );
    range
}

// ── DocumentFragment helpers ──

fn frag_append_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    frag_append_js_child(&vm.current_this, child.clone());
    child
}

fn frag_remove_child(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    frag_remove_js_child(&vm.current_this, element::extract_node_id_pub(&child));
    child
}

/// insertBefore(newChild, refChild) on a DocumentFragment.
fn frag_insert_before(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let new_child = args.first().cloned().unwrap_or(JsValue::Null);
    let ref_child = args.get(1).cloned().unwrap_or(JsValue::Null);
    frag_insert_js_before(&vm.current_this, new_child.clone(), &ref_child);
    new_child
}

/// cloneNode(deep) on a DocumentFragment — returns a new empty fragment
/// (deep cloning of virtual children is not supported yet).
fn frag_clone_node(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    doc_create_document_fragment(_vm, _args)
}

/// querySelector on a DocumentFragment — searches children by tag/id/class.
fn frag_query_selector(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("children") {
            if let JsValue::Array(arr) = &p.value {
                for (_k, child) in &arr.borrow().elements {
                    if frag_matches_selector(child, &sel) {
                        return child.clone();
                    }
                }
            }
        }
    }
    JsValue::Null
}

/// querySelectorAll on a DocumentFragment.
fn frag_query_selector_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    let mut results = Vec::new();
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("children") {
            if let JsValue::Array(arr) = &p.value {
                for (_k, child) in &arr.borrow().elements {
                    if frag_matches_selector(child, &sel) {
                        results.push(child.clone());
                    }
                }
            }
        }
    }
    make_array(results)
}

/// getElementById on a DocumentFragment.
fn frag_get_element_by_id(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = arg_string(args, 0);
    if id.is_empty() {
        return JsValue::Null;
    }
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("children") {
            if let JsValue::Array(arr) = &p.value {
                for (_k, child) in &arr.borrow().elements {
                    let child_id = child.get_property("id").to_js_string();
                    if child_id == id {
                        return child.clone();
                    }
                }
            }
        }
    }
    JsValue::Null
}

/// prepend(child) on a DocumentFragment — inserts at the beginning.
fn frag_prepend(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let child = args.first().cloned().unwrap_or(JsValue::Null);
    frag_prepend_js_child(&vm.current_this, child.clone());
    JsValue::Undefined
}

/// replaceChildren(...nodes) on a DocumentFragment — removes all children then appends args.
fn frag_replace_children(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    frag_replace_js_children(&vm.current_this, args);
    JsValue::Undefined
}

fn refresh_fragment_children_metadata(fragment: &JsValue) {
    refresh_element_children_metadata(fragment);
    let ordered_children = match fragment.get_property("children") {
        JsValue::Array(arr) => arr.borrow().values_vec(),
        _ => Vec::new(),
    };
    for child in &ordered_children {
        if let JsValue::Object(cobj) = child {
            cobj.borrow_mut()
                .set(String::from("parentElement"), JsValue::Null);
        }
    }
}

fn frag_clear_parent_links(child: &JsValue) {
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

fn frag_remove_js_child(fragment: &JsValue, child_id: i64) {
    if child_id == -9999 {
        return;
    }
    let children = fragment.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut()
            .elements
            .retain(|_k, el| element::extract_node_id_pub(el) != child_id);
        refresh_fragment_children_metadata(fragment);
    }
}

fn frag_detach_from_old_parent(child: &JsValue) {
    let old_parent = child.get_property("parentNode");
    let child_id = element::extract_node_id_pub(child);
    if !old_parent.is_null() && !old_parent.is_undefined() {
        if old_parent.get_property("nodeType").to_number() == 11.0 {
            frag_remove_js_child(&old_parent, child_id);
        } else {
            element::js_remove_child(&old_parent, child_id);
        }
    }
    element::clear_js_parent_links(child);
}

fn frag_append_js_child(fragment: &JsValue, child: JsValue) {
    frag_detach_from_old_parent(&child);
    let children = fragment.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut().push(child);
        refresh_fragment_children_metadata(fragment);
    }
}

fn frag_prepend_js_child(fragment: &JsValue, child: JsValue) {
    frag_detach_from_old_parent(&child);
    let children = fragment.get_property("children");
    if let JsValue::Array(arr) = &children {
        arr.borrow_mut().insert_and_shift(0, child);
        refresh_fragment_children_metadata(fragment);
    }
}

fn frag_insert_js_before(fragment: &JsValue, child: JsValue, ref_child: &JsValue) {
    frag_detach_from_old_parent(&child);
    let ref_id = element::extract_node_id_pub(ref_child);
    let children = fragment.get_property("children");
    if let JsValue::Array(arr) = &children {
        let mut arr_mut = arr.borrow_mut();
        if ref_id == -9999 {
            arr_mut.push(child);
        } else if let Some(idx) = arr_mut
            .elements
            .iter()
            .find(|(_k, el)| element::extract_node_id_pub(el) == ref_id)
            .map(|(k, _)| *k)
        {
            arr_mut.insert_and_shift(idx, child);
        } else {
            arr_mut.push(child);
        }
        refresh_fragment_children_metadata(fragment);
    }
}

fn frag_replace_js_children(fragment: &JsValue, args: &[JsValue]) {
    let old_children = match fragment.get_property("children") {
        JsValue::Array(arr) => arr.borrow().values_vec(),
        _ => Vec::new(),
    };
    for child in &old_children {
        frag_clear_parent_links(child);
    }
    let children = fragment.get_property("children");
    if let JsValue::Array(arr) = &children {
        let mut arr_mut = arr.borrow_mut();
        arr_mut.elements.clear();
        arr_mut.length = 0;
    }
    refresh_fragment_children_metadata(fragment);
    for arg in args {
        frag_append_js_child(fragment, arg.clone());
    }
}

/// Simple selector matching for DocumentFragment children.
/// Supports: tag name, #id, .class (shallow, single-level only).
fn frag_matches_selector(el: &JsValue, sel: &str) -> bool {
    let sel = sel.trim();
    if sel.is_empty() {
        return false;
    }
    if sel.starts_with('#') {
        let id = el.get_property("id").to_js_string();
        return id == &sel[1..];
    }
    if sel.starts_with('.') {
        let class = el.get_property("className").to_js_string();
        let target = &sel[1..];
        return class.split_whitespace().any(|c| c == target);
    }
    // Tag name match (case-insensitive).
    let tag = el.get_property("tagName").to_js_string();
    tag.eq_ignore_ascii_case(sel)
}

/// Image constructor: `new Image()` → `document.createElement('img')`.
pub fn native_image_ctor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    doc_create_element(vm, &[JsValue::String(String::from("img"))])
}
