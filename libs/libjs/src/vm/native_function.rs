//! Function.prototype methods: call, apply, bind, toString.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use core::cell::RefCell;

use crate::value::*;
use super::Vm;

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
        Some(JsValue::Array(arr)) => arr.borrow().elements.clone(),
        _ => Vec::new(),
    };

    invoke_with_this(vm, &func, &this_arg, &call_args)
}

/// Function.prototype.bind(thisArg, ...args)
pub fn function_bind(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let func = vm.current_this.clone();
    let bound_this = args.first().cloned().unwrap_or(JsValue::Undefined);
    let _bound_args: Vec<JsValue> = if args.len() > 1 {
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
                upvalues: original.upvalues.clone(),
                prototype: original.prototype.clone(),
                own_props: original.own_props.clone(),
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
            vm.current_this = this_val.clone();

            match kind {
                FnKind::Native(native) => native(vm, args),
                FnKind::Bytecode(chunk) => {
                    let captured_upvalues = func_rc.borrow().upvalues.clone();
                    let local_count = chunk.local_count as usize;
                    let mut locals: alloc::vec::Vec<alloc::rc::Rc<core::cell::RefCell<JsValue>>> =
                        (0..local_count).map(|_| alloc::rc::Rc::new(core::cell::RefCell::new(JsValue::Undefined))).collect();
                    for (i, arg) in args.iter().enumerate() {
                        if i < local_count { *locals[i].borrow_mut() = arg.clone(); }
                    }
                    let frame = super::CallFrame {
                        chunk,
                        ip: 0,
                        stack_base: vm.stack.len(),
                        locals,
                        upvalue_cells: captured_upvalues,
                        this_val: this_val.clone(),
                        is_constructor: false,
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
