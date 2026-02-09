//! System call interface (`int 0x80`) -- dispatch, number definitions, and register layout.
//!
//! User programs invoke syscalls via `int 0x80` with the syscall number in RAX and up to
//! five arguments in RBX, RCX, RDX, RSI, RDI. The assembly stub (`syscall_entry.asm`) saves
//! registers and calls [`syscall_dispatch`], which routes to the appropriate handler.
//!
//! For 32-bit compat processes the convention is the same (EAX, EBX, ECX, EDX, ESI, EDI)
//! with registers zero-extended to 64-bit by the CPU on transition to long mode.

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

// Window manager / GUI
pub const SYS_WIN_CREATE: u32 = 50;
pub const SYS_WIN_DESTROY: u32 = 51;
pub const SYS_WIN_SET_TITLE: u32 = 52;
pub const SYS_WIN_GET_EVENT: u32 = 53;
pub const SYS_WIN_FILL_RECT: u32 = 54;
pub const SYS_WIN_DRAW_TEXT: u32 = 55;
pub const SYS_WIN_PRESENT: u32 = 56;
pub const SYS_WIN_GET_SIZE: u32 = 57;
pub const SYS_WIN_DRAW_TEXT_MONO: u32 = 58;
pub const SYS_WIN_BLIT: u32 = 59;
pub const SYS_WIN_LIST: u32 = 70;
pub const SYS_WIN_FOCUS: u32 = 71;
pub const SYS_SCREEN_SIZE: u32 = 72;
pub const SYS_SET_RESOLUTION: u32 = 110;
pub const SYS_LIST_RESOLUTIONS: u32 = 111;
pub const SYS_GPU_INFO: u32 = 112;

// Window creation flags
pub const WIN_FLAG_NOT_RESIZABLE: u32 = 0x01;
pub const WIN_FLAG_BORDERLESS: u32 = 0x02;
pub const WIN_FLAG_ALWAYS_ON_TOP: u32 = 0x04;

/// Register frame pushed by `syscall_entry.asm` before calling [`syscall_dispatch`].
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
    crate::serial_println!("[OK] Syscall interface initialized (int 0x80)");
}

/// Called from syscall_entry.asm.
///
/// INT 0x80 convention: RAX=num, RBX=arg1, RCX=arg2, RDX=arg3, RSI=arg4, RDI=arg5.
/// For 32-bit compat processes the same registers are used (zero-extended by CPU).
/// All handler args remain u32 for now; they will be widened to u64 in Phase 6.
#[no_mangle]
pub extern "C" fn syscall_dispatch(regs: &mut SyscallRegs) -> u32 {
    let syscall_num = regs.rax as u32;
    let arg1 = regs.rbx as u32;
    let arg2 = regs.rcx as u32;
    let arg3 = regs.rdx as u32;
    let arg4 = regs.rsi as u32;
    let arg5 = regs.rdi as u32;

    match syscall_num {
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

        // Window manager
        SYS_WIN_CREATE => handlers::sys_win_create(arg1, arg2, arg3, arg4, arg5),
        SYS_WIN_DESTROY => handlers::sys_win_destroy(arg1),
        SYS_WIN_SET_TITLE => handlers::sys_win_set_title(arg1, arg2, arg3),
        SYS_WIN_GET_EVENT => handlers::sys_win_get_event(arg1, arg2),
        SYS_WIN_FILL_RECT => handlers::sys_win_fill_rect(arg1, arg2),
        SYS_WIN_DRAW_TEXT => handlers::sys_win_draw_text(arg1, arg2),
        SYS_WIN_DRAW_TEXT_MONO => handlers::sys_win_draw_text_mono(arg1, arg2),
        SYS_WIN_PRESENT => handlers::sys_win_present(arg1),
        SYS_WIN_GET_SIZE => handlers::sys_win_get_size(arg1, arg2),
        SYS_WIN_BLIT => handlers::sys_win_blit(arg1, arg2),
        SYS_WIN_LIST => handlers::sys_win_list(arg1, arg2),
        SYS_WIN_FOCUS => handlers::sys_win_focus(arg1),
        SYS_SCREEN_SIZE => handlers::sys_screen_size(arg1),

        // Display / GPU
        SYS_SET_RESOLUTION => handlers::sys_set_resolution(arg1, arg2),
        SYS_LIST_RESOLUTIONS => handlers::sys_list_resolutions(arg1, arg2),
        SYS_GPU_INFO => handlers::sys_gpu_info(arg1, arg2),

        _ => {
            crate::serial_println!("Unknown syscall: {}", syscall_num);
            u32::MAX
        }
    }
}
