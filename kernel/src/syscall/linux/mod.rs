//! lxe Linux x86_64 syscall dispatch.
//!
//! This is intentionally a narrow Tier-0 bridge. Unsupported syscalls return
//! `-ENOSYS` using Linux's negative-errno convention.

use super::{handlers, SyscallRegs};
use alloc::string::String;
use alloc::vec::Vec;

mod abi;
mod fb;
mod fs;
mod io;
mod ipc;
mod memory;
mod path;
mod process;
mod procfs;
mod socket;
mod trace;

use abi::*;
use fb::*;
use fs::*;
use io::*;
use ipc::*;
use memory::*;
use path::*;
use process::*;
use procfs::*;
use socket::*;

pub(crate) use process::futex_wake_addr;
pub(crate) use socket::{socket_decref, socket_incref};

pub(crate) fn set_trace_enabled(enabled: bool) {
    trace::set_enabled(enabled);
}

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const ENOENT: i32 = 2;
const ENOSYS: i32 = 38;
const EACCES: i32 = 13;
const EAGAIN: i32 = 11;
const ENOTDIR: i32 = 20;
const EISDIR: i32 = 21;
const ENOTTY: i32 = 25;
const ENODEV: i32 = 19;
const EFBIG: i32 = 27;
const ESPIPE: i32 = 29;
const EPERM: i32 = 1;
const ESRCH: i32 = 3;
const EINTR: i32 = 4;
const E2BIG: i32 = 7;
const ENOEXEC: i32 = 8;
const ECHILD: i32 = 10;
const EIO: i32 = 5;
const EBUSY: i32 = 16;
const EEXIST: i32 = 17;
const ELOOP: i32 = 40;
const ENOMSG: i32 = 42;
const ENODATA: i32 = 61;
const EAFNOSUPPORT: i32 = 97;
const EOPNOTSUPP: i32 = 95;
const EISCONN: i32 = 106;
const ECONNREFUSED: i32 = 111;
const ENOTCONN: i32 = 107;
const EALREADY: i32 = 114;
const EINPROGRESS: i32 = 115;
const EPROTONOSUPPORT: i32 = 93;
const ETIMEDOUT: i32 = 110;

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
const LINUX_SYS_RT_SIGRETURN: u64 = 15;
const LINUX_SYS_IOCTL: u64 = 16;
const LINUX_SYS_PREAD64: u64 = 17;
const LINUX_SYS_PWRITE64: u64 = 18;
const LINUX_SYS_READV: u64 = 19;
const LINUX_SYS_WRITEV: u64 = 20;
const LINUX_SYS_ACCESS: u64 = 21;
const LINUX_SYS_PIPE: u64 = 22;
const LINUX_SYS_SELECT: u64 = 23;
const LINUX_SYS_SCHED_YIELD: u64 = 24;
const LINUX_SYS_MREMAP: u64 = 25;
const LINUX_SYS_MSYNC: u64 = 26;
const LINUX_SYS_MADVISE: u64 = 28;
const LINUX_SYS_SHMGET: u64 = 29;
const LINUX_SYS_SHMAT: u64 = 30;
const LINUX_SYS_SHMCTL: u64 = 31;
const LINUX_SYS_PAUSE: u64 = 34;
const LINUX_SYS_DUP: u64 = 32;
const LINUX_SYS_DUP2: u64 = 33;
const LINUX_SYS_NANOSLEEP: u64 = 35;
const LINUX_SYS_ALARM: u64 = 37;
const LINUX_SYS_GETPID: u64 = 39;
const LINUX_SYS_SENDFILE: u64 = 40;
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
const LINUX_SYS_SEMGET: u64 = 64;
const LINUX_SYS_SEMOP: u64 = 65;
const LINUX_SYS_SEMCTL: u64 = 66;
const LINUX_SYS_SHMDT: u64 = 67;
const LINUX_SYS_MSGGET: u64 = 68;
const LINUX_SYS_MSGSND: u64 = 69;
const LINUX_SYS_MSGRCV: u64 = 70;
const LINUX_SYS_MSGCTL: u64 = 71;
const LINUX_SYS_FSYNC: u64 = 74;
const LINUX_SYS_FDATASYNC: u64 = 75;
const LINUX_SYS_TRUNCATE: u64 = 76;
const LINUX_SYS_FTRUNCATE: u64 = 77;
const LINUX_SYS_GETDENTS: u64 = 78;
const LINUX_SYS_FCNTL: u64 = 72;
const LINUX_SYS_FLOCK: u64 = 73;
const LINUX_SYS_GETCWD: u64 = 79;
const LINUX_SYS_CHDIR: u64 = 80;
const LINUX_SYS_FCHDIR: u64 = 81;
const LINUX_SYS_RENAME: u64 = 82;
const LINUX_SYS_MKDIR: u64 = 83;
const LINUX_SYS_RMDIR: u64 = 84;
const LINUX_SYS_CREAT: u64 = 85;
const LINUX_SYS_LINK: u64 = 86;
const LINUX_SYS_UNLINK: u64 = 87;
const LINUX_SYS_SYMLINK: u64 = 88;
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
const LINUX_SYS_SETREUID: u64 = 113;
const LINUX_SYS_SETREGID: u64 = 114;
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
const LINUX_SYS_UTIME: u64 = 132;
const LINUX_SYS_STATFS: u64 = 137;
const LINUX_SYS_FSTATFS: u64 = 138;
const LINUX_SYS_GETPRIORITY: u64 = 140;
const LINUX_SYS_SETPRIORITY: u64 = 141;
const LINUX_SYS_SYNC: u64 = 162;
const LINUX_SYS_PRCTL: u64 = 157;
const LINUX_SYS_ARCH_PRCTL: u64 = 158;
const LINUX_SYS_SETRLIMIT: u64 = 160;
const LINUX_SYS_SWAPON: u64 = 167;
const LINUX_SYS_SWAPOFF: u64 = 168;
const LINUX_SYS_GETTID: u64 = 186;
const LINUX_SYS_SETXATTR: u64 = 188;
const LINUX_SYS_LSETXATTR: u64 = 189;
const LINUX_SYS_FSETXATTR: u64 = 190;
const LINUX_SYS_GETXATTR: u64 = 191;
const LINUX_SYS_LGETXATTR: u64 = 192;
const LINUX_SYS_FGETXATTR: u64 = 193;
const LINUX_SYS_LISTXATTR: u64 = 194;
const LINUX_SYS_LLISTXATTR: u64 = 195;
const LINUX_SYS_FLISTXATTR: u64 = 196;
const LINUX_SYS_REMOVEXATTR: u64 = 197;
const LINUX_SYS_LREMOVEXATTR: u64 = 198;
const LINUX_SYS_FREMOVEXATTR: u64 = 199;
const LINUX_SYS_TIME: u64 = 201;
const LINUX_SYS_FUTEX: u64 = 202;
const LINUX_SYS_SCHED_SETAFFINITY: u64 = 203;
const LINUX_SYS_SCHED_GETAFFINITY: u64 = 204;
const LINUX_SYS_SET_TID_ADDRESS: u64 = 218;
const LINUX_SYS_FADVISE64: u64 = 221;
const LINUX_SYS_GETDENTS64: u64 = 217;
const LINUX_SYS_CLOCK_GETTIME: u64 = 228;
const LINUX_SYS_CLOCK_GETRES: u64 = 229;
const LINUX_SYS_CLOCK_NANOSLEEP: u64 = 230;
const LINUX_SYS_EXIT_GROUP: u64 = 231;
const LINUX_SYS_TGKILL: u64 = 234;
const LINUX_SYS_UTIMES: u64 = 235;
const LINUX_SYS_OPENAT: u64 = 257;
const LINUX_SYS_MKDIRAT: u64 = 258;
const LINUX_SYS_FCHOWNAT: u64 = 260;
const LINUX_SYS_FUTIMESAT: u64 = 261;
const LINUX_SYS_NEWFSTATAT: u64 = 262;
const LINUX_SYS_UNLINKAT: u64 = 263;
const LINUX_SYS_RENAMEAT: u64 = 264;
const LINUX_SYS_LINKAT: u64 = 265;
const LINUX_SYS_SYMLINKAT: u64 = 266;
const LINUX_SYS_READLINKAT: u64 = 267;
const LINUX_SYS_FCHMODAT: u64 = 268;
const LINUX_SYS_FACCESSAT: u64 = 269;
const LINUX_SYS_PSELECT6: u64 = 270;
const LINUX_SYS_SET_ROBUST_LIST: u64 = 273;
const LINUX_SYS_SYNC_FILE_RANGE: u64 = 277;
const LINUX_SYS_UTIMENSAT: u64 = 280;
const LINUX_SYS_FALLOCATE: u64 = 285;
const LINUX_SYS_DUP3: u64 = 292;
const LINUX_SYS_PIPE2: u64 = 293;
const LINUX_SYS_PRLIMIT64: u64 = 302;
const LINUX_SYS_SYNCFS: u64 = 306;
const LINUX_SYS_SENDMMSG: u64 = 307;
const LINUX_SYS_RENAMEAT2: u64 = 316;
const LINUX_SYS_GETRANDOM: u64 = 318;
const LINUX_SYS_COPY_FILE_RANGE: u64 = 326;
const LINUX_SYS_STATX: u64 = 332;
const LINUX_SYS_RSEQ: u64 = 334;
const LINUX_SYS_CLONE3: u64 = 435;
const LINUX_SYS_FACCESSAT2: u64 = 439;

