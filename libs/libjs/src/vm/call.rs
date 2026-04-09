//! Function call, method call, and constructor handling.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{CallFrame, LocalSlot, Vm};
use crate::value::*;

impl Vm {
    fn materialize_prototype_object_from_value(
        &mut self,
        value: &JsValue,
    ) -> Option<Rc<RefCell<JsObject>>> {
        match value {
            JsValue::Object(obj_rc) => Some(obj_rc.clone()),
            JsValue::Array(arr_rc) => {
                let arr = arr_rc.borrow();
                let mut obj = JsObject::new();
                obj.prototype = Some(self.array_proto.clone());
                obj.properties.insert(
                    String::from("length"),
                    Property::data(JsValue::Number(arr.length as f64)),
                );
                for (&idx, val) in &arr.elements {
                    obj.properties
                        .insert(idx.to_string(), Property::data(val.clone()));
                }
                for (key, prop) in &arr.properties {
                    obj.properties.insert(key.clone(), prop.clone());
                }
                Some(Rc::new(RefCell::new(obj)))
            }
            JsValue::Function(func_rc) => {
                let func = func_rc.borrow();
                let mut obj = JsObject::new();
                obj.prototype = Some(self.function_proto.clone());
                for (key, val) in &func.own_props {
                    obj.properties.insert(key.clone(), Property::data(val.clone()));
                }
                Some(Rc::new(RefCell::new(obj)))
            }
            _ => None,
        }
    }

    fn is_constructable_value(&mut self, value: &JsValue) -> bool {
        match value {
            JsValue::Function(func_rc) => {
                let func = func_rc.borrow();
                match &func.kind {
                    FnKind::Bytecode(chunk) => !chunk.is_arrow && !chunk.is_generator,
                    FnKind::Native(_) => matches!(
                        func.own_props.get("__constructable__"),
                        Some(JsValue::Bool(true))
                    ),
                }
            }
            JsValue::Object(obj_rc)
                if obj_rc.borrow().internal_tag.as_deref()
                    == Some(super::native_proxy::PROXY_TAG) =>
            {
                super::native_proxy::proxy_target(value)
                    .map(|target| self.is_constructable_value(&target))
                    .unwrap_or(false)
            }
            _ => false,
        }
    }

    fn unwrap_namespace_constructor_candidate(
        &mut self,
        value: &JsValue,
        preferred_prop: Option<&str>,
        depth: usize,
    ) -> Option<JsValue> {
        if depth == 0 {
            return None;
        }
        if self.is_constructable_value(value) {
            return Some(value.clone());
        }

        if let Some(prop_name) = preferred_prop {
            let candidate = self.get_property_invoking_getter(value, prop_name);
            if self.is_constructable_value(&candidate) {
                return Some(candidate);
            }
            if let Some(unwrapped) =
                self.unwrap_namespace_constructor_candidate(&candidate, None, depth - 1)
            {
                return Some(unwrapped);
            }
        }

        let keys = match value {
            JsValue::Object(obj) => {
                let mut keys = obj.borrow().keys();
                keys.sort();
                keys
            }
            _ => return None,
        };

        if keys.len() != 1 {
            return None;
        }

        let candidate = self.get_property_invoking_getter(value, &keys[0]);
        if self.is_constructable_value(&candidate) {
            return Some(candidate);
        }
        self.unwrap_namespace_constructor_candidate(&candidate, None, depth - 1)
    }

