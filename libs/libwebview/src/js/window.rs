//! Native window host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use libjs::value::JsObject;
use libjs::vm::native_fn;
use libjs::JsValue;
use libjs::Vm;

use super::fetch;
use super::storage;
use super::xhr;
use super::{arg_string, get_bridge, make_array};
use crate::dom::NodeType;

// ── JsValue option helpers (not on the type itself) ──────────────────────────

/// Extract a number from a JsValue, returning `default` for Null/Undefined.
#[inline]
fn opt_num(v: JsValue, default: f64) -> JsValue {
    JsValue::Number(if v.is_undefined() || v.is_null() {
        default
    } else {
        v.to_number()
    })
}

/// Extract a String from a JsValue, returning `default` for Null/Undefined.
#[inline]
fn opt_str(v: JsValue, default: &str) -> JsValue {
    if v.is_undefined() || v.is_null() {
        JsValue::String(String::from(default))
    } else {
        JsValue::String(v.to_js_string())
    }
}

fn promise_resolve_value(vm: &mut Vm, value: JsValue) -> JsValue {
    let promise_ctor = vm.get_global("Promise");
    if let JsValue::Function(_) = &promise_ctor {
        let resolve_fn = promise_ctor.get_property("resolve");
        if let JsValue::Function(f) = resolve_fn {
            let kind = f.borrow().kind.clone();
            if let libjs::value::FnKind::Native(native) = kind {
                return native(vm, &[value]);
            }
        }
    }
    value
}

fn make_native_constructor(
    vm: &Vm,
    name: &str,
    native: fn(&mut Vm, &[JsValue]) -> JsValue,
    parent_proto: Option<Rc<RefCell<JsObject>>>,
) -> JsValue {
    let ctor = native_fn(name, native);
    if let JsValue::Function(func) = &ctor {
        let mut proto = JsObject::new();
        proto.prototype = Some(parent_proto.unwrap_or_else(|| vm.object_proto.clone()));
        let proto = Rc::new(RefCell::new(proto));
        proto
            .borrow_mut()
            .set(String::from("constructor"), ctor.clone());
        func.borrow_mut().prototype = Some(proto);
    }
    ctor
}

