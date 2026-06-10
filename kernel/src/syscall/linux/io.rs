use super::*;
use alloc::vec::Vec;

const LINUX_COPY_CHUNK: usize = 16 * 1024;
const LINUX_WRITEV_COALESCE_MAX: usize = 128 * 1024;
static TIOCL_VESA_BLANK_MODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static TIOCL_KMSG_REDIRECT: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static TIOCL_BLANKED_CONSOLE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

pub(super) fn linux_close(fd: u64) -> u64 {
    let tid = crate::task::scheduler::current_tid();
    if fd > i32::MAX as u64 {
        crate::serial_verbose_println!("lxe linux close: tid={} fd={:#x} -> EBADF", tid, fd);
        return linux_err(EBADF);
    }

    let fd_u32 = fd as u32;
    let before_path =
        crate::task::scheduler::current_fd_get(fd_u32).and_then(|entry| match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                crate::fs::vfs::get_fd_path(global_id).ok()
            }
            _ => None,
        });
    let (ret, closed_kind) = handlers::sys_close_with_kind(fd_u32);
    let linux_ret = match ret {
        0 => 0,
        u32::MAX => linux_err(EBADF),
        ret => anyos_u32_ret(ret),
    };
    match closed_kind {
        Some(crate::fs::fd_table::FdKind::File { global_id }) => {
            if let Some(path) = before_path {
                crate::serial_verbose_println!(
                    "lxe linux close: tid={} fd={} file global={} path='{}' -> {:#x}",
                    tid,
                    fd_u32,
                    global_id,
                    path,
                    linux_ret
                );
            } else {
                crate::serial_verbose_println!(
                    "lxe linux close: tid={} fd={} file global={} -> {:#x}",
                    tid,
                    fd_u32,
                    global_id,
                    linux_ret
                );
            }
        }
        Some(kind) => {
            crate::serial_verbose_println!(
                "lxe linux close: tid={} fd={} kind={:?} -> {:#x}",
                tid,
                fd_u32,
                kind,
                linux_ret
            );
        }
        None => {
            crate::serial_verbose_println!(
                "lxe linux close: tid={} fd={} missing -> {:#x}",
                tid,
                fd_u32,
                linux_ret
            );
        }
    }
    linux_ret
}

pub(super) fn linux_dup(fd: u64) -> u64 {
    if fd > i32::MAX as u64 {
        return linux_err(EBADF);
    }
    match handlers::sys_dup(fd as u32) {
        u32::MAX => linux_err(EBADF),
        ret => ret as u64,
    }
}

pub(super) fn linux_dup2(old_fd: u64, new_fd: u64) -> u64 {
    if old_fd > i32::MAX as u64 || new_fd > i32::MAX as u64 {
        return linux_err(EBADF);
    }
    match handlers::sys_dup2(old_fd as u32, new_fd as u32) {
        u32::MAX => linux_err(EBADF),
        ret => ret as u64,
    }
}

pub(super) fn linux_dup3(old_fd: u64, new_fd: u64, flags: u64) -> u64 {
    const O_CLOEXEC: u64 = 0o2000000;

    if (flags & !O_CLOEXEC) != 0 {
        return linux_err(EINVAL);
    }
    if old_fd == new_fd {
        return linux_err(EINVAL);
    }
    let ret = linux_dup2(old_fd, new_fd);
    if (ret as i64) >= 0 && (flags & O_CLOEXEC) != 0 {
        crate::task::scheduler::current_fd_set_cloexec(ret as u32, true);
    }
    ret
}

pub(super) fn linux_fcntl(fd: u32, cmd: u32, arg: u64) -> u64 {
    const F_GETFL: u32 = 3;
    const F_SETFL: u32 = 4;
    const F_GETLK: u32 = 5;
    const F_SETLK: u32 = 6;
    const F_SETLKW: u32 = 7;
    const F_UNLCK: u16 = 2;
    match cmd {
        F_GETFL => linux_fcntl_getfl(fd),
        F_SETFL => match handlers::sys_fcntl(fd, cmd, arg as u32) {
            u32::MAX => linux_err(EBADF),
            ret => anyos_u32_ret(ret),
        },
        F_GETLK => {
            if crate::task::scheduler::current_fd_get(fd).is_none() {
                return linux_err(EBADF);
            }
            if arg != 0 {
                if !handlers::helpers::is_user_range_accessible(arg, 2) {
                    return linux_err(EFAULT);
                }
                unsafe {
                    *((arg) as *mut u16) = F_UNLCK;
                }
            }
            0
        }
        F_SETLK | F_SETLKW => {
            if crate::task::scheduler::current_fd_get(fd).is_none() {
                linux_err(EBADF)
            } else {
                0
            }
        }
        _ => match handlers::sys_fcntl(fd, cmd, arg as u32) {
            u32::MAX => linux_err(EBADF),
            ret => anyos_u32_ret(ret),
        },
    }
}

fn linux_fcntl_getfl(fd: u32) -> u64 {
    const O_WRONLY: u64 = 0o1;
    const O_RDWR: u64 = 0o2;
    const O_APPEND: u64 = 0o2000;
    const O_NONBLOCK: u64 = 0o4000;

    let Some(entry) = crate::task::scheduler::current_fd_get(fd) else {
        return linux_err(EBADF);
    };

    let mut flags = if entry.flags.nonblock { O_NONBLOCK } else { 0 };
    flags |= match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => {
            match crate::fs::vfs::get_fd_flags(global_id) {
                Ok(file_flags) => {
                    let access = match (file_flags.read, file_flags.write) {
                        (true, true) => O_RDWR,
                        (false, true) => O_WRONLY,
                        _ => 0,
                    };
                    access | if file_flags.append { O_APPEND } else { 0 }
                }
                Err(_) => return linux_err(EBADF),
            }
        }
        crate::fs::fd_table::FdKind::PipeRead { .. }
        | crate::fs::fd_table::FdKind::LinuxProc { .. } => 0,
        crate::fs::fd_table::FdKind::PipeWrite { .. } => O_WRONLY,
        crate::fs::fd_table::FdKind::Tty
        | crate::fs::fd_table::FdKind::PtySlave { .. }
        | crate::fs::fd_table::FdKind::LinuxSocket { .. }
        | crate::fs::fd_table::FdKind::LinuxFramebuffer { .. } => O_RDWR,
        crate::fs::fd_table::FdKind::None => return linux_err(EBADF),
    };

    flags
}

