//! System call interface — dual-path dispatch for 32-bit and 64-bit user processes.
//!
//! **INT 0x80 path** (`syscall_dispatch_32`):
//!   Used by 32-bit compatibility mode processes (libc, TCC).
//!   Convention: EAX=num, EBX=arg1, ECX=arg2, EDX=arg3, ESI=arg4, EDI=arg5.
//!   CPU zero-extends 32-bit registers to 64-bit on ring transition.
//!   All arguments are explicitly treated as u32.
//!
//! **SYSCALL path** (`syscall_dispatch_64`):
//!   Used by native 64-bit Rust processes (compositor, terminal, etc.).
//!   Convention: RAX=num, RBX=arg1, R10=arg2 (RCX clobbered), RDX=arg3, RSI=arg4, RDI=arg5.
//!   Arguments are full 64-bit values (currently truncated to u32 for handler compatibility,
//!   but the separation allows future widening without touching the 32-bit path).

mod defs;
pub mod handlers;
pub mod table;
pub use defs::*;

/// Register the `int 0x80` syscall trap gate and log readiness.
pub fn init() {
    crate::serial_println!("[OK] Syscall interface initialized (int 0x80 + SYSCALL)");
}

// =========================================================================
// Shared dispatch logic — routes syscall number to handler.
// Both 32-bit and 64-bit entry points extract args into u32 and call this.
// =========================================================================