/// Create the native `window` host object.
///
/// * `origin` — the page origin (e.g. `"https://example.com"`) used to key
///   the persistent localStorage file.
pub fn make_window(
    vm: &mut Vm,
    document: JsValue,
    origin: &str,
    viewport_w: u32,
    viewport_h: u32,
) -> JsValue {
    let mut obj = JsObject::new();

    obj.set(String::from("document"), document.clone());

    // location — share from document.
    let loc = document.get_property("location");
    obj.set(String::from("location"), loc);

    // navigator.
    let nav = JsValue::new_object();
    nav.set_property(
        String::from("userAgent"),
        JsValue::String(String::from("anyOS Surf/1.0")),
    );
    nav.set_property(
        String::from("language"),
        JsValue::String(String::from("en-US")),
    );
    nav.set_property(
        String::from("languages"),
        make_array(vec![JsValue::String(String::from("en-US"))]),
    );
    nav.set_property(
        String::from("platform"),
        JsValue::String(String::from("anyOS")),
    );
    nav.set_property(String::from("cookieEnabled"), JsValue::Bool(true));
    nav.set_property(String::from("onLine"), JsValue::Bool(true));
    nav.set_property(
        String::from("vendor"),
        JsValue::String(String::from("anyOS")),
    );
    nav.set_property(
        String::from("appName"),
        JsValue::String(String::from("Surf")),
    );
    nav.set_property(
        String::from("appVersion"),
        JsValue::String(String::from("1.0")),
    );
    let permissions = JsValue::new_object();
    permissions.set_property(
        String::from("query"),
        native_fn("query", navigator_permissions_query),
    );
    nav.set_property(String::from("permissions"), permissions);
    let storage = JsValue::new_object();
    storage.set_property(
        String::from("estimate"),
        native_fn("estimate", navigator_storage_estimate),
    );
    storage.set_property(
        String::from("persist"),
        native_fn("persist", navigator_storage_persist),
    );
    storage.set_property(
        String::from("persisted"),
        native_fn("persisted", navigator_storage_persisted),
    );
    nav.set_property(String::from("storage"), storage);
    let clipboard = JsValue::new_object();
    clipboard.set_property(
        String::from("readText"),
        native_fn("readText", navigator_clipboard_read_text),
    );
    clipboard.set_property(
        String::from("writeText"),
        native_fn("writeText", navigator_clipboard_write_text),
    );
    nav.set_property(String::from("clipboard"), clipboard);
    nav.set_property(
        String::from("sendBeacon"),
        native_fn("sendBeacon", navigator_send_beacon),
    );
    nav.set_property(String::from("share"), native_fn("share", navigator_share));
    obj.set(String::from("navigator"), nav);

    // screen.
    let screen = JsValue::new_object();
    screen.set_property(String::from("width"), JsValue::Number(viewport_w as f64));
    screen.set_property(String::from("height"), JsValue::Number(viewport_h as f64));
    screen.set_property(
        String::from("availWidth"),
        JsValue::Number(viewport_w as f64),
    );
    screen.set_property(
        String::from("availHeight"),
        JsValue::Number(viewport_h as f64),
    );
    screen.set_property(String::from("colorDepth"), JsValue::Number(32.0));
    screen.set_property(String::from("pixelDepth"), JsValue::Number(32.0));
    let orient = JsValue::new_object();
    orient.set_property(
        String::from("type"),
        JsValue::String(String::from("landscape-primary")),
    );
    orient.set_property(String::from("angle"), JsValue::Number(0.0));
    screen.set_property(String::from("orientation"), orient);
    obj.set(String::from("screen"), screen);

    // Dimensions.
    obj.set(
        String::from("innerWidth"),
        JsValue::Number(viewport_w as f64),
    );
    obj.set(
        String::from("innerHeight"),
        JsValue::Number(viewport_h as f64),
    );
    obj.set(
        String::from("outerWidth"),
        JsValue::Number(viewport_w as f64),
    );
    obj.set(
        String::from("outerHeight"),
        JsValue::Number(viewport_h as f64),
    );
    obj.set(String::from("devicePixelRatio"), JsValue::Number(1.0));
    obj.set(String::from("pageXOffset"), JsValue::Number(0.0));
    obj.set(String::from("pageYOffset"), JsValue::Number(0.0));
    obj.set(String::from("scrollX"), JsValue::Number(0.0));
    obj.set(String::from("scrollY"), JsValue::Number(0.0));

    // Timer functions (backed by real timer infrastructure in mod.rs).
    obj.set(String::from("alert"), native_fn("alert", native_alert));
    obj.set(
        String::from("setTimeout"),
        native_fn("setTimeout", super::native_set_timeout),
    );
    obj.set(
        String::from("setInterval"),
        native_fn("setInterval", super::native_set_interval),
    );
    obj.set(
        String::from("clearTimeout"),
        native_fn("clearTimeout", super::native_clear_timeout),
    );
    obj.set(
        String::from("clearInterval"),
        native_fn("clearInterval", super::native_clear_interval),
    );

    // Style.
    obj.set(
        String::from("getComputedStyle"),
        native_fn("getComputedStyle", win_get_computed_style),
    );
    obj.set(
        String::from("requestAnimationFrame"),
        native_fn(
            "requestAnimationFrame",
            super::native_request_animation_frame,
        ),
    );
    obj.set(
        String::from("cancelAnimationFrame"),
        native_fn("cancelAnimationFrame", super::native_clear_timeout),
    );

    // Events.
    obj.set(
        String::from("addEventListener"),
        native_fn("addEventListener", win_add_event_listener),
    );
    obj.set(
        String::from("installListener"),
        native_fn("installListener", win_add_event_listener),
    );
    obj.set(
        String::from("removeEventListener"),
        native_fn("removeEventListener", super::native_remove_event_listener),
    );
    obj.set(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", |_, _| JsValue::Bool(true)),
    );

    // Base64 encoding/decoding (W3C HTML §8.3).
    obj.set(String::from("atob"), native_fn("atob", win_atob));
    obj.set(String::from("btoa"), native_fn("btoa", win_btoa));

    // Network.
    obj.set(
        String::from("fetch"),
        native_fn("fetch", fetch::native_fetch),
    );
    obj.set(String::from("XMLHttpRequest"), xhr::make_xhr_constructor());
    obj.set(
        String::from("Headers"),
        native_fn("Headers", fetch::native_headers_ctor),
    );

    // Performance (W3C Performance Timeline §4).
    let perf = JsValue::new_object();
    perf.set_property(String::from("now"), native_fn("now", win_performance_now));
    perf.set_property(String::from("mark"), native_fn("mark", win_noop));
    perf.set_property(String::from("measure"), native_fn("measure", win_noop));
    perf.set_property(
        String::from("getEntriesByName"),
        native_fn("getEntriesByName", |_, _| make_array(Vec::new())),
    );
    perf.set_property(
        String::from("getEntriesByType"),
        native_fn("getEntriesByType", |_, _| make_array(Vec::new())),
    );
    perf.set_property(String::from("timeOrigin"), JsValue::Number(0.0));
    obj.set(String::from("performance"), perf);

    // Storage.
    obj.set(
        String::from("localStorage"),
        storage::make_storage(origin, true),
    );
    obj.set(
        String::from("sessionStorage"),
        storage::make_storage(origin, false),
    );

    // History.
    let history = JsValue::new_object();
    history.set_property(String::from("length"), JsValue::Number(1.0));
    history.set_property(String::from("state"), JsValue::Null);
    history.set_property(String::from("pushState"), native_fn("pushState", win_noop));
    history.set_property(
        String::from("replaceState"),
        native_fn("replaceState", win_noop),
    );
    history.set_property(String::from("back"), native_fn("back", win_noop));
    history.set_property(String::from("forward"), native_fn("forward", win_noop));
    history.set_property(String::from("go"), native_fn("go", win_noop));
    obj.set(String::from("history"), history);

    // Scroll.
    obj.set(String::from("scrollTo"), native_fn("scrollTo", win_noop));
    obj.set(String::from("scrollBy"), native_fn("scrollBy", win_noop));

    // Dialogs.
    obj.set(
        String::from("open"),
        native_fn("open", |_, _| JsValue::Null),
    );
    obj.set(String::from("close"), native_fn("close", win_noop));
    obj.set(String::from("print"), native_fn("print", win_noop));
    obj.set(
        String::from("confirm"),
        native_fn("confirm", |_, _| JsValue::Bool(false)),
    );
    obj.set(String::from("prompt"), native_fn("prompt", win_prompt));
    obj.set(
        String::from("postMessage"),
        native_fn("postMessage", win_post_message),
    );

    // Media queries.
    obj.set(
        String::from("matchMedia"),
        native_fn("matchMedia", win_match_media),
    );
    obj.set(
        String::from("getSelection"),
        native_fn("getSelection", win_get_selection),
    );

    // Observer stubs.
    obj.set(
        String::from("ResizeObserver"),
        native_fn("ResizeObserver", win_observer_ctor),
    );
    obj.set(
        String::from("MutationObserver"),
        native_fn("MutationObserver", win_mutation_observer_ctor),
    );
    obj.set(
        String::from("IntersectionObserver"),
        native_fn("IntersectionObserver", win_observer_ctor),
    );

    // Event constructors (W3C DOM Events Level 3 / UIEvents / Pointer Events).
    obj.set(String::from("Event"), native_fn("Event", win_event));
    obj.set(
        String::from("CustomEvent"),
        native_fn("CustomEvent", win_custom_event),
    );
    obj.set(
        String::from("MouseEvent"),
        native_fn("MouseEvent", win_mouse_event),
    );
    obj.set(
        String::from("KeyboardEvent"),
        native_fn("KeyboardEvent", win_keyboard_event),
    );
    obj.set(
        String::from("InputEvent"),
        native_fn("InputEvent", win_input_event),
    );
    obj.set(
        String::from("FocusEvent"),
        native_fn("FocusEvent", win_focus_event),
    );
    obj.set(
        String::from("WheelEvent"),
        native_fn("WheelEvent", win_wheel_event),
    );
    obj.set(
        String::from("PointerEvent"),
        native_fn("PointerEvent", win_pointer_event),
    );

    // MessageChannel (W3C HTML §9.4 — used by React Scheduler for task deferral).
    obj.set(
        String::from("MessageChannel"),
        native_fn("MessageChannel", win_message_channel),
    );

    // URL / misc.
    obj.set(String::from("URL"), native_fn("URL", win_url_ctor));
    obj.set(
        String::from("URLSearchParams"),
        native_fn("URLSearchParams", win_url_search_params),
    );
    obj.set(
        String::from("TextEncoder"),
        native_fn("TextEncoder", win_text_encoder),
    );
    obj.set(
        String::from("TextDecoder"),
        native_fn("TextDecoder", win_text_decoder),
    );
    obj.set(
        String::from("AbortController"),
        native_fn("AbortController", win_abort_controller),
    );
    obj.set(
        String::from("queueMicrotask"),
        native_fn("queueMicrotask", win_queue_microtask),
    );
    obj.set(
        String::from("structuredClone"),
        native_fn("structuredClone", win_structured_clone),
    );
    obj.set(String::from("getCookie"), native_fn("getCookie", win_get_cookie));
    obj.set(
        String::from("getParameterByName"),
        native_fn("getParameterByName", win_get_parameter_by_name),
    );
    obj.set(
        String::from("clearEventListeners"),
        native_fn("clearEventListeners", win_noop),
    );
    obj.set(
        String::from("getRequestUUID"),
        native_fn("getRequestUUID", win_get_request_uuid),
    );
    obj.set(
        String::from("DOMParser"),
        native_fn("DOMParser", win_dom_parser),
    );
    let crypto = JsValue::new_object();
    crypto.set_property(
        String::from("randomUUID"),
        native_fn("randomUUID", win_get_request_uuid),
    );
    obj.set(String::from("crypto"), crypto);
    obj.set(
        String::from("Image"),
        native_fn("Image", super::document::native_image_ctor),
    );
    let node_ctor = make_native_constructor(vm, "Node", win_dom_ctor, None);
    let node_proto = match &node_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    let document_ctor = make_native_constructor(vm, "Document", win_dom_ctor, node_proto.clone());
    let document_proto = match &document_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    let document_fragment_ctor =
        make_native_constructor(vm, "DocumentFragment", win_dom_ctor, node_proto.clone());
    let character_data_ctor =
        make_native_constructor(vm, "CharacterData", win_dom_ctor, node_proto.clone());
    let character_data_proto = match &character_data_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    let document_type_ctor =
        make_native_constructor(vm, "DocumentType", win_dom_ctor, node_proto.clone());
    let text_ctor = make_native_constructor(vm, "Text", win_dom_ctor, character_data_proto.clone());
    let comment_ctor =
        make_native_constructor(vm, "Comment", win_dom_ctor, character_data_proto.clone());
    let element_ctor = make_native_constructor(vm, "Element", win_dom_ctor, node_proto.clone());
    let element_proto = match &element_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    // Populate Element.prototype with standard DOM methods so that
    // polyfill / framework code that feature-detects via
    // `Element.prototype.replaceWith` etc. finds them.
    if let Some(ref proto_rc) = element_proto {
        let proto_val = JsValue::Object(proto_rc.clone());
        super::element::populate_element_prototype(&proto_val);
    }
    let html_element_ctor =
        make_native_constructor(vm, "HTMLElement", win_dom_ctor, element_proto.clone());
    let custom_element_registry_ctor = make_native_constructor(
        vm,
        "CustomElementRegistry",
        win_custom_element_registry_ctor,
        None,
    );
    let custom_elements = win_custom_element_registry_ctor(vm, &[]);

    obj.set(String::from("Node"), node_ctor);
    obj.set(String::from("Document"), document_ctor);
    obj.set(String::from("DocumentFragment"), document_fragment_ctor);
    obj.set(String::from("CharacterData"), character_data_ctor);
    obj.set(String::from("DocumentType"), document_type_ctor);
    obj.set(String::from("Text"), text_ctor);
    obj.set(String::from("Comment"), comment_ctor);
    obj.set(String::from("Element"), element_ctor);
    obj.set(String::from("HTMLElement"), html_element_ctor);
    obj.set(
        String::from("CustomElementRegistry"),
        custom_element_registry_ctor,
    );
    obj.set(String::from("customElements"), custom_elements);

    // Set document.defaultView = window (after creation).
    let win = JsValue::Object(Rc::new(RefCell::new(obj)));
    let frames = JsValue::new_object();
    frames.set_property(String::from("length"), JsValue::Number(0.0));
    let parent = JsValue::new_object();
    parent.set_property(String::from("frames"), frames.clone());
    parent.set_property(String::from("length"), JsValue::Number(0.0));
    parent.set_property(String::from("frameElement"), JsValue::Null);
    parent.set_property(String::from("__tcfapi"), native_fn("__tcfapi", win_tcfapi));
    parent.set_property(String::from("__cmp"), native_fn("__cmp", win_cmp_stub));
    parent.set_property(
        String::from("__uspapi"),
        native_fn("__uspapi", win_cmp_stub),
    );
    parent.set_property(String::from("__tcfapiLocator"), JsValue::new_object());
    win.set_property(String::from("frames"), frames);
    win.set_property(String::from("frameElement"), JsValue::Null);
    win.set_property(String::from("opener"), JsValue::Null);
    win.set_property(String::from("name"), JsValue::String(String::new()));
    win.set_property(String::from("length"), JsValue::Number(0.0));
    win.set_property(String::from("__tcfapi"), native_fn("__tcfapi", win_tcfapi));
    win.set_property(String::from("__cmp"), native_fn("__cmp", win_cmp_stub));
    win.set_property(
        String::from("__uspapi"),
        native_fn("__uspapi", win_cmp_stub),
    );
    win.set_property(String::from("__tcfapiLocator"), JsValue::new_object());
    // Top-level documents are their own parent/top. Exposing a detached stub
    // here breaks feature detection on sites that access helpers through
    // `window.top` / `window.parent`.
    win.set_property(String::from("top"), win.clone());
    win.set_property(String::from("parent"), win.clone());
    if let (JsValue::Object(doc_obj), Some(doc_proto)) = (&document, document_proto) {
        doc_obj.borrow_mut().prototype = Some(doc_proto);
    }
    document.set_property(String::from("defaultView"), win.clone());
    win
}

