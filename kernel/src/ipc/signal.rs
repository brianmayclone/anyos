//! POSIX-style process signals with bitmask-based pending/blocked tracking.
//!
//! Supports up to 32 signal numbers (0-31). Signals can be sent, blocked, and delivered
//! through per-process [`SignalState`]. Custom handlers can be registered per signal.

use crate::sync::spinlock::Spinlock;

/// Terminate signal (graceful shutdown request).
pub const SIG_TERM: u32 = 1;
/// Kill signal (cannot be blocked or handled).
pub const SIG_KILL: u32 = 2;
/// Interrupt signal (e.g. Ctrl-C).
pub const SIG_INT: u32 = 3;
/// User-defined signal 1.
pub const SIG_USR1: u32 = 10;
/// User-defined signal 2.
pub const SIG_USR2: u32 = 11;

/// Callback type for signal handlers: receives the signal number.
pub type SignalHandler = fn(u32);

/// Per-process signal state: pending mask, blocked mask, and handler table.
pub struct SignalState {
    /// Bitmask of signals that have been sent but not yet delivered.
    pub pending: u32,
    /// Bitmask of signals that are currently blocked from delivery.
    pub blocked: u32,
    /// Registered handler for each signal number (0-31), or `None` for default action.
    pub handlers: [Option<SignalHandler>; 32],
}

impl SignalState {
    /// Create a new signal state with no pending, blocked, or handled signals.
    pub const fn new() -> Self {
        SignalState {
            pending: 0,
            blocked: 0,
            handlers: [None; 32],
        }
    }

    /// Mark a signal as pending for this process.
    pub fn send(&mut self, signal: u32) {
        if signal < 32 {
            self.pending |= 1 << signal;
        }
    }

    /// Check if a signal is pending and not blocked.
    pub fn is_pending(&self, signal: u32) -> bool {
        signal < 32 && (self.pending & (1 << signal)) != 0 && (self.blocked & (1 << signal)) == 0
    }

    /// Register a handler function for the given signal number.
    pub fn set_handler(&mut self, signal: u32, handler: SignalHandler) {
        if signal < 32 {
            self.handlers[signal as usize] = Some(handler);
        }
    }

    /// Process the next pending, unblocked signal. Returns true if one was handled.
    pub fn deliver_next(&mut self) -> bool {
        let deliverable = self.pending & !self.blocked;
        if deliverable == 0 {
            return false;
        }

        // Find lowest set bit
        let sig = deliverable.trailing_zeros();
        self.pending &= !(1 << sig);

        if let Some(handler) = self.handlers[sig as usize] {
            handler(sig);
        }

        true
    }
}
