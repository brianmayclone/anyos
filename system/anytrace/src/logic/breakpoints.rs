//! Software breakpoint management.

use alloc::vec::Vec;
use anyos_std::debug;

/// A software breakpoint.
#[derive(Clone)]
pub struct Breakpoint {
    /// Virtual address of the breakpoint.
    pub addr: u64,
    /// Whether this breakpoint is currently active (INT3 installed).
    pub active: bool,
    /// Optional symbol name for display.
    pub label: Option<alloc::string::String>,
}

/// Manages the set of breakpoints for a debug session.
pub struct BreakpointManager {
    pub breakpoints: Vec<Breakpoint>,
}

impl BreakpointManager {
    /// Create a new empty breakpoint manager.
    pub fn new() -> Self {
        Self {
            breakpoints: Vec::new(),
        }
    }

    /// Set a breakpoint at the given address.
    pub fn set(&mut self, tid: u32, addr: u64) -> bool {
        // Check if already exists
        if self.breakpoints.iter().any(|bp| bp.addr == addr) {
            return true;
        }
        if debug::set_breakpoint(tid, addr) {
            self.breakpoints.push(Breakpoint {
                addr,
                active: true,
                label: None,
            });
            true
        } else {
            false
        }
    }

    /// Clear a breakpoint at the given address.
    pub fn clear(&mut self, tid: u32, addr: u64) -> bool {
        if let Some(pos) = self.breakpoints.iter().position(|bp| bp.addr == addr) {
            debug::clear_breakpoint(tid, addr);
            self.breakpoints.remove(pos);
            true
        } else {
            false
        }
    }

    /// Clear all breakpoints.
    pub fn clear_all(&mut self, tid: u32) {
        for bp in &self.breakpoints {
            debug::clear_breakpoint(tid, bp.addr);
        }
        self.breakpoints.clear();
    }

    /// Check if an address has a breakpoint.
    pub fn has_breakpoint(&self, addr: u64) -> bool {
        self.breakpoints.iter().any(|bp| bp.addr == addr)
    }
}
