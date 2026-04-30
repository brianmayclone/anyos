// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Generator object and protocol implementation.
//!
//! A generator function (`function*`) returns a GeneratorObject.
//! The GeneratorObject stores a suspended call frame (chunk, ip, locals, stack).
//! Calling `.next(value)` resumes execution from where `yield` suspended it.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{LocalSlot, Vm};
use crate::bytecode::Chunk;
use crate::value::*;

/// Internal tag for generator objects.
pub const GENERATOR_TAG: &str = "__generator__";

/// Generator state.
#[derive(Debug, Clone, PartialEq)]
pub enum GeneratorState {
    /// Created but `.next()` not yet called.
    Suspended,
    /// Currently executing (re-entrancy guard).
    Executing,
    /// Finished (returned or threw).
    Completed,
}

/// Suspended generator frame state.
#[derive(Clone)]
pub struct GeneratorFrame {
    pub chunk: Rc<Chunk>,
    pub ip: usize,
    pub locals: Vec<LocalSlot>,
    pub upvalue_cells: Vec<Rc<RefCell<JsValue>>>,
    pub this_val: JsValue,
    pub stack_snapshot: Vec<JsValue>,
    pub state: GeneratorState,
}

/// Key used to store GeneratorFrame index in hidden property.
const GEN_FRAME_KEY: &str = "__gen_id__";

/// Global storage for generator frames (keyed by sequential ID).
/// We use a simple Vec; the index is stored in the generator object.
static mut GENERATOR_FRAMES: Option<Vec<Option<GeneratorFrame>>> = None;

fn frames() -> &'static mut Vec<Option<GeneratorFrame>> {
    unsafe {
        if GENERATOR_FRAMES.is_none() {
            GENERATOR_FRAMES = Some(Vec::new());
        }
        GENERATOR_FRAMES.as_mut().unwrap()
    }
}

/// Allocate a generator frame and return its ID.
pub fn alloc_frame(frame: GeneratorFrame) -> u32 {
    let fs = frames();
    // Reuse a freed slot if available
    for (i, slot) in fs.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(frame);
            return i as u32;
        }
    }
    let id = fs.len() as u32;
    fs.push(Some(frame));
    id
}

/// Get a mutable reference to a generator frame.
pub fn get_frame(id: u32) -> Option<&'static mut GeneratorFrame> {
    let fs = frames();
    fs.get_mut(id as usize).and_then(|s| s.as_mut())
}

/// Free a generator frame.
pub fn free_frame(id: u32) {
    let fs = frames();
    if let Some(slot) = fs.get_mut(id as usize) {
        *slot = None;
    }
}

/// Create a generator object from a generator function call.
pub fn create_generator_object(
    vm: &Vm,
    chunk: Rc<Chunk>,
    start_ip: usize,
    locals: Vec<LocalSlot>,
    upvalue_cells: Vec<Rc<RefCell<JsValue>>>,
    this_val: JsValue,
    stack_snapshot: Vec<JsValue>,
) -> JsValue {
    let frame = GeneratorFrame {
        chunk,
        ip: start_ip,
        locals,
        upvalue_cells,
        this_val,
        stack_snapshot,
        state: GeneratorState::Suspended,
    };
    let id = alloc_frame(frame);

    let mut obj = JsObject::with_tag(GENERATOR_TAG);
    obj.prototype = Some(vm.generator_proto.clone());
    obj.set_hidden(String::from(GEN_FRAME_KEY), JsValue::Number(id as f64));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

fn get_gen_id(this: &JsValue) -> Option<u32> {
    if let JsValue::Object(obj) = this {
        let o = obj.borrow();
        if o.internal_tag.as_deref() != Some(GENERATOR_TAG) {
            return None;
        }
        if let JsValue::Number(n) = o.get(GEN_FRAME_KEY) {
            return Some(n as u32);
        }
    }
    None
}

/// Build `{ value, done }` iterator result.
fn iter_result(vm: &Vm, value: JsValue, done: bool) -> JsValue {
    let mut obj = JsObject::new();
    obj.prototype = Some(vm.object_proto.clone());
    obj.set(String::from("value"), value);
    obj.set(String::from("done"), JsValue::Bool(done));
    JsValue::Object(Rc::new(RefCell::new(obj)))
}

/// `Generator.prototype.next(value)`
pub fn generator_next(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let send_val = args.first().cloned().unwrap_or(JsValue::Undefined);

    let gen_id = match get_gen_id(&this) {
        Some(id) => id,
        None => return iter_result(vm, JsValue::Undefined, true),
    };

    let frame = match get_frame(gen_id) {
        Some(f) => f,
        None => return iter_result(vm, JsValue::Undefined, true),
    };

    match frame.state {
        GeneratorState::Completed => {
            return iter_result(vm, JsValue::Undefined, true);
        }
        GeneratorState::Executing => {
            return iter_result(vm, JsValue::Undefined, true);
        }
        GeneratorState::Suspended => {}
    }

    // Resume generator execution
    frame.state = GeneratorState::Executing;

    // Clone what we need from the frame
    let chunk = frame.chunk.clone();
    let ip = frame.ip;
    let locals = frame.locals.clone();
    let upvalue_cells = frame.upvalue_cells.clone();
    let this_val = frame.this_val.clone();
    let stack_snapshot = frame.stack_snapshot.clone();

    // Run the generator's bytecode in the VM from the saved IP
    let result = vm.run_generator_step(
        chunk,
        ip,
        locals,
        upvalue_cells,
        this_val,
        stack_snapshot,
        send_val,
    );

    match result {
        GeneratorResult::Yielded {
            value,
            ip: new_ip,
            locals: new_locals,
            stack: new_stack,
        } => {
            // Save the new state
            if let Some(frame) = get_frame(gen_id) {
                frame.ip = new_ip;
                frame.locals = new_locals;
                frame.stack_snapshot = new_stack;
                frame.state = GeneratorState::Suspended;
            }
            iter_result(vm, value, false)
        }
        GeneratorResult::Returned(value) => {
            if let Some(frame) = get_frame(gen_id) {
                frame.state = GeneratorState::Completed;
            }
            iter_result(vm, value, true)
        }
        GeneratorResult::Threw(err) => {
            if let Some(frame) = get_frame(gen_id) {
                frame.state = GeneratorState::Completed;
            }
            vm.throw_native(err);
            JsValue::Undefined
        }
    }
}

/// `Generator.prototype.return(value)`
pub fn generator_return(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);

    if let Some(gen_id) = get_gen_id(&this) {
        if let Some(frame) = get_frame(gen_id) {
            frame.state = GeneratorState::Completed;
        }
    }
    iter_result(vm, value, true)
}

