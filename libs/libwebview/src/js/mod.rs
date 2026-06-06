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
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::value::{JsArray, JsObject};
use libjs::vm::{native_ctor_fn, native_fn};
use libjs::{JsEngine, JsValue, Vm};

use crate::css::{Declaration, KeyframeSet};
use crate::dom::{Dom, NodeId, NodeType, Tag};
use crate::style::{apply_timing, TimingFunction, TransitionDef};

const MAX_CONSOLE_MESSAGES: usize = 512;
const MAX_PENDING_TIMERS: usize = 1024;
const JS_TRACE: bool = false;
const DEFAULT_TIMER_CALLBACK_STEP_LIMIT: u64 = 8_000_000;
const DEFAULT_SCRIPT_STEP_LIMIT: u64 = 8_000_000;
const DEFAULT_MAX_SCRIPTS: usize = 48;
const DEFAULT_MAX_SCRIPT_BYTES: usize = 1024 * 1024;
const SURF_IMMEDIATE_SCHEDULER_JS: &str = r#"
(function(){
    function patch(s){
      if (!s || s.__surfImmediateScheduler) return;
      var run = function(cb){ if (typeof cb === 'function') return cb(); return cb; };
      var defer = function(cb){
        if (typeof cb !== 'function') return cb;
        var budget = s.__surfImmediateBudget == null ? 0 : s.__surfImmediateBudget;
        if (budget > 0) {
          s.__surfImmediateBudget = budget - 1;
          return cb();
        }
        if (typeof setImmediate === 'function') return setImmediate(cb);
        if (typeof setTimeout === 'function') return setTimeout(cb, 0);
        return cb();
      };
    var runPri = function(_pri, cb){ return run(cb); };
    s.defer = defer;
    s.deferUserBlockingRunAtCurrentPri_DO_NOT_USE = defer;
    s.scheduleImmediatePriCallback = defer;
    s.scheduleLoggingPriCallback = defer;
    s.scheduleNormalPriCallback = defer;
    s.scheduleSpeculativeCallback = defer;
    s.scheduleUserBlockingPriCallback = defer;
    s.scheduleDelayedCallback_DO_NOT_USE = function(_pri, _delay, cb){ return defer(cb); };
    s.cancelCallback = function(){};
    s.cancelDelayedCallback_DO_NOT_USE = function(){};
    s.getCallbackScheduler = function(){ return defer; };
    s.getUserBlockingRunAtCurrentPriCallbackScheduler_DO_NOT_USE = function(){ return defer; };
    s.runWithPriority = runPri;
    s.runWithPriority_DO_NOT_USE = runPri;
    s.shouldYield = function(){ return false; };
    s.__surfImmediateScheduler = true;
  }
  try {
    if (typeof ifRequireable === 'function') {
      ifRequireable('JSScheduler', patch, function(){});
    } else if (typeof require === 'function') {
      patch(require('JSScheduler'));
    }
  } catch (_e) {}
})();
"#;

const SURF_SERVERJS_DEFINE_FAST_PATH_JS: &str = r#"
(function(){
  try {
    if (typeof require !== 'function' || typeof document === 'undefined') return;
    var ServerJSDefine = require('ServerJSDefine');
    if (!ServerJSDefine || typeof ServerJSDefine.handleDefines !== 'function') return;
    var processed = window.__surfProcessedSjsDefineIndexes || (window.__surfProcessedSjsDefineIndexes = {});
    window.__surfSjsDefineCount = window.__surfSjsDefineCount || 0;
    window.__surfSjsDefineErrors = window.__surfSjsDefineErrors || 0;
    window.__surfSjsFirstDefineError = window.__surfSjsFirstDefineError || '';
    window.__surfSjsDefineAttempts = window.__surfSjsDefineAttempts || 0;
    var scripts = document.querySelectorAll('script[data-sjs]');
    Array.from(scripts).forEach(function(el, index){
      if (processed[index]) return;
      var payload;
      try { payload = JSON.parse(el.textContent || ''); } catch (_e) { return; }
      var didWork = false;
      (payload.require || []).forEach(function(req){
        try {
          if (!req || req[0] === 'qplTimingsServerJS') return;
          if ((req[0] === 'ScheduledServerJS' || req[0] === 'ScheduledServerJSWithCSS') && req[1] === 'handle') {
            var args = req[3] || [];
            args.forEach(function(arg){
              var bbox = arg && arg.__bbox;
              if (bbox && bbox.define && bbox.define.length) {
                window.__surfSjsDefineAttempts += bbox.define.length;
                ServerJSDefine.handleDefines(bbox.define);
                window.__surfSjsDefineCount += bbox.define.length;
                didWork = true;
              }
            });
          } else if (req[0] === 'ScheduledServerJSDefine' && req[1] === 'handleDefines') {
            var args = req[3] || [];
            if (args[0] && args[0].length) {
              window.__surfSjsDefineAttempts += args[0].length;
              ServerJSDefine.handleDefines(args[0], args[1]);
              window.__surfSjsDefineCount += args[0].length;
              didWork = true;
            }
          }
        } catch (_e) {
          window.__surfSjsDefineErrors += 1;
          try {
            if (!window.__surfSjsFirstDefineError && _e && _e.message) window.__surfSjsFirstDefineError = _e.message;
          } catch (_ignored) {}
        }
      });
      if (didWork) processed[index] = true;
    });
  } catch (_e) {
    try { window.__surfSjsDefineErrors = (window.__surfSjsDefineErrors || 0) + 1; } catch (_ignored) {}
    try { if (!window.__surfSjsFirstDefineError && _e && _e.message) window.__surfSjsFirstDefineError = _e.message; } catch (_ignored) {}
  }
})();
"#;

const SURF_SERVERJS_ROOT_FAST_PATH_JS: &str = r#"
(function(){
  try {
    if (typeof require !== 'function' || typeof document === 'undefined') return;
    if (window.__surfRanSjsRootRequire) return;
    var scripts = document.querySelectorAll('script[data-sjs]');
    var rootEntries = [];
    Array.from(scripts).forEach(function(el){
      var payload;
      try { payload = JSON.parse(el.textContent || ''); } catch (_e) { return; }
      (payload.require || []).forEach(function(req){
        if (!req || (req[0] !== 'ScheduledServerJS' && req[0] !== 'ScheduledServerJSWithCSS') || req[1] !== 'handle') return;
        (req[3] || []).forEach(function(arg){
          var bbox = arg && arg.__bbox;
          (bbox && bbox.require || []).forEach(function(entry){
            if (entry && entry[0] === 'CometPlatformRootClient' && entry[1] === 'initialize') {
              rootEntries.push(entry);
            }
          });
        });
      });
    });
    rootEntries.forEach(function(entry){
      var mod = require(entry[0]);
      var fn = mod && mod[entry[1]];
      if (typeof fn === 'function') {
        fn.apply(mod, entry[3] || []);
        window.__surfRanSjsRootRequire = true;
      }
    });
  } catch (_e) {}
})();
"#;

#[derive(Clone, Copy)]
pub struct ScriptExecutionLimits {
    pub max_scripts: usize,
    pub max_script_bytes: Option<usize>,
}

impl Default for ScriptExecutionLimits {
    fn default() -> Self {
        ScriptExecutionLimits {
            max_scripts: DEFAULT_MAX_SCRIPTS,
            max_script_bytes: Some(DEFAULT_MAX_SCRIPT_BYTES),
        }
    }
}

fn configured_script_step_limit() -> u64 {
    #[cfg(feature = "host")]
    {
        if let Ok(raw) = std::env::var("LIBJS_SCRIPT_STEP_LIMIT") {
            if let Ok(limit) = raw.parse::<u64>() {
                return limit.max(1_000_000);
            }
        }
    }
    DEFAULT_SCRIPT_STEP_LIMIT
}

fn configured_timer_callback_step_limit() -> u64 {
    #[cfg(feature = "host")]
    {
        if let Ok(raw) = std::env::var("LIBJS_TIMER_CALLBACK_STEP_LIMIT") {
            if let Ok(limit) = raw.parse::<u64>() {
                return limit.max(100_000);
            }
        }
    }
    DEFAULT_TIMER_CALLBACK_STEP_LIMIT
}
const QUIET_SELF_RESCHEDULE_MIN_DELAY_MS: u64 = 250;

macro_rules! js_trace {
    ($($arg:tt)*) => {{
        if JS_TRACE {
            anyos_std::println!($($arg)*);
        }
    }};
}

#[cfg(feature = "host")]
fn debug_class_mutations_enabled() -> bool {
    std::env::var_os("SURF_DEBUG_CLASS_MUTATIONS").is_some()
}

#[cfg(feature = "host")]
fn debug_all_class_mutations_enabled() -> bool {
    std::env::var("SURF_DEBUG_CLASS_MUTATIONS")
        .map(|value| value.eq_ignore_ascii_case("all"))
        .unwrap_or(false)
}

#[cfg(feature = "host")]
fn debug_dom_apply_enabled() -> bool {
    std::env::var_os("SURF_DEBUG_DOM_APPLY").is_some()
}

#[cfg(not(feature = "host"))]
fn debug_class_mutations_enabled() -> bool {
    false
}

#[cfg(not(feature = "host"))]
fn debug_all_class_mutations_enabled() -> bool {
    false
}

#[cfg(not(feature = "host"))]
fn debug_dom_apply_enabled() -> bool {
    false
}

// ═══════════════════════════════════════════════════════════
// Property write interception — static target for set_hook
// ═══════════════════════════════════════════════════════════

/// Points to the current DomBridge.mutations during JS execution.
/// Set before executing JS, cleared after. Used by dom_property_hook.
static mut MUTATION_TARGET: *mut Vec<DomMutation> = core::ptr::null_mut();
/// Points to the current DomBridge.virtual_nodes during JS execution.
static mut VIRTUAL_NODES_TARGET: *mut Vec<VirtualNode> = core::ptr::null_mut();
/// Points to the current DomBridge.pending_navigation_requests during JS execution.
static mut NAVIGATION_TARGET: *mut Vec<PendingNavigationRequest> = core::ptr::null_mut();
/// Points to the current DomBridge.event_listeners during JS execution.
static mut EVENT_LISTENERS_TARGET: *mut Vec<EventListener> = core::ptr::null_mut();
/// Points to final compositor-friendly styles inferred from React/Framer props.
static mut MOTION_FINAL_STYLES_TARGET: *mut Vec<MotionFinalStyle> = core::ptr::null_mut();

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
        key if key.starts_with("on") && key.len() > 2 => {
            if matches!(value, JsValue::Function(_) | JsValue::Object(_)) {
                let listeners = unsafe {
                    if EVENT_LISTENERS_TARGET.is_null() {
                        return;
                    }
                    &mut *EVENT_LISTENERS_TARGET
                };
                listeners.push(EventListener {
                    node_id: if node_id >= 0 {
                        node_id as usize
                    } else {
                        usize::MAX
                    },
                    event: String::from(&key[2..]),
                    callback: value.clone(),
                    capture: false,
                });
            }
        }
        key if key.starts_with("__reactProps$") => {
            let class_value = value.get_property("className");
            let cls = match class_value {
                JsValue::Undefined | JsValue::Null => String::new(),
                _ => class_value.to_js_string(),
            };
            if !cls.is_empty() && cls != "undefined" {
                mutations.push(DomMutation::SetAttribute {
                    node_id,
                    name: String::from("class"),
                    value: cls.clone(),
                });
            }
            if !cls.is_empty()
                && cls != "undefined"
                && debug_class_mutations_enabled()
                && (debug_all_class_mutations_enabled()
                    || cls.contains("max-w-7xl")
                    || cls.contains("text-center")
                    || cls.contains("relative mx-auto")
                    || cls.contains("max-w-4xl"))
            {
                #[cfg(feature = "host")]
                eprintln!(
                    "[js-dom-debug] react props class nid={} value={}",
                    node_id, cls
                );
            }
            for (prop_name, attr_name) in [
                ("src", "src"),
                ("srcSet", "srcset"),
                ("alt", "alt"),
                ("decoding", "decoding"),
                ("loading", "loading"),
                ("fetchPriority", "fetchpriority"),
                ("href", "href"),
                ("target", "target"),
                ("rel", "rel"),
                ("title", "title"),
                ("width", "width"),
                ("height", "height"),
                ("max", "max"),
                ("viewBox", "viewBox"),
                ("fill", "fill"),
                ("stroke", "stroke"),
                ("strokeWidth", "strokeWidth"),
                ("strokeLinecap", "strokeLinecap"),
                ("strokeLinejoin", "strokeLinejoin"),
                ("xmlns", "xmlns"),
                ("role", "role"),
                ("aria-hidden", "aria-hidden"),
                ("focusable", "focusable"),
            ] {
                let attr_value = value.get_property(prop_name);
                if !matches!(attr_value, JsValue::Undefined | JsValue::Null) {
                    mutations.push(DomMutation::SetAttribute {
                        node_id,
                        name: String::from(attr_name),
                        value: attr_value.to_js_string(),
                    });
                }
            }
            apply_react_motion_final_styles(node_id, value, mutations);
        }
        "textContent" | "innerText" | "nodeValue" | "data" => {
            mutations.push(DomMutation::SetTextContent {
                node_id,
                text: value.to_js_string(),
            });
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
            if debug_class_mutations_enabled()
                && (debug_all_class_mutations_enabled()
                    || cls.contains("max-w-7xl")
                    || cls.contains("text-center")
                    || cls.contains("relative mx-auto")
                    || cls.contains("max-w-4xl"))
            {
                #[cfg(feature = "host")]
                eprintln!(
                    "[js-dom-debug] className property nid={} value={}",
                    node_id, cls
                );
            }
            // Always emit SetAttribute mutation — for virtual nodes,
            // apply_mutations resolves the ID via id_map.
            mutations.push(DomMutation::SetAttribute {
                node_id: node_id,
                name: String::from("class"),
                value: cls,
            });
        }
        "value" | "src" | "href" | "id" | "name" | "type" | "width" | "height" | "max"
        | "viewBox" | "fill" | "stroke" | "strokeWidth" | "strokeLinecap" | "strokeLinejoin"
        | "xmlns" | "role" | "aria-hidden" | "focusable" | "alt" | "decoding" | "loading"
        | "target" | "rel" | "title" => {
            mutations.push(DomMutation::SetAttribute {
                node_id,
                name: String::from(key),
                value: value.to_js_string(),
            });
        }
        "srcSet" => {
            mutations.push(DomMutation::SetAttribute {
                node_id,
                name: String::from("srcset"),
                value: value.to_js_string(),
            });
        }
        "fetchPriority" => {
            mutations.push(DomMutation::SetAttribute {
                node_id,
                name: String::from("fetchpriority"),
                value: value.to_js_string(),
            });
        }
        "checked" | "disabled" => {
            if value.to_boolean() {
                mutations.push(DomMutation::SetAttribute {
                    node_id,
                    name: String::from(key),
                    value: String::new(),
                });
            } else {
                mutations.push(DomMutation::RemoveAttribute {
                    node_id,
                    name: String::from(key),
                });
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
                    smooth: None,
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
                    smooth: None,
                });
            }
        }
        // Ignore internal properties and methods.
        _ => {}
    }
}

fn dataset_property_hook(data: *mut u8, key: &str, value: &JsValue) {
    let mutations = unsafe {
        if MUTATION_TARGET.is_null() {
            return;
        }
        &mut *MUTATION_TARGET
    };
    let node_id = data as usize as i64;
    let mut attr = String::from("data-");
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            attr.push('-');
            attr.push(ch.to_ascii_lowercase());
        } else {
            attr.push(ch);
        }
    }
    mutations.push(DomMutation::SetAttribute {
        node_id,
        name: attr,
        value: value.to_js_string(),
    });
}

