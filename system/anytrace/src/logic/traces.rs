//! Execution trace recording.
//!
//! Records a sequence of executed instructions (via repeated single-step)
//! for post-mortem analysis of execution flow.

use alloc::vec::Vec;
use alloc::string::String;

/// A single trace entry (one executed instruction).
#[derive(Clone)]
pub struct TraceEntry {
    /// Sequence number.
    pub seq: u32,
    /// Timestamp (uptime_ms).
    pub timestamp: u64,
    /// Target TID.
    pub tid: u32,
    /// Instruction pointer.
    pub rip: u64,
    /// Decoded instruction mnemonic.
    pub mnemonic: String,
    /// Decoded operands.
    pub operands: String,
}

/// Execution trace store.
pub struct TraceStore {
    pub entries: Vec<TraceEntry>,
    /// Whether tracing is active (single-step loop).
    pub active: bool,
    next_seq: u32,
    /// Maximum number of trace entries.
    pub max_entries: usize,
}

impl TraceStore {
    /// Create a new empty trace store.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            active: false,
            next_seq: 0,
            max_entries: 50000,
        }
    }

    /// Start tracing.
    pub fn start(&mut self) {
        self.entries.clear();
        self.next_seq = 0;
        self.active = true;
    }

    /// Stop tracing.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Record a trace entry.
    pub fn record(&mut self, tid: u32, rip: u64, mnemonic: &str, operands: &str) {
        if !self.active || self.entries.len() >= self.max_entries {
            return;
        }
        self.entries.push(TraceEntry {
            seq: self.next_seq,
            timestamp: anyos_std::sys::uptime_ms() as u64,
            tid,
            rip,
            mnemonic: String::from(mnemonic),
            operands: String::from(operands),
        });
        self.next_seq += 1;
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_seq = 0;
    }
}