// ═══════════════════════════════════════════════════════════
// Window method implementations
// ═══════════════════════════════════════════════════════════

pub fn native_alert(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(msg) = args.first() {
        vm.console_output
            .push(alloc::format!("[alert] {}", msg.to_js_string()));
    }
    JsValue::Undefined
}

fn win_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = super::arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    // Third argument: boolean or EventListenerOptions { capture: bool }.
    let capture = match args.get(2) {
        Some(JsValue::Bool(b)) => *b,
        Some(JsValue::Object(_)) => args[2].get_property("capture").to_boolean(),
        _ => false,
    };

    // For load/DOMContentLoaded, fire immediately.
    if event == "load" || event == "DOMContentLoaded" {
        if let JsValue::Function(_) = &callback {
            vm.call_value(&callback, &[], JsValue::Undefined);
        }
        return JsValue::Undefined;
    }

    // Store for later dispatch.
    if let Some(bridge) = super::get_bridge(vm) {
        bridge.event_listeners.push(super::EventListener {
            node_id: usize::MAX, // window pseudo-node
            event,
            callback,
            capture,
        });
    }
    JsValue::Undefined
}

fn win_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}
fn win_noop_obj(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_object()
}
fn win_dom_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_object()
}

fn win_custom_element_registry_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let registry = JsValue::new_object();
    registry.set_property(String::from("__entries"), JsValue::new_object());
    registry.set_property(
        String::from("define"),
        native_fn("define", win_custom_elements_define),
    );
    registry.set_property(
        String::from("get"),
        native_fn("get", win_custom_elements_get),
    );
    registry.set_property(
        String::from("whenDefined"),
        native_fn("whenDefined", win_custom_elements_when_defined),
    );
    registry.set_property(String::from("upgrade"), native_fn("upgrade", win_noop));
    registry
}

fn win_custom_elements_define(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    let ctor = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let entries = vm.current_this.get_property("__entries");
    if let JsValue::Object(obj) = entries {
        obj.borrow_mut().set(name, ctor);
    }
    JsValue::Undefined
}

fn win_custom_elements_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    let entries = vm.current_this.get_property("__entries");
    if let JsValue::Object(obj) = entries {
        return obj.borrow().get(&name);
    }
    JsValue::Undefined
}

fn win_custom_elements_when_defined(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let ctor = win_custom_elements_get(vm, args);
    let promise_ctor = vm.get_global("Promise");
    if let JsValue::Function(_) = &promise_ctor {
        let resolve_fn = promise_ctor.get_property("resolve");
        if let JsValue::Function(f) = resolve_fn {
            let kind = f.borrow().kind.clone();
            if let libjs::value::FnKind::Native(native) = kind {
                return native(vm, &[ctor]);
            }
        }
    }
    ctor
}

fn win_passthrough(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.first()
        .cloned()
        .unwrap_or(JsValue::String(String::new()))
}

