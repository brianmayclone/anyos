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
- [log -- Structured Logging](#log----structured-logging)
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
- [hashmap -- Hash Map](#hashmap----hash-map)
- [collections -- Collections](#collections----collections)
- [json -- JSON Parser & Serializer](#json----json-parser--serializer)
- [xml -- XML Parser & Serializer](#xml----xml-parser--serializer)
- [error -- Error Types](#error----error-types)
- [path -- Path Helpers](#path----path-helpers)
- [fmt -- Formatting Helpers](#fmt----formatting-helpers)
- [i18n -- Internationalization](#i18n----internationalization)
- [shell -- Shell Parsing Library](#shell----shell-parsing-library)
- [debug -- Debugger API](#debug----debugger-api)
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

These are re-exported from `alloc` and internal modules for convenience:

```rust
pub use alloc::boxed::Box;
pub use alloc::string::String;
pub use alloc::vec::Vec;
pub use alloc::{format, vec};
pub use hashmap::HashMap;
pub use collections::HashSet;
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
| `try_waitpid` | `fn try_waitpid(tid: u32) -> u32` | Non-blocking wait. Returns exit code, `STOPPED`, `STILL_RUNNING`, or `u32::MAX`. |
| `kill` | `fn kill(tid: u32) -> u32` | Kill a thread (SIGKILL). Returns 0 on success. |
| `send_signal` | `fn send_signal(tid: u32, sig: u32) -> u32` | Send a specific signal to a thread. Returns 0 on success. |
| `getargs` | `fn getargs(buf: &mut [u8]) -> usize` | Get raw command-line arguments (includes argv[0]). |
| `args` | `fn args(buf: &mut [u8; 256]) -> &str` | Get arguments, skipping program name. Handles quoted argv[0] for paths with spaces (e.g. `"/Applications/My App.app" file.md`). |
| `thread_create` | `fn thread_create(entry: fn(), stack_top: usize, name: &str) -> u32` | Create a new thread. Returns TID. |
| `thread_create_with_priority` | `fn thread_create_with_priority(entry: fn(), stack_top: usize, name: &str, priority: u8) -> u32` | Create thread with priority. |
| `set_priority` | `fn set_priority(tid: u32, priority: u8) -> u32` | Set thread priority (0=highest, 255=lowest). |
| `get_capabilities` | `fn get_capabilities() -> u32` | Get capability flags for current process. |
| `getuid` | `fn getuid() -> u16` | Get current user ID. |
| `getgid` | `fn getgid() -> u16` | Get current group ID. |
| `authenticate` | `fn authenticate(username: &str, password: &str) -> bool` | Authenticate credentials. |
| `getusername` | `fn getusername(uid: u16, buf: &mut [u8]) -> u32` | Resolve UID to username. |
| `set_identity` | `fn set_identity(uid: u16) -> u32` | Switch to a different user identity. |
| `fork` | `fn fork() -> u32` | Fork the current process. Returns child TID in parent, 0 in child, `u32::MAX` on error. |
| `exec` | `fn exec(path: &str, args: &str) -> u32` | Replace current process with a new program. On success, never returns. On failure, returns `u32::MAX`. |
| `launch_app` | `fn launch_app(path: &str, args: &str) -> u32` | Launch a `.app` bundle via the Sessionhost process (handles permission checks). Returns TID or `u32::MAX`. |
| `shutdown` | `fn shutdown() -> !` | Power off the system. Requires `CAP_SYSTEM`. Does not return. |
| `reboot` | `fn reboot() -> !` | Reboot the system. Requires `CAP_SYSTEM`. Does not return. |
| `sleep_us` | `fn sleep_us(us: u32)` | Sleep for `us` microseconds. For durations >= 1 ms uses scheduler blocking; for sub-ms uses TSC-based busy-wait. |

### Thread

A handle to a spawned thread with RAII stack management. The default stack size is 64 KiB, allocated via `mmap`.

```rust
pub struct Thread {
    tid: u32,
    stack_ptr: *mut u8,
    stack_size: usize,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `spawn` | `fn spawn(entry: fn(), name: &str) -> error::Result<Thread>` | Spawn a new thread with default 64 KiB stack. |
| `spawn_with_stack` | `fn spawn_with_stack(entry: fn(), stack_size: usize, name: &str) -> error::Result<Thread>` | Spawn with custom stack size. |
| `tid` | `fn tid(&self) -> u32` | Get the thread ID. |
| `join` | `fn join(self) -> u32` | Wait for thread to finish, return exit code, free stack. Consumes handle. |

When a `Thread` handle is dropped without calling `join()`, it automatically waits for the thread and frees the stack to avoid leaking memory.

### Child

A handle to a spawned child process.

```rust
pub struct Child {
    tid: u32,
}
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `spawn` | `fn spawn(path: &str, args: &str) -> error::Result<Child>` | Spawn a new child process. |
| `tid` | `fn tid(&self) -> u32` | Get the child's thread ID. |
| `wait` | `fn wait(self) -> u32` | Wait for child to exit. Returns exit code. Consumes handle. |
| `try_wait` | `fn try_wait(&self) -> Option<u32>` | Non-blocking check. Returns `Some(exit_code)` if terminated, `None` if still running. |
| `kill` | `fn kill(&self) -> error::Result<()>` | Kill the child process. |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `STILL_RUNNING` | `u32::MAX - 1` | Return from `try_waitpid()` when thread is still alive |
| `STOPPED` | `u32::MAX - 2` | Return from `try_waitpid()` when thread is stopped by signal (SIGTSTP/SIGSTOP) |
| `SIGTSTP` | `20` | Terminal stop signal (Ctrl+Z) |
| `SIGCONT` | `18` | Continue stopped process |

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
| `capture_screen` | `fn capture_screen(buf: &mut [u32], info: &mut [u32; 3]) -> bool` | Capture framebuffer. info = [width, height, pitch_bytes]. |
| `set_critical` | `fn set_critical()` | Mark current thread as critical (won't be killed by OOM). |
| `random` | `fn random(buf: &mut [u8]) -> u32` | Fill buffer with random bytes (max 256). Returns bytes written. |
| `set_serial_verbose` | `fn set_serial_verbose(enable: bool) -> u32` | Enable/disable verbose serial output (driver messages). |
| `devlist` | `fn devlist(buf: &mut [u8]) -> u32` | List detected devices (64-byte entries). Returns device count. |
| `get_crash_info` | `fn get_crash_info(tid: u32, buf: &mut [u8]) -> u32` | Retrieve crash report for a terminated thread. Returns bytes written or 0 if none. |
| `pipe_list` | `fn pipe_list(buf: &mut [u8]) -> u32` | List active pipes (80-byte entries). Returns pipe count. |
| `set_time` | `fn set_time(buf: &[u8; 8]) -> u32` | Set system date/time via RTC. buf: [year_lo, year_hi, month, day, hour, min, sec, 0]. Returns 0 on success. |
| `uptime_ms` | `fn uptime_ms() -> u32` | Uptime in milliseconds (TSC-based, sub-ms precision). Wraps at ~49 days. |
| `get_hostname` | `fn get_hostname(buf: &mut [u8]) -> u32` | Get system hostname. Returns bytes written or `u32::MAX`. |
| `set_hostname` | `fn set_hostname(name: &str) -> u32` | Set system hostname (max 63 bytes). Returns 0 on success. |

### Disk / Partition Management

| Function | Signature | Description |
|----------|-----------|-------------|
| `disk_list` | `fn disk_list(buf: &mut [u8]) -> u32` | List all block devices (32-byte entries). Returns device count. |
| `disk_partitions` | `fn disk_partitions(disk_id: u32, buf: &mut [u8]) -> u32` | List partitions for a disk (32-byte entries). Returns partition count. |
| `disk_read` | `fn disk_read(device_id: u32, lba: u64, count: u32, buf: &mut [u8]) -> u32` | Read raw sectors. Returns 0 on success. |
| `disk_write` | `fn disk_write(device_id: u32, lba: u64, count: u32, buf: &[u8]) -> u32` | Write raw sectors. Returns 0 on success. |
| `partition_create` | `fn partition_create(disk_id: u32, entry: &[u8; 16]) -> u32` | Create/update an MBR partition entry. Returns 0 on success. |
| `partition_delete` | `fn partition_delete(disk_id: u32, index: u32) -> u32` | Delete an MBR partition entry. Returns 0 on success. |
| `partition_rescan` | `fn partition_rescan(disk_id: u32) -> u32` | Re-scan partition table. Returns partition count found. |
| `disk_eject` | `fn disk_eject(disk_id: u32) -> u32` | Safely eject a removable disk. Returns 0 on success. |

### Console API (nogui mode)

| Function | Signature | Description |
|----------|-----------|-------------|
| `con_write` | `fn con_write(s: &str) -> u32` | Write to kernel framebuffer text console. Returns bytes written. |
| `con_read_line` | `fn con_read_line(buf: &mut [u8]) -> usize` | Read a line with echo. Blocks until Enter. |
| `con_read_password` | `fn con_read_password(buf: &mut [u8]) -> usize` | Read a password (shows `*`). Blocks until Enter. |
| `con_poll_key` | `fn con_poll_key() -> u32` | Non-blocking keyboard poll. Returns codepoint or 0 if none. |
| `con_get_size` | `fn con_get_size() -> (u32, u32)` | Get console size as (columns, rows). |
| `con_resize` | `fn con_resize(cols: u32, rows: u32) -> (u32, u32)` | Resize console. Returns new (cols, rows). |
| `con_set_mode` | `fn con_set_mode(flags: u32) -> u32` | Set console mode flags. Returns previous flags. |

Console mode flags: `CON_MODE_HIDE_CURSOR` (0x01), `CON_MODE_NO_AUTOSCROLL` (0x02).

Key code prefixes: `KEY_ARROW_PREFIX` (0x10_0000), `KEY_NAV_PREFIX` (0x20_0000), `KEY_FN_PREFIX` (0x30_0000), `KEY_SHIFT_BIT` (0x1400_0000).

### Thermal Sensors

| Function | Signature | Description |
|----------|-----------|-------------|
| `thermal_read` | `fn thermal_read(max: u32) -> Vec<ThermalEntry>` | Read all thermal sensors. Returns up to `max` entries. |
| `thermal_cpu` | `fn thermal_cpu() -> Option<i32>` | Read primary CPU temperature in 0.1 C units. |

```rust
pub struct ThermalEntry {
    pub src_type: u8,   // 0=IntelCpu, 1=AmdCpu, 2=Lm75, 3=Smbus
    pub src_id: u8,     // core index or SMBus address
    pub temp_x10: i32,  // temperature in 0.1 C units
}
```

### ACPI Power Management

| Function | Signature | Description |
|----------|-----------|-------------|
| `acpi_sleep` | `fn acpi_sleep(state: u32) -> Result<(), ()>` | Request ACPI sleep state (0=S0, 3=S3, 4=S4, 5=S5 power off). |
| `acpi_perf_get` | `fn acpi_perf_get() -> Option<u8>` | Get current CPU P-state frequency ratio byte. |
| `acpi_perf_set` | `fn acpi_perf_set(ratio: u8) -> Result<(), ()>` | Set CPU P-state frequency ratio. |

### I2C / SMBus

| Function | Signature | Description |
|----------|-----------|-------------|
| `i2c_read_byte` | `fn i2c_read_byte(addr: u8, reg: u8) -> Option<u8>` | Read a byte from I2C device. |
| `i2c_write_byte` | `fn i2c_write_byte(addr: u8, reg: u8, value: u8) -> Result<(), ()>` | Write a byte to I2C device. |
| `i2c_detect` | `fn i2c_detect(addr: u8) -> bool` | Probe whether an I2C device is present. |

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
| `stat` | `fn stat(path: &str, buf: &mut [u32; 7]) -> u32` | File status (follows symlinks). Writes `[type, size, flags, uid, gid, mode, mtime]`. Returns 0 on success. |
| `lstat` | `fn lstat(path: &str, buf: &mut [u32; 7]) -> u32` | File status (no symlink follow). Same layout as `stat`. |
| `fstat` | `fn fstat(fd: u32, buf: &mut [u32; 4]) -> u32` | FD status. Writes `[type, size, position, mtime]`. Returns 0 on success. |
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
| `rename` | `fn rename(old_path: &str, new_path: &str) -> u32` | Rename (move) a file or directory. 0 on success. |
| `sync` | `fn sync()` | Flush all filesystem metadata and storage write caches to disk. |
| `fsync` | `fn fsync(fd: i32) -> bool` | Flush deferred metadata for a specific open file to disk (like POSIX fsync). |
| `chmod` | `fn chmod(path: &str, mode: u16) -> u32` | Change file permissions. 0 on success. |
| `chown` | `fn chown(path: &str, uid: u16, gid: u16) -> u32` | Change file owner/group. 0 on success. |
| `read_nonblock` | `fn read_nonblock(fd: u32, buf: &mut [u8]) -> i32` | Non-blocking read from FD. Returns bytes read, -11 (EAGAIN) if pipe empty, -1 on error. Requires `set_fd_nonblock` first. |
| `set_fd_nonblock` | `fn set_fd_nonblock(fd: u32, nonblock: bool)` | Set or clear O_NONBLOCK on a file descriptor. |
| `statfs` | `fn statfs(path: &str) -> Option<StatFs>` | Get filesystem statistics for a mount point. |

### High-Level File I/O

| Function | Signature | Description |
|----------|-----------|-------------|
| `read_to_string` | `fn read_to_string(path: &str) -> error::Result<String>` | Read an entire file into a `String`. |
| `read_to_vec` | `fn read_to_vec(path: &str) -> error::Result<Vec<u8>>` | Read an entire file into a `Vec<u8>`. Pre-allocates based on file size. |
| `write_bytes` | `fn write_bytes(path: &str, data: &[u8]) -> error::Result<()>` | Write bytes to a file (creates or truncates). |
| `read_dir` | `fn read_dir(path: &str) -> error::Result<ReadDir>` | Read directory entries and return an iterator. |

### File Struct

An open file handle with RAII (auto-close on drop). Implements `Read` and `Write` traits.

```rust
pub struct File { fd: u32 }
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `open` | `fn open(path: &str) -> error::Result<File>` | Open an existing file for reading. |
| `create` | `fn create(path: &str) -> error::Result<File>` | Create a new file (or truncate existing) for writing. |
| `open_with` | `fn open_with(path: &str, flags: u32) -> error::Result<File>` | Open a file with explicit flags. |
| `fd` | `fn fd(&self) -> u32` | Get the raw file descriptor. |
| `metadata` | `fn metadata(&self) -> error::Result<[u32; 4]>` | Get file metadata via fstat. Returns `[type, size, position, mtime]`. |

### Read / Write Traits

```rust
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> error::Result<usize>;
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> error::Result<usize>;
    fn read_to_string(&mut self, out: &mut String) -> error::Result<usize>;
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> error::Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> error::Result<()>;
    fn flush(&mut self) -> error::Result<()>;
}
```

### DirEntry / ReadDir

```rust
pub struct DirEntry {
    pub name: String,
    pub file_type: u8,  // 0=file, 1=directory, 2=symlink
    pub size: u32,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool;
    pub fn is_file(&self) -> bool;
}
```

`ReadDir` implements `Iterator<Item = DirEntry>`.

### StatFs

```rust
pub struct StatFs {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
}
```

### Directory Entry Format

`readdir()` returns entries in a packed format, 64 bytes each:

| Offset | Size | Field |
|--------|------|-------|
| 0 | 1 byte | Type (0=file, 1=directory, 2=symlink) |
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
| `reload_hosts` | `fn reload_hosts() -> u32` | Reload the hosts file from disk. Returns 0 on success. |
| `nic_driver_name` | `fn nic_driver_name(buf: &mut [u8; 64]) -> u32` | Get NIC driver name. Returns bytes written or 0 if no NIC. |
| `get_interfaces` | `fn get_interfaces(buf: &mut [u8; 512]) -> u32` | Get interface configurations (64-byte entries). Returns interface count. |
| `set_interfaces` | `fn set_interfaces(buf: &[u8]) -> u32` | Save and apply interface configurations. Returns 0 on success. |

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
| `tcp_recv_available` | `fn tcp_recv_available(socket_id: u32) -> u32` | Bytes available to read without blocking. >0=bytes, 0=no data, `u32::MAX-1`=EOF, `u32::MAX`=error. |
| `tcp_close` | `fn tcp_close(socket_id: u32) -> u32` | Close connection. |
| `tcp_status` | `fn tcp_status(socket_id: u32) -> u32` | Connection state: 0=Closed, 2=Established, etc. |
| `tcp_listen` | `fn tcp_listen(port: u16, backlog: u16) -> u32` | Listen on a TCP port. Returns listener socket_id or `u32::MAX`. |
| `tcp_accept` | `fn tcp_accept(listener_id: u32) -> (u32, [u8; 4], u16)` | Accept a connection (blocking, 30s timeout). Returns (socket_id, remote_ip, remote_port). |
| `tcp_accept_nowait` | `fn tcp_accept_nowait(listener_id: u32) -> (u32, [u8; 4], u16)` | Non-blocking accept. Returns `(u32::MAX, [0;4], 0)` if none pending. |
| `tcp_list` | `fn tcp_list() -> Vec<TcpConnInfo>` | List all active TCP connections/listeners. |

### UDP

| Function | Signature | Description |
|----------|-----------|-------------|
| `udp_bind` | `fn udp_bind(port: u16) -> u32` | Bind to a UDP port. Returns 0 on success. |
| `udp_unbind` | `fn udp_unbind(port: u16) -> u32` | Release a UDP port. |
| `udp_sendto` | `fn udp_sendto(dst_ip: &[u8; 4], dst_port: u16, src_port: u16, data: &[u8], flags: u32) -> u32` | Send UDP datagram. |
| `udp_recvfrom` | `fn udp_recvfrom(port: u16, buf: &mut [u8]) -> u32` | Receive UDP datagram. Returns bytes read. |
| `udp_set_opt` | `fn udp_set_opt(port: u16, opt: u32, val: u32) -> u32` | Set UDP socket option (1=SO_BROADCAST, 2=SO_RCVTIMEO). |
| `udp_list` | `fn udp_list() -> Vec<UdpBindInfo>` | List all bound UDP ports. |

### Network Statistics

| Function | Signature | Description |
|----------|-----------|-------------|
| `net_stats` | `fn net_stats() -> Option<NetStats>` | Get network protocol statistics. |

```rust
pub struct NetStats {
    pub rx_packets: u64, pub tx_packets: u64,
    pub rx_bytes: u64, pub tx_bytes: u64,
    pub rx_errors: u64, pub tx_errors: u64,
    pub tcp_active_opens: u64, pub tcp_passive_opens: u64,
    pub tcp_segments_sent: u64, pub tcp_segments_recv: u64,
    pub tcp_retransmits: u64, pub tcp_resets_sent: u64,
    pub tcp_curr_established: u32, pub tcp_conn_errors: u32,
}

pub struct TcpConnInfo {
    pub local_ip: [u8; 4], pub local_port: u16,
    pub remote_ip: [u8; 4], pub remote_port: u16,
    pub state: u8, pub owner_tid: u8, pub recv_buf_len: u16,
}

pub struct UdpBindInfo {
    pub port: u16, pub owner_tid: u16, pub recv_queue_len: u16,
}
```

### WiFi

| Function | Signature | Description |
|----------|-----------|-------------|
| `wifi_available` | `fn wifi_available() -> bool` | Check whether a WiFi driver is present. |
| `wifi_state` | `fn wifi_state() -> WifiState` | Get current WiFi state. |
| `wifi_scan` | `fn wifi_scan()` | Start a new WiFi scan. Results via `wifi_scan_results`. |
| `wifi_scan_results` | `fn wifi_scan_results(max: usize) -> Vec<BssInfo>` | Read scanned access points. |
| `wifi_connect` | `fn wifi_connect(ssid: &str, password: &str) -> u32` | Connect to WPA2 network. Asynchronous -- poll `wifi_state`. Returns 0 on success. |
| `wifi_disconnect` | `fn wifi_disconnect()` | Disconnect from WiFi. |
| `wifi_status` | `fn wifi_status() -> WifiStatus` | Get current connection status. |

```rust
pub enum WifiState { Disconnected, Scanning, Associating, Authenticating, Connected }
pub enum WifiSecurity { Open, Wpa2Personal }

pub struct BssInfo {
    pub bssid: [u8; 6], pub ssid: [u8; 32], pub ssid_len: usize,
    pub channel: u8, pub rssi: i8, pub security: WifiSecurity,
}

pub struct WifiStatus {
    pub state: WifiState, pub connected: bool, pub channel: u8,
    pub bssid: [u8; 6], pub ssid: [u8; 32], pub ssid_len: usize,
}
```

---

## `log` -- Structured Logging

Sends structured log messages to the central `logd` daemon via the `"log"` named pipe. The pipe is opened lazily on first use and cached for the process lifetime. If logd is not running, messages are silently dropped.

See [services.md](services.md) for the full logging system documentation.

### Macros

| Macro | Level | Description |
|-------|-------|-------------|
| `log_info!(...)` | INFO | Informational message |
| `log_warn!(...)` | WARN | Warning condition |
| `log_error!(...)` | ERROR | Error condition |
| `log_debug!(...)` | DEBUG | Debug/diagnostic output |

All macros accept `format_args!()` syntax (zero heap allocation):

```rust
anyos_std::log_info!("server started on port {}", port);
anyos_std::log_warn!("connection pool {}% full", usage);
anyos_std::log_error!("failed to open {}: {}", path, err);
anyos_std::log_debug!("packet: {} bytes, seq={}", len, seq);
```

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `log::log_msg` | `fn log_msg(level: &str, args: Arguments)` | Core logging function (used by macros) |

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `LEVEL_INFO` | `"INFO"` | Informational level |
| `LEVEL_WARN` | `"WARN"` | Warning level |
| `LEVEL_ERROR` | `"ERROR"` | Error level |
| `LEVEL_DEBUG` | `"DEBUG"` | Debug level |

### Wire Protocol

Messages are sent as: `LEVEL|source|message\n`

The **source** is automatically derived from the program's argv[0] (filename after last `/`). Messages are built in a 512-byte stack buffer with no heap allocation.

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
| `pipe_bytes_available_fd` | `fn pipe_bytes_available_fd(fd: u32) -> u32` | Non-blocking check of anonymous pipe FD. >0=bytes ready, 0=empty, `u32::MAX-1`=EOF, `u32::MAX`=not a pipe. |

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
| `evt_chan_wait` | `fn evt_chan_wait(channel_id: u32, sub_id: u32, timeout_ms: u32) -> u32` | Block until an event is available or timeout. Returns 1 if events ready, 0 on timeout. `u32::MAX` = wait indefinitely. |

### Session Host

| Function | Signature | Description |
|----------|-----------|-------------|
| `register_sessionhost` | `fn register_sessionhost() -> u32` | Register calling process as the session host. Returns 0 on success. |

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
| `grant_framebuffer` | `fn grant_framebuffer(target_tid: u32, out_info: &mut FbMapInfo) -> u32` | Grant a thread direct framebuffer access. Returns 0 on success. |
| `revoke_framebuffer` | `fn revoke_framebuffer(target_tid: u32) -> u32` | Revoke a thread's framebuffer access. Returns 0 on success. |
| `gpu_vram_size` | `fn gpu_vram_size() -> u32` | Query total GPU VRAM size in bytes. Returns 0 if no GPU. |
| `vram_map` | `fn vram_map(target_tid: u32, vram_byte_offset: u32, num_bytes: u32) -> u32` | Map VRAM into target's address space. Returns user VA on success. |
| `gpu_register_backbuffer` | `fn gpu_register_backbuffer(buf_ptr: u32, buf_size: u32) -> u32` | Register compositor back buffer for GPU DMA. Returns 0 on success. |
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
| `unset` | `fn unset(key: &str) -> u32` | Remove variable (calls syscall with null pointer to clear the key). |
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
| `sha256` | `fn sha256(input: &[u8]) -> [u8; 32]` | Compute SHA-256 hash (32 raw bytes). |
| `sha256_hex` | `fn sha256_hex(input: &[u8]) -> [u8; 64]` | Compute SHA-256 hash as hex string bytes. |

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
| `load` | `fn load() -> Self` | Load MIME database from disk (system + user overrides). |
| `lookup` | `fn lookup(&self, ext: &str) -> Option<&MimeEntry>` | Lookup by file extension. |
| `icon_for_ext` | `fn icon_for_ext(&self, ext: &str) -> &str` | Get icon path for extension. |
| `app_for_ext` | `fn app_for_ext(&self, ext: &str) -> Option<&str>` | Get default app for extension. |
| `set_user_default` | `fn set_user_default(&mut self, ext: &str, app_path: &str)` | Set a per-user default app for an extension. Persists to `$HOME/.mime_overrides.json`. |

#### MimeOverride

Per-user MIME association override.

```rust
pub struct MimeOverride {
    pub ext: String,
    pub app: String,
}
```

### Utility Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `is_app_bundle` | `fn is_app_bundle(path: &str) -> bool` | Check if path is a `.app` bundle. |
| `app_bundle_name` | `fn app_bundle_name(bundle_path: &str) -> String` | Extract display name from bundle path. |
| `app_icon_path` | `fn app_icon_path(bin_path: &str) -> String` | Find icon for an application binary. |

---

## `hashmap` -- Hash Map

A no_std hash map using FNV-1a hashing with open addressing (linear probing). Power-of-2 table size, resizes at 75% load factor. Re-exported as `anyos_std::HashMap`.

### HashMap<K, V>

```rust
pub struct HashMap<K: Hash + Eq, V> { /* ... */ }
```

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new() -> Self` | Create an empty HashMap. |
| `with_capacity` | `fn with_capacity(cap: usize) -> Self` | Create with pre-allocated capacity. |
| `insert` | `fn insert(&mut self, key: K, value: V) -> Option<V>` | Insert key-value pair. Returns previous value if key existed. |
| `get` | `fn get(&self, key: &K) -> Option<&V>` | Get reference to value for key. |
| `get_mut` | `fn get_mut(&mut self, key: &K) -> Option<&mut V>` | Get mutable reference to value. |
| `remove` | `fn remove(&mut self, key: &K) -> Option<V>` | Remove key-value pair. Returns value if existed. |
| `contains_key` | `fn contains_key(&self, key: &K) -> bool` | Check if key exists. |
| `len` | `fn len(&self) -> usize` | Number of entries. |
| `is_empty` | `fn is_empty(&self) -> bool` | Whether the map is empty. |
| `capacity` | `fn capacity(&self) -> usize` | Number of allocated bucket slots. |
| `clear` | `fn clear(&mut self)` | Remove all entries (keeps allocation). |
| `iter` | `fn iter(&self) -> Iter<K, V>` | Iterate over `(&K, &V)` pairs. |
| `iter_mut` | `fn iter_mut(&mut self) -> IterMut<K, V>` | Iterate over `(&K, &mut V)` pairs. |
| `keys` | `fn keys(&self) -> Keys<K, V>` | Iterate over keys. |
| `values` | `fn values(&self) -> Values<K, V>` | Iterate over values. |

Implements: `Default`, `Debug`, `Clone`, `Index<&K>`, `FromIterator`, `IntoIterator`.

### Example

```rust
use anyos_std::HashMap;

let mut map = HashMap::new();
map.insert("name", "anyOS");
map.insert("version", "1.0");
assert_eq!(map.get(&"name"), Some(&"anyOS"));
assert_eq!(map["version"], "1.0");
```

---

## `collections` -- Collections

The `collections` module provides standard collection types for `no_std` programs.

### Re-exported Types

| Type | Source | Description |
|------|--------|-------------|
| `HashMap<K, V>` | `collections::hash_map` | Hash map (FNV-1a, open addressing) |
| `HashSet<T>` | `collections::hash_set` | Hash set built on HashMap |
| `BTreeMap<K, V>` | `alloc::collections` | Sorted map (B-tree) |
| `BTreeSet<T>` | `alloc::collections` | Sorted set (B-tree) |
| `VecDeque<T>` | `alloc::collections` | Double-ended queue |
| `BinaryHeap<T>` | `alloc::collections` | Priority queue (max-heap) |
| `LinkedList<T>` | `alloc::collections` | Doubly-linked list |

### Usage

```rust
use anyos_std::collections::{HashMap, HashSet, BTreeMap, VecDeque};
```

`HashMap` and `HashSet` are also re-exported at the crate root: `anyos_std::HashMap`, `anyos_std::HashSet`.

---

## `json` -- JSON Parser & Serializer

Full RFC 8259 JSON parser and serializer with pretty-printing support.

### Value

```rust
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Object),
}

pub enum Number {
    Int(i64),
    Float(f64),
}
```

### Value Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `parse` | `fn parse(input: &str) -> Result<Value, ParseError>` | Parse a JSON string. |
| `new_object` | `fn new_object() -> Value` | Create empty object. |
| `new_array` | `fn new_array() -> Value` | Create empty array. |
| `to_json_string` | `fn to_json_string(&self) -> String` | Serialize to compact JSON. |
| `to_json_string_pretty` | `fn to_json_string_pretty(&self) -> String` | Serialize with 2-space indent. |
| `to_json_string_indent` | `fn to_json_string_indent(&self, indent: usize) -> String` | Serialize with custom indent. |
| `is_null` | `fn is_null(&self) -> bool` | Type check. |
| `is_bool` / `is_number` / `is_string` / `is_array` / `is_object` | | Type checks. |
| `as_bool` | `fn as_bool(&self) -> Option<bool>` | Extract bool. |
| `as_i64` | `fn as_i64(&self) -> Option<i64>` | Extract integer (converts float). |
| `as_u64` | `fn as_u64(&self) -> Option<u64>` | Extract unsigned integer. |
| `as_f64` | `fn as_f64(&self) -> Option<f64>` | Extract float (converts int). |
| `as_str` | `fn as_str(&self) -> Option<&str>` | Extract string. |
| `as_array` / `as_array_mut` | | Extract array reference. |
| `as_object` / `as_object_mut` | | Extract object reference. |
| `set` | `fn set(&mut self, key: &str, value: Value)` | Set key on object (no-op if not object). |
| `push` | `fn push(&mut self, value: Value)` | Push to array (no-op if not array). |

Index operators: `val["key"]` returns `Value::Null` for missing keys. `val[0]` for arrays.

### Object

Ordered key-value store (preserves insertion order) with O(1) HashMap-backed lookup.

| Method | Signature | Description |
|--------|-----------|-------------|
| `insert` | `fn insert(&mut self, key: String, value: Value) -> Option<Value>` | Insert or update. |
| `get` | `fn get(&self, key: &str) -> Option<&Value>` | Lookup by key. |
| `get_mut` | `fn get_mut(&mut self, key: &str) -> Option<&mut Value>` | Mutable lookup. |
| `remove` | `fn remove(&mut self, key: &str) -> Option<Value>` | Remove by key. |
| `contains_key` | `fn contains_key(&self, key: &str) -> bool` | Check if key exists. |
| `len` / `is_empty` | | Entry count. |
| `iter` | `fn iter(&self) -> impl Iterator<Item = (&str, &Value)>` | Iterate in insertion order. |
| `keys` / `values` | | Key/value iterators. |

### From Conversions

`Value` implements `From<T>` for: `bool`, `i32`, `i64`, `u32`, `u64`, `f64`, `&str`, `String`, `Vec<T: Into<Value>>`.

### Example

```rust
use anyos_std::json::{Value, Number};

// Parse
let val = Value::parse(r#"{"name": "anyOS", "version": 1}"#).unwrap();
assert_eq!(val["name"].as_str(), Some("anyOS"));
assert_eq!(val["version"].as_i64(), Some(1));

// Build
let mut obj = Value::new_object();
obj.set("name", "anyOS".into());
obj.set("version", Value::Number(Number::Int(1)));
obj.set("features", vec!["multitasking", "networking"].into());

println!("{}", obj.to_json_string_pretty());
// {
//   "name": "anyOS",
//   "version": 1,
//   "features": [
//     "multitasking",
//     "networking"
//   ]
// }
```

---

## `xml` -- XML Parser & Serializer

XML 1.0 parser and serializer supporting elements, attributes, text, CDATA, comments, processing instructions, and entity references.

### Types

```rust
pub struct Document {
    pub declaration: Option<Declaration>,
    pub root: Element,
}

pub struct Declaration {
    pub version: String,
    pub encoding: Option<String>,
    pub standalone: Option<bool>,
}

pub enum XmlNode {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
    PI(String, String),  // target, data
}
```

### Document Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `parse` | `fn parse(input: &str) -> Result<Document, XmlError>` | Parse XML string. |
| `new` | `fn new(root: Element) -> Document` | Create document with root element. |
| `with_declaration` | `fn with_declaration(root: Element, version: &str) -> Document` | Create with XML declaration. |
| `to_xml_string` | `fn to_xml_string(&self) -> String` | Serialize to compact XML. |
| `to_xml_string_pretty` | `fn to_xml_string_pretty(&self) -> String` | Serialize with 2-space indent. |
| `to_xml_string_indent` | `fn to_xml_string_indent(&self, indent: usize) -> String` | Serialize with custom indent. |

### Element Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `fn new(name: &str) -> Element` | Create empty element. |
| `name` | `fn name(&self) -> &str` | Tag name. |
| `attr` | `fn attr(&self, name: &str) -> Option<&str>` | Get attribute value. |
| `set_attr` | `fn set_attr(&mut self, name: &str, value: &str)` | Set attribute. |
| `remove_attr` | `fn remove_attr(&mut self, name: &str) -> bool` | Remove attribute. |
| `attributes` | `fn attributes(&self) -> &[(String, String)]` | All attributes. |
| `children` | `fn children(&self) -> &[XmlNode]` | All child nodes. |
| `child_elements` | `fn child_elements(&self) -> impl Iterator<Item = &Element>` | Child elements only. |
| `child` | `fn child(&self, name: &str) -> Option<&Element>` | First child by name. |
| `child_mut` | `fn child_mut(&mut self, name: &str) -> Option<&mut Element>` | Mutable child by name. |
| `children_named` | `fn children_named(&self, name: &str) -> impl Iterator<Item = &Element>` | All children by name. |
| `text` | `fn text(&self) -> Option<&str>` | First text/CDATA child. |
| `text_content` | `fn text_content(&self) -> String` | Concatenated text of all text/CDATA children. |
| `add_child_element` | `fn add_child_element(&mut self, element: Element)` | Add child element. |
| `add_text` | `fn add_text(&mut self, text: &str)` | Add text node. |
| `add_cdata` | `fn add_cdata(&mut self, data: &str)` | Add CDATA section. |
| `add_comment` | `fn add_comment(&mut self, text: &str)` | Add comment. |
| `element_count` | `fn element_count(&self) -> usize` | Number of child elements. |
| `is_empty` | `fn is_empty(&self) -> bool` | Whether element has no children. |

### Supported Features

- XML declaration (`<?xml version="1.0" encoding="utf-8"?>`)
- Attributes (single or double-quoted)
- Self-closing elements (`<br/>`)
- Entity references: `&amp;` `&lt;` `&gt;` `&quot;` `&apos;`
- Numeric character references: `&#65;` `&#x41;`
- CDATA sections (`<![CDATA[...]]>`)
- Comments (`<!-- ... -->`)
- Processing instructions (`<?target data?>`)
- Pretty-printing with configurable indent

### Example

```rust
use anyos_std::xml::{Document, Element};

// Parse
let doc = Document::parse(r#"<config version="1.0"><item key="name">anyOS</item></config>"#).unwrap();
assert_eq!(doc.root.name(), "config");
assert_eq!(doc.root.attr("version"), Some("1.0"));
let item = doc.root.child("item").unwrap();
assert_eq!(item.attr("key"), Some("name"));
assert_eq!(item.text(), Some("anyOS"));

// Build
let mut root = Element::new("config");
root.set_attr("version", "2.0");
let mut item = Element::new("setting");
item.set_attr("key", "theme");
item.add_text("dark");
root.add_child_element(item);

let doc = Document::with_declaration(root, "1.0");
println!("{}", doc.to_xml_string_pretty());
// <?xml version="1.0"?>
// <config version="2.0">
//   <setting key="theme">dark</setting>
// </config>
```

---

## `error` -- Error Types

Provides an `Error` enum that maps kernel errno values to named variants, and a `Result<T>` type alias.

### Error Enum

```rust
pub enum Error {
    NotFound,           // ENOENT = 2
    WouldBlock,         // EAGAIN = 11
    PermissionDenied,   // EACCES = 13
    AlreadyExists,      // EEXIST = 17
    NotADirectory,      // ENOTDIR = 20
    IsADirectory,       // EISDIR = 21
    InvalidInput,       // EINVAL = 22
    NoSpace,            // ENOSPC = 28
    BrokenPipe,         // EPIPE = 32
    TimedOut,           // ETIMEDOUT = 110
    ConnectionRefused,  // ECONNREFUSED = 111
    OutOfMemory,        // ENOMEM = 12
    Other(u32),         // unmapped errno
}

pub type Result<T> = core::result::Result<T, Error>;
```

### Error Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `from_errno` | `fn from_errno(errno: u32) -> Error` | Convert kernel errno to Error variant. |
| `from_syscall` | `fn from_syscall(v: u32) -> Result<u32>` | Convert raw syscall return (negative = error). |
| `from_raw` | `fn from_raw(v: u32) -> Result<u32>` | Like `from_syscall` but also treats `u32::MAX` as `NotFound`. |

`Error` implements `Display` and can be used with `println!("{}", err)`.

### DllError

```rust
pub enum DllError {
    LoadFailed,
    SymbolNotFound,
}
```

Errors for dynamic library loading. Implements `Display`.

---

## `path` -- Path Helpers

Zero-allocation path manipulation utilities. `basename`, `parent`, and `extension` return borrowed slices. Only `join` allocates.

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `basename` | `fn basename(path: &str) -> &str` | Extract last component (filename). Trailing slashes ignored. |
| `parent` | `fn parent(path: &str) -> &str` | Extract parent directory. Returns `"."` if no separator, `"/"` for root. |
| `extension` | `fn extension(path: &str) -> Option<&str>` | Extract file extension (without dot). Hidden files return `None`. |
| `join` | `fn join(dir: &str, name: &str) -> String` | Join directory and filename with `/`. |

### Example

```rust
use anyos_std::path;

assert_eq!(path::basename("/usr/bin/ls"), "ls");
assert_eq!(path::parent("/usr/bin/ls"), "/usr/bin");
assert_eq!(path::extension("file.txt"), Some("txt"));
assert_eq!(path::join("/usr/bin", "ls"), "/usr/bin/ls");
```

---

## `fmt` -- Formatting Helpers

Number and hex formatting utilities for `no_std` environments. Buffer-based functions write into caller-provided arrays and return `&str` slices with zero allocation.

### Integer Formatting

| Function | Signature | Description |
|----------|-----------|-------------|
| `fmt_u32` | `fn fmt_u32(buf: &mut [u8; 12], val: u32) -> &str` | Format u32 as decimal string. |
| `fmt_i32` | `fn fmt_i32(buf: &mut [u8; 14], val: i32) -> &str` | Format i32 as decimal string. |
| `fmt_u64` | `fn fmt_u64(buf: &mut [u8; 20], val: u64) -> &str` | Format u64 as decimal string. |
| `fmt_i64` | `fn fmt_i64(buf: &mut [u8; 21], val: i64) -> &str` | Format i64 as decimal string. |
| `fmt_f64` | `fn fmt_f64(val: f64) -> String` | Format f64 as decimal string (allocating). Handles NaN, infinity. |

### Hex Formatting

| Function | Signature | Description |
|----------|-----------|-------------|
| `hex64` | `fn hex64(val: u64) -> String` | Format u64 as `0x` + 16 hex digits. |
| `hex32` | `fn hex32(val: u32) -> String` | Format u32 as `0x` + 8 hex digits. |
| `hex_byte` | `fn hex_byte(val: u8) -> [u8; 2]` | Format u8 as 2 hex digits (no prefix). |
| `hex_bytes` | `fn hex_bytes(data: &[u8]) -> String` | Format byte slice as space-separated hex. |

### Composite Formatters

| Function | Signature | Description |
|----------|-----------|-------------|
| `fmt_pct` | `fn fmt_pct(buf: &mut [u8; 12], pct_x10: u32) -> &str` | Format percentage from fixed-point x10 value (155 -> "15.5%"). |
| `fmt_mem_pages` | `fn fmt_mem_pages(buf: &mut [u8; 16], pages: u32) -> &str` | Format page count as memory size ("X.Y M" or "X K"). |
| `fmt_bytes` | `fn fmt_bytes(buf: &mut [u8; 20], bytes: u64) -> &str` | Format byte count as human-readable size ("X.Y MiB", "X KiB"). |

---

## `i18n` -- Internationalization

Simple translation system based on JSON files stored in `/System/translations/{lang}.json`. English strings are used as keys -- if no translation is found, the original English text is returned.

### Language Detection Order

1. `LANG` environment variable (e.g. `"de"`)
2. `/System/settings/language.conf` file contents
3. Fallback: `"en"`

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `init` | `fn init()` | Detect language and load translation file. Call once at startup. |
| `t` | `fn t(key: &str) -> &str` | Translate a string. Returns original key if no translation found. |
| `lang` | `fn lang() -> &'static str` | Returns current language code (e.g. "en", "de"). Returns "en" if `init()` not called. |

### Example

```rust
use anyos_std::i18n;

i18n::init();
let label = i18n::t("Save");   // "Speichern" if lang=de
let lang  = i18n::lang();      // "de"
```

---

## `shell` -- Shell Parsing Library

Shared POSIX-style shell utilities used by both `terminal` (GUI) and `textmode_console` (nogui). Provides tokenization, variable expansion, redirect parsing, glob expansion, and pipeline execution.

### Types

```rust
pub struct Redirect {
    pub target: String,
    pub append: bool,
}

pub struct InputRedirect {
    pub source: String,
}

pub struct PipelineResult {
    pub last_tid: u32,       // TID of last process in pipeline
    pub display_pipe: u32,   // Pipe ID for combined stdout
    pub extra_pipes: Vec<u32>, // Intermediate pipes (must be closed)
}
```

### Tokenizer

| Function | Signature | Description |
|----------|-----------|-------------|
| `tokenize` | `fn tokenize(input: &str) -> Vec<String>` | Tokenize shell command respecting POSIX quoting (single/double quotes, backslash). |
| `join` | `fn join(tokens: &[String]) -> String` | Re-join tokens into shell-safe string (quotes tokens with spaces). |

### Redirect Parsing

| Function | Signature | Description |
|----------|-----------|-------------|
| `parse_redirects` | `fn parse_redirects(line: &str, cwd: &str) -> (String, Option<Redirect>)` | Strip output redirect (`>`, `>>`, `2>`, `2>>`, `2>&1`) from command line. |
| `parse_input_redirect` | `fn parse_input_redirect(line: &str, cwd: &str) -> (String, Option<InputRedirect>)` | Strip input redirect (`< file`) from command line. |
| `write_redirect` | `fn write_redirect(redirect: &mut Redirect, data: &str)` | Write data to redirect target file. First call truncates, subsequent calls append. |

### PATH Resolution

| Function | Signature | Description |
|----------|-----------|-------------|
| `resolve_from_path` | `fn resolve_from_path(cmd: &str) -> Option<String>` | Search PATH env var for command. Returns full path or None. |
| `resolve_cmd_path` | `fn resolve_cmd_path(cmd: &str, cwd: &str) -> String` | Resolve command to full path (absolute, `./relative`, or PATH search). |

### Variable & Tilde Expansion

| Function | Signature | Description |
|----------|-----------|-------------|
| `expand_tilde` | `fn expand_tilde(token: &str) -> String` | Expand `~` or `~/` to HOME env var. |
| `expand_vars` | `fn expand_vars(token: &str) -> String` | Expand `$VAR`, `${VAR}`, `$(cmd)`, backticks, `$$`, `$?`. |
| `expand_args` | `fn expand_args(raw: &str, cwd: &str) -> Vec<String>` | Full expansion pipeline: tokenize + vars + tildes + globs. |

### Command Substitution

| Function | Signature | Description |
|----------|-----------|-------------|
| `capture_command_output` | `fn capture_command_output(cmd: &str) -> String` | Spawn command, capture stdout, return as String (trailing newlines stripped). |

### Pipeline Execution

| Function | Signature | Description |
|----------|-----------|-------------|
| `run_pipeline` | `fn run_pipeline(line: &str, cwd: &str, pipe_counter: &mut u32) -> Option<PipelineResult>` | Execute `cmd1 \| cmd2 \| cmdN` pipeline. Creates pipes, spawns all processes. |
| `split_pipe_segments` | `fn split_pipe_segments(line: &str) -> Vec<&str>` | Split on unquoted `\|` characters (respects quoting). |
| `has_pipe` | `fn has_pipe(line: &str) -> bool` | Check if line contains an unquoted pipe character. |

---

## `debug` -- Debugger API

Userspace wrappers for the debug syscalls. All functions require `CAP_DEBUG` capability. Used by the anyTrace debugger.

### Event Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `EVENT_BREAKPOINT` | `1` | Thread hit a software breakpoint (INT3) |
| `EVENT_SINGLE_STEP` | `2` | Thread completed a single-step (#DB with TF) |
| `EVENT_EXIT` | `3` | Thread exited while debug-attached |

### Types

```rust
pub struct DebugRegs {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64,
    pub r8: u64, pub r9: u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rsp: u64, pub rip: u64, pub rflags: u64, pub cr3: u64,
}

pub struct DebugEvent {
    pub event_type: u32,  // EVENT_BREAKPOINT, EVENT_SINGLE_STEP, or EVENT_EXIT
    pub addr: u64,        // RIP at breakpoint/step, exit code for exit
}

pub struct MemoryRegion {
    pub start: u64,  // start address (inclusive)
    pub end: u64,    // end address (exclusive)
    pub flags: u64,  // page table flags (P, RW, US, NX, etc.)
}

pub struct ThreadInfoEx {
    pub parent_tid: u32, pub state: u32, pub priority: u32,
    pub cpu_ticks: u32, pub last_cpu: u32, pub user_pages: u32,
    pub brk: u32, pub mmap_next: u32,
    pub rip: u64, pub rsp: u64, pub cr3: u64,
    pub io_read_bytes: u64, pub io_write_bytes: u64,
    pub capabilities: u32, pub uid: u16, pub gid: u16,
    pub debug_attached_by: u32,
    pub name: [u8; 32],
    pub arch_mode: u32,
}
```

### Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `attach` | `fn attach(tid: u32) -> bool` | Attach to a running thread as debugger. Target is suspended. |
| `detach` | `fn detach(tid: u32) -> bool` | Detach from thread. All breakpoints removed, thread resumes. |
| `suspend` | `fn suspend(tid: u32) -> bool` | Suspend a debug-attached thread. |
| `resume` | `fn resume(tid: u32) -> bool` | Resume a suspended debug-attached thread. |
| `get_regs` | `fn get_regs(tid: u32, regs: &mut DebugRegs) -> bool` | Read target thread's register state. |
| `set_regs` | `fn set_regs(tid: u32, regs: &DebugRegs) -> bool` | Write register state. Kernel validates RIP/RSP and masks RFLAGS. |
| `read_mem` | `fn read_mem(tid: u32, addr: u64, buf: &mut [u8]) -> usize` | Read memory from target (max 4096 bytes). Returns bytes read. |
| `write_mem` | `fn write_mem(tid: u32, addr: u64, data: &[u8]) -> usize` | Write memory to target (max 256 bytes). Returns bytes written. |
| `set_breakpoint` | `fn set_breakpoint(tid: u32, addr: u64) -> bool` | Set a software breakpoint (INT3) at address. |
| `clear_breakpoint` | `fn clear_breakpoint(tid: u32, addr: u64) -> bool` | Clear a breakpoint, restoring original byte. |
| `single_step` | `fn single_step(tid: u32) -> bool` | Execute one instruction and suspend. Sets RFLAGS.TF. |
| `wait_event` | `fn wait_event(tid: u32, event: &mut DebugEvent) -> u32` | Poll for debug event (non-blocking). Returns event type or 0. |
| `get_memory_map` | `fn get_memory_map(tid: u32, regions: &mut [MemoryRegion]) -> usize` | Get target's virtual memory map. Returns region count. |
| `thread_info_ex` | `fn thread_info_ex(tid: u32, info: &mut ThreadInfoEx) -> bool` | Get extended thread information (128 bytes). |

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
| `WIN_FLAG_SCALE_CONTENT` | `0x80` | Scale window content (compositor-side scaling) |
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
| `APP_MENU_ABOUT` | `0xFFF0` | Standard "About" item ID |
| `APP_MENU_HIDE` | `0xFFF1` | Standard "Hide" item ID |
| `APP_MENU_QUIT` | `0xFFF2` | Standard "Quit" item ID |

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

### Clipboard

| Function | Signature | Description |
|----------|-----------|-------------|
| `clipboard_set` | `fn clipboard_set(text: &str)` | Set clipboard contents (plain text). |
| `clipboard_set_with_format` | `fn clipboard_set_with_format(data: &[u8], format: u32)` | Set clipboard contents with explicit format. |
| `clipboard_get` | `fn clipboard_get() -> Option<String>` | Get clipboard contents as String. Returns None if empty. |

Clipboard format constants: `CLIPBOARD_TEXT` (0), `CLIPBOARD_URI_LIST` (1).

### Desktop Notifications

| Function | Signature | Description |
|----------|-----------|-------------|
| `show_notification` | `fn show_notification(title: &str, message: &str, timeout_ms: u32)` | Show a desktop notification banner. `timeout_ms=0` means default (5 seconds). |

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
