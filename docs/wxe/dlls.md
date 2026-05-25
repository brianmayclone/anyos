# WXE DLL Surface

Windows console programs normally call Win32 APIs exported from DLLs, not raw
kernel syscalls. WXE therefore needs real PE DLLs in the WXE root and a clear
routing rule from those DLLs down to the anyOS kernel.

## Layering

```text
Windows PE application
  -> Win32 / CRT DLL exports
     -> kernelbase.dll / kernel32.dll user-mode compatibility code
        -> ntdll.dll NT wrappers
           -> WXE NT syscall dispatcher in the anyOS kernel
```

Only `ntdll.dll` should contain syscall stubs. Higher-level DLLs should stay in
user space unless a specific operation truly belongs in the kernel.

## DLL Location

The default Windows system directory is:

```text
C:\Windows\System32
```

Backed by:

```text
/System/var/wxe/drive_c/Windows/System32
```

`wxe init` creates the directory and installs the WXE-owned DLL set there as
generated PE32+ DLL images.
It also writes route manifests under:

```text
/System/var/wxe/db/nt-services
/System/var/wxe/db/dll-routes
/System/var/wxe/db/ui-routes
/System/var/wxe/db/anyui-bindings
```

Those manifests are the build and loader contract for generated WXE DLLs. They
do not claim that every route is executable yet; they keep the surface explicit
so unsupported imports fail by name instead of through vague loader crashes.
`ui-routes` names the Win32 exports, while `anyui-bindings` pins the concrete
libanyui exports used by the WXE UI backend.

## First DLL Set

Tier 0 console programs need a small but coherent DLL surface:

| DLL | Role |
| --- | --- |
| `ntdll.dll` | NT syscall stubs, loader helpers, PEB/TEB helpers, RTL string/heap primitives |
| `kernelbase.dll` | Main Win32 implementation layer for file, console, process, memory and time APIs |
| `kernel32.dll` | Compatibility exports and forwarders into `kernelbase.dll` |
| `msvcrt.dll` | Legacy C runtime surface for simple MinGW/MSVC-style programs |
| `ucrtbase.dll` | Modern Universal CRT subset |
| `vcruntime140.dll` | Minimal MSVC runtime support, mostly forwarding/stubbing at first |
| `api-ms-win-core-*.dll` | Api-set forwarder DLLs resolving to `kernelbase.dll`/`ntdll.dll` |
| `advapi32.dll` | Minimal stubs for identity/registry probes used by CRTs |
| `shell32.dll` | Minimal stubs for command-line parsing helpers if imported |
| `user32.dll` / `gdi32.dll` | Explicit GUI stubs that return unsupported for Tier 0 |
| `win32u.dll` | User/GDI kernel-call boundary for the later anyOS compositor bridge |
| `comctl32.dll` / `comdlg32.dll` | Common-control/dialog imports, explicit stubs until GUI tier |

Do not silently pretend GUI APIs work. For console acceptance, GUI DLL imports
may be satisfied only when the called exports fail predictably.

## Export Routing Rules

The import resolver must support:

- named imports
- ordinal imports where WXE DLLs intentionally export stable ordinals
- export forwarders, for example:
  - `kernel32.dll!CreateFileW -> kernelbase.dll!CreateFileW`
  - `api-ms-win-core-file-l1-1-0.dll!CreateFileW -> kernelbase.dll!CreateFileW`
- delay-load imports later, after the first static import path is stable

Missing DLLs and missing exports must stop process launch with a diagnostic:

```text
wxe import: missing export kernel32.dll!GetConsoleMode for C:\Tools\app.exe
```

## Building WXE DLLs

WXE DLLs must be PE/COFF images because Windows applications import PE export
tables. There are two acceptable implementation paths:

1. Generate minimal PE DLLs with a host tool such as `buildsystem/wxedll`.
   This keeps the build self-hostable and avoids depending on a MinGW toolchain.
2. Build from C/assembly with a cross PE toolchain only if that dependency is
   made optional and the generated DLLs can still be reproduced by anyOS tools.

The active first path is the PE DLL generator in `libs/libwxecore/src/wxedll.rs`:

- reads a manifest of exports and forwarders
- emits `.text` and `.edata`
- emits syscall stubs for `ntdll.dll`
- emits indirect thunks for kernelbase/kernel32 APIs
- produces deterministic DLLs for `/System/var/wxe/drive_c/Windows/System32`

The current generator emits `.text` and `.edata`, WXE-owned `ntdll` syscall
stubs, PE export forwarders, Win32 wrappers for the first console calls
(`ReadFile`, `WriteFile`, `CloseHandle`, `Sleep`, virtual memory and simple
heap routes), and explicit fallback stubs for routes whose user-mode
implementation is not wired yet. It is intentionally deterministic and runs
from `wxe init` so a repaired WXE root can recreate the DLL set without shipping
Microsoft DLLs.

