//! Native window host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use libjs::value::{JsObject, Property};
use libjs::vm::{native_ctor_fn, native_fn, native_symbol};
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
    let ctor = native_ctor_fn(name, native);
    if let JsValue::Function(func) = &ctor {
        let mut proto = JsObject::new();
        proto.prototype = Some(parent_proto.unwrap_or_else(|| vm.object_proto.clone()));
        let proto = Rc::new(RefCell::new(proto));
        proto
            .borrow_mut()
            .set(String::from("constructor"), ctor.clone());
        let mut func = func.borrow_mut();
        func.prototype = Some(proto.clone());
        func.own_props
            .insert(String::from("prototype"), JsValue::Object(proto));
    }
    ctor
}

fn install_event_target_noop_methods(obj: &JsValue) {
    obj.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    obj.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    obj.set_property(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", |_, _| JsValue::Bool(true)),
    );
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
    let node_filter = JsValue::new_object();
    node_filter.set_property(String::from("FILTER_ACCEPT"), JsValue::Number(1.0));
    node_filter.set_property(String::from("FILTER_REJECT"), JsValue::Number(2.0));
    node_filter.set_property(String::from("FILTER_SKIP"), JsValue::Number(3.0));
    node_filter.set_property(
        String::from("SHOW_ALL"),
        JsValue::Number(0xFFFF_FFFFu32 as f64),
    );
    node_filter.set_property(String::from("SHOW_ELEMENT"), JsValue::Number(0x1 as f64));
    node_filter.set_property(String::from("SHOW_TEXT"), JsValue::Number(0x4 as f64));
    node_filter.set_property(String::from("SHOW_COMMENT"), JsValue::Number(0x80 as f64));
    obj.set(String::from("NodeFilter"), node_filter);

    // location — share from document.
    let loc = document.get_property("location");
    obj.set(String::from("location"), loc);

    // navigator. Keep these values aligned with Surf's HTTP User-Agent.  Modern
    // sites compare the network and JS-visible browser identity during feature
    // checks; reporting "anyOS Surf" here while the request uses a Chrome-like
    // UA makes otherwise valid sessions look inconsistent.
    let nav = JsValue::new_object();
    nav.set_property(
        String::from("userAgent"),
        JsValue::String(String::from(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36 Surf/1.0",
        )),
    );
    nav.set_property(
        String::from("language"),
        JsValue::String(String::from("de-DE")),
    );
    nav.set_property(
        String::from("languages"),
        make_array(vec![
            JsValue::String(String::from("de-DE")),
            JsValue::String(String::from("de")),
            JsValue::String(String::from("en-US")),
            JsValue::String(String::from("en")),
        ]),
    );
    nav.set_property(
        String::from("platform"),
        JsValue::String(String::from("Linux x86_64")),
    );
    nav.set_property(
        String::from("product"),
        JsValue::String(String::from("Gecko")),
    );
    nav.set_property(String::from("webdriver"), JsValue::Bool(false));
    nav.set_property(String::from("hardwareConcurrency"), JsValue::Number(8.0));
    nav.set_property(String::from("deviceMemory"), JsValue::Number(8.0));
    nav.set_property(String::from("cookieEnabled"), JsValue::Bool(true));
    nav.set_property(String::from("onLine"), JsValue::Bool(true));
    let connection = JsValue::new_object();
    connection.set_property(
        String::from("effectiveType"),
        JsValue::String(String::from("4g")),
    );
    connection.set_property(String::from("downlink"), JsValue::Number(10.0));
    connection.set_property(String::from("rtt"), JsValue::Number(50.0));
    connection.set_property(String::from("saveData"), JsValue::Bool(false));
    connection.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    connection.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    nav.set_property(String::from("connection"), connection);
    nav.set_property(
        String::from("vendor"),
        JsValue::String(String::from("Google Inc.")),
    );
    nav.set_property(
        String::from("appName"),
        JsValue::String(String::from("Netscape")),
    );
    nav.set_property(
        String::from("appVersion"),
        JsValue::String(String::from(
            "5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124 Safari/537.36 Surf/1.0",
        )),
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
    let service_worker = JsValue::new_object();
    service_worker.set_property(String::from("controller"), JsValue::Null);
    service_worker.set_property(
        String::from("register"),
        native_fn("register", navigator_service_worker_register),
    );
    service_worker.set_property(
        String::from("getRegistration"),
        native_fn("getRegistration", navigator_service_worker_get_registration),
    );
    service_worker.set_property(
        String::from("getRegistrations"),
        native_fn(
            "getRegistrations",
            navigator_service_worker_get_registrations,
        ),
    );
    service_worker.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    service_worker.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    nav.set_property(String::from("serviceWorker"), service_worker);
    obj.set(String::from("navigator"), nav);

    let chrome = JsValue::new_object();
    chrome.set_property(String::from("runtime"), JsValue::new_object());
    obj.set(String::from("chrome"), chrome);

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
    let visual_viewport = JsValue::new_object();
    visual_viewport.set_property(String::from("width"), JsValue::Number(viewport_w as f64));
    visual_viewport.set_property(String::from("height"), JsValue::Number(viewport_h as f64));
    visual_viewport.set_property(String::from("offsetLeft"), JsValue::Number(0.0));
    visual_viewport.set_property(String::from("offsetTop"), JsValue::Number(0.0));
    visual_viewport.set_property(String::from("pageLeft"), JsValue::Number(0.0));
    visual_viewport.set_property(String::from("pageTop"), JsValue::Number(0.0));
    visual_viewport.set_property(String::from("scale"), JsValue::Number(1.0));
    visual_viewport.set_property(String::from("onresize"), JsValue::Null);
    visual_viewport.set_property(String::from("onscroll"), JsValue::Null);
    install_event_target_noop_methods(&visual_viewport);
    obj.set(String::from("visualViewport"), visual_viewport);

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
        String::from("setImmediate"),
        native_fn("setImmediate", super::native_set_immediate),
    );
    obj.set(
        String::from("requestIdleCallback"),
        native_fn("requestIdleCallback", super::native_set_timeout),
    );
    obj.set(
        String::from("clearTimeout"),
        native_fn("clearTimeout", super::native_clear_timeout),
    );
    obj.set(
        String::from("clearInterval"),
        native_fn("clearInterval", super::native_clear_interval),
    );
    obj.set(
        String::from("clearImmediate"),
        native_fn("clearImmediate", super::native_clear_timeout),
    );
    obj.set(
        String::from("cancelIdleCallback"),
        native_fn("cancelIdleCallback", super::native_clear_timeout),
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
    obj.set(
        String::from("__shady_native_addEventListener"),
        native_fn("addEventListener", win_add_event_listener),
    );
    obj.set(
        String::from("__shady_native_removeEventListener"),
        native_fn("removeEventListener", super::native_remove_event_listener),
    );
    obj.set(
        String::from("__shady_native_dispatchEvent"),
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
    obj.set(String::from("Headers"), fetch::make_headers_constructor());
    obj.set(String::from("Request"), fetch::make_request_constructor());
    obj.set(String::from("Response"), fetch::make_response_constructor());

    // Performance (W3C Performance Timeline §4).
    let perf = JsValue::new_object();
    let navigation_entry = make_navigation_timing_entry(viewport_w, viewport_h);
    perf.set_hidden_property(
        String::from("__surfPerformanceEntries"),
        JsValue::new_array(vec![navigation_entry]),
    );
    perf.set_property(String::from("now"), native_fn("now", win_performance_now));
    perf.set_property(
        String::from("mark"),
        native_fn("mark", win_performance_mark),
    );
    perf.set_property(
        String::from("measure"),
        native_fn("measure", win_performance_measure),
    );
    let timing = JsValue::new_object();
    for key in [
        "navigationStart",
        "fetchStart",
        "domainLookupStart",
        "domainLookupEnd",
        "connectStart",
        "connectEnd",
        "requestStart",
        "responseStart",
        "responseEnd",
        "domLoading",
        "domInteractive",
        "domContentLoadedEventStart",
        "domContentLoadedEventEnd",
        "domComplete",
        "loadEventStart",
        "loadEventEnd",
    ] {
        timing.set_property(String::from(key), JsValue::Number(1.0));
    }
    perf.set_property(String::from("timing"), timing);
    let navigation = JsValue::new_object();
    navigation.set_property(String::from("TYPE_NAVIGATE"), JsValue::Number(0.0));
    navigation.set_property(String::from("TYPE_RELOAD"), JsValue::Number(1.0));
    navigation.set_property(String::from("TYPE_BACK_FORWARD"), JsValue::Number(2.0));
    navigation.set_property(String::from("TYPE_RESERVED"), JsValue::Number(255.0));
    navigation.set_property(String::from("type"), JsValue::Number(0.0));
    navigation.set_property(String::from("redirectCount"), JsValue::Number(0.0));
    perf.set_property(String::from("navigation"), navigation);
    perf.set_property(
        String::from("getEntriesByName"),
        native_fn("getEntriesByName", win_performance_get_entries_by_name),
    );
    perf.set_property(
        String::from("getEntriesByType"),
        native_fn("getEntriesByType", win_performance_get_entries_by_type),
    );
    perf.set_property(
        String::from("getEntries"),
        native_fn("getEntries", win_performance_get_entries),
    );
    perf.set_property(
        String::from("clearMarks"),
        native_fn("clearMarks", win_performance_clear_marks),
    );
    perf.set_property(
        String::from("clearMeasures"),
        native_fn("clearMeasures", win_performance_clear_measures),
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
    history.set_property(
        String::from("pushState"),
        native_fn("pushState", history_push_state),
    );
    history.set_property(
        String::from("replaceState"),
        native_fn("replaceState", history_replace_state),
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
    let css = JsValue::new_object();
    css.set_property(
        String::from("supports"),
        native_fn("supports", win_css_supports),
    );
    css.set_property(String::from("escape"), native_fn("escape", win_css_escape));
    css.set_property(
        String::from("registerProperty"),
        native_fn("registerProperty", win_noop),
    );
    obj.set(String::from("CSS"), css);

    // Observer stubs.
    obj.set(
        String::from("ResizeObserver"),
        native_ctor_fn("ResizeObserver", win_resize_observer_ctor),
    );
    obj.set(
        String::from("MutationObserver"),
        native_ctor_fn("MutationObserver", win_mutation_observer_ctor),
    );
    obj.set(
        String::from("IntersectionObserver"),
        native_ctor_fn("IntersectionObserver", win_intersection_observer_ctor),
    );

    // Event constructors (W3C DOM Events Level 3 / UIEvents / Pointer Events).
    let event_target_ctor = make_native_constructor(vm, "EventTarget", win_event_target, None);
    if let JsValue::Function(func) = &event_target_ctor {
        if let Some(proto) = func.borrow().prototype.clone() {
            let proto_val = JsValue::Object(proto);
            let add = native_fn("addEventListener", win_noop);
            let remove = native_fn("removeEventListener", win_noop);
            let dispatch = native_fn("dispatchEvent", |_, _| JsValue::Bool(true));
            proto_val.set_property(String::from("addEventListener"), add.clone());
            proto_val.set_property(String::from("removeEventListener"), remove.clone());
            proto_val.set_property(String::from("dispatchEvent"), dispatch.clone());
            proto_val.set_property(String::from("__shady_native_addEventListener"), add);
            proto_val.set_property(String::from("__shady_native_removeEventListener"), remove);
            proto_val.set_property(String::from("__shady_native_dispatchEvent"), dispatch);
        }
    }
    obj.set(String::from("EventTarget"), event_target_ctor);
    obj.set(String::from("Event"), native_ctor_fn("Event", win_event));
    obj.set(
        String::from("CustomEvent"),
        native_ctor_fn("CustomEvent", win_custom_event),
    );
    obj.set(
        String::from("MouseEvent"),
        native_ctor_fn("MouseEvent", win_mouse_event),
    );
    obj.set(
        String::from("KeyboardEvent"),
        native_ctor_fn("KeyboardEvent", win_keyboard_event),
    );
    obj.set(
        String::from("InputEvent"),
        native_ctor_fn("InputEvent", win_input_event),
    );
    obj.set(
        String::from("FocusEvent"),
        native_ctor_fn("FocusEvent", win_focus_event),
    );
    obj.set(
        String::from("WheelEvent"),
        native_ctor_fn("WheelEvent", win_wheel_event),
    );
    obj.set(
        String::from("PointerEvent"),
        native_ctor_fn("PointerEvent", win_pointer_event),
    );

    // MessageChannel (W3C HTML §9.4 — used by React Scheduler for task deferral).
    obj.set(
        String::from("MessageChannel"),
        native_ctor_fn("MessageChannel", win_message_channel),
    );

    // URL / misc.
    obj.set(String::from("URL"), native_ctor_fn("URL", win_url_ctor));
    obj.set(
        String::from("URLSearchParams"),
        native_ctor_fn("URLSearchParams", win_url_search_params),
    );
    obj.set(
        String::from("TextEncoder"),
        native_ctor_fn("TextEncoder", win_text_encoder),
    );
    obj.set(
        String::from("TextDecoder"),
        native_ctor_fn("TextDecoder", win_text_decoder),
    );
    obj.set(
        String::from("TextEncoderStream"),
        native_ctor_fn("TextEncoderStream", win_text_encoder_stream),
    );
    obj.set(
        String::from("TextDecoderStream"),
        native_ctor_fn("TextDecoderStream", win_text_decoder_stream),
    );
    obj.set(
        String::from("AbortController"),
        native_ctor_fn("AbortController", win_abort_controller),
    );
    let abort_signal_ctor = native_ctor_fn("AbortSignal", win_abort_signal);
    abort_signal_ctor.set_property(
        String::from("abort"),
        native_fn("abort", win_abort_signal_abort),
    );
    abort_signal_ctor.set_property(
        String::from("timeout"),
        native_fn("timeout", win_abort_signal_timeout),
    );
    abort_signal_ctor.set_property(String::from("any"), native_fn("any", win_abort_signal_any));
    obj.set(String::from("AbortSignal"), abort_signal_ctor);
    obj.set(String::from("Blob"), native_ctor_fn("Blob", win_blob));
    let dom_string_map_ctor = make_native_constructor(vm, "DOMStringMap", win_dom_ctor, None);
    obj.set(String::from("DOMStringMap"), dom_string_map_ctor);
    obj.set(
        String::from("ReadableStream"),
        native_ctor_fn("ReadableStream", win_readable_stream),
    );
    obj.set(
        String::from("WritableStream"),
        native_ctor_fn("WritableStream", win_writable_stream),
    );
    obj.set(
        String::from("TransformStream"),
        native_ctor_fn("TransformStream", win_transform_stream),
    );
    obj.set(
        String::from("queueMicrotask"),
        native_fn("queueMicrotask", win_queue_microtask),
    );
    obj.set(
        String::from("structuredClone"),
        native_fn("structuredClone", win_structured_clone),
    );
    obj.set(
        String::from("getCookie"),
        native_fn("getCookie", win_get_cookie),
    );
    obj.set(
        String::from("getParameterByName"),
        native_fn("getParameterByName", win_get_parameter_by_name),
    );
    obj.set(
        String::from("clearEventListeners"),
        native_fn("clearEventListeners", win_noop),
    );
    obj.set(String::from("clarity"), native_fn("clarity", win_noop));
    obj.set(
        String::from("renderClarity"),
        native_fn("renderClarity", win_noop),
    );
    obj.set(
        String::from("getRequestUUID"),
        native_fn("getRequestUUID", win_get_request_uuid),
    );
    obj.set(
        String::from("DOMParser"),
        native_ctor_fn("DOMParser", win_dom_parser),
    );
    let crypto = JsValue::new_object();
    crypto.set_property(
        String::from("randomUUID"),
        native_fn("randomUUID", win_get_request_uuid),
    );
    obj.set(String::from("crypto"), crypto);
    obj.set(
        String::from("Image"),
        native_ctor_fn("Image", super::document::native_image_ctor),
    );
    let node_ctor = make_native_constructor(vm, "Node", win_dom_ctor, None);
    let window_ctor = make_native_constructor(vm, "Window", win_dom_ctor, None);
    let node_proto = match &node_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    if let Some(ref proto_rc) = node_proto {
        let proto_val = JsValue::Object(proto_rc.clone());
        super::element::populate_node_prototype(&proto_val);
    }
    let document_ctor = make_native_constructor(vm, "Document", win_dom_ctor, node_proto.clone());
    let document_proto = match &document_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => None,
    };
    if let Some(ref proto_rc) = document_proto {
        let proto_val = JsValue::Object(proto_rc.clone());
        super::document::populate_document_prototype(&proto_val);
    }
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
    let html_element_proto = match &html_element_ctor {
        JsValue::Function(func) => func.borrow().prototype.clone(),
        _ => element_proto.clone(),
    };
    let node_list_ctor = make_native_constructor(vm, "NodeList", win_dom_ctor, None);
    let html_collection_ctor = make_native_constructor(vm, "HTMLCollection", win_dom_ctor, None);
    let attr_ctor = make_native_constructor(vm, "Attr", win_attr_ctor, node_proto.clone());
    if let JsValue::Function(func) = &attr_ctor {
        if let Some(proto) = func.borrow().prototype.clone() {
            proto.borrow_mut().properties.insert(
                String::from("value"),
                Property::accessor(
                    Some(native_fn("get value", attr_value_get)),
                    Some(native_fn("set value", attr_value_set)),
                ),
            );
        }
    }
    let custom_element_registry_ctor = make_native_constructor(
        vm,
        "CustomElementRegistry",
        win_custom_element_registry_ctor,
        None,
    );
    let custom_elements = win_custom_element_registry_ctor(vm, &[]);

    obj.set(String::from("Node"), node_ctor);
    obj.set(String::from("Window"), window_ctor);
    obj.set(String::from("Document"), document_ctor);
    obj.set(String::from("DocumentFragment"), document_fragment_ctor);
    obj.set(
        String::from("ShadowRoot"),
        make_native_constructor(vm, "ShadowRoot", win_dom_ctor, node_proto.clone()),
    );
    obj.set(String::from("CharacterData"), character_data_ctor);
    obj.set(
        String::from("CDATASection"),
        make_native_constructor(
            vm,
            "CDATASection",
            win_dom_ctor,
            character_data_proto.clone(),
        ),
    );
    obj.set(
        String::from("ProcessingInstruction"),
        make_native_constructor(
            vm,
            "ProcessingInstruction",
            win_dom_ctor,
            character_data_proto.clone(),
        ),
    );
    obj.set(String::from("DocumentType"), document_type_ctor);
    obj.set(String::from("Text"), text_ctor);
    obj.set(String::from("Comment"), comment_ctor);
    obj.set(String::from("Element"), element_ctor);
    obj.set(String::from("HTMLElement"), html_element_ctor);
    obj.set(String::from("NodeList"), node_list_ctor);
    obj.set(String::from("HTMLCollection"), html_collection_ctor);
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLAnchorElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLAreaElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLBodyElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLBRElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLButtonElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLCanvasElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLDivElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLFormElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLHeadElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLHeadingElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLHtmlElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLIFrameElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLImageElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLInputElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLLabelElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLLIElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLLinkElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLMediaElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLMetaElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLAudioElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLVideoElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLSourceElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLPictureElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLOptionElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLParagraphElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLScriptElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLSelectElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLSlotElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLSpanElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLStyleElement", html_element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "HTMLTableElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLTemplateElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLTextAreaElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "HTMLUListElement", html_element_proto.clone());
    install_html_element_constructor(
        vm,
        &mut obj,
        "HTMLUnknownElement",
        html_element_proto.clone(),
    );
    install_html_element_constructor(vm, &mut obj, "SVGElement", element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "SVGSVGElement", element_proto.clone());
    install_html_element_constructor(vm, &mut obj, "SVGGraphicsElement", element_proto.clone());
    obj.set(String::from("Attr"), attr_ctor);
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
    if let JsValue::Object(win_obj) = &win {
        let mut win_obj = win_obj.borrow_mut();
        win_obj.set_hook = Some(super::dom_property_hook);
        win_obj.set_hook_data = usize::MAX as *mut u8;
    }
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
        let window = vm.get_global("window");
        super::call_event_listener(vm, &callback, &JsValue::Undefined, &window);
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

fn history_push_state(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    history_state_update(vm, args, false)
}

fn history_replace_state(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    history_state_update(vm, args, true)
}

fn history_state_update(vm: &mut Vm, args: &[JsValue], replace: bool) -> JsValue {
    let state = args.first().cloned().unwrap_or(JsValue::Null);
    vm.current_this
        .set_property(String::from("state"), state.clone());

    if !replace {
        let len = vm.current_this.get_property("length").to_number();
        vm.current_this
            .set_property(String::from("length"), JsValue::Number((len + 1.0).max(1.0)));
    }

    let Some(url_arg) = args.get(2) else {
        return JsValue::Undefined;
    };
    if url_arg.is_undefined() || url_arg.is_null() {
        return JsValue::Undefined;
    }

    let window = vm.get_global("window");
    let location = window.get_property("location");
    let current_href = location.get_property("href").to_js_string();
    let href = resolve_history_url(&current_href, &url_arg.to_js_string());
    if href.is_empty() {
        return JsValue::Undefined;
    }

    update_location_object(&location, &href);
    let document = vm.get_global("document");
    if !document.is_undefined() {
        document.set_property(String::from("URL"), JsValue::String(href));
    }
    JsValue::Undefined
}

fn update_location_object(location: &JsValue, href: &str) {
    let (protocol, hostname, host, port, pathname, search, hash, origin) =
        super::document::parse_location_fields(href);
    set_location_data(location, "href", JsValue::String(String::from(href)));
    set_location_data(location, "hostname", JsValue::String(hostname));
    set_location_data(location, "host", JsValue::String(host));
    set_location_data(location, "port", JsValue::String(port));
    set_location_data(location, "pathname", JsValue::String(pathname));
    set_location_data(location, "protocol", JsValue::String(protocol));
    set_location_data(location, "search", JsValue::String(search));
    set_location_data(location, "hash", JsValue::String(hash));
    set_location_data(location, "origin", JsValue::String(origin));
}

fn set_location_data(location: &JsValue, key: &str, value: JsValue) {
    if let JsValue::Object(obj) = location {
        let mut borrowed = obj.borrow_mut();
        let hook = borrowed.set_hook.take();
        let hook_data = borrowed.set_hook_data;
        borrowed.set(String::from(key), value);
        borrowed.set_hook = hook;
        borrowed.set_hook_data = hook_data;
    } else {
        location.set_property(String::from(key), value);
    }
}

fn resolve_history_url(current_href: &str, raw_url: &str) -> String {
    let raw = raw_url.trim();
    if raw.is_empty() {
        return String::from(current_href);
    }
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return String::from(raw);
    }

    let (_, _, _, _, pathname, search, _, origin) = super::document::parse_location_fields(current_href);
    if raw.starts_with('/') {
        return alloc::format!("{}{}", origin, raw);
    }
    if raw.starts_with('?') {
        return alloc::format!("{}{}{}", origin, pathname, raw);
    }
    if raw.starts_with('#') {
        return alloc::format!("{}{}{}{}", origin, pathname, search, raw);
    }

    let base_dir = match pathname.rfind('/') {
        Some(pos) => &pathname[..pos + 1],
        None => "/",
    };
    alloc::format!("{}{}{}", origin, base_dir, raw)
}

fn win_noop_obj(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_object()
}
fn win_dom_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::new_object()
}
fn win_event_target(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let target = JsValue::new_object();
    target.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    target.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    target.set_property(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", |_, _| JsValue::Bool(true)),
    );
    target
}

fn install_html_element_constructor(
    vm: &Vm,
    window: &mut JsObject,
    name: &str,
    element_proto: Option<Rc<RefCell<JsObject>>>,
) {
    let ctor = make_native_constructor(vm, name, win_dom_ctor, element_proto);
    if let JsValue::Function(func) = &ctor {
        if let Some(proto) = func.borrow().prototype.clone() {
            let proto_val = JsValue::Object(proto);
            super::element::install_event_handler_accessors_value(&proto_val);
            match name {
                "HTMLAnchorElement" | "HTMLAreaElement" => {
                    super::element::install_reflected_accessors_value(&proto_val, &["href"]);
                }
                "HTMLIFrameElement" => {
                    super::element::install_reflected_accessors_value(
                        &proto_val,
                        &["src", "srcdoc", "contentDocument", "contentWindow"],
                    );
                }
                "HTMLImageElement" => {
                    super::element::install_reflected_accessors_value(&proto_val, &["src"]);
                }
                "HTMLMetaElement" => {
                    super::element::install_reflected_accessors_value(
                        &proto_val,
                        &["content", "httpEquiv"],
                    );
                }
                "HTMLScriptElement" => {
                    super::element::install_reflected_accessors_value(&proto_val, &["src", "text"]);
                }
                _ => {}
            }
            if name == "HTMLAnchorElement" || name == "HTMLAreaElement" {
                proto_val.set_property(String::from("href"), JsValue::String(String::new()));
                proto_val.set_property(String::from("protocol"), JsValue::String(String::new()));
                proto_val.set_property(String::from("host"), JsValue::String(String::new()));
                proto_val.set_property(String::from("hostname"), JsValue::String(String::new()));
                proto_val.set_property(String::from("pathname"), JsValue::String(String::new()));
                proto_val.set_property(String::from("search"), JsValue::String(String::new()));
                proto_val.set_property(String::from("hash"), JsValue::String(String::new()));
            }
            if name == "HTMLImageElement" {
                proto_val.set_property(String::from("complete"), JsValue::Bool(false));
                proto_val.set_property(String::from("naturalWidth"), JsValue::Number(0.0));
                proto_val.set_property(String::from("naturalHeight"), JsValue::Number(0.0));
            }
        }
    }
    window.set(String::from(name), ctor);
}

fn win_attr_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    let value = args
        .get(1)
        .map(|v| v.to_js_string())
        .unwrap_or_else(String::new);
    let attr = JsValue::new_object();
    attr.set_property(String::from("name"), JsValue::String(name.clone()));
    attr.set_property(String::from("nodeName"), JsValue::String(name));
    attr.set_property(String::from("value"), JsValue::String(value.clone()));
    attr.set_property(String::from("nodeValue"), JsValue::String(value));
    attr.set_property(String::from("specified"), JsValue::Bool(true));
    attr.set_property(String::from("ownerElement"), JsValue::Null);
    attr
}