pub(super) fn linux_flock(fd: u32, operation: u64) -> u64 {
    const LOCK_SH: u64 = 1;
    const LOCK_EX: u64 = 2;
    const LOCK_NB: u64 = 4;
    const LOCK_UN: u64 = 8;

    if crate::task::scheduler::current_fd_get(fd).is_none() {
        return linux_err(EBADF);
    }

    match operation & !(LOCK_NB) {
        LOCK_SH | LOCK_EX | LOCK_UN => 0,
        _ => linux_err(EINVAL),
    }
}

pub(super) fn linux_read(fd: u32, buf_ptr: u64, len: u64) -> u64 {
    if len > u32::MAX as u64 {
        return linux_err(EINVAL);
    }
    let Some(entry) = crate::task::scheduler::current_fd_get(fd) else {
        return linux_err(EBADF);
    };
    match entry.kind {
        crate::fs::fd_table::FdKind::LinuxProc {
            file,
            pid,
            position,
        } => {
            return linux_read_proc_file(fd, file, pid, position, buf_ptr, len as u32);
        }
        crate::fs::fd_table::FdKind::LinuxSocket { .. } => {
            return socket_read(fd, buf_ptr, len);
        }
        crate::fs::fd_table::FdKind::LinuxFramebuffer { position } => {
            return linux_fb_read(fd, position, buf_ptr, len);
        }
        _ => {}
    }
    let ret = anyos_u32_ret(handlers::sys_read(fd, buf_ptr, len as u32));
    if let crate::fs::fd_table::FdKind::File { global_id } = entry.kind {
        trace::trace_file_io("file-read", fd, global_id, len, ret, buf_ptr);
    }
    ret
}

pub(super) fn linux_write(fd: u32, buf_ptr: u64, len: u64) -> u64 {
    if len > u32::MAX as u64 {
        return linux_err(EINVAL);
    }
    let Some(entry) = crate::task::scheduler::current_fd_get(fd) else {
        return linux_err(EBADF);
    };
    if let crate::fs::fd_table::FdKind::LinuxSocket { .. } = entry.kind {
        return socket_write(fd, buf_ptr, len);
    }
    if let crate::fs::fd_table::FdKind::LinuxFramebuffer { position } = entry.kind {
        return linux_fb_write(fd, position, buf_ptr, len);
    }
    let ret = anyos_u32_ret(handlers::sys_write(fd, buf_ptr, len as u32));
    if let crate::fs::fd_table::FdKind::File { global_id } = entry.kind {
        trace::trace_file_io("file-write", fd, global_id, len, ret, buf_ptr);
    }
    ret
}

pub(super) fn linux_pipe2(pipefd_ptr: u64, linux_flags: u64) -> u64 {
    const O_NONBLOCK: u64 = 0x800;
    const O_CLOEXEC: u64 = 0x80000;
    const SUPPORTED: u64 = O_NONBLOCK | O_CLOEXEC;

    if (linux_flags & !SUPPORTED) != 0 {
        return linux_err(EINVAL);
    }
    if pipefd_ptr == 0 || !handlers::helpers::is_user_range_accessible(pipefd_ptr, 8) {
        return linux_err(EFAULT);
    }

    let anyos_flags = if (linux_flags & O_CLOEXEC) != 0 {
        0x10
    } else {
        0
    };
    let ret = handlers::sys_pipe2(pipefd_ptr, anyos_flags);
    if (ret as i32) < 0 {
        return anyos_u32_ret(ret);
    }

    if (linux_flags & O_NONBLOCK) != 0 {
        let read_fd = unsafe { *((pipefd_ptr) as *const u32) };
        let write_fd = unsafe { *((pipefd_ptr + 4) as *const u32) };
        crate::task::scheduler::current_fd_set_nonblock(read_fd, true);
        crate::task::scheduler::current_fd_set_nonblock(write_fd, true);
    }

    0
}

pub(super) fn linux_lseek(fd: u32, offset: u64, whence: u64) -> u64 {
    // Positions cover the full u64 range — the final range check happens in
    // vfs::lseek after whence is applied.
    let signed_offset = offset as i64;
    let whence = match whence {
        0 | 1 | 2 => whence as u32,
        _ => return linux_err(EINVAL),
    };

    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        match entry.kind {
            crate::fs::fd_table::FdKind::File { global_id } => {
                return match crate::fs::vfs::lseek(global_id, signed_offset, whence) {
                    Ok(pos) => pos as u64,
                    Err(e) => linux_err(fs_errno(e)),
                };
            }
            crate::fs::fd_table::FdKind::LinuxProc {
                file,
                pid,
                position,
            } => {
                let size = linux_proc_content_len(file, pid) as i64;
                let base = match whence {
                    0 => 0,
                    1 => position as i64,
                    2 => size,
                    _ => unreachable!(),
                };
                let next = base.saturating_add(signed_offset);
                if next < 0 || next > u32::MAX as i64 {
                    return linux_err(EINVAL);
                }
                if !crate::task::scheduler::current_fd_set_linux_proc_position(fd, next as u32) {
                    return linux_err(EBADF);
                }
                return next as u64;
            }
            crate::fs::fd_table::FdKind::LinuxFramebuffer { position } => {
                return linux_fb_lseek(fd, position, signed_offset, whence);
            }
            crate::fs::fd_table::FdKind::PipeRead { .. }
            | crate::fs::fd_table::FdKind::PipeWrite { .. }
            | crate::fs::fd_table::FdKind::Tty
            | crate::fs::fd_table::FdKind::PtySlave { .. }
            | crate::fs::fd_table::FdKind::LinuxSocket { .. } => return linux_err(ESPIPE),
            crate::fs::fd_table::FdKind::None => return linux_err(EBADF),
        }
    } else {
        return linux_err(EBADF);
    }
}

pub(super) fn linux_pread64(fd: u32, buf_ptr: u64, len: u64, offset: u64) -> u64 {
    if buf_ptr == 0 || len > usize::MAX as u64 {
        return linux_err(EFAULT);
    }
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxProc { file, pid, .. } = entry.kind {
            if offset > u32::MAX as u64 || len > u32::MAX as u64 {
                return linux_err(EINVAL);
            }
            return linux_copy_proc_content(file, pid, offset as u32, buf_ptr, len as u32);
        }
        if let crate::fs::fd_table::FdKind::LinuxFramebuffer { .. } = entry.kind {
            return linux_fb_pread(buf_ptr, len, offset);
        }
    }
    match linux_read_fd_at(fd, buf_ptr, len as usize, offset) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
}

