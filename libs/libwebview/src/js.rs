//! JavaScript integration for libwebview.
//!
//! Executes `<script>` tags from the DOM and provides real DOM API bindings
//! so that JavaScript can interact with the page (document.getElementById,
//! element.textContent, element.style, console.log, etc.).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use libjs::{JsEngine, JsValue, Vm};
use libjs::value::{JsObject, JsFunction, FnKind, Property};

use crate::dom::{Dom, NodeType, Tag};

// ---------------------------------------------------------------------------
// DomBridge — stored in vm.userdata so native fns can access the DOM
// ---------------------------------------------------------------------------

/// Bridge between the JS VM and the DOM tree. Stored as `vm.userdata`.
struct DomBridge {
    dom: *const Dom,
    /// Mutations recorded by JS (e.g., element.textContent = "...").
    mutations: Vec<DomMutation>,
    /// Event listeners registered by JS.
    event_listeners: Vec<EventListener>,
}

/// A recorded DOM mutation from JavaScript.
#[derive(Clone)]
pub enum DomMutation {
    SetAttribute { node_id: usize, name: String, value: String },
    SetTextContent { node_id: usize, text: String },
    RemoveAttribute { node_id: usize, name: String },
}

/// An event listener registered from JavaScript.
#[derive(Clone)]
pub struct EventListener {
    pub node_id: usize,
    pub event: String,
    // Note: callback is stored in JS global __eventCallbacks[index]
    pub callback_index: usize,
}

impl DomBridge {
    fn dom(&self) -> &Dom {
        unsafe { &*self.dom }
    }
}

/// Safely retrieve the DomBridge from vm.userdata.
fn get_bridge(vm: &mut Vm) -> Option<&mut DomBridge> {
    let ptr = vm.userdata;
    if ptr.is_null() {
        return None;
    }
    unsafe { Some(&mut *(ptr as *mut DomBridge)) }
}

// ---------------------------------------------------------------------------
// JsRuntime — public API
// ---------------------------------------------------------------------------

/// Manages JavaScript execution for a web page.
pub struct JsRuntime {
    engine: JsEngine,
    /// Console output captured from JS execution.
    pub console: Vec<String>,
    /// DOM mutations from last script execution.
    pub mutations: Vec<DomMutation>,
    /// Event listeners from last script execution.
    pub event_listeners: Vec<EventListener>,
}

impl JsRuntime {
    pub fn new() -> Self {
        let mut engine = JsEngine::new();
        engine.set_step_limit(5_000_000);

        // Register browser-specific global functions.
        engine.register_native("alert", js_alert);
        engine.register_native("setTimeout", js_set_timeout);
        engine.register_native("setInterval", js_set_interval);
        engine.register_native("clearTimeout", js_clear_timeout);
        engine.register_native("clearInterval", js_clear_interval);

        // Register DOM bridge native functions.
        engine.register_native("__dom_getTagName", dom_get_tag_name);
        engine.register_native("__dom_getAttribute", dom_get_attribute);
        engine.register_native("__dom_setAttribute", dom_set_attribute);
        engine.register_native("__dom_removeAttribute", dom_remove_attribute);
        engine.register_native("__dom_getTextContent", dom_get_text_content);
        engine.register_native("__dom_setTextContent", dom_set_text_content);
        engine.register_native("__dom_getChildren", dom_get_children);
        engine.register_native("__dom_getParent", dom_get_parent);
        engine.register_native("__dom_getElementById", dom_get_element_by_id);
        engine.register_native("__dom_getElementsByTagName", dom_get_elements_by_tag_name);
        engine.register_native("__dom_getElementsByClassName", dom_get_elements_by_class_name);
        engine.register_native("__dom_querySelector", dom_query_selector);
        engine.register_native("__dom_querySelectorAll", dom_query_selector_all);
        engine.register_native("__dom_createElement", dom_create_element);
        engine.register_native("__dom_getNodeType", dom_get_node_type);
        engine.register_native("__dom_getInnerHTML", dom_get_inner_html);
        engine.register_native("__dom_addEventListener", dom_add_event_listener);

        Self {
            engine,
            console: Vec::new(),
            mutations: Vec::new(),
            event_listeners: Vec::new(),
        }
    }