fn attr_value_get(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let value = vm.current_this.get_property("__attr_value");
    if !value.is_undefined() {
        return value;
    }
    vm.current_this.get_property("value")
}

fn attr_value_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let value = args
        .first()
        .cloned()
        .unwrap_or(JsValue::String(String::new()));
    vm.current_this
        .set_property(String::from("__attr_value"), value.clone());
    vm.current_this
        .set_property(String::from("nodeValue"), value.clone());
    JsValue::Undefined
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

fn win_css_supports(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let prop = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let value = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    if prop.is_empty() {
        return JsValue::Bool(false);
    }
    let prop = prop.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    let supported = match prop.as_str() {
        "display" => matches!(
            value.as_str(),
            "block"
                | "inline"
                | "inline-block"
                | "none"
                | "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
        ),
        "position" => matches!(
            value.as_str(),
            "static" | "relative" | "absolute" | "fixed" | "sticky"
        ),
        "animation-timing-function" => {
            value.starts_with("linear(")
                || value.starts_with("cubic-bezier(")
                || matches!(
                    value.as_str(),
                    "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out"
                )
        }
        "color" | "background-color" | "opacity" | "transform" | "filter" => !value.is_empty(),
        _ => false,
    };
    JsValue::Bool(supported)
}

fn win_css_escape(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    if input.is_empty() {
        return JsValue::String(String::new());
    }
    let mut out = String::new();
    for (idx, ch) in input.chars().enumerate() {
        let is_ident = ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() || (ch as u32) >= 0x80;
        if idx == 0 && ch.is_ascii_digit() {
            out.push('\\');
            out.push(ch);
        } else if is_ident {
            out.push(ch);
        } else {
            out.push('\\');
            out.push(ch);
        }
    }
    JsValue::String(out)
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
        } else if inner == "prefers-reduced-motion" {
            return false;
        } else if inner == "prefers-reduced-motion: reduce" {
            return false;
        } else if inner == "prefers-reduced-motion: no-preference" {
            continue;
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

fn win_resize_observer_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let obs = JsValue::new_object();
    obs.set_property(String::from("__callback"), callback);
    obs.set_property(
        String::from("observe"),
        native_fn("observe", win_resize_observer_observe),
    );
    obs.set_property(String::from("unobserve"), native_fn("unobserve", win_noop));
    obs.set_property(
        String::from("disconnect"),
        native_fn("disconnect", win_noop),
    );
    obs
}

