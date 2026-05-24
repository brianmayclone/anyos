# WXE Roadmap

## Goal

`wxe` is the Windows counterpart to LXE: a direct Windows PE/COFF and NT ABI
path on the anyOS kernel, without a hypervisor.

The first success is not "all Windows applications". The first success is a
clean console slice:

1. Start PE32+ console applications.
2. Load WXE-provided Windows DLLs.
3. Route DLL low-level operations to Windows-shaped NT services in the kernel.
4. Support Windows paths and drive letters in WXE Shell.
5. Keep native anyOS and LXE behavior unchanged.

## Product Boundaries

- WXE is not a VM and does not use a Windows kernel.
- WXE implements a Windows x86_64 user-mode ABI profile on the anyOS kernel.
- WXE is opt-in: processes run as `AnyOs`, `LinuxX86_64` or `WindowsX86_64`.
- WXE uses WXE-owned DLLs. It does not ship Microsoft DLLs.
- Console applications are the first target.
- GUI applications come later.

## Architecture

### Kernel

- Add `AbiPersonality::WindowsX86_64`.
- Add `kernel/src/syscall/windows/` with NT service dispatch.
- Add `SYS_WXE_SPAWN`.
- Add `kernel/src/task/loader/windows.rs`.
- Add Windows process metadata:
  - WXE root/drive mapping reference
  - PEB/TEB addresses
  - Windows GS base
  - per-drive current directories
  - Windows handle table
  - last Windows NT service for diagnostics
- Ensure scheduler/context-switch code treats Linux FS base and Windows GS base
  as separate ABI state.

### Loader

- Parse PE32+.
- Reject unsupported machine/subsystem early.
- Map sections with correct flags.
- Apply base relocations.
- Resolve imports from WXE DLLs.
- Support export forwarders.
- Build PEB, TEB and process parameters.
- Install standard handles.
- Run TLS callbacks.
- Enter the image through a WXE loader thunk.

### DLLs

- Build or generate PE DLLs for:
  - `ntdll.dll`
  - `kernelbase.dll`
  - `kernel32.dll`
  - `msvcrt.dll`
  - `ucrtbase.dll`
  - `vcruntime140.dll`
  - common `api-ms-win-core-*.dll` forwarders
  - minimal stubs for `advapi32.dll`, `shell32.dll`, `user32.dll`, `gdi32.dll`
- Keep syscall stubs only in `ntdll.dll`.
- Keep Win32 policy in user-mode DLL code.

### Userland

- `/System/bin/wxe` with `status`, `init`, `repair`, `run`, `shell`,
  `inspect`, `dlls`.
- `libs/libwxecore` for config, PE inspection, root layout and CLI logic.
- `system/daemons/wxed` for root/DLL manifest ownership and runtime leases.
- `apps/wxeshell` for the first graphical WXE console.
- `apps/wxemanager` for diagnostics after the core slice is stable.

## Compatibility Tiers

### Tier 0: Static PE Console Smoke

Goal: hand-built or tiny PE32+ console binaries with minimal imports.

Kernel:

- `WindowsX86_64` personality.
- `SYS_WXE_SPAWN`.
- PE loader skeleton.
- `NtTerminateProcess`
- `NtWriteFile`
- `NtReadFile`
- `NtClose`
- `NtAllocateVirtualMemory`
- `NtFreeVirtualMemory`
- `NtQuerySystemTime`

Userland:

- `wxe status`
- `wxe init`
- `wxe run`
- `wxe inspect`
- generated `ntdll.dll`
- minimal `kernel32.dll`/`kernelbase.dll` exports for `ExitProcess`,
  `GetStdHandle`, `WriteFile`, `ReadFile`, `GetLastError`, `SetLastError`

Acceptance:

```text
wxe run C:\Tests\hello.exe
```

prints text and exits with status 0.

### Tier 1: CRT Console Programs

Goal: small console programs built with common C runtimes.

Add:

