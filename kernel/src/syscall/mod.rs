//! System call interface — dual-path dispatch for 32-bit and 64-bit user processes.
//!
//! **INT 0x80 path** (`syscall_dispatch_32`):
//!   Used by 32-bit compatibility mode processes (libc, TCC, Doom, etc.).
//!   Convention: EAX=num, EBX=arg1, ECX=arg2, EDX=arg3, ESI=arg4, EDI=arg5.
//!   CPU zero-extends 32-bit registers to 64-bit on ring transition.
//!   All arguments are explicitly treated as u32.
//!
//! **SYSCALL path** (`syscall_dispatch_64`):
//!   Used by native 64-bit Rust processes (compositor, terminal, etc.).
//!   Convention: RAX=num, RBX=arg1, R10=arg2 (RCX clobbered), RDX=arg3, RSI=arg4, RDI=arg5.
//!   Arguments are full 64-bit values (currently truncated to u32 for handler compatibility,
//!   but the separation allows future widening without touching the 32-bit path).

pub mod handlers;
pub mod table;

// =========================================================================
// Syscall numbers (must match stdlib/src/syscall.rs)
// =========================================================================

// Process management
pub const SYS_EXIT: u32 = 1;
pub const SYS_WRITE: u32 = 2;
pub const SYS_READ: u32 = 3;
pub const SYS_OPEN: u32 = 4;
pub const SYS_CLOSE: u32 = 5;
pub const SYS_GETPID: u32 = 6;
pub const SYS_YIELD: u32 = 7;
pub const SYS_SLEEP: u32 = 8;
pub const SYS_SBRK: u32 = 9;
pub const SYS_FORK: u32 = 10;
pub const SYS_EXEC: u32 = 11;
pub const SYS_WAITPID: u32 = 12;
pub const SYS_KILL: u32 = 13;
pub const SYS_MMAP: u32 = 14;
pub const SYS_MUNMAP: u32 = 15;

// Device management
pub const SYS_DEVLIST: u32 = 16;
pub const SYS_DEVOPEN: u32 = 17;
pub const SYS_DEVCLOSE: u32 = 18;
pub const SYS_DEVREAD: u32 = 19;
pub const SYS_DEVWRITE: u32 = 20;
pub const SYS_DEVIOCTL: u32 = 21;
pub const SYS_IRQWAIT: u32 = 22;

// Filesystem
pub const SYS_READDIR: u32 = 23;
pub const SYS_STAT: u32 = 24;
pub const SYS_GETCWD: u32 = 25;
pub const SYS_CHDIR: u32 = 26;

// Process spawning
pub const SYS_SPAWN: u32 = 27;
pub const SYS_GETARGS: u32 = 28;
pub const SYS_TRY_WAITPID: u32 = 29;

// System information
pub const SYS_TIME: u32 = 30;
pub const SYS_UPTIME: u32 = 31;
pub const SYS_SYSINFO: u32 = 32;
pub const SYS_DMESG: u32 = 33;
pub const SYS_TICK_HZ: u32 = 34;

// Networking
pub const SYS_NET_CONFIG: u32 = 40;
pub const SYS_NET_PING: u32 = 41;
pub const SYS_NET_DHCP: u32 = 42;
pub const SYS_NET_DNS: u32 = 43;
pub const SYS_NET_ARP: u32 = 44;

// Pipes (named IPC)
pub const SYS_PIPE_CREATE: u32 = 45;
pub const SYS_PIPE_READ: u32 = 46;
pub const SYS_PIPE_CLOSE: u32 = 47;
pub const SYS_PIPE_WRITE: u32 = 48;
pub const SYS_PIPE_OPEN: u32 = 49;

// Filesystem (extended)
pub const SYS_MKDIR: u32 = 90;
pub const SYS_UNLINK: u32 = 91;
pub const SYS_TRUNCATE: u32 = 92;

