//! File descriptor I/O syscall handlers.
//!
//! Covers FD-based operations: read, write, open, close, lseek, fstat,
//! isatty, ftruncate, and POSIX FD duplication (pipe2, dup, dup2, fcntl).

use super::helpers::{
    copy_to_user_bytes, copy_user_bytes, fs_err, is_valid_user_ptr, read_user_str_safe,
    resolve_path,
};
use crate::fs::permissions::{check_permission, PERM_CREATE};

// Keep VFS writes small. CoreFS currently shows intermittent corruption when a
// single userspace write is forwarded as several large 64 KiB appends before
// metadata is flushed; 16 KiB matches the stable download/write path.
const WRITE_COPY_CHUNK: usize = 16 * 1024;
const MAX_FILE_WRITE_COPY_CHUNK: usize = 128 * 1024;
const FILE_READ_COPY_CHUNK: usize = 128 * 1024;
const PIPE_READ_COPY_CHUNK: usize = 16 * 1024;
const EAGAIN_SENTINEL: u32 = u32::MAX - 10;

/// sys_write - Write to a file descriptor
/// fd=1 -> stdout (pipe if configured, else serial), fd=2 -> stderr (same), fd>=3 -> VFS file
pub fn sys_write(fd: u32, buf_ptr: u64, len: u32) -> u32 {
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
            match entry.kind {
                FdKind::File { global_id } => {
                    let mut total = 0usize;
                    let chunk_cap = crate::fs::vfs::preferred_write_chunk(global_id)
                        .min(MAX_FILE_WRITE_COPY_CHUNK)
                        .max(WRITE_COPY_CHUNK);
                    while total < len as usize {
                        let chunk_len = ((len as usize) - total).min(chunk_cap);
                        let Some(buf) = copy_user_bytes(
                            buf_ptr.wrapping_add(total as u64),
                            chunk_len,
                            chunk_cap,
                        ) else {
                            return if total > 0 { total as u32 } else { u32::MAX };
                        };
                        match crate::fs::vfs::write(global_id, &buf) {
                            Ok(n) => {
                                crate::task::scheduler::record_io_write(n as u64);
                                total += n;
                                if n < chunk_len {
                                    return total as u32;
                                }
                            }
                            Err(e) => {
                                return if total > 0 { total as u32 } else { fs_err(e) };
                            }
                        }
                    }
                    total as u32
                }
                FdKind::PipeWrite { pipe_id } => {
                    let chunk_len = (len as usize).min(WRITE_COPY_CHUNK);
                    let Some(buf) = copy_user_bytes(buf_ptr, chunk_len, WRITE_COPY_CHUNK) else {
                        return u32::MAX;
                    };
                    if entry.flags.nonblock {
                        // O_NONBLOCK: return EAGAIN if no buffer space available
                        use crate::ipc::anon_pipe::PIPE_BUF_SIZE;
                        let avail = crate::ipc::anon_pipe::bytes_available(pipe_id);
                        if avail >= PIPE_BUF_SIZE as u32 {
                            return EAGAIN_SENTINEL;
                        }
                    }
                    crate::ipc::anon_pipe::write(pipe_id, &buf)
                }
                FdKind::Tty => {
                    let chunk_len = (len as usize).min(WRITE_COPY_CHUNK);
                    let Some(buf) = copy_user_bytes(buf_ptr, chunk_len, WRITE_COPY_CHUNK) else {
                        return u32::MAX;
                    };
                    // Terminal I/O: write to named stdout pipe + serial (if verbose)
                    let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                    if pipe_id != 0 {
                        crate::ipc::pipe::write(pipe_id, &buf);
                    }
                    if crate::drivers::serial::is_verbose() {
                        let lock_state = crate::drivers::serial::output_lock_acquire();
                        for &byte in &buf {
                            crate::drivers::serial::write_byte(byte);
                        }
                        crate::drivers::serial::output_lock_release(lock_state);
                    }
                    chunk_len as u32
                }
                FdKind::PtySlave { pty_id } => {
                    let chunk_len = (len as usize).min(WRITE_COPY_CHUNK);
                    let Some(buf) = copy_user_bytes(buf_ptr, chunk_len, WRITE_COPY_CHUNK) else {
                        return u32::MAX;
                    };
                    if crate::drivers::serial::is_verbose() {
                        let lock_state = crate::drivers::serial::output_lock_acquire();
                        for &byte in &buf {
                            crate::drivers::serial::write_byte(byte);
                        }
                        crate::drivers::serial::output_lock_release(lock_state);
                    }
                    crate::ipc::pty::write_slave(pty_id, &buf)
                }
                FdKind::PipeRead { .. }
                | FdKind::LinuxProc { .. }
                | FdKind::LinuxSocket { .. }
                | FdKind::LinuxFramebuffer { .. }
                | FdKind::None => u32::MAX,
            }
        }
        None => {
            // Backward compat for kernel threads (no FdTable setup)
            if fd == 1 || fd == 2 {
                let chunk_len = (len as usize).min(WRITE_COPY_CHUNK);
                let Some(buf) = copy_user_bytes(buf_ptr, chunk_len, WRITE_COPY_CHUNK) else {
                    return u32::MAX;
                };
                let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                if pipe_id != 0 {
                    crate::ipc::pipe::write(pipe_id, &buf);
                }
                if crate::drivers::serial::is_verbose() {
                    let lock_state = crate::drivers::serial::output_lock_acquire();
                    for &byte in &buf {
                        crate::drivers::serial::write_byte(byte);
                    }
                    crate::drivers::serial::output_lock_release(lock_state);
                }
                chunk_len as u32
            } else {
                u32::MAX
            }
        }
    }
}

