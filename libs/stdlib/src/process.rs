//! Process management — exit, spawn, wait, sleep, etc.

use core::arch::asm;
use crate::raw::*;

pub fn exit(code: u32) -> ! {
    unsafe {
        #[cfg(target_arch = "x86_64")]
        asm!(
            "mov rbx, {code}",
            "int 0x80",
            code = in(reg) code as u64,
            in("rax") SYS_EXIT as u64,
            options(noreturn)
        );
        #[cfg(target_arch = "aarch64")]
        asm!(
            "svc #0",
            in("x0") code as u64,
            in("x8") SYS_EXIT as u64,
            options(noreturn, nostack)
        );
    }
}

pub fn getpid() -> u32 {
    syscall0(SYS_GETPID)
}

pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

pub fn sbrk(increment: i32) -> usize {
    syscall1(SYS_SBRK, increment as i64 as u64) as usize
}

/// Map anonymous pages into the process address space.
/// Returns a pointer to the mapped region, or null on failure.
/// The memory is zeroed and page-aligned.
/// Use `munmap` to free it when no longer needed.
pub fn mmap(size: usize) -> *mut u8 {
    let result = syscall1(SYS_MMAP, size as u64);
    if result == u32::MAX {
        core::ptr::null_mut()
    } else {
        result as *mut u8
    }
}

/// Unmap pages previously mapped with `mmap`, freeing the physical memory.
/// `addr` must be the value returned by `mmap` (page-aligned).
/// `size` is the original size passed to `mmap`.
/// Returns true on success.
pub fn munmap(addr: *mut u8, size: usize) -> bool {
    syscall2(SYS_MUNMAP, addr as u64, size as u64) == 0
}

pub fn waitpid(tid: u32) -> u32 {
    // Must use syscall3 to explicitly pass child_tid_ptr=0, options=0.
    // syscall1 leaves RDX (options) unset — if bit 0 is set by leftover
    // register content, the kernel treats it as WNOHANG (non-blocking),
    // causing waitpid to return STILL_RUNNING instead of blocking.
    syscall3(SYS_WAITPID, tid as u64, 0, 0)
}

/// Non-blocking check if process exited.
/// Returns exit code if terminated, u32::MAX if not found, STILL_RUNNING if alive.
pub const STILL_RUNNING: u32 = u32::MAX - 1;
pub fn try_waitpid(tid: u32) -> u32 {
    syscall1(SYS_TRY_WAITPID, tid as u64)
}

/// Kill a thread by TID (sends SIGKILL). Returns 0 on success, u32::MAX on failure.
pub fn kill(tid: u32) -> u32 {
    syscall2(SYS_KILL, tid as u64, 9) // 9 = SIGKILL
}

/// Power off the system. Requires `CAP_SYSTEM`.
///
/// This function does not return.
pub fn shutdown() -> ! {
    syscall1(SYS_SHUTDOWN, 0);
    loop {}
}

/// Reboot the system. Requires `CAP_SYSTEM`.
///
/// This function does not return.
pub fn reboot() -> ! {
    syscall1(SYS_SHUTDOWN, 1);
    loop {}
}

/// Fork the current process. Returns:
/// - In the parent: the child's TID (> 0)
/// - In the child: 0
/// - On error: u32::MAX
pub fn fork() -> u32 {
    syscall0(SYS_FORK)
}

/// Replace the current process with a new program.
/// `path` — filesystem path to the binary.
/// `args` — command-line arguments string (space-separated).
/// On success, never returns. On failure, returns u32::MAX.
pub fn exec(path: &str, args: &str) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;

    let mut args_buf = [0u8; 257];
    let alen = args.len().min(256);
    args_buf[..alen].copy_from_slice(&args.as_bytes()[..alen]);
    args_buf[alen] = 0;

    syscall2(SYS_EXEC, path_buf.as_ptr() as u64, args_buf.as_ptr() as u64)
}

/// Spawn a new process. Returns TID or u32::MAX on error.
///
/// For `.app` bundles that require sensitive permissions and have not been
/// approved by the user yet, this automatically launches the PermissionDialog,
/// waits for the user's decision, and retries the spawn.
pub fn spawn(path: &str, args: &str) -> u32 {
    let tid = spawn_piped(path, args, 0);
    if tid == crate::permissions::PERM_NEEDED {
        // Read pending info stored by the kernel
        let mut buf = [0u8; 512];
        let len = crate::permissions::perm_pending_info(&mut buf);
        if len == 0 {
            return u32::MAX;
        }
        // Build dialog args: pass the raw pending info as the argument
        let info = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        let dialog_tid = spawn_piped("/System/permdialog", info, 0);
        if dialog_tid == u32::MAX || dialog_tid == crate::permissions::PERM_NEEDED {
            return u32::MAX;
        }
        let exit_code = waitpid(dialog_tid);
        if exit_code != 0 {
            return u32::MAX; // User cancelled or dialog error
        }
        // Retry the original spawn — permissions should now be stored
        return spawn_piped(path, args, 0);
    }
    tid
}

