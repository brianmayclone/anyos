//! Syscall ABI definitions and stable syscall number registry.

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
pub const SYS_UPTIME_MS: u32 = 35;
pub const SYS_SLEEP_US: u32 = 36;
pub const SYS_SET_TIME: u32 = 37;

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

// Network polling
pub const SYS_NET_POLL: u32 = 50;

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
pub const SYS_EVT_CHAN_WAIT: u32 = 70;

// Display / GPU
pub const SYS_SCREEN_SIZE: u32 = 72;

// DLL
pub const SYS_DLL_LOAD: u32 = 80;

// Filesystem (extended)
pub const SYS_MKDIR: u32 = 90;
pub const SYS_UNLINK: u32 = 91;
pub const SYS_TRUNCATE: u32 = 92;

// Mount/unmount
pub const SYS_MOUNT: u32 = 93;
pub const SYS_UMOUNT: u32 = 94;
pub const SYS_LIST_MOUNTS: u32 = 95;

// Symlinks
pub const SYS_SYMLINK: u32 = 96;
pub const SYS_READLINK: u32 = 97;
pub const SYS_LSTAT: u32 = 98;

// Filesystem (POSIX-like)
pub const SYS_RENAME: u32 = 99;

// TCP networking
pub const SYS_TCP_CONNECT: u32 = 100;
pub const SYS_TCP_SEND: u32 = 101;
pub const SYS_TCP_RECV: u32 = 102;
pub const SYS_TCP_CLOSE: u32 = 103;
pub const SYS_TCP_STATUS: u32 = 104;
pub const SYS_LSEEK: u32 = 105;
pub const SYS_FSTAT: u32 = 106;
pub const SYS_FTRUNCATE: u32 = 107;
pub const SYS_ISATTY: u32 = 108;
pub const SYS_STATFS: u32 = 109;
pub const SYS_SET_RESOLUTION: u32 = 110;
pub const SYS_LIST_RESOLUTIONS: u32 = 111;
pub const SYS_GPU_INFO: u32 = 112;

// Audio syscalls
pub const SYS_AUDIO_WRITE: u32 = 120;
pub const SYS_AUDIO_CTL: u32 = 121;

pub const SYS_TCP_RECV_AVAILABLE: u32 = 130;
pub const SYS_TCP_SHUTDOWN_WR: u32 = 131;
pub const SYS_TCP_LISTEN: u32 = 132;
pub const SYS_TCP_ACCEPT: u32 = 133;
pub const SYS_TCP_LIST: u32 = 134;
pub const SYS_GPU_HAS_ACCEL: u32 = 135;
pub const SYS_TCP_ACCEPT_NOWAIT: u32 = 136;
pub const SYS_BOOT_READY: u32 = 137;
pub const SYS_GPU_HAS_HW_CURSOR: u32 = 138;

// Shared memory
pub const SYS_SHM_CREATE: u32 = 140;
pub const SYS_SHM_MAP: u32 = 141;
pub const SYS_SHM_UNMAP: u32 = 142;
pub const SYS_SHM_DESTROY: u32 = 143;
pub const SYS_MAP_FRAMEBUFFER: u32 = 144;
pub const SYS_GPU_COMMAND: u32 = 145;
pub const SYS_INPUT_POLL: u32 = 146;
pub const SYS_REGISTER_COMPOSITOR: u32 = 147;
pub const SYS_CURSOR_TAKEOVER: u32 = 148;

// UDP networking
pub const SYS_UDP_BIND: u32 = 150;
pub const SYS_UDP_UNBIND: u32 = 151;
pub const SYS_UDP_SENDTO: u32 = 152;
pub const SYS_UDP_RECVFROM: u32 = 153;
pub const SYS_UDP_SET_OPT: u32 = 154;
pub const SYS_UDP_LIST: u32 = 155;
pub const SYS_NET_STATS: u32 = 156;
pub const SYS_PIPE_BYTES_AVAILABLE: u32 = 157;
pub const SYS_WIFI: u32 = 158;
pub const SYS_NET_PING6: u32 = 159;
pub const SYS_NET_DNS6: u32 = 160;
pub const SYS_CAPTURE_SCREEN: u32 = 161;
pub const SYS_TCP_CONNECT_V6: u32 = 163;

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

// DLIB shared page write
pub const SYS_SET_DLL_U32: u32 = 190;

// Keyboard layout
pub const SYS_KBD_GET_LAYOUT: u32 = 200;
pub const SYS_KBD_SET_LAYOUT: u32 = 201;
pub const SYS_KBD_LIST_LAYOUTS: u32 = 202;