const LINUX_AT_FDCWD: i32 = -100;
const LINUX_AT_SYMLINK_NOFOLLOW: u64 = 0x100;
const LINUX_AT_EACCESS: u64 = 0x200;
const LINUX_AT_REMOVEDIR: u64 = 0x200;
const LINUX_AT_EMPTY_PATH: u64 = 0x1000;
const LINUX_MAP_ANONYMOUS: u64 = 0x20;
const LINUX_MAP_SHARED: u64 = 0x01;
const LINUX_MAP_PRIVATE: u64 = 0x02;
const LINUX_MAP_FIXED: u64 = 0x10;
const LINUX_PROT_READ: u64 = 0x1;
const LINUX_PROT_WRITE: u64 = 0x2;
const LINUX_PROT_EXEC: u64 = 0x4;
const LINUX_ARCH_SET_FS: u64 = 0x1002;
const LINUX_ARCH_GET_FS: u64 = 0x1003;
const LXE_ROOTFS: &str = "/System/var/lxe/rootfs";
const LINUX_PROC_FILESYSTEMS: u16 = 1;
const LINUX_PROC_MOUNTS: u16 = 2;
const LINUX_PROC_LOGINUID: u16 = 3;
const LINUX_PROC_STATUS: u16 = 4;
const LINUX_PROC_MOUNTINFO: u16 = 5;
const LINUX_PROC_FIPS_ENABLED: u16 = 6;
const LINUX_PROC_PARTITIONS: u16 = 7;
const LINUX_PROC_ROOT_DIR: u16 = 100;
const LINUX_PROC_SELF_DIR: u16 = 101;
const LINUX_PROC_PID_DIR: u16 = 102;
const LINUX_PROC_PID_STAT: u16 = 103;
const LINUX_PROC_PID_STATUS: u16 = 104;
const LINUX_PROC_PID_CMDLINE: u16 = 105;
const LINUX_PROC_PID_COMM: u16 = 106;
const LINUX_PROC_STAT: u16 = 107;
const LINUX_PROC_UPTIME: u16 = 108;
const LINUX_PROC_MEMINFO: u16 = 109;
const LINUX_PROC_CPUINFO: u16 = 110;
const LINUX_PROC_LOADAVG: u16 = 111;
const LINUX_PROC_VERSION: u16 = 112;
const LINUX_PROC_PID_STATM: u16 = 113;
const LINUX_PROC_SYS_DIR: u16 = 114;
const LINUX_PROC_SYS_CRYPTO_DIR: u16 = 115;
const LINUX_PROC_PID_FD_DIR: u16 = 116;
const LINUX_PROC_PID_FD_ENTRY: u16 = 117;

