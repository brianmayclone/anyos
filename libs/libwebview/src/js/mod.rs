//! JavaScript integration for libwebview — native host object approach.
//!
//! All DOM objects (Element, Document, Window) are created as native
//! JsObject instances in Rust, with native function methods — no JS
//! injection.  This mirrors how real browsers (Chromium/Blink, Gecko)
//! expose the DOM to their JavaScript engines.

mod element;
mod classlist;
mod document;
mod window;
mod xhr;
mod fetch;
mod storage;
mod http;
mod selector;
pub mod websocket;

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::{JsEngine, JsValue, Vm};
use libjs::value::JsArray;
use libjs::vm::native_fn;

use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::css::{Declaration, KeyframeSet};
use crate::style::{apply_timing, TimingFunction, TransitionDef};

// ═══════════════════════════════════════════════════════════
// Property write interception — static target for set_hook
// ═══════════════════════════════════════════════════════════

/// Points to the current DomBridge.mutations during JS execution.
/// Set before executing JS, cleared after. Used by dom_property_hook.
static mut MUTATION_TARGET: *mut Vec<DomMutation> = core::ptr::null_mut();

/// Hook called by JsObject::set() on DOM element objects.
/// Records DOM mutations when JS writes to properties like
/// textContent, innerHTML, className, value, etc.
fn dom_property_hook(data: *mut u8, key: &str, value: &JsValue) {
    let mutations = unsafe {
        if MUTATION_TARGET.is_null() { return; }
        &mut *MUTATION_TARGET
    };
    // Decode node_id from pointer (round-trips correctly for negative i64 on 64-bit).
    let node_id = data as usize as i64;

    match key {
        "textContent" | "innerText" => {
            if node_id >= 0 {
                mutations.push(DomMutation::SetTextContent {
                    node_id: node_id as usize,
                    text: value.to_js_string(),
                });
            }
        }
        "innerHTML" => {
            mutations.push(DomMutation::SetInnerHTML {
                node_id,
                html: value.to_js_string(),
            });
        }
        "className" => {
            if node_id >= 0 {
                mutations.push(DomMutation::SetAttribute {
                    node_id: node_id as usize,
                    name: String::from("class"),
                    value: value.to_js_string(),
                });
            }
        }
        "value" | "src" | "href" | "id" | "name" | "type" => {
            if node_id >= 0 {
                mutations.push(DomMutation::SetAttribute {
                    node_id: node_id as usize,
                    name: String::from(key),
                    value: value.to_js_string(),
                });
            }
        }
        "checked" | "disabled" => {
            if node_id >= 0 {
                if value.to_boolean() {
                    mutations.push(DomMutation::SetAttribute {
                        node_id: node_id as usize,
                        name: String::from(key),
                        value: String::new(),
                    });
                } else {
                    mutations.push(DomMutation::RemoveAttribute {
                        node_id: node_id as usize,
                        name: String::from(key),
                    });
                }
            }
        }
        // Ignore internal properties and methods.
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════════
// DomBridge — stored in vm.userdata so native fns can reach the DOM
// ═══════════════════════════════════════════════════════════

struct DomBridge {
    dom: *const Dom,
    mutations: Vec<DomMutation>,
    event_listeners: Vec<EventListener>,
    /// Counter for virtual (createElement'd) node IDs: -1, -2, -3, …
    next_virtual_id: i64,
    /// Virtual nodes created by createElement.
    virtual_nodes: Vec<VirtualNode>,
    /// Pending HTTP requests from XHR / fetch.
    pending_http_requests: Vec<PendingHttpRequest>,
    /// Pending timers (setTimeout / setInterval).
    timers: Vec<PendingTimer>,
    /// Next timer ID.
    next_timer_id: u32,
    /// Set by `stopPropagation()` during event dispatch to halt bubbling.
    propagation_stopped: bool,
    /// Pending WebSocket connect requests from `new WebSocket(url)`.
    pending_ws_connects: Vec<PendingWsConnect>,
    /// Pending WebSocket send requests from `ws.send(data)`.
    pending_ws_sends: Vec<PendingWsSend>,
    /// Pending WebSocket close requests from `ws.close()`.
    pending_ws_closes: Vec<PendingWsClose>,
    /// Live WebSocket objects: (ws_id → JsValue clone) for callback delivery.
    ws_registry: Vec<(u64, JsValue)>,
}

impl DomBridge {
    fn dom(&self) -> &Dom {
        unsafe { &*self.dom }
    }

    fn alloc_virtual_id(&mut self) -> i64 {
        let id = self.next_virtual_id;
        self.next_virtual_id -= 1;
        id
    }

    fn get_virtual(&self, id: i64) -> Option<&VirtualNode> {
        self.virtual_nodes.iter().find(|v| v.id == id)
    }

    fn get_virtual_mut(&mut self, id: i64) -> Option<&mut VirtualNode> {
        self.virtual_nodes.iter_mut().find(|v| v.id == id)
    }
}

/// Retrieve the DomBridge from vm.userdata.
fn get_bridge(vm: &mut Vm) -> Option<&mut DomBridge> {
    let ptr = vm.userdata;
    if ptr.is_null() { return None; }
    unsafe { Some(&mut *(ptr as *mut DomBridge)) }
}

// ═══════════════════════════════════════════════════════════
// Public types
// ═══════════════════════════════════════════════════════════

#[allow(dead_code)]
/// A virtual node created via document.createElement().
struct VirtualNode {
    id: i64,
    tag: String,
    attrs: Vec<(String, String)>,
    text_content: String,
    child_ids: Vec<i64>,
    parent_id: Option<i64>,
}

/// A recorded DOM mutation from JavaScript.
#[derive(Clone)]
pub enum DomMutation {
    SetAttribute { node_id: usize, name: String, value: String },
    SetTextContent { node_id: usize, text: String },
    RemoveAttribute { node_id: usize, name: String },
    CreateElement { virtual_id: i64, tag: String },
    AppendChild { parent_id: i64, child_id: i64 },
    RemoveChild { parent_id: i64, child_id: i64 },
    InsertBefore { parent_id: i64, new_child_id: i64, ref_child_id: i64 },
    ReplaceChild { parent_id: i64, new_child_id: i64, old_child_id: i64 },
    RemoveNode { node_id: i64 },
    SetInnerHTML { node_id: i64, html: String },
    SetStyleProperty { node_id: i64, property: String, value: String },
    /// A `document.cookie = "..."` assignment from JavaScript.
    /// The host application should parse this Set-Cookie string and update its
    /// cookie jar accordingly.
    SetCookie { value: String },
}

/// A pending HTTP request from XMLHttpRequest / fetch.
#[derive(Clone)]
pub struct PendingHttpRequest {
    pub id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// An event listener registered from JavaScript.
#[derive(Clone)]
pub struct EventListener {
    pub node_id: usize,
    pub event: String,
    pub callback: JsValue,
}

/// A `new WebSocket(url)` call from JavaScript — the host must open the
/// TCP connection and perform the HTTP Upgrade handshake.
#[derive(Clone)]
pub struct PendingWsConnect {
    /// Unique identifier for this WebSocket instance.
    pub id: u64,
    /// The `ws://` or `wss://` URL to connect to.
    pub url: String,
    /// Requested sub-protocols (may be empty).
    pub protocols: Vec<String>,
}

/// A `ws.send(data)` call — the host must encode as a WebSocket text frame
/// and write it to the corresponding TCP socket.
#[derive(Clone)]
pub struct PendingWsSend {
    /// WebSocket instance identifier.
    pub id: u64,
    /// Raw payload bytes (UTF-8 for text frames).
    pub data: Vec<u8>,
    /// True for binary frames, false for text.
    pub is_binary: bool,
}

/// A `ws.close(code, reason)` call — the host must send a Close frame and
/// shut down the TCP connection.
#[derive(Clone)]
pub struct PendingWsClose {
    /// WebSocket instance identifier.
    pub id: u64,
    /// Status code (1000 = normal closure).
    pub code: u16,
    /// Optional textual reason.
    pub reason: String,
}

/// A pending timer (setTimeout or setInterval).
#[derive(Clone)]
pub struct PendingTimer {
    pub id: u32,
    pub callback: JsValue,
    pub delay_ms: u64,
    pub repeat: bool,
    /// Accumulated time since creation/last fire.
    pub elapsed_ms: u64,
}

// ═══════════════════════════════════════════════════════════
// JsRuntime — public API
// ═══════════════════════════════════════════════════════════

/// A running CSS `@keyframes` animation for one DOM node.
pub struct ActiveAnimation {
    pub node_id: NodeId,
    /// Name of the `@keyframes` block.
    pub keyframe_name: String,
    pub duration_ms: u32,
    pub timing: TimingFunction,
    pub delay_ms: u32,
    /// 0 = infinite.
    pub iteration_count: u32,
    pub alternate: bool,
    /// Elapsed time since the animation started (after delay).
    pub elapsed_ms: u64,
    /// Current iteration number (0-based).
    pub current_iteration: u32,
}

/// A running CSS transition for one property on one DOM node.
pub struct ActiveTransition {
    pub node_id: NodeId,
    /// CSS property name (e.g. `"opacity"`, `"color"`).
    pub property: String,
    pub duration_ms: u32,
    pub timing: TimingFunction,
    pub delay_ms: u32,
    pub elapsed_ms: u64,
    /// Declarations that represent the *from* state of the property.
    pub from_decl: Option<Declaration>,
    /// Declarations that represent the *to* state of the property.
    pub to_decl: Declaration,
}

pub struct JsRuntime {
    engine: JsEngine,
    pub console: Vec<String>,
    pub mutations: Vec<DomMutation>,
    pub event_listeners: Vec<EventListener>,
    pub pending_http_requests: Vec<PendingHttpRequest>,
    pub timers: Vec<PendingTimer>,
    next_timer_id: u32,
    /// Cookie string for the current page (e.g. `"name=value; n2=v2"`).
    /// Set by the host before calling `execute_scripts`.
    pub cookies: String,
    /// Pending WebSocket connection requests (from `new WebSocket(url)`).
    pub pending_ws_connects: Vec<PendingWsConnect>,
    /// Pending WebSocket send requests (from `ws.send(data)`).
    pub pending_ws_sends: Vec<PendingWsSend>,
    /// Pending WebSocket close requests (from `ws.close()`).
    pub pending_ws_closes: Vec<PendingWsClose>,
    /// Registry of live WebSocket JS objects: (id, JsValue) for callback delivery.
    ws_registry: Vec<(u64, JsValue)>,
    /// Currently running `@keyframes` animations.
    pub active_animations: Vec<ActiveAnimation>,
    /// Currently running CSS transitions.
    pub active_transitions: Vec<ActiveTransition>,
}

impl JsRuntime {
    pub fn new() -> Self {
        let engine = JsEngine::new();
        Self {
            engine,
            console: Vec::new(),
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            pending_http_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            cookies: String::new(),
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
            active_animations: Vec::new(),
            active_transitions: Vec::new(),
        }
    }

    /// Set the cookie string that will be exposed as `document.cookie` during
    /// the next `execute_scripts` call.  The value should be in the same format
    /// as the `Cookie` HTTP request header: `"name=value; name2=value2"`.
    pub fn set_cookies(&mut self, cookies: &str) {
        self.cookies = String::from(cookies);
    }

    /// Execute all `<script>` tags in the DOM.
    ///
    /// * `url` — the current page URL, used to populate `window.location` /
    ///   `document.location` inside the JS environment.
    pub fn execute_scripts(&mut self, dom: &Dom, url: &str) {
        let mut scripts: Vec<String> = Vec::new();
        for i in 0..dom.nodes.len() {
            if let NodeType::Element { tag: Tag::Script, attrs } = &dom.nodes[i].node_type {
                let has_src = attrs.iter().any(|a| a.name == "src");
                if has_src { continue; }
                let type_attr = attrs.iter().find(|a| a.name == "type");
                if let Some(t) = type_attr {
                    let lower = t.value.to_ascii_lowercase();
                    if !lower.is_empty()
                        && lower != "text/javascript"
                        && lower != "application/javascript"
                        && lower != "module"
                    { continue; }
                }
                let text = dom.text_content(i);
                if !text.is_empty() {
                    scripts.push(text);
                }
            }
        }

        if scripts.is_empty() { return; }

        let total_bytes: usize = scripts.iter().map(|s| s.len()).sum();
        anyos_std::println!("[js] {} script(s) found, {} bytes total",
            scripts.len(), total_bytes);

        // Cap large pages: skip very large scripts (>64 KiB) and limit total
        // number of scripts to avoid blocking the UI thread for too long.
        const MAX_SCRIPTS: usize = 16;
        const MAX_SCRIPT_BYTES: usize = 64 * 1024;

        // Lower the per-script step limit to keep pages responsive.
        self.engine.set_step_limit(2_000_000);

        // Set up DOM bridge via userdata.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            next_virtual_id: -1,
            virtual_nodes: Vec::new(),
            pending_http_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            propagation_stopped: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        // Set up native host objects (document, window, etc.).
        self.setup_native_api(dom, url, &self.cookies.clone());

        // Enable property-write interception.
        unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

        // Execute each script (with limits to keep UI responsive).
        let script_count = scripts.len().min(MAX_SCRIPTS);
        for (idx, script) in scripts.iter().take(script_count).enumerate() {
            if script.len() > MAX_SCRIPT_BYTES {
                anyos_std::println!("[js] skipping script #{} ({} bytes — too large)", idx, script.len());
                continue;
            }
            anyos_std::println!("[js] eval #{}: {} bytes", idx, script.len());
            self.engine.eval(script);
        }
        if scripts.len() > script_count {
            anyos_std::println!("[js] skipped {} script(s) (limit={})",
                scripts.len() - script_count, MAX_SCRIPTS);
        }

        // Disable interception.
        unsafe { MUTATION_TARGET = core::ptr::null_mut(); }

        // Capture output.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();

        self.mutations = bridge.mutations;
        self.event_listeners = bridge.event_listeners;
        self.pending_http_requests = bridge.pending_http_requests;
        self.timers.extend(bridge.timers);
        self.pending_ws_connects.extend(bridge.pending_ws_connects);
        self.pending_ws_sends.extend(bridge.pending_ws_sends);
        self.pending_ws_closes.extend(bridge.pending_ws_closes);
        self.ws_registry.extend(bridge.ws_registry);
        self.engine.vm().userdata = core::ptr::null_mut();
        crate::debug_surf!("[js] execute_scripts complete: {} mutations, {} listeners",
            self.mutations.len(), self.event_listeners.len());
    }

    /// Set up all native host objects — zero JS injection.
    ///
    /// * `url`     — current page URL (populates `window.location`).
    /// * `cookies` — cookie string for this domain (populates `document.cookie`).
    fn setup_native_api(&mut self, dom: &Dom, url: &str, cookies: &str) {
        let vm = self.engine.vm();

        // Event callback storage (only tiny bit of eval for array init).
        vm.set_global("__eventCallbacks", JsValue::Array(Rc::new(RefCell::new(JsArray::new()))));

        // Create document object natively.
        let doc = document::make_document(vm, dom, url, cookies);
        vm.set_global("document", doc.clone());

        // Extract origin (scheme + "://" + host) for localStorage key isolation.
        let origin = extract_origin(url);

        // Create window object natively.
        let win = window::make_window(vm, doc, &origin);
        vm.set_global("window", win.clone());
        vm.set_global("self", win.clone());
        vm.set_global("globalThis", win);

        // Top-level constructors/functions from window.
        vm.set_global("alert", native_fn("alert", window::native_alert));
        vm.set_global("fetch", native_fn("fetch", fetch::native_fetch));
        vm.set_global("XMLHttpRequest", xhr::make_xhr_constructor());
        vm.set_global("WebSocket", websocket::make_ws_constructor());
        vm.set_global("Headers", native_fn("Headers", fetch::native_headers_ctor));
        vm.set_global("Image", native_fn("Image", document::native_image_ctor));

        // Timer globals.
        vm.set_global("setTimeout", native_fn("setTimeout", native_set_timeout));
        vm.set_global("setInterval", native_fn("setInterval", native_set_interval));
        vm.set_global("clearTimeout", native_fn("clearTimeout", native_clear_timeout));
        vm.set_global("clearInterval", native_fn("clearInterval", native_clear_interval));
        vm.set_global("requestAnimationFrame", native_fn("requestAnimationFrame", native_request_animation_frame));
        vm.set_global("cancelAnimationFrame", native_fn("cancelAnimationFrame", native_clear_timeout));
    }

    pub fn eval(&mut self, source: &str) -> JsValue {
        let result = self.engine.eval(source);
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        result
    }

    pub fn eval_with_dom(&mut self, source: &str, dom: &Dom) -> JsValue {
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            next_virtual_id: -1,
            virtual_nodes: Vec::new(),
            pending_http_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: self.next_timer_id,
            propagation_stopped: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }
        let result = self.engine.eval(source);
        unsafe { MUTATION_TARGET = core::ptr::null_mut(); }

        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.pending_http_requests.extend(bridge.pending_http_requests);
        self.next_timer_id = bridge.next_timer_id;
        self.timers.extend(bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();
        result
    }

    pub fn get_console(&self) -> &[String] { &self.console }
    pub fn clear_console(&mut self) { self.console.clear(); }

    pub fn take_mutations(&mut self) -> Vec<DomMutation> {
        core::mem::take(&mut self.mutations)
    }

    pub fn take_event_listeners(&mut self) -> Vec<EventListener> {
        core::mem::take(&mut self.event_listeners)
    }

    pub fn take_pending_http_requests(&mut self) -> Vec<PendingHttpRequest> {
        core::mem::take(&mut self.pending_http_requests)
    }

    pub fn take_timers(&mut self) -> Vec<PendingTimer> {
        core::mem::take(&mut self.timers)
    }

    /// Take all pending WebSocket connection requests recorded during script execution.
    pub fn take_ws_connects(&mut self) -> Vec<PendingWsConnect> {
        core::mem::take(&mut self.pending_ws_connects)
    }

    /// Take all pending WebSocket send requests.
    pub fn take_ws_sends(&mut self) -> Vec<PendingWsSend> {
        core::mem::take(&mut self.pending_ws_sends)
    }

    /// Take all pending WebSocket close requests.
    pub fn take_ws_closes(&mut self) -> Vec<PendingWsClose> {
        core::mem::take(&mut self.pending_ws_closes)
    }

    // ── WebSocket callback delivery ──────────────────────────────────────────

    /// Called by the host when a WebSocket connection is established.
    /// Sets `readyState = OPEN` and fires `onopen`.
    pub fn ws_opened(&mut self, id: u64, negotiated_protocol: &str) {
        if let Some(ws_obj) = self.find_ws(id) {
            ws_obj.set_property(String::from("readyState"), JsValue::Number(1.0));
            ws_obj.set_property(
                String::from("protocol"),
                JsValue::String(String::from(negotiated_protocol)),
            );
            let cb = ws_obj.get_property("onopen");
            self.fire_ws_callback(cb, &ws_obj, &[]);
        }
    }

    /// Called by the host when a text message frame is received.
    /// Fires `onmessage` with a MessageEvent-like object.
    pub fn ws_message(&mut self, id: u64, data: &str) {
        if let Some(ws_obj) = self.find_ws(id) {
            let evt = JsValue::new_object();
            evt.set_property(String::from("data"), JsValue::String(String::from(data)));
            evt.set_property(String::from("type"), JsValue::String(String::from("message")));
            evt.set_property(String::from("origin"), JsValue::String(String::new()));
            evt.set_property(String::from("source"), JsValue::Null);
            let cb = ws_obj.get_property("onmessage");
            self.fire_ws_callback(cb, &ws_obj, &[evt]);
        }
    }

    /// Called by the host when a binary frame is received.
    /// Fires `onmessage` with the data represented as a JS string (UTF-8 lossy).
    pub fn ws_message_binary(&mut self, id: u64, data: &[u8]) {
        let text = core::str::from_utf8(data).unwrap_or("[binary]");
        self.ws_message(id, text);
    }

    /// Called by the host when a connection error occurs.
    /// Sets `readyState = CLOSED` and fires `onerror` then `onclose`.
    pub fn ws_error(&mut self, id: u64) {
        if let Some(ws_obj) = self.find_ws(id) {
            ws_obj.set_property(String::from("readyState"), JsValue::Number(3.0));
            let err_cb = ws_obj.get_property("onerror");
            let close_cb = ws_obj.get_property("onclose");
            self.fire_ws_callback(err_cb, &ws_obj, &[]);
            let close_evt = make_close_event(1006, "Abnormal closure", false);
            self.fire_ws_callback(close_cb, &ws_obj, &[close_evt]);
            self.remove_ws(id);
        }
    }

    /// Called by the host when the connection is cleanly closed.
    /// Sets `readyState = CLOSED` and fires `onclose`.
    pub fn ws_closed(&mut self, id: u64, code: u16, reason: &str, clean: bool) {
        if let Some(ws_obj) = self.find_ws(id) {
            ws_obj.set_property(String::from("readyState"), JsValue::Number(3.0));
            let cb = ws_obj.get_property("onclose");
            let close_evt = make_close_event(code, reason, clean);
            self.fire_ws_callback(cb, &ws_obj, &[close_evt]);
            self.remove_ws(id);
        }
    }

    // ── Private WS helpers ───────────────────────────────────────────────────

    /// Find a WebSocket JS object in the registry by ID.
    fn find_ws(&self, id: u64) -> Option<JsValue> {
        self.ws_registry.iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, v)| v.clone())
    }

    /// Remove a closed WebSocket from the registry.
    fn remove_ws(&mut self, id: u64) {
        self.ws_registry.retain(|(wid, _)| *wid != id);
    }

    /// Fire a WS callback (onopen/onmessage/onerror/onclose) through the VM.
    fn fire_ws_callback(&mut self, cb: JsValue, this: &JsValue, args: &[JsValue]) {
        if !matches!(cb, JsValue::Function(_)) { return; }
        self.engine.vm().call_value(&cb, args, this.clone());
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
    }

    /// Apply recorded mutations to the real DOM.
    /// Returns a map from virtual_id → real NodeId for newly created elements.
    pub fn apply_mutations(&mut self, dom: &mut Dom) -> BTreeMap<i64, usize> {
        let mutations = core::mem::take(&mut self.mutations);
        let mut id_map: BTreeMap<i64, usize> = BTreeMap::new();

        for m in &mutations {
            match m {
                DomMutation::CreateElement { virtual_id, tag } => {
                    let real_tag = Tag::from_str(tag);
                    let real_id = dom.add_node(NodeType::Element { tag: real_tag, attrs: Vec::new() }, None);
                    id_map.insert(*virtual_id, real_id);
                }
                DomMutation::SetAttribute { node_id, name, value } => {
                    dom.set_attr(*node_id, name, value);
                }
                DomMutation::RemoveAttribute { node_id, name } => {
                    dom.remove_attr(*node_id, name);
                }
                DomMutation::SetTextContent { node_id, text } => {
                    dom.set_text(*node_id, text);
                }
                DomMutation::AppendChild { parent_id, child_id } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_child = resolve_id(*child_id, &id_map);
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.append_child(p, c);
                    }
                }
                DomMutation::RemoveChild { parent_id, child_id } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_child = resolve_id(*child_id, &id_map);
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.remove_child(p, c);
                    }
                }
                DomMutation::InsertBefore { parent_id, new_child_id, ref_child_id } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_new = resolve_id(*new_child_id, &id_map);
                    let real_ref = resolve_id(*ref_child_id, &id_map);
                    if let (Some(p), Some(n), Some(r)) = (real_parent, real_new, real_ref) {
                        dom.insert_before(p, n, r);
                    }
                }
                DomMutation::ReplaceChild { parent_id, new_child_id, old_child_id } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_new = resolve_id(*new_child_id, &id_map);
                    let real_old = resolve_id(*old_child_id, &id_map);
                    if let (Some(p), Some(n), Some(o)) = (real_parent, real_new, real_old) {
                        dom.remove_child(p, o);
                        dom.append_child(p, n);
                    }
                }
                DomMutation::RemoveNode { node_id } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        // Remove from parent.
                        if let Some(pid) = dom.nodes.get(real_id).and_then(|n| n.parent) {
                            dom.remove_child(pid, real_id);
                        }
                    }
                }
                DomMutation::SetInnerHTML { node_id, html } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        // Remove old children.
                        let children: Vec<usize> = dom.nodes.get(real_id)
                            .map(|n| n.children.clone())
                            .unwrap_or_default();
                        for cid in children {
                            dom.remove_child(real_id, cid);
                        }
                        // Parse HTML fragment and adopt new children.
                        if !html.is_empty() {
                            let fragment = crate::html::parse_fragment(html);
                            dom.adopt_children_from(real_id, &fragment);
                        }
                    }
                }
                DomMutation::SetStyleProperty { node_id, property, value } => {
                    // Store style as a `style` attribute for now.
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        let existing = String::from(dom.attr(real_id, "style")
                            .unwrap_or(""));
                        let new_style = if existing.is_empty() {
                            alloc::format!("{}: {}", property, value)
                        } else {
                            alloc::format!("{}; {}: {}", existing, property, value)
                        };
                        dom.set_attr(real_id, "style", &new_style);
                    }
                }
                DomMutation::SetCookie { .. } => {
                    // Cookie mutations do not modify the DOM tree.
                    // The host application (e.g. surf) reads these via
                    // `take_mutations()` and updates its cookie jar.
                }
            }
        }
        id_map
    }

    /// Dispatch an event to matching listeners, bubbling up the DOM ancestor chain.
    ///
    /// Fires the event at `node_id` first (target phase), then walks up through
    /// parent nodes (bubble phase).  A listener calling `event.stopPropagation()`
    /// halts the walk.
    pub fn dispatch_event(&mut self, dom: &Dom, node_id: usize, event_name: &str) {
        // Build the ancestor chain for bubbling: [target, parent, grandparent, …]
        let ancestors: Vec<usize> = {
            let mut chain = Vec::new();
            let mut cur = Some(node_id);
            while let Some(id) = cur {
                chain.push(id);
                cur = dom.nodes.get(id).and_then(|n| n.parent);
            }
            chain
        };

        // Fast exit: skip if no listener in the entire ancestor chain.
        let has_any = ancestors.iter().any(|&nid|
            self.event_listeners.iter().any(|l| l.node_id == nid && l.event == event_name)
        );
        if !has_any { return; }

        // Create event object.
        let evt = JsValue::new_object();
        evt.set_property(String::from("type"), JsValue::String(String::from(event_name)));
        let target_el = element::make_element(self.engine.vm(), node_id as i64);
        evt.set_property(String::from("target"), target_el.clone());
        evt.set_property(String::from("currentTarget"), target_el);
        evt.set_property(String::from("preventDefault"), native_fn("preventDefault", |_,_| JsValue::Undefined));
        // stopPropagation sets the bridge flag, halting the bubble walk.
        evt.set_property(String::from("stopPropagation"), native_fn("stopPropagation", native_stop_propagation));
        evt.set_property(String::from("stopImmediatePropagation"), native_fn("stopImmediatePropagation", native_stop_propagation));
        evt.set_property(String::from("bubbles"), JsValue::Bool(true));
        evt.set_property(String::from("cancelable"), JsValue::Bool(true));

        // Set up bridge for DOM access during callbacks.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            next_virtual_id: -1,
            virtual_nodes: Vec::new(),
            pending_http_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: self.next_timer_id,
            propagation_stopped: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

        // Fire at target then bubble up.
        'bubble: for &nid in &ancestors {
            // Update currentTarget so listeners can distinguish target vs ancestor.
            let cur_el = element::make_element(self.engine.vm(), nid as i64);
            evt.set_property(String::from("currentTarget"), cur_el);

            let matching: Vec<JsValue> = self.event_listeners.iter()
                .filter(|l| l.node_id == nid && l.event == event_name)
                .map(|l| l.callback.clone())
                .collect();

            for cb in &matching {
                self.engine.vm().call_value(cb, &[evt.clone()], JsValue::Undefined);
                if bridge.propagation_stopped { break 'bubble; }
            }
        }

        unsafe { MUTATION_TARGET = core::ptr::null_mut(); }

        // Capture side effects.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.pending_http_requests.extend(bridge.pending_http_requests);
        self.next_timer_id = bridge.next_timer_id;
        self.timers.extend(bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();
    }

    /// Advance timers by `delta_ms` and execute any that are due.
    /// Returns the number of timers fired.
    pub fn tick(&mut self, dom: &Dom, delta_ms: u64) -> usize {
        // Short-circuit: no allocation or work when there are no timers.
        if self.timers.is_empty() { return 0; }

        let mut fired = 0usize;
        let mut keep = Vec::new();
        let timers = core::mem::take(&mut self.timers);

        for mut t in timers {
            t.elapsed_ms += delta_ms;
            if t.elapsed_ms >= t.delay_ms {
                // Timer is due — execute callback.
                let mut bridge = DomBridge {
                    dom: dom as *const Dom,
                    mutations: Vec::new(),
                    event_listeners: Vec::new(),
                    next_virtual_id: -1,
                    virtual_nodes: Vec::new(),
                    pending_http_requests: Vec::new(),
                    timers: Vec::new(),
                    next_timer_id: self.next_timer_id,
                    propagation_stopped: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
                };
                self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
                unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

                // Timer callbacks get a smaller step budget to keep ticks fast.
                self.engine.set_step_limit(500_000);
                self.engine.vm().call_value(&t.callback, &[], JsValue::Undefined);

                unsafe { MUTATION_TARGET = core::ptr::null_mut(); }
                for msg in self.engine.console_output() {
                    self.console.push(msg.clone());
                }
                self.engine.clear_console();
                self.mutations.extend(bridge.mutations);
                self.event_listeners.extend(bridge.event_listeners);
                self.pending_http_requests.extend(bridge.pending_http_requests);
                self.next_timer_id = bridge.next_timer_id;
                // New timers created during callback.
                keep.extend(bridge.timers);
                self.engine.vm().userdata = core::ptr::null_mut();

                fired += 1;

                if t.repeat {
                    t.elapsed_ms = 0;
                    keep.push(t);
                }
            } else {
                keep.push(t);
            }
        }
        self.timers = keep;
        fired
    }

    pub fn engine(&mut self) -> &mut JsEngine { &mut self.engine }

    /// Register `@keyframes` animation starts for every node whose computed
    /// style requests an animation that is not already running.
    ///
    /// Call this after `execute_scripts()` / relayout when styles change.
    pub fn start_animations(
        &mut self,
        styles: &[crate::style::ComputedStyle],
    ) {
        for (node_id, style) in styles.iter().enumerate() {
            'anim: for adef in &style.animations {
                if adef.name.is_empty() || adef.duration_ms == 0 { continue; }
                // Check if this animation is already running for this node.
                for active in &self.active_animations {
                    if active.node_id == node_id && active.keyframe_name == adef.name {
                        continue 'anim;
                    }
                }
                self.active_animations.push(ActiveAnimation {
                    node_id,
                    keyframe_name: adef.name.clone(),
                    duration_ms: adef.duration_ms,
                    timing: adef.timing,
                    delay_ms: adef.delay_ms,
                    iteration_count: adef.iteration_count,
                    alternate: adef.alternate,
                    elapsed_ms: 0,
                    current_iteration: 0,
                });
            }
        }
    }

    /// Advance all active animations and transitions by `delta_ms`.
    ///
    /// Returns a Vec of `(node_id, Vec<Declaration>)` — style overrides to
    /// apply on top of computed styles before the next relayout.
    /// Returns `true` if any animation is still running (relayout needed).
    pub fn advance_animations(
        &mut self,
        delta_ms: u64,
        keyframe_sets: &[KeyframeSet],
    ) -> (bool, Vec<(NodeId, Vec<Declaration>)>) {
        let mut overrides: Vec<(NodeId, Vec<Declaration>)> = Vec::new();
        let mut any_active = false;

        // ── Keyframe animations ──────────────────────────────────────────────
        let anims = core::mem::take(&mut self.active_animations);
        let mut keep_anims = Vec::new();
        for mut anim in anims {
            // Respect delay.
            if (anim.elapsed_ms as u32) < anim.delay_ms {
                anim.elapsed_ms += delta_ms;
                any_active = true;
                keep_anims.push(anim);
                continue;
            }
            let anim_elapsed = anim.elapsed_ms.saturating_sub(anim.delay_ms as u64) + delta_ms;
            anim.elapsed_ms = anim_elapsed + anim.delay_ms as u64;

            let dur = anim.duration_ms as u64;
            if dur == 0 { continue; }

            // Compute t ∈ [0, 1000] within the current iteration.
            let iter_elapsed = anim_elapsed % dur;
            let t_raw = ((iter_elapsed * 1000) / dur) as i32;
            let t_raw = if anim.alternate && anim.current_iteration % 2 == 1 {
                1000 - t_raw
            } else {
                t_raw
            };
            let t = apply_timing(anim.timing, t_raw).clamp(0, 1000);

            if let Some(kf) = keyframe_sets.iter().find(|k| k.name == anim.keyframe_name) {
                let decls = interpolate_keyframe(kf, t);
                if !decls.is_empty() {
                    overrides.push((anim.node_id, decls));
                }
            }

            let finished = if anim_elapsed >= dur {
                anim.current_iteration += 1;
                anim.iteration_count != 0 && anim.current_iteration >= anim.iteration_count
            } else {
                false
            };

            if !finished {
                any_active = true;
                keep_anims.push(anim);
            }
        }
        self.active_animations = keep_anims;

        // ── CSS transitions ──────────────────────────────────────────────────
        let transitions = core::mem::take(&mut self.active_transitions);
        let mut keep_transitions = Vec::new();
        for mut tr in transitions {
            if tr.duration_ms == 0 { continue; }
            tr.elapsed_ms += delta_ms;
            let elapsed = tr.elapsed_ms.saturating_sub(tr.delay_ms as u64);
            let t_raw = ((elapsed * 1000) / tr.duration_ms as u64).min(1000) as i32;
            let t = apply_timing(tr.timing, t_raw).clamp(0, 1000);

            let decl = interpolate_decl(tr.from_decl.as_ref(), &tr.to_decl, t);
            if let Some(d) = decl {
                overrides.push((tr.node_id, vec![d]));
            }

            if t < 1000 {
                any_active = true;
                keep_transitions.push(tr);
            }
        }
        self.active_transitions = keep_transitions;

        (any_active, overrides)
    }
}

