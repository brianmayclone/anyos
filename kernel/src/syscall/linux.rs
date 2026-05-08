//! licof Linux x86_64 syscall dispatch.
//!
//! This is intentionally a narrow Tier-0 bridge. Unsupported syscalls return
//! `-ENOSYS` using Linux's negative-errno convention.

use super::{handlers, SyscallRegs};

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOENT: i32 = 2;
const ENOSYS: i32 = 38;

const LINUX_SYS_READ: u64 = 0;
const LINUX_SYS_WRITE: u64 = 1;
const LINUX_SYS_OPEN: u64 = 2;
const LINUX_SYS_CLOSE: u64 = 3;
const LINUX_SYS_STAT: u64 = 4;
const LINUX_SYS_FSTAT: u64 = 5;
const LINUX_SYS_LSTAT: u64 = 6;
const LINUX_SYS_LSEEK: u64 = 8;
const LINUX_SYS_MMAP: u64 = 9;
const LINUX_SYS_MUNMAP: u64 = 11;
const LINUX_SYS_BRK: u64 = 12;
const LINUX_SYS_GETPID: u64 = 39;
const LINUX_SYS_UNAME: u64 = 63;
const LINUX_SYS_EXIT: u64 = 60;
const LINUX_SYS_GETCWD: u64 = 79;
const LINUX_SYS_CHDIR: u64 = 80;
const LINUX_SYS_READLINK: u64 = 89;
const LINUX_SYS_GETUID: u64 = 102;
const LINUX_SYS_GETGID: u64 = 104;
const LINUX_SYS_ARCH_PRCTL: u64 = 158;
const LINUX_SYS_SET_TID_ADDRESS: u64 = 218;
const LINUX_SYS_EXIT_GROUP: u64 = 231;
const LINUX_SYS_OPENAT: u64 = 257;
const LINUX_SYS_NEWFSTATAT: u64 = 262;
const LINUX_SYS_SET_ROBUST_LIST: u64 = 273;
const LINUX_SYS_GETRANDOM: u64 = 318;
const LINUX_SYS_RSEQ: u64 = 334;

const LINUX_AT_FDCWD: i32 = -100;
const LINUX_AT_SYMLINK_NOFOLLOW: u64 = 0x100;
const LINUX_MAP_ANONYMOUS: u64 = 0x20;
const LINUX_MAP_PRIVATE: u64 = 0x02;
const LINUX_ARCH_SET_FS: u64 = 0x1002;
const LINUX_ARCH_GET_FS: u64 = 0x1003;

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax;
    let a1 = regs.rdi;
    let a2 = regs.rsi;
    let a3 = regs.rdx;
    let a4 = regs.r10;
    let a5 = regs.r8;
    let _a6 = regs.r9;

    match nr {
        LINUX_SYS_READ => anyos_u32_ret(handlers::sys_read(a1 as u32, a2, a3 as u32)),
        LINUX_SYS_WRITE => anyos_u32_ret(handlers::sys_write(a1 as u32, a2, a3 as u32)),
        LINUX_SYS_OPEN => linux_open(a1, a2),
        LINUX_SYS_OPENAT => linux_openat(a1, a2, a3),
        LINUX_SYS_CLOSE => anyos_u32_ret(handlers::sys_close(a1 as u32)),
        LINUX_SYS_STAT => linux_stat(a1, a2, false),
        LINUX_SYS_LSTAT => linux_stat(a1, a2, true),
        LINUX_SYS_FSTAT => linux_fstat(a1 as u32, a2),
        LINUX_SYS_LSEEK => anyos_u32_ret(handlers::sys_lseek(a1 as u32, a2 as u32, a3 as u32)),
        LINUX_SYS_BRK => linux_brk(a1),
        LINUX_SYS_MMAP => linux_mmap(a1, a2, a3, a4, a5),
        LINUX_SYS_MUNMAP => linux_munmap(a1, a2),
        LINUX_SYS_ARCH_PRCTL => linux_arch_prctl(a1, a2),
        LINUX_SYS_GETPID => handlers::sys_getpid() as u64,
        LINUX_SYS_UNAME => linux_uname(a1),
        LINUX_SYS_GETCWD => linux_getcwd(a1, a2),
        LINUX_SYS_CHDIR => anyos_u32_ret(handlers::sys_chdir(a1)),
        LINUX_SYS_READLINK => anyos_u32_ret(handlers::sys_readlink(a1, a2, a3 as u32)),
        LINUX_SYS_GETUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETGID => handlers::sys_getgid() as u64,
        LINUX_SYS_SET_TID_ADDRESS => handlers::sys_getpid() as u64,
        LINUX_SYS_EXIT | LINUX_SYS_EXIT_GROUP => handlers::sys_exit(a1 as u32) as u64,
        LINUX_SYS_NEWFSTATAT => linux_newfstatat(a1, a2, a3, a4),
        LINUX_SYS_SET_ROBUST_LIST => 0,
        LINUX_SYS_GETRANDOM => linux_getrandom(a1, a2),
        LINUX_SYS_RSEQ => linux_err(ENOSYS),
        _ => linux_err(ENOSYS),
    }
}

