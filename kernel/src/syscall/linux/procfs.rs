use super::*;

pub(super) fn linux_open_proc_file(abs: &str, file: u8, linux_flags: u64) -> u64 {
    let flags = map_open_flags(linux_flags);
    if (flags & 1) != 0 || (flags & 4) != 0 || (flags & 8) != 0 {
        return linux_err(EACCES);
    }
    let fd =
        match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::LinuxProc {
            file,
            position: 0,
        }) {
            Some(fd) => fd,
            None => return linux_err(EBADF),
        };
    if (linux_flags & 0o2000000) != 0 {
        crate::task::scheduler::current_fd_set_cloexec(fd, true);
    }
    crate::serial_println!("licof linux open-proc: ok linux='{}' fd={}", abs, fd);
    fd as u64
}

pub(super) fn linux_read_proc_file(
    fd: u32,
    file: u8,
    position: u32,
    buf_ptr: u64,
    len: u32,
) -> u64 {
    let copied = linux_copy_proc_content(file, position, buf_ptr, len);
    if (copied as i64) < 0 {
        return copied;
    }
    let next = position.saturating_add(copied as u32);
    if !crate::task::scheduler::current_fd_set_linux_proc_position(fd, next) {
        return linux_err(EBADF);
    }
    copied
}

pub(super) fn linux_copy_proc_content(file: u8, offset: u32, buf_ptr: u64, len: u32) -> u64 {
    if len == 0 {
        return 0;
    }
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let content = linux_proc_content(file);
    let start = offset as usize;
    if start >= content.len() {
        return 0;
    }
    let count = (content.len() - start).min(len as usize);
    if !super::handlers::helpers::copy_to_user_bytes(buf_ptr, &content[start..start + count], count)
    {
        return linux_err(EFAULT);
    }
    count as u64
}

pub(super) fn linux_proc_file_id(path: &str) -> Option<u8> {
    match path {
        "/proc/filesystems" => Some(LINUX_PROC_FILESYSTEMS),
        "/proc/mounts" | "/proc/self/mounts" => Some(LINUX_PROC_MOUNTS),
        "/proc/self/loginuid" => Some(LINUX_PROC_LOGINUID),
        "/proc/self/status" => Some(LINUX_PROC_STATUS),
        _ => None,
    }
}

pub(super) fn linux_proc_content(file: u8) -> &'static [u8] {
    match file {
        LINUX_PROC_FILESYSTEMS => {
            b"nodev\tsysfs\nnodev\trootfs\nnodev\tproc\nnodev\tdevtmpfs\nnodev\tdevpts\nnodev\ttmpfs\nnodev\texfat\nnodev\tcorefs\n"
        }
        LINUX_PROC_MOUNTS => {
            b"rootfs / rootfs rw 0 0\nproc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n"
        }
        LINUX_PROC_LOGINUID => b"4294967295\n",
        LINUX_PROC_STATUS => b"Name:\tlicof\nState:\tR (running)\nTgid:\t1\nPid:\t1\nPPid:\t0\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\n",
        _ => b"",
    }
}

pub(super) fn linux_write_proc_stat(stat_ptr: u64, file: u8) -> u64 {
    write_linux_stat(
        stat_ptr,
        0x1cf,
        0x7000 + file as u64,
        0,
        linux_proc_content(file).len() as u64,
        handlers::sys_getuid(),
        handlers::sys_getgid(),
        0o444,
        0,
    );
    0
}