fn apply_react_motion_final_styles(
    node_id: i64,
    props: &JsValue,
    mutations: &mut Vec<DomMutation>,
) {
    let final_style = {
        let while_in_view = props.get_property("whileInView");
        if while_in_view.is_object() {
            #[cfg(feature = "host")]
            if std::env::var_os("SURF_DEBUG_MOTION_PROPS").is_some() {
                eprintln!("[js-dom-debug] motion whileInView nid={}", node_id);
            }
            while_in_view
        } else {
            let animate = props.get_property("animate");
            if animate.is_object() {
                #[cfg(feature = "host")]
                if std::env::var_os("SURF_DEBUG_MOTION_PROPS").is_some() {
                    eprintln!("[js-dom-debug] motion animate nid={}", node_id);
                }
                animate
            } else {
                return;
            }
        }
    };

    let opacity = final_style.get_property("opacity");
    let final_opacity = if !opacity.is_undefined() && !opacity.is_null() {
        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_MOTION_PROPS").is_some() {
            eprintln!(
                "[js-dom-debug] motion opacity nid={} value={}",
                node_id,
                opacity.to_js_string()
            );
        }
        let value = opacity.to_js_string();
        mutations.push(DomMutation::SetStyleProperty {
            node_id,
            property: String::from("opacity"),
            value: value.clone(),
        });
        Some(value)
    } else {
        None
    };

    let x = final_style.get_property("x");
    let y = final_style.get_property("y");
    let final_transform =
        if (!x.is_undefined() && !x.is_null()) || (!y.is_undefined() && !y.is_null()) {
            let tx = if x.is_undefined() || x.is_null() {
                0.0
            } else {
                x.to_number()
            };
            let ty = if y.is_undefined() || y.is_null() {
                0.0
            } else {
                y.to_number()
            };
            let value = alloc::format!("translate({}px, {}px)", tx as i32, ty as i32);
            mutations.push(DomMutation::SetStyleProperty {
                node_id,
                property: String::from("transform"),
                value: value.clone(),
            });
            Some(value)
        } else {
            None
        };

    if final_opacity.is_some() || final_transform.is_some() {
        let final_styles = unsafe {
            if MOTION_FINAL_STYLES_TARGET.is_null() {
                None
            } else {
                Some(&mut *MOTION_FINAL_STYLES_TARGET)
            }
        };
        if let Some(final_styles) = final_styles {
            if let Some(existing) = final_styles
                .iter_mut()
                .find(|entry| entry.node_id == node_id)
            {
                if final_opacity.is_some() {
                    existing.opacity = final_opacity;
                }
                if final_transform.is_some() {
                    existing.transform = final_transform;
                }
            } else {
                final_styles.push(MotionFinalStyle {
                    node_id,
                    opacity: final_opacity,
                    transform: final_transform,
                });
            }
        }
    }
}

