//! Process state snapshots.
//!
//! A snapshot captures the full register state at a point in time,
//! allowing the user to compare states across debugging steps.

use alloc::vec::Vec;
use alloc::string::String;
use anyos_std::debug::DebugRegs;

/// A snapshot of the process state.
#[derive(Clone)]
pub struct Snapshot {
    /// Snapshot index (auto-incremented).
    pub index: u32,
    /// Timestamp (uptime_ms).
    pub timestamp: u64,
    /// Target TID.
    pub tid: u32,
    /// Register state at snapshot time.
    pub regs: DebugRegs,
    /// Optional label.
    pub label: String,
}

/// Snapshot storage.
pub struct SnapshotStore {
    pub snapshots: Vec<Snapshot>,
    next_index: u32,
}

impl SnapshotStore {
    /// Create a new empty snapshot store.
    pub fn new() -> Self {
        Self {
            snapshots: Vec::new(),
            next_index: 0,
        }
    }

    /// Take a snapshot of the current state.
    pub fn take(&mut self, tid: u32, regs: &DebugRegs, label: &str) -> u32 {
        let index = self.next_index;
        self.next_index += 1;
        self.snapshots.push(Snapshot {
            index,
            timestamp: anyos_std::sys::uptime_ms() as u64,
            tid,
            regs: *regs,
            label: String::from(label),
        });
        index
    }

    /// Get a snapshot by index.
    pub fn get(&self, index: u32) -> Option<&Snapshot> {
        self.snapshots.iter().find(|s| s.index == index)
    }

    /// Clear all snapshots.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}
