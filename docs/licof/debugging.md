# licof Debugging Guide

This guide documents the diagnostics that are useful while bringing up Linux
ELF64 binaries under `licof`.

## First Checks

Start with:

```sh
licof status
licof repair
```

Then verify the binary path resolves inside the active Linux base:

```sh
licof run /usr/bin/<tool> [args...]
```

The CLI prints `PT_INTERP` diagnostics before spawning:

```text
licof run: PT_INTERP /lib64/ld-linux-x86-64.so.2 -> /System/var/licof/rootfs/lib64/ld-linux-x86-64.so.2
```

If the target is missing, repair or reinstall the packages that provide the
dynamic loader and runtime libraries.

## Serial Log Markers

Kernel Linux-ABI diagnostics are printed on the serial console.

### Unsupported Syscall

```text
licof linux: unsupported syscall nr=<nr> rip=<rip> args=<a1>,<a2>,<a3>,<a4>,<a5>,<a6>
```

This means the Linux syscall table returned `-ENOSYS` for an unknown syscall.
Add the syscall only after checking Linux x86_64 argument order and Linux errno
semantics.

`rseq` returning `ENOSYS` is normal and intentionally quiet.

### `brk` / `sbrk`

```text
licof linux brk: query -> 0x60d000
licof linux brk: current=0x60d000 requested=0x62e000 delta=135168 -> 0x62e000
licof linux brk: failed current=... requested=... delta=...
licof linux sbrk: reject low-limit ...
licof linux sbrk: reject high-limit ...
licof linux sbrk: reject map-page ...
licof linux sbrk: reject phys-oom ...
```

Use these lines when glibc reports allocation failures during dynamic loader
startup. A failed early `brk` can surface as a loader message that looks higher
level than the real kernel failure.

### `mmap`

```text
licof linux mmap: ok addr=0x0 len=0x2000 prot=0x3 flags=0x22 fd=0xffffffff off=0x0 -> 0x100000000
licof linux mmap: alloc failed ...
licof linux mmap: fill failed ... errno=...
licof linux mmap: reject anon fd ...
```

Important flag examples:

| Flags | Meaning |
| --- | --- |
| `0x22` | `MAP_PRIVATE | MAP_ANONYMOUS` |
| `0x12` | `MAP_PRIVATE | MAP_FIXED` |

For anonymous mappings, Linux `fd = -1` can arrive as either
`0xffffffffffffffff` or `0x00000000ffffffff`. Both must be accepted.

### Path Resolution / `open`

Linux `open()` and `openat()` failures print the Linux-visible path, the
translated anyOS path and the final resolved path:

```text
licof linux open: failed errno=5 linux='/lib/x86_64-linux-gnu/libpam.so.0' translated='/System/var/licof/rootfs/lib/x86_64-linux-gnu/libpam.so.0' resolved='/System/var/licof/rootfs/lib/x86_64-linux-gnu/libpam.so.0.83.1'
```

Absolute symlink targets inside the Linux base are resolved relative to
`/System/var/licof/rootfs`, not relative to the anyOS root. This keeps Debian
links such as `/lib/...` inside the Linux base.

## Common Loader Errors

### `FATAL: kernel too old`

Cause: `uname()` reported a kernel release older than glibc accepts.

Expected licof behavior:

```text
sysname = Linux
release = 3.2.0-licof
machine = x86_64
```

If this regresses, check `linux_uname()`.

### `missing PT_INTERP`

The CLI prints:

```text
licof passwd: missing PT_INTERP /lib64/ld-linux-x86-64.so.2
```

Check:

```sh
ls /System/var/licof/rootfs/lib64/
ls /System/var/licof/rootfs/lib/x86_64-linux-gnu/
```

`licof init` does not run runtime repair after bootstrap. If the loader is
present only as a symlink, the kernel loader must resolve that symlink inside
`/System/var/licof/rootfs`. `licof repair` can recreate missing loader and
SONAME symlinks, but it should not copy shared libraries over package
metadata.

### `cannot create cache for search path: Cannot allocate memory`

This is a dynamic-loader allocation failure. Check serial `brk` and `mmap`
logs directly before the message.

Known historical causes:

- high Linux PIE `brk` blocked by the low anyOS heap limit.
- anonymous `mmap` rejected because `fd = 0xffffffff` was not treated as
  Linux `-1`.

### Exit Status `127`

Usually a dynamic loader failure before the program's `main`.

Look for:

- missing interpreter.
- missing shared library.
- `uname()` compatibility failure.
- failed `brk` / `mmap`.
- rejected file-backed mapping.

### Exit Status `139`

Segmentation fault. Check:

- page fault report.
- `!ISR RSP` recovery logs.
- current Linux syscall and return path.
- stack pointer and `AT_*` auxv setup.

## Package Download Debugging

`licof apt` downloads with `libhttp_client` when available and falls back to
`wget`.

Useful messages:

```text
licof download: failed after 4 attempts: ...
licof apt: downloaded package index is not gzip (... first bytes ...)
licof apt: response looks like HTML; archive server returned an error page
licof apt: failed to decompress package index: ...
licof apt: checksum mismatch ...
licof apt: invalid package size ...
```

If the first bytes are `00 00 00 00`, suspect a write/flush/filesystem issue or
a failed downloader that produced a sparse/zeroed file. If the first bytes look
like HTML, the archive returned an error page and the URL or mirror state is
wrong.

## Package Extraction Debugging

Supported `.deb` data members:

```text
data.tar.gz
data.tar.xz
```

Extraction uses `libzip_client`. Tar metadata is used for:

- regular files.
- directories.
- symlink targets.
- hardlink targets.
- mode, uid and gid best-effort.

If dynamic libraries are missing after extraction:

1. Check the package installed count.
2. Check whether the source ELF exists under `lib/x86_64-linux-gnu`.
3. Check whether the SONAME symlink exists and points to the versioned library.
4. Re-run `licof repair` only to recreate missing symlinks, or reinstall the
   package if the versioned library itself is absent.

## Linux Base Debugging

Default paths:

```text
/System/var/licof/rootfs
/System/var/licof/cache
/System/var/licof/db
```

Check dynamic loader:

```sh
ls /System/var/licof/rootfs/lib64/
ls /System/var/licof/rootfs/lib/x86_64-linux-gnu/
```

Check minimal account files when debugging `passwd`:

```sh
ls /System/var/licof/rootfs/etc/
```

At minimum, `/etc/passwd`, `/etc/group`, `/etc/shadow`, `/etc/gshadow`, and
`/etc/nsswitch.conf` should exist. `licof init` creates conservative seed
versions only when they are missing, because Debian maintainer scripts are not
executed during package extraction.

## Adding a New Syscall

1. Confirm the Linux x86_64 syscall number and argument order.
2. Add the constant to `kernel/src/syscall/linux.rs`.
3. Add a match arm in `dispatch`.
4. Translate Linux structs and flags explicitly; do not cast to anyOS structs.
5. Return Linux negative errno.
6. Add serial diagnostics while developing.
7. Remove noisy success diagnostics once the path is stable.
8. Test the real binary that required the syscall.

## When to Keep a Stub

Some syscalls are safe as temporary stubs when glibc only probes them or when
the program tolerates a no-op:

- `rseq`: return `ENOSYS`.
- `madvise`: return success for currently ignored hints.
- selected `prctl` options: minimal support or `EINVAL`.
- `set_robust_list`: success for single-thread startup paths.

Do not stub syscalls that mutate visible state unless the expected Linux
behavior is understood. Silent success can be worse than `ENOSYS`.
