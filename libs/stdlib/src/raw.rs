//! Raw syscall primitives — private to the crate.
//!
//! This module contains the inline-assembly SYSCALL wrappers and
//! the syscall number constants.  Higher-level modules (process, fs,
//! net, …) build on these.
//!
//! SYSCALL convention (matches kernel/asm/syscall_fast.asm):
//!   RAX = syscall number
//!   RBX = arg1, R10 = arg2, RDX = arg3, RSI = arg4, RDI = arg5
//!   Return value in RAX
//!   Clobbers: RCX (← user RIP), R11 (← user RFLAGS)
//!
//! NOTE: LLVM reserves RBX on x86_64 (callee-saved base register).
//! We work around this by manually saving/restoring RBX inside the asm block.

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
pub(crate) const SYS_MMAP: u32 = 14;
pub(crate) const SYS_MUNMAP: u32 = 15;
pub(crate) const SYS_KILL: u32 = 13;
pub(crate) const SYS_SPAWN: u32 = 27;
pub(crate) const SYS_GETARGS: u32 = 28;
pub(crate) const SYS_TRY_WAITPID: u32 = 29;

// Filesystem
pub(crate) const SYS_READDIR: u32 = 23;
pub(crate) const SYS_STAT: u32 = 24;
pub(crate) const SYS_MKDIR: u32 = 90;
pub(crate) const SYS_UNLINK: u32 = 91;
pub(crate) const SYS_TRUNCATE: u32 = 92;
pub(crate) const SYS_MOUNT: u32 = 93;
pub(crate) const SYS_UMOUNT: u32 = 94;
pub(crate) const SYS_LIST_MOUNTS: u32 = 95;
pub(crate) const SYS_SYMLINK: u32 = 96;
pub(crate) const SYS_READLINK: u32 = 97;
pub(crate) const SYS_LSTAT: u32 = 98;

// System information
pub(crate) const SYS_TIME: u32 = 30;
pub(crate) const SYS_UPTIME: u32 = 31;
pub(crate) const SYS_SYSINFO: u32 = 32;
pub(crate) const SYS_DMESG: u32 = 33;
pub(crate) const SYS_TICK_HZ: u32 = 34;

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
pub(crate) const SYS_SET_DLL_U32: u32 = 190;

// Audio
pub(crate) const SYS_AUDIO_WRITE: u32 = 120;
pub(crate) const SYS_AUDIO_CTL: u32 = 121;

// Display / GPU
pub(crate) const SYS_SET_RESOLUTION: u32 = 110;
pub(crate) const SYS_LIST_RESOLUTIONS: u32 = 111;
pub(crate) const SYS_GPU_INFO: u32 = 112;

// Filesystem (POSIX-like)
pub(crate) const SYS_GETCWD: u32 = 25;
pub(crate) const SYS_CHDIR: u32 = 26;
pub(crate) const SYS_LSEEK: u32 = 105;
pub(crate) const SYS_FSTAT: u32 = 106;
pub(crate) const SYS_ISATTY: u32 = 108;

// TCP networking
pub(crate) const SYS_TCP_CONNECT: u32 = 100;
pub(crate) const SYS_TCP_SEND: u32 = 101;
pub(crate) const SYS_TCP_RECV: u32 = 102;
pub(crate) const SYS_TCP_CLOSE: u32 = 103;
pub(crate) const SYS_TCP_STATUS: u32 = 104;

// Display / GPU / wallpaper
pub(crate) const SYS_GPU_HAS_ACCEL: u32 = 135;
pub(crate) const SYS_SET_WALLPAPER: u32 = 136;
pub(crate) const SYS_BOOT_READY: u32 = 137;

// Shared memory
pub(crate) const SYS_SHM_CREATE: u32 = 140;
pub(crate) const SYS_SHM_MAP: u32 = 141;
pub(crate) const SYS_SHM_UNMAP: u32 = 142;
pub(crate) const SYS_SHM_DESTROY: u32 = 143;

// Compositor-privileged
pub(crate) const SYS_MAP_FRAMEBUFFER: u32 = 144;
pub(crate) const SYS_GPU_COMMAND: u32 = 145;
pub(crate) const SYS_INPUT_POLL: u32 = 146;
pub(crate) const SYS_REGISTER_COMPOSITOR: u32 = 147;
pub(crate) const SYS_CURSOR_TAKEOVER: u32 = 148;