// ═══════════════════════════════════════════════════════════
// Shared helpers (used by sub-modules via super::)
// ═══════════════════════════════════════════════════════════

/// Get __nodeId from vm.current_this.
fn this_node_id(vm: &Vm) -> i64 {
    if let JsValue::Object(obj) = &vm.current_this {
        if let Some(prop) = obj.borrow().properties.get("__nodeId") {
            return prop.value.to_number() as i64;
        }
    }
    -9999
}

/// Get a string argument at given index.
fn arg_string(args: &[JsValue], index: usize) -> String {
    args.get(index).map(|v| v.to_js_string()).unwrap_or_else(String::new)
}

/// Create a JS array from a Vec.
fn make_array(elements: Vec<JsValue>) -> JsValue {
    JsValue::Array(Rc::new(RefCell::new(JsArray::from_vec(elements))))
}

/// Read an attribute from a real DOM node or virtual node.
fn read_attribute(vm: &mut Vm, node_id: i64, name: &str) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return match dom.attr(nid, name) {
                    Some(val) => JsValue::String(String::from(val)),
                    None => JsValue::Null,
                };
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            for (k, v) in &vn.attrs {
                if k == name { return JsValue::String(v.clone()); }
            }
            return JsValue::Null;
        }
    }
    JsValue::Null
}

