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

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::{JsEngine, JsValue, Vm};
use libjs::value::JsArray;
use libjs::vm::native_fn;

use crate::dom::{Dom, NodeType, Tag};

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
    pub callback_index: usize,
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

        if scripts.is_empty() { return; }

        // Set up DOM bridge via userdata.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            next_virtual_id: -1,
            virtual_nodes: Vec::new(),
            pending_http_requests: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        // Set up native host objects (document, window, etc.).
        self.setup_native_api(dom);

        // Execute each script.
        for script in &scripts {
            self.engine.eval(script);
        }

        // Capture output.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();

        self.mutations = bridge.mutations;
        self.event_listeners = bridge.event_listeners;
        self.pending_http_requests = bridge.pending_http_requests;
        self.engine.vm().userdata = core::ptr::null_mut();
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
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        let result = self.engine.eval(source);
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.pending_http_requests.extend(bridge.pending_http_requests);
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
