# anyOS Syscall Reference

Complete reference for all 238 system calls in anyOS. Syscalls are the interface between user-space programs and the kernel.

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
| 12 | `waitpid` | tid, child_tid_ptr, options | exit_code | Block until process exits; returns its exit code. tid=0xFFFFFFFF: wait for any child (writes actual child TID to child_tid_ptr if non-zero). options bit 0 = WNOHANG (return immediately if no child exited) |
| 13 | `kill` | tid, sig | 0 or error | Send signal to thread. sig=0 or 9→SIGKILL (force-kill), 20→SIGTSTP (stop), 18→SIGCONT (continue), others→queued for delivery |
| 29 | `try_waitpid` | tid | code, 0xFFFFFFFD, 0xFFFFFFFE, or 0xFFFFFFFF | Non-blocking: exit code if done, `STOPPED` (0xFFFFFFFD) if stopped by signal, `STILL_RUNNING` (0xFFFFFFFE) if alive, `NOT_FOUND` (0xFFFFFFFF) if invalid |
| 247 | `getppid` | — | parent_tid | Get parent process/thread ID |

## Process Spawning

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 27 | `spawn` | path_ptr, stdout_pipe, args_ptr, stdin_pipe | tid or 0xFFFFFFFF | Spawn process from filesystem path with optional pipe I/O redirection |
| 28 | `getargs` | buf_ptr, buf_size | bytes_written | Get command-line arguments string for current process |
| 314 | `detach` | child_tid | 0 or 0xFFFFFFFF | Detach a child process so it survives parent exit. Only the direct parent may detach. Sets the child's parent_tid to 0, exempting it from cascade-kill on parent termination. Returns 0 on success, 0xFFFFFFFF if child not found or not owned by caller |
| 710 | `lxe_spawn` | path_ptr, args_ptr | tid or 0xFFFFFFFF | Spawn a Linux x86_64 ELF through the LXE ABI layer |
| 711 | `wxe_spawn` | path_ptr, args_ptr | tid or 0xFFFFFFFF | Spawns a Windows x86_64 PE through WXE. Current tier supports PE32+ console image loading, WXE System32 DLL import mapping, fixed PEB/process parameters, name/ordinal export lookup, PE forwarders and first fd-backed console/file APIs; TLS callbacks still fail closed |

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
| 109 | `statfs` | path_ptr, path_len, buf_ptr | 0 or error | Get filesystem statistics. Output: 24 bytes [total_bytes:u64, used_bytes:u64, free_bytes:u64] LE |
| 224 | `chmod` | path_ptr, mode (u16) | 0 or error | Change file permission mode (owner or root only) |
| 225 | `chown` | path_ptr, uid, gid | 0 or error | Change file owner/group (root only) |

## Mount System

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 93 | `mount` | mount_path_ptr, device_path_ptr, fs_type | 0 or error | Mount filesystem. fs_type: 0=FAT/exFAT, 1=ISO9660, 4=NTFS, 5=SMB, 6=CoreFS. Device string is numeric device_id for FAT/exFAT/CoreFS, device path for ISO9660, or `//ip/share` for SMB. |
| 94 | `umount` | mount_path_ptr | 0 or error | Unmount filesystem |
| 95 | `list_mounts` | buf_ptr, buf_len | bytes_written | List mounted filesystems ("mount_path\tfs_type\n" format) |

## Networking — Configuration & Diagnostics

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 40 | `net_config` | cmd, buf_ptr | status | cmd: 0=get, 1=set, 2=disable, 3=enable, 4=query_enabled, 5=query_available, 6=reload_hosts, 13=flush_dns_cache. Buffer: 24 bytes [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1] |
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
| 134 | `tcp_list` | buf_ptr, buf_size | entry_count | List all active TCP connections. Each entry: 16 bytes [local_ip:4, local_port:u16, remote_ip:4, remote_port:u16, state:u8, owner_tid:u8, recv_buf_len:u16] |
| 136 | `tcp_accept_nowait` | listener_id, result_ptr | 0 or 0xFFFFFFFF | Non-blocking accept. Returns immediately: connection if pending, or 0xFFFFFFFF if none ready. Same result format as `tcp_accept` |