/// sys_read - Read from a file descriptor (local FD from per-process table).
/// Dispatches to VFS file read or pipe read based on FdKind.
/// Falls back to legacy stdin_pipe for fd=0 if not in FD table.
pub fn sys_read(fd: u32, buf_ptr: u64, len: u32) -> u32 {
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
            match entry.kind {
                FdKind::File { global_id } => {
                    let mut total = 0usize;
                    while total < len as usize {
                        let chunk_len = ((len as usize) - total).min(FILE_READ_COPY_CHUNK);
                        let mut chunk = alloc::vec![0u8; chunk_len];
                        match crate::fs::vfs::read(global_id, &mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                if !copy_to_user_bytes(
                                    buf_ptr.wrapping_add(total as u64),
                                    &chunk[..n],
                                    FILE_READ_COPY_CHUNK,
                                ) {
                                    return if total > 0 { total as u32 } else { u32::MAX };
                                }
                                crate::task::scheduler::record_io_read(n as u64);
                                total += n;
                                if n < chunk_len {
                                    break;
                                }
                            }
                            Err(e) => {
                                return if total > 0 { total as u32 } else { fs_err(e) };
                            }
                        }
                    }
                    total as u32
                }
                FdKind::PipeRead { pipe_id } => {
                    if entry.flags.nonblock {
                        // O_NONBLOCK: return EAGAIN (-11 as u32) if pipe is empty and open
                        let avail = crate::ipc::anon_pipe::bytes_available(pipe_id);
                        if avail == 0 && !crate::ipc::anon_pipe::is_write_closed(pipe_id) {
                            return EAGAIN_SENTINEL;
                        }
                    }
                    read_anon_pipe_to_user(pipe_id, buf_ptr, len as usize)
                }
                FdKind::Tty => {
                    // Terminal I/O: read from named stdin pipe
                    let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                    if pipe != 0 {
                        read_tty_pipe_to_user(pipe, buf_ptr, len as usize, !entry.flags.nonblock)
                    } else {
                        0 // no stdin
                    }
                }
                FdKind::PtySlave { pty_id } => {
                    read_pty_to_user(pty_id, buf_ptr, len as usize, !entry.flags.nonblock)
                }
                FdKind::PipeWrite { .. }
                | FdKind::LinuxProc { .. }
                | FdKind::LinuxSocket { .. }
                | FdKind::LinuxFramebuffer { .. }
                | FdKind::None => u32::MAX,
            }
        }
        None => {
            // Backward compat for kernel threads (no FdTable setup)
            if fd == 0 {
                let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                if pipe != 0 {
                    return read_tty_pipe_to_user(pipe, buf_ptr, len as usize, true);
                }
                0 // no stdin pipe
            } else {
                u32::MAX
            }
        }
    }
}

fn read_anon_pipe_to_user(pipe_id: u32, buf_ptr: u64, len: usize) -> u32 {
    let copy_len = len.min(PIPE_READ_COPY_CHUNK);
    let mut chunk = alloc::vec![0u8; copy_len];
    let n = crate::ipc::anon_pipe::read(pipe_id, &mut chunk);
    if n == 0 || n == EAGAIN_SENTINEL {
        return n;
    }
    let n_usize = n as usize;
    if n_usize > copy_len {
        return u32::MAX;
    }
    if !copy_to_user_bytes(buf_ptr, &chunk[..n_usize], PIPE_READ_COPY_CHUNK) {
        return u32::MAX;
    }
    n
}

