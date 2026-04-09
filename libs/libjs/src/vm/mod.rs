//! JavaScript virtual machine — executes bytecode.
//!
//! Stack-based VM with prototype chain support, closures,
//! reference-semantics (Rc<RefCell>) and ECMAScript-compatible semantics.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use core::cell::RefCell;

use crate::bytecode::{Chunk, Constant, Op};
use crate::value::*;

pub mod builtins;
pub mod call;
pub mod event_loop;
pub mod iter;
pub mod native_array;
pub mod native_console;
pub mod native_date;
pub mod native_error;
pub mod native_es2024;
pub mod native_function;
pub mod native_generator;
pub mod native_globals;
pub mod native_json;
pub mod native_map;
pub mod native_math;
pub mod native_number;
pub mod native_object;
pub mod native_promise;
pub mod native_proxy;
pub mod native_regexp;
pub mod native_string;
pub mod native_symbol;
pub mod native_timer;
pub mod native_typed_array;
pub mod native_weakref;

// ── Internal structures ──

/// A local variable slot — either a plain value (fast path, no heap allocation)
/// or a shared cell (required when the local is captured by an inner closure).
#[derive(Clone)]
pub enum LocalSlot {
    /// Non-captured local: stored inline, no Rc/RefCell overhead.
    Direct(JsValue),
    /// Captured local: shared with closures via Rc<RefCell>.
    Cell(Rc<RefCell<JsValue>>),
}

impl LocalSlot {
    /// Read the value.
    #[inline(always)]
    pub fn get(&self) -> JsValue {
        match self {
            LocalSlot::Direct(v) => v.clone(),
            LocalSlot::Cell(c) => c.borrow().clone(),
        }
    }

    /// Write a value.
    #[inline(always)]
    pub fn set(&mut self, val: JsValue) {
        match self {
            LocalSlot::Direct(v) => *v = val,
            LocalSlot::Cell(c) => *c.borrow_mut() = val,
        }
    }

    /// Get the shared cell (panics if Direct — caller must know this is captured).
    #[inline(always)]
    pub fn as_cell(&self) -> &Rc<RefCell<JsValue>> {
        match self {
            LocalSlot::Cell(c) => c,
            LocalSlot::Direct(_) => panic!("LocalSlot::as_cell on non-captured local"),
        }
    }
}

/// Call frame for function invocations.
pub struct CallFrame {
    pub chunk: Chunk,
    pub ip: usize,
    pub stack_base: usize,
    /// Local variable slots — a mix of Direct (non-captured) and Cell (captured).
    /// Non-captured locals avoid Rc<RefCell> overhead entirely.
    pub locals: Vec<LocalSlot>,
    /// Upvalue cells captured by this function's closure.
    pub upvalue_cells: Vec<Rc<RefCell<JsValue>>>,
    pub this_val: JsValue,
    /// True when this frame was entered via `new Constructor()`.
    /// On Return, if the constructor returned a non-object, `this_val` is used instead.
    pub is_constructor: bool,
    /// All arguments passed to this call (used by rest params, `arguments` object, etc.).
    pub all_args: Vec<JsValue>,
    /// The function value currently executing (used by `LoadSelf` for named function exprs).
    pub self_ref: JsValue,
}

/// Exception handler for try-catch.
pub struct TryHandler {
    pub catch_ip: usize,
    pub stack_depth: usize,
    pub frame_depth: usize,
}

// ── The VM ──

