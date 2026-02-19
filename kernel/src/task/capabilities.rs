//! App-manifest permission system.
//!
//! Each thread carries a `CapSet` bitmask that gates which syscalls it may invoke.
//! `.app` bundles declare capabilities in Info.conf (`capabilities=network,audio,...`).
//! CLI programs inherit a default set from their parent (capped at `CAP_DEFAULT`).
//! The kernel checks capabilities in `dispatch_inner()` before invoking any handler.

use crate::syscall;

/// Capability bitmask type — each bit represents one permission category.
pub type CapSet = u32;

// ---- Individual capability bits ----

pub const CAP_FILESYSTEM: CapSet = 1 << 0;
pub const CAP_NETWORK: CapSet    = 1 << 1;
pub const CAP_AUDIO: CapSet      = 1 << 2;
pub const CAP_DISPLAY: CapSet    = 1 << 3;
pub const CAP_DEVICE: CapSet     = 1 << 4;
pub const CAP_PROCESS: CapSet    = 1 << 5;
pub const CAP_PIPE: CapSet       = 1 << 6;
pub const CAP_SHM: CapSet        = 1 << 7;
pub const CAP_EVENT: CapSet      = 1 << 8;
pub const CAP_COMPOSITOR: CapSet = 1 << 9;
pub const CAP_SYSTEM: CapSet     = 1 << 10;
pub const CAP_DLL: CapSet        = 1 << 11;
pub const CAP_THREAD: CapSet     = 1 << 12;
pub const CAP_MANAGE_PERMS: CapSet = 1 << 13;

// ---- Predefined sets ----

/// All capabilities — for system apps (compositor, terminal, finder).
pub const CAP_ALL: CapSet = (1 << 14) - 1; // bits 0..13

/// Default for CLI programs spawned from terminal.
pub const CAP_DEFAULT: CapSet = CAP_FILESYSTEM | CAP_PROCESS | CAP_PIPE
                              | CAP_EVENT | CAP_DLL | CAP_THREAD;

/// Infrastructure capabilities every GUI app needs — granted without user consent.
pub const CAP_AUTO_GRANTED: CapSet = CAP_DLL | CAP_THREAD | CAP_SHM | CAP_EVENT | CAP_PIPE;

/// Capabilities that require explicit user consent on first launch.
pub const CAP_SENSITIVE: CapSet = CAP_FILESYSTEM | CAP_NETWORK | CAP_AUDIO | CAP_DISPLAY
                                | CAP_DEVICE | CAP_PROCESS | CAP_SYSTEM | CAP_COMPOSITOR;

/// Parse a comma-separated capability string into a bitmask.
///
/// Recognized names: `all`, `filesystem`, `network`, `audio`, `display`, `device`,
/// `process`, `pipe`, `shm`, `event`, `compositor`, `system`, `dll`, `thread`.
/// Unknown names are silently ignored.
pub fn parse_capabilities(s: &str) -> CapSet {
    let mut caps: CapSet = 0;
    for part in s.split(',') {
        let name = part.trim();
        caps |= match name {
            "all" => CAP_ALL,
            "filesystem" => CAP_FILESYSTEM,
            "network" => CAP_NETWORK,
            "audio" => CAP_AUDIO,
            "display" => CAP_DISPLAY,
            "device" => CAP_DEVICE,
            "process" => CAP_PROCESS,
            "pipe" => CAP_PIPE,
            "shm" => CAP_SHM,
            "event" => CAP_EVENT,
            "compositor" => CAP_COMPOSITOR,
            "system" => CAP_SYSTEM,
            "dll" => CAP_DLL,
            "thread" => CAP_THREAD,
            "manage_perms" => CAP_MANAGE_PERMS,
            _ => 0,
        };
    }
    caps
}

