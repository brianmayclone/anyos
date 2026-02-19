# anyOS Standard Library API Reference

The **anyos_std** crate is the standard library for user-space Rust programs on anyOS. It provides syscall wrappers, formatted I/O, memory allocation, an entry point macro, networking, IPC, audio, window management, and more -- everything needed to build `#![no_std]` applications.

**Crate:** `anyos_std` (version 0.1.0, edition 2021)

---

## Table of Contents

- [Getting Started](#getting-started)
- [Entry Point](#entry-point)
- [Re-exported Types](#re-exported-types)
- [io -- Printing](#io----printing)
- [process -- Process Management](#process----process-management)
- [sys -- System Information](#sys----system-information)
- [heap -- Memory Allocation](#heap----memory-allocation)
- [fs -- Filesystem Operations](#fs----filesystem-operations)
- [net -- Networking](#net----networking)
- [ipc -- Inter-Process Communication](#ipc----inter-process-communication)
- [env -- Environment Variables](#env----environment-variables)
- [users -- User & Group Management](#users----user--group-management)
- [kbd -- Keyboard Layouts](#kbd----keyboard-layouts)
- [crypto -- Cryptography](#crypto----cryptography)
- [audio -- Audio Playback](#audio----audio-playback)
- [dll -- Dynamic Library Loading](#dll----dynamic-library-loading)
- [args -- Argument Parser](#args----argument-parser)
- [anim -- Animation Engine](#anim----animation-engine)
- [permissions -- App Permissions](#permissions----app-permissions)
- [bundle -- App Bundle Discovery](#bundle----app-bundle-discovery)
- [icons -- Icon & MIME Type Lookup](#icons----icon--mime-type-lookup)
- [ui::window -- Window Management](#uiwindow----window-management)
- [ui::dialog -- Modal Dialogs](#uidialog----modal-dialogs)
- [ui::filedialog -- File & Folder Dialogs](#uifiledialog----file--folder-dialogs)

---

## Getting Started

### Minimum Program Template

```rust
#![no_std]
#![no_main]

use anyos_std::*;

anyos_std::entry!(main);

fn main() {
    println!("Hello from anyOS!");
}
```

### Program Requirements

1. **Cargo.toml**: Depend on `anyos_std`
2. **build.rs**: Set linker script (`-T stdlib/link.ld`)
3. Add to root `Cargo.toml` exclude list
4. Add to `CMakeLists.txt` via `add_rust_user_program()`

### Memory Layout

| Region | Address Range | Size |
|--------|--------------|------|
| Program text + data | `0x08000000`+ | Varies |
| Heap (grows up via sbrk) | After BSS -- `0x0BFEFFFF` | Up to ~64 MB |
| Stack (grows down) | `0x0BFF0000` -- `0x0C000000` | 64 KiB |

---

## Entry Point

### `entry!` Macro

```rust
anyos_std::entry!(main);
```

Generates the `_start` entry point for your program. It:
1. Declares `extern crate alloc` (enables `Vec`, `String`, `Box`, etc.)
2. Calls `heap::init()` to initialize the memory allocator
3. Calls your `main()` function
4. Calls `process::exit(0)` on return

The `main` function can return `()` or `u32` (exit code) via the `MainReturn` trait.

The stdlib also provides:
- **Panic handler**: Prints panic message to stdout, then calls `process::exit(1)`
- **Alloc error handler**: Prints "ALLOC ERROR: out of memory", then calls `process::exit(2)`

---

## Re-exported Types

These are re-exported from `alloc` for convenience:

```rust
pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};
```

---

## `io` -- Printing

### Macros

| Macro | Description |
|-------|-------------|
| `print!($($arg:tt)*)` | Print formatted text to stdout (no newline) |
| `println!()` | Print a newline |
| `println!($($arg:tt)*)` | Print formatted text with trailing newline |

Output goes to file descriptor 1 (stdout) via the `fs::write()` syscall.

---

## `process` -- Process Management

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `exit` | `fn exit(code: u32) -> !` | Terminate process with exit code. Never returns. |
| `getpid` | `fn getpid() -> u32` | Get current thread ID. |
| `yield_cpu` | `fn yield_cpu()` | Voluntary context switch to scheduler. |
| `sleep` | `fn sleep(ms: u32)` | Sleep for `ms` milliseconds. |
| `sbrk` | `fn sbrk(increment: i32) -> usize` | Grow/shrink heap. Returns new program break address. |
| `mmap` | `fn mmap(size: usize) -> *mut u8` | Map anonymous pages. Returns pointer or null. |
| `munmap` | `fn munmap(addr: *mut u8, size: usize) -> bool` | Unmap pages. Returns true on success. |
| `spawn` | `fn spawn(path: &str, args: &str) -> u32` | Spawn new process. Automatically shows permission dialog for `.app` bundles on first launch. Returns TID or `u32::MAX` on error. |
| `spawn_piped` | `fn spawn_piped(path: &str, args: &str, pipe_id: u32) -> u32` | Spawn with stdout redirected to a pipe. |
| `spawn_piped_full` | `fn spawn_piped_full(path: &str, args: &str, stdout_pipe: u32, stdin_pipe: u32) -> u32` | Spawn with both stdin and stdout pipes. |
| `waitpid` | `fn waitpid(tid: u32) -> u32` | Block until thread terminates. Returns exit code. |
| `try_waitpid` | `fn try_waitpid(tid: u32) -> u32` | Non-blocking wait. Returns exit code, `STILL_RUNNING`, or `u32::MAX`. |
| `kill` | `fn kill(tid: u32) -> u32` | Kill a thread. Returns 0 on success. |
| `getargs` | `fn getargs(buf: &mut [u8]) -> usize` | Get raw command-line arguments (includes argv[0]). |
| `args` | `fn args(buf: &mut [u8; 256]) -> &str` | Get arguments, skipping program name. |
| `thread_create` | `fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32` | Create a new thread. Returns TID. |
| `thread_create_with_priority` | `fn thread_create_with_priority(entry: fn(), stack_top: usize, name: &str, priority: u8) -> u32` | Create thread with priority. |
| `set_priority` | `fn set_priority(tid: u32, priority: u8) -> u32` | Set thread priority (0=highest, 255=lowest). |
| `get_capabilities` | `fn get_capabilities() -> u32` | Get capability flags for current process. |
| `getuid` | `fn getuid() -> u16` | Get current user ID. |
| `getgid` | `fn getgid() -> u16` | Get current group ID. |
| `authenticate` | `fn authenticate(username: &str, password: &str) -> bool` | Authenticate credentials. |
| `getusername` | `fn getusername(uid: u16, buf: &mut [u8]) -> u32` | Resolve UID to username. |
| `set_identity` | `fn set_identity(uid: u16) -> u32` | Switch to a different user identity. |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `STILL_RUNNING` | `u32::MAX - 1` | Return from `try_waitpid()` when thread is still alive |

---

## `sys` -- System Information

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `time` | `fn time(buf: &mut [u8; 8]) -> u32` | Get current time. Writes `[year_lo, year_hi, month, day, hour, min, sec, 0]`. |
| `uptime` | `fn uptime() -> u32` | System uptime in ticks. Divide by `tick_hz()` for seconds. |
| `tick_hz` | `fn tick_hz() -> u32` | Timer tick frequency (typically 1000 Hz). |
| `sysinfo` | `fn sysinfo(cmd: u32, buf: &mut [u8]) -> u32` | Query system info. cmd: 0=memory, 1=threads, 2=cpus. |
| `dmesg` | `fn dmesg(buf: &mut [u8]) -> u32` | Read kernel log buffer. Returns bytes written. |
| `boot_ready` | `fn boot_ready()` | Signal that boot is complete (compositor startup). |
| `capture_screen` | `fn capture_screen(buf: &mut [u32], info: &mut [u32; 2]) -> bool` | Capture framebuffer. info = [width, height]. |
| `set_critical` | `fn set_critical()` | Mark current thread as critical (won't be killed by OOM). |
| `random` | `fn random(buf: &mut [u8]) -> u32` | Fill buffer with random bytes. Returns bytes written. |
| `devlist` | `fn devlist(buf: &mut [u8]) -> u32` | List detected devices. Returns bytes written. |
| `pipe_list` | `fn pipe_list(buf: &mut [u8]) -> u32` | List active pipes. Returns bytes written. |

---

## `heap` -- Memory Allocation

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `init` | `fn init()` | Initialize heap allocator. Called automatically by `entry!` macro. |

### Implementation Details

- **Allocator type**: Bump allocator (stack-style)
- **Growth**: Via `process::sbrk()`, rounded to 4 KiB pages
- **Deallocation**: No-op (memory reclaimed when process exits)
- **Thread safety**: Single-threaded (one thread per process)

Once initialized, standard `alloc` types work: `Box`, `Vec`, `String`, `BTreeMap`, etc.

---

## `fs` -- Filesystem Operations

### Open Flags

| Constant | Value | Description |
|----------|-------|-------------|
| `O_WRITE` | `1` | Open for writing |
| `O_APPEND` | `2` | Append to file |
| `O_CREATE` | `4` | Create if doesn't exist |
| `O_TRUNC` | `8` | Truncate to zero length |

Combine with `|`: `fs::open("file.txt", fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC)`

### Seek Whence

| Constant | Value | Description |
|----------|-------|-------------|
| `SEEK_SET` | `0` | Seek from start |
| `SEEK_CUR` | `1` | Seek from current position |
| `SEEK_END` | `2` | Seek from end |

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `open` | `fn open(path: &str, flags: u32) -> u32` | Open file. Returns FD or `u32::MAX`. |
| `close` | `fn close(fd: u32) -> u32` | Close file descriptor. |
| `read` | `fn read(fd: u32, buf: &mut [u8]) -> u32` | Read from FD. Returns bytes read. |
| `write` | `fn write(fd: u32, buf: &[u8]) -> u32` | Write to FD. Returns bytes written. |
| `lseek` | `fn lseek(fd: u32, offset: i32, whence: u32) -> u32` | Seek within file. Returns new position. |
| `readdir` | `fn readdir(path: &str, buf: &mut [u8]) -> u32` | List directory. Returns entry count or `u32::MAX`. |
| `stat` | `fn stat(path: &str, buf: &mut [u32; 6]) -> u32` | File status. Returns 0 on success. |
| `lstat` | `fn lstat(path: &str, buf: &mut [u32; 6]) -> u32` | File status (no symlink follow). |
| `fstat` | `fn fstat(fd: u32, buf: &mut [u32; 3]) -> u32` | FD status. Writes `[type, size, position]`. |
| `mkdir` | `fn mkdir(path: &str) -> u32` | Create directory. 0 on success. |
| `unlink` | `fn unlink(path: &str) -> u32` | Delete file. 0 on success. |
| `truncate` | `fn truncate(path: &str) -> u32` | Truncate file to zero. 0 on success. |
| `getcwd` | `fn getcwd(buf: &mut [u8]) -> u32` | Get current working directory. |
| `chdir` | `fn chdir(path: &str) -> u32` | Change working directory. 0 on success. |
| `isatty` | `fn isatty(fd: u32) -> u32` | Check if FD is a terminal. 1=yes, 0=no. |
| `symlink` | `fn symlink(target: &str, link_path: &str) -> u32` | Create symbolic link. 0 on success. |
| `readlink` | `fn readlink(path: &str, buf: &mut [u8]) -> u32` | Read symlink target. Returns bytes written. |
| `mount` | `fn mount(mount_path: &str, device: &str, fs_type: u32) -> u32` | Mount filesystem. 0 on success. |
| `umount` | `fn umount(mount_path: &str) -> u32` | Unmount filesystem. 0 on success. |
| `list_mounts` | `fn list_mounts(buf: &mut [u8]) -> u32` | List mounted filesystems. |
| `chmod` | `fn chmod(path: &str, mode: u16) -> u32` | Change file permissions. 0 on success. |
| `chown` | `fn chown(path: &str, uid: u16, gid: u16) -> u32` | Change file owner/group. 0 on success. |

### Directory Entry Format

`readdir()` returns entries in a packed format, 64 bytes each:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 byte | Type (1=file, 2=directory) |
| 1 | 1 byte | Name length |
| 2 | 2 bytes | Padding |
| 4 | 4 bytes | File size |
| 8 | 56 bytes | Filename (null-terminated) |

### Standard File Descriptors

| FD | Stream | Destination |
|----|--------|-------------|
| 0 | stdin | Keyboard input |
| 1 | stdout | Serial output |
| 2 | stderr | Serial output |

---

## `net` -- Networking

### IP Configuration

| Function | Signature | Description |
|----------|-----------|-------------|
| `get_config` | `fn get_config(buf: &mut [u8; 24]) -> u32` | Get network config: `[ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]` |
| `set_config` | `fn set_config(buf: &[u8; 16]) -> u32` | Set network config: `[ip:4, mask:4, gw:4, dns:4]` |
| `dhcp` | `fn dhcp(buf: &mut [u8; 16]) -> u32` | Auto-configure via DHCP. 0 on success. |

### NIC Control

| Function | Signature | Description |
|----------|-----------|-------------|
| `disable_nic` | `fn disable_nic() -> u32` | Disable the network interface. |
| `enable_nic` | `fn enable_nic() -> u32` | Enable the network interface. |
| `is_nic_enabled` | `fn is_nic_enabled() -> bool` | Check if NIC is enabled. |
| `is_nic_available` | `fn is_nic_available() -> bool` | Check if NIC hardware is present. |

### ICMP, DNS, ARP

| Function | Signature | Description |
|----------|-----------|-------------|
| `ping` | `fn ping(ip: &[u8; 4], seq: u32, timeout: u32) -> u32` | ICMP ping. Returns RTT in ticks or `u32::MAX` on timeout. |
| `dns` | `fn dns(hostname: &str, result: &mut [u8; 4]) -> u32` | Resolve hostname to IP. 0 on success. |
| `arp` | `fn arp(buf: &mut [u8]) -> u32` | Get ARP table. Each entry 12 bytes: `[ip:4, mac:6, pad:2]`. |

### TCP

| Function | Signature | Description |
|----------|-----------|-------------|
| `tcp_connect` | `fn tcp_connect(ip: &[u8; 4], port: u16, timeout_ms: u32) -> u32` | Connect to TCP server. Returns socket_id or `u32::MAX`. |
| `tcp_send` | `fn tcp_send(socket_id: u32, data: &[u8]) -> u32` | Send data. Returns bytes sent or `u32::MAX`. |
| `tcp_recv` | `fn tcp_recv(socket_id: u32, buf: &mut [u8]) -> u32` | Receive data. 0=EOF, `u32::MAX`=error. |
| `tcp_close` | `fn tcp_close(socket_id: u32) -> u32` | Close connection. |
| `tcp_status` | `fn tcp_status(socket_id: u32) -> u32` | Connection state: 0=Closed, 2=Established, etc. |

### UDP

| Function | Signature | Description |
|----------|-----------|-------------|
| `udp_bind` | `fn udp_bind(port: u16) -> u32` | Bind to a UDP port. Returns 0 on success. |
| `udp_unbind` | `fn udp_unbind(port: u16) -> u32` | Release a UDP port. |
| `udp_sendto` | `fn udp_sendto(dst_ip: &[u8; 4], dst_port: u16, src_port: u16, data: &[u8], flags: u32) -> u32` | Send UDP datagram. |
| `udp_recvfrom` | `fn udp_recvfrom(port: u16, buf: &mut [u8]) -> u32` | Receive UDP datagram. Returns bytes read. |
| `udp_set_opt` | `fn udp_set_opt(port: u16, opt: u32, val: u32) -> u32` | Set UDP socket option. |

---

## `ipc` -- Inter-Process Communication

### Named Pipes

| Function | Signature | Description |
|----------|-----------|-------------|
| `pipe_create` | `fn pipe_create(name: &str) -> u32` | Create named pipe. Returns pipe_id (always > 0). |
| `pipe_open` | `fn pipe_open(name: &str) -> u32` | Open existing pipe. Returns pipe_id or 0 if not found. |
| `pipe_read` | `fn pipe_read(pipe_id: u32, buf: &mut [u8]) -> u32` | Read from pipe. 0=empty, `u32::MAX`=not found. |
| `pipe_write` | `fn pipe_write(pipe_id: u32, data: &[u8]) -> u32` | Write to pipe. Returns bytes written. |
| `pipe_close` | `fn pipe_close(pipe_id: u32) -> u32` | Close and destroy pipe. |

### System Event Bus

| Function | Signature | Description |
|----------|-----------|-------------|
| `evt_sys_subscribe` | `fn evt_sys_subscribe(filter: u32) -> u32` | Subscribe to system events. filter=0 for all. Returns sub_id. |
| `evt_sys_poll` | `fn evt_sys_poll(sub_id: u32, buf: &mut [u32; 5]) -> bool` | Poll next event. Returns true if received. |
| `evt_sys_unsubscribe` | `fn evt_sys_unsubscribe(sub_id: u32)` | Unsubscribe. |

### Module Event Channels

| Function | Signature | Description |
|----------|-----------|-------------|
| `evt_chan_create` | `fn evt_chan_create(name: &str) -> u32` | Create named channel. Returns channel_id. |
| `evt_chan_subscribe` | `fn evt_chan_subscribe(channel_id: u32, filter: u32) -> u32` | Subscribe. Returns sub_id. |
| `evt_chan_emit` | `fn evt_chan_emit(channel_id: u32, event: &[u32; 5])` | Emit event to all subscribers. |
| `evt_chan_emit_to` | `fn evt_chan_emit_to(channel_id: u32, sub_id: u32, event: &[u32; 5])` | Emit event to a specific subscriber. |
| `evt_chan_poll` | `fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: &mut [u32; 5]) -> bool` | Poll next event. |
| `evt_chan_unsubscribe` | `fn evt_chan_unsubscribe(channel_id: u32, sub_id: u32)` | Unsubscribe. |
| `evt_chan_destroy` | `fn evt_chan_destroy(channel_id: u32)` | Destroy channel. |

### Shared Memory (SHM)

| Function | Signature | Description |
|----------|-----------|-------------|
| `shm_create` | `fn shm_create(size: u32) -> u32` | Create shared memory region. Returns shm_id. |
| `shm_map` | `fn shm_map(shm_id: u32) -> u32` | Map SHM into process address space. Returns virtual address. |
| `shm_unmap` | `fn shm_unmap(shm_id: u32) -> u32` | Unmap SHM from process. |
| `shm_destroy` | `fn shm_destroy(shm_id: u32) -> u32` | Destroy SHM region. |

### Compositor-Privileged API

These functions are only available to the compositor process (registered via `register_compositor()`).

| Function | Signature | Description |
|----------|-----------|-------------|
| `register_compositor` | `fn register_compositor() -> u32` | Register as the system compositor. |
| `map_framebuffer` | `fn map_framebuffer() -> Option<FbMapInfo>` | Map the physical framebuffer. |
| `gpu_command` | `fn gpu_command(cmds: &[[u32; 9]]) -> u32` | Submit GPU commands (VMware SVGA II). |
| `input_poll` | `fn input_poll(buf: &mut [[u32; 5]]) -> u32` | Poll raw keyboard/mouse input. |
| `cursor_takeover` | `fn cursor_takeover() -> (i32, i32)` | Take control of cursor, returns position. |

### FbMapInfo

```rust
pub struct FbMapInfo {
    pub fb_addr: u32,   // Virtual address of framebuffer
    pub width: u32,     // Screen width in pixels
    pub height: u32,    // Screen height in pixels
    pub pitch: u32,     // Bytes per row
}
```

---

## `env` -- Environment Variables

Per-process key-value environment variable storage.

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `set` | `fn set(key: &str, value: &str) -> u32` | Set variable. 0 on success. |
| `unset` | `fn unset(key: &str) -> u32` | Remove variable. Pass empty value to `set`. |
| `get` | `fn get(key: &str, buf: &mut [u8]) -> u32` | Get variable value. Returns length or 0 if not found. |
| `list` | `fn list(buf: &mut [u8]) -> u32` | List all variables. Returns bytes written. |

### Example

```rust
use anyos_std::*;

fn main() {
    env::set("HOME", "/home/user");
    let mut buf = [0u8; 256];
    let len = env::get("HOME", &mut buf);
    if len > 0 {
        let val = core::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        println!("HOME={}", val);
    }
}
```

---

## `users` -- User & Group Management

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `adduser` | `fn adduser(username: &str, password: &str, fullname: &str, homedir: &str) -> u32` | Create user account. Returns UID or `u32::MAX`. |
| `chpasswd` | `fn chpasswd(username: &str, old_password: &str, new_password: &str) -> u32` | Change password. 0 on success. |
| `deluser` | `fn deluser(uid: u16) -> u32` | Delete user account. |
| `listusers` | `fn listusers(buf: &mut [u8]) -> u32` | List all users. Returns bytes written. |
| `addgroup` | `fn addgroup(name: &str, gid: u16) -> u32` | Create a group. |
| `delgroup` | `fn delgroup(gid: u16) -> u32` | Delete a group. |
| `listgroups` | `fn listgroups(buf: &mut [u8]) -> u32` | List all groups. Returns bytes written. |

User identity functions (`getuid`, `getgid`, `authenticate`, `getusername`, `set_identity`) are in the `process` module.

---

## `kbd` -- Keyboard Layouts

### Types

```rust
pub struct LayoutInfo {
    pub id: u32,         // Layout ID
    pub code: [u8; 8],   // Layout code (e.g. "en-us\0\0\0")
    pub label: [u8; 4],  // Short label (e.g. "US\0\0")
}
```

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `get_layout` | `fn get_layout() -> u32` | Get active keyboard layout ID. |
| `set_layout` | `fn set_layout(id: u32) -> u32` | Switch keyboard layout. |
| `list_layouts` | `fn list_layouts(buf: &mut [LayoutInfo]) -> u32` | List available layouts. Returns count. |
| `label_str` | `fn label_str(label: &[u8; 4]) -> &str` | Convert label bytes to string (trims nulls). |
| `code_str` | `fn code_str(code: &[u8; 8]) -> &str` | Convert code bytes to string (trims nulls). |

### Example

```rust
use anyos_std::kbd;

fn main() {
    let mut layouts = [kbd::LayoutInfo { id: 0, code: [0; 8], label: [0; 4] }; 16];
    let count = kbd::list_layouts(&mut layouts);
    for i in 0..count as usize {
        println!("{}: {} ({})", layouts[i].id,
            kbd::code_str(&layouts[i].code),
            kbd::label_str(&layouts[i].label));
    }
}
```

---

## `crypto` -- Cryptography

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `md5` | `fn md5(input: &[u8]) -> [u8; 16]` | Compute MD5 hash (16 raw bytes). |
| `md5_hex` | `fn md5_hex(input: &[u8]) -> [u8; 32]` | Compute MD5 hash as hex string bytes. |

### Example

```rust
use anyos_std::crypto;

let hash = crypto::md5_hex(b"hello");
let hex_str = core::str::from_utf8(&hash).unwrap_or("");
println!("MD5: {}", hex_str);
```

---

## `audio` -- Audio Playback

Audio output is 48 kHz, 16-bit signed stereo (native AC'97 format).

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `audio_write` | `fn audio_write(pcm_data: &[u8]) -> u32` | Write raw PCM data to audio output. Returns bytes accepted. |
| `audio_stop` | `fn audio_stop()` | Stop audio playback. |
| `audio_set_volume` | `fn audio_set_volume(vol: u8)` | Set master volume (0 = mute, 100 = max). |
| `audio_get_volume` | `fn audio_get_volume() -> u8` | Get current master volume (0-100). |
| `audio_is_playing` | `fn audio_is_playing() -> bool` | Check if audio playback is active. |
| `audio_is_available` | `fn audio_is_available() -> bool` | Check if audio hardware is available. |
| `play_wav` | `fn play_wav(data: &[u8]) -> Result<(), &'static str>` | Parse and play a WAV file from raw bytes. |

### PCM Format

Raw PCM data passed to `audio_write()` must be:
- **Sample rate:** 48,000 Hz
- **Bit depth:** 16-bit signed little-endian
- **Channels:** Stereo (interleaved L, R)
- **Frame size:** 4 bytes (2 bytes left + 2 bytes right)

### WAV Support

`play_wav()` handles format conversion automatically:
- **Input:** RIFF/WAVE PCM format (audio format tag 1)
- **Bit depths:** 8-bit unsigned, 16-bit signed
- **Channels:** Mono (duplicated to stereo) or stereo
- **Sample rate:** Any (resampled to 48 kHz via nearest-neighbor)

---

## `dll` -- Dynamic Library Loading

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `dll_load` | `fn dll_load(path: &str) -> u32` | Load DLL by path. Returns base virtual address or 0. |
| `set_dll_u32` | `fn set_dll_u32(dll_base: u64, offset: u32, value: u32) -> u32` | Write a u32 into DLL data section. |

DLLs are loaded at fixed virtual addresses (starting at `0x04000000`). See the dedicated DLL API docs for each library.

---

## `args` -- Argument Parser

A zero-allocation command-line argument parser.

### Types

```rust
pub struct ParsedArgs<'a> {
    pub positional: [&'a str; 8],  // Positional arguments
    pub pos_count: usize,          // Number of positional args
    // ... internal flag/option storage
}
```

### ParsedArgs Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `has` | `fn has(&self, flag: u8) -> bool` | Check if a boolean flag is set (e.g. `b'v'` for `-v`). |
| `opt` | `fn opt(&self, flag: u8) -> Option<&str>` | Get value of an option flag (e.g. `-o value`). |
| `opt_u32` | `fn opt_u32(&self, flag: u8, default: u32) -> u32` | Get option value as u32 with default. |
| `first_or` | `fn first_or(&self, default: &str) -> &str` | First positional arg or default. |
| `pos` | `fn pos(&self, idx: usize) -> Option<&str>` | Get positional argument by index. |

### Function

```rust
pub fn parse<'a>(raw: &'a str, opts_with_values: &[u8]) -> ParsedArgs<'a>
```

Parse a raw argument string. `opts_with_values` lists flags that expect a value.

### Example

```rust
use anyos_std::args;

fn main() {
    let mut buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut buf);
    let parsed = args::parse(raw, b"o"); // -o takes a value

    if parsed.has(b'v') {
        println!("Verbose mode");
    }
    if let Some(output) = parsed.opt(b'o') {
        println!("Output: {}", output);
    }
    let file = parsed.first_or("default.txt");
    println!("File: {}", file);
}
```

---

## `anim` -- Animation Engine

Tick-based animation system with easing functions.

### Easing

```rust
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
}
```

### Anim

A single animation interpolating between two values.

```rust
pub struct Anim {
    pub from: i32,
    pub to: i32,
    pub easing: Easing,
    // ... internal timing fields
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new(from: i32, to: i32, duration_ms: u32, easing: Easing) -> Self` | Create animation starting now. |
| `new_at` | `fn new_at(from: i32, to: i32, duration_ms: u32, easing: Easing, start: u32) -> Self` | Create animation at specific tick. |
| `progress` | `fn progress(&self, now_tick: u32) -> u32` | Get progress 0..65536 (16.16 fixed-point). |
| `value` | `fn value(&self, now_tick: u32) -> i32` | Get interpolated value at tick. |
| `done` | `fn done(&self, now_tick: u32) -> bool` | Check if animation is complete. |

### AnimSet

Manages multiple named animations.

```rust
pub struct AnimSet { /* ... */ }
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new() -> Self` | Create empty animation set. |
| `start` | `fn start(&mut self, id: u32, from: i32, to: i32, duration_ms: u32, easing: Easing)` | Start animation with ID. |
| `start_at` | `fn start_at(&mut self, id: u32, from: i32, to: i32, duration_ms: u32, easing: Easing, start: u32)` | Start at specific tick. |
| `value` | `fn value(&self, id: u32, now: u32) -> Option<i32>` | Get current value by ID. |
| `value_or` | `fn value_or(&self, id: u32, now: u32, default: i32) -> i32` | Get value or default. |
| `is_active` | `fn is_active(&self, id: u32, now: u32) -> bool` | Check if animation is running. |
| `has_active` | `fn has_active(&self, now: u32) -> bool` | Check if any animation is running. |
| `remove_done` | `fn remove_done(&mut self, now: u32)` | Remove completed animations. |
| `remove` | `fn remove(&mut self, id: u32)` | Remove animation by ID. |
| `len` | `fn len(&self) -> usize` | Number of active animations. |

### Utility

```rust
pub fn color_blend(c1: u32, c2: u32, t: u32) -> u32
```

Blend two ARGB colors. `t` is 0..65536 (0 = c1, 65536 = c2).

---

## `permissions` -- App Permissions

Runtime per-user, per-app permission management. Used by the PermissionDialog and Settings app.

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `PERM_NEEDED` | `u32::MAX - 2` | Sentinel returned by `spawn()` when the app needs permission approval |

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `perm_check` | `fn perm_check(app_id: &str, uid: u16) -> u32` | Check stored permissions. Returns granted bitmask or `u32::MAX` if not found. `uid=0` uses caller's uid. |
| `perm_store` | `fn perm_store(app_id: &str, granted: u32, uid: u16) -> bool` | Store granted permissions. Returns true on success. `uid=0` uses caller's uid. |
| `perm_list` | `fn perm_list(buf: &mut [u8]) -> u32` | List all apps with stored permissions. Writes `"app_id\x1Fgranted_hex\n"` entries. Returns entry count. |
| `perm_delete` | `fn perm_delete(app_id: &str) -> bool` | Delete stored permissions for an app. Returns true on success. |
| `perm_pending_info` | `fn perm_pending_info(buf: &mut [u8]) -> u32` | Read pending permission info from current thread. Returns bytes written (0 if none). |

### Permission Flow

The `spawn()` function in the `process` module automatically handles the permission flow:

1. If `spawn()` returns `PERM_NEEDED`, it reads pending info via `perm_pending_info()`
2. Launches `/System/permdialog` with the app's permission requirements
3. Waits for the dialog to complete (exit code 0 = user approved)
4. Retries the spawn — the kernel now finds the stored permission file

This is transparent to callers of `spawn()` — they simply see either a valid TID or an error.

---

## `bundle` -- App Bundle Discovery

Discover app bundle paths and metadata for `.app` directories.

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `bundle_path` | `fn bundle_path() -> Option<&'static str>` | Get the current app's bundle directory. |
| `resource_path` | `fn resource_path(name: &str) -> Option<String>` | Resolve a resource file path within the bundle. |
| `bundle_name` | `fn bundle_name() -> Option<String>` | Get the bundle display name. |
| `bundle_info` | `fn bundle_info(key: &str) -> Option<String>` | Read a key from the bundle's info file. |

---

## `icons` -- Icon & MIME Type Lookup

### Constants

| Constant | Value |
|----------|-------|
| `APP_ICONS_DIR` | `"/System/media/icons/apps"` |
| `DEFAULT_APP_ICON` | `"/System/media/icons/apps/default.ico"` |
| `DEFAULT_FILE_ICON` | `"/System/media/icons/default.ico"` |
| `FOLDER_ICON` | `"/System/media/icons/folder.ico"` |

### MimeDb

Database of file extension to application and icon mappings.

```rust
pub struct MimeDb { /* ... */ }
pub struct MimeEntry {
    pub ext: String,
    pub app: String,
    pub icon_path: String,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `load` | `fn load() -> Self` | Load MIME database from disk. |
| `lookup` | `fn lookup(&self, ext: &str) -> Option<&MimeEntry>` | Lookup by file extension. |
| `icon_for_ext` | `fn icon_for_ext(&self, ext: &str) -> &str` | Get icon path for extension. |
| `app_for_ext` | `fn app_for_ext(&self, ext: &str) -> Option<&str>` | Get default app for extension. |

### Utility Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `is_app_bundle` | `fn is_app_bundle(path: &str) -> bool` | Check if path is a `.app` bundle. |
| `app_bundle_name` | `fn app_bundle_name(bundle_path: &str) -> String` | Extract display name from bundle path. |
| `app_icon_path` | `fn app_icon_path(bin_path: &str) -> String` | Find icon for an application binary. |

---

## `ui::window` -- Window Management

### Event Types

| Constant | Value | Description |
|----------|-------|-------------|
| `EVENT_KEY_DOWN` | `1` | Key pressed. `p2` = key code. |
| `EVENT_KEY_UP` | `2` | Key released. |
| `EVENT_RESIZE` | `3` | Window resized. `p1` = width, `p2` = height. |
| `EVENT_MOUSE_DOWN` | `4` | Mouse button pressed. `p1` = x, `p2` = y. |
| `EVENT_MOUSE_UP` | `5` | Mouse button released. |
| `EVENT_MOUSE_MOVE` | `6` | Mouse moved. `p1` = x, `p2` = y. |
| `EVENT_MOUSE_SCROLL` | `7` | Mouse scroll. `p1` = dz (signed). |
| `EVENT_WINDOW_CLOSE` | `8` | Window close requested. |
| `EVENT_MENU_ITEM` | `9` | Menu item selected. `p1` = item_id. |

### Window Creation Flags

| Constant | Value | Description |
|----------|-------|-------------|
| `WIN_FLAG_BORDERLESS` | `0x01` | No title bar or border |
| `WIN_FLAG_NOT_RESIZABLE` | `0x02` | Disallow window resizing |
| `WIN_FLAG_ALWAYS_ON_TOP` | `0x04` | Stay above other windows |
| `WIN_FLAG_NO_CLOSE` | `0x08` | Hide close button |
| `WIN_FLAG_NO_MINIMIZE` | `0x10` | Hide minimize button |
| `WIN_FLAG_NO_MAXIMIZE` | `0x20` | Hide maximize button |
| `WIN_FLAG_SHADOW` | `0x40` | Enable window shadow |
| `WIN_FLAG_NO_MOVE` | `0x100` | Prevent window dragging |

### Font Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `FONT_REGULAR` | `0` | Regular weight |
| `FONT_BOLD` | `1` | Bold weight |
| `FONT_THIN` | `2` | Thin weight |
| `FONT_ITALIC` | `3` | Italic style |

### Menu Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `MENU_FLAG_DISABLED` | `0x01` | Greyed out, not clickable |
| `MENU_FLAG_SEPARATOR` | `0x02` | Separator line |
| `MENU_FLAG_CHECKED` | `0x04` | Checkmark visible |
| `APP_MENU_ABOUT` | `0xFFFE` | Standard "About" item ID |
| `APP_MENU_HIDE` | `0xFFFD` | Standard "Hide" item ID |
| `APP_MENU_QUIT` | `0xFFFF` | Standard "Quit" item ID |

### Window Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `create` | `fn create(title: &str, x: u16, y: u16, w: u16, h: u16) -> u32` | Create window. Returns window_id or `u32::MAX`. |
| `create_ex` | `fn create_ex(title: &str, x: u16, y: u16, w: u16, h: u16, flags: u32) -> u32` | Create window with flags. |
| `destroy` | `fn destroy(window_id: u32) -> u32` | Destroy window. |
| `set_title` | `fn set_title(window_id: u32, title: &str) -> u32` | Update title bar text. |
| `get_event` | `fn get_event(window_id: u32, event: &mut [u32; 5]) -> u32` | Poll event. 1=received, 0=none. |
| `get_size` | `fn get_size(window_id: u32) -> Option<(u32, u32)>` | Get content area size. |

### Drawing Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `fill_rect` | `fn fill_rect(win: u32, x: i16, y: i16, w: u16, h: u16, color: u32) -> u32` | Fill rectangle with ARGB color. |
| `fill_rounded_rect` | `fn fill_rounded_rect(win: u32, x: i16, y: i16, w: u16, h: u16, radius: u16, color: u32) -> u32` | Fill rounded rectangle. |
| `draw_text` | `fn draw_text(win: u32, x: i16, y: i16, color: u32, text: &str) -> u32` | Draw proportional text. |
| `draw_text_mono` | `fn draw_text_mono(win: u32, x: i16, y: i16, color: u32, text: &str) -> u32` | Draw monospace text (8x16). |
| `draw_text_ex` | `fn draw_text_ex(win: u32, x: i16, y: i16, color: u32, font_id: u16, size: u16, text: &str) -> u32` | Draw text with custom font and size. |
| `blit` | `fn blit(win: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32` | Blit ARGB pixel array (opaque). |
| `blit_alpha` | `fn blit_alpha(win: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32` | Blit ARGB pixel array (alpha blended). |
| `present` | `fn present(win: u32) -> u32` | Flush to compositor. **Required after drawing.** |

### Surface Access

| Function | Signature | Description |
|----------|-----------|-------------|
| `surface_ptr` | `fn surface_ptr(win: u32) -> *mut u32` | Get raw pixel buffer pointer. |
| `surface_info` | `fn surface_info(win: u32) -> Option<(*mut u32, u32, u32)>` | Get (pointer, width, height). |

### Display Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `screen_size` | `fn screen_size() -> (u32, u32)` | Get screen dimensions. |
| `set_resolution` | `fn set_resolution(w: u32, h: u32) -> bool` | Change display resolution. |
| `list_resolutions` | `fn list_resolutions() -> Vec<(u32, u32)>` | List supported resolutions. |
| `gpu_name` | `fn gpu_name() -> String` | Get GPU driver name. |
| `gpu_has_accel` | `fn gpu_has_accel() -> bool` | Check if GPU acceleration is available. |
| `set_wallpaper` | `fn set_wallpaper(path: &str) -> u32` | Set desktop wallpaper image. |
| `get_theme` | `fn get_theme() -> u32` | Get current UI theme ID. |
| `set_theme` | `fn set_theme(theme: u32)` | Set UI theme. |

### Window Management

| Function | Signature | Description |
|----------|-----------|-------------|
| `list_windows` | `fn list_windows(buf: &mut [u8]) -> u32` | List open windows. |
| `focus` | `fn focus(win: u32) -> u32` | Focus/raise window. |

### Font Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `font_load` | `fn font_load(path: &str) -> Option<u32>` | Load a font file. Returns font_id. |
| `font_unload` | `fn font_unload(font_id: u32)` | Unload a font. |
| `font_measure` | `fn font_measure(font_id: u16, size: u16, text: &str) -> (u32, u32)` | Measure text (width, height). |
| `font_render_buf` | `fn font_render_buf(font_id: u16, size: u16, buf: &mut [u32], buf_w: u32, buf_h: u32, x: i32, y: i32, color: u32, text: &str) -> u32` | Render text into pixel buffer. |

### Menu Bar

```rust
pub struct MenuBarBuilder { /* ... */ }
pub struct MenuBuilder { /* ... */ }
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `MenuBarBuilder::new` | `fn new() -> Self` | Create builder. |
| `MenuBarBuilder::menu` | `fn menu(self, title: &str) -> MenuBuilder` | Start a menu. |
| `MenuBarBuilder::build` | `fn build(&mut self) -> &[u8]` | Finalize and get binary data. |
| `MenuBuilder::item` | `fn item(self, id: u32, label: &str, flags: u32) -> Self` | Add menu item. |
| `MenuBuilder::separator` | `fn separator(self) -> Self` | Add separator. |
| `MenuBuilder::end_menu` | `fn end_menu(self) -> MenuBarBuilder` | End current menu. |

**Menu Functions:**

| Function | Signature | Description |
|----------|-----------|-------------|
| `set_menu` | `fn set_menu(win: u32, data: &[u8])` | Set window's menu bar. |
| `update_menu_item` | `fn update_menu_item(win: u32, item_id: u32, new_flags: u32)` | Update menu item flags. |
| `enable_menu_item` | `fn enable_menu_item(win: u32, item_id: u32)` | Enable a menu item. |
| `disable_menu_item` | `fn disable_menu_item(win: u32, item_id: u32)` | Disable a menu item. |

### Color Format

Colors are 32-bit ARGB: `0xAARRGGBB`

| Example | Value |
|---------|-------|
| Opaque black | `0xFF000000` |
| Opaque white | `0xFFFFFFFF` |
| Opaque red | `0xFFFF0000` |
| 50% transparent blue | `0x800000FF` |

---

## `ui::dialog` -- Modal Dialogs

### DialogType

```rust
pub enum DialogType {
    Info,
    Warning,
    Error,
    Success,
    Question,
}
```

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `show` | `fn show(parent: u32, dtype: DialogType, title: &str, message: &str, buttons: &[&str]) -> u32` | Show dialog with custom buttons. Returns button index. |
| `show_error` | `fn show_error(parent: u32, title: &str, msg: &str) -> u32` | Error dialog with OK button. |
| `show_warning` | `fn show_warning(parent: u32, title: &str, msg: &str) -> u32` | Warning dialog with OK button. |
| `show_info` | `fn show_info(parent: u32, title: &str, msg: &str) -> u32` | Info dialog with OK button. |
| `show_confirm` | `fn show_confirm(parent: u32, title: &str, msg: &str) -> u32` | Confirm dialog with OK/Cancel. |
| `show_success` | `fn show_success(parent: u32, title: &str, msg: &str) -> u32` | Success dialog with OK button. |

### Example

```rust
use anyos_std::ui::dialog;

let result = dialog::show_confirm(win, "Confirm", "Save changes?");
if result == 0 {
    // User clicked OK
}
```

---

## `ui::filedialog` -- File & Folder Dialogs

### FileDialogResult

```rust
pub enum FileDialogResult {
    Selected(String),
    Cancelled,
}
```

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `open_file` | `fn open_file(starting_path: &str) -> FileDialogResult` | Show file open dialog. |
| `open_folder` | `fn open_folder(starting_path: &str) -> FileDialogResult` | Show folder selection dialog. |
| `save_file` | `fn save_file(starting_path: &str, default_name: &str) -> FileDialogResult` | Show file save dialog. |
| `create_folder` | `fn create_folder(parent_path: &str) -> FileDialogResult` | Show create folder dialog. |

### Example

```rust
use anyos_std::ui::filedialog::{self, FileDialogResult};

match filedialog::open_file("/") {
    FileDialogResult::Selected(path) => println!("Selected: {}", path),
    FileDialogResult::Cancelled => println!("Cancelled"),
}
```