/// Read the text content of a real or virtual node.
fn read_text_content(vm: &mut Vm, node_id: i64) -> String {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return dom.text_content(nid);
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            return vn.text_content.clone();
        }
    }
    String::new()
}

/// Read the tag name of a real or virtual node.
fn read_tag_name(vm: &mut Vm, node_id: i64) -> String {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return match dom.tag(nid) {
                    Some(tag) => String::from(tag.tag_name()),
                    None => String::from("#text"),
                };
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            return vn.tag.clone();
        }
    }
    String::from("UNKNOWN")
}

/// Read child node IDs.
fn read_child_ids(vm: &mut Vm, node_id: i64) -> Vec<i64> {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return dom.get(nid).children.iter()
                    .filter(|&&cid| matches!(&dom.nodes[cid].node_type, NodeType::Element { .. }))
                    .map(|&cid| cid as i64)
                    .collect();
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            return vn.child_ids.clone();
        }
    }
    Vec::new()
}

#[allow(dead_code)]
/// Read the parent node ID.
fn read_parent_id(vm: &mut Vm, node_id: i64) -> i64 {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return match dom.get(nid).parent {
                    Some(pid) => pid as i64,
                    None => -9999,
                };
            }
        } else if let Some(vn) = bridge.get_virtual(node_id) {
            return vn.parent_id.unwrap_or(-9999);
        }
    }
    -9999
}