fn win_resize_observer_observe(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let callback = vm.current_this.get_property("__callback");
    if callback.is_function() {
        let rect = target.get_property("getBoundingClientRect");
        let content_rect = if rect.is_function() {
            vm.call_value(&rect, &[], target.clone())
        } else {
            make_dom_rect(0.0, 0.0, 0.0, 0.0)
        };
        let entry = JsValue::new_object();
        entry.set_property(String::from("target"), target);
        entry.set_property(String::from("contentRect"), content_rect);
        let entries = make_array(vec![entry]);
        let observer = vm.current_this.clone();
        vm.call_value(&callback, &[entries, observer], JsValue::Undefined);
    }
    JsValue::Undefined
}

fn win_intersection_observer_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let options = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    #[cfg(feature = "host")]
    if std::env::var_os("SURF_DEBUG_OBSERVERS").is_some() {
        eprintln!(
            "[libwebview] IntersectionObserver ctor callback_fn={}",
            callback.is_function()
        );
    }
    let obs = JsValue::new_object();
    obs.set_property(String::from("__callback"), callback);
    obs.set_property(String::from("__options"), options);
    obs.set_property(
        String::from("observe"),
        native_fn("observe", win_intersection_observer_observe),
    );
    obs.set_property(String::from("unobserve"), native_fn("unobserve", win_noop));
    obs.set_property(
        String::from("disconnect"),
        native_fn("disconnect", win_noop),
    );
    obs.set_property(
        String::from("takeRecords"),
        native_fn("takeRecords", |_, _| make_array(Vec::new())),
    );
    obs
}