pub(super) fn motion_final_style_value(node_id: i64, property: &str) -> Option<String> {
    let final_styles = unsafe {
        if MOTION_FINAL_STYLES_TARGET.is_null() {
            return None;
        }
        &mut *MOTION_FINAL_STYLES_TARGET
    };
    let entry = final_styles.iter().find(|entry| entry.node_id == node_id)?;
    match property {
        "opacity" => entry.opacity.clone(),
        "transform" => entry.transform.clone(),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// DomBridge — stored in vm.userdata so native fns can reach the DOM
// ═══════════════════════════════════════════════════════════

struct DomBridge {
    dom: *const Dom,
    mutations: Vec<DomMutation>,
    event_listeners: Vec<EventListener>,
    installed_event_listeners: *const Vec<EventListener>,
    /// Counter for virtual (createElement'd) node IDs: -1, -2, -3, …
    next_virtual_id: i64,
    /// Virtual nodes created by createElement.
    virtual_nodes: Vec<VirtualNode>,
    /// Persistent mapping from JS virtual node IDs to real DOM node IDs.
    real_node_ids: BTreeMap<i64, usize>,
    /// Pending HTTP requests from XHR / fetch.
    pending_http_requests: Vec<PendingHttpRequest>,
    /// Pending page navigations requested by JavaScript.
    pending_navigation_requests: Vec<PendingNavigationRequest>,
    /// Pending timers (setTimeout / setInterval).
    timers: Vec<PendingTimer>,
    /// Pending Web Animations started from Element.animate().
    pending_style_animations: Vec<StyleAnimation>,
    /// Final visible styles inferred from React/Framer motion props.
    motion_final_styles: Vec<MotionFinalStyle>,
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

    fn resolve_node_id(&self, id: i64) -> Option<usize> {
        if id >= 0 {
            Some(id as usize)
        } else {
            self.real_node_ids.get(&id).copied()
        }
    }

    fn installed_event_listeners(&self) -> &[EventListener] {
        if self.installed_event_listeners.is_null() {
            &[]
        } else {
            unsafe { &*self.installed_event_listeners }
        }
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
#[derive(Clone)]
struct VirtualNode {
    id: i64,
    tag: String,
    attrs: Vec<(String, String)>,
    text_content: String,
    child_ids: Vec<i64>,
    parent_id: Option<i64>,
}

#[derive(Clone)]
struct MotionFinalStyle {
    node_id: i64,
    opacity: Option<String>,
    transform: Option<String>,
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
        smooth: Option<bool>,
    },
    /// Set the horizontal scroll offset on an overflow container (JS `element.scrollLeft = n`).
    SetScrollLeft {
        node_id: usize,
        value: i32,
        smooth: Option<bool>,
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

#[derive(Clone)]
pub struct StyleAnimation {
    pub node_id: i64,
    pub duration_ms: u64,
    pub delay_ms: u64,
    pub elapsed_ms: u64,
    pub iterations: u32,
    pub fill_forwards: bool,
    pub from_opacity: Option<f32>,
    pub to_opacity: Option<f32>,
    pub from_transform: Option<String>,
    pub to_transform: Option<String>,
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

/// A page navigation requested by JavaScript.
#[derive(Clone)]
pub struct PendingNavigationRequest {
    pub url: String,
    pub replace: bool,
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

fn call_event_listener(vm: &mut Vm, callback: &JsValue, evt: &JsValue, current_target: &JsValue) {
    match callback {
        JsValue::Function(_) => {
            vm.call_value(callback, &[evt.clone()], current_target.clone());
        }
        JsValue::Object(_) => {
            let handler = callback.get_property("handleEvent");
            if let JsValue::Function(_) = handler {
                vm.call_value(&handler, &[evt.clone()], callback.clone());
            }
        }
        _ => {}
    }
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
    /// `this` value used when invoking the callback.
    pub this_arg: JsValue,
    /// Arguments passed to the callback when the timer fires.
    pub args: Vec<JsValue>,
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
    Module,
}

#[derive(Clone)]
pub enum ScriptEntry {
    /// Inline `<script>` with text content.
    Inline { text: String, mode: ScriptMode },
    /// External `<script src="url">` — the host must fetch the URL and provide the text.
    External { src: String, mode: ScriptMode },
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn skip_js_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\n' | b'\r' | b'\t' | 0x0c => i += 1,
            _ => break,
        }
    }
    i
}

fn parse_quoted_js_string(bytes: &[u8], mut i: usize) -> Option<(String, usize)> {
    if i >= bytes.len() || (bytes[i] != b'\'' && bytes[i] != b'"') {
        return None;
    }
    let quote = bytes[i];
    i += 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            return Some((out, i + 1));
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 1;
            out.push(bytes[i] as char);
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    None
}

fn push_unique_spec(specs: &mut Vec<String>, spec: String) {
    if is_prefetchable_module_specifier(&spec) && !specs.iter().any(|s| s == &spec) {
        specs.push(spec);
    }
}

fn bytes_start_with(bytes: &[u8], i: usize, pat: &[u8]) -> bool {
    i + pat.len() <= bytes.len() && &bytes[i..i + pat.len()] == pat
}

fn is_prefetchable_module_specifier(spec: &str) -> bool {
    let s = spec.trim();
    if s.is_empty() {
        return false;
    }
    // Host-side module prefetch can only resolve URL-like module specifiers.
    // Bare specifiers need import-map/package resolution, and one-character
    // punctuation strings show up in heavily minified bundles near `import`
    // tokens even though they are not fetchable chunks.
    s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with('/')
        || s.starts_with("http://")
        || s.starts_with("https://")
}

/// Extract simple static/dynamic ES module specifiers from a script.
///
/// This is intentionally conservative and string-aware enough for bundled
/// browser code (`import{...}from"chunk.js"`, `import "chunk.js"`,
/// `export{...}from"chunk.js"`). Dynamic `import("chunk.js")` is deliberately
/// handled by [`extract_module_specifiers_for_page_with_page_id`] because large
/// Vite/Vike manifests contain hundreds of lazy route chunks that must not be
/// fetched eagerly. The real parser still
/// owns JS semantics; this helper only lets hosts prefetch module chunks before
/// `__import__()` resolves them.
pub fn extract_module_specifiers(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut specs = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i = i.saturating_add(2);
                    } else if bytes[i] == quote {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = core::cmp::min(i + 2, bytes.len());
            }
            b'i' if bytes_start_with(bytes, i, b"import")
                && (i == 0 || !is_ident_byte(bytes[i - 1]))
                && (i + 6 >= bytes.len() || !is_ident_byte(bytes[i + 6])) =>
            {
                let mut j = skip_js_ws(bytes, i + 6);
                if j < bytes.len() && bytes[j] == b'(' {
                    // Dynamic import: runtime-triggered. Do not eagerly
                    // prefetch here; the page-aware scanner below narrows it
                    // to the current route when possible.
                } else if let Some((spec, end)) = parse_quoted_js_string(bytes, j) {
                    push_unique_spec(&mut specs, spec);
                    i = end;
                    continue;
                } else {
                    while j + 4 <= bytes.len() {
                        if bytes_start_with(bytes, j, b"from")
                            && (j == 0 || !is_ident_byte(bytes[j - 1]))
                            && (j + 4 >= bytes.len() || !is_ident_byte(bytes[j + 4]))
                        {
                            let k = skip_js_ws(bytes, j + 4);
                            if let Some((spec, end)) = parse_quoted_js_string(bytes, k) {
                                push_unique_spec(&mut specs, spec);
                                i = end;
                                break;
                            }
                        }
                        if matches!(bytes[j], b';' | b'\n' | b'\r') {
                            break;
                        }
                        j += 1;
                    }
                }
                i += 1;
            }
            b'e' if bytes_start_with(bytes, i, b"export")
                && (i == 0 || !is_ident_byte(bytes[i - 1]))
                && (i + 6 >= bytes.len() || !is_ident_byte(bytes[i + 6])) =>
            {
                let mut j = i + 6;
                while j + 4 <= bytes.len() {
                    if bytes_start_with(bytes, j, b"from")
                        && (j == 0 || !is_ident_byte(bytes[j - 1]))
                        && (j + 4 >= bytes.len() || !is_ident_byte(bytes[j + 4]))
                    {
                        let k = skip_js_ws(bytes, j + 4);
                        if let Some((spec, end)) = parse_quoted_js_string(bytes, k) {
                            push_unique_spec(&mut specs, spec);
                            i = end;
                            break;
                        }
                    }
                    if matches!(bytes[j], b';' | b'\n' | b'\r') {
                        break;
                    }
                    j += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    specs
}

/// Extract module specifiers for a concrete page load.
///
/// This includes the conservative static scan plus a contextual dynamic import
/// fallback for generated route bundles. We intentionally avoid returning every
/// lazy route chunk from large Vite/Vike manifests because that blocks Surf on
/// hundreds of unrelated downloads before the page can hydrate.
pub fn extract_module_specifiers_for_page(source: &str, page_url: &str) -> Vec<String> {
    extract_module_specifiers_for_page_with_page_id(source, page_url, None)
}

pub fn extract_module_specifiers_for_page_with_page_id(
    source: &str,
    page_url: &str,
    current_page_id: Option<&str>,
) -> Vec<String> {
    let mut specs = extract_module_specifiers(source);
    let bytes = source.as_bytes();
    let _ = page_url;
    let page_module_key = current_page_id.and_then(page_id_module_key);
    let mut dynamic_specs = Vec::new();
    let mut j = 0usize;
    while j < bytes.len() {
        if bytes_start_with(bytes, j, b"import")
            && (j == 0 || !is_ident_byte(bytes[j - 1]))
            && (j + 6 >= bytes.len() || !is_ident_byte(bytes[j + 6]))
        {
            let mut k = skip_js_ws(bytes, j + 6);
            if k < bytes.len() && bytes[k] == b'(' {
                k = skip_js_ws(bytes, k + 1);
                if let Some((spec, end)) = parse_quoted_js_string(bytes, k) {
                    if is_prefetchable_module_specifier(&spec)
                        && !dynamic_specs.iter().any(|s| s == &spec)
                    {
                        dynamic_specs.push(spec);
                    }
                    j = end;
                    continue;
                }
            }
        }
        j += 1;
    }

    if let Some(page_module_key) = page_module_key {
        for spec in dynamic_specs {
            if spec.contains(&page_module_key) {
                push_unique_spec(&mut specs, spec);
            }
        }
    } else if dynamic_specs.len() <= 32 {
        for spec in dynamic_specs {
            push_unique_spec(&mut specs, spec);
        }
    } else {
        // Large Vite/Vike manifests list every lazy route as `import(...)`.
        // Browsers do not prefetch those blindly; they use `modulepreload`
        // links plus the runtime-selected route. If we cannot determine the
        // current page id, keep dynamic imports lazy instead of flooding the
        // network/JS queues with hundreds of unrelated route chunks.
    }
    specs
}

pub fn extract_modulepreload_links_from_dom(dom: &crate::dom::Dom) -> Vec<String> {
    let mut links = Vec::new();
    for (node_id, _) in dom.nodes.iter().enumerate() {
        if !dom.has_tag_name(node_id, "link") {
            continue;
        }
        let rel = dom.attr(node_id, "rel").unwrap_or("");
        if !rel
            .split_ascii_whitespace()
            .any(|token| token.eq_ignore_ascii_case("modulepreload"))
        {
            continue;
        }
        let Some(href) = dom.attr(node_id, "href") else {
            continue;
        };
        if href.is_empty() || links.iter().any(|existing| existing == href) {
            continue;
        }
        links.push(String::from(href));
    }
    links
}

pub fn extract_vike_page_id_from_dom(dom: &crate::dom::Dom) -> Option<String> {
    for (node_id, _) in dom.nodes.iter().enumerate() {
        if !dom.has_tag_name(node_id, "script") {
            continue;
        }
        if dom.attr(node_id, "id") != Some("vike_pageContext") {
            continue;
        }
        let text = dom.text_content(node_id);
        if let Some(page_id) = extract_json_string_field(&text, "pageId") {
            return Some(page_id.replace("\\/", "/"));
        }
    }
    None
}

fn extract_json_string_field(source: &str, key: &str) -> Option<String> {
    let needle = alloc::format!("\"{}\"", key);
    let key_pos = source.find(&needle)?;
    let after_key = &source[key_pos + needle.len()..];
    let colon = after_key.find(':')?;
    let mut rest = after_key[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    rest = &rest[1..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push('\\');
                    out.push(next);
                }
            }
            _ => out.push(ch),
        }
    }
    None
}

fn page_id_module_key(page_id: &str) -> Option<String> {
    let page_id = page_id.trim().trim_start_matches('/');
    if page_id.is_empty()
        || page_id
            .bytes()
            .any(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_')))
    {
        return None;
    }
    Some(page_id.replace('/', "_"))
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
    next_virtual_id: i64,
    real_node_ids: BTreeMap<i64, usize>,
    pub event_listeners: Vec<EventListener>,
    pub pending_http_requests: Vec<PendingHttpRequest>,
    pub pending_navigation_requests: Vec<PendingNavigationRequest>,
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
    /// Native Web Animations API animations driven by the browser tick.
    pub active_style_animations: Vec<StyleAnimation>,
    /// Total elapsed time since page load (ms) — used as DOMHighResTimeStamp for rAF callbacks.
    total_elapsed_ms: u64,
    /// Viewport dimensions exposed to `window`.
    viewport_width: u32,
    viewport_height: u32,
    native_api_initialized: bool,
    native_api_url: String,
}

fn js_exception_summary(exc: &JsValue) -> String {
    match exc {
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
            if let JsValue::String(stack) = o.get("stack") {
                let mut frames = Vec::new();
                for line in stack.lines().skip(1).take(4) {
                    let line = line.trim();
                    if !line.is_empty() {
                        frames.push(String::from(line));
                    }
                }
                if !frames.is_empty() {
                    return format!("{}: {} [{}]", name, msg, frames.join(" <- "));
                }
            }
            format!("{}: {}", name, msg)
        }
        other => format!("{:?}", other),
    }
}

impl JsRuntime {
    pub fn push_console_line(&mut self, msg: String) {
        push_console_message(&mut self.console, msg);
    }

    fn collect_engine_console(&mut self) {
        let messages: Vec<String> = self.engine.console_output().iter().cloned().collect();
        for msg in messages {
            push_console_message(&mut self.console, msg);
        }
        self.engine.clear_console();
    }

    fn install_immediate_scheduler_fast_path(&mut self) {
        let _ = self.engine.eval(SURF_IMMEDIATE_SCHEDULER_JS);
        self.engine.vm().last_exception = None;
        self.engine.vm().pending_exception = None;
        self.engine.vm().frames.clear();
        self.engine.vm().stack.clear();
        self.engine.clear_console();
    }

    fn install_serverjs_define_fast_path(&mut self) {
        let _ = self.engine.eval(SURF_SERVERJS_DEFINE_FAST_PATH_JS);
        self.engine.vm().last_exception = None;
        self.engine.vm().pending_exception = None;
        self.engine.vm().frames.clear();
        self.engine.vm().stack.clear();
        self.engine.clear_console();
    }

    fn install_serverjs_root_fast_path(&mut self) {
        let _ = self.engine.eval(SURF_SERVERJS_ROOT_FAST_PATH_JS);
        self.engine.vm().last_exception = None;
        self.engine.vm().pending_exception = None;
        self.engine.vm().frames.clear();
        self.engine.vm().stack.clear();
        self.engine.clear_console();
    }

    pub fn new() -> Self {
        let engine = JsEngine::new();
        Self {
            engine,
            console: Vec::new(),
            mutations: Vec::new(),
            virtual_nodes: Vec::new(),
            next_virtual_id: -1,
            real_node_ids: BTreeMap::new(),
            event_listeners: Vec::new(),
            pending_http_requests: Vec::new(),
            pending_navigation_requests: Vec::new(),
            timers: Vec::new(),
            next_timer_id: 1,
            cookies: String::new(),
            pending_ws_connects: Vec::new(),
            pending_ws_sends: Vec::new(),
            pending_ws_closes: Vec::new(),
            ws_registry: Vec::new(),
            active_animations: Vec::new(),
            active_transitions: Vec::new(),
            active_style_animations: Vec::new(),
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
                doc.set_property(
                    String::from("cookie"),
                    JsValue::String(self.cookies.clone()),
                );
            }
        }
    }

    pub fn register_module_source(&mut self, specifier: &str, source: &str) {
        self.engine.register_module_source(specifier, source);
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
                let is_module = type_attr
                    .map(|t| t.value.eq_ignore_ascii_case("module"))
                    .unwrap_or(false);
                let src = attrs
                    .iter()
                    .find(|a| a.name == "src")
                    .map(|a| a.value.as_str());
                let has_async = attrs.iter().any(|a| a.name == "async");
                let has_defer = attrs.iter().any(|a| a.name == "defer");
                let mode = if is_module {
                    // Module scripts are deferred by default and must execute
                    // through the module loader so imports, exports, and
                    // once-per-module evaluation semantics are preserved.
                    ScriptMode::Module
                } else if has_async {
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
        self.execute_script_sources_with_limits(
            dom,
            url,
            scripts,
            ScriptExecutionLimits::default(),
        );
    }

    pub fn execute_script_sources_with_limits(
        &mut self,
        dom: &Dom,
        url: &str,
        scripts: &[String],
        limits: ScriptExecutionLimits,
    ) {
        if scripts.is_empty() {
            return;
        }

        let total_bytes: usize = scripts.iter().map(|s| s.len()).sum();
        js_trace!(
            "[js] {} script(s) to execute, {} bytes total",
            scripts.len(),
            total_bytes
        );

        // Per-script step limit to keep pages responsive.
        //
        // Surf runs page JavaScript on the UI thread today. A very high budget
        // lets heavy third-party bundles monopolize the browser for seconds and
        // makes already-fetched images/fonts appear as slow "UI" work in the
        // network panel. Keep this deliberately tight until scripts can run on
        // a preemptible worker.
        let script_step_limit = configured_script_step_limit();
        self.engine.set_step_limit(script_step_limit);

        // Set up DOM bridge via userdata.
        js_trace!(
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
            installed_event_listeners: &self.event_listeners as *const Vec<EventListener>,
            next_virtual_id: self.next_virtual_id,
            virtual_nodes: core::mem::take(&mut self.virtual_nodes),
            real_node_ids: core::mem::take(&mut self.real_node_ids),
            pending_http_requests: Vec::new(),
            pending_navigation_requests: Vec::new(),
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
            pending_style_animations: Vec::new(),
            motion_final_styles: Vec::new(),
        };
        js_trace!(
            "[js] setup bridge ready: mutations={} listeners={} pending_http={} timers={}",
            bridge.mutations.len(),
            bridge.event_listeners.len(),
            bridge.pending_http_requests.len(),
            bridge.timers.len()
        );
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        js_trace!(
            "[js] setup userdata installed: frames={} stack={} vm_userdata_set=true",
            self.engine.vm().frames.len(),
            self.engine.vm().stack.len()
        );

        // Set up native host objects (document, window, etc.) once per navigation.
        if !self.native_api_initialized || self.native_api_url != url {
            js_trace!("[js] setup native api begin");
            self.setup_native_api(dom, url, &self.cookies.clone());
            self.native_api_initialized = true;
            self.native_api_url = String::from(url);
            js_trace!(
                "[js] setup native api done: console_msgs={} engine_logs={}",
                self.engine.console_output().len(),
                self.engine.vm().engine_log.len()
            );
        } else {
            js_trace!("[js] setup native api reuse: url={}", url);
            let doc = self.engine.vm().get_global("document");
            if !doc.is_undefined() {
                doc.set_property(
                    String::from("cookie"),
                    JsValue::String(self.cookies.clone()),
                );
            }
        }

        // Enable property-write interception.
        js_trace!("[js] setup mutation interception begin");
        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
            NAVIGATION_TARGET =
                &mut bridge.pending_navigation_requests as *mut Vec<PendingNavigationRequest>;
            EVENT_LISTENERS_TARGET = &mut bridge.event_listeners as *mut Vec<EventListener>;
            MOTION_FINAL_STYLES_TARGET =
                &mut bridge.motion_final_styles as *mut Vec<MotionFinalStyle>;
        }
        js_trace!(
            "[js] setup mutation interception done: mutation_target=true virtual_nodes_target=true"
        );

        // Execute each script (with limits to keep UI responsive).
        let script_count = scripts.len().min(limits.max_scripts);
        for (idx, script) in scripts.iter().take(script_count).enumerate() {
            // Some site loaders replace ScheduleJSWork with their own queued
            // scheduler.  In Surf's current screenshot/AnyOS turn model that can
            // strand server-render payloads behind timers that are never reached
            // before the first paint.  Keep the host bridge deterministic per
            // script until JS runs on its own event loop thread.
            reinstall_schedule_js_work(self.engine.vm());
            #[cfg(feature = "host")]
            if std::env::var_os("LIBWEBVIEW_DEBUG_SCRIPT_GLOBALS").is_some() {
                let d = self.engine.vm().get_global("__d").type_of();
                let d_stub = self.engine.vm().get_global("__d_stub").type_of();
                let require_lazy = self.engine.vm().get_global("requireLazy").type_of();
                anyos_std::println!(
                    "[js-debug] before #{}: __d={} __d_stub={} requireLazy={}",
                    idx,
                    d,
                    d_stub,
                    require_lazy
                );
            }
            if limits
                .max_script_bytes
                .map(|max| script.len() > max)
                .unwrap_or(false)
            {
                anyos_std::println!(
                    "[js] skipping script #{} ({} bytes — too large)",
                    idx,
                    script.len()
                );
                continue;
            }
            // Reset step counter and engine state before each script so each gets the full budget.
            js_trace!(
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
            self.engine.set_step_limit(script_step_limit);
            // Clear any leftover call frames from aborted scripts (e.g. step-limit abort).
            self.engine.vm().frames.clear();
            self.engine.vm().stack.clear();
            js_trace!(
                "[js] prepare #{} reset done: frames={} stack={} step_limit={}",
                idx,
                self.engine.vm().frames.len(),
                self.engine.vm().stack.len(),
                self.engine.vm().step_limit
            );
            js_trace!(
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
                js_trace!(
                    "[js] script #{} completed: {} steps (limit {})",
                    idx,
                    steps_used,
                    self.engine.vm().step_limit
                );
            }

            // Check for unhandled exceptions.
            if let Some(ref exc) = self.engine.vm().last_exception {
                let exc_str = js_exception_summary(exc);
                anyos_std::println!("[js] !! script #{} EXCEPTION: {}", idx, exc_str);
                // Clear last_exception so next script can run fresh.
                self.engine.vm().last_exception = None;
            }

            // Print engine log messages from this script.
            {
                let logs = &self.engine.vm().engine_log;
                if !logs.is_empty() {
                    for log_msg in logs.iter() {
                        js_trace!("[js] engine #{}: {}", idx, log_msg);
                    }
                    self.engine.vm().engine_log.clear();
                }
            }

            // Flush console output after each script.
            for msg in self.engine.console_output() {
                js_trace!("[js] console #{}: {}", idx, msg);
                push_console_message(&mut self.console, msg.clone());
            }
            self.engine.clear_console();

            // Print first 80 chars of result if not undefined.
            if !matches!(result, JsValue::Undefined) {
                let r = alloc::format!("{:?}", result);
                let truncated = if r.len() > 80 { &r[..80] } else { &r };
                js_trace!("[js] result #{}: {}", idx, truncated);
            }

            self.install_immediate_scheduler_fast_path();
            self.install_serverjs_define_fast_path();

            #[cfg(feature = "host")]
            if std::env::var_os("LIBWEBVIEW_DEBUG_SCRIPT_EVAL").is_some() {
                let module_count = self.engine.eval(
                    "try{typeof require==='function'&&typeof require('__debug')==='object'?Object.keys(require('__debug').modulesMap||{}).length:-1}catch(e){-2}",
                );
                self.engine.vm().last_exception = None;
                self.engine.vm().pending_exception = None;
                self.engine.vm().frames.clear();
                self.engine.vm().stack.clear();

                let pending_sjs = self.engine.eval(
                    "try{document.querySelectorAll('script[data-sjs]:not([data-processed])').length}catch(e){-1}",
                );
                self.engine.vm().last_exception = None;
                self.engine.vm().pending_exception = None;
                self.engine.vm().frames.clear();
                self.engine.vm().stack.clear();

                let step_limit = self.engine.vm().step_limit;
                let last_exc = self.engine.vm().last_exception.is_some();
                let pending_exc = self.engine.vm().pending_exception.is_some();
                anyos_std::println!(
                    "[js-debug] after #{}: bytes={} steps={} limit={} last_exc={} pending_exc={} modules={:?} pending_sjs={:?}",
                    idx,
                    script.len(),
                    steps_used,
                    step_limit,
                    last_exc,
                    pending_exc,
                    module_count,
                    pending_sjs
                );
            }
        }
        if scripts.len() > script_count {
            anyos_std::println!(
                "[js] skipped {} script(s) (limit={})",
                scripts.len() - script_count,
                limits.max_scripts
            );
        }

        self.install_serverjs_define_fast_path();
        self.install_serverjs_root_fast_path();

        // Disable interception.
        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
            NAVIGATION_TARGET = core::ptr::null_mut();
            EVENT_LISTENERS_TARGET = core::ptr::null_mut();
            MOTION_FINAL_STYLES_TARGET = core::ptr::null_mut();
        }

        self.mutations = bridge.mutations;
        self.virtual_nodes = bridge.virtual_nodes;
        self.next_virtual_id = bridge.next_virtual_id;
        self.real_node_ids = bridge.real_node_ids;
        self.event_listeners = bridge.event_listeners;
        self.pending_http_requests = bridge.pending_http_requests;
        self.pending_navigation_requests = bridge.pending_navigation_requests;
        self.next_timer_id = bridge.next_timer_id;
        extend_pending_timers(&mut self.timers, bridge.timers);
        self.active_style_animations
            .extend(bridge.pending_style_animations);
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
        js_trace!("[js] setup native api: event-callbacks begin");

        // Event callback storage (only tiny bit of eval for array init).
        vm.set_global(
            "__eventCallbacks",
            JsValue::Array(Rc::new(RefCell::new(JsArray::new()))),
        );
        js_trace!("[js] setup native api: event-callbacks done");

        // Create document object natively.
        js_trace!("[js] setup native api: document make begin");
        let doc = document::make_document(vm, dom, url, cookies);
        js_trace!("[js] setup native api: document make done");
        js_trace!("[js] setup native api: document global begin");
        vm.set_global("document", doc.clone());
        js_trace!("[js] setup native api: document global done");

        // Extract origin (scheme + "://" + host) for localStorage key isolation.
        js_trace!("[js] setup native api: origin extract begin");
        let origin = extract_origin(url);
        js_trace!("[js] setup native api: origin extract done: {}", origin);

        // Create window object natively.
        js_trace!("[js] setup native api: window make begin");
        let win = window::make_window(vm, doc, &origin, self.viewport_width, self.viewport_height);
        win.set_property(
            String::from("ScheduleJSWork"),
            native_fn("ScheduleJSWork", native_schedule_js_work),
        );
        js_trace!("[js] setup native api: window make done");
        js_trace!("[js] setup native api: global window begin");
        vm.set_global("window", win.clone());
        js_trace!("[js] setup native api: global window done");
        js_trace!("[js] setup native api: global self begin");
        vm.set_global("self", win.clone());
        js_trace!("[js] setup native api: global self done");
        js_trace!("[js] setup native api: global globalThis begin");
        vm.set_global("globalThis", win.clone());
        js_trace!("[js] setup native api: global globalThis done");

        // Google's gbar bootstrap installs a diagnostic `_DumpException`
        // hook early and then calls it from many catch blocks.  Provide the
        // browser-global namespace up front so a missing diagnostics hook does
        // not turn recoverable site exceptions into fatal script failures.
        let gbar = JsValue::Object(Rc::new(RefCell::new(JsObject::new())));
        gbar.set_property(
            String::from("_DumpException"),
            native_fn("_DumpException", |_, _| JsValue::Undefined),
        );
        win.set_property(String::from("gbar_"), gbar.clone());
        vm.set_global("gbar_", gbar);
        vm.set_global(
            "_DumpException",
            native_fn("_DumpException", |_, _| JsValue::Undefined),
        );

        // Browser semantics: `globalThis` is the Window object, so built-in
        // constructors and namespaces must also be visible as window properties.
        // Many modern bundles intentionally read `globalThis.Object`,
        // `globalThis.Symbol`, or typed-array constructors instead of bare
        // globals.  Keep these in sync before page scripts start.
        for key in &[
            "Object",
            "Array",
            "String",
            "Number",
            "Boolean",
            "Function",
            "Error",
            "TypeError",
            "RangeError",
            "ReferenceError",
            "SyntaxError",
            "URIError",
            "EvalError",
            "AggregateError",
            "Promise",
            "Map",
            "Set",
            "WeakMap",
            "WeakSet",
            "WeakRef",
            "FinalizationRegistry",
            "Date",
            "RegExp",
            "Symbol",
            "Proxy",
            "BigInt",
            "ArrayBuffer",
            "DataView",
            "Int8Array",
            "Uint8Array",
            "Uint8ClampedArray",
            "Int16Array",
            "Uint16Array",
            "Int32Array",
            "Uint32Array",
            "Float32Array",
            "Float64Array",
            "Math",
            "JSON",
            "console",
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "eval",
            "encodeURIComponent",
            "decodeURIComponent",
            "encodeURI",
            "decodeURI",
            "Infinity",
            "NaN",
            "undefined",
        ] {
            let val = vm.get_global(key);
            if !val.is_undefined() || *key == "undefined" {
                win.set_property(String::from(*key), val);
            }
        }

        // In browsers, window IS the global object — properties on window are directly
        // accessible as global variables (e.g. `MutationObserver` === `window.MutationObserver`).
        // Explicitly mirror all window constructors/functions as top-level globals so that
        // modern bundlers (Vite/React) that reference them without the `window.` prefix work.
        for key in &[
            "Node",
            "Window",
            "Document",
            "DocumentFragment",
            "ShadowRoot",
            "CharacterData",
            "CDATASection",
            "ProcessingInstruction",
            "DocumentType",
            "Text",
            "Comment",
            "Element",
            "HTMLElement",
            "NodeList",
            "HTMLCollection",
            "HTMLAnchorElement",
            "HTMLAreaElement",
            "HTMLBodyElement",
            "HTMLBRElement",
            "HTMLButtonElement",
            "HTMLCanvasElement",
            "HTMLDivElement",
            "HTMLFormElement",
            "HTMLHeadElement",
            "HTMLHeadingElement",
            "HTMLHtmlElement",
            "HTMLIFrameElement",
            "HTMLImageElement",
            "HTMLInputElement",
            "HTMLLabelElement",
            "HTMLLIElement",
            "HTMLLinkElement",
            "HTMLMediaElement",
            "HTMLMetaElement",
            "HTMLAudioElement",
            "HTMLVideoElement",
            "HTMLSourceElement",
            "HTMLPictureElement",
            "HTMLOptionElement",
            "HTMLParagraphElement",
            "HTMLScriptElement",
            "HTMLSelectElement",
            "HTMLSlotElement",
            "HTMLSpanElement",
            "HTMLStyleElement",
            "HTMLTableElement",
            "HTMLTemplateElement",
            "HTMLTextAreaElement",
            "HTMLUListElement",
            "HTMLUnknownElement",
            "SVGElement",
            "SVGSVGElement",
            "SVGGraphicsElement",
            "Attr",
            "NodeFilter",
            "CustomElementRegistry",
            "customElements",
            "MutationObserver",
            "ResizeObserver",
            "IntersectionObserver",
            "AbortController",
            "AbortSignal",
            "Blob",
            "DOMStringMap",
            "ReadableStream",
            "WritableStream",
            "TransformStream",
            "queueMicrotask",
            "ScheduleJSWork",
            "TextEncoder",
            "TextDecoder",
            "TextEncoderStream",
            "TextDecoderStream",
            "URL",
            "URLSearchParams",
            "EventTarget",
            "CustomEvent",
            "Event",
            "MouseEvent",
            "KeyboardEvent",
            "InputEvent",
            "FocusEvent",
            "WheelEvent",
            "PointerEvent",
            "MessageChannel",
            "structuredClone",
            "DOMParser",
            "performance",
            "history",
            "location",
            "localStorage",
            "sessionStorage",
            "navigator",
            "screen",
            "visualViewport",
            "matchMedia",
            "CSS",
            "getSelection",
            "scrollTo",
            "scrollBy",
            "addEventListener",
            "removeEventListener",
            "dispatchEvent",
            "__shady_native_addEventListener",
            "__shady_native_removeEventListener",
            "__shady_native_dispatchEvent",
            "setTimeout",
            "setInterval",
            "clearTimeout",
            "clearInterval",
            "requestAnimationFrame",
            "cancelAnimationFrame",
            "atob",
            "btoa",
            "fetch",
            "XMLHttpRequest",
            "Headers",
            "Request",
            "Response",
            "confirm",
            "prompt",
            "getCookie",
            "getParameterByName",
            "clearEventListeners",
            "clarity",
            "renderClarity",
            "getRequestUUID",
            "crypto",
            "__tcfapi",
            "__cmp",
            "__uspapi",
        ] {
            js_trace!("[js] setup native api: mirror {} begin", key);
            let val = win.get_property(key);
            if !val.is_undefined() {
                vm.set_global(key, val);
                js_trace!("[js] setup native api: mirror {} done", key);
            } else {
                js_trace!("[js] setup native api: mirror {} skipped(undefined)", key);
            }
        }

        // Top-level constructors/functions from window.
        js_trace!("[js] setup native api: top-level globals begin");
        vm.set_global("alert", native_fn("alert", window::native_alert));
        vm.set_global("fetch", native_fn("fetch", fetch::native_fetch));
        vm.set_global("XMLHttpRequest", xhr::make_xhr_constructor());
        vm.set_global("WebSocket", websocket::make_ws_constructor());
        vm.set_global("Headers", fetch::make_headers_constructor());
        vm.set_global("Request", fetch::make_request_constructor());
        vm.set_global("Response", fetch::make_response_constructor());
        vm.set_global(
            "Image",
            native_ctor_fn("Image", document::native_image_ctor),
        );
        vm.set_global("FormData", native_ctor_fn("FormData", native_formdata_ctor));
        js_trace!("[js] setup native api: top-level globals done");

        // Timer globals.
        js_trace!("[js] setup native api: timer globals begin");
        vm.set_global("setTimeout", native_fn("setTimeout", native_set_timeout));
        vm.set_global("setInterval", native_fn("setInterval", native_set_interval));
        vm.set_global(
            "setImmediate",
            native_fn("setImmediate", native_set_immediate),
        );
        vm.set_global(
            "clearTimeout",
            native_fn("clearTimeout", native_clear_timeout),
        );
        vm.set_global(
            "clearInterval",
            native_fn("clearInterval", native_clear_interval),
        );
        vm.set_global(
            "clearImmediate",
            native_fn("clearImmediate", native_clear_timeout),
        );
        vm.set_global(
            "requestAnimationFrame",
            native_fn("requestAnimationFrame", native_request_animation_frame),
        );
        vm.set_global(
            "cancelAnimationFrame",
            native_fn("cancelAnimationFrame", native_clear_timeout),
        );
        js_trace!("[js] setup native api: timer globals done");
    }

    pub fn eval(&mut self, source: &str) -> JsValue {
        let result = self.engine.eval(source);
        self.collect_engine_console();
        result
    }

    pub fn eval_with_dom(&mut self, source: &str, dom: &Dom) -> JsValue {
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            installed_event_listeners: &self.event_listeners as *const Vec<EventListener>,
            next_virtual_id: self.next_virtual_id,
            virtual_nodes: self.virtual_nodes.clone(),
            real_node_ids: self.real_node_ids.clone(),
            pending_http_requests: Vec::new(),
            pending_navigation_requests: Vec::new(),
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
            pending_style_animations: Vec::new(),
            motion_final_styles: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;

        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
            NAVIGATION_TARGET =
                &mut bridge.pending_navigation_requests as *mut Vec<PendingNavigationRequest>;
            EVENT_LISTENERS_TARGET = &mut bridge.event_listeners as *mut Vec<EventListener>;
            MOTION_FINAL_STYLES_TARGET =
                &mut bridge.motion_final_styles as *mut Vec<MotionFinalStyle>;
        }
        let result = self.engine.eval(source);
        if let Some(exc) = self.engine.vm().last_exception.take() {
            self.console
                .push(alloc::format!("[exception] {}", js_exception_summary(&exc)));
        }
        if let Some(exc) = self.engine.vm().pending_exception.take() {
            self.console.push(alloc::format!(
                "[pending exception] {}",
                js_exception_summary(&exc)
            ));
        }
        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
            NAVIGATION_TARGET = core::ptr::null_mut();
            EVENT_LISTENERS_TARGET = core::ptr::null_mut();
            MOTION_FINAL_STYLES_TARGET = core::ptr::null_mut();
        }

        self.collect_engine_console();
        self.mutations.extend(bridge.mutations);
        self.virtual_nodes = bridge.virtual_nodes;
        self.next_virtual_id = bridge.next_virtual_id;
        self.real_node_ids = bridge.real_node_ids;
        self.event_listeners.extend(bridge.event_listeners);
        self.apply_remove_listeners(&bridge.remove_listeners);
        self.pending_http_requests
            .extend(bridge.pending_http_requests);
        self.pending_navigation_requests
            .extend(bridge.pending_navigation_requests);
        self.next_timer_id = bridge.next_timer_id;
        extend_pending_timers(&mut self.timers, bridge.timers);
        self.active_style_animations
            .extend(bridge.pending_style_animations);
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
        self.virtual_nodes.clear();
        self.next_virtual_id = -1;
        self.real_node_ids.clear();
        self.event_listeners.clear();
        self.pending_http_requests.clear();
        self.pending_navigation_requests.clear();
        self.timers.clear();
        self.next_timer_id = 1;
        self.cookies.clear();
        self.pending_ws_connects.clear();
        self.pending_ws_sends.clear();
        self.pending_ws_closes.clear();
        self.ws_registry.clear();
        self.active_animations.clear();
        self.active_transitions.clear();
        self.active_style_animations.clear();
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

    pub fn take_pending_navigation_requests(&mut self) -> Vec<PendingNavigationRequest> {
        core::mem::take(&mut self.pending_navigation_requests)
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
        self.collect_engine_console();
    }

    /// Apply recorded mutations to the real DOM.
    /// Returns a map from virtual_id → real NodeId for newly created elements.
    pub fn apply_mutations(&mut self, dom: &mut Dom) -> BTreeMap<i64, usize> {
        let mutations = core::mem::take(&mut self.mutations);
        let host_side_effects: Vec<DomMutation> = mutations
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    DomMutation::SetCookie { .. }
                        | DomMutation::FormSubmit { .. }
                        | DomMutation::FormReset { .. }
                )
            })
            .cloned()
            .collect();
        let mut id_map: BTreeMap<i64, usize> = BTreeMap::new();
        let mut created_ids: Vec<usize> = Vec::new();
        let mut expected_parents: Vec<(usize, usize)> = Vec::new();
        let mut explicitly_detached: Vec<usize> = Vec::new();

        for m in &mutations {
            match m {
                DomMutation::CreateElement { virtual_id, tag } => {
                    let real_tag = Tag::from_str(tag);
                    // Copy attributes from virtual node if they were set before insertion
                    let mut attrs: Vec<crate::dom::Attr> = self
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
                    if real_tag == Tag::Unknown && attrs.iter().all(|a| a.name != "\x00") {
                        attrs.push(crate::dom::Attr {
                            name: String::from("\x00"),
                            value: tag.to_ascii_lowercase(),
                        });
                    }
                    let real_id = dom.add_node(
                        NodeType::Element {
                            tag: real_tag,
                            attrs,
                        },
                        None,
                    );
                    id_map.insert(*virtual_id, real_id);
                    self.real_node_ids.insert(*virtual_id, real_id);
                    created_ids.push(real_id);
                }
                DomMutation::CreateTextNode { virtual_id, text } => {
                    let real_id = dom.add_node(NodeType::Text(text.clone()), None);
                    id_map.insert(*virtual_id, real_id);
                    self.real_node_ids.insert(*virtual_id, real_id);
                    created_ids.push(real_id);
                }
                _ => {}
            }
        }

        for m in &mutations {
            match m {
                DomMutation::CreateElement { .. } | DomMutation::CreateTextNode { .. } => {}
                DomMutation::SetAttribute {
                    node_id,
                    name,
                    value,
                } => {
                    if debug_class_mutations_enabled()
                        && name.eq_ignore_ascii_case("class")
                        && (debug_all_class_mutations_enabled()
                            || value.contains("max-w-7xl")
                            || value.contains("text-center")
                            || value.contains("relative mx-auto")
                            || value.contains("max-w-4xl"))
                    {
                        #[cfg(feature = "host")]
                        eprintln!(
                            "[js-dom-debug] apply SetAttribute class node_id={} value={}",
                            node_id, value
                        );
                    }
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
                        dom.set_attr(real_id, name, value);
                    } else if *node_id < 0 {
                        if let Some(vn) = self.virtual_nodes.iter_mut().find(|vn| vn.id == *node_id)
                        {
                            if let Some((_, existing)) =
                                vn.attrs.iter_mut().find(|(k, _)| k == name)
                            {
                                *existing = value.clone();
                            } else {
                                vn.attrs.push((name.clone(), value.clone()));
                            }
                        }
                    }
                }
                DomMutation::RemoveAttribute { node_id, name } => {
                    if debug_class_mutations_enabled() && name.eq_ignore_ascii_case("class") {
                        #[cfg(feature = "host")]
                        eprintln!(
                            "[js-dom-debug] apply RemoveAttribute class node_id={}",
                            node_id
                        );
                    }
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
                        dom.remove_attr(real_id, name);
                    } else if *node_id < 0 {
                        if let Some(vn) = self.virtual_nodes.iter_mut().find(|vn| vn.id == *node_id)
                        {
                            vn.attrs.retain(|(k, _)| k != name);
                        }
                    }
                }
                DomMutation::SetTextContent { node_id, text } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
                        dom.set_text(real_id, text);
                    } else if *node_id < 0 {
                        if let Some(vn) = self.virtual_nodes.iter_mut().find(|vn| vn.id == *node_id)
                        {
                            vn.text_content = text.clone();
                        }
                    }
                }
                DomMutation::AppendChild {
                    parent_id,
                    child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map, &self.real_node_ids);
                    let real_child = resolve_id(*child_id, &id_map, &self.real_node_ids);
                    if debug_dom_apply_enabled() {
                        #[cfg(feature = "host")]
                        eprintln!(
                            "[js-dom-apply] AppendChild parent={} child={} -> parent={:?} child={:?}",
                            parent_id, child_id, real_parent, real_child
                        );
                    }
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.append_child(p, c);
                        expected_parents.push((c, p));
                    }
                }
                DomMutation::RemoveChild {
                    parent_id,
                    child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map, &self.real_node_ids);
                    let real_child = resolve_id(*child_id, &id_map, &self.real_node_ids);
                    if let (Some(p), Some(c)) = (real_parent, real_child) {
                        dom.remove_child(p, c);
                        explicitly_detached.push(c);
                    }
                }
                DomMutation::InsertBefore {
                    parent_id,
                    new_child_id,
                    ref_child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map, &self.real_node_ids);
                    let real_new = resolve_id(*new_child_id, &id_map, &self.real_node_ids);
                    let real_ref = resolve_id(*ref_child_id, &id_map, &self.real_node_ids);
                    if debug_dom_apply_enabled() {
                        #[cfg(feature = "host")]
                        eprintln!(
                            "[js-dom-apply] InsertBefore parent={} new={} ref={} -> parent={:?} new={:?} ref={:?}",
                            parent_id, new_child_id, ref_child_id, real_parent, real_new, real_ref
                        );
                    }
                    if let (Some(p), Some(n)) = (real_parent, real_new) {
                        if let Some(r) = real_ref {
                            dom.insert_before(p, n, r);
                        } else {
                            dom.append_child(p, n);
                        }
                        expected_parents.push((n, p));
                    }
                }
                DomMutation::ReplaceChild {
                    parent_id,
                    new_child_id,
                    old_child_id,
                } => {
                    let real_parent = resolve_id(*parent_id, &id_map, &self.real_node_ids);
                    let real_new = resolve_id(*new_child_id, &id_map, &self.real_node_ids);
                    let real_old = resolve_id(*old_child_id, &id_map, &self.real_node_ids);
                    if let (Some(p), Some(n), Some(o)) = (real_parent, real_new, real_old) {
                        dom.remove_child(p, o);
                        explicitly_detached.push(o);
                        dom.append_child(p, n);
                        expected_parents.push((n, p));
                    }
                }
                DomMutation::RemoveNode { node_id } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
                        // Remove from parent.
                        if let Some(pid) = dom.nodes.get(real_id).and_then(|n| n.parent) {
                            dom.remove_child(pid, real_id);
                        }
                        explicitly_detached.push(real_id);
                    }
                }
                DomMutation::SetInnerHTML { node_id, html } => {
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
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
                    if let Some(real_id) = resolve_id(*node_id, &id_map, &self.real_node_ids) {
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
        for (child, parent) in expected_parents {
            if explicitly_detached.iter().any(|&id| id == child) {
                continue;
            }
            if child >= dom.nodes.len() || parent >= dom.nodes.len() {
                continue;
            }
            if dom.nodes[child].parent != Some(parent) {
                if debug_dom_apply_enabled() {
                    #[cfg(feature = "host")]
                    eprintln!(
                        "[js-dom-apply] repairing parent link child={} parent={}",
                        child, parent
                    );
                }
                dom.append_child(parent, child);
            }
        }
        self.adopt_detached_framework_roots(dom, &created_ids);
        self.mutations.extend(host_side_effects);
        id_map
    }

    fn adopt_detached_framework_roots(&self, dom: &mut Dom, created_ids: &[usize]) {
        let Some(root_id) = find_element_by_id(dom, "root") else {
            return;
        };
        if !dom.nodes[root_id].children.is_empty() {
            return;
        }

        let mut roots: Vec<usize> = Vec::new();
        for &node_id in created_ids {
            let Some(node) = dom.nodes.get(node_id) else {
                continue;
            };
            if node.parent.is_some() || !detached_node_is_visual_root(node) {
                continue;
            }
            roots.push(node_id);
        }
        if roots.is_empty() {
            return;
        }

        if debug_dom_apply_enabled() {
            #[cfg(feature = "host")]
            eprintln!(
                "[js-dom-apply] adopting {} detached framework root(s) into #root",
                roots.len()
            );
        }
        for node_id in roots {
            dom.append_child(root_id, node_id);
        }
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
        if !self.has_event_listener_on_path_slice(&path, event_name) {
            return true;
        }

        // Whether this event type bubbles by default.
        // (focus/blur/scroll/load do not bubble — they have focused-capture semantics.)
        let bubbles = !matches!(
            event_name,
            "focus" | "blur" | "scroll" | "load" | "unload" | "error" | "mouseenter" | "mouseleave"
        );
        let cancelable = !matches!(event_name, "scroll" | "load" | "unload");

        // Build the event object with full W3C properties.
        let target_el = element::make_element(self.engine.vm(), node_id as i64);
        let evt = build_event_object(event_name, data, target_el, bubbles, cancelable);
        let window_obj = self.engine.vm().get_global("window");

        // Set up bridge so DOM-access and event-control native functions work.
        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            installed_event_listeners: &self.event_listeners as *const Vec<EventListener>,
            next_virtual_id: self.next_virtual_id,
            virtual_nodes: self.virtual_nodes.clone(),
            real_node_ids: self.real_node_ids.clone(),
            pending_http_requests: Vec::new(),
            pending_navigation_requests: Vec::new(),
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
            pending_style_animations: Vec::new(),
            motion_final_styles: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
            NAVIGATION_TARGET =
                &mut bridge.pending_navigation_requests as *mut Vec<PendingNavigationRequest>;
            EVENT_LISTENERS_TARGET = &mut bridge.event_listeners as *mut Vec<EventListener>;
            MOTION_FINAL_STYLES_TARGET =
                &mut bridge.motion_final_styles as *mut Vec<MotionFinalStyle>;
        }

        // ── Phase 1: CAPTURE (eventPhase = 1) ───────────────────────────────
        // Window is the top of the composed event path in browsers. Many
        // framework event systems install delegated capture/bubble handlers
        // there, so it must fire around the DOM path.
        if !bridge.propagation_stopped && !bridge.immediate_stopped {
            evt.set_property(String::from("currentTarget"), window_obj.clone());
            evt.set_property(String::from("eventPhase"), JsValue::Number(1.0));

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == usize::MAX && l.event == event_name && l.capture)
                .map(|l| l.callback.clone())
                .collect();

            for cb in &matching {
                call_event_listener(self.engine.vm(), cb, &evt, &window_obj);
                evt.set_property(
                    String::from("defaultPrevented"),
                    JsValue::Bool(bridge.prevented),
                );
                if bridge.immediate_stopped || bridge.propagation_stopped {
                    break;
                }
            }
        }

        // Fire capture-listeners from root down to (but not including) the target.
        'capture: for i in 0..target_idx {
            if bridge.propagation_stopped || bridge.immediate_stopped {
                break;
            }
            let nid = path[i];
            let cur_el = element::make_element(self.engine.vm(), nid as i64);
            evt.set_property(String::from("currentTarget"), cur_el);
            evt.set_property(String::from("eventPhase"), JsValue::Number(1.0));
            let this_val = element::make_element(self.engine.vm(), nid as i64);

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == nid && l.event == event_name && l.capture)
                .map(|l| l.callback.clone())
                .collect();

            for cb in &matching {
                call_event_listener(self.engine.vm(), cb, &evt, &this_val);
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
            let this_val = element::make_element(self.engine.vm(), node_id as i64);

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == node_id && l.event == event_name)
                .map(|l| l.callback.clone())
                .collect();

            'target: for cb in &matching {
                call_event_listener(self.engine.vm(), cb, &evt, &this_val);
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
                let this_val = element::make_element(self.engine.vm(), nid as i64);

                let matching: Vec<JsValue> = self
                    .event_listeners
                    .iter()
                    .filter(|l| l.node_id == nid && l.event == event_name && !l.capture)
                    .map(|l| l.callback.clone())
                    .collect();

                for cb in &matching {
                    call_event_listener(self.engine.vm(), cb, &evt, &this_val);
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

        // Bubble listeners on window fire after the DOM ancestors.
        if bubbles && !bridge.propagation_stopped && !bridge.immediate_stopped {
            evt.set_property(String::from("currentTarget"), window_obj.clone());
            evt.set_property(String::from("eventPhase"), JsValue::Number(3.0));

            let matching: Vec<JsValue> = self
                .event_listeners
                .iter()
                .filter(|l| l.node_id == usize::MAX && l.event == event_name && !l.capture)
                .map(|l| l.callback.clone())
                .collect();

            for cb in &matching {
                call_event_listener(self.engine.vm(), cb, &evt, &window_obj);
                evt.set_property(
                    String::from("defaultPrevented"),
                    JsValue::Bool(bridge.prevented),
                );
                if bridge.immediate_stopped || bridge.propagation_stopped {
                    break;
                }
            }
        }

        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
            NAVIGATION_TARGET = core::ptr::null_mut();
            EVENT_LISTENERS_TARGET = core::ptr::null_mut();
            MOTION_FINAL_STYLES_TARGET = core::ptr::null_mut();
        }

        // Drain microtask queue after event dispatch (ECMAScript spec:
        // microtasks run to completion after each task/callback).
        self.engine.vm().drain_microtasks();

        // Collect all side-effects from the dispatch.
        self.collect_engine_console();
        self.mutations.extend(bridge.mutations);
        self.virtual_nodes = bridge.virtual_nodes;
        self.next_virtual_id = bridge.next_virtual_id;
        self.real_node_ids = bridge.real_node_ids;
        self.event_listeners.extend(bridge.event_listeners);
        self.apply_remove_listeners(&bridge.remove_listeners);
        self.pending_http_requests
            .extend(bridge.pending_http_requests);
        self.pending_navigation_requests
            .extend(bridge.pending_navigation_requests);
        self.next_timer_id = bridge.next_timer_id;
        extend_pending_timers(&mut self.timers, bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();

        // Return true when preventDefault() was NOT called (default action proceeds).
        !bridge.prevented
    }

    pub fn has_event_listener_for_node_path(
        &self,
        dom: &Dom,
        node_id: usize,
        event_name: &str,
    ) -> bool {
        let mut path = Vec::new();
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            path.push(id);
            cur = dom.nodes.get(id).and_then(|n| n.parent);
        }
        self.has_event_listener_on_path_slice(&path, event_name)
    }

    fn has_event_listener_on_path_slice(&self, path: &[usize], event_name: &str) -> bool {
        self.event_listeners
            .iter()
            .any(|l| l.node_id == usize::MAX && l.event == event_name)
            || path.iter().any(|&nid| {
                self.event_listeners
                    .iter()
                    .any(|l| l.node_id == nid && l.event == event_name)
            })
    }

    /// Advance timers by `delta_ms` and execute any that are due.
    /// Returns the number of timers fired.
    pub fn tick(&mut self, dom: &Dom, delta_ms: u64) -> usize {
        self.tick_with_budget(dom, delta_ms, usize::MAX)
    }

    pub fn has_pending_microtasks(&self) -> bool {
        self.engine.has_pending_microtasks()
    }

    pub fn has_pending_js_work(&self) -> bool {
        !self.timers.is_empty() || self.has_pending_microtasks()
    }

    fn run_microtask_checkpoint(&mut self, dom: &Dom) -> bool {
        if !self.engine.has_pending_microtasks() {
            return false;
        }

        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            eprintln!("[js-dom-debug] drain microtasks");
        }

        let mut bridge = DomBridge {
            dom: dom as *const Dom,
            mutations: Vec::new(),
            event_listeners: Vec::new(),
            installed_event_listeners: &self.event_listeners as *const Vec<EventListener>,
            next_virtual_id: self.next_virtual_id,
            virtual_nodes: core::mem::take(&mut self.virtual_nodes),
            real_node_ids: core::mem::take(&mut self.real_node_ids),
            pending_http_requests: Vec::new(),
            pending_navigation_requests: Vec::new(),
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
            pending_style_animations: Vec::new(),
            motion_final_styles: Vec::new(),
        };
        self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
        unsafe {
            MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
            VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
            NAVIGATION_TARGET =
                &mut bridge.pending_navigation_requests as *mut Vec<PendingNavigationRequest>;
            EVENT_LISTENERS_TARGET = &mut bridge.event_listeners as *mut Vec<EventListener>;
            MOTION_FINAL_STYLES_TARGET =
                &mut bridge.motion_final_styles as *mut Vec<MotionFinalStyle>;
        }

        self.engine
            .set_step_limit(configured_timer_callback_step_limit());
        self.engine.vm().steps = 0;
        self.engine.vm().drain_microtasks();

        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            if let Some(ref exc) = self.engine.vm().last_exception {
                eprintln!(
                    "[js-dom-debug] microtask exception: {}",
                    js_exception_summary(exc)
                );
            }
            if let Some(ref exc) = self.engine.vm().pending_exception {
                eprintln!(
                    "[js-dom-debug] microtask pending exception: {}",
                    js_exception_summary(exc)
                );
            }
        }

        self.engine.vm().last_exception = None;
        self.engine.vm().pending_exception = None;
        self.engine.vm().frames.clear();
        self.engine.vm().stack.clear();

        unsafe {
            MUTATION_TARGET = core::ptr::null_mut();
            VIRTUAL_NODES_TARGET = core::ptr::null_mut();
            NAVIGATION_TARGET = core::ptr::null_mut();
            EVENT_LISTENERS_TARGET = core::ptr::null_mut();
            MOTION_FINAL_STYLES_TARGET = core::ptr::null_mut();
        }
        self.collect_engine_console();
        for log_msg in self.engine.vm().engine_log.iter() {
            js_trace!("[js] microtask: {}", log_msg);
        }
        self.engine.vm().engine_log.clear();

        let produced_work = !bridge.mutations.is_empty()
            || !bridge.event_listeners.is_empty()
            || !bridge.remove_listeners.is_empty()
            || !bridge.pending_http_requests.is_empty()
            || !bridge.pending_navigation_requests.is_empty()
            || !bridge.pending_ws_connects.is_empty()
            || !bridge.pending_ws_sends.is_empty()
            || !bridge.pending_ws_closes.is_empty()
            || !bridge.pending_style_animations.is_empty()
            || !bridge.motion_final_styles.is_empty()
            || !bridge.timers.is_empty();

        self.mutations.extend(bridge.mutations);
        self.virtual_nodes = bridge.virtual_nodes;
        self.next_virtual_id = bridge.next_virtual_id;
        self.real_node_ids = bridge.real_node_ids;
        self.event_listeners.extend(bridge.event_listeners);
        self.pending_http_requests
            .extend(bridge.pending_http_requests);
        self.pending_navigation_requests
            .extend(bridge.pending_navigation_requests);
        self.next_timer_id = bridge.next_timer_id;
        self.active_style_animations
            .extend(bridge.pending_style_animations);
        extend_pending_timers(&mut self.timers, bridge.timers);
        self.engine.vm().userdata = core::ptr::null_mut();

        produced_work
    }

    /// Advance timers by `delta_ms`, executing at most `max_callbacks` due
    /// callbacks. Due timers beyond the budget remain queued for the next host
    /// tick so timer-heavy pages cannot monopolize the UI thread.
    pub fn tick_with_budget(&mut self, dom: &Dom, delta_ms: u64, max_callbacks: usize) -> usize {
        self.total_elapsed_ms += delta_ms;

        // Short-circuit only when neither macrotasks nor Promise jobs exist.
        if self.timers.is_empty() {
            self.run_microtask_checkpoint(dom);
            return 0;
        }

        let mut fired = 0usize;
        let mut keep = Vec::new();
        let timers = core::mem::take(&mut self.timers);

        for mut t in timers {
            t.elapsed_ms += delta_ms;
            if t.elapsed_ms >= t.delay_ms {
                if fired >= max_callbacks {
                    keep.push(t);
                    continue;
                }
                // Timer is due — execute callback.
                #[cfg(feature = "host")]
                if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "[js-dom-debug] fire timer id={} delay={} raf={} elapsed={}",
                        t.id, t.delay_ms, t.is_raf, t.elapsed_ms
                    );
                }
                let mut bridge = DomBridge {
                    dom: dom as *const Dom,
                    mutations: Vec::new(),
                    event_listeners: Vec::new(),
                    installed_event_listeners: &self.event_listeners as *const Vec<EventListener>,
                    next_virtual_id: self.next_virtual_id,
                    virtual_nodes: core::mem::take(&mut self.virtual_nodes),
                    real_node_ids: core::mem::take(&mut self.real_node_ids),
                    pending_http_requests: Vec::new(),
                    pending_navigation_requests: Vec::new(),
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
                    pending_style_animations: Vec::new(),
                    motion_final_styles: Vec::new(),
                };
                self.engine.vm().userdata = &mut bridge as *mut DomBridge as *mut u8;
                unsafe {
                    MUTATION_TARGET = &mut bridge.mutations as *mut Vec<DomMutation>;
                    VIRTUAL_NODES_TARGET = &mut bridge.virtual_nodes as *mut Vec<VirtualNode>;
                    NAVIGATION_TARGET = &mut bridge.pending_navigation_requests
                        as *mut Vec<PendingNavigationRequest>;
                    EVENT_LISTENERS_TARGET = &mut bridge.event_listeners as *mut Vec<EventListener>;
                    MOTION_FINAL_STYLES_TARGET =
                        &mut bridge.motion_final_styles as *mut Vec<MotionFinalStyle>;
                }

                // Timer callbacks should be short tasks. Heavy recurring
                // analytics/ad loops must not burn a full script-sized budget
                // on every frame.
                self.engine
                    .set_step_limit(configured_timer_callback_step_limit());
                self.engine.vm().steps = 0;
                // rAF callbacks receive a DOMHighResTimeStamp (W3C spec).
                let cb_args: Vec<JsValue> = if t.is_raf {
                    vec![JsValue::Number(self.total_elapsed_ms as f64)]
                } else {
                    t.args.clone()
                };
                self.engine
                    .vm()
                    .call_value(&t.callback, &cb_args, t.this_arg.clone());

                // Drain microtask queue after each macrotask (ECMAScript spec:
                // all microtasks run to completion before the next macrotask).
                self.engine.vm().drain_microtasks();

                #[cfg(feature = "host")]
                if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
                    if let Some(ref exc) = self.engine.vm().last_exception {
                        eprintln!(
                            "[js-dom-debug] timer id={} exception: {}",
                            t.id,
                            js_exception_summary(exc)
                        );
                    }
                    if let Some(ref exc) = self.engine.vm().pending_exception {
                        eprintln!(
                            "[js-dom-debug] timer id={} pending exception: {}",
                            t.id,
                            js_exception_summary(exc)
                        );
                    }
                }

                // Clear any timer callback exceptions so next timer can run fresh.
                self.engine.vm().last_exception = None;
                self.engine.vm().pending_exception = None;
                self.engine.vm().frames.clear();
                self.engine.vm().stack.clear();

                unsafe {
                    MUTATION_TARGET = core::ptr::null_mut();
                    VIRTUAL_NODES_TARGET = core::ptr::null_mut();
                    NAVIGATION_TARGET = core::ptr::null_mut();
                    EVENT_LISTENERS_TARGET = core::ptr::null_mut();
                    MOTION_FINAL_STYLES_TARGET = core::ptr::null_mut();
                }
                self.collect_engine_console();
                // Print engine log from timer callback for diagnostics
                for log_msg in self.engine.vm().engine_log.iter() {
                    js_trace!("[js] timer: {}", log_msg);
                }
                self.engine.vm().engine_log.clear();
                let quiet_self_reschedule = bridge.mutations.is_empty()
                    && bridge.event_listeners.is_empty()
                    && bridge.remove_listeners.is_empty()
                    && bridge.pending_http_requests.is_empty()
                    && bridge.pending_navigation_requests.is_empty()
                    && bridge.pending_ws_connects.is_empty()
                    && bridge.pending_ws_sends.is_empty()
                    && bridge.pending_ws_closes.is_empty()
                    && bridge.pending_style_animations.is_empty()
                    && bridge.motion_final_styles.is_empty();
                if quiet_self_reschedule {
                    for next in &mut bridge.timers {
                        if !next.repeat
                            && !next.is_raf
                            && next.delay_ms < QUIET_SELF_RESCHEDULE_MIN_DELAY_MS
                            && same_timer_callback(&next.callback, &t.callback)
                        {
                            next.delay_ms = QUIET_SELF_RESCHEDULE_MIN_DELAY_MS;
                        }
                    }
                }

                self.mutations.extend(bridge.mutations);
                self.virtual_nodes = bridge.virtual_nodes;
                self.next_virtual_id = bridge.next_virtual_id;
                self.real_node_ids = bridge.real_node_ids;
                self.event_listeners.extend(bridge.event_listeners);
                self.pending_http_requests
                    .extend(bridge.pending_http_requests);
                self.pending_navigation_requests
                    .extend(bridge.pending_navigation_requests);
                self.next_timer_id = bridge.next_timer_id;
                self.active_style_animations
                    .extend(bridge.pending_style_animations);
                // New timers created during callback.
                extend_pending_timers(&mut keep, bridge.timers);
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
        self.run_microtask_checkpoint(dom);
        fired
    }

    pub fn next_timer_delay_ms(&self) -> Option<u64> {
        self.timers
            .iter()
            .map(|timer| timer.delay_ms.saturating_sub(timer.elapsed_ms))
            .min()
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

    /// Advance native Web Animations API animations created by Element.animate().
    ///
    /// These animations write only compositor-friendly inline style properties
    /// (`opacity`, `transform`) into the normal mutation path. That keeps the
    /// implementation cheap and lets the existing incremental relayout/paint
    /// path decide how much needs to be refreshed.
    pub fn advance_style_animations(&mut self, delta_ms: u64) -> usize {
        if self.active_style_animations.is_empty() {
            return 0;
        }

        let mut changed = 0usize;
        let mut keep = Vec::new();
        let animations = core::mem::take(&mut self.active_style_animations);
        for mut anim in animations {
            anim.elapsed_ms = anim.elapsed_ms.saturating_add(delta_ms);
            let active_elapsed = anim.elapsed_ms.saturating_sub(anim.delay_ms);
            let duration = anim.duration_ms.max(1);
            let iteration = active_elapsed / duration;
            let iteration_count = if anim.iterations == 0 {
                u64::MAX
            } else {
                anim.iterations as u64
            };
            let finished = iteration >= iteration_count;
            let local_elapsed = if finished {
                duration
            } else {
                active_elapsed % duration
            };
            let t = if anim.elapsed_ms < anim.delay_ms {
                0.0
            } else {
                (local_elapsed as f32 / duration as f32).clamp(0.0, 1.0)
            };

            if !finished || anim.fill_forwards {
                if let (Some(from), Some(to)) = (anim.from_opacity, anim.to_opacity) {
                    let value = from + (to - from) * t;
                    self.mutations.push(DomMutation::SetStyleProperty {
                        node_id: anim.node_id,
                        property: String::from("opacity"),
                        value: format!("{}", value.clamp(0.0, 1.0)),
                    });
                    changed += 1;
                }
                if let (Some(from), Some(to)) = (&anim.from_transform, &anim.to_transform) {
                    let value = interpolate_transform_value(from, to, (t * 1000.0) as i32)
                        .unwrap_or_else(|| if t >= 1.0 { to.clone() } else { from.clone() });
                    self.mutations.push(DomMutation::SetStyleProperty {
                        node_id: anim.node_id,
                        property: String::from("transform"),
                        value,
                    });
                    changed += 1;
                }
            }

            if !finished {
                keep.push(anim);
            }
        }
        self.active_style_animations = keep;
        changed
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
        if let Some(nid) = bridge.resolve_node_id(node_id) {
            let dom = bridge.dom();
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
    if let Some(bridge) = get_bridge(vm) {
        let Some(nid) = bridge.resolve_node_id(node_id) else {
            return String::new();
        };
        let dom = bridge.dom();
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
fn resolve_id(
    id: i64,
    map: &BTreeMap<i64, usize>,
    persistent_map: &BTreeMap<i64, usize>,
) -> Option<usize> {
    if id >= 0 {
        Some(id as usize)
    } else {
        map.get(&id)
            .copied()
            .or_else(|| persistent_map.get(&id).copied())
    }
}

fn same_timer_callback(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Function(a), JsValue::Function(b)) => alloc::rc::Rc::ptr_eq(a, b),
        (JsValue::Object(a), JsValue::Object(b)) => alloc::rc::Rc::ptr_eq(a, b),
        _ => false,
    }
}

fn find_element_by_id(dom: &Dom, id_value: &str) -> Option<usize> {
    for (idx, node) in dom.nodes.iter().enumerate() {
        let NodeType::Element { attrs, .. } = &node.node_type else {
            continue;
        };
        if attrs
            .iter()
            .any(|attr| attr.name.eq_ignore_ascii_case("id") && attr.value == id_value)
        {
            return Some(idx);
        }
    }
    None
}

fn detached_node_is_visual_root(node: &crate::dom::DomNode) -> bool {
    let NodeType::Element { tag, attrs } = &node.node_type else {
        return false;
    };
    if matches!(
        tag,
        Tag::Html | Tag::Head | Tag::Body | Tag::Link | Tag::Meta | Tag::Script | Tag::Style
    ) {
        return false;
    }
    !node.children.is_empty()
        || attrs.iter().any(|attr| {
            attr.name.eq_ignore_ascii_case("class")
                || attr.name.eq_ignore_ascii_case("id")
                || attr.name.eq_ignore_ascii_case("src")
                || attr.name.eq_ignore_ascii_case("href")
        })
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
pub(super) fn build_event_object(
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
    obj.set(
        String::from("getAll"),
        native_fn("getAll", formdata_get_all),
    );
    obj.set(String::from("has"), native_fn("has", formdata_has));
    obj.set(String::from("delete"), native_fn("delete", formdata_delete));
    obj.set(
        String::from("entries"),
        native_fn("entries", formdata_entries),
    );
    obj.set(String::from("keys"), native_fn("keys", formdata_keys));
    obj.set(String::from("values"), native_fn("values", formdata_values));
    obj.set(
        String::from("forEach"),
        native_fn("forEach", formdata_foreach),
    );
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

#[cfg(test)]
mod tests {
    use super::{
        extract_module_specifiers_for_page_with_page_id, extract_vike_page_id_from_dom, JsRuntime,
    };
    use crate::html;
    use alloc::string::String;
    use libjs::JsValue;

    #[test]
    fn vike_page_id_is_extracted_from_json_script() {
        let dom = html::parse(
            r#"<script id="vike_pageContext" type="application/json">{"pageId":"\/src\/frontend\/pages\/generated-module-pages\/channel-1\/focus"}</script>"#,
        );

        assert_eq!(
            extract_vike_page_id_from_dom(&dom).as_deref(),
            Some("/src/frontend/pages/generated-module-pages/channel-1/focus")
        );
    }

    #[test]
    fn page_aware_module_scan_prefetches_only_current_vike_route() {
        let source = r#"
            const pageFilesLazy = {
                "/src/frontend/pages/generated-module-pages/channel-0/focus": () => import("./src_frontend_pages_generated-module-pages_channel-0_focus.A.js"),
                "/src/frontend/pages/generated-module-pages/channel-1/focus": () => import("./src_frontend_pages_generated-module-pages_channel-1_focus.B.js"),
                "/src/frontend/pages/generated-module-pages_news-article-0/focus": () => import("./src_frontend_pages_generated-module-pages_news-article-0_focus.C.js")
            };
            import "./chunk-static.js";
        "#;

        let specs = extract_module_specifiers_for_page_with_page_id(
            source,
            "https://www.focus.de/",
            Some("/src/frontend/pages/generated-module-pages/channel-1/focus"),
        );

        assert!(specs.iter().any(|s| s == "./chunk-static.js"));
        assert!(specs
            .iter()
            .any(|s| s == "./src_frontend_pages_generated-module-pages_channel-1_focus.B.js"));
        assert!(!specs
            .iter()
            .any(|s| s.contains("channel-0_focus") || s.contains("news-article-0_focus")));
    }

    #[test]
    fn browser_native_constructors_are_constructable() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            var smokeOk = true;
            var smokeCount = 0;
            var smokeFailures = '';
            function checkSmoke(name, fn) {
              smokeCount++;
              var ok = false;
              try {
                ok = !!fn();
              } catch (e) {
                ok = false;
              }
              if (!ok) {
                smokeOk = false;
                if (smokeFailures.length) smokeFailures += ',';
                smokeFailures += name;
              }
            }
            checkSmoke('event', function(){ return new Event('x').type === 'x'; });
            checkSmoke('customEvent', function(){ return new CustomEvent('y', { detail: 7 }).detail === 7; });
            checkSmoke('mouseEvent', function(){ return new MouseEvent('click').type === 'click'; });
            checkSmoke('keyboardEvent', function(){ return new KeyboardEvent('keydown').type === 'keydown'; });
            checkSmoke('inputEvent', function(){ return new InputEvent('input').type === 'input'; });
            checkSmoke('focusEvent', function(){ return new FocusEvent('focus').type === 'focus'; });
            checkSmoke('wheelEvent', function(){ return new WheelEvent('wheel').type === 'wheel'; });
            checkSmoke('pointerEvent', function(){ return new PointerEvent('pointerdown').type === 'pointerdown'; });
            checkSmoke('mutationObserver', function(){ return typeof new MutationObserver(function(){}).observe === 'function'; });
            checkSmoke('resizeObserver', function(){ return typeof new ResizeObserver(function(){}).observe === 'function'; });
            checkSmoke('intersectionObserver', function(){ return typeof new IntersectionObserver(function(){}).observe === 'function'; });
            checkSmoke('messageChannel', function(){ return !!new MessageChannel().port1; });
            checkSmoke('url', function(){ return new URL('https://example.com/path').href === 'https://example.com/path'; });
            checkSmoke('urlSearchParams', function(){ return new URL('https://example.com/path?q=test#x').searchParams.get('q') === 'test'; });
            checkSmoke('searchParams', function(){ return new URLSearchParams('a=1').get('a') === '1'; });
            checkSmoke('searchParamsIterator', function(){ return Array.from(new URLSearchParams('a=1&b=2'))[1][0] === 'b'; });
            var smokeBlob = new Blob(['ab', 'c'], { type: 'TEXT/PLAIN' });
            checkSmoke('blobSize', function(){ return smokeBlob.size === 3; });
            checkSmoke('blobType', function(){ return smokeBlob.type === 'text/plain'; });
            checkSmoke('blobArrayBuffer', function(){ return typeof smokeBlob.arrayBuffer === 'function'; });
            checkSmoke('blobObjectUrl', function(){
              var url = URL.createObjectURL(smokeBlob);
              return typeof url === 'string' && url.indexOf('blob:anyos/') === 0;
            });
            checkSmoke('blobRevokeObjectUrl', function(){ return URL.revokeObjectURL(URL.createObjectURL(smokeBlob)) === undefined; });
            smokeBlob.text().then(function(text){ globalThis.__blob_text = text; });
            checkSmoke('textEncoder', function(){ return typeof new TextEncoder().encode === 'function'; });
            checkSmoke('textDecoder', function(){ return typeof new TextDecoder().decode === 'function'; });
            checkSmoke('abortController', function(){ return !!new AbortController().signal; });
            checkSmoke('abortSignal', function(){ return typeof AbortSignal.abort === 'function' && AbortSignal.abort().aborted === true; });
            checkSmoke('domParser', function(){ return typeof new DOMParser().parseFromString === 'function'; });
            checkSmoke('xhr', function(){ return typeof new XMLHttpRequest().open === 'function'; });
            checkSmoke('headers', function(){ return new Headers({ Foo: 'Bar' }).get('foo') === 'Bar'; });
            checkSmoke('image', function(){ return typeof new Image().addEventListener === 'function'; });
            checkSmoke('formData', function(){ return typeof new FormData().append === 'function'; });
            checkSmoke('nodeList', function(){ return typeof NodeList === 'function' && !!NodeList.prototype; });
            checkSmoke('htmlCollection', function(){ return typeof HTMLCollection === 'function' && !!HTMLCollection.prototype; });
            globalThis.__ctor_smoke_ok = smokeOk;
            globalThis.__ctor_smoke_count = smokeCount;
            globalThis.__ctor_smoke_failures = smokeFailures;
            globalThis.__ws_ctor_ok = new WebSocket('wss://example.com/socket').url === 'wss://example.com/socket';
        "#;

        runtime.execute_script_sources(&dom, "https://example.com/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__ctor_smoke_ok"),
                JsValue::Bool(true)
            ),
            "constructor smoke test failed: {}",
            runtime
                .engine
                .vm()
                .get_global("__ctor_smoke_failures")
                .to_js_string()
        );
        runtime.run_microtask_checkpoint(&dom);
        assert_eq!(
            runtime.engine.vm().get_global("__blob_text").to_js_string(),
            "abc"
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__ws_ctor_ok"),
                JsValue::Bool(true)
            ),
            "websocket constructor smoke test failed"
        );
        assert_eq!(runtime.pending_ws_connects.len(), 1);
    }

    #[test]
    fn browser_window_properties_are_global_names() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            window.google = { marker: 42 };
            globalThis.__window_to_global_ok = google.marker === 42;
            bareAssignmentFromScript = 'visible-on-window';
            globalThis.__global_to_window_ok = window.bareAssignmentFromScript === 'visible-on-window';
        "#;

        runtime.execute_script_sources(&dom, "https://www.google.de/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__window_to_global_ok"),
                JsValue::Bool(true)
            ),
            "window property was not readable as a global name"
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__global_to_window_ok"),
                JsValue::Bool(true)
            ),
            "global assignment was not mirrored to window"
        );
    }

    #[test]
    fn fetch_returns_internal_promise_when_page_wraps_promise_resolve() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            Promise.resolve = function(value) { return value; };
            var p = fetch('/gen_204');
            globalThis.__fetch_thenable_ok = typeof p.then === 'function';
            globalThis.__fetch_rejection_seen = false;
            p.then(
              function() { globalThis.__fetch_rejection_seen = 'unexpected'; },
              function() { globalThis.__fetch_rejection_seen = true; }
            );
        "#;

        runtime.execute_script_sources(&dom, "https://www.google.de/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__fetch_thenable_ok"),
                JsValue::Bool(true)
            ),
            "fetch did not return a thenable"
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__fetch_rejection_seen"),
                JsValue::Bool(true)
            ),
            "fetch rejection callback did not run"
        );
    }

    #[test]
    fn btoa_encodes_binary_string_code_units() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            globalThis.__btoa_ascii = btoa(String.fromCharCode(0x8a, 0xb6, 0xba));
            globalThis.__btoa_roundtrip =
                btoa(atob('ira6')) === 'ira6';
        "#;

        runtime.execute_script_sources(&dom, "https://example.com/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert_eq!(
            runtime
                .engine
                .vm()
                .get_global("__btoa_ascii")
                .to_js_string(),
            "ira6"
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__btoa_roundtrip"),
                JsValue::Bool(true)
            ),
            "atob/btoa did not preserve binary string bytes"
        );
    }

    #[test]
    fn browser_global_functions_are_window_properties() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            globalThis.__global_function_window_ok =
                window.parseFloat('12.5px') === 12.5 &&
                globalThis.parseInt('10', 10) === 10 &&
                window.isFinite(4) === true &&
                window.isNaN(NaN) === true &&
                typeof window.eval === 'function' &&
                window.eval('1 + 2') === 3 &&
                window.decodeURIComponent(encodeURIComponent('ä')) === 'ä';
        "#;

        runtime.execute_script_sources(&dom, "https://example.com/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert!(
            matches!(
                runtime
                    .engine
                    .vm()
                    .get_global("__global_function_window_ok"),
                JsValue::Bool(true)
            ),
            "browser global functions were not visible through window/globalThis"
        );
    }

    #[test]
    fn performance_timeline_records_marks_and_measures() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        let script = r#"
            performance.mark('start');
            performance.mark('end');
            const measure = performance.measure('total', 'start', 'end');
            const marks = performance.getEntriesByName('start', 'mark');
            const measures = performance.getEntriesByType('measure');
            performance.clearMarks('start');
            globalThis.__performance_timeline_ok =
                marks.length === 1 &&
                measures.length === 1 &&
                measures[0].name === 'total' &&
                measure.entryType === 'measure' &&
                performance.getEntriesByName('start', 'mark').length === 0;
        "#;

        runtime.execute_script_sources(&dom, "https://browserbench.org/", &[script.to_string()]);

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert!(
            matches!(
                runtime.engine.vm().get_global("__performance_timeline_ok"),
                JsValue::Bool(true)
            ),
            "performance timeline API did not retain mark/measure entries"
        );
    }

    #[test]
    fn class_name_property_assignment_updates_virtual_node_attribute() {
        let mut dom = html::parse("<html><body><div id=\"root\"></div></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const el = document.createElement('div');
                el.className = 'text-white bg-dark';
                el.textContent = 'CoreVM';
                document.getElementById('root').appendChild(el);
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        assert!(
            dom.nodes.iter().any(|node| {
                matches!(
                    &node.node_type,
                    crate::dom::NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "class" && a.value == "text-white bg-dark")
                )
            }),
            "className assignment should become a real class attribute"
        );
    }

    #[test]
    fn insert_before_null_appends_virtual_node_to_real_dom() {
        let mut dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const frame = document.createElement('iframe');
                frame.id = 'bench-frame';
                document.body.insertBefore(frame, null);
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        let body_id = dom.find_body().expect("body should exist");
        assert!(
            dom.nodes[body_id].children.iter().any(|&child_id| {
                matches!(
                    &dom.nodes[child_id].node_type,
                    crate::dom::NodeType::Element { tag: crate::dom::Tag::Iframe, attrs }
                        if attrs.iter().any(|a| a.name == "id" && a.value == "bench-frame")
                )
            }),
            "insertBefore(node, null) should append the virtual node into the real DOM"
        );
    }

    #[test]
    fn progress_max_property_assignment_updates_real_dom_attribute() {
        let mut dom = html::parse(
            "<html><body><progress id=\"progress-completed\"></progress></body></html>",
        );
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://browserbench.org/Speedometer3.1/",
            &[String::from(
                r#"
                const progress = document.getElementById('progress-completed');
                progress.max = 580;
                progress.value = 290;
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        let progress_id = dom
            .nodes
            .iter()
            .enumerate()
            .find_map(|(idx, node)| {
                matches!(
                    &node.node_type,
                    crate::dom::NodeType::Element {
                        tag: crate::dom::Tag::Progress,
                        ..
                    }
                )
                .then_some(idx)
            })
            .expect("progress element should exist");

        assert_eq!(dom.attr(progress_id, "max"), Some("580"));
        assert_eq!(dom.attr(progress_id, "value"), Some("290"));
    }

    #[test]
    fn moving_node_detaches_from_previous_js_parent() {
        let mut dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const a = document.createElement('section');
                const b = document.createElement('section');
                const frame = document.createElement('iframe');
                document.body.appendChild(a);
                document.body.appendChild(b);
                a.appendChild(frame);
                b.appendChild(frame);
                globalThis.__move_status = [
                    a.children.length,
                    a.firstChild === null,
                    frame.parentNode === b,
                    b.firstChild === frame,
                    b.children.length
                ].join('|');
                globalThis.__move_detached_old =
                    a.children.length === 0 &&
                    a.firstChild === null &&
                    frame.parentNode === b &&
                    b.firstChild === frame;
                frame.parentNode.removeChild(frame);
                globalThis.__move_removed_current =
                    b.children.length === 0 &&
                    b.firstChild === null &&
                    frame.parentNode === null;
                "#,
            )],
        );

        let window = runtime.engine.vm().get_global("window");
        assert!(
            matches!(
                window.get_property("__move_detached_old"),
                JsValue::Bool(true)
            ),
            "move status: {}",
            window.get_property("__move_status").to_js_string()
        );
        assert!(matches!(
            window.get_property("__move_removed_current"),
            JsValue::Bool(true)
        ));

        runtime.apply_mutations(&mut dom);
        let body_id = dom.find_body().expect("body should exist");
        let live_iframe_count = dom.nodes[body_id]
            .children
            .iter()
            .filter(|&&child_id| matches!(dom.tag(child_id), Some(crate::dom::Tag::Iframe)))
            .count();
        assert_eq!(
            live_iframe_count, 0,
            "moved iframe should not remain attached after current-parent removeChild"
        );
    }

    #[test]
    fn timer_mutations_keep_virtual_node_identity_after_initial_insert() {
        let mut dom = html::parse("<html><body><div id=\"root\"></div></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const el = document.createElement('div');
                el.className = 'before';
                document.getElementById('root').appendChild(el);
                setTimeout(() => {
                    el.setAttribute('className', 'after');
                    el.textContent = 'updated';
                }, 1);
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        assert_eq!(runtime.tick(&dom, 1), 1);
        runtime.apply_mutations(&mut dom);

        assert!(
            dom.nodes.iter().any(|node| {
                matches!(
                    &node.node_type,
                    crate::dom::NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "class" && a.value == "after")
                )
            }),
            "timer mutation should resolve the original virtual element to its real DOM node"
        );
        assert!(
            dom.nodes.iter().enumerate().any(|(idx, node)| {
                matches!(
                    &node.node_type,
                    crate::dom::NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "class" && a.value == "after")
                            && dom.text_content(idx) == "updated"
                )
            }),
            "timer textContent mutation should update the inserted element"
        );
    }

    #[test]
    fn set_immediate_runs_as_zero_delay_macrotask_with_args() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                globalThis.__immediate_value = 'pending';
                setImmediate((a, b) => {
                    globalThis.__immediate_value = a + ':' + b;
                }, 'core', 'vm');
                "#,
            )],
        );

        assert_eq!(runtime.tick(&dom, 0), 1);
        assert!(matches!(
            runtime.engine.vm().get_global("__immediate_value"),
            JsValue::String(ref s) if s == "core:vm"
        ));
    }

    #[test]
    fn prefers_reduced_motion_reports_no_preference() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                globalThis.__reduced_motion_bool = matchMedia('(prefers-reduced-motion)').matches;
                globalThis.__reduced_motion_reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
                globalThis.__reduced_motion_none = matchMedia('(prefers-reduced-motion: no-preference)').matches;
                "#,
            )],
        );

        assert!(matches!(
            runtime.engine.vm().get_global("__reduced_motion_bool"),
            JsValue::Bool(false)
        ));
        assert!(matches!(
            runtime.engine.vm().get_global("__reduced_motion_reduce"),
            JsValue::Bool(false)
        ));
        assert!(matches!(
            runtime.engine.vm().get_global("__reduced_motion_none"),
            JsValue::Bool(true)
        ));
    }

    #[test]
    fn request_animation_frame_can_drive_style_frames() {
        let mut dom = html::parse("<html><body><div id=\"root\"></div></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const el = document.createElement('div');
                el.style.opacity = '0';
                document.getElementById('root').appendChild(el);
                let frames = 0;
                function step() {
                    frames++;
                    el.style.opacity = frames >= 2 ? '1' : '0.5';
                    if (frames < 2) requestAnimationFrame(step);
                }
                requestAnimationFrame(step);
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        assert_eq!(runtime.tick(&dom, 16), 1);
        runtime.apply_mutations(&mut dom);
        assert!(runtime.timers.len() >= 1);
        assert_eq!(runtime.tick(&dom, 16), 1);
        runtime.apply_mutations(&mut dom);

        assert!(dom.nodes.iter().any(|node| {
            matches!(
                &node.node_type,
                crate::dom::NodeType::Element { attrs, .. }
                    if attrs.iter().any(|a| a.name == "style" && a.value.contains("opacity: 1"))
            )
        }));
    }

    #[test]
    fn object_assign_class_name_updates_virtual_node_attribute() {
        let mut dom = html::parse("<html><body><div id=\"root\"></div></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                const el = document.createElement('div');
                Object.assign(el, { className: 'relative mx-auto max-w-7xl px-6' });
                document.getElementById('root').appendChild(el);
                "#,
            )],
        );
        runtime.apply_mutations(&mut dom);

        assert!(
            dom.nodes.iter().any(|node| {
                matches!(
                    &node.node_type,
                    crate::dom::NodeType::Element { attrs, .. }
                        if attrs.iter().any(|a| a.name == "class" && a.value == "relative mx-auto max-w-7xl px-6")
                )
            }),
            "Object.assign should route className writes through the DOM property hook"
        );
    }

    #[test]
    fn element_click_dispatches_listener_and_respects_prevent_default() {
        let dom = html::parse(
            "<html><body><form id=\"f\"><button id=\"b\" name=\"go\" value=\"1\">Go</button></form></body></html>",
        );
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from(
                r#"
                let seen = 0;
                const b = document.getElementById('b');
                b.addEventListener('click', (event) => {
                    seen += 1;
                    event.preventDefault();
                });
                b.click();
                globalThis.__click_seen = seen;
                "#,
            )],
        );

        assert_eq!(
            runtime.engine.vm().get_global("__click_seen").to_number() as i32,
            1
        );
        assert!(
            !runtime
                .mutations
                .iter()
                .any(|m| matches!(m, super::DomMutation::FormSubmit { .. })),
            "preventDefault on a synthetic click must cancel the submit default action"
        );
    }

    #[test]
    fn element_click_queues_default_form_submit() {
        let dom = html::parse(
            "<html><body><form id=\"f\"><button id=\"b\" name=\"go\" value=\"1\">Go</button></form></body></html>",
        );
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://example.test/",
            &[String::from("document.getElementById('b').click();")],
        );

        assert!(
            runtime
                .mutations
                .iter()
                .any(|m| matches!(m, super::DomMutation::FormSubmit { form_node_id } if dom.attr(*form_node_id, "id") == Some("f"))),
            "programmatic button.click() should perform the browser submit default action"
        );
    }

    #[test]
    fn iframe_src_assignment_fires_load_and_keeps_frame_context() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://browserbench.org/Speedometer3.1/",
            &[String::from(
                r#"
                const frame = document.createElement('iframe');
                globalThis.__iframe_loaded = 0;
                frame.onload = () => {
                    globalThis.__iframe_loaded = 1;
                    globalThis.__iframe_context_ok =
                        frame.contentDocument !== document &&
                        frame.contentWindow !== window &&
                        frame.contentWindow.document === frame.contentDocument &&
                        typeof frame.contentDocument.querySelector === 'function' &&
                        typeof frame.contentWindow.requestAnimationFrame === 'function';
                    globalThis.__iframe_query_ok =
                        !!frame.contentDocument.querySelector('.new-todo');
                    const child = frame.contentDocument.createElement('section');
                    child.setAttribute('id', 'inside-frame');
                    child.setAttribute('class', 'todo active');
                    child.appendChild(frame.contentDocument.createTextNode('ready'));
                    frame.contentDocument.body.appendChild(child);
                    globalThis.__iframe_dom_ops_ok =
                        frame.contentDocument.body.firstChild === child &&
                        child.parentNode === frame.contentDocument.body &&
                        child.getAttribute('class') === 'todo active' &&
                        child.hasAttribute('id') &&
                        child.matches('.todo') &&
                        child.closest('section') === child &&
                        frame.contentWindow.getComputedStyle(child) === child.style &&
                        frame.contentDocument.createDocumentFragment().nodeType === 11;
                };
                document.body.appendChild(frame);
                frame.src = 'resources/warmup/index.html';
                globalThis.__iframe_src = frame.src;
                "#,
            )],
        );

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        assert_eq!(runtime.tick(&dom, 0), 1);
        let window = runtime.engine.vm().get_global("window");
        assert_eq!(window.get_property("__iframe_loaded").to_number() as i32, 1);
        assert!(matches!(
            window.get_property("__iframe_context_ok"),
            JsValue::Bool(true)
        ));
        assert!(matches!(
            window.get_property("__iframe_query_ok"),
            JsValue::Bool(true)
        ));
        assert!(matches!(
            window.get_property("__iframe_dom_ops_ok"),
            JsValue::Bool(true)
        ));
        assert_eq!(
            window.get_property("__iframe_src").to_js_string(),
            "resources/warmup/index.html"
        );
    }

    #[test]
    fn speedometer_perf_dashboard_iframe_chain_reaches_score_callback() {
        let dom = html::parse("<html><body></body></html>");
        let mut runtime = JsRuntime::new();
        runtime.execute_script_sources(
            &dom,
            "https://browserbench.org/Speedometer3.1/",
            &[String::from(
                r#"
                const params = {
                    measurementMethod: 'raf',
                    viewport: { width: 800, height: 600 },
                    waitBeforeSync: 0,
                    warmupBeforeSync: 0
                };

                class BenchmarkTestStep {
                    constructor(name, run) {
                        this.name = name;
                        this.run = run;
                    }
                }

                class Page {
                    constructor(frame) {
                        this._frame = frame;
                    }
                    async waitForElement(selector) {
                        return new Promise((resolve) => {
                            const resolveIfReady = () => {
                                const element = this.querySelector(selector);
                                window.requestAnimationFrame(element ? () => resolve(element) : resolveIfReady);
                            };
                            resolveIfReady();
                        });
                    }
                    querySelector(selector) {
                        const element = this._frame.contentDocument.querySelector(selector);
                        return element === null ? null : this._wrapElement(element);
                    }
                    call(functionName) {
                        this._frame.contentWindow[functionName]();
                        return null;
                    }
                    callAsync(functionName) {
                        setTimeout(() => {
                            this._frame.contentWindow[functionName]();
                        }, 0);
                    }
                    callToGetElement(functionName) {
                        return this._wrapElement(this._frame.contentWindow[functionName]());
                    }
                    _wrapElement(element) {
                        return new PageElement(element);
                    }
                }

                class PageElement {
                    #node;
                    constructor(node) {
                        this.#node = node;
                    }
                    dispatchKeyEvent(type, keyCode, key, options) {
                        let eventOptions = { bubbles: true, cancelable: true, keyCode, which: keyCode, key };
                        if (options !== undefined)
                            eventOptions = Object.assign(eventOptions, options);
                        const event = new KeyboardEvent(type, eventOptions);
                        this.#node.dispatchEvent(event);
                    }
                    dispatchMouseEvent(type, offsetX, offsetY, options) {
                        const boundingRect = this.#node.getBoundingClientRect();
                        const clientX = offsetX + boundingRect.x;
                        const clientY = offsetY + boundingRect.y;
                        const contentWindow = this.#node.ownerDocument.defaultView;
                        const screenX = clientX + contentWindow.screenX;
                        const screenY = clientY + contentWindow.screenY;
                        let eventOptions = { bubbles: true, cancelable: true, clientX, clientY, screenX, screenY };
                        if (options !== undefined)
                            eventOptions = Object.assign(eventOptions, options);
                        const event = new contentWindow.MouseEvent(type, eventOptions);
                        this.#node.dispatchEvent(event);
                    }
                }

                class TestInvoker {
                    constructor(syncCallback, asyncCallback, reportCallback) {
                        this._syncCallback = syncCallback;
                        this._asyncCallback = asyncCallback;
                        this._reportCallback = reportCallback;
                    }
                }

                class RAFTestInvoker extends TestInvoker {
                    start() {
                        return new Promise((resolve) => {
                            requestAnimationFrame(() => this._syncCallback());
                            requestAnimationFrame(() => {
                                setTimeout(() => {
                                    this._asyncCallback();
                                    setTimeout(async () => {
                                        await this._reportCallback();
                                        resolve();
                                    }, 0);
                                }, 0);
                            });
                        });
                    }
                }

                class BenchmarkRunner {
                    constructor(suites, client) {
                        this._suites = suites;
                        this._client = client;
                        this._frame = null;
                        this._page = null;
                        this._metrics = null;
                        this._measuredValues = null;
                    }
                    async runMultipleIterations() {
                        try {
                            await this._runAllSuites();
                        } catch (error) {
                            globalThis.__speedometer_error = error && (error.message || String(error));
                            return;
                        }
                        if (this._client?.didFinishLastIteration)
                            await this._client.didFinishLastIteration(this._metrics);
                    }
                    async _runAllSuites() {
                        this._measuredValues = { tests: {}, total: 0, mean: NaN, geomean: NaN, score: NaN };
                        this._removeFrame();
                        await this._appendFrame();
                        this._page = new Page(this._frame);
                        for (const suite of this._suites)
                            await this._runSuite(suite);
                        await this._finishRunAllSuites();
                    }
                    async _appendFrame() {
                        const frame = document.createElement('iframe');
                        const style = frame.style;
                        style.width = `${params.viewport.width}px`;
                        style.height = `${params.viewport.height}px`;
                        style.border = '0px none';
                        frame.className = 'test-runner';
                        document.body.insertBefore(frame, document.body.firstChild);
                        this._frame = frame;
                        return frame;
                    }
                    _removeFrame() {
                        if (this._frame) {
                            this._frame.parentNode.removeChild(this._frame);
                            this._frame = null;
                            globalThis.__speedometer_frame_removed = 1;
                        }
                    }
                    async _finishRunAllSuites() {
                        this._removeFrame();
                        await this._finalize();
                    }
                    async _runSuite(suite) {
                        await this._prepareSuite(suite);
                        for (const test of suite.tests)
                            await this._runTestAndRecordResults(suite, test);
                    }
                    async _prepareSuite(suite) {
                        return new Promise((resolve) => {
                            const frame = this._page._frame;
                            frame.onload = async () => {
                                await suite.prepare(this._page);
                                resolve();
                            };
                            frame.src = `resources/${suite.url}`;
                        });
                    }
                    async _runTestAndRecordResults(suite, test) {
                        if (this._client?.willRunTest)
                            await this._client.willRunTest(suite, test);
                        let syncTime = 0;
                        let asyncTime = 0;
                        let asyncStartTime = 0;
                        const runSync = () => {
                            const syncStartTime = performance.now();
                            test.run(this._page);
                            syncTime = performance.now() - syncStartTime;
                            asyncStartTime = performance.now();
                        };
                        const measureAsync = () => {
                            const height = this._frame.contentDocument.body.getBoundingClientRect().height;
                            asyncTime = performance.now() - asyncStartTime;
                            this._frame.contentWindow._unusedHeightValue = height;
                        };
                        const report = () => this._recordTestResults(suite, test, syncTime, asyncTime);
                        return new RAFTestInvoker(runSync, measureAsync, report).start();
                    }
                    async _recordTestResults(suite, test, syncTime, asyncTime) {
                        const suiteResults = this._measuredValues.tests[suite.name] || { tests: {}, total: 0 };
                        const total = syncTime + asyncTime;
                        this._measuredValues.tests[suite.name] = suiteResults;
                        suiteResults.tests[test.name] = { tests: { Sync: syncTime, Async: asyncTime }, total };
                        suiteResults.total += total;
                        if (this._client?.didRunTest)
                            await this._client.didRunTest(suite, test);
                    }
                    async _finalize() {
                        const values = [];
                        let product = 1;
                        for (const suiteName in this._measuredValues.tests) {
                            const suiteTotal = this._measuredValues.tests[suiteName].total;
                            product *= suiteTotal;
                            values.push(suiteTotal);
                        }
                        const total = values.reduce((a, b) => a + b);
                        const geomean = Math.pow(product, 1 / values.length);
                        this._measuredValues.total = total;
                        this._measuredValues.mean = total / values.length;
                        this._measuredValues.geomean = geomean;
                        this._measuredValues.score = 1000 / geomean;
                        if (this._client?.didRunSuites)
                            await this._client.didRunSuites(this._measuredValues);
                    }
                }

                const suite = {
                    name: 'Perf-Dashboard',
                    url: 'perf.webkit.org/public/v3/#/charts/',
                    async prepare(page) {
                        await page.waitForElement('#app-is-ready');
                        page.call('startTest');
                        page.callAsync('serviceRAF');
                        await new Promise((resolve) => setTimeout(resolve, 1));
                    },
                    tests: [
                        new BenchmarkTestStep('Render', (page) => {
                            page.call('openCharts');
                            page.call('serviceRAF');
                        }),
                        new BenchmarkTestStep('SelectingPoints', (page) => {
                            const chartPane = page.callToGetElement('getChartPane');
                            for (let i = 0; i < 20; ++i) {
                                chartPane.dispatchKeyEvent('keydown', 39, 'ArrowRight');
                                page.call('serviceRAF');
                            }
                        }),
                        new BenchmarkTestStep('SelectingRange', (page) => {
                            const canvas = page.callToGetElement('getChartCanvas');
                            const startingX = 200;
                            const startingY = 200;
                            const endingX = 600;
                            const endingY = 400;
                            canvas.dispatchMouseEvent('mousedown', startingX, startingY);
                            page.call('serviceRAF');
                            for (let i = 1; i <= 4; ++i) {
                                canvas.dispatchMouseEvent('mousemove', startingX + ((endingX - startingX) * i) / 4, startingY + ((endingY - startingY) * i) / 4);
                                page.call('serviceRAF');
                            }
                            canvas.dispatchMouseEvent('mouseup', endingX, endingY);
                            page.call('serviceRAF');
                        }),
                    ],
                };

                const client = {
                    async willRunTest() {
                        globalThis.__speedometer_will = (globalThis.__speedometer_will || 0) + 1;
                    },
                    async didRunTest() {
                        globalThis.__speedometer_did = (globalThis.__speedometer_did || 0) + 1;
                    },
                    async didRunSuites(values) {
                        globalThis.__speedometer_score = values.score;
                        globalThis.__speedometer_done = 1;
                    },
                    async didFinishLastIteration() {
                        globalThis.__speedometer_finished = 1;
                    }
                };

                globalThis.__speedometer_error = '';
                globalThis.__speedometer_done = 0;
                globalThis.__speedometer_finished = 0;
                globalThis.__speedometer_frame_removed = 0;
                new BenchmarkRunner([suite], client).runMultipleIterations().then(() => {
                    globalThis.__speedometer_resolved = 1;
                }, (error) => {
                    globalThis.__speedometer_error = error && (error.message || String(error));
                });
                "#,
            )],
        );

        assert!(
            runtime.engine.vm().last_exception.is_none(),
            "unexpected JS exception: {:?}",
            runtime.engine.vm().last_exception
        );
        for _ in 0..80 {
            if !runtime.has_pending_js_work() {
                break;
            }
            runtime.tick(&dom, 50);
        }
        assert!(
            !runtime.has_pending_js_work(),
            "speedometer mini-run left JS work pending"
        );

        let window = runtime.engine.vm().get_global("window");
        assert_eq!(
            window.get_property("__speedometer_error").to_js_string(),
            "",
            "speedometer mini-run threw an error"
        );
        assert_eq!(
            window.get_property("__speedometer_will").to_number() as i32,
            3
        );
        assert_eq!(
            window.get_property("__speedometer_did").to_number() as i32,
            3
        );
        assert!(matches!(
            window.get_property("__speedometer_frame_removed"),
            JsValue::Number(1.0)
        ));
        assert!(matches!(
            window.get_property("__speedometer_done"),
            JsValue::Number(1.0)
        ));
        assert!(matches!(
            window.get_property("__speedometer_finished"),
            JsValue::Number(1.0)
        ));
        assert!(matches!(
            window.get_property("__speedometer_resolved"),
            JsValue::Number(1.0)
        ));
        assert!(
            window
                .get_property("__speedometer_score")
                .to_number()
                .is_finite(),
            "score callback should receive a finite score"
        );
    }
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
                            let name = pair
                                .elements
                                .get(&0)
                                .map(|v| v.to_js_string())
                                .unwrap_or_default();
                            let value = pair
                                .elements
                                .get(&1)
                                .map(|v| v.to_js_string())
                                .unwrap_or_default();
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
        obj_rc
            .borrow_mut()
            .set(String::from("__entries"), JsValue::new_array(pairs));
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
    let keys: Vec<JsValue> = entries
        .iter()
        .map(|(n, _)| JsValue::String(n.clone()))
        .collect();
    JsValue::new_array(keys)
}

