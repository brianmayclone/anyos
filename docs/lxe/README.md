# lxe Documentation

`lxe` is the Linux Experience Extension for anyOS. It is the direct
Linux ELF64 compatibility path and runs selected Linux x86_64 binaries on the
anyOS kernel without starting a Linux VM.

ASL remains the VM-first Linux environment for full distributions, kernel
modules, systemd-heavy workloads and high compatibility. `lxe` is an
opt-in, process-level ABI personality for smaller Linux programs and for
incrementally bootstrapping one Debian-based Linux base inside an anyOS
directory.

## Documents

- [Kernel Extensions](kernel-extensions.md)
  describes the kernel-side ABI personality, Linux syscall dispatch, ELF64
  loader, memory layout, Linux base path translation and current compatibility
  boundaries.
- [CLI and Linux Base](cli.md)
  describes `/System/bin/lxe`, Linux base initialization, Debian package installation,
  configuration keys and the download/extract pipeline.
- [Debugging](debugging.md)
  describes the current diagnostics for missing syscalls, loader failures,
  package downloads, gzip/tar extraction and Linux process crashes.
- [Roadmap](roadmap.md)
  tracks the intended compatibility tiers and next milestones.

## Current Scope

The current implementation targets Linux x86_64 ELF64 binaries. The main
supported path is one Debian 12 Bookworm amd64 Linux base bootstrapped from
`deb.debian.org`:

```sh
lxe init
lxe apt install <package>
lxe run /usr/bin/<tool> [args...]
```

The kernel can load dynamic glibc binaries through `PT_INTERP`, build a
Linux-style initial stack with `argc`/`argv`/`envp`/`auxv`, dispatch a growing
subset of Linux syscalls and translate Linux paths into the active lxe Linux
base.

The implementation is still intentionally narrow. Unsupported syscalls return
Linux `-ENOSYS`; known compatibility shims are added only when a real binary
needs them and when the anyOS behavior can be made explicit.

## Main Components

| Area | Files |
| --- | --- |
| CLI entry point | `bin/lxe/src/main.rs` |
| Core library | `libs/liblxecore/src` |
| CLI config / confd manifest | `libs/liblxecore/src/config.rs` |
| CLI model types | `libs/liblxecore/src/model.rs` |
| Linux syscall dispatch | `kernel/src/syscall/linux/` |
| lxe spawn syscall | `kernel/src/syscall/handlers/process.rs` |
| ELF64 loader / Linux initial stack | `kernel/src/task/loader.rs` |
| ABI personality | `kernel/src/task/abi.rs`, scheduler thread state |
| VMA / mmap allocation | `kernel/src/memory/vma.rs`, `kernel/src/memory/user_vmap.rs` |

## Design Rules

- A process is either native anyOS or `LinuxX86_64`; the Linux path is selected
  by kernel thread ABI personality.
- Linux compatibility uses Linux errno semantics: syscall errors are returned
  as negative errno values in `RAX`.
- Linux paths are never allowed to escape the active lxe Linux base.
- The Linux base is ordinary anyOS filesystem content under
  `/System/var/lxe/rootfs`.
- Debian package maintainer scripts are not executed by `lxe`.
- Package symlinks are preserved as symlinks. If symlink creation or
  verification fails, package extraction fails instead of copying the target
  over the link. Hardlinks are materialized because the package archive does
  not currently have a native hardlink operation in the anyOS fs API.
- `lxe init` does not run runtime repair after bootstrap; broken Linux links
  must be fixed in the symlink-aware loader/VFS path instead of by copying
  libraries over package metadata.
- Installed-package markers must prove their payload paths still exist. Stale
  markers are ignored so a half-written Linux base cannot be reported as a
  complete bootstrap.
- `lxe init` writes `<paths/db>/bootstrap-state` while it runs. The file is
  recomputed from validated package markers and lists installed, missing and
  failed bootstrap seed packages so a later `lxe init` can continue.
- The CLI reads policy and paths from `confd` via the `services/lxe`
  manifest. Built-in defaults exist so a system can bootstrap without a
  pre-existing config tree.

## Fast Smoke Flow

```sh
lxe status
lxe init
lxe run /usr/bin/passwd root
```

During active development, also watch the serial log for lines beginning with:

```text
lxe linux:
lxe linux brk:
lxe linux mmap:
lxe linux sbrk:
```

Those lines come from the kernel Linux-ABI bridge and usually identify the next
missing syscall, memory mapping issue or loader assumption.