pub(super) fn linux_pwrite64(fd: u32, buf_ptr: u64, len: u64, offset: u64) -> u64 {
    if len == 0 {
        return 0;
    }
    if buf_ptr == 0 || len > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxFramebuffer { .. } = entry.kind {
            return linux_fb_pwrite(buf_ptr, len, offset);
        }
    }
    match linux_write_fd_at(fd, buf_ptr, len as usize, offset) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
}

pub(super) fn linux_sendfile(out_fd: u32, in_fd: u32, offset_ptr: u64, count: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    if offset_ptr != 0 && !handlers::helpers::is_user_range_accessible(offset_ptr, 8) {
        return linux_err(EFAULT);
    }
    let mut explicit_offset = if offset_ptr != 0 {
        let raw = unsafe { read_u64(offset_ptr, 0) } as i64;
        if raw < 0 {
            return linux_err(EINVAL);
        }
        Some(raw as u64)
    } else {
        None
    };

    let mut total = 0usize;
    let limit = count.min(u32::MAX as u64) as usize;
    let mut buf = [0u8; LINUX_COPY_CHUNK];
    while total < limit {
        let want = core::cmp::min(buf.len(), limit - total);
        let read_offset = explicit_offset;
        let nread = match linux_read_fd_kernel(in_fd, &mut buf[..want], read_offset) {
            Ok(0) => break,
            Ok(n) => n,
            Err(errno) => {
                return if total > 0 {
                    total as u64
                } else {
                    linux_err(errno)
                };
            }
        };
        let nwritten = match linux_write_fd_kernel(out_fd, &buf[..nread], None) {
            Ok(n) => n,
            Err(errno) => {
                return if total > 0 {
                    total as u64
                } else {
                    linux_err(errno)
                };
            }
        };
        if let Some(off) = explicit_offset {
            explicit_offset = Some(off.saturating_add(nwritten as u64));
        }
        total += nwritten;
        if nwritten < nread {
            break;
        }
    }

    if let Some(off) = explicit_offset {
        unsafe {
            write_u64(offset_ptr, 0, off);
        }
    }
    total as u64
}

pub(super) fn linux_copy_file_range(
    fd_in: u32,
    off_in_ptr: u64,
    fd_out: u32,
    off_out_ptr: u64,
    len: u64,
    flags: u64,
) -> u64 {
    if flags != 0 {
        return linux_err(EINVAL);
    }
    if len == 0 {
        return 0;
    }
    if off_in_ptr != 0 && !handlers::helpers::is_user_range_accessible(off_in_ptr, 8) {
        return linux_err(EFAULT);
    }
    if off_out_ptr != 0 && !handlers::helpers::is_user_range_accessible(off_out_ptr, 8) {
        return linux_err(EFAULT);
    }

    let mut off_in = if off_in_ptr != 0 {
        Some(unsafe { read_u64(off_in_ptr, 0) })
    } else {
        None
    };
    let mut off_out = if off_out_ptr != 0 {
        Some(unsafe { read_u64(off_out_ptr, 0) })
    } else {
        None
    };

    let mut total = 0usize;
    let limit = len.min(u32::MAX as u64) as usize;
    let mut buf = [0u8; LINUX_COPY_CHUNK];
    while total < limit {
        let want = core::cmp::min(buf.len(), limit - total);
        let nread = match linux_read_fd_kernel(fd_in, &mut buf[..want], off_in) {
            Ok(0) => break,
            Ok(n) => n,
            Err(errno) => {
                return if total > 0 {
                    total as u64
                } else {
                    linux_err(errno)
                };
            }
        };
        let nwritten = match linux_write_fd_kernel(fd_out, &buf[..nread], off_out) {
            Ok(n) => n,
            Err(errno) => {
                return if total > 0 {
                    total as u64
                } else {
                    linux_err(errno)
                };
            }
        };
        if let Some(off) = off_in {
            off_in = Some(off.saturating_add(nwritten as u64));
        }
        if let Some(off) = off_out {
            off_out = Some(off.saturating_add(nwritten as u64));
        }
        total += nwritten;
        if nwritten < nread {
            break;
        }
    }

    if let Some(off) = off_in {
        unsafe {
            write_u64(off_in_ptr, 0, off);
        }
    }
    if let Some(off) = off_out {
        unsafe {
            write_u64(off_out_ptr, 0, off);
        }
    }
    total as u64
}

fn linux_file_global_id(fd: u32) -> Result<u32, i32> {
    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => Ok(global_id),
        _ => Err(EBADF),
    }
}

fn linux_read_fd_kernel(fd: u32, buf: &mut [u8], offset: Option<u64>) -> Result<usize, i32> {
    let global_id = linux_file_global_id(fd)?;
    if let Some(offset) = offset {
        if offset > i64::MAX as u64 {
            return Err(EINVAL);
        }
        // Position-independent read: read_at does not disturb the file's
        // stored position, so no fstat (a full directory walk) or lseek
        // save/restore is needed. Fall back to the seek dance only if the
        // backend cannot do positional reads.
        match crate::fs::vfs::read_at(global_id, offset, buf) {
            Ok(n) => Ok(n),
            Err(crate::fs::vfs::FsError::NotSupported) => {
                let old_pos = crate::fs::vfs::lseek(global_id, 0, 1).map_err(fs_errno)?;
                crate::fs::vfs::lseek(global_id, offset as i64, 0).map_err(fs_errno)?;
                let result = crate::fs::vfs::read(global_id, buf).map_err(fs_errno);
                let _ = crate::fs::vfs::lseek(global_id, old_pos as i64, 0);
                result
            }
            Err(e) => Err(fs_errno(e)),
        }
    } else {
        crate::fs::vfs::read(global_id, buf).map_err(fs_errno)
    }
}

fn linux_write_fd_kernel(fd: u32, buf: &[u8], offset: Option<u64>) -> Result<usize, i32> {
    let global_id = linux_file_global_id(fd)?;
    if let Some(offset) = offset {
        if offset > i64::MAX as u64 {
            return Err(EINVAL);
        }
        // Capture the position with SEEK_CUR (cheap) instead of fstat (a full
        // directory walk) just to restore it after the positional write.
        let old_pos = crate::fs::vfs::lseek(global_id, 0, 1).map_err(fs_errno)?;
        crate::fs::vfs::lseek(global_id, offset as i64, 0).map_err(fs_errno)?;
        let result = crate::fs::vfs::write(global_id, buf).map_err(fs_errno);
        let _ = crate::fs::vfs::lseek(global_id, old_pos as i64, 0);
        result
    } else {
        crate::fs::vfs::write(global_id, buf).map_err(fs_errno)
    }
}