/// Read the node type (1 = element, 3 = text).
fn read_node_type(vm: &mut Vm, node_id: i64) -> f64 {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return match &dom.nodes[nid].node_type {
                    NodeType::Element { .. } => 1.0,
                    NodeType::Text(_) => 3.0,
                };
            }
        }
    }
    1.0 // virtual nodes are always elements
}

/// Read innerHTML for a real node.
fn read_inner_html(vm: &mut Vm, node_id: i64) -> String {
    if node_id < 0 { return String::new(); }
    if let Some(bridge) = get_bridge(vm) {
        let dom = bridge.dom();
        let nid = node_id as usize;
        if nid < dom.nodes.len() {
            let mut html = String::new();
            for &cid in &dom.get(nid).children {
                serialize_node(dom, cid, &mut html);
            }
            return html;
        }
    }
    String::new()
}

/// Serialize a DOM node to HTML string.
fn serialize_node(dom: &Dom, node_id: usize, out: &mut String) {
    match &dom.nodes[node_id].node_type {
        NodeType::Text(t) => out.push_str(t),
        NodeType::Element { tag, attrs } => {
            out.push('<');
            let tn = tag.tag_name();
            for b in tn.as_bytes() { out.push((*b | 32) as char); }
            for a in attrs {
                out.push(' ');
                out.push_str(&a.name);
                out.push_str("=\"");
                out.push_str(&a.value);
                out.push('"');
            }
            out.push('>');
            for &cid in &dom.get(node_id).children {
                serialize_node(dom, cid, out);
            }
            out.push_str("</");
            for b in tn.as_bytes() { out.push((*b | 32) as char); }
            out.push('>');
        }
    }
}