fn win_get_computed_style(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Build a CSSStyleDeclaration-like object with computed values.
    let el = match args.first() {
        Some(el) => el,
        None => return JsValue::new_object(),
    };

    let mut obj = JsObject::new();

    // Copy all properties from the element's inline style object.
    let style = el.get_property("style");
    if let JsValue::Object(ref s) = style {
        let s_borrowed = s.borrow();
        for (key, prop) in &s_borrowed.properties {
            obj.set(key.clone(), prop.value.clone());
        }
    }

    // Try to read computed styles from the layout engine via the DOM bridge.
    let node_id = el.get_property("__nodeId").to_number() as i64;
    if node_id >= 0 {
        if let Some(bridge) = get_bridge(vm) {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                // Read inline style attribute values as additional computed properties.
                if let NodeType::Element { ref attrs, .. } = dom.nodes[nid].node_type {
                    for attr in attrs {
                        if attr.name == "style" {
                            // Parse inline style string into individual properties.
                            for decl in attr.value.split(';') {
                                let decl = decl.trim();
                                if let Some(colon) = decl.find(':') {
                                    let prop = decl[..colon].trim();
                                    let val = decl[colon + 1..].trim();
                                    if !prop.is_empty() && !val.is_empty() {
                                        let camel = css_to_camel(prop);
                                        // Don't overwrite properties already set from style object.
                                        let existing = obj.get(&camel);
                                        if existing.is_undefined() || existing.is_null() {
                                            obj.set(camel, JsValue::String(String::from(val)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Add getPropertyValue method.
    obj.set(
        String::from("getPropertyValue"),
        native_fn("getPropertyValue", computed_get_property_value),
    );
    // setProperty (no-op for computed styles, but scripts expect it to exist).
    obj.set(
        String::from("setProperty"),
        native_fn("setProperty", |_, _| JsValue::Undefined),
    );
    // removeProperty (no-op).
    obj.set(
        String::from("removeProperty"),
        native_fn("removeProperty", |_, _| JsValue::String(String::new())),
    );

    // Set default values for commonly queried CSS properties if not already set.
    let defaults: &[(&str, &str)] = &[
        ("display", "block"),
        ("visibility", "visible"),
        ("opacity", "1"),
        ("position", "static"),
        ("overflow", "visible"),
        ("pointerEvents", "auto"),
        ("userSelect", "auto"),
        ("float", "none"),
        ("clear", "none"),
        ("boxSizing", "content-box"),
        ("zIndex", "auto"),
        ("cursor", "auto"),
        ("textAlign", "start"),
        ("textDecoration", "none"),
        ("textTransform", "none"),
        ("whiteSpace", "normal"),
        ("wordBreak", "normal"),
        ("overflowWrap", "normal"),
        ("lineHeight", "normal"),
        ("fontStyle", "normal"),
        ("fontWeight", "400"),
        ("fontSize", "16px"),
        ("fontFamily", "serif"),
        ("color", "rgb(0, 0, 0)"),
        ("backgroundColor", "rgba(0, 0, 0, 0)"),
        ("margin", "0px"),
        ("marginTop", "0px"),
        ("marginRight", "0px"),
        ("marginBottom", "0px"),
        ("marginLeft", "0px"),
        ("padding", "0px"),
        ("paddingTop", "0px"),
        ("paddingRight", "0px"),
        ("paddingBottom", "0px"),
        ("paddingLeft", "0px"),
        ("borderStyle", "none"),
        ("borderWidth", "0px"),
        ("borderColor", "rgb(0, 0, 0)"),
        ("width", "auto"),
        ("height", "auto"),
        ("maxWidth", "none"),
        ("maxHeight", "none"),
        ("minWidth", "0px"),
        ("minHeight", "0px"),
        ("top", "auto"),
        ("right", "auto"),
        ("bottom", "auto"),
        ("left", "auto"),
        ("transform", "none"),
        ("transition", "all 0s ease 0s"),
        ("verticalAlign", "baseline"),
    ];
    for &(prop, default) in defaults {
        let existing = obj.get(prop);
        if existing.is_undefined() || existing.is_null() {
            obj.set(String::from(prop), JsValue::String(String::from(default)));
        }
    }

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `getPropertyValue(name)` on a computed style object.
/// Looks up both camelCase and kebab-case variants.
fn computed_get_property_value(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = arg_string(args, 0);
    let camel = css_to_camel(&prop);
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        // Try camelCase first.
        let val = o.get(&camel);
        if !val.is_undefined() && !val.is_null() {
            return JsValue::String(val.to_js_string());
        }
        // Try the raw property name (kebab-case).
        let val = o.get(&prop);
        if !val.is_undefined() && !val.is_null() {
            return JsValue::String(val.to_js_string());
        }
    }
    JsValue::String(String::new())
}

/// Convert a CSS kebab-case property name to camelCase.
/// e.g. "background-color" → "backgroundColor", "pointer-events" → "pointerEvents".
fn css_to_camel(name: &str) -> String {
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

fn win_prompt(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.get(1).cloned().unwrap_or(JsValue::Null)
}

fn win_match_media(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let q = arg_string(args, 0);
    let (vw, vh) = current_viewport(vm);
    let matches = eval_media_query(&q, vw, vh);
    let mql = JsValue::new_object();
    mql.set_property(String::from("matches"), JsValue::Bool(matches));
    mql.set_property(String::from("media"), JsValue::String(q));
    mql.set_property(String::from("onchange"), JsValue::Null);
    mql.set_property(
        String::from("addListener"),
        native_fn("addListener", win_noop),
    );
    mql.set_property(
        String::from("removeListener"),
        native_fn("removeListener", win_noop),
    );
    mql.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    mql.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    mql
}

fn current_viewport(vm: &mut Vm) -> (u32, u32) {
    let win = vm.get_global("window");
    let vw = match win.get_property("innerWidth") {
        JsValue::Number(n) if n > 0.0 => n as u32,
        _ => 1024,
    };
    let vh = match win.get_property("innerHeight") {
        JsValue::Number(n) if n > 0.0 => n as u32,
        _ => 768,
    };
    (vw, vh)
}

/// Evaluate common CSS media query patterns against viewport dimensions.
///
/// Supports: `(min-width: Npx)`, `(max-width: Npx)`, `(min-height: Npx)`,
/// `(max-height: Npx)`, `screen`, `all`, `(prefers-color-scheme: light|dark)`,
/// `(pointer: fine|coarse)`.
fn eval_media_query(query: &str, vw: u32, vh: u32) -> bool {
    let q = query.trim().to_ascii_lowercase();
    // "all" and "screen" always match.
    if q == "all" || q == "screen" || q.is_empty() {
        return true;
    }
    if q == "print" {
        return false;
    }

    // Check individual conditions separated by " and ".
    let conditions: Vec<&str> = if q.contains(" and ") {
        q.split(" and ").collect()
    } else {
        vec![q.as_str()]
    };

    for cond in conditions {
        let c = cond
            .trim()
            .trim_start_matches("screen")
            .trim()
            .trim_start_matches("all")
            .trim();
        if c.is_empty() || c == "screen" || c == "all" {
            continue;
        }
        let inner = c.trim_matches('(').trim_matches(')').trim();

        if inner.starts_with("min-width:") {
            if let Some(px) = parse_px_value(&inner[10..]) {
                if (vw as f64) < px {
                    return false;
                }
            }
        } else if inner.starts_with("max-width:") {
            if let Some(px) = parse_px_value(&inner[10..]) {
                if (vw as f64) > px {
                    return false;
                }
            }
        } else if inner.starts_with("min-height:") {
            if let Some(px) = parse_px_value(&inner[11..]) {
                if (vh as f64) < px {
                    return false;
                }
            }
        } else if inner.starts_with("max-height:") {
            if let Some(px) = parse_px_value(&inner[11..]) {
                if (vh as f64) > px {
                    return false;
                }
            }
        } else if inner == "prefers-color-scheme: dark" {
            return false; // we're light mode
        } else if inner == "prefers-color-scheme: light" {
            continue; // matches
        } else if inner == "pointer: fine" {
            continue; // desktop = fine pointer
        } else if inner == "pointer: coarse" {
            return false;
        } else if inner == "hover: hover" {
            continue; // desktop has hover
        } else if inner == "hover: none" {
            return false;
        } else if inner.starts_with("prefers-reduced-motion") {
            if inner.contains("reduce") {
                return false;
            }
        }
        // Unknown conditions: treat as matching (lenient).
    }
    true
}

fn parse_px_value(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches("px").trim();
    s.parse::<f64>().ok()
}

fn win_get_selection(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let sel = JsValue::new_object();
    sel.set_property(
        String::from("toString"),
        native_fn("toString", |_, _| JsValue::String(String::new())),
    );
    sel.set_property(String::from("rangeCount"), JsValue::Number(0.0));
    sel
}

fn win_observer_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let obs = JsValue::new_object();
    obs.set_property(String::from("observe"), native_fn("observe", win_noop));
    obs.set_property(String::from("unobserve"), native_fn("unobserve", win_noop));
    obs.set_property(
        String::from("disconnect"),
        native_fn("disconnect", win_noop),
    );
    obs
}

fn win_mutation_observer_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let obs = JsValue::new_object();

    // Store callback and observed config.
    obs.set_property(String::from("__callback"), callback);
    obs.set_property(String::from("__observing"), JsValue::Bool(false));
    obs.set_property(String::from("__target"), JsValue::Null);
    obs.set_property(String::from("__records"), JsValue::new_array(Vec::new()));
    // Options booleans.
    obs.set_property(String::from("__childList"), JsValue::Bool(false));
    obs.set_property(String::from("__attributes"), JsValue::Bool(false));
    obs.set_property(String::from("__characterData"), JsValue::Bool(false));
    obs.set_property(String::from("__subtree"), JsValue::Bool(false));

    obs.set_property(
        String::from("observe"),
        native_fn("observe", |vm, args| {
            let target = args.first().cloned().unwrap_or(JsValue::Null);
            let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());

            if let JsValue::Object(obj) = &vm.current_this {
                let mut o = obj.borrow_mut();
                o.set(String::from("__observing"), JsValue::Bool(true));
                o.set(String::from("__target"), target);
                o.set(
                    String::from("__childList"),
                    JsValue::Bool(opts.get_property("childList").to_boolean()),
                );
                o.set(
                    String::from("__attributes"),
                    JsValue::Bool(opts.get_property("attributes").to_boolean()),
                );
                o.set(
                    String::from("__characterData"),
                    JsValue::Bool(opts.get_property("characterData").to_boolean()),
                );
                o.set(
                    String::from("__subtree"),
                    JsValue::Bool(opts.get_property("subtree").to_boolean()),
                );
            }

            // Register in bridge so mutations trigger callback.
            let observer = vm.current_this.clone();
            if let Some(bridge) = super::get_bridge(vm) {
                bridge.event_listeners.push(super::EventListener {
                    node_id: usize::MAX - 1, // special sentinel for mutation observers
                    event: String::from("__mutation_observer"),
                    callback: observer,
                    capture: false,
                });
            }
            JsValue::Undefined
        }),
    );

    obs.set_property(
        String::from("disconnect"),
        native_fn("disconnect", |vm, _| {
            if let JsValue::Object(obj) = &vm.current_this {
                obj.borrow_mut()
                    .set(String::from("__observing"), JsValue::Bool(false));
            }
            JsValue::Undefined
        }),
    );

    obs.set_property(
        String::from("takeRecords"),
        native_fn("takeRecords", |vm, _| {
            let records = vm.current_this.get_property("__records");
            // Clear records after taking.
            vm.current_this
                .set_property(String::from("__records"), JsValue::new_array(Vec::new()));
            records
        }),
    );

    obs
}

fn win_post_message(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let data = args.first().cloned().unwrap_or(JsValue::Null);
    let evt = JsValue::new_object();
    evt.set_property(
        String::from("type"),
        JsValue::String(String::from("message")),
    );
    evt.set_property(String::from("data"), data);
    evt.set_property(
        String::from("origin"),
        vm.get_global("location").get_property("origin"),
    );

    if let Some(bridge) = super::get_bridge(vm) {
        let listeners = bridge.event_listeners.clone();
        for listener in listeners
            .iter()
            .filter(|l| l.node_id == usize::MAX && l.event == "message")
        {
            vm.call_value(&listener.callback, &[evt.clone()], JsValue::Undefined);
        }
    }
    JsValue::Undefined
}

fn win_cmp_stub(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(cb) = args.get(1).or_else(|| args.get(2)) {
        if cb.is_function() {
            let payload = JsValue::new_object();
            payload.set_property(String::from("success"), JsValue::Bool(false));
            vm.call_value(cb, &[payload, JsValue::Bool(false)], JsValue::Undefined);
        }
    }
    JsValue::Undefined
}

fn navigator_permissions_query(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let result = JsValue::new_object();
    result.set_property(
        String::from("state"),
        JsValue::String(String::from("denied")),
    );
    result.set_property(String::from("onchange"), JsValue::Null);
    promise_resolve_value(vm, result)
}

fn navigator_storage_estimate(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let result = JsValue::new_object();
    result.set_property(String::from("quota"), JsValue::Number(0.0));
    result.set_property(String::from("usage"), JsValue::Number(0.0));
    promise_resolve_value(vm, result)
}

fn navigator_storage_persist(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Bool(false))
}

fn navigator_storage_persisted(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Bool(false))
}

fn navigator_clipboard_read_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::String(String::new()))
}

fn navigator_clipboard_write_text(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Undefined)
}

fn navigator_send_beacon(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn navigator_share(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Undefined)
}

fn win_tcfapi(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let command = arg_string(args, 0).to_ascii_lowercase();
    let callback = args.get(2).cloned().unwrap_or(JsValue::Undefined);

    let response = JsValue::new_object();
    match command.as_str() {
        "ping" => {
            response.set_property(String::from("cmpLoaded"), JsValue::Bool(false));
            response.set_property(
                String::from("cmpStatus"),
                JsValue::String(String::from("stub")),
            );
            response.set_property(
                String::from("displayStatus"),
                JsValue::String(String::from("hidden")),
            );
            response.set_property(String::from("gdprApplies"), JsValue::Bool(false));
            response.set_property(
                String::from("apiVersion"),
                JsValue::String(String::from("2.0")),
            );
        }
        "addEventListener" => {
            response.set_property(String::from("listenerId"), JsValue::Number(0.0));
            response.set_property(
                String::from("eventStatus"),
                JsValue::String(String::from("tcloaded")),
            );
            response.set_property(
                String::from("cmpStatus"),
                JsValue::String(String::from("stub")),
            );
            response.set_property(String::from("gdprApplies"), JsValue::Bool(false));
        }
        _ => {
            response.set_property(
                String::from("cmpStatus"),
                JsValue::String(String::from("stub")),
            );
            response.set_property(String::from("gdprApplies"), JsValue::Bool(false));
        }
    }

    if callback.is_function() {
        vm.call_value(
            &callback,
            &[response.clone(), JsValue::Bool(true)],
            JsValue::Undefined,
        );
    }
    response
}

fn win_custom_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    evt.set_property(String::from("detail"), opts.get_property("detail"));
    evt
}

fn win_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    make_base_event(&typ, &opts)
}

fn win_mouse_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // MouseEventInit dictionary properties (W3C UIEvents §5.3).
    evt.set_property(
        String::from("clientX"),
        opt_num(opts.get_property("clientX"), 0.0),
    );
    evt.set_property(
        String::from("clientY"),
        opt_num(opts.get_property("clientY"), 0.0),
    );
    evt.set_property(
        String::from("screenX"),
        opt_num(opts.get_property("screenX"), 0.0),
    );
    evt.set_property(
        String::from("screenY"),
        opt_num(opts.get_property("screenY"), 0.0),
    );
    evt.set_property(
        String::from("pageX"),
        opt_num(opts.get_property("pageX"), 0.0),
    );
    evt.set_property(
        String::from("pageY"),
        opt_num(opts.get_property("pageY"), 0.0),
    );
    evt.set_property(
        String::from("offsetX"),
        opt_num(opts.get_property("offsetX"), 0.0),
    );
    evt.set_property(
        String::from("offsetY"),
        opt_num(opts.get_property("offsetY"), 0.0),
    );
    evt.set_property(
        String::from("x"),
        opt_num(opts.get_property("clientX"), 0.0),
    );
    evt.set_property(
        String::from("y"),
        opt_num(opts.get_property("clientY"), 0.0),
    );
    evt.set_property(
        String::from("button"),
        opt_num(opts.get_property("button"), 0.0),
    );
    evt.set_property(
        String::from("buttons"),
        opt_num(opts.get_property("buttons"), 0.0),
    );
    evt.set_property(
        String::from("ctrlKey"),
        JsValue::Bool(opts.get_property("ctrlKey").to_boolean()),
    );
    evt.set_property(
        String::from("shiftKey"),
        JsValue::Bool(opts.get_property("shiftKey").to_boolean()),
    );
    evt.set_property(
        String::from("altKey"),
        JsValue::Bool(opts.get_property("altKey").to_boolean()),
    );
    evt.set_property(
        String::from("metaKey"),
        JsValue::Bool(opts.get_property("metaKey").to_boolean()),
    );
    evt.set_property(String::from("movementX"), JsValue::Number(0.0));
    evt.set_property(String::from("movementY"), JsValue::Number(0.0));
    evt.set_property(String::from("relatedTarget"), JsValue::Null);
    evt
}

fn win_keyboard_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // KeyboardEventInit dictionary (W3C UIEvents §8.2).
    evt.set_property(String::from("key"), opt_str(opts.get_property("key"), ""));
    evt.set_property(String::from("code"), opt_str(opts.get_property("code"), ""));
    evt.set_property(
        String::from("keyCode"),
        opt_num(opts.get_property("keyCode"), 0.0),
    );
    evt.set_property(
        String::from("which"),
        opt_num(opts.get_property("which"), 0.0),
    );
    evt.set_property(
        String::from("charCode"),
        opt_num(opts.get_property("charCode"), 0.0),
    );
    evt.set_property(
        String::from("ctrlKey"),
        JsValue::Bool(opts.get_property("ctrlKey").to_boolean()),
    );
    evt.set_property(
        String::from("shiftKey"),
        JsValue::Bool(opts.get_property("shiftKey").to_boolean()),
    );
    evt.set_property(
        String::from("altKey"),
        JsValue::Bool(opts.get_property("altKey").to_boolean()),
    );
    evt.set_property(
        String::from("metaKey"),
        JsValue::Bool(opts.get_property("metaKey").to_boolean()),
    );
    evt.set_property(
        String::from("repeat"),
        JsValue::Bool(opts.get_property("repeat").to_boolean()),
    );
    evt.set_property(
        String::from("isComposing"),
        JsValue::Bool(opts.get_property("isComposing").to_boolean()),
    );
    evt.set_property(
        String::from("location"),
        opt_num(opts.get_property("location"), 0.0),
    );
    evt.set_property(
        String::from("DOM_KEY_LOCATION_STANDARD"),
        JsValue::Number(0.0),
    );
    evt.set_property(String::from("DOM_KEY_LOCATION_LEFT"), JsValue::Number(1.0));
    evt.set_property(String::from("DOM_KEY_LOCATION_RIGHT"), JsValue::Number(2.0));
    evt.set_property(
        String::from("DOM_KEY_LOCATION_NUMPAD"),
        JsValue::Number(3.0),
    );
    evt.set_property(
        String::from("getModifierState"),
        native_fn("getModifierState", |_, _| JsValue::Bool(false)),
    );
    evt
}

