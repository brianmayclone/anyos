//! Per-process address space and metadata management.
//!
//! Each process owns a page directory, a list of thread IDs, and lifecycle state.
//! This module handles creation and teardown of process-level resources.

use alloc::vec::Vec;

static mut NEXT_PID: u32 = 1;

/// Lifecycle state of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Process is running or has runnable threads.
    Active,
    /// Process has exited but has not been fully reaped.
    Zombie,
}

/// A process with its own address space, consisting of one or more threads.
pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
    pub state: ProcessState,
    pub page_directory: u32, // Physical address of page directory
    pub thread_ids: Vec<u32>,
    pub name: [u8; 64],
}

impl Process {
    /// Create a new process with the given name and page directory physical address.
    /// Assigns a unique PID automatically.
    pub fn new(name: &str, page_directory: u32) -> Self {
        let pid = unsafe {
            let p = NEXT_PID;
            NEXT_PID += 1;
            p
        };

        let mut name_buf = [0u8; 64];
        let bytes = name.as_bytes();
        let len = bytes.len().min(63);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        Process {
            pid,
            parent_pid: 0,
            state: ProcessState::Active,
            page_directory,
            thread_ids: Vec::new(),
            name: name_buf,
        }
    }

    /// Return the process name as a UTF-8 string slice.
    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }
}