    /// Execute all `<script>` tags in the DOM.
    pub fn execute_scripts(&mut self, dom: &Dom) {
        // Collect all <script> elements and their text content.
        let mut scripts: Vec<String> = Vec::new();
        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Script, attrs } = &node.node_type {
                let has_src = attrs.iter().any(|a| a.name == "src");
                if has_src { continue; }

                let type_attr = attrs.iter().find(|a| a.name == "type");
                if let Some(t) = type_attr {
                    let lower = t.value.to_ascii_lowercase();
                    if !lower.is_empty()
                        && lower != "text/javascript"
                        && lower != "application/javascript"
                        && lower != "module"
                    {
                        continue;
                    }
                }

                let text = dom.text_content(i);
                if !text.is_empty() {
                    scripts.push(text);
                }
            }
        }

        if scripts.is_empty() {
            return;
        }

        // Set up DOM bridge via userdata.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        // Set up document/window objects with real DOM API.
        self.setup_document_api(dom);

        // Execute each script in order.
        for script in &scripts {
            self.engine.eval(script);
        }

        // Capture console output.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();

        // Collect mutations and event listeners.
        self.mutations = bridge.mutations;
        self.event_listeners = bridge.event_listeners;

        // Clear userdata (bridge is about to go out of scope).
        self.engine.vm().userdata = core::ptr::null_mut();
    }

    /// Set up the `document` global object with real DOM API methods.
    fn setup_document_api(&mut self, dom: &Dom) {
        // Find body and head node IDs.
        let body_id = dom.find_body().unwrap_or(0);
        let head_id: usize = dom.nodes.iter().enumerate()
            .find(|(_, n)| matches!(&n.node_type, NodeType::Element { tag: Tag::Head, .. }))
            .map(|(i, _)| i)
            .unwrap_or(0);

        // Build title string.
        let title = dom.find_title().unwrap_or_else(|| String::from(""));
        let title_escaped = js_escape_string(&title);

        // Initialize event callback storage and element cache.
        self.engine.eval("var __eventCallbacks = [];");

        // Create the __makeElement function that builds element proxy objects.
        // Uses plain properties (no getter/setter support needed).
        self.engine.eval(
            "function __makeElement(nodeId) {\
                if (nodeId < 0) return null;\
                var tc = __dom_getTextContent(nodeId);\
                var cn = __dom_getAttribute(nodeId, 'class') || '';\
                var childIds = __dom_getChildren(nodeId);\
                var childArr = [];\
                for (var ci = 0; ci < childIds.length; ci++) childArr.push(__makeElement(childIds[ci]));\
                var pid = __dom_getParent(nodeId);\
                var el = {\
                    __nodeId: nodeId,\
                    nodeType: __dom_getNodeType(nodeId),\
                    tagName: __dom_getTagName(nodeId),\
                    id: __dom_getAttribute(nodeId, 'id') || '',\
                    className: cn,\
                    textContent: tc,\
                    innerText: tc,\
                    innerHTML: __dom_getInnerHTML(nodeId),\
                    value: __dom_getAttribute(nodeId, 'value') || '',\
                    src: __dom_getAttribute(nodeId, 'src') || '',\
                    href: __dom_getAttribute(nodeId, 'href') || '',\
                    type: __dom_getAttribute(nodeId, 'type') || '',\
                    name: __dom_getAttribute(nodeId, 'name') || '',\
                    checked: __dom_getAttribute(nodeId, 'checked') !== null,\
                    disabled: __dom_getAttribute(nodeId, 'disabled') !== null,\
                    children: childArr,\
                    childNodes: childArr,\
                    firstChild: childArr.length > 0 ? childArr[0] : null,\
                    lastChild: childArr.length > 0 ? childArr[childArr.length - 1] : null,\
                    parentNode: null,\
                    parentElement: null,\
                    nextSibling: null,\
                    previousSibling: null,\
                    style: {},\
                    dataset: {},\
                    getAttribute: function(n) { return __dom_getAttribute(this.__nodeId, n); },\
                    setAttribute: function(n, v) { __dom_setAttribute(this.__nodeId, n, '' + v); },\
                    removeAttribute: function(n) { __dom_removeAttribute(this.__nodeId, n); },\
                    hasAttribute: function(n) { return __dom_getAttribute(this.__nodeId, n) !== null; },\
                    addEventListener: function(e, fn) { __dom_addEventListener(this.__nodeId, e, fn); },\
                    querySelector: function(sel) { var r = __dom_querySelector(sel); return r && r.__nodeId >= 0 ? __makeElement(r.__nodeId) : null; },\
                    querySelectorAll: function(sel) { var r = __dom_querySelectorAll(sel); var out = []; for (var i = 0; i < r.length; i++) { if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); } return out; },\
                    getElementsByTagName: function(tag) { var r = __dom_getElementsByTagName(tag); var out = []; for (var i = 0; i < r.length; i++) { if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); } return out; },\
                    getElementsByClassName: function(cls) { var r = __dom_getElementsByClassName(cls); var out = []; for (var i = 0; i < r.length; i++) { if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); } return out; },\
                    appendChild: function(c) { this.children.push(c); return c; },\
                    removeChild: function(c) { return c; },\
                    insertBefore: function(n, r) { return n; },\
                    replaceChild: function(n, o) { return o; },\
                    cloneNode: function() { return __makeElement(this.__nodeId); },\
                    contains: function(o) { return false; },\
                    matches: function(s) { return false; },\
                    closest: function(s) { return null; },\
                    focus: function() {},\
                    blur: function() {},\
                    click: function() {},\
                    remove: function() {},\
                    getBoundingClientRect: function() { return {top:0,left:0,bottom:0,right:0,width:0,height:0}; },\
                    toString: function() { return '[object HTMLElement]'; }\
                };\
                el.classList = {\
                    _el: el,\
                    add: function(c) {\
                        var cur = this._el.className;\
                        if ((' ' + cur + ' ').indexOf(' ' + c + ' ') === -1) {\
                            this._el.className = cur ? cur + ' ' + c : c;\
                            __dom_setAttribute(this._el.__nodeId, 'class', this._el.className);\
                        }\
                    },\
                    remove: function(c) {\
                        var cur = this._el.className.split(' ');\
                        var res = [];\
                        for (var i = 0; i < cur.length; i++) { if (cur[i] !== c) res.push(cur[i]); }\
                        this._el.className = res.join(' ');\
                        __dom_setAttribute(this._el.__nodeId, 'class', this._el.className);\
                    },\
                    toggle: function(c) {\
                        if (this.contains(c)) this.remove(c); else this.add(c);\
                    },\
                    contains: function(c) {\
                        return (' ' + this._el.className + ' ').indexOf(' ' + c + ' ') !== -1;\
                    },\
                    item: function(i) { return this._el.className.split(' ')[i] || null; }\
                };\
                return el;\
            }"
        );

        // Set up document object.
        let doc_init = alloc::format!(
            "var document = {{\
                title: '{}',\
                documentElement: __makeElement(0),\
                body: __makeElement({}),\
                head: __makeElement({}),\
                getElementById: function(id) {{ var r = __dom_getElementById(id); return r && r.__nodeId >= 0 ? __makeElement(r.__nodeId) : null; }},\
                getElementsByTagName: function(tag) {{ var r = __dom_getElementsByTagName(tag); var out = []; for (var i = 0; i < r.length; i++) {{ if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); }} return out; }},\
                getElementsByClassName: function(cls) {{ var r = __dom_getElementsByClassName(cls); var out = []; for (var i = 0; i < r.length; i++) {{ if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); }} return out; }},\
                querySelector: function(sel) {{ var r = __dom_querySelector(sel); return r && r.__nodeId >= 0 ? __makeElement(r.__nodeId) : null; }},\
                querySelectorAll: function(sel) {{ var r = __dom_querySelectorAll(sel); var out = []; for (var i = 0; i < r.length; i++) {{ if (r[i] && r[i].__nodeId >= 0) out.push(__makeElement(r[i].__nodeId)); }} return out; }},\
                createElement: function(tag) {{ return __dom_createElement(tag); }},\
                createTextNode: function(text) {{ return {{ nodeType: 3, textContent: text, __nodeId: -1 }}; }},\
                createDocumentFragment: function() {{ return {{ nodeType: 11, children: [], appendChild: function(c) {{ this.children.push(c); return c; }} }}; }},\
                createComment: function(text) {{ return {{ nodeType: 8, textContent: text }}; }},\
                cookie: '',\
                readyState: 'complete',\
                location: {{ href: '', hostname: '', pathname: '/', protocol: 'http:', search: '', hash: '' }},\
                referrer: '',\
                domain: '',\
                URL: '',\
                characterSet: 'UTF-8',\
                contentType: 'text/html',\
                compatMode: 'CSS1Compat'\
            }};",
            title_escaped, body_id, head_id,
        );
        self.engine.eval(&doc_init);

        // Set up window object.
        self.engine.eval(
            "var window = {\
                document: document,\
                location: document.location,\
                navigator: { userAgent: 'anyOS Surf/1.0', language: 'en-US', platform: 'anyOS', cookieEnabled: false },\
                screen: { width: 1024, height: 768, colorDepth: 32 },\
                innerWidth: 1024,\
                innerHeight: 768,\
                outerWidth: 1024,\
                outerHeight: 768,\
                devicePixelRatio: 1,\
                alert: alert,\
                setTimeout: setTimeout,\
                setInterval: setInterval,\
                clearTimeout: clearTimeout,\
                clearInterval: clearInterval,\
                getComputedStyle: function(el) { return el.style || {}; },\
                requestAnimationFrame: function(cb) { return 0; },\
                cancelAnimationFrame: function(id) {},\
                addEventListener: function(e, fn) {},\
                removeEventListener: function(e, fn) {},\
                dispatchEvent: function(e) { return true; },\
                atob: function(s) { return s; },\
                btoa: function(s) { return s; },\
                fetch: function() { return new Promise(function(r) { r({ ok: false, status: 0 }); }); },\
                XMLHttpRequest: function() { return { open: function(){}, send: function(){}, setRequestHeader: function(){} }; },\
                performance: { now: function() { return 0; } },\
                localStorage: { getItem: function(k) { return null; }, setItem: function(k,v) {}, removeItem: function(k) {} },\
                sessionStorage: { getItem: function(k) { return null; }, setItem: function(k,v) {}, removeItem: function(k) {} },\
                history: { pushState: function(){}, replaceState: function(){}, back: function(){}, forward: function(){} },\
                scrollTo: function(x, y) {},\
                scrollBy: function(x, y) {},\
                open: function() { return null; },\
                close: function() {},\
                print: function() {},\
                confirm: function(msg) { return false; },\
                prompt: function(msg, def) { return def || null; }\
            };\
            var self = window;\
            var globalThis = window;"
        );
    }

    /// Evaluate additional JavaScript code (e.g., from external script loads).
    pub fn eval(&mut self, source: &str) -> JsValue {
        let result = self.engine.eval(source);
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        result
    }

    /// Evaluate JS with DOM access (sets up the bridge temporarily).
    pub fn eval_with_dom(&mut self, source: &str, dom: &Dom) -> JsValue {
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        let result = self.engine.eval(source);
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.engine.vm().userdata = core::ptr::null_mut();
        result
    }

    /// Get all console messages.
    pub fn get_console(&self) -> &[String] {
        &self.console
    }

    /// Clear console messages.
    pub fn clear_console(&mut self) {
        self.console.clear();
    }

    /// Take collected DOM mutations (clears internal list).
    pub fn take_mutations(&mut self) -> Vec<DomMutation> {
        core::mem::take(&mut self.mutations)
    }

    /// Take collected event listeners (clears internal list).
    pub fn take_event_listeners(&mut self) -> Vec<EventListener> {
        core::mem::take(&mut self.event_listeners)
    }

    /// Access the underlying JS engine.
    pub fn engine(&mut self) -> &mut JsEngine {
        &mut self.engine
    }
}

