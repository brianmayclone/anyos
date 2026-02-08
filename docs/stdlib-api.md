# anyOS Standard Library API Reference

The **anyos_std** crate is the standard library for user-space Rust programs on anyOS. It provides syscall wrappers, formatted I/O, memory allocation, and an entry point macro -- everything needed to build `#![no_std]` applications.

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
- [ui::window -- Window Management](#uiwindow----window-management)
- [dll -- Dynamic Library Loading](#dll----dynamic-library-loading)
- [Syscall Numbers](#syscall-numbers)

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

**Example:**
```rust
use anyos_std::*;

println!("The answer is {}", 42);
print!("No newline here");
println!();
```

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
| `sbrk` | `fn sbrk(increment: i32) -> u32` | Grow/shrink heap. Returns new program break address. |
| `spawn` | `fn spawn(path: &str, args: &str) -> u32` | Spawn new process from filesystem path. Returns TID or `u32::MAX` on error. |
| `spawn_piped` | `fn spawn_piped(path: &str, args: &str, pipe_id: u32) -> u32` | Spawn process with stdout redirected to a pipe. `pipe_id=0` means no pipe. |
| `waitpid` | `fn waitpid(tid: u32) -> u32` | Block until thread terminates. Returns exit code. |
| `try_waitpid` | `fn try_waitpid(tid: u32) -> u32` | Non-blocking wait. Returns exit code, `STILL_RUNNING`, or `u32::MAX` (not found). |
| `kill` | `fn kill(tid: u32) -> u32` | Kill a thread. Returns 0 on success, `u32::MAX` on failure. |
| `getargs` | `fn getargs(buf: &mut [u8]) -> usize` | Get raw command-line arguments (includes argv[0]). |
| `args` | `fn args(buf: &mut [u8; 256]) -> &str` | Get arguments, skipping program name. |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `STILL_RUNNING` | `u32::MAX - 1` | Return from `try_waitpid()` when thread is still alive |

### Example: Spawning a Child Process

```rust
use anyos_std::*;

fn main() {
    let tid = process::spawn("/bin/ls", "ls /bin");
    if tid != u32::MAX {
        let exit_code = process::waitpid(tid);
        println!("Child exited with code {}", exit_code);
    }
}
```

### Example: Non-blocking Wait with Pipe

```rust
use anyos_std::*;

fn main() {
    let pipe = ipc::pipe_create("output");
    let tid = process::spawn_piped("/bin/ls", "ls /bin", pipe);

    let mut buf = [0u8; 1024];
    loop {
        let n = ipc::pipe_read(pipe, &mut buf);
        if n > 0 {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            print!("{}", s);
        }
        let status = process::try_waitpid(tid);
        if status != process::STILL_RUNNING {
            break;
        }
        process::yield_cpu();
    }
    ipc::pipe_close(pipe);
}
```

---

## `sys` -- System Information

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `time` | `fn time(buf: &mut [u8; 8]) -> u32` | Get current time. Writes `[year_lo, year_hi, month, day, hour, min, sec, 0]`. |
| `uptime` | `fn uptime() -> u32` | System uptime in PIT ticks (100 Hz). Divide by 100 for seconds. |
| `sysinfo` | `fn sysinfo(cmd: u32, buf: &mut [u8]) -> u32` | Query system info. cmd: 0=memory, 1=threads, 2=cpus. |
| `dmesg` | `fn dmesg(buf: &mut [u8]) -> u32` | Read kernel log buffer. Returns bytes written. |

### Example: Reading Current Time

```rust
use anyos_std::*;

fn main() {
    let mut buf = [0u8; 8];
    sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    println!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, buf[2], buf[3], buf[4], buf[5], buf[6]);
}
```

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
| `open` | `fn open(path: &str, flags: u32) -> u32` | Open file. Returns FD or error. |
| `close` | `fn close(fd: u32) -> u32` | Close file descriptor. |
| `read` | `fn read(fd: u32, buf: &mut [u8]) -> u32` | Read from FD. Returns bytes read. |
| `write` | `fn write(fd: u32, buf: &[u8]) -> u32` | Write to FD. Returns bytes written. |
| `lseek` | `fn lseek(fd: u32, offset: i32, whence: u32) -> u32` | Seek within file. Returns new position. |
| `readdir` | `fn readdir(path: &str, buf: &mut [u8]) -> u32` | List directory. Returns entry count or `u32::MAX`. |
| `stat` | `fn stat(path: &str, buf: &mut [u32; 2]) -> u32` | File status. Writes `[type, size]`. |
| `fstat` | `fn fstat(fd: u32, buf: &mut [u32; 3]) -> u32` | FD status. Writes `[type, size, position]`. |
| `mkdir` | `fn mkdir(path: &str) -> u32` | Create directory. 0 on success. |
| `unlink` | `fn unlink(path: &str) -> u32` | Delete file. 0 on success. |
| `truncate` | `fn truncate(path: &str) -> u32` | Truncate file to zero. 0 on success. |
| `getcwd` | `fn getcwd(buf: &mut [u8]) -> u32` | Get current working directory. |
| `isatty` | `fn isatty(fd: u32) -> u32` | Check if FD is a terminal. 1=yes, 0=no. |

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

### Example: Reading a File

```rust
use anyos_std::*;

fn main() {
    let fd = fs::open("/bin/hello", 0); // 0 = read-only
    if fd == u32::MAX {
        println!("Failed to open file");
        return;
    }

    let mut buf = [0u8; 256];
    let n = fs::read(fd, &mut buf);
    println!("Read {} bytes", n);
    fs::close(fd);
}
```

### Example: Writing a File

```rust
use anyos_std::*;

fn main() {
    let fd = fs::open("/tmp/test.txt", fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, b"Hello, world!\n");
        fs::close(fd);
    }
}
```

---

## `net` -- Networking

### IP Configuration

| Function | Signature | Description |
|----------|-----------|-------------|
| `get_config` | `fn get_config(buf: &mut [u8; 24]) -> u32` | Get network config: `[ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]` |
| `set_config` | `fn set_config(buf: &[u8; 16]) -> u32` | Set network config: `[ip:4, mask:4, gw:4, dns:4]` |
| `dhcp` | `fn dhcp(buf: &mut [u8; 16]) -> u32` | Auto-configure via DHCP. 0 on success. |

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

### Example: DNS Lookup and Ping

```rust
use anyos_std::*;

fn main() {
    let mut ip = [0u8; 4];
    if net::dns("example.com", &mut ip) == 0 {
        println!("Resolved to {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
        let rtt = net::ping(&ip, 1, 500);
        if rtt != u32::MAX {
            println!("Ping: {} ms", rtt * 10);
        }
    }
}
```

### Example: HTTP GET via TCP

```rust
use anyos_std::*;

fn main() {
    let ip = [93, 184, 216, 34]; // example.com
    let sock = net::tcp_connect(&ip, 80, 5000);
    if sock == u32::MAX { return; }

    let request = b"GET / HTTP/1.0\r\nHost: example.com\r\n\r\n";
    net::tcp_send(sock, request);

    let mut buf = [0u8; 4096];
    loop {
        let n = net::tcp_recv(sock, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        print!("{}", s);
    }
    net::tcp_close(sock);
}
```

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
| `evt_chan_emit` | `fn evt_chan_emit(channel_id: u32, event: &[u32; 5])` | Emit event to channel. |
| `evt_chan_poll` | `fn evt_chan_poll(channel_id: u32, sub_id: u32, buf: &mut [u32; 5]) -> bool` | Poll next event. |
| `evt_chan_unsubscribe` | `fn evt_chan_unsubscribe(channel_id: u32, sub_id: u32)` | Unsubscribe. |
| `evt_chan_destroy` | `fn evt_chan_destroy(channel_id: u32)` | Destroy channel. |

---

## `ui::window` -- Window Management

### Window Event Types

| Constant | Value | Description |
|----------|-------|-------------|
| `EVENT_KEY_DOWN` | `1` | Key pressed. `p2` = key code. |
| `EVENT_KEY_UP` | `2` | Key released. |
| `EVENT_RESIZE` | `3` | Window resized. `p1` = new width, `p2` = new height. |
| `EVENT_MOUSE_DOWN` | `4` | Mouse button pressed. `p1` = x, `p2` = y. |
| `EVENT_MOUSE_UP` | `5` | Mouse button released. |
| `EVENT_MOUSE_MOVE` | `6` | Mouse moved. `p1` = x, `p2` = y. |
| `EVENT_MOUSE_SCROLL` | `7` | Mouse scroll. `p1` = dz (signed). |

### Window Creation Flags

| Constant | Value | Description |
|----------|-------|-------------|
| `WIN_FLAG_NOT_RESIZABLE` | `0x01` | Disallow window resizing |
| `WIN_FLAG_BORDERLESS` | `0x02` | No title bar or border |
| `WIN_FLAG_ALWAYS_ON_TOP` | `0x04` | Stay above all other windows |

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `create` | `fn create(title: &str, x: u16, y: u16, w: u16, h: u16) -> u32` | Create window. Returns window_id or `u32::MAX`. |
| `create_ex` | `fn create_ex(title: &str, x: u16, y: u16, w: u16, h: u16, flags: u32) -> u32` | Create window with flags. |
| `destroy` | `fn destroy(window_id: u32) -> u32` | Destroy window. |
| `set_title` | `fn set_title(window_id: u32, title: &str) -> u32` | Update title bar text. |
| `get_event` | `fn get_event(window_id: u32, event: &mut [u32; 5]) -> u32` | Poll event. 1=received, 0=none. |
| `get_size` | `fn get_size(window_id: u32) -> Option<(u32, u32)>` | Get content area size. |
| `fill_rect` | `fn fill_rect(window_id: u32, x: i16, y: i16, w: u16, h: u16, color: u32) -> u32` | Fill rectangle with ARGB color. |
| `draw_text` | `fn draw_text(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32` | Draw proportional text. |
| `draw_text_mono` | `fn draw_text_mono(window_id: u32, x: i16, y: i16, color: u32, text: &str) -> u32` | Draw monospace text (8x16). |
| `blit` | `fn blit(window_id: u32, x: i16, y: i16, w: u16, h: u16, data: &[u32]) -> u32` | Blit ARGB pixel array. |
| `present` | `fn present(window_id: u32) -> u32` | Flush to compositor. **Required after drawing.** |
| `list_windows` | `fn list_windows(buf: &mut [u8]) -> u32` | List open windows. |
| `focus` | `fn focus(window_id: u32) -> u32` | Focus/raise window. |
| `screen_size` | `fn screen_size() -> (u32, u32)` | Get screen dimensions. |
| `set_resolution` | `fn set_resolution(width: u32, height: u32) -> bool` | Change display resolution. |
| `list_resolutions` | `fn list_resolutions() -> Vec<(u32, u32)>` | List supported resolutions. |
| `gpu_name` | `fn gpu_name() -> String` | Get GPU driver name. |

### Color Format

Colors are 32-bit ARGB: `0xAARRGGBB`

| Example | Value |
|---------|-------|
| Opaque black | `0xFF000000` |
| Opaque white | `0xFFFFFFFF` |
| Opaque red | `0xFFFF0000` |
| 50% transparent blue | `0x800000FF` |

### Example: Simple Window

```rust
#![no_std]
#![no_main]

use anyos_std::*;

anyos_std::entry!(main);

fn main() {
    let win = ui::window::create("My App", 100, 100, 400, 300);
    if win == u32::MAX { return; }

    let mut event = [0u32; 5];
    loop {
        // Process events
        while ui::window::get_event(win, &mut event) == 1 {
            if event[0] == 0 { // Window closed
                ui::window::destroy(win);
                return;
            }
        }

        // Draw
        ui::window::fill_rect(win, 0, 0, 400, 300, 0xFF1E1E1E);
        ui::window::draw_text(win, 20, 20, 0xFFE6E6E6, "Hello, World!");
        ui::window::present(win);

        process::yield_cpu();
    }
}
```

---

## `dll` -- Dynamic Library Loading

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `dll_load` | `fn dll_load(path: &str) -> u32` | Load DLL by path. Returns base virtual address or 0 on failure. |

DLLs are loaded at fixed virtual addresses (starting at `0x04000000`). The main DLL is `uisys.dll`, which provides UI components. See the [uisys API reference](uisys-api.md) for details.

---

## Syscall Numbers

All syscalls use `int 0x80` with EAX = syscall number and EBX-EDI for arguments.

### Process (1-13, 27-29)

| Number | Name | Description |
|--------|------|-------------|
| 1 | SYS_EXIT | Terminate process |
| 6 | SYS_GETPID | Get thread ID |
| 7 | SYS_YIELD | Yield CPU |
| 8 | SYS_SLEEP | Sleep (ms) |
| 9 | SYS_SBRK | Grow heap |
| 12 | SYS_WAITPID | Wait for thread |
| 13 | SYS_KILL | Kill thread |
| 27 | SYS_SPAWN | Spawn process |
| 28 | SYS_GETARGS | Get arguments |
| 29 | SYS_TRY_WAITPID | Non-blocking wait |

### Filesystem (2-5, 23-25, 90-92, 105-108)

| Number | Name | Description |
|--------|------|-------------|
| 2 | SYS_WRITE | Write to FD |
| 3 | SYS_READ | Read from FD |
| 4 | SYS_OPEN | Open file |
| 5 | SYS_CLOSE | Close FD |
| 23 | SYS_READDIR | List directory |
| 24 | SYS_STAT | File status |
| 25 | SYS_GETCWD | Get working directory |
| 90 | SYS_MKDIR | Create directory |
| 91 | SYS_UNLINK | Delete file |
| 92 | SYS_TRUNCATE | Truncate file |
| 105 | SYS_LSEEK | Seek in file |
| 106 | SYS_FSTAT | FD status |
| 108 | SYS_ISATTY | Check terminal |

### Time / System (30-33)

| Number | Name | Description |
|--------|------|-------------|
| 30 | SYS_TIME | Get time |
| 31 | SYS_UPTIME | Get uptime |
| 32 | SYS_SYSINFO | System info |
| 33 | SYS_DMESG | Kernel log |

### Networking (40-44, 100-104)

| Number | Name | Description |
|--------|------|-------------|
| 40 | SYS_NET_CONFIG | Network config |
| 41 | SYS_NET_PING | ICMP ping |
| 42 | SYS_NET_DHCP | DHCP config |
| 43 | SYS_NET_DNS | DNS resolve |
| 44 | SYS_NET_ARP | ARP table |
| 100 | SYS_TCP_CONNECT | TCP connect |
| 101 | SYS_TCP_SEND | TCP send |
| 102 | SYS_TCP_RECV | TCP receive |
| 103 | SYS_TCP_CLOSE | TCP close |
| 104 | SYS_TCP_STATUS | TCP status |

### IPC (45-49, 60-68)

| Number | Name | Description |
|--------|------|-------------|
| 45 | SYS_PIPE_CREATE | Create pipe |
| 46 | SYS_PIPE_READ | Read pipe |
| 47 | SYS_PIPE_CLOSE | Close pipe |
| 48 | SYS_PIPE_WRITE | Write pipe |
| 49 | SYS_PIPE_OPEN | Open pipe |
| 60 | SYS_EVT_SYS_SUBSCRIBE | Subscribe system events |
| 61 | SYS_EVT_SYS_POLL | Poll system events |
| 62 | SYS_EVT_SYS_UNSUBSCRIBE | Unsubscribe |
| 63 | SYS_EVT_CHAN_CREATE | Create channel |
| 64 | SYS_EVT_CHAN_SUBSCRIBE | Subscribe channel |
| 65 | SYS_EVT_CHAN_EMIT | Emit event |
| 66 | SYS_EVT_CHAN_POLL | Poll channel |
| 67 | SYS_EVT_CHAN_UNSUBSCRIBE | Unsubscribe channel |
| 68 | SYS_EVT_CHAN_DESTROY | Destroy channel |

### Window Manager (50-59, 70-72)

| Number | Name | Description |
|--------|------|-------------|
| 50 | SYS_WIN_CREATE | Create window |
| 51 | SYS_WIN_DESTROY | Destroy window |
| 52 | SYS_WIN_SET_TITLE | Set title |
| 53 | SYS_WIN_GET_EVENT | Poll event |
| 54 | SYS_WIN_FILL_RECT | Fill rectangle |
| 55 | SYS_WIN_DRAW_TEXT | Draw text |
| 56 | SYS_WIN_PRESENT | Flush to screen |
| 57 | SYS_WIN_GET_SIZE | Get size |
| 58 | SYS_WIN_DRAW_TEXT_MONO | Draw mono text |
| 59 | SYS_WIN_BLIT | Blit pixels |
| 70 | SYS_WIN_LIST | List windows |
| 71 | SYS_WIN_FOCUS | Focus window |
| 72 | SYS_SCREEN_SIZE | Screen dimensions |

### DLL (80)

| Number | Name | Description |
|--------|------|-------------|
| 80 | SYS_DLL_LOAD | Load DLL |

### Display (110-112)

| Number | Name | Description |
|--------|------|-------------|
| 110 | SYS_SET_RESOLUTION | Change resolution |
| 111 | SYS_LIST_RESOLUTIONS | List modes |
| 112 | SYS_GPU_INFO | GPU info |
