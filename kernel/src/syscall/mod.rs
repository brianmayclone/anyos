//! System call interface — single-path SYSCALL dispatch for 64-bit user processes.
//!
//! All user space is 64-bit; the legacy INT 0x80 path was removed together
//! with 32-bit user-space support.
//!
//! **SYSCALL path** (`syscall_dispatch_64`):
//!   Convention: RAX=num, RBX=arg1, R10=arg2 (RCX clobbered by SYSCALL),
//!   RDX=arg3, RSI=arg4, RDI=arg5.
//!   Arguments are full 64-bit values throughout the dispatch path. Handlers
//!   that take only u32 scalars (fds, counts, flags) get an `as u32` cast
//!   applied at their match arm; handlers that take user pointers receive
//!   the full u64 untruncated.

mod defs;
pub mod handlers;
pub(crate) mod linux;
pub mod table;
pub use defs::*;

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn syscall_diag_putc(b: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F8u16,
            in("al") b,
            options(nomem, nostack, preserves_flags)
        );
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn syscall_diag_puts(s: &str) {
    for &b in s.as_bytes() {
        syscall_diag_putc(b);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn syscall_diag_hex(mut n: u64) {
    let mut buf = [0u8; 16];
    let mut i = 0usize;
    if n == 0 {
        syscall_diag_putc(b'0');
        return;
    }
    while n > 0 && i < buf.len() {
        let d = (n & 0xF) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + (d - 10) };
        n >>= 4;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        syscall_diag_putc(buf[i]);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn syscall_diag_dec(mut n: u64) {
    let mut buf = [0u8; 20];
    let mut i = 0usize;
    if n == 0 {
        syscall_diag_putc(b'0');
        return;
    }
    while n > 0 && i < buf.len() {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        syscall_diag_putc(buf[i]);
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn syscall_diag_dump_return_frame(tag: &str, regs: &SyscallRegs, result: u64) {
    let cpu = crate::arch::hal::cpu_id() as u64;
    let tid = crate::task::scheduler::current_tid() as u64;
    syscall_diag_puts(tag);
    syscall_diag_puts(" cpu=");
    syscall_diag_dec(cpu);
    syscall_diag_puts(" tid=");
    syscall_diag_dec(tid);
    syscall_diag_puts(" ret=0x");
    syscall_diag_hex(result);
    syscall_diag_puts(" rip=0x");
    syscall_diag_hex(regs.rip);
    syscall_diag_puts(" cs=0x");
    syscall_diag_hex(regs.cs);
    syscall_diag_puts(" rflags=0x");
    syscall_diag_hex(regs.rflags);
    syscall_diag_puts(" rsp=0x");
    syscall_diag_hex(regs.rsp);
    syscall_diag_puts(" ss=0x");
    syscall_diag_hex(regs.ss);
    syscall_diag_putc(b'\n');
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
pub(crate) fn dispatch_inner(
    syscall_num: u32,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u32 {
    // User space now lives in the upper canonical-low half (above 4 GiB).
    // All args are carried as u64; handlers that accept only u32 scalars
    // (fds, counts, flags) get `as u32` casts at their match arm.

    // Record last syscall for crash diagnostics (lock-free, per-CPU).
    let cpu_id = crate::arch::hal::cpu_id();
    crate::task::scheduler::set_last_syscall(cpu_id, syscall_num);

    // ── Fast path for high-frequency syscalls (no capability check needed) ──
    // These 5 syscalls account for ~80% of all calls. Skipping the capability
    // lookup and match-table indirection saves 2-3 branch mispredictions.
    match syscall_num {
        SYS_UPTIME_MS => return handlers::sys_uptime_ms(),
        SYS_YIELD => return handlers::sys_yield(),
        SYS_WRITE => return handlers::sys_write(arg1 as u32, arg2, arg3 as u32),
        SYS_READ => return handlers::sys_read(arg1 as u32, arg2, arg3 as u32),
        SYS_SLEEP => return handlers::sys_sleep(arg1 as u32),
        _ => {}
    }

    // Capability permission check — deny syscalls the thread lacks permission for.
    let required = crate::task::capabilities::required_cap(syscall_num);
    if required != 0 {
        let caps = crate::task::scheduler::current_thread_capabilities();
        if caps & required != required {
            crate::serial_println!(
                "DENIED: T{} syscall {}({}) requires cap {:#x}, has {:#x}",
                crate::task::scheduler::current_tid(),
                table::syscall_name(syscall_num),
                syscall_num,
                required,
                caps
            );
            return u32::MAX;
        }
    }

    let result = match syscall_num {
        // Process management
        SYS_EXIT => handlers::sys_exit(arg1 as u32),
        SYS_WRITE => handlers::sys_write(arg1 as u32, arg2, arg3 as u32),
        SYS_READ => handlers::sys_read(arg1 as u32, arg2, arg3 as u32),
        SYS_OPEN => handlers::sys_open(arg1, arg2 as u32, arg3 as u32),
        SYS_CLOSE => handlers::sys_close(arg1 as u32),
        SYS_GETPID => handlers::sys_getpid(),
        SYS_YIELD => handlers::sys_yield(),
        SYS_SLEEP => handlers::sys_sleep(arg1 as u32),
        SYS_SLEEP_US => handlers::sys_sleep_us(arg1 as u32),
        SYS_SBRK => handlers::sys_sbrk(arg1 as i32),
        // SYS_MMAP / SYS_MUNMAP are intercepted in syscall_dispatch_64
        // (u64 ABI). The legacy u32 handlers below are kept for in-kernel
        // call sites that still go through dispatch_inner directly.
        SYS_MMAP => handlers::sys_mmap(arg1 as u32),
        SYS_MUNMAP => handlers::sys_munmap(arg1 as u32, arg2 as u32),
        SYS_WAITPID => handlers::sys_waitpid(arg1 as u32, arg2, arg3 as u32),
        SYS_KILL => handlers::sys_kill(arg1 as u32, arg2 as u32),
        SYS_SPAWN => handlers::sys_spawn(arg1, arg2 as u32, arg3, arg4 as u32),
        SYS_LXE_SPAWN => handlers::sys_lxe_spawn(arg1, arg2),
        SYS_DETACH => handlers::sys_detach(arg1 as u32),
        SYS_EXEC => handlers::sys_exec(arg1, arg2),
        SYS_GETARGS => handlers::sys_getargs(arg1, arg2 as u32),
        SYS_TRY_WAITPID => handlers::sys_try_waitpid(arg1 as u32),

        // Device management
        SYS_DEVLIST => handlers::sys_devlist(arg1, arg2 as u32),
        SYS_DEVOPEN => handlers::sys_devopen(arg1, arg2 as u32),
        SYS_DEVCLOSE => handlers::sys_devclose(arg1 as u32),
        SYS_DEVREAD => handlers::sys_devread(arg1 as u32, arg2, arg3 as u32),
        SYS_DEVWRITE => handlers::sys_devwrite(arg1 as u32, arg2, arg3 as u32),
        SYS_DEVIOCTL => handlers::sys_devioctl(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_IRQWAIT => handlers::sys_irqwait(arg1 as u32),

        // Filesystem
        SYS_READDIR => handlers::sys_readdir(arg1, arg2, arg3 as u32),
        SYS_STAT => handlers::sys_stat(arg1, arg2),
        SYS_GETCWD => handlers::sys_getcwd(arg1, arg2 as u32),
        SYS_CHDIR => handlers::sys_chdir(arg1),
        SYS_MKDIR => handlers::sys_mkdir(arg1),
        SYS_UNLINK => handlers::sys_unlink(arg1),
        SYS_TRUNCATE => handlers::sys_truncate(arg1),
        SYS_SYMLINK => handlers::sys_symlink(arg1, arg2),
        SYS_READLINK => handlers::sys_readlink(arg1, arg2, arg3 as u32),
        SYS_LSTAT => handlers::sys_lstat(arg1, arg2),
        SYS_MOUNT => handlers::sys_mount(arg1, arg2, arg3 as u32),
        SYS_UMOUNT => handlers::sys_umount(arg1),
        SYS_LIST_MOUNTS => handlers::sys_list_mounts(arg1, arg2 as u32),
        SYS_STATFS => handlers::sys_statfs(arg1, arg2 as u32, arg3),
        SYS_RENAME => handlers::sys_rename(arg1, arg2),
        SYS_LSEEK => handlers::sys_lseek(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_FSTAT => handlers::sys_fstat(arg1 as u32, arg2),
        SYS_FTRUNCATE => handlers::sys_ftruncate(arg1 as u32, arg2 as u32),
        SYS_ISATTY => handlers::sys_isatty(arg1 as u32),

        // System info
        SYS_TIME => handlers::sys_time(arg1),
        SYS_SET_TIME => handlers::sys_set_time(arg1),
        SYS_UPTIME => handlers::sys_uptime(),
        SYS_SYSINFO => handlers::sys_sysinfo(arg1 as u32, arg2, arg3 as u32),
        SYS_DMESG => handlers::sys_dmesg(arg1, arg2 as u32),
        SYS_TICK_HZ => handlers::sys_tick_hz(),
        SYS_UPTIME_MS => handlers::sys_uptime_ms(),

        // Networking
        SYS_NET_CONFIG => handlers::sys_net_config(arg1 as u32, arg2),
        SYS_NET_PING => handlers::sys_net_ping(arg1, arg2 as u32, arg3 as u32),
        SYS_NET_DHCP => handlers::sys_net_dhcp(arg1),
        SYS_NET_DNS => handlers::sys_net_dns(arg1, arg2),
        SYS_NET_ARP => handlers::sys_net_arp(arg1, arg2 as u32),

        // TCP
        SYS_TCP_CONNECT => handlers::sys_tcp_connect(arg1),
        SYS_TCP_SEND => handlers::sys_tcp_send(arg1 as u32, arg2, arg3 as u32),
        SYS_TCP_RECV => handlers::sys_tcp_recv(arg1 as u32, arg2, arg3 as u32),
        SYS_TCP_CLOSE => handlers::sys_tcp_close(arg1 as u32),
        SYS_TCP_STATUS => handlers::sys_tcp_status(arg1 as u32),
        SYS_TCP_RECV_AVAILABLE => handlers::sys_tcp_recv_available(arg1 as u32),
        SYS_TCP_SHUTDOWN_WR => handlers::sys_tcp_shutdown_wr(arg1 as u32),
        SYS_TCP_LISTEN => handlers::sys_tcp_listen(arg1 as u32, arg2 as u32),
        SYS_TCP_ACCEPT => handlers::sys_tcp_accept(arg1 as u32, arg2),
        SYS_TCP_ACCEPT_NOWAIT => handlers::sys_tcp_accept_nowait(arg1 as u32, arg2),
        SYS_TCP_LIST => handlers::sys_tcp_list(arg1, arg2 as u32),

        // Network polling
        SYS_NET_POLL => handlers::sys_net_poll(),

        // UDP
        SYS_UDP_BIND => handlers::sys_udp_bind(arg1 as u32),
        SYS_UDP_UNBIND => handlers::sys_udp_unbind(arg1 as u32),
        SYS_UDP_SENDTO => handlers::sys_udp_sendto(arg1),
        SYS_UDP_RECVFROM => handlers::sys_udp_recvfrom(arg1 as u32, arg2, arg3 as u32),
        SYS_UDP_SET_OPT => handlers::sys_udp_set_opt(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_UDP_LIST => handlers::sys_udp_list(arg1, arg2 as u32),
        SYS_NET_STATS => handlers::sys_net_stats(arg1, arg2 as u32),
        SYS_PIPE_BYTES_AVAILABLE => handlers::sys_pipe_bytes_available(arg1 as u32),
        SYS_WIFI => handlers::sys_wifi(arg1 as u32, arg2, arg3 as u32),

        // IPv6
        SYS_NET_PING6 => handlers::sys_net_ping6(arg1, arg2 as u32, arg3 as u32),
        SYS_NET_DNS6 => handlers::sys_net_dns6(arg1, arg2),
        SYS_UDP_SENDTO_V6 => handlers::sys_udp_sendto_v6(arg1),
        SYS_TCP_CONNECT_V6 => handlers::sys_tcp_connect_v6(arg1),
        SYS_UDP_RECVFROM_V6 => handlers::sys_udp_recvfrom_v6(arg1 as u32, arg2, arg3 as u32),
        SYS_TCP_ACCEPT_V6 => handlers::sys_tcp_accept_v6(arg1 as u32, arg2),

        // Pipes
        SYS_PIPE_CREATE => handlers::sys_pipe_create(arg1),
        SYS_PIPE_READ => handlers::sys_pipe_read(arg1 as u32, arg2, arg3 as u32),
        SYS_PIPE_CLOSE => handlers::sys_pipe_close(arg1 as u32),
        SYS_PIPE_WRITE => handlers::sys_pipe_write(arg1 as u32, arg2, arg3 as u32),
        SYS_PIPE_OPEN => handlers::sys_pipe_open(arg1),

        // DLL
        SYS_DLL_LOAD => handlers::sys_dll_load(arg1, arg2 as u32),

        // Event bus
        SYS_EVT_SYS_SUBSCRIBE => handlers::sys_evt_sys_subscribe(arg1 as u32),
        SYS_EVT_SYS_POLL => handlers::sys_evt_sys_poll(arg1 as u32, arg2),
        SYS_EVT_SYS_UNSUBSCRIBE => handlers::sys_evt_sys_unsubscribe(arg1 as u32),
        SYS_EVT_CHAN_CREATE => handlers::sys_evt_chan_create(arg1, arg2 as u32),
        SYS_EVT_CHAN_SUBSCRIBE => handlers::sys_evt_chan_subscribe(arg1 as u32, arg2 as u32),
        SYS_EVT_CHAN_EMIT => handlers::sys_evt_chan_emit(arg1 as u32, arg2),
        SYS_EVT_CHAN_POLL => handlers::sys_evt_chan_poll(arg1 as u32, arg2 as u32, arg3),
        SYS_EVT_CHAN_UNSUBSCRIBE => handlers::sys_evt_chan_unsubscribe(arg1 as u32, arg2 as u32),
        SYS_EVT_CHAN_DESTROY => handlers::sys_evt_chan_destroy(arg1 as u32),
        SYS_EVT_CHAN_EMIT_TO => handlers::sys_evt_chan_emit_to(arg1 as u32, arg2 as u32, arg3),
        SYS_EVT_CHAN_WAIT => handlers::sys_evt_chan_wait(arg1 as u32, arg2 as u32, arg3 as u32),

        // Display / GPU
        SYS_SCREEN_SIZE => handlers::sys_screen_size(arg1),
        SYS_SET_RESOLUTION => handlers::sys_set_resolution(arg1 as u32, arg2 as u32),
        SYS_LIST_RESOLUTIONS => handlers::sys_list_resolutions(arg1, arg2 as u32),
        SYS_GPU_INFO => handlers::sys_gpu_info(arg1, arg2 as u32),
        SYS_GPU_HAS_ACCEL => handlers::sys_gpu_has_accel(),
        SYS_GPU_HAS_HW_CURSOR => handlers::sys_gpu_has_hw_cursor(),
        SYS_BOOT_READY => handlers::sys_boot_ready(),

        // Multi-monitor display syscalls.
        SYS_DISPLAY_LIST => handlers::sys_display_list(arg1, arg2 as u32),
        SYS_DISPLAY_SET_LAYOUT => handlers::sys_display_set_layout(arg1, arg2 as u32),
        SYS_DISPLAY_MAP_FB => handlers::sys_display_map_fb(arg1 as u32, arg2),
        SYS_DISPLAY_FLUSH => handlers::sys_display_flush(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_DISPLAY_POLL_EVENT => handlers::sys_display_poll_event(),
        SYS_REGISTER_DISPLAY_OWNER => handlers::sys_register_display_owner(),
        SYS_DISPLAY_GET_ROTATION => handlers::sys_display_get_rotation(arg1 as u32),

        // Audio
        SYS_AUDIO_WRITE => handlers::sys_audio_write(arg1, arg2 as u32),
        SYS_AUDIO_CTL => handlers::sys_audio_ctl(arg1 as u32, arg2 as u32),

        // Shared memory
        SYS_SHM_CREATE => handlers::sys_shm_create(arg1 as u32),
        SYS_SHM_MAP => handlers::sys_shm_map(arg1 as u32),
        SYS_SHM_UNMAP => handlers::sys_shm_unmap(arg1 as u32),
        SYS_SHM_DESTROY => handlers::sys_shm_destroy(arg1 as u32),

        // Compositor-privileged (SYS_MAP_FRAMEBUFFER is dispatched on the
        // 64-bit path in syscall_dispatch_64 since it takes a user pointer).
        SYS_GPU_COMMAND => handlers::sys_gpu_command(arg1, arg2 as u32),
        SYS_INPUT_POLL => handlers::sys_input_poll(arg1, arg2 as u32),
        SYS_REGISTER_COMPOSITOR => handlers::sys_register_compositor(),
        SYS_CURSOR_TAKEOVER => handlers::sys_cursor_takeover(),

        // Screen capture
        SYS_CAPTURE_SCREEN => handlers::sys_capture_screen(arg1, arg2 as u32, arg3),

        // Threading
        SYS_THREAD_CREATE => {
            handlers::sys_thread_create(arg1, arg2, arg3, arg4 as u32, arg5 as u32)
        }
        SYS_SET_PRIORITY => handlers::sys_set_priority(arg1 as u32, arg2 as u32),
        SYS_SET_CRITICAL => handlers::sys_set_critical(),

        // Pipe listing
        SYS_PIPE_LIST => handlers::sys_pipe_list(arg1, arg2 as u32),

        // Environment variables
        SYS_SETENV => handlers::sys_setenv(arg1, arg2),
        SYS_GETENV => handlers::sys_getenv(arg1, arg2, arg3 as u32),
        SYS_LISTENV => handlers::sys_listenv(arg1, arg2 as u32),

        // DLIB shared page write
        SYS_SET_DLL_U32 => handlers::sys_set_dll_u32(arg1 as u32, arg2 as u32, arg3 as u32),

        // Keyboard layout
        SYS_KBD_GET_LAYOUT => handlers::sys_kbd_get_layout(),
        SYS_KBD_SET_LAYOUT => handlers::sys_kbd_set_layout(arg1 as u32),
        SYS_KBD_LIST_LAYOUTS => handlers::sys_kbd_list_layouts(arg1, arg2 as u32),

        // Random number generation
        SYS_RANDOM => handlers::sys_random(arg1, arg2 as u32),

        // Capabilities query
        SYS_GET_CAPABILITIES => handlers::sys_get_capabilities(),

        // User identity & management
        SYS_GETUID => handlers::sys_getuid(),
        SYS_GETGID => handlers::sys_getgid(),
        SYS_AUTHENTICATE => handlers::sys_authenticate(arg1, arg2),
        SYS_CHMOD => handlers::sys_chmod(arg1, arg2 as u32),
        SYS_CHOWN => handlers::sys_chown(arg1, arg2 as u32, arg3 as u32),
        SYS_ADDUSER => handlers::sys_adduser(arg1),
        SYS_DELUSER => handlers::sys_deluser(arg1 as u32),
        SYS_LISTUSERS => handlers::sys_listusers(arg1, arg2 as u32),
        SYS_ADDGROUP => handlers::sys_addgroup(arg1),
        SYS_DELGROUP => handlers::sys_delgroup(arg1 as u32),
        SYS_LISTGROUPS => handlers::sys_listgroups(arg1, arg2 as u32),
        SYS_GETUSERNAME => handlers::sys_getusername(arg1 as u32, arg2, arg3 as u32),
        SYS_SET_IDENTITY => handlers::sys_set_identity(arg1 as u32),
        SYS_CHPASSWD => handlers::sys_chpasswd(arg1),

        // POSIX anonymous pipes / FD duplication
        SYS_PIPE2 => handlers::sys_pipe2(arg1, arg2 as u32),
        SYS_DUP => handlers::sys_dup(arg1 as u32),
        SYS_DUP2 => handlers::sys_dup2(arg1 as u32, arg2 as u32),
        SYS_FCNTL => handlers::sys_fcntl(arg1 as u32, arg2 as u32, arg3 as u32),

        // POSIX signals (SYS_SIGRETURN intercepted at dispatch level, not here)
        SYS_SIGACTION => handlers::sys_sigaction(arg1 as u32, arg2),
        SYS_SIGPROCMASK => handlers::sys_sigprocmask(arg1 as u32, arg2 as u32),

        // Process identity (extended)
        SYS_GETPPID => handlers::sys_getppid(),

        // VRAM direct surface
        SYS_GPU_VRAM_SIZE => handlers::sys_gpu_vram_size(),
        SYS_VRAM_MAP => handlers::sys_vram_map(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_GPU_REGISTER_BACKBUFFER => handlers::sys_gpu_register_backbuffer(arg1, arg2 as u32),
        SYS_GRANT_FRAMEBUFFER => handlers::sys_grant_framebuffer(arg1 as u32, arg2),
        SYS_REVOKE_FRAMEBUFFER => handlers::sys_revoke_framebuffer(arg1 as u32),

        // App permissions
        SYS_PERM_CHECK => handlers::sys_perm_check(arg1, arg2 as u32),
        SYS_PERM_STORE => handlers::sys_perm_store(arg1, arg2 as u32, arg3 as u32),
        SYS_PERM_LIST => handlers::sys_perm_list(arg1, arg2 as u32),
        SYS_PERM_DELETE => handlers::sys_perm_delete(arg1),
        SYS_PERM_PENDING_INFO => handlers::sys_perm_pending_info(arg1, arg2 as u32),
        SYS_REGISTER_SESSIONHOST => handlers::sys_register_sessionhost(),

        // Crash info
        SYS_GET_CRASH_INFO => handlers::sys_get_crash_info(arg1 as u32, arg2, arg3 as u32),

        // Disk / partition management
        SYS_DISK_LIST => handlers::sys_disk_list(arg1, arg2 as u32),
        SYS_DISK_PARTITIONS => handlers::sys_disk_partitions(arg1 as u32, arg2, arg3 as u32),
        SYS_DISK_EJECT => handlers::sys_disk_eject(arg1 as u32),
        SYS_DISK_READ => {
            handlers::sys_disk_read(arg1 as u32, arg2 as u32, arg3 as u32, arg4, arg5 as u32)
        }
        SYS_DISK_WRITE => {
            handlers::sys_disk_write(arg1 as u32, arg2 as u32, arg3 as u32, arg4, arg5 as u32)
        }
        SYS_PARTITION_CREATE => handlers::sys_partition_create(arg1 as u32, arg2, arg3 as u32),
        SYS_PARTITION_DELETE => handlers::sys_partition_delete(arg1 as u32, arg2 as u32),
        SYS_PARTITION_RESCAN => handlers::sys_partition_rescan(arg1 as u32),

        // GPU 3D acceleration (SVGA3D)
        SYS_GPU_3D_SUBMIT => handlers::sys_gpu_3d_submit(arg1, arg2 as u32),
        SYS_GPU_3D_QUERY => handlers::sys_gpu_3d_query(arg1 as u32),
        SYS_GPU_3D_SYNC => handlers::sys_gpu_3d_sync(),
        SYS_GPU_3D_SURFACE_DMA => handlers::sys_gpu_3d_surface_dma(
            arg1 as u32,
            arg2,
            arg3 as u32,
            arg4 as u32,
            arg5 as u32,
        ),
        SYS_GPU_3D_SURFACE_DMA_READ => handlers::sys_gpu_3d_surface_dma_read(
            arg1 as u32,
            arg2,
            arg3 as u32,
            arg4 as u32,
            arg5 as u32,
        ),
        SYS_GPU_QUERY_TYPE => handlers::sys_gpu_query_type(arg1, arg2 as u32),
        SYS_GPU_3D_RESOURCE_CREATE => handlers::sys_gpu_3d_resource_create(
            arg1 as u32,
            arg2 as u32,
            arg3 as u32,
            arg4 as u32,
            arg5 as u32,
        ),
        SYS_GPU_3D_RESOURCE_DESTROY => handlers::sys_gpu_3d_resource_destroy(arg1 as u32),

        // Hostname
        SYS_GET_HOSTNAME => handlers::sys_get_hostname(arg1, arg2 as u32),
        SYS_SET_HOSTNAME => handlers::sys_set_hostname(arg1, arg2 as u32),

        // Power management
        SYS_SHUTDOWN => handlers::sys_shutdown(arg1 as u32),
        SYS_SYNC => handlers::sys_sync(),
        SYS_FSYNC => handlers::sys_fsync(arg1 as u32),

        // Kernel debug settings
        SYS_SET_SERIAL_VERBOSE => handlers::sys_set_serial_verbose(arg1 as u32),

        // Text-mode console I/O
        SYS_CON_WRITE => handlers::sys_con_write(arg1, arg2 as u32),
        SYS_CON_READ => handlers::sys_con_read(arg1, arg2 as u32),
        SYS_CON_POLL_KEY => handlers::sys_con_poll_key(),
        SYS_CON_GET_SIZE => handlers::sys_con_get_size(),
        SYS_CON_SET_MODE => handlers::sys_con_set_mode(arg1 as u32),
        SYS_CON_RESIZE => handlers::sys_con_resize(arg1 as u32),

        // Platform / thermal / ACPI / I²C
        #[cfg(target_arch = "x86_64")]
        SYS_THERMAL_READ => handlers::sys_thermal_read(arg1, arg2 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_THERMAL_CPU => handlers::sys_thermal_cpu(),
        #[cfg(target_arch = "x86_64")]
        SYS_ACPI_SLEEP => handlers::sys_acpi_sleep(arg1 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_ACPI_PERF => handlers::sys_acpi_perf(arg1 as u32, arg2 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_READ => handlers::sys_i2c_read(arg1 as u32, arg2 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_WRITE => handlers::sys_i2c_write(arg1 as u32, arg2 as u32, arg3 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_DETECT => handlers::sys_i2c_detect(arg1 as u32),

        // Monitor detection
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_COUNT => handlers::sys_monitor_count(),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_INFO => handlers::sys_monitor_info(arg1 as u32, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_EDID => handlers::sys_monitor_edid(arg1 as u32, arg2, arg3 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_MODES => handlers::sys_monitor_modes(arg1 as u32, arg2, arg3 as u32),

        // Debug / trace (anyTrace)
        SYS_DEBUG_ATTACH => handlers::sys_debug_attach(arg1 as u32),
        SYS_DEBUG_DETACH => handlers::sys_debug_detach(arg1 as u32),
        SYS_DEBUG_SUSPEND => handlers::sys_debug_suspend(arg1 as u32),
        SYS_DEBUG_RESUME => handlers::sys_debug_resume(arg1 as u32),
        SYS_DEBUG_GET_REGS => handlers::sys_debug_get_regs(arg1 as u32, arg2 as u32, arg3 as u32),
        SYS_DEBUG_SET_REGS => handlers::sys_debug_set_regs(arg1 as u32, arg2, arg3 as u32),
        SYS_DEBUG_READ_MEM => {
            handlers::sys_debug_read_mem(arg1 as u32, arg2 as u32, arg3 as u32, arg4)
        }
        SYS_DEBUG_WRITE_MEM => {
            handlers::sys_debug_write_mem(arg1 as u32, arg2 as u32, arg3 as u32, arg4)
        }
        SYS_DEBUG_SET_BREAKPOINT => handlers::sys_debug_set_breakpoint(arg1 as u32, arg2 as u32),
        SYS_DEBUG_CLR_BREAKPOINT => handlers::sys_debug_clr_breakpoint(arg1 as u32, arg2 as u32),
        SYS_DEBUG_SINGLE_STEP => handlers::sys_debug_single_step(arg1 as u32),
        SYS_DEBUG_GET_MEM_MAP => handlers::sys_debug_get_mem_map(arg1 as u32, arg2, arg3 as u32),
        SYS_DEBUG_WAIT_EVENT => handlers::sys_debug_wait_event(arg1 as u32, arg2, arg3 as u32),
        SYS_THREAD_INFO_EX => handlers::sys_thread_info_ex(arg1 as u32, arg2, arg3 as u32),

        // Hardware virtualization (VT-x / AMD-V)
        #[cfg(target_arch = "x86_64")]
        SYS_VM_CREATE => crate::arch::x86::virt::syscalls::sys_vm_create(),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_DESTROY => crate::arch::x86::virt::syscalls::sys_vm_destroy(arg1 as u32),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_SET_MEMORY => crate::arch::x86::virt::syscalls::sys_vm_set_memory(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_SET_CPUID => crate::arch::x86::virt::syscalls::sys_vm_set_cpuid(
            arg1 as u32,
            arg2 as u64,
            arg3 as u32,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_HW_INFO => crate::arch::x86::virt::syscalls::sys_vm_hw_info(),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_GET_DIRTY_LOG => {
            crate::arch::x86::virt::syscalls::sys_vm_get_dirty_log(arg1 as u32, arg2 as u64)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_CREATE => {
            crate::arch::x86::virt::syscalls::sys_vcpu_create(arg1 as u32, arg2 as u32)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_RUN => {
            crate::arch::x86::virt::syscalls::sys_vcpu_run(arg1 as u32, arg2 as u32, arg3 as u64)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_PAUSE => {
            crate::arch::x86::virt::syscalls::sys_vcpu_pause(arg1 as u32, arg2 as u32)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_RESUME => {
            crate::arch::x86::virt::syscalls::sys_vcpu_resume(arg1 as u32, arg2 as u32)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_REGS => crate::arch::x86::virt::syscalls::sys_vcpu_get_regs(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_REGS => crate::arch::x86::virt::syscalls::sys_vcpu_set_regs(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_SREGS => crate::arch::x86::virt::syscalls::sys_vcpu_get_sregs(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_SREGS => crate::arch::x86::virt::syscalls::sys_vcpu_set_sregs(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_FPU => crate::arch::x86::virt::syscalls::sys_vcpu_get_fpu(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_FPU => crate::arch::x86::virt::syscalls::sys_vcpu_set_fpu(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_IRQ => crate::arch::x86::virt::syscalls::sys_vcpu_inject_irq(
            arg1 as u32,
            arg2 as u32,
            arg3 as u32,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_EXCEPTION => crate::arch::x86::virt::syscalls::sys_vcpu_inject_exception(
            arg1 as u32,
            arg2 as u32,
            arg3 as u32,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_NMI => {
            crate::arch::x86::virt::syscalls::sys_vcpu_inject_nmi(arg1 as u32, arg2 as u32)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_MP_STATE => {
            crate::arch::x86::virt::syscalls::sys_vcpu_get_mp_state(arg1 as u32, arg2 as u32)
        }
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_MP_STATE => crate::arch::x86::virt::syscalls::sys_vcpu_set_mp_state(
            arg1 as u32,
            arg2 as u32,
            arg3 as u32,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_TRANSLATE => crate::arch::x86::virt::syscalls::sys_vcpu_translate(
            arg1 as u32,
            arg2 as u32,
            arg3 as u64,
        ),
        #[cfg(target_arch = "x86_64")]
        SYS_AVM_IOCTL => crate::arch::x86::virt::avm::sys_avm_ioctl(
            arg1 as u64,
            arg2 as u32,
            arg3 as u64,
            arg4 as u64,
        ) as u32,

        _ => {
            crate::serial_println!("Unknown syscall: {}", syscall_num);
            u32::MAX
        }
    };

    // Log only failed syscalls (result == u32::MAX) in debug_verbose mode
    #[cfg(feature = "debug_verbose")]
    {
        if result == u32::MAX {
            let tid = crate::task::scheduler::current_tid();
            let name = table::syscall_name(syscall_num);
            crate::debug_println!(
                "ERR [T{}] {}({}) args=({:#x},{:#x},{:#x},{:#x}) -> FAIL",
                tid,
                name,
                syscall_num,
                arg1 as u32,
                arg2 as u32,
                arg3 as u32,
                arg4 as u32
            );
        }
    }

    // Post-syscall stack canary check: catch overflows before returning to user
    crate::task::scheduler::check_current_stack_canary(syscall_num);

    result
}

// =========================================================================
// 64-bit dispatch — called from syscall_fast.asm (SYSCALL instruction)
// =========================================================================
// The legacy INT 0x80 / syscall_dispatch_32 path was removed when 32-bit
// user space was dropped. The signal-return trampoline now uses SYSCALL
// too (see kernel/src/task/loader.rs).

/// Called from `syscall_fast.asm` for native 64-bit processes.
///
/// SYSCALL convention: RAX=num, RBX=arg1 as u32, R10=arg2 as u32, RDX=arg3 as u32, RSI=arg4 as u32, RDI=arg5 as u32.
/// The assembly stub pushes R10 into the RCX slot of `SyscallRegs`, so `regs.rcx`
/// contains the caller's R10 value (arg2 as u32).
///
/// Arguments are extracted as full u64 values. Currently truncated to u32 for
/// handler compatibility (all user addresses are below 4 GiB), but this entry
/// point is the place to widen handlers to u64 in the future.
#[no_mangle]
pub extern "C" fn syscall_dispatch_64(regs: &mut SyscallRegs) -> u64 {
    let syscall_num = regs.rax as u32;

    #[cfg(target_arch = "x86_64")]
    if crate::task::scheduler::current_thread_abi() == crate::task::abi::AbiPersonality::LinuxX86_64
    {
        let result = linux::dispatch(regs);
        return handlers::deliver_pending_signal_linux64(regs, result);
    }

    // fork() needs the full register frame — intercept before dispatch_inner.
    // Currently x86_64-only; ARM64 fork uses a separate ERET-based path.
    #[cfg(target_arch = "x86_64")]
    if syscall_num == SYS_FORK {
        let result = handlers::sys_fork(regs);
        handlers::deliver_pending_signal_default();
        return result as u64;
    }

    // sigreturn restores saved context — needs the full register frame.
    // The signal-return trampoline calls SYS_SIGRETURN via SYSCALL after
    // a user signal handler returns.
    if syscall_num == SYS_SIGRETURN {
        return handlers::sys_sigreturn(regs) as u64;
    }

    // Full 64-bit argument extraction (R10 is in the RCX slot per syscall_fast.asm)
    let arg1_64: u64 = regs.rbx;
    let arg2_64: u64 = regs.rcx; // actually R10
    let arg3_64: u64 = regs.rdx;
    let arg4_64: u64 = regs.rsi;
    let arg5_64: u64 = regs.rdi;

    match syscall_num {
        SYS_SBRK => {
            // u64 ABI: brk address can live anywhere in the canonical-low
            // half (including above 4 GiB once programs are upper-half).
            let r = handlers::sys_sbrk_u64(arg1_64 as i64);
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_MMAP => {
            // SYS_MMAP now returns the full 64-bit virtual address; user
            // space uses libsyscall::mmap() which already plumbs u64.
            let r = handlers::sys_mmap_u64(arg1_64);
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_MUNMAP => {
            // u64 address ABI for the standard mmap region.
            let r = handlers::sys_munmap_u64(arg1_64, arg2_64);
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_MMAP64 => {
            let r = handlers::sys_mmap64(arg1_64);
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_MUNMAP64 => {
            let r = handlers::sys_munmap64(arg1_64, arg2_64);
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_MAP_FRAMEBUFFER => {
            // Takes a u64 user-pointer (FbMapInfo out-buffer); user space now
            // lives in the upper canonical-low half, so truncating to u32 here
            // would corrupt the pointer.
            let r = handlers::sys_map_framebuffer(arg1_64);
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        _ => {}
    }

    // Hardware virtualization syscalls require full 64-bit pointer args — dispatch
    // them before the u32 truncation below so kernel addresses are preserved.
    #[cfg(target_arch = "x86_64")]
    match syscall_num {
        SYS_AVM_IOCTL => {
            let r = crate::arch::x86::virt::avm::sys_avm_ioctl(
                arg1_64,
                arg2_64 as u32,
                arg3_64,
                arg4_64,
            );
            handlers::deliver_pending_signal_default();
            return r;
        }
        SYS_VM_SET_MEMORY => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_set_memory(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_RUN => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_run(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_REGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_regs(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_REGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_regs(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_SREGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_sregs(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_SREGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_sregs(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VM_SET_CPUID => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_set_cpuid(
                arg1_64 as u32,
                arg2_64,
                arg3_64 as u32,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VM_GET_DIRTY_LOG => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_get_dirty_log(arg1_64 as u32, arg2_64);
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_FPU => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_fpu(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_FPU => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_fpu(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_TRANSLATE => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_translate(
                arg1_64 as u32,
                arg2_64 as u32,
                arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        _ => {}
    }

    // dispatch_inner takes the full 64-bit args; per-handler truncation
    // happens inside dispatch_inner via local u32 aliases.
    let result = dispatch_inner(syscall_num, arg1_64, arg2_64, arg3_64, arg4_64, arg5_64);
    handlers::deliver_pending_signal_default();
    result as u64
}