fn win_input_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // InputEventInit (W3C Input Events Level 2 §4.1).
    let data_val = opts.get_property("data");
    evt.set_property(
        String::from("data"),
        if matches!(data_val, JsValue::Null | JsValue::Undefined) {
            JsValue::Null
        } else {
            data_val
        },
    );
    evt.set_property(
        String::from("inputType"),
        opt_str(opts.get_property("inputType"), ""),
    );
    evt.set_property(
        String::from("isComposing"),
        JsValue::Bool(opts.get_property("isComposing").to_boolean()),
    );
    evt.set_property(String::from("dataTransfer"), JsValue::Null);
    evt
}

fn win_focus_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // FocusEventInit (W3C UIEvents §6.2).
    evt.set_property(String::from("relatedTarget"), JsValue::Null);
    let _ = opts; // relatedTarget requires a live element object; not yet supported
    evt
}

fn win_wheel_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // WheelEventInit (W3C UIEvents §7.3) — extends MouseEventInit.
    evt.set_property(
        String::from("deltaX"),
        opt_num(opts.get_property("deltaX"), 0.0),
    );
    evt.set_property(
        String::from("deltaY"),
        opt_num(opts.get_property("deltaY"), 0.0),
    );
    evt.set_property(
        String::from("deltaZ"),
        opt_num(opts.get_property("deltaZ"), 0.0),
    );
    evt.set_property(
        String::from("deltaMode"),
        opt_num(opts.get_property("deltaMode"), 0.0),
    );
    evt.set_property(String::from("DOM_DELTA_PIXEL"), JsValue::Number(0.0));
    evt.set_property(String::from("DOM_DELTA_LINE"), JsValue::Number(1.0));
    evt.set_property(String::from("DOM_DELTA_PAGE"), JsValue::Number(2.0));
    evt.set_property(
        String::from("clientX"),
        opt_num(opts.get_property("clientX"), 0.0),
    );
    evt.set_property(
        String::from("clientY"),
        opt_num(opts.get_property("clientY"), 0.0),
    );
    evt.set_property(
        String::from("button"),
        opt_num(opts.get_property("button"), 0.0),
    );
    evt.set_property(
        String::from("buttons"),
        opt_num(opts.get_property("buttons"), 0.0),
    );
    evt.set_property(
        String::from("ctrlKey"),
        JsValue::Bool(opts.get_property("ctrlKey").to_boolean()),
    );
    evt.set_property(
        String::from("shiftKey"),
        JsValue::Bool(opts.get_property("shiftKey").to_boolean()),
    );
    evt.set_property(
        String::from("altKey"),
        JsValue::Bool(opts.get_property("altKey").to_boolean()),
    );
    evt.set_property(
        String::from("metaKey"),
        JsValue::Bool(opts.get_property("metaKey").to_boolean()),
    );
    evt
}

