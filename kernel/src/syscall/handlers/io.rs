//! File descriptor I/O syscall handlers.
//!
//! Covers FD-based operations: read, write, open, close, lseek, fstat,
//! isatty, ftruncate, and POSIX FD duplication (pipe2, dup, dup2, fcntl).

use super::helpers::{fs_err, is_valid_user_ptr, read_user_str_safe, resolve_path};
use crate::fs::permissions::{check_permission, PERM_CREATE};

/// sys_write - Write to a file descriptor
/// fd=1 -> stdout (pipe if configured, else serial), fd=2 -> stderr (same), fd>=3 -> VFS file
pub fn sys_write(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    if len > 0x1000_0000 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }

    use crate::fs::fd_table::FdKind;

    // Look up the FD in the per-process table
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => {
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
            match entry.kind {
                FdKind::File { global_id } => {
                    match crate::fs::vfs::write(global_id, buf) {
                        Ok(n) => {
                            crate::task::scheduler::record_io_write(n as u64);
                            n as u32
                        }
                        Err(e) => fs_err(e),
                    }
                }
                FdKind::PipeWrite { pipe_id } => {
                    if entry.flags.nonblock {
                        // O_NONBLOCK: return EAGAIN if no buffer space available
                        use crate::ipc::anon_pipe::PIPE_BUF_SIZE;
                        let avail = crate::ipc::anon_pipe::bytes_available(pipe_id);
                        if avail >= PIPE_BUF_SIZE as u32 {
                            return u32::MAX - 10; // EAGAIN sentinel
                        }
                    }
                    crate::ipc::anon_pipe::write(pipe_id, buf)
                }
                FdKind::Tty => {
                    // Terminal I/O: write to named stdout pipe + serial
                    let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                    if pipe_id != 0 {
                        crate::ipc::pipe::write(pipe_id, buf);
                    }
                    let lock_state = crate::drivers::serial::output_lock_acquire();
                    for &byte in buf {
                        crate::drivers::serial::write_byte(byte);
                    }
                    crate::drivers::serial::output_lock_release(lock_state);
                    len
                }
                FdKind::PipeRead { .. } | FdKind::None => u32::MAX,
            }
        }
        None => {
            // Backward compat for kernel threads (no FdTable setup)
            if fd == 1 || fd == 2 {
                let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len as usize) };
                let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                if pipe_id != 0 {
                    crate::ipc::pipe::write(pipe_id, buf);
                }
                let lock_state = crate::drivers::serial::output_lock_acquire();
                for &byte in buf {
                    crate::drivers::serial::write_byte(byte);
                }
                crate::drivers::serial::output_lock_release(lock_state);
                len
            } else {
                u32::MAX
            }
        }
    }
}

/// sys_read - Read from a file descriptor (local FD from per-process table).
/// Dispatches to VFS file read or pipe read based on FdKind.
/// Falls back to legacy stdin_pipe for fd=0 if not in FD table.
pub fn sys_read(fd: u32, buf_ptr: u32, len: u32) -> u32 {
    if buf_ptr == 0 || len == 0 {
        return 0;
    }
    if len > 0x1000_0000 || !is_valid_user_ptr(buf_ptr as u64, len as u64) {
        return u32::MAX;
    }

    use crate::fs::fd_table::FdKind;

    // Look up the FD in the per-process table
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => {
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
            match entry.kind {
                FdKind::File { global_id } => {
                    match crate::fs::vfs::read(global_id, buf) {
                        Ok(n) => {
                            crate::task::scheduler::record_io_read(n as u64);
                            n as u32
                        }
                        Err(e) => fs_err(e),
                    }
                }
                FdKind::PipeRead { pipe_id } => {
                    if entry.flags.nonblock {
                        // O_NONBLOCK: return EAGAIN (-11 as u32) if pipe is empty and open
                        let avail = crate::ipc::anon_pipe::bytes_available(pipe_id);
                        if avail == 0 && !crate::ipc::anon_pipe::is_write_closed(pipe_id) {
                            return u32::MAX - 10; // EAGAIN sentinel
                        }
                    }
                    crate::ipc::anon_pipe::read(pipe_id, buf)
                }
                FdKind::Tty => {
                    // Terminal I/O: read from named stdin pipe
                    let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                    if pipe != 0 {
                        crate::ipc::pipe::read(pipe, buf) as u32
                    } else {
                        0 // no stdin
                    }
                }
                FdKind::PipeWrite { .. } | FdKind::None => u32::MAX,
            }
        }
        None => {
            // Backward compat for kernel threads (no FdTable setup)
            if fd == 0 {
                let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                if pipe != 0 {
                    let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len as usize) };
                    return crate::ipc::pipe::read(pipe, buf) as u32;
                }
                0 // no stdin pipe
            } else {
                u32::MAX
            }
        }
    }
}