## Networking — UDP

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 150 | `udp_bind` | port | 0 or error | Bind to UDP port, create receive queue |
| 151 | `udp_unbind` | port | 0 | Release UDP port binding |
| 152 | `udp_sendto` | params_ptr | bytes_sent or error | Send datagram. Params: 20 bytes [dst_ip:4, dst_port:u32, src_port:u32, data_ptr:u32, data_len:u32] |
| 153 | `udp_recvfrom` | port, buf_ptr, buf_len | total_bytes | Receive datagram. Header: [src_ip:4, src_port:u16, payload_len:u16] + payload |
| 154 | `udp_set_opt` | port, opt, val | 0 or error | Set socket option. opt: 1=SO_BROADCAST, 2=SO_RCVTIMEO (ms, 0=non-blocking) |
| 155 | `udp_list` | buf_ptr, max_entries | entry_count | List all bound UDP ports. Each entry: 8 bytes [port:u16, owner_tid:u16, recv_queue_len:u16, pad:u16] |
| 156 | `net_stats` | buf_ptr, buf_size | 0 or error | Get network protocol statistics. Output: 104 bytes [rx/tx packets, bytes, errors (each u64), TCP counters] |

## Networking — WiFi

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 158 | `wifi` | cmd, buf_ptr, buf_len | varies | WiFi management. cmd: 0=is_available (returns 1/0), 1=get_state (0=disconnected, 1=scanning, 2=associating, 3=authenticating, 4=connected), 2=start_scan, 3=read_scan_results (each entry 48 bytes: [bssid:6, ssid:32, ssid_len:1, channel:1, rssi:1, security:1, pad:6]), 4=connect (buf: [ssid_len:1, ssid:32, pw_len:1, pw:64]), 5=disconnect, 6=get_connection_status (48-byte struct) |

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
| 32 | `sysinfo` | cmd, buf_ptr, buf_size | varies | cmd: 0=memory, 1=threads, 2=cpus, 3=cpu_load, 4=hardware, 5=cpu_power, 6=cpu_frequency. Memory returns u32 words: total/free frames, heap used/total, swap total/free pages, swap areas |
| 33 | `dmesg` | buf_ptr, buf_size | bytes_written | Read kernel log ring buffer |
| 34 | `tick_hz` | — | hz | Get PIT tick frequency in Hz |
| 35 | `uptime_ms` | — | ms | System uptime in milliseconds (TSC-based, sub-ms precision) |
| 36 | `sleep_us` | microseconds | 0 | Sleep for N microseconds (high-resolution sleep) |
| 37 | `set_time` | buf_ptr (8 bytes) | 0 or error | Set RTC date/time. Input: [year_lo, year_hi, month, day, hour, min, sec, 0]. Year must be 2000–2099. Writes directly to CMOS RTC registers |

## Swap Control

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 633 | `swapon` | path_ptr, flags | 0 or error | Enable an existing regular file as swap backing store |
| 634 | `swapoff` | path_ptr | 0 or error | Disable a swap file when no swap slots are in use |

`swapon` and `swapoff` are mechanism-only syscalls. They do not read system
configuration and do not create or resize swap files. Boot-time swap policy is
owned by `/System/init`, which reads `system/kernel/swap/*` from `confd`,
prepares the configured file, then calls `swapon`. See
[Kernel Configuration](kernel-config.md).

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
| 259 | `grant_framebuffer` | target_tid, out_info_ptr | 0 or error | Map GPU framebuffer into target app's address space at 0x19000000 for fullscreen direct access. Writes [fb_va, width, height, pitch] to out_info_ptr (compositor only) |
| 261 | `revoke_framebuffer` | target_tid | 0 or error | Unmap the framebuffer from a target app's address space (compositor only). Removes mapping created by `grant_framebuffer` |

## Session Host

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 255 | `register_sessionhost` | — | 0 or error | Register as session host (first caller wins). Session host manages app launching and permission dialogs |

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

### Signal Numbers

| Signal | # | Default Action | Description |
|--------|---|---------------|-------------|
| SIGHUP | 1 | Terminate | Hangup |
| SIGINT | 2 | Terminate | Interrupt (Ctrl+C) |
| SIGQUIT | 3 | Terminate | Quit |
| SIGKILL | 9 | Terminate | Force kill (cannot be caught/blocked) |
| SIGPIPE | 13 | Terminate | Broken pipe |
| SIGALRM | 14 | Terminate | Alarm clock |
| SIGTERM | 15 | Terminate | Termination request |
| SIGCHLD | 17 | Ignore | Child status changed |
| SIGCONT | 18 | Continue | Resume stopped process (clears pending SIGTSTP/SIGSTOP) |
| SIGSTOP | 19 | Stop | Force stop (cannot be caught/blocked) |
| SIGTSTP | 20 | Stop | Terminal stop (Ctrl+Z) |
| SIGTTIN | 21 | Stop | Background read from terminal |
| SIGTTOU | 22 | Stop | Background write to terminal |