fn win_pointer_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = make_base_event(&typ, &opts);
    // PointerEventInit (W3C Pointer Events §4.2) — extends MouseEventInit.
    evt.set_property(
        String::from("clientX"),
        opt_num(opts.get_property("clientX"), 0.0),
    );
    evt.set_property(
        String::from("clientY"),
        opt_num(opts.get_property("clientY"), 0.0),
    );
    evt.set_property(
        String::from("screenX"),
        opt_num(opts.get_property("screenX"), 0.0),
    );
    evt.set_property(
        String::from("screenY"),
        opt_num(opts.get_property("screenY"), 0.0),
    );
    evt.set_property(
        String::from("pageX"),
        opt_num(opts.get_property("pageX"), 0.0),
    );
    evt.set_property(
        String::from("pageY"),
        opt_num(opts.get_property("pageY"), 0.0),
    );
    evt.set_property(
        String::from("button"),
        opt_num(opts.get_property("button"), 0.0),
    );
    evt.set_property(
        String::from("buttons"),
        opt_num(opts.get_property("buttons"), 0.0),
    );
    evt.set_property(
        String::from("ctrlKey"),
        JsValue::Bool(opts.get_property("ctrlKey").to_boolean()),
    );
    evt.set_property(
        String::from("shiftKey"),
        JsValue::Bool(opts.get_property("shiftKey").to_boolean()),
    );
    evt.set_property(
        String::from("altKey"),
        JsValue::Bool(opts.get_property("altKey").to_boolean()),
    );
    evt.set_property(
        String::from("metaKey"),
        JsValue::Bool(opts.get_property("metaKey").to_boolean()),
    );
    evt.set_property(
        String::from("pointerId"),
        opt_num(opts.get_property("pointerId"), 1.0),
    );
    evt.set_property(
        String::from("width"),
        opt_num(opts.get_property("width"), 1.0),
    );
    evt.set_property(
        String::from("height"),
        opt_num(opts.get_property("height"), 1.0),
    );
    evt.set_property(
        String::from("pressure"),
        opt_num(opts.get_property("pressure"), 0.0),
    );
    evt.set_property(String::from("tangentialPressure"), JsValue::Number(0.0));
    evt.set_property(
        String::from("tiltX"),
        opt_num(opts.get_property("tiltX"), 0.0),
    );
    evt.set_property(
        String::from("tiltY"),
        opt_num(opts.get_property("tiltY"), 0.0),
    );
    evt.set_property(String::from("twist"), JsValue::Number(0.0));
    evt.set_property(
        String::from("pointerType"),
        opt_str(opts.get_property("pointerType"), "mouse"),
    );
    evt.set_property(
        String::from("isPrimary"),
        JsValue::Bool(opts.get_property("isPrimary").to_boolean()),
    );
    evt.set_property(String::from("relatedTarget"), JsValue::Null);
    evt.set_property(
        String::from("getCoalescedEvents"),
        native_fn("getCoalescedEvents", |_, _| super::make_array(Vec::new())),
    );
    evt.set_property(
        String::from("getPredictedEvents"),
        native_fn("getPredictedEvents", |_, _| super::make_array(Vec::new())),
    );
    evt
}

/// Build the common `Event` interface properties shared by all event types.
///
/// Accepts an `EventInit` dictionary object (`opts`) that may contain
/// `bubbles`, `cancelable`, and `composed`.
fn make_base_event(typ: &str, opts: &JsValue) -> JsValue {
    let bubbles = opts.get_property("bubbles").to_boolean();
    let cancelable = opts.get_property("cancelable").to_boolean();
    let composed = opts.get_property("composed").to_boolean();
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"), JsValue::String(String::from(typ)));
    evt.set_property(String::from("bubbles"), JsValue::Bool(bubbles));
    evt.set_property(String::from("cancelable"), JsValue::Bool(cancelable));
    evt.set_property(String::from("composed"), JsValue::Bool(composed));
    evt.set_property(String::from("isTrusted"), JsValue::Bool(false));
    evt.set_property(String::from("defaultPrevented"), JsValue::Bool(false));
    evt.set_property(String::from("target"), JsValue::Null);
    evt.set_property(String::from("currentTarget"), JsValue::Null);
    evt.set_property(String::from("eventPhase"), JsValue::Number(0.0));
    evt.set_property(String::from("timeStamp"), JsValue::Number(0.0));
    evt.set_property(String::from("NONE"), JsValue::Number(0.0));
    evt.set_property(String::from("CAPTURING_PHASE"), JsValue::Number(1.0));
    evt.set_property(String::from("AT_TARGET"), JsValue::Number(2.0));
    evt.set_property(String::from("BUBBLING_PHASE"), JsValue::Number(3.0));
    evt.set_property(
        String::from("preventDefault"),
        native_fn("preventDefault", super::native_prevent_default),
    );
    evt.set_property(
        String::from("stopPropagation"),
        native_fn("stopPropagation", super::native_stop_propagation),
    );
    evt.set_property(
        String::from("stopImmediatePropagation"),
        native_fn(
            "stopImmediatePropagation",
            super::native_stop_immediate_propagation,
        ),
    );
    evt.set_property(
        String::from("composedPath"),
        native_fn("composedPath", |_, _| super::make_array(Vec::new())),
    );
    evt
}