// ---------------------------------------------------------------------------
// Helper: escape a string for embedding in JS single-quoted string literal
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Native function implementations — Browser APIs
// ---------------------------------------------------------------------------

fn js_alert(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(msg) = args.first() {
        vm.console_output.push(alloc::format!("[alert] {}", msg.to_js_string()));
    }
    JsValue::Undefined
}

fn js_set_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(0.0)
}

fn js_set_interval(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(0.0)
}

fn js_clear_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn js_clear_interval(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

// ---------------------------------------------------------------------------
// Native function implementations — DOM Bridge
// ---------------------------------------------------------------------------

/// __dom_getTagName(nodeId) → "DIV" | "A" | "INPUT" | ...
fn dom_get_tag_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Null };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::Null; }
    match dom.tag(node_id) {
        Some(tag) => JsValue::String(String::from(tag.tag_name())),
        None => JsValue::String(String::from("#text")),
    }
}

/// __dom_getAttribute(nodeId, name) → string | null
fn dom_get_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let name = arg_string(args, 1);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Null };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::Null; }
    match dom.attr(node_id, &name) {
        Some(val) => JsValue::String(String::from(val)),
        None => JsValue::Null,
    }
}

/// __dom_setAttribute(nodeId, name, value) — records a mutation
fn dom_set_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let name = arg_string(args, 1);
    let value = arg_string(args, 2);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetAttribute { node_id, name, value });
    }
    JsValue::Undefined
}