fn win_intersection_observer_observe(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let callback = vm.current_this.get_property("__callback");
    #[cfg(feature = "host")]
    if std::env::var_os("SURF_DEBUG_OBSERVERS").is_some() {
        eprintln!(
            "[libwebview] IntersectionObserver.observe target_node={} callback_fn={}",
            target.get_property("__nodeId").to_js_string(),
            callback.is_function()
        );
    }
    if callback.is_function() {
        let rect_fn = target.get_property("getBoundingClientRect");
        let rect = if rect_fn.is_function() {
            vm.call_value(&rect_fn, &[], target.clone())
        } else {
            make_dom_rect(0.0, 0.0, 0.0, 0.0)
        };
        let root_bounds = make_dom_rect(
            0.0,
            0.0,
            vm.get_global("innerWidth").to_number(),
            vm.get_global("innerHeight").to_number(),
        );
        let entry = JsValue::new_object();
        entry.set_property(String::from("time"), JsValue::Number(0.0));
        entry.set_property(String::from("target"), target);
        entry.set_property(String::from("rootBounds"), root_bounds);
        entry.set_property(String::from("boundingClientRect"), rect.clone());
        entry.set_property(String::from("intersectionRect"), rect);
        entry.set_property(String::from("isIntersecting"), JsValue::Bool(true));
        entry.set_property(String::from("intersectionRatio"), JsValue::Number(1.0));
        let entries = make_array(vec![entry]);
        let observer = vm.current_this.clone();
        vm.call_value(&callback, &[entries, observer], JsValue::Undefined);
    }
    JsValue::Undefined
}

