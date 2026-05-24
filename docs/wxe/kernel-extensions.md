# WXE Kernel Extensions

This document describes the kernel-side Windows compatibility layer planned
for `wxe`.

## ABI Personality

anyOS threads already carry an ABI personality for native anyOS and LXE. WXE
adds a third value:

| Personality | Meaning |
| --- | --- |
| `AnyOs` | Native anyOS syscall ABI and loader behavior. |
| `LinuxX86_64` | Linux x86_64 syscall ABI, Linux path translation and Linux ELF64 startup. |
| `WindowsX86_64` | Windows x86_64 NT syscall ABI, Windows path translation, PE32+ startup and WXE DLL environment. |

`SYS_WXE_SPAWN` creates a process with `WindowsX86_64` personality. Fork-style
inheritance is not a Windows primitive and must not reuse LXE `fork` semantics.
Later WXE process creation should be modeled through Windows process APIs
(`CreateProcessW`/`NtCreateUserProcess`) and use a fresh PE loader path.

Normal anyOS spawn and LXE spawn are unchanged. The Windows path is explicit so
native apps and Linux ABI processes never enter Windows syscall semantics by
accident.

## Syscall Entry and Dispatch

x86_64 `SYSCALL` still enters the common kernel syscall path. In
`syscall_dispatch_64`, the current thread personality selects the dispatcher:

- `AnyOs` goes to the normal anyOS syscall table.
- `LinuxX86_64` goes to `kernel/src/syscall/linux/`.
- `WindowsX86_64` goes to `kernel/src/syscall/windows/`.

The WXE syscall ABI is the Windows x64 `ntdll` convention:

| Register/source | Meaning |
| --- | --- |
| `RAX` / `EAX` | WXE NT service number |
| `R10` | arg1 copied from user `RCX` by the `ntdll` syscall stub |
| `RDX` | arg2 |
| `R8` | arg3 |
| `R9` | arg4 |
| user stack | arg5 and later, after the Windows x64 shadow space |
| return `RAX` | `NTSTATUS` or success payload, depending on service |

The WXE-owned `ntdll.dll` is the syscall ABI boundary. Windows syscall numbers
are not stable across Windows releases, so WXE must define an explicit syscall
profile for its own `ntdll.dll` and document that profile. Direct raw syscalls
from third-party binaries are not a Tier-0 compatibility contract.

Unsupported service numbers are logged on the serial console and return
`STATUS_NOT_IMPLEMENTED`:

```text
wxe nt: unsupported service nr=<nr> rip=<rip> args=<a1>,<a2>,<a3>,<a4>,...
```

## Kernel Module Layout

The Windows syscall layer should be split by responsibility, mirroring LXE but
using Windows names and data contracts:

| File | Responsibility |
| --- | --- |
| `kernel/src/syscall/windows/mod.rs` | service constants, dispatch table and tracing |
| `kernel/src/syscall/windows/abi.rs` | user-copy helpers, NTSTATUS helpers, UTF-16 helpers |
| `kernel/src/syscall/windows/path.rs` | drive-letter, DOS path, NT path and object-name translation |
| `kernel/src/syscall/windows/handle.rs` | per-process HANDLE table and object type routing |
| `kernel/src/syscall/windows/file.rs` | `NtCreateFile`, `NtReadFile`, `NtWriteFile`, metadata |
| `kernel/src/syscall/windows/memory.rs` | `NtAllocateVirtualMemory`, sections, map/unmap/protect |
| `kernel/src/syscall/windows/process.rs` | process/thread basics, exit, wait, time, priority |
| `kernel/src/syscall/windows/sync.rs` | events, waits, timers and critical startup synchronization |
| `kernel/src/syscall/windows/console.rs` | console object behavior behind `CONIN$` and `CONOUT$` |
| `kernel/src/syscall/windows/registry.rs` | minimal registry facade, initially read-only or stubbed |

## Initial NT Service Surface

Tier 0 should cover PE loader startup and small console CRT programs:

- Process basics:
  - `NtTerminateProcess`
  - `NtTerminateThread`
  - `NtQueryInformationProcess`
  - `NtQueryInformationThread`
  - `NtWaitForSingleObject`
  - `NtDelayExecution`
- File and console I/O:
  - `NtCreateFile`
  - `NtOpenFile`
  - `NtReadFile`
  - `NtWriteFile`
  - `NtClose`
  - `NtQueryInformationFile`
  - `NtSetInformationFile`
  - `NtFlushBuffersFile`
- Memory:
  - `NtAllocateVirtualMemory`
  - `NtFreeVirtualMemory`
  - `NtProtectVirtualMemory`
  - `NtQueryVirtualMemory`
  - `NtCreateSection`
  - `NtMapViewOfSection`
  - `NtUnmapViewOfSection`
- Time and system data:
  - `NtQuerySystemTime`
  - `NtQueryPerformanceCounter`
  - `NtQuerySystemInformation` for the narrow classes used by CRT startup
- Synchronization:
  - `NtCreateEvent`
  - `NtSetEvent`
  - `NtResetEvent`
  - `NtWaitForMultipleObjects` as soon as console runtimes need it
- Object/query helpers:
  - `NtQueryObject`
  - `NtDuplicateObject`