### Job Control Flow

```
Terminal Ctrl+Z → send_signal(tid, SIGTSTP=20)
  → Kernel sets ThreadState::Stopped, thread removed from scheduler
  → try_waitpid() returns STOPPED (0xFFFFFFFD)
  → Terminal moves process to stopped job list

Terminal "fg" → send_signal(tid, SIGCONT=18)
  → Kernel sets ThreadState::Ready, re-enqueues in run queue
  → Terminal re-attaches process as foreground

Terminal "bg" → send_signal(tid, SIGCONT=18)
  → Process resumes in background
```

### Thread States

| State | Value | Description |
|-------|-------|-------------|
| Ready | 0 | Eligible for scheduling |
| Running | 1 | Currently on CPU |
| Blocked | 2 | Waiting for event (waitpid, sleep) |
| Terminated | 3 | Exited, awaiting reaping |
| Stopped | 4 | Stopped by signal, not schedulable until SIGCONT |

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
| 277 | `disk_eject` | disk_id | 0 or error | Safely eject a removable disk: flush all dirty data, unmount all partitions, remove block device |

## Filesystem Sync

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 284 | `sync` | — | 0 | Flush all deferred filesystem metadata and dirty buffers to disk |
| 285 | `fsync` | fd | 0 or error | Flush deferred metadata for a specific open file descriptor to disk |

## Hostname

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 280 | `get_hostname` | buf_ptr, buf_size | bytes_written or error | Get system hostname. Copies null-terminated hostname string to buffer |
| 281 | `set_hostname` | name_ptr, name_len | 0 or error | Set system hostname. Max 63 bytes |

## Power Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 282 | `shutdown` | cmd | — | Power management. cmd: 0=shutdown, 1=reboot |
| 283 | `set_serial_verbose` | enable | 0 | Enable (1) or disable (0) verbose serial output. When enabled, driver and subsystem debug messages are printed to serial console |

## Debug / Trace (anyTrace)

Debugging syscalls for the anyTrace debugger. Allows attaching to processes, reading/writing memory and registers, setting breakpoints, and single-stepping.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 300 | `debug_attach` | tid | 0 or error | Attach debugger to target thread |
| 301 | `debug_detach` | tid | 0 or error | Detach debugger from target thread |
| 302 | `debug_suspend` | tid | 0 or error | Suspend target thread execution |
| 303 | `debug_resume` | tid | 0 or error | Resume suspended target thread |
| 304 | `debug_get_regs` | tid, buf_ptr, size | 0 or error | Read target thread register state into buffer |
| 305 | `debug_set_regs` | tid, buf_ptr, size | 0 or error | Write target thread register state from buffer |
| 306 | `debug_read_mem` | tid, addr, buf_ptr, len | bytes_read | Read memory from target process address space |
| 307 | `debug_write_mem` | tid, addr, buf_ptr, len | bytes_written | Write memory to target process address space |
| 308 | `debug_set_breakpoint` | tid, addr | 0 or error | Set hardware breakpoint at address |
| 309 | `debug_clr_breakpoint` | tid, addr | 0 or error | Clear hardware breakpoint at address |
| 310 | `debug_single_step` | tid | 0 or error | Single-step target thread (execute one instruction) |
| 311 | `debug_get_mem_map` | tid, buf_ptr, buf_size | entry_count | Get memory map of target process |
| 312 | `debug_wait_event` | tid, buf_ptr, timeout_ms | event_type or 0 | Wait for debug event (breakpoint hit, single-step complete) |
| 313 | `thread_info_ex` | tid, buf_ptr, buf_size | bytes_written | Get extended thread information (name, state, priority, CPU time) |

## GPU 3D Acceleration (SVGA3D / virgl)

