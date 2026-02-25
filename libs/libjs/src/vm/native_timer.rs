//! setTimeout / setInterval / clearTimeout / clearInterval.
//!
//! In the libjs core these are stubs that record pending timers.
//! The host environment (libwebview) can consume them via the
//! `pending_timers` field on the VM, or override them with real
//! native implementations that hook into an event loop.

use alloc::string::String;

use crate::value::*;
use super::Vm;

// Timer ID counter — simple global.
static mut NEXT_TIMER_ID: f64 = 1.0;

/// `setTimeout(callback, delay)` — stub that returns a timer ID.
pub fn set_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let id = unsafe {
        let id = NEXT_TIMER_ID;
        NEXT_TIMER_ID += 1.0;
        id
    };
    JsValue::Number(id)
}

/// `setInterval(callback, delay)` — stub that returns a timer ID.
pub fn set_interval(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    let id = unsafe {
        let id = NEXT_TIMER_ID;
        NEXT_TIMER_ID += 1.0;
        id
    };
    JsValue::Number(id)
}

/// `clearTimeout(id)` — stub.
pub fn clear_timeout(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}

/// `clearInterval(id)` — stub.
pub fn clear_interval(_vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    JsValue::Undefined
}