fn make_dom_rect(x: f64, y: f64, width: f64, height: f64) -> JsValue {
    let rect = JsValue::new_object();
    rect.set_property(String::from("x"), JsValue::Number(x));
    rect.set_property(String::from("y"), JsValue::Number(y));
    rect.set_property(String::from("left"), JsValue::Number(x));
    rect.set_property(String::from("top"), JsValue::Number(y));
    rect.set_property(String::from("width"), JsValue::Number(width));
    rect.set_property(String::from("height"), JsValue::Number(height));
    rect.set_property(String::from("right"), JsValue::Number(x + width));
    rect.set_property(String::from("bottom"), JsValue::Number(y + height));
    rect
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

fn navigator_service_worker_register(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Undefined)
}

fn navigator_service_worker_get_registration(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Undefined)
}

fn navigator_service_worker_get_registrations(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, make_array(Vec::new()))
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
            #[cfg(feature = "host")]
            if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
                eprintln!("[js-dom-debug] MessageChannel postMessage timer id={}", id);
            }
            // We wrap: call onmessage(evt) when the timer fires.
            // Since we can't close over msg_evt, we store callback+arg
            // by creating a wrapper native function that calls the peer callback.
            // Simpler: just fire the callback immediately — React only needs
            // the deferral, and our tick() runs on the next frame anyway.
            super::push_pending_timer(
                &mut bridge.timers,
                super::PendingTimer {
                    id,
                    callback,
                    this_arg: peer,
                    args: alloc::vec![msg_evt],
                    delay_ms: 0,
                    repeat: false,
                    elapsed_ms: 0,
                    is_raf: false,
                },
            );
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