// ═══════════════════════════════════════════════════════════
// MessageChannel (W3C HTML Living Standard §9.4)
// ═══════════════════════════════════════════════════════════
//
// React Scheduler (react-dom 18+) uses `MessageChannel` to defer work to the
// next macrotask.  The pattern is:
//
//   const channel = new MessageChannel();
//   channel.port1.onmessage = performWork;
//   channel.port2.postMessage(null);   // schedules performWork for next task
//
// We implement this by scheduling a timer (delay=0) in `postMessage`, which
// is picked up by `JsRuntime::tick()` on the next frame.

fn win_message_channel(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let port1 = JsValue::new_object();
    let port2 = JsValue::new_object();

    // Each port stores a reference to the other port so postMessage can
    // read the peer's `onmessage`.
    port1.set_property(String::from("__peer"), port2.clone());
    port2.set_property(String::from("__peer"), port1.clone());

    // Default onmessage = null.
    port1.set_property(String::from("onmessage"), JsValue::Null);
    port2.set_property(String::from("onmessage"), JsValue::Null);

    // postMessage: schedules peer.onmessage via a 0ms timer.
    fn port_post_message(vm: &mut Vm, args: &[JsValue]) -> JsValue {
        // `this` is the port that postMessage was called on.
        let peer = vm.current_this.get_property("__peer");
        let callback = peer.get_property("onmessage");
        if !callback.is_function() {
            return JsValue::Undefined;
        }

        // Build a minimal MessageEvent.
        let msg_evt = JsValue::new_object();
        msg_evt.set_property(
            String::from("data"),
            args.first().cloned().unwrap_or(JsValue::Undefined),
        );
        msg_evt.set_property(
            String::from("type"),
            JsValue::String(String::from("message")),
        );
        msg_evt.set_property(String::from("origin"), JsValue::String(String::new()));
        msg_evt.set_property(String::from("lastEventId"), JsValue::String(String::new()));
        msg_evt.set_property(String::from("source"), JsValue::Null);
        msg_evt.set_property(
            String::from("ports"),
            JsValue::new_array(alloc::vec::Vec::new()),
        );

        // Schedule via timer infrastructure (delay=0 → fires on next tick).
        if let Some(bridge) = super::get_bridge(vm) {
            let id = bridge.next_timer_id;
            bridge.next_timer_id += 1;
            // We wrap: call onmessage(evt) when the timer fires.
            // Since we can't close over msg_evt, we store callback+arg
            // by creating a wrapper native function that calls the peer callback.
            // Simpler: just fire the callback immediately — React only needs
            // the deferral, and our tick() runs on the next frame anyway.
            bridge.timers.push(super::PendingTimer {
                id,
                callback,
                delay_ms: 0,
                repeat: false,
                elapsed_ms: 0,
                is_raf: false,
            });
        }
        JsValue::Undefined
    }

    port1.set_property(
        String::from("postMessage"),
        native_fn("postMessage", port_post_message),
    );
    port2.set_property(
        String::from("postMessage"),
        native_fn("postMessage", port_post_message),
    );

    // start() / close() — no-ops for our implementation.
    port1.set_property(String::from("start"), native_fn("start", win_noop));
    port1.set_property(String::from("close"), native_fn("close", win_noop));
    port2.set_property(String::from("start"), native_fn("start", win_noop));
    port2.set_property(String::from("close"), native_fn("close", win_noop));

    // addEventListener — stores as onmessage for simplicity.
    fn port_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
        let event = super::arg_string(args, 0);
        let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        if event == "message" {
            if let JsValue::Object(obj) = &vm.current_this {
                obj.borrow_mut().set(String::from("onmessage"), callback);
            }
        }
        JsValue::Undefined
    }
    port1.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", port_add_event_listener),
    );
    port2.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", port_add_event_listener),
    );

    let channel = JsValue::new_object();
    channel.set_property(String::from("port1"), port1);
    channel.set_property(String::from("port2"), port2);
    channel
}

fn win_url_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let url = arg_string(args, 0);
    let u = JsValue::new_object();
    u.set_property(String::from("href"), JsValue::String(url.clone()));
    u.set_property(
        String::from("toString"),
        native_fn("toString", |vm, _| {
            if let JsValue::Object(o) = &vm.current_this {
                return o.borrow().get("href");
            }
            JsValue::String(String::new())
        }),
    );
    u
}

fn win_text_encoder(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let enc = JsValue::new_object();
    enc.set_property(
        String::from("encoding"),
        JsValue::String(String::from("utf-8")),
    );
    enc.set_property(
        String::from("encode"),
        native_fn("encode", |_vm, args| {
            let text = super::arg_string(args, 0);
            let bytes = text.as_bytes();
            // Return a Uint8Array-like object with the UTF-8 bytes.
            let elements: Vec<JsValue> = bytes.iter().map(|&b| JsValue::Number(b as f64)).collect();
            let arr = JsValue::new_array(elements);
            // Set byteLength and length for TypedArray compat.
            arr.set_property(
                String::from("byteLength"),
                JsValue::Number(bytes.len() as f64),
            );
            arr
        }),
    );
    enc.set_property(
        String::from("encodeInto"),
        native_fn("encodeInto", |_vm, args| {
            let text = super::arg_string(args, 0);
            let result = JsValue::new_object();
            result.set_property(String::from("read"), JsValue::Number(text.len() as f64));
            result.set_property(String::from("written"), JsValue::Number(text.len() as f64));
            result
        }),
    );
    enc
}

fn win_text_decoder(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let dec = JsValue::new_object();
    dec.set_property(
        String::from("encoding"),
        JsValue::String(String::from("utf-8")),
    );
    dec.set_property(String::from("fatal"), JsValue::Bool(false));
    dec.set_property(String::from("ignoreBOM"), JsValue::Bool(false));
    dec.set_property(
        String::from("decode"),
        native_fn("decode", |_, args| {
            // Accept an array-like of byte values and decode as UTF-8.
            let input = args.first().cloned().unwrap_or(JsValue::Undefined);
            if let JsValue::Array(arr) = &input {
                let elements = &arr.borrow().elements;
                let bytes: Vec<u8> = elements.values().map(|v| v.to_number() as u8).collect();
                let text = String::from_utf8_lossy(&bytes);
                return JsValue::String(String::from(text.as_ref()));
            }
            // Fallback: convert to string directly.
            JsValue::String(input.to_js_string())
        }),
    );
    dec
}

fn win_abort_controller(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let sig = JsValue::new_object();
    sig.set_property(String::from("aborted"), JsValue::Bool(false));
    let ctrl = JsValue::new_object();
    ctrl.set_property(String::from("signal"), sig);
    ctrl.set_property(
        String::from("abort"),
        native_fn("abort", |vm, _| {
            if let JsValue::Object(o) = &vm.current_this {
                let sig = o.borrow().get("signal");
                sig.set_property(String::from("aborted"), JsValue::Bool(true));
            }
            JsValue::Undefined
        }),
    );
    ctrl
}

fn win_queue_microtask(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(callback) = args.first() {
        if callback.is_function() {
            _vm.enqueue_microtask(callback.clone(), alloc::vec::Vec::new());
        }
    }
    JsValue::Undefined
}