// Filesystem (POSIX-like)
pub const SYS_LSEEK: u32 = 105;
pub const SYS_FSTAT: u32 = 106;
pub const SYS_ISATTY: u32 = 108;

// TCP networking
pub const SYS_TCP_CONNECT: u32 = 100;
pub const SYS_TCP_SEND: u32 = 101;
pub const SYS_TCP_RECV: u32 = 102;
pub const SYS_TCP_CLOSE: u32 = 103;
pub const SYS_TCP_STATUS: u32 = 104;

// DLL
pub const SYS_DLL_LOAD: u32 = 80;

// Event bus
pub const SYS_EVT_SYS_SUBSCRIBE: u32 = 60;
pub const SYS_EVT_SYS_POLL: u32 = 61;
pub const SYS_EVT_SYS_UNSUBSCRIBE: u32 = 62;
pub const SYS_EVT_CHAN_CREATE: u32 = 63;
pub const SYS_EVT_CHAN_SUBSCRIBE: u32 = 64;
pub const SYS_EVT_CHAN_EMIT: u32 = 65;
pub const SYS_EVT_CHAN_POLL: u32 = 66;
pub const SYS_EVT_CHAN_UNSUBSCRIBE: u32 = 67;
pub const SYS_EVT_CHAN_DESTROY: u32 = 68;
pub const SYS_EVT_CHAN_EMIT_TO: u32 = 69;

// Display / GPU
pub const SYS_SCREEN_SIZE: u32 = 72;
pub const SYS_SET_RESOLUTION: u32 = 110;
pub const SYS_LIST_RESOLUTIONS: u32 = 111;
pub const SYS_GPU_INFO: u32 = 112;
pub const SYS_GPU_HAS_ACCEL: u32 = 135;

// Audio syscalls
pub const SYS_AUDIO_WRITE: u32 = 120;
pub const SYS_AUDIO_CTL: u32 = 121;

// Shared memory
pub const SYS_SHM_CREATE: u32 = 140;
pub const SYS_SHM_MAP: u32 = 141;
pub const SYS_SHM_UNMAP: u32 = 142;
pub const SYS_SHM_DESTROY: u32 = 143;

// UDP networking
pub const SYS_UDP_BIND: u32 = 150;
pub const SYS_UDP_UNBIND: u32 = 151;
pub const SYS_UDP_SENDTO: u32 = 152;
pub const SYS_UDP_RECVFROM: u32 = 153;
pub const SYS_UDP_SET_OPT: u32 = 154;

// Compositor-privileged syscalls
pub const SYS_MAP_FRAMEBUFFER: u32 = 144;
pub const SYS_GPU_COMMAND: u32 = 145;
pub const SYS_INPUT_POLL: u32 = 146;
pub const SYS_REGISTER_COMPOSITOR: u32 = 147;
pub const SYS_CURSOR_TAKEOVER: u32 = 148;

// Screen capture
pub const SYS_CAPTURE_SCREEN: u32 = 161;

// Threading
pub const SYS_THREAD_CREATE: u32 = 170;
pub const SYS_SET_PRIORITY: u32 = 171;
pub const SYS_SET_CRITICAL: u32 = 172;

// Pipe listing
pub const SYS_PIPE_LIST: u32 = 180;

// Environment variables
pub const SYS_SETENV: u32 = 182;
pub const SYS_GETENV: u32 = 183;
pub const SYS_LISTENV: u32 = 184;

