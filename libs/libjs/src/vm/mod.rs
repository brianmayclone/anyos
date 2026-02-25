//! JavaScript virtual machine — executes bytecode.
//!
//! Stack-based VM with prototype chain support, closures,
//! reference-semantics (Rc<RefCell>) and ECMAScript-compatible semantics.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use alloc::format;

use core::cell::RefCell;

use crate::bytecode::{Chunk, Constant, Op};
use crate::value::*;

pub mod call;
pub mod builtins;
pub mod native_array;
pub mod native_string;
pub mod native_object;
pub mod native_number;
pub mod native_function;
pub mod native_console;
pub mod native_error;
pub mod native_globals;
pub mod native_math;
pub mod native_json;
pub mod native_promise;
pub mod native_map;
pub mod native_date;
pub mod native_timer;
pub mod iter;

// ── Internal structures ──

/// Call frame for function invocations.
pub struct CallFrame {
    pub chunk: Chunk,
    pub ip: usize,
    pub stack_base: usize,
    pub locals: Vec<JsValue>,
    pub this_val: JsValue,
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
    pub globals: JsObject,
    pub try_handlers: Vec<TryHandler>,
    pub console_output: Vec<String>,
    pub engine_log: Vec<String>,
    pub object_proto: Rc<RefCell<JsObject>>,
    pub array_proto: Rc<RefCell<JsObject>>,
    pub string_proto: Rc<RefCell<JsObject>>,
    pub function_proto: Rc<RefCell<JsObject>>,
    pub number_proto: Rc<RefCell<JsObject>>,
    pub error_proto: Rc<RefCell<JsObject>>,
    pub step_limit: u64,
    pub steps: u64,
    pub userdata: *mut u8,
    /// Current `this` binding for the active native call.
    pub current_this: JsValue,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals: JsObject::new(),
            try_handlers: Vec::new(),
            console_output: Vec::new(),
            engine_log: Vec::new(),
            object_proto: Rc::new(RefCell::new(JsObject::new())),
            array_proto: Rc::new(RefCell::new(JsObject::new())),
            string_proto: Rc::new(RefCell::new(JsObject::new())),
            function_proto: Rc::new(RefCell::new(JsObject::new())),
            number_proto: Rc::new(RefCell::new(JsObject::new())),
            error_proto: Rc::new(RefCell::new(JsObject::new())),
            step_limit: 10_000_000,
            steps: 0,
            userdata: core::ptr::null_mut(),
            current_this: JsValue::Undefined,
        };
        vm.init_prototypes();
        vm.init_globals();
        vm.log_engine("[libjs] VM initialized");
        vm
    }

    pub fn set_step_limit(&mut self, limit: u64) {
        self.step_limit = limit;
    }

    pub fn execute(&mut self, chunk: Chunk) -> JsValue {
        self.steps = 0;
        let local_count = chunk.local_count as usize;
        let frame = CallFrame {
            chunk,
            ip: 0,
            stack_base: self.stack.len(),
            locals: vec![JsValue::Undefined; local_count],
            this_val: JsValue::Undefined,
        };
        self.frames.push(frame);
        self.run()
    }

    pub fn set_global(&mut self, name: &str, value: JsValue) {
        self.globals.set(String::from(name), value);
    }

    pub fn get_global(&mut self, name: &str) -> JsValue {
        self.globals.get(name)
    }

    pub fn register_native(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
        let f = JsFunction {
            name: Some(String::from(name)),
            params: Vec::new(),
            kind: FnKind::Native(func),
            this_binding: None,
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

            if self.frames.is_empty() {
                return self.stack.pop().unwrap_or(JsValue::Undefined);
            }

            let frame_idx = self.frames.len() - 1;
            let ip = self.frames[frame_idx].ip;
            if ip >= self.frames[frame_idx].chunk.code.len() {
                if self.frames.len() <= 1 {
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
                Op::LoadUndefined => self.stack.push(JsValue::Undefined),
                Op::LoadNull => self.stack.push(JsValue::Null),
                Op::LoadTrue => self.stack.push(JsValue::Bool(true)),
                Op::LoadFalse => self.stack.push(JsValue::Bool(false)),
                Op::Pop => { self.stack.pop(); }
                Op::Dup => {
                    if let Some(val) = self.stack.last().cloned() {
                        self.stack.push(val);
                    }
                }

                // ── Variables ──
                Op::LoadLocal(slot) => {
                    let val = self.frames[frame_idx].locals
                        .get(slot as usize).cloned()
                        .unwrap_or(JsValue::Undefined);
                    self.stack.push(val);
                }
                Op::StoreLocal(slot) => {
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    let locals = &mut self.frames[frame_idx].locals;
                    while locals.len() <= slot as usize {
                        locals.push(JsValue::Undefined);
                    }
                    locals[slot as usize] = val;
                }
                Op::LoadGlobal(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.globals.get(&name);
                    self.stack.push(val);
                }
                Op::StoreGlobal(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.stack.last().cloned().unwrap_or(JsValue::Undefined);
                    self.globals.set(name, val);
                }
                Op::LoadUpvalue(_) | Op::StoreUpvalue(_) => {
                    // Simplified: treat upvalues as undefined for now
                    self.stack.push(JsValue::Undefined);
                }

                // ── Arithmetic ──
                Op::Add => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(self.op_add(&a, &b));
                }
                Op::Sub => self.binary_num_op(|a, b| a - b),
                Op::Mul => self.binary_num_op(|a, b| a * b),
                Op::Div => self.binary_num_op(|a, b| a / b),
                Op::Mod => self.binary_num_op(|a, b| a % b),
                Op::Exp => self.binary_num_op(|a, b| native_math::pow_f64(a, b)),
                Op::Neg => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(-a.to_number()));
                }
                Op::Pos => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(a.to_number()));
                }

                // ── Bitwise ──
                Op::BitAnd => self.binary_int_op(|a, b| a & b),
                Op::BitOr  => self.binary_int_op(|a, b| a | b),
                Op::BitXor => self.binary_int_op(|a, b| a ^ b),
                Op::BitNot => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number((!(a.to_number() as i32)) as f64));
                }
                Op::Shl  => self.binary_int_op(|a, b| a << (b & 31)),
                Op::Shr  => self.binary_int_op(|a, b| a >> (b & 31)),
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
                    let frame = self.frames.pop().unwrap();
                    self.stack.truncate(frame.stack_base);
                    self.stack.push(val.clone());
                    if self.frames.is_empty() {
                        return val;
                    }
                }
                Op::Closure(idx) => {
                    let chunk = match &self.frames[frame_idx].chunk.constants[idx as usize] {
                        Constant::Function(c) => c.clone(),
                        _ => Chunk::new(),
                    };
                    let func = JsFunction {
                        name: chunk.name.clone(),
                        params: Vec::new(),
                        kind: FnKind::Bytecode(chunk),
                        this_binding: None,
                    };
                    self.stack.push(JsValue::Function(Rc::new(RefCell::new(func))));
                }

                // ── Objects and Properties ──
                Op::GetProp => {
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_str = key.to_js_string();
                    let val = self.get_property_with_proto(&obj, &key_str);
                    self.stack.push(val);
                }
                Op::SetProp => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    obj.set_property(key.to_js_string(), val.clone());
                    self.stack.push(val);
                }
                Op::GetPropNamed(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let val = self.get_property_with_proto(&obj, &name);
                    self.stack.push(val);
                }
                Op::SetPropNamed(name_idx) => {
                    let name = self.get_const_string(frame_idx, name_idx);
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    obj.set_property(name, val.clone());
                    self.stack.push(obj);
                }
                Op::NewObject => {
                    let obj = JsObject {
                        properties: BTreeMap::new(),
                        prototype: Some(self.object_proto.clone()),
                        internal_tag: None,
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

                // ── Special operators ──
                Op::Typeof => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::String(String::from(val.type_of())));
                }
                Op::Void => {
                    self.stack.pop();
                    self.stack.push(JsValue::Undefined);
                }
                Op::Delete => {
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let success = obj.delete_property(&key.to_js_string());
                    self.stack.push(JsValue::Bool(success));
                }
                Op::InstanceOf => {
                    let right = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let left = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let result = self.instance_of(&left, &right);
                    self.stack.push(JsValue::Bool(result));
                }
                Op::In => {
                    let obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_str = key.to_js_string();
                    let result = match &obj {
                        JsValue::Object(o) => o.borrow().has(&key_str),
                        JsValue::Array(a) => {
                            let arr = a.borrow();
                            if let Some(idx) = try_parse_index(&key_str) {
                                idx < arr.elements.len()
                            } else {
                                arr.properties.contains_key(&key_str)
                            }
                        }
                        _ => false,
                    };
                    self.stack.push(JsValue::Bool(result));
                }

                // ── Iteration ──
                Op::GetIterator => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let iter_obj = self.create_iterator(&val);
                    self.stack.push(iter_obj);
                }
                Op::IterNext => {
                    let (value, has_more) = self.iter_next_mut();
                    self.stack.push(value);
                    self.stack.push(JsValue::Bool(has_more));
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
                    self.log_engine(&format!("[libjs] exception thrown: {:?}", val));
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

                // ── Spread ──
                Op::Spread => { /* handled during NewArray */ }

                Op::Debugger | Op::Nop => {}
            }
        }
    }

    // ── Helpers ──

    pub fn load_constant(&self, frame_idx: usize, idx: u16) -> JsValue {
        match &self.frames[frame_idx].chunk.constants[idx as usize] {
            Constant::Number(n) => JsValue::Number(*n),
            Constant::String(s) => JsValue::String(s.clone()),
            Constant::Function(chunk) => {
                let func = JsFunction {
                    name: chunk.name.clone(),
                    params: Vec::new(),
                    kind: FnKind::Bytecode(chunk.clone()),
                    this_binding: None,
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

    /// Get property with prototype chain lookup.
    pub fn get_property_with_proto(&self, val: &JsValue, key: &str) -> JsValue {
        match val {
            JsValue::Object(obj) => {
                let o = obj.borrow();
                if let Some(prop) = o.properties.get(key) {
                    return prop.value.clone();
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
                    return JsValue::Number(a.elements.len() as f64);
                }
                if let Some(idx) = try_parse_index(key) {
                    return a.get(idx);
                }
                if let Some(prop) = a.properties.get(key) {
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
            JsValue::Number(_) => {
                get_proto_prop_rc(&self.number_proto, key)
            }
            JsValue::Function(f) => {
                let func = f.borrow();
                if key == "name" {
                    return func.name.as_ref()
                        .map(|n| JsValue::String(n.clone()))
                        .unwrap_or(JsValue::String(String::new()));
                }
                if key == "length" {
                    return JsValue::Number(func.params.len() as f64);
                }
                if key == "prototype" {
                    return JsValue::Object(Rc::new(RefCell::new(JsObject::new())));
                }
                drop(func);
                get_proto_prop_rc(&self.function_proto, key)
            }
            _ => JsValue::Undefined,
        }
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
        let b = self.stack.pop().unwrap_or(JsValue::Undefined).to_number();
        let a = self.stack.pop().unwrap_or(JsValue::Undefined).to_number();
        self.stack.push(JsValue::Number(f(a, b)));
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
            let cmp = if *sa < *sb { -1.0 } else if *sa > *sb { 1.0 } else { 0.0 };
            self.stack.push(JsValue::Bool(f(cmp, 0.0)));
        } else {
            self.stack.push(JsValue::Bool(f(a.to_number(), b.to_number())));
        }
    }

    fn handle_exception(&mut self, val: JsValue) -> bool {
        if let Some(handler) = self.try_handlers.pop() {
            self.stack.truncate(handler.stack_depth);
            while self.frames.len() > handler.frame_depth {
                self.frames.pop();
            }
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = handler.catch_ip;
            }
            self.stack.push(val);
            true
        } else {
            self.log_engine("[libjs] WARN: unhandled exception");
            false
        }
    }
}

// ── Free functions ──

/// Walk prototype chain (free function to avoid borrow conflicts on Vm).
pub fn get_proto_prop_rc(proto: &Rc<RefCell<JsObject>>, key: &str) -> JsValue {
    let p = proto.borrow();
    if let Some(prop) = p.properties.get(key) {
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
    if s.is_empty() { return None; }
    let mut n: usize = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
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
    })))
}