fn formdata_values(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let entries = formdata_read_entries(vm);
    let vals: Vec<JsValue> = entries
        .iter()
        .map(|(_, v)| JsValue::String(v.clone()))
        .collect();
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

fn reinstall_schedule_js_work(vm: &mut Vm) {
    let schedule = native_fn("ScheduleJSWork", native_schedule_js_work);
    vm.set_global("ScheduleJSWork", schedule.clone());
    let win = vm.get_global("window");
    if !win.is_undefined() {
        win.set_property(String::from("ScheduleJSWork"), schedule);
    }
}

fn native_schedule_js_work(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let cb = args.first().cloned().unwrap_or(JsValue::Undefined);
    if matches!(cb, JsValue::Function(_)) {
        // Meta's loader treats ScheduleJSWork as a wrapper factory:
        //   ScheduleJSWork(callback)(...args)
        // Returning the callback itself keeps the work synchronous in our
        // current single-turn VM, but preserves the observable contract.
        return cb;
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
        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            eprintln!("[js-dom-debug] setTimeout id={} delay={}", id, delay);
        }
        push_pending_timer(
            &mut bridge.timers,
            PendingTimer {
                id,
                callback,
                this_arg: JsValue::Undefined,
                args: args.iter().skip(2).cloned().collect(),
                delay_ms: delay,
                repeat: false,
                elapsed_ms: 0,
                is_raf: false,
            },
        );
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
        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            eprintln!("[js-dom-debug] setInterval id={} delay={}", id, delay);
        }
        push_pending_timer(
            &mut bridge.timers,
            PendingTimer {
                id,
                callback,
                this_arg: JsValue::Undefined,
                args: args.iter().skip(2).cloned().collect(),
                delay_ms: delay,
                repeat: true,
                elapsed_ms: 0,
                is_raf: false,
            },
        );
        return JsValue::Number(id as f64);
    }
    JsValue::Number(0.0)
}