/// sys_open - Open a file. arg1=path_ptr (null-terminated), arg2=flags, arg3=unused
/// Returns local file descriptor or u32::MAX on error.
pub fn sys_open(path_ptr: u32, flags: u32, _arg3: u32) -> u32 {
    let path = match read_user_str_safe(path_ptr) {
        Some(s) => s,
        None => return u32::MAX,
    };
    let cloexec = (flags & 0x10) != 0; // O_CLOEXEC
    let file_flags = crate::fs::file::FileFlags {
        read: true,
        write: (flags & 1) != 0,
        append: (flags & 2) != 0,
        create: (flags & 4) != 0,
        truncate: (flags & 8) != 0,
    };
    let resolved = resolve_path(path);

    // VFS permission check
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&resolved) {
        use crate::fs::permissions::*;
        let needed = if file_flags.write { PERM_READ | PERM_MODIFY } else { PERM_READ };
        if !check_permission(uid, gid, mode, needed) {
            return u32::MAX;
        }
    } else if file_flags.create {
        // File doesn't exist, check create permission on parent
        if let Some(parent) = resolved.rfind('/') {
            let parent_path = if parent == 0 { "/" } else { &resolved[..parent] };
            if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(parent_path) {
                if !check_permission(uid, gid, mode, PERM_CREATE) {
                    return u32::MAX;
                }
            }
        }
    }

    // VFS open returns a global slot_id
    let global_id = match crate::fs::vfs::open(&resolved, file_flags) {
        Ok(id) => id,
        Err(e) => return fs_err(e),
    };

    // Allocate a local FD in the current thread's FD table
    use crate::fs::fd_table::FdKind;
    let local_fd = match crate::task::scheduler::current_fd_alloc(FdKind::File { global_id }) {
        Some(fd) => fd,
        None => {
            // No local FD available — close the global slot
            crate::fs::vfs::decref(global_id);
            return u32::MAX;
        }
    };

    if cloexec {
        crate::task::scheduler::current_fd_set_cloexec(local_fd, true);
    }

    crate::debug_println!("  open({:?}) -> local_fd={} global_id={}", resolved, local_fd, global_id);
    local_fd
}

/// sys_close - Close a file descriptor (local FD from per-process table).
pub fn sys_close(fd: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    // Close the local FD entry — returns the old FdKind for resource cleanup
    match crate::task::scheduler::current_fd_close(fd) {
        Some(FdKind::File { global_id }) => {
            crate::fs::vfs::decref(global_id);
            0
        }
        Some(FdKind::PipeRead { pipe_id }) => {
            crate::ipc::anon_pipe::decref_read(pipe_id);
            0
        }
        Some(FdKind::PipeWrite { pipe_id }) => {
            crate::ipc::anon_pipe::decref_write(pipe_id);
            0
        }
        Some(FdKind::Tty) => {
            0 // Tty slot cleared, no resource to decref
        }
        Some(FdKind::None) | None => {
            // FD was not open — for backward compat, still try VFS close
            // (kernel-internal callers like users.rs use global slot IDs directly)
            if fd >= 3 {
                match crate::fs::vfs::close(fd) {
                    Ok(()) => 0,
                    Err(_) => u32::MAX,
                }
            } else {
                0
            }
        }
    }
}

/// sys_lseek - Seek within an open file.
/// arg1=fd, arg2=offset (signed i32), arg3=whence (0=SET, 1=CUR, 2=END)
/// Returns new position, or u32::MAX on error.
pub fn sys_lseek(fd: u32, offset: u32, whence: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => {
                match crate::fs::vfs::lseek(global_id, offset as i32, whence) {
                    Ok(pos) => pos,
                    Err(e) => fs_err(e),
                }
            }
            _ => 0,
        },
        None => {
            // Backward compat for kernel callers using global slot IDs
            if fd >= 3 {
                match crate::fs::vfs::lseek(fd, offset as i32, whence) {
                    Ok(pos) => pos,
                    Err(e) => fs_err(e),
                }
            } else {
                0
            }
        }
    }
}

