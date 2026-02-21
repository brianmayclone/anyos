//! Error types for anyos_std.
//!
//! Provides an `Error` enum that maps kernel errno values to named variants,
//! and a `Result<T>` type alias for convenience.

use core::fmt;

/// Errors returned by anyos_std functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// File or directory not found (ENOENT = 2).
    NotFound,
    /// Operation would block (EAGAIN = 11).
    WouldBlock,
    /// Permission denied (EACCES = 13).
    PermissionDenied,
    /// File or directory already exists (EEXIST = 17).
    AlreadyExists,
    /// Not a directory (ENOTDIR = 20).
    NotADirectory,
    /// Is a directory (EISDIR = 21).
    IsADirectory,
    /// Invalid argument (EINVAL = 22).
    InvalidInput,
    /// No space left on device (ENOSPC = 28).
    NoSpace,
    /// Broken pipe — no readers (EPIPE = 32).
    BrokenPipe,
    /// Connection timed out (ETIMEDOUT = 110).
    TimedOut,
    /// Connection refused (ECONNREFUSED = 111).
    ConnectionRefused,
    /// Out of memory (mmap/sbrk failed).
    OutOfMemory,
    /// Unknown or unmapped errno value.
    Other(u32),
}

/// Convenience alias: `Result<T>` = `core::result::Result<T, Error>`.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    /// Convert a kernel errno value to an `Error`.
    pub fn from_errno(errno: u32) -> Error {
        match errno {
            2 => Error::NotFound,
            11 => Error::WouldBlock,
            13 => Error::PermissionDenied,
            17 => Error::AlreadyExists,
            20 => Error::NotADirectory,
            21 => Error::IsADirectory,
            22 => Error::InvalidInput,
            28 => Error::NoSpace,
            32 => Error::BrokenPipe,
            110 => Error::TimedOut,
            111 => Error::ConnectionRefused,
            12 => Error::OutOfMemory,
            other => Error::Other(other),
        }
    }

    /// Convert a raw syscall return value to `Result<u32>`.
    ///
    /// The kernel returns `(-errno)` as `u32` on error (top bit set).
    /// On success, returns the non-negative result.
    #[inline]
    pub fn from_syscall(v: u32) -> Result<u32> {
        let signed = v as i32;
        if signed < 0 {
            Err(Error::from_errno((-signed) as u32))
        } else {
            Ok(v)
        }
    }

    /// Same as `from_syscall` but also treats `u32::MAX` (old-style error)
    /// as `NotFound` for backward compatibility with functions that use `sys_err()`.
    #[inline]
    pub fn from_raw(v: u32) -> Result<u32> {
        if v == u32::MAX {
            Err(Error::NotFound)
        } else {
            Error::from_syscall(v)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => f.write_str("not found"),
            Error::WouldBlock => f.write_str("would block"),
            Error::PermissionDenied => f.write_str("permission denied"),
            Error::AlreadyExists => f.write_str("already exists"),
            Error::NotADirectory => f.write_str("not a directory"),
            Error::IsADirectory => f.write_str("is a directory"),
            Error::InvalidInput => f.write_str("invalid input"),
            Error::NoSpace => f.write_str("no space left"),
            Error::BrokenPipe => f.write_str("broken pipe"),
            Error::TimedOut => f.write_str("timed out"),
            Error::ConnectionRefused => f.write_str("connection refused"),
            Error::OutOfMemory => f.write_str("out of memory"),
            Error::Other(e) => write!(f, "os error {}", e),
        }
    }
}
