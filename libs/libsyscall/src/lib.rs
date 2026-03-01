//! Central syscall library for anyOS DLLs and shared libraries.
//!
//! Provides raw SYSCALL/SVC wrappers for both x86_64 and AArch64,
//! syscall number constants, and common high-level wrappers.
//!
//! All DLLs/shared libraries should depend on this crate instead of
//! implementing their own inline assembly syscall wrappers.
//!
//! ## Calling Conventions
//!
//! **x86_64:** `SYSCALL` — RAX=num, RBX=arg1, R10=arg2, RDX=arg3, RSI=arg4, RDI=arg5
//! **AArch64:** `SVC #0` — X8=num, X0=arg1, X1=arg2, X2=arg3, X3=arg4, X4=arg5

#![no_std]

use core::arch::asm;

// =========================================================================
// Syscall numbers (must match kernel/src/syscall/mod.rs)
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
pub const SYS_MMAP: u32 = 14;
pub const SYS_MUNMAP: u32 = 15;

// Filesystem
pub const SYS_READDIR: u32 = 23;
pub const SYS_STAT: u32 = 24;
pub const SYS_GETCWD: u32 = 25;
pub const SYS_MKDIR: u32 = 90;
pub const SYS_LSEEK: u32 = 105;
pub const SYS_FSTAT: u32 = 106;

// DLL
pub const SYS_DLL_LOAD: u32 = 80;

// Networking
pub const SYS_NET_DNS: u32 = 43;
pub const SYS_TCP_CONNECT: u32 = 100;
pub const SYS_TCP_SEND: u32 = 101;
pub const SYS_TCP_RECV: u32 = 102;
pub const SYS_TCP_CLOSE: u32 = 103;
pub const SYS_TCP_STATUS: u32 = 104;
pub const SYS_TCP_RECV_AVAILABLE: u32 = 130;

// Display / GPU
pub const SYS_SCREEN_SIZE: u32 = 72;
pub const SYS_GPU_HAS_ACCEL: u32 = 135;
pub const SYS_GPU_HAS_HW_CURSOR: u32 = 138;
pub const SYS_GPU_3D_QUERY: u32 = 513;
pub const SYS_GPU_3D_SUBMIT: u32 = 512;
pub const SYS_GPU_3D_SYNC: u32 = 514;
pub const SYS_GPU_3D_SURFACE_DMA: u32 = 515;
pub const SYS_GPU_3D_SURFACE_DMA_READ: u32 = 516;

// Shared memory
pub const SYS_SHM_CREATE: u32 = 140;
pub const SYS_SHM_MAP: u32 = 141;
pub const SYS_SHM_UNMAP: u32 = 142;
pub const SYS_SHM_DESTROY: u32 = 143;

// Event channels
pub const SYS_EVT_CHAN_CREATE: u32 = 63;
pub const SYS_EVT_CHAN_SUBSCRIBE: u32 = 64;
pub const SYS_EVT_CHAN_EMIT: u32 = 65;
pub const SYS_EVT_CHAN_POLL: u32 = 66;
pub const SYS_EVT_CHAN_EMIT_TO: u32 = 69;
pub const SYS_EVT_CHAN_WAIT: u32 = 70;

// System info
pub const SYS_UPTIME_MS: u32 = 35;

// Random
pub const SYS_RANDOM: u32 = 210;