/// Spawn a new process with stdout redirected to a pipe.
/// pipe_id=0 means no pipe (stdout goes to serial only).
/// Returns TID or u32::MAX on error.
pub fn spawn_piped(path: &str, args: &str, pipe_id: u32) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;

    let mut args_buf = [0u8; 257];
    let alen = args.len().min(256);
    args_buf[..alen].copy_from_slice(&args.as_bytes()[..alen]);
    args_buf[alen] = 0;

    let args_ptr = if args.is_empty() { 0u64 } else { args_buf.as_ptr() as u64 };
    syscall3(SYS_SPAWN, path_buf.as_ptr() as u64, pipe_id as u64, args_ptr)
}

/// Spawn a new process with both stdout and stdin redirected to pipes.
/// stdout_pipe=0 means no stdout pipe, stdin_pipe=0 means no stdin pipe.
/// Returns TID or u32::MAX on error.
pub fn spawn_piped_full(path: &str, args: &str, stdout_pipe: u32, stdin_pipe: u32) -> u32 {
    let mut path_buf = [0u8; 257];
    let plen = path.len().min(256);
    path_buf[..plen].copy_from_slice(&path.as_bytes()[..plen]);
    path_buf[plen] = 0;

    let mut args_buf = [0u8; 257];
    let alen = args.len().min(256);
    args_buf[..alen].copy_from_slice(&args.as_bytes()[..alen]);
    args_buf[alen] = 0;

    let args_ptr = if args.is_empty() { 0u64 } else { args_buf.as_ptr() as u64 };
    syscall4(SYS_SPAWN, path_buf.as_ptr() as u64, stdout_pipe as u64, args_ptr, stdin_pipe as u64)
}

/// Create a new thread in the current process, sharing the same address space.
///
/// `entry` is a function pointer for the new thread's entry point.
/// `stack_top` is the top of a user-allocated stack (must be 8-byte aligned,
/// and the caller should subtract 8 from the true top for ABI alignment).
/// `name` is a human-readable thread name (max 31 chars, shown in task manager/logs).
///
/// Returns the TID of the new thread, or 0 on error.
/// The new thread inherits the parent's priority.
pub fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32 {
    thread_create_with_priority(entry, stack_top, name, 0)
}

/// Create a new thread with an explicit priority (1-255, higher = more CPU time).
/// Priority 0 inherits from the parent thread.
pub fn thread_create_with_priority(entry: fn(), stack_top: usize, name: &str, priority: u8) -> u32 {
    syscall5(
        SYS_THREAD_CREATE,
        entry as u64,
        stack_top as u64,
        name.as_ptr() as u64,
        name.len() as u64,
        priority as u64,
    )
}

/// Set the priority of a thread (1-255, higher = more CPU time).
/// `tid` = 0 means the calling thread itself.
/// Returns 0 on success.
pub fn set_priority(tid: u32, priority: u8) -> u32 {
    syscall2(SYS_SET_PRIORITY, tid as u64, priority as u64)
}

/// Return the calling thread's capability bitmask.
pub fn get_capabilities() -> u32 {
    syscall0(SYS_GET_CAPABILITIES)
}

/// Return the calling thread's user ID.
pub fn getuid() -> u16 {
    syscall0(SYS_GETUID) as u16
}

/// Return the calling thread's group ID.
pub fn getgid() -> u16 {
    syscall0(SYS_GETGID) as u16
}

/// Authenticate with username and password. On success, sets the process
/// identity (uid/gid) and returns true. On failure returns false.
pub fn authenticate(username: &str, password: &str) -> bool {
    let mut ubuf = [0u8; 33];
    let ulen = username.len().min(32);
    ubuf[..ulen].copy_from_slice(&username.as_bytes()[..ulen]);
    ubuf[ulen] = 0;

    let mut pbuf = [0u8; 65];
    let plen = password.len().min(64);
    pbuf[..plen].copy_from_slice(&password.as_bytes()[..plen]);
    pbuf[plen] = 0;

    syscall2(SYS_AUTHENTICATE, ubuf.as_ptr() as u64, pbuf.as_ptr() as u64) == 0
}

