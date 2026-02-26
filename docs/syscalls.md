# anyOS Syscall Reference

Complete reference for all 155+ system calls in anyOS. Syscalls are the interface between user-space programs and the kernel.

## Calling Conventions

anyOS supports two syscall entry paths:

### 64-bit SYSCALL (Rust programs via `anyos_std`)

| Register | Purpose |
|----------|---------|
| RAX | Syscall number (in) / return value (out) |
| RBX | Argument 1 |
| R10 | Argument 2 (not RCX — SYSCALL clobbers RCX) |
| RDX | Argument 3 |
| RSI | Argument 4 |
| RDI | Argument 5 |

Clobbers: RCX (user RIP), R11 (user RFLAGS). ~10x faster than INT 0x80.

### 32-bit INT 0x80 (C programs via libc)

| Register | Purpose |
|----------|---------|
| EAX | Syscall number (in) / return value (out) |
| EBX | Argument 1 |
| ECX | Argument 2 |
| EDX | Argument 3 |
| ESI | Argument 4 |
| EDI | Argument 5 |

Used by 32-bit compatibility mode (libc, TCC-compiled programs).

### Return Values

- **0**: Success (for most syscalls)
- **Positive value**: Success with data (fd, tid, byte count, etc.)
- **0xFFFFFFFF** (`u32::MAX`): Error / not found
- **0xFFFFFFFE** (`u32::MAX - 1`): Special (e.g. `STILL_RUNNING` for `try_waitpid`)
- **0xFFFFFFFD** (`u32::MAX - 2`): `PERM_NEEDED` — app requires user permission approval before spawning

---

## Process Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 1 | `exit` | status | — | Terminate current process with exit code |
| 6 | `getpid` | — | tid | Get current thread ID |
| 7 | `yield` | — | 0 | Yield CPU time slice to scheduler |
| 8 | `sleep` | ms | 0 | Sleep for N milliseconds (blocks thread) |
| 9 | `sbrk` | increment (i32) | old_brk | Grow/shrink process heap; returns previous break address |
| 10 | `fork` | — | child_tid (parent) / 0 (child) | Fork current process. Child gets copy of address space, returns 0. Parent returns child TID |
| 11 | `exec` | path_ptr, args_ptr | never returns / 0xFFFFFFFF | Replace current process image with new program. On failure returns error |
| 12 | `waitpid` | tid | exit_code | Block until process exits; returns its exit code |
| 13 | `kill` | tid | 0 or error | Terminate thread by TID |
| 29 | `try_waitpid` | tid | code, 0xFFFFFFFE, or 0xFFFFFFFF | Non-blocking: exit code if done, `STILL_RUNNING` if alive, `NOT_FOUND` if invalid |
| 247 | `getppid` | — | parent_tid | Get parent process/thread ID |

## Process Spawning

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 27 | `spawn` | path_ptr, stdout_pipe, args_ptr, stdin_pipe | tid or 0xFFFFFFFF | Spawn process from filesystem path with optional pipe I/O redirection |
| 28 | `getargs` | buf_ptr, buf_size | bytes_written | Get command-line arguments string for current process |

