use crate::sync::spinlock::Spinlock;

pub const SIG_TERM: u32 = 1;
pub const SIG_KILL: u32 = 2;
pub const SIG_INT: u32 = 3;
pub const SIG_USR1: u32 = 10;
pub const SIG_USR2: u32 = 11;

pub type SignalHandler = fn(u32);

pub struct SignalState {
    pub pending: u32,   // Bitmask of pending signals
    pub blocked: u32,   // Bitmask of blocked signals
    pub handlers: [Option<SignalHandler>; 32],
}

impl SignalState {
    pub const fn new() -> Self {
        SignalState {
            pending: 0,
            blocked: 0,
            handlers: [None; 32],
        }
    }

    pub fn send(&mut self, signal: u32) {
        if signal < 32 {
            self.pending |= 1 << signal;
        }
    }

    pub fn is_pending(&self, signal: u32) -> bool {
        signal < 32 && (self.pending & (1 << signal)) != 0 && (self.blocked & (1 << signal)) == 0
    }

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