fn stream_resolved_undefined(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    promise_resolve_value(vm, JsValue::Undefined)
}

fn readable_reader_read(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let result = JsValue::new_object();
    result.set_property(String::from("done"), JsValue::Bool(true));
    result.set_property(String::from("value"), JsValue::Undefined);
    promise_resolve_value(vm, result)
}

fn make_readable_stream_controller() -> JsValue {
    let ctrl = JsValue::new_object();
    ctrl.set_property(String::from("desiredSize"), JsValue::Number(1.0));
    ctrl.set_property(String::from("enqueue"), native_fn("enqueue", win_noop));
    ctrl.set_property(String::from("close"), native_fn("close", win_noop));
    ctrl.set_property(String::from("error"), native_fn("error", win_noop));
    ctrl
}

fn make_readable_stream_object() -> JsValue {
    let stream = JsValue::new_object();
    stream.set_property(String::from("locked"), JsValue::Bool(false));
    stream.set_property(
        String::from("getReader"),
        native_fn("getReader", |_vm, _args| {
            let reader = JsValue::new_object();
            reader.set_property(
                String::from("closed"),
                promise_resolve_value(_vm, JsValue::Undefined),
            );
            reader.set_property(
                String::from("read"),
                native_fn("read", readable_reader_read),
            );
            reader.set_property(
                String::from("releaseLock"),
                native_fn("releaseLock", win_noop),
            );
            reader.set_property(
                String::from("cancel"),
                native_fn("cancel", stream_resolved_undefined),
            );
            reader
        }),
    );
    stream.set_property(
        String::from("cancel"),
        native_fn("cancel", stream_resolved_undefined),
    );
    stream.set_property(
        String::from("pipeTo"),
        native_fn("pipeTo", stream_resolved_undefined),
    );
    stream.set_property(
        String::from("pipeThrough"),
        native_fn("pipeThrough", |vm, args| {
            let transform = args.first().cloned().unwrap_or(JsValue::Undefined);
            let readable = transform.get_property("readable");
            if !readable.is_undefined() {
                return readable;
            }
            vm.current_this.clone()
        }),
    );
    stream.set_property(
        String::from("tee"),
        native_fn("tee", |_vm, _args| {
            make_array(vec![
                make_readable_stream_object(),
                make_readable_stream_object(),
            ])
        }),
    );
    stream
}

fn win_readable_stream(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let stream = make_readable_stream_object();
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start = source.get_property("start");
    if start.is_function() {
        let controller = make_readable_stream_controller();
        vm.call_value(&start, &[controller], source);
    }
    stream
}

fn make_writable_stream_object() -> JsValue {
    let stream = JsValue::new_object();
    stream.set_property(String::from("locked"), JsValue::Bool(false));
    stream.set_property(
        String::from("getWriter"),
        native_fn("getWriter", |vm, _args| {
            let writer = JsValue::new_object();
            writer.set_property(
                String::from("closed"),
                promise_resolve_value(vm, JsValue::Undefined),
            );
            writer.set_property(
                String::from("ready"),
                promise_resolve_value(vm, JsValue::Undefined),
            );
            writer.set_property(String::from("desiredSize"), JsValue::Number(1.0));
            writer.set_property(
                String::from("write"),
                native_fn("write", stream_resolved_undefined),
            );
            writer.set_property(
                String::from("close"),
                native_fn("close", stream_resolved_undefined),
            );
            writer.set_property(
                String::from("abort"),
                native_fn("abort", stream_resolved_undefined),
            );
            writer.set_property(
                String::from("releaseLock"),
                native_fn("releaseLock", win_noop),
            );
            writer
        }),
    );
    stream.set_property(
        String::from("abort"),
        native_fn("abort", stream_resolved_undefined),
    );
    stream.set_property(
        String::from("close"),
        native_fn("close", stream_resolved_undefined),
    );
    stream
}

fn win_writable_stream(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let stream = make_writable_stream_object();
    let sink = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start = sink.get_property("start");
    if start.is_function() {
        let controller = JsValue::new_object();
        controller.set_property(String::from("error"), native_fn("error", win_noop));
        vm.call_value(&start, &[controller], sink);
    }
    stream
}

fn win_transform_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let transform = JsValue::new_object();
    transform.set_property(String::from("readable"), make_readable_stream_object());
    transform.set_property(String::from("writable"), make_writable_stream_object());
    transform
}

fn win_url_ctor(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let url = arg_string(args, 0);
    let u = JsValue::new_object();
    let (protocol, host, pathname, search, hash, origin) = parse_url_parts(&url);
    u.set_property(String::from("href"), JsValue::String(url.clone()));
    u.set_property(String::from("protocol"), JsValue::String(protocol));
    u.set_property(String::from("host"), JsValue::String(host.clone()));
    u.set_property(String::from("hostname"), JsValue::String(host));
    u.set_property(String::from("pathname"), JsValue::String(pathname));
    u.set_property(String::from("search"), JsValue::String(search.clone()));
    u.set_property(String::from("hash"), JsValue::String(hash));
    u.set_property(String::from("origin"), JsValue::String(origin));
    u.set_property(
        String::from("searchParams"),
        win_url_search_params(vm, &[JsValue::String(search)]),
    );
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

fn parse_url_parts(url: &str) -> (String, String, String, String, String, String) {
    let (protocol, rest) = if let Some(pos) = url.find("://") {
        (String::from(&url[..pos + 1]), &url[pos + 3..])
    } else {
        (String::new(), url)
    };
    let (without_hash, hash) = if let Some(pos) = rest.find('#') {
        (&rest[..pos], String::from(&rest[pos..]))
    } else {
        (rest, String::new())
    };
    let (without_search, search) = if let Some(pos) = without_hash.find('?') {
        (&without_hash[..pos], String::from(&without_hash[pos..]))
    } else {
        (without_hash, String::new())
    };
    let (host, pathname) = if protocol.is_empty() {
        (String::new(), String::from(without_search))
    } else if let Some(pos) = without_search.find('/') {
        (
            String::from(&without_search[..pos]),
            String::from(&without_search[pos..]),
        )
    } else {
        (String::from(without_search), String::from("/"))
    };
    let origin = if protocol.is_empty() || host.is_empty() {
        String::new()
    } else {
        let mut out = protocol.clone();
        out.push_str("//");
        out.push_str(&host);
        out
    };
    (protocol, host, pathname, search, hash, origin)
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

fn make_text_codec_stream(encoding: &str) -> JsValue {
    let stream = JsValue::new_object();
    stream.set_property(String::from("readable"), make_readable_stream_object());
    stream.set_property(String::from("writable"), make_writable_stream_object());
    stream.set_property(
        String::from("encoding"),
        JsValue::String(String::from(encoding)),
    );
    stream
}

fn win_text_encoder_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_text_codec_stream("utf-8")
}

fn win_text_decoder_stream(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_text_codec_stream("utf-8")
}

fn make_abort_signal(aborted: bool, reason: JsValue) -> JsValue {
    let sig = JsValue::new_object();
    sig.set_property(String::from("aborted"), JsValue::Bool(aborted));
    sig.set_property(String::from("reason"), reason);
    sig.set_property(String::from("onabort"), JsValue::Null);
    sig.set_property(
        String::from("addEventListener"),
        native_fn("addEventListener", win_noop),
    );
    sig.set_property(
        String::from("removeEventListener"),
        native_fn("removeEventListener", win_noop),
    );
    sig.set_property(
        String::from("dispatchEvent"),
        native_fn("dispatchEvent", |_, _| JsValue::Bool(true)),
    );
    sig.set_property(
        String::from("throwIfAborted"),
        native_fn("throwIfAborted", |vm, _| {
            if vm.current_this.get_property("aborted").to_boolean() {
                let err = vm.make_type_error("operation aborted");
                vm.throw_native(err);
                return JsValue::Undefined;
            }
            JsValue::Undefined
        }),
    );
    sig
}

fn win_abort_signal(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_abort_signal(false, JsValue::Undefined)
}

fn win_abort_signal_abort(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let reason = args
        .first()
        .cloned()
        .unwrap_or_else(|| JsValue::String(String::from("AbortError")));
    make_abort_signal(true, reason)
}

fn win_abort_signal_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    make_abort_signal(false, JsValue::Undefined)
}

