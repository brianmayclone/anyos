//! licof Linux x86_64 syscall dispatch.
//!
//! This is intentionally a narrow Tier-0 bridge. Unsupported syscalls return
//! `-ENOSYS` using Linux's negative-errno convention.

use super::{handlers, SyscallRegs};
use alloc::string::String;
use alloc::vec::Vec;

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOENT: i32 = 2;
const ENOSYS: i32 = 38;
const EACCES: i32 = 13;
const EAGAIN: i32 = 11;
const ENOTTY: i32 = 25;
const EPERM: i32 = 1;
const ELOOP: i32 = 40;

const LINUX_SYS_READ: u64 = 0;
const LINUX_SYS_WRITE: u64 = 1;
const LINUX_SYS_OPEN: u64 = 2;
const LINUX_SYS_CLOSE: u64 = 3;
const LINUX_SYS_STAT: u64 = 4;
const LINUX_SYS_FSTAT: u64 = 5;
const LINUX_SYS_LSTAT: u64 = 6;
const LINUX_SYS_POLL: u64 = 7;
const LINUX_SYS_LSEEK: u64 = 8;
const LINUX_SYS_MMAP: u64 = 9;
const LINUX_SYS_MPROTECT: u64 = 10;
const LINUX_SYS_MUNMAP: u64 = 11;
const LINUX_SYS_BRK: u64 = 12;
const LINUX_SYS_RT_SIGACTION: u64 = 13;
const LINUX_SYS_RT_SIGPROCMASK: u64 = 14;
const LINUX_SYS_IOCTL: u64 = 16;
const LINUX_SYS_PREAD64: u64 = 17;
const LINUX_SYS_READV: u64 = 19;
const LINUX_SYS_WRITEV: u64 = 20;
const LINUX_SYS_ACCESS: u64 = 21;
const LINUX_SYS_PIPE: u64 = 22;
const LINUX_SYS_MADVISE: u64 = 28;
const LINUX_SYS_DUP: u64 = 32;
const LINUX_SYS_DUP2: u64 = 33;
const LINUX_SYS_NANOSLEEP: u64 = 35;
const LINUX_SYS_GETPID: u64 = 39;
const LINUX_SYS_UNAME: u64 = 63;
const LINUX_SYS_EXIT: u64 = 60;
const LINUX_SYS_FSYNC: u64 = 74;
const LINUX_SYS_FDATASYNC: u64 = 75;
const LINUX_SYS_TRUNCATE: u64 = 76;
const LINUX_SYS_FTRUNCATE: u64 = 77;
const LINUX_SYS_FCNTL: u64 = 72;
const LINUX_SYS_GETCWD: u64 = 79;
const LINUX_SYS_CHDIR: u64 = 80;
const LINUX_SYS_RENAME: u64 = 82;
const LINUX_SYS_MKDIR: u64 = 83;
const LINUX_SYS_RMDIR: u64 = 84;
const LINUX_SYS_CREAT: u64 = 85;
const LINUX_SYS_UNLINK: u64 = 87;
const LINUX_SYS_READLINK: u64 = 89;
const LINUX_SYS_CHMOD: u64 = 90;
const LINUX_SYS_FCHMOD: u64 = 91;
const LINUX_SYS_CHOWN: u64 = 92;
const LINUX_SYS_FCHOWN: u64 = 93;
const LINUX_SYS_LCHOWN: u64 = 94;
const LINUX_SYS_UMASK: u64 = 95;
const LINUX_SYS_GETTIMEOFDAY: u64 = 96;
const LINUX_SYS_GETRLIMIT: u64 = 97;
const LINUX_SYS_SYSINFO: u64 = 99;
const LINUX_SYS_GETUID: u64 = 102;
const LINUX_SYS_GETGID: u64 = 104;
const LINUX_SYS_SETUID: u64 = 105;
const LINUX_SYS_SETGID: u64 = 106;
const LINUX_SYS_GETEUID: u64 = 107;
const LINUX_SYS_GETEGID: u64 = 108;
const LINUX_SYS_GETPPID: u64 = 110;
const LINUX_SYS_GETGROUPS: u64 = 115;
const LINUX_SYS_SETGROUPS: u64 = 116;
const LINUX_SYS_SETRESUID: u64 = 117;
const LINUX_SYS_GETRESUID: u64 = 118;
const LINUX_SYS_SETRESGID: u64 = 119;
const LINUX_SYS_GETRESGID: u64 = 120;
const LINUX_SYS_SETFSUID: u64 = 122;
const LINUX_SYS_SETFSGID: u64 = 123;
const LINUX_SYS_CAPGET: u64 = 125;
const LINUX_SYS_CAPSET: u64 = 126;
const LINUX_SYS_STATFS: u64 = 137;
const LINUX_SYS_FSTATFS: u64 = 138;
const LINUX_SYS_PRCTL: u64 = 157;
const LINUX_SYS_ARCH_PRCTL: u64 = 158;
const LINUX_SYS_GETTID: u64 = 186;
const LINUX_SYS_TIME: u64 = 201;
const LINUX_SYS_FUTEX: u64 = 202;
const LINUX_SYS_SET_TID_ADDRESS: u64 = 218;
const LINUX_SYS_GETDENTS64: u64 = 217;
const LINUX_SYS_CLOCK_GETTIME: u64 = 228;
const LINUX_SYS_EXIT_GROUP: u64 = 231;
const LINUX_SYS_OPENAT: u64 = 257;
const LINUX_SYS_MKDIRAT: u64 = 258;
const LINUX_SYS_FCHOWNAT: u64 = 260;
const LINUX_SYS_NEWFSTATAT: u64 = 262;
const LINUX_SYS_UNLINKAT: u64 = 263;
const LINUX_SYS_RENAMEAT: u64 = 264;
const LINUX_SYS_READLINKAT: u64 = 267;
const LINUX_SYS_FCHMODAT: u64 = 268;
const LINUX_SYS_FACCESSAT: u64 = 269;
const LINUX_SYS_SET_ROBUST_LIST: u64 = 273;
const LINUX_SYS_UTIMENSAT: u64 = 280;
const LINUX_SYS_DUP3: u64 = 292;
const LINUX_SYS_PIPE2: u64 = 293;
const LINUX_SYS_PRLIMIT64: u64 = 302;
const LINUX_SYS_GETRANDOM: u64 = 318;
const LINUX_SYS_RSEQ: u64 = 334;