fn win_structured_clone(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Simplified: JSON round-trip.
    let json = vm.get_global("JSON");
    let stringify = json.get_property("stringify");
    let parse = json.get_property("parse");
    if let (JsValue::Function(sf), JsValue::Function(pf)) = (&stringify, &parse) {
        let sk = sf.borrow().kind.clone();
        if let libjs::value::FnKind::Native(s_native) = sk {
            let str_val = s_native(vm, args);
            let pk = pf.borrow().kind.clone();
            if let libjs::value::FnKind::Native(p_native) = pk {
                return p_native(vm, &[str_val]);
            }
        }
    }
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

fn win_dom_parser(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let parser = JsValue::new_object();
    parser.set_property(
        String::from("parseFromString"),
        native_fn("parseFromString", |vm, _| vm.get_global("document")),
    );
    parser
}

// ═══════════════════════════════════════════════════════════
// Base64 (W3C HTML §8.3)
// ═══════════════════════════════════════════════════════════

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `atob(data)` — decode a Base64-encoded ASCII string to binary.
fn win_atob(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = arg_string(args, 0);
    let clean: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &ch in &clean {
        let val = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => continue,
            _ => continue,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    let text = String::from_utf8_lossy(&out);
    JsValue::String(String::from(text.as_ref()))
}

/// `btoa(data)` — encode a binary string to Base64 ASCII.
fn win_btoa(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = arg_string(args, 0);
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(B64_CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    JsValue::String(out)
}

// ═══════════════════════════════════════════════════════════
// performance.now() (W3C High Resolution Time §4)
// ═══════════════════════════════════════════════════════════

/// Returns monotonic timestamp in milliseconds.  On anyOS this uses the
/// system tick counter; on host builds it uses a simple incrementing counter
/// so React Scheduler can measure elapsed time between calls.
fn win_performance_now(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(anyos_std::sys::uptime_ms() as f64)
}

fn document_cookie_string(vm: &mut Vm) -> String {
    vm.get_global("document").get_property("cookie").to_js_string()
}

fn parse_cookie_value(cookie_string: &str, name: &str) -> Option<String> {
    for part in cookie_string.split(';') {
        let trimmed = part.trim();
        let mut pieces = trimmed.splitn(2, '=');
        let key = pieces.next().unwrap_or("").trim();
        if key == name {
            return Some(url_decode(pieces.next().unwrap_or("").trim()));
        }
    }
    None
}

fn win_get_cookie(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    if name.is_empty() {
        return JsValue::Null;
    }
    parse_cookie_value(&document_cookie_string(vm), &name)
        .map(JsValue::String)
        .unwrap_or(JsValue::Null)
}

fn extract_query_string(url: &str) -> &str {
    match url.find('?') {
        Some(start) => {
            let query = &url[start + 1..];
            match query.find('#') {
                Some(end) => &query[..end],
                None => query,
            }
        }
        None => url,
    }
}

fn win_get_parameter_by_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    if name.is_empty() {
        return JsValue::Null;
    }
    let url = args
        .get(1)
        .map(|v| v.to_js_string())
        .filter(|s| !s.is_empty() && s != "undefined")
        .unwrap_or_else(|| vm.get_global("location").get_property("href").to_js_string());
    let query = extract_query_string(&url);
    for pair in query.split('&').filter(|s| !s.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        let key = url_decode(parts.next().unwrap_or(""));
        if key == name {
            return JsValue::String(url_decode(parts.next().unwrap_or("")));
        }
    }
    JsValue::Null
}

fn win_get_request_uuid(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    static NEXT_REQUEST_UUID: AtomicU32 = AtomicU32::new(1);
    let seq = NEXT_REQUEST_UUID.fetch_add(1, Ordering::Relaxed);
    let now = anyos_std::sys::uptime_ms();
    JsValue::String(alloc::format!(
        "{:08x}-{:08x}-{:08x}",
        now,
        seq,
        now ^ seq.rotate_left(13)
    ))
}

// ═══════════════════════════════════════════════════════════
// URLSearchParams (W3C URL §5)
// ═══════════════════════════════════════════════════════════

fn win_url_search_params(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let init = arg_string(args, 0);
    let query = if init.starts_with('?') {
        &init[1..]
    } else {
        init.as_str()
    };

    // Parse key=value pairs.
    let entries: Vec<(String, String)> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = url_decode(parts.next().unwrap_or(""));
            let val = url_decode(parts.next().unwrap_or(""));
            (key, val)
        })
        .collect();

    let params = JsValue::new_object();

    // Store entries as a hidden array for iteration.
    let entries_arr: Vec<JsValue> = entries
        .iter()
        .map(|(k, v)| {
            let pair =
                JsValue::new_array(vec![JsValue::String(k.clone()), JsValue::String(v.clone())]);
            pair
        })
        .collect();
    params.set_property(String::from("__entries"), JsValue::new_array(entries_arr));

    params.set_property(
        String::from("get"),
        native_fn("get", |vm, args| {
            let key = super::arg_string(args, 0);
            let entries = vm.current_this.get_property("__entries");
            if let JsValue::Array(arr) = &entries {
                for (_k, e) in &arr.borrow().elements {
                    if let JsValue::Array(pair) = e {
                        let p = pair.borrow();
                        if p.get(0).to_js_string() == key {
                            return p.get(1);
                        }
                    }
                }
            }
            JsValue::Null
        }),
    );

    params.set_property(
        String::from("has"),
        native_fn("has", |vm, args| {
            let key = super::arg_string(args, 0);
            let entries = vm.current_this.get_property("__entries");
            if let JsValue::Array(arr) = &entries {
                for (_k, e) in &arr.borrow().elements {
                    if let JsValue::Array(pair) = e {
                        if pair.borrow().get(0).to_js_string() == key {
                            return JsValue::Bool(true);
                        }
                    }
                }
            }
            JsValue::Bool(false)
        }),
    );

    params.set_property(
        String::from("getAll"),
        native_fn("getAll", |vm, args| {
            let key = super::arg_string(args, 0);
            let entries = vm.current_this.get_property("__entries");
            let mut results = Vec::new();
            if let JsValue::Array(arr) = &entries {
                for (_k, e) in &arr.borrow().elements {
                    if let JsValue::Array(pair) = e {
                        let p = pair.borrow();
                        if p.get(0).to_js_string() == key {
                            results.push(p.get(1));
                        }
                    }
                }
            }
            super::make_array(results)
        }),
    );

    params.set_property(
        String::from("toString"),
        native_fn("toString", |vm, _| {
            let entries = vm.current_this.get_property("__entries");
            let mut out = String::new();
            if let JsValue::Array(arr) = &entries {
                for (i, (_k, e)) in arr.borrow().elements.iter().enumerate() {
                    if i > 0 {
                        out.push('&');
                    }
                    if let JsValue::Array(pair) = e {
                        let p = pair.borrow();
                        out.push_str(&p.get(0).to_js_string());
                        out.push('=');
                        out.push_str(&p.get(1).to_js_string());
                    }
                }
            }
            JsValue::String(out)
        }),
    );

    params.set_property(
        String::from("forEach"),
        native_fn("forEach", |vm, args| {
            let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
            if !cb.is_function() {
                return JsValue::Undefined;
            }
            let entries = vm.current_this.get_property("__entries");
            if let JsValue::Array(arr) = &entries {
                for (_k, e) in arr.borrow().elements.clone() {
                    if let JsValue::Array(pair) = &e {
                        let p = pair.borrow();
                        let val = p.get(1);
                        let key = p.get(0);
                        vm.call_value(&cb, &[val, key], JsValue::Undefined);
                    }
                }
            }
            JsValue::Undefined
        }),
    );

    params.set_property(
        String::from("entries"),
        native_fn("entries", |vm, _| vm.current_this.get_property("__entries")),
    );

    params.set_property(String::from("set"), native_fn("set", win_noop));
    params.set_property(String::from("delete"), native_fn("delete", win_noop));
    params.set_property(String::from("append"), native_fn("append", win_noop));
    params.set_property(String::from("sort"), native_fn("sort", win_noop));

    params
}

/// Simple percent-decoding for URL parameters.
fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
