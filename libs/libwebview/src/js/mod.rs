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

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::{JsEngine, JsValue, Vm};
use libjs::value::JsArray;
use libjs::vm::native_fn;

use crate::dom::{Dom, NodeType, Tag};

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

pub struct JsRuntime {
    engine: JsEngine,
    pub console: Vec<String>,
    pub mutations: Vec<DomMutation>,
    pub event_listeners: Vec<EventListener>,
    pub pending_http_requests: Vec<PendingHttpRequest>,
    pub timers: Vec<PendingTimer>,
    next_timer_id: u32,
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
        }
    }

    /// Execute all `<script>` tags in the DOM.
    pub fn execute_scripts(&mut self, dom: &Dom) {
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

        crate::debug_surf!("[js] execute_scripts: {} script(s) found", scripts.len());
        if scripts.is_empty() { return; }

        #[cfg(feature = "debug_surf")]
        {
            let total_bytes: usize = scripts.iter().map(|s| s.len()).sum();
            crate::debug_surf!("[js] total script bytes: {}", total_bytes);
            crate::debug_surf!("[js]   RSP=0x{:X} heap=0x{:X}", crate::debug_rsp(), crate::debug_heap_pos());
        }

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
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        // Set up native host objects (document, window, etc.).
        self.setup_native_api(dom);

        // Enable property-write interception.
        unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

        // Execute each script.
        for (idx, script) in scripts.iter().enumerate() {
            crate::debug_surf!("[js] eval script #{}: {} bytes", idx, script.len());
            self.engine.eval(script);
            crate::debug_surf!("[js] eval script #{} done", idx);
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
        self.engine.vm().userdata = core::ptr::null_mut();
        crate::debug_surf!("[js] execute_scripts complete: {} mutations, {} listeners",
            self.mutations.len(), self.event_listeners.len());
    }

    /// Set up all native host objects — zero JS injection.
    fn setup_native_api(&mut self, dom: &Dom) {
        let vm = self.engine.vm();

        // Event callback storage (only tiny bit of eval for array init).
        vm.set_global("__eventCallbacks", JsValue::Array(Rc::new(RefCell::new(JsArray::new()))));

        // Create document object natively.
        let doc = document::make_document(vm, dom);
        vm.set_global("document", doc.clone());

        // Create window object natively.
        let win = window::make_window(vm, doc);
        vm.set_global("window", win.clone());
        vm.set_global("self", win.clone());
        vm.set_global("globalThis", win);

        // Top-level constructors/functions from window.
        vm.set_global("alert", native_fn("alert", window::native_alert));
        vm.set_global("fetch", native_fn("fetch", fetch::native_fetch));
        vm.set_global("XMLHttpRequest", xhr::make_xhr_constructor());
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
            }
        }
        id_map
    }

    /// Dispatch an event to all matching listeners.
    /// Calls the JS callback functions via the VM.
    pub fn dispatch_event(&mut self, dom: &Dom, node_id: usize, event_name: &str) {
        // Find matching listeners.
        let matching: Vec<JsValue> = self.event_listeners.iter()
            .filter(|l| l.node_id == node_id && l.event == event_name)
            .map(|l| l.callback.clone())
            .collect();

        if matching.is_empty() { return; }

        // Create event object.
        let evt = JsValue::new_object();
        evt.set_property(String::from("type"), JsValue::String(String::from(event_name)));
        let target_el = element::make_element(self.engine.vm(), node_id as i64);
        evt.set_property(String::from("target"), target_el.clone());
        evt.set_property(String::from("currentTarget"), target_el);
        evt.set_property(String::from("preventDefault"), native_fn("preventDefault", |_,_| JsValue::Undefined));
        evt.set_property(String::from("stopPropagation"), native_fn("stopPropagation", |_,_| JsValue::Undefined));
        evt.set_property(String::from("bubbles"), JsValue::Bool(true));
        evt.set_property(String::from("cancelable"), JsValue::Bool(true));

        // Set up bridge for DOM access during callback.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            next_virtual_id: -1,
            virtual_nodes: Vec::new(),
            pending_http_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: self.next_timer_id,
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

        // Invoke each callback.
        for cb in &matching {
            self.engine.vm().call_value(cb, &[evt.clone()], JsValue::Undefined);
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
                };
                self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
                unsafe { MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>; }

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