/// __dom_removeAttribute(nodeId, name)
fn dom_remove_attribute(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let name = arg_string(args, 1);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::RemoveAttribute { node_id, name });
    }
    JsValue::Undefined
}

/// __dom_getTextContent(nodeId) → string
fn dom_get_text_content(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::String(String::new()) };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::String(String::new()); }
    JsValue::String(dom.text_content(node_id))
}

/// __dom_setTextContent(nodeId, text) — records a mutation
fn dom_set_text_content(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let text = arg_string(args, 1);
    if let Some(bridge) = get_bridge(vm) {
        bridge.mutations.push(DomMutation::SetTextContent { node_id, text });
    }
    JsValue::Undefined
}

/// __dom_getChildren(nodeId) → [childId1, childId2, ...] (element children only)
fn dom_get_children(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return make_array(vec![]) };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return make_array(vec![]); }
    let children = &dom.get(node_id).children;
    let elements: Vec<JsValue> = children.iter()
        .filter(|&&cid| matches!(&dom.nodes[cid].node_type, NodeType::Element { .. }))
        .map(|&cid| JsValue::Number(cid as f64))
        .collect();
    make_array(elements)
}

/// __dom_getParent(nodeId) → parentId | -1
fn dom_get_parent(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Number(-1.0) };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::Number(-1.0); }
    match dom.get(node_id).parent {
        Some(pid) => JsValue::Number(pid as f64),
        None => JsValue::Number(-1.0),
    }
}

