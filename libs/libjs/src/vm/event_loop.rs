//! Event loop with microtask and macrotask queues.
//!
//! Implements the ECMAScript Job Queue semantics:
//! - **Microtasks** (Promise callbacks, queueMicrotask) run after each macrotask
//!   and before the next macrotask.
//! - **Macrotasks** (setTimeout, setInterval) run one per event loop iteration.
//!
//! The event loop is pumped by calling `drain_microtasks()` after each script
//! evaluation, and `tick()` for timer-based macrotasks.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::value::JsValue;

/// A queued task (microtask or macrotask).
#[derive(Clone)]
pub struct Task {
    /// The callback function to invoke.
    pub callback: JsValue,
    /// Arguments to pass.
    pub args: Vec<JsValue>,
}

/// A scheduled timer (setTimeout/setInterval).
#[derive(Clone)]
pub struct TimerEntry {
    pub id: u32,
    pub callback: JsValue,
    pub args: Vec<JsValue>,
    /// Delay in milliseconds.
    pub delay_ms: u32,
    /// Elapsed time since creation (incremented by tick).
    pub elapsed_ms: u32,
    /// If true, this is a repeating interval timer.
    pub is_interval: bool,
    /// If true, this timer has been cleared.
    pub cleared: bool,
}

/// The event loop state.
pub struct EventLoop {
    /// Microtask queue (FIFO). Drained completely after each macrotask.
    pub microtasks: VecDeque<Task>,
    /// Timer registry.
    pub timers: Vec<TimerEntry>,
    /// Next timer ID.
    pub next_timer_id: u32,
}

impl EventLoop {
    pub fn new() -> Self {
        EventLoop {
            microtasks: VecDeque::new(),
            timers: Vec::new(),
            next_timer_id: 1,
        }
    }

    /// Enqueue a microtask (e.g., Promise .then callback).
    pub fn enqueue_microtask(&mut self, callback: JsValue, args: Vec<JsValue>) {
        self.microtasks.push_back(Task { callback, args });
    }

    /// Register a timer (setTimeout or setInterval). Returns the timer ID.
    pub fn set_timer(&mut self, callback: JsValue, delay_ms: u32, is_interval: bool) -> u32 {
        let id = self.next_timer_id;
        self.next_timer_id += 1;
        self.timers.push(TimerEntry {
            id,
            callback,
            args: Vec::new(),
            delay_ms,
            elapsed_ms: 0,
            is_interval,
            cleared: false,
        });
        id
    }

    /// Clear a timer by ID.
    pub fn clear_timer(&mut self, id: u32) {
        for timer in &mut self.timers {
            if timer.id == id {
                timer.cleared = true;
            }
        }
    }

    /// Dequeue the next microtask, if any.
    pub fn pop_microtask(&mut self) -> Option<Task> {
        self.microtasks.pop_front()
    }

    /// Advance timers by `dt_ms` milliseconds and return any that have fired.
    pub fn tick(&mut self, dt_ms: u32) -> Vec<Task> {
        let mut fired = Vec::new();
        for timer in &mut self.timers {
            if timer.cleared { continue; }
            timer.elapsed_ms += dt_ms;
            if timer.elapsed_ms >= timer.delay_ms {
                fired.push(Task {
                    callback: timer.callback.clone(),
                    args: timer.args.clone(),
                });
                if timer.is_interval {
                    timer.elapsed_ms = 0; // reset for next interval
                } else {
                    timer.cleared = true; // one-shot
                }
            }
        }
        // Clean up cleared timers
        self.timers.retain(|t| !t.cleared || t.is_interval);
        fired
    }

    /// Check if there are pending microtasks.
    pub fn has_microtasks(&self) -> bool {
        !self.microtasks.is_empty()
    }

    /// Check if there are any pending timers.
    pub fn has_pending_timers(&self) -> bool {
        self.timers.iter().any(|t| !t.cleared)
    }
}