/// sys_fstat - Get file information by fd.
/// arg1=fd, arg2=stat_buf_ptr: output [type:u32, size:u32, position:u32, mtime:u32] = 16 bytes
/// Returns 0 on success, u32::MAX on error.
pub fn sys_fstat(fd: u32, buf_ptr: u32) -> u32 {
    if buf_ptr == 0 { return u32::MAX; }

    use crate::fs::fd_table::FdKind;

    // Check per-process FD table first
    let global_id = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => Some(global_id),
            FdKind::PipeRead { .. } | FdKind::PipeWrite { .. } | FdKind::Tty => {
                // Pipe/Tty FDs: report as character device, size 0
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = 2; // device
                    *buf.add(1) = 0;
                    *buf.add(2) = 0;
                    *buf.add(3) = 0;
                }
                return 0;
            }
            FdKind::None => None,
        },
        None => None,
    };

    let slot = match global_id {
        Some(id) => id,
        None => {
            // Backward compat: fd 0-2 are stdin/stdout/stderr
            if fd < 3 {
                unsafe {
                    let buf = buf_ptr as *mut u32;
                    *buf = 2; // device
                    *buf.add(1) = 0;
                    *buf.add(2) = 0;
                    *buf.add(3) = 0;
                }
                return 0;
            }
            // Try global slot ID directly (kernel callers)
            fd
        }
    };

    match crate::fs::vfs::fstat(slot) {
        Ok((file_type, size, position, mtime)) => {
            unsafe {
                let buf = buf_ptr as *mut u32;
                *buf = match file_type {
                    crate::fs::file::FileType::Regular => 0,
                    crate::fs::file::FileType::Directory => 1,
                    crate::fs::file::FileType::Device => 2,
                };
                *buf.add(1) = size;
                *buf.add(2) = position;
                *buf.add(3) = mtime;
            }
            0
        }
        Err(e) => fs_err(e),
    }
}

/// sys_isatty - Check if a file descriptor refers to a terminal.
/// Returns 1 for stdin/stdout/stderr (when not redirected to a file/pipe), 0 otherwise.
pub fn sys_isatty(fd: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::Tty => 1,
            FdKind::File { .. } | FdKind::PipeRead { .. } | FdKind::PipeWrite { .. } => 0,
            FdKind::None => 0,
        },
        None => {
            // fd not in FD table: 0-2 are terminals (via legacy stdout/stdin pipe)
            if fd <= 2 { 1 } else { 0 }
        }
    }
}

/// sys_ftruncate - Truncate a file by fd to zero length.
/// arg1 = fd, arg2 = length (currently ignored, always truncates to 0).
/// Returns 0 on success, u32::MAX on error.
pub fn sys_ftruncate(fd: u32, _length: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    let global_id = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => global_id,
            _ => return u32::MAX,
        },
        None => {
            // Backward compat: try fd as global slot
            if fd < 3 { return u32::MAX; }
            fd
        }
    };
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(p) => p,
        Err(e) => return fs_err(e),
    };
    match crate::fs::vfs::truncate(&path) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// sys_pipe2 - Create an anonymous pipe.
/// arg1 = user pointer to int[2] (receives [read_fd, write_fd]).
/// arg2 = flags (O_CLOEXEC = 0x10).
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_pipe2(pipefd_ptr: u32, flags: u32) -> u32 {
    if pipefd_ptr == 0 || !is_valid_user_ptr(pipefd_ptr as u64, 8) {
        return u32::MAX;
    }

    let cloexec = (flags & 0x10) != 0;

    // Create the kernel pipe
    let pipe_id = crate::ipc::anon_pipe::create();
    if pipe_id == 0 {
        return u32::MAX; // table full
    }

    use crate::fs::fd_table::FdKind;

    // Allocate read-end FD
    let read_fd = match crate::task::scheduler::current_fd_alloc(FdKind::PipeRead { pipe_id }) {
        Some(fd) => fd,
        None => {
            // Clean up: decref both sides
            crate::ipc::anon_pipe::decref_read(pipe_id);
            crate::ipc::anon_pipe::decref_write(pipe_id);
            return u32::MAX;
        }
    };

    // Allocate write-end FD
    let write_fd = match crate::task::scheduler::current_fd_alloc(FdKind::PipeWrite { pipe_id }) {
        Some(fd) => fd,
        None => {
            // Clean up: close read-end, decref write
            crate::task::scheduler::current_fd_close(read_fd);
            crate::ipc::anon_pipe::decref_read(pipe_id);
            crate::ipc::anon_pipe::decref_write(pipe_id);
            return u32::MAX;
        }
    };

    if cloexec {
        crate::task::scheduler::current_fd_set_cloexec(read_fd, true);
        crate::task::scheduler::current_fd_set_cloexec(write_fd, true);
    }

    // Write [read_fd, write_fd] to user memory as two u32 values
    unsafe {
        let ptr = pipefd_ptr as *mut u32;
        *ptr = read_fd;
        *ptr.add(1) = write_fd;
    }

    crate::debug_println!("sys_pipe2: pipe_id={}, read_fd={}, write_fd={}", pipe_id, read_fd, write_fd);
    0
}