Each handler returns Windows `NTSTATUS` values. User-mode WXE DLLs are
responsible for mapping NTSTATUS to Win32 `GetLastError()` codes.

## PE32+ Loading

The WXE loader lives in `kernel/src/task/loader/windows.rs`.

Supported first-slice inputs:

- PE32+ x86_64 executable images (`Machine = AMD64`, optional header magic
  `0x20b`).
- Console subsystem binaries.
- Images with normal section tables, import tables, base relocations and
  optional TLS callbacks.
- ASLR-capable and fixed-image binaries where the preferred base is available.

Rejected first-slice inputs:

- PE32 32-bit images.
- ARM/ARM64 Windows images.
- Kernel drivers.
- GUI subsystem images.
- Images requiring unsupported import DLLs or exports.

The loader must:

1. Validate DOS, PE and optional headers.
2. Reserve `SizeOfImage` in the process address space.
3. Map headers and sections with correct executable/writable/NX flags.
4. Apply `.reloc` base relocations when loaded away from `ImageBase`.
5. Build a PEB, TEB and process parameters block.
6. Install a WXE `ntdll.dll` and resolve imports recursively.
7. Run TLS callbacks in Windows order.
8. Enter `ntdll!LdrInitializeThunk` or an equivalent WXE loader thunk that
   calls the program entry with Windows x64 stack alignment and shadow space.

## PEB, TEB and Startup State

Console programs and CRTs expect Windows process structures to exist even when
they never call raw NT services directly.

Tier 0 must provide:

- TEB pointer through `GS`.
- PEB pointer reachable from the TEB.
- Process parameters:
  - image path
  - command line as UTF-16
  - current directory
  - environment block as UTF-16 `KEY=VALUE\0...\0\0`
  - standard handles
- Loader data list entries for the main executable and loaded WXE DLLs.
- Thread-local storage slots required by PE TLS and CRT startup.

The scheduler already stores Linux FS base for LXE. WXE should add Windows GS
base state separately so Linux TLS handling is not reused or regressed.

## Windows Path Translation

WXE supports three path syntaxes:

| Windows input | Meaning |
| --- | --- |
| `C:\path\file.txt` | DOS drive path under configured `C:` root |
| `C:relative\file.txt` | relative to `C:`'s current directory |
| `\path\file.txt` | rooted path on the current drive |
| `\\?\C:\path` | normalized extended DOS path |
| `\??\C:\path` | NT object-manager DOS-device path |

Default drive mappings:

| Drive/object | anyOS path |
| --- | --- |
| `C:` | `/System/var/wxe/drive_c` |
| `C:\Windows\System32` | `/System/var/wxe/drive_c/Windows/System32` |
| `NUL` | discard/read-empty device |
| `CONIN$` | process console input |
| `CONOUT$` | process console output |

Host-root drives such as `Z:` must be opt-in because they weaken the WXE
sandbox boundary.

Path lookup should be case-insensitive inside WXE even when the backing
filesystem is case-sensitive. The first implementation can do component-wise
case folding during path resolution; it must not globally change anyOS VFS
semantics.

## HANDLE Model

Windows `HANDLE`s are not POSIX file descriptors. WXE should maintain a
per-process handle table that can wrap anyOS objects:

- files and directories
- console input/output
- anonymous pipes
- section objects
- events/timers
- process and thread handles

The handle table should live beside, not inside, LXE's Linux fd behavior. Some
low-level storage can be shared with the kernel fd/object primitives, but the
observable semantics must remain Windows-shaped: invalid handle values,
inheritability, close behavior and waitability differ from POSIX fds.

## Memory Layout

WXE uses the same kernel VM infrastructure as anyOS and LXE, but with Windows
process layout rules:

| Region | Policy |
| --- | --- |
| Main image | preferred `ImageBase` when possible, relocated otherwise |
| WXE DLLs | deterministic high user range with ASLR offset |
| Heap/VirtualAlloc | VMA-backed, page-granular, Windows allocation/protect flags |
| Stack | 8 MiB initial stack with guard page, Windows x64 shadow-space entry |
| PEB/TEB | fixed or loader-chosen low canonical user mappings, read/write user |

Do not reuse LXE's ELF `PT_INTERP`, auxv, Linux rootfs or Linux FS-base paths.
WXE has its own PE import graph, PEB/TEB state and GS-base handling.

## Spawn and Exec

Planned native syscall:

```text
SYS_WXE_SPAWN(path_ptr, args_ptr) -> tid or u32::MAX
```

`path_ptr` may be an anyOS path or a Windows path. The userland `wxe` tool
normalizes common command forms before calling the syscall, but the kernel
loader still validates all inputs.

Process creation from inside Windows code should later be implemented through
`CreateProcessW`/`NtCreateUserProcess`, not by exposing LXE-style `fork`.

## Isolation Rules

- WXE code must not add Windows special cases to `kernel/src/syscall/linux/`.
- LXE path state, Linux rootfs and Linux FS base must remain Linux-only.
- WXE DLL loading must not use anyOS ELF `SYS_DLL_LOAD`.
- WXE errors must be `NTSTATUS` in kernel and Win32 errors in user-mode DLLs.
- GUI subsystem binaries must fail before any user code runs in Tier 0.
