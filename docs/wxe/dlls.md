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

`wxe init` creates the directory and installs the WXE-owned DLL set there.

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

The preferred first path is a PE DLL generator:

- reads a manifest of exports and forwarders
- emits `.text`, `.rdata`, `.edata` and relocation data
- emits syscall stubs for `ntdll.dll`
- emits indirect thunks for kernelbase/kernel32 APIs
- produces deterministic DLLs for `/System/var/wxe/drive_c/Windows/System32`

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

## `kernelbase.dll` / `kernel32.dll`

`kernelbase.dll` should implement most Win32 functions. `kernel32.dll` should
export the legacy names and forward or thunk into `kernelbase.dll`.

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
