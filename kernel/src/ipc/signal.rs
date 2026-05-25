//! POSIX-style process signals with bitmask-based pending/blocked tracking.
//!
//! Supports up to 32 signal numbers (1-31, signal 0 is reserved for error checking).
//! Each thread has a [`SignalState`] with pending/blocked bitmasks and a handler table.
//!
//! Handler types:
//! - `SIG_DFL` (0): default action (terminate for most, ignore for SIGCHLD/SIGURG)
//! - `SIG_IGN` (1): ignore the signal
//! - Any other value: user-space handler address (delivered via signal trampoline)

/// Standard POSIX signal numbers (Linux i386 ABI).
pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGTRAP: u32 = 5;
pub const SIGABRT: u32 = 6;
pub const SIGBUS: u32 = 7;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGUSR1: u32 = 10;
pub const SIGSEGV: u32 = 11;
pub const SIGUSR2: u32 = 12;
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;
pub const SIGCHLD: u32 = 17;
pub const SIGCONT: u32 = 18;
pub const SIGSTOP: u32 = 19;
pub const SIGTSTP: u32 = 20;
pub const SIGTTIN: u32 = 21;
pub const SIGTTOU: u32 = 22;
pub const SIGWINCH: u32 = 28;

/// Sentinel value meaning "default action" (matches libc SIG_DFL = 0).
pub const SIG_DFL: u64 = 0;
/// Sentinel value meaning "ignore this signal" (matches libc SIG_IGN = 1).
pub const SIG_IGN: u64 = 1;

#[derive(Clone, Copy)]
pub struct SignalInfo {
    pub code: i32,
    pub pid: u32,
    pub uid: u32,
    pub status: u32,
}

impl SignalInfo {
    pub const fn empty() -> Self {
        Self {
            code: 0,
            pid: 0,
            uid: 0,
            status: 0,
        }
    }
}

/// Per-process signal state: pending mask, blocked mask, and handler table.
///
/// Handler values: 0 = SIG_DFL, 1 = SIG_IGN, >1 = user-space function address.
#[derive(Clone)]
pub struct SignalState {
    /// Bitmask of signals that have been sent but not yet delivered.
    pub pending: u32,
    /// Bitmask of signals that are currently blocked from delivery.
    pub blocked: u32,
    /// Handler address for each signal (0=SIG_DFL, 1=SIG_IGN, else user addr).
    pub handlers: [u64; 32],
    /// Per-signal action flags, as passed by Linux rt_sigaction.
    pub flags: [u64; 32],
    /// Per-signal restorer address for SA_RESTORER-style Linux handlers.
    pub restorers: [u64; 32],
    /// Per-signal handler mask.
    pub masks: [u64; 32],
    /// Last queued siginfo payload for each pending signal.
    pub infos: [SignalInfo; 32],
}

impl SignalState {
    /// Create a new signal state with default handlers and nothing pending/blocked.
    pub const fn new() -> Self {
        SignalState {
            pending: 0,
            blocked: 0,
            handlers: [SIG_DFL; 32],
            flags: [0; 32],
            restorers: [0; 32],
            masks: [0; 32],
            infos: [SignalInfo::empty(); 32],
        }
    }

    /// Mark a signal as pending. SIGKILL and SIGSTOP cannot be blocked.
    pub fn send(&mut self, sig: u32) {
        self.send_with_info(sig, SignalInfo::empty());
    }

    pub fn send_with_info(&mut self, sig: u32, info: SignalInfo) {
        if sig > 0 && sig < 32 {
            self.pending |= 1 << sig;
            self.infos[sig as usize] = info;
        }
    }

    /// Get the lowest-numbered pending, unblocked signal, or None.
    pub fn dequeue(&mut self) -> Option<u32> {
        // SIGKILL (9) and SIGSTOP (19) cannot be blocked
        let effective_blocked = self.blocked & !((1 << SIGKILL) | (1 << SIGSTOP));
        let deliverable = self.pending & !effective_blocked;
        if deliverable == 0 {
            return None;
        }
        let sig = deliverable.trailing_zeros();
        self.pending &= !(1 << sig);
        Some(sig)
    }

