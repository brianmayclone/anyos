//! Native window host object.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::{make_array, arg_string};
use super::storage;
use super::xhr;
use super::fetch;

/// Create the native `window` host object.
///
/// * `origin` — the page origin (e.g. `"https://example.com"`) used to key
///   the persistent localStorage file.
pub fn make_window(_vm: &mut Vm, document: JsValue, origin: &str) -> JsValue {
    let mut obj = JsObject::new();

    obj.set(String::from("document"), document.clone());

    // location — share from document.
    let loc = document.get_property("location");
    obj.set(String::from("location"), loc);

    // navigator.
    let nav = JsValue::new_object();
    nav.set_property(String::from("userAgent"), JsValue::String(String::from("anyOS Surf/1.0")));
    nav.set_property(String::from("language"), JsValue::String(String::from("en-US")));
    nav.set_property(String::from("languages"), make_array(vec![JsValue::String(String::from("en-US"))]));
    nav.set_property(String::from("platform"), JsValue::String(String::from("anyOS")));
    nav.set_property(String::from("cookieEnabled"), JsValue::Bool(true));
    nav.set_property(String::from("onLine"), JsValue::Bool(true));
    nav.set_property(String::from("vendor"), JsValue::String(String::from("anyOS")));
    nav.set_property(String::from("appName"), JsValue::String(String::from("Surf")));
    nav.set_property(String::from("appVersion"), JsValue::String(String::from("1.0")));
    obj.set(String::from("navigator"), nav);

    // screen.
    let screen = JsValue::new_object();
    screen.set_property(String::from("width"), JsValue::Number(1024.0));
    screen.set_property(String::from("height"), JsValue::Number(768.0));
    screen.set_property(String::from("availWidth"), JsValue::Number(1024.0));
    screen.set_property(String::from("availHeight"), JsValue::Number(768.0));
    screen.set_property(String::from("colorDepth"), JsValue::Number(32.0));
    screen.set_property(String::from("pixelDepth"), JsValue::Number(32.0));
    let orient = JsValue::new_object();
    orient.set_property(String::from("type"), JsValue::String(String::from("landscape-primary")));
    orient.set_property(String::from("angle"), JsValue::Number(0.0));
    screen.set_property(String::from("orientation"), orient);
    obj.set(String::from("screen"), screen);

    // Dimensions.
    obj.set(String::from("innerWidth"), JsValue::Number(1024.0));
    obj.set(String::from("innerHeight"), JsValue::Number(768.0));
    obj.set(String::from("outerWidth"), JsValue::Number(1024.0));
    obj.set(String::from("outerHeight"), JsValue::Number(768.0));
    obj.set(String::from("devicePixelRatio"), JsValue::Number(1.0));
    obj.set(String::from("pageXOffset"), JsValue::Number(0.0));
    obj.set(String::from("pageYOffset"), JsValue::Number(0.0));
    obj.set(String::from("scrollX"), JsValue::Number(0.0));
    obj.set(String::from("scrollY"), JsValue::Number(0.0));

    // Timer functions (backed by real timer infrastructure in mod.rs).
    obj.set(String::from("alert"), native_fn("alert", native_alert));
    obj.set(String::from("setTimeout"), native_fn("setTimeout", super::native_set_timeout));
    obj.set(String::from("setInterval"), native_fn("setInterval", super::native_set_interval));
    obj.set(String::from("clearTimeout"), native_fn("clearTimeout", super::native_clear_timeout));
    obj.set(String::from("clearInterval"), native_fn("clearInterval", super::native_clear_interval));

    // Style.
    obj.set(String::from("getComputedStyle"), native_fn("getComputedStyle", win_get_computed_style));
    obj.set(String::from("requestAnimationFrame"), native_fn("requestAnimationFrame", super::native_request_animation_frame));
    obj.set(String::from("cancelAnimationFrame"), native_fn("cancelAnimationFrame", super::native_clear_timeout));

    // Events.
    obj.set(String::from("addEventListener"), native_fn("addEventListener", win_add_event_listener));
    obj.set(String::from("removeEventListener"), native_fn("removeEventListener", win_noop));
    obj.set(String::from("dispatchEvent"), native_fn("dispatchEvent", |_,_| JsValue::Bool(true)));

    // Encoding stubs.
    obj.set(String::from("atob"), native_fn("atob", win_passthrough));
    obj.set(String::from("btoa"), native_fn("btoa", win_passthrough));

    // Network.
    obj.set(String::from("fetch"), native_fn("fetch", fetch::native_fetch));
    obj.set(String::from("XMLHttpRequest"), xhr::make_xhr_constructor());
    obj.set(String::from("Headers"), native_fn("Headers", fetch::native_headers_ctor));

    // Performance.
    let perf = JsValue::new_object();
    perf.set_property(String::from("now"), native_fn("now", |_,_| JsValue::Number(0.0)));
    perf.set_property(String::from("mark"), native_fn("mark", win_noop));
    perf.set_property(String::from("measure"), native_fn("measure", win_noop));
    perf.set_property(String::from("getEntriesByName"), native_fn("getEntriesByName", |_,_| make_array(Vec::new())));
    obj.set(String::from("performance"), perf);

    // Storage.
    obj.set(String::from("localStorage"), storage::make_storage(origin, true));
    obj.set(String::from("sessionStorage"), storage::make_storage(origin, false));

    // History.
    let history = JsValue::new_object();
    history.set_property(String::from("length"), JsValue::Number(1.0));
    history.set_property(String::from("state"), JsValue::Null);
    history.set_property(String::from("pushState"), native_fn("pushState", win_noop));
    history.set_property(String::from("replaceState"), native_fn("replaceState", win_noop));
    history.set_property(String::from("back"), native_fn("back", win_noop));
    history.set_property(String::from("forward"), native_fn("forward", win_noop));
    history.set_property(String::from("go"), native_fn("go", win_noop));
    obj.set(String::from("history"), history);

    // Scroll.
    obj.set(String::from("scrollTo"), native_fn("scrollTo", win_noop));
    obj.set(String::from("scrollBy"), native_fn("scrollBy", win_noop));

    // Dialogs.
    obj.set(String::from("open"), native_fn("open", |_,_| JsValue::Null));
    obj.set(String::from("close"), native_fn("close", win_noop));
    obj.set(String::from("print"), native_fn("print", win_noop));
    obj.set(String::from("confirm"), native_fn("confirm", |_,_| JsValue::Bool(false)));
    obj.set(String::from("prompt"), native_fn("prompt", win_prompt));

    // Media queries.
    obj.set(String::from("matchMedia"), native_fn("matchMedia", win_match_media));
    obj.set(String::from("getSelection"), native_fn("getSelection", win_get_selection));

    // Observer stubs.
    obj.set(String::from("ResizeObserver"), native_fn("ResizeObserver", win_observer_ctor));
    obj.set(String::from("MutationObserver"), native_fn("MutationObserver", win_mutation_observer_ctor));
    obj.set(String::from("IntersectionObserver"), native_fn("IntersectionObserver", win_observer_ctor));

    // Event constructors.
    obj.set(String::from("CustomEvent"), native_fn("CustomEvent", win_custom_event));
    obj.set(String::from("Event"), native_fn("Event", win_event));

    // URL / misc.
    obj.set(String::from("URL"), native_fn("URL", win_url_ctor));
    obj.set(String::from("URLSearchParams"), native_fn("URLSearchParams", win_noop_obj));
    obj.set(String::from("TextEncoder"), native_fn("TextEncoder", win_text_encoder));
    obj.set(String::from("TextDecoder"), native_fn("TextDecoder", win_text_decoder));
    obj.set(String::from("AbortController"), native_fn("AbortController", win_abort_controller));
    obj.set(String::from("queueMicrotask"), native_fn("queueMicrotask", win_queue_microtask));
    obj.set(String::from("structuredClone"), native_fn("structuredClone", win_structured_clone));
    obj.set(String::from("DOMParser"), native_fn("DOMParser", win_dom_parser));
    obj.set(String::from("Image"), native_fn("Image", super::document::native_image_ctor));

    // Set document.defaultView = window (after creation).
    let win = JsValue::Object(Rc::new(RefCell::new(obj)));
    document.set_property(String::from("defaultView"), win.clone());
    win
}