fn native_set_immediate(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if let Some(bridge) = get_bridge(vm) {
        let id = bridge.next_timer_id;
        bridge.next_timer_id += 1;
        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            eprintln!("[js-dom-debug] setImmediate id={}", id);
        }
        push_pending_timer(
            &mut bridge.timers,
            PendingTimer {
                id,
                callback,
                this_arg: JsValue::Undefined,
                args: args.iter().skip(1).cloned().collect(),
                delay_ms: 0,
                repeat: false,
                elapsed_ms: 0,
                is_raf: false,
            },
        );
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
        #[cfg(feature = "host")]
        if std::env::var_os("SURF_DEBUG_TIMERS").is_some() {
            eprintln!("[js-dom-debug] requestAnimationFrame id={}", id);
        }
        push_pending_timer(
            &mut bridge.timers,
            PendingTimer {
                id,
                callback,
                this_arg: JsValue::Undefined,
                args: Vec::new(),
                delay_ms: 16,
                repeat: false,
                elapsed_ms: 0,
                is_raf: true,
            },
        );
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
        (Some(CssValue::Keyword(a)), CssValue::Keyword(b))
            if matches!(to.property, Property::Transform) =>
        {
            interpolate_transform_value(a, b, t).map(CssValue::Keyword)?
        }
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

#[derive(Clone, Copy)]
struct TransformParts {
    tx: i32,
    ty: i32,
    tx_pct: i32,
    ty_pct: i32,
    sx: i32,
    sy: i32,
    rotate: i32,
}

impl TransformParts {
    fn identity() -> Self {
        Self {
            tx: 0,
            ty: 0,
            tx_pct: 0,
            ty_pct: 0,
            sx: 1000,
            sy: 1000,
            rotate: 0,
        }
    }
}

fn interpolate_transform_value(from: &str, to: &str, t: i32) -> Option<String> {
    let a = parse_transform_parts(from)?;
    let b = parse_transform_parts(to)?;
    let tx = lerp_i32(a.tx, b.tx, t);
    let ty = lerp_i32(a.ty, b.ty, t);
    let tx_pct = lerp_i32(a.tx_pct, b.tx_pct, t);
    let ty_pct = lerp_i32(a.ty_pct, b.ty_pct, t);
    let sx = lerp_i32(a.sx, b.sx, t).max(1);
    let sy = lerp_i32(a.sy, b.sy, t).max(1);
    let rotate = lerp_i32(a.rotate, b.rotate, t);
    Some(format!(
        "translate({}px, {}px) translate({}%, {}%) scale({}, {}) rotate({}deg)",
        tx / 100,
        ty / 100,
        tx_pct / 100,
        ty_pct / 100,
        sx as f32 / 1000.0,
        sy as f32 / 1000.0,
        rotate as f32 / 100.0
    ))
}

fn parse_transform_parts(s: &str) -> Option<TransformParts> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Some(TransformParts::identity());
    }

    let mut out = TransformParts::identity();
    let mut pos = 0usize;
    let bytes = s.as_bytes();
    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        let name_start = pos;
        while pos < bytes.len() && bytes[pos] != b'(' && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let name = core::str::from_utf8(&bytes[name_start..pos]).ok()?;
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() || bytes[pos] != b'(' {
            return None;
        }
        pos += 1;
        let args_start = pos;
        let mut depth = 1u32;
        while pos < bytes.len() && depth > 0 {
            match bytes[pos] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            pos += 1;
        }
        if depth != 0 {
            return None;
        }
        let args = core::str::from_utf8(&bytes[args_start..pos.saturating_sub(1)]).ok()?;
        let parts: Vec<&str> = if args.contains(',') {
            args.split(',').map(str::trim).collect()
        } else {
            args.split_whitespace().collect()
        };
        match name.to_ascii_lowercase().as_str() {
            "translate" | "translate3d" => {
                if let Some(x) = parts.first() {
                    let (px, pct) = parse_transform_length_component(x)?;
                    out.tx += px;
                    out.tx_pct += pct;
                }
                if let Some(y) = parts.get(1) {
                    let (px, pct) = parse_transform_length_component(y)?;
                    out.ty += px;
                    out.ty_pct += pct;
                }
            }
            "translatex" => {
                let (px, pct) =
                    parse_transform_length_component(parts.first().copied().unwrap_or("0"))?;
                out.tx += px;
                out.tx_pct += pct;
            }
            "translatey" => {
                let (px, pct) =
                    parse_transform_length_component(parts.first().copied().unwrap_or("0"))?;
                out.ty += px;
                out.ty_pct += pct;
            }
            "scale" => {
                let sx = parse_scale_component(parts.first().copied().unwrap_or("1"))?;
                let sy = if let Some(v) = parts.get(1) {
                    parse_scale_component(v)?
                } else {
                    sx
                };
                out.sx = out.sx * sx / 1000;
                out.sy = out.sy * sy / 1000;
            }
            "scalex" => {
                out.sx =
                    out.sx * parse_scale_component(parts.first().copied().unwrap_or("1"))? / 1000
            }
            "scaley" => {
                out.sy =
                    out.sy * parse_scale_component(parts.first().copied().unwrap_or("1"))? / 1000
            }
            "rotate" | "rotatez" => {
                out.rotate += parse_angle_component(parts.first().copied().unwrap_or("0"))?;
            }
            _ => return None,
        }
    }
    Some(out)
}