Hardware-accelerated 3D graphics via VMware SVGA3D or virgl. Requires `gpu_has_accel`.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 512 | `gpu_3d_submit` | cmd_buf_ptr, cmd_size | 0 or error | Submit SVGA3D command buffer to GPU |
| 513 | `gpu_3d_query` | query_type | result | Query 3D GPU capability. query_type: 0=has_3d (returns 0/1), 1=hw_version (returns version number) |
| 514 | `gpu_3d_sync` | — | 0 or error | Synchronize GPU: wait for all submitted 3D commands to complete |
| 515 | `gpu_3d_surface_dma` | sid, buf_ptr, buf_len, width, height | 0 or error | DMA upload from system memory to GPU surface |
| 516 | `gpu_3d_surface_dma_read` | sid, buf_ptr, buf_len, width, height | 0 or error | DMA transfer from GPU surface to system memory (readback) |
| 517 | `gpu_query_type` | buf_ptr, buf_len | name_length | Get GPU driver type name string (e.g. "svga3d", "virgl", "none"). Writes null-terminated string to buffer |
| 518 | `gpu_3d_resource_create` | target, format, bind, width, height | resource_id or error | Create a virgl 3D resource. Uses defaults: depth=1, array_size=1, last_level=0, nr_samples=0, flags=0 |
| 519 | `gpu_3d_resource_destroy` | resource_id | 0 or error | Destroy a virgl 3D resource |

## Text Console I/O

Text-mode console syscalls for headless / nogui mode. These bypass the compositor and render directly to the framebuffer using a built-in font. Only functional on x86_64.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 290 | `con_write` | buf_ptr, len | bytes_written or error | Write UTF-8 text string to the text console. Max 64 KB per call |
| 291 | `con_read` | buf_ptr, buf_len | bytes_read or error | Read a line from keyboard with echo. Blocks until Enter. buf_len high bit (0x80000000): suppress echo (password mode). Max 4 KB |
| 292 | `con_poll_key` | — | codepoint or 0 | Non-blocking keyboard poll. Returns Unicode codepoint of next key press, or 0 if none pending. Bit 29 set = Ctrl modifier. Special: 0x03=Ctrl+C, 0x04=Ctrl+D |
| 293 | `con_get_size` | — | cols<<16 \| rows | Get console dimensions. High 16 bits = columns, low 16 bits = rows |
| 294 | `con_set_mode` | flags | previous_flags | Set console mode flags. Bit 0 (0x01): hide cursor. Bit 1 (0x02): disable auto-scroll. Returns previous flags |
| 295 | `con_resize` | cols<<16 \| rows | new_packed_size or 0 | Resize the text console. Recomputes cell width/height and repaints. Returns new packed size (same format as `con_get_size`), or 0 on error |

## Platform / Thermal / ACPI / I2C

Hardware platform syscalls. Only functional on x86_64.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 320 | `thermal_read` | buf_ptr, max_count | entry_count | Read all thermal sensors. Each entry: 8 bytes [source_type:u8 (0=IntelCpu, 1=AmdCpu, 2=Lm75, 3=Smbus), source_id:u8, pad:u16, temp_c_x10:i32 LE (0.1 C units)] |
| 321 | `thermal_cpu` | — | temp or 0xFFFFFFFF | Read CPU temperature in 0.1 C units. Returns `u32::MAX` if no CPU thermal sensor |
| 322 | `acpi_sleep` | state | 0 or error | Request ACPI sleep state. 0=S0 (no-op), 3=S3 (suspend to RAM), 4=S4 (hibernate), 5=S5 (power off) |
| 323 | `acpi_perf` | cmd, arg | result | CPU P-state frequency ratio. cmd: 0=get current ratio (returns ratio), 1=set ratio to arg (returns 0) |
| 324 | `i2c_read` | addr, reg | byte_value or error | Read byte from I2C device at 7-bit address, register offset. Returns 0-255 or `u32::MAX` on error |
| 325 | `i2c_write` | addr, reg, value | 0 or error | Write byte to I2C device at 7-bit address, register offset |
| 326 | `i2c_detect` | addr | 1 or 0 | Probe I2C device presence at 7-bit address. Returns 1 if ACK, 0 if absent |

## Monitor Detection

Monitor EDID and display mode enumeration. Only functional on x86_64.

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 327 | `monitor_count` | — | count | Get number of detected monitors |
| 328 | `monitor_info` | monitor_id, buf_ptr | 0 or error | Get packed monitor info (92 bytes). Fields: id, gpu_output, manufacturer, model_name, serial, dimensions_mm, gamma, native resolution, color depth, chromaticity |
| 329 | `monitor_edid` | monitor_id, buf_ptr, buf_len | bytes_written or 0 | Copy raw EDID bytes to user buffer |
| 330 | `monitor_modes` | monitor_id, buf_ptr, buf_len | mode_count | List supported display modes. Each mode: 16 bytes [width:u32, height:u32, refresh_hz_100:u32, flags:u32]. flags bit 0=preferred, bit 1=interlaced |