fn win_abort_signal_any(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Array(arr)) = args.first() {
        for (_idx, signal) in arr.borrow().elements.iter() {
            if signal.get_property("aborted").to_boolean() {
                return make_abort_signal(true, signal.get_property("reason"));
            }
        }
    }
    make_abort_signal(false, JsValue::Undefined)
}

fn win_abort_controller(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let sig = make_abort_signal(false, JsValue::Undefined);
    let ctrl = JsValue::new_object();
    ctrl.set_property(String::from("signal"), sig);
    ctrl.set_property(
        String::from("abort"),
        native_fn("abort", |vm, _| {
            if let JsValue::Object(o) = &vm.current_this {
                let sig = o.borrow().get("signal");
                sig.set_property(String::from("aborted"), JsValue::Bool(true));
                sig.set_property(
                    String::from("reason"),
                    JsValue::String(String::from("AbortError")),
                );
            }
            JsValue::Undefined
        }),
    );
    ctrl
}

fn win_blob(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parts = args.first().cloned().unwrap_or(JsValue::Undefined);
    let opts = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let typ = opts.get_property("type").to_js_string();
    let mut size = 0usize;
    if let JsValue::Array(arr) = &parts {
        for (_idx, part) in arr.borrow().elements.iter() {
            size = size.saturating_add(part.to_js_string().len());
        }
    }
    let blob = JsValue::new_object();
    blob.set_property(String::from("size"), JsValue::Number(size as f64));
    blob.set_property(String::from("type"), JsValue::String(typ));
    blob.set_property(
        String::from("text"),
        native_fn("text", |vm, _| {
            promise_resolve_value(vm, JsValue::String(String::new()))
        }),
    );
    blob.set_property(
        String::from("arrayBuffer"),
        native_fn("arrayBuffer", |vm, _| {
            promise_resolve_value(vm, JsValue::new_array(Vec::new()))
        }),
    );
    blob.set_property(
        String::from("slice"),
        native_fn("slice", |_vm, _| JsValue::new_object()),
    );
    blob
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
    let mut text = String::with_capacity(out.len());
    for byte in out {
        text.push(byte as char);
    }
    JsValue::String(text)
}

