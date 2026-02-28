//! Shared helper functions used by all syscall handler modules.
//!
//! These are `pub(super)` so they're accessible within the `handlers` module
//! but not exported outside it.

use alloc::string::String;

/// Make a relative path absolute using the current thread's working directory.
/// Normalizes `.` and `..` components so e.g. `"."` resolves to the CWD itself.
pub(super) fn resolve_path(path: &str) -> String {
    let abs = if path.starts_with('/') {
        String::from(path)
    } else {
        let mut cwd_buf = [0u8; 512];
        let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
        let cwd = core::str::from_utf8(&cwd_buf[..cwd_len]).unwrap_or("/");
        if cwd == "/" || cwd.is_empty() {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd, path)
        }
    };
    crate::fs::path::normalize(&abs)
}

/// Convert an FsError into a negated-errno return value.
/// Convention: success = 0, error = (-errno) as u32.
/// The libc side checks `(int)ret < 0` and does `errno = -(int)ret`.
pub(super) fn fs_err(e: crate::fs::vfs::FsError) -> u32 {
    use crate::fs::vfs::FsError;
    let errno: i32 = match e {
        FsError::NotFound => 2,        // ENOENT
        FsError::PermissionDenied => 13, // EACCES
        FsError::AlreadyExists => 17,   // EEXIST
        FsError::NotADirectory => 20,   // ENOTDIR
        FsError::IsADirectory => 21,    // EISDIR
        FsError::NoSpace => 28,         // ENOSPC
        FsError::IoError => 5,          // EIO
        FsError::InvalidPath => 22,     // EINVAL
        FsError::TooManyOpenFiles => 24, // EMFILE
        FsError::BadFd => 9,            // EBADF
    };
    (-errno) as u32
}

/// Validate that a user pointer is in user address space (below kernel half).
/// Returns false if the pointer is NULL, in kernel space, or if ptr+len overflows.
#[inline]
pub(super) fn is_valid_user_ptr(ptr: u64, len: u64) -> bool {
    if ptr == 0 {
        return false;
    }
    // User space is below 0x0000_8000_0000_0000 (canonical lower half)
    let end = ptr.checked_add(len);
    match end {
        Some(e) => e <= 0x0000_8000_0000_0000,
        None => false, // overflow
    }
}

/// Read a null-terminated string from user memory (max 4096 bytes).
/// Returns None if the pointer is invalid.
pub(super) fn read_user_str_safe(ptr: u32) -> Option<&'static str> {
    if !is_valid_user_ptr(ptr as u64, 1) {
        return None;
    }
    let p = ptr as *const u8;
    let mut len = 0usize;
    unsafe {
        while len < 4096 && *p.add(len) != 0 {
            len += 1;
        }
        Some(core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len)))
    }
}

/// Read a null-terminated string from user memory (max 4096 bytes).
/// Returns "" if the pointer is invalid (NULL or kernel space).
pub(super) unsafe fn read_user_str(ptr: u32) -> &'static str {
    if !is_valid_user_ptr(ptr as u64, 1) {
        return "";
    }
    let p = ptr as *const u8;
    let mut len = 0usize;
    while len < 4096 && *p.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len))
}