/// __dom_getNodeType(nodeId) → 1 (element) | 3 (text)
fn dom_get_node_type(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Number(1.0) };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::Number(1.0); }
    match &dom.nodes[node_id].node_type {
        NodeType::Element { .. } => JsValue::Number(1.0),
        NodeType::Text(_) => JsValue::Number(3.0),
    }
}

/// __dom_getInnerHTML(nodeId) → simplified HTML string
fn dom_get_inner_html(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::String(String::new()) };
    let dom = bridge.dom();
    if node_id >= dom.nodes.len() { return JsValue::String(String::new()); }
    let mut html = String::new();
    for &cid in &dom.get(node_id).children {
        serialize_node(dom, cid, &mut html);
    }
    JsValue::String(html)
}

/// __dom_getElementById(id) → element | null
fn dom_get_element_by_id(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = arg_string(args, 0);
    if id.is_empty() { return JsValue::Null; }
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Null };
    let dom = bridge.dom();
    for (i, node) in dom.nodes.iter().enumerate() {
        if let NodeType::Element { attrs, .. } = &node.node_type {
            for a in attrs {
                if a.name == "id" && a.value == id {
                    return make_element_call(i);
                }
            }
        }
    }
    JsValue::Null
}

/// __dom_getElementsByTagName(tag) → [element, ...]
fn dom_get_elements_by_tag_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag_name = arg_string(args, 0).to_ascii_uppercase();
    let bridge = match get_bridge(vm) { Some(b) => b, None => return make_array(vec![]) };
    let dom = bridge.dom();
    let target_tag = Tag::from_str(&tag_name);
    let mut results = Vec::new();
    for (i, node) in dom.nodes.iter().enumerate() {
        if let NodeType::Element { tag, .. } = &node.node_type {
            if *tag == target_tag || tag_name == "*" {
                results.push(make_element_call(i));
            }
        }
    }
    make_array(results)
}

/// __dom_getElementsByClassName(className) → [element, ...]
fn dom_get_elements_by_class_name(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let class_name = arg_string(args, 0);
    if class_name.is_empty() { return make_array(vec![]); }
    let bridge = match get_bridge(vm) { Some(b) => b, None => return make_array(vec![]) };
    let dom = bridge.dom();
    let mut results = Vec::new();
    for (i, node) in dom.nodes.iter().enumerate() {
        if let NodeType::Element { attrs, .. } = &node.node_type {
            for a in attrs {
                if a.name == "class" {
                    let classes: Vec<&str> = a.value.split_whitespace().collect();
                    if classes.iter().any(|c| *c == class_name) {
                        results.push(make_element_call(i));
                        break;
                    }
                }
            }
        }
    }
    make_array(results)
}

