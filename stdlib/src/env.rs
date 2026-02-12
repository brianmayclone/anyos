//! Environment variable access.

use crate::raw::*;

/// Set an environment variable. Pass value="" to set an empty value.
/// Returns 0 on success.
pub fn set(key: &str, value: &str) -> u32 {
    let mut key_buf = [0u8; 65];
    let klen = key.len().min(64);
    key_buf[..klen].copy_from_slice(&key.as_bytes()[..klen]);
    key_buf[klen] = 0;

    let mut val_buf = [0u8; 257];
    let vlen = value.len().min(256);
    val_buf[..vlen].copy_from_slice(&value.as_bytes()[..vlen]);
    val_buf[vlen] = 0;

    syscall2(SYS_SETENV, key_buf.as_ptr() as u64, val_buf.as_ptr() as u64)
}

/// Remove an environment variable. Returns 0 on success.
pub fn unset(key: &str) -> u32 {
    let mut key_buf = [0u8; 65];
    let klen = key.len().min(64);
    key_buf[..klen].copy_from_slice(&key.as_bytes()[..klen]);
    key_buf[klen] = 0;

    syscall2(SYS_SETENV, key_buf.as_ptr() as u64, 0)
}

/// Get an environment variable. Returns the value length, or u32::MAX if not found.
/// The value is written to `buf` (null-terminated if space permits).
pub fn get(key: &str, buf: &mut [u8]) -> u32 {
    let mut key_buf = [0u8; 65];
    let klen = key.len().min(64);
    key_buf[..klen].copy_from_slice(&key.as_bytes()[..klen]);
    key_buf[klen] = 0;

    syscall3(SYS_GETENV, key_buf.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// List all environment variables into buf as "KEY=VALUE\0KEY2=VALUE2\0..." entries.
/// Returns total bytes needed.
pub fn list(buf: &mut [u8]) -> u32 {
    syscall2(SYS_LISTENV, buf.as_mut_ptr() as u64, buf.len() as u64)
}
