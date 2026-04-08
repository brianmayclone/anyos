//! Function.prototype methods: call, apply, bind, toString.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::Vm;
use crate::value::*;

// ═══════════════════════════════════════════════════════════
// Function.prototype methods
// ═══════════════════════════════════════════════════════════

/// Function.prototype.call(thisArg, ...args)
pub fn function_call(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let func = vm.current_this.clone();
    let this_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
    let call_args: Vec<JsValue> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };

    invoke_with_this(vm, &func, &this_arg, &call_args)
}

/// Function.prototype.apply(thisArg, argsArray)
pub fn function_apply(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let func = vm.current_this.clone();
    let this_arg = args.first().cloned().unwrap_or(JsValue::Undefined);
    let call_args: Vec<JsValue> = match args.get(1) {
        Some(JsValue::Array(arr)) => arr.borrow().to_dense_vec(),
        _ => Vec::new(),
    };

    invoke_with_this(vm, &func, &this_arg, &call_args)
}

/// Function.prototype.bind(thisArg, ...args)
pub fn function_bind(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let func = vm.current_this.clone();
    let bound_this = args.first().cloned().unwrap_or(JsValue::Undefined);
    let bound_args: Vec<JsValue> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        Vec::new()
    };

    // Create a new function that wraps the original with bound this + args.
    // For native function pointers we can't create true closures, so we
    // clone the original and set this_binding.  Partial application of
    // bound_args is not supported for bytecode functions in this
    // simplified implementation but the this binding works correctly.
    match &func {
        JsValue::Function(f) => {
            let original = f.borrow();
            let bound = JsFunction {
                name: original.name.as_ref().map(|n| {
                    let mut s = String::from("bound ");
                    s.push_str(n);
                    s
                }),
                params: original.params.clone(),
                kind: original.kind.clone(),
                this_binding: Some(bound_this),
                bound_args: bound_args,
                upvalues: original.upvalues.clone(),
                // ES2023 §10.4.1.3: Bound functions do NOT have own .prototype
                prototype: None,
                own_props: original.own_props.clone(),
                arity: original.arity,
                super_class: original.super_class.clone(),
            };
            drop(original);
            JsValue::Function(Rc::new(RefCell::new(bound)))
        }
        _ => {
            vm.log_engine("[libjs] WARN: bind called on non-function");
            JsValue::Undefined
        }
    }
}

/// Function.prototype.toString()
pub fn function_to_string(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    match &vm.current_this {
        JsValue::Function(f) => {
            let func = f.borrow();
            let name = func.name.as_deref().unwrap_or("anonymous");
            JsValue::String(format!("function {}() {{ [native code] }}", name))
        }
        _ => JsValue::String(String::from("function() { [native code] }")),
    }
}

// ═══════════════════════════════════════════════════════════
// Helper
// ═══════════════════════════════════════════════════════════

fn invoke_with_this(vm: &mut Vm, func: &JsValue, this_val: &JsValue, args: &[JsValue]) -> JsValue {
    match func {
        JsValue::Function(func_rc) => {
            let kind = func_rc.borrow().kind.clone();
            let this_bind = func_rc.borrow().this_binding.clone();
            let captured_upvalues = func_rc.borrow().upvalues.clone();
            let bound_args = func_rc.borrow().bound_args.clone();

            let effective_this = this_bind.unwrap_or_else(|| this_val.clone());
            vm.current_this = effective_this.clone();

            let effective_args: Vec<JsValue>;
            let args = if bound_args.is_empty() {
                args
            } else {
                effective_args = bound_args.into_iter().chain(args.iter().cloned()).collect();
                &effective_args
            };

            match kind {
                FnKind::Native(native) => native(vm, args),
                FnKind::Bytecode(chunk) => {
                    let mut locals = super::Vm::make_locals(&chunk);
                    for (i, arg) in args.iter().enumerate() {
                        if i < locals.len() {
                            locals[i].set(arg.clone());
                        }
                    }
                    let frame = super::CallFrame {
                        chunk,
                        ip: 0,
                        stack_base: vm.stack.len(),
                        locals,
                        upvalue_cells: captured_upvalues,
                        this_val: effective_this,
                        is_constructor: false,
                        all_args: args.to_vec(),
                        self_ref: func.clone(),
                    };
                    vm.frames.push(frame);
                    vm.run()
                }
            }
        }
        _ => {
            vm.log_engine("[libjs] WARN: call/apply on non-function");
            JsValue::Undefined
        }
    }
}