#[allow(dead_code)]
/// Escape a string for use in JS string literals.
fn js_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Resolve a (possibly virtual) node ID to a real DOM NodeId.
fn resolve_id(id: i64, map: &BTreeMap<i64, usize>) -> Option<usize> {
    if id >= 0 {
        Some(id as usize)
    } else {
        map.get(&id).copied()
    }
}

// ═══════════════════════════════════════════════════════════
// WebSocket CloseEvent factory
// ═══════════════════════════════════════════════════════════

/// Build a CloseEvent-like JS object for `onclose` callbacks.
fn make_close_event(code: u16, reason: &str, was_clean: bool) -> JsValue {
    let evt = JsValue::new_object();
    evt.set_property(String::from("type"),     JsValue::String(String::from("close")));
    evt.set_property(String::from("code"),     JsValue::Number(code as f64));
    evt.set_property(String::from("reason"),   JsValue::String(String::from(reason)));
    evt.set_property(String::from("wasClean"), JsValue::Bool(was_clean));
    evt
}

// ═══════════════════════════════════════════════════════════
// Native event functions
// ═══════════════════════════════════════════════════════════

/// Native `stopPropagation()` / `stopImmediatePropagation()` handler.
/// Sets `DomBridge.propagation_stopped` so `dispatch_event` halts bubbling.
fn native_stop_propagation(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        bridge.propagation_stopped = true;
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// URL helpers
// ═══════════════════════════════════════════════════════════

/// Extract the origin (`scheme://host[:port]`) from a full URL string.
///
/// Returns an empty string for malformed URLs so the caller can silently
/// skip persistence (the storage still works, just in-memory only).
fn extract_origin(url: &str) -> String {
    // Find "://"
    let after_scheme = match url.find("://") {
        Some(pos) => pos + 3,
        None => return String::new(),
    };
    let scheme = &url[..after_scheme - 3];
    let rest = &url[after_scheme..];
    // Host ends at '/', '?', '#' or end of string.
    let host_end = rest
        .find(|c| c == '/' || c == '?' || c == '#')
        .unwrap_or(rest.len());
    let host = &rest[..host_end];
    let mut origin = String::from(scheme);
    origin.push_str("://");
    origin.push_str(host);
    origin
}

// ═══════════════════════════════════════════════════════════
// Native timer functions
// ═══════════════════════════════════════════════════════════

fn native_set_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args.get(1).map(|v| v.to_number().max(0.0) as u64).unwrap_or(0);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        bridge.timers.push(PendingTimer {
            id,
            callback,
            delay_ms: delay,
            repeat: false,
            elapsed_ms: 0,
        });
        return JsValue::Number(id as f64);
    }
    JsValue::Number(0.0)
}