The kernel WXE loader maps these generated DLLs from the WXE `System32`
directory, resolves named and ordinal imports, follows PE forwarder exports such
as `kernel32.WriteFile -> kernelbase.WriteFile`, patches the process IAT, then
protects sections according to their PE flags. TLS callbacks and GUI subsystem
entry remain gated.

## `ntdll.dll`

`ntdll.dll` is the only DLL that enters the kernel directly. Its syscall stubs
use the Windows x64 convention:

```asm
mov r10, rcx
mov eax, SERVICE_ID
syscall
ret
```

It also owns user-mode helpers that many CRTs expect:

- `RtlGetCurrentPeb`
- `RtlAllocateHeap`, `RtlFreeHeap`, `RtlReAllocateHeap`
- `RtlInitUnicodeString`
- `RtlUnicodeStringToAnsiString`
- `RtlAnsiStringToUnicodeString`
- `RtlDosPathNameToNtPathName_U`
- `RtlGetVersion`
- loader-list helpers used by `GetModuleHandleW` and `GetProcAddress`

Heap helpers may initially route to `NtAllocateVirtualMemory` and a simple
user-mode heap.

Current WXE NT service IDs are fixed by the `win10_19041` WXE profile:

| ID | Export names | Current behavior |
| --- | --- | --- |
| `0x0001` | `NtTerminateProcess`, `ZwTerminateProcess` | Current process exit |
| `0x0002` | `NtTerminateThread`, `ZwTerminateThread` | Current thread exit |
| `0x0003` | `NtClose`, `ZwClose` | Standard handles and fd-backed handles |
| `0x0004` | `NtReadFile`, `ZwReadFile` | fd-backed reads plus stdin |
| `0x0005` | `NtWriteFile`, `ZwWriteFile` | fd-backed writes plus stdout/stderr |
| `0x0006` | `NtDelayExecution`, `ZwDelayExecution` | Relative 100 ns timeout sleeps |
| `0x0007` | `NtQuerySystemTime`, `ZwQuerySystemTime` | Windows FILETIME |
| `0x0008` | `NtQueryPerformanceCounter`, `ZwQueryPerformanceCounter` | uptime-ms counter, 1 kHz frequency |
| `0x0009` | `NtAllocateVirtualMemory`, `ZwAllocateVirtualMemory` | anonymous page allocation |
| `0x000a` | `NtFreeVirtualMemory`, `ZwFreeVirtualMemory` | anonymous unmap |
| `0x000b` | `NtProtectVirtualMemory`, `ZwProtectVirtualMemory` | validates ranges, protection no-op for now |
| `0x000c` | `NtQueryInformationProcess`, `ZwQueryInformationProcess` | class 0 basic info |
| `0x000d` | `NtQueryInformationThread`, `ZwQueryInformationThread` | class 0 basic info |
| `0x000e` | `NtCreateFile`, `ZwCreateFile` | OBJECT_ATTRIBUTES path open/create routed to anyOS fd handles |
| `0x000f` | `NtOpenFile`, `ZwOpenFile` | OBJECT_ATTRIBUTES path open routed to anyOS fd handles |
| `0x0010` | `NtQueryInformationFile`, `ZwQueryInformationFile` | FileStandardInformation for fd-backed handles |
| `0x0100`-`0x010a` | WXE private Win32 helpers | module lookup/export lookup and CreateFile/FileAttributes/GetFileSizeEx/SetFilePointerEx thunks |

The real Windows syscall numbers are intentionally not reused. WXE's generated
`ntdll.dll` must emit stubs for these WXE-owned IDs.

## `kernelbase.dll` / `kernel32.dll`

`kernelbase.dll` should implement most Win32 functions. `kernel32.dll` should
export the legacy names and forward or thunk into `kernelbase.dll`.

The current generated DLLs already return the fixed WXE command line and
environment blocks, expose `RtlGetCurrentPeb`, resolve loaded-module handles
and `GetProcAddress` through the kernel WXE process registry, and route the
first fd-backed file helpers to the kernel path translator.

Minimum console API set:

- Process and command line:
  - `ExitProcess`
  - `GetCommandLineA`, `GetCommandLineW`
  - `GetEnvironmentStringsA`, `GetEnvironmentStringsW`
  - `FreeEnvironmentStringsA`, `FreeEnvironmentStringsW`
  - `GetCurrentProcess`, `GetCurrentThread`
  - `GetCurrentProcessId`, `GetCurrentThreadId`
- Module/import helpers:
  - `GetModuleHandleA`, `GetModuleHandleW`
  - `GetProcAddress`
  - `LoadLibraryA`, `LoadLibraryW`, `LoadLibraryExW`