// =========================================================================
// Raw syscall wrappers — x86_64
// =========================================================================

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall0(num: u32) -> u64 {
    let ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") num as u64 => ret,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall1(num: u32, a1: u64) -> u64 {
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
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall2(num: u32, a1: u64, a2: u64) -> u64 {
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
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall3(num: u32, a1: u64, a2: u64, a3: u64) -> u64 {
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
            in("rdx") a3,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall4(num: u32, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
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
            in("rdx") a3,
            in("rsi") a4,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub fn syscall5(num: u32, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
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
            in("rdx") a3,
            in("rsi") a4,
            in("rdi") a5,
            out("rcx") _,
            out("r11") _,
        );
    }
    ret
}

// =========================================================================
// Raw syscall wrappers — AArch64
// =========================================================================

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall0(num: u32) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") 0u64 => ret,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall1(num: u32, a1: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") a1 => ret,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall2(num: u32, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall3(num: u32, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall4(num: u32, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn syscall5(num: u32, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!("svc #0",
            inlateout("x0") a1 => ret,
            in("x1") a2,
            in("x2") a3,
            in("x3") a4,
            in("x4") a5,
            in("x8") num as u64,
            options(nostack),
        );
    }
    ret
}

// =========================================================================
// Common high-level wrappers
// =========================================================================

/// Exit the current process.
pub fn exit(code: u32) -> ! {
    syscall1(SYS_EXIT, code as u64);
    loop {}
}

/// Extend the process heap by `increment` bytes.
/// Returns previous break address, or `u64::MAX` on failure.
pub fn sbrk(increment: u32) -> u64 {
    syscall1(SYS_SBRK, increment as u64)
}

/// Map anonymous pages. Returns address or `u64::MAX` on failure.
pub fn mmap(size: u32) -> u64 {
    let ret = syscall1(SYS_MMAP, size as u64);
    if ret == u32::MAX as u64 { u64::MAX } else { ret }
}

/// Unmap pages previously mapped with `mmap`. Returns 0 on success.
pub fn munmap(addr: u64, size: u32) -> u64 {
    syscall2(SYS_MUNMAP, addr, size as u64)
}

/// Write bytes to a file descriptor. Returns bytes written, or `u32::MAX` on error.
pub fn write(fd: u32, buf: &[u8]) -> u32 {
    let ret = syscall3(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Read from fd into buffer. Returns bytes read, or `u32::MAX` on error.
pub fn read(fd: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Open a file by path. Returns fd on success, `u32::MAX` on error.
pub fn open(path: &str, flags: u32) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall3(SYS_OPEN, buf.as_ptr() as u64, flags as u64, 0);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Open a file from a byte slice path. Returns fd on success, `u32::MAX` on error.
pub fn open_bytes(path: &[u8]) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    let ret = syscall3(SYS_OPEN, buf.as_ptr() as u64, 0, 0);
    ret as u32
}

/// Close a file descriptor.
pub fn close(fd: u32) {
    syscall1(SYS_CLOSE, fd as u64);
}

/// Seek within an open file. Returns new position or `u32::MAX` on error.
pub fn lseek(fd: u32, offset: i32, whence: u32) -> u32 {
    let ret = syscall3(SYS_LSEEK, fd as u64, offset as i64 as u64, whence as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Get file size via fstat. Returns file size or 0 on error.
pub fn file_size(fd: u32) -> u32 {
    let mut stat_buf = [0u32; 4];
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { 0 } else { stat_buf[1] }
}

/// Get file stats. Returns 0 on success.
pub fn fstat(fd: u32, stat_buf: &mut [u32; 4]) -> u32 {
    let ret = syscall2(SYS_FSTAT, fd as u64, stat_buf.as_mut_ptr() as u64);
    ret as u32
}

/// Sleep for `ms` milliseconds.
pub fn sleep(ms: u32) {
    syscall1(SYS_SLEEP, ms as u64);
}

/// Yield the CPU to another thread.
pub fn yield_cpu() {
    syscall0(SYS_YIELD);
}

/// Get the current thread ID.
pub fn get_tid() -> u32 {
    syscall0(SYS_GETPID) as u32
}

/// Get uptime in milliseconds.
pub fn uptime_ms() -> u32 {
    syscall0(SYS_UPTIME_MS) as u32
}

/// Write to stdout (fd=1) for debug logging.
pub fn log(msg: &[u8]) {
    write(1, msg);
}

/// Load/map a shared library by path. Returns base address, or 0 on failure.
pub fn dll_load(path: &[u8]) -> u64 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    syscall2(SYS_DLL_LOAD, buf.as_ptr() as u64, len as u64)
}

/// Read directory entries.
/// Returns number of entries, or `u32::MAX` on error.
pub fn readdir(path: &[u8], buf: &mut [u8]) -> u32 {
    syscall3(SYS_READDIR, path.as_ptr() as u64, buf.as_mut_ptr() as u64, buf.len() as u64) as u32
}

/// Create a directory (str path). Returns 0 on success.
pub fn mkdir(path: &str) -> u32 {
    mkdir_bytes(path.as_bytes())
}

/// Create a directory (byte slice path, must be null-terminated or will be). Returns 0 on success.
pub fn mkdir_bytes(path: &[u8]) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path[..len]);
    buf[len] = 0;
    let ret = syscall1(SYS_MKDIR, buf.as_ptr() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Get current working directory. Returns length or `u32::MAX` on error.
pub fn getcwd(buf: &mut [u8]) -> u32 {
    syscall2(SYS_GETCWD, buf.as_mut_ptr() as u64, buf.len() as u64) as u32
}

/// Stat a file by path. Returns 0 on success.
pub fn stat(path: &str, stat_buf: &mut [u32; 7]) -> u32 {
    let mut buf = [0u8; 257];
    let len = path.len().min(256);
    buf[..len].copy_from_slice(&path.as_bytes()[..len]);
    buf[len] = 0;
    let ret = syscall2(SYS_STAT, buf.as_ptr() as u64, stat_buf.as_mut_ptr() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Fill buffer with random bytes. Returns bytes written.
pub fn random(buf: &mut [u8]) -> u32 {
    let len = buf.len().min(256);
    let ret = syscall2(SYS_RANDOM, buf.as_mut_ptr() as u64, len as u64);
    ret as u32
}

/// Query screen size.
pub fn screen_size(out_w: *mut u32, out_h: *mut u32) {
    let mut buf = [0u32; 2];
    syscall1(SYS_SCREEN_SIZE, buf.as_mut_ptr() as u64);
    unsafe {
        *out_w = buf[0];
        *out_h = buf[1];
    }
}

// ── Open flags ───────────────────────────────────────────────────────

pub const O_WRITE: u32 = 1;
pub const O_CREATE: u32 = 4;
pub const O_TRUNC: u32 = 8;
pub const SEEK_SET: u32 = 0;

// ── Shared Memory ────────────────────────────────────────────────────

pub fn shm_create(size: u32) -> u32 {
    syscall1(SYS_SHM_CREATE, size as u64) as u32
}

pub fn shm_map(shm_id: u32) -> u64 {
    syscall1(SYS_SHM_MAP, shm_id as u64)
}

pub fn shm_unmap(shm_id: u32) -> u32 {
    syscall1(SYS_SHM_UNMAP, shm_id as u64) as u32
}

pub fn shm_destroy(shm_id: u32) -> u32 {
    syscall1(SYS_SHM_DESTROY, shm_id as u64) as u32
}

// ── Event Channels ───────────────────────────────────────────────────

pub fn evt_chan_create(name_ptr: *const u8, name_len: u32) -> u32 {
    syscall2(SYS_EVT_CHAN_CREATE, name_ptr as u64, name_len as u64) as u32
}

pub fn evt_chan_subscribe(channel_id: u32, filter: u32) -> u32 {
    syscall2(SYS_EVT_CHAN_SUBSCRIBE, channel_id as u64, filter as u64) as u32
}

pub fn evt_chan_emit(channel_id: u32, event: *const [u32; 5]) {
    syscall2(SYS_EVT_CHAN_EMIT, channel_id as u64, event as u64);
}

pub fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: *mut [u32; 5]) -> bool {
    syscall3(SYS_EVT_CHAN_POLL, channel_id as u64, sub_id as u64, buf as u64) != 0
}

pub fn evt_chan_emit_to(channel_id: u32, sub_id: u32, event: *const [u32; 5]) {
    syscall3(SYS_EVT_CHAN_EMIT_TO, channel_id as u64, sub_id as u64, event as u64);
}

pub fn evt_chan_wait(channel_id: u32, sub_id: u32, timeout_ms: u32) -> u32 {
    syscall3(SYS_EVT_CHAN_WAIT, channel_id as u64, sub_id as u64, timeout_ms as u64) as u32
}

// ── Networking ───────────────────────────────────────────────────────

/// Resolve a hostname to an IPv4 address. Returns 0 on success.
pub fn dns_resolve(hostname: &str, result: &mut [u8; 4]) -> u32 {
    let mut host_buf = [0u8; 257];
    let len = hostname.len().min(256);
    host_buf[..len].copy_from_slice(&hostname.as_bytes()[..len]);
    host_buf[len] = 0;
    let ret = syscall2(SYS_NET_DNS, host_buf.as_ptr() as u64, result.as_mut_ptr() as u64);
    ret as u32
}

/// Connect to a TCP server. Returns socket id, or `u32::MAX` on error.
pub fn tcp_connect(ip: &[u8; 4], port: u16, timeout_ms: u32) -> u32 {
    #[repr(C, packed)]
    struct TcpConnectParams {
        ip: [u8; 4],
        port: u16,
        _pad: u16,
        timeout: u32,
    }
    let params = TcpConnectParams {
        ip: *ip,
        port,
        _pad: 0,
        timeout: timeout_ms,
    };
    let ret = syscall1(SYS_TCP_CONNECT, &params as *const _ as u64);
    if (ret as i64) < 0 || ret == u64::MAX { u32::MAX } else { ret as u32 }
}

/// Send data over a TCP socket. Returns bytes sent, or `u32::MAX` on error.
pub fn tcp_send(sock: u32, data: &[u8]) -> u32 {
    let ret = syscall3(SYS_TCP_SEND, sock as u64, data.as_ptr() as u64, data.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Receive data from a TCP socket. Returns bytes received, or `u32::MAX` on error.
pub fn tcp_recv(sock: u32, buf: &mut [u8]) -> u32 {
    let ret = syscall3(SYS_TCP_RECV, sock as u64, buf.as_mut_ptr() as u64, buf.len() as u64);
    if (ret as i64) < 0 { u32::MAX } else { ret as u32 }
}

/// Close a TCP socket.
pub fn tcp_close(sock: u32) {
    syscall1(SYS_TCP_CLOSE, sock as u64);
}

/// Check bytes available to read on a TCP socket.
pub fn tcp_recv_available(sock: u32) -> u32 {
    syscall1(SYS_TCP_RECV_AVAILABLE, sock as u64) as u32
}

/// Get TCP connection state.
pub fn tcp_status(sock: u32) -> u32 {
    syscall1(SYS_TCP_STATUS, sock as u64) as u32
}

// ── GPU 3D ───────────────────────────────────────────────────────────

/// Query whether 3D hardware acceleration is available.
pub fn gpu_3d_has_hw() -> bool {
    syscall1(SYS_GPU_3D_QUERY, 0) != 0
}

/// Query the 3D hardware version.
pub fn gpu_3d_hw_version() -> u32 {
    syscall1(SYS_GPU_3D_QUERY, 1) as u32
}

/// Submit raw SVGA3D command words to the GPU. Returns 0 on success.
pub fn gpu_3d_submit(words: &[u32]) -> u32 {
    if words.is_empty() { return 0; }
    syscall2(SYS_GPU_3D_SUBMIT, words.as_ptr() as u64, words.len() as u64) as u32
}

/// Wait for all pending 3D commands to complete.
pub fn gpu_3d_sync() {
    syscall1(SYS_GPU_3D_SYNC, 0);
}

/// Upload data to a GPU surface via kernel-mediated DMA. Returns 0 on success.
pub fn gpu_3d_surface_dma(sid: u32, data: &[u8], width: u32, height: u32) -> u32 {
    if data.is_empty() { return 0; }
    syscall5(
        SYS_GPU_3D_SURFACE_DMA,
        sid as u64,
        data.as_ptr() as u64,
        data.len() as u64,
        width as u64,
        height as u64,
    ) as u32
}

/// Read back data from a GPU surface via kernel-mediated DMA. Returns 0 on success.
pub fn gpu_3d_surface_dma_read(sid: u32, buf: &mut [u8], width: u32, height: u32) -> u32 {
    if buf.is_empty() { return 0; }
    syscall5(
        SYS_GPU_3D_SURFACE_DMA_READ,
        sid as u64,
        buf.as_mut_ptr() as u64,
        buf.len() as u64,
        width as u64,
        height as u64,
    ) as u32
}

// ── Serial print (for DLLs without anyos_std) ────────────────────────

/// Write bytes to stdout (fd=1).
pub fn write_bytes(buf: &[u8]) {
    syscall3(SYS_WRITE, 1, buf.as_ptr() as u64, buf.len() as u64);
}

/// Print formatted output to serial.
pub fn serial_print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    struct SerialWriter;
    impl Write for SerialWriter {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            write_bytes(s.as_bytes());
            Ok(())
        }
    }
    let _ = SerialWriter.write_fmt(args);
}

/// Print to serial output (for DLLs without anyos_std).
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        $crate::serial_print(format_args!($($arg)*));
        $crate::write_bytes(b"\n");
    }};
}