const LINUX_AT_FDCWD: i32 = -100;
const LINUX_AT_SYMLINK_NOFOLLOW: u64 = 0x100;
const LINUX_AT_REMOVEDIR: u64 = 0x200;
const LINUX_AT_EMPTY_PATH: u64 = 0x1000;
const LINUX_MAP_ANONYMOUS: u64 = 0x20;
const LINUX_MAP_PRIVATE: u64 = 0x02;
const LINUX_MAP_FIXED: u64 = 0x10;
const LINUX_ARCH_SET_FS: u64 = 0x1002;
const LINUX_ARCH_GET_FS: u64 = 0x1003;
const LICOF_ROOTFS: &str = "/System/var/licof/rootfs";

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax;
    let a1 = regs.rdi;
    let a2 = regs.rsi;
    let a3 = regs.rdx;
    let a4 = regs.r10;
    let a5 = regs.r8;
    let a6 = regs.r9;

    match nr {
        LINUX_SYS_READ => anyos_u32_ret(handlers::sys_read(a1 as u32, a2, a3 as u32)),
        LINUX_SYS_WRITE => anyos_u32_ret(handlers::sys_write(a1 as u32, a2, a3 as u32)),
        LINUX_SYS_OPEN => linux_open(a1, a2),
        LINUX_SYS_OPENAT => linux_openat(a1, a2, a3),
        LINUX_SYS_CLOSE => anyos_u32_ret(handlers::sys_close(a1 as u32)),
        LINUX_SYS_STAT => linux_stat(a1, a2, false),
        LINUX_SYS_LSTAT => linux_stat(a1, a2, true),
        LINUX_SYS_FSTAT => linux_fstat(a1 as u32, a2),
        LINUX_SYS_POLL => linux_poll(a1, a2, a3),
        LINUX_SYS_LSEEK => anyos_u32_ret(handlers::sys_lseek(a1 as u32, a2 as u32, a3 as u32)),
        LINUX_SYS_BRK => linux_brk(a1),
        LINUX_SYS_MMAP => linux_mmap(a1, a2, a3, a4, a5, a6),
        LINUX_SYS_MPROTECT => linux_mprotect(a1, a2, a3),
        LINUX_SYS_MUNMAP => linux_munmap(a1, a2),
        LINUX_SYS_RT_SIGACTION => linux_rt_sigaction(a1, a2, a3, a4),
        LINUX_SYS_RT_SIGPROCMASK => linux_rt_sigprocmask(a1, a2, a3, a4),
        LINUX_SYS_IOCTL => linux_ioctl(a1 as u32, a2, a3),
        LINUX_SYS_PREAD64 => linux_pread64(a1 as u32, a2, a3, a4),
        LINUX_SYS_READV => linux_readv(a1 as u32, a2, a3),
        LINUX_SYS_WRITEV => linux_writev(a1 as u32, a2, a3),
        LINUX_SYS_ACCESS => linux_access(a1, a2),
        LINUX_SYS_PIPE => anyos_u32_ret(handlers::sys_pipe2(a1, 0)),
        LINUX_SYS_PIPE2 => anyos_u32_ret(handlers::sys_pipe2(a1, a2 as u32)),
        LINUX_SYS_DUP => anyos_u32_ret(handlers::sys_dup(a1 as u32)),
        LINUX_SYS_DUP2 => anyos_u32_ret(handlers::sys_dup2(a1 as u32, a2 as u32)),
        LINUX_SYS_DUP3 => {
            let ret = handlers::sys_dup2(a1 as u32, a2 as u32);
            if ret != u32::MAX && (a3 & 0o2000000) != 0 {
                crate::task::scheduler::current_fd_set_cloexec(ret, true);
            }
            anyos_u32_ret(ret)
        }
        LINUX_SYS_MADVISE => 0,
        LINUX_SYS_NANOSLEEP => 0,
        LINUX_SYS_ARCH_PRCTL => linux_arch_prctl(a1, a2),
        LINUX_SYS_GETPID => handlers::sys_getpid() as u64,
        LINUX_SYS_UNAME => linux_uname(a1),
        LINUX_SYS_FSYNC | LINUX_SYS_FDATASYNC => anyos_u32_ret(handlers::sys_fsync(a1 as u32)),
        LINUX_SYS_TRUNCATE => linux_truncate(a1, a2),
        LINUX_SYS_FTRUNCATE => linux_ftruncate(a1 as u32, a2),
        LINUX_SYS_FCNTL => linux_fcntl(a1 as u32, a2 as u32, a3),
        LINUX_SYS_GETCWD => linux_getcwd(a1, a2),
        LINUX_SYS_CHDIR => linux_chdir(a1),
        LINUX_SYS_RENAME => linux_rename(a1, a2),
        LINUX_SYS_MKDIR => linux_mkdir(a1, a2),
        LINUX_SYS_RMDIR => linux_unlink_path(a1),
        LINUX_SYS_CREAT => linux_creat(a1),
        LINUX_SYS_UNLINK => linux_unlink_path(a1),
        LINUX_SYS_READLINK => linux_readlink(a1, a2, a3),
        LINUX_SYS_CHMOD => linux_chmod(a1, a2),
        LINUX_SYS_FCHMOD => linux_fchmod(a1 as u32, a2),
        LINUX_SYS_CHOWN | LINUX_SYS_LCHOWN => linux_chown(a1, a2, a3),
        LINUX_SYS_FCHOWN => linux_fchown(a1 as u32, a2, a3),
        LINUX_SYS_UMASK => 0,
        LINUX_SYS_GETTIMEOFDAY => linux_gettimeofday(a1),
        LINUX_SYS_GETRLIMIT => linux_prlimit64(0, a1, 0, a2),
        LINUX_SYS_SYSINFO => linux_sysinfo(a1),
        LINUX_SYS_GETUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETGID => handlers::sys_getgid() as u64,
        LINUX_SYS_SETUID => linux_set_root_or_current(a1, true),
        LINUX_SYS_SETGID => linux_set_root_or_current(a1, false),
        LINUX_SYS_GETEUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETEGID => handlers::sys_getgid() as u64,
        LINUX_SYS_GETPPID => handlers::sys_getppid() as u64,
        LINUX_SYS_GETGROUPS => linux_getgroups(a1, a2),
        LINUX_SYS_SETGROUPS => linux_setgroups(a1, a2),
        LINUX_SYS_SETRESUID => linux_setres_id(a1, a2, a3, true),
        LINUX_SYS_GETRESUID => linux_getres_id(a1, a2, a3, true),
        LINUX_SYS_SETRESGID => linux_setres_id(a1, a2, a3, false),
        LINUX_SYS_GETRESGID => linux_getres_id(a1, a2, a3, false),
        LINUX_SYS_SETFSUID => linux_setfs_id(a1, true),
        LINUX_SYS_SETFSGID => linux_setfs_id(a1, false),
        LINUX_SYS_CAPGET => linux_capget(a1, a2),
        LINUX_SYS_CAPSET => linux_capset(a1, a2),
        LINUX_SYS_STATFS => linux_statfs(a1, a2),
        LINUX_SYS_FSTATFS => linux_fstatfs(a1 as u32, a2),
        LINUX_SYS_PRCTL => linux_prctl(a1, a2),
        LINUX_SYS_GETTID => crate::task::scheduler::current_tid() as u64,
        LINUX_SYS_TIME => linux_time(a1),
        LINUX_SYS_FUTEX => linux_futex(a1, a2, a3),
        LINUX_SYS_GETDENTS64 => linux_getdents64(a1 as u32, a2, a3),
        LINUX_SYS_SET_TID_ADDRESS => handlers::sys_getpid() as u64,
        LINUX_SYS_CLOCK_GETTIME => linux_clock_gettime(a1, a2),
        LINUX_SYS_EXIT | LINUX_SYS_EXIT_GROUP => handlers::sys_exit(a1 as u32) as u64,
        LINUX_SYS_MKDIRAT => linux_mkdirat(a1, a2, a3),
        LINUX_SYS_FCHOWNAT => linux_fchownat(a1, a2, a3, a4),
        LINUX_SYS_NEWFSTATAT => linux_newfstatat(a1, a2, a3, a4),
        LINUX_SYS_UNLINKAT => linux_unlinkat(a1, a2, a3),
        LINUX_SYS_RENAMEAT => linux_renameat(a1, a2, a3, a4),
        LINUX_SYS_READLINKAT => linux_readlinkat(a1, a2, a3, a4),
        LINUX_SYS_FCHMODAT => linux_fchmodat(a1, a2, a3, a4),
        LINUX_SYS_FACCESSAT => linux_faccessat(a1, a2, a3, a4),
        LINUX_SYS_SET_ROBUST_LIST => 0,
        LINUX_SYS_UTIMENSAT => 0,
        LINUX_SYS_PRLIMIT64 => linux_prlimit64(a1, a2, a3, a4),
        LINUX_SYS_GETRANDOM => linux_getrandom(a1, a2),
        LINUX_SYS_RSEQ => linux_err(ENOSYS),
        _ => linux_unsupported_syscall(regs, a1, a2, a3, a4, a5, a6),
    }
}