## Hardware Virtualization (VT-x / AMD-V)

Hardware-assisted virtualization syscalls for creating and managing virtual machines. Only functional on x86_64 with VT-x (Intel) or SVM (AMD-V) support.

### VM Lifecycle

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 600 | `vm_create` | — | vm_id or 0 | Create a new virtual machine. Returns VM ID, or 0 on failure |
| 601 | `vm_destroy` | vm_id | 0 or error | Destroy a virtual machine and free all resources |
| 602 | `vm_set_memory` | vm_id, slot, desc_ptr | 0 or error | Map a memory region into guest physical address space. desc_ptr points to {guest_phys:u64, size:u64, host_vaddr:u64}. Pages are translated UVA-to-HPA and programmed into EPT/NPT |
| 612 | `vm_set_cpuid` | vm_id, entries_ptr, count | 0 or error | Set CPUID emulation table. entries_ptr = array of CpuidEntry structs |
| 613 | `vm_hw_info` | — | type | Query hardware virtualization type: 0=none, 1=VMX (Intel VT-x), 2=SVM (AMD-V) |
| 614 | `vm_get_dirty_log` | vm_id, req_ptr | 0 or error | Get dirty-page bitmap for a memory slot. req_ptr points to {slot:u32, pad:u32, bitmap_ptr:u64, bitmap_size:u64}. One bit per 4 KB page |

### vCPU Management

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 603 | `vcpu_create` | vm_id, vcpu_id | 0 or error | Create a vCPU within a VM |
| 604 | `vcpu_run` | vm_id, vcpu_id, exit_info_ptr | 0 or error | Run vCPU until VM-exit. Writes VmExitInfo struct to exit_info_ptr |
| 615 | `vcpu_pause` | vm_id, vcpu_id | 0 or error | Pause a vCPU (stop execution until resumed) |
| 616 | `vcpu_resume` | vm_id, vcpu_id | 0 or error | Resume a previously paused vCPU |

### vCPU Register Access

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 605 | `vcpu_get_regs` | vm_id, vcpu_id, regs_ptr | 0 or error | Get guest general-purpose registers (GuestGprs struct) |
| 606 | `vcpu_set_regs` | vm_id, vcpu_id, regs_ptr | 0 or error | Set guest general-purpose registers from GuestGprs struct |
| 607 | `vcpu_get_sregs` | vm_id, vcpu_id, sregs_ptr | 0 or error | Get guest segment/control registers (GuestSregs struct) |
| 608 | `vcpu_set_sregs` | vm_id, vcpu_id, sregs_ptr | 0 or error | Set guest segment/control registers from GuestSregs struct |
| 617 | `vcpu_get_fpu` | vm_id, vcpu_id, fpu_ptr | 0 or error | Get guest FPU/SSE/AVX state (512 bytes, FXSAVE layout) |
| 618 | `vcpu_set_fpu` | vm_id, vcpu_id, fpu_ptr | 0 or error | Set guest FPU/SSE/AVX state from GuestFpuState struct |

### vCPU Event Injection

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 609 | `vcpu_inject_irq` | vm_id, vcpu_id, vector | 0 or error | Inject external interrupt into vCPU |
| 610 | `vcpu_inject_exception` | vm_id, vcpu_id, info | 0 or error | Inject exception into vCPU. info: low 8 bits = vector, bits 8-31 = error code |
| 611 | `vcpu_inject_nmi` | vm_id, vcpu_id | 0 or error | Inject NMI (non-maskable interrupt) into vCPU |

### vCPU Multi-Processor State

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 619 | `vcpu_get_mp_state` | vm_id, vcpu_id | state or error | Get vCPU MP state: 0=runnable, 1=uninitialized, 2=halted, 3=init. Returns `u32::MAX` on error |
| 620 | `vcpu_set_mp_state` | vm_id, vcpu_id, state | 0 or error | Set vCPU MP state value |

### Guest Virtual Address Translation

| # | Name | Args | Return | Description |
|---|------|------|--------|-------------|
| 621 | `vcpu_translate` | vm_id, vcpu_id, req_ptr | 0 | Translate guest virtual address to guest physical address. req_ptr points to {gva:u64, out_gpa:u64, out_valid:u32}. Always returns 0; caller checks out_valid (1=mapped, 0=not mapped) |