fn native_set_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args.get(1).map(|v| v.to_number().max(1.0) as u64).unwrap_or(10);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        bridge.timers.push(PendingTimer {
            id,
            callback,
            delay_ms: delay,
            repeat: true,
            elapsed_ms: 0,
        });
        return JsValue::Number(id as f64);
    }
    JsValue::Number(0.0)
}

fn native_clear_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = args.first().map(|v| v.to_number() as u32).unwrap_or(0);
    if let Some(bridge) = get_bridge(vm) {
        bridge.timers.retain(|t| t.id != id);
    }
    JsValue::Undefined
}

fn native_clear_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    native_clear_timeout(vm, args)
}

fn native_request_animation_frame(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Treat as a ~16ms setTimeout (60fps).
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        bridge.timers.push(PendingTimer {
            id,
            callback,
            delay_ms: 16,
            repeat: false,
            elapsed_ms: 0,
        });
        return JsValue::Number(id as f64);
    }
    JsValue::Number(0.0)
}

// ═══════════════════════════════════════════════════════════
// Animation / transition interpolation
// ═══════════════════════════════════════════════════════════

/// Interpolate a complete keyframe set at time `t` (0–1000 fixed-point).
fn interpolate_keyframe(kf: &KeyframeSet, t: i32) -> Vec<crate::css::Declaration> {
    if kf.stops.is_empty() { return Vec::new(); }

    let t_pct = t / 10; // map 0–1000 → 0–100

    // Find the two surrounding stops (stops are sorted by offset 0–100).
    let mut prev_idx = 0usize;
    let mut next_idx = 0usize;
    for (i, stop) in kf.stops.iter().enumerate() {
        if stop.offset <= t_pct { prev_idx = i; }
    }
    next_idx = prev_idx;
    for (i, stop) in kf.stops.iter().enumerate() {
        if stop.offset >= t_pct {
            next_idx = i;
            break;
        }
    }

    let prev = &kf.stops[prev_idx];
    let next = &kf.stops[next_idx];

    if prev_idx == next_idx {
        return prev.declarations.clone();
    }

    // Local t within the segment [prev.offset, next.offset].
    let seg_len = (next.offset - prev.offset).max(1);
    let seg_t = ((t_pct - prev.offset) * 1000 / seg_len).clamp(0, 1000);

    let mut result = Vec::new();
    for next_decl in &next.declarations {
        let from_decl = prev.declarations.iter()
            .find(|d| core::mem::discriminant(&d.property) == core::mem::discriminant(&next_decl.property));
        if let Some(blended) = interpolate_decl(from_decl, next_decl, seg_t) {
            result.push(blended);
        }
    }
    result
}