/// `Generator.prototype.throw(exception)`
///
/// Resumes the generator at the yield point and throws the exception there,
/// so that try-catch inside the generator can catch it (ES2023 §27.5.3.4).
pub fn generator_throw(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let this = vm.current_this.clone();
    let err = args.first().cloned().unwrap_or(JsValue::Undefined);

    let gen_id = match get_gen_id(&this) {
        Some(id) => id,
        None => {
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    };

    let frame = match get_frame(gen_id) {
        Some(f) => f,
        None => {
            vm.throw_native(err);
            return JsValue::Undefined;
        }
    };

    match frame.state {
        GeneratorState::Completed => {
            // Already completed — propagate as unhandled.
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        GeneratorState::Executing => {
            vm.throw_native(err);
            return JsValue::Undefined;
        }
        GeneratorState::Suspended => {}
    }

    // Resume the generator, but pre-set pending_exception so the VM will
    // throw at the yield resume point, giving try-catch a chance to catch it.
    frame.state = GeneratorState::Executing;
    let chunk = frame.chunk.clone();
    let ip = frame.ip;
    let locals = frame.locals.clone();
    let upvalue_cells = frame.upvalue_cells.clone();
    let this_val = frame.this_val.clone();
    let stack_snapshot = frame.stack_snapshot.clone();

    // Set the exception BEFORE resuming so the VM picks it up immediately.
    vm.pending_exception = Some(err);

    let result = vm.run_generator_step(
        chunk,
        ip,
        locals,
        upvalue_cells,
        this_val,
        stack_snapshot,
        JsValue::Undefined,
    );

    match result {
        GeneratorResult::Yielded {
            value,
            ip: new_ip,
            locals: new_locals,
            stack: new_stack,
        } => {
            if let Some(frame) = get_frame(gen_id) {
                frame.ip = new_ip;
                frame.locals = new_locals;
                frame.stack_snapshot = new_stack;
                frame.state = GeneratorState::Suspended;
            }
            iter_result(vm, value, false)
        }
        GeneratorResult::Returned(value) => {
            if let Some(frame) = get_frame(gen_id) {
                frame.state = GeneratorState::Completed;
            }
            iter_result(vm, value, true)
        }
        GeneratorResult::Threw(thrown) => {
            if let Some(frame) = get_frame(gen_id) {
                frame.state = GeneratorState::Completed;
            }
            vm.throw_native(thrown);
            JsValue::Undefined
        }
    }
}

/// Result of running a generator step.
pub enum GeneratorResult {
    /// Hit a `yield` — save state and return value.
    Yielded {
        value: JsValue,
        ip: usize,
        locals: Vec<LocalSlot>,
        stack: Vec<JsValue>,
    },
    /// Hit a `return` — generator is done.
    Returned(JsValue),
    /// Hit an unhandled exception.
    Threw(JsValue),
}