    /// Check if there are any pending, unblocked signals.
    pub fn has_pending(&self) -> bool {
        let effective_blocked = self.blocked & !((1 << SIGKILL) | (1 << SIGSTOP));
        (self.pending & !effective_blocked) != 0
    }

    /// Set handler for a signal. Returns the old handler. SIGKILL/SIGSTOP cannot be caught.
    pub fn set_handler(&mut self, sig: u32, handler: u64) -> u64 {
        if sig == 0 || sig >= 32 || sig == SIGKILL || sig == SIGSTOP {
            return SIG_DFL;
        }
        let old = self.handlers[sig as usize];
        self.handlers[sig as usize] = handler;
        self.flags[sig as usize] = 0;
        self.restorers[sig as usize] = 0;
        self.masks[sig as usize] = 0;
        old
    }

    /// Set full signal action. Returns old handler, flags, restorer, and mask.
    pub fn set_action(
        &mut self,
        sig: u32,
        handler: u64,
        flags: u64,
        restorer: u64,
        mask: u64,
    ) -> (u64, u64, u64, u64) {
        if sig == 0 || sig >= 32 || sig == SIGKILL || sig == SIGSTOP {
            return (SIG_DFL, 0, 0, 0);
        }
        let idx = sig as usize;
        let old = (
            self.handlers[idx],
            self.flags[idx],
            self.restorers[idx],
            self.masks[idx],
        );
        self.handlers[idx] = handler;
        self.flags[idx] = flags;
        self.restorers[idx] = restorer;
        self.masks[idx] = mask;
        old
    }

    /// Get handler for a signal.
    pub fn get_handler(&self, sig: u32) -> u64 {
        if sig >= 32 {
            SIG_DFL
        } else {
            self.handlers[sig as usize]
        }
    }

    /// Get full signal action: handler, flags, restorer, mask.
    pub fn get_action(&self, sig: u32) -> (u64, u64, u64, u64) {
        if sig >= 32 {
            (SIG_DFL, 0, 0, 0)
        } else {
            let idx = sig as usize;
            (
                self.handlers[idx],
                self.flags[idx],
                self.restorers[idx],
                self.masks[idx],
            )
        }
    }

    pub fn get_info(&self, sig: u32) -> SignalInfo {
        if sig >= 32 {
            SignalInfo::empty()
        } else {
            self.infos[sig as usize]
        }
    }

    /// Reset caught signal handlers for execve().
    ///
    /// POSIX/Linux exec preserves ignored dispositions and the blocked mask, but
    /// handlers pointing into the old program image must become SIG_DFL.
    pub fn reset_caught_for_exec(&mut self) {
        for sig in 1..32usize {
            let handler = self.handlers[sig];
            if handler != SIG_DFL && handler != SIG_IGN {
                self.handlers[sig] = SIG_DFL;
                self.flags[sig] = 0;
                self.restorers[sig] = 0;
                self.masks[sig] = 0;
            }
        }
    }

    /// Returns true if the default action for this signal is to terminate the process.
    pub fn default_is_terminate(sig: u32) -> bool {
        matches!(
            sig,
            SIGHUP
                | SIGINT
                | SIGQUIT
                | SIGILL
                | SIGTRAP
                | SIGABRT
                | SIGBUS
                | SIGFPE
                | SIGKILL
                | SIGUSR1
                | SIGSEGV
                | SIGUSR2
                | SIGPIPE
                | SIGALRM
                | SIGTERM
        )
    }

    /// Returns true if the default action for this signal is to ignore it.
    pub fn default_is_ignore(sig: u32) -> bool {
        matches!(sig, SIGCHLD | SIGCONT | SIGWINCH)
    }

    /// Returns true if the default action for this signal is to stop the process.
    pub fn default_is_stop(sig: u32) -> bool {
        matches!(sig, SIGTSTP | SIGSTOP | SIGTTIN | SIGTTOU)
    }
}
