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
        Some(JsValue::Object(_)) => array_like_to_vec(args.get(1).unwrap()),
        Some(JsValue::String(s)) => s
            .chars()
            .map(|ch| {
                let mut out = String::new();
                out.push(ch);
                JsValue::String(out)
            })
            .collect(),
        Some(JsValue::Null | JsValue::Undefined) | None => Vec::new(),
        _ => Vec::new(),
    };

    invoke_with_this(vm, &func, &this_arg, &call_args)
}

fn array_like_to_vec(value: &JsValue) -> Vec<JsValue> {
    let len = value.get_property("length").to_number().max(0.0) as usize;
    let mut out = Vec::new();
    for idx in 0..len {
        out.push(value.get_property(&format!("{}", idx)));
    }
    out
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
    // We clone the original function, install the bound `this` binding, and
    // store the bound argument list — `invoke_with_this` below prepends them
    // to the call-site arguments for both native and bytecode functions.
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
                object_proto: original.object_proto.clone(),
                this_binding: Some(bound_this),
                bound_args: bound_args,
                upvalues: original.upvalues.clone(),
                with_scopes: original.with_scopes.clone(),
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
            let saved_current_this = vm.current_this.clone();
            vm.current_this = effective_this.clone();

            let effective_args: Vec<JsValue>;
            let args = if bound_args.is_empty() {
                args
            } else {
                effective_args = bound_args.into_iter().chain(args.iter().cloned()).collect();
                &effective_args
            };

            match kind {
                FnKind::Native(native) => {
                    let result = native(vm, args);
                    vm.current_this = saved_current_this;
                    result
                }
                FnKind::Bytecode(chunk) => {
                    #[cfg(feature = "host")]
                    if std::env::var_os("LIBJS_DEBUG_CALLS").is_some() && chunk.code.len() > 1000 {
                        eprintln!(
                            "[libjs-call] function.call/apply bytecode name={} ops={} args={} this={}",
                            chunk.name.as_deref().unwrap_or("<anon>"),
                            chunk.code.len(),
                            args.len(),
                            effective_this.type_of()
                        );
                    }
                    let captured_with_scopes = match &func {
                        JsValue::Function(f) => f.borrow().with_scopes.clone(),
                        _ => Vec::new(),
                    };
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
                        with_scopes: captured_with_scopes,
                        captured_with_scope_len: match &func {
                            JsValue::Function(f) => f.borrow().with_scopes.len(),
                            _ => 0,
                        },
                        this_val: effective_this,
                        is_constructor: false,
                        all_args: args.to_vec(),
                        self_ref: func.clone(),
                        new_target: JsValue::Undefined,
                    };
                    vm.frames.push(frame);
                    vm.current_this = saved_current_this;
                    JsValue::Empty
                }
            }
        }
        _ => {
            vm.log_engine("[libjs] WARN: call/apply on non-function");
            JsValue::Undefined
        }
    }
}