#[inline(always)]
pub(crate) fn dispatch_inner(syscall_num: u32, arg1: u32, arg2: u32, arg3: u32, arg4: u32, arg5: u32) -> u32 {
    // Record last syscall for crash diagnostics (lock-free, per-CPU).
    let cpu_id = crate::arch::hal::cpu_id();
    crate::task::scheduler::set_last_syscall(cpu_id, syscall_num);

    // ── Fast path for high-frequency syscalls (no capability check needed) ──
    // These 5 syscalls account for ~80% of all calls. Skipping the capability
    // lookup and match-table indirection saves 2-3 branch mispredictions.
    match syscall_num {
        SYS_UPTIME_MS => return handlers::sys_uptime_ms(),
        SYS_YIELD => return handlers::sys_yield(),
        SYS_WRITE => return handlers::sys_write(arg1, arg2, arg3),
        SYS_READ => return handlers::sys_read(arg1, arg2, arg3),
        SYS_SLEEP => return handlers::sys_sleep(arg1),
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
                table::syscall_name(syscall_num), syscall_num, required, caps
            );
            return u32::MAX;
        }
    }

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
        SYS_SLEEP_US => handlers::sys_sleep_us(arg1),
        SYS_SBRK => handlers::sys_sbrk(arg1 as i32),
        SYS_MMAP => handlers::sys_mmap(arg1),
        SYS_MUNMAP => handlers::sys_munmap(arg1, arg2),
        SYS_WAITPID => handlers::sys_waitpid(arg1, arg2, arg3),
        SYS_KILL => handlers::sys_kill(arg1, arg2),
        SYS_SPAWN => handlers::sys_spawn(arg1, arg2, arg3, arg4),
        SYS_EXEC => handlers::sys_exec(arg1, arg2),
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
        SYS_CHDIR => handlers::sys_chdir(arg1),
        SYS_MKDIR => handlers::sys_mkdir(arg1),
        SYS_UNLINK => handlers::sys_unlink(arg1),
        SYS_TRUNCATE => handlers::sys_truncate(arg1),
        SYS_SYMLINK => handlers::sys_symlink(arg1, arg2),
        SYS_READLINK => handlers::sys_readlink(arg1, arg2, arg3),
        SYS_LSTAT => handlers::sys_lstat(arg1, arg2),
        SYS_MOUNT => handlers::sys_mount(arg1, arg2, arg3),
        SYS_UMOUNT => handlers::sys_umount(arg1),
        SYS_LIST_MOUNTS => handlers::sys_list_mounts(arg1, arg2),
        SYS_STATFS => handlers::sys_statfs(arg1, arg2, arg3),
        SYS_RENAME => handlers::sys_rename(arg1, arg2),
        SYS_LSEEK => handlers::sys_lseek(arg1, arg2, arg3),
        SYS_FSTAT => handlers::sys_fstat(arg1, arg2),
        SYS_FTRUNCATE => handlers::sys_ftruncate(arg1, arg2),
        SYS_ISATTY => handlers::sys_isatty(arg1),

        // System info
        SYS_TIME => handlers::sys_time(arg1),
        SYS_SET_TIME => handlers::sys_set_time(arg1),
        SYS_UPTIME => handlers::sys_uptime(),
        SYS_SYSINFO => handlers::sys_sysinfo(arg1, arg2, arg3),
        SYS_DMESG => handlers::sys_dmesg(arg1, arg2),
        SYS_TICK_HZ => handlers::sys_tick_hz(),
        SYS_UPTIME_MS => handlers::sys_uptime_ms(),

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
        SYS_TCP_RECV_AVAILABLE => handlers::sys_tcp_recv_available(arg1),
        SYS_TCP_SHUTDOWN_WR => handlers::sys_tcp_shutdown_wr(arg1),
        SYS_TCP_LISTEN => handlers::sys_tcp_listen(arg1, arg2),
        SYS_TCP_ACCEPT => handlers::sys_tcp_accept(arg1, arg2),
        SYS_TCP_ACCEPT_NOWAIT => handlers::sys_tcp_accept_nowait(arg1, arg2),
        SYS_TCP_LIST => handlers::sys_tcp_list(arg1, arg2),

        // Network polling
        SYS_NET_POLL => handlers::sys_net_poll(),

        // UDP
        SYS_UDP_BIND => handlers::sys_udp_bind(arg1),
        SYS_UDP_UNBIND => handlers::sys_udp_unbind(arg1),
        SYS_UDP_SENDTO => handlers::sys_udp_sendto(arg1),
        SYS_UDP_RECVFROM => handlers::sys_udp_recvfrom(arg1, arg2, arg3),
        SYS_UDP_SET_OPT => handlers::sys_udp_set_opt(arg1, arg2, arg3),
        SYS_UDP_LIST => handlers::sys_udp_list(arg1, arg2),
        SYS_NET_STATS => handlers::sys_net_stats(arg1, arg2),
        SYS_PIPE_BYTES_AVAILABLE => handlers::sys_pipe_bytes_available(arg1),
        SYS_WIFI => handlers::sys_wifi(arg1, arg2, arg3),

        // IPv6
        SYS_NET_PING6 => handlers::sys_net_ping6(arg1, arg2, arg3),
        SYS_NET_DNS6 => handlers::sys_net_dns6(arg1, arg2),
        SYS_TCP_CONNECT_V6 => handlers::sys_tcp_connect_v6(arg1),

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
        SYS_EVT_CHAN_WAIT => handlers::sys_evt_chan_wait(arg1, arg2, arg3),

        // Display / GPU
        SYS_SCREEN_SIZE => handlers::sys_screen_size(arg1),
        SYS_SET_RESOLUTION => handlers::sys_set_resolution(arg1, arg2),
        SYS_LIST_RESOLUTIONS => handlers::sys_list_resolutions(arg1, arg2),
        SYS_GPU_INFO => handlers::sys_gpu_info(arg1, arg2),
        SYS_GPU_HAS_ACCEL => handlers::sys_gpu_has_accel(),
        SYS_GPU_HAS_HW_CURSOR => handlers::sys_gpu_has_hw_cursor(),
        SYS_BOOT_READY => handlers::sys_boot_ready(),

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

        // DLIB shared page write
        SYS_SET_DLL_U32 => handlers::sys_set_dll_u32(arg1, arg2, arg3),

        // Keyboard layout
        SYS_KBD_GET_LAYOUT => handlers::sys_kbd_get_layout(),
        SYS_KBD_SET_LAYOUT => handlers::sys_kbd_set_layout(arg1),
        SYS_KBD_LIST_LAYOUTS => handlers::sys_kbd_list_layouts(arg1, arg2),

        // Random number generation
        SYS_RANDOM => handlers::sys_random(arg1, arg2),

        // Capabilities query
        SYS_GET_CAPABILITIES => handlers::sys_get_capabilities(),

        // User identity & management
        SYS_GETUID => handlers::sys_getuid(),
        SYS_GETGID => handlers::sys_getgid(),
        SYS_AUTHENTICATE => handlers::sys_authenticate(arg1, arg2),
        SYS_CHMOD => handlers::sys_chmod(arg1, arg2),
        SYS_CHOWN => handlers::sys_chown(arg1, arg2, arg3),
        SYS_ADDUSER => handlers::sys_adduser(arg1),
        SYS_DELUSER => handlers::sys_deluser(arg1),
        SYS_LISTUSERS => handlers::sys_listusers(arg1, arg2),
        SYS_ADDGROUP => handlers::sys_addgroup(arg1),
        SYS_DELGROUP => handlers::sys_delgroup(arg1),
        SYS_LISTGROUPS => handlers::sys_listgroups(arg1, arg2),
        SYS_GETUSERNAME => handlers::sys_getusername(arg1, arg2, arg3),
        SYS_SET_IDENTITY => handlers::sys_set_identity(arg1),
        SYS_CHPASSWD => handlers::sys_chpasswd(arg1),

        // POSIX anonymous pipes / FD duplication
        SYS_PIPE2 => handlers::sys_pipe2(arg1, arg2),
        SYS_DUP => handlers::sys_dup(arg1),
        SYS_DUP2 => handlers::sys_dup2(arg1, arg2),
        SYS_FCNTL => handlers::sys_fcntl(arg1, arg2, arg3),

        // POSIX signals (SYS_SIGRETURN intercepted at dispatch level, not here)
        SYS_SIGACTION => handlers::sys_sigaction(arg1, arg2),
        SYS_SIGPROCMASK => handlers::sys_sigprocmask(arg1, arg2),

        // Process identity (extended)
        SYS_GETPPID => handlers::sys_getppid(),

        // VRAM direct surface
        SYS_GPU_VRAM_SIZE => handlers::sys_gpu_vram_size(),
        SYS_VRAM_MAP => handlers::sys_vram_map(arg1, arg2, arg3),
        SYS_GPU_REGISTER_BACKBUFFER => handlers::sys_gpu_register_backbuffer(arg1, arg2),
        SYS_GRANT_FRAMEBUFFER => handlers::sys_grant_framebuffer(arg1, arg2),
        SYS_REVOKE_FRAMEBUFFER => handlers::sys_revoke_framebuffer(arg1),

        // App permissions
        SYS_PERM_CHECK => handlers::sys_perm_check(arg1, arg2),
        SYS_PERM_STORE => handlers::sys_perm_store(arg1, arg2, arg3),
        SYS_PERM_LIST => handlers::sys_perm_list(arg1, arg2),
        SYS_PERM_DELETE => handlers::sys_perm_delete(arg1),
        SYS_PERM_PENDING_INFO => handlers::sys_perm_pending_info(arg1, arg2),
        SYS_REGISTER_SESSIONHOST => handlers::sys_register_sessionhost(),

        // Crash info
        SYS_GET_CRASH_INFO => handlers::sys_get_crash_info(arg1, arg2, arg3),

        // Disk / partition management
        SYS_DISK_LIST => handlers::sys_disk_list(arg1, arg2),
        SYS_DISK_PARTITIONS => handlers::sys_disk_partitions(arg1, arg2, arg3),
        SYS_DISK_EJECT => handlers::sys_disk_eject(arg1),
        SYS_DISK_READ => handlers::sys_disk_read(arg1, arg2, arg3, arg4, arg5),
        SYS_DISK_WRITE => handlers::sys_disk_write(arg1, arg2, arg3, arg4, arg5),
        SYS_PARTITION_CREATE => handlers::sys_partition_create(arg1, arg2, arg3),
        SYS_PARTITION_DELETE => handlers::sys_partition_delete(arg1, arg2),
        SYS_PARTITION_RESCAN => handlers::sys_partition_rescan(arg1),

        // GPU 3D acceleration (SVGA3D)
        SYS_GPU_3D_SUBMIT => handlers::sys_gpu_3d_submit(arg1, arg2),
        SYS_GPU_3D_QUERY => handlers::sys_gpu_3d_query(arg1),
        SYS_GPU_3D_SYNC => handlers::sys_gpu_3d_sync(),
        SYS_GPU_3D_SURFACE_DMA => handlers::sys_gpu_3d_surface_dma(arg1, arg2, arg3, arg4, arg5),
        SYS_GPU_3D_SURFACE_DMA_READ => handlers::sys_gpu_3d_surface_dma_read(arg1, arg2, arg3, arg4, arg5),
        SYS_GPU_QUERY_TYPE => handlers::sys_gpu_query_type(arg1, arg2),
        SYS_GPU_3D_RESOURCE_CREATE => handlers::sys_gpu_3d_resource_create(arg1, arg2, arg3, arg4, arg5),
        SYS_GPU_3D_RESOURCE_DESTROY => handlers::sys_gpu_3d_resource_destroy(arg1),

        // Hostname
        SYS_GET_HOSTNAME => handlers::sys_get_hostname(arg1, arg2),
        SYS_SET_HOSTNAME => handlers::sys_set_hostname(arg1, arg2),

        // Power management
        SYS_SHUTDOWN => handlers::sys_shutdown(arg1),
        SYS_SYNC => handlers::sys_sync(),
        SYS_FSYNC => handlers::sys_fsync(arg1),

        // Kernel debug settings
        SYS_SET_SERIAL_VERBOSE => handlers::sys_set_serial_verbose(arg1),

        // Text-mode console I/O
        SYS_CON_WRITE     => handlers::sys_con_write(arg1, arg2),
        SYS_CON_READ      => handlers::sys_con_read(arg1, arg2),
        SYS_CON_POLL_KEY  => handlers::sys_con_poll_key(),
        SYS_CON_GET_SIZE  => handlers::sys_con_get_size(),
        SYS_CON_SET_MODE  => handlers::sys_con_set_mode(arg1),
        SYS_CON_RESIZE    => handlers::sys_con_resize(arg1),

        // Platform / thermal / ACPI / I²C
        #[cfg(target_arch = "x86_64")]
        SYS_THERMAL_READ => handlers::sys_thermal_read(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_THERMAL_CPU => handlers::sys_thermal_cpu(),
        #[cfg(target_arch = "x86_64")]
        SYS_ACPI_SLEEP => handlers::sys_acpi_sleep(arg1),
        #[cfg(target_arch = "x86_64")]
        SYS_ACPI_PERF => handlers::sys_acpi_perf(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_READ => handlers::sys_i2c_read(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_WRITE => handlers::sys_i2c_write(arg1, arg2, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_I2C_DETECT => handlers::sys_i2c_detect(arg1),

        // Monitor detection
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_COUNT => handlers::sys_monitor_count(),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_INFO => handlers::sys_monitor_info(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_EDID => handlers::sys_monitor_edid(arg1, arg2, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_MONITOR_MODES => handlers::sys_monitor_modes(arg1, arg2, arg3),

        // Debug / trace (anyTrace)
        SYS_DEBUG_ATTACH => handlers::sys_debug_attach(arg1),
        SYS_DEBUG_DETACH => handlers::sys_debug_detach(arg1),
        SYS_DEBUG_SUSPEND => handlers::sys_debug_suspend(arg1),
        SYS_DEBUG_RESUME => handlers::sys_debug_resume(arg1),
        SYS_DEBUG_GET_REGS => handlers::sys_debug_get_regs(arg1, arg2, arg3),
        SYS_DEBUG_SET_REGS => handlers::sys_debug_set_regs(arg1, arg2, arg3),
        SYS_DEBUG_READ_MEM => handlers::sys_debug_read_mem(arg1, arg2, arg3, arg4),
        SYS_DEBUG_WRITE_MEM => handlers::sys_debug_write_mem(arg1, arg2, arg3, arg4),
        SYS_DEBUG_SET_BREAKPOINT => handlers::sys_debug_set_breakpoint(arg1, arg2),
        SYS_DEBUG_CLR_BREAKPOINT => handlers::sys_debug_clr_breakpoint(arg1, arg2),
        SYS_DEBUG_SINGLE_STEP => handlers::sys_debug_single_step(arg1),
        SYS_DEBUG_GET_MEM_MAP => handlers::sys_debug_get_mem_map(arg1, arg2, arg3),
        SYS_DEBUG_WAIT_EVENT => handlers::sys_debug_wait_event(arg1, arg2, arg3),
        SYS_THREAD_INFO_EX => handlers::sys_thread_info_ex(arg1, arg2, arg3),

        // Hardware virtualization (VT-x / AMD-V)
        #[cfg(target_arch = "x86_64")]
        SYS_VM_CREATE => crate::arch::x86::virt::syscalls::sys_vm_create(),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_DESTROY => crate::arch::x86::virt::syscalls::sys_vm_destroy(arg1),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_SET_MEMORY => crate::arch::x86::virt::syscalls::sys_vm_set_memory(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_SET_CPUID => crate::arch::x86::virt::syscalls::sys_vm_set_cpuid(arg1, arg2 as u64, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_HW_INFO => crate::arch::x86::virt::syscalls::sys_vm_hw_info(),
        #[cfg(target_arch = "x86_64")]
        SYS_VM_GET_DIRTY_LOG => crate::arch::x86::virt::syscalls::sys_vm_get_dirty_log(arg1, arg2 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_CREATE => crate::arch::x86::virt::syscalls::sys_vcpu_create(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_RUN => crate::arch::x86::virt::syscalls::sys_vcpu_run(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_PAUSE => crate::arch::x86::virt::syscalls::sys_vcpu_pause(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_RESUME => crate::arch::x86::virt::syscalls::sys_vcpu_resume(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_REGS => crate::arch::x86::virt::syscalls::sys_vcpu_get_regs(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_REGS => crate::arch::x86::virt::syscalls::sys_vcpu_set_regs(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_SREGS => crate::arch::x86::virt::syscalls::sys_vcpu_get_sregs(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_SREGS => crate::arch::x86::virt::syscalls::sys_vcpu_set_sregs(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_FPU => crate::arch::x86::virt::syscalls::sys_vcpu_get_fpu(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_FPU => crate::arch::x86::virt::syscalls::sys_vcpu_set_fpu(arg1, arg2, arg3 as u64),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_IRQ => crate::arch::x86::virt::syscalls::sys_vcpu_inject_irq(arg1, arg2, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_EXCEPTION => crate::arch::x86::virt::syscalls::sys_vcpu_inject_exception(arg1, arg2, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_INJECT_NMI => crate::arch::x86::virt::syscalls::sys_vcpu_inject_nmi(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_GET_MP_STATE => crate::arch::x86::virt::syscalls::sys_vcpu_get_mp_state(arg1, arg2),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_SET_MP_STATE => crate::arch::x86::virt::syscalls::sys_vcpu_set_mp_state(arg1, arg2, arg3),
        #[cfg(target_arch = "x86_64")]
        SYS_VCPU_TRANSLATE => crate::arch::x86::virt::syscalls::sys_vcpu_translate(arg1, arg2, arg3 as u64),

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
            crate::debug_println!("ERR [T{}] {}({}) args=({:#x},{:#x},{:#x},{:#x}) -> FAIL",
                tid, name, syscall_num, arg1, arg2, arg3, arg4);
        }
    }

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

    // fork() needs the full register frame — intercept before dispatch_inner.
    // Currently x86_64-only; ARM64 fork uses a separate ERET-based path.
    #[cfg(target_arch = "x86_64")]
    if syscall_num == SYS_FORK {
        let result = handlers::sys_fork(regs);
        handlers::deliver_pending_signal_32(regs, result);
        return result;
    }

    // sigreturn restores saved context — needs full register frame
    if syscall_num == SYS_SIGRETURN {
        return handlers::sys_sigreturn_32(regs);
    }

    let arg1 = regs.rbx as u32;
    let arg2 = regs.rcx as u32;
    let arg3 = regs.rdx as u32;
    let arg4 = regs.rsi as u32;
    let arg5 = regs.rdi as u32;

    let result = dispatch_inner(syscall_num, arg1, arg2, arg3, arg4, arg5);
    handlers::deliver_pending_signal_32(regs, result);
    result
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

    // fork() needs the full register frame — intercept before dispatch_inner.
    // Currently x86_64-only; ARM64 fork uses a separate ERET-based path.
    #[cfg(target_arch = "x86_64")]
    if syscall_num == SYS_FORK {
        let result = handlers::sys_fork(regs);
        handlers::deliver_pending_signal_default();
        return result as u64;
    }

    // Full 64-bit argument extraction (R10 is in the RCX slot per syscall_fast.asm)
    let arg1_64: u64 = regs.rbx;
    let arg2_64: u64 = regs.rcx; // actually R10
    let arg3_64: u64 = regs.rdx;
    let arg4_64: u64 = regs.rsi;
    let arg5_64: u64 = regs.rdi;

    // Hardware virtualization syscalls require full 64-bit pointer args — dispatch
    // them before the u32 truncation below so kernel addresses are preserved.
    #[cfg(target_arch = "x86_64")]
    match syscall_num {
        SYS_VM_SET_MEMORY => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_set_memory(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_RUN => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_run(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_REGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_regs(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_REGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_regs(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_SREGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_sregs(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_SREGS => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_sregs(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VM_SET_CPUID => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_set_cpuid(
                arg1_64 as u32, arg2_64, arg3_64 as u32,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VM_GET_DIRTY_LOG => {
            let r = crate::arch::x86::virt::syscalls::sys_vm_get_dirty_log(
                arg1_64 as u32, arg2_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_GET_FPU => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_get_fpu(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_SET_FPU => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_set_fpu(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        SYS_VCPU_TRANSLATE => {
            let r = crate::arch::x86::virt::syscalls::sys_vcpu_translate(
                arg1_64 as u32, arg2_64 as u32, arg3_64,
            );
            handlers::deliver_pending_signal_default();
            return r as u64;
        }
        _ => {}
    }

    // Truncate to u32 for existing handler signatures.
    let arg1 = arg1_64 as u32;
    let arg2 = arg2_64 as u32;
    let arg3 = arg3_64 as u32;
    let arg4 = arg4_64 as u32;
    let arg5 = arg5_64 as u32;

    let result = dispatch_inner(syscall_num, arg1, arg2, arg3, arg4, arg5);
    handlers::deliver_pending_signal_default();
    result as u64
}