/// Return the capability bit(s) required to invoke a given syscall.
///
/// Returns `0` for always-allowed syscalls (EXIT, GETPID, YIELD, SLEEP, etc.).
/// The caller checks `(thread_caps & required) == required`.
pub fn required_cap(syscall_num: u32) -> CapSet {
    match syscall_num {
        // Always allowed — basic process lifecycle + info
        syscall::SYS_EXIT
        | syscall::SYS_GETPID
        | syscall::SYS_YIELD
        | syscall::SYS_SLEEP
        | syscall::SYS_SBRK
        | syscall::SYS_GETARGS
        | syscall::SYS_TIME
        | syscall::SYS_UPTIME
        | syscall::SYS_TICK_HZ
        | syscall::SYS_GETENV
        | syscall::SYS_KBD_GET_LAYOUT
        | syscall::SYS_KBD_LIST_LAYOUTS
        | syscall::SYS_RANDOM
        | syscall::SYS_ISATTY
        | syscall::SYS_MMAP
        | syscall::SYS_MUNMAP
        | syscall::SYS_GETUID
        | syscall::SYS_GETGID
        | syscall::SYS_AUTHENTICATE
        | syscall::SYS_LISTUSERS
        | syscall::SYS_LISTGROUPS
        | syscall::SYS_GETUSERNAME
        | syscall::SYS_SET_IDENTITY
        | syscall::SYS_GET_CAPABILITIES => 0,

        // Filesystem
        syscall::SYS_OPEN
        | syscall::SYS_READ
        | syscall::SYS_WRITE
        | syscall::SYS_CLOSE
        | syscall::SYS_READDIR
        | syscall::SYS_STAT
        | syscall::SYS_LSTAT
        | syscall::SYS_MKDIR
        | syscall::SYS_UNLINK
        | syscall::SYS_TRUNCATE
        | syscall::SYS_SYMLINK
        | syscall::SYS_READLINK
        | syscall::SYS_MOUNT
        | syscall::SYS_UMOUNT
        | syscall::SYS_LIST_MOUNTS
        | syscall::SYS_LSEEK
        | syscall::SYS_FSTAT
        | syscall::SYS_CHDIR
        | syscall::SYS_GETCWD
        | syscall::SYS_CHMOD
        | syscall::SYS_CHOWN => CAP_FILESYSTEM,

        // Networking
        syscall::SYS_NET_CONFIG
        | syscall::SYS_NET_PING
        | syscall::SYS_NET_DHCP
        | syscall::SYS_NET_DNS
        | syscall::SYS_NET_ARP
        | syscall::SYS_NET_POLL => CAP_NETWORK,

        // TCP
        syscall::SYS_TCP_CONNECT
        | syscall::SYS_TCP_SEND
        | syscall::SYS_TCP_RECV
        | syscall::SYS_TCP_CLOSE
        | syscall::SYS_TCP_STATUS
        | syscall::SYS_TCP_RECV_AVAILABLE
        | syscall::SYS_TCP_SHUTDOWN_WR => CAP_NETWORK,

        // UDP
        syscall::SYS_UDP_BIND
        | syscall::SYS_UDP_UNBIND
        | syscall::SYS_UDP_SENDTO
        | syscall::SYS_UDP_RECVFROM
        | syscall::SYS_UDP_SET_OPT => CAP_NETWORK,

        // Audio
        syscall::SYS_AUDIO_WRITE
        | syscall::SYS_AUDIO_CTL => CAP_AUDIO,

        // Display / GPU (non-compositor)
        syscall::SYS_SCREEN_SIZE
        | syscall::SYS_SET_RESOLUTION
        | syscall::SYS_LIST_RESOLUTIONS
        | syscall::SYS_GPU_INFO
        | syscall::SYS_GPU_HAS_ACCEL
        | syscall::SYS_CAPTURE_SCREEN => CAP_DISPLAY,

        // Raw devices
        syscall::SYS_DEVLIST
        | syscall::SYS_DEVOPEN
        | syscall::SYS_DEVCLOSE
        | syscall::SYS_DEVREAD
        | syscall::SYS_DEVWRITE
        | syscall::SYS_DEVIOCTL
        | syscall::SYS_IRQWAIT => CAP_DEVICE,

        // Process management
        syscall::SYS_SPAWN
        | syscall::SYS_KILL
        | syscall::SYS_WAITPID
        | syscall::SYS_TRY_WAITPID
        | syscall::SYS_SET_PRIORITY => CAP_PROCESS,

        // Pipes
        syscall::SYS_PIPE_CREATE
        | syscall::SYS_PIPE_READ
        | syscall::SYS_PIPE_WRITE
        | syscall::SYS_PIPE_CLOSE
        | syscall::SYS_PIPE_OPEN
        | syscall::SYS_PIPE_LIST => CAP_PIPE,

        // Shared memory
        syscall::SYS_SHM_CREATE
        | syscall::SYS_SHM_MAP
        | syscall::SYS_SHM_UNMAP
        | syscall::SYS_SHM_DESTROY => CAP_SHM,

        // Event bus
        syscall::SYS_EVT_SYS_SUBSCRIBE
        | syscall::SYS_EVT_SYS_POLL
        | syscall::SYS_EVT_SYS_UNSUBSCRIBE
        | syscall::SYS_EVT_CHAN_CREATE
        | syscall::SYS_EVT_CHAN_SUBSCRIBE
        | syscall::SYS_EVT_CHAN_EMIT
        | syscall::SYS_EVT_CHAN_POLL
        | syscall::SYS_EVT_CHAN_UNSUBSCRIBE
        | syscall::SYS_EVT_CHAN_DESTROY
        | syscall::SYS_EVT_CHAN_EMIT_TO => CAP_EVENT,

        // Compositor-privileged
        syscall::SYS_MAP_FRAMEBUFFER
        | syscall::SYS_GPU_COMMAND
        | syscall::SYS_INPUT_POLL
        | syscall::SYS_REGISTER_COMPOSITOR
        | syscall::SYS_CURSOR_TAKEOVER
        | syscall::SYS_BOOT_READY => CAP_COMPOSITOR,

        // System admin
        syscall::SYS_SYSINFO
        | syscall::SYS_DMESG
        | syscall::SYS_SETENV
        | syscall::SYS_LISTENV
        | syscall::SYS_KBD_SET_LAYOUT
        | syscall::SYS_SET_CRITICAL
        | syscall::SYS_SET_DLL_U32
        | syscall::SYS_ADDUSER
        | syscall::SYS_DELUSER
        | syscall::SYS_ADDGROUP
        | syscall::SYS_DELGROUP => CAP_SYSTEM,

        // DLL loading
        syscall::SYS_DLL_LOAD => CAP_DLL,

        // Thread creation
        syscall::SYS_THREAD_CREATE => CAP_THREAD,

        // App permissions — check and pending info are always allowed;
        // store, list, and delete require CAP_MANAGE_PERMS.
        syscall::SYS_PERM_CHECK
        | syscall::SYS_PERM_PENDING_INFO => 0,

        syscall::SYS_PERM_STORE
        | syscall::SYS_PERM_LIST
        | syscall::SYS_PERM_DELETE => CAP_MANAGE_PERMS,

        // Unknown syscalls — let the dispatch handle it (returns u32::MAX)
        _ => 0,
    }
}