fn read_pty_to_user(pty_id: u32, buf_ptr: u64, len: usize, blocking: bool) -> u32 {
    let copy_len = len.min(PIPE_READ_COPY_CHUNK);
    let mut chunk = alloc::vec![0u8; copy_len];
    let n = crate::ipc::pty::read_slave(pty_id, &mut chunk, blocking);
    if n == 0 || n == EAGAIN_SENTINEL {
        return n;
    }
    let n_usize = n as usize;
    if n_usize > copy_len {
        return u32::MAX;
    }
    if !copy_to_user_bytes(buf_ptr, &chunk[..n_usize], PIPE_READ_COPY_CHUNK) {
        return u32::MAX;
    }
    n
}

/// Write a kernel-owned byte slice to an fd, mirroring `sys_write`'s FD-kind
/// dispatch. wxe console services synthesize bytes (e.g. UTF-16 → UTF-8) and so
/// cannot hand a user pointer to `sys_write`; this is their write path.
pub fn write_fd_kernel(fd: u32, data: &[u8]) -> u32 {
    use crate::fs::fd_table::FdKind;
    if data.is_empty() {
        return 0;
    }
    fn serial_echo(bytes: &[u8]) {
        if crate::drivers::serial::is_verbose() {
            let lock_state = crate::drivers::serial::output_lock_acquire();
            for &b in bytes {
                crate::drivers::serial::write_byte(b);
            }
            crate::drivers::serial::output_lock_release(lock_state);
        }
    }
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => match crate::fs::vfs::write(global_id, data) {
                Ok(n) => {
                    crate::task::scheduler::record_io_write(n as u64);
                    n as u32
                }
                Err(e) => fs_err(e),
            },
            FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::write(pipe_id, data),
            FdKind::Tty => {
                let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                if pipe_id != 0 {
                    crate::ipc::pipe::write(pipe_id, data);
                }
                serial_echo(data);
                data.len() as u32
            }
            FdKind::PtySlave { pty_id } => {
                serial_echo(data);
                crate::ipc::pty::write_slave(pty_id, data)
            }
            _ => u32::MAX,
        },
        None => {
            if fd == 1 || fd == 2 {
                let pipe_id = crate::task::scheduler::current_thread_stdout_pipe();
                if pipe_id != 0 {
                    crate::ipc::pipe::write(pipe_id, data);
                }
                serial_echo(data);
                data.len() as u32
            } else {
                u32::MAX
            }
        }
    }
}

/// Read into a kernel-owned buffer from an fd, mirroring `sys_read`'s dispatch.
/// Returns bytes read (0 = no data when non-blocking / EOF). Used by wxe
/// ReadConsole services that convert UTF-8 input into UTF-16.
pub fn read_fd_kernel(fd: u32, out: &mut [u8], blocking: bool) -> u32 {
    use crate::fs::fd_table::FdKind;
    if out.is_empty() {
        return 0;
    }
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => match crate::fs::vfs::read(global_id, out) {
                Ok(n) => {
                    crate::task::scheduler::record_io_read(n as u64);
                    n as u32
                }
                Err(e) => fs_err(e),
            },
            FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::read(pipe_id, out),
            FdKind::Tty => {
                let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                if pipe != 0 {
                    read_tty_pipe(pipe, out, blocking)
                } else {
                    0
                }
            }
            FdKind::PtySlave { pty_id } => crate::ipc::pty::read_slave(pty_id, out, blocking),
            _ => u32::MAX,
        },
        None => {
            if fd == 0 {
                let pipe = crate::task::scheduler::current_thread_stdin_pipe();
                if pipe != 0 {
                    read_tty_pipe(pipe, out, blocking)
                } else {
                    0
                }
            } else {
                u32::MAX
            }
        }
    }
}

