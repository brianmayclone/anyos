//! JavaScript integration for libwebview — native host object approach.
//!
//! All DOM objects (Element, Document, Window) are created as native
//! JsObject instances in Rust, with native function methods — no JS
//! injection.  This mirrors how real browsers (Chromium/Blink, Gecko)
//! expose the DOM to their JavaScript engines.

mod classlist;
mod document;
mod element;
mod fetch;
mod http;
mod selector;
mod storage;
pub mod websocket;
mod window;
mod xhr;

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::value::{JsArray, JsObject};
use libjs::vm::native_fn;
use libjs::{JsEngine, JsValue, Vm};

use crate::css::{Declaration, KeyframeSet};
use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{apply_timing, TimingFunction, TransitionDef};

// ═══════════════════════════════════════════════════════════
// Property write interception — static target for set_hook
// ═══════════════════════════════════════════════════════════

/// Points to the current DomBridge.mutations during JS execution.
/// Set before executing JS, cleared after. Used by dom_property_hook.
static mut MUTATION_TARGET: *mut Vec<DomMutation> = core::ptr::null_mut();
/// Points to the current DomBridge.virtual_nodes during JS execution.
static mut VIRTUAL_NODES_TARGET: *mut Vec<VirtualNode> = core::ptr::null_mut();

