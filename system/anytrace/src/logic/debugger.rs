//! Debug session management — attach, detach, suspend, resume, step.

use anyos_std::debug::{self, DebugRegs, DebugEvent};

/// State of a debug session.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// No target attached.
    Detached,
    /// Attached and target is suspended.
    Suspended,
    /// Attached and target is running.
    Running,
}

/// A debug session with a single target thread.
pub struct DebugSession {
    pub target_tid: u32,
    pub state: SessionState,
    pub regs: DebugRegs,
    pub last_event: Option<DebugEvent>,
}

impl DebugSession {
    /// Create a new empty (detached) debug session.
    pub fn new() -> Self {
        Self {
            target_tid: 0,
            state: SessionState::Detached,
            regs: DebugRegs::default(),
            last_event: None,
        }
    }

    /// Attach to a target thread. Returns true on success.
    pub fn attach(&mut self, tid: u32) -> bool {
        if self.state != SessionState::Detached {
            self.detach();
        }
        if debug::attach(tid) {
            self.target_tid = tid;
            self.state = SessionState::Suspended;
            self.refresh_regs();
            true
        } else {
            false
        }
    }

    /// Detach from the current target.
    pub fn detach(&mut self) {
        if self.target_tid != 0 {
            debug::detach(self.target_tid);
        }
        self.target_tid = 0;
        self.state = SessionState::Detached;
        self.last_event = None;
    }

    /// Suspend a running target.
    pub fn suspend(&mut self) -> bool {
        if self.state != SessionState::Running {
            return false;
        }
        if debug::suspend(self.target_tid) {
            self.state = SessionState::Suspended;
            self.refresh_regs();
            true
        } else {
            false
        }
    }

    /// Resume a suspended target.
    pub fn resume(&mut self) -> bool {
        if self.state != SessionState::Suspended {
            return false;
        }
        if debug::resume(self.target_tid) {
            self.state = SessionState::Running;
            true
        } else {
            false
        }
    }

    /// Single-step one instruction.
    pub fn step_into(&mut self) -> bool {
        if self.state != SessionState::Suspended {
            return false;
        }
        if debug::single_step(self.target_tid) {
            self.state = SessionState::Running;
            true
        } else {
            false
        }
    }

    /// Refresh the register snapshot from the target.
    pub fn refresh_regs(&mut self) {
        if self.target_tid != 0 {
            debug::get_regs(self.target_tid, &mut self.regs);
        }
    }

    /// Read memory from the target.
    pub fn read_mem(&self, addr: u64, buf: &mut [u8]) -> usize {
        if self.target_tid == 0 {
            return 0;
        }
        debug::read_mem(self.target_tid, addr, buf)
    }

    /// Poll for debug events. Returns true if an event was received.
    pub fn poll_event(&mut self) -> bool {
        if self.target_tid == 0 {
            return false;
        }
        let mut event = DebugEvent::default();
        let etype = debug::wait_event(self.target_tid, &mut event);
        if etype != 0 {
            self.last_event = Some(event);
            if etype == debug::EVENT_BREAKPOINT || etype == debug::EVENT_SINGLE_STEP {
                self.state = SessionState::Suspended;
                self.refresh_regs();
            }
            true
        } else {
            false
        }
    }

    /// Check if we are attached.
    pub fn is_attached(&self) -> bool {
        self.state != SessionState::Detached
    }

    /// Check if the target is suspended.
    pub fn is_suspended(&self) -> bool {
        self.state == SessionState::Suspended
    }
}
