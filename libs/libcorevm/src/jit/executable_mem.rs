//! Executable memory management for JIT-compiled code.
//!
//! Allocates a buffer for JIT code and manages the W^X (Write XOR Execute)
//! lifecycle. Currently uses a simple heap-allocated buffer that is marked
//! executable via a platform-specific mechanism.
//!
//! # anyOS Integration
//!
//! anyOS enforces NX-bit (DEP): heap memory is not executable by default.
//! A future `SYS_MPROTECT` syscall will allow proper W^X toggling. Until
//! then, this module provides the allocation/layout logic and the compiled
//! code can be tested via the interpreter fallback path.

use alloc::vec::Vec;

/// Default JIT buffer size (4 MiB).
const DEFAULT_JIT_BUFFER_SIZE: usize = 4 * 1024 * 1024;

/// A buffer that holds JIT-compiled machine code.
///
/// Code is written sequentially into the buffer. Each `emit()` call
/// appends code and returns a pointer to the start of the emitted region.
/// After all code for a batch is emitted, `make_executable()` switches
/// the buffer to execute-only mode.
pub struct JitBuffer {
    /// Backing storage for compiled code.
    data: Vec<u8>,
    /// Number of bytes currently used.
    used: usize,
    /// Whether the buffer is currently in executable mode.
    executable: bool,
}

impl JitBuffer {
    /// Allocate a new JIT buffer with the default size.
    pub fn new() -> Self {
        JitBuffer {
            data: Vec::with_capacity(DEFAULT_JIT_BUFFER_SIZE),
            used: 0,
            executable: false,
        }
    }

    /// Allocate a JIT buffer with a specific size.
    pub fn with_capacity(size: usize) -> Self {
        JitBuffer {
            data: Vec::with_capacity(size),
            used: 0,
            executable: false,
        }
    }

    /// Write compiled code into the buffer and return a function pointer.
    ///
    /// Returns the byte offset within the buffer where the code was placed.
    /// Returns `None` if the buffer doesn't have enough remaining capacity.
    pub fn emit(&mut self, code: &[u8]) -> Option<usize> {
        let offset = self.used;
        let new_used = self.used + code.len();
        if new_used > self.data.capacity() {
            return None; // Buffer full
        }
        // Extend the Vec to cover the new region.
        if new_used > self.data.len() {
            self.data.resize(new_used, 0);
        }
        self.data[offset..new_used].copy_from_slice(code);
        self.used = new_used;
        Some(offset)
    }

    /// Get a raw pointer to code at the given offset.
    ///
    /// # Safety
    ///
    /// The returned pointer is only safe to execute if `make_executable()`
    /// has been called and the buffer has not been modified since.
    pub unsafe fn code_ptr(&self, offset: usize) -> *const u8 {
        self.data.as_ptr().add(offset)
    }

    /// Mark the buffer as executable (W→X transition).
    ///
    /// After this call, the buffer should not be written to until
    /// `make_writable()` is called.
    ///
    /// Currently a no-op pending `SYS_MPROTECT` support in anyOS.
    pub fn make_executable(&mut self) {
        // TODO: Call sys_mprotect(data.as_ptr(), data.len(), PROT_READ | PROT_EXEC)
        // when anyOS supports it.
        self.executable = true;
    }

    /// Mark the buffer as writable (X→W transition).
    ///
    /// Currently a no-op pending `SYS_MPROTECT` support in anyOS.
    pub fn make_writable(&mut self) {
        // TODO: Call sys_mprotect(data.as_ptr(), data.len(), PROT_READ | PROT_WRITE)
        self.executable = false;
    }

    /// Whether the buffer is currently marked executable.
    pub fn is_executable(&self) -> bool {
        self.executable
    }

    /// Total capacity of the buffer in bytes.
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    /// Number of bytes currently used.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Number of bytes remaining.
    pub fn remaining(&self) -> usize {
        self.data.capacity().saturating_sub(self.used)
    }

    /// Reset the buffer, discarding all compiled code.
    ///
    /// Does not free memory — the capacity is retained for reuse.
    pub fn reset(&mut self) {
        self.used = 0;
        self.executable = false;
    }
}
