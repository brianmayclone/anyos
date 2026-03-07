//! Executable memory management for JIT-compiled code.
//!
//! In `host_test` builds we allocate RWX memory via `mmap`. No mprotect
//! toggling — the buffer is always readable, writable, and executable.

#[cfg(not(feature = "host_test"))]
use alloc::vec::Vec;

/// Default JIT buffer size (4 MiB).
const DEFAULT_JIT_BUFFER_SIZE: usize = 4 * 1024 * 1024;

#[cfg(all(feature = "host_test", unix))]
unsafe extern "C" {
    fn mmap(
        addr: *mut core::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: isize,
    ) -> *mut core::ffi::c_void;
    fn munmap(addr: *mut core::ffi::c_void, len: usize) -> i32;
}

#[cfg(all(feature = "host_test", unix))]
const PROT_READ: i32 = 0x1;
#[cfg(all(feature = "host_test", unix))]
const PROT_WRITE: i32 = 0x2;
#[cfg(all(feature = "host_test", unix))]
const PROT_EXEC: i32 = 0x4;
#[cfg(all(feature = "host_test", unix))]
const MAP_PRIVATE: i32 = 0x02;
#[cfg(all(feature = "host_test", unix))]
const MAP_ANONYMOUS: i32 = 0x20;
#[cfg(all(feature = "host_test", unix))]
const MAP_FAILED: *mut core::ffi::c_void = !0usize as *mut core::ffi::c_void;

#[cfg(all(feature = "host_test", windows))]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut core::ffi::c_void, size: usize, typ: u32, protect: u32) -> *mut core::ffi::c_void;
    fn VirtualFree(addr: *mut core::ffi::c_void, size: usize, typ: u32) -> i32;
}
#[cfg(all(feature = "host_test", windows))]
const MEM_COMMIT: u32 = 0x1000;
#[cfg(all(feature = "host_test", windows))]
const MEM_RESERVE: u32 = 0x2000;
#[cfg(all(feature = "host_test", windows))]
const MEM_RELEASE: u32 = 0x8000;
#[cfg(all(feature = "host_test", windows))]
const PAGE_EXECUTE_READWRITE: u32 = 0x40;

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

#[cfg(all(feature = "host_test", unix))]
impl Drop for JitBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.capacity != 0 {
            unsafe { let _ = munmap(self.ptr as *mut core::ffi::c_void, self.capacity); }
        }
    }
}

#[cfg(all(feature = "host_test", windows))]
impl Drop for JitBuffer {
    fn drop(&mut self) {
        if !self.ptr.is_null() && self.capacity != 0 {
            unsafe { VirtualFree(self.ptr as *mut core::ffi::c_void, 0, MEM_RELEASE); }
        }
    }
}

impl JitBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_JIT_BUFFER_SIZE)
    }

    pub fn with_capacity(size: usize) -> Self {
        #[cfg(all(feature = "host_test", unix))]
        {
            let ptr = unsafe {
                mmap(
                    core::ptr::null_mut(),
                    size,
                    PROT_READ | PROT_WRITE | PROT_EXEC,
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
                executable: true,
            }
        }
        #[cfg(all(feature = "host_test", windows))]
        {
            let ptr = unsafe {
                VirtualAlloc(core::ptr::null_mut(), size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
            };
            assert!(!ptr.is_null(), "JIT VirtualAlloc failed");
            JitBuffer {
                ptr: ptr as *mut u8,
                capacity: size,
                used: 0,
                executable: true,
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

    /// No-op with RWX mapping; kept for API compatibility.
    pub fn make_executable(&mut self) {
        self.executable = true;
    }

    /// No-op with RWX mapping; kept for API compatibility.
    pub fn make_writable(&mut self) {
        // RWX — always writable
    }

    pub fn is_executable(&self) -> bool {
        self.executable
    }

    pub fn capacity(&self) -> usize {
        #[cfg(feature = "host_test")]
        { self.capacity }
        #[cfg(not(feature = "host_test"))]
        { self.data.capacity() }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn remaining(&self) -> usize {
        self.capacity().saturating_sub(self.used)
    }

    pub fn reset(&mut self) {
        self.used = 0;
    }
}