    fn constructor_prototype_from_value(
        &mut self,
        value: &JsValue,
    ) -> Option<Rc<RefCell<JsObject>>> {
        match value {
            JsValue::Function(func_rc) => {
                let maybe_proto = {
                    let f = func_rc.borrow();
                    if let Some(proto_val) = f.own_props.get("prototype") {
                        match proto_val {
                            JsValue::Object(proto_obj) => Some(proto_obj.clone()),
                            JsValue::Array(_) | JsValue::Function(_) => None,
                            _ => Some(self.object_proto.clone()),
                        }
                    } else if matches!(
                        &f.kind,
                        FnKind::Native(_) if !matches!(
                            f.own_props.get("__constructable__"),
                            Some(JsValue::Bool(true))
                        )
                    ) {
                        None
                    } else {
                        f.prototype.clone()
                    }
                };
                if maybe_proto.is_some() {
                    return maybe_proto;
                }

                let own_proto_val = {
                    let f = func_rc.borrow();
                    f.own_props.get("prototype").cloned()
                };
                if let Some(proto_val @ (JsValue::Array(_) | JsValue::Function(_))) = own_proto_val {
                    if let Some(proto_obj) = self.materialize_prototype_object_from_value(&proto_val)
                    {
                        func_rc.borrow_mut().prototype = Some(proto_obj.clone());
                        return Some(proto_obj);
                    }
                }

                let proto = Rc::new(RefCell::new(JsObject::new()));
                proto.borrow_mut().set(
                    String::from("constructor"),
                    JsValue::Function(func_rc.clone()),
                );
                func_rc.borrow_mut().prototype = Some(proto.clone());
                Some(proto)
            }
            JsValue::Object(obj_rc)
                if obj_rc.borrow().internal_tag.as_deref()
                    == Some(super::native_proxy::PROXY_TAG) =>
            {
                super::native_proxy::proxy_target(value)
                    .and_then(|target| self.constructor_prototype_from_value(&target))
            }
            _ => None,
        }
    }

