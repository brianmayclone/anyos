//! std::ffi compatible FFI types.
//!
//! On anyOS, OsStr/OsString are always valid UTF-8.

// Re-export from path module where OsStr/OsString are defined
pub use crate::path::{OsStr, OsString};

/// A borrowed C string (null-terminated).
pub struct CStr {
    inner: [u8],
}

impl CStr {
    /// Create a CStr from a byte slice with a null terminator.
    pub unsafe fn from_ptr<'a>(ptr: *const u8) -> &'a CStr {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(ptr, len + 1);
        &*(slice as *const [u8] as *const CStr)
    }

    /// Create a CStr from a byte slice including the null terminator.
    pub fn from_bytes_with_nul(bytes: &[u8]) -> Result<&CStr, FromBytesWithNulError> {
        if bytes.is_empty() || bytes[bytes.len() - 1] != 0 {
            return Err(FromBytesWithNulError(()));
        }
        // Check for interior nulls
        if bytes[..bytes.len() - 1].contains(&0) {
            return Err(FromBytesWithNulError(()));
        }
        Ok(unsafe { &*(bytes as *const [u8] as *const CStr) })
    }

    /// Get the bytes without the null terminator.
    pub fn to_bytes(&self) -> &[u8] {
        &self.inner[..self.inner.len() - 1]
    }

    /// Get the bytes including the null terminator.
    pub fn to_bytes_with_nul(&self) -> &[u8] {
        &self.inner
    }

    /// Convert to a string slice (may fail if not valid UTF-8).
    pub fn to_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.to_bytes())
    }

    /// Get the pointer to the null-terminated string.
    pub fn as_ptr(&self) -> *const u8 {
        self.inner.as_ptr()
    }
}

/// An owned C string.
pub struct CString {
    inner: alloc::vec::Vec<u8>,
}

impl CString {
    /// Create a new CString from bytes (must not contain null bytes).
    pub fn new<T: Into<alloc::vec::Vec<u8>>>(t: T) -> Result<CString, NulError> {
        let bytes: alloc::vec::Vec<u8> = t.into();
        if bytes.contains(&0) {
            return Err(NulError(bytes.iter().position(|&b| b == 0).unwrap()));
        }
        let mut inner = bytes;
        inner.push(0);
        Ok(CString { inner })
    }

    /// Get as a CStr reference.
    pub fn as_c_str(&self) -> &CStr {
        unsafe { &*(self.inner.as_slice() as *const [u8] as *const CStr) }
    }

    /// Get the pointer to the null-terminated string.
    pub fn as_ptr(&self) -> *const u8 {
        self.inner.as_ptr()
    }

    /// Convert into raw pointer (caller must free).
    pub fn into_raw(self) -> *mut u8 {
        let mut v = core::mem::ManuallyDrop::new(self.inner);
        v.as_mut_ptr()
    }

    /// Reconstruct from a raw pointer.
    pub unsafe fn from_raw(ptr: *mut u8) -> CString {
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let inner = alloc::vec::Vec::from_raw_parts(ptr, len + 1, len + 1);
        CString { inner }
    }

    /// Convert to bytes without the null terminator.
    pub fn to_bytes(&self) -> &[u8] {
        &self.inner[..self.inner.len() - 1]
    }

    /// Convert to string (may fail).
    pub fn to_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(self.to_bytes())
    }

    /// Consume and return the underlying bytes without null terminator.
    pub fn into_bytes(mut self) -> alloc::vec::Vec<u8> {
        self.inner.pop(); // remove null terminator
        self.inner
    }
}

impl core::ops::Deref for CString {
    type Target = CStr;
    fn deref(&self) -> &CStr {
        self.as_c_str()
    }
}

impl core::fmt::Debug for CString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CString({:?})", self.to_bytes())
    }
}

/// Error for CStr::from_bytes_with_nul.
#[derive(Debug, Clone)]
pub struct FromBytesWithNulError(());

impl core::fmt::Display for FromBytesWithNulError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("data provided is not nul terminated or contains interior nul bytes")
    }
}

/// Error for CString::new when the input contains a null byte.
#[derive(Debug, Clone)]
pub struct NulError(usize);

impl NulError {
    pub fn nul_position(&self) -> usize {
        self.0
    }
}

impl core::fmt::Display for NulError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "nul byte found in provided data at position: {}", self.0)
    }
}