// Random number generation
pub const SYS_RANDOM: u32 = 210;

// Capabilities query
pub const SYS_GET_CAPABILITIES: u32 = 220;

// User identity & management
pub const SYS_GETUID: u32 = 221;
pub const SYS_GETGID: u32 = 222;
pub const SYS_AUTHENTICATE: u32 = 223;
pub const SYS_CHMOD: u32 = 224;
pub const SYS_CHOWN: u32 = 225;
pub const SYS_ADDUSER: u32 = 226;
pub const SYS_DELUSER: u32 = 227;
pub const SYS_LISTUSERS: u32 = 228;
pub const SYS_ADDGROUP: u32 = 229;
pub const SYS_DELGROUP: u32 = 230;
pub const SYS_LISTGROUPS: u32 = 231;
pub const SYS_GETUSERNAME: u32 = 232;
pub const SYS_SET_IDENTITY: u32 = 233;
pub const SYS_CHPASSWD: u32 = 234;

// POSIX anonymous pipe / FD duplication
pub const SYS_PIPE2: u32 = 240;
pub const SYS_DUP: u32 = 241;
pub const SYS_DUP2: u32 = 242;
pub const SYS_FCNTL: u32 = 243;

// POSIX signals
pub const SYS_SIGACTION: u32 = 244;
pub const SYS_SIGPROCMASK: u32 = 245;
pub const SYS_SIGRETURN: u32 = 246;
pub const SYS_GETPPID: u32 = 247;

// App permissions
pub const SYS_PERM_CHECK: u32 = 250;
pub const SYS_PERM_STORE: u32 = 251;
pub const SYS_PERM_LIST: u32 = 252;
pub const SYS_PERM_DELETE: u32 = 253;
pub const SYS_PERM_PENDING_INFO: u32 = 254;
pub const SYS_REGISTER_SESSIONHOST: u32 = 255;

// VRAM direct surface / GPU DMA
pub const SYS_GPU_VRAM_SIZE: u32 = 256;
pub const SYS_VRAM_MAP: u32 = 257;
pub const SYS_GPU_REGISTER_BACKBUFFER: u32 = 258;
pub const SYS_GRANT_FRAMEBUFFER: u32 = 259;
pub const SYS_GET_CRASH_INFO: u32 = 260;
pub const SYS_REVOKE_FRAMEBUFFER: u32 = 261;

// Disk / partition management
pub const SYS_DISK_LIST: u32 = 270;
pub const SYS_DISK_PARTITIONS: u32 = 271;
pub const SYS_DISK_READ: u32 = 272;
pub const SYS_DISK_WRITE: u32 = 273;
pub const SYS_PARTITION_CREATE: u32 = 274;
pub const SYS_PARTITION_DELETE: u32 = 275;
pub const SYS_PARTITION_RESCAN: u32 = 276;
pub const SYS_DISK_EJECT: u32 = 277;

// Hostname
pub const SYS_GET_HOSTNAME: u32 = 280;
pub const SYS_SET_HOSTNAME: u32 = 281;

// Power management / debug settings
pub const SYS_SHUTDOWN: u32 = 282;
pub const SYS_SET_SERIAL_VERBOSE: u32 = 283;
pub const SYS_SYNC: u32 = 284;
pub const SYS_FSYNC: u32 = 285;

// Text-mode console I/O
pub const SYS_CON_WRITE: u32 = 290;
pub const SYS_CON_READ: u32 = 291;
pub const SYS_CON_POLL_KEY: u32 = 292;
pub const SYS_CON_GET_SIZE: u32 = 293;
pub const SYS_CON_SET_MODE: u32 = 294;
pub const SYS_CON_RESIZE: u32 = 295;

// Debug / trace (anyTrace)
pub const SYS_DEBUG_ATTACH: u32 = 300;
pub const SYS_DEBUG_DETACH: u32 = 301;
pub const SYS_DEBUG_SUSPEND: u32 = 302;
pub const SYS_DEBUG_RESUME: u32 = 303;
pub const SYS_DEBUG_GET_REGS: u32 = 304;
pub const SYS_DEBUG_SET_REGS: u32 = 305;
pub const SYS_DEBUG_READ_MEM: u32 = 306;
pub const SYS_DEBUG_WRITE_MEM: u32 = 307;
pub const SYS_DEBUG_SET_BREAKPOINT: u32 = 308;
pub const SYS_DEBUG_CLR_BREAKPOINT: u32 = 309;
pub const SYS_DEBUG_SINGLE_STEP: u32 = 310;
pub const SYS_DEBUG_GET_MEM_MAP: u32 = 311;
pub const SYS_DEBUG_WAIT_EVENT: u32 = 312;
pub const SYS_THREAD_INFO_EX: u32 = 313;
/// Detach a child process so it survives parent exit (no cascade kill).
/// Sets child's parent_tid to 0. Only the direct parent may detach.
pub const SYS_DETACH: u32 = 314;