fn linux_write_fd_at(fd: u32, buf_ptr: u64, len: usize, offset: u64) -> Result<usize, i32> {
    // off_t is signed; positions beyond i64::MAX are invalid.
    if offset > i64::MAX as u64 {
        return Err(EINVAL);
    }
    let global_id = linux_file_global_id(fd)?;
    // SEEK_CUR captures the position without the directory walk fstat does.
    let old_pos = crate::fs::vfs::lseek(global_id, 0, 1).map_err(fs_errno)?;
    crate::fs::vfs::lseek(global_id, offset as i64, 0).map_err(fs_errno)?;

    let mut total = 0usize;
    let mut result = Ok(0usize);
    while total < len {
        let chunk_len = core::cmp::min(LINUX_COPY_CHUNK, len - total);
        let Some(buf) = handlers::helpers::copy_user_bytes(
            buf_ptr.wrapping_add(total as u64),
            chunk_len,
            LINUX_COPY_CHUNK,
        ) else {
            result = if total > 0 { Ok(total) } else { Err(EFAULT) };
            break;
        };
        match crate::fs::vfs::write(global_id, &buf).map_err(fs_errno) {
            Ok(0) => {
                result = Ok(total);
                break;
            }
            Ok(n) => {
                total += n;
                if n < chunk_len {
                    result = Ok(total);
                    break;
                }
                result = Ok(total);
            }
            Err(errno) => {
                result = if total > 0 { Ok(total) } else { Err(errno) };
                break;
            }
        }
    }

    let _ = crate::fs::vfs::lseek(global_id, old_pos as i64, 0);
    result
}

pub(super) fn linux_readv(fd: u32, iov_ptr: u64, iovcnt: u64) -> u64 {
    linux_iov_io(fd, iov_ptr, iovcnt, false)
}

pub(super) fn linux_writev(fd: u32, iov_ptr: u64, iovcnt: u64) -> u64 {
    linux_iov_io(fd, iov_ptr, iovcnt, true)
}

pub(super) fn linux_iov_io(fd: u32, iov_ptr: u64, iovcnt: u64, write: bool) -> u64 {
    const IOV_MAX: u64 = 1024;
    if iov_ptr == 0 || iovcnt > IOV_MAX {
        return linux_err(EINVAL);
    }
    let Some(iov_bytes) = iovcnt.checked_mul(16) else {
        return linux_err(EINVAL);
    };
    if !handlers::helpers::is_user_range_accessible(iov_ptr, iov_bytes) {
        return linux_err(EFAULT);
    }
    if write {
        if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
            if let crate::fs::fd_table::FdKind::File { global_id } = entry.kind {
                return linux_writev_file(global_id, iov_ptr, iovcnt);
            }
        }
    }
    let mut total = 0u64;
    for i in 0..iovcnt {
        let base = unsafe { read_u64(iov_ptr, i * 16) };
        let len = unsafe { read_u64(iov_ptr, i * 16 + 8) };
        if len == 0 {
            continue;
        }
        if len > u32::MAX as u64 {
            return linux_err(EINVAL);
        }
        let ret = if write {
            let ret = linux_write(fd, base, len);
            if (ret as i64) < 0 {
                ret as u32
            } else {
                ret as u32
            }
        } else {
            let ret = linux_read(fd, base, len);
            if (ret as i64) < 0 {
                ret as u32
            } else {
                ret as u32
            }
        };
        if (ret as i32) < 0 {
            return if total == 0 {
                anyos_u32_ret(ret)
            } else {
                total
            };
        }
        total += ret as u64;
        if ret as u64 != len {
            break;
        }
    }
    total
}

fn linux_writev_file(global_id: u32, iov_ptr: u64, iovcnt: u64) -> u64 {
    let coalesce_cap = crate::fs::vfs::preferred_write_chunk(global_id)
        .min(LINUX_WRITEV_COALESCE_MAX)
        .max(LINUX_COPY_CHUNK);
    let mut pending = Vec::with_capacity(coalesce_cap);
    let mut total = 0u64;

    for i in 0..iovcnt {
        let base = unsafe { read_u64(iov_ptr, i * 16) };
        let len = unsafe { read_u64(iov_ptr, i * 16 + 8) };
        if len == 0 {
            continue;
        }
        if len > u32::MAX as u64 {
            return linux_err(EINVAL);
        }

        let mut copied = 0usize;
        let len_usize = len as usize;
        while copied < len_usize {
            if pending.len() == coalesce_cap {
                match linux_writev_flush_pending(global_id, &mut pending, &mut total) {
                    Ok(true) => {}
                    Ok(false) => return total,
                    Err(errno) => return if total == 0 { linux_err(errno) } else { total },
                }
            }

            let room = coalesce_cap - pending.len();
            let take = (len_usize - copied).min(room);
            let Some(ptr) = base.checked_add(copied as u64) else {
                match linux_writev_flush_pending(global_id, &mut pending, &mut total) {
                    Ok(true) | Ok(false) => {}
                    Err(errno) => return if total == 0 { linux_err(errno) } else { total },
                }
                return if total == 0 { linux_err(EFAULT) } else { total };
            };

            if !linux_append_user_bytes(&mut pending, ptr, take) {
                match linux_writev_flush_pending(global_id, &mut pending, &mut total) {
                    Ok(true) | Ok(false) => {}
                    Err(errno) => return if total == 0 { linux_err(errno) } else { total },
                }
                return if total == 0 { linux_err(EFAULT) } else { total };
            }
            copied += take;
        }
    }

    match linux_writev_flush_pending(global_id, &mut pending, &mut total) {
        Ok(_) => total,
        Err(errno) => {
            if total == 0 {
                linux_err(errno)
            } else {
                total
            }
        }
    }
}

fn linux_append_user_bytes(out: &mut Vec<u8>, ptr: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    if !handlers::helpers::is_user_range_accessible(ptr, len as u64) {
        return false;
    }
    unsafe {
        let src = core::slice::from_raw_parts(ptr as usize as *const u8, len);
        out.extend_from_slice(src);
    }
    true
}

fn linux_writev_flush_pending(
    global_id: u32,
    pending: &mut Vec<u8>,
    total: &mut u64,
) -> Result<bool, i32> {
    if pending.is_empty() {
        return Ok(true);
    }
    let requested = pending.len();
    match crate::fs::vfs::write(global_id, pending.as_slice()).map_err(fs_errno) {
        Ok(n) => {
            *total += n as u64;
            pending.clear();
            Ok(n == requested)
        }
        Err(errno) => Err(errno),
    }
}

