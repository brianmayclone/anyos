use super::*;

pub(super) fn linux_fcntl(fd: u32, cmd: u32, arg: u64) -> u64 {
    const F_GETLK: u32 = 5;
    const F_SETLK: u32 = 6;
    const F_SETLKW: u32 = 7;
    const F_UNLCK: u16 = 2;
    match cmd {
        F_GETLK => {
            if crate::task::scheduler::current_fd_get(fd).is_none() {
                return linux_err(EBADF);
            }
            if arg != 0 {
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
        _ => anyos_u32_ret(handlers::sys_fcntl(fd, cmd, arg as u32)),
    }
}

pub(super) fn linux_read(fd: u32, buf_ptr: u64, len: u64) -> u64 {
    if len > u32::MAX as u64 {
        return linux_err(EINVAL);
    }
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxProc { file, position } = entry.kind {
            return linux_read_proc_file(fd, file, position, buf_ptr, len as u32);
        }
    }
    anyos_u32_ret(handlers::sys_read(fd, buf_ptr, len as u32))
}

pub(super) fn linux_lseek(fd: u32, offset: u64, whence: u64) -> u64 {
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxProc { file, position } = entry.kind {
            let size = linux_proc_content(file).len() as i64;
            let base = match whence {
                0 => 0,
                1 => position as i64,
                2 => size,
                _ => return linux_err(EINVAL),
            };
            let next = base.saturating_add(offset as i64);
            if next < 0 || next > u32::MAX as i64 {
                return linux_err(EINVAL);
            }
            if !crate::task::scheduler::current_fd_set_linux_proc_position(fd, next as u32) {
                return linux_err(EBADF);
            }
            return next as u64;
        }
    }
    anyos_u32_ret(handlers::sys_lseek(fd, offset as u32, whence as u32))
}

pub(super) fn linux_pread64(fd: u32, buf_ptr: u64, len: u64, offset: u64) -> u64 {
    if buf_ptr == 0 || len > usize::MAX as u64 {
        return linux_err(EFAULT);
    }
    if let Some(entry) = crate::task::scheduler::current_fd_get(fd) {
        if let crate::fs::fd_table::FdKind::LinuxProc { file, .. } = entry.kind {
            if offset > u32::MAX as u64 || len > u32::MAX as u64 {
                return linux_err(EINVAL);
            }
            return linux_copy_proc_content(file, offset as u32, buf_ptr, len as u32);
        }
    }
    match linux_read_fd_at(fd, buf_ptr, len as usize, offset) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
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
            handlers::sys_write(fd, base, len as u32)
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

pub(super) fn linux_select(
    nfds: u64,
    readfds: u64,
    writefds: u64,
    exceptfds: u64,
    _timeout: u64,
) -> u64 {
    if nfds > 1024 {
        return linux_err(EINVAL);
    }
    if nfds == 0 {
        return 0;
    }
    let fdset_bytes = (((nfds + 63) / 64) * 8) as usize;
    for ptr in [readfds, writefds, exceptfds] {
        if ptr != 0 && !handlers::helpers::is_user_range_accessible(ptr, fdset_bytes as u64) {
            return linux_err(EFAULT);
        }
    }

    let mut ready = 0u64;
    for fd in 0..nfds {
        let valid = fd < 3 || crate::task::scheduler::current_fd_get(fd as u32).is_some();
        if select_fd_is_set(readfds, fd) {
            if valid {
                ready += 1;
            } else {
                select_fd_clear(readfds, fd);
            }
        }
        if select_fd_is_set(writefds, fd) {
            if valid {
                ready += 1;
            } else {
                select_fd_clear(writefds, fd);
            }
        }
        if select_fd_is_set(exceptfds, fd) {
            select_fd_clear(exceptfds, fd);
        }
    }
    ready
}

fn select_fd_is_set(set_ptr: u64, fd: u64) -> bool {
    if set_ptr == 0 {
        return false;
    }
    unsafe {
        let byte = *((set_ptr + fd / 8) as *const u8);
        (byte & (1u8 << (fd & 7))) != 0
    }
}

fn select_fd_clear(set_ptr: u64, fd: u64) {
    if set_ptr == 0 {
        return;
    }
    unsafe {
        let byte = (set_ptr + fd / 8) as *mut u8;
        *byte &= !(1u8 << (fd & 7));
    }
}

pub(super) fn linux_poll(fds_ptr: u64, nfds: u64, _timeout: u64) -> u64 {
    if nfds == 0 {
        return 0;
    }
    if fds_ptr == 0 || nfds > 1024 {
        return linux_err(EFAULT);
    }
    let mut ready = 0u64;
    for i in 0..nfds {
        let base = fds_ptr + i * 8;
        let fd = unsafe { *((base) as *const i32) };
        let events = unsafe { *((base + 4) as *const i16) };
        let mut revents = 0i16;
        if fd >= 0 && (fd < 3 || crate::task::scheduler::current_fd_get(fd as u32).is_some()) {
            revents = events & 0x0005; // POLLIN | POLLOUT
            if revents != 0 {
                ready += 1;
            }
        } else if fd >= 0 {
            revents = 0x0020; // POLLNVAL
            ready += 1;
        }
        unsafe {
            *((base + 6) as *mut i16) = revents;
        }
    }
    ready
}

pub(super) fn linux_ioctl(fd: u32, request: u64, arg: u64) -> u64 {
    const TCGETS: u64 = 0x5401;
    const TCSETS: u64 = 0x5402;
    const TCSETSW: u64 = 0x5403;
    const TCSETSF: u64 = 0x5404;
    const TIOCGPGRP: u64 = 0x540F;
    const TIOCSPGRP: u64 = 0x5410;
    const TIOCGWINSZ: u64 = 0x5413;
    match request {
        TCGETS => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 {
                if !handlers::helpers::is_user_range_accessible(arg, 36) {
                    return linux_err(EFAULT);
                }
                linux_write_termios(arg);
            }
            crate::serial_println!("licof linux ioctl: TCGETS fd={} -> ok", fd);
            0
        }
        TCSETS | TCSETSW | TCSETSF => {
            if !linux_fd_is_tty(fd) {
                return linux_err(EBADF);
            }
            if arg != 0 && !handlers::helpers::is_user_range_accessible(arg, 36) {
                return linux_err(EFAULT);
            }
            crate::serial_println!(
                "licof linux ioctl: TCSETS* fd={} request={:#x} -> ok",
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
                let packed = handlers::sys_con_get_size();
                let cols = (packed >> 16) as u16;
                let rows = (packed & 0xFFFF) as u16;
                unsafe {
                    *((arg + 0) as *mut u16) = rows;
                    *((arg + 2) as *mut u16) = cols;
                    *((arg + 4) as *mut u16) = 0;
                    *((arg + 6) as *mut u16) = 0;
                }
            }
            crate::serial_println!("licof linux ioctl: TIOCGWINSZ fd={} -> ok", fd);
            0
        }
        _ => {
            crate::serial_println!(
                "licof linux ioctl: unsupported fd={} request={:#x}",
                fd,
                request
            );
            linux_err(ENOTTY)
        }
    }
}

pub(super) fn linux_fd_is_tty(fd: u32) -> bool {
    if fd < 3 {
        return true;
    }
    matches!(
        crate::task::scheduler::current_fd_get(fd).map(|entry| entry.kind),
        Some(crate::fs::fd_table::FdKind::Tty)
    )
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