## Threading

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 170 | `thread_create` | entry_rip, user_rsp, name_ptr, name_len, priority | tid or 0 | Create new thread in current process address space |
| 171 | `set_priority` | tid (0=self), priority (0–127) | 0 or error | Change thread scheduling priority (0=lowest/idle, 127=highest/real-time) |
| 172 | `set_critical` | — | 0 | Mark thread as critical (won't be killed on process exit) |

## Memory Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 14 | `mmap` | size | vaddr or 0xFFFFFFFF | Allocate anonymous pages (returns address from `0x20000000`) |
| 15 | `munmap` | addr, size | 0 or error | Free mapped pages; addr must be page-aligned |

## File I/O

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 2 | `write` | fd, buf_ptr, len | bytes_written | Write to file descriptor (1=stdout, 2=stderr, 3+=files) |
| 3 | `read` | fd, buf_ptr, len | bytes_read | Read from file descriptor (0=stdin, 3+=files) |
| 4 | `open` | path_ptr, flags, — | fd or 0xFFFFFFFF | Open file. Flags: 1=write, 2=append, 4=create, 8=truncate |
| 5 | `close` | fd | 0 or error | Close file descriptor |
| 105 | `lseek` | fd, offset (i32), whence | new_position | Seek in file. Whence: 0=SET, 1=CUR, 2=END |
| 106 | `fstat` | fd, buf_ptr | 0 or error | Get file info by fd. Output: type(u32), size(u32), position(u32) |
| 107 | `ftruncate` | fd, length | 0 or error | Truncate open file to given length |
| 108 | `isatty` | fd | 1 or 0 | Returns 1 for stdin/stdout/stderr, 0 for files |

## Filesystem Operations

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 23 | `readdir` | path_ptr, buf_ptr, buf_size | entry_count | List directory entries. Each entry: 64 bytes (type, name_len, flags, size, name) |
| 24 | `stat` | path_ptr, buf_ptr | 0 or error | Get file status (follows symlinks). Output: 24 bytes [type, size, flags, uid, gid, mode] |
| 25 | `getcwd` | buf_ptr, buf_size | length | Get current working directory path |
| 26 | `chdir` | path_ptr | 0 or error | Change working directory |
| 90 | `mkdir` | path_ptr | 0 or error | Create directory |
| 91 | `unlink` | path_ptr | 0 or error | Delete file |
| 92 | `truncate` | path_ptr | 0 or error | Truncate file to zero bytes |
| 96 | `symlink` | target_ptr, link_path_ptr | 0 or error | Create symbolic link |
| 97 | `readlink` | path_ptr, buf_ptr, buf_size | bytes_read | Read symlink target path |
| 98 | `lstat` | path_ptr, buf_ptr | 0 or error | Like stat but does NOT follow final symlink |
| 99 | `rename` | old_path_ptr, new_path_ptr | 0 or error | Rename/move file or directory |
| 224 | `chmod` | path_ptr, mode (u16) | 0 or error | Change file permission mode (owner or root only) |
| 225 | `chown` | path_ptr, uid, gid | 0 or error | Change file owner/group (root only) |

## Mount System

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 93 | `mount` | mount_path_ptr, device_path_ptr, fs_type | 0 or error | Mount filesystem. fs_type: 0=FAT, 1=ISO9660 |
| 94 | `umount` | mount_path_ptr | 0 or error | Unmount filesystem |
| 95 | `list_mounts` | buf_ptr, buf_len | bytes_written | List mounted filesystems ("mount_path\tfs_type\n" format) |

## Networking — Configuration & Diagnostics

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 40 | `net_config` | cmd, buf_ptr | status | cmd: 0=get, 1=set, 2=disable, 3=enable, 4=query_enabled, 5=query_available. Buffer: 24 bytes [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1] |
| 41 | `net_ping` | ip_ptr, seq, timeout_ticks | rtt_ticks or 0xFFFFFFFF | ICMP echo request; returns round-trip time in PIT ticks |
| 42 | `net_dhcp` | buf_ptr | 0 or error | DHCP discovery and auto-configuration |
| 43 | `net_dns` | hostname_ptr, result_ptr | 0 or error | DNS name resolution; writes 4-byte IPv4 address to result_ptr |
| 44 | `net_arp` | buf_ptr, buf_size | entry_count | Get ARP table. Each entry: 12 bytes [ip:4, mac:6, pad:2] |
| 50 | `net_poll` | — | 0 | Process pending network packets (triggers RX ring processing and TCP dispatch) |

## Networking — TCP

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 100 | `tcp_connect` | params_ptr, —, — | socket_id or 0xFFFFFFFF | Connect to remote host. Params: 12 bytes [ip:4, port:u32, timeout:u32] |
| 101 | `tcp_send` | socket_id, buf_ptr, len | bytes_sent or error | Send data on TCP socket |
| 102 | `tcp_recv` | socket_id, buf_ptr, len | bytes_received (0=EOF) | Receive data from TCP socket |
| 103 | `tcp_close` | socket_id | 0 | Close TCP connection |
| 104 | `tcp_status` | socket_id | state_enum | Get TCP connection state |
| 130 | `tcp_recv_available` | socket_id | bytes or 0xFFFFFFFE (EOF) | Check bytes available without blocking |
| 131 | `tcp_shutdown_wr` | socket_id | 0 | Half-close: send FIN, can still receive |
| 132 | `tcp_listen` | port, backlog | listener_id or 0xFFFFFFFF | Listen for incoming TCP connections on port |
| 133 | `tcp_accept` | listener_id, result_ptr | 0 or 0xFFFFFFFF | Accept connection. Writes to result_ptr: [socket_id:u32, ip:u8[4], port:u16, pad:u16] |
| 134 | `tcp_list` | buf_ptr, buf_size | entry_count | List all active TCP connections |

## Networking — UDP

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 150 | `udp_bind` | port | 0 or error | Bind to UDP port, create receive queue |
| 151 | `udp_unbind` | port | 0 | Release UDP port binding |
| 152 | `udp_sendto` | params_ptr | bytes_sent or error | Send datagram. Params: 20 bytes [dst_ip:4, dst_port:u32, src_port:u32, data_ptr:u32, data_len:u32] |
| 153 | `udp_recvfrom` | port, buf_ptr, buf_len | total_bytes | Receive datagram. Header: [src_ip:4, src_port:u16, payload_len:u16] + payload |
| 154 | `udp_set_opt` | port, opt, val | 0 or error | Set socket option. opt: 1=SO_BROADCAST, 2=SO_RCVTIMEO |

## Pipes / Named IPC

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 45 | `pipe_create` | name_ptr | pipe_id (>0) | Create named pipe |
| 46 | `pipe_read` | pipe_id, buf_ptr, len | bytes_read | Read from pipe (returns 0 if empty) |
| 47 | `pipe_close` | pipe_id | 0 | Destroy pipe and free buffer |
| 48 | `pipe_write` | pipe_id, buf_ptr, len | bytes_written | Write to pipe |
| 49 | `pipe_open` | name_ptr | pipe_id or 0 | Open existing pipe by name |
| 180 | `pipe_list` | buf_ptr, buf_size | pipe_count | List open pipes. Each entry: 80 bytes [id, buffered_bytes, name] |

## Shared Memory

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 140 | `shm_create` | size | shm_id (>0) or 0 | Create shared memory region |
| 141 | `shm_map` | shm_id | vaddr or 0 | Map shared memory into current process |
| 142 | `shm_unmap` | shm_id | 0 or error | Unmap shared memory from current process |
| 143 | `shm_destroy` | shm_id | 0 or error | Destroy shared memory (creator only) |

## Event Bus

### System Events

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 60 | `evt_sys_subscribe` | filter | sub_id | Subscribe to system events matching filter bitmask |
| 61 | `evt_sys_poll` | sub_id, buf_ptr (20 bytes) | 1 or 0 | Poll next system event. Returns 1 if event written to buf |
| 62 | `evt_sys_unsubscribe` | sub_id | 0 | Unsubscribe from system events |

### Module Channels

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 63 | `evt_chan_create` | name_ptr, name_len | channel_id | Create named event channel |
| 64 | `evt_chan_subscribe` | chan_id, filter | sub_id | Subscribe to channel events |
| 65 | `evt_chan_emit` | chan_id, event_ptr (20 bytes) | 0 | Broadcast event to all channel subscribers |
| 66 | `evt_chan_poll` | chan_id, sub_id, buf_ptr | 1 or 0 | Poll next event from channel subscription |
| 67 | `evt_chan_unsubscribe` | chan_id, sub_id | 0 | Unsubscribe from channel |
| 68 | `evt_chan_destroy` | chan_id | 0 | Destroy channel (creator only) |
| 69 | `evt_chan_emit_to` | chan_id, sub_id, event_ptr | 0 | Unicast event to specific subscriber |
| 70 | `evt_chan_wait` | chan_id, sub_id, timeout_ms | 1 or 0 | Blocking wait for channel event with timeout |

## Display / GPU

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 72 | `screen_size` | buf_ptr | 0 | Get screen dimensions. Output: 2 u32s [width, height] |
| 110 | `set_resolution` | width, height | 0 or error | Change display resolution |
| 111 | `list_resolutions` | buf_ptr, buf_len | mode_count | List supported modes. Each: 8 bytes [width:u32, height:u32] |
| 112 | `gpu_info` | buf_ptr, buf_len | name_length | Get GPU driver name string |
| 135 | `gpu_has_accel` | — | 1 or 0 | Query if GPU acceleration is available |
| 137 | `boot_ready` | — | 0 | Signal desktop is fully loaded (boot timing marker) |
| 138 | `gpu_has_hw_cursor` | — | 1 or 0 | Query if GPU hardware cursor is available |
| 161 | `capture_screen` | buf_ptr, buf_size, info_ptr | 0, 1 (no GPU), or 2 (too small) | Capture framebuffer to user buffer |
| 258 | `gpu_register_backbuffer` | buf_ptr, buf_size | 0 or error | Register GPU backbuffer for DMA write (compositor-only) |

## Audio

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 120 | `audio_write` | buf_ptr, buf_len | bytes_written | Write PCM data to audio output (48kHz 16-bit stereo) |
| 121 | `audio_ctl` | cmd, arg | result | cmd: 0=stop, 1=set_volume(0–100), 2=get_volume, 3=is_playing, 4=is_available |

## System Information

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 30 | `time` | buf_ptr (8 bytes) | 0 | Get RTC time: [year_lo, year_hi, month, day, hour, min, sec, 0] |
| 31 | `uptime` | — | ticks | System uptime in PIT ticks |
| 32 | `sysinfo` | cmd, buf_ptr, buf_size | varies | cmd: 0=memory, 1=threads, 2=cpus, 3=cpu_load, 4=hardware |
| 33 | `dmesg` | buf_ptr, buf_size | bytes_written | Read kernel log ring buffer |
| 34 | `tick_hz` | — | hz | Get PIT tick frequency in Hz |
| 35 | `uptime_ms` | — | ms | System uptime in milliseconds (TSC-based, sub-ms precision) |

## Device Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 16 | `devlist` | buf_ptr, buf_size | device_count | List devices. Each entry: 64 bytes [path, driver_name, type] |
| 17 | `devopen` | path_ptr, flags | 0 or error | Check if device exists |
| 18 | `devclose` | handle | 0 | Close device handle (no-op) |
| 19 | `devread` | handle, buf_ptr, len | error | Read from device (stub) |
| 20 | `devwrite` | handle, buf_ptr, len | error | Write to device (stub) |
| 21 | `devioctl` | dtype, cmd, arg | result or error | Send ioctl command to driver by type |
| 22 | `irqwait` | irq | 0 | Wait for IRQ (stub) |

## DLL System

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 80 | `dll_load` | path_ptr, path_len | base_vaddr or 0 | Load/map DLL into current process, returns base address |
| 190 | `set_dll_u32` | dll_base_lo, offset, value | 0 or error | Write u32 to shared DLL page (used for theme switching) |

## Compositor-Privileged

These syscalls require prior `register_compositor()` call (first caller wins).

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 144 | `map_framebuffer` | out_info_ptr (16 bytes) | 0 or error | Map GPU framebuffer to 0x20000000. Output: [vaddr, width, height, pitch] |
| 145 | `gpu_command` | cmd_buf_ptr, cmd_count | cmds_executed | Submit GPU commands: UPDATE, FILL_RECT, COPY_RECT, CURSOR, DEFINE_CURSOR, FLIP |
| 146 | `input_poll` | buf_ptr, max_events | event_count | Poll raw keyboard/mouse events. Each: 20 bytes [type, args[4]] |
| 147 | `register_compositor` | — | 0 or error | Register as compositor (first caller wins, sets priority 127) |
| 148 | `cursor_takeover` | — | (x<<16)\|(y&0xFFFF) | Take cursor control from boot splash; returns splash cursor position |
| 256 | `gpu_vram_size` | — | bytes | Get total GPU VRAM size in bytes (compositor only) |
| 257 | `vram_map` | target_tid, vram_offset, num_bytes | 0x18000000 or 0 | Map VRAM into target process at 0x18000000 with Write-Through caching (compositor only) |

## Environment Variables

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 182 | `setenv` | key_ptr, val_ptr (0 to unset) | 0 | Set or remove environment variable |
| 183 | `getenv` | key_ptr, val_buf_ptr, val_buf_size | length or error | Get environment variable value |
| 184 | `listenv` | buf_ptr, buf_size | bytes_needed | List all env vars as "KEY=VALUE\0" entries |

## Keyboard Layout

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 200 | `kbd_get_layout` | — | layout_id | Get active keyboard layout ID |
| 201 | `kbd_set_layout` | layout_id | 0 or error | Set keyboard layout |
| 202 | `kbd_list_layouts` | buf_ptr, max_entries | entry_count | List available layouts. Each: LayoutInfo struct |

## Random Number Generation

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 210 | `random` | buf_ptr, len (max 256) | bytes_written | Fill buffer with random bytes (RDRAND or TSC-based fallback) |

## Capabilities

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 220 | `get_capabilities` | — | bitmask | Get calling thread's capability flags |

## User Identity & Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 221 | `getuid` | — | uid | Get calling process user ID |
| 222 | `getgid` | — | gid | Get calling process group ID |
| 223 | `authenticate` | username_ptr, password_ptr | 0 or error | Verify credentials, set uid/gid on process |
| 226 | `adduser` | data_ptr | uid or error | Add user. data_ptr: 4 u64 pointers [username, password, fullname, homedir]. Root only |
| 227 | `deluser` | uid | 0 or error | Delete user by UID. Root only |
| 228 | `listusers` | buf_ptr, buf_len | bytes_written | List all users ("uid:username\n" format) |
| 229 | `addgroup` | data_ptr (name_ptr, gid) | 0 or error | Add group. Root only |
| 230 | `delgroup` | gid | 0 or error | Delete group by GID. Root only |
| 231 | `listgroups` | buf_ptr, buf_len | bytes_written | List all groups |
| 232 | `getusername` | uid, buf_ptr, buf_len | bytes_written | Get username for UID |
| 233 | `set_identity` | uid | 0 or error | Set uid/gid on calling process. Root only |
| 234 | `chpasswd` | data_ptr | 0 or error | Change password. data_ptr: 3 u64 pointers [username, old_pass, new_pass] |

## App Permissions

Runtime per-user, per-app permission management. Apps declare capabilities in their `Info.conf`; sensitive capabilities require user consent via a permission dialog on first launch. Permissions are stored in `/System/users/perm/{uid}/{app_id}`.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 250 | `perm_check` | app_id_ptr, uid (0=caller) | granted_bitmask or 0xFFFFFFFF | Check stored permissions for an app. Returns granted capability bitmask, or `u32::MAX` if no permission file exists |
| 251 | `perm_store` | app_id_ptr, granted, uid (0=caller) | 0 or error | Store granted permissions for an app. Requires `CAP_MANAGE_PERMS` |
| 252 | `perm_list` | buf_ptr, buf_size | entry_count | List all apps with stored permissions for caller's uid. Format: `"app_id\x1Fgranted_hex\n"` per entry. Requires `CAP_MANAGE_PERMS` |
| 253 | `perm_delete` | app_id_ptr | 0 or error | Delete stored permissions for an app. Requires `CAP_MANAGE_PERMS` |
| 254 | `perm_pending_info` | buf_ptr, buf_size | bytes_written | Read pending permission info from current thread. Format: `"app_id\x1Fapp_name\x1Fcaps_hex\x1Fbundle_path"`. Set by kernel when `spawn()` returns `PERM_NEEDED` |

### Capability Bits

| Bit | Name | Sensitive | Description |
|-----|------|-----------|-------------|
| 0 | `FILESYSTEM` | Yes | Read and write files |
| 1 | `NETWORK` | Yes | Send and receive network data |
| 2 | `AUDIO` | Yes | Play sounds and music |
| 3 | `DISPLAY` | Yes | Control display settings |
| 4 | `DEVICE` | Yes | Access hardware devices |
| 5 | `PROCESS` | Yes | Start and stop processes |
| 6 | `SYSTEM` | Yes | Manage system settings |
| 7 | `DLL` | No | Load shared libraries (auto-granted) |
| 8 | `THREAD` | No | Create threads (auto-granted) |
| 9 | `SHM` | No | Shared memory (auto-granted) |
| 10 | `EVENT` | No | Event bus (auto-granted) |
| 11 | `PIPE` | No | Named pipes (auto-granted) |
| 12 | `COMPOSITOR` | Yes | Direct compositor access |
| 13 | `MANAGE_PERMS` | — | Manage permission files (kernel allowlist only) |

### Permission Flow

1. `SYS_SPAWN` for a `.app` bundle → kernel reads `Info.conf` → checks `/System/users/perm/{uid}/{app_id}`
2. If no permission file exists and app requests sensitive capabilities → returns `PERM_NEEDED` (`0xFFFFFFFD`)
3. Stdlib `spawn()` detects `PERM_NEEDED`, reads pending info via `perm_pending_info`, launches `/System/permdialog`
4. PermissionDialog shows user-friendly consent dialog → user grants/denies → calls `perm_store`
5. Stdlib retries `spawn()` — kernel finds permission file, intersects declared caps with granted caps

---

## POSIX File Descriptor Operations

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 157 | `pipe_bytes_available` | fd | bytes, 0, `u32::MAX-1`, or `u32::MAX` | Non-blocking pipe poll by FD number. Returns: `>0`=bytes ready, `0`=pipe open but empty, `u32::MAX-1`=EOF (write end closed + empty), `u32::MAX`=FD is not a pipe read-end (regular file/Tty, libc `poll()` treats these as always readable) |
| 240 | `pipe2` | pipefd_ptr (int[2]), flags | 0 or error | Create anonymous pipe. Writes [read_fd, write_fd] to pipefd_ptr. Flags: 0x10=O_CLOEXEC |
| 241 | `dup` | old_fd | new_fd or error | Duplicate file descriptor, returns lowest available FD |
| 242 | `dup2` | old_fd, new_fd | new_fd or error | Duplicate old_fd to new_fd; closes new_fd first if open |
| 243 | `fcntl` | fd, cmd, arg | result or error | File control. cmd: 0=F_DUPFD, 1=F_GETFD, 2=F_SETFD, 3=F_GETFL, 4=F_SETFL, 1030=F_DUPFD_CLOEXEC |

> **FD limits**: Each process has up to **256** open file descriptors (FDs 0–255). Socket FDs start at 256 (`SOCKET_FD_BASE`) to avoid namespace collision with file FDs. The global open-file table supports **1024** concurrent open slots across all processes.

## POSIX Signals

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 244 | `sigaction` | sig, handler_addr | old_handler | Set or query signal handler. handler: 0=SIG_DFL, 1=SIG_IGN, or user function address. SIGKILL/SIGSTOP cannot be caught |
| 245 | `sigprocmask` | how, set | old_mask | Modify signal mask. how: 0=SIG_BLOCK, 1=SIG_UNBLOCK, 2=SIG_SETMASK. SIGKILL/SIGSTOP cannot be blocked |
| 246 | `sigreturn` | — | — | Return from signal handler (called by trampoline, not user code). Restores saved register context |

## Crash Diagnostics

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 260 | `get_crash_info` | tid, buf_ptr, buf_size | bytes_written or 0 | Get crash report for a terminated thread. Copies raw CrashReport struct to user buffer |

## Disk / Partition Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 270 | `disk_list` | buf_ptr, buf_size | device_count | List disk devices |
| 271 | `disk_partitions` | disk_id, buf_ptr, buf_size | partition_count | List partitions on a disk |
| 272 | `disk_read` | device_id, lba, count, buf_ptr, buf_size | bytes_read or error | Read sectors by LBA |
| 273 | `disk_write` | device_id, lba, count, buf_ptr, buf_size | bytes_written or error | Write sectors by LBA |
| 274 | `partition_create` | disk_id, entry_ptr, entry_size | 0 or error | Create a new partition entry |
| 275 | `partition_delete` | disk_id, index | 0 or error | Delete a partition |
| 276 | `partition_rescan` | disk_id | 0 or error | Rescan disk partitions |