/// The JavaScript virtual machine.
pub struct Vm {
    pub stack: Vec<JsValue>,
    pub frames: Vec<CallFrame>,
    pub globals: Rc<RefCell<JsObject>>,
    pub try_handlers: Vec<TryHandler>,
    pub console_output: Vec<String>,
    pub engine_log: Vec<String>,
    pub object_proto: Rc<RefCell<JsObject>>,
    pub array_proto: Rc<RefCell<JsObject>>,
    pub string_proto: Rc<RefCell<JsObject>>,
    pub function_proto: Rc<RefCell<JsObject>>,
    pub number_proto: Rc<RefCell<JsObject>>,
    pub boolean_proto: Rc<RefCell<JsObject>>,
    pub error_proto: Rc<RefCell<JsObject>>,
    pub regexp_proto: Rc<RefCell<JsObject>>,
    pub generator_proto: Rc<RefCell<JsObject>>,
    pub typed_array_proto: Rc<RefCell<JsObject>>,
    pub step_limit: u64,
    pub steps: u64,
    pub userdata: *mut u8,
    /// Current `this` binding for the active native call.
    pub current_this: JsValue,
    /// Target frame depth for re-entrant run() calls (0 = run to completion).
    pub run_target_depth: usize,
    /// Pending exception set by native functions via `throw_native()`.
    /// Checked after every native call and turned into a VM-level throw.
    pub pending_exception: Option<JsValue>,
    /// Last unhandled exception (no try/catch caught it).
    /// Set by `handle_exception` when there is no handler.
    pub last_exception: Option<JsValue>,
    /// Set when a native call threw and control was already redirected into a
    /// JS catch handler. Internal helpers must stop normal execution in that case.
    pub control_flow_restored: bool,
    /// Pending generator yield: (value, ip, locals, stack_snapshot).
    /// Set by `Op::Yield` handler, consumed by `run_generator_step`.
    pub pending_generator_yield: Option<(JsValue, usize, Vec<LocalSlot>, Vec<JsValue>)>,
    /// Event loop for microtask queue and timers.
    pub event_loop: event_loop::EventLoop,
    /// Tracks Rust-level call_value() re-entrancy depth (independent of JS frames).
    pub call_value_depth: usize,
    /// Module registry: maps module specifier → cached namespace object.
    /// Populated by `register_module_source()` / `register_module_object()`.
    /// Used by the `__import__()` built-in.
    pub module_registry: BTreeMap<String, JsValue>,
    /// Module source registry: maps module specifier → JS source text.
    /// When `__import__` is called for a specifier not yet in `module_registry`,
    /// it compiles and executes the source, caches the result, and returns it.
    pub module_sources: BTreeMap<String, String>,
    /// `with` statement scope chain — stack of objects to check before globals.
    pub with_scopes: Vec<Rc<RefCell<JsObject>>>,
    /// Nesting depth of active native constructor invocations entered via `new`.
    /// Needed because native built-ins like `String` are callable both as
    /// functions and constructors, so `current_this` alone is not enough.
    pub native_constructor_depth: usize,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals: Rc::new(RefCell::new(JsObject::new())),
            try_handlers: Vec::new(),
            console_output: Vec::new(),
            engine_log: Vec::new(),
            object_proto: Rc::new(RefCell::new(JsObject::new())),
            array_proto: Rc::new(RefCell::new(JsObject::new())),
            string_proto: Rc::new(RefCell::new(JsObject::new())),
            function_proto: Rc::new(RefCell::new(JsObject::new())),
            number_proto: Rc::new(RefCell::new(JsObject::new())),
            boolean_proto: Rc::new(RefCell::new(JsObject::new())),
            error_proto: Rc::new(RefCell::new(JsObject::new())),
            regexp_proto: Rc::new(RefCell::new(JsObject::new())),
            generator_proto: Rc::new(RefCell::new(JsObject::new())),
            typed_array_proto: Rc::new(RefCell::new(JsObject::new())),
            step_limit: 10_000_000,
            steps: 0,
            userdata: core::ptr::null_mut(),
            current_this: JsValue::Undefined,
            run_target_depth: 0,
            pending_exception: None,
            last_exception: None,
            control_flow_restored: false,
            pending_generator_yield: None,
            event_loop: event_loop::EventLoop::new(),
            call_value_depth: 0,
            module_registry: BTreeMap::new(),
            module_sources: BTreeMap::new(),
            with_scopes: Vec::new(),
            native_constructor_depth: 0,
        };
        vm.init_prototypes();
        vm.init_globals();
        vm.log_engine("[libjs] VM initialized");
        vm
    }

    pub fn set_step_limit(&mut self, limit: u64) {
        self.step_limit = limit;
    }

    pub fn is_in_constructor_call(&self) -> bool {
        self.native_constructor_depth > 0 || self.frames.last().is_some_and(|f| f.is_constructor)
    }

    fn mangle_private_name(name: &str) -> String {
        alloc::format!("__private_slot_{}", name)
    }

    fn close_iterator_on_abrupt(&mut self, iter: &JsValue, original_exc: JsValue) -> JsValue {
        let return_fn = self.get_property_invoking_getter(iter, "return");
        if let Some(close_exc) = self.last_exception.take() {
            return close_exc;
        }
        if let Some(close_exc) = self.pending_exception.take() {
            return close_exc;
        }
        if matches!(return_fn, JsValue::Undefined | JsValue::Null) {
            return original_exc;
        }
        if !return_fn.is_function() {
            return self.make_type_error("IteratorClose return method is not callable");
        }

        let result = self.call_value(&return_fn, &[], iter.clone());
        let close_threw = self
            .last_exception
            .take()
            .or_else(|| self.pending_exception.take());

        // Per IteratorClose, an original throw completion wins over errors from
        // the return() call itself once return() has been invoked.
        if !matches!(original_exc, JsValue::Undefined) {
            return original_exc;
        }

        if let Some(close_exc) = close_threw {
            return close_exc;
        }
        if !matches!(result, JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)) {
            return self.make_type_error("IteratorClose return() must return an object");
        }
        original_exc
    }

    fn is_active_iterator_candidate(&self, val: &JsValue) -> bool {
        match val {
            JsValue::Object(obj) => {
                if obj.borrow().internal_tag.as_deref() == Some("__iterator__") {
                    return true;
                }
                self.get_property_with_proto(val, "next").is_function()
            }
            JsValue::Array(_) | JsValue::Function(_) => {
                self.get_property_with_proto(val, "next").is_function()
            }
            _ => false,
        }
    }

    fn close_iterators_in_stack_range(&mut self, start: usize) {
        if start >= self.stack.len() {
            return;
        }
        let values: Vec<JsValue> = self.stack[start..].to_vec();
        for val in values.iter().rev() {
            if !self.is_active_iterator_candidate(val) {
                continue;
            }
            let _ = self.close_iterator_on_abrupt(val, JsValue::Undefined);
            let _ = self.pending_exception.take();
            let _ = self.last_exception.take();
        }
    }

    /// Create a local-slot vector for a chunk.  Non-captured locals get
    /// `LocalSlot::Direct` (zero heap alloc), captured locals get
    /// `LocalSlot::Cell` (shared Rc<RefCell> for closure capture).
    pub fn make_locals(chunk: &Chunk) -> Vec<LocalSlot> {
        let n = chunk.local_count as usize;
        let captured = &chunk.captured_locals;
        (0..n)
            .map(|i| {
                if i < captured.len() && captured[i] {
                    LocalSlot::Cell(Rc::new(RefCell::new(JsValue::Undefined)))
                } else {
                    LocalSlot::Direct(JsValue::Undefined)
                }
            })
            .collect()
    }

    /// Signal an exception from a native Rust function.
    ///
    /// The exception is stored in `pending_exception` and processed by
    /// `invoke_function`/`new_object` after the native call returns.
    pub fn throw_native(&mut self, val: JsValue) {
        self.pending_exception = Some(val);
    }

    /// Create a `TypeError` object (for use in native function throws).
    pub fn make_syntax_error(&self, message: &str) -> JsValue {
        let stack_str = self.make_stack_trace("SyntaxError", message);
        let mut obj = JsObject::new();
        obj.prototype = Some(self.error_proto.clone());
        obj.set(
            String::from("name"),
            JsValue::String(String::from("SyntaxError")),
        );
        obj.set(
            String::from("message"),
            JsValue::String(String::from(message)),
        );
        obj.set(String::from("stack"), JsValue::String(stack_str));
        let ctor = self.globals.borrow().get("SyntaxError");
        if !matches!(ctor, JsValue::Undefined) {
            obj.set(String::from("constructor"), ctor);
        }
        JsValue::Object(Rc::new(RefCell::new(obj)))
    }

    pub fn make_type_error(&self, message: &str) -> JsValue {
        let stack_str = self.make_stack_trace("TypeError", message);
        let mut obj = JsObject::new();
        obj.prototype = Some(self.error_proto.clone());
        obj.set(
            String::from("name"),
            JsValue::String(String::from("TypeError")),
        );
        obj.set(
            String::from("message"),
            JsValue::String(String::from(message)),
        );
        obj.set(String::from("stack"), JsValue::String(stack_str));
        let ctor = self.globals.borrow().get("TypeError");
        if !matches!(ctor, JsValue::Undefined) {
            obj.set(String::from("constructor"), ctor);
        }
        JsValue::Object(Rc::new(RefCell::new(obj)))
    }

    /// Create a generic `Error` object.
    pub fn make_error(&self, message: &str) -> JsValue {
        let stack_str = self.make_stack_trace("Error", message);
        let mut obj = JsObject::new();
        obj.prototype = Some(self.error_proto.clone());
        obj.set(
            String::from("name"),
            JsValue::String(String::from("Error")),
        );
        obj.set(
            String::from("message"),
            JsValue::String(String::from(message)),
        );
        obj.set(String::from("stack"), JsValue::String(stack_str));
        JsValue::Object(Rc::new(RefCell::new(obj)))
    }

    /// Create a `RangeError` object.
    pub fn make_range_error(&self, message: &str) -> JsValue {
        let stack_str = self.make_stack_trace("RangeError", message);
        let mut obj = JsObject::new();
        obj.prototype = Some(self.error_proto.clone());
        obj.set(
            String::from("name"),
            JsValue::String(String::from("RangeError")),
        );
        obj.set(
            String::from("message"),
            JsValue::String(String::from(message)),
        );
        obj.set(String::from("stack"), JsValue::String(stack_str));
        let ctor = self.globals.borrow().get("RangeError");
        if !matches!(ctor, JsValue::Undefined) {
            obj.set(String::from("constructor"), ctor);
        }
        JsValue::Object(Rc::new(RefCell::new(obj)))
    }

    /// Create a `ReferenceError` object.
    pub fn make_reference_error(&self, message: &str) -> JsValue {
        let stack_str = self.make_stack_trace("ReferenceError", message);
        let mut obj = JsObject::new();
        obj.prototype = Some(self.error_proto.clone());
        obj.set(
            String::from("name"),
            JsValue::String(String::from("ReferenceError")),
        );
        obj.set(
            String::from("message"),
            JsValue::String(String::from(message)),
        );
        obj.set(String::from("stack"), JsValue::String(stack_str));
        let ctor = self.globals.borrow().get("ReferenceError");
        if !matches!(ctor, JsValue::Undefined) {
            obj.set(String::from("constructor"), ctor);
        }
        JsValue::Object(Rc::new(RefCell::new(obj)))
    }

    /// Build a V8-style stack trace string from the current call frames.
    fn make_stack_trace(&self, error_name: &str, message: &str) -> String {
        let mut s = format!("{}: {}", error_name, message);
        for frame in self.frames.iter().rev().take(10) {
            let fname = frame.chunk.name.as_deref().unwrap_or("<anonymous>");
            s.push_str("\n    at ");
            s.push_str(fname);
            // Append line number if available from the line_map.
            let ip = if frame.ip > 0 { frame.ip - 1 } else { 0 };
            if let Some(&line) = frame.chunk.line_map.get(ip) {
                if line > 0 {
                    s.push_str(&format!(" (line {})", line));
                }
            }
        }
        s
    }

    fn frame_stack_summary(&self, max_frames: usize) -> String {
        let mut stack_info = String::new();
        for (fi, frame) in self.frames.iter().rev().take(max_frames).enumerate() {
            let fname = frame.chunk.name.as_deref().unwrap_or("(anon)");
            if fi > 0 {
                stack_info.push_str(" <- ");
            }
            stack_info.push_str(fname);
        }
        stack_info
    }

    fn describe_exception(&self, val: &JsValue) -> String {
        match val {
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
                    let first_stack_line = stack.lines().nth(1).unwrap_or("");
                    if !first_stack_line.is_empty() {
                        return format!("{}: {} [{}]", name, msg, first_stack_line.trim());
                    }
                }
                format!("{}: {}", name, msg)
            }
            JsValue::String(s) => format!("String throw: {}", s),
            JsValue::Bool(_) | JsValue::Number(_) | JsValue::Null => format!("{:?}", val),
            _ => val.to_js_string(),
        }
    }

    pub fn execute(&mut self, chunk: Chunk) -> JsValue {
        self.steps = 0;
        self.last_exception = None;
        let locals = Self::make_locals(&chunk);
        let frame = CallFrame {
            chunk,
            ip: 0,
            stack_base: self.stack.len(),
            locals,
            upvalue_cells: Vec::new(),
            this_val: JsValue::Object(self.globals.clone()),
            is_constructor: false,
            all_args: Vec::new(),
            self_ref: JsValue::Undefined,
        };
        self.frames.push(frame);
        let result = self.run();
        // Drain microtask queue after script execution
        self.drain_microtasks();
        result
    }

    /// Drain all pending microtasks (Promise callbacks, queueMicrotask).
    pub fn drain_microtasks(&mut self) {
        let mut safety = 0u32;
        while let Some(task) = self.event_loop.pop_microtask() {
            // Use call_value (not invoke_function) so bytecode callbacks are
            // executed re-entrantly — invoke_function only pushes a frame
            // without calling run(), which does nothing after the main loop exits.
            self.call_value(&task.callback, &task.args, JsValue::Undefined);
            safety += 1;
            if safety > 10000 {
                break;
            } // prevent infinite microtask loop
        }
    }

    /// Enqueue a microtask (called by Promise .then resolution).
    pub fn enqueue_microtask(&mut self, callback: JsValue, args: Vec<JsValue>) {
        self.event_loop.enqueue_microtask(callback, args);
    }

    /// Advance the event loop by `dt_ms` milliseconds.
    ///
    /// Fires any due timers (setTimeout/setInterval) and drains the
    /// microtask queue after each one — implementing the ECMAScript
    /// event loop semantics: one macrotask → drain all microtasks → next
    /// macrotask.
    ///
    /// Returns the number of timer callbacks that fired.
    pub fn tick(&mut self, dt_ms: u32) -> usize {
        let fired = self.event_loop.tick(dt_ms);
        let count = fired.len();
        for task in fired {
            self.call_value(&task.callback, &task.args, JsValue::Undefined);
            self.drain_microtasks();
        }
        count
    }

    /// Run the event loop until no microtasks and no pending timers remain,
    /// or a safety limit is reached.  Useful after `execute()` to let
    /// async code (Promises, setTimeout-based state machines) complete.
    ///
    /// `max_rounds` caps the number of 1ms-tick iterations (default: 10000,
    /// i.e. simulates up to 10 seconds of wall time).
    pub fn run_event_loop(&mut self, max_rounds: u32) {
        for _ in 0..max_rounds {
            self.drain_microtasks();
            if !self.event_loop.has_microtasks() && !self.event_loop.has_pending_timers() {
                break;
            }
            self.tick(1);
        }
        // Final drain.
        self.drain_microtasks();
    }

    pub fn set_global(&mut self, name: &str, value: JsValue) {
        self.globals.borrow_mut().set(String::from(name), value);
    }

    pub fn get_global(&mut self, name: &str) -> JsValue {
        self.globals.borrow().get(name)
    }

    /// Look up a name in the with-scope chain (most-recently-pushed first).
    /// Returns `Some(value)` if found, `None` otherwise.
    fn with_scope_get(&self, name: &str) -> Option<JsValue> {
        for scope in self.with_scopes.iter().rev() {
            let obj = scope.borrow();
            if obj.has(name) {
                return Some(obj.get(name));
            }
        }
        None
    }

    /// Try to store a value in the with-scope chain.
    /// Returns `true` if the name was found on a with-object and set there.
    fn with_scope_set(&self, name: &str, val: &JsValue) -> bool {
        for scope in self.with_scopes.iter().rev() {
            if scope.borrow().has(name) {
                scope.borrow_mut().set(String::from(name), val.clone());
                return true;
            }
        }
        false
    }

    pub fn register_native(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
        let f = JsFunction {
            name: Some(String::from(name)),
            params: Vec::new(),
            kind: FnKind::Native(func),
            this_binding: None,
            bound_args: Vec::new(),
            upvalues: Vec::new(),
            prototype: None,
            own_props: BTreeMap::new(),
            arity: None,
            super_class: None,
        };
        self.set_global(name, JsValue::Function(Rc::new(RefCell::new(f))));
    }

    /// Append a diagnostic message to the engine log.
    pub fn log_engine(&mut self, msg: &str) {
        self.engine_log.push(String::from(msg));
    }

    // ── Main execution loop ──

    pub fn run(&mut self) -> JsValue {
        loop {
            self.steps += 1;
            if self.steps > self.step_limit {
                self.log_engine("[libjs] WARN: step limit reached — aborting execution");
                return JsValue::Undefined;
            }

            if self.frames.is_empty() || self.frames.len() <= self.run_target_depth {
                return self.stack.pop().unwrap_or(JsValue::Undefined);
            }

            // Process pending exceptions (e.g. from generator.throw()).
            if let Some(exc) = self.pending_exception.take() {
                if !self.handle_exception(exc) {
                    return JsValue::Undefined;
                }
                continue;
            }
            if let Some(exc) = self.last_exception.take() {
                self.pending_exception = Some(exc);
                continue;
            }

            let frame_idx = self.frames.len() - 1;
            let ip = self.frames[frame_idx].ip;
            if ip >= self.frames[frame_idx].chunk.code.len() {
                if self.frames.len() <= self.run_target_depth + 1 {
                    self.frames.pop();
                    return self.stack.pop().unwrap_or(JsValue::Undefined);
                }
                self.frames.pop();
                continue;
            }

            let op = self.frames[frame_idx].chunk.code[ip].clone();
            self.frames[frame_idx].ip += 1;

            match op {
                // ── Stack operations ──
                Op::LoadConst(idx) => {
                    let val = self.load_constant(frame_idx, idx);
                    self.stack.push(val);
                }
                Op::LoadEmpty => self.stack.push(JsValue::Empty),
                Op::LoadUndefined => self.stack.push(JsValue::Undefined),
                Op::LoadNull => self.stack.push(JsValue::Null),
                Op::LoadTrue => self.stack.push(JsValue::Bool(true)),
                Op::LoadFalse => self.stack.push(JsValue::Bool(false)),
                Op::Pop => {
                    self.stack.pop();
                }
                Op::Dup => {
                    if let Some(val) = self.stack.last().cloned() {
                        self.stack.push(val);
                    }
                }

                // ── Variables ──
                Op::LoadLocal(slot) => {
                    let val = self.frames[frame_idx]
                        .locals
                        .get(slot as usize)
                        .map(|s| s.get())
                        .unwrap_or(JsValue::Undefined);
                    self.stack.push(val);
                }
                Op::StoreLocal(slot) => {
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    let locals = &mut self.frames[frame_idx].locals;
                    while locals.len() <= slot as usize {
                        locals.push(LocalSlot::Direct(JsValue::Undefined));
                    }
                    locals[slot as usize].set(val);
                }
                Op::LoadGlobal(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    if name == "globalThis" {
                        // ES2020: globalThis — return the live global object
                        self.stack.push(JsValue::Object(self.globals.clone()));
                    } else if let Some(val) = self.with_scope_get(&name) {
                        self.stack.push(val);
                    } else if self.globals.borrow().has(&name) {
                        let val = self.globals.borrow().get(&name);
                        self.stack.push(val);
                    } else {
                        let msg = format!("{} is not defined", name);
                        let err = self.make_reference_error(&msg);
                        if !self.handle_exception(err) {
                            return JsValue::Undefined;
                        }
                    }
                }
                Op::LoadGlobalSafe(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    if name == "globalThis" {
                        self.stack.push(JsValue::Object(self.globals.clone()));
                    } else if let Some(val) = self.with_scope_get(&name) {
                        self.stack.push(val);
                    } else {
                        let val = self.globals.borrow().get(&name);
                        self.stack.push(val);
                    }
                }
                Op::StoreGlobal(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    // Check with-scope first: if the name exists on a with-object, set it there.
                    if self.with_scope_set(&name, &val) {
                        // stored in with-scope
                    } else if self.frames[frame_idx].chunk.strict
                        && !self.globals.borrow().properties.contains_key(&name)
                        && !self.frames[frame_idx]
                            .chunk
                            .declared_globals
                            .iter()
                            .any(|g| g == &name)
                    {
                        let msg = format!("{} is not defined", name);
                        let err = self.make_reference_error(&msg);
                        if !self.handle_exception(err) {
                            return JsValue::Undefined;
                        }
                    } else {
                        self.globals.borrow_mut().set(name, val);
                    }
                }
                Op::EnterWith => {
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj_rc = match obj {
                        JsValue::Object(rc) => rc,
                        other => {
                            // ToObject: wrap primitive in object
                            let mut wrapper = JsObject::new();
                            wrapper.set(String::from("__primitiveValue__"), other);
                            Rc::new(RefCell::new(wrapper))
                        }
                    };
                    self.with_scopes.push(obj_rc);
                }
                Op::DeleteName(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let mut deleted = false;
                    // Check with-scopes first (most recent first)
                    for scope in self.with_scopes.iter().rev() {
                        if scope.borrow().has(&name) {
                            scope.borrow_mut().delete(&name);
                            deleted = true;
                            break;
                        }
                    }
                    if !deleted {
                        // Try global scope
                        if self.globals.borrow().has(&name) {
                            self.globals.borrow_mut().delete(&name);
                            deleted = true;
                        }
                    }
                    self.stack.push(JsValue::Bool(deleted));
                }
                Op::DeclareGlobal(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    if !self.globals.borrow().has(&name) {
                        self.globals
                            .borrow_mut()
                            .set(name, JsValue::Undefined);
                    }
                }
                Op::LeaveWith => {
                    self.with_scopes.pop();
                }
                Op::LoadUpvalue(idx) => {
                    let val = self.frames[frame_idx]
                        .upvalue_cells
                        .get(idx as usize)
                        .map(|c| c.borrow().clone())
                        .unwrap_or(JsValue::Undefined);
                    self.stack.push(val);
                }
                Op::StoreUpvalue(idx) => {
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let Some(cell) = self.frames[frame_idx].upvalue_cells.get(idx as usize) {
                        *cell.borrow_mut() = val;
                    }
                }

                // ── Arithmetic ──
                Op::Add => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);

                    let depth_before = self.frames.len();

                    // ToPrimitive(a, "default")
                    let a_prim = if matches!(
                        a,
                        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
                    ) {
                        let result = self.to_primitive_for_op(a, "default");
                        if self.frames.len() < depth_before {
                            continue;
                        }
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        result
                    } else {
                        a
                    };

                    // ToPrimitive(b, "default")
                    let b_prim = if matches!(
                        b,
                        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
                    ) {
                        let result = self.to_primitive_for_op(b, "default");
                        if self.frames.len() < depth_before {
                            continue;
                        }
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        result
                    } else {
                        b
                    };

                    // Symbol + anything → TypeError (ES2023 §13.15.3)
                    if is_symbol_value(&a_prim) || is_symbol_value(&b_prim) {
                        let err = self.make_type_error("Cannot convert a Symbol value to a number");
                        if !self.handle_exception(err) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }

                    // Perform addition
                    let result = match (&a_prim, &b_prim) {
                        // BigInt + BigInt
                        (JsValue::BigInt(a), JsValue::BigInt(b)) => {
                            JsValue::BigInt(a.add(b))
                        }
                        // BigInt + Number or Number + BigInt → TypeError
                        (JsValue::BigInt(_), JsValue::Number(_))
                        | (JsValue::Number(_), JsValue::BigInt(_)) => {
                            let err = self.make_type_error(
                                "Cannot mix BigInt and other types, use explicit conversions",
                            );
                            if !self.handle_exception(err) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        (JsValue::String(sa), _) => {
                            let mut r = sa.clone();
                            r.push_str(&b_prim.to_js_string());
                            JsValue::String(r)
                        }
                        (_, JsValue::String(sb)) => {
                            let mut r = a_prim.to_js_string();
                            r.push_str(sb);
                            JsValue::String(r)
                        }
                        _ => JsValue::Number(a_prim.to_number() + b_prim.to_number()),
                    };
                    self.stack.push(result);
                }
                Op::Sub => self.binary_num_or_bigint_op(|a, b| a - b, |a, b| a.sub(b)),
                Op::Mul => self.binary_num_or_bigint_op(|a, b| a * b, |a, b| a.mul(b)),
                Op::Div => self.binary_num_or_bigint_op(|a, b| a / b, |a, b| a.div(b)),
                Op::Mod => self.binary_num_or_bigint_op(|a, b| a % b, |a, b| a.rem(b)),
                Op::Exp => self.binary_num_op(|a, b| native_math::pow_f64(a, b)),
                Op::Neg => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if let JsValue::BigInt(bi) = &a {
                        self.stack.push(JsValue::BigInt(bi.neg()));
                    } else {
                        self.stack.push(JsValue::Number(-a.to_number()));
                    }
                }
                Op::Pos => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(a.to_number()));
                }

                // ── Bitwise ──
                Op::BitAnd => self.binary_int_op(|a, b| a & b),
                Op::BitOr => self.binary_int_op(|a, b| a | b),
                Op::BitXor => self.binary_int_op(|a, b| a ^ b),
                Op::BitNot => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack
                        .push(JsValue::Number((!(a.to_number() as i32)) as f64));
                }
                Op::Shl => self.binary_int_op(|a, b| a << (b & 31)),
                Op::Shr => self.binary_int_op(|a, b| a >> (b & 31)),
                Op::UShr => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as u32;
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as u32;
                    self.stack.push(JsValue::Number((a >> (b & 31)) as f64));
                }

                // ── Comparison ──
                Op::Eq => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(a.abstract_eq(&b)));
                }
                Op::Ne => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(!a.abstract_eq(&b)));
                }
                Op::StrictEq => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(a.strict_eq(&b)));
                }
                Op::StrictNe => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(!a.strict_eq(&b)));
                }
                Op::Lt => self.compare_op(|a, b| a < b),
                Op::Le => self.compare_op(|a, b| a <= b),
                Op::Gt => self.compare_op(|a, b| a > b),
                Op::Ge => self.compare_op(|a, b| a >= b),

                // ── Logical ──
                Op::Not => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(!a.to_boolean()));
                }

                // ── Control flow ──
                Op::Jump(offset) => {
                    let ip = self.frames[frame_idx].ip as i32 + offset;
                    self.frames[frame_idx].ip = ip as usize;
                }
                Op::JumpIfTrue(offset) => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if val.to_boolean() {
                        let ip = self.frames[frame_idx].ip as i32 + offset;
                        self.frames[frame_idx].ip = ip as usize;
                    }
                }
                Op::JumpIfFalse(offset) => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if !val.to_boolean() {
                        let ip = self.frames[frame_idx].ip as i32 + offset;
                        self.frames[frame_idx].ip = ip as usize;
                    }
                }
                Op::JumpIfNullish(offset) => {
                    let val = self.stack.last().unwrap_or(&JsValue::Undefined).clone();
                    if val.is_nullish() {
                        let ip = self.frames[frame_idx].ip as i32 + offset;
                        self.frames[frame_idx].ip = ip as usize;
                    }
                }

                // ── Functions ──
                Op::Call(argc) => {
                    self.call_function(argc as usize);
                }
                Op::CallMethod(argc) => {
                    self.call_method(argc as usize);
                }
                Op::Return => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // Clean up any try-catch handlers that belong to the
                    // returning frame.  This handles `try { return x; }`
                    // where Op::TryEnd is never reached.
                    let returning_depth = self.frames.len();
                    while let Some(h) = self.try_handlers.last() {
                        if h.frame_depth >= returning_depth {
                            self.try_handlers.pop();
                        } else {
                            break;
                        }
                    }
                    let frame = self.frames.pop().unwrap();
                    let is_async_fn = frame.chunk.is_async;
                    self.close_iterators_in_stack_range(frame.stack_base);
                    self.stack.truncate(frame.stack_base);
                    // `new` calls: constructor return value semantics (ES2023 §9.2.2 step 13).
                    let ret = if frame.is_constructor
                        && !val.is_object()
                        && !matches!(val, JsValue::Function(_))
                    {
                        // Derived class constructor: if return value is not undefined,
                        // throw TypeError (§9.2.2 step 13c).
                        let is_derived = match &frame.self_ref {
                            JsValue::Function(f) => f.borrow().super_class.is_some(),
                            _ => false,
                        };
                        if is_derived && !matches!(val, JsValue::Undefined) {
                            let exc = self.make_type_error(
                                "Derived constructors may only return object or undefined",
                            );
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        // Base class: ignore non-object return, use `this`.
                        frame.this_val
                    } else {
                        val
                    };
                    // Async functions always return a Promise, even when the
                    // function body returns a plain object.
                    let ret = if is_async_fn {
                        native_promise::make_resolved_promise(ret)
                    } else {
                        ret
                    };
                    self.stack.push(ret.clone());
                    if self.frames.is_empty() || self.frames.len() <= self.run_target_depth {
                        return ret;
                    }
                }
                Op::Closure(idx) => {
                    let chunk = match &self.frames[frame_idx].chunk.constants[idx as usize] {
                        Constant::Function(c) => c.clone(),
                        _ => Chunk::new(),
                    };
                    // Capture upvalue cells as described by the chunk's upvalue_refs.
                    let mut upvalue_cells: Vec<Rc<RefCell<JsValue>>> = Vec::new();
                    for uv_ref in &chunk.upvalues.clone() {
                        let cell = if uv_ref.is_local {
                            // Capture the shared Rc<RefCell> cell of a captured local.
                            match self.frames[frame_idx].locals.get(uv_ref.index as usize) {
                                Some(LocalSlot::Cell(c)) => c.clone(),
                                Some(LocalSlot::Direct(_)) => {
                                    // Fallback: promote Direct → Cell on the fly
                                    // (should not happen if compiler marks correctly,
                                    //  but be safe for edge cases)
                                    let val = self.frames[frame_idx].locals[uv_ref.index as usize].get();
                                    let c = Rc::new(RefCell::new(val));
                                    self.frames[frame_idx].locals[uv_ref.index as usize] = LocalSlot::Cell(c.clone());
                                    c
                                }
                                None => Rc::new(RefCell::new(JsValue::Undefined)),
                            }
                        } else {
                            // Re-capture from this frame's own upvalue cells (upvalue-of-upvalue).
                            self.frames[frame_idx]
                                .upvalue_cells
                                .get(uv_ref.index as usize)
                                .cloned()
                                .unwrap_or_else(|| Rc::new(RefCell::new(JsValue::Undefined)))
                        };
                        upvalue_cells.push(cell);
                    }
                    // Populate `params` with `param_count` entries so `fn.length` works.
                    let param_stubs: Vec<alloc::string::String> = (0..chunk.param_count)
                        .map(|_| alloc::string::String::new())
                        .collect();
                    // Arrow functions lexically capture `this` from the enclosing
                    // scope (ES6 §14.2.16).  Regular functions leave this_binding
                    // as None so that the caller determines `this` at call time.
                    let is_arrow = chunk.is_arrow;
                    let this_binding = if is_arrow {
                        Some(self.frames[frame_idx].this_val.clone())
                    } else {
                        None
                    };
                    let func_rc = Rc::new(RefCell::new(JsFunction {
                        name: chunk.name.clone(),
                        params: param_stubs,
                        kind: FnKind::Bytecode(chunk),
                        this_binding,
                        bound_args: Vec::new(),
                        upvalues: upvalue_cells,
                        prototype: None,
                        own_props: BTreeMap::new(),
                        arity: None,
                        super_class: None,
                    }));
                    // Arrow functions are NOT constructable and have no .prototype
                    // (ES6 §14.2.17).  Only regular functions get a prototype.
                    if !is_arrow {
                        let proto = Rc::new(RefCell::new(JsObject::new()));
                        proto.borrow_mut().prototype = Some(self.object_proto.clone());
                        proto.borrow_mut().set(
                            String::from("constructor"),
                            JsValue::Function(func_rc.clone()),
                        );
                        func_rc.borrow_mut().prototype = Some(proto);
                    }
                    self.stack.push(JsValue::Function(func_rc));
                }

                // ── Objects and Properties ──
                Op::GetProp => {
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if matches!(obj, JsValue::Null | JsValue::Undefined) {
                        let key_str = key.to_js_string();
                        let msg = alloc::format!(
                            "Cannot read properties of {} (reading '{}')",
                            if matches!(obj, JsValue::Null) {
                                "null"
                            } else {
                                "undefined"
                            },
                            key_str
                        );
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                    } else {
                        let key_str = key.to_js_string();
                        if let JsValue::Object(ref o) = obj {
                            if o.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                                let val = native_proxy::proxy_get(self, &obj, &key_str)
                                    .unwrap_or(JsValue::Undefined);
                                self.stack.push(val);
                            } else if let Some(getter) = self.find_getter(&obj, &key_str) {
                                self.invoke_function(&getter, &[], obj.clone());
                            } else {
                                let val = self.get_property_with_proto(&obj, &key_str);
                                self.stack.push(val);
                            }
                        } else if let Some(getter) = self.find_getter(&obj, &key_str) {
                            self.invoke_function(&getter, &[], obj.clone());
                        } else {
                            let val = self.get_property_with_proto(&obj, &key_str);
                            self.stack.push(val);
                        }
                    }
                }
                Op::SetProp => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_str = key.to_js_string();
                    // Strict mode: TypeError when setting property on primitive.
                    if self.frames[frame_idx].chunk.strict {
                        if matches!(obj, JsValue::Undefined | JsValue::Null) {
                            let msg = alloc::format!(
                                "Cannot set properties of {} (setting '{}')",
                                if matches!(obj, JsValue::Null) {
                                    "null"
                                } else {
                                    "undefined"
                                },
                                key_str
                            );
                            let exc = self.make_type_error(&msg);
                            self.stack.push(val);
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        if let JsValue::Object(ref o) = obj {
                            let is_non_writable = {
                                let ob = o.borrow();
                                ob.properties
                                    .get(&key_str)
                                    .map(|p| !p.writable && !p.is_accessor())
                                    .unwrap_or(false)
                            };
                            if is_non_writable {
                                let msg = alloc::format!(
                                    "Cannot assign to read only property '{}'",
                                    key_str
                                );
                                let exc = self.make_type_error(&msg);
                                self.stack.push(val);
                                if !self.handle_exception(exc) {
                                    return JsValue::Undefined;
                                }
                                continue;
                            }
                        }
                    }
                    // `__proto__` assignment updates the actual prototype chain.
                    if key_str == "__proto__" {
                        if let JsValue::Object(obj_rc) = &obj {
                            match &val {
                                JsValue::Object(proto_rc) => {
                                    obj_rc.borrow_mut().prototype = Some(proto_rc.clone());
                                }
                                JsValue::Null => {
                                    obj_rc.borrow_mut().prototype = None;
                                }
                                _ => {}
                            }
                        }
                    } else if let Some(setter) = self.find_setter(&obj, &key_str) {
                        self.invoke_function(&setter, &[val.clone()], obj.clone());
                    } else if let JsValue::Object(ref o) = obj {
                        if o.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                            native_proxy::proxy_set(self, &obj, &key_str, &val);
                        } else {
                            obj.set_property(key_str, val.clone());
                        }
                    } else {
                        obj.set_property(key_str, val.clone());
                    }
                    self.stack.push(val);
                }
                Op::GetPropNamed(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if matches!(obj, JsValue::Null | JsValue::Undefined) {
                        let msg = alloc::format!(
                            "Cannot read properties of {} (reading '{}')",
                            if matches!(obj, JsValue::Null) {
                                "null"
                            } else {
                                "undefined"
                            },
                            name
                        );
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                    // Private member brand check (ES2023 §7.3.29 PrivateGet)
                    } else if name.starts_with('#') {
                        let private_name = Self::mangle_private_name(&name);
                        let has_private = self.has_private_member(&obj, &private_name);
                        if has_private {
                            if let Some(getter) = self.find_getter(&obj, &private_name) {
                                self.invoke_function(&getter, &[], obj.clone());
                            } else {
                                let val = self.get_property_with_proto(&obj, &private_name);
                                self.stack.push(val);
                            }
                        } else {
                            // Check prototype for private method/accessor (installed on proto)
                            let proto_val = self.get_property_with_proto(&obj, &private_name);
                            if !matches!(proto_val, JsValue::Undefined) {
                                // Private getter without getter on this brand
                                if let Some(getter) = self.find_getter(&obj, &private_name) {
                                    self.invoke_function(&getter, &[], obj.clone());
                                } else {
                                    self.stack.push(proto_val);
                                }
                            } else {
                                let msg = alloc::format!(
                                    "Cannot read private member {} from an object whose class did not declare it",
                                    name
                                );
                                let exc = self.make_type_error(&msg);
                                if !self.handle_exception(exc) {
                                    return JsValue::Undefined;
                                }
                            }
                        }
                    } else if let JsValue::Object(ref o) = obj {
                        if o.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                            let val = native_proxy::proxy_get(self, &obj, &name)
                                .unwrap_or(JsValue::Undefined);
                            self.stack.push(val);
                        } else if let Some(getter) = self.find_getter(&obj, &name) {
                            self.invoke_function(&getter, &[], obj.clone());
                        } else {
                            let val = self.get_property_with_proto(&obj, &name);
                            self.stack.push(val);
                        }
                    } else if let Some(getter) = self.find_getter(&obj, &name) {
                        self.invoke_function(&getter, &[], obj.clone());
                    } else {
                        let val = self.get_property_with_proto(&obj, &name);
                        self.stack.push(val);
                    }
                }
                Op::SetPropNamed(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // TypeError: setting property on null/undefined.
                    if matches!(obj, JsValue::Null | JsValue::Undefined) {
                        let msg = alloc::format!(
                            "Cannot set properties of {} (setting '{}')",
                            if matches!(obj, JsValue::Null) {
                                "null"
                            } else {
                                "undefined"
                            },
                            name
                        );
                        let exc = self.make_type_error(&msg);
                        self.stack.push(val);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    // Private member write check (ES2023 §7.3.30 PrivateSet)
                    if name.starts_with('#') {
                        let private_name = Self::mangle_private_name(&name);
                        // Check if the target has this private name
                        let has_own = self.has_private_member(&obj, &private_name);
                        if has_own {
                            // Field exists on own object — check if it's a method (not writable)
                            let is_method = match &obj {
                                JsValue::Object(o) => {
                                    matches!(o.borrow().get(&private_name), JsValue::Function(_))
                                        && !o.borrow().properties.get(&private_name)
                                            .map_or(false, |p| p.writable)
                                }
                                JsValue::Function(f) => {
                                    let func = f.borrow();
                                    matches!(
                                        func.own_props.get(&private_name),
                                        Some(JsValue::Function(_))
                                    ) && !func
                                        .own_props
                                        .contains_key(&alloc::format!("__set_{}", private_name))
                                }
                                _ => false,
                            };
                            if is_method {
                                let msg = alloc::format!(
                                    "Private method '{}' is not writable",
                                    name
                                );
                                let exc = self.make_type_error(&msg);
                                self.stack.push(val);
                                if !self.handle_exception(exc) {
                                    return JsValue::Undefined;
                                }
                                continue;
                            }
                            // Check for setter (accessor property)
                            if let Some(setter) = self.find_setter(&obj, &private_name) {
                                self.invoke_function(&setter, &[val.clone()], obj.clone());
                            } else {
                                obj.set_property(private_name.clone(), val.clone());
                            }
                        } else {
                            // Check prototype for private method/accessor
                            let proto_val = self.get_property_with_proto(&obj, &private_name);
                            if matches!(proto_val, JsValue::Function(_)) {
                                // Trying to write to a private method — TypeError
                                let msg = alloc::format!(
                                    "Private method '{}' is not writable",
                                    name
                                );
                                let exc = self.make_type_error(&msg);
                                self.stack.push(val);
                                if !self.handle_exception(exc) {
                                    return JsValue::Undefined;
                                }
                                continue;
                            }
                            // Check for setter on prototype
                            if let Some(setter) = self.find_setter(&obj, &private_name) {
                                self.invoke_function(&setter, &[val.clone()], obj.clone());
                            } else {
                                let msg = alloc::format!(
                                    "Cannot write private member {} to an object whose class did not declare it",
                                    name
                                );
                                let exc = self.make_type_error(&msg);
                                self.stack.push(val);
                                if !self.handle_exception(exc) {
                                    return JsValue::Undefined;
                                }
                                continue;
                            }
                        }
                        self.stack.push(val);
                        continue;
                    }
                    // `__proto__` assignment updates the actual prototype chain.
                    if name == "__proto__" {
                        if let JsValue::Object(obj_rc) = &obj {
                            match &val {
                                JsValue::Object(proto_rc) => {
                                    obj_rc.borrow_mut().prototype = Some(proto_rc.clone());
                                }
                                JsValue::Null => {
                                    obj_rc.borrow_mut().prototype = None;
                                }
                                _ => {}
                            }
                        }
                    } else {
                        // Check for setter (accessor property)
                        if let Some(setter) = self.find_setter(&obj, &name) {
                            self.invoke_function(&setter, &[val.clone()], obj.clone());
                        } else {
                            // Strict mode checks
                            if self.frames[frame_idx].chunk.strict {
                                let reject = if let JsValue::Object(obj_rc) = &obj {
                                    let o = obj_rc.borrow();
                                    if let Some(prop) = o.properties.get(&name) {
                                        if prop.is_accessor() {
                                            // Accessor without setter → TypeError
                                            true
                                        } else if !prop.writable {
                                            // Non-writable data property → TypeError
                                            true
                                        } else {
                                            false
                                        }
                                    } else {
                                        // Property doesn't exist on own object.
                                        // Check if object is non-extensible (preventExtensions/seal/freeze).
                                        let non_extensible =
                                            o.properties.contains_key("__non_extensible__");
                                        if non_extensible {
                                            true
                                        } else {
                                            // Check prototype chain for accessor without setter
                                            drop(o);
                                            self.proto_has_readonly_or_setter_undef(&obj, &name)
                                        }
                                    }
                                } else {
                                    false
                                };
                                if reject {
                                    let msg = alloc::format!(
                                        "Cannot assign to read only property '{}'",
                                        name
                                    );
                                    let exc = self.make_type_error(&msg);
                                    self.stack.push(val);
                                    if !self.handle_exception(exc) {
                                        return JsValue::Undefined;
                                    }
                                    continue;
                                }
                            }
                            obj.set_property(name, val.clone());
                        }
                    }
                    self.stack.push(val);
                }
                Op::NewObject => {
                    let obj = JsObject {
                        properties: BTreeMap::new(),
                        prototype: Some(self.object_proto.clone()),
                        internal_tag: None,
                        primitive_value: None,
                        set_hook: None,
                        set_hook_data: core::ptr::null_mut(),
                    };
                    self.stack.push(JsValue::Object(Rc::new(RefCell::new(obj))));
                }
                Op::NewArray(count) => {
                    let start = self.stack.len().saturating_sub(count as usize);
                    let elements: Vec<JsValue> = self.stack.drain(start..).collect();
                    let arr = JsArray::from_vec(elements);
                    self.stack.push(JsValue::Array(Rc::new(RefCell::new(arr))));
                }

                // ── Constructors ──
                Op::New(argc) => {
                    self.new_object(argc as usize);
                }
                Op::SuperCall(argc) => {
                    self.super_call(argc as usize);
                }
                Op::SuperCallSpread => {
                    // Stack: [..., SuperClass, args_array]
                    let args_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let args: Vec<JsValue> = match &args_val {
                        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
                        _ => Vec::new(),
                    };
                    // Push super class + expanded args back, then call super_call.
                    // super_call expects: [..., SuperClass, arg1, ..., argN]
                    // SuperClass is already on the stack.
                    for arg in &args {
                        self.stack.push(arg.clone());
                    }
                    self.super_call(args.len());
                }

                Op::SetSuperClass => {
                    // Stack: [..., Constructor, SuperClass]
                    let super_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // ES2023 §15.7.11 ClassDefinitionEvaluation:
                    // SuperClass must be null or a constructable function.
                    if !matches!(super_val, JsValue::Null) {
                        let is_ctor = matches!(super_val, JsValue::Function(_));
                        if !is_ctor {
                            let exc = self.make_type_error(
                                "Class extends value is not a constructor or null",
                            );
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                        // SuperClass.prototype must be an object or null.
                        let proto = self.get_property_invoking_getter(&super_val, "prototype");
                        if !matches!(proto, JsValue::Object(_) | JsValue::Null | JsValue::Function(_))
                            && !proto.is_array()
                        {
                            // Undefined prototype means bound function without prototype — TypeError
                            let exc = self.make_type_error(
                                "Class extends value does not have valid prototype property",
                            );
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            continue;
                        }
                    }
                    if let Some(JsValue::Function(ref ctor_fn)) = self.stack.last() {
                        ctor_fn.borrow_mut().super_class = Some(super_val);
                    }
                }

                // ── Special operators ──
                Op::Typeof => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack
                        .push(JsValue::String(String::from(val.type_of())));
                }
                Op::Void => {
                    self.stack.pop();
                    self.stack.push(JsValue::Undefined);
                }
                Op::Delete => {
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // ES2023 §13.5.1.2: ToObject on null/undefined throws TypeError
                    if matches!(obj, JsValue::Null | JsValue::Undefined) {
                        let type_str = if matches!(obj, JsValue::Null) { "null" } else { "undefined" };
                        let msg = alloc::format!(
                            "Cannot convert {} to object",
                            type_str
                        );
                        let exc = self.make_type_error(&msg);
                        self.stack.push(JsValue::Bool(false));
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    let key_str = key.to_js_string();
                    let success = obj.delete_property(&key_str);
                    // Strict mode: TypeError when deleting non-configurable property.
                    if !success && self.frames[frame_idx].chunk.strict {
                        let msg = alloc::format!(
                            "Cannot delete property '{}' of {}",
                            key_str,
                            if matches!(obj, JsValue::Object(_)) {
                                "#<Object>"
                            } else {
                                "value"
                            }
                        );
                        let exc = self.make_type_error(&msg);
                        self.stack.push(JsValue::Bool(false));
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    self.stack.push(JsValue::Bool(success));
                }
                Op::InstanceOf => {
                    let right = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let left = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // ES2023 §13.10.2: TypeError if right side is not callable.
                    if !matches!(right, JsValue::Function(_)) {
                        let exc =
                            self.make_type_error("Right-hand side of instanceof is not callable");
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    let result = self.instance_of(&left, &right);
                    self.stack.push(JsValue::Bool(result));
                }
                Op::In => {
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // ES2023 §13.10.2: TypeError if right side is not an object.
                    if !matches!(
                        obj,
                        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
                    ) {
                        let msg = alloc::format!(
                            "Cannot use 'in' operator to search for '{}' in {}",
                            key.to_js_string(),
                            obj.type_of()
                        );
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    let key_str = key.to_js_string();
                    let result = match &obj {
                        JsValue::Object(o) => o.borrow().has(&key_str),
                        JsValue::Array(a) => {
                            let arr = a.borrow();
                            if let Some(idx) = try_parse_index(&key_str) {
                                arr.has(idx)
                            } else {
                                key_str == "length" || arr.properties.contains_key(&key_str)
                            }
                        }
                        JsValue::Function(f) => {
                            let func = f.borrow();
                            let builtin_deleted = |name: &str| {
                                matches!(
                                    func.own_props.get(&alloc::format!("__deleted_builtin_{}", name)),
                                    Some(JsValue::Bool(true))
                                )
                            };
                            // Check own_props, then built-in properties
                            if func.own_props.contains_key(&key_str) {
                                true
                            } else {
                                match key_str.as_str() {
                                    "name" => !builtin_deleted("name"),
                                    "length" => !builtin_deleted("length"),
                                    "prototype" => {
                                        !func.kind.is_arrow() && !builtin_deleted("prototype")
                                    }
                                    _ => {
                                        // Check function prototype chain
                                        drop(func);
                                        let proto_val =
                                            self.get_property_with_proto(&obj, &key_str);
                                        !matches!(proto_val, JsValue::Undefined)
                                    }
                                }
                            }
                        }
                        _ => false,
                    };
                    self.stack.push(JsValue::Bool(result));
                }
                Op::ToPropertyKey => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key = match &val {
                        JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => {
                            let prim = self.to_primitive_for_op(val, "string");
                            if self.pending_exception.is_some() {
                                return JsValue::Undefined;
                            }
                            JsValue::String(prim.to_js_string())
                        }
                        _ => JsValue::String(val.to_js_string()),
                    };
                    self.stack.push(key);
                }

                // ── Iteration ──
                Op::GetIterator => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let iter_obj = self.create_iterator(&val);
                    if matches!(iter_obj, JsValue::Empty) {
                        continue;
                    }
                    if let Some(exc) = self.pending_exception.take() {
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    self.stack.push(iter_obj);
                }
                Op::GetForInIterator => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let iter_obj = self.create_for_in_iterator(&val);
                    self.stack.push(iter_obj);
                }
                Op::IterNext => {
                    // Pop the iterator, advance it, push it back + value + has_more.
                    let iter = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let (value, has_more) = self.iter_next_for(&iter);
                    if matches!(value, JsValue::Empty) {
                        continue;
                    }
                    if let Some(exc) = self.pending_exception.take() {
                        let exc = self.close_iterator_on_abrupt(&iter, exc);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                        continue;
                    }
                    self.stack.push(iter);
                    self.stack.push(value);
                    self.stack.push(JsValue::Bool(has_more));
                }
                Op::IterCollectRest => {
                    let iter = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let mut items = Vec::new();
                    let mut resumed_via_exception = false;
                    loop {
                        let (value, has_more) = self.iter_next_for(&iter);
                        if matches!(value, JsValue::Empty) {
                            resumed_via_exception = true;
                            break;
                        }
                        if let Some(exc) = self.pending_exception.take() {
                            let exc = self.close_iterator_on_abrupt(&iter, exc);
                            if !self.handle_exception(exc) {
                                return JsValue::Undefined;
                            }
                            resumed_via_exception = true;
                            break;
                        }
                        if !has_more {
                            break;
                        }
                        items.push(value);
                    }
                    if resumed_via_exception {
                        continue;
                    }
                    self.stack.push(iter);
                    self.stack.push(JsValue::new_array(items));
                }
                Op::IteratorClose => {
                    let iter = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    let return_fn = self.get_property_invoking_getter(&iter, "return");
                    if let Some(exc) = self.last_exception.take() {
                        self.pending_exception = Some(exc);
                    }
                    if matches!(return_fn, JsValue::Undefined | JsValue::Null) {
                        continue;
                    }
                    if !return_fn.is_function() {
                        self.pending_exception =
                            Some(self.make_type_error("IteratorClose return method is not callable"));
                    } else {
                        let result = self.call_value(&return_fn, &[], iter.clone());
                        if let Some(exc) = self.last_exception.take() {
                            self.pending_exception = Some(exc);
                        } else if !matches!(
                            result,
                            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
                        ) {
                            self.pending_exception = Some(
                                self.make_type_error("IteratorClose return() must return an object"),
                            );
                        }
                    }
                    if let Some(exc) = self.pending_exception.take() {
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                    }
                }

                // ── Exception handling ──
                Op::TryCatch(catch_off, _finally_off) => {
                    let catch_ip = (self.frames[frame_idx].ip as i32 + catch_off) as usize;
                    self.try_handlers.push(TryHandler {
                        catch_ip,
                        stack_depth: self.stack.len(),
                        frame_depth: self.frames.len(),
                    });
                }
                Op::TryEnd => {
                    self.try_handlers.pop();
                }
                Op::Throw => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let detail = self.describe_exception(&val);
                    let stack_info = self.frame_stack_summary(6);
                    // Log with extra detail for non-Error thrown values (like `false`)
                    if matches!(&val, JsValue::Bool(_) | JsValue::Number(_) | JsValue::Null) {
                        self.log_engine(&format!(
                            "[libjs] exception thrown NON-ERROR: {:?} [{}]",
                            val, stack_info
                        ));
                    } else {
                        self.log_engine(&format!(
                            "[libjs] exception thrown: {} [{}]",
                            detail, stack_info
                        ));
                    }
                    if !self.handle_exception(val) {
                        return JsValue::Undefined;
                    }
                }

                // ── Inc/Dec ──
                Op::Inc => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(val.to_number() + 1.0));
                }
                Op::Dec => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(val.to_number() - 1.0));
                }

                // ── This ──
                Op::LoadThis => {
                    let this_val = self.frames[frame_idx].this_val.clone();
                    self.stack.push(this_val);
                }

                // ── Spread / ArrayPush ──
                Op::Spread => {
                    // Stack: [..., target_array, value_to_spread]
                    // Pop both, extend target with elements of value, push target back.
                    let src = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let tgt = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if let JsValue::Array(tgt_rc) = &tgt {
                        match &src {
                            JsValue::Array(src_rc) => {
                                let src_a = src_rc.borrow();
                                let mut tgt_a = tgt_rc.borrow_mut();
                                for (_, el) in src_a.iter_entries() {
                                    tgt_a.push(el.clone());
                                }
                            }
                            JsValue::String(s) => {
                                for ch in s.chars() {
                                    let mut cs = String::new();
                                    cs.push(ch);
                                    tgt_rc.borrow_mut().push(JsValue::String(cs));
                                }
                            }
                            JsValue::Object(_) => {
                                // Use Symbol.iterator protocol
                                let depth_before = self.frames.len();
                                let sym_iter_key = native_symbol::WELL_KNOWN_ITERATOR;
                                let iter_fn = self.get_property_with_proto(&src, sym_iter_key);
                                let mut spread_exc_handled = false;
                                if matches!(iter_fn, JsValue::Function(_)) {
                                    let iterator = self.call_value(&iter_fn, &[], src.clone());
                                    if self.frames.len() < depth_before {
                                        self.stack.push(tgt);
                                        continue;
                                    }
                                    // Propagate unhandled exceptions from call_value
                                    if let Some(exc) = self.last_exception.take() {
                                        self.pending_exception = Some(exc);
                                    }
                                    if let Some(exc) = self.pending_exception.take() {
                                        if !self.handle_exception(exc) {
                                            return JsValue::Undefined;
                                        }
                                        // handle_exception already set up the stack for the catch block
                                        continue;
                                    }
                                    // ES spec 7.4.1: if iterator is not an object, throw TypeError
                                    if !matches!(iterator, JsValue::Object(_) | JsValue::Array(_)) {
                                        let err = self.make_type_error(
                                            "Result of the Symbol.iterator method is not an object",
                                        );
                                        if !self.handle_exception(err) {
                                            return JsValue::Undefined;
                                        }
                                        continue;
                                    }
                                    // Call next() repeatedly
                                    loop {
                                        let next_fn =
                                            self.get_property_with_proto(&iterator, "next");
                                        if !matches!(next_fn, JsValue::Function(_)) {
                                            break;
                                        }
                                        let next_result =
                                            self.call_value(&next_fn, &[], iterator.clone());
                                        if self.frames.len() < depth_before {
                                            break;
                                        }
                                        // Propagate unhandled exceptions from call_value
                                        if let Some(exc) = self.last_exception.take() {
                                            self.pending_exception = Some(exc);
                                        }
                                        if let Some(exc) = self.pending_exception.take() {
                                            if !self.handle_exception(exc) {
                                                return JsValue::Undefined;
                                            }
                                            // handle_exception already set up the stack for the catch block
                                            spread_exc_handled = true;
                                            break;
                                        }
                                        let done =
                                            self.get_property_invoking_getter(&next_result, "done");
                                        if let Some(exc) = self.pending_exception.take() {
                                            if !self.handle_exception(exc) {
                                                return JsValue::Undefined;
                                            }
                                            spread_exc_handled = true;
                                            break;
                                        }
                                        if done.to_boolean() {
                                            break;
                                        }
                                        let value = self
                                            .get_property_invoking_getter(&next_result, "value");
                                        if let Some(exc) = self.pending_exception.take() {
                                            if !self.handle_exception(exc) {
                                                return JsValue::Undefined;
                                            }
                                            spread_exc_handled = true;
                                            break;
                                        }
                                        tgt_rc.borrow_mut().push(value);
                                    }
                                }
                                if spread_exc_handled {
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    self.stack.push(tgt);
                }
                Op::ObjectSpread => {
                    // Stack: [..., target_object, source_object]
                    // CopyDataProperties: copy all own enumerable properties of source into target.
                    // Getters must be invoked (ES2023 §7.3.25 CopyDataProperties).
                    let src = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let tgt = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(tgt_rc) = &tgt {
                        match &src {
                            JsValue::Object(src_rc) => {
                                // Collect property keys and their getters/values.
                                let props: Vec<(String, Option<JsValue>, JsValue)> = src_rc
                                    .borrow()
                                    .properties
                                    .iter()
                                    .filter(|(_, p)| p.enumerable)
                                    .map(|(k, p)| (k.clone(), p.getter.clone(), p.value.clone()))
                                    .collect();
                                for (k, getter, val) in props {
                                    let v = if let Some(ref getter_fn) = getter {
                                        let r = self.call_value(getter_fn, &[], src.clone());
                                        if let Some(exc) = self.last_exception.take() {
                                            self.pending_exception = Some(exc);
                                        }
                                        if let Some(exc) = self.pending_exception.take() {
                                            if !self.handle_exception(exc) {
                                                return JsValue::Undefined;
                                            }
                                            continue;
                                        }
                                        r
                                    } else {
                                        val
                                    };
                                    tgt_rc.borrow_mut().set(k, v);
                                }
                            }
                            _ => {}
                        }
                    }
                    // target_object already on top of stack; nothing to push
                }
                Op::ArrayPush => {
                    // Stack: [..., target_array, value]
                    // Pop both, push value to target, push target back.
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let tgt = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if let JsValue::Array(tgt_rc) = &tgt {
                        tgt_rc.borrow_mut().push(val);
                    }
                    self.stack.push(tgt);
                }
                Op::LoadArgsArray(start) => {
                    // Create an Array containing all call arguments from index `start` onward.
                    let all = self.frames[frame_idx].all_args.clone();
                    let elems: Vec<JsValue> = all.into_iter().skip(start as usize).collect();
                    self.stack.push(JsValue::new_array(elems));
                }
                Op::LoadSelf => {
                    let self_val = self.frames[frame_idx].self_ref.clone();
                    self.stack.push(self_val);
                }
                Op::NewTarget => {
                    // Walk up call frames to find the nearest constructor call.
                    // Arrow functions lexically inherit new.target (ES2023 §15.3).
                    let mut target = JsValue::Undefined;
                    for fi in (0..self.frames.len()).rev() {
                        let f = &self.frames[fi];
                        if f.is_constructor {
                            target = f.self_ref.clone();
                            break;
                        }
                        // Check if this frame is an arrow function — arrows inherit new.target.
                        let is_arrow_frame = if let JsValue::Function(ref func_rc) = f.self_ref {
                            let borrowed = func_rc.borrow();
                            if let FnKind::Bytecode(ref ch) = borrowed.kind {
                                ch.is_arrow
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !is_arrow_frame {
                            break; // Non-arrow, non-constructor → new.target is undefined
                        }
                    }
                    self.stack.push(target);
                }
                Op::CallSpread => {
                    // Stack: [..., callee, args_array]
                    let args_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let callee = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let args: Vec<JsValue> = match &args_val {
                        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
                        _ => Vec::new(),
                    };
                    self.current_this = JsValue::Undefined;
                    self.invoke_function(&callee, &args, JsValue::Undefined);
                }
                Op::CallMethodSpread => {
                    // Stack: [..., this_obj, method_fn, args_array]
                    let args_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let callee = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let this_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let args: Vec<JsValue> = match &args_val {
                        JsValue::Array(arr) => arr.borrow().to_dense_vec(),
                        _ => Vec::new(),
                    };
                    self.current_this = this_val.clone();
                    self.invoke_function(&callee, &args, this_val);
                }

                // ── Async ──
                Op::Await => {
                    // Await: if the value is a promise, drain microtasks until
                    // it settles (or a safety limit is hit).  This implements
                    // the ECMAScript AwaitExpression semantics for our
                    // single-threaded VM: microtasks queued by the promise
                    // executor or `.then` chains are processed, which may
                    // resolve the awaited promise.
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(ref obj) = val {
                        let is_promise =
                            obj.borrow().internal_tag.as_deref() == Some("__promise__");
                        if is_promise {
                            // Drain microtasks — this may settle the promise.
                            self.drain_microtasks();

                            // Also run a few timer ticks for promises that
                            // resolve via setTimeout(resolve, 0) patterns.
                            let mut await_rounds = 0u32;
                            while obj.borrow().get("__state").to_js_string() == "pending"
                                && await_rounds < 100
                            {
                                let fired = self.event_loop.tick(1);
                                if fired.is_empty() {
                                    break; // No timers can advance this further
                                }
                                for task in fired {
                                    self.call_value(
                                        &task.callback,
                                        &task.args,
                                        JsValue::Undefined,
                                    );
                                }
                                self.drain_microtasks();
                                await_rounds += 1;
                            }

                            let state = obj.borrow().get("__state").to_js_string();
                            if state == "fulfilled" {
                                let resolved = obj.borrow().get("__value");
                                self.stack.push(resolved);
                            } else if state == "rejected" {
                                let reason = obj.borrow().get("__value");
                                if !self.handle_exception(reason) {
                                    return JsValue::Undefined;
                                }
                            } else {
                                // Still pending after draining — push undefined
                                // (best-effort in single-threaded VM).
                                self.stack.push(JsValue::Undefined);
                            }
                        } else {
                            // Non-promise object: wrap in Promise.resolve semantics.
                            // Check for thenable (.then method).
                            let then_fn = val.get_property("then");
                            if then_fn.is_function() {
                                // It's a thenable — call .then and await
                                let result = self.call_value(
                                    &then_fn,
                                    &[],
                                    val.clone(),
                                );
                                self.drain_microtasks();
                                self.stack.push(result);
                            } else {
                                self.stack.push(val);
                            }
                        }
                    } else {
                        // Non-object value — pass through unchanged.
                        self.stack.push(val);
                    }
                }

                Op::Yield => {
                    let value = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let yield_ip = self.frames[frame_idx].ip;
                    let yield_locals = self.frames[frame_idx].locals.clone();
                    let stack_base = self.frames[frame_idx].stack_base;
                    let yield_stack: Vec<JsValue> = self.stack[stack_base..].to_vec();
                    // Pop the generator frame
                    self.frames.pop();
                    self.stack.truncate(stack_base);
                    // Store pending yield for run_generator_step to consume
                    self.pending_generator_yield =
                        Some((value.clone(), yield_ip, yield_locals, yield_stack));
                    // Push the yield value as the return value of run()
                    self.stack.push(value);
                    return self.stack.pop().unwrap_or(JsValue::Undefined);
                }

                Op::YieldDelegate => {
                    // Simplified: yield* iterable — treat as single yield for now
                    let value = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let yield_ip = self.frames[frame_idx].ip;
                    let yield_locals = self.frames[frame_idx].locals.clone();
                    let stack_base = self.frames[frame_idx].stack_base;
                    let yield_stack: Vec<JsValue> = self.stack[stack_base..].to_vec();
                    self.frames.pop();
                    self.stack.truncate(stack_base);
                    self.pending_generator_yield =
                        Some((value.clone(), yield_ip, yield_locals, yield_stack));
                    self.stack.push(value);
                    return self.stack.pop().unwrap_or(JsValue::Undefined);
                }

                Op::Debugger | Op::Nop => {}

                Op::RequireObjectCoercible => {
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if matches!(val, JsValue::Null | JsValue::Undefined) {
                        let type_str = if matches!(val, JsValue::Null) {
                            "null"
                        } else {
                            "undefined"
                        };
                        let msg =
                            format!("Cannot destructure '{}' as it is {}.", type_str, type_str);
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            return JsValue::Undefined;
                        }
                    }
                }

                Op::ObjectRest(count) => {
                    // Pop `count` excluded key strings, then pop source object.
                    // Push new object with all enumerable own properties except excluded ones.
                    let count = count as usize;
                    let mut excluded: Vec<String> = (0..count)
                        .map(|_| {
                            self.stack
                                .pop()
                                .unwrap_or(JsValue::Undefined)
                                .to_js_string()
                        })
                        .collect();
                    excluded.reverse();
                    let src = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let result = JsValue::new_object();
                    match &src {
                        JsValue::Object(src_rc) => {
                            let keys = src_rc.borrow().keys();
                            for key in keys {
                                if !excluded.contains(&key) {
                                    let val = src_rc.borrow().get(&key);
                                    result.set_property(key, val);
                                }
                            }
                        }
                        JsValue::String(s) => {
                            // Spread string characters as indexed properties
                            for (i, ch) in s.chars().enumerate() {
                                let key = format!("{}", i);
                                if !excluded.contains(&key) {
                                    let mut cs = String::new();
                                    cs.push(ch);
                                    result.set_property(key, JsValue::String(cs));
                                }
                            }
                        }
                        JsValue::Array(arr) => {
                            let a = arr.borrow();
                            for (&i, elem) in a.elements.iter() {
                                let key = format!("{}", i);
                                if !excluded.contains(&key) {
                                    result.set_property(key, elem.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                    self.stack.push(result);
                }

                Op::CloneLocal(slot) => {
                    // Create a fresh Rc<RefCell<JsValue>> cell for this local slot,
                    // copying the current value.  Closures created after this
                    // point capture the new cell (per-iteration let binding).
                    // CloneLocal is only emitted for captured locals in for-loops.
                    let slot = slot as usize;
                    let current_val = self.frames[frame_idx]
                        .locals
                        .get(slot)
                        .map(|s| s.get())
                        .unwrap_or(JsValue::Undefined);
                    let new_cell = Rc::new(RefCell::new(current_val));
                    let frame = &mut self.frames[frame_idx];
                    while frame.locals.len() <= slot {
                        frame.locals.push(LocalSlot::Direct(JsValue::Undefined));
                    }
                    frame.locals[slot] = LocalSlot::Cell(new_cell);
                }
                Op::NewRegExp(pattern_idx, flags_idx) => {
                    let pattern = self.get_const_string(frame_idx, pattern_idx);
                    let flags = self.get_const_string(frame_idx, flags_idx);
                    let regexp = native_regexp::create_regexp_object(self, &pattern, &flags);
                    self.stack.push(regexp);
                }
                Op::DefineGetter(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let getter_fn = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(obj_rc) = &obj {
                        let mut o = obj_rc.borrow_mut();
                        let existing_setter =
                            o.properties.get(&name).and_then(|p| p.setter.clone());
                        o.properties
                            .insert(name, Property::accessor(Some(getter_fn), existing_setter));
                    } else if let JsValue::Function(fn_rc) = &obj {
                        // Static getters on class constructors (which are Functions).
                        let mut f = fn_rc.borrow_mut();
                        f.own_props.insert(name.clone(), JsValue::Undefined); // placeholder
                        drop(f);
                        // Store getter in the accessor system via a wrapper object.
                        // For now, use own_props with a special accessor-aware lookup.
                        // Actually: Functions store properties in own_props as plain values.
                        // We need to use the object-like property system.
                        // Workaround: store getter as __get_<name> and patch find_getter.
                        let mut f = fn_rc.borrow_mut();
                        f.own_props.remove(&name);
                        f.own_props
                            .insert(alloc::format!("__get_{}", name), getter_fn);
                        f.own_props.insert(
                            alloc::format!("__desc_enumerable_{}", name),
                            JsValue::Bool(false),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_configurable_{}", name),
                            JsValue::Bool(true),
                        );
                    }
                }
                Op::DefineGetterComputed => {
                    let getter_fn = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let name = key_val.to_js_string();
                    let obj = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(obj_rc) = &obj {
                        let mut o = obj_rc.borrow_mut();
                        let existing_setter =
                            o.properties.get(&name).and_then(|p| p.setter.clone());
                        o.properties
                            .insert(name, Property::accessor(Some(getter_fn), existing_setter));
                    } else if let JsValue::Function(fn_rc) = &obj {
                        let mut f = fn_rc.borrow_mut();
                        f.own_props.remove(&name);
                        f.own_props
                            .insert(alloc::format!("__get_{}", name), getter_fn);
                        f.own_props.insert(
                            alloc::format!("__desc_enumerable_{}", name),
                            JsValue::Bool(false),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_configurable_{}", name),
                            JsValue::Bool(true),
                        );
                    }
                }
                Op::DefineSetter(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let setter_fn = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(obj_rc) = &obj {
                        let mut o = obj_rc.borrow_mut();
                        let existing_getter =
                            o.properties.get(&name).and_then(|p| p.getter.clone());
                        o.properties
                            .insert(name, Property::accessor(existing_getter, Some(setter_fn)));
                    } else if let JsValue::Function(fn_rc) = &obj {
                        let mut f = fn_rc.borrow_mut();
                        f.own_props
                            .insert(alloc::format!("__set_{}", name), setter_fn);
                        f.own_props.insert(
                            alloc::format!("__desc_enumerable_{}", name),
                            JsValue::Bool(false),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_configurable_{}", name),
                            JsValue::Bool(true),
                        );
                    }
                }
                Op::DefineSetterComputed => {
                    let setter_fn = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let name = key_val.to_js_string();
                    let obj = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(obj_rc) = &obj {
                        let mut o = obj_rc.borrow_mut();
                        let existing_getter =
                            o.properties.get(&name).and_then(|p| p.getter.clone());
                        o.properties
                            .insert(name, Property::accessor(existing_getter, Some(setter_fn)));
                    } else if let JsValue::Function(fn_rc) = &obj {
                        let mut f = fn_rc.borrow_mut();
                        f.own_props
                            .insert(alloc::format!("__set_{}", name), setter_fn);
                        f.own_props.insert(
                            alloc::format!("__desc_enumerable_{}", name),
                            JsValue::Bool(false),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_configurable_{}", name),
                            JsValue::Bool(true),
                        );
                    }
                }
                Op::DefineMethod(name_idx) => {
                    // Like SetPropNamed but non-enumerable (for class methods per ES2023 §14.5.14).
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    if let JsValue::Object(obj_rc) = &obj {
                        let mut o = obj_rc.borrow_mut();
                        o.properties.insert(
                            name,
                            Property {
                                value: val.clone(),
                                writable: true,
                                enumerable: false,
                                configurable: true,
                                getter: None,
                                setter: None,
                            },
                        );
                    } else if let JsValue::Function(fn_rc) = &obj {
                        // Static methods on class constructors.
                        let mut f = fn_rc.borrow_mut();
                        f.own_props.insert(name.clone(), val.clone());
                        f.own_props.insert(
                            alloc::format!("__desc_writable_{}", name),
                            JsValue::Bool(true),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_enumerable_{}", name),
                            JsValue::Bool(false),
                        );
                        f.own_props.insert(
                            alloc::format!("__desc_configurable_{}", name),
                            JsValue::Bool(true),
                        );
                    }
                    self.stack.push(val);
                }
            }
        }
    }

    // ── Helpers ──

    pub fn load_constant(&self, frame_idx: usize, idx: u16) -> JsValue {
        match &self.frames[frame_idx].chunk.constants[idx as usize] {
            Constant::Number(n) => JsValue::Number(*n),
            Constant::String(s) => JsValue::String(s.clone()),
            Constant::BigInt(s) => {
                JsValue::BigInt(JsBigInt::from_str(s).unwrap_or_else(JsBigInt::zero))
            }
            Constant::Function(chunk) => {
                let param_stubs: Vec<alloc::string::String> = (0..chunk.param_count)
                    .map(|_| alloc::string::String::new())
                    .collect();
                let func = JsFunction {
                    name: chunk.name.clone(),
                    params: param_stubs,
                    kind: FnKind::Bytecode(chunk.clone()),
                    this_binding: None,
                    bound_args: Vec::new(),
                    upvalues: Vec::new(),
                    prototype: None,
                    own_props: BTreeMap::new(),
                    arity: None,
                    super_class: None,
                };
                JsValue::Function(Rc::new(RefCell::new(func)))
            }
        }
    }

    pub fn get_const_string(&self, frame_idx: usize, idx: u16) -> String {
        match &self.frames[frame_idx].chunk.constants[idx as usize] {
            Constant::String(s) => s.clone(),
            Constant::Number(n) => format_number(*n),
            _ => String::new(),
        }
    }

    /// Get a property value, automatically invoking getters if the property is an accessor.
    /// Unlike `get_property_with_proto` which returns the getter function, this method
    /// calls the getter and returns its result. Exceptions from getters are propagated
    /// via `pending_exception` / `last_exception`.
    pub fn get_property_invoking_getter(&mut self, obj: &JsValue, key: &str) -> JsValue {
        if let JsValue::Object(o) = obj {
            if o.borrow().internal_tag.as_deref() == Some(native_proxy::PROXY_TAG) {
                return native_proxy::proxy_get(self, obj, key).unwrap_or(JsValue::Undefined);
            }
        }
        if let Some(getter) = self.find_getter(obj, key) {
            let result = self.call_value(&getter, &[], obj.clone());
            // Propagate unhandled exceptions from getter call
            if let Some(exc) = self.last_exception.take() {
                self.pending_exception = Some(exc);
            }
            result
        } else {
            self.get_property_with_proto(obj, key)
        }
    }

    fn has_private_member(&self, val: &JsValue, key: &str) -> bool {
        match val {
            JsValue::Object(obj) => obj.borrow().has(key),
            JsValue::Function(fn_rc) => {
                let f = fn_rc.borrow();
                f.own_props.contains_key(key)
                    || f.own_props.contains_key(&alloc::format!("__get_{}", key))
                    || f.own_props.contains_key(&alloc::format!("__set_{}", key))
            }
            _ => false,
        }
    }

    /// Get property with prototype chain lookup.
    /// Check for a getter in an object's property and return it (without invoking).
    /// Returns `Some(getter_fn)` if found, `None` if it's a data property or not found.
    pub fn find_getter(&self, val: &JsValue, key: &str) -> Option<JsValue> {
        if let JsValue::Object(obj) = val {
            let o = obj.borrow();
            if let Some(prop) = o.properties.get(key) {
                if prop.is_accessor() {
                    return prop.getter.clone();
                }
                return None; // data property found, no getter
            }
            // Walk prototype chain
            if let Some(ref proto) = o.prototype {
                let proto_val = JsValue::Object(proto.clone());
                drop(o);
                return self.find_getter(&proto_val, key);
            }
        }
        if let JsValue::Array(arr) = val {
            let a = arr.borrow();
            if let Some(prop) = a.properties.get(key) {
                if prop.is_accessor() {
                    return prop.getter.clone();
                }
                return None;
            }
            drop(a);
            let proto_val = JsValue::Object(self.array_proto.clone());
            return self.find_getter(&proto_val, key);
        }
        // Static getters on Function objects (class constructors).
        if let JsValue::Function(fn_rc) = val {
            let f = fn_rc.borrow();
            let getter_key = alloc::format!("__get_{}", key);
            if let Some(getter) = f.own_props.get(&getter_key) {
                return Some(getter.clone());
            }
            // Walk to function_proto for inherited accessor properties
            // (e.g. "caller" and "arguments" poison-pill getters)
            drop(f);
            let proto_val = JsValue::Object(self.function_proto.clone());
            return self.find_getter(&proto_val, key);
        }
        None
    }

    /// Check for a setter in an object's property chain.
    /// Check if the prototype chain has a read-only data property or
    /// an accessor property without a setter for the given key.
    fn proto_has_readonly_or_setter_undef(&self, val: &JsValue, key: &str) -> bool {
        if let JsValue::Object(obj) = val {
            let o = obj.borrow();
            if let Some(prop) = o.properties.get(key) {
                if prop.is_accessor() && prop.setter.is_none() {
                    return true;
                }
                if !prop.is_accessor() && !prop.writable {
                    return true;
                }
                return false;
            }
            if let Some(ref proto) = o.prototype {
                let proto_val = JsValue::Object(proto.clone());
                drop(o);
                return self.proto_has_readonly_or_setter_undef(&proto_val, key);
            }
        }
        false
    }

    pub fn find_setter(&self, val: &JsValue, key: &str) -> Option<JsValue> {
        if let JsValue::Object(obj) = val {
            let o = obj.borrow();
            if let Some(prop) = o.properties.get(key) {
                if prop.is_accessor() {
                    return prop.setter.clone();
                }
                return None;
            }
            if let Some(ref proto) = o.prototype {
                let proto_val = JsValue::Object(proto.clone());
                drop(o);
                return self.find_setter(&proto_val, key);
            }
        }
        if let JsValue::Array(arr) = val {
            let a = arr.borrow();
            if let Some(prop) = a.properties.get(key) {
                if prop.is_accessor() {
                    return prop.setter.clone();
                }
                return None;
            }
            drop(a);
            let proto_val = JsValue::Object(self.array_proto.clone());
            return self.find_setter(&proto_val, key);
        }
        // Static setters on Function objects (class constructors).
        if let JsValue::Function(fn_rc) = val {
            let f = fn_rc.borrow();
            let setter_key = alloc::format!("__set_{}", key);
            if let Some(setter) = f.own_props.get(&setter_key) {
                return Some(setter.clone());
            }
            // Walk to function_proto for inherited accessor properties
            drop(f);
            let proto_val = JsValue::Object(self.function_proto.clone());
            return self.find_setter(&proto_val, key);
        }
        None
    }

    pub fn get_property_with_proto(&self, val: &JsValue, key: &str) -> JsValue {
        match val {
            JsValue::Object(obj) => {
                let o = obj.borrow();
                if let Some(prop) = o.properties.get(key) {
                    if prop.is_accessor() {
                        // Getter needs to be invoked by caller (VM run loop)
                        // Return undefined here; the VM will detect and call the getter
                        return prop.getter.as_ref().cloned().unwrap_or(JsValue::Undefined);
                    }
                    return prop.value.clone();
                }
                if let Some(prim) = &o.primitive_value {
                    if let JsValue::String(s) = prim.as_ref() {
                        if key == "length" {
                            return JsValue::Number(s.chars().count() as f64);
                        }
                        if let Some(idx) = try_parse_index(key) {
                            if let Some(ch) = s.chars().nth(idx) {
                                let mut buf = String::new();
                                buf.push(ch);
                                return JsValue::String(buf);
                            }
                            return JsValue::Undefined;
                        }
                    }
                }
                if let Some(ref proto) = o.prototype {
                    let proto_rc = proto.clone();
                    drop(o);
                    return get_proto_prop_rc(&proto_rc, key);
                }
                drop(o);
                get_proto_prop_rc(&self.object_proto, key)
            }
            JsValue::Array(arr) => {
                let a = arr.borrow();
                if key == "length" {
                    return JsValue::Number(a.len() as f64);
                }
                if let Some(idx) = try_parse_index(key) {
                    if a.has(idx) {
                        return a.get(idx);
                    }
                    drop(a);
                    return get_proto_prop_rc(&self.array_proto, key);
                }
                if let Some(prop) = a.properties.get(key) {
                    if prop.is_accessor() {
                        return prop.getter.as_ref().cloned().unwrap_or(JsValue::Undefined);
                    }
                    return prop.value.clone();
                }
                drop(a);
                get_proto_prop_rc(&self.array_proto, key)
            }
            JsValue::String(s) => {
                if key == "length" {
                    return JsValue::Number(s.chars().count() as f64);
                }
                if let Some(idx) = try_parse_index(key) {
                    if let Some(ch) = s.chars().nth(idx) {
                        let mut buf = String::new();
                        buf.push(ch);
                        return JsValue::String(buf);
                    }
                }
                get_proto_prop_rc(&self.string_proto, key)
            }
            JsValue::Number(_) => get_proto_prop_rc(&self.number_proto, key),
            JsValue::Bool(_) => get_proto_prop_rc(&self.boolean_proto, key),
            JsValue::Function(f) => {
                let func = f.borrow();
                let constructable = match &func.kind {
                    FnKind::Bytecode(chunk) => !chunk.is_arrow && !chunk.is_generator,
                    FnKind::Native(_) => matches!(
                        func.own_props.get("__constructable__"),
                        Some(JsValue::Bool(true))
                    ),
                };
                let builtin_deleted = |name: &str| {
                    matches!(
                        func.own_props.get(&alloc::format!("__deleted_builtin_{}", name)),
                        Some(JsValue::Bool(true))
                    )
                };
                // Check own properties first (static methods etc.)
                if let Some(v) = func.own_props.get(key) {
                    return v.clone();
                }
                if key == "name" && !builtin_deleted("name") {
                    return func
                        .name
                        .as_ref()
                        .map(|n| JsValue::String(n.clone()))
                        .unwrap_or(JsValue::String(String::new()));
                }
                if key == "length" && !builtin_deleted("length") {
                    let len = func.arity.unwrap_or(func.params.len());
                    return JsValue::Number(len as f64);
                }
                if key == "prototype" && !builtin_deleted("prototype") {
                    if !constructable {
                        return JsValue::Undefined;
                    }
                    // Arrow functions have no .prototype (ES2023 §14.2.17)
                    if func.kind.is_arrow() {
                        return JsValue::Undefined;
                    }
                    // Bound functions have no own .prototype (ES2023 §10.4.1.3)
                    if func.this_binding.is_some() && func.prototype.is_none() {
                        drop(func);
                        return get_proto_prop_rc(&self.function_proto, key);
                    }
                    // Return the stored prototype object (shared across new calls).
                    if let Some(ref proto) = func.prototype {
                        return JsValue::Object(proto.clone());
                    }
                    // Create and cache a new prototype on first access.
                    // Set constructor back to the function (ES2023 §10.2.4).
                    drop(func);
                    let proto = Rc::new(RefCell::new(JsObject::new()));
                    proto.borrow_mut().prototype = Some(self.object_proto.clone());
                    proto
                        .borrow_mut()
                        .set(String::from("constructor"), JsValue::Function(f.clone()));
                    f.borrow_mut().prototype = Some(proto.clone());
                    return JsValue::Object(proto);
                }
                drop(func);
                get_proto_prop_rc(&self.function_proto, key)
            }
            JsValue::BigInt(_) => val.get_property(key),
            _ => JsValue::Undefined,
        }
    }

    /// ES Abstract ToPrimitive(val, hint) — converts objects to primitives.
    /// Returns the primitive value, or Undefined if an exception was set in pending_exception.
    pub fn to_primitive_for_op(&mut self, val: JsValue, hint: &str) -> JsValue {
        // Already a primitive?
        if !matches!(
            val,
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
        ) {
            return val;
        }

        // Date objects override ToPrimitive: "default" hint becomes "string"
        // (per ES2023 §21.4.4.45 Date.prototype[@@toPrimitive])
        let effective_hint = if hint == "default" {
            if let JsValue::Object(obj) = &val {
                if obj.borrow().internal_tag.as_deref() == Some("__date__") {
                    "string"
                } else {
                    hint
                }
            } else {
                hint
            }
        } else {
            hint
        };

        // Check for Symbol.toPrimitive method
        // get_property_with_proto returns the getter function for accessor properties,
        // so we need to check if the property is an accessor and invoke the getter.
        let sym_to_prim_key = native_symbol::WELL_KNOWN_TO_PRIMITIVE;
        let raw_prop = self.get_property_with_proto(&val, sym_to_prim_key);
        let to_prim_fn = if matches!(raw_prop, JsValue::Function(_)) {
            // Could be a getter or the actual toPrimitive function.
            // Check if the property is defined as an accessor on the object.
            let is_getter = if let JsValue::Object(obj) = &val {
                let o = obj.borrow();
                o.properties
                    .get(sym_to_prim_key)
                    .map(|p| p.is_accessor())
                    .unwrap_or(false)
            } else {
                false
            };
            if is_getter {
                // Invoke the getter to get the actual toPrimitive value
                let getter_result = self.call_value(&raw_prop, &[], val.clone());
                if let Some(exc) = self.last_exception.take() {
                    self.pending_exception = Some(exc);
                    return JsValue::Undefined;
                }
                if self.pending_exception.is_some() {
                    return JsValue::Undefined;
                }
                getter_result
            } else {
                raw_prop
            }
        } else {
            raw_prop
        };
        if matches!(to_prim_fn, JsValue::Function(_)) {
            let hint_val = JsValue::String(alloc::string::String::from(effective_hint));
            let result = self.call_value(&to_prim_fn, &[hint_val], val.clone());
            if self.pending_exception.is_some() {
                return JsValue::Undefined;
            }
            if let Some(exc) = self.last_exception.take() {
                self.pending_exception = Some(exc);
                return JsValue::Undefined;
            }
            // If result is still an object, throw TypeError
            if matches!(
                result,
                JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
            ) {
                let err = self.make_type_error("Cannot convert object to primitive value");
                self.pending_exception = Some(err);
                return JsValue::Undefined;
            }
            return result;
        } else if !matches!(to_prim_fn, JsValue::Undefined | JsValue::Null) {
            let err = self.make_type_error("@@toPrimitive is not callable");
            self.pending_exception = Some(err);
            return JsValue::Undefined;
        }

        // No Symbol.toPrimitive — use valueOf/toString
        // "string" hint: toString first, then valueOf
        // "number" and "default" hints: valueOf first, then toString
        let methods: &[&str] = if effective_hint == "string" {
            &["toString", "valueOf"]
        } else {
            &["valueOf", "toString"]
        };

        for &method_name in methods {
            let method = self.get_property_with_proto(&val, method_name);
            if matches!(method, JsValue::Function(_)) {
                let result = self.call_value(&method, &[], val.clone());
                if self.pending_exception.is_some() {
                    return JsValue::Undefined;
                }
                // If the called function threw (unhandled → last_exception),
                // propagate it as pending_exception for the caller to handle.
                if let Some(exc) = self.last_exception.take() {
                    self.pending_exception = Some(exc);
                    return JsValue::Undefined;
                }
                // Primitive result — use it
                if !matches!(
                    result,
                    JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_)
                ) {
                    return result;
                }
                // Object/Function result — try next method
            }
        }

        // Both methods failed to return a primitive
        let err = self.make_type_error("Cannot convert object to primitive value");
        self.pending_exception = Some(err);
        JsValue::Undefined
    }

    pub fn op_add(&self, a: &JsValue, b: &JsValue) -> JsValue {
        match (a, b) {
            (JsValue::String(sa), _) => {
                let mut result = sa.clone();
                result.push_str(&b.to_js_string());
                JsValue::String(result)
            }
            (_, JsValue::String(sb)) => {
                let mut result = a.to_js_string();
                result.push_str(sb);
                JsValue::String(result)
            }
            _ => JsValue::Number(a.to_number() + b.to_number()),
        }
    }

    fn binary_num_op(&mut self, f: fn(f64, f64) -> f64) {
        let b = self.stack.pop().unwrap_or(JsValue::Undefined);
        let a = self.stack.pop().unwrap_or(JsValue::Undefined);
        // BigInt operands not supported for this op (e.g. **)
        if matches!((&a, &b), (JsValue::BigInt(_), _) | (_, JsValue::BigInt(_))) {
            let err = self.make_type_error(
                "Cannot mix BigInt and other types, use explicit conversions",
            );
            self.handle_exception(err);
            return;
        }
        self.stack.push(JsValue::Number(f(a.to_number(), b.to_number())));
    }

    fn binary_num_or_bigint_op(
        &mut self,
        num_f: fn(f64, f64) -> f64,
        big_f: fn(&JsBigInt, &JsBigInt) -> JsBigInt,
    ) {
        let b = self.stack.pop().unwrap_or(JsValue::Undefined);
        let a = self.stack.pop().unwrap_or(JsValue::Undefined);
        match (&a, &b) {
            (JsValue::BigInt(ba), JsValue::BigInt(bb)) => {
                self.stack.push(JsValue::BigInt(big_f(ba, bb)));
            }
            (JsValue::BigInt(_), _) | (_, JsValue::BigInt(_)) => {
                let err = self.make_type_error(
                    "Cannot mix BigInt and other types, use explicit conversions",
                );
                self.handle_exception(err);
            }
            _ => {
                self.stack
                    .push(JsValue::Number(num_f(a.to_number(), b.to_number())));
            }
        }
    }

    fn binary_int_op(&mut self, f: fn(i32, i32) -> i32) {
        let b = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as i32;
        let a = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as i32;
        self.stack.push(JsValue::Number(f(a, b) as f64));
    }

    fn compare_op(&mut self, f: fn(f64, f64) -> bool) {
        let b = self.stack.pop().unwrap_or(JsValue::Undefined);
        let a = self.stack.pop().unwrap_or(JsValue::Undefined);
        if let (JsValue::String(sa), JsValue::String(sb)) = (&a, &b) {
            let cmp = if *sa < *sb {
                -1.0
            } else if *sa > *sb {
                1.0
            } else {
                0.0
            };
            self.stack.push(JsValue::Bool(f(cmp, 0.0)));
        } else {
            self.stack
                .push(JsValue::Bool(f(a.to_number(), b.to_number())));
        }
    }

    fn handle_exception(&mut self, val: JsValue) -> bool {
        // Log boolean/null/number exceptions — unusual and likely a bug indicator
        if matches!(&val, JsValue::Bool(_) | JsValue::Null) {
            let stack_info = self.frame_stack_summary(8);
            self.log_engine(&format!(
                "[libjs] UNUSUAL exception: {:?} [{}]",
                val, stack_info
            ));
        }
        // Only use try handlers that belong to the current run scope.
        // Handlers with frame_depth <= run_target_depth belong to a parent
        // call_value context and must not catch exceptions from this scope
        // (e.g. valueOf/toString called during ToPrimitive should not be
        // caught by an outer try/catch).
        if let Some(handler) = self.try_handlers.last() {
            if handler.frame_depth > self.run_target_depth {
                let handler = self.try_handlers.pop().unwrap();
                self.close_iterators_in_stack_range(handler.stack_depth);
                self.stack.truncate(handler.stack_depth);
                while self.frames.len() > handler.frame_depth {
                    self.frames.pop();
                }
                if let Some(frame) = self.frames.last_mut() {
                    frame.ip = handler.catch_ip;
                }
                self.stack.push(val);
                return true;
            }
        }
        let detail = self.describe_exception(&val);
        let stack_info = self.frame_stack_summary(8);
        if let Some(frame) = self.frames.last() {
            self.close_iterators_in_stack_range(frame.stack_base);
        }
        self.log_engine(&format!(
            "[libjs] WARN: unhandled exception: {} [{}]",
            detail, stack_info
        ));
        self.last_exception = Some(val);
        false
    }

    /// Run a generator function's bytecode from a saved state until Yield or Return.
    /// Uses the main `run()` loop by pushing a frame and using `run_target_depth`.
    pub fn run_generator_step(
        &mut self,
        chunk: Chunk,
        start_ip: usize,
        locals: Vec<LocalSlot>,
        upvalue_cells: Vec<Rc<RefCell<JsValue>>>,
        this_val: JsValue,
        stack_snapshot: Vec<JsValue>,
        send_value: JsValue,
    ) -> native_generator::GeneratorResult {
        // Push the send_value onto the snapshot stack (result of `yield` expression)
        let stack_base = self.stack.len();
        for v in &stack_snapshot {
            self.stack.push(v.clone());
        }
        if start_ip > 0 {
            // Not the first call — push the sent value as the yield expression result
            self.stack.push(send_value);
        }

        let frame = CallFrame {
            chunk,
            ip: start_ip,
            stack_base,
            locals,
            upvalue_cells,
            this_val,
            is_constructor: false,
            all_args: Vec::new(),
            self_ref: JsValue::Undefined,
        };
        let frame_depth = self.frames.len();
        self.frames.push(frame);

        // Use the main run loop
        let saved_target = self.run_target_depth;
        self.run_target_depth = frame_depth;
        let result = self.run();
        self.run_target_depth = saved_target;

        // Check if we suspended on a Yield (indicated by the generator_yield_* fields)
        if let Some((yield_val, yield_ip, yield_locals, yield_stack)) =
            self.pending_generator_yield.take()
        {
            return native_generator::GeneratorResult::Yielded {
                value: yield_val,
                ip: yield_ip,
                locals: yield_locals,
                stack: yield_stack,
            };
        }

        if let Some(exc) = self.pending_exception.take() {
            self.stack.truncate(stack_base);
            return native_generator::GeneratorResult::Threw(exc);
        }

        if let Some(exc) = self.last_exception.take() {
            self.stack.truncate(stack_base);
            return native_generator::GeneratorResult::Threw(exc);
        }

        // Normal return or exception
        self.stack.truncate(stack_base);
        native_generator::GeneratorResult::Returned(result)
    }
}

// ── Free functions ──

/// Returns true if the value is a JavaScript Symbol (stored as a special prefixed string).
pub fn is_symbol_value(val: &JsValue) -> bool {
    matches!(val, JsValue::String(s) if s.starts_with("__symbol__") || s.starts_with("__symbol_global__"))
}

/// Walk prototype chain (free function to avoid borrow conflicts on Vm).
pub fn get_proto_prop_rc(proto: &Rc<RefCell<JsObject>>, key: &str) -> JsValue {
    let p = proto.borrow();
    if let Some(prop) = p.properties.get(key) {
        if prop.is_accessor() {
            return prop.getter.as_ref().cloned().unwrap_or(JsValue::Undefined);
        }
        return prop.value.clone();
    }
    if let Some(ref parent) = p.prototype {
        let parent_clone = parent.clone();
        drop(p);
        return get_proto_prop_rc(&parent_clone, key);
    }
    JsValue::Undefined
}

pub fn try_parse_index(s: &str) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

/// Helper to create a native JsValue::Function.
pub fn native_fn(name: &str, f: fn(&mut Vm, &[JsValue]) -> JsValue) -> JsValue {
    JsValue::Function(Rc::new(RefCell::new(JsFunction {
        name: Some(String::from(name)),
        params: Vec::new(),
        kind: FnKind::Native(f),
        this_binding: None,
        bound_args: Vec::new(),
        upvalues: Vec::new(),
        prototype: None,
        own_props: BTreeMap::new(),
        arity: None,
        super_class: None,
    })))
}

/// Create a native function that may be used as a constructor via `new`.
pub fn native_ctor_fn(name: &str, f: fn(&mut Vm, &[JsValue]) -> JsValue) -> JsValue {
    let func = native_fn(name, f);
    if let JsValue::Function(f) = &func {
        f.borrow_mut()
            .own_props
            .insert(String::from("__constructable__"), JsValue::Bool(true));
    }
    func
}

/// Create a native function with an explicit `.length` (arity) property.
pub fn native_fn_with_length(
    name: &str,
    f: fn(&mut Vm, &[JsValue]) -> JsValue,
    length: usize,
) -> JsValue {
    JsValue::Function(Rc::new(RefCell::new(JsFunction {
        name: Some(String::from(name)),
        params: Vec::new(),
        kind: FnKind::Native(f),
        this_binding: None,
        bound_args: Vec::new(),
        upvalues: Vec::new(),
        prototype: None,
        own_props: BTreeMap::new(),
        arity: Some(length),
        super_class: None,
    })))
}

pub fn native_ctor_fn_with_length(
    name: &str,
    f: fn(&mut Vm, &[JsValue]) -> JsValue,
    length: usize,
) -> JsValue {
    let func = native_fn_with_length(name, f, length);
    if let JsValue::Function(f) = &func {
        f.borrow_mut()
            .own_props
            .insert(String::from("__constructable__"), JsValue::Bool(true));
    }
    func
}