fn read_tty_pipe_to_user(pipe: u32, buf_ptr: u64, len: usize, blocking: bool) -> u32 {
    let copy_len = len.min(PIPE_READ_COPY_CHUNK);
    let mut chunk = alloc::vec![0u8; copy_len];
    let n = read_tty_pipe(pipe, &mut chunk, blocking);
    if n == 0 || n == EAGAIN_SENTINEL {
        return n;
    }
    let n_usize = n as usize;
    if n_usize > copy_len {
        return u32::MAX;
    }
    if !copy_to_user_bytes(buf_ptr, &chunk[..n_usize], PIPE_READ_COPY_CHUNK) {
        return u32::MAX;
    }
    n
}

fn read_tty_pipe(pipe: u32, buf: &mut [u8], blocking: bool) -> u32 {
    loop {
        let n = crate::ipc::pipe::read(pipe, buf);
        if n != 0 {
            return n;
        }
        if !blocking {
            return EAGAIN_SENTINEL;
        }
        let wake_at = crate::arch::hal::timer_current_ticks().wrapping_add(1);
        crate::task::scheduler::sleep_until(wake_at);
    }
}

/// sys_open - Open a file. arg1=path_ptr (null-terminated), arg2=flags, arg3=unused
/// Returns local file descriptor or u32::MAX on error.
pub fn sys_open(path_ptr: u64, flags: u32, _arg3: u32) -> u32 {
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
        sync: (flags & 0x20) != 0, // O_SYNC
    };
    let resolved = resolve_path(path);

    // VFS permission check
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(&resolved) {
        use crate::fs::permissions::*;
        let needed = if file_flags.write {
            PERM_READ | PERM_MODIFY
        } else {
            PERM_READ
        };
        if !check_permission(uid, gid, mode, needed) {
            return u32::MAX;
        }
    } else if file_flags.create {
        // File doesn't exist, check create permission on parent
        if let Some(parent) = resolved.rfind('/') {
            let parent_path = if parent == 0 {
                "/"
            } else {
                &resolved[..parent]
            };
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

    crate::debug_println!(
        "  open({:?}) -> local_fd={} global_id={}",
        resolved,
        local_fd,
        global_id
    );
    local_fd
}

/// sys_close - Close a file descriptor (local FD from per-process table).
pub fn sys_close(fd: u32) -> u32 {
    sys_close_with_kind(fd).0
}

/// Close a local FD and return the kind that was actually removed.
///
/// LXE uses this for precise diagnostics: logging a separately sampled FD kind
/// before `close()` made repeated-close traces look successful even when the
/// second syscall should be EBADF.
pub fn sys_close_with_kind(fd: u32) -> (u32, Option<crate::fs::fd_table::FdKind>) {
    // Some userspace libraries still propagate negative errno-style sentinels
    // through `u32`-typed FDs. Treat those as inert invalid handles instead of
    // surfacing noisy EBADF failures during startup.
    if (fd as i32) < 0 {
        return (0, None);
    }

    use crate::fs::fd_table::FdKind;
    // Close the local FD entry — returns the old FdKind for resource cleanup
    let closed = crate::task::scheduler::current_fd_close(fd);
    let ret = match closed {
        Some(FdKind::File { global_id }) => match crate::fs::vfs::close(global_id) {
            Ok(()) => 0,
            Err(e) => fs_err(e),
        },
        Some(FdKind::PipeRead { pipe_id }) => {
            crate::ipc::anon_pipe::decref_read(pipe_id);
            0
        }
        Some(FdKind::PipeWrite { pipe_id }) => {
            crate::ipc::anon_pipe::decref_write(pipe_id);
            0
        }
        Some(FdKind::Tty) | Some(FdKind::PtySlave { .. }) => {
            0 // Tty slot cleared, no resource to decref
        }
        Some(FdKind::LinuxProc { .. }) | Some(FdKind::LinuxFramebuffer { .. }) => 0,
        Some(FdKind::LinuxSocket { socket_id }) => {
            crate::syscall::linux::socket_decref(socket_id);
            0
        }
        Some(FdKind::None) | None => {
            // FD was not in per-process table → EBADF.
            // Kernel-internal callers (users.rs, interfaces.rs) call vfs::close()
            // directly with global slot IDs — they don't go through this syscall.
            u32::MAX
        }
    };
    (ret, closed)
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
            // FD not in per-process table → EBADF
            u32::MAX
        }
    }
}

