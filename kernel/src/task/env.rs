//! Per-process environment variables.
//!
//! Environment variables are stored in a global table keyed by page directory
//! physical address (so all threads in the same process share the same env).
//! Accessed via SYS_SETENV / SYS_GETENV syscalls.

use alloc::string::String;
use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;

struct EnvEntry {
    key: String,
    value: String,
}

struct ProcessEnv {
    /// Page directory physical address (identifies the process).
    pd: u64,
    entries: Vec<EnvEntry>,
}

static ENV_TABLE: Spinlock<Vec<ProcessEnv>> = Spinlock::new(Vec::new());

/// Set an environment variable for the process identified by `pd`.
/// If the key already exists, its value is updated.
pub fn set(pd: u64, key: &str, value: &str) {
    let mut table = ENV_TABLE.lock();
    // Find or create the process env
    let proc_env = if let Some(pe) = table.iter_mut().find(|pe| pe.pd == pd) {
        pe
    } else {
        table.push(ProcessEnv { pd, entries: Vec::new() });
        table.last_mut().unwrap()
    };
    // Update existing or insert new
    if let Some(entry) = proc_env.entries.iter_mut().find(|e| e.key == key) {
        entry.value = String::from(value);
    } else {
        proc_env.entries.push(EnvEntry {
            key: String::from(key),
            value: String::from(value),
        });
    }
}

/// Get an environment variable for the process identified by `pd`.
/// Returns the value as a String, or None if not found.
pub fn get(pd: u64, key: &str) -> Option<String> {
    let table = ENV_TABLE.lock();
    if let Some(pe) = table.iter().find(|pe| pe.pd == pd) {
        if let Some(entry) = pe.entries.iter().find(|e| e.key == key) {
            return Some(entry.value.clone());
        }
    }
    None
}

/// Remove an environment variable.
pub fn unset(pd: u64, key: &str) {
    let mut table = ENV_TABLE.lock();
    if let Some(pe) = table.iter_mut().find(|pe| pe.pd == pd) {
        pe.entries.retain(|e| e.key != key);
    }
}

/// List all environment variables for a process.
/// Returns list of "KEY=VALUE\0" entries packed into a buffer.
/// Returns total bytes needed (even if buffer is too small).
pub fn list(pd: u64, buf: &mut [u8]) -> usize {
    let table = ENV_TABLE.lock();
    let mut offset = 0usize;
    if let Some(pe) = table.iter().find(|pe| pe.pd == pd) {
        for entry in &pe.entries {
            let needed = entry.key.len() + 1 + entry.value.len() + 1; // "KEY=VALUE\0"
            if offset + needed <= buf.len() {
                buf[offset..offset + entry.key.len()].copy_from_slice(entry.key.as_bytes());
                buf[offset + entry.key.len()] = b'=';
                let val_start = offset + entry.key.len() + 1;
                buf[val_start..val_start + entry.value.len()].copy_from_slice(entry.value.as_bytes());
                buf[val_start + entry.value.len()] = 0;
            }
            offset += needed;
        }
    }
    offset
}

/// Clean up all environment variables for a process (on process exit).
pub fn cleanup(pd: u64) {
    let mut table = ENV_TABLE.lock();
    table.retain(|pe| pe.pd != pd);
}
