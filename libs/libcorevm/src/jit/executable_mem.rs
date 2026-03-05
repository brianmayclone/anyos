//! Executable memory management for JIT-compiled code.
//!
//! In `host_test` builds we allocate a dedicated code buffer via `mmap` and
//! switch protection with `mprotect` (RW <-> RX). In non-host builds we keep
//! the previous placeholder behavior until the kernel exposes `mprotect`.

#[cfg(not(feature = "host_test"))]
use alloc::vec::Vec;

/// Default JIT buffer size (4 MiB).
const DEFAULT_JIT_BUFFER_SIZE: usize = 4 * 1024 * 1024;

#[cfg(feature = "host_test")]
unsafe extern "C" {
    fn mmap(
        addr: *mut core::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: isize,
    ) -> *mut core::ffi::c_void;
    fn mprotect(addr: *mut core::ffi::c_void, len: usize, prot: i32) -> i32;
    fn munmap(addr: *mut core::ffi::c_void, len: usize) -> i32;
}

#[cfg(feature = "host_test")]
const PROT_READ: i32 = 0x1;
#[cfg(feature = "host_test")]
const PROT_WRITE: i32 = 0x2;
#[cfg(feature = "host_test")]
const PROT_EXEC: i32 = 0x4;
#[cfg(feature = "host_test")]
const MAP_PRIVATE: i32 = 0x02;
#[cfg(feature = "host_test")]
const MAP_ANONYMOUS: i32 = 0x20;
#[cfg(feature = "host_test")]
const MAP_FAILED: *mut core::ffi::c_void = !0usize as *mut core::ffi::c_void;

/// A buffer that holds JIT-compiled machine code.
pub struct JitBuffer {
    #[cfg(not(feature = "host_test"))]
    data: Vec<u8>,
    #[cfg(feature = "host_test")]
    ptr: *mut u8,
    #[cfg(feature = "host_test")]
    capacity: usize,

    used: usize,
    executable: bool,
}

#[cfg(feature = "host_test")]
impl Drop for JitBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.capacity != 0 {
            unsafe {
                let _ = munmap(self.ptr as *mut core::ffi::c_void, self.capacity);
            }
        }
    }
}

impl JitBuffer {
    /// Allocate a new JIT buffer with the default size.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_JIT_BUFFER_SIZE)
    }

    /// Allocate a JIT buffer with a specific size.
    pub fn with_capacity(size: usize) -> Self {
        #[cfg(feature = "host_test")]
        {
            let ptr = unsafe {
                mmap(
                    core::ptr::null_mut(),
                    size,
                    PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            assert!(ptr != MAP_FAILED, "JIT mmap failed");
            JitBuffer {
                ptr: ptr as *mut u8,
                capacity: size,
                used: 0,
                executable: false,
            }
        }
        #[cfg(not(feature = "host_test"))]
        {
            JitBuffer {
                data: Vec::with_capacity(size),
                used: 0,
                executable: false,
            }
        }
    }

    /// Write compiled code into the buffer and return its offset.
    pub fn emit(&mut self, code: &[u8]) -> Option<usize> {
        let offset = self.used;
        let new_used = self.used + code.len();
        if new_used > self.capacity() {
            return None;
        }

        #[cfg(feature = "host_test")]
        unsafe {
            core::ptr::copy_nonoverlapping(code.as_ptr(), self.ptr.add(offset), code.len());
        }

        #[cfg(not(feature = "host_test"))]
        {
            if new_used > self.data.len() {
                self.data.resize(new_used, 0);
            }
            self.data[offset..new_used].copy_from_slice(code);
        }

        self.used = new_used;
        Some(offset)
    }

    /// Get a raw pointer to code at the given offset.
    pub unsafe fn code_ptr(&self, offset: usize) -> *const u8 {
        #[cfg(feature = "host_test")]
        {
            self.ptr.add(offset) as *const u8
        }
        #[cfg(not(feature = "host_test"))]
        {
            self.data.as_ptr().add(offset)
        }
    }

    /// Mark the buffer as executable (W->X transition).
    pub fn make_executable(&mut self) {
        #[cfg(feature = "host_test")]
        unsafe {
            let rc = mprotect(
                self.ptr as *mut core::ffi::c_void,
                self.capacity,
                PROT_READ | PROT_EXEC,
            );
            assert_eq!(rc, 0, "JIT mprotect RX failed");
        }
        self.executable = true;
    }

    /// Mark the buffer as writable (X->W transition).
    pub fn make_writable(&mut self) {
        #[cfg(feature = "host_test")]
        unsafe {
            let rc = mprotect(
                self.ptr as *mut core::ffi::c_void,
                self.capacity,
                PROT_READ | PROT_WRITE,
            );
            assert_eq!(rc, 0, "JIT mprotect RW failed");
        }
        self.executable = false;
    }

    /// Whether the buffer is currently marked executable.
    pub fn is_executable(&self) -> bool {
        self.executable
    }

    /// Total capacity of the buffer in bytes.
    pub fn capacity(&self) -> usize {
        #[cfg(feature = "host_test")]
        {
            self.capacity
        }
        #[cfg(not(feature = "host_test"))]
        {
            self.data.capacity()
        }
    }

    /// Number of bytes currently used.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Number of bytes remaining.
    pub fn remaining(&self) -> usize {
        self.capacity().saturating_sub(self.used)
    }

    /// Reset the buffer, discarding all compiled code.
    pub fn reset(&mut self) {
        self.used = 0;
        self.executable = false;
    }
}