/// Get username for a given uid. Returns the number of bytes written, or u32::MAX.
pub fn getusername(uid: u16, buf: &mut [u8]) -> u32 {
    syscall3(SYS_GETUSERNAME, uid as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// Root-only: Set the calling process's identity to the given uid.
/// The kernel looks up the gid from the user database.
/// Returns 0 on success, u32::MAX on failure.
pub fn set_identity(uid: u16) -> u32 {
    syscall1(SYS_SET_IDENTITY, uid as u64)
}

/// Get command-line arguments (raw). Returns the args length.
/// The raw args string includes argv[0] (the program name).
pub fn getargs(buf: &mut [u8]) -> usize {
    syscall2(SYS_GETARGS, buf.as_mut_ptr() as u64, buf.len() as u64) as usize
}

/// Get command-line arguments, skipping argv[0] (the program name).
/// Returns the argument portion of the args string (after the first space).
///
/// Handles quoted argv[0] for paths with spaces (e.g. `"/Applications/My App.app" file.md`).
pub fn args(buf: &mut [u8; 256]) -> &str {
    let len = getargs(buf);
    let all = core::str::from_utf8(&buf[..len]).unwrap_or("");
    if all.starts_with('"') {
        // Quoted argv[0]: skip to closing quote
        match all[1..].find('"') {
            Some(close) => all[close + 2..].trim_start(),
            None => "",
        }
    } else {
        match all.find(' ') {
            Some(idx) => all[idx + 1..].trim_start(),
            None => "",
        }
    }
}

// =========================================================================
// High-level types — Thread, Child
// =========================================================================

use crate::error;

/// Default stack size for threads (64 KiB).
const DEFAULT_STACK_SIZE: usize = 64 * 1024;

/// A handle to a spawned thread. Provides RAII stack management.
pub struct Thread {
    tid: u32,
    stack_ptr: *mut u8,
    stack_size: usize,
}

impl Thread {
    /// Spawn a new thread with the default stack size (64 KiB).
    pub fn spawn(entry: fn(), name: &str) -> error::Result<Thread> {
        Self::spawn_with_stack(entry, DEFAULT_STACK_SIZE, name)
    }

    /// Spawn a new thread with a custom stack size.
    pub fn spawn_with_stack(entry: fn(), stack_size: usize, name: &str) -> error::Result<Thread> {
        let stack_ptr = mmap(stack_size);
        if stack_ptr.is_null() {
            return Err(error::Error::OutOfMemory);
        }
        // x86_64 ABI: RSP must be STACK_TOP - 8 at function entry
        let stack_top = (stack_ptr as usize) + stack_size - 8;
        let tid = thread_create(entry, stack_top, name);
        if tid == 0 {
            munmap(stack_ptr, stack_size);
            return Err(error::Error::Other(0));
        }
        Ok(Thread { tid, stack_ptr, stack_size })
    }

    /// Get the thread ID.
    pub fn tid(&self) -> u32 {
        self.tid
    }

    /// Wait for the thread to finish. Returns its exit code.
    /// Consumes the handle and frees the thread's stack.
    pub fn join(self) -> u32 {
        let code = waitpid(self.tid);
        munmap(self.stack_ptr, self.stack_size);
        // Prevent Drop from freeing the stack again
        core::mem::forget(self);
        code
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        // If the thread handle is dropped without join(), we still wait
        // and free the stack to avoid leaking memory.
        waitpid(self.tid);
        munmap(self.stack_ptr, self.stack_size);
    }
}

/// A handle to a spawned child process.
pub struct Child {
    tid: u32,
}

impl Child {
    /// Spawn a new child process.
    pub fn spawn(path: &str, args: &str) -> error::Result<Child> {
        let tid = super::process::spawn(path, args);
        if tid == u32::MAX {
            return Err(error::Error::NotFound);
        }
        Ok(Child { tid })
    }

    /// Get the child's thread ID.
    pub fn tid(&self) -> u32 {
        self.tid
    }

    /// Wait for the child to exit. Returns its exit code.
    /// Consumes the handle.
    pub fn wait(self) -> u32 {
        waitpid(self.tid)
    }

    /// Non-blocking check if the child has exited.
    /// Returns `Some(exit_code)` if terminated, `None` if still running.
    pub fn try_wait(&self) -> Option<u32> {
        let ret = try_waitpid(self.tid);
        if ret == STILL_RUNNING || ret == u32::MAX {
            None
        } else {
            Some(ret)
        }
    }

    /// Kill the child process.
    pub fn kill(&self) -> error::Result<()> {
        let ret = kill(self.tid);
        if ret == u32::MAX {
            Err(error::Error::NotFound)
        } else {
            Ok(())
        }
    }
}
