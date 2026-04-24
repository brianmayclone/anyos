//! Kernel debug backend for launched anyCode sessions.

use anyos_std::debug::{self, DebugEvent, DebugRegs};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackendState {
    Detached,
    Suspended,
    Running,
}

pub struct DebugBackend {
    pub target_tid: u32,
    pub state: BackendState,
    pub regs: DebugRegs,
    pub last_event: Option<DebugEvent>,
}

impl DebugBackend {
    pub fn new() -> Self {
        Self {
            target_tid: 0,
            state: BackendState::Detached,
            regs: DebugRegs::default(),
            last_event: None,
        }
    }

    pub fn attach(&mut self, tid: u32) -> bool {
        if self.is_attached() {
            self.detach();
        }

        if debug::attach(tid) {
            self.target_tid = tid;
            self.state = BackendState::Suspended;
            self.refresh_regs();
            true
        } else {
            false
        }
    }

    pub fn detach(&mut self) {
        if self.target_tid != 0 {
            debug::detach(self.target_tid);
        }
        self.target_tid = 0;
        self.state = BackendState::Detached;
        self.last_event = None;
        self.regs = DebugRegs::default();
    }

    pub fn suspend(&mut self) -> bool {
        if self.state != BackendState::Running {
            return false;
        }
        if debug::suspend(self.target_tid) {
            self.state = BackendState::Suspended;
            self.refresh_regs();
            true
        } else {
            false
        }
    }

    pub fn resume(&mut self) -> bool {
        if self.state != BackendState::Suspended {
            return false;
        }
        if debug::resume(self.target_tid) {
            self.state = BackendState::Running;
            true
        } else {
            false
        }
    }

    pub fn single_step(&mut self) -> bool {
        if self.state != BackendState::Suspended {
            return false;
        }
        if debug::single_step(self.target_tid) {
            self.state = BackendState::Running;
            true
        } else {
            false
        }
    }

    pub fn refresh_regs(&mut self) -> bool {
        self.target_tid != 0 && debug::get_regs(self.target_tid, &mut self.regs)
    }

    pub fn read_mem(&self, addr: u64, buf: &mut [u8]) -> usize {
        if self.target_tid == 0 {
            return 0;
        }
        debug::read_mem(self.target_tid, addr, buf)
    }

    pub fn poll_event(&mut self) -> Option<DebugEvent> {
        if self.target_tid == 0 {
            return None;
        }

        let mut event = DebugEvent::default();
        let event_type = debug::wait_event(self.target_tid, &mut event);
        if event_type == 0 {
            return None;
        }

        event.event_type = event_type;
        self.last_event = Some(event);
        if event_type == debug::EVENT_BREAKPOINT || event_type == debug::EVENT_SINGLE_STEP {
            self.state = BackendState::Suspended;
            self.refresh_regs();
        }
        if event_type == debug::EVENT_EXIT {
            self.state = BackendState::Detached;
            self.target_tid = 0;
        }
        Some(event)
    }

    pub fn is_attached(&self) -> bool {
        self.state != BackendState::Detached
    }

    pub fn is_suspended(&self) -> bool {
        self.state == BackendState::Suspended
    }
}

pub fn event_label(event_type: u32) -> &'static str {
    match event_type {
        debug::EVENT_BREAKPOINT => "breakpoint",
        debug::EVENT_SINGLE_STEP => "single step",
        debug::EVENT_EXIT => "exit",
        _ => "event",
    }
}