// ═══════════════════════════════════════════════════════════
// Window method implementations
// ═══════════════════════════════════════════════════════════

pub fn native_alert(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(msg) = args.first() {
        vm.console_output.push(alloc::format!("[alert] {}", msg.to_js_string()));
    }
    JsValue::Undefined
}

fn win_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = super::arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);

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
        });
    }
    JsValue::Undefined
}

fn win_noop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::Undefined }
fn win_noop_obj(_vm: &mut Vm, _args: &[JsValue]) -> JsValue { JsValue::new_object() }

fn win_passthrough(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.first().cloned().unwrap_or(JsValue::String(String::new()))
}

fn win_get_computed_style(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Return the element's style object.
    if let Some(el) = args.first() {
        return el.get_property("style");
    }
    JsValue::new_object()
}

fn win_prompt(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.get(1).cloned().unwrap_or(JsValue::Null)
}

fn win_match_media(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let q = arg_string(args, 0);
    let mql = JsValue::new_object();
    mql.set_property(String::from("matches"), JsValue::Bool(false));
    mql.set_property(String::from("media"), JsValue::String(q));
    mql.set_property(String::from("addListener"), native_fn("addListener", win_noop));
    mql.set_property(String::from("removeListener"), native_fn("removeListener", win_noop));
    mql.set_property(String::from("addEventListener"), native_fn("addEventListener", win_noop));
    mql.set_property(String::from("removeEventListener"), native_fn("removeEventListener", win_noop));
    mql
}

