# licof Documentation

`licof` is the Linux Compatibility Framework for anyOS. It is the direct
Linux ELF64 compatibility path and runs selected Linux x86_64 binaries on the
anyOS kernel without starting a Linux VM.

ASL remains the VM-first Linux environment for full distributions, kernel
modules, systemd-heavy workloads and high compatibility. `licof` is an
opt-in, process-level ABI personality for smaller Linux programs and for
incrementally bootstrapping a Debian userland inside an anyOS directory.

## Documents

- [Kernel Extensions](kernel-extensions.md)
  describes the kernel-side ABI personality, Linux syscall dispatch, ELF64
  loader, memory layout, rootfs path translation and current compatibility
  boundaries.
- [CLI and Rootfs](cli.md)
  describes `/System/bin/licof`, rootfs creation, Debian package installation,
  configuration keys and the download/extract pipeline.
- [Debugging](debugging.md)
  describes the current diagnostics for missing syscalls, loader failures,
  package downloads, gzip/tar extraction and Linux process crashes.
- [Roadmap](roadmap.md)
  tracks the intended compatibility tiers and next milestones.

## Current Scope

The current implementation targets Linux x86_64 ELF64 binaries. The main
supported path is a Debian Wheezy amd64 rootfs bootstrapped from
`archive.debian.org`:

```sh
licof rootfs create debian
licof apt install <package>
licof run /System/var/licof/rootfs/debian/usr/bin/<tool> [args...]
```

The kernel can load dynamic glibc binaries through `PT_INTERP`, build a
Linux-style initial stack with `argc`/`argv`/`envp`/`auxv`, dispatch a growing
subset of Linux syscalls and translate Linux paths into a selected licof
rootfs.

The implementation is still intentionally narrow. Unsupported syscalls return
Linux `-ENOSYS`; known compatibility shims are added only when a real binary
needs them and when the anyOS behavior can be made explicit.

## Main Components

| Area | Files |
| --- | --- |
| CLI | `bin/licof/src/main.rs` |
| CLI config / confd manifest | `bin/licof/src/config.rs` |
| CLI model types | `bin/licof/src/model.rs` |
| Linux syscall dispatch | `kernel/src/syscall/linux.rs` |
| licof spawn syscall | `kernel/src/syscall/handlers/process.rs` |
| ELF64 loader / Linux initial stack | `kernel/src/task/loader.rs` |
| ABI personality | `kernel/src/task/abi.rs`, scheduler thread state |
| VMA / mmap allocation | `kernel/src/memory/vma.rs`, `kernel/src/memory/user_vmap.rs` |

## Design Rules

- A process is either native anyOS or `LinuxX86_64`; the Linux path is selected
  by kernel thread ABI personality.
- Linux compatibility uses Linux errno semantics: syscall errors are returned
  as negative errno values in `RAX`.
- Linux paths are never allowed to escape the selected licof rootfs.
- The rootfs is ordinary anyOS filesystem content under `/System/var/licof`.
- Debian package maintainer scripts are not executed by `licof`.
- Symlinks and hardlinks from packages are materialized best-effort. Important
  runtime links such as `/lib64/ld-linux-x86-64.so.2` are repaired after
  package extraction.
- The CLI reads policy and paths from `confd` via the `services/licof`
  manifest. Built-in defaults exist so a system can bootstrap without a
  pre-existing config tree.

## Fast Smoke Flow

```sh
licof status
licof rootfs create debian
licof run /System/var/licof/rootfs/debian/usr/bin/passwd root
```

During active development, also watch the serial log for lines beginning with:

```text
licof linux:
licof linux brk:
licof linux mmap:
licof linux sbrk:
```

Those lines come from the kernel Linux-ABI bridge and usually identify the next
missing syscall, memory mapping issue or loader assumption.