fn linux_unsupported_syscall(
    regs: &SyscallRegs,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> u64 {
    crate::serial_println!(
        "licof linux: unsupported syscall nr={} rip={:#x} args={:#x},{:#x},{:#x},{:#x},{:#x},{:#x}",
        regs.rax,
        regs.rip,
        a1,
        a2,
        a3,
        a4,
        a5,
        a6
    );
    linux_err(ENOSYS)
}

fn linux_open(path_ptr: u64, linux_flags: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    linux_open_linux_path(raw_path, linux_flags)
}

fn linux_openat(dirfd: u64, path_ptr: u64, linux_flags: u64) -> u64 {
    let dirfd = dirfd as i32;
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if dirfd == LINUX_AT_FDCWD || raw_path.starts_with('/') {
        return linux_open_linux_path(raw_path, linux_flags);
    }
    let path = match linux_translate_at_path(dirfd as u64, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_open_translated(&path, linux_flags, raw_path)
}

fn linux_stat(path_ptr: u64, stat_ptr: u64, nofollow: bool) -> u64 {
    if path_ptr == 0 || stat_ptr == 0 {
        return linux_err(EFAULT);
    }
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    linux_stat_translated(&path, stat_ptr, nofollow)
}

fn linux_newfstatat(dirfd: u64, path_ptr: u64, stat_ptr: u64, flags: u64) -> u64 {
    if stat_ptr == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = if path_ptr == 0 {
        ""
    } else {
        match super::handlers::helpers::read_user_str_safe(path_ptr) {
            Some(path) => path,
            None => return linux_err(EFAULT),
        }
    };
    if raw_path.is_empty() && (flags & LINUX_AT_EMPTY_PATH) != 0 {
        return linux_fstat(dirfd as u32, stat_ptr);
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_stat_translated(&path, stat_ptr, (flags & LINUX_AT_SYMLINK_NOFOLLOW) != 0)
}

fn linux_access(path_ptr: u64, _mode: u64) -> u64 {
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    let resolved = match linux_resolve_translated_path(&path, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::stat(&resolved) {
        Ok(_) => 0,
        Err(crate::fs::vfs::FsError::NotFound) => linux_err(ENOENT),
        Err(crate::fs::vfs::FsError::PermissionDenied) => linux_err(EACCES),
        Err(e) => linux_fs_err(e),
    }
}

fn linux_open_linux_path(path: &str, linux_flags: u64) -> u64 {
    let abs = linux_absolute_path(path);
    if abs == "/dev/tty" {
        let fd = match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::Tty) {
            Some(fd) => fd,
            None => return linux_err(EBADF),
        };
        if (linux_flags & 0o2000000) != 0 {
            crate::task::scheduler::current_fd_set_cloexec(fd, true);
        }
        return fd as u64;
    }
    let translated = linux_translate_absolute_path(&abs);
    linux_open_translated(&translated, linux_flags, &abs)
}

fn linux_open_translated(path: &str, linux_flags: u64, linux_path: &str) -> u64 {
    let flags = map_open_flags(linux_flags);
    let cloexec = (flags & 0x10) != 0;
    let file_flags = crate::fs::file::FileFlags {
        read: true,
        write: (flags & 1) != 0,
        append: (flags & 2) != 0,
        create: (flags & 4) != 0,
        truncate: (flags & 8) != 0,
        sync: (flags & 0x20) != 0,
    };
    let resolved_path = match linux_resolve_translated_path(path, true, file_flags.create) {
        Ok(path) => path,
        Err(errno) => {
            linux_log_path_error("open-resolve", linux_path, path, path, errno);
            return linux_err(errno);
        }
    };
    let global_id = match crate::fs::vfs::open(&resolved_path, file_flags) {
        Ok(id) => id,
        Err(e) => {
            let errno = fs_errno(e);
            linux_log_path_error("open", linux_path, path, &resolved_path, errno);
            return linux_err(errno);
        }
    };
    let local_fd =
        match crate::task::scheduler::current_fd_alloc(crate::fs::fd_table::FdKind::File {
            global_id,
        }) {
            Some(fd) => fd,
            None => {
                crate::fs::vfs::decref(global_id);
                return linux_err(EBADF);
            }
        };
    if cloexec {
        crate::task::scheduler::current_fd_set_cloexec(local_fd, true);
    }
    local_fd as u64
}

fn linux_stat_translated(path: &str, stat_ptr: u64, nofollow: bool) -> u64 {
    let path = match linux_resolve_translated_path(path, !nofollow, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let ret = if nofollow {
        crate::fs::vfs::lstat(&path)
    } else {
        crate::fs::vfs::stat(&path)
    };
    match ret {
        Ok(st) => {
            let type_val = match st.file_type {
                crate::fs::file::FileType::Directory => 1,
                crate::fs::file::FileType::Device => 2,
                _ => 0,
            };
            let uid = if st.uid == 0 {
                handlers::sys_getuid()
            } else {
                st.uid as u32
            };
            write_linux_stat(
                stat_ptr,
                type_val,
                st.size as u64,
                uid,
                st.gid as u32,
                st.mode as u32,
                st.mtime,
            );
            0
        }
        Err(crate::fs::vfs::FsError::NotFound) => linux_err(ENOENT),
        Err(e) => linux_fs_err(e),
    }
}

fn linux_fstat(fd: u32, stat_ptr: u64) -> u64 {
    if stat_ptr == 0 {
        return linux_err(EFAULT);
    }

    use crate::fs::fd_table::FdKind;
    match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => match entry.kind {
            FdKind::File { global_id } => match crate::fs::vfs::fstat(global_id) {
                Ok((file_type, size, _position, mtime)) => {
                    write_linux_stat(
                        stat_ptr,
                        anyos_file_type(file_type),
                        size as u64,
                        handlers::sys_getuid(),
                        handlers::sys_getgid(),
                        0o777,
                        mtime,
                    );
                    0
                }
                Err(_) => linux_err(EBADF),
            },
            FdKind::PipeRead { .. } | FdKind::PipeWrite { .. } | FdKind::Tty => {
                write_linux_stat(stat_ptr, 2, 0, 0, 0, 0o666, 0);
                0
            }
            FdKind::None => linux_err(EBADF),
        },
        None if fd < 3 => {
            write_linux_stat(stat_ptr, 2, 0, 0, 0, 0o666, 0);
            0
        }
        None => linux_err(EBADF),
    }
}

fn linux_brk(new_brk: u64) -> u64 {
    let current = crate::task::scheduler::current_thread_brk();
    if new_brk == 0 {
        crate::serial_println!("licof linux brk: query -> {:#x}", current);
        return current;
    }
    let delta = new_brk as i64 - current as i64;
    let old = handlers::sys_sbrk_u64(delta);
    if old == u64::MAX {
        crate::serial_println!(
            "licof linux brk: failed current={:#x} requested={:#x} delta={}",
            current,
            new_brk,
            delta
        );
        current
    } else {
        let updated = crate::task::scheduler::current_thread_brk();
        crate::serial_println!(
            "licof linux brk: current={:#x} requested={:#x} delta={} -> {:#x}",
            current,
            new_brk,
            delta,
            updated
        );
        updated
    }
}

fn linux_mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: u64, offset: u64) -> u64 {
    if len == 0 {
        crate::serial_println!(
            "licof linux mmap: reject zero len addr={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EINVAL);
    }
    let anonymous = (flags & LINUX_MAP_ANONYMOUS) != 0;
    let private = (flags & LINUX_MAP_PRIVATE) != 0;
    let fixed = (flags & LINUX_MAP_FIXED) != 0;
    if !private {
        crate::serial_println!(
            "licof linux mmap: reject !private addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOSYS);
    }
    if anonymous && !linux_fd_is_minus_one(fd) {
        crate::serial_println!(
            "licof linux mmap: reject anon fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EINVAL);
    }
    if !anonymous && linux_fd_is_minus_one(fd) {
        crate::serial_println!(
            "licof linux mmap: reject file fd addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(EBADF);
    }

    let mapped = if fixed {
        if addr == 0 {
            crate::serial_println!(
                "licof linux mmap: reject fixed-null len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
                len,
                prot,
                flags,
                fd,
                offset
            );
            return linux_err(EINVAL);
        }
        match linux_map_fixed(addr, len) {
            Some(addr) => addr,
            None => {
                crate::serial_println!(
                    "licof linux mmap: fixed failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
                    addr,
                    len,
                    prot,
                    flags,
                    fd,
                    offset
                );
                return linux_err(ENOMEM);
            }
        }
    } else {
        handlers::sys_mmap_u64(len)
    };
    if mapped == u64::MAX {
        crate::serial_println!(
            "licof linux mmap: alloc failed addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x}",
            addr,
            len,
            prot,
            flags,
            fd,
            offset
        );
        return linux_err(ENOMEM);
    }

    if !anonymous {
        if let Err(errno) = linux_fill_mapping_from_fd(fd as u32, mapped, len, offset) {
            let _ = handlers::sys_munmap_u64(mapped, len);
            crate::serial_println!(
                "licof linux mmap: fill failed mapped={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} errno={}",
                mapped,
                len,
                prot,
                flags,
                fd,
                offset,
                errno
            );
            return linux_err(errno);
        }
    }
    crate::serial_println!(
        "licof linux mmap: ok addr={:#x} len={:#x} prot={:#x} flags={:#x} fd={:#x} off={:#x} -> {:#x}",
        addr,
        len,
        prot,
        flags,
        fd,
        offset,
        mapped
    );
    mapped
}

fn linux_fd_is_minus_one(fd: u64) -> bool {
    fd == u64::MAX || fd == u32::MAX as u64
}

fn linux_munmap(addr: u64, len: u64) -> u64 {
    let ret = if addr <= u32::MAX as u64 {
        handlers::sys_munmap(addr as u32, len as u32) as u64
    } else {
        handlers::sys_munmap_u64(addr, len)
    };
    if ret == u64::MAX {
        linux_err(EINVAL)
    } else {
        0
    }
}

fn linux_mprotect(_addr: u64, len: u64, _prot: u64) -> u64 {
    if len == 0 {
        linux_err(EINVAL)
    } else {
        0
    }
}

fn linux_arch_prctl(code: u64, addr: u64) -> u64 {
    match code {
        LINUX_ARCH_SET_FS => {
            crate::task::scheduler::set_current_thread_linux_fs_base(addr);
            #[cfg(target_arch = "x86_64")]
            unsafe {
                crate::arch::x86::power::wrmsr(0xC000_0100, addr);
            }
            0
        }
        LINUX_ARCH_GET_FS => {
            if addr == 0 {
                return linux_err(EFAULT);
            }
            let fs_base = crate::task::scheduler::current_thread_linux_fs_base();
            unsafe {
                write_u64(addr, 0, fs_base);
            }
            0
        }
        _ => linux_err(EINVAL),
    }
}

fn linux_getcwd(buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 || buf_size > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    let ret = handlers::sys_getcwd(buf_ptr, buf_size as u32);
    if ret == u32::MAX {
        linux_err(EINVAL)
    } else {
        ret as u64 + 1
    }
}

fn linux_chdir(path_ptr: u64) -> u64 {
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    let linux_path = linux_absolute_path(raw_path);
    let host_path = linux_translate_path(&linux_path);
    match crate::fs::vfs::read_dir(&host_path) {
        Ok(_) => {
            crate::task::scheduler::set_thread_cwd(
                crate::task::scheduler::current_tid(),
                &linux_path,
            );
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

fn linux_readlink(path_ptr: u64, buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 {
        return linux_err(EFAULT);
    }
    let raw_path = match super::handlers::helpers::read_user_str_safe(path_ptr) {
        Some(path) => path,
        None => return linux_err(EFAULT),
    };
    if raw_path == "/proc/self/exe" {
        return linux_err(ENOENT);
    }
    let path = linux_translate_path(raw_path);
    match crate::fs::vfs::readlink(&path) {
        Ok(target) => {
            let visible = linux_strip_rootfs(&target);
            let bytes = visible.as_bytes();
            let to_copy = bytes.len().min(buf_size as usize);
            if !super::handlers::helpers::copy_to_user_bytes(buf_ptr, &bytes[..to_copy], to_copy) {
                return linux_err(EFAULT);
            }
            to_copy as u64
        }
        Err(e) => linux_fs_err(e),
    }
}

fn linux_readlinkat(dirfd: u64, path_ptr: u64, buf_ptr: u64, buf_size: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_readlink_translated(&path, buf_ptr, buf_size)
}

fn linux_readlink_translated(path: &str, buf_ptr: u64, buf_size: u64) -> u64 {
    if buf_ptr == 0 || buf_size == 0 {
        return linux_err(EFAULT);
    }
    match crate::fs::vfs::readlink(path) {
        Ok(target) => {
            let visible = linux_strip_rootfs(&target);
            let bytes = visible.as_bytes();
            let to_copy = bytes.len().min(buf_size as usize);
            if !super::handlers::helpers::copy_to_user_bytes(buf_ptr, &bytes[..to_copy], to_copy) {
                return linux_err(EFAULT);
            }
            to_copy as u64
        }
        Err(e) => linux_fs_err(e),
    }
}

fn linux_rename(old_ptr: u64, new_ptr: u64) -> u64 {
    let Some(old_path) = linux_translate_user_path(old_ptr) else {
        return linux_err(EFAULT);
    };
    let Some(new_path) = linux_translate_user_path(new_ptr) else {
        return linux_err(EFAULT);
    };
    match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_renameat(old_dirfd: u64, old_ptr: u64, new_dirfd: u64, new_ptr: u64) -> u64 {
    let old_path = match linux_translate_at_path(old_dirfd, old_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let new_path = match linux_translate_at_path(new_dirfd, new_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::rename(&old_path, &new_path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_mkdir(path_ptr: u64, _mode: u64) -> u64 {
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_mkdirat(dirfd: u64, path_ptr: u64, _mode: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::mkdir(&path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_unlink_path(path_ptr: u64) -> u64 {
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    linux_unlink_translated(&path)
}

fn linux_unlinkat(dirfd: u64, path_ptr: u64, flags: u64) -> u64 {
    if (flags & !LINUX_AT_REMOVEDIR) != 0 {
        return linux_err(EINVAL);
    }
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_unlink_translated(&path)
}

fn linux_unlink_translated(path: &str) -> u64 {
    match crate::fs::vfs::delete(path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_creat(path_ptr: u64) -> u64 {
    const O_CREAT_WRONLY_TRUNC: u64 = 0x40 | 0x01 | 0x200;
    linux_open(path_ptr, O_CREAT_WRONLY_TRUNC)
}

fn linux_truncate(path_ptr: u64, len: u64) -> u64 {
    if len != 0 {
        return linux_err(ENOSYS);
    }
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    match crate::fs::vfs::truncate(&path) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_ftruncate(fd: u32, len: u64) -> u64 {
    if len != 0 {
        return linux_err(ENOSYS);
    }
    anyos_u32_ret(handlers::sys_ftruncate(fd, len as u32))
}

fn linux_chmod(path_ptr: u64, mode: u64) -> u64 {
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_fchmod(fd: u32, mode: u64) -> u64 {
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_fchmodat(dirfd: u64, path_ptr: u64, mode: u64, _flags: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::set_mode(&path, mode as u16) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_chown(path_ptr: u64, uid: u64, gid: u64) -> u64 {
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    linux_chown_translated(&path, uid, gid)
}

fn linux_fchown(fd: u32, uid: u64, gid: u64) -> u64 {
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_chown_translated(&path, uid, gid)
}

fn linux_fchownat(dirfd: u64, path_ptr: u64, uid: u64, gid: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_chown_translated(&path, uid, gid)
}

fn linux_chown_translated(path: &str, uid: u64, gid: u64) -> u64 {
    let (current_uid, current_gid, _) = crate::fs::vfs::get_permissions(path).unwrap_or((0, 0, 0));
    let uid = if uid == u32::MAX as u64 {
        current_uid
    } else {
        uid as u16
    };
    let gid = if gid == u32::MAX as u64 {
        current_gid
    } else {
        gid as u16
    };
    match crate::fs::vfs::set_owner(path, uid, gid) {
        Ok(()) => 0,
        Err(e) => linux_fs_err(e),
    }
}

fn linux_faccessat(dirfd: u64, path_ptr: u64, mode: u64, _flags: u64) -> u64 {
    let path = match linux_translate_at_path(dirfd, path_ptr) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    let resolved = match linux_resolve_translated_path(&path, true, false) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    match crate::fs::vfs::stat(&resolved) {
        Ok(_) => {
            let _ = mode;
            0
        }
        Err(e) => linux_fs_err(e),
    }
}

fn linux_fcntl(fd: u32, cmd: u32, arg: u64) -> u64 {
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

fn linux_getdents64(fd: u32, dirent_ptr: u64, count: u64) -> u64 {
    if dirent_ptr == 0 || count < 24 || count > u32::MAX as u64 {
        return linux_err(EFAULT);
    }
    let entry = match crate::task::scheduler::current_fd_get(fd) {
        Some(entry) => entry,
        None => return linux_err(EBADF),
    };
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return linux_err(EBADF),
    };
    let (file_type, _size, position, _mtime) = match crate::fs::vfs::fstat(global_id) {
        Ok(st) => st,
        Err(e) => return linux_fs_err(e),
    };
    if file_type != crate::fs::file::FileType::Directory {
        return linux_err(ENOTTY);
    }
    let path = match crate::fs::vfs::get_fd_path(global_id) {
        Ok(path) => path,
        Err(e) => return linux_fs_err(e),
    };
    let entries = match crate::fs::vfs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) => return linux_fs_err(e),
    };

    let start = position as usize;
    let mut written = 0usize;
    let mut next_index = start;
    for (idx, entry) in entries.iter().enumerate().skip(start) {
        let name = entry.name.as_bytes();
        let reclen = align_up(19 + name.len() + 1, 8);
        if written + reclen > count as usize {
            break;
        }
        let base = dirent_ptr + written as u64;
        unsafe {
            write_u64(base, 0, (idx + 1) as u64);
            write_u64(base, 8, (idx + 1) as u64);
            *((base + 16) as *mut u16) = reclen as u16;
            *((base + 18) as *mut u8) = linux_dir_type(entry.file_type);
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
    let _ = crate::fs::vfs::lseek(global_id, next_index as i32, 0);
    written as u64
}

fn linux_uname(buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 65 * 6);
    }
    write_linux_uts_field(buf_ptr, 0, b"Linux");
    write_linux_uts_field(buf_ptr, 1, b"anyos");
    write_linux_uts_field(buf_ptr, 2, b"3.2.0-licof");
    write_linux_uts_field(buf_ptr, 3, b"#1 anyOS licof Linux ABI");
    write_linux_uts_field(buf_ptr, 4, b"x86_64");
    write_linux_uts_field(buf_ptr, 5, b"anyos");
    0
}

fn linux_getrandom(buf_ptr: u64, len: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let len = len.min(256);
    let mut seed = crate::arch::hal::timer_current_ticks() as u64
        ^ ((crate::task::scheduler::current_tid() as u64) << 32)
        ^ 0xD1CF_600D_51A7_EA5Eu64;
    unsafe {
        for i in 0..len {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            *((buf_ptr + i) as *mut u8) = (seed >> ((i & 7) * 8)) as u8;
        }
    }
    len
}

fn linux_pread64(fd: u32, buf_ptr: u64, len: u64, offset: u64) -> u64 {
    if buf_ptr == 0 || len > usize::MAX as u64 {
        return linux_err(EFAULT);
    }
    match linux_read_fd_at(fd, buf_ptr, len as usize, offset) {
        Ok(n) => n as u64,
        Err(errno) => linux_err(errno),
    }
}

fn linux_readv(fd: u32, iov_ptr: u64, iovcnt: u64) -> u64 {
    linux_iov_io(fd, iov_ptr, iovcnt, false)
}

fn linux_writev(fd: u32, iov_ptr: u64, iovcnt: u64) -> u64 {
    linux_iov_io(fd, iov_ptr, iovcnt, true)
}

fn linux_iov_io(fd: u32, iov_ptr: u64, iovcnt: u64, write: bool) -> u64 {
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
            handlers::sys_read(fd, base, len as u32)
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

fn linux_poll(fds_ptr: u64, nfds: u64, _timeout: u64) -> u64 {
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

fn linux_rt_sigaction(_sig: u64, _act: u64, oldact: u64, sigsetsize: u64) -> u64 {
    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if oldact != 0 {
        unsafe {
            core::ptr::write_bytes(oldact as *mut u8, 0, 32);
        }
    }
    0
}

fn linux_rt_sigprocmask(_how: u64, _set: u64, oldset: u64, sigsetsize: u64) -> u64 {
    if sigsetsize != 8 {
        return linux_err(EINVAL);
    }
    if oldset != 0 {
        unsafe {
            write_u64(oldset, 0, 0);
        }
    }
    0
}

fn linux_ioctl(fd: u32, request: u64, arg: u64) -> u64 {
    const TCGETS: u64 = 0x5401;
    const TIOCGWINSZ: u64 = 0x5413;
    match request {
        TCGETS => {
            if fd > 2 && crate::task::scheduler::current_fd_get(fd).is_none() {
                return linux_err(EBADF);
            }
            if arg != 0 {
                unsafe {
                    core::ptr::write_bytes(arg as *mut u8, 0, 60);
                }
            }
            0
        }
        TIOCGWINSZ => {
            if arg != 0 {
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
            0
        }
        _ => linux_err(ENOTTY),
    }
}

fn linux_gettimeofday(tv_ptr: u64) -> u64 {
    if tv_ptr != 0 {
        let sec = linux_now_seconds();
        unsafe {
            write_u64(tv_ptr, 0, sec);
            write_u64(tv_ptr, 8, 0);
        }
    }
    0
}

fn linux_time(time_ptr: u64) -> u64 {
    let sec = linux_now_seconds();
    if time_ptr != 0 {
        unsafe {
            write_u64(time_ptr, 0, sec);
        }
    }
    sec
}

fn linux_clock_gettime(_clock_id: u64, ts_ptr: u64) -> u64 {
    if ts_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u64(ts_ptr, 0, linux_now_seconds());
        write_u64(ts_ptr, 8, 0);
    }
    0
}

fn linux_sysinfo(info_ptr: u64) -> u64 {
    if info_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(info_ptr as *mut u8, 0, 112);
        write_u64(info_ptr, 0, crate::arch::hal::timer_current_ticks() as u64);
        write_u64(info_ptr, 8, 1);
        write_u64(info_ptr, 16, 1);
        write_u64(info_ptr, 24, 1);
        write_u64(info_ptr, 32, 0);
        write_u64(info_ptr, 40, 0);
        write_u64(info_ptr, 48, 0);
        write_u64(info_ptr, 56, 0);
        write_u16(info_ptr, 104, 1);
    }
    0
}

fn linux_getgroups(size: u64, list_ptr: u64) -> u64 {
    if size == 0 {
        return 1;
    }
    if list_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        write_u32(list_ptr, 0, handlers::sys_getgid());
    }
    1
}

fn linux_setgroups(size: u64, _list_ptr: u64) -> u64 {
    if handlers::sys_getuid() != 0 {
        return linux_err(EPERM);
    }
    if size > 1024 {
        return linux_err(EINVAL);
    }
    0
}

fn linux_setres_id(real: u64, effective: u64, saved: u64, uid: bool) -> u64 {
    for value in [real, effective, saved] {
        if value != u32::MAX as u64 && value != 0 && value != current_linux_id(uid) as u64 {
            return linux_err(EPERM);
        }
    }
    0
}

fn linux_getres_id(real_ptr: u64, effective_ptr: u64, saved_ptr: u64, uid: bool) -> u64 {
    if real_ptr == 0 || effective_ptr == 0 || saved_ptr == 0 {
        return linux_err(EFAULT);
    }
    let id = current_linux_id(uid);
    unsafe {
        write_u32(real_ptr, 0, id);
        write_u32(effective_ptr, 0, id);
        write_u32(saved_ptr, 0, id);
    }
    0
}

fn linux_setfs_id(id: u64, uid: bool) -> u64 {
    let old = current_linux_id(uid);
    if id != u32::MAX as u64 && id != 0 && id != old as u64 {
        return linux_err(EPERM);
    }
    old as u64
}

fn current_linux_id(uid: bool) -> u32 {
    if uid {
        handlers::sys_getuid()
    } else {
        handlers::sys_getgid()
    }
}

fn linux_set_root_or_current(id: u64, uid: bool) -> u64 {
    if id == 0 || id == current_linux_id(uid) as u64 {
        0
    } else {
        linux_err(EPERM)
    }
}

fn linux_capget(header_ptr: u64, data_ptr: u64) -> u64 {
    if header_ptr == 0 {
        return linux_err(EFAULT);
    }
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
    unsafe {
        write_u32(header_ptr, 0, LINUX_CAPABILITY_VERSION_3);
    }
    if data_ptr != 0 {
        let effective = if handlers::sys_getuid() == 0 {
            u32::MAX
        } else {
            0
        };
        unsafe {
            write_u32(data_ptr, 0, effective);
            write_u32(data_ptr, 4, effective);
            write_u32(data_ptr, 8, 0);
            write_u32(data_ptr, 12, effective);
            write_u32(data_ptr, 16, effective);
            write_u32(data_ptr, 20, 0);
        }
    }
    0
}

fn linux_capset(_header_ptr: u64, _data_ptr: u64) -> u64 {
    if handlers::sys_getuid() == 0 {
        0
    } else {
        linux_err(EPERM)
    }
}

fn linux_statfs(path_ptr: u64, buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let Some(path) = linux_translate_user_path(path_ptr) else {
        return linux_err(EFAULT);
    };
    linux_statfs_translated(&path, buf_ptr)
}

fn linux_fstatfs(fd: u32, buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    let path = match linux_fd_path(fd) {
        Ok(path) => path,
        Err(errno) => return linux_err(errno),
    };
    linux_statfs_translated(&path, buf_ptr)
}

fn linux_statfs_translated(path: &str, buf_ptr: u64) -> u64 {
    let st = crate::fs::vfs::statfs(path).unwrap_or(crate::fs::vfs::StatFs {
        total_bytes: 64 * 1024 * 1024,
        used_bytes: 0,
        free_bytes: 64 * 1024 * 1024,
    });
    let block_size = 4096u64;
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 120);
        write_u64(buf_ptr, 0, 0xEF53);
        write_u64(buf_ptr, 8, block_size);
        write_u64(buf_ptr, 16, st.total_bytes / block_size);
        write_u64(buf_ptr, 24, st.free_bytes / block_size);
        write_u64(buf_ptr, 32, st.free_bytes / block_size);
        write_u64(buf_ptr, 40, 1024 * 1024);
        write_u64(buf_ptr, 48, 1024 * 1024);
        write_u32(buf_ptr, 72, 255);
        write_u64(buf_ptr, 80, block_size);
    }
    0
}

fn linux_prctl(option: u64, arg2: u64) -> u64 {
    match option {
        3 => 1,  // PR_GET_DUMPABLE
        4 => 0,  // PR_SET_DUMPABLE
        15 => 0, // PR_SET_NAME
        16 => {
            if arg2 == 0 {
                return linux_err(EFAULT);
            }
            let name = b"licof\0";
            if !super::handlers::helpers::copy_to_user_bytes(arg2, name, name.len()) {
                return linux_err(EFAULT);
            }
            0
        }
        38 => 0, // PR_SET_NO_NEW_PRIVS
        39 => 0, // PR_GET_NO_NEW_PRIVS
        _ => linux_err(EINVAL),
    }
}

fn linux_prlimit64(_pid: u64, resource: u64, _new_limit: u64, old_limit: u64) -> u64 {
    if old_limit == 0 {
        return 0;
    }
    let (cur, max) = match resource {
        3 => (8 * 1024 * 1024, 8 * 1024 * 1024), // RLIMIT_STACK
        7 => (64, 64),                           // RLIMIT_NOFILE
        _ => (u64::MAX, u64::MAX),
    };
    unsafe {
        write_u64(old_limit, 0, cur);
        write_u64(old_limit, 8, max);
    }
    0
}

fn linux_futex(_uaddr: u64, op: u64, _val: u64) -> u64 {
    let cmd = op & 0x7F;
    match cmd {
        0 => linux_err(EAGAIN), // FUTEX_WAIT
        1 => 0,                 // FUTEX_WAKE
        _ => linux_err(ENOSYS),
    }
}

fn linux_map_fixed(addr: u64, len: u64) -> Option<u64> {
    use crate::memory::address::VirtAddr;
    use crate::memory::{physical, virtual_mem};

    const PAGE_SIZE: u64 = 4096;
    if addr & (PAGE_SIZE - 1) != 0 {
        return None;
    }
    let aligned_size = len.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    if addr <= u32::MAX as u64 {
        let _ = handlers::sys_munmap(addr as u32, aligned_size as u32);
    } else {
        let _ = handlers::sys_munmap_u64(addr, aligned_size);
    }

    let pd = crate::task::scheduler::current_thread_page_directory()?;
    if !crate::memory::vma::alloc_fixed_region64(pd, addr, aligned_size) {
        return None;
    }

    let mut mapped_until = addr;
    while mapped_until < addr + aligned_size {
        let phys = match physical::alloc_frame_with(physical::FrameAllocPolicy::Any) {
            Some(phys) => phys,
            None => {
                let _ = linux_munmap(addr, aligned_size);
                return None;
            }
        };
        if !virtual_mem::zero_frame(phys)
            || !virtual_mem::map_page(VirtAddr::new(mapped_until), phys, 0x02 | 0x04)
        {
            physical::free_frame(phys);
            let _ = linux_munmap(addr, aligned_size);
            return None;
        }
        mapped_until += PAGE_SIZE;
    }
    Some(addr)
}

fn linux_fill_mapping_from_fd(fd: u32, addr: u64, len: u64, offset: u64) -> Result<(), i32> {
    let mut copied = 0usize;
    let len = len as usize;
    let mut tmp = [0u8; 4096];
    while copied < len {
        let want = (len - copied).min(tmp.len());
        let n = linux_read_fd_at(fd, tmp.as_mut_ptr() as u64, want, offset + copied as u64)?;
        if n == 0 {
            break;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(tmp.as_ptr(), (addr as usize + copied) as *mut u8, n);
        }
        copied += n;
    }
    Ok(())
}

fn linux_read_fd_at(fd: u32, buf_ptr: u64, len: usize, offset: u64) -> Result<usize, i32> {
    if offset > i32::MAX as u64 {
        return Err(EINVAL);
    }
    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return Err(EBADF),
    };
    let (_file_type, _size, old_pos, _mtime) =
        crate::fs::vfs::fstat(global_id).map_err(fs_errno)?;
    crate::fs::vfs::lseek(global_id, offset as i32, 0).map_err(fs_errno)?;
    let read_result = unsafe {
        let out = core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len);
        crate::fs::vfs::read(global_id, out).map_err(fs_errno)
    };
    let _ = crate::fs::vfs::lseek(global_id, old_pos as i32, 0);
    read_result
}

fn linux_translate_user_path(path_ptr: u64) -> Option<String> {
    super::handlers::helpers::read_user_str_safe(path_ptr).map(linux_translate_path)
}

fn linux_translate_at_path(dirfd: u64, path_ptr: u64) -> Result<String, i32> {
    let path = super::handlers::helpers::read_user_str_safe(path_ptr).ok_or(EFAULT)?;
    if path.starts_with('/') || (dirfd as i32) == LINUX_AT_FDCWD {
        return Ok(linux_translate_path(path));
    }
    let base = linux_fd_path(dirfd as u32)?;
    Ok(crate::fs::path::normalize(&alloc::format!(
        "{}/{}", base, path
    )))
}

fn linux_translate_path(path: &str) -> String {
    let abs = linux_absolute_path(path);
    linux_translate_absolute_path(&abs)
}

fn linux_translate_absolute_path(abs: &str) -> String {
    match abs {
        "/dev/null" | "/dev/zero" | "/dev/console" => return String::from(abs),
        "/dev/tty" => return String::from("/dev/console"),
        _ => {}
    }
    let rootfs = current_linux_rootfs();
    if abs == "/" {
        rootfs
    } else {
        alloc::format!("{}{}", rootfs, abs)
    }
}

fn linux_absolute_path(path: &str) -> String {
    if path.starts_with('/') {
        crate::fs::path::normalize(path)
    } else {
        let mut cwd_buf = [0u8; 512];
        let cwd_len = crate::task::scheduler::current_thread_cwd(&mut cwd_buf);
        let cwd = core::str::from_utf8(&cwd_buf[..cwd_len]).unwrap_or("/");
        if cwd == "/" || cwd.is_empty() {
            crate::fs::path::normalize(&alloc::format!("/{}", path))
        } else {
            crate::fs::path::normalize(&alloc::format!("{}/{}", cwd, path))
        }
    }
}

fn linux_fd_path(fd: u32) -> Result<String, i32> {
    let entry = crate::task::scheduler::current_fd_get(fd).ok_or(EBADF)?;
    let global_id = match entry.kind {
        crate::fs::fd_table::FdKind::File { global_id } => global_id,
        _ => return Err(EBADF),
    };
    crate::fs::vfs::get_fd_path(global_id).map_err(fs_errno)
}

fn linux_strip_rootfs(path: &str) -> String {
    let rootfs = current_linux_rootfs();
    if path == rootfs {
        String::from("/")
    } else if path.starts_with(&rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        String::from(&path[rootfs.len()..])
    } else {
        String::from(path)
    }
}

fn linux_resolve_translated_path(
    path: &str,
    follow_last: bool,
    allow_missing_final: bool,
) -> Result<String, i32> {
    let rootfs = current_linux_rootfs();
    let normalized = crate::fs::path::normalize(path);
    if !linux_path_under_rootfs(&rootfs, &normalized) {
        return Ok(normalized);
    }
    linux_resolve_translated_path_inner(&rootfs, &normalized, follow_last, allow_missing_final, 0)
}

fn linux_resolve_translated_path_inner(
    rootfs: &str,
    path: &str,
    follow_last: bool,
    allow_missing_final: bool,
    depth: u32,
) -> Result<String, i32> {
    if depth > 16 {
        return Err(ELOOP);
    }
    let normalized = crate::fs::path::normalize(path);
    let rel = linux_rootfs_relative(rootfs, &normalized);
    let components: Vec<&str> = rel
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let mut current = String::from(rootfs);
    if components.is_empty() {
        return Ok(current);
    }

    for (idx, component) in components.iter().enumerate() {
        let is_last = idx == components.len() - 1;
        let candidate = linux_join_path(&current, component);
        match crate::fs::vfs::lstat(&candidate) {
            Ok(st) if st.is_symlink && (!is_last || follow_last) => {
                let target = crate::fs::vfs::readlink(&candidate).map_err(fs_errno)?;
                let parent = linux_parent_path(&candidate);
                let next =
                    linux_resolve_link_target(rootfs, &parent, &target, &components[idx + 1..]);
                return linux_resolve_translated_path_inner(
                    rootfs,
                    &next,
                    true,
                    allow_missing_final,
                    depth + 1,
                );
            }
            Ok(_) => {
                current = candidate;
            }
            Err(crate::fs::vfs::FsError::NotFound) if is_last && allow_missing_final => {
                return Ok(candidate);
            }
            Err(e) => return Err(fs_errno(e)),
        }
    }
    Ok(current)
}

fn linux_path_under_rootfs(rootfs: &str, path: &str) -> bool {
    path == rootfs || (path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/'))
}

fn linux_rootfs_relative<'a>(rootfs: &str, path: &'a str) -> &'a str {
    if path == rootfs {
        ""
    } else if path.starts_with(rootfs) && path.as_bytes().get(rootfs.len()) == Some(&b'/') {
        &path[rootfs.len() + 1..]
    } else {
        path.trim_start_matches('/')
    }
}

fn linux_join_path(base: &str, component: &str) -> String {
    if base == "/" {
        alloc::format!("/{}", component)
    } else if base.ends_with('/') {
        alloc::format!("{}{}", base, component)
    } else {
        alloc::format!("{}/{}", base, component)
    }
}

fn linux_parent_path(path: &str) -> String {
    let normalized = crate::fs::path::normalize(path);
    match normalized.rfind('/') {
        Some(0) | None => String::from("/"),
        Some(idx) => String::from(&normalized[..idx]),
    }
}

fn linux_resolve_link_target(
    rootfs: &str,
    parent: &str,
    target: &str,
    remaining: &[&str],
) -> String {
    let mut path = if target.starts_with('/') {
        if target == "/" {
            String::from(rootfs)
        } else {
            alloc::format!("{}{}", rootfs, target)
        }
    } else {
        linux_join_path(parent, target)
    };
    for component in remaining {
        path = linux_join_path(&path, component);
    }
    crate::fs::path::normalize(&path)
}

fn linux_log_path_error(
    op: &str,
    linux_path: &str,
    translated_path: &str,
    resolved_path: &str,
    errno: i32,
) {
    crate::serial_println!(
        "licof linux {}: failed errno={} linux='{}' translated='{}' resolved='{}'",
        op,
        errno,
        linux_path,
        translated_path,
        resolved_path
    );
}

fn current_linux_rootfs() -> String {
    let mut buf = [0u8; 512];
    let len = crate::task::scheduler::current_thread_linux_rootfs(&mut buf);
    if len == 0 {
        return String::from(LICOF_ROOTFS);
    }
    core::str::from_utf8(&buf[..len])
        .map(String::from)
        .unwrap_or_else(|_| String::from(LICOF_ROOTFS))
}

fn linux_now_seconds() -> u64 {
    let ticks = crate::arch::hal::timer_current_ticks() as u64;
    let hz = crate::arch::hal::timer_frequency_hz() as u64;
    if hz == 0 {
        0
    } else {
        ticks / hz
    }
}

fn linux_fs_err(e: crate::fs::vfs::FsError) -> u64 {
    linux_err(fs_errno(e))
}

fn fs_errno(e: crate::fs::vfs::FsError) -> i32 {
    use crate::fs::vfs::FsError;
    match e {
        FsError::NotFound => 2,
        FsError::PermissionDenied => 13,
        FsError::AlreadyExists => 17,
        FsError::NotADirectory => 20,
        FsError::IsADirectory => 21,
        FsError::NoSpace => 28,
        FsError::IoError => 5,
        FsError::InvalidPath => 22,
        FsError::TooManyOpenFiles => 24,
        FsError::BadFd => 9,
        FsError::DirectoryNotEmpty => 39,
        FsError::NotSupported => 95,
    }
}

fn write_linux_uts_field(base: u64, index: u64, value: &[u8]) {
    let len = value.len().min(64);
    unsafe {
        core::ptr::copy_nonoverlapping(value.as_ptr(), (base + index * 65) as *mut u8, len);
    }
}

fn write_linux_stat(
    stat_ptr: u64,
    anyos_type: u32,
    size: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    mtime: u32,
) {
    let file_type_bits = match anyos_type {
        1 => 0o040000,
        2 => 0o020000,
        _ => 0o100000,
    };
    let full_mode = file_type_bits | (mode & 0o7777);
    unsafe {
        core::ptr::write_bytes(stat_ptr as *mut u8, 0, 144);
        write_u64(stat_ptr, 0, 0);
        write_u64(stat_ptr, 8, 1);
        write_u64(stat_ptr, 16, 1);
        write_u32(stat_ptr, 24, full_mode);
        write_u32(stat_ptr, 28, uid);
        write_u32(stat_ptr, 32, gid);
        write_u64(stat_ptr, 40, 0);
        write_u64(stat_ptr, 48, size);
        write_u64(stat_ptr, 56, 4096);
        write_u64(stat_ptr, 64, (size + 511) / 512);
        write_u64(stat_ptr, 72, mtime as u64);
        write_u64(stat_ptr, 88, mtime as u64);
        write_u64(stat_ptr, 104, mtime as u64);
    }
}

fn anyos_file_type(file_type: crate::fs::file::FileType) -> u32 {
    match file_type {
        crate::fs::file::FileType::Regular => 0,
        crate::fs::file::FileType::Directory => 1,
        crate::fs::file::FileType::Device => 2,
    }
}

unsafe fn write_u32(base: u64, offset: u64, value: u32) {
    *((base + offset) as *mut u32) = value;
}

unsafe fn write_u16(base: u64, offset: u64, value: u16) {
    *((base + offset) as *mut u16) = value;
}

unsafe fn write_u64(base: u64, offset: u64, value: u64) {
    *((base + offset) as *mut u64) = value;
}

unsafe fn read_u64(base: u64, offset: u64) -> u64 {
    *((base + offset) as *const u64)
}

fn linux_dir_type(file_type: crate::fs::file::FileType) -> u8 {
    match file_type {
        crate::fs::file::FileType::Regular => 8,   // DT_REG
        crate::fs::file::FileType::Directory => 4, // DT_DIR
        crate::fs::file::FileType::Device => 2,    // DT_CHR
    }
}

fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

fn map_open_flags(flags: u64) -> u32 {
    let mut out = 0u32;
    let accmode = flags & 0x3;
    if accmode == 1 || accmode == 2 {
        out |= 1;
    }
    if (flags & 0x40) != 0 {
        out |= 4;
    }
    if (flags & 0x200) != 0 {
        out |= 8;
    }
    if (flags & 0x400) != 0 {
        out |= 2;
    }
    if (flags & 0x80000) != 0 {
        out |= 0x10;
    }
    out
}

fn anyos_u32_ret(ret: u32) -> u64 {
    if (ret as i32) < 0 {
        (ret as i32 as i64) as u64
    } else {
        ret as u64
    }
}

fn linux_err(errno: i32) -> u64 {
    (-(errno as i64)) as u64
}
