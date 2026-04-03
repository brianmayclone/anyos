//! Function call, method call, and constructor handling.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use crate::value::*;
use super::{Vm, CallFrame};

impl Vm {
    /// Regular function call: Stack = [..., callee, arg1..argN]
    pub fn call_function(&mut self, argc: usize) {
        if self.stack.len() < argc + 1 {
            self.stack.push(JsValue::Undefined);
            return;
        }
        let args_start = self.stack.len() - argc;
        let callee_idx = args_start - 1;

        let callee = self.stack[callee_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(callee_idx);

        self.current_this = JsValue::Undefined;
        self.invoke_function(&callee, &args, JsValue::Undefined);
    }

    /// Method call: Stack = [..., this_obj, method_fn, arg1..argN]
    pub fn call_method(&mut self, argc: usize) {
        if self.stack.len() < argc + 2 {
            self.stack.push(JsValue::Undefined);
            return;
        }
        let args_start = self.stack.len() - argc;
        let method_idx = args_start - 1;
        let this_idx = method_idx - 1;

        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        let callee = self.stack[method_idx].clone();
        let this_val = self.stack[this_idx].clone();
        self.stack.truncate(this_idx);

        self.current_this = this_val.clone();
        self.invoke_function(&callee, &args, this_val);
    }

    /// Invoke a function value with the given arguments and this binding.
    pub(super) fn invoke_function(&mut self, callee: &JsValue, args: &[JsValue], this_val: JsValue) {
        match callee {
            JsValue::Function(func_rc) => {
                // Extract what we need before any mutable VM operations
                let kind = func_rc.borrow().kind.clone();
                let this_bind = func_rc.borrow().this_binding.clone();
                let captured_upvalues = func_rc.borrow().upvalues.clone();
                let bound_args = func_rc.borrow().bound_args.clone();

                let effective_this = this_bind.unwrap_or(this_val);
                self.current_this = effective_this.clone();

                // Prepend bound arguments (from Function.prototype.bind) to call args
                // per ES2023 §10.4.1.1 [[Call]].
                let effective_args: Vec<JsValue>;
                let args = if bound_args.is_empty() {
                    args
                } else {
                    effective_args = bound_args.into_iter().chain(args.iter().cloned()).collect();
                    &effective_args
                };

                match kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, args);
                        // Check if the native function signalled an exception.
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                self.stack.push(JsValue::Undefined);
                            }
                        } else {
                            self.stack.push(result);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let local_count = chunk.local_count as usize;
                        let mut locals: Vec<Rc<RefCell<JsValue>>> = (0..local_count)
                            .map(|_| Rc::new(RefCell::new(JsValue::Undefined)))
                            .collect();
                        for (i, arg) in args.iter().enumerate() {
                            if i < local_count {
                                *locals[i].borrow_mut() = arg.clone();
                            }
                        }

                        // Generator function: return a GeneratorObject instead of executing
                        if chunk.is_generator {
                            let gen_obj = super::native_generator::create_generator_object(
                                self,
                                chunk,
                                locals,
                                captured_upvalues,
                                effective_this,
                            );
                            self.stack.push(gen_obj);
                            return;
                        }

                        let frame = CallFrame {
                            chunk,
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            upvalue_cells: captured_upvalues,
                            this_val: effective_this,
                            is_constructor: false,
                            all_args: args.to_vec(),
                            self_ref: callee.clone(),
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _other => {
                // Try to extract the method name from the current call frame's bytecode
                let mut context = alloc::string::String::new();
                if let Some(frame) = self.frames.last() {
                    let ip = frame.ip;
                    // Look backward through bytecode for the property name that was loaded
                    let code = &frame.chunk.code;
                    let consts = &frame.chunk.constants;
                    if ip >= 2 {
                        for back in 1..ip.min(10) {
                            let check_ip = ip - back;
                            if check_ip < code.len() {
                                if let crate::bytecode::Op::GetPropNamed(ci) = &code[check_ip] {
                                    if let Some(crate::bytecode::Constant::String(s)) = consts.get(*ci as usize) {
                                        context = alloc::format!(" prop=.{}", s);
                                        break;
                                    }
                                }
                                if let crate::bytecode::Op::LoadGlobal(ci) = &code[check_ip] {
                                    if let Some(crate::bytecode::Constant::String(s)) = consts.get(*ci as usize) {
                                        context = alloc::format!(" global={}", s);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                // Include call stack for debugging
                let mut stack = alloc::string::String::new();
                for (fi, frame) in self.frames.iter().rev().take(6).enumerate() {
                    let fname = frame.chunk.name.as_deref().unwrap_or("(anon)");
                    if fi > 0 { stack.push_str(" <- "); }
                    stack.push_str(fname);
                }
                self.log_engine(&alloc::format!(
                    "[libjs] WARN: attempted to call non-function{} [{}]", context, stack
                ));
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    /// `new Constructor(args)` — creates a new object and calls constructor.
    pub fn new_object(&mut self, argc: usize) {
        if self.stack.len() < argc + 1 {
            self.stack.push(JsValue::Undefined);
            return;
        }
        let args_start = self.stack.len() - argc;
        let ctor_idx = args_start - 1;

        let ctor = self.stack[ctor_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(ctor_idx);

        match ctor {
            JsValue::Function(func_rc) => {
                let kind = func_rc.borrow().kind.clone();

                // ES2023 §14.2.17: Arrow functions are not constructable
                let is_arrow = match &kind {
                    FnKind::Bytecode(chunk) => chunk.is_arrow,
                    _ => false,
                };
                if is_arrow {
                    let name = func_rc.borrow().name.clone().unwrap_or_default();
                    let msg = alloc::format!("{} is not a constructor", if name.is_empty() { "(intermediate value)".into() } else { name });
                    let exc = self.make_type_error(&msg);
                    if !self.handle_exception(exc) {
                        self.stack.push(JsValue::Undefined);
                        return;
                    }
                    return;
                }

                // Use Constructor.prototype as the new object's prototype (JS spec).
                // Prefer own_props["prototype"] (set by `Dog.prototype = ...` assignments)
                // over the internal prototype field.
                let ctor_proto = {
                    let f = func_rc.borrow();
                    if let Some(JsValue::Object(proto_obj)) = f.own_props.get("prototype") {
                        Some(proto_obj.clone())
                    } else if let Some(ref proto) = f.prototype {
                        Some(proto.clone())
                    } else {
                        // Lazy-create prototype with constructor back-link (ES2023 §10.2.4)
                        drop(f);
                        let proto = Rc::new(RefCell::new(JsObject::new()));
                        proto.borrow_mut().set(
                            String::from("constructor"),
                            JsValue::Function(func_rc.clone()),
                        );
                        func_rc.borrow_mut().prototype = Some(proto.clone());
                        Some(proto)
                    }
                };
                let new_obj = JsValue::Object(Rc::new(RefCell::new(JsObject {
                    properties: alloc::collections::BTreeMap::new(),
                    prototype: ctor_proto.or(Some(self.object_proto.clone())),
                    internal_tag: None,
                    primitive_value: None,
                    set_hook: None,
                    set_hook_data: core::ptr::null_mut(),
                })));

                self.current_this = new_obj.clone();

                match kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, &args);
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                self.stack.push(JsValue::Undefined);
                            }
                        } else if result.is_object() || result.is_array() {
                            self.stack.push(result);
                        } else {
                            self.stack.push(new_obj);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let captured_upvalues = func_rc.borrow().upvalues.clone();
                        let local_count = chunk.local_count as usize;
                        let mut locals: Vec<Rc<RefCell<JsValue>>> = (0..local_count)
                            .map(|_| Rc::new(RefCell::new(JsValue::Undefined)))
                            .collect();
                        for (i, arg) in args.iter().enumerate() {
                            if i < local_count {
                                *locals[i].borrow_mut() = arg.clone();
                            }
                        }
                        let ctor_ref = JsValue::Function(func_rc.clone());
                        let frame = CallFrame {
                            chunk,
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            upvalue_cells: captured_upvalues,
                            this_val: new_obj,
                            is_constructor: true,
                            all_args: args.to_vec(),
                            self_ref: ctor_ref,
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _ => {
                self.log_engine("[libjs] WARN: new called on non-function");
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    /// Call a JS function value directly from Rust.
    /// Handles both native and bytecode functions, including re-entrant execution.
    pub fn call_value(&mut self, callee: &JsValue, args: &[JsValue], this_val: JsValue) -> JsValue {
        let saved_depth = self.frames.len();
        let stack_before = self.stack.len();
        self.invoke_function(callee, args, this_val);

        // Native function: result already on stack, no new frame pushed.
        if self.frames.len() <= saved_depth {
            return self.stack.pop().unwrap_or(JsValue::Undefined);
        }

        // Bytecode function: run until we're back to saved depth.
        let prev_target = self.run_target_depth;
        self.run_target_depth = saved_depth;
        let result = self.run();
        self.run_target_depth = prev_target;
        // Op::Return pushes the return value onto the stack AND returns it
        // from run(). Restore stack to pre-call depth to avoid pollution.
        // Use truncate (not pop) because run() might exit without pushing
        // (e.g. step limit, empty frames, exception).
        self.stack.truncate(stack_before);
        result
    }

    /// `super(args)` — call parent constructor, forwarding new.target from the
    /// current (derived) constructor frame.  Stack: [..., SuperClass, arg1..argN]
    pub fn super_call(&mut self, argc: usize) {
        if self.stack.len() < argc + 1 {
            self.stack.push(JsValue::Undefined);
            return;
        }
        let args_start = self.stack.len() - argc;
        let ctor_idx = args_start - 1;

        let super_ctor = self.stack[ctor_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(ctor_idx);

        // Find the current new.target from the enclosing constructor frame.
        let mut new_target = JsValue::Undefined;
        for f in self.frames.iter().rev() {
            if f.is_constructor {
                new_target = f.self_ref.clone();
                break;
            }
        }

        // Get `this` from the current constructor frame.
        let this_val = self.frames.last()
            .map(|f| f.this_val.clone())
            .unwrap_or(JsValue::Undefined);

        match super_ctor {
            JsValue::Function(func_rc) => {
                let kind = func_rc.borrow().kind.clone();
                let captured_upvalues = func_rc.borrow().upvalues.clone();

                self.current_this = this_val.clone();

                match kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, &args);
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                self.stack.push(JsValue::Undefined);
                            }
                        } else {
                            self.stack.push(result);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let local_count = chunk.local_count as usize;
                        let mut locals: Vec<Rc<RefCell<JsValue>>> = (0..local_count)
                            .map(|_| Rc::new(RefCell::new(JsValue::Undefined)))
                            .collect();
                        for (i, arg) in args.iter().enumerate() {
                            if i < local_count {
                                *locals[i].borrow_mut() = arg.clone();
                            }
                        }
                        let frame = CallFrame {
                            chunk,
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            upvalue_cells: captured_upvalues,
                            this_val,
                            is_constructor: true,
                            all_args: args.to_vec(),
                            // new.target is forwarded from the derived constructor.
                            self_ref: new_target,
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _ => {
                self.log_engine("[libjs] WARN: super() called on non-function");
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    /// Simplified instanceof check.
    pub fn instance_of(&self, left: &JsValue, _right: &JsValue) -> bool {
        matches!(left, JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_))
    }
}
