# lxe Kernel Extensions

This document describes the kernel-side Linux compatibility layer used by
`lxe`.

## ABI Personality

anyOS threads carry an ABI personality. The relevant values are:

| Personality | Meaning |
| --- | --- |
| `AnyOs` | Native anyOS syscall ABI and loader behavior. |
| `LinuxX86_64` | Linux x86_64 syscall ABI, Linux path translation and Linux ELF64 startup. |

`SYS_LXE_SPAWN` creates a process with `LinuxX86_64` personality. Forked
Linux children inherit the active Linux base path, FS base, mmap state and pipe
state through the scheduler fork snapshot.

Normal anyOS `spawn` is unchanged. The Linux path is explicit so native apps do
not accidentally enter Linux syscall semantics.

## Syscall Entry and Dispatch

x86_64 `SYSCALL` still enters the common kernel syscall path. In
`syscall_dispatch_64`, the current thread personality decides which dispatcher
is used:

- `AnyOs` goes to the normal anyOS syscall table.
- `LinuxX86_64` goes to `kernel/src/syscall/linux/`.

Linux syscall arguments follow the Linux x86_64 convention:

| Register | Meaning |
| --- | --- |
| `RAX` | Linux syscall number |
| `RDI` | arg1 |
| `RSI` | arg2 |
| `RDX` | arg3 |
| `R10` | arg4 |
| `R8` | arg5 |
| `R9` | arg6 |
| return `RAX` | success value or negative Linux errno |

Unsupported syscall numbers are logged on the serial console and return
`-ENOSYS`:

```text
lxe linux: unsupported syscall nr=<nr> rip=<rip> args=<a1>,<a2>,<a3>,<a4>,<a5>,<a6>
```

`rseq` is intentionally quiet and returns `-ENOSYS`, because glibc commonly
probes it and treats `ENOSYS` as acceptable.

The Linux syscall layer is split by responsibility:

| File | Responsibility |
| --- | --- |
| `kernel/src/syscall/linux/mod.rs` | syscall constants and dispatch table |
| `kernel/src/syscall/linux/abi.rs` | user-copy helpers and errno conversion |
| `kernel/src/syscall/linux/path.rs` | Linux base translation and symlink resolution |
| `kernel/src/syscall/linux/procfs.rs` | small pseudo `/proc` files |
| `kernel/src/syscall/linux/fs.rs` | path, metadata and directory syscalls |
| `kernel/src/syscall/linux/io.rs` | fd I/O, polling, fcntl and ioctl |
| `kernel/src/syscall/linux/memory.rs` | `brk`, `mmap`, `mprotect` and `arch_prctl` |
| `kernel/src/syscall/linux/process.rs` | identity, time, signals, rlimits and process shims |

## Implemented Linux Syscall Surface

The current table covers the startup and package bootstrap path for dynamic
glibc binaries. It includes:

- Process basics: `exit`, `exit_group`, `getpid`, `getppid`, `gettid`,
  `set_tid_address`, `set_robust_list`.
- File I/O: `read`, `write`, `close`, `open`, `openat`, `lseek`, `pread64`,
  `readv`, `writev`, `fsync`, `fdatasync`.
- Metadata: `stat`, `lstat`, `fstat`, `newfstatat`, `statfs`, `fstatfs`,
  `access`, `faccessat`, `readlink`, `readlinkat`, `getdents64`.
- Filesystem mutation: `mkdir`, `mkdirat`, `unlink`, `unlinkat`, `rename`,
  `renameat`, `creat`, `truncate`, `ftruncate`, `chmod`, `fchmod`,
  `fchmodat`, `chown`, `lchown`, `fchown`, `fchownat`, `utimensat`.
- Memory: `brk`, `mmap`, `munmap`, `mprotect`, `madvise`.
- Identity/capability stubs: `getuid`, `geteuid`, `getgid`, `getegid`,
  `setuid`, `setgid`, `getgroups`, `setgroups`, `setresuid`, `getresuid`,
  `setresgid`, `getresgid`, `setfsuid`, `setfsgid`, `capget`, `capset`.
- Runtime support: `arch_prctl`, `futex`, `clock_gettime`, `gettimeofday`,
  `time`, `sysinfo`, `uname`, `getrandom`, `getrlimit`, `setrlimit`,
  `prlimit64`, `fcntl`, `ioctl`, `poll`, `nanosleep`.
- Pipes and fd duplication: `pipe`, `pipe2`, `dup`, `dup2`, `dup3`.
- Network probes: `socket` returns `-EAFNOSUPPORT` for now so glibc/NSS probes
  for services such as `nscd` fail cleanly and fall back to local files.
  The common AF_UNIX stream probe is intentionally quiet.

Many of these are pragmatic shims, not full Linux implementations. For
example, `mprotect` currently validates enough for loader flows, `futex` only
implements the minimal single-thread startup behavior, and `ioctl` only covers
terminal requests required by early userland.

## Errno Mapping

Linux handlers must return Linux errno values, not anyOS sentinel values.
Helpers translate anyOS `u32::MAX` style failures into Linux negative errno
where possible.

The dispatcher rule is:

```text
success: non-negative RAX
error:   -errno in RAX
```

Examples:

| Condition | Linux errno |
| --- | --- |
| invalid pointer | `EFAULT` |
| bad fd | `EBADF` |
| not found | `ENOENT` |
| permission denied | `EACCES` / `EPERM` |
| unsupported syscall or unsupported mode | `ENOSYS` |
| allocation failure | `ENOMEM` |