/// Hook called by JsObject::set() on DOM element objects.
/// Records DOM mutations when JS writes to properties like
/// textContent, innerHTML, className, value, etc.
fn dom_property_hook(data: *mut u8, key: &str, value: &JsValue) {
    let mutations = unsafe {
        if MUTATION_TARGET.is_null() {
            return;
        }
        &mut *MUTATION_TARGET
    };
    // Decode node_id from pointer (round-trips correctly for negative i64 on 64-bit).
    let node_id = data as usize as i64;

    match key {
        "textContent" | "innerText" => {
            if node_id >= 0 {
                mutations.push(DomMutation::SetTextContent {
                    node_id: node_id,
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
            let mut cls = value.to_js_string();
            if cls.contains("client-js") {
                cls = cls.replace("client-js", "client-nojs");
            }
            // Always emit SetAttribute mutation — for virtual nodes,
            // apply_mutations resolves the ID via id_map.
            mutations.push(DomMutation::SetAttribute {
                node_id: node_id,
                name: String::from("class"),
                value: cls,
            });
        }
        "value" | "src" | "href" | "id" | "name" | "type" => {
            if node_id >= 0 {
                mutations.push(DomMutation::SetAttribute {
                    node_id: node_id,
                    name: String::from(key),
                    value: value.to_js_string(),
                });
            }
        }
        "checked" | "disabled" => {
            if node_id >= 0 {
                if value.to_boolean() {
                    mutations.push(DomMutation::SetAttribute {
                        node_id: node_id,
                        name: String::from(key),
                        value: String::new(),
                    });
                } else {
                    mutations.push(DomMutation::RemoveAttribute {
                        node_id: node_id,
                        name: String::from(key),
                    });
                }
            }
        }
        "scrollTop" => {
            if node_id >= 0 {
                let n = match value {
                    JsValue::Number(f) => *f as i32,
                    _ => 0,
                };
                mutations.push(DomMutation::SetScrollTop {
                    node_id: node_id as usize,
                    value: n.max(0),
                });
            }
        }
        "scrollLeft" => {
            if node_id >= 0 {
                let n = match value {
                    JsValue::Number(f) => *f as i32,
                    _ => 0,
                };
                mutations.push(DomMutation::SetScrollLeft {
                    node_id: node_id as usize,
                    value: n.max(0),
                });
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
    /// Set by `stopPropagation()` — halts moving to the next node, but
    /// remaining listeners on the *current* node still fire.
    propagation_stopped: bool,
    /// Set by `stopImmediatePropagation()` — halts all further listeners,
    /// including remaining ones on the current node.
    immediate_stopped: bool,
    /// Set by `preventDefault()` — signals that the default action is cancelled.
    prevented: bool,
    /// Pending WebSocket connect requests from `new WebSocket(url)`.
    pending_ws_connects: Vec<PendingWsConnect>,
    /// Pending WebSocket send requests from `ws.send(data)`.
    pending_ws_sends: Vec<PendingWsSend>,
    /// Pending WebSocket close requests from `ws.close()`.
    pending_ws_closes: Vec<PendingWsClose>,
    /// Live WebSocket objects: (ws_id → JsValue clone) for callback delivery.
    ws_registry: Vec<(u64, JsValue)>,
    /// Pending removeEventListener requests: (node_id, event_name, callback, capture).
    remove_listeners: Vec<(usize, String, JsValue, bool)>,
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
    if ptr.is_null() {
        return None;
    }
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
    SetAttribute {
        node_id: i64,
        name: String,
        value: String,
    },
    SetTextContent {
        node_id: i64,
        text: String,
    },
    RemoveAttribute {
        node_id: i64,
        name: String,
    },
    CreateElement {
        virtual_id: i64,
        tag: String,
    },
    CreateTextNode {
        virtual_id: i64,
        text: String,
    },
    AppendChild {
        parent_id: i64,
        child_id: i64,
    },
    RemoveChild {
        parent_id: i64,
        child_id: i64,
    },
    InsertBefore {
        parent_id: i64,
        new_child_id: i64,
        ref_child_id: i64,
    },
    ReplaceChild {
        parent_id: i64,
        new_child_id: i64,
        old_child_id: i64,
    },
    RemoveNode {
        node_id: i64,
    },
    SetInnerHTML {
        node_id: i64,
        html: String,
    },
    SetStyleProperty {
        node_id: i64,
        property: String,
        value: String,
    },
    /// A `document.cookie = "..."` assignment from JavaScript.
    /// The host application should parse this Set-Cookie string and update its
    /// cookie jar accordingly.
    SetCookie {
        value: String,
    },
    /// Set the vertical scroll offset on an overflow container (JS `element.scrollTop = n`).
    SetScrollTop {
        node_id: usize,
        value: i32,
    },
    /// Set the horizontal scroll offset on an overflow container (JS `element.scrollLeft = n`).
    SetScrollLeft {
        node_id: usize,
        value: i32,
    },
    /// JS `form.submit()` — programmatic form submission.
    FormSubmit {
        form_node_id: usize,
    },
    /// JS `form.reset()` — programmatic form reset.
    FormReset {
        form_node_id: usize,
    },
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
///
/// `capture` corresponds to the third argument of `addEventListener()`:
/// - `true`  → listener fires during the capture phase (root → target).
/// - `false` → listener fires during the bubble phase  (target → root).
#[derive(Clone)]
pub struct EventListener {
    pub node_id: usize,
    pub event: String,
    pub callback: JsValue,
    /// True when registered with `{ capture: true }` or a bare `true` third arg.
    pub capture: bool,
}

// ═══════════════════════════════════════════════════════════
// EventData — typed event payload (W3C DOM Events Level 3)
// ═══════════════════════════════════════════════════════════

/// Event-specific properties passed to [`JsRuntime::dispatch_event`].
///
/// Each variant maps directly to the matching W3C interface:
/// - [`EventData::Mouse`]   → [MouseEvent](https://www.w3.org/TR/uievents/#mouseevent)
/// - [`EventData::Keyboard`] → [KeyboardEvent](https://www.w3.org/TR/uievents/#keyboardevent)
/// - [`EventData::Input`]   → [InputEvent](https://www.w3.org/TR/input-events-2/)
/// - [`EventData::Focus`]   → [FocusEvent](https://www.w3.org/TR/uievents/#focusevent)
/// - [`EventData::Wheel`]   → [WheelEvent](https://www.w3.org/TR/uievents/#wheelevent)
/// - [`EventData::Pointer`] → [PointerEvent](https://www.w3.org/TR/pointerevents3/)
#[derive(Clone)]
pub enum EventData {
    /// Plain Event — no extra properties.
    None,
    /// MouseEvent (click, mousedown, mouseup, mousemove, mouseover, mouseout, mouseenter, mouseleave).
    Mouse {
        client_x: f64,
        client_y: f64,
        page_x: f64,
        page_y: f64,
        screen_x: f64,
        screen_y: f64,
        offset_x: f64,
        offset_y: f64,
        /// 0=main, 1=aux, 2=secondary
        button: u8,
        /// Bitmask of currently pressed buttons.
        buttons: u8,
        ctrl_key: bool,
        shift_key: bool,
        alt_key: bool,
        meta_key: bool,
    },
    /// KeyboardEvent (keydown, keyup, keypress).
    Keyboard {
        /// Printable character or key name per W3C key values spec.
        key: String,
        /// Physical key code (e.g. "KeyA", "ArrowLeft").
        code: String,
        /// Legacy keyCode for compatibility.
        key_code: u32,
        /// Legacy which (same as key_code for most keys).
        which: u32,
        /// Legacy charCode (only meaningful for keypress).
        char_code: u32,
        ctrl_key: bool,
        shift_key: bool,
        alt_key: bool,
        meta_key: bool,
        /// True when the key is held down and auto-repeat fires.
        repeat: bool,
        is_composing: bool,
    },
    /// InputEvent (input, beforeinput).
    Input {
        /// The inserted/deleted characters, or None for non-printable actions.
        data: Option<String>,
        /// W3C inputType (e.g. "insertText", "deleteContentBackward").
        input_type: String,
        is_composing: bool,
    },
    /// FocusEvent (focus, blur, focusin, focusout).
    Focus {
        /// node_id of the element losing/gaining focus, if any.
        related_target_id: Option<usize>,
    },
    /// WheelEvent (wheel).
    Wheel {
        delta_x: f64,
        delta_y: f64,
        delta_z: f64,
        /// 0=pixel, 1=line, 2=page
        delta_mode: u32,
        client_x: f64,
        client_y: f64,
        ctrl_key: bool,
        shift_key: bool,
        alt_key: bool,
        meta_key: bool,
    },
    /// PointerEvent (pointerdown, pointerup, pointermove, etc.).
    Pointer {
        client_x: f64,
        client_y: f64,
        page_x: f64,
        page_y: f64,
        screen_x: f64,
        screen_y: f64,
        pointer_id: i32,
        /// "mouse", "pen", or "touch".
        pointer_type: String,
        pressure: f64,
        tilt_x: f64,
        tilt_y: f64,
        is_primary: bool,
        button: u8,
        buttons: u8,
        ctrl_key: bool,
        shift_key: bool,
        alt_key: bool,
        meta_key: bool,
    },
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
    /// True for requestAnimationFrame timers — callback receives a DOMHighResTimeStamp.
    pub is_raf: bool,
}

/// A script entry found in the DOM — either inline text or an external URL.
#[derive(Clone)]
pub enum ScriptMode {
    Blocking,
    Defer,
    Async,
}

#[derive(Clone)]
pub enum ScriptEntry {
    /// Inline `<script>` with text content.
    Inline { text: String, mode: ScriptMode },
    /// External `<script src="url">` — the host must fetch the URL and provide the text.
    External { src: String, mode: ScriptMode },
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
    /// Virtual DOM nodes created by JS but not yet inserted into real DOM.
    pub virtual_nodes: Vec<VirtualNode>,
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
    /// Total elapsed time since page load (ms) — used as DOMHighResTimeStamp for rAF callbacks.
    total_elapsed_ms: u64,
    /// Viewport dimensions exposed to `window`.
    viewport_width: u32,
    viewport_height: u32,
    native_api_initialized: bool,
    native_api_url: String,
}

impl JsRuntime {
    pub fn new() -> Self {
        let engine = JsEngine::new();
        Self {
            engine,
            console: Vec::new(),
            mutations: Vec::new(),
            virtual_nodes: Vec::new(),
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
            total_elapsed_ms: 0,
            viewport_width: 1024,
            viewport_height: 768,
            native_api_initialized: false,
            native_api_url: String::new(),
        }
    }

    pub fn set_viewport(&mut self, width: u32, height: u32) {
        self.viewport_width = width.max(1);
        self.viewport_height = height.max(1);
    }

    /// Set the cookie string that will be exposed as `document.cookie` during
    /// the next `execute_scripts` call.  The value should be in the same format
    /// as the `Cookie` HTTP request header: `"name=value; name2=value2"`.
    pub fn set_cookies(&mut self, cookies: &str) {
        self.cookies = String::from(cookies);
        if self.native_api_initialized {
            let doc = self.engine.vm().get_global("document");
            if !doc.is_undefined() {
                doc.set_property(String::from("cookie"), JsValue::String(self.cookies.clone()));
            }
        }
    }

    /// Collect all `<script>` entries from the DOM in document order.
    ///
    /// Returns a list of [`ScriptEntry`] — either inline text or external URLs.
    /// The host can then fetch external scripts and pass the resolved texts to
    /// [`execute_script_sources`].
    pub fn collect_script_entries(dom: &Dom) -> Vec<ScriptEntry> {
        let mut entries = Vec::new();
        for i in 0..dom.nodes.len() {
            if let NodeType::Element {
                tag: Tag::Script,
                attrs,
            } = &dom.nodes[i].node_type
            {
                // Check type attribute — skip non-JS types.
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
                let src = attrs
                    .iter()
                    .find(|a| a.name == "src")
                    .map(|a| a.value.as_str());
                let has_async = attrs.iter().any(|a| a.name == "async");
                let has_defer = attrs.iter().any(|a| a.name == "defer");
                let mode = if has_async {
                    ScriptMode::Async
                } else if has_defer {
                    ScriptMode::Defer
                } else {
                    ScriptMode::Blocking
                };
                if let Some(url) = src {
                    if !url.is_empty() {
                        entries.push(ScriptEntry::External {
                            src: String::from(url),
                            mode,
                        });
                    }
                } else {
                    let text = dom.text_content(i);
                    if !text.is_empty() {
                        entries.push(ScriptEntry::Inline { text, mode });
                    }
                }
            }
        }
        entries
    }

    /// Execute pre-resolved script sources (inline or fetched external).
    ///
    /// Call this after the host has resolved all [`ScriptEntry::External`] URLs
    /// into actual source text.  Pass each script's text in the `scripts` slice
    /// (in document order).
    ///
    /// * `url` — the current page URL, used to populate `window.location` /
    ///   `document.location` inside the JS environment.
    pub fn execute_script_sources(&mut self, dom: &Dom, url: &str, scripts: &[String]) {
        if scripts.is_empty() {
            return;
        }

        let total_bytes: usize = scripts.iter().map(|s| s.len()).sum();
        anyos_std::println!(
            "[js] {} script(s) to execute, {} bytes total",
            scripts.len(),
            total_bytes
        );

        // Cap large pages: limit total number of scripts.
        // Script size limit raised to 4 MiB to support modern bundlers (Vite, webpack)
        // that produce single bundles larger than 256 KiB (e.g. React apps at ~400+ KiB).
        const MAX_SCRIPTS: usize = 32;
        const MAX_SCRIPT_BYTES: usize = 4 * 1024 * 1024;

        // Per-script step limit to keep pages responsive.
        // Raised to 20 M to allow React's initial render tree to complete.
        self.engine.set_step_limit(20_000_000);

        // Set up DOM bridge via userdata.
        anyos_std::println!(
            "[js] setup begin: url={} scripts={} next_timer_id={} cookies_len={}",
            url,
            scripts.len(),
            self.next_timer_id,
            self.cookies.len()
        );
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
            immediate_stopped: false,
            prevented: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
            remove_listeners: Vec::new(),
        };
        anyos_std::println!(
            "[js] setup bridge ready: mutations={} listeners={} pending_http={} timers={}",
            bridge.mutations.len(),
            bridge.event_listeners.len(),
            bridge.pending_http_requests.len(),
            bridge.timers.len()
        );
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        anyos_std::println!(
            "[js] setup userdata installed: frames={} stack={} vm_userdata_set=true",
            self.engine.vm().frames.len(),
            self.engine.vm().stack.len()
        );

        // Set up native host objects (document, window, etc.) once per navigation.
        if !self.native_api_initialized || self.native_api_url != url {
            anyos_std::println!("[js] setup native api begin");
            self.setup_native_api(dom, url, &self.cookies.clone());
            self.native_api_initialized = true;
            self.native_api_url = String::from(url);
            anyos_std::println!(
                "[js] setup native api done: console_msgs={} engine_logs={}",
                self.engine.console_output().len(),
                self.engine.vm().engine_log.len()
            );
        } else {
            anyos_std::println!("[js] setup native api reuse: url={}", url);
            let doc = self.engine.vm().get_global("document");
            if !doc.is_undefined() {
                doc.set_property(String::from("cookie"), JsValue::String(self.cookies.clone()));
            }
        }

        // Enable property-write interception.
        anyos_std::println!("[js] setup mutation interception begin");
        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
        }
        anyos_std::println!(
            "[js] setup mutation interception done: mutation_target=true virtual_nodes_target=true"
        );

        // Execute each script (with limits to keep UI responsive).
        let script_count = scripts.len().min(MAX_SCRIPTS);
        for (idx, script) in scripts.iter().take(script_count).enumerate() {
            if script.len() > MAX_SCRIPT_BYTES {
                anyos_std::println!(
                    "[js] skipping script #{} ({} bytes — too large)",
                    idx,
                    script.len()
                );
                continue;
            }
            // Reset step counter and engine state before each script so each gets the full budget.
            anyos_std::println!(
                "[js] prepare #{} begin: bytes={} frames={} stack={} last_exc={} pending_exc={}",
                idx,
                script.len(),
                self.engine.vm().frames.len(),
                self.engine.vm().stack.len(),
                self.engine.vm().last_exception.is_some(),
                self.engine.vm().pending_exception.is_some()
            );
            self.engine.vm().steps = 0;
            self.engine.vm().last_exception = None;
            self.engine.set_step_limit(20_000_000);
            // Clear any leftover call frames from aborted scripts (e.g. step-limit abort).
            self.engine.vm().frames.clear();
            self.engine.vm().stack.clear();
            anyos_std::println!(
                "[js] prepare #{} reset done: frames={} stack={} step_limit={}",
                idx,
                self.engine.vm().frames.len(),
                self.engine.vm().stack.len(),
                self.engine.vm().step_limit
            );
            anyos_std::println!(
                "[js] eval #{}: {} bytes (frames={} stack={})",
                idx,
                script.len(),
                self.engine.vm().frames.len(),
                self.engine.vm().stack.len()
            );
            let result = self.engine.eval(script);

            // --- Per-script diagnostics ---
            let steps_used = self.engine.vm().steps;
            let hit_limit = steps_used > self.engine.vm().step_limit;
            if hit_limit {
                anyos_std::println!(
                    "[js] !! script #{} HIT STEP LIMIT ({}/{}) — execution aborted",
                    idx,
                    steps_used,
                    self.engine.vm().step_limit
                );
            } else {
                anyos_std::println!(
                    "[js] script #{} completed: {} steps (limit {})",
                    idx,
                    steps_used,
                    self.engine.vm().step_limit
                );
            }

            // Check for unhandled exceptions.
            if let Some(ref exc) = self.engine.vm().last_exception {
                let exc_str = match exc {
                    JsValue::String(s) => s.clone(),
                    JsValue::Object(obj) => {
                        let o = obj.borrow();
                        let name = match o.get("name") {
                            JsValue::String(s) => s,
                            _ => String::from("Error"),
                        };
                        let msg = match o.get("message") {
                            JsValue::String(s) => s,
                            _ => String::from("(no message)"),
                        };
                        alloc::format!("{}: {}", name, msg)
                    }
                    other => alloc::format!("{:?}", other),
                };
                anyos_std::println!("[js] !! script #{} EXCEPTION: {}", idx, exc_str);
                // Clear last_exception so next script can run fresh.
                self.engine.vm().last_exception = None;
            }

            // Print engine log messages from this script.
            {
                let logs = &self.engine.vm().engine_log;
                if !logs.is_empty() {
                    for log_msg in logs.iter() {
                        anyos_std::println!("[js] engine #{}: {}", idx, log_msg);
                    }
                    self.engine.vm().engine_log.clear();
                }
            }

            // Flush console output after each script.
            for msg in self.engine.console_output() {
                anyos_std::println!("[js] console #{}: {}", idx, msg);
                self.console.push(msg.clone());
            }
            self.engine.clear_console();

            // Print first 80 chars of result if not undefined.
            if !matches!(result, JsValue::Undefined) {
                let r = alloc::format!("{:?}", result);
                let truncated = if r.len() > 80 { &r[..80] } else { &r };
                anyos_std::println!("[js] result #{}: {}", idx, truncated);
            }
        }
        if scripts.len() > script_count {
            anyos_std::println!(
                "[js] skipped {} script(s) (limit={})",
                scripts.len() - script_count,
                MAX_SCRIPTS
            );
        }

        // Disable interception.
        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
        }

        self.mutations = bridge.mutations;
        self.virtual_nodes = bridge.virtual_nodes;
        self.event_listeners = bridge.event_listeners;
        self.pending_http_requests = bridge.pending_http_requests;
        self.next_timer_id = bridge.next_timer_id;
        self.timers.extend(bridge.timers);
        self.pending_ws_connects.extend(bridge.pending_ws_connects);
        self.pending_ws_sends.extend(bridge.pending_ws_sends);
        self.pending_ws_closes.extend(bridge.pending_ws_closes);
        self.ws_registry.extend(bridge.ws_registry);
        self.engine.vm().userdata = core::ptr::null_mut();
        crate::debug_surf!(
            "[js] execute_script_sources complete: {} mutations, {} listeners",
            self.mutations.len(),
            self.event_listeners.len()
        );
    }

    /// Execute all `<script>` tags in the DOM (inline only, skips external).
    ///
    /// This is the legacy method — prefer [`execute_script_sources`] with
    /// [`collect_script_entries`] for full external script support.
    ///
    /// * `url` — the current page URL, used to populate `window.location` /
    ///   `document.location` inside the JS environment.
    pub fn execute_scripts(&mut self, dom: &Dom, url: &str) {
        let entries = Self::collect_script_entries(dom);
        let scripts: Vec<String> = entries
            .into_iter()
            .filter_map(|e| match e {
                ScriptEntry::Inline { text, .. } => Some(text),
                ScriptEntry::External { .. } => None,
            })
            .collect();
        self.execute_script_sources(dom, url, &scripts);
    }

    /// Set up all native host objects — zero JS injection.
    ///
    /// * `url`     — current page URL (populates `window.location`).
    /// * `cookies` — cookie string for this domain (populates `document.cookie`).
    fn setup_native_api(&mut self, dom: &Dom, url: &str, cookies: &str) {
        let vm = self.engine.vm();
        anyos_std::println!("[js] setup native api: event-callbacks begin");

        // Event callback storage (only tiny bit of eval for array init).
        vm.set_global(
            "__eventCallbacks",
            JsValue::Array(Rc::new(RefCell::new(JsArray::new()))),
        );
        anyos_std::println!("[js] setup native api: event-callbacks done");

        // Create document object natively.
        anyos_std::println!("[js] setup native api: document make begin");
        let doc = document::make_document(vm, dom, url, cookies);
        anyos_std::println!("[js] setup native api: document make done");
        anyos_std::println!("[js] setup native api: document global begin");
        vm.set_global("document", doc.clone());
        anyos_std::println!("[js] setup native api: document global done");

        // Extract origin (scheme + "://" + host) for localStorage key isolation.
        anyos_std::println!("[js] setup native api: origin extract begin");
        let origin = extract_origin(url);
        anyos_std::println!("[js] setup native api: origin extract done: {}", origin);

        // Create window object natively.
        anyos_std::println!("[js] setup native api: window make begin");
        let win = window::make_window(vm, doc, &origin, self.viewport_width, self.viewport_height);
        anyos_std::println!("[js] setup native api: window make done");
        anyos_std::println!("[js] setup native api: global window begin");
        vm.set_global("window", win.clone());
        anyos_std::println!("[js] setup native api: global window done");
        anyos_std::println!("[js] setup native api: global self begin");
        vm.set_global("self", win.clone());
        anyos_std::println!("[js] setup native api: global self done");
        anyos_std::println!("[js] setup native api: global globalThis begin");
        vm.set_global("globalThis", win.clone());
        anyos_std::println!("[js] setup native api: global globalThis done");

        // In browsers, window IS the global object — properties on window are directly
        // accessible as global variables (e.g. `MutationObserver` === `window.MutationObserver`).
        // Explicitly mirror all window constructors/functions as top-level globals so that
        // modern bundlers (Vite/React) that reference them without the `window.` prefix work.
        for key in &[
            "Node",
            "Document",
            "DocumentFragment",
            "CharacterData",
            "DocumentType",
            "Text",
            "Comment",
            "Element",
            "HTMLElement",
            "CustomElementRegistry",
            "customElements",
            "MutationObserver",
            "ResizeObserver",
            "IntersectionObserver",
            "AbortController",
            "queueMicrotask",
            "TextEncoder",
            "TextDecoder",
            "URL",
            "URLSearchParams",
            "CustomEvent",
            "Event",
            "structuredClone",
            "DOMParser",
            "performance",
            "history",
            "location",
            "localStorage",
            "sessionStorage",
            "navigator",
            "screen",
            "matchMedia",
            "getSelection",
            "scrollTo",
            "scrollBy",
            "confirm",
            "prompt",
            "getCookie",
            "getParameterByName",
            "clearEventListeners",
            "getRequestUUID",
            "crypto",
            "__tcfapi",
            "__cmp",
            "__uspapi",
        ] {
            anyos_std::println!("[js] setup native api: mirror {} begin", key);
            let val = win.get_property(key);
            if !val.is_undefined() {
                vm.set_global(key, val);
                anyos_std::println!("[js] setup native api: mirror {} done", key);
            } else {
                anyos_std::println!("[js] setup native api: mirror {} skipped(undefined)", key);
            }
        }

        // Top-level constructors/functions from window.
        anyos_std::println!("[js] setup native api: top-level globals begin");
        vm.set_global("alert", native_fn("alert", window::native_alert));
        vm.set_global("fetch", native_fn("fetch", fetch::native_fetch));
        vm.set_global("XMLHttpRequest", xhr::make_xhr_constructor());
        vm.set_global("WebSocket", websocket::make_ws_constructor());
        vm.set_global("Headers", native_fn("Headers", fetch::native_headers_ctor));
        vm.set_global("Image", native_fn("Image", document::native_image_ctor));
        vm.set_global("FormData", native_fn("FormData", native_formdata_ctor));
        anyos_std::println!("[js] setup native api: top-level globals done");

        // Timer globals.
        anyos_std::println!("[js] setup native api: timer globals begin");
        vm.set_global("setTimeout", native_fn("setTimeout", native_set_timeout));
        vm.set_global("setInterval", native_fn("setInterval", native_set_interval));
        vm.set_global(
            "clearTimeout",
            native_fn("clearTimeout", native_clear_timeout),
        );
        vm.set_global(
            "clearInterval",
            native_fn("clearInterval", native_clear_interval),
        );
        vm.set_global(
            "requestAnimationFrame",
            native_fn("requestAnimationFrame", native_request_animation_frame),
        );
        vm.set_global(
            "cancelAnimationFrame",
            native_fn("cancelAnimationFrame", native_clear_timeout),
        );
        anyos_std::println!("[js] setup native api: timer globals done");
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
            immediate_stopped: false,
            prevented: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
            remove_listeners: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
        }
        let result = self.engine.eval(source);
        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
        }

        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.apply_remove_listeners(&bridge.remove_listeners);
        self.pending_http_requests
            .extend(bridge.pending_http_requests);
        self.next_timer_id = bridge.next_timer_id;
        self.timers.extend(bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();
        result
    }

    pub fn get_console(&self) -> &[String] {
        &self.console
    }
    pub fn clear_console(&mut self) {
        self.console.clear();
    }

    /// Reset the entire JS runtime for a new page navigation.
    /// Creates a fresh JS engine and clears all accumulated state
    /// (timers, event listeners, WebSocket connections, animations).
    pub fn reset(&mut self) {
        self.engine = JsEngine::new();
        self.console.clear();
        self.mutations.clear();
        self.event_listeners.clear();
        self.pending_http_requests.clear();
        self.timers.clear();
        self.next_timer_id = 1;
        self.cookies.clear();
        self.pending_ws_connects.clear();
        self.pending_ws_sends.clear();
        self.pending_ws_closes.clear();
        self.ws_registry.clear();
        self.active_animations.clear();
        self.active_transitions.clear();
        self.native_api_initialized = false;
        self.native_api_url.clear();
    }

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
            evt.set_property(
                String::from("type"),
                JsValue::String(String::from("message")),
            );
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
        self.ws_registry
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, v)| v.clone())
    }

    /// Remove a closed WebSocket from the registry.
    fn remove_ws(&mut self, id: u64) {
        self.ws_registry.retain(|(wid, _)| *wid != id);
    }

    /// Fire a WS callback (onopen/onmessage/onerror/onclose) through the VM.
    fn fire_ws_callback(&mut self, cb: JsValue, this: &JsValue, args: &[JsValue]) {
        if !matches!(cb, JsValue::Function(_)) {
            return;
        }
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
                    // Copy attributes from virtual node if they were set before insertion
                    let attrs: Vec<crate::dom::Attr> = self
                        .virtual_nodes
                        .iter()
                        .find(|vn| vn.id == *virtual_id)
                        .map(|vn| {
                            vn.attrs
                                .iter()
                                .map(|(k, v)| crate::dom::Attr {
                                    name: k.clone(),
                                    value: v.clone(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let real_id = dom.add_node(
                        NodeType::Element {
                            tag: real_tag,
                            attrs,
                        },
                        None,
                    );
                    id_map.insert(*virtual_id, real_id);
                }
                DomMutation::CreateTextNode { virtual_id, text } => {
                    let real_id = dom.add_node(NodeType::Text(text.clone()), None);
                    id_map.insert(*virtual_id, real_id);
                }
                DomMutation::SetAttribute {
                    node_id,
                    name,
                    value,
                } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        dom.set_attr(real_id, name, value);
                    }
                }
                DomMutation::RemoveAttribute { node_id, name } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        dom.remove_attr(real_id, name);
                    }
                }
                DomMutation::SetTextContent { node_id, text } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        dom.set_text(real_id, text);
                    }
                }
                DomMutation::AppendChild {
                    parent_id,
                    child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_child = resolve_id(*child_id, &id_map);
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.append_child(p, c);
                    }
                }
                DomMutation::RemoveChild {
                    parent_id,
                    child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_child = resolve_id(*child_id, &id_map);
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.remove_child(p, c);
                    }
                }
                DomMutation::InsertBefore {
                    parent_id,
                    new_child_id,
                    ref_child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map);
                    let real_new = resolve_id(*new_child_id, &id_map);
                    let real_ref = resolve_id(*ref_child_id, &id_map);
                    if let (Some(p), Some(n), Some(r)) = (real_parent, real_new, real_ref) {
                        dom.insert_before(p, n, r);
                    }
                }
                DomMutation::ReplaceChild {
                    parent_id,
                    new_child_id,
                    old_child_id,
                } => {
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
                        let children: Vec<usize> = dom
                            .nodes
                            .get(real_id)
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
                DomMutation::SetStyleProperty {
                    node_id,
                    property,
                    value,
                } => {
                    // Store style as a `style` attribute for now.
                    if let Some(real_id) = resolve_id(*node_id, &id_map) {
                        let existing = String::from(dom.attr(real_id, "style").unwrap_or(""));
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
                DomMutation::SetScrollTop { .. } | DomMutation::SetScrollLeft { .. } => {
                    // Scroll offset mutations do not modify the DOM tree.
                    // They are consumed by WebView::apply_scroll_offsets()
                    // and applied to LayoutBox.scroll_top/scroll_left before rendering.
                }
                DomMutation::FormSubmit { .. } | DomMutation::FormReset { .. } => {
                    // Form actions do not modify the DOM tree.
                    // They are consumed by WebView::drain_form_submits/resets().
                }
            }
        }
        id_map
    }

    /// Dispatch an event per the W3C DOM Events Level 3 algorithm (§10.3).
    ///
    /// Three phases in order:
    /// 1. **Capture** (eventPhase=1) — listeners registered with `capture:true`,
    ///    fired from the document root down to the parent of `node_id`.
    /// 2. **At-target** (eventPhase=2) — all listeners on `node_id` itself,
    ///    regardless of the capture flag.
    /// 3. **Bubble** (eventPhase=3) — listeners registered with `capture:false`,
    ///    fired from the parent of `node_id` up to the root.
    ///    Skipped when `bubbles` is false (focus, blur, scroll, load, …).
    ///
    /// Returns `true` when the default action should proceed (`preventDefault()`
    /// was not called), `false` when it was cancelled.
    pub fn dispatch_event(
        &mut self,
        dom: &Dom,
        node_id: usize,
        event_name: &str,
        data: &EventData,
    ) -> bool {
        // Build the event path root-first: [root, …, parent, target].
        // Per spec §10.3 step 3 this is the "event path" used for all three phases.
        let path: Vec<usize> = {
            let mut chain = Vec::new();
            let mut cur = Some(node_id);
            while let Some(id) = cur {
                chain.push(id);
                cur = dom.nodes.get(id).and_then(|n| n.parent);
            }
            chain.reverse(); // now root → target
            chain
        };
        let target_idx = path.len().saturating_sub(1);

        // Fast exit: skip work entirely when no registered listener matches.
        let has_any = path.iter().any(|&nid| {
            self.event_listeners
                .iter()
                .any(|l| l.node_id == nid && l.event == event_name)
        });
        if !has_any {
            return true;
        }

        // Whether this event type bubbles by default.
        // (focus/blur/scroll/load do not bubble — they have focused-capture semantics.)
        let bubbles = !matches!(
            event_name,
            "focus" | "blur" | "scroll" | "load" | "unload" | "error"
        );
        let cancelable = !matches!(event_name, "scroll" | "load" | "unload");

        // Build the event object with full W3C properties.
        let target_el = element::make_element(self.engine.vm(), node_id as i64);
        let evt = build_event_object(event_name, data, target_el, bubbles, cancelable);

        // Set up bridge so DOM-access and event-control native functions work.
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
            immediate_stopped: false,
            prevented: false,
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
            remove_listeners: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
        }

        // ── Phase 1: CAPTURE (eventPhase = 1) ───────────────────────────────
        // Fire capture-listeners from root down to (but not including) the target.
        'capture: for i in 0..target_idx {
            if bridge.propagation_stopped || bridge.immediate_stopped {
                break;
            }
            let nid = path[i];
            let cur_el = element::make_element(self.engine.vm(), nid as i64);
            evt.set_property(String::from("currentTarget"), cur_el);
            evt.set_property(String::from("eventPhase"), JsValue::Number(1.0));

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == nid && l.event == event_name && l.capture)
                .map(|l| l.callback.clone())
                .collect();

            for cb in &matching {
                self.engine
                    .vm()
                    .call_value(cb, &[evt.clone()], JsValue::Undefined);
                evt.set_property(
                    String::from("defaultPrevented"),
                    JsValue::Bool(bridge.prevented),
                );
                if bridge.immediate_stopped {
                    break 'capture;
                }
                if bridge.propagation_stopped {
                    break 'capture;
                }
            }
        }

        // ── Phase 2: AT TARGET (eventPhase = 2) ─────────────────────────────
        // All listeners on the target node fire, both capture and bubble.
        if !bridge.propagation_stopped && !bridge.immediate_stopped {
            let cur_el = element::make_element(self.engine.vm(), node_id as i64);
            evt.set_property(String::from("currentTarget"), cur_el);
            evt.set_property(String::from("eventPhase"), JsValue::Number(2.0));

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == node_id && l.event == event_name)
                .map(|l| l.callback.clone())
                .collect();

            'target: for cb in &matching {
                self.engine
                    .vm()
                    .call_value(cb, &[evt.clone()], JsValue::Undefined);
                evt.set_property(
                    String::from("defaultPrevented"),
                    JsValue::Bool(bridge.prevented),
                );
                if bridge.immediate_stopped {
                    break 'target;
                }
                // stopPropagation at target does NOT prevent remaining at-target
                // listeners per spec §10.3 step 6.3.  Only stopImmediatePropagation does.
            }
        }

        // ── Phase 3: BUBBLE (eventPhase = 3) ────────────────────────────────
        // Bubble-listeners from parent up to root.  Skipped for non-bubbling events.
        if bubbles && !bridge.propagation_stopped && !bridge.immediate_stopped && target_idx > 0 {
            'bubble: for i in (0..target_idx).rev() {
                let nid = path[i];
                let cur_el = element::make_element(self.engine.vm(), nid as i64);
                evt.set_property(String::from("currentTarget"), cur_el);
                evt.set_property(String::from("eventPhase"), JsValue::Number(3.0));

                let matching: Vec<JsValue> = self
                    .event_listeners
                    .iter()
                    .filter(|l| l.node_id == nid && l.event == event_name && !l.capture)
                    .map(|l| l.callback.clone())
                    .collect();

                for cb in &matching {
                    self.engine
                        .vm()
                        .call_value(cb, &[evt.clone()], JsValue::Undefined);
                    evt.set_property(
                        String::from("defaultPrevented"),
                        JsValue::Bool(bridge.prevented),
                    );
                    if bridge.immediate_stopped || bridge.propagation_stopped {
                        break 'bubble;
                    }
                }
            }
        }

        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
        }

        // Drain microtask queue after event dispatch (ECMAScript spec:
        // microtasks run to completion after each task/callback).
        self.engine.vm().drain_microtasks();

        // Collect all side-effects from the dispatch.
        for msg in self.engine.console_output() {
            self.console.push(msg.clone());
        }
        self.engine.clear_console();
        self.mutations.extend(bridge.mutations);
        self.event_listeners.extend(bridge.event_listeners);
        self.apply_remove_listeners(&bridge.remove_listeners);
        self.pending_http_requests
            .extend(bridge.pending_http_requests);
        self.next_timer_id = bridge.next_timer_id;
        self.timers.extend(bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();

        // Return true when preventDefault() was NOT called (default action proceeds).
        !bridge.prevented
    }

    /// Advance timers by `delta_ms` and execute any that are due.
    /// Returns the number of timers fired.
    pub fn tick(&mut self, dom: &Dom, delta_ms: u64) -> usize {
        self.total_elapsed_ms += delta_ms;

        // Short-circuit: no allocation or work when there are no timers.
        if self.timers.is_empty() {
            return 0;
        }

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
                    immediate_stopped: false,
                    prevented: false,
                    pending_ws_connects: Vec::new(),
                    pending_ws_sends: Vec::new(),
                    pending_ws_closes: Vec::new(),
                    ws_registry: Vec::new(),
                    remove_listeners: Vec::new(),
                };
                self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
                unsafe {
                    MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
                    VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
                }

                // Timer callbacks get a generous step budget for React initial renders.
                self.engine.set_step_limit(5_000_000);
                self.engine.vm().steps = 0;
                // rAF callbacks receive a DOMHighResTimeStamp (W3C spec).
                let cb_args: Vec<JsValue> = if t.is_raf {
                    vec![JsValue::Number(self.total_elapsed_ms as f64)]
                } else {
                    Vec::new()
                };
                self.engine
                    .vm()
                    .call_value(&t.callback, &cb_args, JsValue::Undefined);

                // Drain microtask queue after each macrotask (ECMAScript spec:
                // all microtasks run to completion before the next macrotask).
                self.engine.vm().drain_microtasks();

                // Clear any timer callback exceptions so next timer can run fresh.
                self.engine.vm().last_exception = None;

                unsafe {
                    MUTATION_TARGET = core::ptr::null_mut();
                    VIRTUAL_NODES_TARGET = core::ptr::null_mut();
                }
                for msg in self.engine.console_output() {
                    self.console.push(msg.clone());
                }
                self.engine.clear_console();
                // Print engine log from timer callback for diagnostics
                for log_msg in self.engine.vm().engine_log.iter() {
                    anyos_std::println!("[js] timer: {}", log_msg);
                }
                self.engine.vm().engine_log.clear();
                self.mutations.extend(bridge.mutations);
                self.event_listeners.extend(bridge.event_listeners);
                self.pending_http_requests
                    .extend(bridge.pending_http_requests);
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

    /// Apply pending removeEventListener requests collected during JS execution.
    fn apply_remove_listeners(&mut self, removals: &[(usize, String, JsValue, bool)]) {
        for (node_id, event, callback, capture) in removals {
            if let Some(pos) = self.event_listeners.iter().position(|l| {
                l.node_id == *node_id
                    && l.event == *event
                    && l.capture == *capture
                    && l.callback.strict_eq(callback)
            }) {
                self.event_listeners.remove(pos);
            }
        }
    }

    pub fn engine(&mut self) -> &mut JsEngine {
        &mut self.engine
    }

    /// Detect CSS property changes between old and new computed styles and
    /// start `ActiveTransition` entries for properties that have a
    /// `transition` definition and whose values changed.
    ///
    /// Call this after re-resolving styles, passing both old and new style
    /// arrays.  Only nodes present in both arrays are compared.
    pub fn start_transitions(
        &mut self,
        old_styles: &[crate::style::ComputedStyle],
        new_styles: &[crate::style::ComputedStyle],
    ) {
        let count = old_styles.len().min(new_styles.len());
        for node_id in 0..count {
            let old_s = &old_styles[node_id];
            let new_s = &new_styles[node_id];
            if new_s.transitions.is_empty() {
                continue;
            }

            // For each transition definition on this node, check if the
            // corresponding property changed between old and new styles.
            for tdef in &new_s.transitions {
                if tdef.duration_ms == 0 {
                    continue;
                }

                // "all" means every animatable property.
                let props: Vec<Property> = if tdef.property == "all" {
                    ANIMATABLE_PROPERTIES.to_vec()
                } else if let Some(p) = crate::css::parse_property(&tdef.property) {
                    vec![p]
                } else {
                    continue;
                };

                for prop in &props {
                    let old_decl = computed_style_to_decl(old_s, prop);
                    let new_decl = computed_style_to_decl(new_s, prop);

                    // Both must exist and differ.
                    let (from, to) = match (old_decl, new_decl) {
                        (Some(f), Some(t)) => {
                            if f.value == t.value {
                                continue;
                            }
                            (f, t)
                        }
                        _ => continue,
                    };

                    // Don't start a duplicate transition for the same node + property.
                    let dominated = core::mem::discriminant(&to.property);
                    let already = self.active_transitions.iter().any(|tr| {
                        tr.node_id == node_id
                            && core::mem::discriminant(&tr.to_decl.property) == dominated
                    });
                    if already {
                        continue;
                    }

                    self.active_transitions.push(ActiveTransition {
                        node_id,
                        property: tdef.property.clone(),
                        duration_ms: tdef.duration_ms,
                        timing: tdef.timing,
                        delay_ms: tdef.delay_ms,
                        elapsed_ms: 0,
                        from_decl: Some(from),
                        to_decl: to,
                    });
                }
            }
        }
    }

    /// Register `@keyframes` animation starts for every node whose computed
    /// style requests an animation that is not already running.
    ///
    /// Call this after `execute_scripts()` / relayout when styles change.
    pub fn start_animations(&mut self, styles: &[crate::style::ComputedStyle]) {
        for (node_id, style) in styles.iter().enumerate() {
            'anim: for adef in &style.animations {
                if adef.name.is_empty() || adef.duration_ms == 0 {
                    continue;
                }
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
            if dur == 0 {
                continue;
            }

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

            // Derive the current iteration from total elapsed time.
            let new_iter = (anim_elapsed / dur) as u32;
            if new_iter > anim.current_iteration {
                anim.current_iteration = new_iter;
            }
            let finished =
                anim.iteration_count != 0 && anim.current_iteration >= anim.iteration_count;

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
            if tr.duration_ms == 0 {
                continue;
            }
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
    args.get(index)
        .map(|v| v.to_js_string())
        .unwrap_or_else(String::new)
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
                if k == name {
                    return JsValue::String(v.clone());
                }
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
                return dom
                    .get(nid)
                    .children
                    .iter()
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

/// Read all direct child node IDs, including text nodes.
fn read_all_child_node_ids(vm: &mut Vm, node_id: i64) -> Vec<i64> {
    if let Some(bridge) = get_bridge(vm) {
        if node_id >= 0 {
            let dom = bridge.dom();
            let nid = node_id as usize;
            if nid < dom.nodes.len() {
                return dom
                    .get(nid)
                    .children
                    .iter()
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
    if node_id < 0 {
        return String::new();
    }
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
    evt.set_property(String::from("type"), JsValue::String(String::from("close")));
    evt.set_property(String::from("code"), JsValue::Number(code as f64));
    evt.set_property(
        String::from("reason"),
        JsValue::String(String::from(reason)),
    );
    evt.set_property(String::from("wasClean"), JsValue::Bool(was_clean));
    evt
}

// ═══════════════════════════════════════════════════════════
// Event object builder
// ═══════════════════════════════════════════════════════════

/// Build a fully-populated W3C event object for `dispatch_event`.
///
/// Sets the common `Event` interface properties and then overlays the
/// type-specific properties from `data` (MouseEvent, KeyboardEvent, …).
fn build_event_object(
    event_name: &str,
    data: &EventData,
    target: JsValue,
    bubbles: bool,
    cancelable: bool,
) -> JsValue {
    let evt = JsValue::new_object();

    // ── W3C Event interface (common to all events) ───────────────────────
    evt.set_property(
        String::from("type"),
        JsValue::String(String::from(event_name)),
    );
    evt.set_property(String::from("target"), target.clone());
    evt.set_property(String::from("currentTarget"), target);
    evt.set_property(String::from("bubbles"), JsValue::Bool(bubbles));
    evt.set_property(String::from("cancelable"), JsValue::Bool(cancelable));
    evt.set_property(String::from("defaultPrevented"), JsValue::Bool(false));
    evt.set_property(String::from("composed"), JsValue::Bool(true));
    evt.set_property(String::from("isTrusted"), JsValue::Bool(true));
    evt.set_property(String::from("timeStamp"), JsValue::Number(0.0));
    // eventPhase: 0=NONE before dispatch; updated per phase inside dispatch_event.
    evt.set_property(String::from("eventPhase"), JsValue::Number(0.0));
    evt.set_property(String::from("NONE"), JsValue::Number(0.0));
    evt.set_property(String::from("CAPTURING_PHASE"), JsValue::Number(1.0));
    evt.set_property(String::from("AT_TARGET"), JsValue::Number(2.0));
    evt.set_property(String::from("BUBBLING_PHASE"), JsValue::Number(3.0));

    // Control methods — implementations read/write DomBridge via vm.userdata.
    evt.set_property(
        String::from("preventDefault"),
        native_fn("preventDefault", native_prevent_default),
    );
    evt.set_property(
        String::from("stopPropagation"),
        native_fn("stopPropagation", native_stop_propagation),
    );
    evt.set_property(
        String::from("stopImmediatePropagation"),
        native_fn(
            "stopImmediatePropagation",
            native_stop_immediate_propagation,
        ),
    );
    evt.set_property(
        String::from("composedPath"),
        native_fn("composedPath", |_, _| make_array(Vec::new())),
    );

    // ── Type-specific properties ─────────────────────────────────────────
    match data {
        EventData::None => {}

        EventData::Mouse {
            client_x,
            client_y,
            page_x,
            page_y,
            screen_x,
            screen_y,
            offset_x,
            offset_y,
            button,
            buttons,
            ctrl_key,
            shift_key,
            alt_key,
            meta_key,
        } => {
            evt.set_property(String::from("clientX"), JsValue::Number(*client_x));
            evt.set_property(String::from("clientY"), JsValue::Number(*client_y));
            evt.set_property(String::from("pageX"), JsValue::Number(*page_x));
            evt.set_property(String::from("pageY"), JsValue::Number(*page_y));
            evt.set_property(String::from("screenX"), JsValue::Number(*screen_x));
            evt.set_property(String::from("screenY"), JsValue::Number(*screen_y));
            evt.set_property(String::from("offsetX"), JsValue::Number(*offset_x));
            evt.set_property(String::from("offsetY"), JsValue::Number(*offset_y));
            evt.set_property(String::from("x"), JsValue::Number(*client_x));
            evt.set_property(String::from("y"), JsValue::Number(*client_y));
            evt.set_property(String::from("button"), JsValue::Number(*button as f64));
            evt.set_property(String::from("buttons"), JsValue::Number(*buttons as f64));
            evt.set_property(String::from("ctrlKey"), JsValue::Bool(*ctrl_key));
            evt.set_property(String::from("shiftKey"), JsValue::Bool(*shift_key));
            evt.set_property(String::from("altKey"), JsValue::Bool(*alt_key));
            evt.set_property(String::from("metaKey"), JsValue::Bool(*meta_key));
            evt.set_property(String::from("movementX"), JsValue::Number(0.0));
            evt.set_property(String::from("movementY"), JsValue::Number(0.0));
            evt.set_property(String::from("relatedTarget"), JsValue::Null);
        }

        EventData::Keyboard {
            key,
            code,
            key_code,
            which,
            char_code,
            ctrl_key,
            shift_key,
            alt_key,
            meta_key,
            repeat,
            is_composing,
        } => {
            evt.set_property(String::from("key"), JsValue::String(key.clone()));
            evt.set_property(String::from("code"), JsValue::String(code.clone()));
            evt.set_property(String::from("keyCode"), JsValue::Number(*key_code as f64));
            evt.set_property(String::from("which"), JsValue::Number(*which as f64));
            evt.set_property(String::from("charCode"), JsValue::Number(*char_code as f64));
            evt.set_property(String::from("ctrlKey"), JsValue::Bool(*ctrl_key));
            evt.set_property(String::from("shiftKey"), JsValue::Bool(*shift_key));
            evt.set_property(String::from("altKey"), JsValue::Bool(*alt_key));
            evt.set_property(String::from("metaKey"), JsValue::Bool(*meta_key));
            evt.set_property(String::from("repeat"), JsValue::Bool(*repeat));
            evt.set_property(String::from("isComposing"), JsValue::Bool(*is_composing));
            evt.set_property(String::from("location"), JsValue::Number(0.0));
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
            // getModifierState stub.
            evt.set_property(
                String::from("getModifierState"),
                native_fn("getModifierState", |_, _| JsValue::Bool(false)),
            );
        }

        EventData::Input {
            data: input_data,
            input_type,
            is_composing,
        } => {
            evt.set_property(
                String::from("data"),
                match input_data {
                    Some(s) => JsValue::String(s.clone()),
                    None => JsValue::Null,
                },
            );
            evt.set_property(
                String::from("inputType"),
                JsValue::String(input_type.clone()),
            );
            evt.set_property(String::from("isComposing"), JsValue::Bool(*is_composing));
            // dataTransfer is null for plain text input.
            evt.set_property(String::from("dataTransfer"), JsValue::Null);
        }

        EventData::Focus { related_target_id } => {
            // relatedTarget is the element losing focus (focusin) or gaining it (focusout).
            evt.set_property(String::from("relatedTarget"), JsValue::Null);
            let _ = related_target_id; // exposed as Null for now; caller may set via DOM
        }

        EventData::Wheel {
            delta_x,
            delta_y,
            delta_z,
            delta_mode,
            client_x,
            client_y,
            ctrl_key,
            shift_key,
            alt_key,
            meta_key,
        } => {
            evt.set_property(String::from("deltaX"), JsValue::Number(*delta_x));
            evt.set_property(String::from("deltaY"), JsValue::Number(*delta_y));
            evt.set_property(String::from("deltaZ"), JsValue::Number(*delta_z));
            evt.set_property(
                String::from("deltaMode"),
                JsValue::Number(*delta_mode as f64),
            );
            evt.set_property(String::from("DOM_DELTA_PIXEL"), JsValue::Number(0.0));
            evt.set_property(String::from("DOM_DELTA_LINE"), JsValue::Number(1.0));
            evt.set_property(String::from("DOM_DELTA_PAGE"), JsValue::Number(2.0));
            // WheelEvent extends MouseEvent.
            evt.set_property(String::from("clientX"), JsValue::Number(*client_x));
            evt.set_property(String::from("clientY"), JsValue::Number(*client_y));
            evt.set_property(String::from("ctrlKey"), JsValue::Bool(*ctrl_key));
            evt.set_property(String::from("shiftKey"), JsValue::Bool(*shift_key));
            evt.set_property(String::from("altKey"), JsValue::Bool(*alt_key));
            evt.set_property(String::from("metaKey"), JsValue::Bool(*meta_key));
            evt.set_property(String::from("button"), JsValue::Number(0.0));
            evt.set_property(String::from("buttons"), JsValue::Number(0.0));
        }

        EventData::Pointer {
            client_x,
            client_y,
            page_x,
            page_y,
            screen_x,
            screen_y,
            pointer_id,
            pointer_type,
            pressure,
            tilt_x,
            tilt_y,
            is_primary,
            button,
            buttons,
            ctrl_key,
            shift_key,
            alt_key,
            meta_key,
        } => {
            // PointerEvent extends MouseEvent.
            evt.set_property(String::from("clientX"), JsValue::Number(*client_x));
            evt.set_property(String::from("clientY"), JsValue::Number(*client_y));
            evt.set_property(String::from("pageX"), JsValue::Number(*page_x));
            evt.set_property(String::from("pageY"), JsValue::Number(*page_y));
            evt.set_property(String::from("screenX"), JsValue::Number(*screen_x));
            evt.set_property(String::from("screenY"), JsValue::Number(*screen_y));
            evt.set_property(String::from("button"), JsValue::Number(*button as f64));
            evt.set_property(String::from("buttons"), JsValue::Number(*buttons as f64));
            evt.set_property(String::from("ctrlKey"), JsValue::Bool(*ctrl_key));
            evt.set_property(String::from("shiftKey"), JsValue::Bool(*shift_key));
            evt.set_property(String::from("altKey"), JsValue::Bool(*alt_key));
            evt.set_property(String::from("metaKey"), JsValue::Bool(*meta_key));
            // PointerEvent-specific.
            evt.set_property(
                String::from("pointerId"),
                JsValue::Number(*pointer_id as f64),
            );
            evt.set_property(
                String::from("pointerType"),
                JsValue::String(pointer_type.clone()),
            );
            evt.set_property(String::from("pressure"), JsValue::Number(*pressure));
            evt.set_property(String::from("tangentialPressure"), JsValue::Number(0.0));
            evt.set_property(String::from("tiltX"), JsValue::Number(*tilt_x));
            evt.set_property(String::from("tiltY"), JsValue::Number(*tilt_y));
            evt.set_property(String::from("twist"), JsValue::Number(0.0));
            evt.set_property(String::from("isPrimary"), JsValue::Bool(*is_primary));
            evt.set_property(String::from("width"), JsValue::Number(1.0));
            evt.set_property(String::from("height"), JsValue::Number(1.0));
            evt.set_property(String::from("relatedTarget"), JsValue::Null);
            evt.set_property(
                String::from("getCoalescedEvents"),
                native_fn("getCoalescedEvents", |_, _| make_array(Vec::new())),
            );
            evt.set_property(
                String::from("getPredictedEvents"),
                native_fn("getPredictedEvents", |_, _| make_array(Vec::new())),
            );
        }
    }

    evt
}

// ═══════════════════════════════════════════════════════════
// Native event control functions
// ═══════════════════════════════════════════════════════════

/// `event.preventDefault()` — marks the event as cancelled.
///
/// Sets `DomBridge.prevented = true` so `dispatch_event` returns `false`
/// and the caller knows not to execute the default browser action.
fn native_prevent_default(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        bridge.prevented = true;
    }
    JsValue::Undefined
}

/// `event.stopPropagation()` — stops the event from moving to the next node.
///
/// Remaining listeners on the *current* node still fire.
/// Per W3C DOM Events §10.3 step 6.3.
fn native_stop_propagation(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        bridge.propagation_stopped = true;
    }
    JsValue::Undefined
}

/// Generic `removeEventListener(event, callback [, capture])` native function.
///
/// Schedules removal via `DomBridge.remove_listeners`.  The actual removal
/// from `JsRuntime.event_listeners` happens after execution completes.
fn native_remove_event_listener(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let event = arg_string(args, 0);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let capture = match args.get(2) {
        Some(JsValue::Bool(b)) => *b,
        Some(JsValue::Object(_)) => args[2].get_property("capture").to_boolean(),
        _ => false,
    };
    let nid = this_node_id(vm);
    let node_id = if nid >= 0 { nid as usize } else { usize::MAX };
    if let Some(bridge) = get_bridge(vm) {
        bridge
            .remove_listeners
            .push((node_id, event, callback, capture));
    }
    JsValue::Undefined
}

/// `event.stopImmediatePropagation()` — stops all further listeners immediately.
///
/// Unlike `stopPropagation`, this also prevents remaining listeners on the
/// *current* node from being called.  Per W3C DOM Events §10.3 step 6.4.
fn native_stop_immediate_propagation(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let Some(bridge) = get_bridge(vm) {
        bridge.propagation_stopped = true;
        bridge.immediate_stopped = true;
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
// ═══════════════════════════════════════════════════════════
// FormData API (W3C XMLHttpRequest §5)
// ═══════════════════════════════════════════════════════════

/// `new FormData()` or `new FormData(formElement)`.
/// Stores entries as an internal `__entries` array of [name, value] pairs on the object.
fn native_formdata_ctor(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let mut obj = JsObject::new();
    // Store entries as __entries: [[name, value], ...]
    obj.set(String::from("__entries"), JsValue::new_array(Vec::new()));
    obj.set(String::from("append"), native_fn("append", formdata_append));
    obj.set(String::from("set"), native_fn("set", formdata_set));
    obj.set(String::from("get"), native_fn("get", formdata_get));
    obj.set(String::from("getAll"), native_fn("getAll", formdata_get_all));
    obj.set(String::from("has"), native_fn("has", formdata_has));
    obj.set(String::from("delete"), native_fn("delete", formdata_delete));
    obj.set(String::from("entries"), native_fn("entries", formdata_entries));
    obj.set(String::from("keys"), native_fn("keys", formdata_keys));
    obj.set(String::from("values"), native_fn("values", formdata_values));
    obj.set(String::from("forEach"), native_fn("forEach", formdata_foreach));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// Helper: read all [name, value] pairs from FormData's __entries.
fn formdata_read_entries(vm: &Vm) -> Vec<(String, String)> {
    let this = vm.current_this.clone();
    if let JsValue::Object(ref obj_rc) = this {
        if let Some(prop) = obj_rc.borrow().properties.get("__entries") {
            if let JsValue::Array(ref arr_rc) = prop.value {
                let arr = arr_rc.borrow();
                let mut entries = Vec::new();
                for i in 0..arr.length {
                    if let Some(elem) = arr.elements.get(&i) {
                        if let JsValue::Array(ref pair_rc) = elem {
                            let pair = pair_rc.borrow();
                            let name = pair.elements.get(&0).map(|v| v.to_js_string()).unwrap_or_default();
                            let value = pair.elements.get(&1).map(|v| v.to_js_string()).unwrap_or_default();
                            entries.push((name, value));
                        }
                    }
                }
                return entries;
            }
        }
    }
    Vec::new()
}

/// Helper: write entries back to FormData's __entries.
fn formdata_write_entries(vm: &Vm, entries: &[(String, String)]) {
    let this = vm.current_this.clone();
    if let JsValue::Object(ref obj_rc) = this {
        let pairs: Vec<JsValue> = entries
            .iter()
            .map(|(n, v)| {
                JsValue::new_array(alloc::vec![
                    JsValue::String(n.clone()),
                    JsValue::String(v.clone()),
                ])
            })
            .collect();
        obj_rc.borrow_mut().set(String::from("__entries"), JsValue::new_array(pairs));
    }
}

fn formdata_append(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let value = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    let mut entries = formdata_read_entries(vm);
    entries.push((name, value));
    formdata_write_entries(vm, &entries);
    JsValue::Undefined
}

fn formdata_set(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let value = args.get(1).map(|v| v.to_js_string()).unwrap_or_default();
    let mut entries = formdata_read_entries(vm);
    entries.retain(|(n, _)| n != &name);
    entries.push((name, value));
    formdata_write_entries(vm, &entries);
    JsValue::Undefined
}

fn formdata_get(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let entries = formdata_read_entries(vm);
    for (n, v) in &entries {
        if n == &name {
            return JsValue::String(v.clone());
        }
    }
    JsValue::Null
}

fn formdata_get_all(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let entries = formdata_read_entries(vm);
    let vals: Vec<JsValue> = entries
        .iter()
        .filter(|(n, _)| n == &name)
        .map(|(_, v)| JsValue::String(v.clone()))
        .collect();
    JsValue::new_array(vals)
}

fn formdata_has(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let entries = formdata_read_entries(vm);
    JsValue::Bool(entries.iter().any(|(n, _)| n == &name))
}

fn formdata_delete(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let name = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let mut entries = formdata_read_entries(vm);
    entries.retain(|(n, _)| n != &name);
    formdata_write_entries(vm, &entries);
    JsValue::Undefined
}

fn formdata_entries(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let entries = formdata_read_entries(vm);
    let pairs: Vec<JsValue> = entries
        .iter()
        .map(|(n, v)| {
            JsValue::new_array(alloc::vec![
                JsValue::String(n.clone()),
                JsValue::String(v.clone()),
            ])
        })
        .collect();
    JsValue::new_array(pairs)
}

fn formdata_keys(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let entries = formdata_read_entries(vm);
    let keys: Vec<JsValue> = entries.iter().map(|(n, _)| JsValue::String(n.clone())).collect();
    JsValue::new_array(keys)
}

fn formdata_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let entries = formdata_read_entries(vm);
    let vals: Vec<JsValue> = entries.iter().map(|(_, v)| JsValue::String(v.clone())).collect();
    JsValue::new_array(vals)
}

fn formdata_foreach(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
    let entries = formdata_read_entries(vm);
    for (n, v) in entries {
        vm.call_value(
            &cb,
            &[JsValue::String(v), JsValue::String(n)],
            JsValue::Undefined,
        );
    }
    JsValue::Undefined
}

// ═══════════════════════════════════════════════════════════
// Native timer functions
// ═══════════════════════════════════════════════════════════

fn native_set_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args
        .get(1)
        .map(|v| v.to_number().max(0.0) as u64)
        .unwrap_or(0);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        bridge.timers.push(PendingTimer {
            id,
            callback,
            delay_ms: delay,
            repeat: false,
            elapsed_ms: 0,
            is_raf: false,
        });
        return JsValue::Number(id as f64);
    }
    JsValue::Number(0.0)
}

fn native_set_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args
        .get(1)
        .map(|v| v.to_number().max(1.0) as u64)
        .unwrap_or(10);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        bridge.timers.push(PendingTimer {
            id,
            callback,
            delay_ms: delay,
            repeat: true,
            elapsed_ms: 0,
            is_raf: false,
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
    // Treat as a ~16ms setTimeout (60fps) with DOMHighResTimeStamp callback.
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
            is_raf: true,
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
    if kf.stops.is_empty() {
        return Vec::new();
    }

    let t_pct = t / 10; // map 0–1000 → 0–100

    // Find the two surrounding stops (stops are sorted by offset 0–100).
    let mut prev_idx = 0usize;
    let mut next_idx = 0usize;
    for (i, stop) in kf.stops.iter().enumerate() {
        if stop.offset <= t_pct {
            prev_idx = i;
        }
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
        let from_decl = prev.declarations.iter().find(|d| {
            core::mem::discriminant(&d.property) == core::mem::discriminant(&next_decl.property)
        });
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
        (Some(CssValue::Number(a)), CssValue::Number(b)) => CssValue::Number(lerp_i32(*a, *b, t)),
        (Some(CssValue::Length(a, ua)), CssValue::Length(b, ub)) if ua == ub => {
            CssValue::Length(lerp_i32(*a, *b, t), *ub)
        }
        (Some(CssValue::Percentage(a)), CssValue::Percentage(b)) => {
            CssValue::Percentage(lerp_i32(*a, *b, t))
        }
        (Some(CssValue::Color(a)), CssValue::Color(b)) => CssValue::Color(lerp_color(*a, *b, t)),
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
    let la = [
        (a >> 24) & 0xFF,
        (a >> 16) & 0xFF,
        (a >> 8) & 0xFF,
        a & 0xFF,
    ];
    let lb = [
        (b >> 24) & 0xFF,
        (b >> 16) & 0xFF,
        (b >> 8) & 0xFF,
        b & 0xFF,
    ];
    let mut out = 0u32;
    for i in 0..4 {
        let v = lerp_i32(la[i] as i32 * 100, lb[i] as i32 * 100, t) / 100;
        out = (out << 8) | (v.clamp(0, 255) as u32);
    }
    out
}

// ═══════════════════════════════════════════════════════════
// CSS Transition helpers
// ═══════════════════════════════════════════════════════════

use crate::css::{CssValue, Property, Unit};

/// Properties that are commonly animatable via CSS transitions.
/// When `transition-property: all` is used, we check these.
const ANIMATABLE_PROPERTIES: &[Property] = &[
    Property::Opacity,
    Property::Color,
    Property::BackgroundColor,
    Property::BorderColor,
    Property::Width,
    Property::Height,
    Property::MaxWidth,
    Property::MaxHeight,
    Property::MinWidth,
    Property::MinHeight,
    Property::MarginTop,
    Property::MarginRight,
    Property::MarginBottom,
    Property::MarginLeft,
    Property::PaddingTop,
    Property::PaddingRight,
    Property::PaddingBottom,
    Property::PaddingLeft,
    Property::BorderWidth,
    Property::BorderRadius,
    Property::FontSize,
    Property::LineHeight,
    Property::Top,
    Property::Right,
    Property::Bottom,
    Property::Left,
    Property::FlexGrow,
    Property::FlexShrink,
    Property::LetterSpacing,
    Property::WordSpacing,
    Property::TextIndent,
    Property::RowGap,
    Property::ColumnGap,
    Property::Order,
    Property::ZIndex,
];

/// Extract a `Declaration` from a `ComputedStyle` for a given `Property`.
///
/// Returns `None` for properties we don't track / can't interpolate.
fn computed_style_to_decl(
    s: &crate::style::ComputedStyle,
    prop: &Property,
) -> Option<crate::css::Declaration> {
    let value = match prop {
        Property::Opacity => CssValue::Number(s.opacity),
        Property::Color => CssValue::Color(s.color),
        Property::BackgroundColor => CssValue::Color(s.background_color),
        Property::BorderColor => CssValue::Color(s.border_color),
        Property::Width => match s.width {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::Height => match s.height {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::MaxWidth => match s.max_width {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::MaxHeight => match s.max_height {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::MinWidth => CssValue::Length(s.min_width * 100, Unit::Px),
        Property::MinHeight => CssValue::Length(s.min_height * 100, Unit::Px),
        Property::MarginTop => CssValue::Length(s.margin_top * 100, Unit::Px),
        Property::MarginRight => CssValue::Length(s.margin_right * 100, Unit::Px),
        Property::MarginBottom => CssValue::Length(s.margin_bottom * 100, Unit::Px),
        Property::MarginLeft => CssValue::Length(s.margin_left * 100, Unit::Px),
        Property::PaddingTop => CssValue::Length(s.padding_top * 100, Unit::Px),
        Property::PaddingRight => CssValue::Length(s.padding_right * 100, Unit::Px),
        Property::PaddingBottom => CssValue::Length(s.padding_bottom * 100, Unit::Px),
        Property::PaddingLeft => CssValue::Length(s.padding_left * 100, Unit::Px),
        Property::BorderWidth => CssValue::Length(s.border_width * 100, Unit::Px),
        Property::BorderRadius => CssValue::Length(s.border_radius * 100, Unit::Px),
        Property::FontSize => CssValue::Length(s.font_size * 100, Unit::Px),
        Property::LineHeight => CssValue::Length(s.line_height * 100, Unit::Px),
        Property::Top => match s.top {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::Right => match s.right_offset {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::Bottom => match s.bottom_offset {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::Left => match s.left_offset {
            Some(v) => CssValue::Length(v * 100, Unit::Px),
            None => return None,
        },
        Property::FlexGrow => CssValue::Number(s.flex_grow),
        Property::FlexShrink => CssValue::Number(s.flex_shrink),
        Property::LetterSpacing => CssValue::Length(s.letter_spacing * 100, Unit::Px),
        Property::WordSpacing => CssValue::Length(s.word_spacing * 100, Unit::Px),
        Property::TextIndent => CssValue::Length(s.text_indent * 100, Unit::Px),
        Property::RowGap => CssValue::Length(s.row_gap * 100, Unit::Px),
        Property::ColumnGap => CssValue::Length(s.column_gap * 100, Unit::Px),
        Property::Order => CssValue::Number(s.order),
        Property::ZIndex => CssValue::Number(s.z_index),
        _ => return None,
    };
    Some(crate::css::Declaration {
        property: prop.clone(),
        value,
        important: false,
    })
}
