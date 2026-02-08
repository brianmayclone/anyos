//! Raw syscall primitives — private to the crate.
//!
//! This module contains the inline-assembly `int 0x80` wrappers and
//! the syscall number constants.  Higher-level modules (process, fs,
//! net, …) build on these.

use core::arch::asm;

// =========================================================================
// Syscall numbers (must match kernel/src/syscall/mod.rs)
// =========================================================================

// Process management
pub(crate) const SYS_EXIT: u32 = 1;
pub(crate) const SYS_WRITE: u32 = 2;
pub(crate) const SYS_READ: u32 = 3;
pub(crate) const SYS_OPEN: u32 = 4;
pub(crate) const SYS_CLOSE: u32 = 5;
pub(crate) const SYS_GETPID: u32 = 6;
pub(crate) const SYS_YIELD: u32 = 7;
pub(crate) const SYS_SLEEP: u32 = 8;
pub(crate) const SYS_SBRK: u32 = 9;
pub(crate) const SYS_WAITPID: u32 = 12;
pub(crate) const SYS_KILL: u32 = 13;
pub(crate) const SYS_SPAWN: u32 = 27;
pub(crate) const SYS_GETARGS: u32 = 28;

// Filesystem
pub(crate) const SYS_READDIR: u32 = 23;
pub(crate) const SYS_STAT: u32 = 24;
pub(crate) const SYS_MKDIR: u32 = 90;
pub(crate) const SYS_UNLINK: u32 = 91;
pub(crate) const SYS_TRUNCATE: u32 = 92;

// System information
pub(crate) const SYS_TIME: u32 = 30;
pub(crate) const SYS_UPTIME: u32 = 31;
pub(crate) const SYS_SYSINFO: u32 = 32;
pub(crate) const SYS_DMESG: u32 = 33;

// Networking
pub(crate) const SYS_NET_CONFIG: u32 = 40;
pub(crate) const SYS_NET_PING: u32 = 41;
pub(crate) const SYS_NET_DHCP: u32 = 42;
pub(crate) const SYS_NET_DNS: u32 = 43;
pub(crate) const SYS_NET_ARP: u32 = 44;

// Pipes (named IPC)
pub(crate) const SYS_PIPE_CREATE: u32 = 45;
pub(crate) const SYS_PIPE_READ: u32 = 46;
pub(crate) const SYS_PIPE_CLOSE: u32 = 47;
pub(crate) const SYS_PIPE_WRITE: u32 = 48;
pub(crate) const SYS_PIPE_OPEN: u32 = 49;

// Window manager
pub(crate) const SYS_WIN_CREATE: u32 = 50;
pub(crate) const SYS_WIN_DESTROY: u32 = 51;
pub(crate) const SYS_WIN_SET_TITLE: u32 = 52;
pub(crate) const SYS_WIN_GET_EVENT: u32 = 53;
pub(crate) const SYS_WIN_FILL_RECT: u32 = 54;
pub(crate) const SYS_WIN_DRAW_TEXT: u32 = 55;
pub(crate) const SYS_WIN_PRESENT: u32 = 56;
pub(crate) const SYS_WIN_GET_SIZE: u32 = 57;
pub(crate) const SYS_WIN_DRAW_TEXT_MONO: u32 = 58;
pub(crate) const SYS_WIN_BLIT: u32 = 59;
pub(crate) const SYS_WIN_LIST: u32 = 70;
pub(crate) const SYS_WIN_FOCUS: u32 = 71;
pub(crate) const SYS_SCREEN_SIZE: u32 = 72;

// DLL
pub(crate) const SYS_DLL_LOAD: u32 = 80;

// Display / GPU
pub(crate) const SYS_SET_RESOLUTION: u32 = 110;
pub(crate) const SYS_LIST_RESOLUTIONS: u32 = 111;
pub(crate) const SYS_GPU_INFO: u32 = 112;

// Filesystem (POSIX-like)
pub(crate) const SYS_GETCWD: u32 = 25;
pub(crate) const SYS_LSEEK: u32 = 105;
pub(crate) const SYS_FSTAT: u32 = 106;
pub(crate) const SYS_ISATTY: u32 = 108;

// TCP networking
pub(crate) const SYS_TCP_CONNECT: u32 = 100;
pub(crate) const SYS_TCP_SEND: u32 = 101;
pub(crate) const SYS_TCP_RECV: u32 = 102;
pub(crate) const SYS_TCP_CLOSE: u32 = 103;
pub(crate) const SYS_TCP_STATUS: u32 = 104;

// Event bus
pub(crate) const SYS_EVT_SYS_SUBSCRIBE: u32 = 60;
pub(crate) const SYS_EVT_SYS_POLL: u32 = 61;
pub(crate) const SYS_EVT_SYS_UNSUBSCRIBE: u32 = 62;
pub(crate) const SYS_EVT_CHAN_CREATE: u32 = 63;
pub(crate) const SYS_EVT_CHAN_SUBSCRIBE: u32 = 64;
pub(crate) const SYS_EVT_CHAN_EMIT: u32 = 65;
pub(crate) const SYS_EVT_CHAN_POLL: u32 = 66;
pub(crate) const SYS_EVT_CHAN_UNSUBSCRIBE: u32 = 67;
pub(crate) const SYS_EVT_CHAN_DESTROY: u32 = 68;

// =========================================================================
// Raw syscall helpers
// =========================================================================

#[inline(always)]
pub(crate) fn syscall0(num: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!("int 0x80",
            inlateout("eax") num => ret,
            lateout("ebx") _, lateout("ecx") _, lateout("edx") _,
        );
    }
    ret
}

#[inline(always)]
pub(crate) fn syscall1(num: u32, a1: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!("int 0x80",
            inlateout("eax") num => ret,
            in("ebx") a1,
            lateout("ecx") _, lateout("edx") _,
        );
    }
    ret
}

#[inline(always)]
pub(crate) fn syscall2(num: u32, a1: u32, a2: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!("int 0x80",
            inlateout("eax") num => ret,
            in("ebx") a1, in("ecx") a2,
            lateout("edx") _,
        );
    }
    ret
}

#[inline(always)]
pub(crate) fn syscall3(num: u32, a1: u32, a2: u32, a3: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!("int 0x80",
            inlateout("eax") num => ret,
            in("ebx") a1, in("ecx") a2, in("edx") a3,
        );
    }
    ret
}

#[inline(always)]
pub(crate) fn syscall5(num: u32, a1: u32, a2: u32, a3: u32, a4: u32, a5: u32) -> u32 {
    let ret: u32;
    unsafe {
        asm!(
            "push esi",
            "push edi",
            "mov esi, {a4}",
            "mov edi, {a5}",
            "int 0x80",
            "pop edi",
            "pop esi",
            a4 = in(reg) a4,
            a5 = in(reg) a5,
            inlateout("eax") num => ret,
            in("ebx") a1, in("ecx") a2, in("edx") a3,
        );
    }
    ret
}