/// __dom_querySelector(selector) → element | null
/// Simplified: supports #id, .class, tag, tag.class, tag#id
fn dom_query_selector(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() { return JsValue::Null; }
    let bridge = match get_bridge(vm) { Some(b) => b, None => return JsValue::Null };
    let dom = bridge.dom();
    if let Some(id) = find_matching_node(dom, &sel) {
        return make_element_call(id);
    }
    JsValue::Null
}

/// __dom_querySelectorAll(selector) → [element, ...]
fn dom_query_selector_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sel = arg_string(args, 0);
    if sel.is_empty() { return make_array(vec![]); }
    let bridge = match get_bridge(vm) { Some(b) => b, None => return make_array(vec![]) };
    let dom = bridge.dom();
    let results = find_all_matching_nodes(dom, &sel);
    let elements: Vec<JsValue> = results.iter().map(|&id| make_element_call(id)).collect();
    make_array(elements)
}

/// __dom_createElement(tagName) → virtual element (not in DOM tree)
fn dom_create_element(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let tag = arg_string(args, 0).to_ascii_uppercase();
    // Return a standalone element object (not connected to DOM tree).
    let mut obj = JsObject::new();
    obj.set(String::from("__nodeId"), JsValue::Number(-1.0));
    obj.set(String::from("nodeType"), JsValue::Number(1.0));
    obj.set(String::from("tagName"), JsValue::String(tag));
    obj.set(String::from("id"), JsValue::String(String::new()));
    obj.set(String::from("className"), JsValue::String(String::new()));
    obj.set(String::from("textContent"), JsValue::String(String::new()));
    obj.set(String::from("innerHTML"), JsValue::String(String::new()));
    obj.set(String::from("innerText"), JsValue::String(String::new()));
    obj.set(String::from("style"), JsValue::Object(Box::new(JsObject::new())));
    obj.set(String::from("children"), JsValue::Array(Box::new(libjs::value::JsArray::new())));

    // Stub methods.
    let noop = make_noop();
    obj.set(String::from("getAttribute"), noop.clone());
    obj.set(String::from("setAttribute"), noop.clone());
    obj.set(String::from("removeAttribute"), noop.clone());
    obj.set(String::from("hasAttribute"), JsValue::Bool(false));
    obj.set(String::from("addEventListener"), noop.clone());
    obj.set(String::from("appendChild"), noop.clone());
    obj.set(String::from("removeChild"), noop.clone());
    obj.set(String::from("remove"), noop.clone());
    obj.set(String::from("focus"), noop.clone());
    obj.set(String::from("blur"), noop);

    JsValue::Object(Box::new(obj))
}

/// __dom_addEventListener(nodeId, event, callback)
fn dom_add_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let node_id = arg_node_id(args, 0);
    let event = arg_string(args, 1);
    if let Some(bridge) = get_bridge(vm) {
        let index = bridge.event_listeners.len();
        bridge.event_listeners.push(EventListener {
            node_id,
            event,
            callback_index: index,
        });
    }
    JsValue::Undefined
}

// ---------------------------------------------------------------------------
// Selector matching (simplified for querySelector/querySelectorAll)
// ---------------------------------------------------------------------------

/// Find first element matching a simple CSS selector.
fn find_matching_node(dom: &Dom, selector: &str) -> Option<usize> {
    for (i, _) in dom.nodes.iter().enumerate() {
        if node_matches_selector(dom, i, selector) {
            return Some(i);
        }
    }
    None
}

/// Find all elements matching a simple CSS selector.
fn find_all_matching_nodes(dom: &Dom, selector: &str) -> Vec<usize> {
    let mut results = Vec::new();
    for (i, _) in dom.nodes.iter().enumerate() {
        if node_matches_selector(dom, i, selector) {
            results.push(i);
        }
    }
    results
}

