//! Typed event system — closure registry + event definitions.
//!
//! Events are organized into:
//! - `shared/` — event types used by multiple controls (ClickEvent, ValueChangedEvent, etc.)
//! - Control-specific files — events unique to a single control (ColorSelectedEvent, etc.)

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

pub mod shared;
mod color;

// Re-export all event types at the events:: level
pub use shared::*;
pub use color::ColorSelectedEvent;

// ══════════════════════════════════════════════════════════════════════
//  Closure Registry
// ══════════════════════════════════════════════════════════════════════

type AnyHandler = Box<dyn FnMut(u32, u32)>; // (control_id, event_type)

static mut HANDLERS: Option<Vec<AnyHandler>> = None;

fn handlers() -> &'static mut Vec<AnyHandler> {
    unsafe { HANDLERS.get_or_insert_with(Vec::new) }
}

/// FFI thunk: dispatches to the registered closure at `HANDLERS[userdata]`.
pub(crate) extern "C" fn closure_thunk(id: u32, event_type: u32, userdata: u64) {
    let idx = userdata as usize;
    let h = handlers();
    if idx < h.len() {
        h[idx](id, event_type);
    }
}

/// Register a closure, returning the (thunk, userdata) pair for FFI registration.
pub(crate) fn register(f: impl FnMut(u32, u32) + 'static) -> (crate::Callback, u64) {
    let idx = handlers().len();
    handlers().push(Box::new(f));
    (closure_thunk, idx as u64)
}