#[inline]
fn linux_i32_arg(value: u64) -> i32 {
    value as u32 as i32
}

#[inline]
fn linux_u32_arg(value: u64) -> u32 {
    value as u32
}

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let nr = regs.rax;
    let rbp_before = regs.rbp;
    let a1 = regs.rdi;
    let a2 = regs.rsi;
    let a3 = regs.rdx;
    let a4 = regs.r10;
    let a5 = regs.r8;
    let a6 = regs.r9;

    crate::task::scheduler::set_last_linux_syscall(crate::arch::hal::cpu_id(), nr as u32);

    if is_noncanonical(rbp_before) && looks_like_inline_ascii(rbp_before) {
        crate::serial_verbose_println!(
            "lxe linux regs: suspicious RBP before syscall tid={} sys={}({}) rip={:#x} rsp={:#x} rbp={:#x}",
            crate::task::scheduler::current_tid(),
            nr,
            syscall_name(nr as u32),
            regs.rip,
            regs.rsp,
            rbp_before
        );
    }

    let result = match nr {
        LINUX_SYS_READ => linux_read(a1 as u32, a2, a3),
        LINUX_SYS_WRITE => linux_write(a1 as u32, a2, a3),
        LINUX_SYS_OPEN => linux_open(a1, a2),
        LINUX_SYS_OPENAT => linux_openat(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_CLOSE => linux_close(a1),
        LINUX_SYS_STAT => linux_stat(a1, a2, false),
        LINUX_SYS_LSTAT => linux_stat(a1, a2, true),
        LINUX_SYS_FSTAT => linux_fstat(a1 as u32, a2),
        LINUX_SYS_POLL => linux_poll(a1, a2, a3),
        LINUX_SYS_LSEEK => linux_lseek(a1 as u32, a2, a3),
        LINUX_SYS_BRK => linux_brk(a1),
        LINUX_SYS_MMAP => linux_mmap(a1, a2, a3, a4, a5, a6),
        LINUX_SYS_MPROTECT => linux_mprotect(a1, a2, a3),
        LINUX_SYS_MUNMAP => linux_munmap(a1, a2),
        LINUX_SYS_MREMAP => linux_mremap(a1, a2, a3, a4, a5),
        LINUX_SYS_SHMGET => linux_shmget(a1, a2, a3),
        LINUX_SYS_SHMAT => linux_shmat(a1, a2, a3),
        LINUX_SYS_SHMCTL => linux_shmctl(a1, a2, a3),
        LINUX_SYS_RT_SIGACTION => linux_rt_sigaction(a1, a2, a3, a4),
        LINUX_SYS_RT_SIGPROCMASK => linux_rt_sigprocmask(a1, a2, a3, a4),
        LINUX_SYS_RT_SIGRETURN => handlers::linux_rt_sigreturn(regs),
        LINUX_SYS_IOCTL => linux_ioctl(a1 as u32, a2, a3),
        LINUX_SYS_PREAD64 => linux_pread64(a1 as u32, a2, a3, a4),
        LINUX_SYS_PWRITE64 => linux_pwrite64(a1 as u32, a2, a3, a4),
        LINUX_SYS_READV => linux_readv(a1 as u32, a2, a3),
        LINUX_SYS_WRITEV => linux_writev(a1 as u32, a2, a3),
        LINUX_SYS_ACCESS => linux_access(a1, a2),
        LINUX_SYS_PIPE => linux_pipe2(a1, 0),
        LINUX_SYS_PIPE2 => linux_pipe2(a1, a2),
        LINUX_SYS_SELECT => linux_select(a1, a2, a3, a4, a5),
        LINUX_SYS_PSELECT6 => linux_pselect6(a1, a2, a3, a4, a5, a6),
        LINUX_SYS_MSYNC => linux_msync(a1, a2, a3),
        LINUX_SYS_DUP => linux_dup(a1),
        LINUX_SYS_DUP2 => linux_dup2(a1, a2),
        LINUX_SYS_DUP3 => linux_dup3(a1, a2, a3),
        LINUX_SYS_MADVISE => 0,
        LINUX_SYS_SCHED_YIELD => linux_sched_yield(),
        LINUX_SYS_PAUSE => linux_err(EINTR),
        LINUX_SYS_NANOSLEEP => linux_nanosleep(a1, a2),
        LINUX_SYS_ALARM => 0,
        LINUX_SYS_ARCH_PRCTL => linux_arch_prctl(a1, a2),
        LINUX_SYS_GETPID => handlers::sys_getpid() as u64,
        LINUX_SYS_SENDFILE => linux_sendfile(a1 as u32, a2 as u32, a3, a4),
        LINUX_SYS_SOCKET => linux_socket(a1, a2, a3),
        LINUX_SYS_CONNECT => linux_connect(a1 as u32, a2, a3),
        LINUX_SYS_ACCEPT => linux_accept(a1 as u32, a2, a3),
        LINUX_SYS_SENDTO => linux_sendto(a1 as u32, a2, a3, a4, a5, a6),
        LINUX_SYS_RECVFROM => linux_recvfrom(a1 as u32, a2, a3, a4, a5, a6),
        LINUX_SYS_SENDMSG => linux_sendmsg(a1 as u32, a2, a3),
        LINUX_SYS_RECVMSG => linux_recvmsg(a1 as u32, a2, a3),
        LINUX_SYS_SENDMMSG => linux_sendmmsg(a1 as u32, a2, a3, a4),
        LINUX_SYS_SHUTDOWN => linux_shutdown(a1 as u32, a2),
        LINUX_SYS_BIND => linux_bind(a1 as u32, a2, a3),
        LINUX_SYS_LISTEN => linux_listen(a1 as u32, a2),
        LINUX_SYS_GETSOCKNAME => linux_getsockname(a1 as u32, a2, a3),
        LINUX_SYS_GETPEERNAME => linux_getpeername(a1 as u32, a2, a3),
        LINUX_SYS_SOCKETPAIR => linux_socketpair(a1, a2, a3, a4),
        LINUX_SYS_SETSOCKOPT => linux_setsockopt(a1 as u32, a2, a3, a4, a5),
        LINUX_SYS_GETSOCKOPT => linux_getsockopt(a1 as u32, a2, a3, a4, a5),
        LINUX_SYS_CLONE => linux_clone(regs, a1, a2, a3, a4, a5),
        LINUX_SYS_CLONE3 => linux_clone3(regs, a1, a2),
        LINUX_SYS_FORK => linux_fork(regs),
        LINUX_SYS_VFORK => linux_vfork(regs),
        LINUX_SYS_EXECVE => linux_execve(a1, a2, a3),
        LINUX_SYS_UNAME => linux_uname(a1),
        LINUX_SYS_WAIT4 => linux_wait4(regs.rip, linux_i32_arg(a1) as i64, a2, a3, a4),
        LINUX_SYS_KILL => linux_kill(linux_i32_arg(a1) as i64, a2),
        LINUX_SYS_SEMGET => linux_semget(a1, a2, a3),
        LINUX_SYS_SEMOP => linux_semop(a1, a2, a3),
        LINUX_SYS_SEMCTL => linux_semctl(a1, a2, a3, a4),
        LINUX_SYS_SHMDT => linux_shmdt(a1),
        LINUX_SYS_MSGGET => linux_msgget(a1, a2),
        LINUX_SYS_MSGSND => linux_msgsnd(a1, a2, a3, a4),
        LINUX_SYS_MSGRCV => linux_msgrcv(a1, a2, a3, a4, a5),
        LINUX_SYS_MSGCTL => linux_msgctl(a1, a2, a3),
        LINUX_SYS_FSYNC => linux_fsync(a1 as u32),
        LINUX_SYS_FDATASYNC => linux_fdatasync(a1 as u32),
        LINUX_SYS_TRUNCATE => linux_truncate(a1, a2),
        LINUX_SYS_FTRUNCATE => linux_ftruncate(a1 as u32, a2),
        LINUX_SYS_FCNTL => linux_fcntl(a1 as u32, a2 as u32, a3),
        LINUX_SYS_FLOCK => linux_flock(a1 as u32, a2),
        LINUX_SYS_GETCWD => linux_getcwd(a1, a2),
        LINUX_SYS_CHDIR => linux_chdir(a1),
        LINUX_SYS_FCHDIR => linux_fchdir(a1 as u32),
        LINUX_SYS_RENAME => linux_rename(a1, a2),
        LINUX_SYS_MKDIR => linux_mkdir(a1, a2),
        LINUX_SYS_RMDIR => linux_rmdir_path(a1),
        LINUX_SYS_CREAT => linux_creat(a1),
        LINUX_SYS_LINK => linux_link(a1, a2),
        LINUX_SYS_UNLINK => linux_unlink_path(a1),
        LINUX_SYS_SYMLINK => linux_symlink(a1, a2),
        LINUX_SYS_READLINK => linux_readlink(a1, a2, a3),
        LINUX_SYS_CHMOD => linux_chmod(a1, a2),
        LINUX_SYS_FCHMOD => linux_fchmod(a1 as u32, a2),
        LINUX_SYS_CHOWN | LINUX_SYS_LCHOWN => linux_chown(a1, linux_u32_arg(a2), linux_u32_arg(a3)),
        LINUX_SYS_FCHOWN => linux_fchown(a1 as u32, linux_u32_arg(a2), linux_u32_arg(a3)),
        LINUX_SYS_UMASK => 0,
        LINUX_SYS_GETTIMEOFDAY => linux_gettimeofday(a1, a2),
        LINUX_SYS_GETRLIMIT => linux_prlimit64(0, a1, 0, a2),
        LINUX_SYS_GETRUSAGE => linux_getrusage(a1, a2),
        LINUX_SYS_SYSINFO => linux_sysinfo(a1),
        LINUX_SYS_SWAPON => linux_swapon(a1, a2),
        LINUX_SYS_SWAPOFF => linux_swapoff(a1),
        LINUX_SYS_TIMES => linux_times(a1),
        LINUX_SYS_GETUID => current_linux_real_id(true) as u64,
        LINUX_SYS_GETGID => current_linux_real_id(false) as u64,
        LINUX_SYS_SETUID => linux_set_root_or_current(linux_u32_arg(a1), true),
        LINUX_SYS_SETGID => linux_set_root_or_current(linux_u32_arg(a1), false),
        LINUX_SYS_GETEUID => current_linux_id(true) as u64,
        LINUX_SYS_GETEGID => current_linux_id(false) as u64,
        LINUX_SYS_SETPGID => linux_setpgid(linux_i32_arg(a1), linux_i32_arg(a2)),
        LINUX_SYS_GETPPID => handlers::sys_getppid() as u64,
        LINUX_SYS_GETPGRP => linux_getpgid(0),
        LINUX_SYS_SETSID => linux_setsid(),
        LINUX_SYS_SETREUID => linux_setre_id(linux_u32_arg(a1), linux_u32_arg(a2), true),
        LINUX_SYS_SETREGID => linux_setre_id(linux_u32_arg(a1), linux_u32_arg(a2), false),
        LINUX_SYS_GETGROUPS => linux_getgroups(linux_i32_arg(a1), a2),
        LINUX_SYS_SETGROUPS => linux_setgroups(linux_i32_arg(a1), a2),
        LINUX_SYS_SETRESUID => linux_setres_id(
            linux_u32_arg(a1),
            linux_u32_arg(a2),
            linux_u32_arg(a3),
            true,
        ),
        LINUX_SYS_GETRESUID => linux_getres_id(a1, a2, a3, true),
        LINUX_SYS_SETRESGID => linux_setres_id(
            linux_u32_arg(a1),
            linux_u32_arg(a2),
            linux_u32_arg(a3),
            false,
        ),
        LINUX_SYS_GETRESGID => linux_getres_id(a1, a2, a3, false),
        LINUX_SYS_GETPGID => linux_getpgid(linux_i32_arg(a1)),
        LINUX_SYS_SETFSUID => linux_setfs_id(linux_u32_arg(a1), true),
        LINUX_SYS_SETFSGID => linux_setfs_id(linux_u32_arg(a1), false),
        LINUX_SYS_GETSID => linux_getsid(linux_i32_arg(a1)),
        LINUX_SYS_SETXATTR => linux_setxattr(a1, a2, a3, a4, a5, false),
        LINUX_SYS_LSETXATTR => linux_setxattr(a1, a2, a3, a4, a5, true),
        LINUX_SYS_FSETXATTR => linux_fsetxattr(a1 as u32, a2, a3, a4, a5),
        LINUX_SYS_GETXATTR => linux_getxattr(a1, a2, a3, a4, false),
        LINUX_SYS_LGETXATTR => linux_getxattr(a1, a2, a3, a4, true),
        LINUX_SYS_FGETXATTR => linux_fgetxattr(a1 as u32, a2, a3, a4),
        LINUX_SYS_LISTXATTR => linux_listxattr(a1, a2, a3, false),
        LINUX_SYS_LLISTXATTR => linux_listxattr(a1, a2, a3, true),
        LINUX_SYS_FLISTXATTR => linux_flistxattr(a1 as u32, a2, a3),
        LINUX_SYS_REMOVEXATTR => linux_removexattr(a1, a2, false),
        LINUX_SYS_LREMOVEXATTR => linux_removexattr(a1, a2, true),
        LINUX_SYS_FREMOVEXATTR => linux_fremovexattr(a1 as u32, a2),
        LINUX_SYS_CAPGET => linux_capget(a1, a2),
        LINUX_SYS_CAPSET => linux_capset(a1, a2),
        LINUX_SYS_RT_SIGSUSPEND => linux_rt_sigsuspend(a1, a2),
        LINUX_SYS_SIGALTSTACK => linux_sigaltstack(a1, a2),
        LINUX_SYS_UTIME => linux_utime(a1, a2),
        LINUX_SYS_STATFS => linux_statfs(a1, a2),
        LINUX_SYS_FSTATFS => linux_fstatfs(a1 as u32, a2),
        LINUX_SYS_GETPRIORITY => linux_getpriority(a1, linux_i32_arg(a2)),
        LINUX_SYS_SETPRIORITY => linux_setpriority(a1, linux_i32_arg(a2), linux_i32_arg(a3)),
        LINUX_SYS_SYNC => linux_sync(),
        LINUX_SYS_PRCTL => linux_prctl(a1, a2),
        LINUX_SYS_SETRLIMIT => linux_setrlimit(a1, a2),
        LINUX_SYS_GETTID => crate::task::scheduler::current_tid() as u64,
        LINUX_SYS_TIME => linux_time(a1),
        LINUX_SYS_FUTEX => linux_futex(a1, a2, a3, a4, a5, a6),
        LINUX_SYS_SCHED_SETAFFINITY => linux_sched_setaffinity(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_SCHED_GETAFFINITY => linux_sched_getaffinity(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_GETDENTS => linux_getdents(a1 as u32, a2, a3),
        LINUX_SYS_GETDENTS64 => linux_getdents64(a1 as u32, a2, a3),
        LINUX_SYS_SET_TID_ADDRESS => linux_set_tid_address(a1),
        LINUX_SYS_FADVISE64 => 0,
        LINUX_SYS_CLOCK_GETTIME => linux_clock_gettime(a1, a2),
        LINUX_SYS_CLOCK_GETRES => linux_clock_getres(a1, a2),
        LINUX_SYS_CLOCK_NANOSLEEP => linux_clock_nanosleep(a1, a2, a3, a4),
        LINUX_SYS_EXIT | LINUX_SYS_EXIT_GROUP => handlers::sys_exit(a1 as u32) as u64,
        LINUX_SYS_TGKILL => linux_tgkill(linux_i32_arg(a1), linux_i32_arg(a2), a3),
        LINUX_SYS_UTIMES => linux_futimesat(LINUX_AT_FDCWD, a1, a2),
        LINUX_SYS_MKDIRAT => linux_mkdirat(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_FCHOWNAT => {
            linux_fchownat(linux_i32_arg(a1), a2, linux_u32_arg(a3), linux_u32_arg(a4))
        }
        LINUX_SYS_FUTIMESAT => linux_futimesat(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_NEWFSTATAT => linux_newfstatat(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_UNLINKAT => linux_unlinkat(linux_i32_arg(a1), a2, a3),
        LINUX_SYS_RENAMEAT => linux_renameat(linux_i32_arg(a1), a2, linux_i32_arg(a3), a4),
        LINUX_SYS_LINKAT => linux_linkat(linux_i32_arg(a1), a2, linux_i32_arg(a3), a4, a5),
        LINUX_SYS_SYMLINKAT => linux_symlinkat(a1, linux_i32_arg(a2), a3),
        LINUX_SYS_READLINKAT => linux_readlinkat(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_FCHMODAT => linux_fchmodat(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_FACCESSAT => linux_faccessat(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_FACCESSAT2 => linux_faccessat2(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_SET_ROBUST_LIST => 0,
        LINUX_SYS_SYNC_FILE_RANGE => linux_sync_file_range(a1 as u32, a2, a3, a4),
        LINUX_SYS_FALLOCATE => linux_fallocate(a1 as u32, a2, a3, a4),
        LINUX_SYS_UTIMENSAT => linux_utimensat(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_PRLIMIT64 => linux_prlimit64(linux_i32_arg(a1), a2, a3, a4),
        LINUX_SYS_SYNCFS => linux_syncfs(a1 as u32),
        LINUX_SYS_RENAMEAT2 => {
            linux_renameat2(linux_i32_arg(a1), a2, linux_i32_arg(a3), a4, a5)
        }
        LINUX_SYS_GETRANDOM => linux_getrandom(a1, a2),
        LINUX_SYS_COPY_FILE_RANGE => linux_copy_file_range(a1 as u32, a2, a3 as u32, a4, a5, a6),
        LINUX_SYS_STATX => linux_statx(linux_i32_arg(a1), a2, a3, a4, a5),
        LINUX_SYS_RSEQ => linux_err(ENOSYS),
        _ => linux_unsupported_syscall(regs, a1, a2, a3, a4, a5, a6),
    };

    if nr != LINUX_SYS_RT_SIGRETURN && regs.rbp != rbp_before {
        crate::serial_verbose_println!(
            "lxe linux regs: RBP changed across syscall tid={} sys={}({}) rip={:#x} before={:#x} after={:#x}",
            crate::task::scheduler::current_tid(),
            nr,
            syscall_name(nr as u32),
            regs.rip,
            rbp_before,
            regs.rbp
        );
    }

    result
}

fn is_noncanonical(value: u64) -> bool {
    let sign = (value >> 47) & 1;
    let high = value >> 48;
    (sign == 0 && high != 0) || (sign == 1 && high != 0xffff)
}

fn looks_like_inline_ascii(value: u64) -> bool {
    let bytes = value.to_le_bytes();
    bytes
        .iter()
        .filter(|&&b| b == b'\n' || b == b'\r' || b == b'\t' || (0x20..=0x7e).contains(&b))
        .count()
        >= 6
}

pub(crate) fn syscall_name(num: u32) -> &'static str {
    match num as u64 {
        LINUX_SYS_READ => "read",
        LINUX_SYS_WRITE => "write",
        LINUX_SYS_OPEN => "open",
        LINUX_SYS_CLOSE => "close",
        LINUX_SYS_STAT => "stat",
        LINUX_SYS_FSTAT => "fstat",
        LINUX_SYS_LSTAT => "lstat",
        LINUX_SYS_POLL => "poll",
        LINUX_SYS_LSEEK => "lseek",
        LINUX_SYS_MMAP => "mmap",
        LINUX_SYS_MPROTECT => "mprotect",
        LINUX_SYS_MUNMAP => "munmap",
        LINUX_SYS_MREMAP => "mremap",
        LINUX_SYS_BRK => "brk",
        LINUX_SYS_RT_SIGACTION => "rt_sigaction",
        LINUX_SYS_RT_SIGPROCMASK => "rt_sigprocmask",
        LINUX_SYS_RT_SIGRETURN => "rt_sigreturn",
        LINUX_SYS_IOCTL => "ioctl",
        LINUX_SYS_PREAD64 => "pread64",
        LINUX_SYS_PWRITE64 => "pwrite64",
        LINUX_SYS_READV => "readv",
        LINUX_SYS_WRITEV => "writev",
        LINUX_SYS_ACCESS => "access",
        LINUX_SYS_PIPE => "pipe",
        LINUX_SYS_SELECT => "select",
        LINUX_SYS_DUP => "dup",
        LINUX_SYS_DUP2 => "dup2",
        LINUX_SYS_PAUSE => "pause",
        LINUX_SYS_NANOSLEEP => "nanosleep",
        LINUX_SYS_GETPID => "getpid",
        LINUX_SYS_SOCKET => "socket",
        LINUX_SYS_CONNECT => "connect",
        LINUX_SYS_ACCEPT => "accept",
        LINUX_SYS_SENDTO => "sendto",
        LINUX_SYS_RECVFROM => "recvfrom",
        LINUX_SYS_SENDMSG => "sendmsg",
        LINUX_SYS_RECVMSG => "recvmsg",
        LINUX_SYS_CLONE => "clone",
        LINUX_SYS_LINK => "link",
        LINUX_SYS_FORK => "fork",
        LINUX_SYS_VFORK => "vfork",
        LINUX_SYS_EXECVE => "execve",
        LINUX_SYS_EXIT => "exit",
        LINUX_SYS_WAIT4 => "wait4",
        LINUX_SYS_KILL => "kill",
        LINUX_SYS_UNAME => "uname",
        LINUX_SYS_FSYNC => "fsync",
        LINUX_SYS_FDATASYNC => "fdatasync",
        LINUX_SYS_SYNC => "sync",
        LINUX_SYS_SWAPON => "swapon",
        LINUX_SYS_SWAPOFF => "swapoff",
        LINUX_SYS_SYNC_FILE_RANGE => "sync_file_range",
        LINUX_SYS_FALLOCATE => "fallocate",
        LINUX_SYS_SYNCFS => "syncfs",
        LINUX_SYS_FCNTL => "fcntl",
        LINUX_SYS_GETCWD => "getcwd",
        LINUX_SYS_GETTIMEOFDAY => "gettimeofday",
        LINUX_SYS_GETUID => "getuid",
        LINUX_SYS_GETGID => "getgid",
        LINUX_SYS_GETEUID => "geteuid",
        LINUX_SYS_GETEGID => "getegid",
        LINUX_SYS_SETREUID => "setreuid",
        LINUX_SYS_SETREGID => "setregid",
        LINUX_SYS_GETGROUPS => "getgroups",
        LINUX_SYS_SETGROUPS => "setgroups",
        LINUX_SYS_SETRESUID => "setresuid",
        LINUX_SYS_SETRESGID => "setresgid",
        LINUX_SYS_GETPRIORITY => "getpriority",
        LINUX_SYS_SETPRIORITY => "setpriority",
        LINUX_SYS_GETPPID => "getppid",
        LINUX_SYS_ARCH_PRCTL => "arch_prctl",
        LINUX_SYS_GETTID => "gettid",
        LINUX_SYS_FUTEX => "futex",
        LINUX_SYS_SCHED_SETAFFINITY => "sched_setaffinity",
        LINUX_SYS_SCHED_GETAFFINITY => "sched_getaffinity",
        LINUX_SYS_GETDENTS64 => "getdents64",
        LINUX_SYS_CLOCK_GETTIME => "clock_gettime",
        LINUX_SYS_CLOCK_GETRES => "clock_getres",
        LINUX_SYS_EXIT_GROUP => "exit_group",
        LINUX_SYS_UTIMES => "utimes",
        LINUX_SYS_OPENAT => "openat",
        LINUX_SYS_FUTIMESAT => "futimesat",
        LINUX_SYS_NEWFSTATAT => "newfstatat",
        LINUX_SYS_LINKAT => "linkat",
        LINUX_SYS_PSELECT6 => "pselect6",
        LINUX_SYS_DUP3 => "dup3",
        LINUX_SYS_PIPE2 => "pipe2",
        LINUX_SYS_GETRANDOM => "getrandom",
        LINUX_SYS_STATX => "statx",
        LINUX_SYS_CLONE3 => "clone3",
        _ => "unknown-linux",
    }
}
