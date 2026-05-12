use super::*;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

const PROCFS_DEV: u64 = 0x1cf;
const PROCFS_INO_BASE: u64 = 0x7000;
const DT_REG: u8 = 8;
const DT_DIR: u8 = 4;
const DT_LNK: u8 = 10;
const PROC_FD_PACK_SHIFT: u32 = 16;
const PROC_FD_PACK_MASK: u32 = 0xffff;

struct ProcDirEntry {
    name: String,
    ino: u64,
    dtype: u8,
}

pub(super) fn linux_open_proc_file(abs: &str, file: u16, pid: u32, linux_flags: u64) -> u64 {
    let flags = map_open_flags(linux_flags);
    if (flags & 1) != 0 || (flags & 4) != 0 || (flags & 8) != 0 {
        return linux_err(EACCES);
    }
    let fd =
        match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::LinuxProc {
            file,
            pid,
            position: 0,
        }) {
            Some(fd) => fd,
            None => return linux_err(EBADF),
        };
    if (linux_flags & 0o2000000) != 0 {
        crate::task::scheduler::current_fd_set_cloexec(fd, true);
    }
    crate::serial_verbose_println!("lxe linux open-proc: ok linux='{}' fd={}", abs, fd);
    fd as u64
}

pub(super) fn linux_read_proc_file(
    fd: u32,
    file: u16,
    pid: u32,
    position: u32,
    buf_ptr: u64,
    len: u32,
) -> u64 {
    if linux_proc_is_dir(file) {
        return linux_err(EISDIR);
    }
    let copied = linux_copy_proc_content(file, pid, position, buf_ptr, len);
    if (copied as i64) < 0 {
        return copied;
    }
    let next = position.saturating_add(copied as u32);
    if !crate::task::scheduler::current_fd_set_linux_proc_position(fd, next) {
        return linux_err(EBADF);
    }
    copied
}