fn win_get_selection(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let sel = JsValue::new_object();
    sel.set_property(String::from("toString"), native_fn("toString", |_,_| JsValue::String(String::new())));
    sel.set_property(String::from("rangeCount"), JsValue::Number(0.0));
    sel
}

fn win_observer_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let obs = JsValue::new_object();
    obs.set_property(String::from("observe"), native_fn("observe", win_noop));
    obs.set_property(String::from("unobserve"), native_fn("unobserve", win_noop));
    obs.set_property(String::from("disconnect"), native_fn("disconnect", win_noop));
    obs
}

fn win_mutation_observer_ctor(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let obs = JsValue::new_object();
    obs.set_property(String::from("observe"), native_fn("observe", win_noop));
    obs.set_property(String::from("disconnect"), native_fn("disconnect", win_noop));
    obs.set_property(String::from("takeRecords"), native_fn("takeRecords", |_,_| make_array(Vec::new())));
    obs
}

fn win_custom_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"), JsValue::String(typ));
    evt.set_property(String::from("detail"), opts.get_property("detail"));
    evt.set_property(String::from("bubbles"), JsValue::Bool(opts.get_property("bubbles").to_boolean()));
    evt.set_property(String::from("cancelable"), JsValue::Bool(opts.get_property("cancelable").to_boolean()));
    evt.set_property(String::from("target"), JsValue::Null);
    evt.set_property(String::from("currentTarget"), JsValue::Null);
    evt.set_property(String::from("preventDefault"), native_fn("preventDefault", win_noop));
    evt.set_property(String::from("stopPropagation"), native_fn("stopPropagation", win_noop));
    evt
}

fn win_event(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let typ = arg_string(args, 0);
    let opts = args.get(1).cloned().unwrap_or(JsValue::new_object());
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"), JsValue::String(typ));
    evt.set_property(String::from("bubbles"), JsValue::Bool(opts.get_property("bubbles").to_boolean()));
    evt.set_property(String::from("cancelable"), JsValue::Bool(opts.get_property("cancelable").to_boolean()));
    evt.set_property(String::from("target"), JsValue::Null);
    evt.set_property(String::from("currentTarget"), JsValue::Null);
    evt.set_property(String::from("preventDefault"), native_fn("preventDefault", win_noop));
    evt.set_property(String::from("stopPropagation"), native_fn("stopPropagation", win_noop));
    evt
}

fn win_url_ctor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let url = arg_string(args, 0);
    let u = JsValue::new_object();
    u.set_property(String::from("href"), JsValue::String(url.clone()));
    u.set_property(String::from("toString"), native_fn("toString", |vm, _| {
        if let JsValue::Object(o) = &vm.current_this {
            return o.borrow().get("href");
        }
        JsValue::String(String::new())
    }));
    u
}

fn win_text_encoder(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let enc = JsValue::new_object();
    enc.set_property(String::from("encode"), native_fn("encode", win_passthrough));
    enc
}

fn win_text_decoder(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let dec = JsValue::new_object();
    dec.set_property(String::from("decode"), native_fn("decode", |_, args| {
        args.first().map(|v| JsValue::String(v.to_js_string())).unwrap_or(JsValue::String(String::new()))
    }));
    dec
}

fn win_abort_controller(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let sig = JsValue::new_object();
    sig.set_property(String::from("aborted"), JsValue::Bool(false));
    let ctrl = JsValue::new_object();
    ctrl.set_property(String::from("signal"), sig);
    ctrl.set_property(String::from("abort"), native_fn("abort", |vm, _| {
        if let JsValue::Object(o) = &vm.current_this {
            let sig = o.borrow().get("signal");
            sig.set_property(String::from("aborted"), JsValue::Bool(true));
        }
        JsValue::Undefined
    }));
    ctrl
}

fn win_queue_microtask(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Execute immediately (synchronous environment).
    if let Some(JsValue::Function(f)) = args.first() {
        let kind = f.borrow().kind.clone();
        if let libjs::value::FnKind::Native(native) = kind {
            native(_vm, &[]);
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
    parser.set_property(String::from("parseFromString"), native_fn("parseFromString", |vm, _| {
        vm.get_global("document")
    }));
    parser
}