// Platform / thermal / ACPI / I2C
pub const SYS_THERMAL_READ: u32 = 320;
pub const SYS_THERMAL_CPU: u32 = 321;
pub const SYS_ACPI_SLEEP: u32 = 322;
pub const SYS_ACPI_PERF: u32 = 323;
pub const SYS_I2C_READ: u32 = 324;
pub const SYS_I2C_WRITE: u32 = 325;
pub const SYS_I2C_DETECT: u32 = 326;

// Monitor detection
pub const SYS_MONITOR_COUNT: u32 = 327;
pub const SYS_MONITOR_INFO: u32 = 328;
pub const SYS_MONITOR_EDID: u32 = 329;
pub const SYS_MONITOR_MODES: u32 = 330;

// GPU 3D acceleration (SVGA3D)
pub const SYS_GPU_3D_SUBMIT: u32 = 512;
pub const SYS_GPU_3D_QUERY: u32 = 513;
pub const SYS_GPU_3D_SYNC: u32 = 514;
pub const SYS_GPU_3D_SURFACE_DMA: u32 = 515;
pub const SYS_GPU_3D_SURFACE_DMA_READ: u32 = 516;
pub const SYS_GPU_QUERY_TYPE: u32 = 517;
pub const SYS_GPU_3D_RESOURCE_CREATE: u32 = 518;
pub const SYS_GPU_3D_RESOURCE_DESTROY: u32 = 519;

// Hardware virtualization (VT-x / AMD-V)
pub const SYS_VM_CREATE: u32 = 600;
pub const SYS_VM_DESTROY: u32 = 601;
pub const SYS_VM_SET_MEMORY: u32 = 602;
pub const SYS_VCPU_CREATE: u32 = 603;
pub const SYS_VCPU_RUN: u32 = 604;
pub const SYS_VCPU_GET_REGS: u32 = 605;
pub const SYS_VCPU_SET_REGS: u32 = 606;
pub const SYS_VCPU_GET_SREGS: u32 = 607;
pub const SYS_VCPU_SET_SREGS: u32 = 608;
pub const SYS_VCPU_INJECT_IRQ: u32 = 609;
pub const SYS_VCPU_INJECT_EXCEPTION: u32 = 610;
pub const SYS_VCPU_INJECT_NMI: u32 = 611;
pub const SYS_VM_SET_CPUID: u32 = 612;
pub const SYS_VM_HW_INFO: u32 = 613;
pub const SYS_VM_GET_DIRTY_LOG: u32 = 614;
pub const SYS_VCPU_PAUSE: u32 = 615;
pub const SYS_VCPU_RESUME: u32 = 616;
pub const SYS_VCPU_GET_FPU: u32 = 617;
pub const SYS_VCPU_SET_FPU: u32 = 618;
pub const SYS_VCPU_GET_MP_STATE: u32 = 619;
pub const SYS_VCPU_SET_MP_STATE: u32 = 620;
pub const SYS_VCPU_TRANSLATE: u32 = 621;

// AVM (anyOS Virtual Machine) KVM-style ioctl ABI.
pub const SYS_AVM_IOCTL: u32 = 630;
pub const SYS_MMAP64: u32 = 631;
pub const SYS_MUNMAP64: u32 = 632;

// Multi-monitor display syscalls. The legacy SYS_SCREEN_SIZE / SYS_SET_RESOLUTION
// / SYS_MAP_FRAMEBUFFER continue to operate on output 0 for backwards
// compatibility; these new syscalls let the compositor (and displayd) work
// against the full advertised output set.
pub const SYS_DISPLAY_LIST: u32 = 700;
pub const SYS_DISPLAY_SET_LAYOUT: u32 = 701;
pub const SYS_DISPLAY_MAP_FB: u32 = 702;
pub const SYS_DISPLAY_FLUSH: u32 = 703;
pub const SYS_DISPLAY_POLL_EVENT: u32 = 704;

/// Register frame pushed by `syscall_fast.asm`.
///
/// The layout matches the individual GPR pushes (no pushad in 64-bit mode) plus the
/// CPU-pushed interrupt frame (RIP, CS, RFLAGS, RSP, SS - always pushed in long mode).
#[repr(C)]
pub struct SyscallRegs {
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
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}