pub(super) fn linux_copy_proc_content(
    file: u16,
    pid: u32,
    offset: u32,
    buf_ptr: u64,
    len: u32,
) -> u64 {
    if linux_proc_is_dir(file) {
        return linux_err(EISDIR);
    }
    if len == 0 {
        return 0;
    }
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let content = linux_proc_content(file, pid);
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

pub(super) fn linux_proc_file_id(path: &str) -> Option<u16> {
    linux_proc_node(path).map(|(file, _)| file)
}

pub(super) fn linux_proc_node(path: &str) -> Option<(u16, u32)> {
    let path = trim_proc_path(path);
    if path == "/proc" {
        return Some((LINUX_PROC_ROOT_DIR, 0));
    }
    let rest = path.strip_prefix("/proc/")?;
    match rest {
        "filesystems" => Some((LINUX_PROC_FILESYSTEMS, 0)),
        "mounts" => Some((LINUX_PROC_MOUNTS, 0)),
        "stat" => Some((LINUX_PROC_STAT, 0)),
        "uptime" => Some((LINUX_PROC_UPTIME, 0)),
        "meminfo" => Some((LINUX_PROC_MEMINFO, 0)),
        "cpuinfo" => Some((LINUX_PROC_CPUINFO, 0)),
        "loadavg" => Some((LINUX_PROC_LOADAVG, 0)),
        "version" => Some((LINUX_PROC_VERSION, 0)),
        "sys" => Some((LINUX_PROC_SYS_DIR, 0)),
        "sys/crypto" => Some((LINUX_PROC_SYS_CRYPTO_DIR, 0)),
        "sys/crypto/fips_enabled" => Some((LINUX_PROC_FIPS_ENABLED, 0)),
        "self" => Some((LINUX_PROC_SELF_DIR, crate::task::scheduler::current_tid())),
        _ => {
            if let Some(tail) = rest.strip_prefix("self/") {
                return linux_proc_self_node(tail);
            }
            let (pid, tail) = split_pid(rest)?;
            if !linux_proc_pid_exists(pid) {
                return None;
            }
            linux_proc_pid_node(pid, tail)
        }
    }
}

pub(super) fn linux_proc_at_path(dirfd: i32, raw_path: &str) -> Option<Result<String, i32>> {
    if dirfd == LINUX_AT_FDCWD || raw_path.starts_with('/') {
        return None;
    }
    let entry = crate::task::scheduler::current_fd_get(dirfd as u32)?;
    let crate::fs::fd_table::FdKind::LinuxProc { file, pid, .. } = entry.kind else {
        return None;
    };
    if !linux_proc_is_dir(file) {
        return Some(Err(ENOTDIR));
    }
    let Some(base) = linux_proc_fd_path(file, pid) else {
        return Some(Err(EBADF));
    };
    Some(Ok(crate::fs::path::normalize(&format!(
        "{}/{}",
        base, raw_path
    ))))
}

pub(super) fn linux_proc_fd_path(file: u16, pid: u32) -> Option<String> {
    match file {
        LINUX_PROC_ROOT_DIR => Some(String::from("/proc")),
        LINUX_PROC_SELF_DIR => Some(String::from("/proc/self")),
        LINUX_PROC_PID_DIR => Some(format!("/proc/{}", pid)),
        LINUX_PROC_PID_FD_DIR => {
            if pid == crate::task::scheduler::current_tid() {
                Some(String::from("/proc/self/fd"))
            } else {
                Some(format!("/proc/{}/fd", pid))
            }
        }
        LINUX_PROC_SYS_DIR => Some(String::from("/proc/sys")),
        LINUX_PROC_SYS_CRYPTO_DIR => Some(String::from("/proc/sys/crypto")),
        LINUX_PROC_PID_FD_ENTRY => {
            let (target_pid, fd) = unpack_proc_fd(pid);
            if target_pid == crate::task::scheduler::current_tid() {
                Some(format!("/proc/self/fd/{}", fd))
            } else {
                Some(format!("/proc/{}/fd/{}", target_pid, fd))
            }
        }
        _ => None,
    }
}

pub(super) fn linux_proc_is_dir(file: u16) -> bool {
    matches!(
        file,
        LINUX_PROC_ROOT_DIR
            | LINUX_PROC_SELF_DIR
            | LINUX_PROC_PID_DIR
            | LINUX_PROC_PID_FD_DIR
            | LINUX_PROC_SYS_DIR
            | LINUX_PROC_SYS_CRYPTO_DIR
    )
}

pub(super) fn linux_proc_content_len(file: u16, pid: u32) -> u64 {
    if file == LINUX_PROC_PID_FD_ENTRY {
        return linux_proc_fd_link_target(pid)
            .map(|target| target.len() as u64)
            .unwrap_or(0);
    }
    if linux_proc_is_dir(file) {
        0
    } else {
        linux_proc_content(file, pid).len() as u64
    }
}

pub(super) fn linux_proc_readlink(path: &str) -> Option<Result<String, i32>> {
    let (file, pid) = linux_proc_node(path)?;
    if file == LINUX_PROC_PID_FD_ENTRY {
        return Some(linux_proc_fd_link_target(pid).ok_or(ENOENT));
    }
    None
}

pub(super) fn linux_proc_content(file: u16, pid: u32) -> Vec<u8> {
    match file {
        LINUX_PROC_FILESYSTEMS => {
            b"nodev\tsysfs\nnodev\trootfs\nnodev\tproc\nnodev\tdevtmpfs\nnodev\tdevpts\nnodev\ttmpfs\nexfat\n"
                .to_vec()
        }
        LINUX_PROC_MOUNTS => {
            b"rootfs / rootfs rw 0 0\nproc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n"
                .to_vec()
        }
        LINUX_PROC_MOUNTINFO => {
            b"1 0 0:1 / / rw,relatime - rootfs rootfs rw\n2 1 0:2 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n"
                .to_vec()
        }
        LINUX_PROC_LOGINUID => b"4294967295\n".to_vec(),
        LINUX_PROC_PID_STAT => linux_proc_pid_stat(pid).into_bytes(),
        LINUX_PROC_PID_STATUS | LINUX_PROC_STATUS => linux_proc_pid_status(pid).into_bytes(),
        LINUX_PROC_PID_CMDLINE => linux_proc_pid_cmdline(pid),
        LINUX_PROC_PID_COMM => linux_proc_pid_comm(pid).into_bytes(),
        LINUX_PROC_STAT => linux_proc_system_stat().into_bytes(),
        LINUX_PROC_UPTIME => b"0.00 0.00\n".to_vec(),
        LINUX_PROC_MEMINFO => b"MemTotal:        4174848 kB\nMemFree:         2097152 kB\nMemAvailable:   2097152 kB\n"
            .to_vec(),
        LINUX_PROC_CPUINFO => b"processor\t: 0\nvendor_id\t: anyOS\nmodel name\t: anyOS virtual CPU\n"
            .to_vec(),
        LINUX_PROC_LOADAVG => linux_proc_loadavg().into_bytes(),
        LINUX_PROC_VERSION => b"Linux version 6.1.0-anyos (lxe)\n".to_vec(),
        LINUX_PROC_PID_STATM => linux_proc_pid_statm(pid).into_bytes(),
        LINUX_PROC_FIPS_ENABLED => b"0\n".to_vec(),
        _ => Vec::new(),
    }
}

pub(super) fn linux_write_proc_stat(stat_ptr: u64, file: u16, pid: u32) -> u64 {
    let is_dir = linux_proc_is_dir(file);
    let is_link = file == LINUX_PROC_PID_FD_ENTRY;
    write_linux_stat(
        stat_ptr,
        PROCFS_DEV,
        proc_inode(file, pid),
        if is_dir {
            1
        } else if is_link {
            3
        } else {
            0
        },
        linux_proc_content_len(file, pid),
        handlers::sys_getuid(),
        handlers::sys_getgid(),
        if is_dir {
            0o555
        } else if is_link {
            0o777
        } else {
            0o444
        },
        0,
    );
    0
}

pub(super) fn linux_getdents64_proc(
    fd: u32,
    file: u16,
    pid: u32,
    position: u32,
    dirent_ptr: u64,
    count: u64,
) -> u64 {
    if !linux_proc_is_dir(file) {
        return linux_err(ENOTDIR);
    }
    let entries = linux_proc_dir_entries(file, pid);
    let mut written = 0usize;
    let mut next_index = position as usize;
    for (idx, entry) in entries.iter().enumerate().skip(position as usize) {
        let name = entry.name.as_bytes();
        let reclen = align_up(19 + name.len() + 1, 8);
        if written + reclen > count as usize {
            break;
        }
        let base = dirent_ptr + written as u64;
        unsafe {
            write_u64(base, 0, entry.ino);
            write_u64(base, 8, (idx + 1) as u64);
            write_u16(base, 16, reclen as u16);
            *((base + 18) as *mut u8) = entry.dtype;
            core::ptr::copy_nonoverlapping(name.as_ptr(), (base + 19) as *mut u8, name.len());
            *((base + 19 + name.len() as u64) as *mut u8) = 0;
            if reclen > 20 + name.len() {
                core::ptr::write_bytes(
                    (base + 20 + name.len() as u64) as *mut u8,
                    0,
                    reclen - 20 - name.len(),
                );
            }
        }
        written += reclen;
        next_index = idx + 1;
    }
    let _ = crate::task::scheduler::current_fd_set_linux_proc_position(fd, next_index as u32);
    written as u64
}

pub(super) fn linux_getdents_proc(
    fd: u32,
    file: u16,
    pid: u32,
    position: u32,
    dirent_ptr: u64,
    count: u64,
) -> u64 {
    if !linux_proc_is_dir(file) {
        return linux_err(ENOTDIR);
    }
    let entries = linux_proc_dir_entries(file, pid);
    let mut written = 0usize;
    let mut next_index = position as usize;
    for (idx, entry) in entries.iter().enumerate().skip(position as usize) {
        let name = entry.name.as_bytes();
        let reclen = align_up(18 + name.len() + 2, 8);
        if written + reclen > count as usize {
            break;
        }
        let base = dirent_ptr + written as u64;
        unsafe {
            write_u64(base, 0, entry.ino);
            write_u64(base, 8, (idx + 1) as u64);
            write_u16(base, 16, reclen as u16);
            core::ptr::copy_nonoverlapping(name.as_ptr(), (base + 18) as *mut u8, name.len());
            *((base + 18 + name.len() as u64) as *mut u8) = 0;
            *((base + reclen as u64 - 1) as *mut u8) = entry.dtype;
            if reclen > 19 + name.len() {
                core::ptr::write_bytes(
                    (base + 19 + name.len() as u64) as *mut u8,
                    0,
                    reclen - 20 - name.len(),
                );
            }
        }
        written += reclen;
        next_index = idx + 1;
    }
    let _ = crate::task::scheduler::current_fd_set_linux_proc_position(fd, next_index as u32);
    written as u64
}

pub(super) fn linux_write_proc_statfs(buf_ptr: u64) -> u64 {
    if !super::handlers::helpers::is_user_range_accessible(buf_ptr, 120) {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 120);
        write_u64(buf_ptr, 0, 0x9fa0);
        write_u64(buf_ptr, 8, 4096);
        write_u64(buf_ptr, 16, 0);
        write_u64(buf_ptr, 24, 0);
        write_u64(buf_ptr, 32, 0);
        write_u64(buf_ptr, 40, 1024 * 1024);
        write_u64(buf_ptr, 48, 1024 * 1024);
        write_u32(buf_ptr, 72, 255);
        write_u64(buf_ptr, 80, 4096);
    }
    0
}