/// sys_fstat - Get file information by fd.
/// arg1=fd, arg2=stat_buf_ptr: output [type:u32, size:u32, position:u32, mtime:u32] = 16 bytes
/// Returns 0 on success, u32::MAX on error.
pub fn sys_fstat(fd: u32, buf_ptr: u64) -> u32 {
    if buf_ptr == 0 {
        return u32::MAX;
    }

    use crate::fs::fd_table::FdKind;

    // Check per-process FD table first
    let global_id = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => Some(global_id),
            FdKind::PipeRead { .. }
            | FdKind::PipeWrite { .. }
            | FdKind::Tty
            | FdKind::PtySlave { .. }
            | FdKind::LinuxProc { .. }
            | FdKind::LinuxSocket { .. }
            | FdKind::LinuxFramebuffer { .. } => {
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
            // FD not in per-process table → EBADF
            return u32::MAX;
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
            FdKind::Tty | FdKind::PtySlave { .. } => 1,
            FdKind::File { .. }
            | FdKind::PipeRead { .. }
            | FdKind::PipeWrite { .. }
            | FdKind::LinuxProc { .. }
            | FdKind::LinuxSocket { .. }
            | FdKind::LinuxFramebuffer { .. } => 0,
            FdKind::None => 0,
        },
        None => {
            // fd not in FD table: 0-2 are terminals (via legacy stdout/stdin pipe)
            if fd <= 2 {
                1
            } else {
                0
            }
        }
    }
}

/// sys_ftruncate - Truncate a file by fd.
/// arg1 = fd, arg2 = length.
/// Returns 0 on success, u32::MAX on error.
pub fn sys_ftruncate(fd: u32, length: u32) -> u32 {
    use crate::fs::fd_table::FdKind;
    let global_id = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => global_id,
            _ => return u32::MAX,
        },
        None => {
            // FD not in per-process table → EBADF
            return u32::MAX;
        }
    };
    match crate::fs::vfs::ftruncate_to(global_id, length) {
        Ok(()) => 0,
        Err(e) => fs_err(e),
    }
}

/// sys_pipe2 - Create an anonymous pipe.
/// arg1 = user pointer to int[2] (receives [read_fd, write_fd]).
/// arg2 = flags (O_CLOEXEC = 0x10).
/// Returns 0 on success, u32::MAX on failure.
pub fn sys_pipe2(pipefd_ptr: u64, flags: u32) -> u32 {
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

    crate::debug_println!(
        "sys_pipe2: pipe_id={}, read_fd={}, write_fd={}",
        pipe_id,
        read_fd,
        write_fd
    );
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
        F_GETFD => match crate::task::scheduler::current_fd_get(fd) {
            Some(e) => {
                if e.flags.cloexec {
                    FD_CLOEXEC
                } else {
                    0
                }
            }
            None => u32::MAX,
        },
        F_SETFD => {
            if crate::task::scheduler::current_fd_get(fd).is_none() {
                return u32::MAX;
            }
            crate::task::scheduler::current_fd_set_cloexec(fd, (arg & FD_CLOEXEC) != 0);
            0
        }
        F_GETFL => {
            const O_NONBLOCK: u32 = 0x800;
            match crate::task::scheduler::current_fd_get(fd) {
                Some(e) => {
                    if e.flags.nonblock {
                        O_NONBLOCK
                    } else {
                        0
                    }
                }
                None => u32::MAX,
            }
        }
        F_SETFL => {
            const O_NONBLOCK: u32 = 0x800;
            if crate::task::scheduler::current_fd_get(fd).is_none() {
                return u32::MAX;
            }
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
        FdKind::LinuxSocket { socket_id } => crate::syscall::linux::socket_incref(socket_id),
        FdKind::Tty
        | FdKind::PtySlave { .. }
        | FdKind::LinuxProc { .. }
        | FdKind::LinuxFramebuffer { .. }
        | FdKind::None => {}
    }
}

/// Decrement the reference count for an FdKind resource.
fn decref_fd_kind(kind: crate::fs::fd_table::FdKind) {
    use crate::fs::fd_table::FdKind;
    match kind {
        FdKind::File { global_id } => crate::fs::vfs::decref(global_id),
        FdKind::PipeRead { pipe_id } => crate::ipc::anon_pipe::decref_read(pipe_id),
        FdKind::PipeWrite { pipe_id } => crate::ipc::anon_pipe::decref_write(pipe_id),
        FdKind::LinuxSocket { socket_id } => crate::syscall::linux::socket_decref(socket_id),
        FdKind::Tty
        | FdKind::PtySlave { .. }
        | FdKind::LinuxProc { .. }
        | FdKind::LinuxFramebuffer { .. }
        | FdKind::None => {}
    }
}
