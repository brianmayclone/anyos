//! Function call, method call, and constructor handling.

use alloc::rc::Rc;
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
    fn invoke_function(&mut self, callee: &JsValue, args: &[JsValue], this_val: JsValue) {
        match callee {
            JsValue::Function(func_rc) => {
                // Extract what we need before any mutable VM operations
                let kind = func_rc.borrow().kind.clone();
                let this_bind = func_rc.borrow().this_binding.clone();
                let captured_upvalues = func_rc.borrow().upvalues.clone();

                let effective_this = this_bind.unwrap_or(this_val);
                self.current_this = effective_this.clone();

                match kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, args);
                        self.stack.push(result);
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
                            this_val: effective_this,
                        };
                        self.frames.push(frame);
                    }
                }
            }
            _ => {
                self.log_engine("[libjs] WARN: attempted to call non-function");
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

                // Create new object with prototype
                let new_obj = JsValue::Object(Rc::new(RefCell::new(JsObject {
                    properties: alloc::collections::BTreeMap::new(),
                    prototype: Some(self.object_proto.clone()),
                    internal_tag: None,
                    set_hook: None,
                    set_hook_data: core::ptr::null_mut(),
                })));

                self.current_this = new_obj.clone();

                match kind {
                    FnKind::Native(native_fn) => {
                        let result = native_fn(self, &args);
                        if result.is_object() || result.is_array() {
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
                        let frame = CallFrame {
                            chunk,
                            ip: 0,
                            stack_base: self.stack.len(),
                            locals,
                            upvalue_cells: captured_upvalues,
                            this_val: new_obj,
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
        result
    }

    /// Simplified instanceof check.
    pub fn instance_of(&self, left: &JsValue, _right: &JsValue) -> bool {
        matches!(left, JsValue::Object(_) | JsValue::Array(_) | JsValue::Function(_))
    }
}