// Screen capture
pub(crate) const SYS_CAPTURE_SCREEN: u32 = 161;

// Threading
pub(crate) const SYS_THREAD_CREATE: u32 = 170;
pub(crate) const SYS_SET_PRIORITY: u32 = 171;
pub(crate) const SYS_SET_CRITICAL: u32 = 172;

// Device / Pipe listing
pub(crate) const SYS_DEVLIST: u32 = 16;
pub(crate) const SYS_PIPE_LIST: u32 = 180;

// Environment variables
pub(crate) const SYS_SETENV: u32 = 182;
pub(crate) const SYS_GETENV: u32 = 183;
pub(crate) const SYS_LISTENV: u32 = 184;

// UDP networking
pub(crate) const SYS_UDP_BIND: u32 = 150;
pub(crate) const SYS_UDP_UNBIND: u32 = 151;
pub(crate) const SYS_UDP_SENDTO: u32 = 152;
pub(crate) const SYS_UDP_RECVFROM: u32 = 153;
pub(crate) const SYS_UDP_SET_OPT: u32 = 154;

// Keyboard layout
pub(crate) const SYS_KBD_GET_LAYOUT: u32 = 200;
pub(crate) const SYS_KBD_SET_LAYOUT: u32 = 201;
pub(crate) const SYS_KBD_LIST_LAYOUTS: u32 = 202;

// Random number generation
pub(crate) const SYS_RANDOM: u32 = 210;

// Capabilities query
pub(crate) const SYS_GET_CAPABILITIES: u32 = 220;

// User/group management
pub(crate) const SYS_GETUID: u32 = 221;
pub(crate) const SYS_GETGID: u32 = 222;
pub(crate) const SYS_AUTHENTICATE: u32 = 223;
pub(crate) const SYS_CHMOD: u32 = 224;
pub(crate) const SYS_CHOWN: u32 = 225;
pub(crate) const SYS_ADDUSER: u32 = 226;
pub(crate) const SYS_DELUSER: u32 = 227;
pub(crate) const SYS_LISTUSERS: u32 = 228;
pub(crate) const SYS_ADDGROUP: u32 = 229;
pub(crate) const SYS_DELGROUP: u32 = 230;
pub(crate) const SYS_LISTGROUPS: u32 = 231;
pub(crate) const SYS_GETUSERNAME: u32 = 232;
pub(crate) const SYS_SET_IDENTITY: u32 = 233;

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
pub(crate) const SYS_EVT_CHAN_EMIT_TO: u32 = 69;

// =========================================================================
// Raw syscall helpers (x86-64 SYSCALL instruction)
//
// SYSCALL convention:
//   RAX = syscall number
//   RBX = arg1, R10 = arg2 (not RCX — SYSCALL clobbers RCX/R11),
//   RDX = arg3, RSI = arg4, RDI = arg5
//   Return value in RAX
//   Clobbers: RCX (← user RIP), R11 (← user RFLAGS)
//
// RBX is reserved by LLVM on x86_64, so we manually push/pop it
// inside the asm block and use a temp register to load arg1 into RBX.
// =========================================================================

#[inline(always)]
pub(crate) fn syscall0(num: u32) -> u32 {
    let ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
pub(crate) fn syscall1(num: u32, a1: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
pub(crate) fn syscall2(num: u32, a1: u64, a2: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
pub(crate) fn syscall3(num: u32, a1: u64, a2: u64, a3: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2, in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
pub(crate) fn syscall4(num: u32, a1: u64, a2: u64, a3: u64, a4: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2, in("rdx") a3,
            in("rsi") a4,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}

#[inline(always)]
pub(crate) fn syscall5(num: u32, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u32 {
    let ret: u64;
    unsafe {
        asm!(
            "push rbx",
            "mov rbx, {a1}",
            "syscall",
            "pop rbx",
            a1 = in(reg) a1,
            inlateout("rax") num as u64 => ret,
            in("r10") a2, in("rdx") a3,
            in("rsi") a4, in("rdi") a5,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret as u32
}