pub(super) fn linux_select(
    nfds: u64,
    readfds: u64,
    writefds: u64,
    exceptfds: u64,
    timeout: u64,
) -> u64 {
    if nfds > 1024 {
        return linux_err(EINVAL);
    }
    let fdset_bytes = (((nfds + 63) / 64) * 8) as usize;
    for ptr in [readfds, writefds, exceptfds] {
        if ptr != 0
            && fdset_bytes != 0
            && !handlers::helpers::is_user_range_accessible(ptr, fdset_bytes as u64)
        {
            return linux_err(EFAULT);
        }
    }

    let mut read_in = [0u8; 128];
    let mut write_in = [0u8; 128];
    select_fdset_snapshot(readfds, fdset_bytes, &mut read_in);
    select_fdset_snapshot(writefds, fdset_bytes, &mut write_in);

    let start = crate::arch::hal::timer_current_ticks();
    let timeout_ticks = match select_timeval_ticks(timeout) {
        Ok(ticks) => ticks,
        Err(errno) => return linux_err(errno),
    };

    loop {
        crate::net::poll();
        let ready = linux_select_once(nfds, &read_in, &write_in, readfds, writefds, exceptfds);
        if ready != 0 || timeout_ticks == Some(0) {
            return ready;
        }
        if let Some(limit) = timeout_ticks {
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= limit {
                select_fdset_zero(readfds, fdset_bytes);
                select_fdset_zero(writefds, fdset_bytes);
                select_fdset_zero(exceptfds, fdset_bytes);
                return 0;
            }
        }
        crate::net::wait_for_poll_progress();
    }
}

pub(super) fn linux_pselect6(
    nfds: u64,
    readfds: u64,
    writefds: u64,
    exceptfds: u64,
    timeout: u64,
    sigmask_data: u64,
) -> u64 {
    if timeout != 0 && !handlers::helpers::is_user_range_accessible(timeout, 16) {
        return linux_err(EFAULT);
    }
    if sigmask_data != 0 {
        if !handlers::helpers::is_user_range_accessible(sigmask_data, 16) {
            return linux_err(EFAULT);
        }
        let sigmask = unsafe { read_u64(sigmask_data, 0) };
        let sigsetsize = unsafe { read_u64(sigmask_data, 8) };
        if sigmask != 0 {
            if sigsetsize != 8 {
                return linux_err(EINVAL);
            }
            if !handlers::helpers::is_user_range_accessible(sigmask, sigsetsize) {
                return linux_err(EFAULT);
            }
        }
    }

    linux_pselect(nfds, readfds, writefds, exceptfds, timeout)
}

fn linux_pselect(nfds: u64, readfds: u64, writefds: u64, exceptfds: u64, timeout: u64) -> u64 {
    if nfds > 1024 {
        return linux_err(EINVAL);
    }
    let fdset_bytes = (((nfds + 63) / 64) * 8) as usize;
    for ptr in [readfds, writefds, exceptfds] {
        if ptr != 0
            && fdset_bytes != 0
            && !handlers::helpers::is_user_range_accessible(ptr, fdset_bytes as u64)
        {
            return linux_err(EFAULT);
        }
    }

    let mut read_in = [0u8; 128];
    let mut write_in = [0u8; 128];
    select_fdset_snapshot(readfds, fdset_bytes, &mut read_in);
    select_fdset_snapshot(writefds, fdset_bytes, &mut write_in);

    let start = crate::arch::hal::timer_current_ticks();
    let timeout_ticks = match select_timespec_ticks(timeout) {
        Ok(ticks) => ticks,
        Err(errno) => return linux_err(errno),
    };

    loop {
        crate::net::poll();
        let ready = linux_select_once(nfds, &read_in, &write_in, readfds, writefds, exceptfds);
        if ready != 0 || timeout_ticks == Some(0) {
            return ready;
        }
        if let Some(limit) = timeout_ticks {
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= limit {
                select_fdset_zero(readfds, fdset_bytes);
                select_fdset_zero(writefds, fdset_bytes);
                select_fdset_zero(exceptfds, fdset_bytes);
                return 0;
            }
        }
        crate::net::wait_for_poll_progress();
    }
}

fn linux_select_once(
    nfds: u64,
    read_in: &[u8; 128],
    write_in: &[u8; 128],
    readfds: u64,
    writefds: u64,
    exceptfds: u64,
) -> u64 {
    let mut ready = 0u64;
    select_fdset_zero(readfds, (((nfds + 63) / 64) * 8) as usize);
    select_fdset_zero(writefds, (((nfds + 63) / 64) * 8) as usize);
    select_fdset_zero(exceptfds, (((nfds + 63) / 64) * 8) as usize);

    for fd in 0..nfds {
        let want_read = select_fd_is_set_snapshot(read_in, fd);
        let want_write = select_fd_is_set_snapshot(write_in, fd);
        if !want_read && !want_write {
            continue;
        }

        let requested =
            (if want_read { 0x0001 } else { 0 }) | (if want_write { 0x0004 } else { 0 });
        let revents = linux_fd_poll_revents_for_fd(fd as u32, requested, false);

        if want_read && revents & 0x0001 != 0 {
            ready += 1;
            select_fd_set(readfds, fd);
        }
        if want_write && revents & 0x0004 != 0 {
            ready += 1;
            select_fd_set(writefds, fd);
        }
    }
    ready
}

fn select_fdset_snapshot(set_ptr: u64, len: usize, out: &mut [u8; 128]) {
    if set_ptr == 0 {
        return;
    }
    for i in 0..len {
        out[i] = unsafe { *((set_ptr + i as u64) as *const u8) };
    }
}

fn select_fdset_zero(set_ptr: u64, len: usize) {
    if set_ptr == 0 {
        return;
    }
    for i in 0..len {
        unsafe {
            *((set_ptr + i as u64) as *mut u8) = 0;
        }
    }
}

fn select_fd_is_set_snapshot(set: &[u8; 128], fd: u64) -> bool {
    let idx = (fd / 8) as usize;
    (set[idx] & (1u8 << (fd & 7))) != 0
}

fn select_fd_set(set_ptr: u64, fd: u64) {
    if set_ptr == 0 {
        return;
    }
    unsafe {
        let byte = (set_ptr + fd / 8) as *mut u8;
        *byte |= 1u8 << (fd & 7);
    }
}

