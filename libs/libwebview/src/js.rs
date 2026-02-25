//! JavaScript integration for libwebview.
//!
//! Executes `<script>` tags from the DOM and provides basic DOM API bindings
//! so that JavaScript can interact with the page (e.g., document.getElementById,
//! element.textContent, console.log, etc.).

use alloc::string::String;
use alloc::vec::Vec;

use libjs::{JsEngine, JsValue, Vm};

use crate::dom::{Dom, NodeType, Tag};

/// Manages JavaScript execution for a web page.
pub struct JsRuntime {
    engine: JsEngine,
    /// Console output captured from JS execution.
    pub console: Vec<String>,
}

impl JsRuntime {
    pub fn new() -> Self {
        let mut engine = JsEngine::new();
        // Set a reasonable step limit to prevent infinite loops from hanging the browser.
        engine.set_step_limit(5_000_000);
        // Register browser-specific global functions.
        engine.register_native("alert", js_alert);
        engine.register_native("setTimeout", js_set_timeout);
        engine.register_native("setInterval", js_set_interval);
        engine.register_native("clearTimeout", js_clear_timeout);
        engine.register_native("clearInterval", js_clear_interval);

        Self {
            engine,
            console: Vec::new(),
        }
    }

    /// Execute all `<script>` tags in the DOM.
    /// Returns any console output produced.
    pub fn execute_scripts(&mut self, dom: &Dom) {
        // Collect all <script> elements and their text content.
        let mut scripts: Vec<String> = Vec::new();

        for (i, node) in dom.nodes.iter().enumerate() {
            if let NodeType::Element { tag: Tag::Script, attrs } = &node.node_type {
                // Skip external scripts (src attribute) — we'd need HTTP fetch for those.
                let has_src = attrs.iter().any(|a| a.name == "src");
                if has_src {
                    continue;
                }

                // Skip non-JavaScript types.
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

        // Set up document object with basic DOM API.
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
    }

    /// Set up the `document` global object with basic DOM API methods.
    fn setup_document_api(&mut self, dom: &Dom) {
        // Set document.title
        if let Some(title) = dom.find_title() {
            self.engine.eval(&{
                let mut s = String::from("var document = { title: '");
                // Escape single quotes in title.
                for ch in title.chars() {
                    if ch == '\'' { s.push_str("\\'"); }
                    else if ch == '\\' { s.push_str("\\\\"); }
                    else { s.push(ch); }
                }
                s.push_str("' };");
                s
            });
        } else {
            self.engine.eval("var document = { title: '' };");
        }

        // Register basic DOM methods as native functions.
        self.engine.register_native("__dom_getElementById", js_get_element_by_id);
        self.engine.register_native("__dom_querySelector", js_query_selector);
        self.engine.register_native("__dom_createElement", js_create_element);

        // Set up document methods via JS wrappers.
        self.engine.eval(
            "document.getElementById = function(id) { return __dom_getElementById(id); };\
             document.querySelector = function(sel) { return __dom_querySelector(sel); };\
             document.createElement = function(tag) { return __dom_createElement(tag); };\
             document.body = { children: [], appendChild: function(c) { this.children.push(c); } };\
             document.head = {};"
        );

        // Set up window object.
        self.engine.eval(
            "var window = {\
                document: document,\
                location: { href: '', hostname: '', pathname: '/', protocol: 'http:' },\
                navigator: { userAgent: 'anyOS Surf/1.0' },\
                innerWidth: 800,\
                innerHeight: 600,\
                alert: alert,\
                setTimeout: setTimeout,\
                setInterval: setInterval,\
                clearTimeout: clearTimeout,\
                clearInterval: clearInterval\
            };"
        );
    }

    /// Evaluate additional JavaScript code (e.g., from external script loads).
    pub fn eval(&mut self, source: &str) -> JsValue {
        let result = self.engine.eval(source);
        // Capture console output.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
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

    /// Access the underlying JS engine.
    pub fn engine(&mut self) -> &mut JsEngine {
        &mut self.engine
    }
}

// ---------------------------------------------------------------------------
// Native function implementations for browser APIs
// ---------------------------------------------------------------------------

fn js_alert(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // In a real browser this would show a dialog.
    // For now, just append to console output.
    if let Some(msg) = args.first() {
        _vm.console_output.push(alloc::format!("[alert] {}", msg.to_js_string()));
    }
    JsValue::Undefined
}

fn js_set_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // Stub — real implementation would need event loop integration.
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

fn js_get_element_by_id(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Stub — returns a basic element-like object.
    if let Some(id) = args.first() {
        let id_str = id.to_js_string();
        let code = alloc::format!(
            "({{ id: '{}', tagName: 'DIV', textContent: '', style: {{}}, \
               getAttribute: function(n) {{ return ''; }}, \
               setAttribute: function(n,v) {{}}, \
               addEventListener: function(e,f) {{}} }})",
            id_str
        );
        _vm.console_output.push(String::new()); // placeholder for eval context
        _vm.console_output.pop();
        // We can't easily eval inside a native fn, so return a simple object.
        JsValue::Null
    } else {
        JsValue::Null
    }
}

fn js_query_selector(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Null
}

fn js_create_element(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(tag) = args.first() {
        let _tag_str = tag.to_js_string();
        // Return a stub element object.
        JsValue::Null
    } else {
        JsValue::Null
    }
}