fn parse_transform_length_component(s: &str) -> Option<(i32, i32)> {
    let s = s.trim();
    if s == "0" || s == "+0" || s == "-0" {
        return Some((0, 0));
    }
    if let Some(v) = s.strip_suffix('%') {
        return Some((0, (v.trim().parse::<f32>().ok()? * 100.0) as i32));
    }
    let number = s
        .strip_suffix("px")
        .or_else(|| s.strip_suffix("rem"))
        .or_else(|| s.strip_suffix("em"))
        .unwrap_or(s)
        .trim()
        .parse::<f32>()
        .ok()?;
    Some(((number * 100.0) as i32, 0))
}

fn parse_scale_component(s: &str) -> Option<i32> {
    Some((s.trim().parse::<f32>().ok()? * 1000.0) as i32)
}

fn parse_angle_component(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("deg") {
        Some((v.trim().parse::<f32>().ok()? * 100.0) as i32)
    } else if let Some(v) = s.strip_suffix("turn") {
        Some((v.trim().parse::<f32>().ok()? * 36000.0) as i32)
    } else if let Some(v) = s.strip_suffix("rad") {
        Some((v.trim().parse::<f32>().ok()? * 18000.0 / core::f32::consts::PI) as i32)
    } else {
        Some((s.parse::<f32>().ok()? * 100.0) as i32)
    }
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
    Property::Transform,
];

