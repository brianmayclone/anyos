//! Timer — periodic callbacks fired on the UI thread.
//!
//! Timers are checked once per frame in `run_once()`. When enough time has
//! elapsed since a timer last fired, its callback is added to the pending
//! callback list and invoked in Phase 3 (deferred invocation).
//!
//! # Usage (via client API)
//! ```ignore
//! let id = ui::set_timer(100, || { /* runs every 100ms on UI thread */ });
//! ui::kill_timer(id);
//! ```

use alloc::vec::Vec;
use crate::control::Callback;

/// A single timer slot.
pub struct TimerSlot {
    pub id: u32,
    pub interval_ms: u32,
    pub last_fired_ms: u32,
    pub callback: Callback,
    pub userdata: u64,
}

/// Timer storage, owned by AnyuiState.
pub struct TimerState {
    pub slots: Vec<TimerSlot>,
    next_id: u32,
}

impl TimerState {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            next_id: 1,
        }
    }

    /// Register a new timer. Returns the timer ID (>0).
    pub fn set_timer(&mut self, interval_ms: u32, cb: Callback, userdata: u64) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        let now = crate::syscall::uptime_ms();
        self.slots.push(TimerSlot {
            id,
            interval_ms,
            last_fired_ms: now,
            callback: cb,
            userdata,
        });
        id
    }

    /// Remove a timer by ID. No-op if not found.
    pub fn kill_timer(&mut self, timer_id: u32) {
        self.slots.retain(|t| t.id != timer_id);
    }
}