## Linux Path Translation

Each Linux thread stores the active lxe Linux base path. Absolute Linux paths
are translated by prefixing that base:

```text
/usr/bin/passwd
  -> /System/var/lxe/rootfs/usr/bin/passwd
```

The kernel currently uses one Linux base:

```text
/System/var/lxe/rootfs
```

Relative paths are resolved against the Linux thread CWD, then translated. At
spawn time the Linux CWD is set to `/`.

Symlinks inside the Linux base are resolved with Linux-rooted semantics:

- relative symlink targets are resolved relative to the symlink's parent
  directory.
- absolute symlink targets such as `/lib/...` stay inside
  `/System/var/lxe/rootfs`, not the anyOS root.

The ELF loader applies the same rule while resolving `PT_INTERP`, so
`/lib64/ld-linux-x86-64.so.2` may be a real Debian symlink.

Special path handling currently includes:

- `/dev/null` and common terminal paths map to safe anyOS fd behavior.
- `/proc` and `/sys` are limited; they are not complete virtual filesystems.
- `AT_FDCWD`, `AT_EMPTY_PATH`, `AT_SYMLINK_NOFOLLOW` and `AT_REMOVEDIR` are
  handled where needed by the implemented `*at` syscalls.

## ELF64 Loading

The Linux loader path is implemented in `kernel/src/task/loader.rs`.

Supported inputs:

- ELF64 only.
- Linux x86_64 static or dynamic binaries.
- `ET_EXEC` and `ET_DYN`/PIE.
- `PT_INTERP` with a single dynamic loader.

Rejected inputs:

- ELF32 binaries.
- Nested interpreters.
- Invalid or out-of-bounds program headers.
- Segments outside canonical low-half user space.

### Load Bias

Dynamic objects are mapped with fixed load biases:

| Object | Base |
| --- | --- |
| main `ET_DYN` binary | `0x0000_5555_0000_0000` |
| dynamic interpreter | `0x0000_7000_0000_0000` |

This gives glibc and the main executable separate, predictable regions while
the rest of the kernel gradually grows Linux-like address-space behavior.

### Initial Stack

The Linux initial stack contains:

- `argc`
- `argv[]`
- `envp[]`
- `auxv[]`
- strings
- 16 bytes for `AT_RANDOM`
- `AT_PLATFORM = "x86_64"`

Current default environment:

```text
PATH=/usr/bin:/bin
HOME=/root
PWD=/
TERM=xterm-256color
SHELL=/bin/bash
USER=root
LOGNAME=root
PS1=#<space>
LXE=1
```

Auxiliary vector entries include:

| Entry | Purpose |
| --- | --- |
| `AT_PHDR` | program header table address |
| `AT_PHENT` | program header entry size |
| `AT_PHNUM` | program header count |
| `AT_PAGESZ` | page size |
| `AT_BASE` | interpreter base |
| `AT_ENTRY` | main executable entry |
| `AT_UID`, `AT_EUID`, `AT_GID`, `AT_EGID` | identity |
| `AT_PLATFORM` | platform string |
| `AT_RANDOM` | random bytes |
| `AT_EXECFN` | executable path |

## Memory Layout

Native anyOS keeps historical low user-space regions. Linux processes use the
same kernel VM infrastructure but omit the low identity window so Linux
`ET_EXEC` binaries around `0x400000` can be mapped safely.

Important Linux-related regions:

| Region | Address |
| --- | --- |
| Linux main PIE bias | `0x0000_5555_0000_0000` |
| Linux interpreter bias | `0x0000_7000_0000_0000` |
| high mmap base | `0x0000_0001_0000_0000` |
| user stack top | `0x0000_7FFF_FFFF_F000` |

### `brk`

`brk` uses the thread's current break. Low legacy heaps keep the normal anyOS
guards around DLIB and low mmap regions. High Linux PIE heaps can grow in the
canonical-low half, but are stopped before the fixed interpreter region at
`0x0000_7000_0000_0000`.

### `mmap`

Anonymous and file-backed private mappings are routed through the VMA
allocator. Linux `MAP_ANONYMOUS` accepts `fd = -1` in both common forms:

```text
0xffffffffffffffff
0x00000000ffffffff
```

The second form matters because some call sites pass an `int` `-1` through a
32-bit register write before the syscall.

## Terminal and Pipes

`SYS_LXE_SPAWN` inherits stdin/stdout pipe state from the caller. This lets
Linux children started from Terminal participate in interactive flows such as
`passwd`.

TTY support is still minimal. `isatty`, selected `ioctl` requests and
stdin/stdout/stderr `fstat` behavior are implemented only to the degree needed
by the current bootstrap path.

## Crash Recovery Notes

Linux binaries stress syscall and interrupt paths differently than native
apps. The x86 interrupt entry now detects user-stack RSP use before the first
push and switches to a recovery stack instead of escalating to double fault.
This protects against malformed or user-mode frames during Linux process
startup and syscall return paths.

## Compatibility Boundaries

Current known limitations:

- No complete `/proc`, `/sys` or `/dev`.
- No full signal model.
- No full pthread/futex implementation.
- No `clone`-based threading support for general Linux programs yet.
- No maintainer script execution for Debian packages.
- `ioctl`, `termios`, `mprotect`, `futex`, `prctl` and capability syscalls are
  partial shims.
- Linux security semantics are mapped onto existing anyOS users, permissions
  and capabilities only where currently needed.