/// Interpolate one declaration from `from` to `to` at `t` (0–1000).
fn interpolate_decl(
    from: Option<&crate::css::Declaration>,
    to: &crate::css::Declaration,
    t: i32,
) -> Option<crate::css::Declaration> {
    use crate::css::CssValue;

    let from_val = from.map(|d| &d.value);
    let blended = match (&from_val, &to.value) {
        (Some(CssValue::Number(a)), CssValue::Number(b)) => {
            CssValue::Number(lerp_i32(*a, *b, t))
        }
        (Some(CssValue::Length(a, ua)), CssValue::Length(b, ub)) if ua == ub => {
            CssValue::Length(lerp_i32(*a, *b, t), *ub)
        }
        (Some(CssValue::Percentage(a)), CssValue::Percentage(b)) => {
            CssValue::Percentage(lerp_i32(*a, *b, t))
        }
        (Some(CssValue::Color(a)), CssValue::Color(b)) => {
            CssValue::Color(lerp_color(*a, *b, t))
        }
        _ => {
            if t >= 1000 {
                to.value.clone()
            } else if let Some(f) = from_val {
                f.clone()
            } else {
                to.value.clone()
            }
        }
    };

    Some(crate::css::Declaration {
        property: to.property.clone(),
        value: blended,
        important: to.important,
    })
}

/// Linear interpolation for i32 fixed-point values.
#[inline]
fn lerp_i32(a: i32, b: i32, t: i32) -> i32 {
    a + (((b - a) as i64 * t as i64) / 1000) as i32
}

/// Per-channel linear interpolation for packed ARGB colors.
fn lerp_color(a: u32, b: u32, t: i32) -> u32 {
    let la = [(a >> 24) & 0xFF, (a >> 16) & 0xFF, (a >> 8) & 0xFF, a & 0xFF];
    let lb = [(b >> 24) & 0xFF, (b >> 16) & 0xFF, (b >> 8) & 0xFF, b & 0xFF];
    let mut out = 0u32;
    for i in 0..4 {
        let v = lerp_i32(la[i] as i32 * 100, lb[i] as i32 * 100, t) / 100;
        out = (out << 8) | (v.clamp(0, 255) as u32);
    }
    out
}
