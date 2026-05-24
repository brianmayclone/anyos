# WXE CLI, Shell and Root

This document describes the planned userland surface for the Windows
Experience Extension.

## Commands

The first CLI should mirror LXE's shape:

```text
wxe status
wxe init
wxe init --bootstrap-ms --accept-microsoft-licenses
wxe repair
wxe bootstrap-ms --accept-microsoft-licenses
wxe run <windows-pe> [args...]
wxe shell [--builtin]
wxe inspect <windows-pe>
wxe dlls
wxe import-ms <source>
```

Planned responsibilities:

| Command | Purpose |
| --- | --- |
| `wxe status` | Show WXE profile, root path, drive mappings, DLL health and `wxed` status |
| `wxe init` | Create root layout, install WXE DLLs and seed default files |
| `wxe init --bootstrap-ms --accept-microsoft-licenses` | Also download Microsoft-provided bootstrap payloads from official sources |
| `wxe repair` | Reinstall missing WXE DLLs and repair default drive directories |
| `wxe bootstrap-ms` | Download Microsoft-provided bootstrap payloads after explicit license acceptance |
| `wxe run` | Start one Windows console executable through `SYS_WXE_SPAWN` |
| `wxe shell` | Open the WXE shell app or run the PTY bridge mode |
| `wxe inspect` | Print PE machine, subsystem, imports, relocations and missing DLLs/exports |
| `wxe dlls` | List installed WXE DLLs and exported API subsets |
| `wxe import-ms` | Future opt-in import path for Microsoft payloads after license acceptance |

`bin/wxe` should be tiny, with most behavior in `libs/libwxecore`, matching
the LXE split.

## Config

WXE reads policy and paths from `confd` through a `services/wxe` manifest.
Built-in defaults must be enough to bootstrap a fresh system.

Initial defaults:

```text
paths/root=/System/var/wxe
paths/drive_c=/System/var/wxe/drive_c
paths/cache=/System/var/wxe/cache
paths/db=/System/var/wxe/db
profile/nt=win10_19041
drives/c=/System/var/wxe/drive_c
drives/z=
shell/default_drive=C:
shell/default_cwd=\Users\Default
shell/comspec=C:\Windows\System32\cmd.exe
```

`Z:` and other host mappings are empty by default. Enabling a host-root drive
should require an explicit config setting.

## Root Layout

Default filesystem layout:

```text
/System/var/wxe/
  drive_c/
    Windows/
      System32/
        ntdll.dll
        kernel32.dll
        kernelbase.dll
        msvcrt.dll
        ucrtbase.dll
        api-ms-win-core-*.dll
      Temp/
    Users/
      Default/
    Program Files/
    ProgramData/
  cache/
  db/
    dll-manifest
    drive-map
    init-state
```

The root contains WXE-owned compatibility files, not copied Microsoft Windows
system files. Plain `wxe init` must not download Microsoft binaries. If the
user explicitly runs `wxe init --bootstrap-ms --accept-microsoft-licenses`, WXE
downloads the bootstrap payloads listed in
[`microsoft-payloads.md`](microsoft-payloads.md) into `cache/microsoft`.

## Drive Letters

WXE shell and path translation must support:

- `C:` drive switching.
- `C:\absolute\path`.
- `C:relative\path`, relative to `C:`'s remembered current directory.
- `\rooted\on\current\drive`.
- `/` as a tolerated input alias in `wxe run`, normalized to backslashes before
  WXE execution.

Per-drive current directories are part of the WXE process environment, not the
native anyOS CWD. The shell should keep them in its own state and pass the
current directory to `SYS_WXE_SPAWN`.

## WXE Shell

The target shell process is WXE's own `C:\Windows\System32\cmd.exe`, exposed
through the Windows `COMSPEC` convention. WXE must not ship Microsoft's
`cmd.exe`; the file in the WXE root is an anyOS/WXE implementation with
Windows-compatible command behavior.

`command.com` is intentionally out of scope for the x86_64 WXE layer. It is
the DOS/Windows 9x command interpreter and, on old 32-bit NT systems, belonged
to NTVDM-style DOS compatibility. WXE's first ABI target is the NT x86_64
console world, so `cmd.exe` is the right command processor.

Until WXE can start its PE `cmd.exe`, `/System/bin/wxe shell` may fall back to
a bootstrap shell (`wxe shell --builtin`) with the same drive-letter state and
launch path.

Required built-ins:

- `C:`, `D:` etc. for drive switching.
- `cd`, `chdir`
- `dir`
- `type`
- `echo`
- `set`
- `cls`
- `exit`
- executable lookup through `PATH`
- `.exe` extension probing

It must start Windows console programs attached to a PTY-backed console, using
the same general terminal app approach as LXE Shell but with Windows path and
environment semantics.

The command prompt should expose Windows state:

```text
C:\Users\Default>
```

## `wxe run`

`wxe run` accepts either an anyOS path or a Windows path:

```sh
wxe run /Users/Default/Downloads/hello.exe
wxe run C:\Tools\hello.exe
wxe run C:/Tools/hello.exe
```

The CLI resolves obvious anyOS paths before calling `SYS_WXE_SPAWN`, but the
kernel still validates the PE image, subsystem and import graph.

Arguments are passed to the process as a Windows command line. WXE DLLs derive
`argv` for CRTs from that command line using Windows quoting rules.

## `wxed`

`system/daemons/wxed` should mirror `lxed`'s role:

- serialize writes to the WXE root and DLL manifest
- provide runtime leases for `wxe run`/`wxe shell`
- track active WXE processes
- expose status to `wxe status`

The first implementation can be smaller than `lxed`, because WXE does not need
Debian package downloads. It still gives WXE one owner for root repair and DLL
manifest state.

## Shell App

`apps/wxeshell` should mirror the LXE Shell terminal architecture:

- anyui window with a canvas terminal
- PTY bridge mode through `/System/bin/wxe shell --pty-bridge`
- key input translated to console input
- output decoded as UTF-8 first, then UTF-16 console writes through WXE console
  APIs once the console layer grows
- explicit close/kill behavior for the child process

This keeps the first acceptance step user-visible while avoiding GUI Windows
application support.

## Manager App

`apps/wxemanager` can start as a diagnostic manager:

- status and profile
- drive mappings
- installed WXE DLLs and missing exports
- recent WXE process failures
- buttons for `init`, `repair`, `open shell`

It should not be required for the first console acceptance, but planning it now
keeps WXE parallel to LXE's app surface.