pub(super) fn linux_poll(fds_ptr: u64, nfds: u64, timeout: u64) -> u64 {
    if nfds == 0 {
        return 0;
    }
    if fds_ptr == 0 || nfds > 1024 {
        return linux_err(EFAULT);
    }
    let Some(fds_bytes) = nfds.checked_mul(8) else {
        return linux_err(EFAULT);
    };
    if !handlers::helpers::is_user_range_accessible(fds_ptr, fds_bytes) {
        return linux_err(EFAULT);
    }

    let timeout_ms = timeout as i32;
    let start = crate::arch::hal::timer_current_ticks();
    let timeout_ticks = if timeout_ms < 0 {
        None
    } else {
        Some(ms_to_ticks(timeout_ms as u64))
    };

    loop {
        crate::net::poll();
        let ready = linux_poll_once(fds_ptr, nfds);
        if ready != 0 || timeout_ticks == Some(0) {
            return ready;
        }
        if let Some(limit) = timeout_ticks {
            let now = crate::arch::hal::timer_current_ticks();
            if now.wrapping_sub(start) >= limit {
                return 0;
            }
        }
        crate::net::wait_for_poll_progress();
    }
}

fn linux_poll_once(fds_ptr: u64, nfds: u64) -> u64 {
    let mut ready = 0u64;
    for i in 0..nfds {
        let base = fds_ptr + i * 8;
        let fd = unsafe { *((base) as *const i32) };
        let events = unsafe { *((base + 4) as *const i16) };
        let mut revents = 0i16;
        if fd >= 0 {
            revents = linux_fd_poll_revents_for_fd(fd as u32, events, true);
            if revents != 0 {
                ready += 1;
            }
        }
        unsafe {
            *((base + 6) as *mut i16) = revents;
        }
    }
    ready
}

fn linux_fd_poll_revents_for_fd(fd: u32, events: i16, report_nval: bool) -> i16 {
    const POLLNVAL: i16 = 0x0020;

    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        return linux_fd_poll_revents(entry.kind, events);
    }

    if report_nval {
        POLLNVAL
    } else {
        0
    }
}

fn linux_fd_poll_revents(kind: crate::fs::fd_table::FdKind, events: i16) -> i16 {
    const POLLIN: i16 = 0x0001;
    const POLLOUT: i16 = 0x0004;
    const POLLERR: i16 = 0x0008;
    const POLLHUP: i16 = 0x0010;

    match kind {
        crate::fs::fd_table::FdKind::LinuxSocket { socket_id } => {
            linux_socket_poll_revents(socket_id, events)
        }
        crate::fs::fd_table::FdKind::PipeRead { pipe_id } => {
            let mut revents = 0;
            let available = crate::ipc::anon_pipe::bytes_available(pipe_id);
            if (events & POLLIN) != 0
                && (available > 0 || crate::ipc::anon_pipe::is_write_closed(pipe_id))
            {
                revents |= POLLIN;
            }
            if available == 0 && crate::ipc::anon_pipe::is_write_closed(pipe_id) {
                revents |= POLLHUP;
            }
            revents
        }
        crate::fs::fd_table::FdKind::PipeWrite { pipe_id } => {
            if crate::ipc::anon_pipe::is_read_closed(pipe_id) {
                POLLERR
            } else if (events & POLLOUT) != 0
                && crate::ipc::anon_pipe::bytes_available(pipe_id)
                    < crate::ipc::anon_pipe::PIPE_BUF_SIZE as u32
            {
                POLLOUT
            } else {
                0
            }
        }
        crate::fs::fd_table::FdKind::PtySlave { pty_id } => {
            let mut revents = 0;
            if (events & POLLIN) != 0 && crate::ipc::pty::bytes_available(pty_id) != 0 {
                revents |= POLLIN;
            }
            if (events & POLLOUT) != 0 {
                revents |= POLLOUT;
            }
            revents
        }
        crate::fs::fd_table::FdKind::File { .. }
        | crate::fs::fd_table::FdKind::Tty
        | crate::fs::fd_table::FdKind::LinuxProc { .. }
        | crate::fs::fd_table::FdKind::LinuxFramebuffer { .. } => events & (POLLIN | POLLOUT),
        crate::fs::fd_table::FdKind::None => POLLERR,
    }
}

fn select_timeval_ticks(timeout: u64) -> Result<Option<u32>, i32> {
    if timeout == 0 {
        return Ok(None);
    }
    if !handlers::helpers::is_user_range_accessible(timeout, 16) {
        return Err(EFAULT);
    }
    let sec = unsafe { read_u64(timeout, 0) } as i64;
    let usec = unsafe { read_u64(timeout, 8) } as i64;
    if sec < 0 || !(0..1_000_000).contains(&usec) {
        return Err(EINVAL);
    }
    let ms = (sec as u64)
        .saturating_mul(1000)
        .saturating_add((usec as u64).saturating_add(999) / 1000);
    Ok(Some(ms_to_ticks(ms)))
}

fn select_timespec_ticks(timeout: u64) -> Result<Option<u32>, i32> {
    if timeout == 0 {
        return Ok(None);
    }
    if !handlers::helpers::is_user_range_accessible(timeout, 16) {
        return Err(EFAULT);
    }
    let sec = unsafe { read_u64(timeout, 0) } as i64;
    let nsec = unsafe { read_u64(timeout, 8) } as i64;
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return Err(EINVAL);
    }
    let ms = (sec as u64)
        .saturating_mul(1000)
        .saturating_add((nsec as u64).saturating_add(999_999) / 1_000_000);
    Ok(Some(ms_to_ticks(ms)))
}

fn ms_to_ticks(ms: u64) -> u32 {
    if ms == 0 {
        return 0;
    }
    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u128;
    let ticks = (ms as u128).saturating_mul(hz).saturating_add(999) / 1000;
    ticks.clamp(1, u32::MAX as u128) as u32
}