fn trim_proc_path(path: &str) -> &str {
    if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    }
}

fn linux_proc_self_node(tail: &str) -> Option<(u16, u32)> {
    let pid = crate::task::scheduler::current_tid();
    match tail {
        "mounts" => Some((LINUX_PROC_MOUNTS, 0)),
        "mountinfo" => Some((LINUX_PROC_MOUNTINFO, 0)),
        "loginuid" => Some((LINUX_PROC_LOGINUID, 0)),
        _ => linux_proc_pid_node(pid, tail),
    }
}

fn split_pid(path: &str) -> Option<(u32, &str)> {
    let (pid_str, tail) = match path.split_once('/') {
        Some((pid, tail)) => (pid, tail),
        None => (path, ""),
    };
    if pid_str.is_empty() || !pid_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((pid_str.parse().ok()?, tail))
}

fn linux_proc_pid_node(pid: u32, tail: &str) -> Option<(u16, u32)> {
    match tail {
        "" => Some((LINUX_PROC_PID_DIR, pid)),
        "stat" => Some((LINUX_PROC_PID_STAT, pid)),
        "status" => Some((LINUX_PROC_PID_STATUS, pid)),
        "cmdline" => Some((LINUX_PROC_PID_CMDLINE, pid)),
        "comm" => Some((LINUX_PROC_PID_COMM, pid)),
        "statm" => Some((LINUX_PROC_PID_STATM, pid)),
        "fd" => Some((LINUX_PROC_PID_FD_DIR, pid)),
        _ => {
            let fd_tail = tail.strip_prefix("fd/")?;
            if fd_tail.is_empty()
                || fd_tail.contains('/')
                || !fd_tail.bytes().all(|b| b.is_ascii_digit())
            {
                return None;
            }
            let fd = fd_tail.parse::<u32>().ok()?;
            if crate::task::scheduler::thread_fd_get(pid, fd).is_some() {
                pack_proc_fd(pid, fd).map(|packed| (LINUX_PROC_PID_FD_ENTRY, packed))
            } else {
                None
            }
        }
    }
}