- File and console:
  - `GetStdHandle`, `SetStdHandle`
  - `CreateFileA`, `CreateFileW`
  - `ReadFile`, `WriteFile`
  - `CloseHandle`
  - `GetFileType`
  - `GetFileSizeEx`
  - `SetFilePointerEx`
  - `FlushFileBuffers`
  - `GetConsoleMode`, `SetConsoleMode`
  - `ReadConsoleA`, `ReadConsoleW`
  - `WriteConsoleA`, `WriteConsoleW`
- Paths and directories:
  - `GetCurrentDirectoryA`, `GetCurrentDirectoryW`
  - `SetCurrentDirectoryA`, `SetCurrentDirectoryW`
  - `GetFullPathNameA`, `GetFullPathNameW`
  - `GetFileAttributesA`, `GetFileAttributesW`
  - `FindFirstFileA/W`, `FindNextFileA/W`, `FindClose`
- Memory and heap:
  - `VirtualAlloc`, `VirtualFree`, `VirtualProtect`, `VirtualQuery`
  - `HeapCreate`, `HeapDestroy`, `HeapAlloc`, `HeapFree`, `HeapReAlloc`
  - `GetProcessHeap`
- Time and synchronization:
  - `Sleep`
  - `QueryPerformanceCounter`, `QueryPerformanceFrequency`
  - `GetSystemTimeAsFileTime`
  - `CreateEventA/W`, `SetEvent`, `ResetEvent`
  - `WaitForSingleObject`, `WaitForMultipleObjects`
- Error and TLS:
  - `GetLastError`, `SetLastError`
  - `TlsAlloc`, `TlsFree`, `TlsGetValue`, `TlsSetValue`
  - `InitializeCriticalSection`, `EnterCriticalSection`, `LeaveCriticalSection`

## CRT DLLs

The first CRT goal is small console programs, not full MSVC compatibility.

`msvcrt.dll` should cover:

- startup dependencies imported by simple MinGW binaries
- stdio over `GetStdHandle`/`ReadFile`/`WriteFile`
- argv/env parsing from the PEB command line and environment
- basic allocation through the WXE heap
- process exit through `ExitProcess`

`ucrtbase.dll` and `vcruntime140.dll` should start as explicit subsets and
grow only when a real console program needs more exports.

## Api-Set DLLs

Modern Windows binaries import api-set DLL names rather than concrete DLLs.
WXE must resolve common api-set names without creating independent
implementations for each one.

Initial api-set handling can be static:

```text
api-ms-win-core-file-l1-1-0.dll      -> kernelbase.dll
api-ms-win-core-processthreads-l1-*  -> kernelbase.dll
api-ms-win-core-memory-l1-*          -> kernelbase.dll
api-ms-win-core-synch-l1-*           -> kernelbase.dll
api-ms-win-core-console-l1-*         -> kernelbase.dll
api-ms-win-core-errorhandling-l1-*   -> kernelbase.dll
api-ms-win-core-libraryloader-l1-*   -> kernelbase.dll
api-ms-win-core-heap-l1-*            -> kernelbase.dll
api-ms-win-core-timezone-l1-*        -> kernelbase.dll
```

Later, WXE can add an api-set schema blob in the PEB if binaries inspect it.

## `user32.dll`, `gdi32.dll` and `win32u.dll`

GUI applications remain outside the first acceptance step, but GUI imports need
an explicit shape now. The WXE route manifests define the first UI surface:

- `user32.dll` owns HWND-facing APIs: `MessageBoxA/W`, class registration,
  `CreateWindowExA/W`, `ShowWindow`, `UpdateWindow`, message retrieval,
  dispatch, timers and standard window procedure entry points.
- `gdi32.dll` owns HDC/HGDIOBJ APIs: compatible DC creation, object selection,
  stock objects, pens, brushes, text measurement/output, rectangle fill and
  basic blits.
- `win32u.dll` owns the lower UI-call boundary used by modern Windows user and
  GDI paths. In WXE it routes to the WXE UI backend, which maps onto
  `libanyui.so` exports such as `anyui_create_window`, `anyui_create_control`,
  `anyui_canvas_draw_text`, `anyui_measure_text` and `anyui_run_once`.

Tier 0 console programs may import these DLLs only if the called exports either
have a harmless console fallback, such as `MessageBoxW` printing a diagnostic
and returning `IDOK`, or fail predictably with a Win32 unsupported-function
error. GUI subsystem images are still rejected by the PE loader until the WXE
UI backend exists.

See [Win32 UI Surface](ui.md) for the implementation plan.

## Version Profile

WXE should expose one named Windows compatibility profile first, for example:

```text
WXE_NT_PROFILE=win10_19041
```

This profile controls:

- `RtlGetVersion`
- PEB version fields
- file version resources on WXE DLLs
- supported api-set aliases
- WXE `ntdll.dll` syscall service IDs

The profile is a compatibility contract for WXE's own DLLs, not a promise that
every Windows 10 syscall number used by arbitrary raw-syscall binaries works.