pub(super) fn linux_ioctl(fd: u32, request: u64, arg: u64) -> u64 {
    if let Some(ret) = linux_fb_ioctl(fd, request, arg) {
        return ret;
    }

    const TCGETS: u64 = 0x5401;
    const TCSETS: u64 = 0x5402;
    const TCSETSW: u64 = 0x5403;
    const TCSETSF: u64 = 0x5404;
    const TCGETS2: u64 = 0x802c542a;
    const TIOCGPGRP: u64 = 0x540F;
    const TIOCSPGRP: u64 = 0x5410;
    const TIOCGWINSZ: u64 = 0x5413;
    const TIOCSWINSZ: u64 = 0x5414;
    const FIONREAD: u64 = 0x541B;
    const TIOCLINUX: u64 = 0x541C;
    match request {
        TCGETS => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 {
                if !handlers::helpers::is_user_range_accessible(arg, 36) {
                    return linux_err(EFAULT);
                }
                if let Some(pty_id) = linux_fd_pty_id(fd) {
                    let termios = crate::ipc::pty::get_termios(pty_id)
                        .unwrap_or_else(crate::ipc::pty::Termios::default);
                    linux_write_termios_value(arg, termios);
                } else {
                    linux_write_termios(arg);
                }
            }
            crate::serial_verbose_println!("lxe linux ioctl: TCGETS fd={} -> ok", fd);
            0
        }
        TCGETS2 => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 {
                if !handlers::helpers::is_user_range_accessible(arg, 44) {
                    return linux_err(EFAULT);
                }
                if let Some(pty_id) = linux_fd_pty_id(fd) {
                    let termios = crate::ipc::pty::get_termios(pty_id)
                        .unwrap_or_else(crate::ipc::pty::Termios::default);
                    linux_write_termios2_value(arg, termios);
                } else {
                    linux_write_termios2(arg);
                }
            }
            crate::serial_verbose_println!("lxe linux ioctl: TCGETS2 fd={} -> ok", fd);
            0
        }
        TCSETS | TCSETSW | TCSETSF => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 && !handlers::helpers::is_user_range_accessible(arg, 36) {
                return linux_err(EFAULT);
            }
            if arg != 0 {
                if let Some(pty_id) = linux_fd_pty_id(fd) {
                    let termios = linux_read_termios_value(arg);
                    crate::ipc::pty::set_termios(pty_id, termios);
                }
            }
            crate::serial_verbose_println!(
                "lxe linux ioctl: TCSETS* fd={} request={:#x} -> ok",
                fd,
                request
            );
            0
        }
        TIOCGPGRP => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 4) {
                return linux_err(EFAULT);
            }
            unsafe {
                write_u32(arg, 0, crate::task::scheduler::current_tid());
            }
            0
        }
        TIOCSPGRP => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 && !handlers::helpers::is_user_range_accessible(arg, 4) {
                return linux_err(EFAULT);
            }
            0
        }
        TIOCGWINSZ => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 {
                if !handlers::helpers::is_user_range_accessible(arg, 8) {
                    return linux_err(EFAULT);
                }
                let (rows, cols) = if let Some(pty_id) = linux_fd_pty_id(fd) {
                    crate::ipc::pty::get_winsize(pty_id).unwrap_or((25, 80))
                } else {
                    let packed = handlers::sys_con_get_size();
                    ((packed & 0xFFFF) as u16, (packed >> 16) as u16)
                };
                unsafe {
                    *((arg + 0) as *mut u16) = rows;
                    *((arg + 2) as *mut u16) = cols;
                    *((arg + 4) as *mut u16) = 0;
                    *((arg + 6) as *mut u16) = 0;
                }
            }
            crate::serial_verbose_println!("lxe linux ioctl: TIOCGWINSZ fd={} -> ok", fd);
            0
        }
        TIOCSWINSZ => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 && !handlers::helpers::is_user_range_accessible(arg, 8) {
                return linux_err(EFAULT);
            }
            if arg != 0 {
                if let Some(pty_id) = linux_fd_pty_id(fd) {
                    let rows = unsafe { *((arg + 0) as *const u16) };
                    let cols = unsafe { *((arg + 2) as *const u16) };
                    let _ = crate::ipc::pty::set_winsize(pty_id, rows, cols);
                }
            }
            crate::serial_verbose_println!("lxe linux ioctl: TIOCSWINSZ fd={} -> ok", fd);
            0
        }
        FIONREAD => {
            if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
                if let crate::fs::fd_table::FdKind::LinuxSocket { socket_id } = entry.kind {
                    return linux_socket_fionread(socket_id, arg);
                }
                if let crate::fs::fd_table::FdKind::PipeRead { pipe_id } = entry.kind {
                    if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 4) {
                        return linux_err(EFAULT);
                    }
                    unsafe {
                        write_u32(arg, 0, crate::ipc::anon_pipe::bytes_available(pipe_id));
                    }
                    return 0;
                }
                if let crate::fs::fd_table::FdKind::PtySlave { pty_id } = entry.kind {
                    if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 4) {
                        return linux_err(EFAULT);
                    }
                    unsafe {
                        write_u32(arg, 0, crate::ipc::pty::bytes_available(pty_id));
                    }
                    return 0;
                }
            }
            if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 4) {
                return linux_err(EFAULT);
            }
            unsafe {
                write_u32(arg, 0, 0);
            }
            0
        }
        TIOCLINUX => linux_tioclinux(fd, arg),
        _ => {
            crate::serial_verbose_println!(
                "lxe linux ioctl: unsupported fd={} request={:#x}",
                fd,
                request
            );
            linux_err(ENOTTY)
        }
    }
}

