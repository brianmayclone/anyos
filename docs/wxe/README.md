# WXE Documentation

`wxe` is the Windows Experience Extension for anyOS. It is the direct
Windows PE/COFF compatibility path and runs selected Windows x86_64
applications on the anyOS kernel without starting a Windows VM.

LXE remains the Linux ABI layer. WXE must be implemented as a third, explicit
ABI personality beside native anyOS and LXE. The first acceptance target is
not desktop Windows compatibility; it is a stable console vertical slice:

```text
wxe init
wxe shell
C:
cd \Users\Default
hello.exe
```

The shell must understand Windows drive letters and paths, console programs
must receive a Windows-style process environment, and the Windows DLLs needed
for console startup must route their low-level work onto the anyOS kernel.

## Documents

- [Kernel Extensions](kernel-extensions.md)
  describes the kernel-side ABI personality, Windows NT syscall dispatch,
  PE/COFF loader, handle model, memory layout and path translation.
- [CLI, Shell and WXE Root](cli.md)
  describes `/System/bin/wxe`, the WXE root layout, drive-letter mapping,
  shell behavior and initialization flow.
- [DLL Surface](dlls.md)
  describes the initial Windows DLL set, export routing rules and the split
  between user-mode Win32 wrappers and kernel NT services.
- [Debugging](debugging.md)
  describes tracing, loader diagnostics, missing DLL/export reporting and
  smoke-test expectations.
- [Microsoft Payload Policy](microsoft-payloads.md)
  describes the license boundary for optional Microsoft-provided tools.
- [Roadmap](roadmap.md)
  tracks the intended compatibility tiers and next milestones.

## Current Scope

The first implementation targets Windows x86_64 PE32+ console binaries.
Unsupported inputs must fail visibly and safely:

- PE32 32-bit binaries are out of the first slice.
- GUI subsystem binaries are detected but not launched yet.
- Native packed binaries that issue raw Windows syscalls without going through
  the WXE `ntdll.dll` profile are out of scope.
- Kernel drivers, services, COM desktop integration and registry-heavy
  installers are out of scope.

The implementation should be broad enough for small console programs compiled
with common Windows C runtimes, but narrow enough that unsupported Windows
behavior is explicit instead of accidentally corrupting native anyOS or LXE
state.

## Main Components

| Area | Planned files |
| --- | --- |
| CLI entry point | `bin/wxe/src/main.rs` |
| Core library | `libs/libwxecore/src` |
| CLI config / confd manifest | `libs/libwxecore/src/config.rs` |
| PE/COFF inspection | `libs/libwxecore/src/pe.rs` |
| WXE root and drives | `libs/libwxecore/src/rootfs.rs`, `kernel/src/syscall/windows/path.rs` |
| Windows syscall dispatch | `kernel/src/syscall/windows/` |
| WXE spawn syscall | `kernel/src/syscall/handlers/process.rs` |
| PE32+ loader | `kernel/src/task/loader/windows.rs` |
| ABI personality | `kernel/src/task/abi.rs`, scheduler thread state |
| WXE DLL sources/generator | `libs/libwxe_dlls/` or `buildsystem/wxedll` |
| Runtime broker | `system/daemons/wxed/` |
| Shell app | `apps/wxeshell/` |
| Manager app | `apps/wxemanager/` |

## Design Rules

- A process is exactly one of `AnyOs`, `LinuxX86_64` or `WindowsX86_64`.
- Normal anyOS `spawn` and `SYS_LXE_SPAWN` behavior must remain unchanged.
- WXE is entered only through `SYS_WXE_SPAWN`, `wxe run` or WXE exec-style
  replacement inside an existing WXE process.
- Windows console compatibility is layered:
  - PE loader maps the executable and WXE DLLs.
  - `ntdll.dll` exposes the NT syscall boundary.
  - `kernel32.dll`, `kernelbase.dll`, CRT DLLs and api-set DLLs are user-mode
    compatibility libraries that route to `ntdll.dll` where needed.
  - Kernel code implements NT-style services, not high-level Win32 policy.
- WXE paths are Windows paths until translated by the WXE path layer.
- Windows drive mappings are ordinary anyOS directories. The default `C:` root
  is `/System/var/wxe/drive_c`.
- The first shell must support per-drive current directories, backslash paths
  and case-insensitive lookup where the backing filesystem allows it.
- Unsupported NT syscalls return an NTSTATUS failure, not anyOS or Linux errno.
- Missing DLLs and missing exports must be reported with DLL/export names.
- GUI DLLs may exist as explicit stubs, but GUI applications are rejected until
  the later GUI milestone.

## Fast Smoke Flow

```sh
wxe status
wxe init
wxe shell
```

Inside WXE Shell:

```text
C:
cd \Users\Default
hello.exe
echo %CD%
type C:\Windows\System32\drivers\etc\hosts
```

During development, watch the serial log for lines beginning with:

```text
wxe loader:
wxe import:
wxe nt:
wxe path:
wxe handle:
```

Those lines should identify the next missing PE loader feature, DLL export,
NT syscall, path translation rule or handle/object behavior.