/// sys_dup - Duplicate a file descriptor, returning the lowest available FD.
/// Returns the new FD, or u32::MAX on error.
pub fn sys_dup(old_fd: u32) -> u32 {
    let entry = match crate::task::scheduler::current_fd_get(old_fd) {
        Some(e) => e,
        None => return u32::MAX,
    };

    let kind = entry.kind;

    // Allocate new FD with same kind
    let new_fd = match crate::task::scheduler::current_fd_alloc(kind) {
        Some(fd) => fd,
        None => return u32::MAX,
    };

    // Incref the underlying resource
    incref_fd_kind(kind);

    new_fd
}

/// sys_dup2 - Duplicate old_fd to new_fd. If new_fd is open, close it first.
/// Returns new_fd on success, u32::MAX on error.
pub fn sys_dup2(old_fd: u32, new_fd: u32) -> u32 {
    if old_fd == new_fd {
        // POSIX: if old_fd == new_fd and old_fd is valid, return new_fd
        return match crate::task::scheduler::current_fd_get(old_fd) {
            Some(_) => new_fd,
            None => u32::MAX,
        };
    }

    let entry = match crate::task::scheduler::current_fd_get(old_fd) {
        Some(e) => e,
        None => return u32::MAX,
    };

    let kind = entry.kind;

    // Close new_fd if it's currently open
    if let Some(old_kind) = crate::task::scheduler::current_fd_close(new_fd) {
        decref_fd_kind(old_kind);
    }

    // Place old_fd's resource at new_fd
    if !crate::task::scheduler::current_fd_alloc_at(new_fd, kind) {
        return u32::MAX;
    }

    // Incref the underlying resource
    incref_fd_kind(kind);

    new_fd
}

/// sys_fcntl - File control operations.
/// cmd: F_DUPFD=0, F_GETFD=1, F_SETFD=2, F_GETFL=3, F_SETFL=4, F_DUPFD_CLOEXEC=1030.
/// Returns result or u32::MAX on error.
pub fn sys_fcntl(fd: u32, cmd: u32, arg: u32) -> u32 {
    const F_DUPFD: u32 = 0;
    const F_GETFD: u32 = 1;
    const F_SETFD: u32 = 2;
    const F_GETFL: u32 = 3;
    const F_SETFL: u32 = 4;
    const F_DUPFD_CLOEXEC: u32 = 1030;
    const FD_CLOEXEC: u32 = 1;

    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let entry = match crate::task::scheduler::current_fd_get(fd) {
                Some(e) => e,
                None => return u32::MAX,
            };
            let kind = entry.kind;

            // Allocate lowest FD >= arg
            let new_fd = match crate::task::scheduler::current_fd_alloc_above(arg, kind) {
                Some(fd) => fd,
                None => return u32::MAX,
            };

            incref_fd_kind(kind);

            if cmd == F_DUPFD_CLOEXEC {
                crate::task::scheduler::current_fd_set_cloexec(new_fd, true);
            }

            new_fd
        }
        F_GETFD => {
            match crate::task::scheduler::current_fd_get(fd) {
                Some(e) => if e.flags.cloexec { FD_CLOEXEC } else { 0 },
                None => u32::MAX,
            }
        }
        F_SETFD => {
            crate::task::scheduler::current_fd_set_cloexec(fd, (arg & FD_CLOEXEC) != 0);
            0
        }
        F_GETFL => {
            const O_NONBLOCK: u32 = 0x800;
            match crate::task::scheduler::current_fd_get(fd) {
                Some(e) => if e.flags.nonblock { O_NONBLOCK } else { 0 },
                None => u32::MAX,
            }
        }
        F_SETFL => {
            const O_NONBLOCK: u32 = 0x800;
            crate::task::scheduler::current_fd_set_nonblock(fd, (arg & O_NONBLOCK) != 0);
            0
        }
        _ => u32::MAX,
    }
}

/// Increment the reference count for an FdKind resource.
fn incref_fd_kind(kind: crate::fs::fd_table::FdKind) {
    use crate::fs::fd_table::FdKind;
    match kind {
        FdKind::File { global_id } => crate::fs::vfs::incref(global_id),
        FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::incref_read(pipe_id),
        FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::incref_write(pipe_id),
        FdKind::Tty | FdKind::None => {}
    }
}

/// Decrement the reference count for an FdKind resource.
fn decref_fd_kind(kind: crate::fs::fd_table::FdKind) {
    use crate::fs::fd_table::FdKind;
    match kind {
        FdKind::File { global_id } => crate::fs::vfs::decref(global_id),
        FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(pipe_id),
        FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(pipe_id),
        FdKind::Tty | FdKind::None => {}
    }
}
