//! Syscall number-to-name mapping table.
//!
//! Provides a static lookup table used for debug logging and introspection of
//! syscall numbers.

use super::*;

/// Function signature for a generic three-argument syscall handler.
pub type SyscallFn = fn(u32, u32, u32) -> u32;

/// Static table mapping syscall numbers to their human-readable names.
pub const SYSCALL_TABLE: &[(u32, &str)] = &[
    (SYS_EXIT, "exit"),
    (SYS_WRITE, "write"),
    (SYS_READ, "read"),
    (SYS_OPEN, "open"),
    (SYS_CLOSE, "close"),
    (SYS_GETPID, "getpid"),
    (SYS_YIELD, "yield"),
    (SYS_SLEEP, "sleep"),
    (SYS_SBRK, "sbrk"),
    (SYS_FORK, "fork"),
    (SYS_EXEC, "exec"),
    (SYS_WAITPID, "waitpid"),
    (SYS_TRY_WAITPID, "try_waitpid"),
    (SYS_KILL, "kill"),
    (SYS_MMAP, "mmap"),
    (SYS_MUNMAP, "munmap"),
    (SYS_DEVLIST, "devlist"),
    (SYS_DEVOPEN, "devopen"),
    (SYS_DEVCLOSE, "devclose"),
    (SYS_DEVREAD, "devread"),
    (SYS_DEVWRITE, "devwrite"),
    (SYS_DEVIOCTL, "devioctl"),
    (SYS_IRQWAIT, "irqwait"),
];