/// Register frame pushed by `syscall_entry.asm` / `syscall_fast.asm`.
///
/// The layout matches the individual GPR pushes (no pushad in 64-bit mode) plus the
/// CPU-pushed interrupt frame (RIP, CS, RFLAGS, RSP, SS — always pushed in long mode).
#[repr(C)]
pub struct SyscallRegs {
    // Pushed by stub (last push = lowest address = first field)
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    // CPU-pushed (INT 0x80)
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Register the `int 0x80` syscall trap gate and log readiness.
pub fn init() {
    crate::serial_println!("[OK] Syscall interface initialized (int 0x80 + SYSCALL)");
}

// =========================================================================
// Shared dispatch logic — routes syscall number to handler.
// Both 32-bit and 64-bit entry points extract args into u32 and call this.
// =========================================================================

#[inline(always)]
fn dispatch_inner(syscall_num: u32, arg1: u32, arg2: u32, arg3: u32, arg4: u32, arg5: u32) -> u32 {
    let result = match syscall_num {
        // Process management
        SYS_EXIT => handlers::sys_exit(arg1),
        SYS_WRITE => handlers::sys_write(arg1, arg2, arg3),
        SYS_READ => handlers::sys_read(arg1, arg2, arg3),
        SYS_OPEN => handlers::sys_open(arg1, arg2, arg3),
        SYS_CLOSE => handlers::sys_close(arg1),
        SYS_GETPID => handlers::sys_getpid(),
        SYS_YIELD => handlers::sys_yield(),
        SYS_SLEEP => handlers::sys_sleep(arg1),
        SYS_SBRK => handlers::sys_sbrk(arg1 as i32),
        SYS_WAITPID => handlers::sys_waitpid(arg1),
        SYS_KILL => handlers::sys_kill(arg1),
        SYS_SPAWN => handlers::sys_spawn(arg1, arg2, arg3, arg4),
        SYS_GETARGS => handlers::sys_getargs(arg1, arg2),
        SYS_TRY_WAITPID => handlers::sys_try_waitpid(arg1),

        // Device management
        SYS_DEVLIST => handlers::sys_devlist(arg1, arg2),
        SYS_DEVOPEN => handlers::sys_devopen(arg1, arg2),
        SYS_DEVCLOSE => handlers::sys_devclose(arg1),
        SYS_DEVREAD => handlers::sys_devread(arg1, arg2, arg3),
        SYS_DEVWRITE => handlers::sys_devwrite(arg1, arg2, arg3),
        SYS_DEVIOCTL => handlers::sys_devioctl(arg1, arg2, arg3),
        SYS_IRQWAIT => handlers::sys_irqwait(arg1),

        // Filesystem
        SYS_READDIR => handlers::sys_readdir(arg1, arg2, arg3),
        SYS_STAT => handlers::sys_stat(arg1, arg2),
        SYS_GETCWD => handlers::sys_getcwd(arg1, arg2),
        SYS_MKDIR => handlers::sys_mkdir(arg1),
        SYS_UNLINK => handlers::sys_unlink(arg1),
        SYS_TRUNCATE => handlers::sys_truncate(arg1),
        SYS_LSEEK => handlers::sys_lseek(arg1, arg2, arg3),
        SYS_FSTAT => handlers::sys_fstat(arg1, arg2),
        SYS_ISATTY => handlers::sys_isatty(arg1),

        // System info
        SYS_TIME => handlers::sys_time(arg1),
        SYS_UPTIME => handlers::sys_uptime(),
        SYS_SYSINFO => handlers::sys_sysinfo(arg1, arg2, arg3),
        SYS_DMESG => handlers::sys_dmesg(arg1, arg2),
        SYS_TICK_HZ => handlers::sys_tick_hz(),

        // Networking
        SYS_NET_CONFIG => handlers::sys_net_config(arg1, arg2),
        SYS_NET_PING => handlers::sys_net_ping(arg1, arg2, arg3),
        SYS_NET_DHCP => handlers::sys_net_dhcp(arg1),
        SYS_NET_DNS => handlers::sys_net_dns(arg1, arg2),
        SYS_NET_ARP => handlers::sys_net_arp(arg1, arg2),

        // TCP
        SYS_TCP_CONNECT => handlers::sys_tcp_connect(arg1),
        SYS_TCP_SEND => handlers::sys_tcp_send(arg1, arg2, arg3),
        SYS_TCP_RECV => handlers::sys_tcp_recv(arg1, arg2, arg3),
        SYS_TCP_CLOSE => handlers::sys_tcp_close(arg1),
        SYS_TCP_STATUS => handlers::sys_tcp_status(arg1),

        // UDP
        SYS_UDP_BIND => handlers::sys_udp_bind(arg1),
        SYS_UDP_UNBIND => handlers::sys_udp_unbind(arg1),
        SYS_UDP_SENDTO => handlers::sys_udp_sendto(arg1),
        SYS_UDP_RECVFROM => handlers::sys_udp_recvfrom(arg1, arg2, arg3),
        SYS_UDP_SET_OPT => handlers::sys_udp_set_opt(arg1, arg2, arg3),

        // Pipes
        SYS_PIPE_CREATE => handlers::sys_pipe_create(arg1),
        SYS_PIPE_READ => handlers::sys_pipe_read(arg1, arg2, arg3),
        SYS_PIPE_CLOSE => handlers::sys_pipe_close(arg1),
        SYS_PIPE_WRITE => handlers::sys_pipe_write(arg1, arg2, arg3),
        SYS_PIPE_OPEN => handlers::sys_pipe_open(arg1),

        // DLL
        SYS_DLL_LOAD => handlers::sys_dll_load(arg1, arg2),

        // Event bus
        SYS_EVT_SYS_SUBSCRIBE => handlers::sys_evt_sys_subscribe(arg1),
        SYS_EVT_SYS_POLL => handlers::sys_evt_sys_poll(arg1, arg2),
        SYS_EVT_SYS_UNSUBSCRIBE => handlers::sys_evt_sys_unsubscribe(arg1),
        SYS_EVT_CHAN_CREATE => handlers::sys_evt_chan_create(arg1, arg2),
        SYS_EVT_CHAN_SUBSCRIBE => handlers::sys_evt_chan_subscribe(arg1, arg2),
        SYS_EVT_CHAN_EMIT => handlers::sys_evt_chan_emit(arg1, arg2),
        SYS_EVT_CHAN_POLL => handlers::sys_evt_chan_poll(arg1, arg2, arg3),
        SYS_EVT_CHAN_UNSUBSCRIBE => handlers::sys_evt_chan_unsubscribe(arg1, arg2),
        SYS_EVT_CHAN_DESTROY => handlers::sys_evt_chan_destroy(arg1),
        SYS_EVT_CHAN_EMIT_TO => handlers::sys_evt_chan_emit_to(arg1, arg2, arg3),

        // Display / GPU
        SYS_SCREEN_SIZE => handlers::sys_screen_size(arg1),
        SYS_SET_RESOLUTION => handlers::sys_set_resolution(arg1, arg2),
        SYS_LIST_RESOLUTIONS => handlers::sys_list_resolutions(arg1, arg2),
        SYS_GPU_INFO => handlers::sys_gpu_info(arg1, arg2),
        SYS_GPU_HAS_ACCEL => handlers::sys_gpu_has_accel(),

        // Audio
        SYS_AUDIO_WRITE => handlers::sys_audio_write(arg1, arg2),
        SYS_AUDIO_CTL => handlers::sys_audio_ctl(arg1, arg2),

        // Shared memory
        SYS_SHM_CREATE => handlers::sys_shm_create(arg1),
        SYS_SHM_MAP => handlers::sys_shm_map(arg1),
        SYS_SHM_UNMAP => handlers::sys_shm_unmap(arg1),
        SYS_SHM_DESTROY => handlers::sys_shm_destroy(arg1),

        // Compositor-privileged
        SYS_MAP_FRAMEBUFFER => handlers::sys_map_framebuffer(arg1),
        SYS_GPU_COMMAND => handlers::sys_gpu_command(arg1, arg2),
        SYS_INPUT_POLL => handlers::sys_input_poll(arg1, arg2),
        SYS_REGISTER_COMPOSITOR => handlers::sys_register_compositor(),
        SYS_CURSOR_TAKEOVER => handlers::sys_cursor_takeover(),

        // Screen capture
        SYS_CAPTURE_SCREEN => handlers::sys_capture_screen(arg1, arg2, arg3),

        // Threading
        SYS_THREAD_CREATE => handlers::sys_thread_create(arg1, arg2, arg3, arg4, arg5),
        SYS_SET_PRIORITY => handlers::sys_set_priority(arg1, arg2),
        SYS_SET_CRITICAL => handlers::sys_set_critical(),

        // Pipe listing
        SYS_PIPE_LIST => handlers::sys_pipe_list(arg1, arg2),

        // Environment variables
        SYS_SETENV => handlers::sys_setenv(arg1, arg2),
        SYS_GETENV => handlers::sys_getenv(arg1, arg2, arg3),
        SYS_LISTENV => handlers::sys_listenv(arg1, arg2),

        _ => {
            crate::serial_println!("Unknown syscall: {}", syscall_num);
            u32::MAX
        }
    };

    // Post-syscall stack canary check: catch overflows before returning to user
    crate::task::scheduler::check_current_stack_canary(syscall_num);

    result
}

// =========================================================================
// 32-bit dispatch — called from syscall_entry.asm (INT 0x80)
// =========================================================================

/// Called from `syscall_entry.asm` for 32-bit compatibility mode processes.
///
/// INT 0x80 convention: EAX=num, EBX=arg1, ECX=arg2, EDX=arg3, ESI=arg4, EDI=arg5.
/// The CPU zero-extends 32-bit registers to 64-bit on the ring transition.
/// We explicitly mask to u32 to guarantee clean 32-bit values regardless of
/// any garbage the caller may have left in the upper 32 bits.
#[no_mangle]
pub extern "C" fn syscall_dispatch_32(regs: &mut SyscallRegs) -> u32 {
    let syscall_num = regs.rax as u32;
    let arg1 = regs.rbx as u32;
    let arg2 = regs.rcx as u32;
    let arg3 = regs.rdx as u32;
    let arg4 = regs.rsi as u32;
    let arg5 = regs.rdi as u32;

    dispatch_inner(syscall_num, arg1, arg2, arg3, arg4, arg5)
}

// =========================================================================
// 64-bit dispatch — called from syscall_fast.asm (SYSCALL instruction)
// =========================================================================

/// Called from `syscall_fast.asm` for native 64-bit processes.
///
/// SYSCALL convention: RAX=num, RBX=arg1, R10=arg2, RDX=arg3, RSI=arg4, RDI=arg5.
/// The assembly stub pushes R10 into the RCX slot of `SyscallRegs`, so `regs.rcx`
/// contains the caller's R10 value (arg2).
///
/// Arguments are extracted as full u64 values. Currently truncated to u32 for
/// handler compatibility (all user addresses are below 4 GiB), but this entry
/// point is the place to widen handlers to u64 in the future.
#[no_mangle]
pub extern "C" fn syscall_dispatch_64(regs: &mut SyscallRegs) -> u64 {
    let syscall_num = regs.rax as u32;
    // Full 64-bit argument extraction (R10 is in the RCX slot per syscall_fast.asm)
    let _arg1_64: u64 = regs.rbx;
    let _arg2_64: u64 = regs.rcx; // actually R10
    let _arg3_64: u64 = regs.rdx;
    let _arg4_64: u64 = regs.rsi;
    let _arg5_64: u64 = regs.rdi;

    // Truncate to u32 for existing handler signatures.
    // When handlers are widened to u64, use the _argN_64 values directly.
    let arg1 = _arg1_64 as u32;
    let arg2 = _arg2_64 as u32;
    let arg3 = _arg3_64 as u32;
    let arg4 = _arg4_64 as u32;
    let arg5 = _arg5_64 as u32;

    dispatch_inner(syscall_num, arg1, arg2, arg3, arg4, arg5) as u64
}