fn linux_open(path_ptr: u64, linux_flags: u64) -> u64 {
    anyos_u32_ret(handlers::sys_open(path_ptr, map_open_flags(linux_flags), 0))
}

fn linux_openat(dirfd: u64, path_ptr: u64, linux_flags: u64) -> u64 {
    let dirfd = dirfd as i32;
    if dirfd != LINUX_AT_FDCWD {
        return linux_err(ENOSYS);
    }
    anyos_u32_ret(handlers::sys_open(path_ptr, map_open_flags(linux_flags), 0))
}

fn linux_stat(path_ptr: u64, stat_ptr: u64, nofollow: bool) -> u64 {
    if path_ptr == 0 || stat_ptr == 0 {
        return linux_err(EFAULT);
    }
    let mut tmp = [0u32; 7];
    let ret = if nofollow {
        handlers::sys_lstat(path_ptr, tmp.as_mut_ptr() as u64)
    } else {
        handlers::sys_stat(path_ptr, tmp.as_mut_ptr() as u64)
    };
    if ret != 0 {
        return linux_err(ENOENT);
    }
    write_linux_stat(
        stat_ptr,
        tmp[0],
        tmp[1] as u64,
        tmp[3],
        tmp[4],
        tmp[5],
        tmp[6],
    );
    0
}

fn linux_newfstatat(dirfd: u64, path_ptr: u64, stat_ptr: u64, flags: u64) -> u64 {
    if (dirfd as i32) != LINUX_AT_FDCWD {
        return linux_err(ENOSYS);
    }
    linux_stat(path_ptr, stat_ptr, (flags & LINUX_AT_SYMLINK_NOFOLLOW) != 0)
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
        return current;
    }
    let delta = new_brk as i64 - current as i64;
    let old = handlers::sys_sbrk_u64(delta);
    if old == u64::MAX {
        current
    } else {
        crate::task::scheduler::current_thread_brk()
    }
}

fn linux_mmap(addr: u64, len: u64, _prot: u64, flags: u64, fd: u64) -> u64 {
    if len == 0 {
        return linux_err(EINVAL);
    }
    if addr != 0 {
        return linux_err(ENOSYS);
    }
    let anonymous = (flags & LINUX_MAP_ANONYMOUS) != 0;
    let private = (flags & LINUX_MAP_PRIVATE) != 0;
    if !anonymous || !private || fd as i64 != -1 {
        return linux_err(ENOSYS);
    }
    let mapped = handlers::sys_mmap_u64(len);
    if mapped == u64::MAX {
        linux_err(ENOMEM)
    } else {
        mapped
    }
}

fn linux_munmap(addr: u64, len: u64) -> u64 {
    let ret = handlers::sys_munmap_u64(addr, len);
    if ret == u64::MAX {
        linux_err(EINVAL)
    } else {
        0
    }
}

fn linux_arch_prctl(code: u64, addr: u64) -> u64 {
    match code {
        // The kernel does not yet persist FS base across context switches.
        // Returning success for SET_FS is enough for static smoke binaries and
        // keeps the ABI hook in one place for the real TLS implementation.
        LINUX_ARCH_SET_FS => {
            let _ = addr;
            0
        }
        LINUX_ARCH_GET_FS => linux_err(ENOSYS),
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

fn linux_uname(buf_ptr: u64) -> u64 {
    if buf_ptr == 0 {
        return linux_err(EFAULT);
    }
    unsafe {
        core::ptr::write_bytes(buf_ptr as *mut u8, 0, 65 * 6);
    }
    write_linux_uts_field(buf_ptr, 0, b"Linux");
    write_linux_uts_field(buf_ptr, 1, b"anyos");
    write_linux_uts_field(buf_ptr, 2, b"0.0-licof");
    write_linux_uts_field(buf_ptr, 3, b"licof");
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

unsafe fn write_u64(base: u64, offset: u64, value: u64) {
    *((base + offset) as *mut u64) = value;
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