fn linux_proc_pid_exists(pid: u32) -> bool {
    crate::task::scheduler::list_threads()
        .iter()
        .any(|thread| thread.tid == pid)
}

fn linux_proc_dir_entries(file: u16, pid: u32) -> Vec<ProcDirEntry> {
    let mut entries = Vec::new();
    push_dirent(&mut entries, ".", proc_inode(file, pid), DT_DIR);
    push_dirent(
        &mut entries,
        "..",
        proc_inode(LINUX_PROC_ROOT_DIR, 0),
        DT_DIR,
    );
    match file {
        LINUX_PROC_ROOT_DIR => {
            push_dirent(
                &mut entries,
                "self",
                proc_inode(LINUX_PROC_SELF_DIR, 0),
                DT_DIR,
            );
            push_dirent(
                &mut entries,
                "filesystems",
                proc_inode(LINUX_PROC_FILESYSTEMS, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "mounts",
                proc_inode(LINUX_PROC_MOUNTS, 0),
                DT_REG,
            );
            push_dirent(&mut entries, "stat", proc_inode(LINUX_PROC_STAT, 0), DT_REG);
            push_dirent(
                &mut entries,
                "uptime",
                proc_inode(LINUX_PROC_UPTIME, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "meminfo",
                proc_inode(LINUX_PROC_MEMINFO, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "cpuinfo",
                proc_inode(LINUX_PROC_CPUINFO, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "loadavg",
                proc_inode(LINUX_PROC_LOADAVG, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "version",
                proc_inode(LINUX_PROC_VERSION, 0),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "sys",
                proc_inode(LINUX_PROC_SYS_DIR, 0),
                DT_DIR,
            );
            for thread in crate::task::scheduler::list_threads() {
                push_dirent(
                    &mut entries,
                    &format!("{}", thread.tid),
                    proc_inode(LINUX_PROC_PID_DIR, thread.tid),
                    DT_DIR,
                );
            }
        }
        LINUX_PROC_SYS_DIR => {
            push_dirent(
                &mut entries,
                "crypto",
                proc_inode(LINUX_PROC_SYS_CRYPTO_DIR, 0),
                DT_DIR,
            );
        }
        LINUX_PROC_SYS_CRYPTO_DIR => {
            push_dirent(
                &mut entries,
                "fips_enabled",
                proc_inode(LINUX_PROC_FIPS_ENABLED, 0),
                DT_REG,
            );
        }
        LINUX_PROC_SELF_DIR | LINUX_PROC_PID_DIR => {
            let target_pid = if file == LINUX_PROC_SELF_DIR {
                crate::task::scheduler::current_tid()
            } else {
                pid
            };
            push_dirent(
                &mut entries,
                "stat",
                proc_inode(LINUX_PROC_PID_STAT, target_pid),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "status",
                proc_inode(LINUX_PROC_PID_STATUS, target_pid),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "cmdline",
                proc_inode(LINUX_PROC_PID_CMDLINE, target_pid),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "comm",
                proc_inode(LINUX_PROC_PID_COMM, target_pid),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "statm",
                proc_inode(LINUX_PROC_PID_STATM, target_pid),
                DT_REG,
            );
            push_dirent(
                &mut entries,
                "fd",
                proc_inode(LINUX_PROC_PID_FD_DIR, target_pid),
                DT_DIR,
            );
            if file == LINUX_PROC_SELF_DIR {
                push_dirent(
                    &mut entries,
                    "mounts",
                    proc_inode(LINUX_PROC_MOUNTS, 0),
                    DT_REG,
                );
                push_dirent(
                    &mut entries,
                    "mountinfo",
                    proc_inode(LINUX_PROC_MOUNTINFO, 0),
                    DT_REG,
                );
                push_dirent(
                    &mut entries,
                    "loginuid",
                    proc_inode(LINUX_PROC_LOGINUID, 0),
                    DT_REG,
                );
            }
        }
        LINUX_PROC_PID_FD_DIR => {
            for fd in crate::task::scheduler::thread_fd_numbers(pid) {
                push_dirent(
                    &mut entries,
                    &format!("{}", fd),
                    proc_inode(
                        LINUX_PROC_PID_FD_ENTRY,
                        pack_proc_fd(pid, fd).unwrap_or(pid),
                    ),
                    DT_LNK,
                );
            }
        }
        _ => {}
    }
    entries
}

fn push_dirent(entries: &mut Vec<ProcDirEntry>, name: &str, ino: u64, dtype: u8) {
    entries.push(ProcDirEntry {
        name: String::from(name),
        ino,
        dtype,
    });
}

fn linux_proc_pid_stat(pid: u32) -> String {
    let Some(thread) = proc_thread(pid) else {
        return String::new();
    };
    let name = sanitize_comm(&thread.name);
    let state = proc_state_char(thread.state);
    let utime = thread.cpu_ticks as u64;
    let rss = thread.user_pages as u64;
    let vsize = rss.saturating_mul(4096);
    format!(
        "{} ({}) {} {} {} {} 0 -1 0 0 0 0 0 {} 0 0 20 0 1 0 0 {} {} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        thread.tid,
        name,
        state,
        thread.parent_tid,
        thread.tid,
        thread.tid,
        utime,
        vsize,
        rss
    )
}

fn linux_proc_pid_status(pid: u32) -> String {
    let Some(thread) = proc_thread(pid) else {
        return String::new();
    };
    let state = proc_state_name(thread.state);
    format!(
        "Name:\t{}\nState:\t{}\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nUid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nThreads:\t1\nVmSize:\t{} kB\nVmRSS:\t{} kB\n",
        sanitize_comm(&thread.name),
        state,
        thread.tid,
        thread.tid,
        thread.parent_tid,
        thread.uid,
        thread.uid,
        thread.uid,
        thread.uid,
        thread.gid,
        thread.gid,
        thread.gid,
        thread.gid,
        thread.user_pages.saturating_mul(4),
        thread.user_pages.saturating_mul(4)
    )
}

fn linux_proc_pid_cmdline(pid: u32) -> Vec<u8> {
    let Some(thread) = proc_thread(pid) else {
        return Vec::new();
    };
    let source = if thread.args.is_empty() {
        thread.name
    } else {
        thread.args
    };
    let mut out = Vec::new();
    for part in source.split_whitespace() {
        if !out.is_empty() {
            out.push(0);
        }
        out.extend_from_slice(part.as_bytes());
    }
    if !out.is_empty() {
        out.push(0);
    }
    out
}

fn linux_proc_pid_comm(pid: u32) -> String {
    let Some(thread) = proc_thread(pid) else {
        return String::new();
    };
    format!("{}\n", sanitize_comm(&thread.name))
}

fn linux_proc_pid_statm(pid: u32) -> String {
    let Some(thread) = proc_thread(pid) else {
        return String::new();
    };
    let pages = thread.user_pages.max(1);
    format!("{} {} 0 0 0 0 0\n", pages, pages)
}

fn linux_proc_system_stat() -> String {
    let threads = crate::task::scheduler::list_threads();
    let ticks: u64 = threads.iter().map(|thread| thread.cpu_ticks as u64).sum();
    let running = threads
        .iter()
        .filter(|thread| thread.state == "running" || thread.state == "ready")
        .count();
    format!(
        "cpu  {} 0 0 0 0 0 0 0 0 0\nbtime 0\nprocesses {}\nprocs_running {}\nprocs_blocked 0\n",
        ticks,
        threads.len(),
        running
    )
}

fn linux_proc_loadavg() -> String {
    let threads = crate::task::scheduler::list_threads();
    let running = threads
        .iter()
        .filter(|thread| thread.state == "running" || thread.state == "ready")
        .count();
    let last_pid = threads.iter().map(|thread| thread.tid).max().unwrap_or(1);
    format!(
        "0.00 0.00 0.00 {}/{} {}\n",
        running.max(1),
        threads.len().max(1),
        last_pid
    )
}

fn proc_thread(pid: u32) -> Option<crate::task::scheduler::ThreadInfo> {
    crate::task::scheduler::list_threads()
        .into_iter()
        .find(|thread| thread.tid == pid)
}

fn proc_state_char(state: &str) -> &'static str {
    match state {
        "running" | "ready" => "R",
        "stopped" => "T",
        _ => "S",
    }
}

fn proc_state_name(state: &str) -> &'static str {
    match state {
        "running" | "ready" => "R (running)",
        "stopped" => "T (stopped)",
        _ => "S (sleeping)",
    }
}

fn sanitize_comm(name: &str) -> String {
    name.chars()
        .filter(|ch| *ch != '(' && *ch != ')' && *ch != '\n' && *ch != '\0')
        .collect()
}

fn proc_inode(file: u16, pid: u32) -> u64 {
    PROCFS_INO_BASE + ((pid as u64) << 8) + file as u64
}

fn pack_proc_fd(pid: u32, fd: u32) -> Option<u32> {
    if pid > PROC_FD_PACK_MASK || fd > PROC_FD_PACK_MASK {
        return None;
    }
    Some((pid << PROC_FD_PACK_SHIFT) | fd)
}

fn unpack_proc_fd(packed: u32) -> (u32, u32) {
    (packed >> PROC_FD_PACK_SHIFT, packed & PROC_FD_PACK_MASK)
}

fn linux_proc_fd_link_target(packed: u32) -> Option<String> {
    let (pid, fd) = unpack_proc_fd(packed);
    let entry = crate::task::scheduler::thread_fd_get(pid, fd)?;
    match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => crate::fs::vfs::get_fd_path(global_id)
            .ok()
            .map(|path| linux_strip_rootfs(&path))
            .or_else(|| Some(format!("anon_inode:[file{}]", global_id))),
        crate::fs::fd_table::FdKind::PipeRead { pipe_id }
        | crate::fs::fd_table::FdKind::PipeWrite { pipe_id } => Some(format!("pipe:[{}]", pipe_id)),
        crate::fs::fd_table::FdKind::Tty | crate::fs::fd_table::FdKind::PtySlave { .. } => {
            Some(String::from("/dev/tty"))
        }
        crate::fs::fd_table::FdKind::LinuxProc { file, pid, .. } => {
            linux_proc_fd_path(file, pid).or_else(|| Some(String::from("/proc")))
        }
        crate::fs::fd_table::FdKind::LinuxSocket { socket_id } => {
            Some(format!("socket:[{}]", socket_id))
        }
        crate::fs::fd_table::FdKind::None => None,
    }
}