    pub(super) fn construct_with_new_target(
        &mut self,
        ctor: &JsValue,
        args: &[JsValue],
        new_target: &JsValue,
    ) -> bool {
        if !self.is_constructable_value(new_target) {
            let exc = self.make_type_error("newTarget is not a constructor");
            if !self.handle_exception(exc) {
                self.stack.push(JsValue::Undefined);
            }
            return true;
        }
        let mut ctor = ctor.clone();
        loop {
            match ctor {
                JsValue::Function(func_rc) => {
                    let kind = func_rc.borrow().kind.clone();
                    let constructable = {
                        let f = func_rc.borrow();
                        match &f.kind {
                            FnKind::Bytecode(chunk) => !chunk.is_arrow && !chunk.is_generator,
                            FnKind::Native(_) => matches!(
                                f.own_props.get("__constructable__"),
                                Some(JsValue::Bool(true))
                            ),
                        }
                    };
                    if !constructable {
                        let name = func_rc.borrow().name.clone().unwrap_or_default();
                        let msg = alloc::format!(
                            "{} is not a constructor",
                            if name.is_empty() {
                                "(intermediate value)".into()
                            } else {
                                name
                            }
                        );
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            self.stack.push(JsValue::Undefined);
                        }
                        return true;
                    }

                    let is_arrow = match &kind {
                        FnKind::Bytecode(chunk) => chunk.is_arrow,
                        _ => false,
                    };
                    // Arrow functions and generators cannot be constructors (ES2023 §15.4.1)
                    let is_generator = match &kind {
                        FnKind::Bytecode(chunk) => chunk.is_generator,
                        _ => false,
                    };
                    if is_arrow || is_generator {
                        let name = func_rc.borrow().name.clone().unwrap_or_default();
                        let msg = alloc::format!(
                            "{} is not a constructor",
                            if name.is_empty() {
                                "(intermediate value)".into()
                            } else {
                                name
                            }
                        );
                        let exc = self.make_type_error(&msg);
                        if !self.handle_exception(exc) {
                            self.stack.push(JsValue::Undefined);
                        }
                        return true;
                    }

                    let ctor_proto = self
                        .constructor_prototype_from_value(new_target)
                        .or(Some(self.object_proto.clone()));
                    let new_obj = JsValue::Object(Rc::new(RefCell::new(JsObject {
                        properties: alloc::collections::BTreeMap::new(),
                        prototype: ctor_proto,
                        internal_tag: None,
                        primitive_value: None,
                        set_hook: None,
                        set_hook_data: core::ptr::null_mut(),
                    })));

                    self.current_this = new_obj.clone();

                    match kind {
                        FnKind::Native(native_fn) => {
                            self.native_constructor_depth += 1;
                            let result = native_fn(self, args);
                            self.native_constructor_depth -= 1;
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
                            let mut locals = Vm::make_locals(&chunk);
                            for (i, arg) in args.iter().enumerate() {
                                if i < locals.len() {
                                    locals[i].set(arg.clone());
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
                    return true;
                }
                JsValue::Object(ref obj_rc)
                    if obj_rc.borrow().internal_tag.as_deref()
                        == Some(super::native_proxy::PROXY_TAG) =>
                {
                    if let Some(result) = super::native_proxy::proxy_construct(self, &ctor, args) {
                        self.stack.push(result);
                        return true;
                    }
                    if let Some(target) = super::native_proxy::proxy_target(&ctor) {
                        ctor = target;
                        continue;
                    }
                }
                _ => return false,
            }
        }
    }

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

    /// Maximum call depth to prevent stack overflow (Rust stack).
    const MAX_CALL_DEPTH: usize = 100;

    /// Invoke a function value with the given arguments and this binding.
    pub(super) fn invoke_function(
        &mut self,
        callee: &JsValue,
        args: &[JsValue],
        this_val: JsValue,
    ) {
        // Guard against stack overflow from deep recursion
        if self.frames.len() >= Self::MAX_CALL_DEPTH {
            let err = self.make_range_error("Maximum call stack size exceeded");
            if !self.handle_exception(err) {
                self.stack.push(JsValue::Undefined);
            }
            return;
        }
        match callee {
            JsValue::Function(func_rc) => {
                // Extract what we need before any mutable VM operations
                let kind = func_rc.borrow().kind.clone();
                let this_bind = func_rc.borrow().this_binding.clone();
                let captured_upvalues = func_rc.borrow().upvalues.clone();
                let bound_args = func_rc.borrow().bound_args.clone();

                // ES2023 §15.7.1: Class constructors cannot be invoked without `new`.
                let is_class_ctor = match &kind {
                    FnKind::Bytecode(chunk) => chunk.is_class_constructor,
                    _ => false,
                };
                if is_class_ctor {
                    let name = func_rc.borrow().name.clone().unwrap_or_default();
                    let msg = alloc::format!(
                        "Class constructor {} cannot be invoked without 'new'",
                        if name.is_empty() { "(anonymous)".into() } else { name }
                    );
                    let exc = self.make_type_error(&msg);
                    if !self.handle_exception(exc) {
                        self.stack.push(JsValue::Undefined);
                    }
                    return;
                }

                let effective_this = this_bind.unwrap_or(this_val);
                // ES2023 §10.2.1.1: In sloppy mode, if `this` is undefined/null,
                // coerce to the global object (the "this substitution" rule).
                // Arrow functions don't have their own `this` — they inherit from
                // the enclosing scope via `this_binding`, so this only applies
                // to regular functions.
                let is_arrow = match &kind {
                    FnKind::Bytecode(c) => c.is_arrow,
                    _ => false,
                };
                let is_strict_fn = match &kind {
                    FnKind::Bytecode(c) => c.strict,
                    _ => false,
                };
                let effective_this = if !is_arrow
                    && !is_strict_fn
                    && matches!(effective_this, JsValue::Undefined | JsValue::Null)
                {
                    JsValue::Object(self.globals.clone())
                } else {
                    effective_this
                };
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
                            } else {
                                // Control transferred into a JS catch handler.
                                // Internal helpers must stop normal execution
                                // without disturbing the handler-prepared stack.
                                self.control_flow_restored = true;
                            }
                        } else {
                            self.stack.push(result);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let mut locals = Vm::make_locals(&chunk);
                        for (i, arg) in args.iter().enumerate() {
                            if i < locals.len() {
                                locals[i].set(arg.clone());
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
            JsValue::Object(ref obj_rc)
                if obj_rc.borrow().internal_tag.as_deref()
                    == Some(super::native_proxy::PROXY_TAG) =>
            {
                if let Some(result) =
                    super::native_proxy::proxy_apply(self, callee, &this_val, args)
                {
                    self.stack.push(result);
                } else if let Some(target) = super::native_proxy::proxy_target(callee) {
                    self.invoke_function(&target, args, this_val);
                } else {
                    let exc = self.make_type_error("is not a function");
                    if !self.handle_exception(exc) {
                        self.stack.push(JsValue::Undefined);
                    }
                }
            }
            _other => {
                // Try to extract the method name from the current call frame's bytecode
                let mut context = alloc::string::String::new();
                if let Some(frame) = self.frames.last() {
                    let ip = frame.ip;
                    let code = &frame.chunk.code;
                    let consts = &frame.chunk.constants;
                    if ip >= 2 {
                        for back in 1..ip.min(10) {
                            let check_ip = ip - back;
                            if check_ip < code.len() {
                                if let crate::bytecode::Op::GetPropNamed(ci) = &code[check_ip] {
                                    if let Some(crate::bytecode::Constant::String(s)) =
                                        consts.get(*ci as usize)
                                    {
                                        // Show the `this` object type for method calls
                                        let this_desc = match &this_val {
                                            JsValue::Empty => "empty",
                                            JsValue::Undefined => "undefined",
                                            JsValue::Null => "null",
                                            JsValue::Number(_) => "number",
                                            JsValue::String(_) => "string",
                                            JsValue::Bool(_) => "bool",
                                            JsValue::Object(o) => {
                                                let b = o.borrow();
                                                if b.has("__nodeId") {
                                                    "DOMElement"
                                                } else if b.internal_tag.as_deref() == Some("Map") {
                                                    "Map"
                                                } else if b.internal_tag.as_deref() == Some("Set") {
                                                    "Set"
                                                } else {
                                                    "object"
                                                }
                                            }
                                            JsValue::Array(_) => "array",
                                            JsValue::Function(_) => "function",
                                            JsValue::BigInt(_) => "bigint",
                                        };
                                        context = alloc::format!(" {}.{}()", this_desc, s);
                                        break;
                                    }
                                }
                                if let crate::bytecode::Op::LoadGlobal(ci) = &code[check_ip] {
                                    if let Some(crate::bytecode::Constant::String(s)) =
                                        consts.get(*ci as usize)
                                    {
                                        context = alloc::format!(" global={}", s);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                let mut stack = alloc::string::String::new();
                for (fi, frame) in self.frames.iter().rev().take(6).enumerate() {
                    let fname = frame.chunk.name.as_deref().unwrap_or("(anon)");
                    if fi > 0 {
                        stack.push_str(" <- ");
                    }
                    stack.push_str(fname);
                }
                // ES2023 §7.3.13: TypeError when calling a non-callable value.
                let desc = if context.is_empty() {
                    alloc::format!("{} is not a function", callee.type_of())
                } else {
                    alloc::format!("{} is not a function", context.trim())
                };
                let exc = self.make_type_error(&desc);
                if !self.handle_exception(exc) {
                    self.stack.push(JsValue::Undefined);
                }
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

        if self.construct_with_new_target(&ctor, &args, &ctor) {
            return;
        }

        {
            let mut context = alloc::string::String::new();
            let mut fallback_prop: Option<alloc::string::String> = None;
            if let Some(frame) = self.frames.last() {
                let ip = frame.ip;
                let code = &frame.chunk.code;
                let consts = &frame.chunk.constants;
                if ip >= 1 {
                    for back in 1..ip.min(10) {
                        let check_ip = ip - back;
                        if check_ip < code.len() {
                            if let crate::bytecode::Op::LoadGlobal(ci) = &code[check_ip] {
                                if let Some(crate::bytecode::Constant::String(s)) =
                                    consts.get(*ci as usize)
                                {
                                    context = alloc::format!(" global={}", s);
                                    break;
                                }
                            }
                            if let crate::bytecode::Op::GetPropNamed(ci) = &code[check_ip] {
                                if let Some(crate::bytecode::Constant::String(s)) =
                                    consts.get(*ci as usize)
                                {
                                    context = alloc::format!(" prop={}", s);
                                    fallback_prop = Some(s.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if let Some(maybe_ctor) =
                self.unwrap_namespace_constructor_candidate(&ctor, fallback_prop.as_deref(), 4)
            {
                if self.construct_with_new_target(&maybe_ctor, &args, &maybe_ctor) {
                    self.log_engine("[libjs] recovered constructor via namespace wrapper");
                    return;
                }
            }

            let value_type = match &ctor {
                JsValue::Empty => "empty",
                JsValue::Undefined => "undefined",
                JsValue::Null => "null",
                JsValue::Bool(_) => "bool",
                JsValue::Number(_) => "number",
                JsValue::String(_) => "string",
                JsValue::Object(obj) => {
                    let b = obj.borrow();
                    let mut keys = b.keys();
                    keys.sort();
                    let preview: alloc::vec::Vec<alloc::string::String> =
                        keys.into_iter().take(6).collect();
                    self.log_engine(&alloc::format!(
                        "[libjs] WARN: constructor target object keys={:?}{}",
                        preview,
                        context
                    ));
                    if let Some(tag) = &b.internal_tag {
                        self.log_engine(&alloc::format!(
                            "[libjs] WARN: constructor target object tag={}{}",
                            tag,
                            context
                        ));
                    }
                    "object"
                }
                JsValue::Array(_) => "array",
                JsValue::Function(_) => "function",
                JsValue::BigInt(_) => "bigint",
            };
            self.log_engine(&alloc::format!(
                "[libjs] WARN: new on non-constructor type={}{}",
                value_type,
                context
            ));
            let exc = self.make_type_error("is not a constructor");
            if !self.handle_exception(exc) {
                self.stack.push(JsValue::Undefined);
            }
            return;
        }
    }

    /// Call a JS function value directly from Rust.
    /// Handles both native and bytecode functions, including re-entrant execution.
    pub fn call_value(&mut self, callee: &JsValue, args: &[JsValue], this_val: JsValue) -> JsValue {
        // Guard against Rust-level stack overflow from re-entrant run() calls.
        // Each run() frame is large (~100KB on the Rust stack), so we limit depth.
        self.call_value_depth += 1;
        if self.call_value_depth > 50 {
            self.call_value_depth -= 1;
            let err = self.make_range_error("Maximum call stack size exceeded (re-entrant)");
            self.last_exception = Some(err);
            return JsValue::Undefined;
        }

        let saved_depth = self.frames.len();
        let stack_before = self.stack.len();
        self.invoke_function(callee, args, this_val);

        // Native function: result already on stack, no new frame pushed.
        if self.frames.len() <= saved_depth {
            self.call_value_depth -= 1;
            if self.control_flow_restored {
                self.control_flow_restored = false;
                return JsValue::Empty;
            }
            let result = self.stack.pop().unwrap_or(JsValue::Undefined);
            return result;
        }

        // Bytecode function: run until we're back to saved depth.
        let prev_target = self.run_target_depth;
        self.run_target_depth = saved_depth;
        let result = self.run();
        self.run_target_depth = prev_target;
        self.call_value_depth -= 1;
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

        let mut super_ctor = self.stack[ctor_idx].clone();
        let args: Vec<JsValue> = self.stack[args_start..].to_vec();
        self.stack.truncate(ctor_idx);

        // If stack value isn't a function, try the constructor's super_class field.
        // This handles cases where the upvalue for $$super$$ was lost (e.g. timer callbacks).
        if !matches!(super_ctor, JsValue::Function(_)) {
            for f in self.frames.iter().rev() {
                if f.is_constructor {
                    if let JsValue::Function(ref ctor_fn) = f.self_ref {
                        if let Some(ref sc) = ctor_fn.borrow().super_class {
                            super_ctor = sc.clone();
                        }
                    }
                    break;
                }
            }
        }

        // Find the current new.target from the enclosing constructor frame.
        let mut new_target = JsValue::Undefined;
        for f in self.frames.iter().rev() {
            if f.is_constructor {
                new_target = f.self_ref.clone();
                break;
            }
        }

        // Get `this` from the current constructor frame.
        let this_val = self
            .frames
            .last()
            .map(|f| f.this_val.clone())
            .unwrap_or(JsValue::Undefined);

        match super_ctor {
            JsValue::Function(func_rc) => {
                let kind = func_rc.borrow().kind.clone();
                let captured_upvalues = func_rc.borrow().upvalues.clone();

                self.current_this = this_val.clone();
                match kind {
                    FnKind::Native(native_fn) => {
                        self.native_constructor_depth += 1;
                        let result = native_fn(self, &args);
                        self.native_constructor_depth -= 1;
                        if let Some(exc) = self.pending_exception.take() {
                            if !self.handle_exception(exc) {
                                self.stack.push(JsValue::Undefined);
                            }
                        } else {
                            self.stack.push(result);
                        }
                    }
                    FnKind::Bytecode(chunk) => {
                        let mut locals = Vm::make_locals(&chunk);
                        for (i, arg) in args.iter().enumerate() {
                            if i < locals.len() {
                                locals[i].set(arg.clone());
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
                let mut stack_info = alloc::string::String::new();
                for (fi, frame) in self.frames.iter().rev().take(6).enumerate() {
                    let fname = frame.chunk.name.as_deref().unwrap_or("(anon)");
                    if fi > 0 {
                        stack_info.push_str(" <- ");
                    }
                    stack_info.push_str(fname);
                }
                let super_type = match &super_ctor {
                    JsValue::Undefined => "undefined",
                    JsValue::Null => "null",
                    JsValue::Object(_) => "object",
                    JsValue::String(_) => "string",
                    JsValue::Number(_) => "number",
                    JsValue::Bool(_) => "bool",
                    _ => "other",
                };
                self.log_engine(&alloc::format!(
                    "[libjs] WARN: super() called on non-function (got {}) [{}]",
                    super_type,
                    stack_info
                ));
                self.stack.push(JsValue::Undefined);
            }
        }
    }

    /// `left instanceof right` — walks the left-hand prototype chain and
    /// compares it against `right.prototype`.
    pub fn instance_of(&self, left: &JsValue, right: &JsValue) -> bool {
        let target_proto = match right {
            JsValue::Function(func) => {
                let f = func.borrow();
                if let Some(proto_val) = f.own_props.get("prototype") {
                    match proto_val {
                        JsValue::Object(proto_obj) => Some(proto_obj.clone()),
                        JsValue::Array(_) | JsValue::Function(_) => f.prototype.clone(),
                        _ => Some(self.object_proto.clone()),
                    }
                } else {
                    f.prototype.clone()
                }
            }
            _ => None,
        };
        let Some(target_proto) = target_proto else {
            return false;
        };

        let mut current_proto = match left {
            JsValue::Object(obj) => obj.borrow().prototype.clone(),
            JsValue::Array(_) => Some(self.array_proto.clone()),
            JsValue::Function(func) => func.borrow().prototype.clone(),
            _ => None,
        };

        while let Some(proto) = current_proto {
            if Rc::ptr_eq(&proto, &target_proto) {
                return true;
            }
            current_proto = proto.borrow().prototype.clone();
        }

        false
    }
}
