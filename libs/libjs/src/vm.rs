//! JavaScript virtual machine — executes bytecode.
//!
//! Stack-based VM with prototype chain support, closures,
//! and ECMAScript-compatible semantics.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use alloc::format;
use alloc::string::ToString;

use crate::bytecode::{Chunk, Constant, Op};
use crate::value::{JsValue, JsObject, JsArray, JsFunction, FnKind, Property};

/// Call frame for function invocations.
struct CallFrame {
    chunk: Chunk,
    ip: usize,
    stack_base: usize,
    locals: Vec<JsValue>,
    this_val: JsValue,
}

/// Exception handler for try-catch.
struct TryHandler {
    catch_ip: usize,
    stack_depth: usize,
    frame_depth: usize,
}

/// The JavaScript virtual machine.
pub struct Vm {
    /// Value stack.
    stack: Vec<JsValue>,
    /// Call frames.
    frames: Vec<CallFrame>,
    /// Global object (global scope).
    globals: JsObject,
    /// Exception handlers.
    try_handlers: Vec<TryHandler>,
    /// Console output buffer.
    pub console_output: Vec<String>,
    /// Built-in prototypes.
    pub object_proto: Box<JsObject>,
    pub array_proto: Box<JsObject>,
    pub string_proto: Box<JsObject>,
    pub function_proto: Box<JsObject>,
    pub number_proto: Box<JsObject>,
    /// Maximum execution steps (to prevent infinite loops).
    step_limit: u64,
    steps: u64,
    /// Opaque userdata pointer for host integration (e.g., DOM bridge).
    /// The host sets this before calling into the VM; native functions read it.
    pub userdata: *mut u8,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Vm {
            stack: Vec::with_capacity(256),
            frames: Vec::new(),
            globals: JsObject::new(),
            try_handlers: Vec::new(),
            console_output: Vec::new(),
            object_proto: Box::new(JsObject::new()),
            array_proto: Box::new(JsObject::new()),
            string_proto: Box::new(JsObject::new()),
            function_proto: Box::new(JsObject::new()),
            number_proto: Box::new(JsObject::new()),
            step_limit: 10_000_000,
            steps: 0,
            userdata: core::ptr::null_mut(),
        };
        vm.init_prototypes();
        vm.init_globals();
        vm
    }

    /// Set the maximum number of execution steps.
    pub fn set_step_limit(&mut self, limit: u64) {
        self.step_limit = limit;
    }

    /// Execute a compiled chunk and return the result.
    pub fn execute(&mut self, chunk: Chunk) -> JsValue {
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

    /// Set a global variable.
    pub fn set_global(&mut self, name: &str, value: JsValue) {
        self.globals.set(String::from(name), value);
    }

    /// Get a global variable.
    pub fn get_global(&mut self, name: &str) -> JsValue {
        self.globals.get(name)
    }

    /// Register a native function as a global.
    pub fn register_native(&mut self, name: &str, func: fn(&mut Vm, &[JsValue]) -> JsValue) {
        let f = JsFunction {
            name: Some(String::from(name)),
            params: Vec::new(),
            kind: FnKind::Native(func),
            this_binding: None,
        };
        self.set_global(name, JsValue::Function(Box::new(f)));
    }

    // ── VM Execution Loop ──

    fn run(&mut self) -> JsValue {
        loop {
            self.steps += 1;
            if self.steps > self.step_limit {
                return JsValue::Undefined; // execution limit reached
            }

            let frame_idx = self.frames.len() - 1;
            let ip = self.frames[frame_idx].ip;
            if ip >= self.frames[frame_idx].chunk.code.len() {
                // End of code — implicit return
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

                // Variables
                Op::LoadLocal(slot) => {
                    let val = self.frames[frame_idx].locals
                        .get(slot as usize)
                        .cloned()
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
                    // Simplified: treat upvalues as globals for now
                    self.stack.push(JsValue::Undefined);
                }

                // Arithmetic
                Op::Add => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(self.op_add(&a, &b));
                }
                Op::Sub => { self.binary_num_op(|a, b| a - b); }
                Op::Mul => { self.binary_num_op(|a, b| a * b); }
                Op::Div => { self.binary_num_op(|a, b| a / b); }
                Op::Mod => { self.binary_num_op(|a, b| a % b); }
                Op::Exp => { self.binary_num_op(|a, b| pow_f64(a, b)); }
                Op::Neg => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(-a.to_number()));
                }
                Op::Pos => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(a.to_number()));
                }

                // Bitwise
                Op::BitAnd => { self.binary_int_op(|a, b| a & b); }
                Op::BitOr => { self.binary_int_op(|a, b| a | b); }
                Op::BitXor => { self.binary_int_op(|a, b| a ^ b); }
                Op::BitNot => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let n = a.to_number() as i32;
                    self.stack.push(JsValue::Number((!n) as f64));
                }
                Op::Shl => { self.binary_int_op(|a, b| a << (b & 31)); }
                Op::Shr => { self.binary_int_op(|a, b| a >> (b & 31)); }
                Op::UShr => {
                    let b = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as u32;
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined).to_number() as u32;
                    self.stack.push(JsValue::Number((a >> (b & 31)) as f64));
                }

                // Comparison
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
                Op::Lt => { self.compare_op(|a, b| a < b); }
                Op::Le => { self.compare_op(|a, b| a <= b); }
                Op::Gt => { self.compare_op(|a, b| a > b); }
                Op::Ge => { self.compare_op(|a, b| a >= b); }

                // Logical
                Op::Not => {
                    let a = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Bool(!a.to_boolean()));
                }

                // Control flow
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
                    let val = self.stack.last().unwrap_or(&JsValue::Undefined);
                    if val.is_nullish() {
                        let ip = self.frames[frame_idx].ip as i32 + offset;
                        self.frames[frame_idx].ip = ip as usize;
                    }
                }

                // Functions
                Op::Call(argc) => {
                    self.call_function(argc as usize);
                }
                Op::Return => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let frame = self.frames.pop().unwrap();
                    // Clean up stack
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
                    self.stack.push(JsValue::Function(Box::new(func)));
                }

                // Objects
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
                    let mut obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_str = key.to_js_string();
                    obj.set_property(key_str, val.clone());
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
                    let mut obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    obj.set_property(name, val.clone());
                    // Push the object back (it was consumed)
                    self.stack.push(obj);
                }
                Op::NewObject => {
                    self.stack.push(JsValue::Object(Box::new(JsObject {
                        properties: BTreeMap::new(),
                        prototype: Some(self.object_proto.clone()),
                    })));
                }
                Op::NewArray(count) => {
                    let start = if self.stack.len() >= count as usize {
                        self.stack.len() - count as usize
                    } else {
                        0
                    };
                    let elements: Vec<JsValue> = self.stack.drain(start..).collect();
                    let arr = JsArray {
                        elements,
                        properties: BTreeMap::new(),
                    };
                    self.stack.push(JsValue::Array(Box::new(arr)));
                }

                // Constructors
                Op::New(argc) => {
                    self.new_object(argc as usize);
                }

                // Special operators
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
                    let mut obj = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let key_str = key.to_js_string();
                    let success = match &mut obj {
                        JsValue::Object(o) => o.delete(&key_str),
                        _ => true,
                    };
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
                        JsValue::Object(o) => o.has(&key_str),
                        JsValue::Array(a) => {
                            let f = crate::value::parse_js_float(&key_str);
                            if !f.is_nan() && f >= 0.0 {
                                (f as usize) < a.elements.len()
                            } else {
                                a.properties.contains_key(&key_str)
                            }
                        }
                        _ => false,
                    };
                    self.stack.push(JsValue::Bool(result));
                }

                // Iteration
                Op::GetIterator => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    // Create an iterator object
                    let iter = self.create_iterator(&val);
                    self.stack.push(iter);
                }
                Op::IterNext => {
                    // Pop iterator state, push (value, has_more)
                    let iter = self.stack.pop().unwrap_or(JsValue::Undefined);
                    let (value, done) = self.iter_next(&iter);
                    self.stack.push(iter); // put iterator back (might be modified)
                    self.stack.push(value);
                    self.stack.push(JsValue::Bool(!done)); // true = more items
                }

                // Exception handling
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
                    if !self.handle_exception(val.clone()) {
                        // Unhandled exception
                        return JsValue::Undefined;
                    }
                }

                // Inc/Dec
                Op::Inc => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(val.to_number() + 1.0));
                }
                Op::Dec => {
                    let val = self.stack.pop().unwrap_or(JsValue::Undefined);
                    self.stack.push(JsValue::Number(val.to_number() - 1.0));
                }

                // This
                Op::LoadThis => {
                    let this_val = self.frames[frame_idx].this_val.clone();
                    self.stack.push(this_val);
                }

                // Spread
                Op::Spread => {
                    // Leave as-is; arrays already expanded during NewArray
                }

                Op::Debugger | Op::Nop => {}
            }
        }
    }

    // ── Helper Methods ──

    fn load_constant(&self, frame_idx: usize, idx: u16) -> JsValue {
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
                JsValue::Function(Box::new(func))
            }
        }
    }

    fn get_const_string(&self, frame_idx: usize, idx: u16) -> String {
        match &self.frames[frame_idx].chunk.constants[idx as usize] {
            Constant::String(s) => s.clone(),
            Constant::Number(n) => crate::value::format_number(*n),
            _ => String::from(""),
        }
    }

    /// Get property with prototype chain lookup.
    fn get_property_with_proto(&self, val: &JsValue, key: &str) -> JsValue {
        match val {
            JsValue::Object(obj) => {
                if let Some(prop) = obj.properties.get(key) {
                    return prop.value.clone();
                }
                // Walk prototype chain
                if let Some(ref proto) = obj.prototype {
                    return self.get_proto_prop(proto, key);
                }
                // Check Object.prototype
                self.get_proto_prop(&self.object_proto, key)
            }
            JsValue::Array(arr) => {
                if key == "length" {
                    return JsValue::Number(arr.elements.len() as f64);
                }
                // Numeric index
                if let Some(idx) = try_parse_index(key) {
                    return arr.get(idx);
                }
                if let Some(prop) = arr.properties.get(key) {
                    return prop.value.clone();
                }
                // Array prototype
                self.get_proto_prop(&self.array_proto, key)
            }
            JsValue::String(s) => {
                if key == "length" {
                    return JsValue::Number(s.len() as f64);
                }
                if let Some(idx) = try_parse_index(key) {
                    if let Some(ch) = s.chars().nth(idx) {
                        let mut buf = String::new();
                        buf.push(ch);
                        return JsValue::String(buf);
                    }
                }
                // String prototype
                self.get_proto_prop(&self.string_proto, key)
            }
            JsValue::Number(_) => {
                self.get_proto_prop(&self.number_proto, key)
            }
            JsValue::Function(f) => {
                if key == "name" {
                    return f.name.as_ref()
                        .map(|n| JsValue::String(n.clone()))
                        .unwrap_or(JsValue::String(String::new()));
                }
                if key == "prototype" {
                    return JsValue::Object(Box::new(JsObject::new()));
                }
                self.get_proto_prop(&self.function_proto, key)
            }
            _ => JsValue::Undefined,
        }
    }

    fn get_proto_prop(&self, proto: &JsObject, key: &str) -> JsValue {
        if let Some(prop) = proto.properties.get(key) {
            return prop.value.clone();
        }
        if let Some(ref parent) = proto.prototype {
            return self.get_proto_prop(parent, key);
        }
        JsValue::Undefined
    }

    fn op_add(&self, a: &JsValue, b: &JsValue) -> JsValue {
        // String concatenation if either operand is a string
        match (a, b) {
            (JsValue::String(sa), _) => {
                let sb = b.to_js_string();
                let mut result = sa.clone();
                result.push_str(&sb);
                JsValue::String(result)
            }
            (_, JsValue::String(sb)) => {
                let sa = a.to_js_string();
                let mut result = sa;
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
        // String comparison
        if let (JsValue::String(sa), JsValue::String(sb)) = (&a, &b) {
            self.stack.push(JsValue::Bool(f(
                if *sa < *sb { -1.0 } else if *sa > *sb { 1.0 } else { 0.0 },
                0.0,
            )));
        } else {
            self.stack.push(JsValue::Bool(f(a.to_number(), b.to_number())));
        }
    }

    fn call_function(&mut self, argc: usize) {
        // Stack: [..., callee, arg1, arg2, ...]
        // OR for method calls: [..., this, callee, arg1, arg2, ...]
        let args_start = self.stack.len() - argc;
        let callee_idx = args_start - 1;

        if callee_idx >= self.stack.len() {
            self.stack.push(JsValue::Undefined);
            return;
        }

        let callee = self.stack[callee_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(callee_idx);

        match callee {
            JsValue::Function(func) => {
                match &func.kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, &args);
                        self.stack.push(result);
                    }
                    FnKind::Bytecode(chunk) => {
                        let local_count = chunk.local_count as usize;
                        let mut locals = vec![JsValue::Undefined; local_count];
                        // Copy arguments into locals
                        for (i, arg) in args.iter().enumerate() {
                            if i < local_count {
                                locals[i] = arg.clone();
                            }
                        }
                        let this_val = func.this_binding
                            .as_ref()
                            .map(|b| *b.clone())
                            .unwrap_or(JsValue::Undefined);
                        let frame = CallFrame {
                            chunk: chunk.clone(),
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            this_val,
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _ => {
                // Not callable
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    fn new_object(&mut self, argc: usize) {
        let args_start = self.stack.len() - argc;
        let ctor_idx = args_start - 1;

        if ctor_idx >= self.stack.len() {
            self.stack.push(JsValue::Undefined);
            return;
        }

        let ctor = self.stack[ctor_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(ctor_idx);

        match ctor {
            JsValue::Function(func) => {
                // Create new object with constructor's prototype
                let new_obj = JsValue::Object(Box::new(JsObject {
                    properties: BTreeMap::new(),
                    prototype: Some(self.object_proto.clone()),
                }));

                match &func.kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, &args);
                        if result.is_object() {
                            self.stack.push(result);
                        } else {
                            self.stack.push(new_obj);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let local_count = chunk.local_count as usize;
                        let mut locals = vec![JsValue::Undefined; local_count];
                        for (i, arg) in args.iter().enumerate() {
                            if i < local_count {
                                locals[i] = arg.clone();
                            }
                        }
                        let frame = CallFrame {
                            chunk: chunk.clone(),
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            this_val: new_obj,
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _ => {
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    fn instance_of(&self, left: &JsValue, _right: &JsValue) -> bool {
        // Simplified instanceof — check prototype chain
        match left {
            JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_) => true,
            _ => false,
        }
    }

    fn create_iterator(&self, val: &JsValue) -> JsValue {
        match val {
            JsValue::Array(arr) => {
                // Create iterator as an object with __items__ and __index__
                let mut iter = JsObject::new();
                iter.set(String::from("__index__"), JsValue::Number(0.0));
                let items = JsValue::Array(arr.clone());
                iter.set(String::from("__items__"), items);
                JsValue::Object(Box::new(iter))
            }
            JsValue::String(s) => {
                // Iterate over characters
                let chars: Vec<JsValue> = s.chars()
                    .map(|c| {
                        let mut cs = String::new();
                        cs.push(c);
                        JsValue::String(cs)
                    })
                    .collect();
                let mut iter = JsObject::new();
                iter.set(String::from("__index__"), JsValue::Number(0.0));
                iter.set(String::from("__items__"), JsValue::Array(Box::new(JsArray::from_vec(chars))));
                JsValue::Object(Box::new(iter))
            }
            JsValue::Object(obj) => {
                // For-in: iterate over keys
                let keys: Vec<JsValue> = obj.keys()
                    .into_iter()
                    .map(JsValue::String)
                    .collect();
                let mut iter = JsObject::new();
                iter.set(String::from("__index__"), JsValue::Number(0.0));
                iter.set(String::from("__items__"), JsValue::Array(Box::new(JsArray::from_vec(keys))));
                JsValue::Object(Box::new(iter))
            }
            _ => JsValue::Undefined,
        }
    }

    fn iter_next(&self, iter: &JsValue) -> (JsValue, bool) {
        match iter {
            JsValue::Object(obj) => {
                let index = obj.get("__index__").to_number() as usize;
                let items = obj.get("__items__");
                match items {
                    JsValue::Array(arr) => {
                        if index < arr.elements.len() {
                            let val = arr.elements[index].clone();
                            // Note: we can't modify the iterator here since we have &self
                            // The VM loop would need to handle this differently
                            // For now, return the value
                            (val, false)
                        } else {
                            (JsValue::Undefined, true)
                        }
                    }
                    _ => (JsValue::Undefined, true),
                }
            }
            _ => (JsValue::Undefined, true),
        }
    }

    fn handle_exception(&mut self, val: JsValue) -> bool {
        if let Some(handler) = self.try_handlers.pop() {
            // Unwind stack
            self.stack.truncate(handler.stack_depth);
            // Unwind frames
            while self.frames.len() > handler.frame_depth {
                self.frames.pop();
            }
            // Jump to catch handler
            if let Some(frame) = self.frames.last_mut() {
                frame.ip = handler.catch_ip;
            }
            // Push exception value
            self.stack.push(val);
            true
        } else {
            false
        }
    }

    // ── Built-in Prototype Initialization ──

    fn init_prototypes(&mut self) {
        // Object.prototype methods
        self.object_proto.set(
            String::from("hasOwnProperty"),
            native_fn("hasOwnProperty", has_own_property),
        );
        self.object_proto.set(
            String::from("toString"),
            native_fn("toString", object_to_string),
        );
        self.object_proto.set(
            String::from("valueOf"),
            native_fn("valueOf", object_value_of),
        );

        // Array.prototype methods
        self.array_proto.prototype = Some(self.object_proto.clone());
        self.array_proto.set(String::from("push"), native_fn("push", array_push));
        self.array_proto.set(String::from("pop"), native_fn("pop", array_pop));
        self.array_proto.set(String::from("shift"), native_fn("shift", array_shift));
        self.array_proto.set(String::from("unshift"), native_fn("unshift", array_unshift));
        self.array_proto.set(String::from("indexOf"), native_fn("indexOf", array_index_of));
        self.array_proto.set(String::from("includes"), native_fn("includes", array_includes));
        self.array_proto.set(String::from("join"), native_fn("join", array_join));
        self.array_proto.set(String::from("slice"), native_fn("slice", array_slice));
        self.array_proto.set(String::from("splice"), native_fn("splice", array_splice));
        self.array_proto.set(String::from("concat"), native_fn("concat", array_concat));
        self.array_proto.set(String::from("reverse"), native_fn("reverse", array_reverse));
        self.array_proto.set(String::from("map"), native_fn("map", array_map));
        self.array_proto.set(String::from("filter"), native_fn("filter", array_filter));
        self.array_proto.set(String::from("forEach"), native_fn("forEach", array_for_each));
        self.array_proto.set(String::from("reduce"), native_fn("reduce", array_reduce));
        self.array_proto.set(String::from("find"), native_fn("find", array_find));
        self.array_proto.set(String::from("some"), native_fn("some", array_some));
        self.array_proto.set(String::from("every"), native_fn("every", array_every));
        self.array_proto.set(String::from("flat"), native_fn("flat", array_flat));
        self.array_proto.set(String::from("toString"), native_fn("toString", array_to_string));

        // String.prototype methods
        self.string_proto.prototype = Some(self.object_proto.clone());
        self.string_proto.set(String::from("charAt"), native_fn("charAt", string_char_at));
        self.string_proto.set(String::from("charCodeAt"), native_fn("charCodeAt", string_char_code_at));
        self.string_proto.set(String::from("indexOf"), native_fn("indexOf", string_index_of));
        self.string_proto.set(String::from("lastIndexOf"), native_fn("lastIndexOf", string_last_index_of));
        self.string_proto.set(String::from("includes"), native_fn("includes", string_includes));
        self.string_proto.set(String::from("startsWith"), native_fn("startsWith", string_starts_with));
        self.string_proto.set(String::from("endsWith"), native_fn("endsWith", string_ends_with));
        self.string_proto.set(String::from("slice"), native_fn("slice", string_slice));
        self.string_proto.set(String::from("substring"), native_fn("substring", string_substring));
        self.string_proto.set(String::from("toLowerCase"), native_fn("toLowerCase", string_to_lower));
        self.string_proto.set(String::from("toUpperCase"), native_fn("toUpperCase", string_to_upper));
        self.string_proto.set(String::from("trim"), native_fn("trim", string_trim));
        self.string_proto.set(String::from("split"), native_fn("split", string_split));
        self.string_proto.set(String::from("replace"), native_fn("replace", string_replace));
        self.string_proto.set(String::from("repeat"), native_fn("repeat", string_repeat));
        self.string_proto.set(String::from("padStart"), native_fn("padStart", string_pad_start));
        self.string_proto.set(String::from("padEnd"), native_fn("padEnd", string_pad_end));
        self.string_proto.set(String::from("toString"), native_fn("toString", string_to_string));

        // Number.prototype
        self.number_proto.prototype = Some(self.object_proto.clone());
        self.number_proto.set(String::from("toFixed"), native_fn("toFixed", number_to_fixed));
        self.number_proto.set(String::from("toString"), native_fn("toString", number_to_string_method));

        // Function.prototype
        self.function_proto.prototype = Some(self.object_proto.clone());
        self.function_proto.set(String::from("call"), native_fn("call", function_call));
        self.function_proto.set(String::from("apply"), native_fn("apply", function_apply));
        self.function_proto.set(String::from("bind"), native_fn("bind", function_bind));
    }

    fn init_globals(&mut self) {
        // console object
        let mut console = JsObject::new();
        console.set(String::from("log"), native_fn("log", console_log));
        console.set(String::from("warn"), native_fn("warn", console_log));
        console.set(String::from("error"), native_fn("error", console_log));
        console.set(String::from("info"), native_fn("info", console_log));
        self.globals.set(String::from("console"), JsValue::Object(Box::new(console)));

        // Math object
        let mut math = JsObject::new();
        math.set(String::from("PI"), JsValue::Number(core::f64::consts::PI));
        math.set(String::from("E"), JsValue::Number(core::f64::consts::E));
        math.set(String::from("LN2"), JsValue::Number(core::f64::consts::LN_2));
        math.set(String::from("LN10"), JsValue::Number(core::f64::consts::LN_10));
        math.set(String::from("SQRT2"), JsValue::Number(core::f64::consts::SQRT_2));
        math.set(String::from("abs"), native_fn("abs", math_abs));
        math.set(String::from("floor"), native_fn("floor", math_floor));
        math.set(String::from("ceil"), native_fn("ceil", math_ceil));
        math.set(String::from("round"), native_fn("round", math_round));
        math.set(String::from("max"), native_fn("max", math_max));
        math.set(String::from("min"), native_fn("min", math_min));
        math.set(String::from("pow"), native_fn("pow", math_pow));
        math.set(String::from("sqrt"), native_fn("sqrt", math_sqrt));
        math.set(String::from("random"), native_fn("random", math_random));
        math.set(String::from("trunc"), native_fn("trunc", math_trunc));
        math.set(String::from("sign"), native_fn("sign", math_sign));
        math.set(String::from("log"), native_fn("log", math_log_fn));
        self.globals.set(String::from("Math"), JsValue::Object(Box::new(math)));

        // JSON object
        let mut json = JsObject::new();
        json.set(String::from("stringify"), native_fn("stringify", json_stringify));
        json.set(String::from("parse"), native_fn("parse", json_parse));
        self.globals.set(String::from("JSON"), JsValue::Object(Box::new(json)));

        // Global functions
        self.globals.set(String::from("parseInt"), native_fn("parseInt", parse_int));
        self.globals.set(String::from("parseFloat"), native_fn("parseFloat", parse_float));
        self.globals.set(String::from("isNaN"), native_fn("isNaN", is_nan));
        self.globals.set(String::from("isFinite"), native_fn("isFinite", is_finite));
        self.globals.set(String::from("String"), native_fn("String", global_string));
        self.globals.set(String::from("Number"), native_fn("Number", global_number));
        self.globals.set(String::from("Boolean"), native_fn("Boolean", global_boolean));
        self.globals.set(String::from("Array"), native_fn("Array", global_array));
        self.globals.set(String::from("Object"), native_fn("Object", global_object));

        // Object.keys, Object.values, Object.assign etc.
        let mut object_ctor = JsObject::new();
        object_ctor.set(String::from("keys"), native_fn("keys", object_keys));
        object_ctor.set(String::from("values"), native_fn("values", object_values));
        object_ctor.set(String::from("entries"), native_fn("entries", object_entries));
        object_ctor.set(String::from("assign"), native_fn("assign", object_assign));
        object_ctor.set(String::from("freeze"), native_fn("freeze", object_freeze));
        object_ctor.set(String::from("create"), native_fn("create", object_create));
        self.globals.set(String::from("Object"), JsValue::Object(Box::new(object_ctor)));

        // Array.isArray, Array.from
        let mut array_ctor = JsObject::new();
        array_ctor.set(String::from("isArray"), native_fn("isArray", array_is_array));
        array_ctor.set(String::from("from"), native_fn("from", array_from));
        self.globals.set(String::from("Array"), JsValue::Object(Box::new(array_ctor)));

        // Constants
        self.globals.set(String::from("NaN"), JsValue::Number(f64::NAN));
        self.globals.set(String::from("Infinity"), JsValue::Number(f64::INFINITY));
        self.globals.set(String::from("undefined"), JsValue::Undefined);
    }
}

// ── Helper Functions ──

fn native_fn(name: &str, f: fn(&mut Vm, &[JsValue]) -> JsValue) -> JsValue {
    JsValue::Function(Box::new(JsFunction {
        name: Some(String::from(name)),
        params: Vec::new(),
        kind: FnKind::Native(f),
        this_binding: None,
    }))
}

fn try_parse_index(s: &str) -> Option<usize> {
    let mut n: usize = 0;
    for b in s.bytes() {
        if b < b'0' || b > b'9' { return None; }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    if s.is_empty() { None } else { Some(n) }
}

fn pow_f64(base: f64, exp: f64) -> f64 {
    if exp == 0.0 { return 1.0; }
    if base == 1.0 { return 1.0; }
    if exp.is_nan() { return f64::NAN; }
    // Simple integer powers
    if exp == (exp as i32) as f64 && exp.abs() < 100.0 {
        let n = exp as i32;
        if n >= 0 {
            let mut r = 1.0;
            for _ in 0..n { r *= base; }
            r
        } else {
            let mut r = 1.0;
            for _ in 0..(-n) { r *= base; }
            1.0 / r
        }
    } else {
        // For non-integer exponents, use exp/log approximation
        // This is a simplified version; a real implementation would use libm
        if base <= 0.0 { return f64::NAN; }
        exp_approx(exp * ln_approx(base))
    }
}

fn ln_approx(x: f64) -> f64 {
    if x <= 0.0 { return f64::NEG_INFINITY; }
    // Use the identity: ln(x) = 2 * atanh((x-1)/(x+1))
    let mut val = x;
    let mut exp: f64 = 0.0;
    while val > 2.0 { val /= 2.0; exp += 1.0; }
    while val < 0.5 { val *= 2.0; exp -= 1.0; }
    let t = (val - 1.0) / (val + 1.0);
    let t2 = t * t;
    let mut sum = t;
    let mut term = t;
    for i in 0..20 {
        term *= t2;
        sum += term / (2 * i + 3) as f64;
    }
    2.0 * sum + exp * core::f64::consts::LN_2
}

fn exp_approx(x: f64) -> f64 {
    if x > 709.0 { return f64::INFINITY; }
    if x < -709.0 { return 0.0; }
    // Use Taylor series around 0
    let ratio = x / core::f64::consts::LN_2;
    let k = floor_f64(ratio + 0.5) as i32;
    let r = x - k as f64 * core::f64::consts::LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    for i in 1..30 {
        term *= r / i as f64;
        sum += term;
        if term.abs() < 1e-15 { break; }
    }
    // Multiply by 2^k
    let mut result = sum;
    if k >= 0 {
        for _ in 0..k.min(1023) { result *= 2.0; }
    } else {
        for _ in 0..(-k).min(1023) { result /= 2.0; }
    }
    result
}

// ═══════════════════════════════════════════════════════════
// Native function implementations
// ═══════════════════════════════════════════════════════════

// ── Console ──

fn console_log(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let parts: Vec<String> = args.iter().map(|a| a.to_js_string()).collect();
    let msg = parts.join(" ");
    vm.console_output.push(msg);
    JsValue::Undefined
}

// ── Object.prototype ──

fn has_own_property(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // `this` is not properly passed yet, simplified
    if args.is_empty() { return JsValue::Bool(false); }
    JsValue::Bool(false)
}

fn object_to_string(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("[object Object]"))
}

fn object_value_of(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

// ── Object static methods ──

fn object_keys(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let keys: Vec<JsValue> = obj.keys().into_iter().map(JsValue::String).collect();
        JsValue::Array(Box::new(JsArray::from_vec(keys)))
    } else {
        JsValue::Array(Box::new(JsArray::new()))
    }
}

fn object_values(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let vals: Vec<JsValue> = obj.properties.values()
            .filter(|p| p.enumerable)
            .map(|p| p.value.clone())
            .collect();
        JsValue::Array(Box::new(JsArray::from_vec(vals)))
    } else {
        JsValue::Array(Box::new(JsArray::new()))
    }
}

fn object_entries(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(JsValue::Object(obj)) = args.first() {
        let entries: Vec<JsValue> = obj.properties.iter()
            .filter(|(_, p)| p.enumerable)
            .map(|(k, p)| {
                JsValue::Array(Box::new(JsArray::from_vec(vec![
                    JsValue::String(k.clone()),
                    p.value.clone(),
                ])))
            })
            .collect();
        JsValue::Array(Box::new(JsArray::from_vec(entries)))
    } else {
        JsValue::Array(Box::new(JsArray::new()))
    }
}

fn object_assign(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.is_empty() { return JsValue::Undefined; }
    let mut target = args[0].clone();
    for source in &args[1..] {
        if let JsValue::Object(src) = source {
            for (k, p) in &src.properties {
                if p.enumerable {
                    target.set_property(k.clone(), p.value.clone());
                }
            }
        }
    }
    target
}

fn object_freeze(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

fn object_create(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let proto = match args.first() {
        Some(JsValue::Object(o)) => Some(o.clone()),
        Some(JsValue::Null) => None,
        _ => None,
    };
    JsValue::Object(Box::new(JsObject {
        properties: BTreeMap::new(),
        prototype: proto,
    }))
}

// ── Array.prototype ──

fn array_push(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Simplified — in a real engine `this` would be the array
    JsValue::Number(0.0)
}

fn array_pop(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn array_shift(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn array_unshift(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(0.0)
}

fn array_index_of(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(-1.0)
}

fn array_includes(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn array_join(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let sep = args.first().map(|v| v.to_js_string()).unwrap_or_else(|| String::from(","));
    JsValue::String(sep)
}

fn array_slice(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_splice(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_concat(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_reverse(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_map(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_filter(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_for_each(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn array_reduce(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn array_find(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn array_some(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn array_every(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(true)
}

fn array_flat(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn array_to_string(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn array_is_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(args.first().map(|v| v.is_array()).unwrap_or(false))
}

fn array_from(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    match args.first() {
        Some(JsValue::Array(a)) => JsValue::Array(a.clone()),
        Some(JsValue::String(s)) => {
            let chars: Vec<JsValue> = s.chars().map(|c| {
                let mut cs = String::new();
                cs.push(c);
                JsValue::String(cs)
            }).collect();
            JsValue::Array(Box::new(JsArray::from_vec(chars)))
        }
        _ => JsValue::Array(Box::new(JsArray::new())),
    }
}

// ── String.prototype ──

fn string_char_at(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    // `this` string not available in simplified version
    JsValue::String(String::new())
}

fn string_char_code_at(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(f64::NAN)
}

fn string_index_of(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(-1.0)
}

fn string_last_index_of(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Number(-1.0)
}

fn string_includes(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn string_starts_with(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn string_ends_with(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Bool(false)
}

fn string_slice(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_substring(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_to_lower(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_to_upper(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_trim(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_split(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Array(Box::new(JsArray::new()))
}

fn string_replace(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_repeat(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_pad_start(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_pad_end(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

fn string_to_string(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::new())
}

// ── Number.prototype ──

fn number_to_fixed(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("0"))
}

fn number_to_string_method(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::String(String::from("0"))
}

// ── Function.prototype ──

fn function_call(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn function_apply(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

fn function_bind(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

// ── Math functions ──

fn math_abs(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(if n < 0.0 { -n } else { n })
}

fn math_floor(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(floor_f64(n))
}

fn math_ceil(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(ceil_f64(n))
}

fn math_round(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(floor_f64(n + 0.5))
}

fn math_max(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.is_empty() { return JsValue::Number(f64::NEG_INFINITY); }
    let mut max = f64::NEG_INFINITY;
    for a in args {
        let n = a.to_number();
        if n > max { max = n; }
    }
    JsValue::Number(max)
}

fn math_min(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.is_empty() { return JsValue::Number(f64::INFINITY); }
    let mut min = f64::INFINITY;
    for a in args {
        let n = a.to_number();
        if n < min { min = n; }
    }
    JsValue::Number(min)
}

fn math_pow(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let base = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    let exp = args.get(1).map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(pow_f64(base, exp))
}

fn math_sqrt(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(sqrt_f64(n))
}

fn math_random(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    // Simple pseudo-random (not cryptographically secure)
    // Use a global counter as entropy source
    static mut SEED: u64 = 12345678901234567;
    unsafe {
        SEED ^= SEED << 13;
        SEED ^= SEED >> 7;
        SEED ^= SEED << 17;
        let val = (SEED & 0x000FFFFFFFFFFFFF) as f64 / (0x0010000000000000u64 as f64);
        JsValue::Number(val)
    }
}

fn math_trunc(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(trunc_f64(n))
}

fn math_sign(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    if n.is_nan() {
        JsValue::Number(f64::NAN)
    } else if n > 0.0 {
        JsValue::Number(1.0)
    } else if n < 0.0 {
        JsValue::Number(-1.0)
    } else {
        JsValue::Number(0.0)
    }
}

fn math_log_fn(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Number(ln_approx(n))
}

// ── JSON ──

fn json_stringify(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if let Some(val) = args.first() {
        JsValue::String(stringify_value(val))
    } else {
        JsValue::Undefined
    }
}

fn stringify_value(val: &JsValue) -> String {
    match val {
        JsValue::Undefined => String::from("undefined"),
        JsValue::Null => String::from("null"),
        JsValue::Bool(true) => String::from("true"),
        JsValue::Bool(false) => String::from("false"),
        JsValue::Number(n) => crate::value::format_number(*n),
        JsValue::String(s) => {
            let mut out = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
            out
        }
        JsValue::Array(arr) => {
            let mut out = String::from("[");
            for (i, elem) in arr.elements.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push_str(&stringify_value(elem));
            }
            out.push(']');
            out
        }
        JsValue::Object(obj) => {
            let mut out = String::from("{");
            let mut first = true;
            for (k, p) in &obj.properties {
                if !p.enumerable { continue; }
                if !first { out.push(','); }
                first = false;
                out.push('"');
                out.push_str(k);
                out.push_str("\":");
                out.push_str(&stringify_value(&p.value));
            }
            out.push('}');
            out
        }
        JsValue::Function(_) => String::from("undefined"),
    }
}

fn json_parse(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    // Simplified JSON parser
    if let Some(JsValue::String(s)) = args.first() {
        parse_json_value(s.as_bytes(), &mut 0)
    } else {
        JsValue::Undefined
    }
}

fn parse_json_value(bytes: &[u8], pos: &mut usize) -> JsValue {
    skip_ws(bytes, pos);
    if *pos >= bytes.len() { return JsValue::Undefined; }
    match bytes[*pos] {
        b'"' => {
            *pos += 1;
            let mut s = String::new();
            while *pos < bytes.len() && bytes[*pos] != b'"' {
                if bytes[*pos] == b'\\' {
                    *pos += 1;
                    if *pos < bytes.len() {
                        match bytes[*pos] {
                            b'n' => s.push('\n'),
                            b'r' => s.push('\r'),
                            b't' => s.push('\t'),
                            b'"' => s.push('"'),
                            b'\\' => s.push('\\'),
                            b'/' => s.push('/'),
                            _ => s.push(bytes[*pos] as char),
                        }
                    }
                } else {
                    s.push(bytes[*pos] as char);
                }
                *pos += 1;
            }
            if *pos < bytes.len() { *pos += 1; } // skip "
            JsValue::String(s)
        }
        b'{' => {
            *pos += 1;
            let mut obj = JsObject::new();
            skip_ws(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b'}' {
                *pos += 1;
                return JsValue::Object(Box::new(obj));
            }
            loop {
                skip_ws(bytes, pos);
                let key = if let JsValue::String(k) = parse_json_value(bytes, pos) { k } else { break; };
                skip_ws(bytes, pos);
                if *pos < bytes.len() && bytes[*pos] == b':' { *pos += 1; }
                let val = parse_json_value(bytes, pos);
                obj.set(key, val);
                skip_ws(bytes, pos);
                if *pos < bytes.len() && bytes[*pos] == b',' {
                    *pos += 1;
                } else {
                    break;
                }
            }
            skip_ws(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b'}' { *pos += 1; }
            JsValue::Object(Box::new(obj))
        }
        b'[' => {
            *pos += 1;
            let mut arr = Vec::new();
            skip_ws(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b']' {
                *pos += 1;
                return JsValue::Array(Box::new(JsArray::from_vec(arr)));
            }
            loop {
                let val = parse_json_value(bytes, pos);
                arr.push(val);
                skip_ws(bytes, pos);
                if *pos < bytes.len() && bytes[*pos] == b',' {
                    *pos += 1;
                } else {
                    break;
                }
            }
            skip_ws(bytes, pos);
            if *pos < bytes.len() && bytes[*pos] == b']' { *pos += 1; }
            JsValue::Array(Box::new(JsArray::from_vec(arr)))
        }
        b't' => {
            if bytes[*pos..].starts_with(b"true") { *pos += 4; }
            JsValue::Bool(true)
        }
        b'f' => {
            if bytes[*pos..].starts_with(b"false") { *pos += 5; }
            JsValue::Bool(false)
        }
        b'n' => {
            if bytes[*pos..].starts_with(b"null") { *pos += 4; }
            JsValue::Null
        }
        _ => {
            // Number
            let start = *pos;
            if *pos < bytes.len() && (bytes[*pos] == b'-' || bytes[*pos] == b'+') {
                *pos += 1;
            }
            while *pos < bytes.len() && (bytes[*pos].is_ascii_digit() || bytes[*pos] == b'.' || bytes[*pos] == b'e' || bytes[*pos] == b'E' || bytes[*pos] == b'+' || bytes[*pos] == b'-') {
                if *pos > start + 1 && (bytes[*pos] == b'+' || bytes[*pos] == b'-') && bytes[*pos - 1] != b'e' && bytes[*pos - 1] != b'E' {
                    break;
                }
                *pos += 1;
            }
            let s = core::str::from_utf8(&bytes[start..*pos]).unwrap_or("0");
            JsValue::Number(crate::value::parse_js_float(s))
        }
    }
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    while *pos < bytes.len() && (bytes[*pos] == b' ' || bytes[*pos] == b'\t' || bytes[*pos] == b'\n' || bytes[*pos] == b'\r') {
        *pos += 1;
    }
}

// ── Global functions ──

fn parse_int(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    let radix = args.get(1).map(|v| v.to_number() as u32).unwrap_or(10);
    let s = s.trim();
    if s.is_empty() { return JsValue::Number(f64::NAN); }
    let mut result: f64 = 0.0;
    let mut has_digit = false;
    let negative = s.starts_with('-');
    let start = if negative || s.starts_with('+') { 1 } else { 0 };
    for &b in s[start..].as_bytes() {
        let digit = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'a'..=b'z' => (b - b'a' + 10) as u32,
            b'A'..=b'Z' => (b - b'A' + 10) as u32,
            _ => break,
        };
        if digit >= radix { break; }
        has_digit = true;
        result = result * radix as f64 + digit as f64;
    }
    if !has_digit { return JsValue::Number(f64::NAN); }
    JsValue::Number(if negative { -result } else { result })
}

fn parse_float(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let s = args.first().map(|v| v.to_js_string()).unwrap_or_default();
    JsValue::Number(crate::value::parse_js_float(&s))
}

fn is_nan(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_nan())
}

fn is_finite(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let n = args.first().map(|v| v.to_number()).unwrap_or(f64::NAN);
    JsValue::Bool(n.is_finite())
}

fn global_string(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::String(args.first().map(|v| v.to_js_string()).unwrap_or_default())
}

fn global_number(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Number(args.first().map(|v| v.to_number()).unwrap_or(0.0))
}

fn global_boolean(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    JsValue::Bool(args.first().map(|v| v.to_boolean()).unwrap_or(false))
}

fn global_array(_vm: &mut Vm, args: &[JsValue]) -> JsValue {
    if args.len() == 1 {
        if let JsValue::Number(n) = &args[0] {
            let len = *n as usize;
            let elems = vec![JsValue::Undefined; len];
            return JsValue::Array(Box::new(JsArray::from_vec(elems)));
        }
    }
    JsValue::Array(Box::new(JsArray::from_vec(args.to_vec())))
}

fn global_object(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Object(Box::new(JsObject::new()))
}

// ── Math utility functions (no_std) ──

fn floor_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    let i = n as i64;
    if (i as f64) <= n { i as f64 } else { (i - 1) as f64 }
}

fn ceil_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    let i = n as i64;
    if (i as f64) >= n { i as f64 } else { (i + 1) as f64 }
}

fn trunc_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() { return n; }
    n as i64 as f64
}

fn sqrt_f64(n: f64) -> f64 {
    if n < 0.0 { return f64::NAN; }
    if n == 0.0 || n == 1.0 { return n; }
    if n.is_nan() || n.is_infinite() { return n; }
    // Newton's method
    let mut x = n / 2.0;
    for _ in 0..64 {
        let next = (x + n / x) / 2.0;
        if (next - x).abs() < 1e-15 { break; }
        x = next;
    }
    x
}