/// Extract a `Declaration` from a `ComputedStyle` for a given `Property`.
///
/// Returns `None` for properties we don't track / can't interpolate.
fn computed_style_to_decl(
    s: &crate::style::ComputedStyle,
    prop: &Property,
) -> Option<crate::css::Declaration> {
    let value = match prop {
        Property::Opacity => CssValue::Number((s.opacity * 100 + 127) / 255),
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
        Property::Transform => CssValue::Keyword(format!(
            "translate({}px, {}px) translate({}%, {}%) scale({}, {}) rotate({}deg)",
            s.transform_tx,
            s.transform_ty,
            s.transform_tx_pct / 100,
            s.transform_ty_pct / 100,
            s.transform_sx as f32 / 1000.0,
            s.transform_sy as f32 / 1000.0,
            s.transform_rotate as f32 / 100.0
        )),
        _ => return None,
    };
    Some(crate::css::Declaration {
        property: prop.clone(),
        value,
        important: false,
    })
}

fn push_console_message(console: &mut Vec<String>, msg: String) {
    if console.len() >= MAX_CONSOLE_MESSAGES {
        let overflow = console.len() + 1 - MAX_CONSOLE_MESSAGES;
        console.drain(0..overflow);
    }
    console.push(msg);
}

pub(super) fn push_pending_timer(queue: &mut Vec<PendingTimer>, timer: PendingTimer) {
    if queue.len() >= MAX_PENDING_TIMERS {
        let overflow = queue.len() + 1 - MAX_PENDING_TIMERS;
        queue.drain(0..overflow);
    }
    queue.push(timer);
}

fn extend_pending_timers(queue: &mut Vec<PendingTimer>, timers: Vec<PendingTimer>) {
    if timers.is_empty() {
        return;
    }
    let incoming_len = timers.len();
    if incoming_len >= MAX_PENDING_TIMERS {
        queue.clear();
        queue.extend(timers.into_iter().skip(incoming_len - MAX_PENDING_TIMERS));
        return;
    }
    if queue.len() + incoming_len > MAX_PENDING_TIMERS {
        let overflow = queue.len() + incoming_len - MAX_PENDING_TIMERS;
        queue.drain(0..overflow);
    }
    queue.extend(timers);
}
