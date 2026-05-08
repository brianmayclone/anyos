//! licof Linux x86_64 syscall dispatch.
//!
//! This is intentionally a narrow Tier-0 bridge. Unsupported syscalls return
//! `-ENOSYS` using Linux's negative-errno convention.

use super::{handlers, SyscallRegs};

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const ENOSYS: i32 = 38;

const LINUX_SYS_READ: u64 = 0;
const LINUX_SYS_WRITE: u64 = 1;
const LINUX_SYS_OPEN: u64 = 2;
const LINUX_SYS_CLOSE: u64 = 3;
const LINUX_SYS_MMAP: u64 = 9;
const LINUX_SYS_MUNMAP: u64 = 11;
const LINUX_SYS_BRK: u64 = 12;
const LINUX_SYS_ARCH_PRCTL: u64 = 158;
const LINUX_SYS_GETPID: u64 = 39;
const LINUX_SYS_EXIT: u64 = 60;
const LINUX_SYS_GETUID: u64 = 102;
const LINUX_SYS_GETGID: u64 = 104;
const LINUX_SYS_EXIT_GROUP: u64 = 231;
const LINUX_SYS_OPENAT: u64 = 257;

const LINUX_AT_FDCWD: i32 = -100;
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
        LINUX_SYS_BRK => linux_brk(a1),
        LINUX_SYS_MMAP => linux_mmap(a1, a2, a3, a4, a5),
        LINUX_SYS_MUNMAP => linux_munmap(a1, a2),
        LINUX_SYS_ARCH_PRCTL => linux_arch_prctl(a1, a2),
        LINUX_SYS_GETPID => handlers::sys_getpid() as u64,
        LINUX_SYS_GETUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETGID => handlers::sys_getgid() as u64,
        LINUX_SYS_EXIT | LINUX_SYS_EXIT_GROUP => handlers::sys_exit(a1 as u32) as u64,
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
