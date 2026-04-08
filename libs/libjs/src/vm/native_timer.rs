//! setTimeout / setInterval / clearTimeout / clearInterval.
//!
//! These functions register timers in the VM's `EventLoop`.  The host
//! environment (libwebview) can override them with its own natives that
//! hook into a real frame-driven event loop.  When no host overrides are
//! installed (standalone libjs), the timers live in `vm.event_loop` and
//! are advanced by calling `vm.tick(delta_ms)`.

use alloc::vec::Vec;

use super::Vm;
use crate::value::*;

/// `setTimeout(callback, delay)` — schedules a one-shot timer.
pub fn set_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args
        .get(1)
        .map(|v| v.to_number().max(0.0) as u32)
        .unwrap_or(0);
    if !callback.is_function() {
        return JsValue::Number(0.0);
    }
    let id = vm.event_loop.set_timer(callback, delay, false);
    JsValue::Number(id as f64)
}

/// `setInterval(callback, delay)` — schedules a repeating timer.
pub fn set_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    let delay = args
        .get(1)
        .map(|v| v.to_number().max(1.0) as u32)
        .unwrap_or(10);
    if !callback.is_function() {
        return JsValue::Number(0.0);
    }
    let id = vm.event_loop.set_timer(callback, delay, true);
    JsValue::Number(id as f64)
}

/// `clearTimeout(id)` — cancels a one-shot timer.
pub fn clear_timeout(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let id = args.first().map(|v| v.to_number() as u32).unwrap_or(0);
    vm.event_loop.clear_timer(id);
    JsValue::Undefined
}

/// `clearInterval(id)` — cancels a repeating timer.
pub fn clear_interval(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    clear_timeout(vm, args)
}