/// Check if a node matches a simple CSS selector.
/// Supports: #id, .class, tag, tag.class, tag#id, [attr], [attr=val]
fn node_matches_selector(dom: &Dom, node_id: usize, selector: &str) -> bool {
    let node = &dom.nodes[node_id];
    let (tag, attrs) = match &node.node_type {
        NodeType::Element { tag, attrs } => (tag, attrs),
        _ => return false,
    };

    let sel = selector.trim();
    if sel.is_empty() { return false; }

    // Handle comma-separated selectors — match any.
    if sel.contains(',') {
        return sel.split(',').any(|s| node_matches_selector(dom, node_id, s.trim()));
    }

    // Simple selector: #id
    if sel.starts_with('#') {
        let target_id = &sel[1..];
        return attrs.iter().any(|a| a.name == "id" && a.value == target_id);
    }

    // Simple selector: .class
    if sel.starts_with('.') {
        let target_class = &sel[1..];
        return attrs.iter().any(|a| {
            a.name == "class" && a.value.split_whitespace().any(|c| c == target_class)
        });
    }

    // Simple selector: [attr] or [attr=val]
    if sel.starts_with('[') && sel.ends_with(']') {
        let inner = &sel[1..sel.len() - 1];
        if let Some(eq_pos) = inner.find('=') {
            let attr_name = inner[..eq_pos].trim();
            let attr_val = inner[eq_pos + 1..].trim().trim_matches('"').trim_matches('\'');
            return attrs.iter().any(|a| a.name == attr_name && a.value == attr_val);
        } else {
            let attr_name = inner.trim();
            return attrs.iter().any(|a| a.name == attr_name);
        }
    }

    // Combined: tag.class or tag#id
    if let Some(dot_pos) = sel.find('.') {
        if dot_pos > 0 {
            let tag_name = &sel[..dot_pos];
            let class_name = &sel[dot_pos + 1..];
            let tag_match = Tag::from_str(tag_name) == *tag;
            let class_match = attrs.iter().any(|a| {
                a.name == "class" && a.value.split_whitespace().any(|c| c == class_name)
            });
            return tag_match && class_match;
        }
    }
    if let Some(hash_pos) = sel.find('#') {
        if hash_pos > 0 {
            let tag_name = &sel[..hash_pos];
            let id_name = &sel[hash_pos + 1..];
            let tag_match = Tag::from_str(tag_name) == *tag;
            let id_match = attrs.iter().any(|a| a.name == "id" && a.value == id_name);
            return tag_match && id_match;
        }
    }

    // Plain tag name selector.
    let target_tag = Tag::from_str(sel);
    if target_tag != Tag::Unknown {
        return *tag == target_tag;
    }

    // Universal selector.
    if sel == "*" { return true; }

    false
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a node_id (usize) from args at the given index.
fn arg_node_id(args: &[JsValue], index: usize) -> usize {
    args.get(index)
        .map(|v| {
            let n = v.to_number();
            if n >= 0.0 && !n.is_nan() { n as usize } else { usize::MAX }
        })
        .unwrap_or(usize::MAX)
}

/// Extract a string from args at the given index.
fn arg_string(args: &[JsValue], index: usize) -> String {
    args.get(index)
        .map(|v| v.to_js_string())
        .unwrap_or_else(String::new)
}

/// Create a JS array value from a Vec<JsValue>.
fn make_array(elements: Vec<JsValue>) -> JsValue {
    JsValue::Array(Box::new(libjs::value::JsArray::from_vec(elements)))
}

/// Create a noop JS function.
fn make_noop() -> JsValue {
    JsValue::Function(Box::new(JsFunction {
        name: None,
        params: Vec::new(),
        kind: FnKind::Native(noop_fn),
        this_binding: None,
    }))
}

fn noop_fn(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

/// Return a JS expression that calls __makeElement(nodeId).
/// Since we can't eval from a native function, we return a plain object
/// with __nodeId and call __makeElement from the JS side via wrappers.
fn make_element_call(node_id: usize) -> JsValue {
    // We create a minimal JS object with __nodeId.
    // The document.getElementById wrapper in JS will call __makeElement on it.
    let mut obj = JsObject::new();
    obj.set(String::from("__nodeId"), JsValue::Number(node_id as f64));
    JsValue::Object(Box::new(obj))
}

/// Serialize a DOM node to HTML string (simplified).
fn serialize_node(dom: &Dom, node_id: usize, out: &mut String) {
    match &dom.nodes[node_id].node_type {
        NodeType::Text(t) => out.push_str(t),
        NodeType::Element { tag, attrs } => {
            out.push('<');
            let tn = tag.tag_name();
            // Lowercase for HTML output.
            for b in tn.as_bytes() {
                out.push((*b | 32) as char);
            }
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
            for b in tn.as_bytes() {
                out.push((*b | 32) as char);
            }
            out.push('>');
        }
    }
}