- import recursion and forwarders
- relocations for ASLR
- PEB/TEB and process parameters
- command-line and environment blocks
- TLS callbacks
- `msvcrt.dll` subset
- `ucrtbase.dll` subset
- heap APIs
- file APIs
- `GetCommandLineW/A`
- `GetEnvironmentStringsW/A`
- `GetModuleHandleW/A`
- `GetProcAddress`
- `LoadLibraryW/A`
- `VirtualAlloc`/`VirtualFree`/`VirtualProtect`

Acceptance:

```text
args.exe one two
env.exe
fileio.exe
```

run in WXE Shell with Windows paths.

### Tier 2: WXE Shell and Drive Semantics

Goal: a usable shell for console applications.

Add:

- `apps/wxeshell`
- `wxe shell --pty-bridge`
- drive-letter state
- executable search through `PATH`
- `.exe` probing
- `dir`, `cd`, `type`, `set`, `echo`, `cls`, `exit`
- case-insensitive component lookup
- `CONIN$`, `CONOUT$`, `NUL`
- `GetConsoleMode`, `SetConsoleMode`, `ReadConsoleW`, `WriteConsoleW`

Acceptance:

```text
C:
cd \Users\Default
hello.exe
type C:\Windows\System32\drivers\etc\hosts
```

works from WXE Shell.

### Tier 3: Broader Console Runtime

Goal: real-world CLI tools that use more Windows APIs.

Add:

- events and wait APIs
- pipes and inherited handles
- `CreateProcessW` for child console programs
- directory enumeration APIs
- more file metadata APIs
- minimal registry facade for runtime probes
- codepage/Unicode conversion subset
- structured exception basics for CRT needs

Acceptance:

- a small tree of Windows CLI tools can spawn child programs, use pipes and
  read/write files below `C:`.

### Tier 4: GUI Preparation

Goal: prepare without launching GUI apps yet.

Add:

- explicit `user32.dll`/`gdi32.dll` unsupported stubs
- GUI subsystem detection and error UX
- design notes for future HWND/message loop/compositor bridge

GUI applications are still rejected in this tier.

## First Implementation Phase

1. Add documentation and reserve WXE syscall/profile names.
2. Add `AbiPersonality::WindowsX86_64` without changing dispatch behavior yet.
3. Add `SYS_WXE_SPAWN` constant stubs returning failure until loader exists.
4. Add `libs/libwxecore` with PE inspection and config defaults.
5. Add `bin/wxe status`, `wxe inspect`, `wxe init` root layout.
6. Add PE32+ loader validation and section mapping.
7. Add WXE DLL generator for `ntdll.dll`, `kernelbase.dll`, `kernel32.dll`.
8. Add Windows syscall dispatcher with Tier-0 NT services.
9. Add `wxe run` for `hello.exe`.
10. Add WXE Shell drive-letter state and PTY bridge.

## Risk Register

- Windows x64 syscall numbers are version-specific. WXE must own a documented
  `ntdll.dll` service profile instead of depending on arbitrary raw syscall
  numbers from third-party binaries.
- Many console programs depend on PEB/TEB details before `main` runs.
- CRT startup depends heavily on command-line quoting, environment blocks,
  TLS callbacks and heap behavior.
- Windows `HANDLE` semantics differ from POSIX fds; leaking fd assumptions into
  WXE will cause subtle compatibility bugs.
- Case-insensitive lookup must be local to WXE path resolution.
- GUI DLL stubs must not accidentally imply GUI support.
- Shared syscall entry and scheduler changes can regress LXE if ABI-specific
  state is reused incorrectly.

## Non-Goals for First Acceptance

- Running GUI subsystem applications.
- Running 32-bit Windows PE32 applications.
- Loading Microsoft system DLLs.
- Supporting kernel drivers.
- Implementing services, COM, MSI installers or full registry semantics.
- Supporting anti-cheat, DRM or direct raw-syscall compatibility tricks.