fn linux_tioclinux(fd: u32, arg: u64) -> u64 {
    const TIOCL_SETSEL: u8 = 2;
    const TIOCL_PASTESEL: u8 = 3;
    const TIOCL_UNBLANKSCREEN: u8 = 4;
    const TIOCL_SELLOADLUT: u8 = 5;
    const TIOCL_GETSHIFTSTATE: u8 = 6;
    const TIOCL_GETMOUSEREPORTING: u8 = 7;
    const TIOCL_SETVESABLANK: u8 = 10;
    const TIOCL_SETKMSGREDIRECT: u8 = 11;
    const TIOCL_GETFGCONSOLE: u8 = 12;
    const TIOCL_SCROLLCONSOLE: u8 = 13;
    const TIOCL_BLANKSCREEN: u8 = 14;
    const TIOCL_BLANKEDSCREEN: u8 = 15;
    const TIOCL_GETKMSGREDIRECT: u8 = 17;
    const TIOCL_GETBRACKETEDPASTE: u8 = 18;

    if !linux_fd_is_tty(fd) {
        return linux_err(ENOTTY);
    }
    if arg == 0 || !handlers::helpers::is_user_range_accessible(arg, 1) {
        return linux_err(EINVAL);
    }

    let subcode = unsafe { *(arg as *const u8) };
    let ret = match subcode {
        // Removed by Linux 1.1.92; modern kernels reject them.
        0 | 1 | 8 | 9 | 16 => linux_err(EINVAL),

        TIOCL_SETSEL => {
            if !handlers::helpers::is_user_range_accessible(arg, 12) {
                linux_err(EINVAL)
            } else {
                let mode = unsafe { core::ptr::read_unaligned((arg + 10) as *const u16) };
                // Mouse-reporting selection requires console mouse tracking.
                // We expose no Linux virtual-console mouse reporter yet, so
                // this submode must fail like Linux does when reporting is off.
                if (mode & 0x10) != 0 {
                    linux_err(EINVAL)
                } else {
                    0
                }
            }
        }
        TIOCL_PASTESEL => {
            // No virtual-console selection buffer is maintained. Pasting an
            // empty selection is a successful zero-byte operation.
            0
        }
        TIOCL_UNBLANKSCREEN => {
            TIOCL_BLANKED_CONSOLE.store(0, core::sync::atomic::Ordering::Relaxed);
            0
        }
        TIOCL_SELLOADLUT => {
            if handlers::helpers::is_user_range_accessible(arg, 36) {
                0
            } else {
                linux_err(EINVAL)
            }
        }
        TIOCL_GETSHIFTSTATE => {
            unsafe {
                *(arg as *mut u8) = 0;
            }
            0
        }
        TIOCL_GETMOUSEREPORTING => {
            unsafe {
                *(arg as *mut u8) = 0;
            }
            0
        }
        TIOCL_SETVESABLANK => {
            if !handlers::helpers::is_user_range_accessible(arg, 2) {
                linux_err(EINVAL)
            } else {
                let mode = unsafe { *((arg + 1) as *const u8) };
                if mode <= 2 {
                    TIOCL_VESA_BLANK_MODE.store(mode, core::sync::atomic::Ordering::Relaxed);
                    0
                } else {
                    linux_err(EINVAL)
                }
            }
        }
        TIOCL_SETKMSGREDIRECT => {
            if !handlers::helpers::is_user_range_accessible(arg, 2) {
                linux_err(EINVAL)
            } else {
                let vt = unsafe { *((arg + 1) as *const u8) };
                TIOCL_KMSG_REDIRECT.store(vt, core::sync::atomic::Ordering::Relaxed);
                0
            }
        }
        TIOCL_GETFGCONSOLE => 0,
        TIOCL_SCROLLCONSOLE => {
            if handlers::helpers::is_user_range_accessible(arg, 8) {
                0
            } else {
                linux_err(EINVAL)
            }
        }
        TIOCL_BLANKSCREEN => {
            TIOCL_BLANKED_CONSOLE.store(1, core::sync::atomic::Ordering::Relaxed);
            0
        }
        TIOCL_BLANKEDSCREEN => {
            TIOCL_BLANKED_CONSOLE.load(core::sync::atomic::Ordering::Relaxed) as u64
        }
        TIOCL_GETKMSGREDIRECT => {
            TIOCL_KMSG_REDIRECT.load(core::sync::atomic::Ordering::Relaxed) as u64
        }
        TIOCL_GETBRACKETEDPASTE => 0,
        _ => linux_err(EINVAL),
    };

    crate::serial_verbose_println!(
        "lxe linux ioctl: TIOCLINUX fd={} sub={} -> {:#x}",
        fd,
        subcode,
        ret
    );
    ret
}

pub(super) fn linux_fd_is_tty(fd: u32) -> bool {
    if fd < 3 {
        return matches!(
            crate::task::scheduler::current_fd_get(fd).map(|entry| entry.kind),
            Some(crate::fs::fd_table::FdKind::Tty)
                | Some(crate::fs::fd_table::FdKind::PtySlave { .. })
        );
    }
    matches!(
        crate::task::scheduler::current_fd_get(fd).map(|entry| entry.kind),
        Some(crate::fs::fd_table::FdKind::Tty) | Some(crate::fs::fd_table::FdKind::PtySlave { .. })
    )
}

fn linux_fd_pty_id(fd: u32) -> Option<u32> {
    match crate::task::scheduler::current_fd_get(fd).map(|entry| entry.kind) {
        Some(crate::fs::fd_table::FdKind::PtySlave { pty_id }) => Some(pty_id),
        _ => None,
    }
}

pub(super) fn linux_write_termios(arg: u64) {
    // Linux x86_64 TCGETS uses the kernel ABI termios layout:
    // 4 tcflag_t fields, one line byte, and 19 control chars = 36 bytes.
    unsafe {
        core::ptr::write_bytes(arg as *mut u8, 0, 36);
        write_u32(arg, 0, 0x0500); // ICRNL | IXON
        write_u32(arg, 4, 0x0005); // OPOST | ONLCR
        write_u32(arg, 8, 0x00bf); // B38400 | CS8 | CREAD
        write_u32(arg, 12, 0x8a3b); // ISIG | ICANON | ECHO | ECHOE | ECHOK | IEXTEN ...
        *((arg + 16) as *mut u8) = 0;
        let cc = (arg + 17) as *mut u8;
        let defaults = [
            3u8, 28, 127, 21, 4, 0, 1, 0, 17, 19, 26, 0, 18, 15, 23, 22, 0, 0, 0,
        ];
        core::ptr::copy_nonoverlapping(defaults.as_ptr(), cc, defaults.len());
    }
}

fn linux_write_termios_value(arg: u64, termios: crate::ipc::pty::Termios) {
    unsafe {
        core::ptr::write_bytes(arg as *mut u8, 0, 36);
        write_u32(arg, 0, termios.iflag);
        write_u32(arg, 4, termios.oflag);
        write_u32(arg, 8, termios.cflag);
        write_u32(arg, 12, termios.lflag);
        *((arg + 16) as *mut u8) = termios.line;
        core::ptr::copy_nonoverlapping(
            termios.cc.as_ptr(),
            (arg + 17) as *mut u8,
            termios.cc.len(),
        );
    }
}

fn linux_write_termios2(arg: u64) {
    linux_write_termios(arg);
    unsafe {
        write_u32(arg, 36, 38400);
        write_u32(arg, 40, 38400);
    }
}

fn linux_write_termios2_value(arg: u64, termios: crate::ipc::pty::Termios) {
    linux_write_termios_value(arg, termios);
    unsafe {
        write_u32(arg, 36, 38400);
        write_u32(arg, 40, 38400);
    }
}

fn linux_read_termios_value(arg: u64) -> crate::ipc::pty::Termios {
    let mut cc = [0u8; 19];
    unsafe {
        core::ptr::copy_nonoverlapping((arg + 17) as *const u8, cc.as_mut_ptr(), cc.len());
        crate::ipc::pty::Termios {
            iflag: *((arg + 0) as *const u32),
            oflag: *((arg + 4) as *const u32),
            cflag: *((arg + 8) as *const u32),
            lflag: *((arg + 12) as *const u32),
            line: *((arg + 16) as *const u8),
            cc,
        }
    }
}