/// `btoa(data)` — encode a binary string to Base64 ASCII.
fn win_btoa(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let input = arg_string(args, 0);
    let mut bytes = Vec::with_capacity(input.len());
    for ch in input.chars() {
        let code = ch as u32;
        if code > 0xFF {
            let err = vm.make_type_error("String contains an invalid character");
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        bytes.push(code as u8);
    }
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

fn performance_target(vm: &mut Vm) -> JsValue {
    if !vm
        .current_this
        .get_property("__surfPerformanceEntries")
        .is_undefined()
    {
        return vm.current_this.clone();
    }
    vm.get_global("performance")
}

fn performance_entries(perf: &JsValue) -> JsValue {
    let entries = perf.get_property("__surfPerformanceEntries");
    if !entries.is_undefined() {
        return entries;
    }
    let entries = JsValue::new_array(Vec::new());
    perf.set_hidden_property(String::from("__surfPerformanceEntries"), entries.clone());
    entries
}

fn performance_entry(name: String, entry_type: &str, start_time: f64, duration: f64) -> JsValue {
    let entry = JsValue::new_object();
    entry.set_property(String::from("name"), JsValue::String(name));
    entry.set_property(
        String::from("entryType"),
        JsValue::String(String::from(entry_type)),
    );
    entry.set_property(String::from("startTime"), JsValue::Number(start_time));
    entry.set_property(String::from("duration"), JsValue::Number(duration.max(0.0)));
    entry
}

fn make_navigation_timing_entry(viewport_w: u32, viewport_h: u32) -> JsValue {
    let entry = performance_entry(
        String::from("document"),
        "navigation",
        0.0,
        anyos_std::sys::uptime_ms() as f64,
    );
    entry.set_property(
        String::from("initiatorType"),
        JsValue::String(String::new()),
    );
    entry.set_property(
        String::from("type"),
        JsValue::String(String::from("navigate")),
    );
    entry.set_property(String::from("redirectCount"), JsValue::Number(0.0));
    entry.set_property(String::from("workerStart"), JsValue::Number(0.0));
    entry.set_property(String::from("fetchStart"), JsValue::Number(0.0));
    entry.set_property(String::from("domainLookupStart"), JsValue::Number(0.0));
    entry.set_property(String::from("domainLookupEnd"), JsValue::Number(0.0));
    entry.set_property(String::from("connectStart"), JsValue::Number(0.0));
    entry.set_property(String::from("connectEnd"), JsValue::Number(0.0));
    entry.set_property(String::from("secureConnectionStart"), JsValue::Number(0.0));
    entry.set_property(String::from("requestStart"), JsValue::Number(0.0));
    entry.set_property(String::from("responseStart"), JsValue::Number(1.0));
    entry.set_property(String::from("responseEnd"), JsValue::Number(1.0));
    entry.set_property(String::from("domInteractive"), JsValue::Number(1.0));
    entry.set_property(
        String::from("domContentLoadedEventStart"),
        JsValue::Number(1.0),
    );
    entry.set_property(
        String::from("domContentLoadedEventEnd"),
        JsValue::Number(1.0),
    );
    entry.set_property(String::from("domComplete"), JsValue::Number(1.0));
    entry.set_property(String::from("loadEventStart"), JsValue::Number(1.0));
    entry.set_property(String::from("loadEventEnd"), JsValue::Number(1.0));
    entry.set_property(String::from("transferSize"), JsValue::Number(0.0));
    entry.set_property(String::from("encodedBodySize"), JsValue::Number(0.0));
    entry.set_property(String::from("decodedBodySize"), JsValue::Number(0.0));
    entry.set_property(
        String::from("nextHopProtocol"),
        JsValue::String(String::from("h2")),
    );
    entry.set_property(
        String::from("renderBlockingStatus"),
        JsValue::String(String::from("blocking")),
    );
    entry.set_property(
        String::from("__viewportWidth"),
        JsValue::Number(viewport_w as f64),
    );
    entry.set_property(
        String::from("__viewportHeight"),
        JsValue::Number(viewport_h as f64),
    );
    entry
}

fn push_performance_entry(perf: &JsValue, entry: JsValue) {
    let entries = performance_entries(perf);
    if let JsValue::Array(arr) = entries {
        arr.borrow_mut().push(entry);
    }
}

fn latest_performance_mark(perf: &JsValue, name: &str) -> Option<f64> {
    let entries = performance_entries(perf);
    let JsValue::Array(arr) = entries else {
        return None;
    };
    let arr = arr.borrow();
    for index in (0..arr.len()).rev() {
        let entry = arr.get(index);
        if entry.get_property("entryType").to_js_string() == "mark"
            && entry.get_property("name").to_js_string() == name
        {
            return Some(entry.get_property("startTime").to_number());
        }
    }
    None
}

fn filtered_performance_entries(
    perf: &JsValue,
    name_filter: Option<&str>,
    type_filter: Option<&str>,
) -> JsValue {
    let entries = performance_entries(perf);
    let mut out = Vec::new();
    if let JsValue::Array(arr) = entries {
        let arr = arr.borrow();
        for index in 0..arr.len() {
            let entry = arr.get(index);
            let name_matches = name_filter
                .map(|name| entry.get_property("name").to_js_string() == name)
                .unwrap_or(true);
            let type_matches = type_filter
                .map(|entry_type| entry.get_property("entryType").to_js_string() == entry_type)
                .unwrap_or(true);
            if name_matches && type_matches {
                out.push(entry);
            }
        }
    }
    make_array(out)
}

fn clear_performance_entries(perf: &JsValue, entry_type: &str, name_filter: Option<&str>) {
    let entries = performance_entries(perf);
    let JsValue::Array(arr) = entries else {
        return;
    };
    let mut keep = Vec::new();
    {
        let arr = arr.borrow();
        for index in 0..arr.len() {
            let entry = arr.get(index);
            let is_target_type = entry.get_property("entryType").to_js_string() == entry_type;
            let is_target_name = name_filter
                .map(|name| entry.get_property("name").to_js_string() == name)
                .unwrap_or(true);
            if !(is_target_type && is_target_name) {
                keep.push(entry);
            }
        }
    }
    *arr.borrow_mut() = libjs::value::JsArray::from_vec(keep);
}

fn win_performance_mark(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|v| v.to_js_string())
        .unwrap_or_else(|| String::from(""));
    let perf = performance_target(vm);
    let now = win_performance_now(vm, &[]).to_number();
    let entry = performance_entry(name, "mark", now, 0.0);
    push_performance_entry(&perf, entry.clone());
    entry
}

fn win_performance_measure(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .map(|v| v.to_js_string())
        .unwrap_or_else(|| String::from(""));
    let perf = performance_target(vm);
    let now = win_performance_now(vm, &[]).to_number();
    let start = args
        .get(1)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| latest_performance_mark(&perf, &v.to_js_string()))
        .unwrap_or(0.0);
    let end = args
        .get(2)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .and_then(|v| latest_performance_mark(&perf, &v.to_js_string()))
        .unwrap_or(now);
    let entry = performance_entry(name, "measure", start, (end - start).max(0.0));
    push_performance_entry(&perf, entry.clone());
    entry
}

fn win_performance_get_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let perf = performance_target(vm);
    filtered_performance_entries(&perf, None, None)
}

fn win_performance_get_entries_by_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = arg_string(args, 0);
    let entry_type = args
        .get(1)
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_js_string());
    let perf = performance_target(vm);
    filtered_performance_entries(&perf, Some(&name), entry_type.as_deref())
}

fn win_performance_get_entries_by_type(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let entry_type = arg_string(args, 0);
    let perf = performance_target(vm);
    filtered_performance_entries(&perf, None, Some(&entry_type))
}

fn win_performance_clear_marks(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_js_string());
    let perf = performance_target(vm);
    clear_performance_entries(&perf, "mark", name.as_deref());
    JsValue::Undefined
}

fn win_performance_clear_measures(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args
        .first()
        .filter(|v| !v.is_undefined() && !v.is_null())
        .map(|v| v.to_js_string());
    let perf = performance_target(vm);
    clear_performance_entries(&perf, "measure", name.as_deref());
    JsValue::Undefined
}

fn document_cookie_string(vm: &mut Vm) -> String {
    vm.get_global("document")
        .get_property("cookie")
        .to_js_string()
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
        .unwrap_or_else(|| {
            vm.get_global("location")
                .get_property("href")
                .to_js_string()
        });
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
    params.set_hidden_property(String::from("__entries"), JsValue::new_array(entries_arr));

    params.set_hidden_property(
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

    params.set_hidden_property(
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

    params.set_hidden_property(
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

    params.set_hidden_property(
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

    params.set_hidden_property(
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

    params.set_hidden_property(
        String::from("entries"),
        native_fn("entries", |vm, _| {
            let entries = vm.current_this.get_property("__entries");
            if let JsValue::Array(arr) = entries {
                return vm.make_internal_iterator(arr.borrow().to_dense_vec());
            }
            vm.make_internal_iterator(Vec::new())
        }),
    );
    params.set_hidden_property(
        String::from(native_symbol::WELL_KNOWN_ITERATOR),
        native_fn("[Symbol.iterator]", |vm, _| {
            let entries = vm.current_this.get_property("__entries");
            if let JsValue::Array(arr) = entries {
                return vm.make_internal_iterator(arr.borrow().to_dense_vec());
            }
            vm.make_internal_iterator(Vec::new())
        }),
    );

    params.set_hidden_property(String::from("set"), native_fn("set", win_noop));
    params.set_hidden_property(String::from("delete"), native_fn("delete", win_noop));
    params.set_hidden_property(String::from("append"), native_fn("append", win_noop));
    params.set_hidden_property(String::from("sort"), native_fn("sort", win_noop));

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
