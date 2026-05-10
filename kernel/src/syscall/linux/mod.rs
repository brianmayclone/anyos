//! licof Linux x86_64 syscall dispatch.
//!
//! This is intentionally a narrow Tier-0 bridge. Unsupported syscalls return
//! `-ENOSYS` using Linux's negative-errno convention.

use super::{handlers, SyscallRegs};
use alloc::string::String;
use alloc::vec::Vec;

mod abi;
mod fs;
mod io;
mod memory;
mod path;
mod process;
mod procfs;
mod socket;

use abi::*;
use fs::*;
use io::*;
use memory::*;
use path::*;
use process::*;
use procfs::*;
use socket::*;

pub(crate) use socket::{socket_decref, socket_incref};

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
const ESRCH: i32 = 3;
const EINTR: i32 = 4;
const E2BIG: i32 = 7;
const ENOEXEC: i32 = 8;
const ECHILD: i32 = 10;
const ELOOP: i32 = 40;
const EAFNOSUPPORT: i32 = 97;
const ECONNREFUSED: i32 = 111;
const ENOTCONN: i32 = 107;
const EPROTONOSUPPORT: i32 = 93;

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
const LINUX_SYS_SELECT: u64 = 23;
const LINUX_SYS_SCHED_YIELD: u64 = 24;
const LINUX_SYS_MADVISE: u64 = 28;
const LINUX_SYS_DUP: u64 = 32;
const LINUX_SYS_DUP2: u64 = 33;
const LINUX_SYS_NANOSLEEP: u64 = 35;
const LINUX_SYS_GETPID: u64 = 39;
const LINUX_SYS_SOCKET: u64 = 41;
const LINUX_SYS_CONNECT: u64 = 42;
const LINUX_SYS_ACCEPT: u64 = 43;
const LINUX_SYS_SENDTO: u64 = 44;
const LINUX_SYS_RECVFROM: u64 = 45;
const LINUX_SYS_SENDMSG: u64 = 46;
const LINUX_SYS_RECVMSG: u64 = 47;
const LINUX_SYS_SHUTDOWN: u64 = 48;
const LINUX_SYS_BIND: u64 = 49;
const LINUX_SYS_LISTEN: u64 = 50;
const LINUX_SYS_GETSOCKNAME: u64 = 51;
const LINUX_SYS_GETPEERNAME: u64 = 52;
const LINUX_SYS_SOCKETPAIR: u64 = 53;
const LINUX_SYS_SETSOCKOPT: u64 = 54;
const LINUX_SYS_GETSOCKOPT: u64 = 55;
const LINUX_SYS_CLONE: u64 = 56;
const LINUX_SYS_FORK: u64 = 57;
const LINUX_SYS_VFORK: u64 = 58;
const LINUX_SYS_EXECVE: u64 = 59;
const LINUX_SYS_EXIT: u64 = 60;
const LINUX_SYS_WAIT4: u64 = 61;
const LINUX_SYS_KILL: u64 = 62;
const LINUX_SYS_UNAME: u64 = 63;
const LINUX_SYS_FSYNC: u64 = 74;
const LINUX_SYS_FDATASYNC: u64 = 75;
const LINUX_SYS_TRUNCATE: u64 = 76;
const LINUX_SYS_FTRUNCATE: u64 = 77;
const LINUX_SYS_GETDENTS: u64 = 78;
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
const LINUX_SYS_GETRUSAGE: u64 = 98;
const LINUX_SYS_SYSINFO: u64 = 99;
const LINUX_SYS_TIMES: u64 = 100;
const LINUX_SYS_GETUID: u64 = 102;
const LINUX_SYS_GETGID: u64 = 104;
const LINUX_SYS_SETUID: u64 = 105;
const LINUX_SYS_SETGID: u64 = 106;
const LINUX_SYS_GETEUID: u64 = 107;
const LINUX_SYS_GETEGID: u64 = 108;
const LINUX_SYS_SETPGID: u64 = 109;
const LINUX_SYS_GETPPID: u64 = 110;
const LINUX_SYS_GETPGRP: u64 = 111;
const LINUX_SYS_SETSID: u64 = 112;
const LINUX_SYS_GETGROUPS: u64 = 115;
const LINUX_SYS_SETGROUPS: u64 = 116;
const LINUX_SYS_SETRESUID: u64 = 117;
const LINUX_SYS_GETRESUID: u64 = 118;
const LINUX_SYS_SETRESGID: u64 = 119;
const LINUX_SYS_GETRESGID: u64 = 120;
const LINUX_SYS_GETPGID: u64 = 121;
const LINUX_SYS_SETFSUID: u64 = 122;
const LINUX_SYS_SETFSGID: u64 = 123;
const LINUX_SYS_GETSID: u64 = 124;
const LINUX_SYS_CAPGET: u64 = 125;
const LINUX_SYS_CAPSET: u64 = 126;
const LINUX_SYS_RT_SIGSUSPEND: u64 = 130;
const LINUX_SYS_SIGALTSTACK: u64 = 131;
const LINUX_SYS_STATFS: u64 = 137;
const LINUX_SYS_FSTATFS: u64 = 138;
const LINUX_SYS_PRCTL: u64 = 157;
const LINUX_SYS_ARCH_PRCTL: u64 = 158;
const LINUX_SYS_SETRLIMIT: u64 = 160;
const LINUX_SYS_GETTID: u64 = 186;
const LINUX_SYS_TIME: u64 = 201;
const LINUX_SYS_FUTEX: u64 = 202;
const LINUX_SYS_SET_TID_ADDRESS: u64 = 218;
const LINUX_SYS_FADVISE64: u64 = 221;
const LINUX_SYS_GETDENTS64: u64 = 217;
const LINUX_SYS_CLOCK_GETTIME: u64 = 228;
const LINUX_SYS_EXIT_GROUP: u64 = 231;
const LINUX_SYS_TGKILL: u64 = 234;
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
const LINUX_PROC_FILESYSTEMS: u8 = 1;
const LINUX_PROC_MOUNTS: u8 = 2;
const LINUX_PROC_LOGINUID: u8 = 3;
const LINUX_PROC_STATUS: u8 = 4;

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax;
    let a1 = regs.rdi;
    let a2 = regs.rsi;
    let a3 = regs.rdx;
    let a4 = regs.r10;
    let a5 = regs.r8;
    let a6 = regs.r9;

    match nr {
        LINUX_SYS_READ => linux_read(a1 as u32, a2, a3),
        LINUX_SYS_WRITE => linux_write(a1 as u32, a2, a3),
        LINUX_SYS_OPEN => linux_open(a1, a2),
        LINUX_SYS_OPENAT => linux_openat(a1, a2, a3),
        LINUX_SYS_CLOSE => anyos_u32_ret(handlers::sys_close(a1 as u32)),
        LINUX_SYS_STAT => linux_stat(a1, a2, false),
        LINUX_SYS_LSTAT => linux_stat(a1, a2, true),
        LINUX_SYS_FSTAT => linux_fstat(a1 as u32, a2),
        LINUX_SYS_POLL => linux_poll(a1, a2, a3),
        LINUX_SYS_LSEEK => linux_lseek(a1 as u32, a2, a3),
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
        LINUX_SYS_SELECT => linux_select(a1, a2, a3, a4, a5),
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
        LINUX_SYS_SCHED_YIELD => linux_sched_yield(),
        LINUX_SYS_NANOSLEEP => 0,
        LINUX_SYS_ARCH_PRCTL => linux_arch_prctl(a1, a2),
        LINUX_SYS_GETPID => handlers::sys_getpid() as u64,
        LINUX_SYS_SOCKET => linux_socket(a1, a2, a3),
        LINUX_SYS_CONNECT => linux_connect(a1 as u32, a2, a3),
        LINUX_SYS_ACCEPT => linux_accept(a1 as u32, a2, a3),
        LINUX_SYS_SENDTO => linux_sendto(a1 as u32, a2, a3, a4, a5, a6),
        LINUX_SYS_RECVFROM => linux_recvfrom(a1 as u32, a2, a3, a4, a5, a6),
        LINUX_SYS_SENDMSG => linux_sendmsg(a1 as u32, a2, a3),
        LINUX_SYS_RECVMSG => linux_recvmsg(a1 as u32, a2, a3),
        LINUX_SYS_SHUTDOWN => linux_shutdown(a1 as u32, a2),
        LINUX_SYS_BIND => linux_bind(a1 as u32, a2, a3),
        LINUX_SYS_LISTEN => linux_listen(a1 as u32, a2),
        LINUX_SYS_GETSOCKNAME => linux_getsockname(a1 as u32, a2, a3),
        LINUX_SYS_GETPEERNAME => linux_getpeername(a1 as u32, a2, a3),
        LINUX_SYS_SOCKETPAIR => linux_socketpair(a1, a2, a3, a4),
        LINUX_SYS_SETSOCKOPT => linux_setsockopt(a1 as u32, a2, a3, a4, a5),
        LINUX_SYS_GETSOCKOPT => linux_getsockopt(a1 as u32, a2, a3, a4, a5),
        LINUX_SYS_CLONE => linux_clone(regs, a1, a2, a3, a4, a5),
        LINUX_SYS_FORK => linux_fork(regs),
        LINUX_SYS_VFORK => linux_vfork(regs),
        LINUX_SYS_EXECVE => linux_execve(a1, a2, a3),
        LINUX_SYS_UNAME => linux_uname(a1),
        LINUX_SYS_WAIT4 => linux_wait4(a1 as i64, a2, a3, a4),
        LINUX_SYS_KILL => linux_kill(a1 as i64, a2),
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
        LINUX_SYS_GETRUSAGE => linux_getrusage(a1, a2),
        LINUX_SYS_SYSINFO => linux_sysinfo(a1),
        LINUX_SYS_TIMES => linux_times(a1),
        LINUX_SYS_GETUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETGID => handlers::sys_getgid() as u64,
        LINUX_SYS_SETUID => linux_set_root_or_current(a1, true),
        LINUX_SYS_SETGID => linux_set_root_or_current(a1, false),
        LINUX_SYS_GETEUID => handlers::sys_getuid() as u64,
        LINUX_SYS_GETEGID => handlers::sys_getgid() as u64,
        LINUX_SYS_SETPGID => linux_setpgid(a1, a2),
        LINUX_SYS_GETPPID => handlers::sys_getppid() as u64,
        LINUX_SYS_GETPGRP => linux_getpgid(0),
        LINUX_SYS_SETSID => linux_setsid(),
        LINUX_SYS_GETGROUPS => linux_getgroups(a1, a2),
        LINUX_SYS_SETGROUPS => linux_setgroups(a1, a2),
        LINUX_SYS_SETRESUID => linux_setres_id(a1, a2, a3, true),
        LINUX_SYS_GETRESUID => linux_getres_id(a1, a2, a3, true),
        LINUX_SYS_SETRESGID => linux_setres_id(a1, a2, a3, false),
        LINUX_SYS_GETRESGID => linux_getres_id(a1, a2, a3, false),
        LINUX_SYS_GETPGID => linux_getpgid(a1),
        LINUX_SYS_SETFSUID => linux_setfs_id(a1, true),
        LINUX_SYS_SETFSGID => linux_setfs_id(a1, false),
        LINUX_SYS_GETSID => linux_getsid(a1),
        LINUX_SYS_CAPGET => linux_capget(a1, a2),
        LINUX_SYS_CAPSET => linux_capset(a1, a2),
        LINUX_SYS_RT_SIGSUSPEND => linux_rt_sigsuspend(a1, a2),
        LINUX_SYS_SIGALTSTACK => linux_sigaltstack(a1, a2),
        LINUX_SYS_STATFS => linux_statfs(a1, a2),
        LINUX_SYS_FSTATFS => linux_fstatfs(a1 as u32, a2),
        LINUX_SYS_PRCTL => linux_prctl(a1, a2),
        LINUX_SYS_SETRLIMIT => linux_setrlimit(a1, a2),
        LINUX_SYS_GETTID => crate::task::scheduler::current_tid() as u64,
        LINUX_SYS_TIME => linux_time(a1),
        LINUX_SYS_FUTEX => linux_futex(a1, a2, a3),
        LINUX_SYS_GETDENTS => linux_getdents(a1 as u32, a2, a3),
        LINUX_SYS_GETDENTS64 => linux_getdents64(a1 as u32, a2, a3),
        LINUX_SYS_SET_TID_ADDRESS => linux_set_tid_address(a1),
        LINUX_SYS_FADVISE64 => 0,
        LINUX_SYS_CLOCK_GETTIME => linux_clock_gettime(a1, a2),
        LINUX_SYS_EXIT | LINUX_SYS_EXIT_GROUP => handlers::sys_exit(a1 as u32) as u64,
        LINUX_SYS_TGKILL => linux_tgkill(a1, a2, a3),
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
